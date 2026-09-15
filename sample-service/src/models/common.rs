use chrono::{DateTime, NaiveDateTime, Utc};
use diesel::backend::Backend;
use diesel::deserialize::FromSqlRow;
use diesel::deserialize::{self, FromSql};
use diesel::expression::AsExpression;
use diesel::pg::{Pg, PgValue};
use diesel::serialize::{self, Output, ToSql};
use diesel::sql_types::Binary;
use diesel::sqlite::Sqlite;
use std::fmt::{Display, Formatter};

/// Newtype wrapper so Diesel can store `uuid::Uuid` as 16 bytes.
///
/// - SQLite: `BLOB` column.
/// - Postgres: `BYTEA` column.
///
/// Both map to Diesel's `Binary` SQL type, so a single wrapper works for
/// either backend once `FromSql`/`ToSql` are implemented for both.
#[derive(Debug, Clone, Copy, FromSqlRow, AsExpression, Hash, Eq, PartialEq)]
#[diesel(sql_type = Binary)]
pub struct UUID(pub uuid::Uuid);

impl UUID {
    #[must_use]
    pub fn random() -> Self {
        Self(uuid::Uuid::now_v7())
    }
}

impl From<UUID> for uuid::Uuid {
    fn from(s: UUID) -> Self {
        s.0
    }
}

impl Display for UUID {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromSql<Binary, Sqlite> for UUID {
    fn from_sql(mut value: <Sqlite as Backend>::RawValue<'_>) -> deserialize::Result<Self> {
        let bytes = value.read_blob();
        match uuid::Uuid::from_slice(bytes) {
            Ok(uuid) => Ok(Self(uuid)),
            Err(err) => Err(format!("Invalid UUID bytes: {err}").into()),
        }
    }
}

impl ToSql<Binary, Sqlite> for UUID {
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, Sqlite>) -> serialize::Result {
        <[u8; 16] as ToSql<Binary, Sqlite>>::to_sql(self.0.as_bytes(), out)
    }
}

impl FromSql<Binary, Pg> for UUID {
    fn from_sql(value: PgValue<'_>) -> deserialize::Result<Self> {
        let bytes = value.as_bytes();
        match uuid::Uuid::from_slice(bytes) {
            Ok(uuid) => Ok(Self(uuid)),
            Err(err) => Err(format!("Invalid UUID bytes: {err}").into()),
        }
    }
}

impl ToSql<Binary, Pg> for UUID {
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, Pg>) -> serialize::Result {
        <[u8; 16] as ToSql<Binary, Pg>>::to_sql(self.0.as_bytes(), out)
    }
}

/// Backend-agnostic sample row.
///
/// Both Diesel row types convert to/from this struct, so CRUD business logic
/// in [`crate::store`] is written once against `SampleData` instead of once
/// per backend. `status` is the DB string form (`ACTIVE`/`INACTIVE`/…);
/// `tags` is the JSON-encoded column (`None` = NULL).
#[derive(Debug, Clone)]
pub struct SampleData {
    pub id: UUID,
    pub name: String,
    pub description: Option<String>,
    pub status: String,
    pub tags: Option<String>,
    pub image_uri: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
}

impl SampleData {
    /// Build row data from a `NewSample` API payload.
    #[must_use]
    pub fn from_new_sample(id: UUID, now: NaiveDateTime, sample: crate::NewSample) -> Self {
        Self {
            id,
            name: sample.name,
            description: sample.description,
            status: status_to_string(&sample.status),
            tags: tags_to_json(sample.tags),
            image_uri: sample.image_uri,
            created_at: now,
            updated_at: None,
        }
    }

    /// Merge a `SampleUpdate` payload onto this row. Fields left as `None`
    /// keep their existing values; `updated_at` is set to `now`.
    #[must_use]
    pub fn apply_update(self, body: crate::SampleUpdate, now: NaiveDateTime) -> Self {
        let status = body
            .status
            .as_ref()
            .map_or_else(|| self.status.clone(), |s| s.as_str().to_string());
        let tags = match (body.tags, self.tags) {
            (Some(new_tags), _) => serde_json::to_string(&new_tags).ok(),
            (None, old_json) => old_json,
        };
        Self {
            id: self.id,
            name: body.name.unwrap_or(self.name),
            description: body.description.or(self.description),
            status,
            tags,
            image_uri: body.image_uri.or(self.image_uri),
            created_at: self.created_at,
            updated_at: Some(now),
        }
    }

    /// Convert this row into the public API type.
    #[must_use]
    pub fn to_api(self) -> crate::Sample {
        let tags = tags_from_json(self.tags);
        // Comma-separated view of `tags`.
        let tags_condensed = tags.as_ref().map(|t| t.join(", "));
        crate::Sample {
            id: self.id.to_string(),
            name: self.name,
            description: self.description,
            status: status_from_string(&self.status),
            tags,
            tags_condensed,
            image_uri: self.image_uri,
            created_at: DateTime::from_naive_utc_and_offset(self.created_at, Utc),
            updated_at: self
                .updated_at
                .map(|dt| DateTime::from_naive_utc_and_offset(dt, Utc)),
        }
    }
}

/// Convert a DB `status` string into the API enum. Unknown values fall back to
/// `Active` so conversions stay infallible; writes always use valid values.
pub(crate) fn status_from_string(s: &str) -> crate::Status {
    match s {
        "INACTIVE" => crate::Status::Inactive,
        "ARCHIVED" => crate::Status::Archived,
        _ => crate::Status::Active,
    }
}

pub(crate) fn status_to_string(status: &crate::Status) -> String {
    status.as_str().to_string()
}

/// Decode the JSON-encoded `tags` column. Invalid JSON falls back to `None`
/// so conversions stay infallible; writes always produce valid JSON.
pub(crate) fn tags_from_json(json: Option<String>) -> Option<Vec<String>> {
    json.and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
}

pub(crate) fn tags_to_json(tags: Option<Vec<String>>) -> Option<String> {
    tags.and_then(|t| serde_json::to_string(&t).ok())
}
