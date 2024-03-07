use futures::{future::BoxFuture, TryFutureExt};
use rexa::{
    async_compat::{AsyncRead, AsyncWrite},
    captp::{AbstractCapTpSession, BootstrapEvent, CapTpSession, Event},
    netlayer::Netlayer,
};
use rexa::{
    captp::{object::RemoteObject, RecvError},
    locator::NodeLocator,
};
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use syrup::{FromSyrupItem, Item};
use tokio::{
    sync::{mpsc, oneshot, watch, Notify},
    task::JoinSet,
};

pub type EventSender = tokio::sync::mpsc::UnboundedSender<SneedEvent>;
pub type EventReceiver = tokio::sync::mpsc::UnboundedReceiver<SneedEvent>;
pub type Promise<V> = tokio::sync::oneshot::Receiver<V>;
pub type Resolver<V> = tokio::sync::oneshot::Sender<V>;
pub type Swiss = Vec<u8>;
pub type Outboxes = Arc<dashmap::DashMap<uuid::Uuid, Outbox>>;
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

#[derive(Clone)]
pub struct User {
    pub session: Arc<dyn AbstractCapTpSession + Send + Sync + 'static>,
    pub id: uuid::Uuid,
    pub name: String,
}

impl std::fmt::Debug for User {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("User")
            .field("id", &self.id)
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl User {
    pub fn new(
        local_profile: Profile,
        session: Arc<dyn AbstractCapTpSession + Send + Sync + 'static>,
        id: uuid::Uuid,
        ev_pipe: EventSender,
    ) -> (Self, Arc<Channel>) {
        (
            Self {
                session,
                id,
                name: "<Unknown>".into(),
            },
            Arc::new(Channel {
                profile: local_profile,
                id,
                ev_pipe,
                sessions: Arc::default(),
            }),
        )
    }

    // async fn fetch(
    //     &self,
    //     swiss: &[u8],
    // ) -> Result<impl std::future::Future<Output = Result<u64, syrup::Item>>, SendError>
    // where
    //     Writer: AsyncWrite + Unpin,
    // {
    //     self.session
    //         .clone()
    //         .get_remote_bootstrap()
    //         .fetch(swiss)
    //         .await
    // }
}

pub struct Channel {
    profile: Profile,
    id: uuid::Uuid,
    ev_pipe: EventSender,
    sessions:
        Arc<tokio::sync::RwLock<Vec<(u64, Arc<dyn AbstractCapTpSession + Send + Sync + 'static>)>>>,
}

impl std::fmt::Debug for Channel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Channel")
            .field("profile", &self.profile)
            .field("id", &self.id)
            // .field("sessions", &self.sessions)
            .finish_non_exhaustive()
    }
}

impl std::clone::Clone for Channel {
    fn clone(&self) -> Self {
        Self {
            profile: self.profile.clone(),
            id: self.id,
            ev_pipe: self.ev_pipe.clone(),
            sessions: self.sessions.clone(),
        }
    }
}

impl rexa::captp::object::Object for Channel {
    fn deliver_only(
        &self,
        args: Vec<syrup::Item>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
        tracing::debug!("channel::deliver_only");
        let mut args = args.into_iter();
        match args.next() {
            Some(syrup::Item::Symbol(id)) => match id.as_str() {
                "message" => match args.next() {
                    Some(syrup::Item::String(msg)) => {
                        self.ev_pipe.send(SneedEvent::RecvMessage(self.id, msg))?;
                        Ok(())
                    }
                    Some(s) => Err(format!("invalid message text: {s:?}").into()),
                    None => Err("missing message text".into()),
                },
                "set_name" => match args.next() {
                    Some(syrup::Item::String(name)) => {
                        self.ev_pipe.send(SneedEvent::SetName(self.id, name))?;
                        Ok(())
                    }
                    Some(s) => Err(format!("invalid username: {s:?}").into()),
                    None => Err("missing username".into()),
                },
                id => Err(format!("unrecognized channel function: {id}").into()),
            },
            Some(s) => Err(format!("invalid deliver_only argument: {s:?}").into()),
            None => Err("missing deliver_only arguments".into()),
        }
    }

    fn deliver<'s>(
        &'s self,
        args: Vec<syrup::Item>,
        resolver: rexa::captp::GenericResolver,
    ) -> BoxFuture<'s, Result<(), Box<dyn std::error::Error + Send + Sync + 'static>>> {
        use futures::FutureExt;
        async move {
            tracing::debug!("channel::deliver");
            let mut args = args.into_iter();
            match args.next() {
                Some(syrup::Item::Symbol(id)) => match id.as_str() {
                    "get_profile" => resolver
                        .fulfill(
                            [&self.profile],
                            None,
                            rexa::captp::msg::DescImport::Object(0.into()),
                        )
                        .await
                        .map_err(From::from),
                    id => Err(format!("unrecognized channel function: {id}").into()),
                },
                Some(s) => Err(format!("invalid deliver argument: {s:?}").into()),
                None => Err("missing deliver arguments".into()),
            }
        }
        .boxed()
    }
}

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

#[derive(Debug, Clone, syrup::Serialize, syrup::Deserialize)]
pub struct Profile {
    #[syrup(as = SyrupUuid)]
    pub id: uuid::Uuid,
    pub username: String,
}

pub struct Outbox {
    base: RemoteObject,
}

impl std::fmt::Debug for Outbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Outbox").field("base", &self.base).finish()
    }
}

impl Outbox {
    pub fn new(base: RemoteObject) -> Self {
        Self { base }
    }

    #[tracing::instrument(level = "debug", fields(name = name.as_ref()))]
    pub async fn set_username(&self, name: impl AsRef<str>) -> Result<(), rexa::captp::SendError> {
        tracing::debug!("outbox::set_username");
        self.base.call_only("set_name", &[name.as_ref()]).await
    }

    #[tracing::instrument(level = "debug", fields(msg = msg.as_ref()))]
    pub async fn message(&self, msg: impl AsRef<str>) -> Result<(), rexa::captp::SendError> {
        tracing::debug!("outbox::message");
        self.base.call_only("message", &[msg.as_ref()]).await
    }

    #[tracing::instrument(level = "debug")]
    pub async fn get_profile(
        &self,
    ) -> Result<Profile, Box<dyn std::error::Error + Send + Sync + 'static>> {
        tracing::debug!("outbox::get_profile");
        match self
            .base
            .call_and::<&str>("get_profile", &[])
            .await
            .unwrap()
            .await?
        {
            Ok(res) => {
                let mut args = res.into_iter();
                let profile = match args.next() {
                    Some(item) => match Profile::from_syrup_item(item) {
                        Ok(p) => p,
                        Err(_) => todo!(),
                    },
                    _ => todo!(),
                };
                Ok(profile)
            }
            Err(reason) => todo!(),
        }
    }

    // async fn abort(
    //     &self,
    //     reason: impl Into<rexa::captp::msg::OpAbort>,
    // ) -> Result<(), rexa::captp::SendError>
    // where
    //     Writer: AsyncWrite + Unpin,
    // {
    //     tracing::debug!("outbox::abort");
    //     match self.session.clone().abort(reason).await {
    //         Ok(_) => Ok(()),
    //         Err(SendError::SessionAborted(_) | SendError::SessionAbortedLocally) => Ok(()),
    //         Err(e) => Err(e),
    //     }
    // }
}

pub enum SneedEvent {
    // NewConnection(CapTpSession<Reader, Writer>),
    Fetch(
        Arc<dyn AbstractCapTpSession + Send + Sync + 'static>,
        Swiss,
        Resolver<Arc<Channel>>,
    ),
    SetName(uuid::Uuid, String),
    RecvMessage(uuid::Uuid, String),
    SendMessage(String),
    Connect(NodeLocator<String, syrup::Item>),
    /// Emitted on ctrl+c
    Exit,
}

impl std::fmt::Debug for SneedEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fetch(_, arg1, arg2) => f.debug_tuple("Fetch").field(arg1).field(arg2).finish(),
            Self::SetName(arg0, arg1) => f.debug_tuple("SetName").field(arg0).field(arg1).finish(),
            Self::RecvMessage(arg0, arg1) => f
                .debug_tuple("RecvMessage")
                .field(arg0)
                .field(arg1)
                .finish(),
            Self::SendMessage(arg0) => f.debug_tuple("SendMessage").field(arg0).finish(),
            Self::Exit => write!(f, "Exit"),
            Self::Connect(locator) => write!(f, "Connect({locator:?})"),
        }
    }
}

#[tracing::instrument(skip(event_pipe))]
pub async fn manage_session<Reader, Writer>(
    session: CapTpSession<Reader, Writer>,
    event_pipe: EventSender,
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
        registry: &mut HashMap<Swiss, u64>,
        event_pipe: &EventSender,
        session: &CapTpSession<Reader, Writer>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>>
    where
        Reader: AsyncRead + Unpin + Send + 'static,
        Writer: AsyncWrite + Unpin + Send + 'static,
    {
        if let Some(&pos) = registry.get(&swiss) {
            resolver.fulfill(pos).await?;
        } else {
            let (sender, promise) = tokio::sync::oneshot::channel();
            event_pipe.send(SneedEvent::Fetch(session.as_dyn(), swiss.clone(), sender))?;
            tracing::trace!("awaiting promise response");
            let obj = promise.await?;
            let pos = session.export(obj.clone());
            obj.sessions.write().await.push((pos, session.as_dyn()));
            registry.insert(swiss, pos);
            tracing::trace!("fulfilling promise");
            resolver.fulfill(pos).await?;
            // exports.insert(pos, obj);
        }
        Ok(())
    }
    async fn inner<Reader, Writer>(
        session: CapTpSession<Reader, Writer>,
        event_pipe: EventSender,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>>
    where
        Reader: AsyncRead + Send + Unpin + 'static,
        Writer: AsyncWrite + Send + Unpin + 'static,
    {
        let mut registry = HashMap::<Vec<u8>, u64>::new();
        // let mut exports = HashMap::<u64, Arc<Channel<Reader, Writer>>>::new();
        loop {
            tracing::trace!("awaiting captp event");
            let event = match session.recv_event().await {
                Ok(ev) => ev,
                Err(RecvError::SessionAborted(_) | RecvError::SessionAbortedLocally) => {
                    tracing::warn!("unexpected session abort");
                    break Ok(());
                }
                Err(e) => {
                    tracing::error!("{e}");
                    return Err(e.into());
                }
            };
            tracing::debug!(?event, "received captp event");
            match event {
                Event::Bootstrap(BootstrapEvent::Fetch { swiss, resolver }) => {
                    respond_to_fetch(swiss, resolver, &mut registry, &event_pipe, &session).await?
                }
                Event::Abort(reason) => {
                    tracing::info!(reason, "session aborted");
                    break Ok(());
                }
            }
        }
    }
    tracing::debug!("managing session");
    inner(session, event_pipe).await
}

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

pub type ConnectResult = Result<Arc<dyn AbstractCapTpSession + Send + Sync + 'static>, ()>;
pub type ConnectRequest = (
    NodeLocator<String, syrup::Item>,
    oneshot::Sender<ConnectResult>,
);

#[tracing::instrument(fields(nl = %nl.locator::<String, String>()), skip(event_pipe, end_flag, connect_reqs))]
pub async fn manage_netlayer<Nl: Netlayer>(
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
        let task_name = format!("manage_session: {session:?}");
        session_tasks
            .build_task()
            .name(&task_name)
            .spawn(manage_session(session, event_pipe.clone()))
            .unwrap();
    }
    session_tasks.abort_all();
    while let Some(res) = session_tasks.join_next().await {
        if let Err(error) = res {
            tracing::error!("{error}");
        }
    }
    Ok(())
}
#[tracing::instrument(skip(ev_pipe))]
pub fn handle_input(
    ev_pipe: EventSender,
) -> Result<std::thread::JoinHandle<Result<(), std::io::Error>>, std::io::Error> {
    std::thread::Builder::new()
        .name("handle_input".to_string())
        .spawn(move || {
            tracing::debug!("handling input");
            for msg in std::io::stdin().lines() {
                let msg = msg?;
                match msg.as_bytes()[0] {
                    b'/' if msg.len() > 1 => {
                        let mut split = msg[1..].split_whitespace();
                        let cmd = split.next().unwrap();
                        match cmd {
                            "connect" => {
                                let Some(designator) = split.next() else {
                                    tracing::error!("missing designator");
                                    continue;
                                };
                                ev_pipe
                                    .send(SneedEvent::Connect(NodeLocator::new(
                                        designator.to_owned(),
                                        "tcpip".to_owned(),
                                    )))
                                    .unwrap()
                            }
                            _ => tracing::error!(
                                "unrecognized command: {cmd} {}",
                                split.collect::<String>()
                            ),
                        }
                    }
                    _ => ev_pipe.send(SneedEvent::SendMessage(msg)).unwrap(),
                }
            }
            Ok(())
        })
}
#[tracing::instrument(skip(ev_pipe))]
pub async fn signal_watch(
    ev_pipe: EventSender,
    mut end_flag: tokio::sync::watch::Receiver<bool>,
) -> Result<(), std::io::Error> {
    tokio::select! {
        signal = tokio::signal::ctrl_c() => {
            signal?;
            let _ = ev_pipe.send(SneedEvent::Exit);
            Ok(())
        }
        _ = end_flag.changed() => Ok(())
    }
}
#[tracing::instrument(skip(username, outboxes, session))]
pub async fn fetch_outbox(
    username: String,
    local_id: uuid::Uuid,
    remote_id: uuid::Uuid,
    outboxes: Outboxes,
    session: Arc<impl AbstractCapTpSession + ?Sized>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    tracing::debug!("fetching outbox");
    let outbox = match session
        .into_remote_bootstrap()
        .fetch(local_id.as_bytes())
        .await
        .unwrap()
        .await
    {
        Ok(base) => Outbox { base },
        Err(reason) => {
            tracing::error!(?reason, "failed to fetch outbox");
            // TODO
            return Err(syrup::ser::to_pretty(&reason).unwrap().into());
        }
    };
    outbox.set_username(username).await.unwrap();
    tracing::trace!("outbox fetched");
    outboxes.insert(remote_id, outbox);
    Ok(())
}
