use std::{ops::Deref, sync::Arc};

use dashmap::DashMap;
use ed25519_dalek::{ed25519::signature::SignerMut, Signature, SigningKey, VerifyingKey};
use rexa::{
    captp::{
        msg::DescExport,
        object::{DeliverError, Fetch, FetchError, RemoteBootstrap, RemoteError, RemoteObject},
        AbstractCapTpSession, RemoteKey,
    },
    impl_object,
};
use syrup::{Deserialize, FromSyrupItem, Serialize};
use tokio::sync::{mpsc, RwLock as AsyncRwLock};

use crate::{Channel, ChannelId, ChannelListing, NetworkEvent, PeerKey};

pub const GATEWAY_SWISS: &[u8] = b"gateway";

pub struct Gateway {
    ev_sender: mpsc::UnboundedSender<NetworkEvent>,
}

impl Gateway {
    pub fn new(ev_sender: mpsc::UnboundedSender<NetworkEvent>) -> Self {
        Self { ev_sender }
    }
}

#[impl_object(tracing = ::tracing)]
impl Gateway {
    #[deliver()]
    #[allow(clippy::needless_pass_by_value)]
    fn authenticate(
        &self,
        #[arg(mapped = &*session)] session: &(dyn AbstractCapTpSession + Send + Sync),
        peer_vkey: PeerKey,
        #[arg(syrup_from = syrup::Bytes<Vec<u8>>)] message: Vec<u8>,
        signature: Signature,
    ) -> Result<DescExport, &'static str> {
        if peer_vkey.verify_strict(&message, &signature).is_err() {
            return Err("could not verify signature");
        }
        Ok(session.exports().export(Arc::new(Portal::new())))
    }
}

pub struct RemoteGateway {
    base: RemoteObject,
}

impl Fetch for RemoteGateway {
    type Swiss<'s> = ();
    async fn fetch<'swiss>(
        bootstrap: &RemoteBootstrap,
        _: Self::Swiss<'swiss>,
    ) -> Result<Self, FetchError> {
        Ok(Self {
            base: bootstrap.fetch(GATEWAY_SWISS).await?,
        })
    }
}

impl RemoteGateway {
    pub async fn authenticate_with(
        &self,
        skey: &mut SigningKey,
        message: &[u8],
    ) -> Result<RemotePortal, RemoteError> {
        self.authenticate(&skey.verifying_key(), message, &skey.sign(message))
            .await
    }

    pub async fn authenticate(
        &self,
        vkey: &VerifyingKey,
        message: &[u8],
        signature: &Signature,
    ) -> Result<RemotePortal, RemoteError> {
        match self
            .base
            .call_and(
                "authenticate",
                &syrup::raw_syrup_unwrap![vkey, message, signature],
            )
            .await?
            .pop()
        {
            Some(position) => match DescExport::from_syrup_item(&position) {
                Ok(pos) => Ok(RemotePortal {
                    base: unsafe { self.base.get_remote_object_unchecked(pos) },
                }),
                Err(_) => Err(RemoteError::unexpected("DescExport", 0, position)),
            },
            None => Err(RemoteError::missing(0, "DescExport")),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[syrup(name = "connect-result")]
pub(crate) struct ConnectResult {
    self_key: PeerKey,
    position: DescExport,
    name: String,
}

pub struct Portal {
    channels: DashMap<ChannelId, Channel>,
}

impl std::fmt::Debug for Portal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Portal").finish_non_exhaustive()
    }
}

impl Portal {
    fn new() -> Self {
        Self {
            channels: DashMap::new(),
        }
    }
}

#[impl_object(tracing = ::tracing)]
impl Portal {
    #[deliver(always_fulfill)]
    fn list_channels(&self) -> Vec<ChannelListing> {
        self.channels
            .iter()
            .map(|entry| entry.value().listing())
            .collect()
    }

    // #[deliver()]
    // fn connect(
    //     &self,
    //     #[arg(session)] session: Arc<dyn AbstractCapTpSession + Send + Sync>,
    //     channel_id: SyrupUuid,
    //     outbox: DescExport,
    // ) -> Result<ConnectResult, &'static str> {
    //     let Some(channel) = self.channels.get(&channel_id.0) else {
    //         return Err("unrecognized channel id");
    //     };
    //
    //     // let peers = channel;
    //
    //     let position = match channel.exported_position(session.remote_vkey()) {
    //         Some(pos) => (*pos).into(),
    //         // FIX :: eugh
    //         None => session.exports().export(Arc::new(channel.clone())),
    //     };
    //
    //     channel.connect_peer(*session.remote_vkey(), self.remote_peer_key, unsafe {
    //         session.into_remote_object_unchecked(outbox)
    //     });
    //
    //     Ok(ConnectResult {
    //         self_key: self.local_peer_key,
    //         position,
    //         name: channel.name().clone(),
    //     })
    // }

    #[exported()]
    fn exported(&self, remote_key: &RemoteKey, position: DescExport) {
        tracing::debug!(portal = ?self, ?position, remote_key_hash = rexa::hash(remote_key), "portal exported");
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RemotePortalError {
    #[error(transparent)]
    Fetch(#[from] FetchError),
    #[error(transparent)]
    Gateway(#[from] RemoteError),
}

pub struct RemotePortal {
    base: RemoteObject,
}

impl RemotePortal {
    pub async fn open_with<'result>(
        bootstrap: &'result RemoteBootstrap,
        skey: &'result mut SigningKey,
        message: &'result [u8],
    ) -> Result<Self, RemotePortalError> {
        // TODO :: this could probably be improved with promise pipelining
        bootstrap
            .fetch_with::<RemoteGateway>(())
            .await?
            .authenticate_with(skey, message)
            .await
            .map_err(From::from)
    }

    pub async fn open(
        bootstrap: &RemoteBootstrap,
        vkey: &VerifyingKey,
        message: &[u8],
        signature: &Signature,
    ) -> Result<Self, RemotePortalError> {
        // TODO :: this could probably be improved with promise pipelining
        bootstrap
            .fetch_with::<RemoteGateway>(())
            .await?
            .authenticate(vkey, message, signature)
            .await
            .map_err(From::from)
    }

    pub async fn list_channels(&self) -> Result<Vec<ChannelListing>, DeliverError> {
        match self.base.deliver_and(["list_channels"]).await?.pop() {
            Some(channels) => Ok(Vec::from_syrup_item(&channels).unwrap_or_default()),
            None => Ok(Vec::new()),
        }
    }

    // pub async fn connect(
    //     &self,
    //     channel_id: ChannelId,
    //     ev_sender: mpsc::UnboundedSender<ChannelEvent>,
    // ) -> Result<Channel, ConnectError> {
    //     let Some(arg) = self
    //         .base
    //         .call_and("connect", &[SyrupUuid(channel_id)])
    //         .await?
    //         .pop()
    //     else {
    //         return Err(ConnectError::MissingResult);
    //     };
    //
    //     let Ok(connect) = ConnectResult::from_syrup_item(&arg) else {
    //         return Err(ConnectError::UnexpectedArgument(arg));
    //     };
    //
    //     let channel = Channel::new(channel_id, connect.name, ev_sender);
    //     channel.connect_peer(self.base.remote_vkey(), connect.self_key, unsafe {
    //         self.base.get_remote_object_unchecked(connect.position)
    //     });
    //
    //     Ok(channel)
    // }
}
