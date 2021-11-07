use iced::{window, Application, Settings};
use timeline::Timeline;

mod event_stream;
mod timeline;

pub fn main() -> iced::Result {
    Timeline::run(Settings {
        antialiasing: true,
        window: window::Settings {
            position: window::Position::Centered,
            ..window::Settings::default()
        },
        ..Settings::default()
    })
}
