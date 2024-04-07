use std::{ops::Deref, sync::Arc};

use dashmap::DashMap;
use ed25519_dalek::{ed25519::signature::SignerMut, Signature, SigningKey, VerifyingKey};
use rexa::{
    captp::{
        msg::DescExport,
        object::{
            DeliverError, Fetch, FetchError, ObjectError, RemoteBootstrap, RemoteError,
            RemoteObject,
        },
        AbstractCapTpSession, GenericResolver, RemoteKey,
    },
    impl_object,
};
use syrup::{Deserialize, FromSyrupItem, Serialize};
use tokio::sync::{mpsc, oneshot, RwLock as AsyncRwLock};

use crate::{
    Channel, ChannelEvent, ChannelId, ChannelInfo, ChannelListing, NetworkEvent, PeerKey, SyrupUuid,
};

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
    async fn authenticate(
        &self,
        #[arg(session)] session: Arc<dyn AbstractCapTpSession + Send + Sync>,
        peer_vkey: PeerKey,
        #[arg(syrup_from = syrup::Bytes<Vec<u8>>)] message: Vec<u8>,
        signature: Signature,
        #[arg(resolver)] resolver: GenericResolver,
    ) -> Result<(), ObjectError> {
        tracing::debug!("received authentication request");
        if peer_vkey.verify_strict(&message, &signature).is_err() {
            return resolver
                .break_promise("could not verify signature")
                .await
                .map_err(From::from);
        }
        drop(self.ev_sender.send(NetworkEvent::PortalRequest {
            session,
            peer_vkey,
            resolver,
        }));
        Ok(())
    }
}

pub struct RemoteGateway {
    base: RemoteObject,
}

impl Fetch for RemoteGateway {
    type Swiss<'s> = ();
    #[tracing::instrument(skip_all)]
    async fn fetch<'swiss>(
        bootstrap: &RemoteBootstrap,
        _: Self::Swiss<'swiss>,
    ) -> Result<Self, FetchError> {
        tracing::info!("fetching gateway");
        Ok(Self {
            base: bootstrap.fetch(GATEWAY_SWISS).await?,
        })
    }
}

impl RemoteGateway {
    #[tracing::instrument(skip(self, skey), fields(message = %String::from_utf8_lossy(message)))]
    pub async fn authenticate_with(
        &self,
        skey: &mut SigningKey,
        message: &[u8],
    ) -> Result<RemotePortal, RemoteError> {
        self.authenticate(&skey.verifying_key(), message, &skey.sign(message))
            .await
    }

    #[tracing::instrument(fields(vkey = rexa::hash(vkey), message = %String::from_utf8_lossy(message)), skip(self, signature))]
    pub async fn authenticate(
        &self,
        vkey: &VerifyingKey,
        message: &[u8],
        signature: &Signature,
    ) -> Result<RemotePortal, RemoteError> {
        tracing::trace!("authenticating gateway");
        match self
            .base
            .call_and(
                "authenticate",
                &syrup::raw_syrup_unwrap![vkey, &syrup::Bytes(message), signature],
            )
            .await?
            .pop()
        {
            Some(position) => match DescExport::from_syrup_item(&position) {
                Ok(pos) => Ok(RemotePortal {
                    base: unsafe {
                        self.base
                            .session()
                            .clone()
                            .into_remote_object_unchecked(pos)
                    },
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
    position: DescExport,
}

pub struct Portal {
    remote_key: PeerKey,
    channels: Arc<DashMap<ChannelId, Channel>>,
}

impl std::fmt::Debug for Portal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Portal").finish_non_exhaustive()
    }
}

impl Portal {
    pub(crate) fn new(remote_key: PeerKey, channels: Arc<DashMap<ChannelId, Channel>>) -> Self {
        Self {
            remote_key,
            channels,
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

    #[deliver()]
    fn connect(
        &self,
        #[arg(session)] session: Arc<dyn AbstractCapTpSession + Send + Sync>,
        #[arg(syrup_from = SyrupUuid)] channel_id: ChannelId,
        outbox: DescExport,
    ) -> Result<ConnectResult, &'static str> {
        let Some(channel) = self.channels.get(&channel_id) else {
            return Err("unrecognized channel id");
        };

        // let peers = channel;

        let position = match channel.exported_position(session.remote_vkey()) {
            Some(pos) => (*pos).into(),
            // FIX :: eugh
            None => session.exports().export(Arc::new(channel.clone())),
        };

        channel.connect_peer(*session.remote_vkey(), self.remote_key, unsafe {
            session.into_remote_object_unchecked(outbox)
        });

        Ok(ConnectResult { position })
    }

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
    pub fn open_with<'result>(
        bootstrap: &'result RemoteBootstrap,
        skey: &mut SigningKey,
        message: &'result [u8],
    ) -> impl std::future::Future<Output = Result<Self, RemotePortalError>> + 'result {
        let signature = skey.sign(message);
        let vkey = skey.verifying_key();
        async move { Self::open(bootstrap, &vkey, message, &signature).await }
    }

    #[tracing::instrument(fields(vkey = rexa::hash(vkey), message = %String::from_utf8_lossy(message)), skip(bootstrap, signature))]
    pub async fn open(
        bootstrap: &RemoteBootstrap,
        vkey: &VerifyingKey,
        message: &[u8],
        signature: &Signature,
    ) -> Result<Self, RemotePortalError> {
        // TODO :: this could probably be improved with promise pipelining
        tracing::trace!("opening portal");
        bootstrap
            .fetch_with::<RemoteGateway>(())
            .await?
            .authenticate(vkey, message, signature)
            .await
            .map_err(From::from)
    }

    #[tracing::instrument(skip(self))]
    pub async fn list_channels(&self) -> Result<Vec<ChannelListing>, DeliverError> {
        match self
            .base
            .deliver_and([&syrup::Symbol("list_channels")])
            .await?
            .pop()
        {
            Some(channels) => Ok(Vec::from_syrup_item(&channels).unwrap_or_default()),
            None => Ok(Vec::new()),
        }
    }

    pub async fn connect(
        &self,
        channel_id: ChannelId,
        info: ChannelInfo,
        ev_sender: mpsc::UnboundedSender<ChannelEvent>,
    ) -> Result<Channel, ObjectError> {
        let channel = Channel::new(channel_id, info, ev_sender);

        let pos = self
            .base
            .session()
            .exports()
            .export(Arc::new(channel.clone()));

        let Some(arg) = self
            .base
            .call_and(
                "connect",
                &syrup::raw_syrup_unwrap![&SyrupUuid(channel_id), &pos],
            )
            .await?
            .pop()
        else {
            return Err(ObjectError::missing(0, "ConnectResult"));
        };

        let Ok(connect) = ConnectResult::from_syrup_item(&arg) else {
            return Err(ObjectError::unexpected("ConnectResult", 0, arg));
        };

        // channel.connect_peer(self.base.remote_vkey(), connect.self_key, unsafe {
        //     self.base.get_remote_object_unchecked(connect.position)
        // });

        Ok(channel)
    }
}
