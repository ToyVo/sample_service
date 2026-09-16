use dioxus::prelude::*;

#[cfg(feature = "server")]
use crate::models::SampleData;
#[cfg(feature = "server")]
use crate::{
    AppState, CreateSampleResponse, CreateSampleResponse201, GetSampleResponse,
    GetSampleServerResponse, GetSamplesResponse, GetSamplesServerResponse, NewSample,
    RemoveSampleResponse, RemoveSampleServerResponse, SampleApi, SampleUpdate,
    UpdateSampleResponse, UpdateSampleServerResponse, internal_error, lock_db, not_found, parse_id,
};

/// Echo the user input on the server.
// `async` is required by `#[post]` even though this body needs no `.await`.
#[allow(clippy::unused_async)]
#[post("/api/echo")]
pub async fn echo(input: String) -> Result<String, ServerFnError> {
    Ok(input)
}

// ---------------------------------------------------------------------------
// `SampleApi`: each method locks the connection, calls the single
// backend-agnostic operation in `crate::store`, and converts the domain
// result (`Ok(data)` / `Ok(None)` / `Err(detail)`) into the response shape.
// Response construction lives here; the store never sees response types.
// ---------------------------------------------------------------------------

#[cfg(feature = "server")]
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
