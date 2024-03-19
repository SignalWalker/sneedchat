#![allow(clippy::string_to_string)]

use std::{collections::HashMap, sync::Arc};

use bonfire::{ChannelId, ChannelListing, RemotePortal};
use dioxus::prelude::*;
use rexa::captp::{object::DeliverError, RemoteKey};

use crate::{cfg::Config, gui::chat::REMOTE_PORTALS};

mod chat;

#[tracing::instrument]
pub(super) fn run(cfg: Config) {
    let cfg = Arc::new(cfg);
    LaunchBuilder::new().with_context(cfg).launch(App);
}

#[allow(non_snake_case)]
fn App() -> Element {
    let cfg = use_context::<Arc<Config>>();
    let manager = use_coroutine(move |rx: UnboundedReceiver<()>| chat::manager_coroutine(cfg, rx));
    rsx! {
        Header {}
        main { display: "flex", flex_direction: "row", width: "100%",
            Navigator { }
            Channel { }
        }
    }
}

#[allow(non_snake_case)]
#[component]
fn Header() -> Element {
    rsx! {
        header {
            button {
                "Connect"
            }
        }
    }
}

#[allow(non_snake_case)]
#[component]
fn Navigator() -> Element {
    rsx! {
        nav { width: "33%",
            PortalNav {}
        }
    }
}

#[allow(non_snake_case)]
#[component]
fn PortalNav() -> Element {
    let portals = REMOTE_PORTALS.read();
    let portal_entries = portals
        .iter()
        .map(|entry| PortalEntry(PortalListingComponent::new(entry)));
    rsx! {
        nav {
            h1 { "Portals" },
            menu {
                {portal_entries}
            }
        }
    }
}

#[allow(non_snake_case)]
fn PortalEntry(portal: PortalListingComponent) -> Element {
    let remote = portal.portal.clone();
    let channels = use_resource(move || {
        let inner = remote.clone();
        async move { inner.list_channels().await }
    });
    let key_hash = rexa::hash(&portal.session_key);
    match &*(channels.value().read()) {
        Some(Ok(channels)) => {
            let channels = channels
                .iter()
                .map(|channel| PortalChannelEntry(ChannelListingComponent::new(channel)));
            rsx! {
                li {
                    "{key_hash}",
                    menu {
                        {channels}
                    }
                }
            }
        }
        Some(Err(error)) => {
            rsx! {
                li {
                    "{key_hash} :: Failed to list channels; error: {error}"
                }
            }
        }
        None => rsx! {
            li {
                "{key_hash} :: Loading..."
            }
        },
    }
}

#[allow(non_snake_case)]
fn PortalChannelEntry(channel: ChannelListingComponent) -> Element {
    rsx! {
        li {
            "{channel.name}", code { "{channel.id}" }
        }
    }
}

#[derive(Props, Clone)]
struct PortalListingComponent {
    session_key: RemoteKey,
    portal: Arc<RemotePortal>,
}

impl PartialEq for PortalListingComponent {
    fn eq(&self, other: &Self) -> bool {
        self.session_key == other.session_key
    }
}

impl PortalListingComponent {
    fn new((&session_key, portal): (&RemoteKey, &Arc<RemotePortal>)) -> Self {
        Self {
            session_key,
            portal: portal.clone(),
        }
    }
}

struct ChannelListingComponent {
    id: ChannelId,
    name: String,
}

impl ChannelListingComponent {
    fn new(listing: &ChannelListing) -> Self {
        Self {
            id: listing.id,
            name: listing.name.clone(),
        }
    }
}

// #[allow(non_snake_case)]
// #[component]
// fn UnconnectedChannelListing(portal: ReadOnlRemotePortal) -> Element {
//     let channels = use_resource(portal.list_channels());
//     match channels.read()
// }

#[allow(non_snake_case)]
#[component]
fn Channel() -> Element {
    rsx! {
        section {
            h1 { "Channel!" }
            section {
                Message { msg: MessageData {
                    username: "Ash".to_owned(), message: "Message!".to_owned()
                } }
                Message { msg: MessageData {
                    username: "Amaranth".to_owned(), message: "Message...".to_owned()
                } }
            }
            MessageInput {}
        }
    }
}

pub(super) struct MessageData {
    username: String,
    message: String,
}

#[allow(non_snake_case)]
#[component]
fn Message(msg: ReadOnlySignal<MessageData>) -> Element {
    let MessageData { username, message } = &*msg.read();
    rsx! {
        article {
            h1 { "{username}" }
            p { "{message}" }
        }
    }
}

#[allow(non_snake_case)]
#[component]
fn MessageInput() -> Element {
    rsx! {
        form { onsubmit: move |event| { tracing::info!(?event, "Submitted message form") },
            input { name: "message" }
            input { r#type: "submit", value: "Send" }
        }
    }
}
