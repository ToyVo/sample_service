#[cfg(feature = "server")]
use crate::auth::token::{clear_session_cookie, session_cookie};
#[cfg(feature = "server")]
use crate::models::SampleData;
#[cfg(feature = "server")]
use crate::{
    AppState, CodegenApi, Credentials, FinishPasskeyLoginServerResponse,
    FinishPasskeyLoginServerResponseOkHeaders, FinishPasskeyRegistrationResponse,
    GetAccountServerResponse, LoginServerResponse, LoginServerResponseOkHeaders, LogoutResponse,
    LogoutResponseNoContentHeaders, PasskeyCredential, PasskeyLogin, RegisterResponse,
    RegisterResponseCreatedHeaders, StartPasskeyLoginServerResponse,
    StartPasskeyRegistrationServerResponse,
};
#[cfg(feature = "server")]
use crate::{
    CreateSampleResponse, CreateSampleResponse201, GetSampleResponse, GetSampleResponse404,
    GetSampleServerResponse, GetSamplesResponse, GetSamplesServerResponse, NewSample,
    RemoveSampleResponse, RemoveSampleResponse404, RemoveSampleServerResponse, SampleUpdate,
    ServeSpecResponse, ServeSpecServerResponse, UpdateSampleResponse, UpdateSampleResponse404,
    UpdateSampleServerResponse, internal_error, lock_db, not_found, parse_id,
};

/// Build a generated response envelope (`code`/`data`/`errors`/`type`/
/// `status`/`title`) for `$status`, in the same shape the sample operations
/// spell out by hand.
#[cfg(feature = "server")]
macro_rules! envelope {
    ($ty:ident, $status:literal, $data:expr, $errors:expr) => {{
        let (title, slug) = status_text($status);
        crate::$ty {
            code: Some(title.to_owned()),
            data: $data,
            errors: $errors,
            r#type: Some(format!("https://openapi-to-rust.dev/problems/{slug}")),
            status: Some(i64::from($status)),
            title: Some(title.to_owned()),
        }
    }};
}

/// Envelope `title`/`code` and problem-type slug for an auth status.
#[cfg(feature = "server")]
const fn status_text(status: u16) -> (&'static str, &'static str) {
    match status {
        200 => ("Ok", "ok"),
        201 => ("Created", "created"),
        400 => ("Bad Request", "bad-request"),
        401 => ("Unauthorized", "unauthorized"),
        409 => ("Conflict", "conflict"),
        _ => ("Internal Server Error", "internal-server-error"),
    }
}

/// The session token sources a protected auth operation received.
#[cfg(feature = "server")]
const fn presented<'a>(
    authorization: Option<&'a str>,
    host_session: Option<&'a str>,
) -> crate::auth::Presented<'a> {
    crate::auth::Presented {
        authorization,
        cookie: host_session,
    }
}

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

    // -----------------------------------------------------------------------
    // Auth: logic lives in `crate::auth`; these map each `AuthError` onto
    // the response variants the operation declares (anything undeclared is
    // a 500).
    // -----------------------------------------------------------------------

    async fn register(&self, body: Credentials) -> RegisterResponse {
        match crate::auth::register(self, body).await {
            Ok(session) => {
                let headers = RegisterResponseCreatedHeaders {
                    set_cookie: vec![session_cookie(&session.token)],
                };
                RegisterResponse::Created(
                    envelope!(RegisterResponse201, 201, Some(session), Vec::new()),
                    headers,
                )
            }
            Err(err) if err.status() == 409 => RegisterResponse::Conflict(envelope!(
                RegisterResponse409,
                409,
                None,
                vec![err.to_error()]
            )),
            Err(err) => RegisterResponse::InternalServerError(envelope!(
                RegisterResponse500,
                500,
                None,
                vec![err.to_error()]
            )),
        }
    }

    async fn login(&self, body: Credentials) -> LoginServerResponse {
        match crate::auth::login(self, body).await {
            Ok(session) => {
                let headers = LoginServerResponseOkHeaders {
                    set_cookie: vec![session_cookie(&session.token)],
                };
                LoginServerResponse::Ok(
                    envelope!(LoginResponse, 200, Some(session), Vec::new()),
                    headers,
                )
            }
            Err(err) if err.status() == 401 => LoginServerResponse::Unauthorized(envelope!(
                LoginResponse401,
                401,
                None,
                vec![err.to_error()]
            )),
            Err(err) => LoginServerResponse::InternalServerError(envelope!(
                LoginResponse500,
                500,
                None,
                vec![err.to_error()]
            )),
        }
    }

    async fn logout(&self) -> LogoutResponse {
        LogoutResponse::NoContent(LogoutResponseNoContentHeaders {
            set_cookie: vec![clear_session_cookie()],
        })
    }

    async fn get_account(
        &self,
        authorization: Option<String>,
        host_session: Option<String>,
    ) -> GetAccountServerResponse {
        match crate::auth::account(
            self,
            presented(authorization.as_deref(), host_session.as_deref()),
        ) {
            Ok(account) => GetAccountServerResponse::Ok(envelope!(
                GetAccountResponse,
                200,
                Some(account),
                Vec::new()
            )),
            Err(err) if err.status() == 401 => GetAccountServerResponse::Unauthorized(envelope!(
                GetAccountResponse401,
                401,
                None,
                vec![err.to_error()]
            )),
            Err(err) => GetAccountServerResponse::InternalServerError(envelope!(
                GetAccountResponse500,
                500,
                None,
                vec![err.to_error()]
            )),
        }
    }

    async fn start_passkey_registration(
        &self,
        authorization: Option<String>,
        host_session: Option<String>,
    ) -> StartPasskeyRegistrationServerResponse {
        match crate::auth::start_passkey_registration(
            self,
            presented(authorization.as_deref(), host_session.as_deref()),
        ) {
            Ok(challenge) => StartPasskeyRegistrationServerResponse::Ok(envelope!(
                StartPasskeyRegistrationResponse,
                200,
                Some(challenge),
                Vec::new()
            )),
            Err(err) if err.status() == 401 => {
                StartPasskeyRegistrationServerResponse::Unauthorized(envelope!(
                    StartPasskeyRegistrationResponse401,
                    401,
                    None,
                    vec![err.to_error()]
                ))
            }
            Err(err) => StartPasskeyRegistrationServerResponse::InternalServerError(envelope!(
                StartPasskeyRegistrationResponse500,
                500,
                None,
                vec![err.to_error()]
            )),
        }
    }

    async fn finish_passkey_registration(
        &self,
        authorization: Option<String>,
        host_session: Option<String>,
        body: PasskeyCredential,
    ) -> FinishPasskeyRegistrationResponse {
        match crate::auth::finish_passkey_registration(
            self,
            presented(authorization.as_deref(), host_session.as_deref()),
            body,
        ) {
            Ok(account) => FinishPasskeyRegistrationResponse::Created(envelope!(
                FinishPasskeyRegistrationResponse201,
                201,
                Some(account),
                Vec::new()
            )),
            Err(err) if err.status() == 400 => {
                FinishPasskeyRegistrationResponse::BadRequest(envelope!(
                    FinishPasskeyRegistrationResponse400,
                    400,
                    None,
                    vec![err.to_error()]
                ))
            }
            Err(err) if err.status() == 401 => {
                FinishPasskeyRegistrationResponse::Unauthorized(envelope!(
                    FinishPasskeyRegistrationResponse401,
                    401,
                    None,
                    vec![err.to_error()]
                ))
            }
            Err(err) => FinishPasskeyRegistrationResponse::InternalServerError(envelope!(
                FinishPasskeyRegistrationResponse500,
                500,
                None,
                vec![err.to_error()]
            )),
        }
    }

    async fn start_passkey_login(&self, body: PasskeyLogin) -> StartPasskeyLoginServerResponse {
        match crate::auth::start_passkey_login(self, &body) {
            Ok(challenge) => StartPasskeyLoginServerResponse::Ok(envelope!(
                StartPasskeyLoginResponse,
                200,
                Some(challenge),
                Vec::new()
            )),
            Err(err) if err.status() == 400 => {
                StartPasskeyLoginServerResponse::BadRequest(envelope!(
                    StartPasskeyLoginResponse400,
                    400,
                    None,
                    vec![err.to_error()]
                ))
            }
            Err(err) => StartPasskeyLoginServerResponse::InternalServerError(envelope!(
                StartPasskeyLoginResponse500,
                500,
                None,
                vec![err.to_error()]
            )),
        }
    }

    async fn finish_passkey_login(
        &self,
        body: PasskeyCredential,
    ) -> FinishPasskeyLoginServerResponse {
        match crate::auth::finish_passkey_login(self, body) {
            Ok(session) => {
                let headers = FinishPasskeyLoginServerResponseOkHeaders {
                    set_cookie: vec![session_cookie(&session.token)],
                };
                FinishPasskeyLoginServerResponse::Ok(
                    envelope!(FinishPasskeyLoginResponse, 200, Some(session), Vec::new()),
                    headers,
                )
            }
            Err(err) if err.status() == 400 => {
                FinishPasskeyLoginServerResponse::BadRequest(envelope!(
                    FinishPasskeyLoginResponse400,
                    400,
                    None,
                    vec![err.to_error()]
                ))
            }
            Err(err) if err.status() == 401 => {
                FinishPasskeyLoginServerResponse::Unauthorized(envelope!(
                    FinishPasskeyLoginResponse401,
                    401,
                    None,
                    vec![err.to_error()]
                ))
            }
            Err(err) => FinishPasskeyLoginServerResponse::InternalServerError(envelope!(
                FinishPasskeyLoginResponse500,
                500,
                None,
                vec![err.to_error()]
            )),
        }
    }
}
