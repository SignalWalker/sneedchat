#![allow(clippy::string_to_string)]

use std::{borrow::Cow, collections::HashMap, ops::Deref, str::FromStr, sync::Arc};

use dioxus::prelude::*;
use pulldown_cmark::Parser;
use rexa::{captp::RemoteKey, locator::NodeLocator};
use tokio::sync::mpsc;
use troposphere::{ChannelId, ChannelListing, RemotePortal};

use crate::{
    cfg::Config,
    gui::chat::{ChannelState, ChatState, ManagerEvent},
    spawn_coroutine,
};

pub(crate) mod chat;

mod components;
pub(crate) use components::*;

pub(crate) mod shortcuts;

#[cfg(not(target_family = "wasm"))]
fn ocapn_handler(request: wry::http::Request<Vec<u8>>) -> wry::http::Response<Cow<'static, [u8]>> {
    use wry::http::{Request, Response};
    tracing::info!(?request, "ocapn request");
    println!("buh");
    Response::builder()
        .status(200)
        .body(Cow::from(&[]))
        .unwrap()
}

fn use_current_channel() -> Signal<Option<Arc<ChannelState>>> {
    use_context()
}

#[tracing::instrument]
pub(super) fn run(cfg: Config) {
    let mut builder = LaunchBuilder::new();
    #[cfg(not(target_family = "wasm"))]
    {
        builder = builder.with_cfg(
            dioxus_desktop::Config::new()
                .with_window(
                    dioxus_desktop::WindowBuilder::new()
                        .with_transparent(true)
                        .with_title("Troposphere"),
                )
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

    let current_channel_signal = use_signal(|| None);
    let current_channel =
        use_context_provider::<Signal<Option<Arc<ChannelState>>>>(move || current_channel_signal);

    #[cfg(not(target_family = "wasm"))]
    {
        let _mdns_state = use_context_provider(crate::native::MdnsState::provider);
        let _mdns = spawn_coroutine(crate::native::mdns_coroutine);
    }

    rsx! {
        main {
            link { href: "/assets/style.css", rel: "stylesheet" }
            Navigator { }
            Channel { current_channel }
        }
    }
}

// #[allow(non_snake_case)]
// #[component]
// fn Header() -> Element {
//     const DIALOG_ID: &str = "portal_connect_dialog";
//     fn show_modal(id: &str) -> UseEval {
//         eval(&format!(r#"document.getElementById("{id}").showModal();"#))
//     }
//     rsx! {
//         header {
//             PortalConnectDialog { id: DIALOG_ID }
//             button {
//                 onclick: move |_| { show_modal(DIALOG_ID); },
//                 "Connect",
//             }
//         }
//     }
// }

// #[allow(non_snake_case)]
// #[component]
// fn Footer() -> Element {
//     rsx! {
//         footer {
//             "Troposphere v0.1"
//         }
//     }
// }

// #[component]
// fn PortalConnectDialog(id: &'static str) -> Element {
//     const ADDRESS_ID: &str = "portal_connect_dialog_address";
//     const PLACEHOLDER: &str = "ocapn://127.0.0.1.tcpip:43456";
//     fn set_custom_validity<Error: AsRef<str> + ?Sized>(id: &str, error: Option<&Error>) -> UseEval {
//         let validity = error.map_or("", Error::as_ref);
//         let js = format!(
//             r#"
//             const elem = document.getElementById("{id}");
//             elem.setCustomValidity("{validity}");
//             elem.reportValidity();
//             "#
//         );
//         eval(&js)
//     }
//     fn set_input_value(id: &str, value: &(impl ToString + ?Sized)) -> UseEval {
//         eval(&format!(
//             r#"document.getElementById("{id}").value = "{}";"#,
//             value.to_string()
//         ))
//     }
//     fn close_dialog(id: &str) -> UseEval {
//         eval(&format!(r#"document.getElementById("{id}").close();"#,))
//     }
//     let manager = use_coroutine_handle::<ManagerEvent>();
//     rsx! {
//         dialog {
//             id: id,
//             open: false,
//             form {
//                 oninput: move |event| {
//                     let values = event.values();
//                     match NodeLocator::from_str(&values["address"].as_value()) {
//                         Ok(_) => set_custom_validity::<str>(ADDRESS_ID, None),
//                         Err(error) => set_custom_validity(ADDRESS_ID, Some(&error.to_string()))
//                     };
//                 },
//                 onsubmit: move |event| {
//                     let values = event.values();
//                     let locator = match NodeLocator::from_str(&values["address"].as_value()) {
//                         Ok(locator) => locator,
//                         Err(error) => {
//                             set_custom_validity(ADDRESS_ID, Some(&error.to_string()));
//                             return
//                         }
//                     };
//                     manager.send(ManagerEvent::OpenPortal{
//                         locator
//                     });
//                     set_custom_validity::<str>(ADDRESS_ID, None);
//                     set_input_value(ADDRESS_ID, "");
//                     close_dialog(id);
//
//                 },
//                 method: "dialog",
//                 label {
//                     autofocus: true,
//                     "Address: ",
//                     input {
//                         id: ADDRESS_ID,
//                         r#type: "url",
//                         name: "address",
//                         size: PLACEHOLDER.len() as i64,
//                         placeholder: PLACEHOLDER
//                     }
//                 },
//                 input {
//                     r#type: "submit",
//                     value: "Connect",
//                 }
//             }
//         },
//     }
// }

#[allow(non_snake_case)]
#[component]
fn Navigator() -> Element {
    let navigators = {
        #[cfg(not(target_family = "wasm"))]
        {
            [ChannelNav(), PortalNav(), crate::native::MdnsNav()]
        }

        #[cfg(target_family = "wasm")]
        {
            [ChannelNav(), PortalNav()]
        }
    };

    rsx! {
        nav { id: "navigator",
            {navigators.into_iter()}
        }
    }
}

#[allow(non_snake_case)]
#[component]
fn ChannelNav() -> Element {
    let mut current_channel = use_current_channel();
    let connected_channels = use_context::<ChatState>().connected_channels;
    let chan_ref = connected_channels.read();
    let channels = chan_ref.iter().map(|(id, (channel, state))| {
        let state = state.clone();
        rsx! {
            li {
                onclick: move |_| {
                    *current_channel.write() = Some(state.clone());
                },
                title: channel.info().description.clone(),
                {channel.info().name.clone()}
            }
        }
    });
    rsx! {
        nav {
            h1 { "Channels" },
            menu {
                {channels}
            }
        }
    }
}

#[allow(non_snake_case)]
#[component]
fn PortalNav() -> Element {
    let state = use_context::<ChatState>().opened_portals;
    let state_ref = state.read();
    let portals = state_ref.iter().map(|(session_key, state)| {
        let channels = match &state.channels {
            Some(Ok(channels)) => {
                rsx! {
                    menu {
                        for channel in channels {
                            li { title: channel.info.description.clone(), {channel.info.name.clone()} }
                        }
                    }
                }
            }
            Some(Err(error)) => {
                rsx! {
                    small { "Error fetching channels: {error}" }
                }
            }
            None => rsx! {
                small { "Fetching channels..." }
            },
        };
        rsx! {
            li {
                {rexa::hash(session_key).to_string()}
                {channels}
            }
        }
    });
    rsx! {
        nav {
            h1 { "Portals" },
            menu {
                {portals}
            }
        }
    }
}

#[allow(non_snake_case)]
#[component]
fn About() -> Element {
    rsx! {
        article { class: "about",
            h1 { "Troposphere v0.1.0" }
        }
    }
}

#[allow(non_snake_case)]
#[component]
fn Channel(current_channel: Signal<Option<Arc<ChannelState>>>) -> Element {
    let channel_ref = current_channel.read();

    if channel_ref.is_none() {
        return About();
    }

    // let mut messages = vec![
    //     Message(MessageData {
    //         username: "Morgan".to_owned(),
    //         avatar: "/assets/avatars/morgan-standing.webp",
    //         message: r#"<style>@keyframes rotate { 0% { transform: rotate3d(0, 1, 0, 0deg); } 100% { transform: rotate3d(0, 1, 0, 360deg); } }</style><div style="perspective: 6em"><p style="animation: 3s linear 1s infinite running rotate; background: linear-gradient(to right, purple, teal) text; color: transparent;">I'm a wizard!</p></div>"#.to_owned(),
    //     }),
    //     Message(MessageData {
    //         username: "Amaranth".to_owned(),
    //         avatar: "/assets/avatars/ash-evil.png",
    //         message: "Woah...".to_owned(),
    //     })
    //     // Message(MessageData {
    //     //     username: "Decker".to_owned(),
    //     //     avatar: "/assets/avatars/pond.svg",
    //     //     message: r#"<p>hey check this out</p><iframe src="https://html-classic.itch.zone/html/9531270/index.html"></iframe>"#.to_owned()
    //     // })
    // ];
    //
    // #[cfg(not(target_family = "wasm"))]
    // {
    //     messages.push(Message(MessageData {
    //         username: "Ash".to_owned(),
    //         avatar: "/assets/avatars/ash.png",
    //         message: std::fs::read_to_string("public/assets/dragons-pass.md").unwrap(),
    //     }));
    // }

    let state = channel_ref.as_ref().unwrap();

    let mut peers_changed = use_signal(|| ());
    let peer_notif = state.peer_notif();
    let _peers_future = use_future(move || {
        let peer_notif = peer_notif.clone();
        async move {
            loop {
                peer_notif.notified().await;
                peers_changed.write();
            }
        }
    });

    let mut messages_changed = use_signal(|| ());
    let msg_notif = state.msg_notif();
    let _messages_future = use_future(move || {
        let msg_notif = msg_notif.clone();
        async move {
            loop {
                msg_notif.notified().await;
                messages_changed.write();
            }
        }
    });

    drop(peers_changed.read());
    drop(messages_changed.read());

    let peers = state.peers();
    let peer_list = peers.iter().map(|(_, profile)| {
        rsx! {
            li { {profile.username.clone()} }
        }
    });

    let messages = state.messages();
    let messages = messages.iter().map(|msg| {
        let profile = &peers[&msg.sender];
        Message(MessageData {
            username: profile.username.clone(),
            avatar: format!(
                "/assets/avatars/{}",
                profile.avatar.as_deref().unwrap_or("pond.svg")
            ),
            message: msg.msg.clone(),
        })
    });

    rsx! {
        article { class: "channel",
            div { class: "channel-info",
                header {
                    h1 { {state.channel.info().name.clone()} }
                    {state.channel.info().description.clone()}
                }
                section { class: "peer-list",
                    h1 { "Peers" }
                    ul {
                        {peer_list}
                    }
                }
            }
            div { class: "channel-content",
                {messages}
            }
            {MessageInput(state.cmd_sender.clone())}
        }
    }
}

#[derive(Clone, PartialEq, Props)]
pub(super) struct MessageData {
    username: String,
    avatar: String,
    message: String,
}

#[allow(non_snake_case)]
fn Message(
    MessageData {
        username,
        avatar,
        message,
    }: MessageData,
) -> Element {
    use pulldown_cmark::Options;
    let mut options = Options::empty();
    options.insert(
        Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_TABLES
            | Options::ENABLE_FOOTNOTES
            | Options::ENABLE_TASKLISTS
            | Options::ENABLE_SMART_PUNCTUATION,
    );
    let parser = Parser::new_ext(&message, options);
    let mut parsed_msg = String::new();
    pulldown_cmark::html::push_html(&mut parsed_msg, parser);
    rsx! {
        article { class: "message",
            address {
                img { src: avatar }
                h1 { class: "username", "{username}" }
            }
            div { class: "message-content",
                dangerous_inner_html: parsed_msg
            }
        }
    }
}

#[allow(non_snake_case)]
fn MessageInput(cmd_sender: mpsc::UnboundedSender<chat::ChannelCommand>) -> Element {
    fn set_input_value(id: &str, value: &(impl ToString + ?Sized)) -> UseEval {
        eval(&format!(
            r#"document.getElementById("{id}").value = "{}";"#,
            value.to_string()
        ))
    }
    rsx! {
        form { id: "message-form", class: "channel-input",
            onsubmit: move |event| {
                let values = event.values();
                let message = values["message"].as_value();
                drop(cmd_sender.send(chat::ChannelCommand::SendMsg { message }));
                set_input_value("message-input", "");
            },
            input {
                r#type: "text",
                id: "message-input",
                name: "message",
                spellcheck: true,
                // wrap: "soft",
                autocomplete: "off",
                required: true,
                placeholder: "Markdown text..."
            }
            input { r#type: "submit", value: "Send" }
        }
    }
}
