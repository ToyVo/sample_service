//! Username/password and passkey authentication with stateless tokens.
//!
//! The `/api/auth/*` operations are codegen operations in
//! `domain.oas3.json` (request validation, including username/password
//! rules, happens in the generated router). This module holds their logic;
//! `routes.rs` maps each [`AuthResult`] onto the generated response
//! envelopes, the same split as `store.rs` / `routes.rs` for samples.
//!
//! - Sign-in (`register`, `login`, passkey `finish`) returns a [`Session`]:
//!   an HS256 token valid for [`token::SESSION_TTL`], both in the body and as
//!   an `HttpOnly` [`token::SESSION_COOKIE`] (see [`token::session_cookie`]).
//!   Protected operations accept it as `Authorization: Bearer <token>` (API
//!   clients) or as that cookie (browsers). No session state is stored, so
//!   `logout` can only clear the cookie.
//! - Passkey ceremonies carry their `webauthn-rs` state in a signed ceremony
//!   token (`ceremony`), valid for [`token::CEREMONY_TTL`], so no server
//!   holds that either. Accounts and passkeys are the only persisted rows.

pub mod passkey;
pub mod password;
pub mod store;
pub mod token;

use diesel::result::{DatabaseErrorKind, Error as DieselError};
use webauthn_rs::Webauthn;
use webauthn_rs::prelude::{
    Passkey, PasskeyAuthentication, PasskeyRegistration, PublicKeyCredential,
    RegisterPublicKeyCredential,
};

pub use passkey::PasskeyConfig;
pub use store::{AuthConnection, PasskeyData, UserData};
pub use token::{Purpose, TokenKeys};

use crate::models::UUID;
use crate::{
    Account, AppState, Credentials, Error, ErrorCode, PasskeyChallenge, PasskeyChallengeOptions,
    PasskeyCredential, PasskeyLogin, Session, lock_db,
};

/// Passkey relying-party config plus token signing keys, shared by all
/// requests through [`AppState::auth`].
pub struct AuthState {
    pub passkeys: PasskeyConfig,
    pub tokens: TokenKeys,
}

impl Default for AuthState {
    /// `localhost` passkeys and a random per-process signing key: tokens
    /// don't survive a restart. For tests and bare `AppState`s.
    fn default() -> Self {
        Self {
            passkeys: PasskeyConfig::default(),
            tokens: TokenKeys::random(),
        }
    }
}

impl AuthState {
    /// Read `AUTH_TOKEN_SECRET`, `WEBAUTHN_RP_ID`, and `WEBAUTHN_ORIGIN`.
    ///
    /// # Errors
    ///
    /// Returns a description when the secret is missing or too short, or
    /// the passkey config is invalid.
    pub fn from_env() -> Result<Self, String> {
        let secret = std::env::var("AUTH_TOKEN_SECRET")
            .map_err(|_| "AUTH_TOKEN_SECRET is not set".to_owned())?;
        Ok(Self {
            passkeys: PasskeyConfig::from_env()?,
            tokens: TokenKeys::from_secret(secret.as_bytes())?,
        })
    }
}

/// Every way an auth operation can fail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    /// 401: wrong username/password or a failed passkey assertion. One
    /// message for both so responses don't reveal which part was wrong.
    InvalidCredentials,
    /// 401: missing, malformed, expired, or forged session token (bearer or
    /// cookie), or its account no longer exists.
    Unauthenticated,
    /// 409: the username is already registered.
    UsernameTaken,
    /// 400: the account doesn't exist or has no passkeys.
    NoPasskeys,
    /// 400: missing, expired, forged, or mismatched ceremony token.
    CeremonyExpired,
    /// 400: the authenticator's response is malformed or fails verification.
    InvalidPasskey(String),
    /// 500: storage or configuration failure (detail is logged, not sent).
    Internal(String),
}

impl AuthError {
    /// HTTP status for this error.
    #[must_use]
    pub const fn status(&self) -> u16 {
        match self {
            Self::InvalidCredentials | Self::Unauthenticated => 401,
            Self::UsernameTaken => 409,
            Self::NoPasskeys | Self::CeremonyExpired | Self::InvalidPasskey(_) => 400,
            Self::Internal(_) => 500,
        }
    }

    /// The `errors[]` entry for the response envelope.
    #[must_use]
    pub fn to_error(&self) -> Error {
        let (code, title, location, message) = match self {
            Self::InvalidCredentials => (
                ErrorCode::Unauthorized,
                "Unauthorized",
                "body",
                "Incorrect username or password.".to_owned(),
            ),
            Self::Unauthenticated => (
                ErrorCode::Unauthorized,
                "Unauthorized",
                "header.Authorization | cookie.__Host-session",
                "You need to sign in first.".to_owned(),
            ),
            Self::UsernameTaken => (
                ErrorCode::Conflict,
                "Conflict",
                "body.username",
                "That username is already taken.".to_owned(),
            ),
            Self::NoPasskeys => (
                ErrorCode::BadRequest,
                "Bad Request",
                "body.username",
                "This account has no passkeys. Sign in with your password, then add one."
                    .to_owned(),
            ),
            Self::CeremonyExpired => (
                ErrorCode::BadRequest,
                "Bad Request",
                "body.ceremony",
                "The passkey request expired. Please try again.".to_owned(),
            ),
            Self::InvalidPasskey(detail) => (
                ErrorCode::BadRequest,
                "Bad Request",
                "body.credential",
                format!("The passkey could not be verified: {detail}"),
            ),
            Self::Internal(detail) => {
                tracing::error!(detail, "auth operation failed");
                (
                    ErrorCode::InternalServerError,
                    "Internal Server Error",
                    "server",
                    "Something went wrong. Please try again.".to_owned(),
                )
            }
        };
        Error {
            code,
            id: uuid::Uuid::now_v7().to_string(),
            location: Some(location.to_owned()),
            message: Some(message),
            source: None,
            status: i64::from(self.status()),
            title: title.to_owned(),
        }
    }
}

impl From<DieselError> for AuthError {
    fn from(err: DieselError) -> Self {
        Self::Internal(err.to_string())
    }
}

pub type AuthResult<T> = Result<T, AuthError>;

/// Create an account and sign in.
///
/// The generated router has already enforced the spec's username pattern
/// and password length; usernames are case-insensitive, stored lowercased.
///
/// # Errors
///
/// [`AuthError::UsernameTaken`] or [`AuthError::Internal`].
pub async fn register(state: &AppState, body: Credentials) -> AuthResult<Session> {
    let username = body.username.to_lowercase();
    let password_hash = blocking(move || password::hash_password(&body.password))
        .await?
        .map_err(AuthError::Internal)?;
    let user = UserData {
        id: UUID::random(),
        username,
        password_hash,
        created_at: now(),
    };
    let inserted = lock_db(&state.db).insert_user(user.clone());
    match inserted {
        Ok(()) => session(state, &user),
        Err(DieselError::DatabaseError(DatabaseErrorKind::UniqueViolation, _)) => {
            Err(AuthError::UsernameTaken)
        }
        Err(err) => Err(err.into()),
    }
}

/// Sign in with a username and password.
///
/// # Errors
///
/// [`AuthError::InvalidCredentials`] or [`AuthError::Internal`].
pub async fn login(state: &AppState, body: Credentials) -> AuthResult<Session> {
    let found = lock_db(&state.db).fetch_user_by_username(&body.username.to_lowercase());
    let user = match found {
        Ok(user) => Some(user),
        Err(DieselError::NotFound) => None,
        Err(err) => return Err(err.into()),
    };
    let verified = blocking(move || {
        if let Some(user) = user {
            password::verify_password(&body.password, &user.password_hash).then_some(user)
        } else {
            // Unknown usernames cost a hash too, so timing doesn't reveal them.
            password::verify_dummy_password(&body.password);
            None
        }
    })
    .await?;
    verified.map_or(Err(AuthError::InvalidCredentials), |user| {
        session(state, &user)
    })
}

/// The session token a request presented: an `Authorization` header value
/// and/or the session cookie. The header wins when both are present.
#[derive(Debug, Clone, Copy, Default)]
pub struct Presented<'a> {
    pub authorization: Option<&'a str>,
    pub cookie: Option<&'a str>,
}

/// The account behind a session token.
///
/// # Errors
///
/// [`AuthError::Unauthenticated`] or [`AuthError::Internal`].
pub fn account(state: &AppState, presented: Presented<'_>) -> AuthResult<Account> {
    let user = authenticate(state, presented)?;
    to_account(state, &user)
}

/// WebAuthn creation options for adding a passkey to the signed-in account.
///
/// # Errors
///
/// [`AuthError::Unauthenticated`] or [`AuthError::Internal`].
pub fn start_passkey_registration(
    state: &AppState,
    presented: Presented<'_>,
) -> AuthResult<PasskeyChallenge> {
    let user = authenticate(state, presented)?;
    // Excluding existing credentials stops an authenticator from
    // registering the same passkey twice.
    let exclude = stored_passkeys(state, user.id)?
        .into_iter()
        .map(|(_, passkey)| passkey.cred_id().clone())
        .collect();
    let (options, registration) = webauthn(state)?
        .start_passkey_registration(user.id.0, &user.username, &user.username, Some(exclude))
        .map_err(|err| AuthError::Internal(err.to_string()))?;
    let ceremony = state
        .auth
        .tokens
        .issue_ceremony(
            Purpose::PasskeyRegistration,
            &user.id.to_string(),
            registration,
        )
        .map_err(AuthError::Internal)?;
    Ok(PasskeyChallenge {
        ceremony,
        options: to_options(&options)?,
    })
}

/// Verify a new passkey and store it on the signed-in account.
///
/// # Errors
///
/// [`AuthError::Unauthenticated`], [`AuthError::CeremonyExpired`],
/// [`AuthError::InvalidPasskey`], or [`AuthError::Internal`].
pub fn finish_passkey_registration(
    state: &AppState,
    presented: Presented<'_>,
    body: PasskeyCredential,
) -> AuthResult<Account> {
    let user = authenticate(state, presented)?;
    let registration: PasskeyRegistration =
        ceremony_for(state, Purpose::PasskeyRegistration, &body.ceremony, user.id)?;
    let credential: RegisterPublicKeyCredential = from_credential(body)?;
    let passkey = webauthn(state)?
        .finish_passkey_registration(&credential, &registration)
        .map_err(|err| AuthError::InvalidPasskey(err.to_string()))?;
    let serialized =
        serde_json::to_string(&passkey).map_err(|err| AuthError::Internal(err.to_string()))?;
    let inserted = lock_db(&state.db).insert_passkey(PasskeyData {
        id: UUID::random(),
        user_id: user.id,
        credential_id: passkey.cred_id().to_vec(),
        passkey: serialized,
        created_at: now(),
        last_used_at: None,
    });
    match inserted {
        Ok(()) => to_account(state, &user),
        Err(DieselError::DatabaseError(DatabaseErrorKind::UniqueViolation, _)) => Err(
            AuthError::InvalidPasskey("that passkey is already registered".to_owned()),
        ),
        Err(err) => Err(err.into()),
    }
}

/// WebAuthn request options for signing in with one of an account's
/// passkeys.
///
/// # Errors
///
/// [`AuthError::NoPasskeys`] or [`AuthError::Internal`].
pub fn start_passkey_login(state: &AppState, body: &PasskeyLogin) -> AuthResult<PasskeyChallenge> {
    let found = lock_db(&state.db).fetch_user_by_username(&body.username.to_lowercase());
    let user = match found {
        Ok(user) => user,
        Err(DieselError::NotFound) => return Err(AuthError::NoPasskeys),
        Err(err) => return Err(err.into()),
    };
    let passkeys: Vec<Passkey> = stored_passkeys(state, user.id)?
        .into_iter()
        .map(|(_, passkey)| passkey)
        .collect();
    if passkeys.is_empty() {
        return Err(AuthError::NoPasskeys);
    }
    let (options, authentication) = webauthn(state)?
        .start_passkey_authentication(&passkeys)
        .map_err(|err| AuthError::Internal(err.to_string()))?;
    let ceremony = state
        .auth
        .tokens
        .issue_ceremony(Purpose::PasskeyLogin, &user.id.to_string(), authentication)
        .map_err(AuthError::Internal)?;
    Ok(PasskeyChallenge {
        ceremony,
        options: to_options(&options)?,
    })
}

/// Verify a passkey assertion and sign in.
///
/// # Errors
///
/// [`AuthError::CeremonyExpired`], [`AuthError::InvalidPasskey`],
/// [`AuthError::InvalidCredentials`], or [`AuthError::Internal`].
pub fn finish_passkey_login(state: &AppState, body: PasskeyCredential) -> AuthResult<Session> {
    let (user_id, authentication) = state
        .auth
        .tokens
        .verify_ceremony::<PasskeyAuthentication>(Purpose::PasskeyLogin, &body.ceremony)
        .ok_or(AuthError::CeremonyExpired)?;
    let user_id = parse_user_id(&user_id).ok_or(AuthError::CeremonyExpired)?;
    let credential: PublicKeyCredential = from_credential(body)?;
    let result = webauthn(state)?
        .finish_passkey_authentication(&credential, &authentication)
        .map_err(|err| {
            tracing::warn!(%err, "passkey assertion rejected");
            AuthError::InvalidCredentials
        })?;
    // Persist the authenticator's new signature counter / backup state so a
    // cloned authenticator is detectable on a later sign-in.
    for (row, mut passkey) in stored_passkeys(state, user_id)? {
        if passkey.update_credential(&result).is_some() {
            let serialized = serde_json::to_string(&passkey)
                .map_err(|err| AuthError::Internal(err.to_string()))?;
            lock_db(&state.db).touch_passkey(row.id, serialized, now())?;
        }
    }
    let found = lock_db(&state.db).fetch_user(user_id);
    match found {
        Ok(user) => session(state, &user),
        Err(DieselError::NotFound) => Err(AuthError::InvalidCredentials),
        Err(err) => Err(err.into()),
    }
}

/// The user behind a presented session token.
fn authenticate(state: &AppState, presented: Presented<'_>) -> AuthResult<UserData> {
    let user_id = token::bearer(presented.authorization)
        .or_else(|| presented.cookie.filter(|cookie| !cookie.is_empty()))
        .and_then(|token| state.auth.tokens.verify_session(token))
        .and_then(|sub| parse_user_id(&sub))
        .ok_or(AuthError::Unauthenticated)?;
    let found = lock_db(&state.db).fetch_user(user_id);
    match found {
        Ok(user) => Ok(user),
        Err(DieselError::NotFound) => Err(AuthError::Unauthenticated),
        Err(err) => Err(err.into()),
    }
}

/// Verify a ceremony token of `purpose` issued to `user_id`.
fn ceremony_for<S: serde::de::DeserializeOwned>(
    state: &AppState,
    purpose: Purpose,
    token: &str,
    user_id: UUID,
) -> AuthResult<S> {
    state
        .auth
        .tokens
        .verify_ceremony::<S>(purpose, token)
        .filter(|(sub, _)| parse_user_id(sub) == Some(user_id))
        .map(|(_, ceremony)| ceremony)
        .ok_or(AuthError::CeremonyExpired)
}

fn session(state: &AppState, user: &UserData) -> AuthResult<Session> {
    let (token, expires_at) = state
        .auth
        .tokens
        .issue_session(&user.id.to_string())
        .map_err(AuthError::Internal)?;
    Ok(Session {
        account: to_account(state, user)?,
        expires_at,
        token,
    })
}

fn to_account(state: &AppState, user: &UserData) -> AuthResult<Account> {
    let passkeys = lock_db(&state.db).fetch_passkeys(user.id)?.len();
    Ok(Account {
        id: user.id.to_string(),
        passkeys: i64::try_from(passkeys).unwrap_or(i64::MAX),
        username: user.username.clone(),
    })
}

/// A user's stored passkeys, decoded. Rows that fail to decode are logged
/// and skipped rather than locking the user out of their other passkeys.
fn stored_passkeys(state: &AppState, user_id: UUID) -> AuthResult<Vec<(PasskeyData, Passkey)>> {
    let rows = lock_db(&state.db).fetch_passkeys(user_id)?;
    Ok(rows
        .into_iter()
        .filter_map(|row| match serde_json::from_str::<Passkey>(&row.passkey) {
            Ok(passkey) => Some((row, passkey)),
            Err(err) => {
                tracing::error!(passkey_id = %row.id, %err, "stored passkey failed to decode");
                None
            }
        })
        .collect())
}

/// `webauthn-rs` options as the spec's free-form `options` object.
fn to_options<T: serde::Serialize>(options: &T) -> AuthResult<PasskeyChallengeOptions> {
    match serde_json::to_value(options) {
        Ok(serde_json::Value::Object(map)) => Ok(PasskeyChallengeOptions {
            additional_properties: map.into_iter().collect(),
        }),
        Ok(_) => Err(AuthError::Internal(
            "webauthn options are not an object".to_owned(),
        )),
        Err(err) => Err(AuthError::Internal(err.to_string())),
    }
}

/// The spec's free-form `credential` object as a `webauthn-rs` type.
fn from_credential<T: serde::de::DeserializeOwned>(body: PasskeyCredential) -> AuthResult<T> {
    let value =
        serde_json::Value::Object(body.credential.additional_properties.into_iter().collect());
    serde_json::from_value(value).map_err(|err| AuthError::InvalidPasskey(err.to_string()))
}

fn webauthn(state: &AppState) -> AuthResult<Webauthn> {
    state.auth.passkeys.webauthn().map_err(AuthError::Internal)
}

fn parse_user_id(sub: &str) -> Option<UUID> {
    uuid::Uuid::parse_str(sub).ok().map(UUID)
}

async fn blocking<T, F>(work: F) -> AuthResult<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|err| AuthError::Internal(err.to_string()))
}

fn now() -> chrono::NaiveDateTime {
    chrono::Utc::now().naive_utc()
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn errors_carry_matching_status_and_code() {
        for (error, status, code) in [
            (AuthError::InvalidCredentials, 401, ErrorCode::Unauthorized),
            (AuthError::Unauthenticated, 401, ErrorCode::Unauthorized),
            (AuthError::UsernameTaken, 409, ErrorCode::Conflict),
            (AuthError::NoPasskeys, 400, ErrorCode::BadRequest),
            (AuthError::CeremonyExpired, 400, ErrorCode::BadRequest),
            (
                AuthError::InvalidPasskey("bad".to_owned()),
                400,
                ErrorCode::BadRequest,
            ),
            (
                AuthError::Internal("db down".to_owned()),
                500,
                ErrorCode::InternalServerError,
            ),
        ] {
            assert_eq!(error.status(), status);
            let rendered = error.to_error();
            assert_eq!(rendered.code, code);
            assert_eq!(rendered.status, i64::from(status));
        }
        // Internal details are logged, never sent.
        let rendered = AuthError::Internal("db password=hunter2".to_owned()).to_error();
        assert!(!rendered.message.unwrap_or_default().contains("hunter2"));
    }

    #[test]
    fn options_and_credentials_cross_the_free_form_boundary() {
        let options = to_options(&serde_json::json!({"publicKey": {"challenge": "abc"}}))
            .expect("object converts");
        assert!(options.additional_properties.contains_key("publicKey"));
        assert!(to_options(&serde_json::json!("not an object")).is_err());

        let body = PasskeyCredential {
            ceremony: String::new(),
            credential: crate::PasskeyCredentialCredential {
                additional_properties: std::iter::once(("id".to_owned(), serde_json::json!(1)))
                    .collect(),
            },
        };
        assert!(matches!(
            from_credential::<PublicKeyCredential>(body),
            Err(AuthError::InvalidPasskey(_))
        ));
    }
}
