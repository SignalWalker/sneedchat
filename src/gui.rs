use crate::chat::Peer;
use dioxus::prelude::*;
use std::sync::Arc;
use time::OffsetDateTime;

pub fn run(
    args: crate::cli::Cli,
) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    LaunchBuilder::new().launch(App);
    Ok(())
}

#[allow(non_snake_case)]
fn App() -> Element {
    rsx! {
        header { "Header!" }
        main { display: "flex", flex_direction: "row", width: "100%",
            nav { width: "33%",
                "Channel Nav!"
            }
            section {
                h1 { "Channel!" }
                section {
                    Message { msg: MessageData {
                        username: "Ash".to_owned(), time: time::OffsetDateTime::now_utc(), message: "Message!".to_owned()
                    } }
                }
                MessageInput {}
            }
        }
    }
}

pub struct MessageData {
    username: String,
    time: OffsetDateTime,
    message: String,
}

#[allow(non_snake_case)]
#[component]
fn Message(msg: ReadOnlySignal<MessageData>) -> Element {
    let MessageData {
        username,
        time: _,
        message,
    } = &*msg.read();
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
            input { r#type: "submit" }
        }
    }
}
