use std::sync::{Arc, Mutex, MutexGuard};

use diesel::prelude::*;

use crate::models::UUID;
use crate::store::SampleConnection;
use crate::{Error, ErrorCode};

pub fn internal_error(detail: String) -> Error {
    Error {
        code: ErrorCode::InternalServerError,
        detail: Some(detail),
        id: uuid::Uuid::now_v7().to_string(),
        status: 500,
        title: "Internal Server Error".to_string(),
        source: None,
    }
}

pub fn not_found(sample_id: &str) -> Error {
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
pub fn parse_id(sample_id: &str) -> Result<UUID, Error> {
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
    pub db: Arc<Mutex<dyn SampleConnection>>,
}

impl AppState {
    /// Create SQLite state from an already-established connection.
    #[must_use]
    pub fn new(conn: diesel::sqlite::SqliteConnection) -> Self {
        Self::new_sqlite(conn)
    }

    /// Create SQLite state from an already-established connection.
    #[must_use]
    pub fn new_sqlite(conn: diesel::sqlite::SqliteConnection) -> Self {
        let db: Arc<Mutex<dyn SampleConnection>> = Arc::new(Mutex::new(conn));
        Self { db }
    }

    /// Create Postgres state from an already-established connection.
    #[must_use]
    pub fn new_postgres(conn: diesel::pg::PgConnection) -> Self {
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
pub fn is_postgres_url(url: &str) -> bool {
    url.starts_with("postgres://") || url.starts_with("postgresql://")
}

/// Returns `true` for SQLite `DATABASE_URL` values.
///
/// Accepts `sqlite://…`, `sqlite:…`, plain paths (`./dev.db`), and anything
/// without a `://` scheme. Anything else with a scheme (e.g. `mysql://…`) is
/// not SQLite.
pub fn is_sqlite_url(url: &str) -> bool {
    if url.starts_with("sqlite://") || url.starts_with("sqlite:") {
        return true;
    }
    !url.contains("://")
}

/// Strip the `sqlite://` / `sqlite:` prefix to a filesystem path.
pub fn sqlite_path(url: &str) -> &str {
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
        match diesel::pg::PgConnection::establish(database_url) {
            Ok(conn) => Ok(AppState::new_postgres(conn)),
            Err(err) => Err(err),
        }
    } else if is_sqlite_url(database_url) {
        let path = sqlite_path(database_url);
        match diesel::sqlite::SqliteConnection::establish(path) {
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

pub fn lock_db(
    db: &Arc<Mutex<dyn SampleConnection>>,
) -> MutexGuard<'_, dyn SampleConnection + 'static> {
    match db.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn postgres_url_detection() {
        assert!(is_postgres_url("postgres://localhost/sample"));
        assert!(is_postgres_url(
            "postgresql://postgres@localhost:5433/sample"
        ));
        assert!(is_postgres_url(
            "postgres://user:pass@host:5432/db?sslmode=require"
        ));
        assert!(!is_postgres_url("sqlite://./dev.db"));
        assert!(!is_postgres_url("./dev.db"));
        assert!(!is_postgres_url(""));
        // Matching is case-sensitive, like Diesel/libpq schemes.
        assert!(!is_postgres_url("POSTGRES://localhost/sample"));
        assert!(!is_postgres_url("postgresql"));
    }

    #[test]
    fn sqlite_url_detection() {
        assert!(is_sqlite_url("sqlite://./dev.db"));
        assert!(is_sqlite_url("sqlite://:memory:"));
        assert!(is_sqlite_url("sqlite:./dev.db"));
        assert!(is_sqlite_url("./dev.db"));
        assert!(is_sqlite_url("dev.db"));
        assert!(is_sqlite_url("/abs/path/dev.db"));
        assert!(is_sqlite_url(":memory:"));
        assert!(!is_sqlite_url("postgres://localhost/sample"));
        assert!(!is_sqlite_url("postgresql://localhost/sample"));
        assert!(!is_sqlite_url("mysql://localhost/sample"));
        assert!(!is_sqlite_url("http://localhost/sample"));
    }

    #[test]
    fn sqlite_prefix_stripping() {
        assert_eq!(sqlite_path("sqlite://./dev.db"), "./dev.db");
        assert_eq!(sqlite_path("sqlite://:memory:"), ":memory:");
        assert_eq!(sqlite_path("sqlite:./dev.db"), "./dev.db");
        // Plain paths pass through untouched.
        assert_eq!(sqlite_path("./dev.db"), "./dev.db");
        assert_eq!(sqlite_path(":memory:"), ":memory:");
    }

    #[test]
    fn url_routing_prefers_postgres_over_bare_match() {
        // A postgres URL must not be misclassified as sqlite and vice versa.
        let pg = "postgresql://postgres@localhost:5433/sample";
        assert!(is_postgres_url(pg));
        assert!(!is_sqlite_url(pg));
        let lite = "sqlite://./dev.db";
        assert!(is_sqlite_url(lite));
        assert!(!is_postgres_url(lite));
    }

    #[test]
    fn poisoned_mutex_still_serves_requests() {
        let state = AppState::new_sqlite(migrated_memory_db());
        // Poison the mutex from another thread while holding the guard.
        let db = state.db.clone();
        let handle = std::thread::spawn(move || {
            let _guard = db.lock().unwrap();
            panic!("intentional poison for lock-recovery test");
        });
        assert!(handle.join().is_err());

        // The next lock recovers via `into_inner` instead of hanging.
        assert_eq!(state.backend_name(), "sqlite");
    }

    fn migrated_memory_db() -> diesel::sqlite::SqliteConnection {
        let mut conn = diesel::sqlite::SqliteConnection::establish(":memory:")
            .expect("in-memory sqlite connects");
        crate::models::sqlite::run_sqlite_migrations(&mut conn).expect("migrations apply");
        conn
    }
}
