use crate::{manage_session, EventSender, NetworkEvent};
use futures::{FutureExt, TryFutureExt};
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

pub type ConnectResult =
    Result<Arc<dyn AbstractCapTpSession + Send + Sync + 'static>, ConnectError>;
pub type ConnectRequest = (NodeLocator, oneshot::Sender<ConnectResult>);

#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    #[error(transparent)]
    Send(#[from] mpsc::error::SendError<ConnectRequest>),
    #[error(transparent)]
    Recv(#[from] oneshot::error::RecvError),
    #[error("`{0}` is not a registered transport type")]
    UnregisteredTransport(String),
    #[error(transparent)]
    Netlayer(#[from] Box<dyn std::error::Error + Send + Sync + 'static>),
}

pub struct NetlayerManager {
    tasks: JoinSet<Result<(), Box<dyn std::error::Error + Send + Sync + 'static>>>,
    layers: HashMap<String, mpsc::UnboundedSender<ConnectRequest>>,
    locators: HashMap<String, Vec<NodeLocator>>,
}

impl NetlayerManager {
    pub fn new() -> Self {
        Self {
            tasks: JoinSet::new(),
            layers: HashMap::new(),
            locators: HashMap::new(),
        }
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn register<Nl>(
        &mut self,
        transport: String,
        nl: Nl,
        event_pipe: EventSender,
        end_flag: watch::Receiver<bool>,
    ) -> mpsc::UnboundedSender<ConnectRequest>
    where
        Nl: Netlayer + Send + 'static,
        Nl::Reader: AsyncRead + Send + Unpin,
        Nl::Writer: AsyncWrite + Send + Unpin,
        Nl::Error: std::error::Error + Send + Sync + 'static,
    {
        let (sender, receiver) = mpsc::unbounded_channel();
        self.layers.insert(transport.clone(), sender.clone());
        let locators = nl.locators();
        self.tasks
            .spawn(manage_netlayer(nl, event_pipe, end_flag, receiver).map_err(From::from));
        tracing::info!(%transport, ?locators, "registered netlayer");
        self.locators.insert(transport, locators);
        sender
    }

    pub fn request_connect(
        &self,
        locator: NodeLocator,
    ) -> Result<impl Future<Output = ConnectResult>, ConnectError> {
        let Some(nl) = self.layers.get(&locator.transport) else {
            return Err(ConnectError::UnregisteredTransport(locator.transport));
        };
        let (sender, receiver) = oneshot::channel();
        nl.send((locator, sender))?;
        Ok(receiver.map(|res| match res {
            Ok(Ok(ok)) => Ok(ok),
            Ok(Err(err)) => Err(err),
            Err(err) => Err(err.into()),
        }))
    }

    pub fn locators(&self) -> impl Iterator<Item = &NodeLocator> {
        self.locators.values().flatten()
    }
}

impl Default for NetlayerManager {
    fn default() -> Self {
        Self::new()
    }
}

#[tracing::instrument(fields(nl = ?nl.locators()), skip(event_pipe, end_flag, connect_reqs))]
async fn manage_netlayer<Nl: Netlayer>(
    nl: Nl,
    event_pipe: EventSender,
    mut end_flag: tokio::sync::watch::Receiver<bool>,
    mut connect_reqs: mpsc::UnboundedReceiver<ConnectRequest>,
) -> Result<(), Nl::Error>
where
    Nl::Reader: AsyncRead + Unpin + Send + 'static,
    Nl::Writer: AsyncWrite + Unpin + Send + 'static,
    Nl::Error: std::error::Error + Send + Sync + 'static,
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
                        drop(res_pipe.send(Ok(session.as_dyn())));
                        session
                    }
                    Err(error) => {
                        tracing::error!(?error, "failed to connect to node");
                        drop(res_pipe.send(Err(ConnectError::Netlayer(Box::new(error)))));
                        continue
                    }
                };
                tracing::debug!(?locator, ?session, "connected to node");
                session
            },
            _ = end_flag.changed() => break,
        };
        drop(event_pipe.send(NetworkEvent::SessionStarted(session.as_dyn())));
        // let task_name = format!("manage_session: {session:?}");
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
