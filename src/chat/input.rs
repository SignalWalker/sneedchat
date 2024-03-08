use crate::chat::{ChannelId, EventSender, InboxId, SneedEvent};
use futures::{future::BoxFuture, FutureExt};
use std::future::Future;
use std::str::FromStr;
use tokio::sync::{mpsc, watch};

mod command;
pub use command::*;

#[derive(Debug)]
pub enum IoEvent {
    Input(Result<String, std::io::Error>),
    ChangeTarget(MailboxId),
}

#[tracing::instrument(skip_all)]
pub fn handle_input<'f>(
    io_send: mpsc::UnboundedSender<IoEvent>,
    mut io_recv: mpsc::UnboundedReceiver<IoEvent>,
) -> Result<
    impl FnOnce(
        EventSender,
        watch::Receiver<bool>,
    ) -> BoxFuture<'f, Result<(), Box<dyn std::error::Error + Send + Sync + 'static>>>,
    std::io::Error,
> {
    tracing::debug!("handling input");
    let _input_thread = std::thread::Builder::new()
        .name("read_lines".to_string())
        .spawn(move || {
            for msg in std::io::stdin().lines() {
                io_send.send(IoEvent::Input(msg))?;
            }
            Result::<(), Box<dyn std::error::Error + Send + Sync + 'static>>::Ok(())
        })?;
    Ok(
        move |ev_pipe: EventSender, mut end_flag: watch::Receiver<bool>| {
            async move {
                let mut current_mailbox = None;
                loop {
                    let event = tokio::select! {
                        event = io_recv.recv() => match event {
                            Some(ev) => ev,
                            None => break
                        },
                        _ = end_flag.changed() => break
                    };
                    match event {
                        IoEvent::Input(msg) => {
                            let msg = msg?;
                            match msg.as_bytes()[0] {
                                b'/' if msg.len() > 1 => {
                                    match Command::from_str(&msg[1..]) {
                                        Ok(cmd) => ev_pipe.send(SneedEvent::Command(cmd)).unwrap(),
                                        Err(e) => {
                                            tracing::error!("{e}");
                                            continue;
                                        }
                                    };
                                }
                                _ => match current_mailbox {
                                    Some(mailbox_id) => ev_pipe
                                        .send(SneedEvent::Command(Command::SendMessage {
                                            mailbox_id,
                                            msg,
                                        }))
                                        .unwrap(),
                                    None => {
                                        tracing::error!(
                                            "could not send message: not connected to a mailbox"
                                        )
                                    }
                                },
                            }
                        }
                        IoEvent::ChangeTarget(target) => {
                            current_mailbox = Some(target);
                        }
                    };
                }
                // if input_thread.is_finished() {
                //     let res = input_thread.join();
                // }
                Ok(())
            }
            .boxed()
        },
    )
}
