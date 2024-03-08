use dashmap::DashMap;
use ed25519_dalek::{SigningKey, VerifyingKey};
use rexa::{captp::AbstractCapTpSession, locator::NodeLocator, netlayer::Netlayer};
use std::future::Future;
use std::{collections::HashMap, sync::Arc};
use syrup::RawSyrup;
use tokio::{
    sync::{mpsc, watch, Mutex},
    task::{JoinHandle, JoinSet},
};

mod event;
pub use event::*;

mod user;
pub use user::*;

mod channel;
pub use channel::*;

// mod outbox;
// pub use outbox::*;

mod session;
pub use session::*;

mod netlayer;
pub use netlayer::*;

mod input;
pub use input::*;

pub type EventSender = tokio::sync::mpsc::UnboundedSender<SneedEvent>;
pub type EventReceiver = tokio::sync::mpsc::UnboundedReceiver<SneedEvent>;
pub type Promise<V> = tokio::sync::oneshot::Receiver<V>;
pub type Resolver<V> = tokio::sync::oneshot::Sender<V>;
pub type Swiss = Vec<u8>;
pub type Outboxes = std::sync::Arc<dashmap::DashMap<uuid::Uuid, Outbox>>;
// type TcpIpSession = CapTpSession<
//     <netlayer::TcpIpNetlayer as rexa::netlayer::Netlayer>::Reader,
//     <netlayer::TcpIpNetlayer as rexa::netlayer::Netlayer>::Writer,
// >;
// type TcpIpSneedEvent = SneedEvent<
//     <netlayer::TcpIpNetlayer as rexa::netlayer::Netlayer>::Reader,
//     <netlayer::TcpIpNetlayer as rexa::netlayer::Netlayer>::Writer,
// >;
// type TcpIpUser = User<
//     <netlayer::TcpIpNetlayer as rexa::netlayer::Netlayer>::Reader,
//     <netlayer::TcpIpNetlayer as rexa::netlayer::Netlayer>::Writer,
// >;
// type TcpIpChannel = Channel<
//     <netlayer::TcpIpNetlayer as rexa::netlayer::Netlayer>::Reader,
//     <netlayer::TcpIpNetlayer as rexa::netlayer::Netlayer>::Writer,
// >;

#[derive(Debug, Clone, Copy)]
#[repr(transparent)]
pub struct SyrupUuid(uuid::Uuid);

impl<'input> syrup::Deserialize<'input> for SyrupUuid {
    fn deserialize<D: syrup::de::Deserializer<'input>>(de: D) -> Result<Self, D::Error> {
        Ok(Self(uuid::Uuid::from_bytes(
            syrup::Bytes::<[u8; 16]>::deserialize(de)?.0,
        )))
    }
}

impl syrup::Serialize for SyrupUuid {
    fn serialize<Ser: syrup::ser::Serializer>(&self, s: Ser) -> Result<Ser::Ok, Ser::Error> {
        syrup::Bytes::<&[u8]>(self.0.as_bytes()).serialize(s)
    }
}

impl From<uuid::Uuid> for SyrupUuid {
    fn from(value: uuid::Uuid) -> Self {
        Self(value)
    }
}

impl From<&uuid::Uuid> for SyrupUuid {
    fn from(value: &uuid::Uuid) -> Self {
        Self(*value)
    }
}

impl From<SyrupUuid> for uuid::Uuid {
    fn from(value: SyrupUuid) -> Self {
        value.0
    }
}

#[derive(Debug, Clone)]
pub enum ChatEvent {
    SendMessage {
        channel: Arc<Channel>,
        msg: String,
    },
    SendDirectMessage {
        peer: Arc<Peer>,
        msg: String,
    },
    RecvMessage {
        peer: Arc<Peer>,
        channel: Arc<Channel>,
        msg: String,
    },
    RecvDirectMessage {
        peer: Arc<Peer>,
        inbox: Arc<Inbox>,
        msg: String,
    },
    UpdateProfile {
        old_name: String,
        peer: Arc<Peer>,
    },
    PeerConnected(Arc<Peer>),
    PeerDisconnected(Arc<Peer>),
}

pub struct ChatManagerBuilder {
    signing_key: SigningKey,
    layers: NetlayerManager,
    ev_sender: EventSender,
    ev_receiver: EventReceiver,
    end_flag: watch::Receiver<bool>,
    end_notifier: watch::Sender<bool>,
    subscription_tasks: JoinSet<Result<(), Box<dyn std::error::Error + Send + Sync + 'static>>>,
    username: Option<String>,
}

impl ChatManagerBuilder {
    pub fn with_username(mut self, username: String) -> Self {
        self.username = Some(username);
        self
    }

    pub fn with_initial_remote(self, locator: NodeLocator<String, syrup::Item>) -> Self {
        self.ev_sender
            .send(SneedEvent::Command(Command::Connect(locator)))
            .unwrap();
        self
    }

    pub fn with_netlayer<Nl: Netlayer>(mut self, transport: String, netlayer: Nl) -> Self
    where
        Nl: Send + 'static,
        Nl::Reader: rexa::async_compat::AsyncRead + Unpin + Send,
        Nl::Writer: rexa::async_compat::AsyncWrite + Unpin + Send,
        Nl::Error: std::error::Error + Send + Sync + 'static,
    {
        self.layers.register(
            transport,
            netlayer,
            self.ev_sender.clone(),
            self.end_flag.clone(),
        );
        self
    }

    pub fn subscribe<
        F: Future<Output = Result<(), Box<dyn std::error::Error + Send + Sync + 'static>>>
            + Send
            + 'static,
    >(
        mut self,
        producer: impl FnOnce(EventSender, watch::Receiver<bool>) -> F,
    ) -> Self {
        self.subscription_tasks
            .spawn(producer(self.ev_sender.clone(), self.end_flag.clone()));
        self
    }

    pub fn subscribe_threaded<Res>(self, producer: impl FnOnce(EventSender) -> Res) -> (Self, Res) {
        let res = producer(self.ev_sender.clone());
        (self, res)
    }

    pub fn build(self) -> ChatManager {
        let skey = self.signing_key;
        let vkey = skey.verifying_key();
        let username = self
            .username
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let data = Arc::new(ChatData::default());
        let persona = Persona::new(
            Profile::new(vkey, username),
            data.clone(),
            self.ev_sender.clone(),
        );
        ChatManager {
            signing_key: skey,
            persona,

            layers: Arc::new(self.layers),
            ev_sender: self.ev_sender,
            ev_receiver: self.ev_receiver.into(),
            end_notifier: self.end_notifier,
            subscription_tasks: self.subscription_tasks.into(),

            data,
        }
    }
}

#[derive(Default)]
pub struct ChatData {
    sessions: DashMap<VerifyingKey, Arc<dyn AbstractCapTpSession + Send + Sync + 'static>>,

    /// Map from session -> Peer
    session_peers: DashMap<VerifyingKey, Arc<Peer>>,
    peers: DashMap<PeerKey, Arc<Peer>>,

    inboxes: DashMap<InboxId, Arc<Inbox>>,
    outboxes: DashMap<OutboxId, Arc<Outbox>>,

    channels: DashMap<ChannelId, Arc<Channel>>,
    /// Map from inbox id -> channel
    inbox_channels: DashMap<InboxId, Arc<Channel>>,
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
        if self.session_peers.contains_key(key) {
            tracing::warn!(
                key_hash = rexa::hash(key),
                "removing session for which there is still a peer"
            );
        }
        self.sessions.remove(key)
    }

    fn insert_peer(&self, session_key: VerifyingKey, peer: Arc<Peer>) {
        self.session_peers.insert(session_key, peer.clone());
        self.peers.insert(peer.profile.vkey, peer);
    }

    fn remove_peer(&self, session_key: &VerifyingKey) -> Option<(PeerKey, Arc<Peer>)> {
        self.session_peers
            .remove(session_key)
            .and_then(|(_, peer)| self.peers.remove(&peer.profile.vkey))
    }

    fn fetch_inbox<'s>(
        &'s self,
        swiss: &[u8],
    ) -> Option<dashmap::mapref::one::Ref<'s, InboxId, Arc<Inbox>>> {
        let id = InboxId::from_slice(swiss).ok()?;
        self.inboxes.get(&id)
    }

    fn fetch_channel<'s>(
        &'s self,
        swiss: &[u8],
    ) -> Option<dashmap::mapref::one::Ref<'s, InboxId, Arc<Channel>>> {
        let id = ChannelId::from_slice(swiss).ok()?;
        self.channels.get(&id)
    }

    fn clear(&self) {
        self.session_peers.clear();
        self.peers.clear();
        self.inboxes.clear();
        self.outboxes.clear();
        self.channels.clear();
        self.inbox_channels.clear();
    }
}

pub struct ChatManager {
    pub persona: Arc<Persona>,
    signing_key: SigningKey,

    layers: Arc<NetlayerManager>,
    ev_sender: EventSender,
    end_notifier: watch::Sender<bool>,

    ev_receiver: Mutex<EventReceiver>,
    subscription_tasks:
        Mutex<JoinSet<Result<(), Box<dyn std::error::Error + Send + Sync + 'static>>>>,

    data: Arc<ChatData>,
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
        let (ev_sender, ev_receiver) = tokio::sync::mpsc::unbounded_channel();
        let (end_notifier, mut end_flag) = tokio::sync::watch::channel(false);
        end_flag.mark_unchanged();

        ChatManagerBuilder {
            signing_key,
            layers: NetlayerManager::new(),
            ev_sender,
            ev_receiver,
            end_notifier,
            end_flag,
            subscription_tasks: JoinSet::new(),
            username: None,
        }
    }

    pub fn event_sender(&self) -> &EventSender {
        &self.ev_sender
    }

    async fn spawn_subtask(
        &self,
        subtask: impl Future<Output = Result<SneedEvent, Box<dyn std::error::Error + Send + Sync + 'static>>>
            + Send
            + 'static,
    ) {
        let ev_sender = self.ev_sender.clone();
        self.subscription_tasks.lock().await.spawn(async move {
            subtask
                .await
                .and_then(|event| ev_sender.send(event).map_err(From::from))
        });
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
                SneedEvent::TaskFinished { result } => break result,
                SneedEvent::Command(cmd) => match cmd {
                    Command::SendMessage { mailbox_id, msg } => match mailbox_id {
                        MailboxId::Channel(channel_id) => {
                            let Some(channel) = self.data.channels.get(&channel_id) else {
                                tracing::error!(id = %channel_id, "unrecognized channel id");
                                continue;
                            };
                            channel.send_msg(&msg).await?;
                            break Ok(ChatEvent::SendMessage {
                                channel: channel.clone(),
                                msg,
                            });
                        }
                        MailboxId::Peer(peer_key) => {
                            let Some(peer) = self.data.peers.get(&peer_key).map(|p| p.clone())
                            else {
                                tracing::error!(
                                    key_hash = rexa::hash(&peer_key),
                                    "unrecognized peer key"
                                );
                                continue;
                            };
                            let outbox = peer.get_personal_mailbox().await?;
                            outbox.send_msg(&msg).await?;
                            break Ok(ChatEvent::SendDirectMessage { peer, msg });
                        }
                    },
                    Command::SetName(name) => {
                        todo!()
                    }
                    Command::Connect(locator) => {
                        async fn connect(
                            layers: Arc<NetlayerManager>,
                            locator: rexa::locator::NodeLocator<String, syrup::Item>,
                            persona: Arc<Persona>,
                        ) -> Result<SneedEvent, Box<dyn std::error::Error + Send + Sync + 'static>>
                        {
                            let session = match layers.request_connect(locator.clone())?.await? {
                                Ok(session) => session,
                                Err(error) => {
                                    tracing::error!(
                                        ?locator,
                                        ?error,
                                        "could not connect to session"
                                    );
                                    return Err("could not connect to session".into());
                                }
                            };
                            let session_key = *session.remote_vkey();
                            let peer =
                                match Peer::fetch(session.clone().into_remote_bootstrap(), persona)
                                    .await
                                {
                                    Ok(user) => user,
                                    Err(reason) => {
                                        tokio::spawn(async move {
                                            session.abort("failed to fetch peer".to_owned()).await
                                        });
                                        tracing::error!(?locator, ?reason, "could not fetch peer");
                                        return Err("could not fetch peer".into());
                                    }
                                };

                            Ok(SneedEvent::PeerConnected { session_key, peer })
                        }
                        self.spawn_subtask(connect(
                            self.layers.clone(),
                            locator,
                            self.persona.clone(),
                        ))
                        .await;
                    }
                },
                SneedEvent::Fetch {
                    session_key: _,
                    swiss,
                    resolver,
                } => {
                    fn peer_key_from_slice(bytes: &[u8]) -> Option<PeerKey> {
                        <[u8; ed25519_dalek::PUBLIC_KEY_LENGTH]>::try_from(bytes)
                            .ok()
                            .and_then(|bytes| PeerKey::from_bytes(&bytes).ok())
                    }
                    if let Some(inbox) = self.data.fetch_inbox(&swiss) {
                        let _ = resolver.send(Ok(inbox.clone()));
                    } else if let Some(channel) = self.data.fetch_channel(&swiss) {
                        let _ = resolver.send(Ok(channel.clone()));
                    } else if let Some(_) = peer_key_from_slice(&swiss) {
                        let _ = resolver.send(Ok(self.persona.clone()));
                    } else {
                        let _ = resolver.send(Err(RawSyrup::from_serialize("unrecognized swiss")));
                    }
                }
                SneedEvent::RecvMessage {
                    inbox_id,
                    session_key,
                    msg,
                } => {
                    async fn handle_msg(
                        inbox_id: InboxId,
                        session_key: VerifyingKey,
                        msg: String,
                        persona: Arc<Persona>,
                        data: Arc<ChatData>,
                        ev_pipe: EventSender,
                    ) -> Result<SneedEvent, Box<dyn std::error::Error + Send + Sync + 'static>>
                    {
                        let session = data.sessions.get(&session_key).unwrap().clone();
                        let peer = match data.session_peers.get(&session_key).map(|p| p.clone()) {
                            Some(p) => p,
                            None => {
                                let peer =
                                    Peer::fetch(session.into_remote_bootstrap(), persona).await?;
                                let _ = ev_pipe.send(SneedEvent::PeerConnected {
                                    session_key,
                                    peer: peer.clone(),
                                });
                                peer
                            }
                        };
                        let res = match data.inbox_channels.get(&inbox_id) {
                            Some(channel) => ChatEvent::RecvMessage {
                                peer,
                                channel: channel.clone(),
                                msg,
                            },
                            None => ChatEvent::RecvDirectMessage {
                                peer,
                                inbox: data.inboxes.get(&inbox_id).unwrap().clone(),
                                msg,
                            },
                        };
                        Ok(SneedEvent::TaskFinished { result: Ok(res) })
                    }
                    self.spawn_subtask(handle_msg(
                        inbox_id,
                        session_key,
                        msg,
                        self.persona.clone(),
                        self.data.clone(),
                        self.ev_sender.clone(),
                    ))
                    .await;
                }
                SneedEvent::PeerConnected { session_key, peer } => {
                    self.data.insert_peer(session_key, peer.clone());
                    break Ok(ChatEvent::PeerConnected(peer));
                }
                SneedEvent::NewSession(session) => {
                    self.data.insert_session(session);
                }
                SneedEvent::SessionAborted {
                    session_key,
                    reason,
                } => {
                    tracing::info!(session_key = rexa::hash(&session_key), %reason, "session aborted");
                    self.data.remove_session(&session_key);
                    if let Some((_, peer)) = self.data.remove_peer(&session_key) {
                        break Ok(ChatEvent::PeerDisconnected(peer));
                    }
                }
            }
        }
    }

    pub async fn end(self) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
        self.data.clear();
        self.end_notifier.send(true)?;
        let mut tasks = self.subscription_tasks.lock().await;
        while let Some(res) = tasks.join_next().await {
            res??;
        }
        Ok(())
    }
}
