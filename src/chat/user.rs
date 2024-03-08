use crate::chat::{ChatData, EventSender, SneedEvent, Swiss, SyrupUuid};
use ed25519_dalek::VerifyingKey;
use futures::FutureExt;
use rexa::{
    captp::{
        msg::{DescExport, DescImport},
        object::{Object, RemoteBootstrap, RemoteObject},
        AbstractCapTpSession,
    },
    locator::NodeLocator,
};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock as SyncRwLock},
};
use syrup::FromSyrupItem;
use tokio::{
    sync::{mpsc, Mutex, RwLock},
    task::{JoinSet, LocalSet},
};

#[derive(syrup::Serialize, syrup::Deserialize)]
pub struct ChannelListing {
    #[syrup(as = SyrupUuid)]
    id: uuid::Uuid,
    name: Option<String>,
}

#[derive(Debug, Clone, syrup::Serialize, syrup::Deserialize)]
pub struct Profile {
    pub vkey: VerifyingKey,
    pub username: String,
}

impl Profile {
    pub fn new(vkey: VerifyingKey, username: String) -> Self {
        Self { vkey, username }
    }
}

pub struct Persona {
    pub profile: RwLock<Profile>,
    pm_map: Mutex<HashMap<VerifyingKey, (DescExport, Arc<Inbox>)>>,
    data: Arc<ChatData>,
    ev_sender: mpsc::UnboundedSender<SneedEvent>,
}

impl std::fmt::Debug for Persona {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Persona")
            .field("profile", &self.profile)
            .finish_non_exhaustive()
    }
}

impl Persona {
    pub fn new(
        profile: Profile,
        data: Arc<ChatData>,
        ev_sender: mpsc::UnboundedSender<SneedEvent>,
    ) -> Arc<Self> {
        Arc::new(Self {
            profile: profile.into(),
            pm_map: Mutex::default(),
            data,
            ev_sender,
        })
    }
}

impl Object for Persona {
    #[tracing::instrument(skip(_session))]
    fn deliver_only(
        &self,
        _session: &(dyn AbstractCapTpSession + Sync),
        args: Vec<syrup::Item>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
        tracing::warn!(args = %syrup::ser::to_pretty(&args).unwrap(), "received erroneous deliver_only");
        Ok(())
    }

    fn deliver<'result>(
        &'result self,
        session: &'result (dyn AbstractCapTpSession + Sync),
        args: Vec<syrup::Item>,
        resolver: rexa::captp::GenericResolver,
    ) -> futures::prelude::future::BoxFuture<
        'result,
        Result<(), Box<dyn std::error::Error + Send + Sync + 'static>>,
    > {
        let mut args = args.into_iter();
        async move {
            match args.next() {
                Some(syrup::Item::Symbol(id)) => match id.as_str() {
                    "get_profile" => resolver
                        .fulfill(
                            [&*self.profile.read().await],
                            None,
                            DescImport::Object(0.into()),
                        )
                        .await
                        .map_err(From::from),
                    "get_personal_mailbox" => {
                        let inbox_pos = {
                            let mut inboxes = self.pm_map.lock().await;
                            if let Some((pos, _)) = inboxes.get(session.remote_vkey()) {
                                *pos
                            } else {
                                let inbox =
                                    Inbox::new(uuid::Uuid::new_v4(), self.ev_sender.clone());
                                self.data.inboxes.insert(inbox.id, inbox.clone());
                                let pos = DescExport::from(session.export(inbox.clone()));
                                inboxes.insert(*session.remote_vkey(), (pos, inbox));
                                pos
                            }
                        };
                        resolver
                            .fulfill(&[inbox_pos], None, DescImport::Object(0.into()))
                            .await
                            .map_err(From::from)
                    }
                    "list_channels" => todo!(),
                    "list_friends" => todo!(),
                    id => todo!("unrecognized user method: {id}"),
                },
                Some(_) => todo!(),
                None => todo!(),
            }
        }
        .boxed()
    }
}

pub type InboxId = uuid::Uuid;

/// Local mailbox.
pub struct Inbox {
    id: InboxId,
    ev_sender: mpsc::UnboundedSender<SneedEvent>,
}

impl Inbox {
    pub fn new(id: InboxId, ev_sender: mpsc::UnboundedSender<SneedEvent>) -> Arc<Self> {
        Arc::new(Self { id, ev_sender })
    }
}

impl std::fmt::Debug for Inbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Inbox")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl Object for Inbox {
    fn deliver_only(
        &self,
        session: &(dyn AbstractCapTpSession + Sync),
        mut args: Vec<syrup::Item>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
        let msg = String::from_syrup_item(args.pop().unwrap()).unwrap();
        self.ev_sender
            .send(SneedEvent::RecvMessage {
                inbox_id: self.id,
                session_key: *session.remote_vkey(),
                msg,
            })
            .map_err(From::from)
    }

    fn deliver<'result>(
        &'result self,
        _: &(dyn AbstractCapTpSession + Sync),
        args: Vec<syrup::Item>,
        resolver: rexa::captp::GenericResolver,
    ) -> futures::prelude::future::BoxFuture<
        'result,
        Result<(), Box<dyn std::error::Error + Send + Sync + 'static>>,
    > {
        async move {
            resolver
                .break_promise(format!(
                    "unrecognized delivery: {}",
                    syrup::ser::to_pretty(&args)?
                ))
                .await
                .map_err(From::from)
        }
        .boxed()
    }
}

#[derive(syrup::Serialize, syrup::Deserialize)]
pub struct UserListing {
    profile: Profile,
    address: NodeLocator<String, syrup::Item>,
}

pub type PeerKey = VerifyingKey;

pub struct Peer {
    pub base: RemoteObject,
    pub persona: Arc<Persona>,
    pub profile: Profile,
    pub personal_mailbox: Mutex<Option<Arc<Outbox>>>,
}

impl std::fmt::Debug for Peer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("User")
            .field("profile", &self.profile)
            .finish_non_exhaustive()
    }
}

impl Peer {
    pub async fn fetch(
        bootstrap: RemoteBootstrap,
        persona: Arc<Persona>,
    ) -> Result<Arc<Self>, Box<dyn std::error::Error + Send + Sync + 'static>> {
        let swiss = persona.profile.read().await.vkey.as_bytes().to_owned();
        match bootstrap.fetch(&swiss).await?.await {
            Ok(base) => match base.call_and::<&str>("get_profile", []).await?.await? {
                Ok(mut profile) => Ok(Arc::new(Self {
                    base,
                    persona,
                    profile: Profile::from_syrup_item(profile.pop().unwrap()).unwrap(),
                    personal_mailbox: Mutex::new(None),
                })),
                Err(reason) => Err(syrup::ser::to_pretty(&reason).unwrap().into()),
            },
            Err(reason) => Err(syrup::ser::to_pretty(&reason).unwrap().into()),
        }
    }

    pub async fn get_personal_mailbox(
        &self,
    ) -> Result<Arc<Outbox>, Box<dyn std::error::Error + Send + Sync + 'static>> {
        let mut personal_mailbox = self.personal_mailbox.lock().await;
        if let Some(outbox) = personal_mailbox.as_ref() {
            return Ok(outbox.clone());
        }
        let mut args = self
            .base
            .call_and::<&str>("get_personal_mailbox", [])
            .await?
            .await?
            .map_err(|err| syrup::ser::to_pretty(&err).unwrap())?;
        let pos = DescExport::from_syrup_item(args.pop().unwrap()).unwrap();
        let res = Arc::new(Outbox {
            base: unsafe { self.base.get_remote_object_unchecked(pos.position) },
        });
        *personal_mailbox = Some(res.clone());
        Ok(res)
    }

    pub async fn list_channels(
        &self,
    ) -> Result<
        impl Iterator<Item = ChannelListing>,
        Box<dyn std::error::Error + Send + Sync + 'static>,
    > {
        match self
            .base
            .call_and::<&str>("list_channels", [])
            .await?
            .await?
        {
            Ok(channels) => Ok(channels
                .into_iter()
                .map(|c| ChannelListing::from_syrup_item(c).unwrap())),
            Err(reason) => Err(syrup::ser::to_pretty(&reason).unwrap().into()),
        }
    }

    pub async fn list_friends(
        &self,
    ) -> Result<impl Iterator<Item = UserListing>, Box<dyn std::error::Error + Send + Sync + 'static>>
    {
        match self
            .base
            .call_and::<&str>("list_friends", [])
            .await?
            .await?
        {
            Ok(friends) => Ok(friends
                .into_iter()
                .map(|f| UserListing::from_syrup_item(f).unwrap())),
            Err(reason) => Err(syrup::ser::to_pretty(&reason).unwrap().into()),
        }
    }
}

pub type OutboxId = uuid::Uuid;

/// Remote mailbox.
pub struct Outbox {
    pub base: RemoteObject,
}

impl Outbox {
    pub async fn fetch(
        bootstrap: RemoteBootstrap,
        swiss: &[u8],
    ) -> Result<Arc<Self>, Box<dyn std::error::Error + Send + Sync + 'static>> {
        match bootstrap.fetch(swiss).await?.await {
            Ok(base) => Ok(Self { base }.into()),
            Err(reason) => Err(syrup::ser::to_pretty(&reason).unwrap().into()),
        }
    }

    pub async fn send_msg(
        &self,
        msg: impl AsRef<str>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
        let msg = msg.as_ref();
        self.base.deliver_only(vec![msg]).await.map_err(From::from)
    }

    pub async fn into_send_msg(
        self: Arc<Self>,
        msg: impl AsRef<str>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
        self.send_msg(msg).await
    }
}

pub type ChannelId = uuid::Uuid;
pub struct Channel {
    id: ChannelId,
    pub name: String,
    inbox: Arc<Inbox>,
    outboxes: tokio::sync::RwLock<HashMap<OutboxId, Arc<Outbox>>>,
}

impl std::fmt::Debug for Channel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Channel")
            .field("id", &self.id)
            .field("inbox", &self.inbox)
            .finish_non_exhaustive()
    }
}

impl Channel {
    pub fn new(
        id: uuid::Uuid,
        name: String,
        inbox: Arc<Inbox>,
        initial_outboxes: HashMap<uuid::Uuid, Arc<Outbox>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            id,
            name,
            inbox,
            outboxes: initial_outboxes.into(),
        })
    }

    pub async fn send_msg(
        &self,
        msg: impl AsRef<str>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
        let msg = msg.as_ref();
        let mut send_tasks = JoinSet::new();
        for outbox in self.outboxes.read().await.values() {
            send_tasks.spawn(outbox.clone().into_send_msg(msg.to_owned()));
        }
        while let Some(res) = send_tasks.join_next().await {
            res??;
        }
        Ok(())
    }
}

impl Object for Channel {
    fn deliver_only(
        &self,
        session: &(dyn AbstractCapTpSession + Sync),
        args: Vec<syrup::Item>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
        self.inbox.deliver_only(session, args)
    }

    fn deliver<'result>(
        &'result self,
        session: &'result (dyn AbstractCapTpSession + Sync),
        args: Vec<syrup::Item>,
        resolver: rexa::captp::GenericResolver,
    ) -> futures::prelude::future::BoxFuture<
        'result,
        Result<(), Box<dyn std::error::Error + Send + Sync + 'static>>,
    > {
        self.inbox.deliver(session, args, resolver)
    }
}
