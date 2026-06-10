mod globe;

use globe::{Globe, Interaction};
use globe::camera::Camera;
use globe::sun::Sun;
use iced::widget::{column, container, shader, slider, stack, text};
use iced::{Color, Element, Fill};

#[derive(Default)]
struct App {
    camera: Camera,
    sun: Sun,
}

#[derive(Debug, Clone)]
enum Message {
    Globe(Interaction),
    SunLatitude(f32),
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
        Message::SunLatitude(latitude) => state.sun.latitude = latitude,
    }
}

fn view(state: &App) -> Element<'_, Message> {
    let globe = Element::from(
        shader(Globe::new(state.camera, state.sun))
            .width(Fill)
            .height(Fill),
    )
    .map(Message::Globe);

    // Solar declination spans ±23.44° over the year.
    let sun_control = container(
        column![
            text(format!("Sun latitude: {:.1}°", state.sun.latitude))
                .color(Color::WHITE),
            slider(-23.44..=23.44, state.sun.latitude, Message::SunLatitude)
                .step(0.1),
        ]
        .spacing(5)
        .width(260),
    )
    .padding(10);

    stack![globe, sun_control].into()
}
