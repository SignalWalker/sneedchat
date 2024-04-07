use std::{
    collections::HashMap,
    net::SocketAddr,
    ops::{Deref, DerefMut},
    sync::Arc,
};

use dashmap::DashMap;
use dioxus::prelude::*;
use ed25519_dalek::{ed25519::signature::SignerMut, SigningKey};
use futures::StreamExt;
use parking_lot::{Condvar, Mutex, RwLock};
use rexa::{
    captp::{object::DeliverError, AbstractCapTpSession, RemoteKey},
    locator::{NodeLocator, SturdyRefLocator},
};
use tokio::{sync::mpsc, task::JoinSet};
use troposphere_lib::{
    Channel, ChannelEvent, ChannelId, ChannelInfo, ChannelListing, ChatEvent, ChatManager, Message,
    NetlayerManager, PeerKey, Profile, RemotePortal, RemotePortalError, UserId,
};

use crate::cfg::{Config, WriteError};

#[derive(Debug, thiserror::Error)]
pub(crate) enum ChatError {
    #[error("{0}")]
    Inner(String),
    #[error(transparent)]
    WriteConfig(#[from] WriteError),
    #[error(transparent)]
    PortalOpen(#[from] RemotePortalError),
}

impl From<Box<dyn std::error::Error + Send + Sync + 'static>> for ChatError {
    fn from(value: Box<dyn std::error::Error + Send + Sync + 'static>) -> Self {
        Self::Inner(value.to_string())
    }
}

pub(super) type ListChannelsResult = Result<Vec<ChannelListing>, Arc<DeliverError>>;

#[derive(Default, Clone)]
pub(super) struct PortalState {
    pub(super) channels: Option<ListChannelsResult>,
}

#[derive(Clone, Default)]
pub(crate) struct ChatState {
    pub(crate) self_key: Arc<RwLock<PeerKey>>,

    pub(super) profiles: SyncSignal<HashMap<PeerKey, Profile>>,

    pub(crate) done_binding: Arc<(Mutex<bool>, Condvar)>,
    pub(crate) bound_addresses: Arc<RwLock<Vec<SocketAddr>>>,

    pub(super) opened_portals: SyncSignal<HashMap<RemoteKey, PortalState>>,

    pub(super) connected_channels: SyncSignal<HashMap<ChannelId, (Channel, Arc<ChannelState>)>>,
}

impl ChatState {
    pub(super) fn provider() -> Self {
        Self::default()
    }
}

pub(crate) enum ManagerEvent {
    Chat(ChatEvent),
    ConnectChannel {
        session_key: RemoteKey,
    },
    OpenPortal {
        locator: NodeLocator,
    },
    OpenedPortal {
        session_key: RemoteKey,
        portal: Arc<RemotePortal>,
    },
    ListedChannels {
        session_key: RemoteKey,
        channels: ListChannelsResult,
    },
}

impl From<ChatEvent> for ManagerEvent {
    fn from(value: ChatEvent) -> Self {
        Self::Chat(value)
    }
}

#[tracing::instrument(fields(session = rexa::hash(&session.remote_vkey())), skip(signing_key))]
fn handle_new_session(
    session: Arc<dyn AbstractCapTpSession + Send + Sync>,
    signing_key: &mut SigningKey,
) -> impl std::future::Future<Output = Result<ManagerEvent, ChatError>> {
    tracing::info!("handling new session");
    let signature = signing_key.sign(b"FIXTHIS");
    let vkey = signing_key.verifying_key();
    async move {
        let session_key = *session.remote_vkey();
        let portal = match RemotePortal::open(
            &session.into_remote_bootstrap(),
            &vkey,
            b"FIXTHIS",
            &signature,
        )
        .await
        {
            Ok(p) => Arc::new(p),
            Err(error) => {
                tracing::error!(
                    session = rexa::hash(&session_key),
                    %error,
                    "could not open portal"
                );
                return Err(ChatError::from(error));
            }
        };
        Ok(ManagerEvent::OpenedPortal {
            session_key,
            portal,
        })
    }
}

async fn manager_loop(
    mut input: UnboundedReceiver<ManagerEvent>,
    cfg: Arc<Config>,
    ChatState {
        self_key,
        mut profiles,
        done_binding,
        bound_addresses,
        mut opened_portals,
        mut connected_channels,
    }: ChatState,
) -> Result<(), ChatError> {
    tracing::trace!("initializing manager...");

    let signing_key = cfg.get_key_or_init()?;
    let self_vkey = signing_key.verifying_key();
    *self_key.write() = self_vkey;

    let mut manager = {
        let mut builder = ChatManager::builder(signing_key)
            .with_username(cfg.profile.username.clone())
            .with_avatar(cfg.profile.avatar.clone());

        #[cfg(not(target_family = "wasm"))]
        {
            use rexa_netlayers::datastream::TcpIpNetlayer;

            let tcp = &cfg.netlayers.tcpip;
            let mut listeners = Vec::with_capacity(tcp.listen_addresses.len());
            for addr in &tcp.listen_addresses {
                match tokio::net::TcpListener::bind((*addr, tcp.port)).await {
                    Ok(listener) => listeners.push(listener),
                    Err(error) => tracing::error!(
                        address = %addr,
                        error = %error,
                        "failed to bind tcp listener"
                    ),
                }
            }

            let tcpip = TcpIpNetlayer::new(listeners);

            *bound_addresses.write() = tcpip.addresses().collect();

            builder = builder.with_netlayer("tcpip".to_owned(), tcpip);

            *done_binding.0.lock() = true;
            done_binding.1.notify_all();
        }

        builder.build()
    };

    let self_profile = manager.persona.profile.read().await.clone();

    profiles.write().insert(self_vkey, self_profile.clone());

    tracing::trace!("starting chat manager loop");

    let mut portals = HashMap::<RemoteKey, Arc<RemotePortal>>::new();
    let mut channel_tasks = JoinSet::<Result<(), ChatError>>::new();
    let mut tasks = JoinSet::<Result<ManagerEvent, ChatError>>::new();

    {
        let channel_id = ChannelId::new_v4();
        let (ev_sender, ev_receiver) = mpsc::unbounded_channel();
        let channel = Channel::new(
            channel_id,
            ChannelInfo {
                name: format!("{}'s Channel", cfg.profile.username),
                description: "Channel!".to_string(),
            },
            ev_sender,
        );

        let (cmd_sender, cmd_receiver) = mpsc::unbounded_channel();

        let state = Arc::new(ChannelState::new(
            channel.clone(),
            cmd_sender,
            RwLock::new(HashMap::from_iter([(self_vkey, self_profile)])),
            Default::default(),
        ));

        manager.register_channel(channel.clone());

        connected_channels
            .write()
            .insert(channel_id, (channel.clone(), state.clone()));

        channel_tasks.spawn(manage_channel(
            channel,
            state,
            cmd_receiver,
            ev_receiver,
            manager.layers().clone(),
            manager.signing_key.clone(),
        ));
    }

    loop {
        let event: ManagerEvent = tokio::select! {
            event = manager.recv_event() => event.unwrap().into(),
            Some(Ok(task)) = tasks.join_next() => match task {
                Ok(event) => event,
                Err(error) => {
                    tracing::error!(%error, "chat subtask failed");
                    continue
                }
            },
            Some(event) = input.next() => event,
        };

        match event {
            ManagerEvent::Chat(ChatEvent::SessionStarted { session }) => {
                tasks.spawn(handle_new_session(
                    session,
                    &mut (*manager.signing_key.write()),
                ));
            }
            ManagerEvent::Chat(ChatEvent::SessionAborted {
                session_key,
                reason,
            }) => {
                tracing::trace!("session aborted, removing portal...");
                if portals.remove(&session_key).is_some() {
                    opened_portals.write().remove(&session_key);
                    tracing::info!(session_key = rexa::hash(&session_key), %reason, "session aborted");
                }
            }
            ManagerEvent::ListedChannels {
                session_key,
                channels,
            } => {
                opened_portals
                    .write()
                    .entry(session_key)
                    .or_default()
                    .channels = Some(channels);
            }
            ManagerEvent::OpenPortal { locator } => {
                if let Err(error) = manager.layers().request_connect(locator.clone()) {
                    tracing::error!(?locator, %error, "failed to process connect request");
                }
                // we'll receive a ChatEvent::SessionStarted when the session's connected
            }
            ManagerEvent::OpenedPortal {
                session_key,
                portal,
            } => {
                tracing::info!(session = rexa::hash(&session_key), "opened portal");
                portals.insert(session_key, portal.clone());
                opened_portals
                    .write()
                    .insert(session_key, PortalState::default());

                tasks.spawn(async move {
                    Ok(ManagerEvent::ListedChannels {
                        session_key,
                        channels: portal.list_channels().await.map_err(From::from),
                    })
                });
            }
        }
    }
}

pub(super) fn manager_coroutine(
    rx: UnboundedReceiver<ManagerEvent>,
) -> impl std::future::Future<Output = ()> + Send {
    let cfg = use_context::<Arc<Config>>();

    let mut chat_state = use_context::<ChatState>();
    // chat_state.bound_addresses.write().clear();
    // chat_state.opened_portals.write().clear();

    async move {
        if let Err(error) = manager_loop(rx, cfg, chat_state).await {
            tracing::error!("{error}");
        }
    }
}

mod _channel_state {
    use std::{
        collections::HashMap,
        ops::{Deref, DerefMut},
        sync::Arc,
    };

    use parking_lot::RwLock;
    use tokio::sync::{mpsc, Notify};
    use troposphere_lib::{Channel, Message, PeerKey, Profile};

    use super::ChannelCommand;

    pub(crate) struct ChannelState {
        pub(crate) channel: Channel,

        pub(crate) cmd_sender: mpsc::UnboundedSender<ChannelCommand>,

        peers_changed: Arc<Notify>,
        peers: RwLock<HashMap<PeerKey, Profile>>,

        messages_changed: Arc<Notify>,
        messages: RwLock<Vec<Message>>,
    }

    impl ChannelState {
        pub(super) fn new(
            channel: Channel,
            cmd_sender: mpsc::UnboundedSender<ChannelCommand>,
            peers: RwLock<HashMap<PeerKey, Profile>>,
            messages: RwLock<Vec<Message>>,
        ) -> Self {
            Self {
                channel,
                cmd_sender,
                peers_changed: Default::default(),
                peers,
                messages_changed: Default::default(),
                messages,
            }
        }

        pub(crate) fn peer_notif(&self) -> Arc<Notify> {
            self.peers_changed.clone()
        }

        pub(crate) fn peers<'p>(&'p self) -> impl Deref<Target = HashMap<PeerKey, Profile>> + 'p {
            self.peers.read()
        }

        pub(super) fn peers_mut<'p>(
            &'p self,
        ) -> impl Deref<Target = HashMap<PeerKey, Profile>> + DerefMut + 'p {
            let write = self.peers.write();
            self.peers_changed.notify_waiters();
            write
        }

        pub(crate) fn msg_notif(&self) -> Arc<Notify> {
            self.messages_changed.clone()
        }

        pub(crate) fn messages<'p>(&'p self) -> impl Deref<Target = Vec<Message>> + 'p {
            self.messages.read()
        }

        pub(super) fn messages_mut<'p>(
            &'p self,
        ) -> impl Deref<Target = Vec<Message>> + DerefMut + 'p {
            let write = self.messages.write();
            self.messages_changed.notify_waiters();
            write
        }
    }
}
pub(crate) use _channel_state::*;

pub(super) enum ChannelCommand {
    SendMsg { message: String },
}

#[tracing::instrument(fields(?channel), skip(state, ev_receiver, layers))]
async fn manage_channel(
    channel: Channel,
    mut state: Arc<ChannelState>,
    mut cmd_receiver: mpsc::UnboundedReceiver<ChannelCommand>,
    mut ev_receiver: mpsc::UnboundedReceiver<ChannelEvent>,
    layers: Arc<NetlayerManager>,
    signing_key: Arc<RwLock<SigningKey>>,
) -> Result<(), ChatError> {
    #[tracing::instrument(fields(?channel, peer_key = rexa::hash(&peer_key), ?locator, swiss = rexa::hash(&swiss)), skip(state, layers))]
    async fn introduce_peer(
        state: Arc<ChannelState>,
        layers: Arc<NetlayerManager>,
        channel: Channel,
        peer_key: PeerKey,
        SturdyRefLocator {
            node_locator: locator,
            swiss_num: swiss,
        }: SturdyRefLocator,
    ) {
        let conn_res = match layers.request_connect(locator) {
            Ok(req) => req.await,
            Err(error) => {
                tracing::error!(
                    %error,
                    "failed to send connect request for introduced peer"
                );
                return;
            }
        };

        let session = match conn_res {
            Ok(session) => session,
            Err(error) => {
                tracing::error!(%error, "failed to connect to introduced peer");
                return;
            }
        };

        todo!()
    }

    let self_vkey = signing_key.read().verifying_key();

    let mut tasks = JoinSet::new();

    loop {
        let event = tokio::select! {
            event = ev_receiver.recv() => {
                if let Some(ev) = event {
                    ev
                } else {
                    break
                }
            }
            Some(cmd) = cmd_receiver.recv() => match cmd {
                ChannelCommand::SendMsg { message } => {
                    let message = match Message::new(self_vkey, message, &mut (*signing_key.write())) {
                        Ok(msg) => msg,
                        Err(error) => todo!()
                    };
                    if let Err(error) = channel.send_msg(&message).await {
                        todo!()
                    }
                    state.messages_mut().push(message);
                    continue
                }
            }
        };

        match event {
            ChannelEvent::RecvMessage {
                channel: _,
                message,
            } => {
                state.messages_mut().push(message);
            }
            ChannelEvent::Introduce {
                channel,
                peer_key,
                locator,
            } => {
                tasks.spawn(introduce_peer(
                    state.clone(),
                    layers.clone(),
                    channel,
                    peer_key,
                    locator,
                ));
            }
        }
    }

    while let Some(res) = tasks.join_next().await {
        if let Err(error) = res {
            tracing::error!(%error, "channel task failed");
        }
    }

    Ok(())
}