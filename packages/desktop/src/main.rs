// `asset!` in const position trips a nightly `volatile_composites` false
// positive that cannot be addressed in user code. The catch-all 404 route
// makes the `Routable` derive's no-match branch unreachable (expected).
#![allow(clippy::volatile_composites, unreachable_code)]

use dioxus::prelude::*;

use ui::{ApiExplorer, Navbar, NotFound, THEME_CSS, components::navbar::NavbarItem};

#[derive(Debug, Clone, Routable, PartialEq)]
#[rustfmt::skip]
enum Route {
    #[layout(DesktopNavbar)]
    #[route("/docs", ApiExplorerPage)]
    Docs {},
    // Alias for old bookmarks: same unified page as `/docs`.
    #[route("/console", ApiExplorerPage)]
    Console {},
    // `"/"` needs an explicit route: the catch-all only matches non-empty
    // paths (see `web`).
    #[route("/", RootNotFound)]
    Root {},
    #[route("/:..segments", NotFoundPage)]
    NotFound { segments: Vec<String> },
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

/// A desktop-specific Router around the shared `Navbar` component
/// which allows us to use the desktop-specific `Route` enum.
#[component]
fn DesktopNavbar() -> Element {
    rsx! {
        Navbar {
            NavbarItem {
                index: 0usize,
                value: "api".to_string(),
                to: Route::Docs {},
                "API"
            }
        }

        Outlet::<Route> {}
    }
}

/// Unified API reference + console (shared `ApiExplorer`); `/console` is an
/// alias of `/docs` so old links keep working.
#[component]
fn ApiExplorerPage() -> Element {
    rsx! {
        ApiExplorer {}
    }
}

/// Catch-all 404 page inside the shared navbar layout.
#[component]
fn NotFoundPage(segments: Vec<String>) -> Element {
    rsx! {
        NotFound { segments }
    }
}

/// `"/"` renders the same `NotFound` page explicitly (no segments to capture).
#[component]
fn RootNotFound() -> Element {
    rsx! {
        NotFound { segments: Vec::new() }
    }
}
