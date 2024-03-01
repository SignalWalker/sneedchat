use ed25519_dalek::VerifyingKey;
// use iced::Subscription;
use rexa::{
    captp::{
        msg::{DescImport, DescImportObject, OpAbort, OpDeliver, OpDeliverOnly, Operation},
        CapTpSession, CapTpSessionManager, Delivery, SwissRegistry,
    },
    locator::NodeLocator,
    netlayer::Netlayer,
};
use std::{
    hash::Hash,
    net::SocketAddr,
    sync::{atomic::AtomicU64, Arc},
};
use tokio::{
    net::{
        tcp::{OwnedReadHalf, OwnedWriteHalf},
        TcpListener, TcpStream,
    },
    sync::Mutex,
};

// use crate::{chat::SessionManager, gui::MessageTask};

pub struct TcpIpNetlayer {
    listener: TcpListener,
    manager: Mutex<CapTpSessionManager<OwnedReadHalf, OwnedWriteHalf>>,
}

impl std::fmt::Debug for TcpIpNetlayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TcpIpNetlayer")
            .field(
                "locator",
                &syrup::ser::to_pretty(&self.locator::<String, String>().unwrap()),
            )
            .finish_non_exhaustive()
    }
}

impl rexa::netlayer::Netlayer for TcpIpNetlayer {
    type Reader = tokio::net::tcp::OwnedReadHalf;
    type Writer = tokio::net::tcp::OwnedWriteHalf;
    type Error = std::io::Error;

    #[tracing::instrument(fields(locator = syrup::ser::to_pretty(&locator).unwrap()))]
    async fn connect<HintKey: syrup::Serialize, HintValue: syrup::Serialize>(
        &self,
        locator: rexa::locator::NodeLocator<HintKey, HintValue>,
    ) -> Result<rexa::captp::CapTpSession<Self::Reader, Self::Writer>, Self::Error> {
        let addr = locator.designator.parse::<SocketAddr>();
        let (reader, writer) = TcpStream::connect(addr.unwrap()).await?.into_split();
        self.manager
            .lock()
            .await
            .init_session(reader, writer)
            .and_connect(self.locator::<String, String>()?)
            .await
    }

    #[tracing::instrument()]
    async fn accept(
        &self,
    ) -> Result<rexa::captp::CapTpSession<Self::Reader, Self::Writer>, Self::Error> {
        let (reader, writer) = self.listener.accept().await?.0.into_split();
        self.manager
            .lock()
            .await
            .init_session(reader, writer)
            .and_accept(self.locator::<String, String>()?)
            .await
    }
}

impl TcpIpNetlayer {
    pub fn events(
        &self,
    ) -> impl futures::stream::Stream<Item = Result<rexa::captp::Event, rexa::captp::RecvError>> + '_
    {
        use futures::stream::{Stream, StreamExt};
        async fn filter<Reader, Writer, Error>(
            acc_res: Result<CapTpSession<Reader, Writer>, Error>,
        ) -> Option<impl Stream<Item = Result<rexa::captp::Event, rexa::captp::RecvError>>>
        where
            Writer: rexa::async_compat::AsyncWrite + Send + Unpin + 'static,
            Reader: rexa::async_compat::AsyncRead + Send + Unpin + 'static,
        {
            match acc_res {
                Ok(session) => Some(session.into_event_stream()),
                Err(_) => None,
            }
        }
        let stream = self.stream();
        let stream = stream.filter_map(filter);
        stream.flatten_unordered(None)
    }

    pub fn local_addr(&self) -> Result<std::net::SocketAddr, std::io::Error> {
        self.listener.local_addr()
    }

    pub fn locator<HKey, HVal>(&self) -> Result<NodeLocator<HKey, HVal>, std::io::Error> {
        Ok(NodeLocator::new(
            self.local_addr()?.to_string(),
            "tcpip".to_owned(),
        ))
    }

    pub async fn bind(addrs: impl tokio::net::ToSocketAddrs) -> Result<Self, std::io::Error> {
        Ok(Self {
            listener: TcpListener::bind(addrs).await?,
            manager: Mutex::new(CapTpSessionManager::new()),
        })
    }
}

// impl TcpIpNetlayer<TcpListener, TcpStream> {
// pub fn subscription(
//     self: std::sync::Arc<Self>,
// ) -> Result<iced::Subscription<Result<CapTpSession<TcpStream>, std::io::Error>>, std::io::Error>
// {
//     async fn accept(
//         nl: std::sync::Arc<TcpIpNetlayer<TcpListener, TcpStream>>,
//     ) -> (
//         Result<CapTpSession<TcpStream>, std::io::Error>,
//         std::sync::Arc<TcpIpNetlayer<TcpListener, TcpStream>>,
//     ) {
//         (nl.accept().await, nl)
//     }
//     Ok(iced::subscription::unfold(self.local_addr()?, self, accept))
// }

// pub fn event_subscription(
//     self: Arc<Self>,
//     manager: Arc<crate::chat::SessionManager>,
//     // registry: Arc<SwissRegistry<<Self as Netlayer>::Socket>>, // chat: Arc<crate::chat::ChatManager>,
//     // exports: Arc<ExportManager<TcpPipe>>,
// ) -> Subscription<rexa::captp::Event> {
//     struct Recipe {
//         nl: Arc<TcpIpNetlayer<TcpListener, TcpStream>>,
//     }
//     impl iced::advanced::subscription::Recipe for Recipe {
//         type Output = rexa::captp::Event;
//
//         fn hash(&self, state: &mut iced::advanced::Hasher) {
//             std::any::TypeId::of::<Self>().hash(state);
//         }
//
//         fn stream(
//             self: Box<Self>,
//             _: iced::advanced::subscription::EventStream,
//         ) -> iced::advanced::graphics::futures::BoxStream<Self::Output> {
//             use futures::StreamExt;
//             self.nl.events().boxed()
//         }
//     }
//     Subscription::from_recipe(Recipe { nl: self })
// }
// }

pub struct CapTpRecipe<Nl: Netlayer> {
    netlayer: Arc<Nl>,
    manager: Arc<crate::chat::SessionManager>,
    // chat: Arc<crate::chat::ChatManager>,
    // registry: Arc<SwissRegistry<Nl::Socket>>,
    // exports: Arc<ExportManager<TcpPipe>>,
}

impl<Nl: Netlayer> std::clone::Clone for CapTpRecipe<Nl> {
    fn clone(&self) -> Self {
        Self {
            netlayer: self.netlayer.clone(),
            manager: self.manager.clone(),
            // chat: self.chat.clone(),
            // registry: self.registry.clone(),
            // exports: self.exports.clone(),
        }
    }
}

// #[derive(Debug)]
// pub struct ExportManager<Value> {
//     registry: Arc<SwissRegistry<Arc<Value>>>,
//     map: dashmap::DashMap<u64, Arc<Value>>,
//     swiss_map: dashmap::DashMap<Vec<u8>, u64>,
//     current_key: AtomicU64,
// }
//
// impl<V> ExportManager<V> {
//     pub fn new(registry: Arc<SwissRegistry<Arc<V>>>) -> Self {
//         Self {
//             registry,
//             map: Default::default(),
//             swiss_map: Default::default(),
//             current_key: 1.into(), // skip over 0 bc that's the bootstrap key
//         }
//     }
//     fn next_key(&self) -> u64 {
//         self.current_key
//             .fetch_add(1, std::sync::atomic::Ordering::AcqRel)
//     }
//     fn fetch_exported(&self, swiss: &[u8]) -> Option<(u64, Arc<V>)> {
//         self.swiss_map
//             .get(swiss)
//             .and_then(|pos| self.get(&*pos).map(|v| (*pos, v.clone())))
//     }
//     pub fn fetch(&self, swiss: &[u8]) -> Option<(u64, Arc<V>)> {
//         self.fetch_exported(swiss)
//             .or_else(|| match self.registry.get(swiss) {
//                 Some(v) => {
//                     let key = self.next_key();
//                     self.map.insert(key, v.clone());
//                     self.swiss_map.insert(swiss.to_owned(), key);
//                     Some((key, v.clone()))
//                 }
//                 None => None,
//             })
//     }
//
//     pub fn get<'s>(
//         &'s self,
//         position: &u64,
//     ) -> Option<dashmap::mapref::one::Ref<'s, u64, Arc<V>>> {
//         self.map.get(position)
//     }
// }

pub struct Inbox<Reader, Writer> {
    session: CapTpSession<Reader, Writer>,
    queue: tokio::sync::mpsc::UnboundedReceiver<Delivery>,
}

impl<Reader, Writer> Inbox<Reader, Writer> {
    pub async fn recv(&mut self) -> Option<Delivery> {
        self.queue.recv().await
    }
}

impl<Reader, Writer> futures::stream::Stream for Inbox<Reader, Writer> {
    type Item = Delivery;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.queue.poll_recv(cx)
    }
}

pub type TcpPipe = tokio::sync::mpsc::UnboundedSender<Delivery>;
pub type TcpInbox = Inbox<OwnedReadHalf, OwnedWriteHalf>;

// impl futures::stream::Stream for CapTpRecipe<TcpIpNetlayer<TcpListener, TcpStream>> {
//     type Item = rexa::captp::Event;
//
//     fn poll_next(
//         self: std::pin::Pin<&mut Self>,
//         cx: &mut std::task::Context<'_>,
//     ) -> std::task::Poll<Option<Self::Item>> {
//         todo!()
//     }
// }

// impl iced::advanced::subscription::Recipe for CapTpRecipe<TcpIpNetlayer<TcpListener, TcpStream>> {
//     type Output = crate::gui::Message;
//
//     fn hash(&self, state: &mut iced::advanced::Hasher) {
//         std::any::TypeId::of::<Self>().hash(state)
//     }
//
//     fn stream(
//         self: Box<Self>,
//         _: iced::advanced::subscription::EventStream,
//     ) -> iced::advanced::graphics::futures::BoxStream<Self::Output> {
//         use crate::gui::Message;
//         use futures::stream::{self, Stream, StreamExt};
//         use rexa::captp::Event as CapTpEvent;
//         struct RecvData {
//             key: VerifyingKey,
//             manager: Arc<SessionManager>,
//             session: CapTpSession<TcpStream>,
//             // chat: Arc<crate::chat::ChatManager>,
//             // registry: Arc<SwissRegistry<TcpStream>>,
//             // exports: Arc<ExportManager<TcpPipe>>,
//         }
//         async fn recv(data: RecvData) -> Option<(crate::gui::Message, RecvData)> {
//             let res = loop {
//                 match data.session.recv_event().await {
//                     Ok(CapTpEvent::Bootstrap(b)) => match b {
//                         rexa::captp::BootstrapEvent::Fetch { swiss, resolver } => {
//                             todo!()
//                         }
//                         _ => todo!(),
//                     },
//                     Ok(CapTpEvent::Delivery(del)) => todo!(),
//                     Ok(CapTpEvent::Abort(reason)) => {
//                         let key = data.key;
//                         tracing::info!(reason, id = ?key, "session aborted");
//                         data.manager.remove(&key);
//                         return None;
//                     }
//                     Err(e) => {
//                         let key = data.key;
//                         tracing::error!(error = ?e, id = ?key, "captp error");
//                         data.manager.remove(&key);
//                         return None;
//                     }
//                 }
//             };
//             Some((res, data))
//         }
//         type Recipe = CapTpRecipe<TcpIpNetlayer<TcpListener, TcpStream>>;
//         async fn accept(
//             recipe: Recipe,
//         ) -> Option<(impl Stream<Item = crate::gui::Message> + Unpin, Recipe)> {
//             match recipe.netlayer.accept().await {
//                 Ok(session) => {
//                     let key = session.remote_vkey().clone();
//                     tracing::info!(id = ?key, "accepted connection");
//                     recipe.manager.insert(key, session.clone());
//                     Some((
//                         stream::unfold(
//                             RecvData {
//                                 key,
//                                 manager: recipe.manager.clone(),
//                                 session,
//                                 // chat: recipe.chat.clone(),
//                                 // registry: recipe.registry.clone(),
//                                 // exports: recipe.exports.clone(),
//                             },
//                             recv,
//                         )
//                         .boxed(),
//                         recipe,
//                     ))
//                 }
//                 Err(e) => todo!("handle session accept error: {e:?}"),
//             }
//         }
//         stream::unfold((*self).clone(), accept)
//             .flatten_unordered(None)
//             .boxed()
//     }
// }

// #[derive(Clone)]
// #[repr(transparent)]
// pub struct NetlayerRecipe<Nl>(std::sync::Arc<Nl>);
//
// impl iced::advanced::subscription::Recipe
//     for NetlayerRecipe<TcpIpNetlayer<TcpListener, TcpStream>>
// {
//     type Output = CapTpSession<TcpStream>;
//
//     fn hash(&self, state: &mut iced::advanced::Hasher) {
//         self.0.local_addr().unwrap().hash(state)
//     }
//
//     fn stream(
//         self: Box<Self>,
//         input: iced::advanced::subscription::EventStream,
//     ) -> iced::advanced::graphics::futures::BoxStream<Self::Output> {
//         todo!()
//     }
// }
