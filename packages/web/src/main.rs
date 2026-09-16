// `asset!` in const position trips a nightly `volatile_composites` false
// positive that cannot be addressed in user code.
#![allow(clippy::volatile_composites)]

use dioxus::prelude::*;

use ui::{Console, Docs, Navbar, THEME_CSS, components::navbar::NavbarItem};

#[cfg(feature = "server")]
secretspec_derive::declare_secrets!("../../secretspec.toml");

#[derive(Debug, Clone, Routable, PartialEq, Eq)]
#[rustfmt::skip]
enum Route {
    #[layout(WebNavbar)]
    #[route("/docs", DocsPage)]
    Docs {},
    #[route("/console", ConsolePage)]
    Console {},
}

const FAVICON: Asset = asset!("/assets/favicon.ico");
// Preloaded into base html at compile time to avoid FOUC (Dioxus #3970).
const MAIN_CSS: Asset = asset!(
    "/assets/main.css",
    CssAssetOptions::new().with_preload(true)
);

#[component]
fn App() -> Element {
    // Build cool things ✌️

    rsx! {
        // Global app resources
        document::Link { rel: "icon", href: FAVICON }
        document::Link { rel: "stylesheet", href: MAIN_CSS }
        // Theme variables for all `dioxus-components` in `ui`.
        document::Link { rel: "stylesheet", href: THEME_CSS }

        Router::<Route> {}
    }
}

/// A web-specific Router around the shared `Navbar` component
/// which allows us to use the web-specific `Route` enum.
///
/// Nav items are the styled `dioxus-components` `NavbarItem`, so keyboard
/// navigation, focus management, and active-link styling come for free.
#[component]
fn WebNavbar() -> Element {
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

/// Wraps shared `Docs` with the web-specific console target, so `ui` never
/// names this crate's `Route` enum yet `Link` still navigates client-side.
#[component]
fn DocsPage() -> Element {
    rsx! {
        Docs { console_target: Route::Console {}.into() }
    }
}

/// Wraps shared `Console` with the web-specific docs target.
#[component]
fn ConsolePage() -> Element {
    rsx! {
        Console { docs_target: Route::Docs {}.into() }
    }
}

fn main() {
    // Run `serve()` on the server only
    #[cfg(feature = "server")]
    {
        // Held for process lifetime; flushes telemetry on shutdown.
        // Telemetry degrades to stderr-only logging when no collector is up.
        let _telemetry = api::init_telemetry();
        dioxus::serve(|| async move {
            let resolved = SecretSpec::builder()
                .load()
                .map_err(|err| anyhow::anyhow!("secrets failed to load: {err}"))?;

            resolved.secrets.set_as_env_vars();
            let state = api::establish_connection()
                .map_err(|err| anyhow::anyhow!("database connection failed: {err}"))?;
            api::run_pending_migrations(&state)
                .map_err(|err| anyhow::anyhow!("database migration failed: {err}"))?;

            // Create a new router for our app using the `router` function
            let router = dioxus::server::router(App);
            let openapi = api::router_with_telemetry(state);

            // .. customize the router, adding layers and new routes

            // And then return the router
            Ok(router.merge(openapi))
        });
    }

    // When not on the server, just run `launch()` like normal
    #[cfg(not(feature = "server"))]
    dioxus::launch(App);
}
