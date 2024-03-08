use crate::chat::{ChannelId, InboxId, OutboxId, PeerKey};
use rexa::locator::NodeLocator;
use std::str::FromStr;

#[derive(Clone, Copy)]
pub enum MailboxId {
    Channel(ChannelId),
    Peer(PeerKey),
}

impl std::fmt::Debug for MailboxId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Channel(arg0) => f.debug_tuple("Channel").field(arg0).finish(),
            Self::Peer(arg0) => f.debug_tuple("Peer").field(&rexa::hash(arg0)).finish(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error("unrecognized command: {0} {1:?}")]
    Unrecognized(String, Vec<String>),
    #[error("invalid arguments for command {0}: {0:?}")]
    InvalidArguments(String, Vec<String>),
    #[error("missing argument for command {0}: {1}")]
    MissingArgument(String, &'static str),
}

#[derive(Clone)]
pub enum Command {
    SendMessage { mailbox_id: MailboxId, msg: String },
    Connect(NodeLocator<String, syrup::Item>),
    SetName(String),
}

impl std::fmt::Debug for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SendMessage { mailbox_id, msg } => write!(f, "`/msg {mailbox_id:?} {msg}`"),
            Self::Connect(locator) => write!(f, "`/connect {locator:?}`"),
            Self::SetName(name) => write!(f, "`/setname {name}`"),
        }
    }
}

impl FromStr for Command {
    type Err = CommandError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut split = s.split_whitespace();
        let cmd = split.next().unwrap();
        match cmd {
            "connect" => {
                let Some(designator) = split.next() else {
                    return Err(CommandError::MissingArgument(cmd.to_owned(), "designator"));
                };
                Ok(Command::Connect(NodeLocator::new(
                    designator.to_owned(),
                    "tcpip".to_owned(),
                )))
            }
            "setname" => {
                let Some(name) = split.next() else {
                    return Err(CommandError::MissingArgument(cmd.to_owned(), "username"));
                };
                Ok(Self::SetName(name.to_owned()))
            }
            cmd => Err(CommandError::Unrecognized(
                cmd.to_owned(),
                split.map(ToOwned::to_owned).collect(),
            )),
        }
    }
}
