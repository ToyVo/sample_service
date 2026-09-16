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
#[derive(Debug, Clone, PartialEq, Eq)]
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

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::{NewSample, SampleUpdate, Status};

    fn sample_data() -> SampleData {
        SampleData::from_new_sample(
            UUID::random(),
            chrono::Utc::now().naive_utc(),
            NewSample::builder("base".to_string(), Status::Active)
                .description("desc".to_string())
                .image_uri("https://example.com/i.png".to_string())
                .tags(vec!["a".to_string(), "b".to_string()])
                .build(),
        )
    }

    #[test]
    fn uuid_random_is_unique_and_round_trips() {
        let first = UUID::random();
        let second = UUID::random();
        assert_ne!(first, second);
        let parsed: uuid::Uuid = first.into();
        assert_eq!(first.to_string(), parsed.to_string());
        assert_eq!(UUID(parsed).to_string(), first.to_string());
    }

    #[test]
    fn status_strings_map_to_api_enum() {
        assert_eq!(status_from_string("ACTIVE"), Status::Active);
        assert_eq!(status_from_string("INACTIVE"), Status::Inactive);
        assert_eq!(status_from_string("ARCHIVED"), Status::Archived);
        // Unknown, empty, and wrong-case values fall back to Active.
        assert_eq!(status_from_string("DELETED"), Status::Active);
        assert_eq!(status_from_string(""), Status::Active);
        assert_eq!(status_from_string("active"), Status::Active);
    }

    #[test]
    fn status_enum_serializes_to_db_strings() {
        assert_eq!(status_to_string(&Status::Active), "ACTIVE");
        assert_eq!(status_to_string(&Status::Inactive), "INACTIVE");
        assert_eq!(status_to_string(&Status::Archived), "ARCHIVED");
    }

    #[test]
    fn tags_json_round_trip() {
        assert_eq!(tags_to_json(None), None);
        assert_eq!(
            tags_to_json(Some(vec!["a".to_string(), "b c".to_string()])),
            Some(r#"["a","b c"]"#.to_string())
        );
        assert_eq!(tags_to_json(Some(Vec::new())), Some("[]".to_string()));

        assert_eq!(tags_from_json(None), None);
        assert_eq!(
            tags_from_json(Some(r#"["x","y"]"#.to_string())),
            Some(vec!["x".to_string(), "y".to_string()])
        );
        // Invalid JSON falls back to None instead of failing.
        assert_eq!(tags_from_json(Some("not json".to_string())), None);
        assert_eq!(tags_from_json(Some("{}".to_string())), None);
    }

    #[test]
    fn from_new_sample_maps_every_field() {
        let id = UUID::random();
        let now = chrono::Utc::now().naive_utc();
        let data = SampleData::from_new_sample(
            id,
            now,
            NewSample::builder("n".to_string(), Status::Inactive)
                .description("d".to_string())
                .image_uri("https://example.com/u.png".to_string())
                .tags(vec!["t".to_string()])
                .build(),
        );
        assert_eq!(data.id, id);
        assert_eq!(data.name, "n");
        assert_eq!(data.description, Some("d".to_string()));
        assert_eq!(data.status, "INACTIVE");
        assert_eq!(data.tags, Some(r#"["t"]"#.to_string()));
        assert_eq!(
            data.image_uri,
            Some("https://example.com/u.png".to_string())
        );
        assert_eq!(data.created_at, now);
        assert_eq!(data.updated_at, None);
    }

    #[test]
    fn from_new_sample_without_optionals() {
        let data = SampleData::from_new_sample(
            UUID::random(),
            chrono::Utc::now().naive_utc(),
            NewSample::new("minimal".to_string(), Status::Active),
        );
        assert_eq!(data.description, None);
        assert_eq!(data.tags, None);
        assert_eq!(data.image_uri, None);
    }

    #[test]
    fn to_api_maps_every_field() {
        let api = sample_data().to_api();
        assert_eq!(api.name, "base");
        assert_eq!(api.description, Some("desc".to_string()));
        assert_eq!(api.status, Status::Active);
        assert_eq!(api.tags, Some(vec!["a".to_string(), "b".to_string()]));
        assert_eq!(api.tags_condensed, Some("a, b".to_string()));
        assert_eq!(api.image_uri, Some("https://example.com/i.png".to_string()));
        assert_eq!(api.updated_at, None);
        uuid::Uuid::parse_str(&api.id).expect("api id is a uuid");
    }

    #[test]
    fn to_api_handles_missing_tags_and_status_fallback() {
        let mut data = sample_data();
        data.tags = None;
        data.status = "BOGUS".to_string();
        let api = data.to_api();
        assert_eq!(api.tags, None);
        assert_eq!(api.tags_condensed, None);
        assert_eq!(api.status, Status::Active);
    }

    #[test]
    fn to_api_joins_empty_tags_to_empty_string() {
        let mut data = sample_data();
        data.tags = Some("[]".to_string());
        let api = data.to_api();
        assert_eq!(api.tags, Some(Vec::new()));
        assert_eq!(api.tags_condensed, Some(String::new()));
    }

    #[test]
    fn apply_update_overwrites_only_provided_fields() {
        let data = sample_data();
        let created_at = data.created_at;
        let updated = data.apply_update(
            SampleUpdate {
                name: Some("renamed".to_string()),
                status: Some(Status::Archived),
                ..Default::default()
            },
            chrono::Utc::now().naive_utc(),
        );
        assert_eq!(updated.name, "renamed");
        assert_eq!(updated.status, "ARCHIVED");
        // Untouched fields are preserved.
        assert_eq!(updated.description, Some("desc".to_string()));
        assert_eq!(updated.tags, Some(r#"["a","b"]"#.to_string()));
        assert_eq!(
            updated.image_uri,
            Some("https://example.com/i.png".to_string())
        );
        assert_eq!(updated.created_at, created_at);
        assert!(updated.updated_at.is_some());
    }

    #[test]
    fn apply_update_replaces_tags_and_description() {
        let updated = sample_data().apply_update(
            SampleUpdate {
                description: Some("new desc".to_string()),
                tags: Some(vec!["solo".to_string()]),
                image_uri: Some("https://example.com/new.png".to_string()),
                ..Default::default()
            },
            chrono::Utc::now().naive_utc(),
        );
        assert_eq!(updated.description, Some("new desc".to_string()));
        assert_eq!(updated.tags, Some(r#"["solo"]"#.to_string()));
        assert_eq!(
            updated.image_uri,
            Some("https://example.com/new.png".to_string())
        );
        assert_eq!(updated.name, "base");
    }

    #[test]
    fn apply_empty_update_preserves_row_and_stamps_updated_at() {
        let data = sample_data();
        let snapshot = data.clone();
        let updated = data.apply_update(SampleUpdate::default(), chrono::Utc::now().naive_utc());
        assert_eq!(updated.id, snapshot.id);
        assert_eq!(updated.name, snapshot.name);
        assert_eq!(updated.description, snapshot.description);
        assert_eq!(updated.status, snapshot.status);
        assert_eq!(updated.tags, snapshot.tags);
        assert_eq!(updated.image_uri, snapshot.image_uri);
        assert_eq!(updated.created_at, snapshot.created_at);
        assert!(updated.updated_at.is_some());
    }
}
