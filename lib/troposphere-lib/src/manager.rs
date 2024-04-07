use std::future::Future;
use std::sync::Arc;

use dashmap::DashMap;
use ed25519_dalek::{SigningKey, VerifyingKey};
use rexa::captp::{AbstractCapTpSession, RemoteKey};
use syrup::RawSyrup;
use tokio::{
    sync::{watch, Mutex},
    task::JoinSet,
};

use crate::{
    Channel, ChannelId, EventReceiver, EventSender, Gateway, NetlayerManager, NetworkEvent,
    PeerKey, Persona, Portal, GATEWAY_SWISS,
};

mod builder;
pub use builder::*;

#[derive(Clone)]
pub enum ChatEvent {
    SessionStarted {
        session: Arc<dyn AbstractCapTpSession + Send + Sync>,
    },
    SessionAborted {
        session_key: RemoteKey,
        reason: String,
    },
}

impl std::fmt::Debug for ChatEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SessionStarted { session } => f
                .debug_struct("SessionStarted")
                .field("session", &rexa::hash(session.remote_vkey()))
                .finish(),
            Self::SessionAborted {
                session_key,
                reason,
            } => f
                .debug_struct("SessionAborted")
                .field("session_key", &rexa::hash(session_key))
                .field("reason", reason)
                .finish(),
        }
    }
}

#[derive(Default)]
pub struct ChatData {
    sessions: DashMap<VerifyingKey, Arc<dyn AbstractCapTpSession + Send + Sync + 'static>>,

    channels: DashMap<ChannelId, Arc<Channel>>,
}

impl ChatData {
    fn insert_session(&self, session: Arc<dyn AbstractCapTpSession + Send + Sync + 'static>) {
        self.sessions.insert(*session.remote_vkey(), session);
    }

    fn remove_session(
        &self,
        key: &VerifyingKey,
    ) -> Option<(
        VerifyingKey,
        Arc<dyn AbstractCapTpSession + Send + Sync + 'static>,
    )> {
        self.sessions.remove(key)
    }

    fn fetch_channel<'s>(
        &'s self,
        swiss: &[u8],
    ) -> Option<dashmap::mapref::one::Ref<'s, ChannelId, Arc<Channel>>> {
        ChannelId::from_slice(swiss)
            .ok()
            .and_then(|id| self.channels.get(&id))
    }
}

pub struct ChatManager {
    pub persona: Arc<Persona>,
    pub signing_key: Arc<parking_lot::RwLock<SigningKey>>,

    layers: Arc<NetlayerManager>,
    ev_sender: EventSender,
    end_notifier: watch::Sender<bool>,

    ev_receiver: Mutex<EventReceiver>,
    subscription_tasks:
        Mutex<JoinSet<Result<(), Box<dyn std::error::Error + Send + Sync + 'static>>>>,

    data: Arc<ChatData>,

    gateway: Arc<Gateway>,
    portals: Arc<DashMap<PeerKey, Arc<Portal>>>,
    channels: Arc<DashMap<ChannelId, Channel>>,
}

impl std::fmt::Debug for ChatManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatManager")
            .field("persona", &self.persona)
            .finish_non_exhaustive()
    }
}

impl ChatManager {
    pub fn builder(signing_key: SigningKey) -> ChatManagerBuilder {
        ChatManagerBuilder::new(signing_key)
    }

    pub fn event_sender(&self) -> &EventSender {
        &self.ev_sender
    }

    pub fn layers(&self) -> &Arc<NetlayerManager> {
        &self.layers
    }

    async fn spawn_subtask(
        &self,
        subtask: impl Future<
                Output = Result<NetworkEvent, Box<dyn std::error::Error + Send + Sync + 'static>>,
            > + Send
            + 'static,
    ) {
        let ev_sender = self.ev_sender.clone();
        self.subscription_tasks.lock().await.spawn(async move {
            subtask
                .await
                .and_then(|event| ev_sender.send(event).map_err(From::from))
        });
    }

    pub fn register_channel(&self, channel: Channel) -> Option<Channel> {
        self.channels.insert(*channel.id(), channel)
    }

    pub async fn recv_event(
        &self,
    ) -> Result<ChatEvent, Box<dyn std::error::Error + Send + Sync + 'static>> {
        let mut ev_receiver = self.ev_receiver.lock().await;
        loop {
            tracing::trace!("awaiting chat event");
            let event = ev_receiver.recv().await.unwrap();
            tracing::debug!(?event, "received event");
            match event {
                NetworkEvent::TaskFinished { result } => break result,
                NetworkEvent::Fetch {
                    session_key: _,
                    swiss,
                    resolver,
                } => {
                    if swiss == GATEWAY_SWISS {
                        drop(resolver.send(Ok(self.gateway.clone())));
                    } else if let Some(channel) = self.data.fetch_channel(&swiss) {
                        drop(resolver.send(Ok(channel.clone())));
                    } else {
                        drop(resolver.send(Err(RawSyrup::from_serialize("unrecognized swiss"))));
                    }
                }
                NetworkEvent::SessionStarted(session) => {
                    self.data.insert_session(session.clone());
                    break Ok(ChatEvent::SessionStarted { session });
                }
                NetworkEvent::SessionAborted {
                    session_key,
                    reason,
                } => {
                    tracing::info!(session_key = rexa::hash(&session_key), %reason, "session aborted");
                    self.data.remove_session(&session_key);
                    break Ok(ChatEvent::SessionAborted {
                        session_key,
                        reason,
                    });
                }
                NetworkEvent::PortalRequest {
                    session,
                    peer_vkey,
                    resolver,
                } => {
                    tracing::debug!(
                        peer_vkey = rexa::hash(&peer_vkey),
                        "received portal request"
                    );
                    let pos = session.exports().export(
                        self.portals
                            .entry(peer_vkey)
                            .or_insert_with(|| {
                                Arc::new(Portal::new(peer_vkey, self.channels.clone()))
                            })
                            .clone(),
                    );
                    if let Err(error) = resolver.fulfill([&pos], None, Default::default()).await {
                        tracing::error!(%error, "could not fulfill portal request");
                    };
                }
            }
        }
    }

    pub async fn end(self) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
        self.end_notifier.send(true)?;
        let mut tasks = self.subscription_tasks.lock().await;
        while let Some(res) = tasks.join_next().await {
            res??;
        }
        Ok(())
    }
}
