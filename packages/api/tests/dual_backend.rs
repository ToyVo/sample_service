//! Dual-backend smoke tests: SQLite always runs; Postgres runs when reachable.
//!
//! - SQLite uses an in-memory database so tests never touch `./dev.db`.
//! - Postgres uses `TEST_POSTGRES_URL` or
//!   `postgresql://postgres@localhost:5433/sample` (see `scripts/postgres.rs`).
//!   If no server is reachable the test passes with a skip notice instead of
//!   failing, so `cargo test` stays green on machines without Postgres.

#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::unwrap_used)]

use api::{
    AppState, CodegenApi, CreateSampleResponse, GetSampleServerResponse, GetSamplesServerResponse,
    NewSample, RemoveSampleServerResponse, SampleUpdate, Status, UpdateSampleServerResponse,
};
use diesel::Connection;
use diesel::{PgConnection, SqliteConnection};

fn new_sample(name: &str) -> NewSample {
    NewSample::new(name.to_string(), Status::Active)
}

async fn exercise_backend(state: &AppState) {
    // Create.
    let CreateSampleResponse::Created(created) = state.create_sample(new_sample("smoke")).await;
    assert!(created.errors.is_empty(), "create returned errors");
    let created_id = created.data.expect("create should return data").id;

    // Get.
    let GetSampleServerResponse::Ok(got) = state.get_sample(created_id.clone()).await else {
        panic!("expected HTTP 200")
    };
    assert!(got.errors.is_empty(), "get returned errors");
    assert_eq!(got.data.expect("get should return data").id, created_id);

    // List contains the new row.
    let GetSamplesServerResponse::Ok(listed) = state.get_samples(None, None).await;
    assert!(listed.errors.is_empty(), "list returned errors");
    assert!(
        listed.data.iter().any(|s| s.id == created_id),
        "created row missing from list"
    );

    // Update.
    let update = SampleUpdate {
        name: Some("smoke-updated".to_string()),
        ..Default::default()
    };
    let UpdateSampleServerResponse::Ok(updated) =
        state.update_sample(created_id.clone(), update).await
    else {
        panic!("expected HTTP 200")
    };
    assert!(updated.errors.is_empty(), "update returned errors");
    assert_eq!(
        updated.data.expect("update should return data").name,
        "smoke-updated"
    );

    // Delete.
    let RemoveSampleServerResponse::Ok(removed) = state.remove_sample(created_id.clone()).await
    else {
        panic!("expected HTTP 200")
    };
    assert!(removed.errors.is_empty(), "delete returned errors");
    assert_eq!(
        removed.data.expect("delete should return data").id,
        created_id
    );

    // Get after delete is an HTTP 404 with a NotFound error payload
    // (data None, one error).
    let GetSampleServerResponse::NotFound(gone) = state.get_sample(created_id).await else {
        panic!("expected HTTP 404")
    };
    assert!(gone.data.is_none(), "deleted row should be gone");
    assert_eq!(gone.errors.len(), 1, "expected one NotFound error");
}

#[tokio::test]
async fn sqlite_backend_crud() {
    let mut conn =
        SqliteConnection::establish(":memory:").expect("in-memory sqlite should connect");
    api::models::sqlite::run_sqlite_migrations(&mut conn).expect("sqlite migrations should apply");
    let state = AppState::new_sqlite(conn);
    assert_eq!(state.backend_name(), "sqlite");
    exercise_backend(&state).await;
}

#[cfg(feature = "xitdb-backend")]
#[tokio::test]
async fn xitdb_backend_crud() {
    let path = std::env::temp_dir().join(format!(
        "dual-backend-xitdb-{}-{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock readable")
            .as_nanos()
    ));
    let url = format!("xitdb://{}", path.display());
    let state = api::establish_connection_from_url(&url).expect("xitdb connects");
    api::run_pending_migrations(&state).expect("xitdb needs no migrations");
    assert_eq!(state.backend_name(), "xitdb");
    exercise_backend(&state).await;
    std::fs::remove_file(&path).expect("cleanup");
}

#[tokio::test]
async fn postgres_backend_crud() {
    let url = std::env::var("TEST_POSTGRES_URL")
        .unwrap_or_else(|_| "postgresql://postgres@localhost:5433/sample".to_string());
    let mut conn = match PgConnection::establish(&url) {
        Ok(conn) => conn,
        Err(err) => {
            eprintln!("skipping postgres test (no server at {url}): {err}");
            return;
        }
    };
    if let Err(err) = api::models::postgresql::run_postgres_migrations(&mut conn) {
        eprintln!("skipping postgres test (migration failed at {url}): {err}");
        return;
    }
    let state = AppState::new_postgres(conn);
    assert_eq!(state.backend_name(), "postgresql");
    exercise_backend(&state).await;
}

#[test]
fn establish_connection_routes_by_scheme() {
    let state = api::establish_connection_from_url("sqlite://:memory:").expect("sqlite");
    assert_eq!(state.backend_name(), "sqlite");

    let state = api::establish_connection_from_url("./dev.db").expect("plain path is sqlite");
    assert_eq!(state.backend_name(), "sqlite");

    // Unknown schemes must error rather than silently picking a backend.
    assert!(
        api::establish_connection_from_url("mysql://localhost/sample").is_err(),
        "unknown scheme should error"
    );
}

#[cfg(feature = "xitdb-backend")]
#[test]
fn establish_connection_routes_xitdb() {
    let path = std::env::temp_dir().join(format!(
        "dual-backend-xitdb-routing-{}-{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock readable")
            .as_nanos()
    ));
    for url in [
        format!("xitdb://{}", path.display()),
        format!("xitdb:{}", path.display()),
    ] {
        let state = api::establish_connection_from_url(&url).expect("xitdb routes");
        assert_eq!(state.backend_name(), "xitdb");
    }
    std::fs::remove_file(&path).expect("cleanup");
}
