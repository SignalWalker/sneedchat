use crate::chat::SneedEvent;
use futures::FutureExt;
use rexa::captp::{object::Object, AbstractCapTpSession};
use std::sync::Arc;
use syrup::FromSyrupItem;
use tokio::sync::mpsc;

pub type InboxId = uuid::Uuid;

/// Local mailbox.
pub struct Inbox {
    pub id: InboxId,
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
        let msg = String::from_syrup_item(&args.pop().unwrap()).unwrap();
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
