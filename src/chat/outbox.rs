use std::sync::Arc;

use crate::chat::{Outboxes, Profile};
use rexa::captp::{object::RemoteObject, AbstractCapTpSession};
use syrup::FromSyrupItem;

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

#[tracing::instrument(skip(local_profile, outboxes, session))]
pub async fn fetch_outbox(
    local_profile: Profile,
    remote_id: uuid::Uuid,
    outboxes: Outboxes,
    session: Arc<impl AbstractCapTpSession + ?Sized>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    tracing::debug!("fetching outbox");
    let outbox = match session
        .into_remote_bootstrap()
        .fetch(local_profile.id.as_bytes())
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
    outbox.set_username(local_profile.username).await.unwrap();
    tracing::trace!("outbox fetched");
    outboxes.insert(remote_id, outbox);
    Ok(())
}
