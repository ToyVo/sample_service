//! End-to-end auth coverage over a live socket: registration, password
//! sign-in, bearer and cookie sessions, request validation from the spec,
//! sign-out, and the passkey ceremony endpoints' failure paths. (Completing
//! a passkey ceremony needs a real authenticator, so success paths stop at
//! the WebAuthn options.)

#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::unwrap_used)]

mod support;

use api::AppState;
use reqwest::{StatusCode, header};
use serde_json::{Value, json};
use std::net::SocketAddr;

async fn serve(state: AppState) -> SocketAddr {
    let router = api::router_with_telemetry(state).fallback(api::fallback);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("binds ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, router).await.expect("serves");
    });
    addr
}

struct Api {
    base: String,
    client: reqwest::Client,
}

/// One response, decoded.
struct Reply {
    status: StatusCode,
    set_cookie: Vec<String>,
    body: Value,
}

impl Reply {
    /// `name=value` of the session cookie this response set.
    fn session_cookie(&self) -> String {
        self.set_cookie
            .iter()
            .find(|cookie| cookie.starts_with("__Host-session="))
            .and_then(|cookie| cookie.split(';').next())
            .expect("sets the session cookie")
            .to_owned()
    }

    fn token(&self) -> String {
        self.body["data"]["token"]
            .as_str()
            .expect("token")
            .to_owned()
    }

    fn first_error(&self) -> &Value {
        &self.body["errors"][0]
    }
}

impl Api {
    async fn new(state: AppState) -> Self {
        Self {
            base: format!("http://{}", serve(state).await),
            client: reqwest::Client::new(),
        }
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> Reply {
        let response = request.send().await.expect("request succeeds");
        let status = response.status();
        let set_cookie = response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|value| value.to_str().expect("ascii").to_owned())
            .collect();
        let text = response.text().await.expect("reads body");
        let body = if text.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).expect("json body")
        };
        Reply {
            status,
            set_cookie,
            body,
        }
    }

    async fn post(&self, path: &str, body: Option<Value>, auth: Auth<'_>) -> Reply {
        let mut request = auth.apply(self.client.post(format!("{}{path}", self.base)));
        if let Some(body) = body {
            request = request
                .header(header::CONTENT_TYPE, "application/json")
                .body(body.to_string());
        }
        self.send(request).await
    }

    async fn me(&self, auth: Auth<'_>) -> Reply {
        self.send(auth.apply(self.client.get(format!("{}/api/auth/me", self.base))))
            .await
    }

    async fn register(&self, username: &str, password: &str) -> Reply {
        let body = json!({ "username": username, "password": password });
        self.post("/api/auth/register", Some(body), Auth::None)
            .await
    }

    async fn login(&self, username: &str, password: &str) -> Reply {
        let body = json!({ "username": username, "password": password });
        self.post("/api/auth/login", Some(body), Auth::None).await
    }
}

#[derive(Clone, Copy)]
enum Auth<'a> {
    None,
    Bearer(&'a str),
    Cookie(&'a str),
}

impl Auth<'_> {
    fn apply(self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self {
            Self::None => request,
            Self::Bearer(token) => request.header(header::AUTHORIZATION, format!("Bearer {token}")),
            Self::Cookie(cookie) => request.header(header::COOKIE, cookie),
        }
    }
}

#[tokio::test]
async fn register_signs_in_with_a_hardened_cookie_and_a_bearer_token() {
    crate::on_all_backends!("db_auth_register", |state| {
        let api = Api::new(state.clone()).await;
        let reply = api.register("Alice", "correct horse").await;
        assert_eq!(reply.status, StatusCode::CREATED);
        assert_eq!(reply.body["data"]["account"]["username"], "alice");
        assert_eq!(reply.body["data"]["account"]["passkeys"], 0);
        assert!(reply.body["data"]["expiresAt"].is_string());

        let cookie = reply
            .set_cookie
            .iter()
            .find(|cookie| cookie.starts_with("__Host-session="))
            .expect("session cookie");
        for attribute in ["Path=/", "HttpOnly", "Secure", "SameSite=Lax", "Max-Age="] {
            assert!(cookie.contains(attribute), "{cookie} lacks {attribute}");
        }
        // The cookie and the body carry the same stateless token.
        assert_eq!(
            reply.session_cookie(),
            format!("__Host-session={}", reply.token())
        );

        for auth in [
            Auth::Bearer(&reply.token()),
            Auth::Cookie(&reply.session_cookie()),
        ] {
            let me = api.me(auth).await;
            assert_eq!(me.status, StatusCode::OK);
            assert_eq!(me.body["data"]["username"], "alice");
        }
    });
}

#[tokio::test]
async fn usernames_are_unique_case_insensitively() {
    crate::on_all_backends!("db_auth_unique", |state| {
        let api = Api::new(state.clone()).await;
        assert_eq!(
            api.register("alice", "correct horse").await.status,
            StatusCode::CREATED
        );
        let duplicate = api.register("ALICE", "another password").await;
        assert_eq!(duplicate.status, StatusCode::CONFLICT);
        assert_eq!(duplicate.first_error()["code"], "CONFLICT");
        assert!(duplicate.set_cookie.is_empty());
    });
}

#[tokio::test]
async fn spec_constraints_reject_bad_usernames_and_passwords() {
    crate::on_all_backends!("db_auth_constraints", |state| {
        let api = Api::new(state.clone()).await;
        for (username, password) in [
            ("a b", "correct horse"),
            ("ab", "correct horse"),
            (&"x".repeat(33), "correct horse"),
            ("alice", "short"),
        ] {
            let reply = api.register(username, password).await;
            assert_eq!(
                reply.status,
                StatusCode::UNPROCESSABLE_ENTITY,
                "{username:?} / {password:?}"
            );
        }
        let missing = api
            .post(
                "/api/auth/login",
                Some(json!({ "username": "alice" })),
                Auth::None,
            )
            .await;
        assert_eq!(missing.status, StatusCode::UNPROCESSABLE_ENTITY);
    });
}

#[tokio::test]
async fn login_rejects_wrong_passwords_and_unknown_users_identically() {
    crate::on_all_backends!("db_auth_login", |state| {
        let api = Api::new(state.clone()).await;
        api.register("alice", "correct horse").await;

        let wrong = api.login("alice", "wrong horse").await;
        let unknown = api.login("bob", "correct horse").await;
        for reply in [&wrong, &unknown] {
            assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
            assert!(reply.set_cookie.is_empty());
        }
        assert_eq!(
            wrong.first_error()["message"],
            unknown.first_error()["message"]
        );

        let ok = api.login("ALICE", "correct horse").await;
        assert_eq!(ok.status, StatusCode::OK);
        assert_eq!(ok.body["data"]["account"]["username"], "alice");
        assert!(!ok.session_cookie().is_empty());
    });
}

#[tokio::test]
async fn invalid_or_foreign_tokens_are_unauthenticated() {
    crate::on_all_backends!("db_auth_tokens", |state| {
        let api = Api::new(state.clone()).await;
        let token = api.register("alice", "correct horse").await.token();

        // A token signed by another server's key.
        let other = Api::new(support::sqlite_state()).await;
        let foreign = other.register("alice", "correct horse").await.token();

        let tampered = format!("{token}x");
        for auth in [
            Auth::None,
            Auth::Bearer("garbage"),
            Auth::Bearer(&tampered),
            Auth::Bearer(&foreign),
            Auth::Cookie("__Host-session=garbage"),
        ] {
            let me = api.me(auth).await;
            assert_eq!(me.status, StatusCode::UNAUTHORIZED);
            assert_eq!(me.first_error()["code"], "UNAUTHORIZED");
        }
    });
}

#[tokio::test]
async fn logout_clears_the_cookie() {
    crate::on_all_backends!("db_auth_logout", |state| {
        let api = Api::new(state.clone()).await;
        let reply = api.post("/api/auth/logout", None, Auth::None).await;
        assert_eq!(reply.status, StatusCode::NO_CONTENT);
        let cookie = reply.set_cookie.first().expect("clears cookie");
        assert!(cookie.starts_with("__Host-session=;"), "{cookie}");
        assert!(cookie.contains("Max-Age=0"), "{cookie}");
    });
}

#[tokio::test]
async fn passkey_registration_options_require_a_session() {
    crate::on_all_backends!("db_auth_pk_options", |state| {
        let api = Api::new(state.clone()).await;
        let reply = api.register("alice", "correct horse").await;

        let anonymous = api
            .post("/api/auth/passkeys/register/options", None, Auth::None)
            .await;
        assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);

        let options = api
            .post(
                "/api/auth/passkeys/register/options",
                None,
                Auth::Cookie(&reply.session_cookie()),
            )
            .await;
        assert_eq!(options.status, StatusCode::OK);
        let public_key = &options.body["data"]["options"]["publicKey"];
        assert!(public_key["challenge"].is_string());
        assert_eq!(public_key["rp"]["id"], "localhost");
        assert_eq!(public_key["user"]["name"], "alice");
        assert!(options.body["data"]["ceremony"].is_string());
    });
}

#[tokio::test]
async fn passkey_registration_rejects_forged_ceremonies_and_credentials() {
    crate::on_all_backends!("db_auth_pk_ceremonies", |state| {
        let api = Api::new(state.clone()).await;
        let alice = api.register("alice", "correct horse").await.token();
        let bob = api.register("bobby", "correct horse").await.token();

        let options = api
            .post(
                "/api/auth/passkeys/register/options",
                None,
                Auth::Bearer(&alice),
            )
            .await;
        let ceremony = options.body["data"]["ceremony"].as_str().unwrap();
        let finish = |ceremony: &str| {
            json!({ "ceremony": ceremony, "credential": { "id": "x", "rawId": "eA", "type": "public-key",
                "response": { "attestationObject": "AA", "clientDataJSON": "AA" } } })
        };

        // Alice's ceremony can't be finished by Bob, and a session token is not
        // a ceremony token.
        for (ceremony, token) in [(ceremony, bob.as_str()), (alice.as_str(), alice.as_str())] {
            let reply = api
                .post(
                    "/api/auth/passkeys/register",
                    Some(finish(ceremony)),
                    Auth::Bearer(token),
                )
                .await;
            assert_eq!(reply.status, StatusCode::BAD_REQUEST);
            assert_eq!(reply.first_error()["location"], "body.ceremony");
        }

        // The right ceremony with a bogus attestation fails verification.
        let reply = api
            .post(
                "/api/auth/passkeys/register",
                Some(finish(ceremony)),
                Auth::Bearer(&alice),
            )
            .await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST);
        assert_eq!(reply.first_error()["location"], "body.credential");
    });
}

#[tokio::test]
async fn passkey_login_needs_registered_passkeys_and_valid_ceremonies() {
    crate::on_all_backends!("db_auth_pk_login", |state| {
        let api = Api::new(state.clone()).await;
        api.register("alice", "correct horse").await;

        for username in ["alice", "nobody"] {
            let reply = api
                .post(
                    "/api/auth/passkeys/login/options",
                    Some(json!({ "username": username })),
                    Auth::None,
                )
                .await;
            assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{username}");
            assert_eq!(reply.first_error()["location"], "body.username");
        }

        let reply = api
            .post(
                "/api/auth/passkeys/login",
                Some(json!({ "ceremony": "forged", "credential": {} })),
                Auth::None,
            )
            .await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST);
        assert_eq!(reply.first_error()["location"], "body.ceremony");
        assert!(reply.set_cookie.is_empty());
    });
}

#[tokio::test]
async fn password_flow_works_on_postgres() {
    let Some(guard) = support::PgGuard::setup("auth_password_flow") else {
        return;
    };
    let api = Api::new(guard.state().clone()).await;
    assert_eq!(
        api.register("alice", "correct horse").await.status,
        StatusCode::CREATED
    );
    assert_eq!(
        api.register("Alice", "correct horse").await.status,
        StatusCode::CONFLICT
    );
    let login = api.login("alice", "correct horse").await;
    assert_eq!(login.status, StatusCode::OK);
    let me = api.me(Auth::Cookie(&login.session_cookie())).await;
    assert_eq!(me.body["data"]["username"], "alice");
    guard.cleanup();
}
