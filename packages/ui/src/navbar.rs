// `asset!` in const position trips a nightly `volatile_composites` false
// positive, and `#[component]` generates an empty Props enum for prop-less
// components; neither can be addressed in user code.
#![allow(clippy::volatile_composites, clippy::empty_enums)]

use dioxus::prelude::*;

const NAVBAR_CSS: Asset = asset!("/assets/styling/navbar.css");

#[component]
pub fn Navbar(children: Element) -> Element {
    rsx! {
        document::Link { rel: "stylesheet", href: NAVBAR_CSS }

        div {
            id: "navbar",
            {children}
        }
    }
}
