use std::time::Instant;

use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::window::CursorIcon;

use super::camera::Camera;

/// Minimum release speed, in px/s, for a drag to keep coasting.
const FLICK_SPEED: f32 = 50.0;
/// Coasting stops once it decays below this speed, in px/s.
const STOP_SPEED: f32 = 15.0;
/// Coasting velocity halves every this many seconds.
const HALF_LIFE: f32 = 0.3;
/// A release later than this after the last cursor move is a hold, not a
/// flick, in seconds.
const FLICK_TIMEOUT: f32 = 0.1;
/// Bounds on the zoom glide's half-life, in seconds. The glide adapts to
/// the wheel-event cadence: its half-life tracks the smoothed gap between
/// recent events, so dense events (active trackpad scrolling) zoom
/// near-instantly while sparse ones (a trackpad's synthesized momentum
/// tail, single mouse-wheel notches) are interpolated across the gap that
/// would otherwise show as a step.
const ZOOM_HALF_LIFE_MIN: f32 = 0.01;
const ZOOM_HALF_LIFE_MAX: f32 = 0.1;
/// Cap on a wheel-event gap sample, in seconds; longer pauses just mean
/// "fresh scroll", not an extremely slow cadence.
const WHEEL_GAP_CAP: f32 = 0.25;
/// The zoom coast velocity halves every this many seconds once wheel
/// events stop feeding it.
const ZOOM_COAST_HALF_LIFE: f32 = 0.15;
/// Coasting stops below this zoom rate, in natural-log distance per
/// second (0.1 ~ 10% of the camera distance per second).
const ZOOM_STOP_RATE: f32 = 0.1;

/// Translates window mouse events into camera motion: left-drag pan with
/// flick inertia, right-drag tilt, and wheel zoom.
#[derive(Default)]
pub struct Controller {
    cursor: Option<(f32, f32)>,
    drag: Option<Drag>,
    inertia: Option<Inertia>,
    zoom: Option<Zoom>,
    last_wheel: Option<Instant>,
    /// Smoothed gap between recent wheel events, in seconds.
    wheel_gap: f32,
}

struct Drag {
    button: MouseButton,
    last: (f32, f32),
    /// Smoothed cursor velocity, in px/s.
    velocity: (f32, f32),
    moved_at: Instant,
}

struct Inertia {
    /// Remaining pan velocity, in px/s.
    velocity: (f32, f32),
    tick: Instant,
}

struct Zoom {
    /// Camera distance the glide is heading toward, in globe radii.
    target: f32,
    /// The glide's current half-life, in seconds; follows the wheel
    /// cadence at the time of the last event.
    half_life: f32,
    /// Smoothed rate at which wheel events move the target, in
    /// natural-log distance per second. Keeps the target advancing
    /// between and after events, so the motion never pauses while the
    /// device (or its momentum tail) is between deliveries.
    velocity: f32,
    /// Log-distance the target has been advanced by `velocity` since the
    /// last wheel event. The next event repays it, so bridged motion is
    /// never counted twice.
    bridged: f32,
    tick: Instant,
}

impl Controller {
    /// Applies a window event to the camera. Returns whether the camera
    /// changed (or an animation started) and a redraw is needed.
    pub fn handle_event(
        &mut self,
        event: &WindowEvent,
        camera: &mut Camera,
        viewport_height: f32,
    ) -> bool {
        match event {
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: button @ (MouseButton::Left | MouseButton::Right),
                ..
            } => {
                let Some(position) = self.cursor else {
                    return false;
                };
                // Grabbing the globe stops any coasting.
                self.inertia = None;
                self.drag = Some(Drag {
                    button: *button,
                    last: position,
                    velocity: (0.0, 0.0),
                    moved_at: Instant::now(),
                });

                false
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button,
                ..
            } => {
                let releases_drag = self
                    .drag
                    .as_ref()
                    .is_some_and(|drag| drag.button == *button);

                if !releases_drag {
                    return false;
                }

                let Some(drag) = self.drag.take() else {
                    return false;
                };

                if drag.button == MouseButton::Left {
                    let (vx, vy) = drag.velocity;
                    let speed = vx.hypot(vy);
                    let held = Instant::now()
                        .saturating_duration_since(drag.moved_at)
                        .as_secs_f32();

                    if speed > FLICK_SPEED && held < FLICK_TIMEOUT {
                        self.inertia = Some(Inertia {
                            velocity: drag.velocity,
                            tick: Instant::now(),
                        });

                        return true;
                    }
                }

                false
            }
            WindowEvent::CursorMoved { position, .. } => {
                let position = (position.x as f32, position.y as f32);
                self.cursor = Some(position);

                let Some(drag) = self.drag.as_mut() else {
                    return false;
                };

                let dx = position.0 - drag.last.0;
                let dy = position.1 - drag.last.1;
                drag.last = position;

                let now = Instant::now();
                let dt = now
                    .saturating_duration_since(drag.moved_at)
                    .as_secs_f32()
                    .max(1e-4);
                drag.moved_at = now;

                // Exponential moving average of cursor velocity; the blend
                // weight is time-based so the smoothing is frame-rate
                // independent.
                let alpha = 1.0 - (-dt * 20.0).exp();
                let (vx, vy) = drag.velocity;
                drag.velocity = (vx + (dx / dt - vx) * alpha, vy + (dy / dt - vy) * alpha);

                match drag.button {
                    // Drag moves the globe with the cursor: dragging right
                    // pulls the view west, dragging down pulls it north.
                    MouseButton::Left => {
                        let scale = camera.pan_degrees_per_pixel(viewport_height);
                        camera.pan(-dx * scale, dy * scale);
                    }
                    // Dragging up tilts toward the horizon.
                    MouseButton::Right => camera.tilt_by(-dy * 0.25),
                    _ => unreachable!("only left/right drags are tracked"),
                }

                true
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let ticks = match delta {
                    MouseScrollDelta::LineDelta(_, y) => *y,
                    MouseScrollDelta::PixelDelta(position) => position.y as f32 / 60.0,
                };

                let now = Instant::now();
                let gap = self.last_wheel.map_or(WHEEL_GAP_CAP, |last| {
                    now.saturating_duration_since(last)
                        .as_secs_f32()
                        .min(WHEEL_GAP_CAP)
                });
                self.last_wheel = Some(now);

                // Track the event cadence; the 0.5 blend adapts within a
                // few events when a scroll speeds up or trails off.
                self.wheel_gap += (gap - self.wheel_gap) * 0.5;

                let half_life = self.wheel_gap.clamp(ZOOM_HALF_LIFE_MIN, ZOOM_HALF_LIFE_MAX);

                // Events only move the target; `tick` glides the camera
                // toward it each frame, paced to the event cadence. An
                // in-flight glide keeps its clock: resetting it per event
                // would stall the glide between dense events.
                let delta = ticks * 0.9f32.ln();

                match self.zoom.as_mut() {
                    Some(zoom) => {
                        // Reversing direction kills the coast outright.
                        if delta * zoom.velocity < 0.0 {
                            zoom.velocity = 0.0;
                            zoom.bridged = 0.0;
                        }

                        // Repay target motion the coast already applied
                        // since the last event; only the remainder moves
                        // the target now. An over-bridged surplus carries
                        // forward against the next event.
                        let outstanding = if delta * zoom.bridged > 0.0 {
                            let remaining = delta - zoom.bridged;
                            if remaining * delta > 0.0 {
                                zoom.bridged = 0.0;
                                remaining
                            } else {
                                zoom.bridged -= delta;
                                0.0
                            }
                        } else {
                            delta
                        };

                        zoom.target = Camera::clamp_distance(zoom.target * outstanding.exp());
                        zoom.half_life = half_life;

                        // Velocity follows the event rate; time-based
                        // blend, like the drag velocity EMA.
                        let alpha = 1.0 - (-gap * 20.0).exp();
                        let rate = delta / gap.max(1e-3);
                        zoom.velocity += (rate - zoom.velocity) * alpha;
                    }
                    None => {
                        // A first event carries no rate information, so
                        // the glide starts without coast velocity.
                        self.zoom = Some(Zoom {
                            target: Camera::clamp_distance(camera.distance * delta.exp()),
                            half_life,
                            velocity: 0.0,
                            bridged: 0.0,
                            tick: now,
                        });
                    }
                }

                true
            }
            _ => false,
        }
    }

    /// Advances one frame of camera animation: flick coasting and the
    /// zoom glide. Call from the redraw handler; returns true while
    /// another frame is needed.
    pub fn tick(&mut self, camera: &mut Camera, viewport_height: f32) -> bool {
        let coasting = self.tick_coast(camera, viewport_height);
        let zooming = self.tick_zoom(camera);

        coasting || zooming
    }

    /// Integrates one frame of flick coasting.
    fn tick_coast(&mut self, camera: &mut Camera, viewport_height: f32) -> bool {
        let Some(inertia) = self.inertia.as_mut() else {
            return false;
        };

        let now = Instant::now();
        let dt = now
            .saturating_duration_since(inertia.tick)
            .as_secs_f32()
            .min(0.1);
        inertia.tick = now;

        let scale = camera.pan_degrees_per_pixel(viewport_height);
        let (vx, vy) = inertia.velocity;
        camera.pan(-vx * dt * scale, vy * dt * scale);

        let decay = 0.5f32.powf(dt / HALF_LIFE);
        inertia.velocity = (vx * decay, vy * decay);

        if vx.hypot(vy) * decay < STOP_SPEED {
            self.inertia = None;
        }

        true
    }

    /// Moves the camera one frame closer to the zoom target.
    fn tick_zoom(&mut self, camera: &mut Camera) -> bool {
        let Some(zoom) = self.zoom.as_mut() else {
            return false;
        };

        let now = Instant::now();
        let dt = now
            .saturating_duration_since(zoom.tick)
            .as_secs_f32()
            .min(0.1);
        zoom.tick = now;

        // Coast: keep the target moving at the rate the wheel events
        // established, decaying once they stop. This is what carries the
        // motion across the silence between a finger lift and the first
        // momentum-tail event - without it the glide drains its target
        // and visibly stalls there. The advance is logged in `bridged`
        // and repaid by the next event, so nothing is counted twice.
        zoom.velocity *= 0.5f32.powf(dt / ZOOM_COAST_HALF_LIFE);
        if zoom.velocity.abs() > ZOOM_STOP_RATE {
            let step = zoom.velocity * dt;
            zoom.target = Camera::clamp_distance(zoom.target * step.exp());
            zoom.bridged += step;
        } else {
            zoom.velocity = 0.0;
        }

        // Exponential approach in log space - zoom is multiplicative, so
        // this keeps the glide's perceived speed uniform at any altitude.
        let blend = 1.0 - 0.5f32.powf(dt / zoom.half_life);
        let ratio = zoom.target / camera.distance;
        camera.distance *= ratio.powf(blend);

        if (ratio - 1.0).abs() < 1e-3 && zoom.velocity == 0.0 {
            camera.distance = zoom.target;
            self.zoom = None;
        }

        true
    }

    pub fn cursor_icon(&self) -> CursorIcon {
        if self.drag.is_some() {
            CursorIcon::Grabbing
        } else {
            CursorIcon::Grab
        }
    }
}
