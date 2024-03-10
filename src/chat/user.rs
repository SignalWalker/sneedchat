use crate::chat::{
    ChannelListing, ChatData, EventSender, Inbox, Outbox, SneedEvent, Swiss, SyrupUuid,
};
use ed25519_dalek::VerifyingKey;
use futures::FutureExt;
use rexa::{
    captp::{
        msg::{DescExport, DescImport},
        object::{Object, RemoteBootstrap, RemoteObject},
        AbstractCapTpSession, GenericResolver,
    },
    locator::NodeLocator,
};
use std::{
    collections::HashMap,
    ops::Deref,
    sync::{Arc, RwLock as SyncRwLock},
};
use syrup::FromSyrupItem;
use tokio::{
    sync::{mpsc, Mutex, RwLock},
    task::{JoinSet, LocalSet},
};

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

#[rexa::impl_object]
impl Persona {
    #[deliver_only(verbatim)]
    fn deliver_only(
        &self,
        args: Vec<syrup::Item>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
        tracing::warn!(args = %syrup::ser::to_pretty(&args).unwrap(), "received erroneous deliver_only");
        Ok(())
    }

    #[allow(clippy::needless_lifetimes)]
    #[deliver(always_ok = [&*__res])]
    async fn profile<'s>(&'s self) -> impl Deref<Target = Profile> + 's {
        self.profile.read().await
    }

    #[deliver(always_ok)]
    async fn get_personal_mailbox(
        &self,
        session: &(dyn AbstractCapTpSession + Sync),
    ) -> DescExport {
        let mut inboxes = self.pm_map.lock().await;
        if let Some((pos, _)) = inboxes.get(session.remote_vkey()) {
            *pos
        } else {
            let inbox = Inbox::new(uuid::Uuid::new_v4(), self.ev_sender.clone());
            self.data.inboxes.insert(inbox.id, inbox.clone());
            let pos = DescExport::from(session.export(inbox.clone()));
            inboxes.insert(*session.remote_vkey(), (pos, inbox));
            pos
        }
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
                    profile: Profile::from_syrup_item(&profile.pop().unwrap()).unwrap(),
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
        let pos = DescExport::from_syrup_item(&args.pop().unwrap()).unwrap();
        let res = Arc::new(Outbox {
            base: unsafe { self.base.get_remote_object_unchecked(pos.position) },
        });
        *personal_mailbox = Some(res.clone());
        Ok(res)
    }
}
