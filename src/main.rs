mod globe;

use iced::widget::shader;
use iced::{Element, Fill};

#[derive(Default)]
struct App;

#[derive(Debug, Clone)]
enum Message {}

fn main() -> iced::Result {
    iced::application(App::default, update, view)
        .title("Globe")
        .run()
}

fn update(_state: &mut App, _message: Message) {}

fn view(_state: &App) -> Element<'_, Message> {
    shader(globe::Globe).width(Fill).height(Fill).into()
}
