mod pipeline;

use iced::Rectangle;
use iced::mouse;
use iced::widget::shader;

pub struct Globe;

impl<Message> shader::Program<Message> for Globe {
    type State = ();
    type Primitive = pipeline::Primitive;

    fn draw(
        &self,
        _state: &Self::State,
        _cursor: mouse::Cursor,
        _bounds: Rectangle,
    ) -> Self::Primitive {
        pipeline::Primitive
    }
}
