use crate::chat::ChatManager;

pub mod chat;
mod cli;
// #[cfg(feature = "iced")]
// mod gui;

// #[cfg(feature = "iced")]
// fn main() -> iced::Result {
//     use iced::Application;
//
//     let args = <cli::Cli as clap::Parser>::parse();
//     cli::initialize_tracing(args.log_filter.clone(), args.log_format);
//
//     gui::SneedChat::run(iced::Settings {
//         window: iced::window::Settings {
//             size: iced::Size {
//                 width: 640.0,
//                 height: 480.0,
//             },
//             ..Default::default()
//         },
//         flags: gui::SneedChatFlags::from(args),
//         ..Default::default()
//     })
// }

// #[cfg(feature = "egui")]
// fn main() -> eframe::Result<()> {
//     let args = <cli::Cli as clap::Parser>::parse();
//     cli::initialize_tracing(args.log_filter, args.log_format);
//     let options = eframe::NativeOptions {
//         viewport: egui::ViewportBuilder::default().with_inner_size([640.0, 480.0]),
//         renderer: eframe::Renderer::Wgpu,
//         ..Default::default()
//     };
//     eframe::run_native(
//         "SneedChat",
//         options,
//         Box::new(|cc| {
//             egui_extras::install_image_loaders(&cc.egui_ctx);
//             Box::<SneedChat>::default()
//         }),
//     )
// }

fn get_time(
    offset: time::UtcOffset,
    format: &(impl time::formatting::Formattable + ?Sized),
) -> String {
    time::OffsetDateTime::now_utc()
        .to_offset(offset)
        .format(format)
        .unwrap()
}

// pub struct MessageWriter<'fmt, Writer, TimeFormat: time::formatting::Formattable + ?Sized> {
//     time_offset: time::UtcOffset,
//     time_format: &'fmt TimeFormat,
//     writer: Writer,
//
//     needs_time: bool,
// }
//
// impl<'f, Writer: std::fmt::Write, TimeFmt: time::formatting::Formattable> std::fmt::Write
//     for MessageWriter<'f, Writer, TimeFmt>
// {
//     fn write_str(&mut self, s: &str) -> std::fmt::Result {
//         let split = s.split('\n');
//         for line in split {
//             writeln!(
//                 &mut self.writer,
//                 "[{}] {line}",
//                 get_time(self.time_offset, self.time_format)
//             )?;
//         }
//         Ok(())
//     }
// }
//
// macro_rules! printmsg {
//     ($fmt:expr$(, $($arg:tt)*)?) => {
//         println!(concat!("[{0}] ", $fmt), get_time(time_offset, &time_format)$(, $($arg)*)?)
//     };
// }

// #[cfg(not(feature = "iced"))]
fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    use crate::chat::{handle_input, IoEvent, MailboxId};
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;
    use tokio::sync::mpsc;

    let args = <cli::Cli as clap::Parser>::parse();
    cli::initialize_tracing(args.log_filter, args.log_format);

    let time_offset = time::UtcOffset::current_local_offset().unwrap_or_else(|e| {
        tracing::warn!(reason = %e, "couldn't get local time offset");
        time::UtcOffset::UTC
    });
    let time_format = time::macros::format_description!("[hour]:[minute]:[second]");

    let signing_key = SigningKey::generate(&mut OsRng);

    tokio::runtime::Builder::new_multi_thread()
        .unhandled_panic(tokio::runtime::UnhandledPanic::ShutdownRuntime)
        .enable_all()
        .build()?
        .block_on(async move {
            let (io_send, io_recv) = mpsc::unbounded_channel();

            let mut builder = ChatManager::builder(signing_key)
                .with_netlayer(
                    "tcpip".to_owned(),
                    rexa::netlayer::datastream::TcpIpNetlayer::bind(
                        &([0, 0, 0, 0], args.port).into(),
                    )
                    .await?,
                )
                .subscribe(handle_input(io_send.clone(), io_recv)?);
            if let Some(username) = args.username {
                builder = builder.with_username(username);
            }
            if let Some(designator) = args.remote {
                builder = builder.with_initial_remote(rexa::locator::NodeLocator::new(
                    designator.to_string(),
                    "tcpip".to_owned(),
                ));
            }
            let manager = builder.build();

            let mut current_mailbox = None;

            tokio::spawn(async move {
                loop {
                    let event = tokio::select! {
                        _ = tokio::signal::ctrl_c() => break,
                        event = manager.recv_event() => event?,
                    };
                    match event {
                        chat::ChatEvent::SendMessage { channel, msg } => {
                            println!(
                                "[{}][{}][{}] {msg}",
                                get_time(time_offset, &time_format),
                                channel.name,
                                manager.persona.profile.read().await.username
                            );
                        }
                        chat::ChatEvent::SendDirectMessage { peer, msg } => {
                            println!(
                                "[{}][->{}][{}] {msg}",
                                get_time(time_offset, &time_format),
                                peer.profile.username,
                                manager.persona.profile.read().await.username
                            );
                        }
                        chat::ChatEvent::RecvMessage { peer, channel, msg } => {
                            println!(
                                "[{}][{}][{}] {msg}",
                                get_time(time_offset, &time_format),
                                channel.name,
                                peer.profile.username
                            )
                        }
                        chat::ChatEvent::RecvDirectMessage {
                            peer,
                            inbox: _,
                            msg,
                        } => {
                            println!(
                                "[{}][->{}][{}] {msg}",
                                get_time(time_offset, &time_format),
                                peer.profile.username,
                                peer.profile.username
                            );
                        }
                        chat::ChatEvent::UpdateProfile { old_name, peer } => {
                            println!(
                                "[{}] {old_name} changed their username to {}",
                                get_time(time_offset, &time_format),
                                peer.profile.username
                            );
                        }
                        chat::ChatEvent::PeerConnected(peer) => {
                            println!(
                                "[{}] {} connected.",
                                get_time(time_offset, &time_format),
                                peer.profile.username
                            );
                            if current_mailbox.is_none() {
                                current_mailbox = Some(peer.get_personal_mailbox().await?);
                                io_send
                                    .send(IoEvent::ChangeTarget(MailboxId::Peer(peer.profile.vkey)))
                                    .unwrap();
                                println!(
                                    "[{}] Switched to channel: [->{}]",
                                    get_time(time_offset, &time_format),
                                    peer.profile.username
                                );
                            }
                        }
                        chat::ChatEvent::PeerDisconnected(peer) => {
                            println!(
                                "[{}] {} disconnected.",
                                get_time(time_offset, &time_format),
                                peer.profile.username
                            );
                        }
                    }
                }
                manager.end().await
            })
            .await?
        })
}
