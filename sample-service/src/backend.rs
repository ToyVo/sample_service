use std::sync::{Arc, Mutex, MutexGuard};

use diesel::pg::PgConnection;
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;

use crate::models::{SampleData, UUID};
use crate::store::SampleConnection;
use crate::{
    CreateSampleResponse, CreateSampleResponse201, Error, ErrorCode, GetSampleResponse,
    GetSampleServerResponse, GetSamplesResponse, GetSamplesServerResponse, NewSample,
    RemoveSampleResponse, RemoveSampleServerResponse, SampleApi, SampleUpdate,
    UpdateSampleResponse, UpdateSampleServerResponse,
};

fn internal_error(detail: String) -> Error {
    Error {
        code: ErrorCode::InternalServerError,
        detail: Some(detail),
        id: uuid::Uuid::now_v7().to_string(),
        status: 500,
        title: "Internal Server Error".to_string(),
        source: None,
    }
}

fn not_found(sample_id: &str) -> Error {
    Error {
        code: ErrorCode::NotFound,
        detail: Some(format!("Sample {sample_id} not found")),
        id: uuid::Uuid::now_v7().to_string(),
        status: 404,
        title: "Not Found".to_string(),
        source: None,
    }
}

/// Parse a path `sample_id` into a DB [`UUID`].
///
/// # Errors
///
/// Returns a `NotFound` [`Error`] when `sample_id` is not a valid UUID.
#[allow(clippy::result_large_err)]
fn parse_id(sample_id: &str) -> Result<UUID, Error> {
    uuid::Uuid::parse_str(sample_id)
        .map(UUID)
        .map_err(|_| not_found(sample_id))
}

/// Shared application state.
///
/// Holds a single boxed [`SampleConnection`]: the concrete backend (SQLite or
/// Postgres) is chosen once at startup from `DATABASE_URL` (see
/// [`establish_connection`]), and request handlers never branch on it.
/// Diesel connections are not `Sync`, so the trait object is wrapped in a
/// `std::sync::Mutex`. Handlers lock briefly for each blocking query; no
/// `.await` is held across the lock.
///
/// A single `Mutex` connection serializes all queries. That is fine for local
/// development (`dev.db` / `dev.postgresql/`). For production, replace with a
/// pooled connection (e.g. `diesel::r2d2`).
#[derive(Clone)]
pub struct AppState {
    /// Backend-agnostic storage behind a mutex.
    db: Arc<Mutex<dyn SampleConnection>>,
}

impl AppState {
    /// Create SQLite state from an already-established connection.
    #[must_use]
    pub fn new(conn: SqliteConnection) -> Self {
        Self::new_sqlite(conn)
    }

    /// Create SQLite state from an already-established connection.
    #[must_use]
    pub fn new_sqlite(conn: SqliteConnection) -> Self {
        let db: Arc<Mutex<dyn SampleConnection>> = Arc::new(Mutex::new(conn));
        Self { db }
    }

    /// Create Postgres state from an already-established connection.
    #[must_use]
    pub fn new_postgres(conn: PgConnection) -> Self {
        let db: Arc<Mutex<dyn SampleConnection>> = Arc::new(Mutex::new(conn));
        Self { db }
    }

    /// Backend name for logging (`"sqlite"` or `"postgresql"`).
    #[must_use]
    pub fn backend_name(&self) -> &'static str {
        lock_db(&self.db).backend_name()
    }
}

/// Returns `true` for Postgres `DATABASE_URL` values.
///
/// Accepts both `postgres://…` and `postgresql://…` (case-sensitive, matching
/// Diesel / libpq conventions).
fn is_postgres_url(url: &str) -> bool {
    url.starts_with("postgres://") || url.starts_with("postgresql://")
}

/// Returns `true` for SQLite `DATABASE_URL` values.
///
/// Accepts `sqlite://…`, `sqlite:…`, plain paths (`./dev.db`), and anything
/// without a `://` scheme. Anything else with a scheme (e.g. `mysql://…`) is
/// not SQLite.
fn is_sqlite_url(url: &str) -> bool {
    if url.starts_with("sqlite://") || url.starts_with("sqlite:") {
        return true;
    }
    !url.contains("://")
}

/// Strip the `sqlite://` / `sqlite:` prefix to a filesystem path.
fn sqlite_path(url: &str) -> &str {
    url.strip_prefix("sqlite://")
        .or_else(|| url.strip_prefix("sqlite:"))
        .unwrap_or(url)
}

/// Establish a database connection from `DATABASE_URL`.
///
/// - `sqlite://./dev.db`, `sqlite:./dev.db`, `./dev.db` → SQLite.
/// - `postgres://…` / `postgresql://…` → Postgres.
/// - Unset → `sqlite://./dev.db` (matches `secretspec.toml` development default).
///
/// This is the only place that branches on the backend: the chosen
/// connection is boxed behind [`SampleConnection`] so everything downstream
/// is backend-agnostic. Run [`run_pending_migrations`] after connecting so a
/// fresh `dev.db` or fresh `dev.postgresql/` gets its `samples` table.
///
/// # Errors
///
/// Returns `diesel::ConnectionError` for unknown URL schemes or when the
/// database cannot be opened.
pub fn establish_connection() -> Result<AppState, diesel::ConnectionError> {
    let database_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://./dev.db".to_string());
    establish_connection_from_url(&database_url)
}

/// Establish a database connection from an explicit URL (same routing as
/// [`establish_connection`], without reading the environment).
///
/// Exposed for tests so routing can be checked without mutating `DATABASE_URL`.
///
/// # Errors
///
/// Returns `diesel::ConnectionError` for unknown URL schemes or when the
/// database cannot be opened.
pub fn establish_connection_from_url(
    database_url: &str,
) -> Result<AppState, diesel::ConnectionError> {
    if is_postgres_url(database_url) {
        match PgConnection::establish(database_url) {
            Ok(conn) => Ok(AppState::new_postgres(conn)),
            Err(err) => Err(err),
        }
    } else if is_sqlite_url(database_url) {
        let path = sqlite_path(database_url);
        match SqliteConnection::establish(path) {
            Ok(conn) => Ok(AppState::new_sqlite(conn)),
            Err(err) => Err(err),
        }
    } else {
        Err(diesel::ConnectionError::BadConnection(format!(
            "unsupported DATABASE_URL scheme (expected sqlite://, postgres://, or postgresql://): {database_url}"
        )))
    }
}

/// Run pending migrations for whichever backend `state` holds.
///
/// SQLite applies `migrations/sqlite`; Postgres applies
/// `migrations/postgresql` (see `diesel.sqlite.toml` /
/// `diesel.postgresql.toml`).
///
/// # Errors
///
/// Returns the migration error if any pending migration fails.
pub fn run_pending_migrations(
    state: &AppState,
) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    lock_db(&state.db).run_migrations()
}

fn lock_db(
    db: &Arc<Mutex<dyn SampleConnection>>,
) -> MutexGuard<'_, dyn SampleConnection + 'static> {
    match db.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

// ---------------------------------------------------------------------------
// `SampleApi`: each method locks the connection, calls the single
// backend-agnostic operation in `crate::store`, and converts the domain
// result (`Ok(data)` / `Ok(None)` / `Err(detail)`) into the response shape.
// Response construction lives here; the store never sees response types.
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl SampleApi for AppState {
    async fn create_sample(&self, body: NewSample) -> CreateSampleResponse {
        let mut conn = lock_db(&self.db);
        match crate::store::create_sample(&mut *conn, body) {
            Ok(data) => CreateSampleResponse::Created(CreateSampleResponse201 {
                data: Some(data.to_api()),
                errors: Vec::new(),
            }),
            Err(detail) => CreateSampleResponse::Created(CreateSampleResponse201 {
                data: None,
                errors: vec![internal_error(detail)],
            }),
        }
    }

    async fn remove_sample(&self, sample_id: String) -> RemoveSampleServerResponse {
        let db_id = match parse_id(&sample_id) {
            Ok(id) => id,
            Err(err) => {
                return RemoveSampleServerResponse::Ok(RemoveSampleResponse {
                    data: None,
                    errors: vec![err],
                });
            }
        };
        let mut conn = lock_db(&self.db);
        match crate::store::remove_sample(&mut *conn, db_id) {
            Ok(Some(data)) => RemoveSampleServerResponse::Ok(RemoveSampleResponse {
                data: Some(data.to_api()),
                errors: Vec::new(),
            }),
            Ok(None) => RemoveSampleServerResponse::Ok(RemoveSampleResponse {
                data: None,
                errors: vec![not_found(&sample_id)],
            }),
            Err(detail) => RemoveSampleServerResponse::Ok(RemoveSampleResponse {
                data: None,
                errors: vec![internal_error(detail)],
            }),
        }
    }

    async fn update_sample(
        &self,
        sample_id: String,
        body: SampleUpdate,
    ) -> UpdateSampleServerResponse {
        let db_id = match parse_id(&sample_id) {
            Ok(id) => id,
            Err(err) => {
                return UpdateSampleServerResponse::Ok(UpdateSampleResponse {
                    data: None,
                    errors: vec![err],
                });
            }
        };
        let mut conn = lock_db(&self.db);
        match crate::store::update_sample(&mut *conn, db_id, body) {
            Ok(Some(data)) => UpdateSampleServerResponse::Ok(UpdateSampleResponse {
                data: Some(data.to_api()),
                errors: Vec::new(),
            }),
            Ok(None) => UpdateSampleServerResponse::Ok(UpdateSampleResponse {
                data: None,
                errors: vec![not_found(&sample_id)],
            }),
            Err(detail) => UpdateSampleServerResponse::Ok(UpdateSampleResponse {
                data: None,
                errors: vec![internal_error(detail)],
            }),
        }
    }

    async fn get_sample(&self, sample_id: String) -> GetSampleServerResponse {
        let db_id = match parse_id(&sample_id) {
            Ok(id) => id,
            Err(err) => {
                return GetSampleServerResponse::Ok(GetSampleResponse {
                    data: None,
                    errors: vec![err],
                });
            }
        };
        let mut conn = lock_db(&self.db);
        match crate::store::get_sample(&mut *conn, db_id) {
            Ok(Some(data)) => GetSampleServerResponse::Ok(GetSampleResponse {
                data: Some(data.to_api()),
                errors: Vec::new(),
            }),
            Ok(None) => GetSampleServerResponse::Ok(GetSampleResponse {
                data: None,
                errors: vec![not_found(&sample_id)],
            }),
            Err(detail) => GetSampleServerResponse::Ok(GetSampleResponse {
                data: None,
                errors: vec![internal_error(detail)],
            }),
        }
    }

    async fn get_samples(
        &self,
        offset: Option<i64>,
        limit: Option<i64>,
    ) -> GetSamplesServerResponse {
        let offset = offset.unwrap_or(0).max(0);
        let limit = limit.unwrap_or(100).clamp(1, 1000);
        let mut conn = lock_db(&self.db);
        match crate::store::get_samples(&mut *conn, offset, limit) {
            Ok(rows) => {
                let data = rows.into_iter().map(SampleData::to_api).collect();
                GetSamplesServerResponse::Ok(GetSamplesResponse {
                    data,
                    errors: Vec::new(),
                })
            }
            Err(detail) => GetSamplesServerResponse::Ok(GetSamplesResponse {
                data: Vec::new(),
                errors: vec![internal_error(detail)],
            }),
        }
    }
}
