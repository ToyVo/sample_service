//! Connection-establishment and migration coverage.
//!
//! Proves a fresh database degrades gracefully until migrations run,
//! migrations are idempotent, URL routing rejects unknown schemes with a
//! helpful message, and unreachable hosts fail fast.

#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::unwrap_used)]

mod support;

use api::{AppState, CodegenApi, ErrorCode, GetSampleServerResponse, GetSamplesServerResponse};
use diesel::Connection;

#[tokio::test]
async fn unmigrated_database_degrades_to_internal_errors() {
    let state = AppState::new(diesel::SqliteConnection::establish(":memory:").expect("connects"));
    // No migrations ran: every operation reports an internal error payload
    // instead of panicking or hanging.
    let GetSamplesServerResponse::Ok(listed) = state.get_samples(None, None).await;
    assert!(listed.data.is_empty());
    assert_eq!(listed.errors.len(), 1);
    let error = listed.errors.first().expect("error");
    assert_eq!(error.code, ErrorCode::InternalServerError);
    assert_eq!(error.status, 500);

    let missing = uuid::Uuid::now_v7().to_string();
    let GetSampleServerResponse::Ok(got) = state.get_sample(missing).await else {
        panic!("expected HTTP 200")
    };
    assert!(got.data.is_none());
    assert_eq!(got.errors.len(), 1);
    assert_eq!(got.errors.first().expect("error").status, 500);

    // Migrating afterwards heals the database without reconnecting.
    api::run_pending_migrations(&state).expect("migrations apply");
    let GetSamplesServerResponse::Ok(healed) = state.get_samples(None, None).await;
    assert!(healed.errors.is_empty());
    assert!(healed.data.is_empty());
}

#[test]
fn migrations_are_idempotent_on_all_backends() {
    api::run_pending_migrations(&support::sqlite_state()).expect("sqlite re-migration");
    if let Some(guard) = support::PgGuard::setup("migrations") {
        let state = guard.state();
        api::run_pending_migrations(state).expect("postgres re-migration");
        api::run_pending_migrations(state).expect("third run");
        guard.cleanup();
    }
}

#[test]
fn unknown_scheme_errors_with_helpful_message() {
    let err = match api::establish_connection_from_url("mysql://localhost/sample") {
        Ok(_) => panic!("unknown scheme must fail"),
        Err(err) => err,
    };
    match err {
        diesel::ConnectionError::BadConnection(message) => {
            assert!(
                message.contains("mysql://localhost/sample"),
                "message echoes the url: {message}"
            );
            assert!(message.contains("sqlite://"), "message hints sqlite");
            assert!(message.contains("postgresql://"), "message hints postgres");
        }
        other => panic!("expected BadConnection, got: {other:?}"),
    }
}

#[test]
fn unreachable_postgres_fails_fast() {
    // Port 1 is never a database; connection refusal must surface as Err.
    assert!(api::establish_connection_from_url("postgresql://127.0.0.1:1/sample").is_err());
    assert!(api::establish_connection_from_url("postgres://127.0.0.1:1/sample").is_err());
}

#[test]
fn sqlite_memory_urls_connect_without_files() {
    for url in ["sqlite://:memory:", "sqlite::memory:", ":memory:"] {
        let state =
            api::establish_connection_from_url(url).unwrap_or_else(|_| panic!("connects: {url}"));
        assert_eq!(state.backend_name(), "sqlite");
    }
}

#[test]
fn sqlite_plain_path_creates_a_file() {
    // Plain filesystem paths route to SQLite and create the file on demand.
    // Uses the OS temp dir (never the repo) with a pid-unique name so
    // parallel runners cannot collide.
    let path = std::env::temp_dir().join(format!("sample-routing-{}.db", std::process::id()));
    let url = path.to_str().expect("temp path is utf-8").to_string();
    let state = api::establish_connection_from_url(&url).expect("plain path connects");
    assert_eq!(state.backend_name(), "sqlite");
    drop(state);
    assert!(path.exists(), "database file was created");
    std::fs::remove_file(&path).expect("cleanup temp database");
}

#[test]
fn new_is_a_sqlite_alias() {
    let state = AppState::new(diesel::SqliteConnection::establish(":memory:").expect("connects"));
    assert_eq!(state.backend_name(), "sqlite");
}
