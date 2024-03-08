use crate::chat::{EventSender, SneedEvent, Swiss};
use dashmap::DashMap;
use rexa::captp::{BootstrapEvent, CapTpSession, Event};
use rexa::{
    async_compat::{AsyncRead, AsyncWrite},
    captp::RecvError,
};
use std::{collections::HashMap, sync::Arc};
use tokio::{sync::watch, task::JoinSet};

#[tracing::instrument(skip(event_pipe))]
pub async fn manage_session<Reader, Writer>(
    session: CapTpSession<Reader, Writer>,
    event_pipe: EventSender,
    mut end_flag: watch::Receiver<bool>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>>
where
    Reader: AsyncRead + Unpin + Send + 'static,
    Writer: AsyncWrite + Unpin + Send + 'static,
{
    use rexa::captp::FetchResolver;
    #[tracing::instrument(skip(resolver, registry, event_pipe), fields(swiss = rexa::hash(&swiss)))]
    async fn respond_to_fetch<Reader, Writer>(
        swiss: Swiss,
        resolver: FetchResolver,
        registry: Arc<DashMap<Swiss, u64>>,
        event_pipe: EventSender,
        session: CapTpSession<Reader, Writer>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>>
    where
        Reader: AsyncRead + Unpin + Send + 'static,
        Writer: AsyncWrite + Unpin + Send + 'static,
    {
        if let Some(pos) = registry.get(&swiss) {
            resolver.fulfill(*pos).await?;
        } else {
            let (sender, promise) = tokio::sync::oneshot::channel();
            event_pipe.send(SneedEvent::Fetch {
                session_key: *session.remote_vkey(),
                swiss: swiss.clone(),
                resolver: sender,
            })?;
            tracing::trace!("awaiting promise response");
            match promise.await? {
                Ok(obj) => {
                    let pos = session.export(obj);
                    registry.insert(swiss, pos);
                    tracing::trace!("fulfilling promise");
                    resolver.fulfill(pos).await?;
                }
                Err(reason) => {
                    tracing::trace!("breaking promise");
                    resolver.break_promise(reason).await?;
                }
            }
        }
        Ok(())
    }
    tracing::debug!("managing session");
    let registry = Arc::new(DashMap::<Vec<u8>, u64>::new());
    let mut tasks = JoinSet::new();
    // let mut exports = HashMap::<u64, Arc<Channel<Reader, Writer>>>::new();
    let res = loop {
        tracing::trace!("awaiting captp event");
        let ev_res = tokio::select! {
            ev_res = session.recv_event() => ev_res,
            _ = end_flag.changed() => if *end_flag.borrow() {
                if !session.is_aborted() {
                    tokio::spawn(async move { session.abort("quitting").await });
                }
                break Ok(())
            } else {
                continue
            }
        };
        let event = match ev_res {
            Ok(ev) => ev,
            Err(RecvError::SessionAborted(_) | RecvError::SessionAbortedLocally) => {
                tracing::warn!("unexpected session abort");
                break Ok(());
            }
            Err(error) => {
                tracing::error!(%error, "failed to receive captp event");
                break Err(error.into());
            }
        };
        tracing::debug!(?event, "received captp event");
        match event {
            Event::Bootstrap(BootstrapEvent::Fetch { swiss, resolver }) => {
                tasks.spawn(respond_to_fetch(
                    swiss,
                    resolver,
                    registry.clone(),
                    event_pipe.clone(),
                    session.clone(),
                ));
            }
            Event::Abort(reason) => {
                event_pipe.send(SneedEvent::SessionAborted {
                    session_key: *session.remote_vkey(),
                    reason,
                })?;
                break Ok(());
            }
        }
    };
    tasks.abort_all();
    while let Some(res) = tasks.join_next().await {
        match res {
            Ok(Err(error)) => {
                tracing::error!(%error, "subtask error");
            }
            Err(error) => {
                tracing::error!(%error, "subtask join error");
            }
            _ => {}
        }
    }
    res
}
