// `asset!` in const position trips a nightly `volatile_composites` false
// positive, and `#[component]` generates an empty Props enum for prop-less
// components; neither can be addressed in user code.
#![allow(clippy::volatile_composites, clippy::empty_enums)]

use dioxus::prelude::*;

use crate::components::navbar::Navbar as StyledNavbar;

/// Shared navigation bar built on the styled `dioxus-components` Navbar.
///
/// Router-specific links are passed in as children (styled `NavbarItem`s),
/// so `web` / `desktop` / `mobile` can each supply their own `Route` enum
/// while sharing this wrapper and its styling.
#[component]
pub fn Navbar(children: Element) -> Element {
    rsx! {
        StyledNavbar {
            aria_label: "Main navigation",
            {children}
        }
    }
}
