#[cfg(feature = "iced")]
mod __iced {
    use crate::chat::{ChatEvent, ChatManager, SneedEvent};
    use ed25519_dalek::VerifyingKey;
    use iced::{alignment, Command, Length, Padding, Subscription};
    use rexa::{
        captp::{
            msg::{OpAbort, OpDeliver, OpDeliverOnly},
            CapTpSession,
        },
        locator::NodeLocator,
        netlayer::Netlayer,
    };
    use std::{collections::HashMap, net::SocketAddr, sync::Arc};
    use tokio::{
        net::{TcpListener, TcpStream},
        sync::mpsc,
    };

    #[derive(Default, Clone, Debug)]
    pub struct SneedChatFlags {
        username: Option<String>,
        port: u16,
        remote: Option<SocketAddr>,
    }

    impl From<crate::cli::Cli> for SneedChatFlags {
        fn from(args: crate::cli::Cli) -> Self {
            Self {
                port: args.port,
                username: args.username,
                remote: args.remote,
            }
        }
    }

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

    #[derive(Debug)]
    pub enum Message {
        Loaded(Result<ChatManager, Box<dyn std::error::Error + Send + Sync + 'static>>),
        ShiftFocus(bool),
        ChatEvent(ChatEvent),
    }

    impl Clone for Message {
        fn clone(&self) -> Self {
            match self {
                Self::ShiftFocus(arg0) => Self::ShiftFocus(arg0.clone()),
                Self::ChatEvent(ev) => Self::ChatEvent(ev.clone()),
                _ => todo!(),
            }
        }
    }

    // impl From<MessageTask<'static>> for Message {
    //     fn from(value: MessageTask<'static>) -> Self {
    //         Self::Task(value)
    //     }
    // }

    pub enum SneedChat {
        Loading,
        LoadError {
            error: Box<dyn std::error::Error + Send + Sync + 'static>,
        },
        LoggedIn {
            chat_pipe: mpsc::UnboundedSender<SneedEvent>,
            manager: Arc<ChatManager>,
        },
    }

    impl SneedChat {
        fn new(flags: &<Self as iced::Application>::Flags) -> Self {
            Self::Loading
        }
    }

    impl iced::Application for SneedChat {
        type Executor = iced::executor::Default;
        type Message = Message;
        type Theme = iced::theme::Theme;
        type Flags = SneedChatFlags;

        fn new(flags: Self::Flags) -> (Self, iced::Command<Self::Message>) {
            (
                SneedChat::new(&flags),
                Command::perform(
                    async move {
                        let mut builder = ChatManager::builder(uuid::Uuid::new_v4()).with_netlayer(
                            "tcpip".to_owned(),
                            rexa::netlayer::datastream::TcpIpNetlayer::bind(
                                &([0, 0, 0, 0], flags.port).into(),
                            )
                            .await?,
                        );
                        if let Some(username) = flags.username {
                            builder = builder.with_username(username);
                        }
                        if let Some(designator) = flags.remote {
                            builder = builder.with_initial_remote(NodeLocator::new(
                                designator.to_string(),
                                "tcpip".to_owned(),
                            ));
                        }
                        Ok(builder.build())
                    },
                    Message::Loaded,
                ),
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
                    Message::Loaded(Ok(manager)) => {
                        let manager = Arc::new(manager);
                        *self = Self::LoggedIn {
                            chat_pipe: manager.event_sender().clone(),
                            manager: manager.clone(),
                        };
                        Command::none()
                    }
                    Message::Loaded(Err(error)) => {
                        *self = Self::LoadError { error };
                        Command::none()
                    }
                    _ => Command::none(),
                },
                SneedChat::LoadError { .. } => Command::none(),
                SneedChat::LoggedIn { chat_pipe, manager } => match message {
                    Message::ShiftFocus(backward) => shift_focus(backward),
                    Message::ChatEvent(ev) => match ev {
                        ChatEvent::RecvMessage {
                            user,
                            channel_id,
                            msg,
                        } => todo!(),
                        ChatEvent::UpdateProfile(id) => todo!(),
                        ChatEvent::Exit => todo!(),
                    },
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
                SneedChat::LoadError { error } => w::container(
                    w::text(format!("Error: {error}"))
                        .horizontal_alignment(alignment::Horizontal::Center),
                )
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
                SneedChat::LoggedIn { chat_pipe, manager } => w::container(w::column![
                    w::row![w::button("Connect")],
                    w::row![
                        w::column![w::text("Channels")],
                        w::column![w::text("current channel")]
                    ],
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
                SneedChat::LoggedIn { chat_pipe, manager } => Subscription::batch([
                    iced::subscription::unfold(
                        manager.profile.id,
                        manager.clone(),
                        |manager| async move {
                            (
                                manager.recv_event().await.map(Message::ChatEvent).unwrap(),
                                manager,
                            )
                        },
                    ),
                    keypress,
                ]),
                _ => keypress,
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
