use dashmap::DashMap;
use ed25519_dalek::{SigningKey, VerifyingKey};
use rexa::{
    captp::{msg::OpAbort, CapTpSession},
    locator::NodeLocator,
    netlayer::Netlayer,
};
use std::{collections::HashMap, hash::Hash, sync::Arc};
use syrup::Serialize;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::RwLock,
};

pub type UserId = ed25519_dalek::VerifyingKey;
pub type ChannelId = uuid::Uuid;

#[derive(Debug, syrup::Deserialize, syrup::Serialize)]
pub struct Profile {
    id: UserId,
    name: String,
}

pub struct ChatManager {
    pub key: SigningKey,
    pub profile: Profile,
    pub known_profiles: DashMap<UserId, Profile>,
    pub channels: DashMap<ChannelId, Vec<(UserId, Event)>>,
    // pub netlayer: Arc<TcpIpNetlayer>,
}

impl std::fmt::Debug for ChatManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatManager")
            .field("profile", &self.profile)
            .finish_non_exhaustive()
    }
}

impl ChatManager {
    pub fn new(netlayer: Arc<TcpIpNetlayer>, username: String) -> Arc<Self> {
        let key = SigningKey::generate(&mut rand::rngs::OsRng);
        Arc::new(Self {
            profile: Profile {
                id: key.verifying_key(),
                name: username,
            },
            key,
            known_profiles: DashMap::new(),
            channels: DashMap::new(),
            netlayer,
        })
    }

    // pub fn events(self: Arc<Self>) -> impl futures::Stream<Item = Event> {
    //     futures::stream::unfold(
    //         self,
    //         |manager| async move { match manager.accept().await {} },
    //     )
    // }
}

#[derive(Debug, Clone)]
pub enum Event {}

// #[derive(Debug, Clone)]
// pub enum EventKind {
//     Message(String),
// }
//
// #[derive(Debug, Clone)]
// pub struct Event {
//     sender: UserId,
//     dst: ChannelId,
//     kind: EventKind,
// }
//
// impl Event {
//     pub fn new(sender: UserId, dst: ChannelId, kind: EventKind) -> Self {
//         Self { sender, dst, kind }
//     }
//
//     pub fn message(sender: UserId, dst: ChannelId, data: String) -> Self {
//         Self::new(sender, dst, EventKind::Message(data))
//     }
// }

pub struct SessionManager {
    sessions: DashMap<VerifyingKey, CapTpSession<TcpStream, TcpStream>>,
    profiles: DashMap<VerifyingKey, Profile>,
}

impl SessionManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            sessions: DashMap::new(),
            profiles: DashMap::new(),
        })
    }

    pub fn insert(
        &self,
        key: VerifyingKey,
        val: CapTpSession<TcpStream, TcpStream>,
    ) -> Option<CapTpSession<TcpStream, TcpStream>> {
        self.sessions.insert(key, val)
    }

    pub fn remove(
        &self,
        key: &VerifyingKey,
    ) -> Option<(VerifyingKey, CapTpSession<TcpStream, TcpStream>)> {
        self.sessions.remove(key)
    }
}
