use std::{collections::HashMap, sync::Arc};

use bonfire::{ChatManager, RemotePortal};
use dioxus::{
    hooks::UnboundedReceiver,
    signals::{GlobalSignal, Signal},
};
use futures::StreamExt;
use rexa::captp::RemoteKey;

use crate::cfg::Config;

pub(super) static REMOTE_PORTALS: GlobalSignal<HashMap<RemoteKey, Arc<RemotePortal>>> =
    Signal::global(Default::default);

#[tracing::instrument(level = tracing::Level::DEBUG)]
async fn manager_loop(
    cfg: Arc<Config>,
    mut input: UnboundedReceiver<()>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    tracing::trace!("initializing manager...");
    let mut manager = {
        let mut builder = ChatManager::builder(cfg.get_key_or_init()?)
            .with_username(cfg.profile.username.clone());

        #[cfg(not(target_family = "wasm"))]
        {
            use rexa_netlayers::datastream::TcpIpNetlayer;
            builder = builder.with_netlayer(
                "tcpip".to_owned(),
                TcpIpNetlayer::bind(&cfg.netlayers.tcpip.socket_addr()).await?,
            );
        }

        builder.build()
    };
    tracing::info!("starting chat manager loop");
    let mut portals = HashMap::new();
    loop {
        let event = tokio::select! {
            event = manager.recv_event() => event?,
                _ = input.next() => break
        };

        match event {
            bonfire::ChatEvent::SessionStarted { session } => {
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
                REMOTE_PORTALS.write().insert(session_key, portal);
            }
            bonfire::ChatEvent::SessionAborted {
                session_key,
                reason,
            } => todo!(),
        }
    }
    Ok(())
}

pub(super) async fn manager_coroutine(cfg: Arc<Config>, input: UnboundedReceiver<()>) {
    if let Err(error) = manager_loop(cfg, input).await {
        tracing::error!("{error}");
    }
}
