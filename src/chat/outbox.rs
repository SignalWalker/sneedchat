use rexa::captp::object::{RemoteBootstrap, RemoteObject};
use std::sync::Arc;

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
