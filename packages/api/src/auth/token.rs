//! Stateless signed tokens (HS256 JWTs).
//!
//! Two kinds share one key, told apart by the `typ` claim so neither can
//! stand in for the other:
//!
//! - **Session** tokens ([`SessionClaims`]) are the bearer tokens clients
//!   send as `Authorization: Bearer <token>`. Nothing is stored server-side;
//!   a token is valid until `exp`, so signing out is client-side only.
//!   Rotating `AUTH_TOKEN_SECRET` invalidates every outstanding token.
//! - **Ceremony** tokens ([`CeremonyClaims`]) carry `webauthn-rs`'s
//!   in-flight passkey state between the `options` and `finish` calls, so no
//!   server holds ceremony state either. They expire after
//!   [`CEREMONY_TTL`]. The signature makes the state tamper-proof; it is not
//!   secret (a challenge and public credential ids).

use argon2::password_hash::rand_core::{OsRng, RngCore};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// How long a session token is accepted after sign-in.
pub const SESSION_TTL: chrono::TimeDelta = chrono::TimeDelta::hours(24);

/// How long a passkey ceremony may wait between `options` and `finish`
/// (the WebAuthn-recommended authenticator timeout).
pub const CEREMONY_TTL: chrono::TimeDelta = chrono::TimeDelta::minutes(5);

/// Minimum `AUTH_TOKEN_SECRET` length in bytes (HS256 key strength).
pub const MIN_SECRET_LEN: usize = 32;

/// What a token may be used for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    Session,
    PasskeyRegistration,
    PasskeyLogin,
}

/// Claims of a session (bearer) token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionClaims {
    pub typ: Purpose,
    /// User id.
    pub sub: String,
    pub iat: i64,
    pub exp: i64,
}

/// Claims of a passkey ceremony token: the user it is for plus the
/// `webauthn-rs` state to resume with.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CeremonyClaims<S> {
    pub typ: Purpose,
    /// User id.
    pub sub: String,
    pub iat: i64,
    pub exp: i64,
    pub state: S,
}

/// HS256 signing/verification keys.
pub struct TokenKeys {
    encoding: EncodingKey,
    decoding: DecodingKey,
}

impl TokenKeys {
    /// Keys from a shared secret.
    ///
    /// # Errors
    ///
    /// Returns a description when the secret is shorter than
    /// [`MIN_SECRET_LEN`] bytes.
    pub fn from_secret(secret: &[u8]) -> Result<Self, String> {
        if secret.len() < MIN_SECRET_LEN {
            return Err(format!(
                "AUTH_TOKEN_SECRET must be at least {MIN_SECRET_LEN} bytes"
            ));
        }
        Ok(Self {
            encoding: EncodingKey::from_secret(secret),
            decoding: DecodingKey::from_secret(secret),
        })
    }

    /// Keys from a fresh random secret: tokens die with the process. Used
    /// when no secret is configured (tests, bare `AppState`s).
    #[must_use]
    pub fn random() -> Self {
        let mut secret = [0u8; MIN_SECRET_LEN];
        OsRng.fill_bytes(&mut secret);
        Self {
            encoding: EncodingKey::from_secret(&secret),
            decoding: DecodingKey::from_secret(&secret),
        }
    }

    /// Sign a session token for `user_id`, returning it with its expiry.
    ///
    /// # Errors
    ///
    /// Returns a description when the expiry overflows or signing fails.
    pub fn issue_session(
        &self,
        user_id: &str,
    ) -> Result<(String, chrono::DateTime<chrono::Utc>), String> {
        let (iat, exp) = window(SESSION_TTL)?;
        let claims = SessionClaims {
            typ: Purpose::Session,
            sub: user_id.to_owned(),
            iat: iat.timestamp(),
            exp: exp.timestamp(),
        };
        self.sign(&claims).map(|token| (token, exp))
    }

    /// The user id of a valid, unexpired session token.
    #[must_use]
    pub fn verify_session(&self, token: &str) -> Option<String> {
        self.verify::<SessionClaims>(token)
            .filter(|claims| claims.typ == Purpose::Session)
            .map(|claims| claims.sub)
    }

    /// Sign a ceremony token carrying `state` for `user_id`.
    ///
    /// # Errors
    ///
    /// Returns a description when the expiry overflows or signing fails.
    pub fn issue_ceremony<S: Serialize>(
        &self,
        purpose: Purpose,
        user_id: &str,
        state: S,
    ) -> Result<String, String> {
        let (iat, exp) = window(CEREMONY_TTL)?;
        self.sign(&CeremonyClaims {
            typ: purpose,
            sub: user_id.to_owned(),
            iat: iat.timestamp(),
            exp: exp.timestamp(),
            state,
        })
    }

    /// The user id and state of a valid, unexpired ceremony token of the
    /// given `purpose`.
    #[must_use]
    pub fn verify_ceremony<S: DeserializeOwned>(
        &self,
        purpose: Purpose,
        token: &str,
    ) -> Option<(String, S)> {
        self.verify::<CeremonyClaims<S>>(token)
            .filter(|claims| claims.typ == purpose)
            .map(|claims| (claims.sub, claims.state))
    }

    fn sign<C: Serialize>(&self, claims: &C) -> Result<String, String> {
        jsonwebtoken::encode(&Header::new(Algorithm::HS256), claims, &self.encoding)
            .map_err(|err| err.to_string())
    }

    fn verify<C: DeserializeOwned>(&self, token: &str) -> Option<C> {
        // Pinned to HS256; `exp` is required and checked (default 60s leeway).
        let validation = Validation::new(Algorithm::HS256);
        jsonwebtoken::decode::<C>(token, &self.decoding, &validation)
            .ok()
            .map(|data| data.claims)
    }
}

/// `(now, now + ttl)`.
fn window(
    ttl: chrono::TimeDelta,
) -> Result<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>), String> {
    let now = chrono::Utc::now();
    now.checked_add_signed(ttl)
        .map(|exp| (now, exp))
        .ok_or_else(|| "token expiry out of range".to_owned())
}

/// Name of the cookie carrying the session token. The `__Host-` prefix makes
/// browsers reject it unless it is `Secure`, `Path=/`, and has no `Domain`,
/// so a sibling subdomain cannot plant or overwrite it.
pub const SESSION_COOKIE: &str = "__Host-session";

/// `Set-Cookie` value storing a session `token` for [`SESSION_TTL`].
/// `HttpOnly` keeps it away from page scripts; `SameSite=Lax` keeps it off
/// cross-site POSTs.
#[must_use]
pub fn session_cookie(token: &str) -> String {
    format!(
        "{SESSION_COOKIE}={token}; Path=/; Max-Age={}; HttpOnly; Secure; SameSite=Lax",
        SESSION_TTL.num_seconds()
    )
}

/// `Set-Cookie` value removing the session cookie.
#[must_use]
pub fn clear_session_cookie() -> String {
    format!("{SESSION_COOKIE}=; Path=/; Max-Age=0; HttpOnly; Secure; SameSite=Lax")
}

/// The token from an `Authorization: Bearer <token>` header value.
#[must_use]
pub fn bearer(header: Option<&str>) -> Option<&str> {
    let (scheme, token) = header?.trim().split_once(' ')?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn short_secrets_are_rejected() {
        assert!(TokenKeys::from_secret(b"too short").is_err());
        assert!(TokenKeys::from_secret(&[7; MIN_SECRET_LEN]).is_ok());
    }

    #[test]
    fn session_tokens_round_trip_only_under_their_key() {
        let keys = TokenKeys::random();
        let (token, exp) = keys.issue_session("user-1").expect("signs");
        assert!(exp > chrono::Utc::now());
        assert_eq!(keys.verify_session(&token), Some("user-1".to_owned()));
        assert_eq!(TokenKeys::random().verify_session(&token), None);
        assert_eq!(keys.verify_session("not.a.jwt"), None);
    }

    #[test]
    fn tampered_tokens_are_rejected() {
        let keys = TokenKeys::random();
        let (token, _) = keys.issue_session("user-1").expect("signs");
        let mut forged = token.into_bytes();
        let last = forged.last_mut().expect("non-empty");
        *last = if *last == b'A' { b'B' } else { b'A' };
        let forged = String::from_utf8(forged).expect("ascii");
        assert_eq!(keys.verify_session(&forged), None);
    }

    #[test]
    fn expired_tokens_are_rejected() {
        let keys = TokenKeys::random();
        let past = chrono::Utc::now()
            .checked_sub_signed(chrono::TimeDelta::hours(1))
            .expect("in range")
            .timestamp();
        let token = keys
            .sign(&SessionClaims {
                typ: Purpose::Session,
                sub: "user-1".to_owned(),
                iat: past,
                exp: past,
            })
            .expect("signs");
        assert_eq!(keys.verify_session(&token), None);
    }

    #[test]
    fn purposes_are_not_interchangeable() {
        let keys = TokenKeys::random();
        let ceremony = keys
            .issue_ceremony(Purpose::PasskeyLogin, "user-1", "state".to_owned())
            .expect("signs");
        assert_eq!(keys.verify_session(&ceremony), None);
        assert_eq!(
            keys.verify_ceremony::<String>(Purpose::PasskeyRegistration, &ceremony),
            None
        );
        assert_eq!(
            keys.verify_ceremony::<String>(Purpose::PasskeyLogin, &ceremony),
            Some(("user-1".to_owned(), "state".to_owned()))
        );

        let (session, _) = keys.issue_session("user-1").expect("signs");
        assert_eq!(
            keys.verify_ceremony::<String>(Purpose::PasskeyLogin, &session),
            None
        );
    }

    #[test]
    fn cookies_are_hardened() {
        let cookie = session_cookie("abc");
        assert!(cookie.starts_with("__Host-session=abc;"));
        for attribute in [
            "Path=/",
            "HttpOnly",
            "Secure",
            "SameSite=Lax",
            "Max-Age=86400",
        ] {
            assert!(cookie.contains(attribute), "missing {attribute}");
        }
        assert!(clear_session_cookie().starts_with("__Host-session=;"));
        assert!(clear_session_cookie().contains("Max-Age=0"));
    }

    #[test]
    fn bearer_header_parsing() {
        assert_eq!(bearer(Some("Bearer abc")), Some("abc"));
        assert_eq!(bearer(Some("bearer  abc ")), Some("abc"));
        assert_eq!(bearer(Some("Basic abc")), None);
        assert_eq!(bearer(Some("Bearer ")), None);
        assert_eq!(bearer(Some("abc")), None);
        assert_eq!(bearer(None), None);
    }
}
