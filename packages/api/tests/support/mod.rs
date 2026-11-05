//! Shared helpers for integration tests.
//!
//! Every test file declares `mod support;` to use these. SQLite tests use a
//! fresh `:memory:` database (fully isolated). Postgres tests use a dedicated
//! database per test (`sample_t_<name>`, recreated on setup) so parallel
//! tests never observe each other's rows and reruns start clean.

use api::{AppState, NewSample, Status};
use diesel::{Connection as _, PgConnection, RunQueryDsl as _, SqliteConnection};

/// Maintenance database used for `CREATE`/`DROP DATABASE`.
fn maintenance_url() -> String {
    std::env::var("TEST_PG_MAINTENANCE_URL")
        .unwrap_or_else(|_| "postgresql://postgres@localhost:5433/postgres".to_string())
}

/// Fresh in-memory SQLite state with migrations applied.
#[must_use]
pub fn sqlite_state() -> AppState {
    let mut conn = SqliteConnection::establish(":memory:").expect("in-memory sqlite connects");
    api::models::sqlite::run_sqlite_migrations(&mut conn).expect("sqlite migrations apply");
    AppState::new_sqlite(conn)
}

/// Guard for an isolated Postgres test database. Call [`PgGuard::cleanup`]
/// at the end of the test to drop the database.
pub struct PgGuard {
    db_name: String,
    state: AppState,
}

impl PgGuard {
    /// Postgres state backed by a fresh `sample_t_<test>` database, or `None`
    /// with a skip notice when no server is reachable.
    #[must_use]
    pub fn setup(test: &str) -> Option<Self> {
        let db_name = format!("sample_t_{test}");
        let mut admin = match PgConnection::establish(&maintenance_url()) {
            Ok(conn) => conn,
            Err(err) => {
                eprintln!("skipping postgres test (no server): {err}");
                return None;
            }
        };
        // `DROP IF EXISTS` first so reruns start clean even if a previous
        // cleanup was skipped by a panic.
        for statement in [
            format!(r#"DROP DATABASE IF EXISTS "{db_name}""#),
            format!(r#"CREATE DATABASE "{db_name}""#),
        ] {
            if let Err(err) = diesel::sql_query(statement).execute(&mut admin) {
                eprintln!("skipping postgres test (setup failed): {err}");
                return None;
            }
        }
        let url = format!("postgresql://postgres@localhost:5433/{db_name}");
        let state = match api::establish_connection_from_url(&url) {
            Ok(state) => state,
            Err(err) => {
                eprintln!("skipping postgres test (connect failed): {err}");
                return None;
            }
        };
        if let Err(err) = api::run_pending_migrations(&state) {
            eprintln!("skipping postgres test (migration failed): {err}");
            return None;
        }
        Some(Self { db_name, state })
    }

    /// Borrow the test state.
    #[must_use]
    pub fn state(&self) -> &AppState {
        &self.state
    }

    /// Drop the test database. Best-effort: failures are reported, not fatal.
    pub fn cleanup(self) {
        drop(self.state);
        match PgConnection::establish(&maintenance_url()) {
            Ok(mut admin) => {
                let statement = format!(r#"DROP DATABASE IF EXISTS "{}""#, self.db_name);
                if let Err(err) = diesel::sql_query(statement).execute(&mut admin) {
                    eprintln!("test database cleanup failed: {err}");
                }
            }
            Err(err) => eprintln!("test database cleanup failed: {err}"),
        }
    }
}

/// Guard for an isolated xitdb test database file. Always available
/// (file-backed, no server needed).
#[cfg(feature = "xitdb-backend")]
pub struct XitdbGuard {
    path: std::path::PathBuf,
    state: AppState,
}

#[cfg(feature = "xitdb-backend")]
impl XitdbGuard {
    /// xitdb state on a fresh temp file, plus edge-triggered migrations
    /// (a no-op proving the path works end to end).
    #[must_use]
    pub fn setup(test: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "sample_t_{test}-{}-{}.db",
            std::process::id(),
            // Unique within a process without shared state.
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock readable")
                .as_nanos()
        ));
        let url = format!("xitdb://{}", path.display());
        let state = api::establish_connection_from_url(&url).expect("xitdb connects");
        api::run_pending_migrations(&state).expect("xitdb needs no migrations");
        Self { path, state }
    }

    /// Borrow the test state.
    #[must_use]
    pub fn state(&self) -> &AppState {
        &self.state
    }

    /// Remove the test database file. Best-effort: failures are reported,
    /// not fatal.
    pub fn cleanup(self) {
        drop(self.state);
        if let Err(err) = std::fs::remove_file(&self.path) {
            eprintln!("test database cleanup failed: {err}");
        }
    }
}

/// Run `$block` (with `$state: &AppState` bound) against SQLite and xitdb
/// always, plus an isolated Postgres database when a server is reachable.
/// `$db` names the Postgres test database (`sample_t_<$db>`).
#[macro_export]
macro_rules! on_all_backends {
    ($db:literal, |$state:ident| $block:block) => {{
        {
            let __owned = $crate::support::sqlite_state();
            let $state = &__owned;
            $block
        }
        #[cfg(feature = "xitdb-backend")]
        {
            let __guard = $crate::support::XitdbGuard::setup($db);
            {
                let $state = __guard.state();
                $block
            }
            __guard.cleanup();
        }
        if let Some(guard) = $crate::support::PgGuard::setup($db) {
            {
                let $state = guard.state();
                $block
            }
            guard.cleanup();
        }
    }};
}

/// A fully-populated `NewSample` with a name-derived description.
///
/// Not every test target uses this helper, so dead code is allowed here.
#[allow(dead_code)]
#[must_use]
pub fn full_sample(name: &str) -> NewSample {
    NewSample::builder(name.to_string(), Status::Inactive)
        .description(format!("{name} description"))
        .image_uri("https://example.com/img.png".to_string())
        .tags(vec!["one".to_string(), "two".to_string()])
        .build()
}
