use crate::chat::{Inbox, Outbox, OutboxId, SyrupUuid};
use rexa::captp::{object::Object, AbstractCapTpSession};
use std::{collections::HashMap, sync::Arc};
use tokio::task::JoinSet;

#[derive(syrup::Serialize, syrup::Deserialize)]
pub struct ChannelListing {
    #[syrup(as = SyrupUuid)]
    id: uuid::Uuid,
    name: Option<String>,
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
