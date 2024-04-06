use ed25519_dalek::VerifyingKey;
use rexa::captp::object::{RemoteError, RemoteObject};
use syrup::FromSyrupItem;
use tokio::sync::RwLock;

pub type UserId = uuid::Uuid;

#[derive(Debug, Clone, syrup::Serialize, syrup::Deserialize)]
pub struct Profile {
    pub vkey: VerifyingKey,
    pub username: String,
    pub avatar: Option<String>,
}

impl Profile {
    pub fn new(vkey: VerifyingKey, username: String, avatar: Option<String>) -> Self {
        Self {
            vkey,
            username,
            avatar,
        }
    }
}

pub struct Persona {
    pub profile: RwLock<Profile>,
}

impl std::fmt::Debug for Persona {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Persona")
            .field("profile", &self.profile)
            .finish_non_exhaustive()
    }
}

impl Persona {
    pub fn new(profile: Profile) -> Self {
        Self {
            profile: profile.into(),
        }
    }
}

#[rexa::impl_object(tracing = ::tracing)]
impl Persona {
    #[allow(clippy::needless_lifetimes)]
    #[deliver(always_fulfill = [&*__res])]
    async fn profile<'s>(&'s self) -> impl std::ops::Deref<Target = Profile> + 's {
        self.profile.read().await
    }

    // #[deliver(always_ok)]
    // async fn get_personal_mailbox(
    //     &self,
    //     session: &(dyn AbstractCapTpSession + Sync),
    // ) -> DescExport {
    //     let mut pm_channels = self.pm_map.lock().await;
    //     if let Some((pos, _)) = pm_channels.get(session.remote_vkey()) {
    //         *pos
    //     } else {
    //         let inbox = Channel::new(
    //             uuid::Uuid::new_v4(),
    //             "Personal Inbox".to_owned(),
    //             self.ev_sender.clone(),
    //         );
    //         self.data.inboxes.insert(inbox.id, inbox.clone());
    //         let pos = DescExport::from(session.export(inbox.clone()));
    //         inboxes.insert(*session.remote_vkey(), (pos, inbox));
    //         pos
    //     }
    // }
}

pub type PeerKey = VerifyingKey;

#[derive(Debug)]
pub struct Peer {
    pub base: RemoteObject,
}

impl Peer {
    pub async fn profile(&self) -> Result<Profile, RemoteError> {
        let mut args = self.base.deliver_and(["profile"]).await?;
        let Some(profile) = args.pop() else {
            return Err(RemoteError::missing(0, "Profile"));
        };
        Profile::from_syrup_item(&profile)
            .map_err(|_err| RemoteError::unexpected("Profile", 0, profile))
    }
}
