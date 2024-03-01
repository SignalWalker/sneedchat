#[cfg(feature = "iced")]
mod __iced {
    use crate::{
        chat::{ChannelId, ChatManager, Event, SessionManager, UserId},
        TcpInbox, TcpIpNetlayer, TcpPipe,
    };
    use ed25519_dalek::VerifyingKey;
    use iced::{alignment, Command, Length, Padding, Subscription};
    use rexa::{
        captp::{
            msg::{OpAbort, OpDeliver, OpDeliverOnly, Operation},
            CapTpSession,
        },
        locator::NodeLocator,
        netlayer::Netlayer,
    };
    use std::{collections::HashMap, sync::Arc};
    use tokio::net::{TcpListener, TcpStream};

    pub struct MessageTask<'future> {
        task: Arc<std::pin::Pin<Box<dyn futures::Future<Output = ()> + Send + Sync + 'future>>>,
    }

    impl<'f> std::clone::Clone for MessageTask<'f> {
        fn clone(&self) -> Self {
            Self {
                task: self.task.clone(),
            }
        }
    }

    impl<'f> std::fmt::Debug for MessageTask<'f> {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("MessageTask").finish_non_exhaustive()
        }
    }

    impl<'f, F: futures::Future<Output = ()> + Send + Sync + 'f> From<F> for MessageTask<'f> {
        fn from(value: F) -> Self {
            Self {
                task: Arc::new(Box::pin(value)),
            }
        }
    }

    #[derive(Debug, Clone)]
    pub enum Message {
        Loaded(Result<(), ()>),
        UsernameChanged(String),
        LogIn,
        ShiftFocus(bool),
        ManagerReady(Result<Arc<ChatManager>, String>),
        ChatEvent(Event),
        // Fetch {
        //     swiss: Vec<u8>,
        //     resolver: rexa::captp::FetchResolver<TcpStream>,
        // },
        SentMessage,
    }

    // impl From<MessageTask<'static>> for Message {
    //     fn from(value: MessageTask<'static>) -> Self {
    //         Self::Task(value)
    //     }
    // }

    pub enum SneedChat {
        Loading,
        Login {
            username: String,
        },
        LoggingIn {
            username: String,
        },
        LoggedIn {
            manager: Arc<ChatManager>,
            netlayer: Arc<TcpIpNetlayer<TcpListener, TcpStream>>,
            sessions: Arc<SessionManager>,
            // registry: Arc<SwissRegistry<TcpStream>>,
            // exports: Arc<ExportManager<TcpPipe>>,
            current_channel: Option<ChannelId>,
        },
    }

    impl SneedChat {
        fn new(_flags: <Self as iced::Application>::Flags) -> Self {
            Self::Loading
        }
    }

    impl iced::Application for SneedChat {
        type Executor = iced::executor::Default;

        type Message = Message;

        type Theme = iced::theme::Theme;

        type Flags = ();

        fn new(flags: Self::Flags) -> (Self, iced::Command<Self::Message>) {
            (
                SneedChat::new(flags),
                Command::perform(async { Ok(()) }, Message::Loaded),
            )
        }

        fn title(&self) -> String {
            "SneedChat".into()
        }

        fn update(&mut self, message: Self::Message) -> iced::Command<Self::Message> {
            fn shift_focus<Message: 'static>(backward: bool) -> Command<Message> {
                match backward {
                    true => iced::widget::focus_previous(),
                    false => iced::widget::focus_next(),
                }
            }
            match self {
                SneedChat::Loading => match message {
                    Message::Loaded(Ok(_)) => {
                        *self = Self::Login {
                            username: "".into(),
                        };
                        Command::none()
                    }
                    Message::Loaded(Err(_)) => todo!(),
                    _ => Command::none(),
                },
                SneedChat::Login { username } => match message {
                    Message::UsernameChanged(u) => {
                        *username = u;
                        Command::none()
                    }
                    Message::ShiftFocus(backward) => shift_focus(backward),
                    Message::LogIn => {
                        let username = username.clone();
                        *self = Self::LoggingIn {
                            username: username.clone(),
                        };
                        Command::perform(
                            async move {
                                Ok(ChatManager::new(
                                    Arc::new(
                                        TcpIpNetlayer::bind("0.0.0.0:0")
                                            .await
                                            .map_err(|e| e.to_string())?,
                                    ),
                                    username,
                                ))
                            },
                            Message::ManagerReady,
                        )
                    }
                    _ => Command::none(),
                },
                SneedChat::LoggingIn { .. } => match message {
                    Message::ManagerReady(Ok((m, nl))) => {
                        *self = Self::LoggedIn {
                            manager: m,
                            netlayer: nl.clone(),
                            current_channel: None,
                            sessions: SessionManager::new(),
                            // registry: Arc::default(),
                            // exports,
                        };
                        Command::none()
                    }
                    Message::ManagerReady(Err(e)) => todo!("handle netlayer binding error: {e}"),
                    _ => Command::none(),
                },
                SneedChat::LoggedIn {
                    manager,
                    netlayer,
                    sessions,
                    // registry,
                    // exports,
                    current_channel,
                } => match message {
                    Message::ShiftFocus(backward) => shift_focus(backward),
                    Message::SentMessage => Command::none(),
                    _ => todo!(),
                },
                _ => todo!(),
            }
        }

        fn view(&self) -> iced::Element<'_, Self::Message, Self::Theme, iced::Renderer> {
            use iced::widget as w;
            match self {
                SneedChat::Loading => w::container(
                    w::text("Loading...")
                        .horizontal_alignment(alignment::Horizontal::Center)
                        .size(50),
                )
                .width(Length::Fill)
                .height(Length::Fill)
                .center_y()
                .into(),
                SneedChat::Login { username } => w::container(
                    w::column![
                        w::text("Log In").size(50),
                        w::row![
                            w::text("Username"),
                            w::text_input("<Name>", username)
                                .on_input(Message::UsernameChanged)
                                .on_submit(Message::LogIn)
                        ],
                        w::button("Log In").on_press(Message::LogIn)
                    ]
                    .align_items(alignment::Alignment::Center),
                )
                .into(),
                SneedChat::LoggingIn { username } => w::container(
                    w::text(format!("Logging in as {username}..."))
                        .horizontal_alignment(alignment::Horizontal::Center)
                        .size(50),
                )
                .into(),
                SneedChat::LoggedIn {
                    manager,
                    netlayer,
                    sessions,
                    current_channel,
                    // registry,
                    // exports,
                } => w::container(w::column![
                    w::row![w::button("Connect")],
                    w::row![
                        w::column![w::text("Channels")],
                        w::column![w::text("current channel")]
                    ],
                    w::text(format!(
                        "Address: {}",
                        netlayer.locator::<String, String>().unwrap()
                    ))
                ])
                .into(),
                _ => todo!(),
            }
        }

        fn subscription(&self) -> iced::Subscription<Self::Message> {
            use iced::keyboard::{self, key};
            let keypress = keyboard::on_key_press(|key, modifiers| {
                let keyboard::Key::Named(key) = key else {
                    return None;
                };
                match (key, modifiers) {
                    (key::Named::Tab, _) => Some(Message::ShiftFocus(modifiers.shift())),
                    _ => None,
                }
            });
            match self {
                SneedChat::Loading | SneedChat::Login { .. } | SneedChat::LoggingIn { .. } => {
                    keypress
                }
                SneedChat::LoggedIn {
                    netlayer,
                    sessions,
                    // registry,
                    // exports,
                    ..
                } => Subscription::batch([todo!(), keypress]),
            }
        }
    }
}
#[cfg(feature = "iced")]
pub use __iced::*;

#[cfg(feature = "egui")]
mod __egui {
    #[derive(Debug, Clone, Copy, Default)]
    pub struct SneedChat {}

    impl eframe::App for SneedChat {
        fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.heading("SneedChat");
            });
        }
    }
}
#[cfg(feature = "egui")]
pub use __egui::*;
