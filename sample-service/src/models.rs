use chrono::{DateTime, NaiveDateTime, Utc};
use diesel::backend::Backend;
use diesel::deserialize::FromSqlRow;
use diesel::deserialize::{self, FromSql};
use diesel::expression::AsExpression;
use diesel::prelude::*;
use diesel::serialize::{self, Output, ToSql};
use diesel::sql_types::Binary;
use diesel::sqlite::Sqlite;
use std::fmt::{Display, Formatter};

/// Newtype wrapper so Diesel can store `uuid::Uuid` as SQLite `BLOB` (16 bytes).
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

/// Diesel row type for the `samples` table.
///
/// `tags` is stored as a JSON-encoded `TEXT` column (`None` = NULL, no tags).
/// Timestamps are stored as Diesel `Timestamp` (`NaiveDateTime`) and converted
/// to `DateTime<Utc>` at the API boundary.
#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::schema::samples)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct SampleModel {
    pub id: UUID,
    pub name: String,
    pub description: Option<String>,
    pub status: String,
    pub tags: Option<String>,
    pub image_uri: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
}

/// Convert a DB `status` string into the API enum. Unknown values fall back to
/// `Active` so `From` stays infallible; writes always use valid values.
fn status_from_string(s: &str) -> crate::Status {
    match s {
        "INACTIVE" => crate::Status::Inactive,
        "ARCHIVED" => crate::Status::Archived,
        _ => crate::Status::Active,
    }
}

fn status_to_string(status: &crate::Status) -> String {
    status.as_str().to_string()
}

/// Decode the JSON-encoded `tags` column. Invalid JSON falls back to `None`
/// so `From` stays infallible; writes always produce valid JSON.
fn tags_from_json(json: Option<String>) -> Option<Vec<String>> {
    json.and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
}

fn tags_to_json(tags: Option<Vec<String>>) -> Option<String> {
    tags.and_then(|t| serde_json::to_string(&t).ok())
}

impl SampleModel {
    /// Build a new row from a `NewSample` API payload.
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

impl From<SampleModel> for crate::Sample {
    fn from(s: SampleModel) -> Self {
        s.to_api()
    }
}

impl TryFrom<crate::Sample> for SampleModel {
    type Error = String;

    fn try_from(s: crate::Sample) -> Result<Self, Self::Error> {
        let uuid = uuid::Uuid::parse_str(&s.id).map_err(|err| err.to_string())?;
        Ok(Self {
            id: UUID(uuid),
            name: s.name,
            description: s.description,
            status: status_to_string(&s.status),
            tags: tags_to_json(s.tags),
            image_uri: s.image_uri,
            created_at: s.created_at.naive_utc(),
            updated_at: s.updated_at.map(|dt: DateTime<Utc>| dt.naive_utc()),
        })
    }
}
