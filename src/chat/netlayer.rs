use crate::chat::{manage_session, EventSender, SneedEvent};
use futures::TryFutureExt;
use rexa::{
    async_compat::{AsyncRead, AsyncWrite},
    captp::AbstractCapTpSession,
};
use rexa::{locator::NodeLocator, netlayer::Netlayer};
use std::future::Future;
use std::{collections::HashMap, sync::Arc};
use tokio::{
    sync::{mpsc, oneshot, watch},
    task::JoinSet,
};

pub type ConnectResult = Result<Arc<dyn AbstractCapTpSession + Send + Sync + 'static>, ()>;
pub type ConnectRequest = (
    NodeLocator<String, syrup::Item>,
    oneshot::Sender<ConnectResult>,
);

pub struct NetlayerManager {
    tasks: JoinSet<Result<(), Box<dyn std::error::Error + Send + Sync + 'static>>>,
    layers: HashMap<String, mpsc::UnboundedSender<ConnectRequest>>,
}

impl NetlayerManager {
    pub fn new() -> Self {
        Self {
            tasks: JoinSet::new(),
            layers: HashMap::new(),
        }
    }

    pub fn register<Nl: Netlayer>(
        &mut self,
        transport: String,
        nl: Nl,
        event_pipe: EventSender,
        end_flag: watch::Receiver<bool>,
    ) -> mpsc::UnboundedSender<ConnectRequest>
    where
        Nl: Send + 'static,
        Nl::Reader: AsyncRead + Send + Unpin,
        Nl::Writer: AsyncWrite + Send + Unpin,
        Nl::Error: std::error::Error + Send + Sync + 'static,
    {
        let (sender, receiver) = mpsc::unbounded_channel();
        self.layers.insert(transport.clone(), sender.clone());
        let locator = nl.locator::<String, String>();
        self.tasks
            .spawn(manage_netlayer(nl, event_pipe, end_flag, receiver).map_err(From::from));
        tracing::info!(%transport, %locator, "registered netlayer");
        sender
    }

    pub fn request_connect(
        &self,
        locator: NodeLocator<String, syrup::Item>,
    ) -> Result<
        impl Future<Output = Result<ConnectResult, oneshot::error::RecvError>>,
        mpsc::error::SendError<ConnectRequest>,
    > {
        let Some(nl) = self.layers.get(&locator.transport) else {
            todo!("unregistered transport: {}", locator.transport)
        };
        let (sender, receiver) = oneshot::channel();
        nl.send((locator, sender))?;
        Ok(receiver)
    }
}

#[tracing::instrument(fields(nl = %nl.locator::<String, String>()), skip(event_pipe, end_flag, connect_reqs))]
async fn manage_netlayer<Nl: Netlayer>(
    nl: Nl,
    event_pipe: EventSender,
    mut end_flag: tokio::sync::watch::Receiver<bool>,
    mut connect_reqs: mpsc::UnboundedReceiver<ConnectRequest>,
) -> Result<(), Nl::Error>
where
    Nl::Reader: AsyncRead + Unpin + Send + 'static,
    Nl::Writer: AsyncWrite + Unpin + Send + 'static,
    Nl::Error: std::error::Error,
{
    tracing::debug!("managing netlayer");
    let mut session_tasks = JoinSet::new();
    loop {
        let session = tokio::select! {
            session = nl.accept() => {
                let session = session?;
                tracing::debug!(?session, "accepted connection");
                session
            },
            Some((locator, res_pipe)) = connect_reqs.recv() => {
                let session = match nl.connect(&locator).await {
                    Ok(session) => {
                        let _ = res_pipe.send(Ok(session.as_dyn()));
                        session
                    }
                    Err(error) => {
                        tracing::error!(?error, "failed to connect to node");
                        let _ = res_pipe.send(Err(()));
                        continue
                    }
                };
                tracing::debug!(?locator, ?session, "connected to node");
                session
            },
            _ = end_flag.changed() => break,
        };
        let _ = event_pipe.send(SneedEvent::NewSession(session.as_dyn()));
        let task_name = format!("manage_session: {session:?}");
        session_tasks
            // .build_task()
            // .name(&task_name)
            .spawn(manage_session(
                session,
                event_pipe.clone(),
                end_flag.clone(),
            ));
            // .unwrap();
    }
    session_tasks.abort_all();
    while let Some(res) = session_tasks.join_next().await {
        if let Err(error) = res {
            tracing::error!("{error}");
        }
    }
    Ok(())
}
