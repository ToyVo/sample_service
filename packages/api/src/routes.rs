#[cfg(feature = "server")]
use axum::{
    http::{StatusCode, header},
    response::IntoResponse,
};

#[cfg(feature = "server")]
use crate::models::SampleData;
#[cfg(feature = "server")]
use crate::{
    AppState, CodegenApi, CreateSampleResponse, CreateSampleResponse201, GetSampleResponse,
    GetSampleResponse404, GetSampleServerResponse, GetSamplesResponse, GetSamplesServerResponse,
    NewSample, RemoveSampleResponse, RemoveSampleResponse404, RemoveSampleServerResponse,
    SampleUpdate, ServeSpecResponse, ServeSpecServerResponse, UpdateSampleResponse,
    UpdateSampleResponse404, UpdateSampleServerResponse, internal_error, lock_db, not_found,
    parse_id,
};

/// The `OpenAPI` document, embedded at compile time.
pub const SPEC_JSON: &str = include_str!("../../../domain.oas3.json");

// ---------------------------------------------------------------------------
// `SampleApi`: each method locks the connection, calls the single
// backend-agnostic operation in `crate::store`, and converts the domain
// result (`Ok(data)` / `Ok(None)` / `Err(detail)`) into the response shape.
// Response construction lives here; the store never sees response types.
// ---------------------------------------------------------------------------

#[cfg(feature = "server")]
#[async_trait::async_trait]
impl CodegenApi for AppState {
    async fn serve_spec(&self) -> ServeSpecServerResponse {
        ServeSpecServerResponse::Ok(ServeSpecResponse {
            additional_properties: serde_json::from_str(SPEC_JSON).unwrap_or_default(),
        })
    }

    async fn create_sample(&self, body: NewSample) -> CreateSampleResponse {
        let mut conn = lock_db(&self.db);
        match crate::store::create_sample(&mut *conn, body) {
            Ok(data) => CreateSampleResponse::Created(CreateSampleResponse201 {
                code: Some(String::from("Created")),
                data: Some(data.to_api()),
                errors: Vec::new(),
                r#type: Some(String::from("https://openapi-to-rust.dev/problems/created")),
                status: Some(201),
                title: Some(String::from("Created")),
            }),
            Err(detail) => CreateSampleResponse::Created(CreateSampleResponse201 {
                code: Some(String::from("Internal Server Error")),
                data: None,
                errors: vec![internal_error(detail)],
                r#type: Some(String::from(
                    "https://openapi-to-rust.dev/problems/internal-server-error",
                )),
                status: Some(500),
                title: Some(String::from("Internal Server Error")),
            }),
        }
    }

    async fn remove_sample(&self, sample_id: String) -> RemoveSampleServerResponse {
        let db_id = match parse_id(&sample_id) {
            Ok(id) => id,
            Err(err) => {
                return RemoveSampleServerResponse::NotFound(RemoveSampleResponse404 {
                    code: Some(String::from("Internal Server Error")),
                    data: None,
                    errors: vec![err],
                    r#type: Some(String::from(
                        "https://openapi-to-rust.dev/problems/internal-server-error",
                    )),
                    status: Some(500),
                    title: Some(String::from("Internal Server Error")),
                });
            }
        };
        let mut conn = lock_db(&self.db);
        match crate::store::remove_sample(&mut *conn, db_id) {
            Ok(Some(data)) => RemoveSampleServerResponse::Ok(RemoveSampleResponse {
                code: Some(String::from("Ok")),
                data: Some(data.to_api()),
                errors: Vec::new(),
                r#type: Some(String::from("https://openapi-to-rust.dev/problems/ok")),
                status: Some(200),
                title: Some(String::from("Ok")),
            }),
            Ok(None) => RemoveSampleServerResponse::NotFound(RemoveSampleResponse404 {
                code: Some(String::from("Not Found")),
                data: None,
                errors: vec![not_found(&sample_id)],
                r#type: Some(String::from(
                    "https://openapi-to-rust.dev/problems/not-found",
                )),
                status: Some(404),
                title: Some(String::from("Not Found")),
            }),
            Err(detail) => RemoveSampleServerResponse::Ok(RemoveSampleResponse {
                code: Some(String::from("Internal Server Error")),
                data: None,
                errors: vec![internal_error(detail)],
                r#type: Some(String::from(
                    "https://openapi-to-rust.dev/problems/internal-server-error",
                )),
                status: Some(500),
                title: Some(String::from("Internal Server Error")),
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
                return UpdateSampleServerResponse::NotFound(UpdateSampleResponse404 {
                    code: Some(String::from("Not Found")),
                    data: None,
                    errors: vec![err],
                    r#type: Some(String::from(
                        "https://openapi-to-rust.dev/problems/not-found",
                    )),
                    status: Some(404),
                    title: Some(String::from("Not Found")),
                });
            }
        };
        let mut conn = lock_db(&self.db);
        match crate::store::update_sample(&mut *conn, db_id, body) {
            Ok(Some(data)) => UpdateSampleServerResponse::Ok(UpdateSampleResponse {
                code: Some(String::from("Ok")),
                data: Some(data.to_api()),
                errors: Vec::new(),
                r#type: Some(String::from("https://openapi-to-rust.dev/problems/ok")),
                status: Some(200),
                title: Some(String::from("Ok")),
            }),
            Ok(None) => UpdateSampleServerResponse::NotFound(UpdateSampleResponse404 {
                code: Some(String::from("Not Found")),
                data: None,
                errors: vec![not_found(&sample_id)],
                r#type: Some(String::from(
                    "https://openapi-to-rust.dev/problems/not-found",
                )),
                status: Some(404),
                title: Some(String::from("Not Found")),
            }),
            Err(detail) => UpdateSampleServerResponse::Ok(UpdateSampleResponse {
                code: Some(String::from("Internal Server Error")),
                data: None,
                errors: vec![internal_error(detail)],
                r#type: Some(String::from(
                    "https://openapi-to-rust.dev/problems/internal-server-error",
                )),
                status: Some(500),
                title: Some(String::from("Internal Server Error")),
            }),
        }
    }

    async fn get_sample(&self, sample_id: String) -> GetSampleServerResponse {
        let db_id = match parse_id(&sample_id) {
            Ok(id) => id,
            Err(err) => {
                return GetSampleServerResponse::NotFound(GetSampleResponse404 {
                    code: Some(String::from("Not Found")),
                    data: None,
                    errors: vec![err],
                    r#type: Some(String::from(
                        "https://openapi-to-rust.dev/problems/not-found",
                    )),
                    status: Some(404),
                    title: Some(String::from("Not Found")),
                });
            }
        };
        let mut conn = lock_db(&self.db);
        match crate::store::get_sample(&mut *conn, db_id) {
            Ok(Some(data)) => GetSampleServerResponse::Ok(GetSampleResponse {
                code: Some(String::from("Ok")),
                data: Some(data.to_api()),
                errors: Vec::new(),
                r#type: Some(String::from("https://openapi-to-rust.dev/problems/ok")),
                status: Some(200),
                title: Some(String::from("Ok")),
            }),
            Ok(None) => GetSampleServerResponse::NotFound(GetSampleResponse404 {
                code: Some(String::from("Not Found")),
                data: None,
                errors: vec![not_found(&sample_id)],
                r#type: Some(String::from(
                    "https://openapi-to-rust.dev/problems/not-found",
                )),
                status: Some(404),
                title: Some(String::from("Not Found")),
            }),
            Err(detail) => GetSampleServerResponse::Ok(GetSampleResponse {
                code: Some(String::from("Internal Server Error")),
                data: None,
                errors: vec![internal_error(detail)],
                r#type: Some(String::from(
                    "https://openapi-to-rust.dev/problems/internal-server-error",
                )),
                status: Some(500),
                title: Some(String::from("Internal Server Error")),
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
                    code: Some(String::from("Ok")),
                    data,
                    errors: Vec::new(),
                    r#type: Some(String::from("https://openapi-to-rust.dev/problems/ok")),
                    status: Some(200),
                    title: Some(String::from("Ok")),
                })
            }
            Err(detail) => GetSamplesServerResponse::Ok(GetSamplesResponse {
                code: Some(String::from("Internal Server Error")),
                data: Vec::new(),
                errors: vec![internal_error(detail)],
                r#type: Some(String::from(
                    "https://openapi-to-rust.dev/problems/internal-server-error",
                )),
                status: Some(500),
                title: Some(String::from("Internal Server Error")),
            }),
        }
    }
}
