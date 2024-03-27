#![allow(clippy::string_to_string)]

use std::{borrow::Cow, collections::HashMap, str::FromStr, sync::Arc};

use dioxus::prelude::*;
use rexa::{captp::RemoteKey, locator::NodeLocator};
use troposphere::{ChannelId, ChannelListing, RemotePortal};

use crate::{
    cfg::Config,
    gui::chat::{ChatState, ManagerEvent},
    spawn_coroutine,
};

pub(crate) mod chat;

mod components;
pub(crate) use components::*;

pub(crate) mod shortcuts;

#[cfg(not(target_family = "wasm"))]
const _: &str = manganis::mg!(file("./public/assets/style.css"));

#[cfg(not(target_family = "wasm"))]
fn ocapn_handler(request: wry::http::Request<Vec<u8>>) -> wry::http::Response<Cow<'static, [u8]>> {
    use wry::http::{Request, Response};
    tracing::info!(?request, "ocapn request");
    Response::builder()
        .status(200)
        .body(Cow::from(&[]))
        .unwrap()
}

#[tracing::instrument]
pub(super) fn run(cfg: Config) {
    let mut builder = LaunchBuilder::new();
    #[cfg(not(target_family = "wasm"))]
    {
        builder = builder.with_cfg(
            dioxus_desktop::Config::new()
                .with_window(dioxus_desktop::WindowBuilder::new().with_transparent(true))
                .with_menu(None)
                .with_background_color((0, 0, 0, 0))
                .with_resource_directory(cfg.desktop.directories.data.join("assets"))
                .with_data_directory(cfg.desktop.directories.data.join("dioxus"))
                .with_custom_protocol("ocapn".to_owned(), ocapn_handler),
        );
    }
    builder.with_context(Arc::new(cfg)).launch(App);
}

#[allow(non_snake_case)]
fn App() -> Element {
    let _chat_state = use_context_provider(chat::ChatState::provider);
    let _manager = spawn_coroutine(chat::manager_coroutine);

    #[cfg(not(target_family = "wasm"))]
    {
        let _mdns_state = use_context_provider(crate::native::MdnsState::provider);
        let _mdns = spawn_coroutine(crate::native::mdns_coroutine);
    }

    rsx! {
        main {
            Navigator { }
            Channel { }
        }
    }
}

#[allow(non_snake_case)]
#[component]
fn Header() -> Element {
    const DIALOG_ID: &str = "portal_connect_dialog";
    fn show_modal(id: &str) -> UseEval {
        eval(&format!(r#"document.getElementById("{id}").showModal();"#))
    }
    rsx! {
        header {
            PortalConnectDialog { id: DIALOG_ID }
            button {
                onclick: move |_| { show_modal(DIALOG_ID); },
                "Connect",
            }
        }
    }
}

#[allow(non_snake_case)]
#[component]
fn Footer() -> Element {
    rsx! {
        footer {
            "Troposphere v0.1"
        }
    }
}

#[component]
fn PortalConnectDialog(id: &'static str) -> Element {
    const ADDRESS_ID: &str = "portal_connect_dialog_address";
    const PLACEHOLDER: &str = "ocapn://127.0.0.1.tcpip:43456";
    fn set_custom_validity<Error: AsRef<str> + ?Sized>(id: &str, error: Option<&Error>) -> UseEval {
        let validity = error.map_or("", Error::as_ref);
        let js = format!(
            r#"
            const elem = document.getElementById("{id}");
            elem.setCustomValidity("{validity}");
            elem.reportValidity();
            "#
        );
        eval(&js)
    }
    fn set_input_value(id: &str, value: &(impl ToString + ?Sized)) -> UseEval {
        eval(&format!(
            r#"document.getElementById("{id}").value = "{}";"#,
            value.to_string()
        ))
    }
    fn close_dialog(id: &str) -> UseEval {
        eval(&format!(r#"document.getElementById("{id}").close();"#,))
    }
    let manager = use_coroutine_handle::<ManagerEvent>();
    rsx! {
        dialog {
            id: id,
            open: false,
            form {
                oninput: move |event| {
                    let values = event.values();
                    match NodeLocator::from_str(&values["address"].as_value()) {
                        Ok(_) => set_custom_validity::<str>(ADDRESS_ID, None),
                        Err(error) => set_custom_validity(ADDRESS_ID, Some(&error.to_string()))
                    };
                },
                onsubmit: move |event| {
                    let values = event.values();
                    let locator = match NodeLocator::from_str(&values["address"].as_value()) {
                        Ok(locator) => locator,
                        Err(error) => {
                            set_custom_validity(ADDRESS_ID, Some(&error.to_string()));
                            return
                        }
                    };
                    manager.send(ManagerEvent::OpenPortal{
                        locator
                    });
                    set_custom_validity::<str>(ADDRESS_ID, None);
                    set_input_value(ADDRESS_ID, "");
                    close_dialog(id);

                },
                method: "dialog",
                label {
                    autofocus: true,
                    "Address: ",
                    input {
                        id: ADDRESS_ID,
                        r#type: "url",
                        name: "address",
                        size: PLACEHOLDER.len() as i64,
                        placeholder: PLACEHOLDER
                    }
                },
                input {
                    r#type: "submit",
                    value: "Connect",
                }
            }
        },
    }
}

#[allow(non_snake_case)]
#[component]
fn Navigator() -> Element {
    #[cfg(not(target_family = "wasm"))]
    rsx! {
        nav {
            ChannelNav {}
            PortalNav {}
            crate::native::MdnsNav {}
        }
    }
    #[cfg(target_family = "wasm")]
    rsx! {
        nav {
            ChannelNav {}
            PortalNav {}
        }
    }
}

#[allow(non_snake_case)]
#[component]
fn ChannelNav() -> Element {
    rsx! {
        nav { class: "tree",
            h1 { "Channels" },
            menu {
                li { "..." }
            }
        }
    }
}

#[allow(non_snake_case)]
#[component]
fn PortalNav() -> Element {
    let portals = use_signal_sync::<HashMap<RemoteKey, Arc<RemotePortal>>>(HashMap::new);
    rsx! {
        nav { class: "tree",
            h1 { "Portals" },
            menu {
                {portals
                    .read()
                    .iter()
                    .map(|entry| PortalEntry(PortalListingComponent::new(entry)))
                }
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

#[derive(Props, Clone)]
struct ChannelListingComponent {
    id: ChannelId,
    name: String,
}

impl PartialEq for ChannelListingComponent {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
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
    let mut messages = vec![
        Message(MessageData {
            username: "Ash".to_owned(),
            message: "Message!".to_owned(),
        }),
        Message(MessageData {
            username: "Amaranth".to_owned(),
            message: "Message...".to_owned(),
        }),
    ];

    #[cfg(not(target_family = "wasm"))]
    {
        messages.push(Message(MessageData {
            username: "Ash".to_owned(),
            message: std::fs::read_to_string("public/assets/dragons-pass.md").unwrap(),
        }));
    }

    rsx! {
        article { class: "channel",
            div { class: "channel-info",
                h1 { "Rat Jamboree" }
                "This is a channel!"
            }
            div { class: "channel-content",
                {messages.into_iter()}
            }
            MessageInput {}
        }
    }
}

#[derive(Clone, PartialEq, Props)]
pub(super) struct MessageData {
    username: String,
    message: String,
}

#[allow(non_snake_case)]
fn Message(MessageData { username, message }: MessageData) -> Element {
    rsx! {
        article { class: "message",
            address {
                img { class: "avatar", src: "assets/pond.svg" }
                h1 { class: "username", "{username}" }
            }
            div { class: "message-content",
                {message}
            }
        }
    }
}

#[allow(non_snake_case)]
#[component]
fn MessageInput() -> Element {
    rsx! {
        form { class: "message-input",
            onsubmit: move |event| { tracing::info!(?event, "Submitted message form") },
            textarea { name: "message", spellcheck: true, wrap: "soft", autocomplete: "off" }
            input { r#type: "submit", value: "Send" }
        }
    }
}
