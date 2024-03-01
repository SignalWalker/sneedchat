pub mod chat;
mod cli;

mod netlayer;
pub use netlayer::*;

// mod gui;

// #[cfg(feature = "iced")]
// fn main() -> iced::Result {
//     use iced::Application;
//
//     let args = <cli::Cli as clap::Parser>::parse();
//     cli::initialize_tracing(args.log_filter, args.log_format);
//     gui::SneedChat::run(iced::Settings {
//         window: iced::window::Settings {
//             size: iced::Size {
//                 width: 640.0,
//                 height: 480.0,
//             },
//             ..Default::default()
//         },
//         ..Default::default()
//     })
// }

#[cfg(feature = "egui")]
fn main() -> eframe::Result<()> {
    let args = <cli::Cli as clap::Parser>::parse();
    cli::initialize_tracing(args.log_filter, args.log_format);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([640.0, 480.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "SneedChat",
        options,
        Box::new(|cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            Box::<SneedChat>::default()
        }),
    )
}

// #[cfg(all(not(feature = "iced"), not(feature = "egui")))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use rexa::captp::{RecvError, SendError};
    use rexa::{
        async_compat::{AsyncRead, AsyncWrite},
        captp::{msg::DescImport, BootstrapEvent, CapTpSession, Delivery, Event},
        netlayer::Netlayer,
    };
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::Notify;
    type EventSender<Reader, Writer> =
        tokio::sync::mpsc::UnboundedSender<SneedEvent<Reader, Writer>>;
    type EventReceiver<Reader, Writer> =
        tokio::sync::mpsc::UnboundedReceiver<SneedEvent<Reader, Writer>>;
    type Promise<V> = tokio::sync::oneshot::Receiver<V>;
    type Resolver<V> = tokio::sync::oneshot::Sender<V>;
    type Swiss = Vec<u8>;
    type Outboxes<Reader, Writer> = Arc<dashmap::DashMap<uuid::Uuid, Outbox<Reader, Writer>>>;
    type TcpIpSession = CapTpSession<
        <netlayer::TcpIpNetlayer as rexa::netlayer::Netlayer>::Reader,
        <netlayer::TcpIpNetlayer as rexa::netlayer::Netlayer>::Writer,
    >;
    type TcpIpSneedEvent = SneedEvent<
        <netlayer::TcpIpNetlayer as rexa::netlayer::Netlayer>::Reader,
        <netlayer::TcpIpNetlayer as rexa::netlayer::Netlayer>::Writer,
    >;
    type TcpIpUser = User<
        <netlayer::TcpIpNetlayer as rexa::netlayer::Netlayer>::Reader,
        <netlayer::TcpIpNetlayer as rexa::netlayer::Netlayer>::Writer,
    >;
    type TcpIpChannel = Channel<
        <netlayer::TcpIpNetlayer as rexa::netlayer::Netlayer>::Reader,
        <netlayer::TcpIpNetlayer as rexa::netlayer::Netlayer>::Writer,
    >;

    #[derive(Clone)]
    struct User<Reader, Writer> {
        session: CapTpSession<Reader, Writer>,
        id: uuid::Uuid,
        name: String,
    }

    impl<Reader, Writer> std::fmt::Debug for User<Reader, Writer> {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("User")
                .field("session", &self.session)
                .field("id", &self.id)
                .field("name", &self.name)
                .finish()
        }
    }

    impl<Reader, Writer> User<Reader, Writer> {
        fn new(
            session: CapTpSession<Reader, Writer>,
            id: uuid::Uuid,
            ev_pipe: EventSender<Reader, Writer>,
        ) -> (Self, Arc<Channel<Reader, Writer>>) {
            (
                Self {
                    session,
                    id,
                    name: "<Unknown>".into(),
                },
                Arc::new(Channel {
                    id,
                    ev_pipe,
                    sessions: Arc::default(),
                }),
            )
        }

        async fn fetch(
            &self,
            swiss: &[u8],
        ) -> Result<impl std::future::Future<Output = Result<u64, syrup::Item>>, SendError>
        where
            Writer: AsyncWrite + Unpin,
        {
            self.session
                .clone()
                .get_remote_bootstrap()
                .fetch(swiss)
                .await
        }
    }

    struct Channel<Reader, Writer> {
        id: uuid::Uuid,
        ev_pipe: EventSender<Reader, Writer>,
        sessions: Arc<tokio::sync::RwLock<Vec<(u64, CapTpSession<Reader, Writer>)>>>,
    }

    impl<Reader, Writer> std::fmt::Debug for Channel<Reader, Writer> {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Channel")
                .field("id", &self.id)
                .field("sessions", &self.sessions)
                .finish_non_exhaustive()
        }
    }

    impl<Reader, Writer> std::clone::Clone for Channel<Reader, Writer> {
        fn clone(&self) -> Self {
            Self {
                id: self.id,
                ev_pipe: self.ev_pipe.clone(),
                sessions: self.sessions.clone(),
            }
        }
    }

    impl<Reader, Writer> rexa::captp::object::Object for Channel<Reader, Writer>
    where
        Reader: 'static,
        Writer: 'static,
    {
        fn deliver_only(&self, args: Vec<syrup::Item>) {
            if let Err(e) = self.deliver_only(args) {
                tracing::error!(id = ?self.id, "{e}");
            }
        }

        fn deliver(&self, args: Vec<syrup::Item>, resolver: rexa::captp::GenericResolver) {
            if let Err(e) = self.deliver(args, resolver) {
                tracing::error!(id = ?self.id, "{e}")
            }
        }
    }

    impl<Reader, Writer> Channel<Reader, Writer> {
        #[tracing::instrument(level = tracing::Level::DEBUG)]
        fn deliver_only(&self, args: Vec<syrup::Item>) -> Result<(), Box<dyn std::error::Error>>
        where
            Reader: 'static,
            Writer: 'static,
        {
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

        // #[tracing::instrument(level = tracing::Level::DEBUG)]
        fn deliver(
            &self,
            args: Vec<syrup::Item>,
            resolver: rexa::captp::GenericResolver,
        ) -> Result<(), Box<dyn std::error::Error>> {
            tracing::debug!("channel::deliver");
            let mut args = args.into_iter();
            match args.next() {
                Some(syrup::Item::Symbol(id)) => {
                    let id = id.as_str();
                    Err(format!("unrecognized channel function: {id}").into())
                }
                Some(s) => Err(format!("invalid deliver argument: {s:?}").into()),
                None => Err("missing deliver arguments".into()),
            }
        }
    }

    struct Outbox<Reader, Writer> {
        position: u64,
        session: CapTpSession<Reader, Writer>,
    }

    impl<Reader, Writer> std::fmt::Debug for Outbox<Reader, Writer> {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Outbox")
                .field("position", &self.position)
                .field("session", &self.session)
                .finish()
        }
    }

    impl<Reader, Writer> Outbox<Reader, Writer> {
        #[tracing::instrument(level = "debug", fields(name = name.as_ref()))]
        async fn set_username(&self, name: impl AsRef<str>) -> Result<(), rexa::captp::SendError>
        where
            Writer: AsyncWrite + Unpin,
        {
            tracing::debug!("outbox::set_username");
            self.session
                .deliver_only(
                    self.position,
                    syrup::raw_syrup_unwrap![&syrup::Symbol("set_name"), &name.as_ref()],
                )
                .await
        }

        #[tracing::instrument(level = "debug", fields(msg = msg.as_ref()))]
        async fn message(&self, msg: impl AsRef<str>) -> Result<(), rexa::captp::SendError>
        where
            Writer: AsyncWrite + Unpin,
        {
            tracing::debug!("outbox::message");
            self.session
                .deliver_only(
                    self.position,
                    syrup::raw_syrup_unwrap![&syrup::Symbol("message"), &msg.as_ref()],
                )
                .await
        }

        async fn abort(
            &self,
            reason: impl Into<rexa::captp::msg::OpAbort>,
        ) -> Result<(), rexa::captp::SendError>
        where
            Writer: AsyncWrite + Unpin,
        {
            tracing::debug!("outbox::abort");
            match self.session.clone().abort(reason).await {
                Ok(_) => Ok(()),
                Err(SendError::SessionAborted(_) | SendError::SessionAbortedLocally) => Ok(()),
                Err(e) => Err(e),
            }
        }
    }

    enum SneedEvent<Reader, Writer> {
        // NewConnection(CapTpSession<Reader, Writer>),
        Fetch(
            CapTpSession<Reader, Writer>,
            Swiss,
            Resolver<Arc<Channel<Reader, Writer>>>,
        ),
        SetName(uuid::Uuid, String),
        RecvMessage(uuid::Uuid, String),
        SendMessage(String),
        /// Emitted on ctrl+c
        Exit,
    }

    impl<Reader, Writer> std::fmt::Debug for SneedEvent<Reader, Writer> {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::Fetch(_, _, _) => f.debug_tuple("Fetch").finish(),
                Self::SetName(arg0, arg1) => {
                    f.debug_tuple("SetName").field(arg0).field(arg1).finish()
                }
                Self::RecvMessage(arg0, arg1) => f
                    .debug_tuple("RecvMessage")
                    .field(arg0)
                    .field(arg1)
                    .finish(),
                Self::SendMessage(arg0) => f.debug_tuple("SendMessage").field(arg0).finish(),
                Self::Exit => f.debug_struct("Exit").finish(),
            }
        }
    }

    #[tracing::instrument(skip(event_pipe))]
    async fn manage_session<Reader, Writer>(
        session: CapTpSession<Reader, Writer>,
        event_pipe: EventSender<Reader, Writer>,
    ) where
        Reader: AsyncRead + Unpin + Send + 'static,
        Writer: AsyncWrite + Unpin + Send + 'static,
    {
        use rexa::captp::FetchResolver;
        #[tracing::instrument(skip(resolver, registry, event_pipe), fields(swiss = rexa::hash(&swiss)))]
        async fn respond_to_fetch<Reader, Writer>(
            swiss: Swiss,
            resolver: FetchResolver,
            registry: &mut HashMap<Swiss, u64>,
            event_pipe: &EventSender<Reader, Writer>,
            session: &CapTpSession<Reader, Writer>,
        ) -> Result<(), Box<dyn std::error::Error>>
        where
            Reader: AsyncRead + Unpin + Send + 'static,
            Writer: AsyncWrite + Unpin + Send + 'static,
        {
            if let Some(&pos) = registry.get(&swiss) {
                resolver.fulfill(pos).await?;
            } else {
                let (sender, promise) = tokio::sync::oneshot::channel();
                event_pipe.send(SneedEvent::Fetch(session.clone(), swiss.clone(), sender))?;
                tracing::trace!("awaiting promise response");
                let obj = promise.await?;
                let pos = session.export(obj.clone());
                obj.sessions.write().await.push((pos, session.clone()));
                registry.insert(swiss, pos);
                tracing::trace!("fulfilling promise");
                resolver.fulfill(pos).await?;
                // exports.insert(pos, obj);
            }
            Ok(())
        }
        async fn inner<Reader, Writer>(
            session: CapTpSession<Reader, Writer>,
            event_pipe: EventSender<Reader, Writer>,
        ) -> Result<(), Box<dyn std::error::Error>>
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
                        respond_to_fetch(swiss, resolver, &mut registry, &event_pipe, &session)
                            .await?
                    }
                    Event::Abort(reason) => {
                        tracing::info!(reason, "session aborted");
                        break Ok(());
                    }
                }
            }
        }
        tracing::debug!("managing session");
        if let Err(error) = inner(session, event_pipe).await {
            tracing::error!(error);
        }
    }
    #[tracing::instrument(skip(event_pipe, nl))]
    async fn accept_sessions<Nl: Netlayer>(
        nl: Nl,
        event_pipe: EventSender<Nl::Reader, Nl::Writer>,
        mut session_tasks: tokio::task::JoinSet<()>,
        mut end_flag: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), Nl::Error>
    where
        Nl::Reader: AsyncRead + Unpin + Send + 'static,
        Nl::Writer: AsyncWrite + Unpin + Send + 'static,
    {
        tracing::debug!("accepting sessions");
        loop {
            let session = tokio::select! {
                session = nl.accept() => session?,
                _ = end_flag.changed() => break
            };
            tracing::debug!(?session, "accepted connection");
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
    fn handle_input<Reader, Writer>(
        ev_pipe: EventSender<Reader, Writer>,
    ) -> Result<std::thread::JoinHandle<Result<(), std::io::Error>>, std::io::Error>
    where
        Reader: Send + 'static,
        Writer: Send + 'static,
    {
        std::thread::Builder::new()
            .name("handle_input".to_string())
            .spawn(move || {
                tracing::debug!("handling input");
                for msg in std::io::stdin().lines() {
                    ev_pipe.send(SneedEvent::SendMessage(msg?)).unwrap();
                }
                Ok(())
            })
    }
    #[tracing::instrument(skip(ev_pipe))]
    async fn signal_watch<Reader, Writer>(
        ev_pipe: EventSender<Reader, Writer>,
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
    #[tracing::instrument(skip(username, outboxes))]
    async fn fetch_outbox<Reader, Writer>(
        username: String,
        local_id: uuid::Uuid,
        remote_id: uuid::Uuid,
        outboxes: Outboxes<Reader, Writer>,
        session: CapTpSession<Reader, Writer>,
    ) where
        Writer: AsyncWrite + Unpin,
    {
        tracing::debug!("fetching outbox");
        let outbox = match session
            .clone()
            .get_remote_bootstrap()
            .fetch(local_id.as_bytes())
            .await
            .unwrap()
            .await
        {
            Ok(position) => Outbox { position, session },
            Err(reason) => {
                tracing::error!(?reason, "failed to fetch outbox");
                // TODO
                return;
            }
        };
        outbox.set_username(username).await.unwrap();
        tracing::trace!("outbox fetched");
        outboxes.insert(remote_id, outbox);
    }

    let args = <cli::Cli as clap::Parser>::parse();
    cli::initialize_tracing(args.log_filter, args.log_format);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .unhandled_panic(tokio::runtime::UnhandledPanic::ShutdownRuntime)
        .enable_all()
        .build()?;

    let local_id = uuid::Uuid::new_v4();
    let username = args.username.unwrap_or_else(|| local_id.to_string());

    let (ev_sender, ev_receiver) = tokio::sync::mpsc::unbounded_channel();

    let (end_notifier, mut end_flag) = tokio::sync::watch::channel(false);
    end_flag.mark_unchanged();

    rt.block_on(async move {
        let netlayer: TcpIpNetlayer = TcpIpNetlayer::bind(("0.0.0.0", args.port)).await?;

        let mut main_tasks = tokio::task::JoinSet::new();

        let mut session_tasks = tokio::task::JoinSet::new();
        let mut initial = None;
        let mut initial_outbox_pos = None;

        if let Some(remote) = args.remote {
            tracing::debug!(addr = ?remote, "connecting to initial remote");
            let session = netlayer
                .connect(rexa::locator::NodeLocator::<String, String>::new(
                    remote.to_string(),
                    "tcpip".to_owned(),
                ))
                .await?;

            tracing::debug!(?session, "fetching initial remote outbox");
            initial_outbox_pos = Some(
                session
                    .clone()
                    .get_remote_bootstrap()
                    .fetch(local_id.as_bytes())
                    .await
                    .unwrap(),
            );

            initial = Some(session.clone());

            let task_name = format!("manage_session: {session:?}");
            session_tasks
                .build_task()
                .name(&task_name)
                .spawn(manage_session(session, ev_sender.clone()))?;
        }

        tracing::info!(locator = ?netlayer.locator::<String, String>().unwrap());

        main_tasks
            .build_task()
            .name("accept_sessions")
            .spawn(accept_sessions(
                netlayer,
                ev_sender.clone(),
                session_tasks,
                end_flag.clone(),
            ))?;

        let input_task = handle_input(ev_sender.clone())?;

        main_tasks
            .build_task()
            .name("signal_watch")
            .spawn(signal_watch(ev_sender.clone(), end_flag.clone()))?;

        tracing::info!("ready!");

        tokio::task::Builder::new()
            .name("main_loop")
            .spawn(main_loop_wrapper(
                ev_receiver,
                ev_sender,
                initial,
                initial_outbox_pos,
                username,
                local_id,
            ))?
            .await?;

        tracing::info!("exited main loop, awaiting subtasks...");

        end_notifier.send(true)?;
        while let Some(res) = main_tasks.join_next().await {
            if let Err(error) = res {
                tracing::error!("{error}");
            }
        }

        if input_task.is_finished() {
            match input_task.join() {
                Ok(Err(inner)) => {
                    tracing::error!(name: "sneedchat::handle_input::inner", "{inner}")
                }
                Err(outer) => tracing::error!(name: "sneedchat::handle_input", "{outer:?}"),
                _ => {}
            }
        }

        tracing::info!("exiting sneedchat...");

        async fn main_loop_wrapper(
            ev_receiver: tokio::sync::mpsc::UnboundedReceiver<TcpIpSneedEvent>,
            ev_sender: tokio::sync::mpsc::UnboundedSender<TcpIpSneedEvent>,
            initial: Option<TcpIpSession>,
            initial_outbox_pos: Option<
                impl std::future::Future<Output = Result<u64, syrup::Item>> + Send + 'static,
            >,
            username: String,
            local_id: uuid::Uuid,
        ) {
            if let Err(e) = main_loop(
                ev_receiver,
                ev_sender,
                initial,
                initial_outbox_pos,
                username,
                local_id,
            )
            .await
            {
                tracing::error!("{e}");
            }
        }

        async fn main_loop(
            mut ev_receiver: tokio::sync::mpsc::UnboundedReceiver<TcpIpSneedEvent>,
            ev_sender: tokio::sync::mpsc::UnboundedSender<TcpIpSneedEvent>,
            initial: Option<TcpIpSession>,
            mut initial_outbox_pos: Option<
                impl std::future::Future<Output = Result<u64, syrup::Item>> + Send + 'static,
            >,
            username: String,
            local_id: uuid::Uuid,
        ) -> Result<(), Box<dyn std::error::Error>> {
            let time_offset = time::UtcOffset::current_local_offset().unwrap_or_else(|e| {
                tracing::warn!(reason = %e, "couldn't get local time offset");
                time::UtcOffset::UTC
            });
            let time_format = time::macros::format_description!("[hour]:[minute]:[second]");

            let mut tasks = tokio::task::JoinSet::new();

            let mut users = HashMap::<uuid::Uuid, TcpIpUser>::new();
            let mut channels = HashMap::<uuid::Uuid, Arc<TcpIpChannel>>::new();
            let outboxes = Outboxes::default();
            loop {
                tracing::trace!("awaiting chat event");
                let Some(event) = ev_receiver.recv().await else {
                    tracing::error!("ev_receiver broken");
                    break;
                };
                tracing::debug!(?event, "received event");
                match event {
                    SneedEvent::Fetch(session, swiss, resolver) => {
                        let id = uuid::Uuid::from_slice(&swiss)?;
                        let (user, channel) = User::new(session.clone(), id, ev_sender.clone());
                        users.insert(id, user);
                        channels.insert(id, channel.clone());
                        resolver.send(channel).unwrap();
                        if !outboxes.contains_key(&id) {
                            match &initial {
                                Some(initial)
                                    if initial_outbox_pos.is_some() && session == *initial =>
                                {
                                    async fn get_initial_remote<Reader, Writer>(
                                        username: String,
                                        outboxes: Outboxes<Reader, Writer>,
                                        outbox_pos: impl std::future::Future<
                                            Output = Result<u64, syrup::Item>,
                                        >,
                                        session: CapTpSession<Reader, Writer>,
                                        id: uuid::Uuid,
                                    ) where
                                        Writer: AsyncWrite + Unpin,
                                    {
                                        tracing::debug!("awaiting initial remote outbox position");
                                        let outbox = match outbox_pos.await {
                                            Ok(position) => Outbox { position, session },
                                            Err(error) => {
                                                tracing::error!(
                                                    ?error,
                                                    "failed to fetch initial remote outbox"
                                                );
                                                return;
                                            }
                                        };
                                        tracing::trace!(
                                            "fetched initial remote outbox, sending username..."
                                        );
                                        if let Err(error) = outbox.set_username(username).await {
                                            tracing::error!(
                                                ?error,
                                                "failed to set username on initial remote"
                                            );
                                            return;
                                        }
                                        tracing::trace!("sent username, inserting outbox...");
                                        outboxes.insert(id, outbox);
                                        tracing::trace!("outbox inserted");
                                    }
                                    tracing::debug!(?session, "received fetch from initial remote");

                                    tasks.build_task().name("get_initial_remote").spawn(
                                        get_initial_remote(
                                            username.clone(),
                                            outboxes.clone(),
                                            initial_outbox_pos.take().unwrap(),
                                            session,
                                            id,
                                        ),
                                    )?;
                                }
                                _ => {
                                    tasks.build_task().name("fetch_outbox").spawn(fetch_outbox(
                                        username.clone(),
                                        local_id,
                                        id,
                                        outboxes.clone(),
                                        session,
                                    ))?;
                                }
                            }
                        }
                    }
                    // SneedEvent::NewOutbox() => {
                    // }
                    SneedEvent::RecvMessage(channel_id, msg) => {
                        let user = users.get(&channel_id).unwrap();
                        let time = time::OffsetDateTime::now_utc()
                            .to_offset(time_offset)
                            .format(&time_format)
                            .unwrap();
                        println!("[{time}][{}] {msg}", user.name);
                    }
                    SneedEvent::SetName(channel_id, name) => {
                        tracing::trace!(%name, %channel_id, "update username");
                        users.get_mut(&channel_id).unwrap().name = name;
                    }
                    SneedEvent::SendMessage(msg) => {
                        for entry in outboxes.iter() {
                            let outbox = entry.value();
                            outbox.message(&msg).await?;
                        }
                        let time = time::OffsetDateTime::now_utc()
                            .to_offset(time_offset)
                            .format(&time_format)
                            .unwrap();
                        println!("[{time}][{username}] {msg}");
                    }
                    SneedEvent::Exit => {
                        for entry in outboxes.iter() {
                            let outbox = entry.value();
                            outbox.abort("quitting").await?;
                        }
                        tracing::debug!("received exit request");
                        break;
                    }
                }
            }
            tracing::trace!("ending main loop...");
            tasks.abort_all();
            while let Some(res) = tasks.join_next().await {
                res?;
            }
            Ok(())
        }

        Ok(())
    })
}
