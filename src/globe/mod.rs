pub mod camera;
mod mesh;
mod pipeline;

use iced::widget::shader::{self, Action};
use iced::{Event, Point, Rectangle, mouse};

use camera::Camera;

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
}

impl Globe {
    pub fn new(camera: Camera) -> Self {
        Self { camera }
    }
}

#[derive(Default)]
pub struct State {
    drag: Option<Drag>,
}

struct Drag {
    button: mouse::Button,
    last: Point,
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
                state.drag = Some(Drag {
                    button: *button,
                    last: position,
                });

                Some(Action::capture())
            }
            Event::Mouse(mouse::Event::ButtonReleased(button)) => {
                if state.drag.as_ref().is_some_and(|drag| {
                    drag.button == *button
                }) {
                    state.drag = None;

                    Some(Action::capture())
                } else {
                    None
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let drag = state.drag.as_mut()?;
                let position = cursor.position()?;

                let dx = position.x - drag.last.x;
                let dy = position.y - drag.last.y;
                drag.last = position;

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
