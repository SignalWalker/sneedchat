#![feature(strict_provenance)]
#![feature(must_not_suspend)]
#![feature(multiple_supertrait_upcastable)]

mod event;
pub use event::*;

mod user;
pub use user::*;

mod channel;
pub use channel::*;

mod session;
pub use session::*;

mod netlayer;
pub use netlayer::*;

mod input;
pub use input::*;

mod portal;
pub use portal::*;

mod manager;
pub use manager::*;

pub type EventSender = tokio::sync::mpsc::UnboundedSender<NetworkEvent>;
pub type EventReceiver = tokio::sync::mpsc::UnboundedReceiver<NetworkEvent>;
pub type Promise<V> = tokio::sync::oneshot::Receiver<V>;
pub type Resolver<V> = tokio::sync::oneshot::Sender<V>;
pub type Swiss = Vec<u8>;

#[derive(Debug, Clone, Copy)]
#[repr(transparent)]
pub struct SyrupUuid(uuid::Uuid);

impl<'input> syrup::Deserialize<'input> for SyrupUuid {
    fn deserialize<D: syrup::de::Deserializer<'input>>(de: D) -> Result<Self, D::Error> {
        Ok(Self(uuid::Uuid::from_bytes(
            syrup::Bytes::<[u8; 16]>::deserialize(de)?.0,
        )))
    }
}

impl syrup::Serialize for SyrupUuid {
    fn serialize<Ser: syrup::ser::Serializer>(&self, s: Ser) -> Result<Ser::Ok, Ser::Error> {
        syrup::Bytes::<&[u8]>(self.0.as_bytes()).serialize(s)
    }
}

impl From<uuid::Uuid> for SyrupUuid {
    fn from(value: uuid::Uuid) -> Self {
        Self(value)
    }
}

impl From<&uuid::Uuid> for SyrupUuid {
    fn from(value: &uuid::Uuid) -> Self {
        Self(*value)
    }
}

impl From<SyrupUuid> for uuid::Uuid {
    fn from(value: SyrupUuid) -> Self {
        value.0
    }
}
