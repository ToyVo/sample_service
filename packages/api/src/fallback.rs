//! Transport-level 404 handling: JSON for `/api/*`, HTML otherwise.
//!
//! Domain-level 404s (unknown sample IDs) are HTTP 404 responses carrying
//! the standard `data`/`errors` envelope (see [`crate::not_found`]). These handlers cover
//! unknown *routes*: Axum found no matching route.
//!
//! In fullstack (`web`), the API router contributes a wildcard
//! `/api/{*rest}` route returning [`api_not_found`] (JSON), while the Dioxus
//! SSR fallback renders the frontend `NotFound` page (HTML) for everything
//! else. [`fallback`] (branching on the `/api` prefix) is for standalone API
//! servers and tests without Dioxus.

use axum::{
    Json,
    http::{HeaderValue, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};

use crate::ProblemDetails;

/// JSON 404 for unknown `/api/*` routes.
///
/// Uses the same `application/problem+json` envelope as request-validation
/// rejections so API clients handle all transport errors uniformly.
#[allow(clippy::unused_async)]
pub async fn api_not_found(uri: Uri) -> Response {
    tracing::warn!(path = uri.path(), "API route not found");
    let body = ProblemDetails {
        r#type: "https://openapi-to-rust.dev/problems/not-found".to_owned(),
        title: "Not Found".to_owned(),
        status: 404,
        code: "not_found".to_owned(),
        errors: Vec::new(),
    };
    let mut response = (StatusCode::NOT_FOUND, Json(body)).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/problem+json"),
    );
    response
}

/// HTML 404 for unknown non-API routes (standalone API servers).
///
/// Fullstack servers should prefer Dioxus SSR (which renders the shared
/// `ui::NotFound` page with a 404 status) for these paths; this static page
/// keeps bare-API deployments and integration tests HTML-capable without
/// pulling in the frontend.
#[allow(clippy::unused_async)]
pub async fn html_not_found(uri: Uri) -> Response {
    let path = uri.path().to_owned();
    let safe = escape_html(&path);
    let html = format!(
        "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <title>404 Not Found</title></head><body style=\"font-family: sans-serif; \
         max-width: 600px; margin: 48px auto; text-align: center;\">\
         <h1>404 — Page not found</h1>\
         <p>No page exists at {safe}.</p>\
         <p><a href=\"/docs\">API reference & console</a></p>\
         </body></html>"
    );
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        html,
    )
        .into_response()
}

/// Branching fallback: JSON for `/api*`, HTML otherwise.
///
/// Use as `Router::fallback(fallback)` on standalone API servers. Fullstack
/// servers should *not* use this (it would replace the Dioxus SSR fallback);
/// instead they rely on the `/api/{*rest}` wildcard (JSON) plus Dioxus SSR
/// (HTML) — see [`crate::router_with_telemetry`].
pub async fn fallback(uri: Uri) -> Response {
    if is_api_path(uri.path()) {
        api_not_found(uri).await
    } else {
        html_not_found(uri).await
    }
}

fn is_api_path(path: &str) -> bool {
    path == "/api" || path.starts_with("/api/")
}

fn escape_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn api_paths_detected() {
        assert!(is_api_path("/api"));
        assert!(is_api_path("/api/"));
        assert!(is_api_path("/api/samples"));
        assert!(is_api_path("/api/samples/123"));
        assert!(!is_api_path("/"));
        assert!(!is_api_path("/docs"));
        assert!(!is_api_path("/console"));
        assert!(!is_api_path("/apix"));
    }

    #[test]
    fn html_is_escaped() {
        assert_eq!(escape_html("/a<b>&\"'c"), "/a&lt;b&gt;&amp;&quot;&#x27;c");
        assert_eq!(escape_html("/docs"), "/docs");
    }
}
