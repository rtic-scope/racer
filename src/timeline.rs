use grid::Grid;
use iced::{
    button::{self, Button},
    executor, Alignment, Application, Checkbox, Color, Column, Command, Container, Element, Length,
    Point, Row, Subscription, Text,
};

use crate::event_stream::Progress;

#[derive(Default)]
pub struct Timeline {
    grid: Grid,
    controls: Controls,
}

#[derive(Debug, Clone)]
pub enum Message {
    ToggleGrid(bool),
    Progress(Progress),
    Reset,
    None,
}

impl Application for Timeline {
    type Message = Message;
    type Executor = executor::Default;
    type Flags = ();

    fn new(_flags: ()) -> (Self, Command<Message>) {
        (Self { ..Self::default() }, Command::none())
    }

    fn title(&self) -> String {
        String::from("probe-rs tracer")
    }

    fn update(&mut self, message: Message) -> Command<Message> {
        match message {
            Message::ToggleGrid(show_grid_lines) => self.grid.toggle_grid(show_grid_lines),
            Message::Reset => self.grid.reset_state(),
            Message::None => todo!(),
            Message::Progress(progress) => match progress {
                Progress::Initialized => {
                    self.grid.set_status("Initialized. Waiting for connection.")
                }
                Progress::Connected(address) => {
                    self.grid.set_status(format!("Connected to {:?}.", address))
                }
                Progress::Event(events) => {
                    for event in events.events {
                        self.grid
                            .add_event(events.timestamp.offset.as_nanos() as usize, event);
                    }
                }
                Progress::Error(error) => self.grid.set_status(format!("Error {:?}", error)),
                Progress::None => {}
            },
        }
        Command::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::from_recipe(crate::event_stream::EventStream {}).map(Message::Progress)
    }

    fn view(&mut self) -> Element<Message> {
        let controls = self
            .controls
            .view(true, self.grid.are_lines_visible(), self.grid.status());

        let content = Column::new()
            .push(self.grid.view().map(move |_message| Message::None))
            .push(controls);

        Container::new(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}

mod grid {
    use crate::timeline::to_si_time;

    use super::{Bar, EventStyle, Interaction, Paint};
    use bio::data_structures::interval_tree::IntervalTree;
    use iced::{
        alignment,
        canvas::{self, Cache, Canvas, Cursor, Frame, Geometry, Path, Text},
        canvas::{
            event::{self, Event},
            Stroke,
        },
        mouse, Color, Element, Font, Length, Point, Rectangle, Size,
    };
    use itertools::Itertools;
    use rtic_scope_api::EventType;
    use std::collections::HashMap;

    pub struct Grid {
        interaction: Interaction,
        bar_cache: Cache,
        grid_cache: Cache,
        is_grid_enabled: bool,
        zoom: f32,
        pan: f32,
        bars: IntervalTree<usize, Bar>,
        started_bars: Vec<Bar>,
        channel_map: Vec<String>,
        status: String,
        min: usize,
        max: usize,
        width: usize,
    }

    #[derive(Debug, Clone)]
    pub enum Message {}

    impl Default for Grid {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Grid {
        const INITIAL_ZOOM: f32 = 0.0;
        const INITIAL_PAN: f32 = 0.5;

        pub fn new() -> Self {
            let mut s = Self {
                interaction: Interaction::None,
                bar_cache: Cache::default(),
                grid_cache: Cache::default(),
                is_grid_enabled: true,
                zoom: Self::INITIAL_ZOOM,
                pan: Self::INITIAL_PAN,
                bars: IntervalTree::new(),
                started_bars: vec![],
                channel_map: vec![],
                status: String::new(),
                min: 0,
                max: 0,
                width: 0,
            };
            s.set_zoom(1280.0 / 100.0);
            s.set_bars();
            s
        }

        pub fn view<'a>(&'a mut self) -> Element<'a, Message> {
            Canvas::new(self)
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        }

        fn set_bars(&mut self) {
            self.bars = IntervalTree::new();

            // let mut rng = rand::thread_rng();
            // for j in (0..1000).into_iter().step_by(100) {
            //     for i in 0..10 {
            //         let start = rng.gen_range(0..100) + j;
            //         let end = rng.gen_range(100..200) + j;
            //         self.bars.insert(
            //             start as usize..end as usize,
            //             Bar {
            //                 start_ns: start * 1 as usize,
            //                 end_ns: Some(end * 1 as usize),
            //                 isr: format!("{}", i),
            //                 channel: i,
            //             },
            //         );
            //     }
            // }
        }

        pub fn add_event(&mut self, timestamp: usize, event: EventType) {
            let timestamp = timestamp - 0;
            self.max = self.max.max(timestamp);
            self.min = self.min.min(timestamp);
            match event {
                EventType::Overflow => (),
                EventType::Task { name, action } => {
                    match action {
                        rtic_scope_api::TaskAction::Entered => {
                            let channel = if let Some((index, _name)) =
                                self.channel_map.iter().find_position(|c| *c == &name)
                            {
                                index
                            } else {
                                self.channel_map.push(name.clone());
                                self.channel_map.len() - 1
                            };
                            self.started_bars.push(Bar {
                                start_ns: timestamp,
                                end_ns: None,
                                isr: name,
                                channel,
                            });
                        }
                        rtic_scope_api::TaskAction::Exited => {
                            let mut found = None;
                            for (i, bar) in self.started_bars.iter().enumerate() {
                                if bar.start_ns < timestamp && bar.isr == name {
                                    found = Some(i);
                                    break;
                                }
                            }
                            if let Some(found) = found {
                                let mut bar = self.started_bars.remove(found);
                                bar.end_ns = Some(timestamp);
                                self.bars.insert(bar.start_ns..timestamp, bar);
                            }
                        }
                        rtic_scope_api::TaskAction::Returned => (),
                    };

                    let screen_start = 0f32;
                    let screen_end = self.width as f32;
                    let start = self.min as f32;
                    let end = self.max as f32;

                    // start = screen_start / zoom - pan
                    // end = screen_end / zoom - pan
                    // start - end = screen_start / zoom - screen_end / zoom

                    let zoom = (screen_start - screen_end) / (start - end);
                    let pan = screen_start / zoom - start;

                    self.set_zoom(zoom);
                    self.set_pan(pan);
                    self.bar_cache.clear();
                    self.grid_cache.clear();
                }
                EventType::Unknown(_) => (),
                EventType::Unmappable(_, _) => (),
                EventType::Invalid(_) => (),
            }
        }

        fn update_zoom(&mut self, delta: f32) {
            self.zoom *= 1.0 + (delta / 1e2);
            self.zoom = self.zoom.max(1e-8);
        }

        fn set_zoom(&mut self, zoom: f32) {
            // px / ns
            self.zoom = zoom;
            self.zoom = self.zoom.max(1e-8);
        }

        fn update_pan(&mut self, delta: f32) {
            self.pan += delta / self.zoom; // px / (px / ns) = ns
            self.pan = self.pan.min(0.5);
        }

        fn set_pan(&mut self, pan: f32) {
            self.pan = pan;
            self.pan = self.pan.min(0.5);
        }

        pub(crate) fn reset_state(&mut self) {
            self.set_bars();
            self.zoom = Self::INITIAL_ZOOM;
            self.pan = Self::INITIAL_PAN;
            self.grid_cache.clear();
            self.bar_cache.clear();
        }

        pub(crate) fn toggle_grid(&mut self, enabled: bool) {
            self.is_grid_enabled = enabled;
        }

        pub(crate) fn are_lines_visible(&self) -> bool {
            self.is_grid_enabled
        }

        pub(crate) fn set_status(&mut self, status: impl AsRef<str>) {
            self.status = status.as_ref().to_owned();
        }

        pub(crate) fn status(&self) -> &str {
            &self.status
        }
    }

    impl<'a> canvas::Program<Message> for Grid {
        fn update(
            &mut self,
            event: Event,
            bounds: Rectangle,
            cursor: Cursor,
        ) -> (event::Status, Option<Message>) {
            self.width = bounds.size().width as usize;

            if let Event::Mouse(mouse::Event::ButtonReleased(_)) = event {
                self.interaction = Interaction::None;
            }

            let cursor_position = if let Some(position) = cursor.position() {
                position
            } else {
                return (event::Status::Ignored, None);
            };

            match event {
                Event::Mouse(mouse_event) => match mouse_event {
                    mouse::Event::ButtonPressed(button) => {
                        let message = match button {
                            mouse::Button::Left => None,
                            mouse::Button::Right => {
                                self.interaction = Interaction::Panning {
                                    start: cursor_position,
                                };

                                None
                            }
                            _ => None,
                        };

                        (event::Status::Captured, message)
                    }
                    mouse::Event::CursorMoved { .. } => {
                        let message = match self.interaction {
                            Interaction::Panning { start } => {
                                self.update_pan((cursor_position - start).x);

                                self.bar_cache.clear();
                                self.grid_cache.clear();

                                self.interaction = Interaction::Panning {
                                    start: cursor_position,
                                };

                                None
                            }
                            _ => None,
                        };

                        let event_status = match self.interaction {
                            Interaction::None => event::Status::Ignored,
                            _ => event::Status::Captured,
                        };

                        (event_status, message)
                    }
                    mouse::Event::WheelScrolled { delta } => match delta {
                        mouse::ScrollDelta::Lines { y, .. }
                        | mouse::ScrollDelta::Pixels { y, .. } => {
                            self.update_zoom(y);
                            self.bar_cache.clear();
                            self.grid_cache.clear();
                            (event::Status::Captured, None)
                        }
                    },
                    _ => (event::Status::Ignored, None),
                },
                _ => (event::Status::Ignored, None),
            }
        }

        fn draw(&self, bounds: Rectangle, cursor: Cursor) -> Vec<Geometry> {
            let size = bounds.size();
            let cursor_x = cursor.position().map(|c| c.x).unwrap_or(0.0);
            let cursor_y = cursor.position().map(|c| c.y).unwrap_or(0.0);
            let logical_start = (0.0 - self.pan * self.zoom) / self.zoom;
            let logical_end = (bounds.size().width - self.pan * self.zoom) / self.zoom;
            let logical_cursor_x = ((cursor_x - self.pan * self.zoom) / self.zoom) as usize;

            let bar_height = 20.0;
            let bar_padding = 8.0;
            let offset_top = 20.0;

            let overlay = {
                let mut frame = Frame::new(size);

                for bar in self.bars.find(logical_cursor_x..logical_cursor_x + 1) {
                    let y = bar.data().channel as f32 * (bar_height + bar_padding) + offset_top; // 1 * px + px

                    if y < cursor_y && cursor_y <= y + bar_height {
                        let start = (bar.interval().start as f32 * self.zoom
                            + self.pan * self.zoom)
                            .min(size.width); // ns * px / ns + ns = px
                        let length = (bar.interval().end - bar.interval().start) as f32 * self.zoom; // ns * px / ns = px
                        let y = bar.data().channel as f32 * (bar_height + bar_padding) + offset_top; // 1 * px + px
                        frame.fill_rectangle(
                            Point::new(start, y),
                            Size::new(length, bar_height + bar_height),
                            Color::WHITE,
                        );
                        frame.stroke(
                            &Path::rectangle(
                                Point::new(start, y),
                                Size::new(length, bar_height * 2.0),
                            ),
                            Stroke::default().with_color(Color::BLACK).with_width(1.5),
                        );
                        frame.fill_text(Text {
                            content: format!(
                                "{} - {} : {}",
                                to_si_time(bar.interval().start),
                                to_si_time(bar.interval().end),
                                bar.data().isr
                            ),
                            position: Point::new(start + 2.0, y + bar_height + bar_height / 2.0),
                            color: Color::BLACK,
                            size: 15.0,
                            font: Font::Default,
                            horizontal_alignment: alignment::Horizontal::Left,
                            vertical_alignment: alignment::Vertical::Center,
                        });
                        break;
                    }
                }

                frame.into_geometry()
            };

            let bar = self.bar_cache.draw(size, |frame| {
                let mut isrs = HashMap::<usize, EventStyle>::new();
                let palette: &[Color] = &[
                    Color::from_rgb8(0, 18, 25),
                    Color::from_rgb8(0, 95, 115),
                    Color::from_rgb8(10, 147, 150),
                    Color::from_rgb8(148, 210, 189),
                    Color::from_rgb8(233, 216, 166),
                    Color::from_rgb8(238, 155, 0),
                    Color::from_rgb8(202, 103, 2),
                    Color::from_rgb8(187, 62, 3),
                    Color::from_rgb8(174, 32, 18),
                    Color::from_rgb8(155, 34, 38),
                ];

                // let t = std::time::Instant::now();
                for bar in self
                    .bars
                    .find(logical_start.max(0.0) as usize..logical_end.min(f32::MAX) as usize)
                {
                    let pot_isr = isrs.get(&bar.data().channel).cloned();
                    let (channel, isr) = if let Some(isr) = pot_isr {
                        (bar.data().channel, isr)
                    } else {
                        let isr = EventStyle {
                            paint: Paint {
                                color: palette[isrs.len()],
                            },
                        };
                        isrs.insert(bar.data().channel, isr);
                        (bar.data().channel, isrs[&bar.data().channel])
                    };
                    let start = (bar.interval().start as f32 * self.zoom + self.pan * self.zoom)
                        .min(size.width); // ns * px / ns + ns = px
                    let length = (bar.interval().end - bar.interval().start) as f32 * self.zoom; // ns * px / ns = px
                    let y = channel as f32 * (bar_height + bar_padding) + offset_top; // 1 * px + px
                    frame.fill_rectangle(
                        Point::new(start, y),
                        Size::new(length, bar_height),
                        isr.paint.color,
                    );
                    frame.fill_text(Text {
                        content: format!("{}", bar.data().isr),
                        position: Point::new(start + 2.0, y + bar_height / 2.0),
                        color: Color::BLACK,
                        size: 15.0,
                        font: Font::Default,
                        horizontal_alignment: alignment::Horizontal::Left,
                        vertical_alignment: alignment::Vertical::Center,
                    });
                }
                // println!("{:?}", t.elapsed());
            });

            if self.is_grid_enabled {
                let grid = self.grid_cache.draw(bounds.size(), |frame| {
                    let size = bounds.size();
                    // Find the correct spacing of all the bars.
                    let mut spacing = self.zoom * 1.0; // px / ns * ns = px
                    while size.width as f32 / spacing > 10.0 {
                        // px / px = 1
                        spacing *= 10.0; // px
                    }

                    let y = size.height as f32 - 30.0;

                    let mut x = self.pan * self.zoom;
                    while x < size.width {
                        // Draw the grid.
                        frame.stroke(
                            &Path::line(Point::new(x, 0.0), Point::new(x, y as f32)),
                            Stroke::default().with_color(Color::BLACK),
                        );

                        // Draw all the grid timescale annotations.

                        // Find the number to display.
                        let ns = (-self.pan + x / self.zoom).round() as usize; // --ns + px / (px / ns) = ns

                        frame.fill_text(Text {
                            content: to_si_time(ns),
                            position: Point::new(x, y),
                            color: Color::BLACK,
                            size: 18.0,
                            font: Font::Default,
                            horizontal_alignment: alignment::Horizontal::Center,
                            vertical_alignment: alignment::Vertical::Top,
                        });

                        x += spacing;
                    }
                });
                vec![grid, bar, overlay]
            } else {
                vec![bar, overlay]
            }
        }

        fn mouse_interaction(&self, bounds: Rectangle, cursor: Cursor) -> mouse::Interaction {
            match self.interaction {
                Interaction::Panning { .. } => mouse::Interaction::Grabbing,
                Interaction::None if cursor.is_over(&bounds) => mouse::Interaction::Crosshair,
                _ => mouse::Interaction::default(),
            }
        }
    }
}

fn to_si_time(nanoseconds: usize) -> String {
    const NAMES: &[&'static str] = &["ns", "us", "ms", "s"];

    let levels = if nanoseconds == 0 {
        0f32
    } else {
        ((nanoseconds as f32).ln() / 10f32.ln() + 1e-15).floor()
    };
    let mut display_ns = nanoseconds as f32;
    while display_ns >= 1000.0 {
        display_ns /= 1000.0;
    }

    format!("{}{}", display_ns.round(), NAMES[(levels / 3.0) as usize])
}

enum Interaction {
    None,
    Panning { start: Point },
}

#[derive(Default)]
struct Controls {
    toggle_button: button::State,
    reset_button: button::State,
}

impl Controls {
    fn view<'a>(
        &'a mut self,
        is_playing: bool,
        is_grid_enabled: bool,
        status: impl AsRef<str>,
    ) -> Element<'a, Message> {
        let playback_controls = Row::new().spacing(10).push(Button::new(
            &mut self.toggle_button,
            Text::new(if is_playing { "Pause" } else { "Play" }),
        ));

        let speed_controls = Row::new()
            .push(Text::new(status.as_ref()))
            .width(Length::Fill)
            .align_items(Alignment::Center)
            .spacing(10);

        Row::new()
            .padding(10)
            .spacing(20)
            .align_items(Alignment::Center)
            .push(playback_controls)
            .push(speed_controls)
            .push(
                Checkbox::new(is_grid_enabled, "Grid", Message::ToggleGrid)
                    .size(16)
                    .spacing(5)
                    .text_size(16),
            )
            .push(Button::new(&mut self.reset_button, Text::new("Reset")).on_press(Message::Reset))
            .into()
    }
}

#[derive(Debug, Clone, Copy)]
struct EventStyle {
    paint: Paint,
}

#[derive(Debug, Clone, Copy)]
struct Paint {
    color: Color,
}

#[derive(PartialEq, Eq, Clone, Debug)]
struct Bar {
    start_ns: usize,
    end_ns: Option<usize>,
    isr: String,
    channel: usize,
}

fn _px_to_ns(px: f32, zoom: f32) -> f32 {
    px / (1e3 * zoom)
}

fn _ns_to_px(ns: f32, zoom: f32) -> f32 {
    ns * (1e3 * zoom)
}

mod style {
    use iced::container;
    use iced::Color;

    pub struct Tooltip;

    impl container::StyleSheet for Tooltip {
        fn style(&self) -> container::Style {
            container::Style {
                text_color: Some(Color::from_rgb8(0xEE, 0xEE, 0xEE)),
                background: Some(Color::from_rgb(0.11, 0.42, 0.87).into()),
                border_radius: 12.0,
                ..container::Style::default()
            }
        }
    }
}
