use dioxus::prelude::*;

use crate::components::card::{Card, CardContent, CardDescription, CardHeader, CardTitle};

/// Shared 404 page: HTML response for unknown frontend routes.
///
/// Rendered by the explicit `"/"` route (empty path) and the `/:..segments`
/// catch-all in each host app (`web` / `desktop` / `mobile`), inside the shared `Navbar` layout so users can
/// navigate back. On the server, commits a 404 status via
/// `FullstackContext` while still streaming this HTML (throwing would yield
/// plain-text instead); on the client the status call is compiled out and
/// this is just a normal page.
#[component]
pub fn NotFound(segments: Vec<String>) -> Element {
    #[cfg(feature = "server")]
    {
        dioxus::fullstack::FullstackContext::commit_http_status(
            dioxus::fullstack::StatusCode::NOT_FOUND,
            Some("Page not found".to_owned()),
        );
    }

    let path = if segments.is_empty() {
        "/".to_owned()
    } else {
        format!("/{}", segments.join("/"))
    };

    rsx! {
        document::Title { "404 — Page not found" }
        Card {
            style: "max-width: 600px; margin: 48px auto; text-align: center;",
            CardHeader {
                CardTitle { "404 — Page not found" }
                CardDescription { "No page exists at {path}" }
            }
            CardContent {
                p { "The link may be broken, or the page may have moved." }
                div {
                    style: "display: flex; gap: 16px; justify-content: center; margin-top: 16px;",
                    Link { to: "/docs", "API reference & console" }
                }
            }
        }
    }
}
