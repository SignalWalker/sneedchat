use std::sync::Arc;

use ed25519_dalek::VerifyingKey;
use rexa::captp::{object::Object, AbstractCapTpSession};
use syrup::RawSyrup;
use tokio::sync::oneshot;

use crate::ChatEvent;

pub enum NetworkEvent {
    SessionStarted(Arc<dyn AbstractCapTpSession + Send + Sync + 'static>),
    Fetch {
        session_key: VerifyingKey,
        swiss: Vec<u8>,
        resolver: oneshot::Sender<Result<Arc<dyn Object + Send + Sync>, RawSyrup>>,
    },
    SessionAborted {
        session_key: VerifyingKey,
        reason: String,
    },
    TaskFinished {
        result: Result<ChatEvent, Box<dyn std::error::Error + Send + Sync + 'static>>,
    },
}

impl std::fmt::Debug for NetworkEvent {
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
            Self::SessionAborted {
                session_key,
                reason,
            } => f
                .debug_struct("SessionAborted")
                .field("session_key", &rexa::hash(session_key))
                .field("reason", reason)
                .finish(),
            Self::SessionStarted(session) => f
                .debug_tuple("NewSession")
                .field(&rexa::hash(session.remote_vkey()))
                .finish(),
        }
    }
}
