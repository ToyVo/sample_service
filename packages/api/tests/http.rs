//! End-to-end HTTP coverage: the generated router over a live socket.
//!
//! Locks in the transport contract: success codes (201 for create, 200
//! otherwise), domain-level 404s as HTTP 404 with the same `data`/`errors`
//! envelope, and request validation rejections (422 schema violations with
//! `application/problem+json` bodies, 400 malformed requests, 405 wrong
//! method, 404 unknown route).

#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::unwrap_used)]

mod support;

use api::{
    AppState, CreateSampleResponse201, GetSampleResponse, GetSampleResponse404, GetSamplesResponse,
    RemoveSampleResponse, UpdateSampleResponse,
};
use std::net::SocketAddr;

async fn serve(state: AppState) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    // Standalone API server: wildcard JSON 404s for `/api/*` plus the
    // branching fallback (JSON for `/api*`, HTML otherwise) so non-API
    // 404s are HTML even without Dioxus SSR. Fullstack (`web`) merges the
    // same routers with `dioxus::server::router` instead: `/api/*` still
    // hits the wildcard (JSON), everything else hits SSR (`NotFound` HTML).
    let router = api::router_with_telemetry(state).fallback(api::fallback);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("binds ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, router).await.expect("serves");
    });
    (addr, handle)
}

fn post_json(
    client: &reqwest::Client,
    url: String,
    body: serde_json::Value,
) -> reqwest::RequestBuilder {
    client
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(serde_json::to_string(&body).expect("serializes"))
}

async fn read<T>(response: reqwest::Response) -> (reqwest::StatusCode, T)
where
    T: serde::de::DeserializeOwned,
{
    let status = response.status();
    let text = response.text().await.expect("reads body");
    let parsed: T = serde_json::from_str(&text).expect("parses");
    (status, parsed)
}

#[derive(Debug, serde::Deserialize)]
struct Problem {
    title: String,
    status: u16,
    code: String,
}

async fn read_problem(response: reqwest::Response) -> (reqwest::StatusCode, String, Problem) {
    let status = response.status();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .expect("content type")
        .to_str()
        .expect("header str")
        .to_string();
    let text = response.text().await.expect("reads body");
    let problem: Problem = serde_json::from_str(&text).expect("parses problem");
    (status, content_type, problem)
}

#[tokio::test]
async fn full_crud_flow_over_http() {
    crate::on_all_backends!("db_http_crud", |state| {
        let (addr, server) = serve(state.clone()).await;
        let base = format!("http://{addr}");
        let client = reqwest::Client::new();

        // Create.
        let (status, created): (_, CreateSampleResponse201) = read(
            post_json(
                &client,
                format!("{base}/api/samples"),
                serde_json::json!({"name": "http", "status": "ACTIVE", "tags": ["web"]}),
            )
            .send()
            .await
            .expect("posted"),
        )
        .await;
        assert_eq!(status, reqwest::StatusCode::CREATED);
        assert!(created.errors.is_empty());
        let id = created.data.expect("created").id;

        // Read back.
        let (status, got): (_, GetSampleResponse) = read(
            client
                .get(format!("{base}/api/samples/{id}"))
                .send()
                .await
                .expect("got"),
        )
        .await;
        assert_eq!(status, reqwest::StatusCode::OK);
        assert_eq!(got.data.expect("row").name, "http");

        // List contains it.
        let (status, listed): (_, GetSamplesResponse) = read(
            client
                .get(format!("{base}/api/samples"))
                .send()
                .await
                .expect("listed"),
        )
        .await;
        assert_eq!(status, reqwest::StatusCode::OK);
        assert!(listed.data.iter().any(|row| row.id == id));

        // Patch.
        let (status, patched): (_, UpdateSampleResponse) = read(
            client
                .patch(format!("{base}/api/samples/{id}"))
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(r#"{"name":"http-2","status":"ARCHIVED"}"#)
                .send()
                .await
                .expect("patched"),
        )
        .await;
        assert_eq!(status, reqwest::StatusCode::OK);
        assert_eq!(patched.data.expect("row").name, "http-2");

        // Delete returns the row; a second read is an HTTP 404 with the same
        // envelope shape (data None, one error).
        let (status, removed): (_, RemoveSampleResponse) = read(
            client
                .delete(format!("{base}/api/samples/{id}"))
                .send()
                .await
                .expect("deleted"),
        )
        .await;
        assert_eq!(status, reqwest::StatusCode::OK);
        assert_eq!(removed.data.expect("row").id, id);
        let (status, gone): (_, GetSampleResponse404) = read(
            client
                .get(format!("{base}/api/samples/{id}"))
                .send()
                .await
                .expect("got"),
        )
        .await;
        assert_eq!(status, reqwest::StatusCode::NOT_FOUND);
        assert!(gone.data.is_none());
        assert_eq!(gone.errors.len(), 1);
        assert_eq!(gone.errors.first().expect("error").status, 404);
        server.abort();
    });
}

#[tokio::test]
async fn unknown_id_is_404_with_error_payload() {
    crate::on_all_backends!("db_http_unknown", |state| {
        let (addr, server) = serve(state.clone()).await;
        let client = reqwest::Client::new();
        let missing = uuid::Uuid::now_v7().to_string();
        let (status, body): (_, GetSampleResponse404) = read(
            client
                .get(format!("http://{addr}/api/samples/{missing}"))
                .send()
                .await
                .expect("got"),
        )
        .await;
        // Missing rows are genuine transport 404s carrying the standard envelope.
        assert_eq!(status, reqwest::StatusCode::NOT_FOUND);
        assert!(body.data.is_none());
        assert_eq!(body.errors.len(), 1);
        assert_eq!(body.errors.first().expect("error").status, 404);
        server.abort();
    });
}

#[tokio::test]
async fn request_validation_rejections() {
    crate::on_all_backends!("db_http_validation", |state| {
        let (addr, server) = serve(state.clone()).await;
        let base = format!("http://{addr}");
        let client = reqwest::Client::new();

        // `sampleId` shorter than minLength 5 → 422 with a problem body.
        let (status, content_type, problem) = read_problem(
            client
                .get(format!("{base}/api/samples/abc"))
                .send()
                .await
                .expect("got"),
        )
        .await;
        assert_eq!(status, reqwest::StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            content_type.contains("application/problem+json"),
            "content type: {content_type}"
        );
        assert_eq!(problem.status, 422);
        assert_eq!(problem.title, "Request validation failed");
        assert_eq!(problem.code, "request_validation_failed");

        // `sampleId` longer than maxLength 36 → 422 as well.
        let long_id: String = std::iter::repeat_n('a', 40).collect();
        let response = client
            .get(format!("{base}/api/samples/{long_id}"))
            .send()
            .await
            .expect("got");
        assert_eq!(response.status(), reqwest::StatusCode::UNPROCESSABLE_ENTITY);

        // Missing required `name` → 422.
        let response = post_json(
            &client,
            format!("{base}/api/samples"),
            serde_json::json!({"status": "ACTIVE"}),
        )
        .send()
        .await
        .expect("posted");
        assert_eq!(response.status(), reqwest::StatusCode::UNPROCESSABLE_ENTITY);

        // Unknown `status` enum variant → 422.
        let response = post_json(
            &client,
            format!("{base}/api/samples"),
            serde_json::json!({"name": "x", "status": "BOGUS"}),
        )
        .send()
        .await
        .expect("posted");
        assert_eq!(response.status(), reqwest::StatusCode::UNPROCESSABLE_ENTITY);

        // Unparseable JSON → 400 malformed.
        let response = client
            .post(format!("{base}/api/samples"))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body("{oops")
            .send()
            .await
            .expect("posted");
        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);

        // Out-of-range pagination → 422 (minimum/maximum).
        for query in ["limit=0", "limit=5000", "offset=-3", "limit=abc"] {
            let response = client
                .get(format!("{base}/api/samples?{query}"))
                .send()
                .await
                .expect("got");
            let expected = if query == "limit=abc" {
                // Unparseable scalar is malformed, not a range violation.
                reqwest::StatusCode::BAD_REQUEST
            } else {
                reqwest::StatusCode::UNPROCESSABLE_ENTITY
            };
            assert_eq!(response.status(), expected, "query: {query}");
        }

        // Wrong method and unknown route surface transport-level codes.
        let response = client
            .put(format!("{base}/api/samples"))
            .send()
            .await
            .expect("put");
        assert_eq!(response.status(), reqwest::StatusCode::METHOD_NOT_ALLOWED);
        // Unknown `/api/*` → JSON 404 (`application/problem+json`).
        let response = client
            .get(format!("{base}/api/nope"))
            .send()
            .await
            .expect("got");
        assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .expect("content type")
            .to_str()
            .expect("header str")
            .to_owned();
        assert!(
            content_type.contains("application/problem+json"),
            "api 404 is json: {content_type}"
        );
        let text = response.text().await.expect("reads body");
        let problem: Problem = serde_json::from_str(&text).expect("parses problem");
        assert_eq!(problem.status, 404);
        assert_eq!(problem.code, "not_found");
        // Unknown non-API route → HTML 404.
        let response = client
            .get(format!("{base}/nope"))
            .send()
            .await
            .expect("got");
        assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .expect("content type")
            .to_str()
            .expect("header str")
            .to_owned();
        assert!(
            content_type.contains("text/html"),
            "non-api 404 is html: {content_type}"
        );
        // `"/"` is HTML territory as well (fullstack renders `NotFound`).
        let response = client.get(format!("{base}/")).send().await.expect("got");
        assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .expect("content type")
            .to_str()
            .expect("header str")
            .to_owned();
        assert!(
            content_type.contains("text/html"),
            "root 404 is html: {content_type}"
        );
        server.abort();
    });
}

#[tokio::test]
async fn crud_flow_over_http_against_postgres() {
    let Some(guard) = support::PgGuard::setup("http_pg") else {
        eprintln!("skipping postgres http test (no server)");
        return;
    };
    // Reuse one AppState across the whole flow: clone the guarded state.
    let (addr, server) = serve(guard.state().clone()).await;
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();
    let (status, created): (_, CreateSampleResponse201) = read(
        post_json(
            &client,
            format!("{base}/api/samples"),
            serde_json::json!({"name": "pg-http", "status": "INACTIVE"}),
        )
        .send()
        .await
        .expect("posted"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::CREATED);
    let id = created.data.expect("created").id;
    let (status, got): (_, GetSampleResponse) = read(
        client
            .get(format!("{base}/api/samples/{id}"))
            .send()
            .await
            .expect("got"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(got.data.expect("row").name, "pg-http");
    server.abort();
    guard.cleanup();
}

#[tokio::test]
async fn docs_pages_serve() {
    crate::on_all_backends!("db_http_docs", |state| {
        let (addr, server) = serve(state.clone()).await;
        let base = format!("http://{addr}");
        let client = reqwest::Client::new();

        // The raw spec for tooling.
        let response = client
            .get(format!("{base}/api"))
            .send()
            .await
            .expect("got spec");
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .expect("content type")
            .to_str()
            .expect("header str")
            .to_owned();
        assert!(
            content_type.contains("application/json"),
            "spec is json: {content_type}"
        );
        let text = response.text().await.expect("reads body");
        let spec: serde_json::Value = serde_json::from_str(&text).expect("spec parses");
        let title = spec
            .get("info")
            .and_then(|info| info.get("title"))
            .and_then(serde_json::Value::as_str)
            .expect("title");
        assert_eq!(title, "Samples");
        assert!(
            spec.get("paths")
                .and_then(|paths| paths.get("/api/samples"))
                .is_some(),
            "spec documents /api/samples"
        );

        // NOTE: `/console` is a Dioxus route (see the `web` crate), so it is not
        // mounted on this bare API router; its request-building logic is covered
        // by unit tests in `packages/web/src/views/console.rs`.
        server.abort();
    });
}
