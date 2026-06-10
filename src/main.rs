mod globe;

use globe::{Globe, Interaction};
use globe::camera::Camera;
use iced::widget::shader;
use iced::{Element, Fill};

#[derive(Default)]
struct App {
    camera: Camera,
}

#[derive(Debug, Clone)]
enum Message {
    Globe(Interaction),
}

fn main() -> iced::Result {
    iced::application(App::default, update, view)
        .title("Globe")
        .run()
}

fn update(state: &mut App, message: Message) {
    match message {
        Message::Globe(interaction) => match interaction {
            Interaction::Pan { dlon, dlat } => state.camera.pan(dlon, dlat),
            Interaction::Zoom { factor } => state.camera.zoom(factor),
            Interaction::Tilt { degrees } => state.camera.tilt_by(degrees),
        },
    }
}

fn view(state: &App) -> Element<'_, Message> {
    Element::from(shader(Globe::new(state.camera)).width(Fill).height(Fill))
        .map(Message::Globe)
}
