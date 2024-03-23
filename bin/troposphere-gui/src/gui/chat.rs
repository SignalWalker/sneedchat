use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use dioxus::prelude::*;
use futures::StreamExt;
use parking_lot::{Condvar, Mutex, RwLock};
use rexa::{captp::RemoteKey, locator::NodeLocator};
use tokio::net::TcpListener;
use troposphere::{ChannelId, ChatManager, RemotePortal};

use crate::cfg::{Config, WriteError};

pub(crate) enum ManagerCommand {
    OpenPortal { locator: NodeLocator },
    Connect { id: ChannelId },
    Stop,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ChatError {
    #[error("{0}")]
    Inner(String),
    #[error(transparent)]
    Write(#[from] WriteError),
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

async fn manager_loop(
    mut input: UnboundedReceiver<ManagerCommand>,
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
                match TcpListener::bind((*addr, tcp.port)).await {
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
    loop {
        let event = tokio::select! {
            event = manager.recv_event() => event.unwrap(),
            Some(command) = input.next() => match command {
                ManagerCommand::OpenPortal { locator } => {
                    if let Err(error) = manager.layers().request_connect(locator.clone()) {
                        tracing::error!(?locator, %error, "failed to process connect request");
                    }
                    // we'll receive a ChatEvent::SessionStarted when the session's connected
                    continue
                },
                ManagerCommand::Connect { id } => {
                    continue
                },
                ManagerCommand::Stop => {
                    break Ok(())
                }
            }
        };

        match event {
            troposphere::ChatEvent::SessionStarted { session } => {
                let session_key = *session.remote_vkey();
                let portal = match RemotePortal::open_with(
                    &session.into_remote_bootstrap(),
                    manager.signing_key_mut(),
                    b"FIXTHIS",
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
                        continue;
                    }
                };
                portals.insert(session_key, portal.clone());
                portals_signal.write().insert(session_key, portal);
            }
            troposphere::ChatEvent::SessionAborted {
                session_key,
                reason,
            } => {
                if portals.remove(&session_key).is_some() {
                    portals_signal.write().remove(&session_key);
                    tracing::info!(session_key = rexa::hash(&session_key), %reason, "session aborted");
                }
            }
        }
    }
}

pub(super) fn manager_coroutine(
    rx: UnboundedReceiver<ManagerCommand>,
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
