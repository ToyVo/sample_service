//! WebAuthn relying-party configuration.
//!
//! A passkey ceremony is two round trips: `options` returns a challenge for
//! the browser's authenticator, `finish` verifies the signed response. The
//! server-side half of that state travels in a signed ceremony token (see
//! [`super::token`]), so any server instance can finish any ceremony.

use webauthn_rs::prelude::Url;
use webauthn_rs::{Webauthn, WebauthnBuilder};

/// Relying-party identity, from `WEBAUTHN_RP_ID` / `WEBAUTHN_ORIGIN`.
///
/// `rp_id` is the registrable domain passkeys are bound to (`localhost` in
/// development); `origin` is the exact `scheme://host[:port]` the browser
/// reports. On `localhost` any port is accepted so `dx serve` port changes
/// don't invalidate registered passkeys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasskeyConfig {
    pub rp_id: String,
    pub origin: String,
}

impl Default for PasskeyConfig {
    /// Matches `dx serve` with `[web.https]` enabled (see `Dioxus.toml`).
    fn default() -> Self {
        Self {
            rp_id: "localhost".to_owned(),
            origin: "https://localhost:8080".to_owned(),
        }
    }
}

impl PasskeyConfig {
    /// Read `WEBAUTHN_RP_ID` / `WEBAUTHN_ORIGIN`, falling back to
    /// [`PasskeyConfig::default`], and validate that they agree.
    ///
    /// # Errors
    ///
    /// Returns a description when the origin is not a URL or its host is
    /// not `rp_id` (or a subdomain of it).
    pub fn from_env() -> Result<Self, String> {
        let defaults = Self::default();
        let config = Self {
            rp_id: std::env::var("WEBAUTHN_RP_ID").unwrap_or(defaults.rp_id),
            origin: std::env::var("WEBAUTHN_ORIGIN").unwrap_or(defaults.origin),
        };
        config.webauthn()?;
        Ok(config)
    }

    /// Build the `webauthn-rs` relying party.
    ///
    /// # Errors
    ///
    /// Returns a description when the configuration is invalid.
    pub fn webauthn(&self) -> Result<Webauthn, String> {
        let origin = Url::parse(&self.origin)
            .map_err(|err| format!("WEBAUTHN_ORIGIN {:?} is not a URL: {err}", self.origin))?;
        WebauthnBuilder::new(&self.rp_id, &origin)
            .map(|builder| {
                builder
                    .rp_name("Sample")
                    .allow_any_port(self.rp_id == "localhost")
            })
            .and_then(WebauthnBuilder::build)
            .map_err(|err| {
                format!(
                    "WEBAUTHN_ORIGIN {:?} does not belong to WEBAUTHN_RP_ID {:?}: {err}",
                    self.origin, self.rp_id
                )
            })
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid() {
        assert!(PasskeyConfig::default().webauthn().is_ok());
    }

    #[test]
    fn origin_must_belong_to_rp_id() {
        let config = |origin: &str| PasskeyConfig {
            rp_id: "example.com".to_owned(),
            origin: origin.to_owned(),
        };
        assert!(config("https://example.com").webauthn().is_ok());
        assert!(config("https://app.example.com").webauthn().is_ok());
        assert!(config("https://evil.test").webauthn().is_err());
        assert!(config("https://notexample.com").webauthn().is_err());
        assert!(config("not a url").webauthn().is_err());
    }
}
