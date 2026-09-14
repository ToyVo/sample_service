use std::sync::{Arc, Mutex, MutexGuard};

use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;

use crate::models::{SampleModel, UUID};
use crate::schema::samples;
use crate::{
    CreateSampleResponse, CreateSampleResponse201, Error, ErrorCode, GetSampleResponse,
    GetSampleServerResponse, GetSamplesResponse, GetSamplesServerResponse, NewSample,
    RemoveSampleResponse, RemoveSampleServerResponse, SampleApi, SampleUpdate,
    UpdateSampleResponse, UpdateSampleServerResponse,
};

/// Shared application state holding a pooled SQLite connection.
///
/// Diesel's `SqliteConnection` is not `Sync`, so it is wrapped in a
/// `std::sync::Mutex`. Handlers lock it briefly for each blocking query;
/// no `.await` is held across the lock.
#[derive(Clone)]
pub struct AppState {
    db: Arc<Mutex<SqliteConnection>>,
}

impl AppState {
    /// Create state from an already-established connection.
    #[must_use]
    pub fn new(conn: SqliteConnection) -> Self {
        Self {
            db: Arc::new(Mutex::new(conn)),
        }
    }
}

/// Establish a SQLite connection from `DATABASE_URL`.
///
/// Accepts both plain paths (`./dev.db`) and URL-style values
/// (`sqlite://./dev.db`) as produced by `secretspec`.
///
/// # Errors
///
/// Returns `diesel::ConnectionError` if the database cannot be opened.
pub fn establish_connection() -> Result<SqliteConnection, diesel::ConnectionError> {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "./dev.db".to_string());
    let url: &str = database_url
        .strip_prefix("sqlite://")
        .unwrap_or(database_url.as_str());
    SqliteConnection::establish(url)
}

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

#[allow(clippy::result_large_err)]
fn parse_id(sample_id: &str) -> Result<UUID, Error> {
    uuid::Uuid::parse_str(sample_id)
        .map(UUID)
        .map_err(|_| not_found(sample_id))
}

fn lock_db(state: &AppState) -> MutexGuard<'_, SqliteConnection> {
    match state.db.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[async_trait::async_trait]
impl SampleApi for AppState {
    async fn create_sample(&self, body: NewSample) -> CreateSampleResponse {
        let id = UUID::random();
        let now = chrono::Utc::now().naive_utc();
        let row = SampleModel::from_new_sample(id, now, body);

        let mut conn = lock_db(self);
        let insert = diesel::insert_into(samples::table)
            .values(&row)
            .execute(&mut *conn);
        drop(conn);

        match insert {
            Ok(_) => {}
            Err(err) => {
                return CreateSampleResponse::Created(CreateSampleResponse201 {
                    data: None,
                    errors: vec![internal_error(err.to_string())],
                });
            }
        }

        CreateSampleResponse::Created(CreateSampleResponse201 {
            data: Some(row.to_api()),
            errors: Vec::new(),
        })
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

        let mut conn = lock_db(self);
        let existing: Result<SampleModel, diesel::result::Error> = samples::table
            .filter(samples::id.eq(db_id))
            .select(SampleModel::as_select())
            .first(&mut *conn);
        let existing = match existing {
            Ok(row) => row,
            Err(diesel::result::Error::NotFound) => {
                drop(conn);
                return RemoveSampleServerResponse::Ok(RemoveSampleResponse {
                    data: None,
                    errors: vec![not_found(&sample_id)],
                });
            }
            Err(err) => {
                drop(conn);
                return RemoveSampleServerResponse::Ok(RemoveSampleResponse {
                    data: None,
                    errors: vec![internal_error(err.to_string())],
                });
            }
        };

        let deleted =
            diesel::delete(samples::table.filter(samples::id.eq(db_id))).execute(&mut *conn);
        drop(conn);

        match deleted {
            Ok(_) => {}
            Err(err) => {
                return RemoveSampleServerResponse::Ok(RemoveSampleResponse {
                    data: None,
                    errors: vec![internal_error(err.to_string())],
                });
            }
        }

        RemoveSampleServerResponse::Ok(RemoveSampleResponse {
            data: Some(existing.to_api()),
            errors: Vec::new(),
        })
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

        let mut conn = lock_db(self);
        let existing: Result<SampleModel, diesel::result::Error> = samples::table
            .filter(samples::id.eq(db_id))
            .select(SampleModel::as_select())
            .first(&mut *conn);
        let existing = match existing {
            Ok(row) => row,
            Err(diesel::result::Error::NotFound) => {
                drop(conn);
                return UpdateSampleServerResponse::Ok(UpdateSampleResponse {
                    data: None,
                    errors: vec![not_found(&sample_id)],
                });
            }
            Err(err) => {
                drop(conn);
                return UpdateSampleServerResponse::Ok(UpdateSampleResponse {
                    data: None,
                    errors: vec![internal_error(err.to_string())],
                });
            }
        };

        let now = chrono::Utc::now().naive_utc();
        let status = body
            .status
            .as_ref()
            .map_or_else(|| existing.status.clone(), |s| s.as_str().to_string());
        let tags = match (body.tags, existing.tags) {
            (Some(new_tags), _) => serde_json::to_string(&new_tags).ok(),
            (None, old_json) => old_json,
        };

        let updated_rows = diesel::update(samples::table.filter(samples::id.eq(db_id)))
            .set((
                samples::name.eq(body.name.unwrap_or(existing.name)),
                samples::description.eq(body.description.or(existing.description)),
                samples::status.eq(status),
                samples::tags.eq(tags),
                samples::image_uri.eq(body.image_uri.or(existing.image_uri)),
                samples::updated_at.eq(Some(now)),
            ))
            .execute(&mut *conn);
        match updated_rows {
            Ok(_) => {}
            Err(err) => {
                drop(conn);
                return UpdateSampleServerResponse::Ok(UpdateSampleResponse {
                    data: None,
                    errors: vec![internal_error(err.to_string())],
                });
            }
        }

        let updated: Result<SampleModel, diesel::result::Error> = samples::table
            .filter(samples::id.eq(db_id))
            .select(SampleModel::as_select())
            .first(&mut *conn);
        drop(conn);

        match updated {
            Ok(row) => UpdateSampleServerResponse::Ok(UpdateSampleResponse {
                data: Some(row.to_api()),
                errors: Vec::new(),
            }),
            Err(err) => UpdateSampleServerResponse::Ok(UpdateSampleResponse {
                data: None,
                errors: vec![internal_error(err.to_string())],
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

        let mut conn = lock_db(self);
        let result: Result<SampleModel, diesel::result::Error> = samples::table
            .filter(samples::id.eq(db_id))
            .select(SampleModel::as_select())
            .first(&mut *conn);
        drop(conn);

        match result {
            Ok(row) => GetSampleServerResponse::Ok(GetSampleResponse {
                data: Some(row.to_api()),
                errors: Vec::new(),
            }),
            Err(diesel::result::Error::NotFound) => {
                GetSampleServerResponse::Ok(GetSampleResponse {
                    data: None,
                    errors: vec![not_found(&sample_id)],
                })
            }
            Err(err) => GetSampleServerResponse::Ok(GetSampleResponse {
                data: None,
                errors: vec![internal_error(err.to_string())],
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

        let mut conn = lock_db(self);
        let result: Result<Vec<SampleModel>, diesel::result::Error> = samples::table
            .order(samples::created_at.asc())
            .offset(offset)
            .limit(limit)
            .select(SampleModel::as_select())
            .load(&mut *conn);
        drop(conn);

        match result {
            Ok(rows) => {
                let data = rows.into_iter().map(SampleModel::to_api).collect();
                GetSamplesServerResponse::Ok(GetSamplesResponse {
                    data,
                    errors: Vec::new(),
                })
            }
            Err(err) => GetSamplesServerResponse::Ok(GetSamplesResponse {
                data: Vec::new(),
                errors: vec![internal_error(err.to_string())],
            }),
        }
    }
}
