use iced::Element;
use iced::widget::{center, text};

#[derive(Default)]
struct HelloApp;

#[derive(Debug, Clone)]
enum Message {}

fn main() -> iced::Result {
    iced::application(HelloApp::default, update, view)
        .title("Hello, iced!")
        .run()
}

fn update(_state: &mut HelloApp, _message: Message) {}

fn view(_state: &HelloApp) -> Element<'_, Message> {
    center(text("Hello, world!").size(50)).into()
}
