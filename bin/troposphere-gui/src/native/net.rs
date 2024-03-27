#[cfg(target_family = "unix")]
mod _unix {
    use pnet::ipnetwork::IpNetwork;

    #[repr(transparent)]
    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    pub(crate) struct NetworkInterface {
        pub(super) base: pnet::datalink::NetworkInterface,
    }

    impl From<pnet::datalink::NetworkInterface> for NetworkInterface {
        fn from(base: pnet::datalink::NetworkInterface) -> Self {
            Self { base }
        }
    }

    impl NetworkInterface {
        pub(crate) fn fetch() -> Vec<Self> {
            pnet::datalink::interfaces()
                .into_iter()
                .map(From::from)
                .collect()
        }

        pub(crate) fn is_loopback(&self) -> bool {
            self.base.is_loopback()
        }

        pub(crate) fn is_multicast(&self) -> bool {
            self.base.is_multicast()
        }

        pub(crate) fn is_point_to_point(&self) -> bool {
            self.base.is_point_to_point()
        }

        pub(crate) fn name(&self) -> &str {
            &self.base.name
        }

        pub(crate) fn description(&self) -> &str {
            &self.base.description
        }

        pub(crate) fn ips(&self) -> &[IpNetwork] {
            &self.base.ips
        }
    }
}

#[cfg(target_family = "unix")]
pub(crate) use _unix::*;

use base64::{engine::general_purpose::STANDARD_NO_PAD, Engine};
use dioxus::{core_macro::rsx, prelude::*};
use futures::StreamExt;
use mdns_sd::{IfKind, ServiceDaemon, ServiceInfo};
use rexa::{captp::RemoteKey, locator::NodeLocator};
use tokio::task::JoinSet;
use troposphere::PeerKey;

use std::{
    collections::{HashMap, HashSet},
    net::{IpAddr, SocketAddr},
    sync::Arc,
};

use crate::{
    cfg::Config,
    gui::{chat::ChatState, Locator},
};

pub(crate) fn find_interface(
    pred: impl FnMut(&NetworkInterface) -> bool,
) -> Option<NetworkInterface> {
    NetworkInterface::fetch().into_iter().find(pred)
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum MdnsError {
    #[error(transparent)]
    Mdns(#[from] mdns_sd::Error),
    #[error("could not find suitable network interface")]
    NoSuitableInterface,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum MdnsCommand {
    Stop,
}

#[derive(Clone)]
pub(crate) struct MdnsPeer {
    pub(crate) vkey: PeerKey,
    pub(crate) addr: SocketAddr,
    pub(crate) name: String,
}

impl TryFrom<&ServiceInfo> for MdnsPeer {
    type Error = ();
    fn try_from(info: &ServiceInfo) -> Result<Self, Self::Error> {
        if let (Some(address), Some(vkey)) = (
            info.get_addresses().iter().find(|addr| match addr {
                IpAddr::V4(v4) => v4.is_private(),
                IpAddr::V6(v6) => v6.is_unicast() && v6.is_unique_local(),
            }),
            info.get_property_val_str("verifying_key")
                .and_then(|b64| STANDARD_NO_PAD.decode(b64).ok())
                .and_then(|bytes| PeerKey::try_from(bytes.as_slice()).ok()),
        ) {
            Ok(Self {
                vkey,
                addr: SocketAddr::new(*address, info.get_port()),
                name: info.get_hostname().to_owned(),
            })
        } else {
            Err(())
        }
    }
}

impl std::fmt::Debug for MdnsPeer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MdnsPeer")
            .field("vkey", &rexa::hash(&self.vkey))
            .field("addr", &self.addr)
            .field("name", &self.name)
            .finish()
    }
}

impl MdnsPeer {
    fn merge(&mut self, new: Self) {
        // TODO
    }
}

#[derive(Clone, Copy)]
pub(crate) struct MdnsState {
    pub(crate) status: SyncSignal<Option<MdnsError>>,
    pub(crate) lan_peers: SyncSignal<HashMap<PeerKey, MdnsPeer>>,
}

impl MdnsState {
    pub(crate) fn provider() -> Self {
        Self {
            status: Default::default(),
            lan_peers: Default::default(),
        }
    }
}

#[tracing::instrument(skip(rx))]
pub(crate) fn mdns_coroutine(
    rx: UnboundedReceiver<MdnsCommand>,
) -> impl std::future::Future<Output = ()> + Send {
    const SERVICE_TYPE: &str = "_troposphere._tcp.local.";

    let cfg = use_context::<Arc<Config>>();

    let mut state = use_context::<MdnsState>();
    *state.status.write() = None;
    state.lan_peers.write().clear();

    let chat = use_context::<ChatState>();

    return async move {
        let mdns = match ServiceDaemon::new() {
            Ok(mdns) => mdns,
            Err(error) => {
                tracing::error!(%error, "could not create mdns service daemon");
                *state.status.write() = Some(error.into());
                return;
            }
        };
        let service_names = match manage_mdns(mdns.clone(), rx, cfg, state, chat).await {
            Ok(services) => services,
            Err(error) => {
                tracing::error!(%error, "mdns daemon failed");
                *state.status.write() = Some(error);
                HashSet::with_capacity(0)
            }
        };

        if let Err(error) = mdns.stop_browse(SERVICE_TYPE) {
            tracing::error!(%error, "mdns.stop_browse()");
        }

        let mut service_shutdowns = JoinSet::new();
        for service in &service_names {
            let recv = match mdns.unregister(service) {
                Ok(recv) => recv,
                Err(error) => {
                    tracing::error!(%error, "failed to unregister mdns service");
                    continue;
                }
            };
            service_shutdowns.spawn(async move {
                #[allow(clippy::never_loop)]
                while let Ok(status) = recv.recv_async().await {
                    match status {
                        mdns_sd::UnregisterStatus::OK => break,
                        mdns_sd::UnregisterStatus::NotFound => {
                            tracing::warn!("tried to unregister mdns service, but it wasn't found in the registration");
                            break
                        },
                    }
                }
            });
        }
        while let Some(res) = service_shutdowns.join_next().await {
            if let Err(error) = res {
                tracing::error!(%error, "failed to unregister mdns service");
            }
        }
        'outer: loop {
            match mdns.shutdown() {
                Err(mdns_sd::Error::Again) => continue,
                Ok(status) => {
                    while let Ok(event) = status.recv_async().await {
                        if event == mdns_sd::DaemonStatus::Shutdown {
                            break 'outer;
                        }
                    }
                }
                Err(error) => {
                    tracing::error!(%error, "failed to shutdown mdns service daemon");
                }
            }
            break;
        }
    };

    #[tracing::instrument(skip_all)]
    async fn manage_mdns(
        mdns: ServiceDaemon,
        mut rx: UnboundedReceiver<MdnsCommand>,
        cfg: Arc<Config>,
        mut state: MdnsState,
        chat: ChatState,
    ) -> Result<HashSet<String>, MdnsError> {
        fn register_service(
            mdns: &ServiceDaemon,
            interfaces: &[NetworkInterface],
            hostname: &str,
            self_key: &RemoteKey,
            address: SocketAddr,
        ) -> Result<String, MdnsError> {
            let addresses = interfaces
                .iter()
                .flat_map(|iface| {
                    iface.ips().iter().filter_map(|network| {
                        let ip = network.ip();
                        (ip.is_ipv4() == address.is_ipv4()).then_some(ip)
                    })
                })
                .collect::<Vec<_>>();
            let port = address.port();
            let service = ServiceInfo::new::<&[IpAddr], &[(&str, &str)]>(
                SERVICE_TYPE,
                &format!("{port}.{hostname}"),
                hostname,
                addresses.as_slice(),
                port,
                &[(
                    "verifying_key",
                    &STANDARD_NO_PAD.encode(self_key.to_bytes()),
                )],
            )?;
            let fullname = service.get_fullname().to_owned();
            mdns.register(service)?;
            Ok(fullname)
        }

        let mut interfaces = Vec::new();
        for interface in NetworkInterface::fetch() {
            if !interface.is_multicast() {
                let name = interface.name();
                tracing::debug!(interface = name, "ignoring interface");
                mdns.disable_interface(IfKind::Name(name.to_owned()))?;
            } else {
                interfaces.push(interface);
            }
        }

        let hostname = gethostname::gethostname().to_str().unwrap().to_owned();

        let browse = mdns.browse(SERVICE_TYPE)?;

        let monitor = mdns.monitor()?;

        {
            let mut done_binding = chat.done_binding.0.lock();
            if !*done_binding {
                tracing::trace!("waiting for bound addresses");
                chat.done_binding
                    .1
                    .wait_while(&mut done_binding, |done| !*done);
            }
        }

        let self_key = *chat.self_key.read();

        let mut services = HashSet::new();
        for address in chat.bound_addresses.read().iter() {
            services.insert(register_service(
                &mdns,
                &interfaces,
                &hostname,
                &self_key,
                *address,
            )?);
        }

        tracing::info!(?services, "registered mdns services");

        tracing::debug!("listening for mdns events");

        let mut key_map = HashMap::<String, PeerKey>::new();

        loop {
            let event = tokio::select! {
                command = rx.next() => match command {
                    None |
                    Some(MdnsCommand::Stop) => break
                },
                Ok(event) = browse.recv_async() => event,
                Ok(event) = monitor.recv_async() => match event {
                    mdns_sd::DaemonEvent::Error(error) => {
                        tracing::error!(%error, "daemon error");
                        continue
                    },
                    mdns_sd::DaemonEvent::Announce(fullname, addresses) => {
                        tracing::debug!(%fullname, %addresses, "daemon announce");
                        continue
                    }
                    mdns_sd::DaemonEvent::IpAdd(ip) => {
                        tracing::debug!(%ip, "ip added");
                        continue
                    }
                    mdns_sd::DaemonEvent::IpDel(ip) => {
                        tracing::debug!(%ip, "ip removed");
                        continue
                    },
                    _ => {
                        tracing::warn!(?event, "unrecognized daemon event");
                        continue
                    },
                }
            };
            match event {
                mdns_sd::ServiceEvent::ServiceFound(ty, fullname) => {
                    tracing::debug!(%ty, %fullname, "found service");
                }
                mdns_sd::ServiceEvent::ServiceResolved(info) => {
                    let srv_fullname = info.get_fullname();
                    if !services.contains(srv_fullname) {
                        if let Ok(peer) = MdnsPeer::try_from(&info) {
                            tracing::debug!(?peer, "mdns peer resolved");
                            key_map.insert(srv_fullname.to_owned(), peer.vkey);
                            let mut lan_peers = state.lan_peers.write();
                            if let Some(old) = lan_peers.get_mut(&peer.vkey) {
                                old.merge(peer);
                            } else {
                                lan_peers.insert(peer.vkey, peer);
                            }
                        }
                    }
                }
                mdns_sd::ServiceEvent::ServiceRemoved(_ty, fullname) => {
                    if let Some(vkey) = key_map.remove(&fullname) {
                        if let Some(peer) = state.lan_peers.write().remove(&vkey) {
                            tracing::debug!(?peer, "service removed");
                        }
                    }
                }
                mdns_sd::ServiceEvent::SearchStarted(_)
                | mdns_sd::ServiceEvent::SearchStopped(_) => {}
            }
        }

        Ok(services)
    }
}

#[allow(non_snake_case)]
#[component]
pub(crate) fn MdnsNav() -> Element {
    // let chat = use_coroutine_handle::<ManagerCommand>();

    let mdns = use_context::<MdnsState>();
    tracing::trace!("rendering mdns nav");
    let entries = if let Some(status) = &*mdns.status.read() {
        rsx! { {status.to_string()} }
    } else {
        let lan_peers = mdns.lan_peers.read();
        let locators = lan_peers.iter().map(|(_, peer)| {
            (
                &peer.name,
                NodeLocator {
                    designator: peer.addr.ip().to_string(),
                    transport: "tcpip".to_owned(),
                    hints: HashMap::from_iter([(
                        rexa::syrup::Symbol("port".to_string()),
                        peer.addr.port().to_string(),
                    )]),
                },
            )
        });
        rsx! {
            menu {
                for (name, locator) in locators {
                    li {
                        Locator {
                            locator: locator.clone(),
                            "{name} @ {locator.designator}:{locator[\"port\"]}"
                        }
                    }
                }
            }
        }
    };
    rsx! {
        nav { class: "tree",
            h1 { "LAN Peers" },
            {entries}
        }
    }
}
