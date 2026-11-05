//! Backend-agnostic sample storage.
//!
//! [`SampleConnection`] abstracts over `SqliteConnection` and `PgConnection`
//! so each CRUD operation is implemented exactly once against `dyn
//! SampleConnection` (see [`create_sample`], [`get_sample`],
//! [`get_samples`], [`update_sample`], [`remove_sample`]). The only
//! per-backend code is the trait implementation itself, generated once by
//! [`impl_sample_connection`] for each backend. [`crate::backend::AppState`]
//! holds a single boxed [`SampleConnection`], so request handlers never
//! branch on the backend.
//!
//! Functions here speak domain types ([`SampleData`], `String` details) and
//! never build HTTP responses: `Ok(None)` means "row not found" for
//! single-row operations, `Err(detail)` means an internal failure. Response
//! construction is [`crate::backend`]'s responsibility.

use diesel::prelude::*;

use crate::auth::AuthConnection;
use crate::models::common::{SampleData, UUID};
use crate::{NewSample, SampleUpdate};

/// Storage failure shared by every backend.
///
/// Diesel backends convert via [`From`]; other backends (e.g. xitdb) map
/// their own errors onto the same three cases, so callers never match on
/// backend-specific types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// Nothing matched (row, user, key, or cursor target).
    NotFound,
    /// Insert would violate uniqueness (username, credential id).
    Conflict(String),
    /// Anything else; the detail string preserves the backend's message.
    Backend(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => formatter.write_str("not found"),
            Self::Conflict(detail) | Self::Backend(detail) => formatter.write_str(detail),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<diesel::result::Error> for StoreError {
    fn from(err: diesel::result::Error) -> Self {
        match err {
            diesel::result::Error::NotFound => Self::NotFound,
            diesel::result::Error::DatabaseError(
                diesel::result::DatabaseErrorKind::UniqueViolation,
                info,
            ) => Self::Conflict(info.message().to_string()),
            _ => Self::Backend(err.to_string()),
        }
    }
}

/// Storage primitives every backend connection must provide.
///
/// Methods work on the backend-agnostic [`SampleData`]; each implementation
/// translates to its own Diesel table/row types. Business logic (response
/// mapping, `NotFound` handling) lives in the functions below, so it is
/// written once for all backends.
///
/// The trait is object-safe (no generics, no `Self` in return position),
/// which lets [`crate::backend::AppState`] hold a single
/// `Arc<Mutex<dyn SampleConnection>>` instead of an enum. Account and session
/// storage comes along via the [`AuthConnection`] supertrait.
pub trait SampleConnection: AuthConnection + Send {
    /// Backend name for logging (`"sqlite"` or `"postgresql"`).
    fn backend_name(&self) -> &'static str;

    /// Run pending migrations for this backend's migration directory.
    ///
    /// # Errors
    ///
    /// Returns the migration error if any pending migration fails.
    fn run_migrations(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>>;
    /// Insert a new row. `data` carries the final field values (id and
    /// timestamps are assigned by the caller).
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when the insert fails.
    fn insert_sample(&mut self, data: SampleData) -> Result<(), StoreError>;

    /// Fetch one row by id.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::NotFound`] when no row matches.
    fn fetch_sample(&mut self, id: UUID) -> Result<SampleData, StoreError>;

    /// Fetch a page of rows ordered by `created_at` ascending.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when the query fails.
    fn fetch_samples(&mut self, offset: i64, limit: i64) -> Result<Vec<SampleData>, StoreError>;

    /// Overwrite the mutable fields of the row with `data.id` and return the
    /// refreshed row. `created_at` is preserved (updates never change it).
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when the update or refresh fails.
    fn save_sample(&mut self, data: SampleData) -> Result<SampleData, StoreError>;

    /// Delete the row with `id`. Callers fetch first when they need
    /// `NotFound` mapping or the deleted row.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when the delete fails.
    fn delete_sample(&mut self, id: UUID) -> Result<(), StoreError>;
}

/// Generate a [`SampleConnection`] implementation from a schema module and
/// row type. Invoked once per backend so the Diesel query logic below is the
/// single source of truth for all backends.
macro_rules! impl_sample_connection {
    ($conn:ty, $schema:ident, $row:ty, $name:expr, $migrate:path) => {
        impl SampleConnection for $conn {
            fn backend_name(&self) -> &'static str {
                $name
            }

            fn run_migrations(
                &mut self,
            ) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
                $migrate(self)
            }
            fn insert_sample(&mut self, data: SampleData) -> Result<(), StoreError> {
                let row = <$row>::from(data);
                diesel::insert_into(crate::schemas::$schema::samples::table)
                    .values(&row)
                    .execute(self)
                    .map(|_| ())
                    .map_err(StoreError::from)
            }

            fn fetch_sample(&mut self, id: UUID) -> Result<SampleData, StoreError> {
                crate::schemas::$schema::samples::table
                    .filter(crate::schemas::$schema::samples::id.eq(id))
                    .select(<$row>::as_select())
                    .first(self)
                    .map(SampleData::from)
                    .map_err(StoreError::from)
            }

            fn fetch_samples(
                &mut self,
                offset: i64,
                limit: i64,
            ) -> Result<Vec<SampleData>, StoreError> {
                let rows: Vec<$row> = crate::schemas::$schema::samples::table
                    .order(crate::schemas::$schema::samples::created_at.asc())
                    .offset(offset)
                    .limit(limit)
                    .select(<$row>::as_select())
                    .load(self)?;
                Ok(rows.into_iter().map(SampleData::from).collect())
            }

            fn save_sample(&mut self, data: SampleData) -> Result<SampleData, StoreError> {
                let id = data.id;
                let row = <$row>::from(data);
                diesel::update(
                    crate::schemas::$schema::samples::table
                        .filter(crate::schemas::$schema::samples::id.eq(id)),
                )
                .set((
                    crate::schemas::$schema::samples::name.eq(row.name),
                    crate::schemas::$schema::samples::description.eq(row.description),
                    crate::schemas::$schema::samples::status.eq(row.status),
                    crate::schemas::$schema::samples::tags.eq(row.tags),
                    crate::schemas::$schema::samples::image_uri.eq(row.image_uri),
                    crate::schemas::$schema::samples::updated_at.eq(row.updated_at),
                ))
                .execute(self)?;
                self.fetch_sample(id).map_err(StoreError::from)
            }

            fn delete_sample(&mut self, id: UUID) -> Result<(), StoreError> {
                diesel::delete(
                    crate::schemas::$schema::samples::table
                        .filter(crate::schemas::$schema::samples::id.eq(id)),
                )
                .execute(self)
                .map(|_| ())
                .map_err(StoreError::from)
            }
        }
    };
}

impl_sample_connection!(
    diesel::sqlite::SqliteConnection,
    sqlite,
    crate::models::sqlite::SqliteSampleModel,
    "sqlite",
    crate::models::sqlite::run_sqlite_migrations
);
impl_sample_connection!(
    diesel::pg::PgConnection,
    postgresql,
    crate::models::postgresql::PostgresSampleModel,
    "postgresql",
    crate::models::postgresql::run_postgres_migrations
);

/// Create a sample. Single implementation for all backends.
///
/// # Errors
///
/// Returns the failure detail when the insert fails.
#[tracing::instrument(
    name = "store.create_sample",
    skip(conn),
    fields(db.system.name = tracing::field::Empty, db.operation.name = "insert", db.collection.name = "samples")
)]
pub(crate) fn create_sample(
    conn: &mut dyn SampleConnection,
    body: NewSample,
) -> Result<SampleData, String> {
    tracing::Span::current().record("db.system.name", conn.backend_name());
    let id = UUID::random();
    let now = chrono::Utc::now().naive_utc();
    let data = SampleData::from_new_sample(id, now, body);

    conn.insert_sample(data.clone())
        .map_err(|err| err.to_string())?;
    Ok(data)
}

/// Fetch one sample by id. Single implementation for all backends.
/// Returns `Ok(None)` when no row matches.
///
/// # Errors
///
/// Returns the failure detail when the query fails.
#[tracing::instrument(
    name = "store.get_sample",
    skip(conn),
    fields(db.system.name = tracing::field::Empty, db.operation.name = "select", db.collection.name = "samples")
)]
pub(crate) fn get_sample(
    conn: &mut dyn SampleConnection,
    db_id: UUID,
) -> Result<Option<SampleData>, String> {
    tracing::Span::current().record("db.system.name", conn.backend_name());
    match conn.fetch_sample(db_id) {
        Ok(data) => Ok(Some(data)),
        Err(StoreError::NotFound) => Ok(None),
        Err(err) => Err(err.to_string()),
    }
}

/// List samples ordered by `created_at`. Single implementation for all backends.
///
/// # Errors
///
/// Returns the failure detail when the query fails.
#[tracing::instrument(
    name = "store.get_samples",
    skip(conn),
    fields(db.system.name = tracing::field::Empty, db.operation.name = "select", db.collection.name = "samples")
)]
pub(crate) fn get_samples(
    conn: &mut dyn SampleConnection,
    offset: i64,
    limit: i64,
) -> Result<Vec<SampleData>, String> {
    tracing::Span::current().record("db.system.name", conn.backend_name());
    conn.fetch_samples(offset, limit)
        .map_err(|err| err.to_string())
}

/// Patch a sample, returning the refreshed row.
/// Returns `Ok(None)` when no row matches. Single implementation for all backends.
///
/// # Errors
///
/// Returns the failure detail when the fetch or save fails.
#[tracing::instrument(
    name = "store.update_sample",
    skip(conn),
    fields(db.system.name = tracing::field::Empty, db.operation.name = "update", db.collection.name = "samples")
)]
pub(crate) fn update_sample(
    conn: &mut dyn SampleConnection,
    db_id: UUID,
    body: SampleUpdate,
) -> Result<Option<SampleData>, String> {
    tracing::Span::current().record("db.system.name", conn.backend_name());
    let existing = match conn.fetch_sample(db_id) {
        Ok(data) => data,
        Err(StoreError::NotFound) => return Ok(None),
        Err(err) => return Err(err.to_string()),
    };

    let merged = existing.apply_update(body, chrono::Utc::now().naive_utc());
    conn.save_sample(merged)
        .map(Some)
        .map_err(|err| err.to_string())
}

/// Delete a sample, returning the deleted row.
/// Returns `Ok(None)` when no row matches. Single implementation for all backends.
///
/// # Errors
///
/// Returns the failure detail when the fetch or delete fails.
#[tracing::instrument(
    name = "store.remove_sample",
    skip(conn),
    fields(db.system.name = tracing::field::Empty, db.operation.name = "delete", db.collection.name = "samples")
)]
pub(crate) fn remove_sample(
    conn: &mut dyn SampleConnection,
    db_id: UUID,
) -> Result<Option<SampleData>, String> {
    tracing::Span::current().record("db.system.name", conn.backend_name());
    let existing = match conn.fetch_sample(db_id) {
        Ok(data) => data,
        Err(StoreError::NotFound) => return Ok(None),
        Err(err) => return Err(err.to_string()),
    };

    conn.delete_sample(db_id).map_err(|err| err.to_string())?;
    Ok(Some(existing))
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::{NewSample, SampleUpdate, Status};
    use diesel::sqlite::SqliteConnection;

    fn migrated_connection() -> SqliteConnection {
        let mut conn = SqliteConnection::establish(":memory:").expect("in-memory sqlite connects");
        crate::models::sqlite::run_sqlite_migrations(&mut conn).expect("migrations apply");
        conn
    }

    fn full_sample(name: &str) -> NewSample {
        NewSample::builder(name.to_string(), Status::Inactive)
            .description(format!("{name} description"))
            .image_uri("https://example.com/img.png".to_string())
            .tags(vec!["one".to_string(), "two".to_string()])
            .build()
    }

    #[test]
    fn diesel_not_found_maps_to_store_not_found() {
        let err = StoreError::from(diesel::result::Error::NotFound);
        assert!(matches!(err, StoreError::NotFound));
    }

    #[test]
    fn diesel_unique_violation_maps_to_conflict() {
        use crate::auth::{AuthConnection, UserData};
        let mut conn = migrated_connection();
        let user = UserData {
            id: UUID::random(),
            username: "taken".to_string(),
            password_hash: "hash".to_string(),
            created_at: chrono::Utc::now().naive_utc(),
        };
        conn.insert_user(user.clone()).expect("first insert");
        let err = conn.insert_user(user).expect_err("duplicate must fail");
        assert!(matches!(err, StoreError::Conflict(_)));
    }

    #[test]
    fn diesel_database_error_maps_to_backend() {
        let mut conn = migrated_connection();
        let err = diesel::sql_query("SELECT * FROM missing_table")
            .execute(&mut conn)
            .expect_err("missing table must fail");
        assert!(matches!(StoreError::from(err), StoreError::Backend(_)));
    }

    #[test]
    fn backend_names_match_connections() {
        assert_eq!(migrated_connection().backend_name(), "sqlite");
    }

    #[test]
    fn run_migrations_is_idempotent() {
        let mut conn = migrated_connection();
        conn.run_migrations().expect("second migration run");
        conn.run_migrations().expect("third migration run");
    }

    #[test]
    fn create_then_fetch_round_trips_every_field() {
        let mut conn = migrated_connection();
        let created = create_sample(&mut conn, full_sample("roundtrip")).expect("create succeeds");
        let fetched = get_sample(&mut conn, created.id)
            .expect("fetch succeeds")
            .expect("row exists");
        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.name, "roundtrip");
        assert_eq!(
            fetched.description,
            Some("roundtrip description".to_string())
        );
        assert_eq!(fetched.status, "INACTIVE");
        assert_eq!(fetched.tags, Some(r#"["one","two"]"#.to_string()));
        assert_eq!(
            fetched.image_uri,
            Some("https://example.com/img.png".to_string())
        );
        assert_eq!(fetched.created_at, created.created_at);
        assert_eq!(fetched.updated_at, None);
    }

    #[test]
    fn fetch_missing_returns_none() {
        let mut conn = migrated_connection();
        assert_eq!(get_sample(&mut conn, UUID::random()).expect("query"), None);
    }

    #[test]
    fn duplicate_primary_key_fails_insert() {
        let mut conn = migrated_connection();
        let data = SampleData::from_new_sample(
            UUID::random(),
            chrono::Utc::now().naive_utc(),
            NewSample::new("dup".to_string(), Status::Active),
        );
        conn.insert_sample(data.clone())
            .expect("first insert succeeds");
        // Same primary key twice violates the uniqueness constraint.
        assert!(conn.insert_sample(data).is_err());
    }

    #[test]
    fn query_failures_surface_details_instead_of_missing() {
        let mut conn = migrated_connection();
        diesel::sql_query("DROP TABLE samples")
            .execute(&mut conn)
            .expect("drop succeeds");
        let id = UUID::random();
        // Every wrapper converts a backend failure into a detail string.
        // `get_sample` must not mistake it for "not found".
        let detail = create_sample(&mut conn, NewSample::new("x".to_string(), Status::Active))
            .expect_err("insert fails without table");
        assert_ne!(detail, String::new());
        assert!(get_sample(&mut conn, id).is_err());
        assert!(update_sample(&mut conn, id, SampleUpdate::default()).is_err());
        assert!(remove_sample(&mut conn, id).is_err());
        assert!(get_samples(&mut conn, 0, 10).is_err());
    }

    #[test]
    fn update_missing_returns_none() {
        let mut conn = migrated_connection();
        let result = update_sample(
            &mut conn,
            UUID::random(),
            SampleUpdate {
                name: Some("ghost".to_string()),
                ..Default::default()
            },
        )
        .expect("query");
        assert_eq!(result, None);
    }

    #[test]
    fn update_returns_refreshed_row() {
        let mut conn = migrated_connection();
        let created = create_sample(&mut conn, full_sample("before")).expect("create succeeds");
        let saved = update_sample(
            &mut conn,
            created.id,
            SampleUpdate {
                name: Some("after".to_string()),
                status: Some(Status::Archived),
                ..Default::default()
            },
        )
        .expect("update succeeds")
        .expect("row exists");
        assert_eq!(saved.name, "after");
        assert_eq!(saved.status, "ARCHIVED");
        assert_eq!(saved.created_at, created.created_at);
        assert!(saved.updated_at.is_some());
        // `save_sample` refreshes from the database, not from memory.
        let refetched = get_sample(&mut conn, created.id)
            .expect("fetch")
            .expect("row");
        assert_eq!(refetched.name, "after");
    }

    #[test]
    fn remove_returns_deleted_row_then_gone() {
        let mut conn = migrated_connection();
        let created = create_sample(&mut conn, full_sample("doomed")).expect("create succeeds");
        let deleted = remove_sample(&mut conn, created.id)
            .expect("remove succeeds")
            .expect("row existed");
        assert_eq!(deleted.id, created.id);
        assert_eq!(deleted.name, "doomed");
        assert_eq!(get_sample(&mut conn, created.id).expect("fetch"), None);
    }

    #[test]
    fn remove_missing_returns_none() {
        let mut conn = migrated_connection();
        assert_eq!(
            remove_sample(&mut conn, UUID::random()).expect("query"),
            None
        );
    }

    #[test]
    fn list_returns_created_at_order() {
        let mut conn = migrated_connection();
        for name in ["c", "a", "b"] {
            create_sample(&mut conn, NewSample::new(name.to_string(), Status::Active))
                .expect("create succeeds");
        }
        let rows = get_samples(&mut conn, 0, 100).expect("list succeeds");
        let names: Vec<String> = rows.into_iter().map(|row| row.name).collect();
        // Insertion order is preserved because `created_at` is monotonic here.
        assert_eq!(names, vec!["c", "a", "b"]);
    }

    #[test]
    fn list_pagination_passes_offset_and_limit_through() {
        let mut conn = migrated_connection();
        for index in 0..5 {
            create_sample(
                &mut conn,
                NewSample::new(format!("row-{index}"), Status::Active),
            )
            .expect("create succeeds");
        }
        let page = get_samples(&mut conn, 1, 2).expect("list succeeds");
        assert_eq!(page.len(), 2);
        assert_eq!(page.first().expect("first").name, "row-1");
        let empty = get_samples(&mut conn, 99, 10).expect("list succeeds");
        assert_eq!(empty, Vec::new());
    }
}
