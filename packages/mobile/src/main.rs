// `asset!` in const position trips a nightly `volatile_composites` false
// positive that cannot be addressed in user code.
#![allow(clippy::volatile_composites)]

use dioxus::prelude::*;

use ui::{Console, Docs, Navbar, THEME_CSS, components::navbar::NavbarItem};

#[derive(Debug, Clone, Routable, PartialEq)]
#[rustfmt::skip]
enum Route {
    #[layout(MobileNavbar)]
    #[route("/docs", DocsPage)]
    Docs {},
    #[route("/console", ConsolePage)]
    Console {},
}

// Preloaded into base html at compile time to avoid FOUC (Dioxus #3970).
const MAIN_CSS: Asset = asset!(
    "/assets/main.css",
    CssAssetOptions::new().with_preload(true)
);

fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    // Build cool things ✌️

    rsx! {
        // Global app resources
        document::Link { rel: "stylesheet", href: MAIN_CSS }
        document::Link { rel: "stylesheet", href: THEME_CSS }

        Router::<Route> {}
    }
}

/// A mobile-specific Router around the shared `Navbar` component
/// which allows us to use the mobile-specific `Route` enum.
#[component]
fn MobileNavbar() -> Element {
    rsx! {
        Navbar {
            NavbarItem {
                index: 0usize,
                value: "docs".to_string(),
                to: Route::Docs {},
                "API Docs"
            }
            NavbarItem {
                index: 1usize,
                value: "console".to_string(),
                to: Route::Console {},
                "API Console"
            }
        }

        Outlet::<Route> {}
    }
}

/// Wraps shared `Docs` with the mobile-specific console target, so `ui`
/// never names this crate's `Route` enum yet `Link` still navigates client-side.
#[component]
fn DocsPage() -> Element {
    rsx! {
        Docs { console_target: Route::Console {}.into() }
    }
}

/// Wraps shared `Console` with the mobile-specific docs target.
#[component]
fn ConsolePage() -> Element {
    rsx! {
        Console { docs_target: Route::Docs {}.into() }
    }
}
