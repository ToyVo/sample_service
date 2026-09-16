use chrono::NaiveDateTime;
use diesel::prelude::*;
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};

use super::common::{SampleData, UUID};

pub const SQLITE_MIGRATIONS: EmbeddedMigrations = embed_migrations!("../../migrations/sqlite");

/// Diesel row type for the SQLite `samples` table.
///
/// A thin persistence wrapper: conversions to/from the backend-agnostic
/// [`SampleData`] hold the shared logic, so this struct only declares the
/// table mapping.
#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::schemas::sqlite::samples)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct SqliteSampleModel {
    pub id: UUID,
    pub name: String,
    pub description: Option<String>,
    pub status: String,
    pub tags: Option<String>,
    pub image_uri: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
}

impl From<SqliteSampleModel> for SampleData {
    fn from(row: SqliteSampleModel) -> Self {
        Self {
            id: row.id,
            name: row.name,
            description: row.description,
            status: row.status,
            tags: row.tags,
            image_uri: row.image_uri,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

impl From<SampleData> for SqliteSampleModel {
    fn from(data: SampleData) -> Self {
        Self {
            id: data.id,
            name: data.name,
            description: data.description,
            status: data.status,
            tags: data.tags,
            image_uri: data.image_uri,
            created_at: data.created_at,
            updated_at: data.updated_at,
        }
    }
}

/// Run pending SQLite migrations on an established connection.
///
/// # Errors
///
/// Returns the migration error if any pending migration fails.
pub fn run_sqlite_migrations(
    connection: &mut diesel::sqlite::SqliteConnection,
) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    connection.run_pending_migrations(SQLITE_MIGRATIONS)?;
    Ok(())
}
