// `asset!` in const position trips a nightly `volatile_composites` false
// positive that cannot be addressed in user code.
#![allow(clippy::volatile_composites)]

use dioxus::prelude::*;

use ui::Navbar;
mod views;
use views::{Blog, Home};

#[cfg(feature = "server")]
secretspec_derive::declare_secrets!("../../secretspec.toml");

#[derive(Debug, Clone, Routable, PartialEq, Eq)]
#[rustfmt::skip]
enum Route {
    #[layout(WebNavbar)]
    #[route("/")]
    Home {},
    #[route("/blog/:id")]
    Blog { id: i32 },
}

const FAVICON: Asset = asset!("/assets/favicon.ico");
const MAIN_CSS: Asset = asset!("/assets/main.css");

#[component]
fn App() -> Element {
    // Build cool things ✌️

    rsx! {
        // Global app resources
        document::Link { rel: "icon", href: FAVICON }
        document::Link { rel: "stylesheet", href: MAIN_CSS }

        Router::<Route> {}
    }
}

/// A web-specific Router around the shared `Navbar` component
/// which allows us to use the web-specific `Route` enum.
#[component]
fn WebNavbar() -> Element {
    rsx! {
        Navbar {
            Link {
                to: Route::Home {},
                "Home"
            }
            Link {
                to: Route::Blog { id: 1 },
                "Blog"
            }
        }

        Outlet::<Route> {}
    }
}

fn main() {
    // Run `serve()` on the server only
    #[cfg(feature = "server")]
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
        let openapi = api::sample_api_router(state);

        // .. customize the router, adding layers and new routes

        // And then return the router
        Ok(router.merge(openapi))
    });

    // When not on the server, just run `launch()` like normal
    #[cfg(not(feature = "server"))]
    dioxus::launch(App);
}
