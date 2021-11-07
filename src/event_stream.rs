use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};

use iced::futures::{self, StreamExt};
use rtic_scope_api::EventChunk;
use tokio::{
    io,
    net::{unix::SocketAddr, UnixListener, UnixStream},
};
use tokio_util::io::ReaderStream;

pub struct EventStream {}

// Make sure iced can use our download stream
impl<H, I> iced_native::subscription::Recipe<H, I> for EventStream
where
    H: Hasher,
{
    type Output = Progress;

    fn hash(&self, state: &mut H) {
        struct Marker;
        std::any::TypeId::of::<Marker>().hash(state);
    }

    fn stream(
        self: Box<Self>,
        _input: futures::stream::BoxStream<'static, I>,
    ) -> futures::stream::BoxStream<'static, Self::Output> {
        Box::pin(futures::stream::unfold(
            State::Initializing,
            move |state| async move {
                match state {
                    State::Initializing => {
                        // Create frontend socket in a temporary directory, print it for the parent backend.
                        let socket_dir = match tempfile::TempDir::new() {
                            Ok(v) => v,
                            Err(e) => {
                                return Some((
                                    Progress::Error(Error::TempDir(Arc::new(e))),
                                    State::Done,
                                ))
                            }
                        };
                        let socket_path = socket_dir.path().join("rtic-scope-frontend2.socket");
                        let listener = match UnixListener::bind(&socket_path) {
                            Ok(v) => v,
                            Err(e) => {
                                return Some((Progress::Error(Error::Io(Arc::new(e))), State::Done))
                            }
                        };
                        println!("{}", socket_path.display());
                        Some((Progress::Initialized, State::Listening(listener)))
                    }
                    State::Listening(listener) => {
                        // Deserialize api::EventChunks from socket and print events to
                        // stderr along with nanoseconds timestamp.
                        let (stream, address) = match listener.accept().await {
                            Ok(v) => v,
                            Err(e) => {
                                return Some((Progress::Error(Error::Io(Arc::new(e))), State::Done))
                            }
                        };
                        let stream = ReaderStream::new(stream);
                        Some((
                            Progress::Connected(Arc::new(address)),
                            State::Running {
                                stream,
                                buffer: String::new(),
                            },
                        ))
                    }
                    State::Running {
                        mut stream,
                        mut buffer,
                    } => {
                        // Try to read data, this may still fail with `WouldBlock`
                        // if the readiness event is a false positive.
                        if let Some(chunk) = stream.next().await {
                            match chunk {
                                Ok(v) => {
                                    buffer += &String::from_utf8_lossy(&v);
                                    if let Some(location) = buffer.find('\n') {
                                        let packet =
                                            buffer.drain(0..location + 1).collect::<String>();
                                        let chunk: EventChunk =
                                            match serde_json::from_str(&packet[..packet.len() - 1])
                                            {
                                                Ok(v) => v,
                                                Err(e) => {
                                                    return Some((
                                                        Progress::Error(Error::Serialize((
                                                            e.to_string(),
                                                            packet[..packet.len() - 1].to_string(),
                                                        ))),
                                                        State::Done,
                                                    ))
                                                }
                                            };

                                        Some((
                                            Progress::Event(chunk),
                                            State::Running { stream, buffer },
                                        ))
                                    } else {
                                        Some((Progress::None, State::Running { stream, buffer }))
                                    }
                                }
                                Err(e) => {
                                    return Some((
                                        Progress::Error(Error::Io(Arc::new(e))),
                                        State::Done,
                                    ))
                                }
                            }
                        } else {
                            None
                        }
                    }
                    State::Done => None,
                }
            },
        ))
    }
}

enum State {
    Initializing,
    Listening(UnixListener),
    Running {
        stream: ReaderStream<UnixStream>,
        buffer: String,
    },
    Done,
}

#[derive(Debug, Clone)]
pub enum Progress {
    Initialized,
    Connected(Arc<SocketAddr>),
    Event(EventChunk),
    Error(Error),
    None,
}

#[derive(Debug, Clone)]
pub enum Error {
    TempDir(Arc<std::io::Error>),
    Io(Arc<io::Error>),
    Serialize((String, String)),
}
