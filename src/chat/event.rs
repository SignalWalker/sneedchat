use crate::chat::{self, Channel, ChatEvent, InboxId, Peer, PeerKey, Resolver, Swiss};
use ed25519_dalek::VerifyingKey;
use rexa::captp::{object::Object, AbstractCapTpSession, FetchResolver};
use std::sync::Arc;
use syrup::RawSyrup;
use tokio::sync::oneshot;

pub enum SneedEvent {
    NewSession(Arc<dyn AbstractCapTpSession + Send + Sync + 'static>),
    Fetch {
        session_key: VerifyingKey,
        swiss: Vec<u8>,
        resolver: oneshot::Sender<Result<Arc<dyn Object + Send + Sync>, RawSyrup>>,
    },
    // UpdateProfile {
    //     peer_key: PeerKey,
    //     new_name: String,
    // },
    RecvMessage {
        inbox_id: InboxId,
        session_key: VerifyingKey,
        msg: String,
    },
    PeerConnected {
        session_key: VerifyingKey,
        peer: Arc<Peer>,
    },
    SessionAborted {
        session_key: VerifyingKey,
        reason: String,
    },
    TaskFinished {
        result: Result<ChatEvent, Box<dyn std::error::Error + Send + Sync + 'static>>,
    },
    Command(chat::Command),
}

impl std::fmt::Debug for SneedEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TaskFinished { .. } => f.debug_struct("TaskFinished").finish_non_exhaustive(),
            Self::Fetch {
                session_key, swiss, ..
            } => f
                .debug_struct("Fetch")
                .field("session_key", &rexa::hash(session_key))
                .field("swiss", &String::from_utf8_lossy(swiss))
                .finish_non_exhaustive(),
            // Self::UpdateProfile { peer_key, new_name } => f
            //     .debug_struct("UpdateProfile")
            //     .field("peer_key", &rexa::hash(peer_key))
            //     .field("new_name", new_name)
            //     .finish(),
            Self::RecvMessage {
                inbox_id,
                session_key,
                msg,
            } => f
                .debug_struct("RecvMessage")
                .field("inbox_id", inbox_id)
                .field("session_key", &rexa::hash(session_key))
                .field("msg", msg)
                .finish(),
            Self::Command(cmd) => f.debug_tuple("Command").field(cmd).finish(),
            Self::PeerConnected { session_key, peer } => f
                .debug_struct("PeerConnected")
                .field("session_key", &rexa::hash(session_key))
                .field("peer", peer)
                .finish(),
            Self::SessionAborted {
                session_key,
                reason,
            } => f
                .debug_struct("SessionAborted")
                .field("session_key", &rexa::hash(session_key))
                .field("reason", reason)
                .finish(),
            Self::NewSession(session) => f
                .debug_tuple("NewSession")
                .field(&rexa::hash(session.remote_vkey()))
                .finish(),
        }
    }
}
