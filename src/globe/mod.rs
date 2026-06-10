pub mod camera;
mod mesh;
mod pipeline;
pub mod sun;

use iced::time::Instant;
use iced::widget::shader::{self, Action};
use iced::{Event, Point, Rectangle, mouse, window};

use camera::Camera;
use sun::Sun;

/// Minimum release speed, in px/s, for a drag to keep coasting.
const FLICK_SPEED: f32 = 50.0;
/// Coasting stops once it decays below this speed, in px/s.
const STOP_SPEED: f32 = 15.0;
/// Coasting velocity halves every this many seconds.
const HALF_LIFE: f32 = 0.3;
/// A release later than this after the last cursor move is a hold, not a
/// flick, in seconds.
const FLICK_TIMEOUT: f32 = 0.1;

/// A camera change produced by user input on the globe widget.
#[derive(Debug, Clone, Copy)]
pub enum Interaction {
    /// Move the look-at point by the given degrees.
    Pan { dlon: f32, dlat: f32 },
    /// Scale the camera distance by the given factor.
    Zoom { factor: f32 },
    /// Adjust the tilt by the given degrees.
    Tilt { degrees: f32 },
}

pub struct Globe {
    camera: Camera,
    sun: Sun,
}

impl Globe {
    pub fn new(camera: Camera, sun: Sun) -> Self {
        Self { camera, sun }
    }
}

#[derive(Default)]
pub struct State {
    drag: Option<Drag>,
    inertia: Option<Inertia>,
}

struct Drag {
    button: mouse::Button,
    last: Point,
    /// Smoothed cursor velocity, in px/s.
    velocity: (f32, f32),
    moved_at: Instant,
}

struct Inertia {
    /// Remaining pan velocity, in px/s.
    velocity: (f32, f32),
    tick: Instant,
}

impl shader::Program<Interaction> for Globe {
    type State = State;
    type Primitive = pipeline::Primitive;

    fn update(
        &self,
        state: &mut Self::State,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<Action<Interaction>> {
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(
                button @ (mouse::Button::Left | mouse::Button::Right),
            )) => {
                let position = cursor.position_over(bounds)?;
                // Grabbing the globe stops any coasting.
                state.inertia = None;
                state.drag = Some(Drag {
                    button: *button,
                    last: position,
                    velocity: (0.0, 0.0),
                    moved_at: Instant::now(),
                });

                Some(Action::capture())
            }
            Event::Mouse(mouse::Event::ButtonReleased(button)) => {
                let releases_drag = state
                    .drag
                    .as_ref()
                    .is_some_and(|drag| drag.button == *button);

                if !releases_drag {
                    return None;
                }

                let drag = state.drag.take()?;

                if drag.button == mouse::Button::Left {
                    let (vx, vy) = drag.velocity;
                    let speed = vx.hypot(vy);
                    let held = Instant::now()
                        .saturating_duration_since(drag.moved_at)
                        .as_secs_f32();

                    if speed > FLICK_SPEED && held < FLICK_TIMEOUT {
                        state.inertia = Some(Inertia {
                            velocity: drag.velocity,
                            tick: Instant::now(),
                        });

                        return Some(
                            Action::request_redraw().and_capture(),
                        );
                    }
                }

                Some(Action::capture())
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let drag = state.drag.as_mut()?;
                let position = cursor.position()?;

                let dx = position.x - drag.last.x;
                let dy = position.y - drag.last.y;
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
                drag.velocity = (
                    vx + (dx / dt - vx) * alpha,
                    vy + (dy / dt - vy) * alpha,
                );

                let interaction = match drag.button {
                    // Drag moves the globe with the cursor: dragging right
                    // pulls the view west, dragging down pulls it north.
                    mouse::Button::Left => {
                        let scale =
                            self.camera.pan_degrees_per_pixel(bounds.height);

                        Interaction::Pan {
                            dlon: -dx * scale,
                            dlat: dy * scale,
                        }
                    }
                    // Dragging up tilts toward the horizon.
                    mouse::Button::Right => Interaction::Tilt {
                        degrees: -dy * 0.25,
                    },
                    _ => unreachable!("only left/right drags are tracked"),
                };

                Some(Action::publish(interaction).and_capture())
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                cursor.position_over(bounds)?;

                let ticks = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => *y,
                    mouse::ScrollDelta::Pixels { y, .. } => *y / 60.0,
                };

                Some(
                    Action::publish(Interaction::Zoom {
                        factor: 0.9f32.powf(ticks),
                    })
                    .and_capture(),
                )
            }
            Event::Window(window::Event::RedrawRequested(now)) => {
                let inertia = state.inertia.as_mut()?;

                let dt = now
                    .saturating_duration_since(inertia.tick)
                    .as_secs_f32()
                    .min(0.1);
                inertia.tick = *now;

                let scale = self.camera.pan_degrees_per_pixel(bounds.height);
                let (vx, vy) = inertia.velocity;
                let pan = Interaction::Pan {
                    dlon: -vx * dt * scale,
                    dlat: vy * dt * scale,
                };

                let decay = 0.5f32.powf(dt / HALF_LIFE);
                inertia.velocity = (vx * decay, vy * decay);

                if vx.hypot(vy) * decay < STOP_SPEED {
                    state.inertia = None;
                }

                // Publishing implies a redraw, which keeps the
                // coasting loop ticking until the velocity decays.
                Some(Action::publish(pan))
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &Self::State,
        _cursor: mouse::Cursor,
        _bounds: Rectangle,
    ) -> Self::Primitive {
        pipeline::Primitive {
            camera: self.camera,
            sun: self.sun,
        }
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if state.drag.is_some() {
            mouse::Interaction::Grabbing
        } else {
            mouse::Interaction::Grab
        }
    }
}
