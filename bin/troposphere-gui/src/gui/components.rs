#![allow(non_snake_case)]

use dioxus::prelude::*;
use rexa::locator::NodeLocator;

use crate::gui::chat::ManagerEvent;

#[derive(PartialEq, Props, Clone)]
pub(crate) struct LocatorProps {
    #[props(into)]
    locator: NodeLocator,
    children: Element,
}

pub(crate) fn Locator(LocatorProps { locator, children }: LocatorProps) -> Element {
    let chat = use_coroutine_handle::<ManagerEvent>();
    rsx! {
        button {
            role: "link",
            r#type: "button",
            onclick: move |_event| {
                chat.send(ManagerEvent::OpenPortal { locator: locator.clone() });
            },
            {children}
        }
    }
}
