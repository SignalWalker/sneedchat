use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use dioxus::prelude::*;
use ed25519_dalek::{ed25519::signature::SignerMut, SigningKey};
use futures::StreamExt;
use parking_lot::{Condvar, Mutex, RwLock};
use rexa::{
    captp::{AbstractCapTpSession, RemoteKey},
    locator::NodeLocator,
};
use tokio::task::JoinSet;
use troposphere::{ChannelId, ChatEvent, ChatManager, PeerKey, RemotePortal, RemotePortalError};

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

#[derive(Clone, Default)]
pub(crate) struct ChatState {
    pub(crate) self_key: Arc<RwLock<RemoteKey>>,

    pub(crate) done_binding: Arc<(Mutex<bool>, Condvar)>,
    pub(crate) bound_addresses: Arc<RwLock<Vec<SocketAddr>>>,

    pub(super) local_locators: SyncSignal<Vec<NodeLocator>>,
    pub(super) portals: SyncSignal<HashMap<RemoteKey, Arc<RemotePortal>>>,
}

impl ChatState {
    pub(super) fn provider() -> Self {
        Self::default()
    }
}

pub(crate) enum ManagerEvent {
    Chat(ChatEvent),
    OpenPortal {
        locator: NodeLocator,
    },
    OpenedPortal {
        session_key: RemoteKey,
        portal: Arc<RemotePortal>,
    },
}

impl From<ChatEvent> for ManagerEvent {
    fn from(value: ChatEvent) -> Self {
        Self::Chat(value)
    }
}

fn handle_new_session(
    session: Arc<dyn AbstractCapTpSession + Send + Sync>,
    signing_key: &mut SigningKey,
) -> impl std::future::Future<Output = Result<ManagerEvent, ChatError>> {
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
        done_binding,
        bound_addresses,
        local_locators,
        portals: mut portals_signal,
    }: ChatState,
) -> Result<(), ChatError> {
    tracing::trace!("initializing manager...");

    let signing_key = cfg.get_key_or_init()?;
    *self_key.write() = signing_key.verifying_key();

    let mut manager = {
        let mut builder =
            ChatManager::builder(signing_key).with_username(cfg.profile.username.clone());

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

    tracing::trace!("starting chat manager loop");

    let mut portals = HashMap::<RemoteKey, Arc<RemotePortal>>::new();
    let mut tasks = JoinSet::<Result<ManagerEvent, ChatError>>::new();
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
                tasks.spawn(handle_new_session(session, manager.signing_key_mut()));
            }
            ManagerEvent::Chat(ChatEvent::SessionAborted {
                session_key,
                reason,
            }) => {
                if portals.remove(&session_key).is_some() {
                    portals_signal.write().remove(&session_key);
                    tracing::info!(session_key = rexa::hash(&session_key), %reason, "session aborted");
                }
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
                portals.insert(session_key, portal.clone());
                portals_signal.write().insert(session_key, portal);
            }
        }
    }
}

pub(super) fn manager_coroutine(
    rx: UnboundedReceiver<ManagerEvent>,
) -> impl std::future::Future<Output = ()> + Send {
    let cfg = use_context::<Arc<Config>>();

    let mut chat_state = use_context::<ChatState>();
    chat_state.bound_addresses.write().clear();
    chat_state.portals.write().clear();

    async move {
        if let Err(error) = manager_loop(rx, cfg, chat_state).await {
            tracing::error!("{error}");
        }
    }
}
