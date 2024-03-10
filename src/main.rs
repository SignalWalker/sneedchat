pub mod chat;
mod cli;

#[cfg(feature = "gui")]
mod gui;

#[cfg(feature = "gui")]
fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    let args = <cli::Cli as clap::Parser>::parse();
    cli::initialize_tracing(args.log_filter.clone(), args.log_format);

    gui::run(args)
}

fn get_time(
    offset: time::UtcOffset,
    format: &(impl time::formatting::Formattable + ?Sized),
) -> String {
    time::OffsetDateTime::now_utc()
        .to_offset(offset)
        .format(format)
        .unwrap()
}

#[cfg(not(feature = "gui"))]
fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    use crate::chat::ChatManager;
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
        // .unhandled_panic(tokio::runtime::UnhandledPanic::ShutdownRuntime)
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
