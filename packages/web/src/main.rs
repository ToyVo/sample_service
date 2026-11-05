// `asset!` in const position trips a nightly `volatile_composites` false
// positive that cannot be addressed in user code. The catch-all 404 route
// (`/:..segments`) never fails to parse, so the `Routable` derive's
// no-match branch is unreachable — also expected, silenced here.
#![allow(clippy::volatile_composites, unreachable_code)]

use dioxus::prelude::*;

use ui::{ApiExplorer, Login, Navbar, NotFound, THEME_CSS, components::navbar::NavbarItem};

#[cfg(feature = "server")]
secretspec_derive::declare_secrets!("../../secretspec.toml");

#[derive(Debug, Clone, Routable, PartialEq, Eq)]
#[rustfmt::skip]
enum Route {
    #[layout(WebNavbar)]
    #[route("/docs", ApiExplorerPage)]
    Docs {},
    // Alias for old bookmarks: same unified page as `/docs`.
    #[route("/console", ApiExplorerPage)]
    Console {},
    #[route("/login", LoginPage)]
    SignIn {},
    // `"/"` needs an explicit route: the `/:..segments` catch-all only
    // matches non-empty paths, so without this `"/"` falls through to
    // the SSR error path (plain-text 404) instead of `NotFound` HTML.
    #[route("/", RootNotFound)]
    Root {},
    #[route("/:..segments", NotFoundPage)]
    NotFound { segments: Vec<String> },
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
                value: "api".to_string(),
                to: Route::Docs {},
                "API"
            }
            NavbarItem {
                index: 1usize,
                value: "account".to_string(),
                to: Route::SignIn {},
                "Account"
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

/// Username/password and passkey sign-in (shared `Login`).
#[component]
fn LoginPage() -> Element {
    rsx! {
        Login {}
    }
}

/// Catch-all 404 page inside the shared navbar layout, so unknown URLs get
/// HTML (with a 404 status via `FullstackContext`) instead of plain text.
/// Unknown `/api*` never reaches here — the API router returns JSON.
#[component]
fn NotFoundPage(segments: Vec<String>) -> Element {
    rsx! {
        NotFound { segments }
    }
}

/// `"/"` has no segments for the catch-all to capture, so it renders the
/// same `NotFound` HTML explicitly (empty path → displays as `/`).
#[component]
fn RootNotFound() -> Element {
    rsx! {
        NotFound { segments: Vec::new() }
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
            let auth = api::auth::AuthState::from_env()
                .map_err(|err| anyhow::anyhow!("auth configuration invalid: {err}"))?;
            let state = api::establish_connection()
                .map_err(|err| anyhow::anyhow!("database connection failed: {err}"))?
                .with_auth(auth);
            api::run_pending_migrations(&state)
                .map_err(|err| anyhow::anyhow!("database migration failed: {err}"))?;

            // Create a new router for our app using the `router` function
            let router = dioxus::server::router(App);
            // JSON 404s for unknown `/api/*` (wildcard), HTML 404s for
            // everything else via the Dioxus SSR fallback (`NotFound` page).
            let api_router = api::router_with_telemetry(state);

            // .. customize the router, adding layers and new routes

            // And then return the router
            Ok(router.merge(api_router))
        });
    }

    // When not on the server, just run `launch()` like normal
    #[cfg(not(feature = "server"))]
    dioxus::launch(App);
}
