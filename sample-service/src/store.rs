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

use crate::models::common::{SampleData, UUID};
use crate::{NewSample, SampleUpdate};

/// Storage primitives every backend connection must provide.
///
/// Methods work on the backend-agnostic [`SampleData`]; each implementation
/// translates to its own Diesel table/row types. Business logic (response
/// mapping, `NotFound` handling) lives in the functions below, so it is
/// written once for all backends.
///
/// The trait is object-safe (no generics, no `Self` in return position),
/// which lets [`crate::backend::AppState`] hold a single
/// `Arc<Mutex<dyn SampleConnection>>` instead of an enum.
pub trait SampleConnection: Send {
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
    /// Returns [`diesel::result::Error`] when the insert fails.
    fn insert_sample(&mut self, data: SampleData) -> Result<(), diesel::result::Error>;

    /// Fetch one row by id.
    ///
    /// # Errors
    ///
    /// Returns [`diesel::result::Error::NotFound`] when no row matches.
    fn fetch_sample(&mut self, id: UUID) -> Result<SampleData, diesel::result::Error>;

    /// Fetch a page of rows ordered by `created_at` ascending.
    ///
    /// # Errors
    ///
    /// Returns [`diesel::result::Error`] when the query fails.
    fn fetch_samples(
        &mut self,
        offset: i64,
        limit: i64,
    ) -> Result<Vec<SampleData>, diesel::result::Error>;

    /// Overwrite the mutable fields of the row with `data.id` and return the
    /// refreshed row. `created_at` is preserved (updates never change it).
    ///
    /// # Errors
    ///
    /// Returns [`diesel::result::Error`] when the update or refresh fails.
    fn save_sample(&mut self, data: SampleData) -> Result<SampleData, diesel::result::Error>;

    /// Delete the row with `id`. Callers fetch first when they need
    /// `NotFound` mapping or the deleted row.
    ///
    /// # Errors
    ///
    /// Returns [`diesel::result::Error`] when the delete fails.
    fn delete_sample(&mut self, id: UUID) -> Result<(), diesel::result::Error>;
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
            fn insert_sample(&mut self, data: SampleData) -> Result<(), diesel::result::Error> {
                let row = <$row>::from(data);
                diesel::insert_into(crate::schemas::$schema::samples::table)
                    .values(&row)
                    .execute(self)
                    .map(|_| ())
            }

            fn fetch_sample(&mut self, id: UUID) -> Result<SampleData, diesel::result::Error> {
                crate::schemas::$schema::samples::table
                    .filter(crate::schemas::$schema::samples::id.eq(id))
                    .select(<$row>::as_select())
                    .first(self)
                    .map(SampleData::from)
            }

            fn fetch_samples(
                &mut self,
                offset: i64,
                limit: i64,
            ) -> Result<Vec<SampleData>, diesel::result::Error> {
                let rows: Vec<$row> = crate::schemas::$schema::samples::table
                    .order(crate::schemas::$schema::samples::created_at.asc())
                    .offset(offset)
                    .limit(limit)
                    .select(<$row>::as_select())
                    .load(self)?;
                Ok(rows.into_iter().map(SampleData::from).collect())
            }

            fn save_sample(
                &mut self,
                data: SampleData,
            ) -> Result<SampleData, diesel::result::Error> {
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
                self.fetch_sample(id)
            }

            fn delete_sample(&mut self, id: UUID) -> Result<(), diesel::result::Error> {
                diesel::delete(
                    crate::schemas::$schema::samples::table
                        .filter(crate::schemas::$schema::samples::id.eq(id)),
                )
                .execute(self)
                .map(|_| ())
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
pub(crate) fn create_sample(
    conn: &mut dyn SampleConnection,
    body: NewSample,
) -> Result<SampleData, String> {
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
pub(crate) fn get_sample(
    conn: &mut dyn SampleConnection,
    db_id: UUID,
) -> Result<Option<SampleData>, String> {
    match conn.fetch_sample(db_id) {
        Ok(data) => Ok(Some(data)),
        Err(diesel::result::Error::NotFound) => Ok(None),
        Err(err) => Err(err.to_string()),
    }
}

/// List samples ordered by `created_at`. Single implementation for all backends.
///
/// # Errors
///
/// Returns the failure detail when the query fails.
pub(crate) fn get_samples(
    conn: &mut dyn SampleConnection,
    offset: i64,
    limit: i64,
) -> Result<Vec<SampleData>, String> {
    conn.fetch_samples(offset, limit)
        .map_err(|err| err.to_string())
}

/// Patch a sample, returning the refreshed row.
/// Returns `Ok(None)` when no row matches. Single implementation for all backends.
///
/// # Errors
///
/// Returns the failure detail when the fetch or save fails.
pub(crate) fn update_sample(
    conn: &mut dyn SampleConnection,
    db_id: UUID,
    body: SampleUpdate,
) -> Result<Option<SampleData>, String> {
    let existing = match conn.fetch_sample(db_id) {
        Ok(data) => data,
        Err(diesel::result::Error::NotFound) => return Ok(None),
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
pub(crate) fn remove_sample(
    conn: &mut dyn SampleConnection,
    db_id: UUID,
) -> Result<Option<SampleData>, String> {
    let existing = match conn.fetch_sample(db_id) {
        Ok(data) => data,
        Err(diesel::result::Error::NotFound) => return Ok(None),
        Err(err) => return Err(err.to_string()),
    };

    conn.delete_sample(db_id).map_err(|err| err.to_string())?;
    Ok(Some(existing))
}
