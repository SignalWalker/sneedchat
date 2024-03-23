use std::sync::Arc;

use dashmap::DashMap;
use ed25519_dalek::{Signature, SignatureError};
use rexa::{
    captp::{
        object::{DeliverOnlyError, ObjectError, RemoteObject},
        RemoteKey,
    },
    impl_object,
    locator::NodeLocator,
};
use syrup::{Deserialize, Serialize};
use tokio::{
    sync::mpsc,
    task::{JoinError, JoinSet},
};

use crate::{PeerKey, SyrupUuid, UserId};

pub type MessageId = uuid::Uuid;

#[derive(syrup::Serialize, syrup::Deserialize)]
#[syrup(name = "channel-listing")]
pub struct ChannelListing {
    #[syrup(as = SyrupUuid)]
    pub id: ChannelId,
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone)]
#[syrup(name = "message")]
pub struct Message {
    #[syrup(as = SyrupUuid)]
    id: MessageId,
    #[syrup(as = SyrupUuid)]
    sender: UserId,
    msg: String,
    signature: Signature,
}

impl Message {
    fn fields_to_bytes(id: MessageId, sender: UserId, msg: &str) -> Vec<u8> {
        let mut res = syrup::ser::to_bytes(&SyrupUuid(id)).unwrap();
        res.extend_from_slice(&syrup::ser::to_bytes(&SyrupUuid(sender)).unwrap());
        res.extend_from_slice(&syrup::ser::to_bytes(msg).unwrap());
        res
    }

    pub fn verify_strict(&self, key: &PeerKey) -> Result<(), SignatureError> {
        key.verify_strict(
            &Self::fields_to_bytes(self.id, self.sender, &self.msg),
            &self.signature,
        )
    }
}

pub enum ChannelEvent {
    RecvMessage {
        channel: Channel,
        message: Message,
    },
    Introduce {
        channel: Channel,
        peer_key: PeerKey,
        locator: NodeLocator,
    },
}

struct Outbox {
    base: RemoteObject,
    peer_key: PeerKey,
}

impl Outbox {
    fn new(base: RemoteObject, peer_key: PeerKey) -> Arc<Self> {
        Arc::new(Self { base, peer_key })
    }

    async fn send_msg(&self, message: &Message) -> Result<(), DeliverOnlyError> {
        self.base.call_only("send_msg", [message]).await
    }

    async fn into_send_msg(self: Arc<Self>, msg: Message) -> Result<(), DeliverOnlyError> {
        self.send_msg(&msg).await
    }
}

struct ChannelCore {
    id: ChannelId,
    name: String,

    ev_sender: mpsc::UnboundedSender<ChannelEvent>,

    exported_at: DashMap<RemoteKey, u64>,
    outboxes: DashMap<RemoteKey, Arc<Outbox>>,
}

impl std::fmt::Debug for ChannelCore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChannelCore")
            .field("id", &self.id)
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SendMsgError {
    #[error(transparent)]
    Join(#[from] JoinError),
    #[error(transparent)]
    Deliver(#[from] DeliverOnlyError),
}

pub type ChannelId = uuid::Uuid;
#[derive(Clone, Debug)]
pub struct Channel {
    core: Arc<ChannelCore>,
}

impl Channel {
    pub fn listing(&self) -> ChannelListing {
        ChannelListing {
            id: self.core.id,
            name: self.core.name.clone(),
        }
    }

    pub fn name(&self) -> &String {
        &self.core.name
    }

    pub fn new(
        id: uuid::Uuid,
        name: String,
        ev_sender: mpsc::UnboundedSender<ChannelEvent>,
    ) -> Self {
        Self {
            core: Arc::new(ChannelCore {
                id,
                name,
                ev_sender,

                exported_at: DashMap::new(),
                outboxes: DashMap::new(),
            }),
        }
    }

    pub async fn send_msg(&self, message: &Message) -> Result<(), SendMsgError> {
        let mut send_tasks = JoinSet::new();
        for outbox in &self.core.outboxes {
            send_tasks.spawn(outbox.clone().into_send_msg(message.clone()));
        }
        while let Some(res) = send_tasks.join_next().await {
            res??;
        }
        Ok(())
    }

    pub(super) fn exported_position(
        &self,
        session_key: &RemoteKey,
    ) -> Option<dashmap::mapref::one::Ref<'_, RemoteKey, u64>> {
        self.core.exported_at.get(session_key)
    }

    pub(super) fn connect_peer(
        &self,
        session_key: RemoteKey,
        peer_key: PeerKey,
        outbox: RemoteObject,
    ) {
        self.core
            .outboxes
            .insert(session_key, Outbox::new(outbox, peer_key));
    }
}

#[impl_object(tracing = ::tracing)]
impl Channel {
    #[deliver_only(symbol = "send_msg")]
    fn deliver_msg(&self, message: Message) -> Result<(), ObjectError> {
        drop(self.core.ev_sender.send(ChannelEvent::RecvMessage {
            channel: self.clone(),
            message,
        }));
        Ok(())
    }

    #[deliver_only()]
    fn introduce(&self, peer_key: PeerKey, locator: NodeLocator) -> Result<(), ObjectError> {
        drop(self.core.ev_sender.send(ChannelEvent::Introduce {
            channel: self.clone(),
            peer_key,
            locator,
        }));
        Ok(())
    }

    #[exported()]
    #[tracing::instrument(fields(remote_key = rexa::hash(remote_key)))]
    fn exported(&self, remote_key: &RemoteKey, position: rexa::captp::msg::DescExport) {
        self.core.exported_at.insert(*remote_key, position.position);
        tracing::debug!("channel exported");
    }
}
