mod globe;

use globe::Globe;
use globe::camera::Camera;
use iced::widget::shader;
use iced::{Element, Fill};

#[derive(Default)]
struct App {
    camera: Camera,
}

#[derive(Debug, Clone)]
enum Message {}

fn main() -> iced::Result {
    iced::application(App::default, update, view)
        .title("Globe")
        .run()
}

fn update(_state: &mut App, _message: Message) {}

fn view(state: &App) -> Element<'_, Message> {
    shader(Globe::new(state.camera)).width(Fill).height(Fill).into()
}
