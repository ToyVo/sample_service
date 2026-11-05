//! Sign-in page: username/password and passkeys against `/api/auth/*`.
//!
//! Sessions ride on the `__Host-session` cookie the API sets on sign-in
//! (`HttpOnly`, so this code never touches the token). Requests are
//! same-origin `fetch`es, which send that cookie automatically.
//!
//! Passkey ceremonies call `navigator.credentials` through
//! [`document::eval`]: the API hands back WebAuthn options with binary fields
//! base64url-encoded, [`WEBAUTHN_JS`] converts them to `ArrayBuffer`s, runs
//! the browser ceremony, and re-encodes the resulting credential for the
//! `finish` call.
//!
//! The session check runs in `use_effect`, i.e. only after hydration, so the
//! server-rendered HTML (the "checking" state) always matches the client's
//! first render.

use dioxus::prelude::*;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::components::{
    button::{Button, ButtonVariant},
    card::{Card, CardContent, CardDescription, CardFooter, CardHeader, CardTitle},
    input::Input,
    label::Label,
    separator::Separator,
};
use crate::console::{absolute_url, try_request};

/// The signed-in account (`#/components/schemas/account`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Account {
    pub id: String,
    pub username: String,
    pub passkeys: i64,
}

/// `#/components/schemas/session`; the token itself is ignored because the
/// cookie carries it.
#[derive(Debug, Clone, Deserialize)]
struct Session {
    account: Account,
}

/// `#/components/schemas/passkeyChallenge`.
#[derive(Debug, Clone, Deserialize)]
struct PasskeyChallenge {
    ceremony: String,
    options: Value,
}

/// The `data`/`errors` envelope every auth response uses. Request-validation
/// rejections (`application/problem+json`) share the `errors[].message`
/// shape, so one parser covers both.
#[derive(Debug, Deserialize)]
struct Envelope<T> {
    data: Option<T>,
    #[serde(default)]
    errors: Vec<EnvelopeError>,
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EnvelopeError {
    message: Option<String>,
}

/// Turn a status + body into the envelope's `data`, or a user-facing message.
fn parse_envelope<T: DeserializeOwned>(status: u16, body: &str) -> Result<T, String> {
    let message = match serde_json::from_str::<Envelope<T>>(body) {
        Ok(Envelope {
            data: Some(data), ..
        }) if (200..300).contains(&status) => return Ok(data),
        Ok(envelope) => envelope
            .errors
            .into_iter()
            .find_map(|error| error.message)
            .or(envelope.title),
        Err(_) => None,
    };
    Err(message.unwrap_or_else(|| format!("Request failed ({status}).")))
}

async fn call<T: DeserializeOwned>(
    method: &str,
    path: &str,
    body: Option<Value>,
) -> Result<T, String> {
    let (status, text) = try_request(
        method.to_owned(),
        absolute_url(path),
        body.map(|body| body.to_string()),
    )
    .await?;
    parse_envelope(status, &text)
}

/// Browser half of a passkey ceremony. Receives `{ mode, options }`, returns
/// `{ credential }` or `{ error }` (never throws, so `join` always resolves).
const WEBAUTHN_JS: &str = r#"
const { mode, options } = await dioxus.recv();
const toB64 = (buffer) => btoa(String.fromCharCode(...new Uint8Array(buffer)))
  .replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
const fromB64 = (text) => {
  let base64 = text.replace(/-/g, "+").replace(/_/g, "/");
  while (base64.length % 4) base64 += "=";
  return Uint8Array.from(atob(base64), (c) => c.charCodeAt(0));
};
try {
  if (!window.PublicKeyCredential) {
    return { error: "This browser doesn't support passkeys." };
  }
  const publicKey = options.publicKey;
  publicKey.challenge = fromB64(publicKey.challenge);
  if (publicKey.user) publicKey.user.id = fromB64(publicKey.user.id);
  for (const list of [publicKey.excludeCredentials, publicKey.allowCredentials]) {
    for (const credential of list || []) credential.id = fromB64(credential.id);
  }
  if (mode === "create") {
    const credential = await navigator.credentials.create({ publicKey });
    return { credential: {
      id: credential.id,
      rawId: toB64(credential.rawId),
      type: credential.type,
      response: {
        attestationObject: toB64(credential.response.attestationObject),
        clientDataJSON: toB64(credential.response.clientDataJSON),
      },
      extensions: credential.getClientExtensionResults(),
    } };
  }
  const credential = await navigator.credentials.get({ publicKey });
  const response = credential.response;
  return { credential: {
    id: credential.id,
    rawId: toB64(credential.rawId),
    type: credential.type,
    response: {
      authenticatorData: toB64(response.authenticatorData),
      clientDataJSON: toB64(response.clientDataJSON),
      signature: toB64(response.signature),
      userHandle: response.userHandle ? toB64(response.userHandle) : null,
    },
    extensions: credential.getClientExtensionResults(),
  } };
} catch (error) {
  if (error && error.name === "NotAllowedError") {
    return { error: "The passkey request was cancelled or timed out." };
  }
  if (error && error.name === "InvalidStateError") {
    return { error: "That passkey is already registered." };
  }
  return { error: String((error && error.message) || error) };
}
"#;

#[derive(Debug, Deserialize)]
struct CeremonyOutcome {
    credential: Option<Value>,
    error: Option<String>,
}

/// Run `navigator.credentials.create` (`mode = "create"`) or `.get`.
async fn browser_ceremony(mode: &str, options: Value) -> Result<Value, String> {
    let eval = document::eval(WEBAUTHN_JS);
    eval.send(json!({ "mode": mode, "options": options }))
        .map_err(|err| err.to_string())?;
    let outcome: CeremonyOutcome = eval.join().await.map_err(|err| err.to_string())?;
    match outcome {
        CeremonyOutcome {
            credential: Some(credential),
            ..
        } => Ok(credential),
        CeremonyOutcome {
            error: Some(error), ..
        } => Err(error),
        _ => Err("The browser returned no passkey.".to_owned()),
    }
}

async fn password_sign_in(
    register: bool,
    username: String,
    password: String,
) -> Result<Account, String> {
    let path = if register {
        "/api/auth/register"
    } else {
        "/api/auth/login"
    };
    let body = json!({ "username": username, "password": password });
    call::<Session>("POST", path, Some(body))
        .await
        .map(|session| session.account)
}

async fn passkey_sign_in(username: String) -> Result<Account, String> {
    let challenge: PasskeyChallenge = call(
        "POST",
        "/api/auth/passkeys/login/options",
        Some(json!({ "username": username })),
    )
    .await?;
    let credential = browser_ceremony("get", challenge.options).await?;
    let body = json!({ "ceremony": challenge.ceremony, "credential": credential });
    call::<Session>("POST", "/api/auth/passkeys/login", Some(body))
        .await
        .map(|session| session.account)
}

async fn add_passkey() -> Result<Account, String> {
    let challenge: PasskeyChallenge =
        call("POST", "/api/auth/passkeys/register/options", None).await?;
    let credential = browser_ceremony("create", challenge.options).await?;
    let body = json!({ "ceremony": challenge.ceremony, "credential": credential });
    call("POST", "/api/auth/passkeys/register", Some(body)).await
}

async fn sign_out() -> Result<(), String> {
    let (status, text) =
        try_request("POST".to_owned(), absolute_url("/api/auth/logout"), None).await?;
    if (200..300).contains(&status) {
        Ok(())
    } else {
        parse_envelope::<Value>(status, &text).map(|_| ())
    }
}

/// A status line under the form: success or error.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Notice {
    error: bool,
    text: String,
}

/// Sign-in / create-account page, or the account panel once signed in.
#[component]
pub fn Login() -> Element {
    let mut account = use_signal(|| None::<Account>);
    let mut checked = use_signal(|| false);
    let mut registering = use_signal(|| false);
    let mut username = use_signal(String::new);
    let mut password = use_signal(String::new);
    let mut pending = use_signal(|| false);
    let mut notice = use_signal(|| None::<Notice>);

    // Client-only: hydration has finished before effects run.
    use_effect(move || {
        spawn(async move {
            if let Ok(current) = call::<Account>("GET", "/api/auth/me", None).await {
                account.set(Some(current));
            }
            checked.set(true);
        });
    });

    // Run one action with the shared pending/notice bookkeeping.
    let mut run =
        move |action: std::pin::Pin<Box<dyn Future<Output = Result<Option<String>, String>>>>| {
            if pending() {
                return;
            }
            pending.set(true);
            notice.set(None);
            spawn(async move {
                match action.await {
                    Ok(Some(text)) => notice.set(Some(Notice { error: false, text })),
                    Ok(None) => {}
                    Err(text) => notice.set(Some(Notice { error: true, text })),
                }
                pending.set(false);
            });
        };

    let title = if account().is_some() {
        "Account"
    } else if registering() {
        "Create an account"
    } else {
        "Sign in"
    };

    rsx! {
        document::Title { {title} }
        Card {
            style: "max-width: 420px; margin: 48px auto;",
            if !checked() {
                CardHeader {
                    CardTitle { "Sign in" }
                    CardDescription { "Checking your session…" }
                }
            } else if let Some(current) = account() {
                CardHeader {
                    CardTitle { "Signed in as {current.username}" }
                    CardDescription {
                        match current.passkeys {
                            0 => "No passkeys yet. Add one to sign in without your password.".to_owned(),
                            1 => "1 passkey registered.".to_owned(),
                            count => format!("{count} passkeys registered."),
                        }
                    }
                }
                CardFooter {
                    style: "display: flex; gap: 8px; flex-wrap: wrap;",
                    Button {
                        disabled: pending(),
                        onclick: move |_| run(Box::pin(async move {
                            let updated = add_passkey().await?;
                            account.set(Some(updated));
                            Ok(Some("Passkey added.".to_owned()))
                        })),
                        "Add a passkey"
                    }
                    Button {
                        variant: ButtonVariant::Outline,
                        disabled: pending(),
                        onclick: move |_| run(Box::pin(async move {
                            sign_out().await?;
                            account.set(None);
                            password.set(String::new());
                            Ok(None)
                        })),
                        "Sign out"
                    }
                }
            } else {
                CardHeader {
                    CardTitle { {title} }
                    CardDescription {
                        if registering() {
                            "Pick a username and a password of at least 8 characters."
                        } else {
                            "Use your password or a passkey."
                        }
                    }
                }
                CardContent {
                    form {
                        style: "display: flex; flex-direction: column; gap: 12px;",
                        onsubmit: move |event: FormEvent| {
                            event.prevent_default();
                            let register = registering();
                            let (name, secret) = (username(), password());
                            run(Box::pin(async move {
                                let signed_in = password_sign_in(register, name, secret).await?;
                                account.set(Some(signed_in));
                                password.set(String::new());
                                Ok(None)
                            }));
                        },
                        Label { html_for: "login-username", "Username" }
                        Input {
                            id: "login-username",
                            name: "username",
                            autocomplete: "username webauthn",
                            required: true,
                            value: username(),
                            oninput: move |event: FormEvent| username.set(event.value()),
                        }
                        Label { html_for: "login-password", "Password" }
                        Input {
                            id: "login-password",
                            name: "password",
                            r#type: "password",
                            autocomplete: if registering() { "new-password" } else { "current-password" },
                            required: true,
                            value: password(),
                            oninput: move |event: FormEvent| password.set(event.value()),
                        }
                        Button {
                            r#type: "submit",
                            disabled: pending(),
                            if registering() { "Create account" } else { "Sign in" }
                        }
                    }
                    if !registering() {
                        Separator { style: "margin: 16px 0;" }
                        Button {
                            r#type: "button",
                            variant: ButtonVariant::Outline,
                            style: "width: 100%;",
                            disabled: pending(),
                            onclick: move |_| {
                                let name = username();
                                run(Box::pin(async move {
                                    if name.trim().is_empty() {
                                        return Err("Enter your username, then choose \u{201c}Sign in with a passkey\u{201d}.".to_owned());
                                    }
                                    let signed_in = passkey_sign_in(name).await?;
                                    account.set(Some(signed_in));
                                    Ok(None)
                                }));
                            },
                            "Sign in with a passkey"
                        }
                    }
                }
                CardFooter {
                    Button {
                        r#type: "button",
                        variant: ButtonVariant::Link,
                        onclick: move |_| {
                            registering.toggle();
                            notice.set(None);
                        },
                        if registering() { "Have an account? Sign in" } else { "New here? Create an account" }
                    }
                }
            }
            if let Some(current) = notice() {
                p {
                    role: if current.error { "alert" } else { "status" },
                    style: if current.error { "color: #f87171; margin: 0 24px 24px;" } else { "color: #4ade80; margin: 0 24px 24px;" },
                    "{current.text}"
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_data_is_returned_on_success() {
        let body = r#"{"data":{"id":"1","username":"alice","passkeys":2},"errors":[]}"#;
        let account: Account = parse_envelope(200, body).expect("parses");
        assert_eq!(account.username, "alice");
        assert_eq!(account.passkeys, 2);
    }

    #[test]
    fn error_messages_come_from_envelopes_and_problems() {
        let envelope =
            r#"{"errors":[{"message":"Incorrect username or password."}],"title":"Unauthorized"}"#;
        assert_eq!(
            parse_envelope::<Account>(401, envelope).unwrap_err(),
            "Incorrect username or password."
        );
        // Request-validation problem+json: same `errors[].message` shape.
        let problem = r#"{"type":"x","title":"Unprocessable Entity","status":422,"code":"validation","errors":[{"code":"pattern","location":"/body/username","message":"does not match the required format"}]}"#;
        assert_eq!(
            parse_envelope::<Account>(422, problem).unwrap_err(),
            "does not match the required format"
        );
        assert_eq!(
            parse_envelope::<Account>(502, "<html>").unwrap_err(),
            "Request failed (502)."
        );
        // A 2xx without `data` is still an error, not a panic.
        assert_eq!(
            parse_envelope::<Account>(200, r#"{"errors":[],"title":"Ok"}"#).unwrap_err(),
            "Ok"
        );
    }
}
