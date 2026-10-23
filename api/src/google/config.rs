//! Google OAuth settings: the OAuth client, the token-sealing key and Google's endpoints.
//! All three env settings are optional; without every one of them Google is "not
//! configured" and the rest of the API still runs.

use std::fmt;

use crate::config::DEFAULT_PORT;
use crate::crypto::TokenKey;

pub const CLIENT_ID_VAR: &str = "WAYPOINT_GOOGLE_CLIENT_ID";
pub const CLIENT_SECRET_VAR: &str = "WAYPOINT_GOOGLE_CLIENT_SECRET";
pub const TOKEN_KEY_VAR: &str = "WAYPOINT_TOKEN_KEY";

pub const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
pub const REVOKE_URL: &str = "https://oauth2.googleapis.com/revoke";

/// Google OAuth settings. Its `Debug` shows neither the client secret nor the key.
#[derive(Clone)]
pub struct Config {
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub token_key: Option<TokenKey>,
    pub auth_url: String,
    pub token_url: String,
    pub revoke_url: String,
    /// `http://127.0.0.1:<port>/auth/google/callback`.
    pub redirect_uri: String,
    /// Gmail sync settings (code defaults; tests override them).
    pub gmail: crate::gmail::Config,
}

/// The settings a configured flow needs, borrowed from a [`Config`].
pub struct Credentials<'a> {
    pub client_id: &'a str,
    pub client_secret: &'a str,
    pub token_key: &'a TokenKey,
}

impl fmt::Debug for Credentials<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("client_id", &self.client_id)
            .field("client_secret", &"<redacted>")
            .field("token_key", self.token_key)
            .finish()
    }
}

impl Default for Config {
    /// Not configured, with Google's endpoints and the default port's redirect URI.
    fn default() -> Self {
        Self {
            client_id: None,
            client_secret: None,
            token_key: None,
            auth_url: AUTH_URL.into(),
            token_url: TOKEN_URL.into(),
            revoke_url: REVOKE_URL.into(),
            redirect_uri: redirect_uri(DEFAULT_PORT),
            gmail: crate::gmail::Config::default(),
        }
    }
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let secret = self.client_secret.as_ref().map(|_| "<redacted>");
        f.debug_struct("Config")
            .field("client_id", &self.client_id)
            .field("client_secret", &secret)
            .field("token_key", &self.token_key)
            .field("auth_url", &self.auth_url)
            .field("token_url", &self.token_url)
            .field("revoke_url", &self.revoke_url)
            .field("redirect_uri", &self.redirect_uri)
            .field("gmail", &self.gmail)
            .finish()
    }
}

/// The loopback redirect URI for an API on `port`.
pub fn redirect_uri(port: u16) -> String {
    format!("http://127.0.0.1:{port}/auth/google/callback")
}

impl Config {
    /// Reads `WAYPOINT_GOOGLE_CLIENT_ID`, `WAYPOINT_GOOGLE_CLIENT_SECRET` and
    /// `WAYPOINT_TOKEN_KEY`. Fails only on a `WAYPOINT_TOKEN_KEY` that isn't 32 bytes of
    /// base64.
    pub fn from_env(port: u16) -> Result<Self, String> {
        let env = |key: &str| std::env::var(key).ok();
        Self::from_values(
            env(CLIENT_ID_VAR).as_deref(),
            env(CLIENT_SECRET_VAR).as_deref(),
            env(TOKEN_KEY_VAR).as_deref(),
            port,
        )
    }

    /// [`Config::from_env`] over explicit values; blank means unset.
    pub fn from_values(
        client_id: Option<&str>,
        client_secret: Option<&str>,
        token_key: Option<&str>,
        port: u16,
    ) -> Result<Self, String> {
        let set = |value: Option<&str>| {
            value
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_string)
        };
        let token_key = match set(token_key) {
            None => None,
            Some(raw) => Some(TokenKey::from_base64(&raw).map_err(|why| {
                format!("{TOKEN_KEY_VAR} {why} (make one with `openssl rand -base64 32`)")
            })?),
        };
        Ok(Self {
            client_id: set(client_id),
            client_secret: set(client_secret),
            token_key,
            redirect_uri: redirect_uri(port),
            ..Self::default()
        })
    }

    /// The client and key when all three are set, or the names of the missing settings.
    pub fn credentials(&self) -> Result<Credentials<'_>, Vec<&'static str>> {
        match (&self.client_id, &self.client_secret, &self.token_key) {
            (Some(client_id), Some(client_secret), Some(token_key)) => Ok(Credentials {
                client_id,
                client_secret,
                token_key,
            }),
            _ => {
                let mut missing = Vec::new();
                if self.client_id.is_none() {
                    missing.push(CLIENT_ID_VAR);
                }
                if self.client_secret.is_none() {
                    missing.push(CLIENT_SECRET_VAR);
                }
                if self.token_key.is_none() {
                    missing.push(TOKEN_KEY_VAR);
                }
                Err(missing)
            }
        }
    }

    pub fn is_configured(&self) -> bool {
        self.credentials().is_ok()
    }
}

#[cfg(test)]
mod tests {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;

    use super::*;

    const SECRET: &str = "GOCSPX-test-secret";

    fn key_b64() -> String {
        STANDARD.encode([42u8; 32])
    }

    #[test]
    fn from_values_unset_or_blank_is_not_configured() {
        for blank in [None, Some(""), Some("  ")] {
            let config = Config::from_values(blank, blank, blank, 8000).unwrap();
            assert!(!config.is_configured());
            assert_eq!(
                config.credentials().unwrap_err(),
                [CLIENT_ID_VAR, CLIENT_SECRET_VAR, TOKEN_KEY_VAR]
            );
        }
        assert!(!Config::default().is_configured());
    }

    #[test]
    fn from_values_partial_names_missing() {
        let key = key_b64();
        let config = Config::from_values(Some("id"), None, Some(&key), 8000).unwrap();
        assert!(!config.is_configured());
        assert_eq!(config.credentials().unwrap_err(), [CLIENT_SECRET_VAR]);

        let config = Config::from_values(Some("id"), Some(SECRET), Some(" "), 8000).unwrap();
        assert_eq!(config.credentials().unwrap_err(), [TOKEN_KEY_VAR]);
    }

    #[test]
    fn from_values_rejects_bad_token_key() {
        let short = STANDARD.encode([1u8; 31]);
        for bad in ["not base64!", short.as_str()] {
            let err = Config::from_values(None, None, Some(bad), 8000).unwrap_err();
            assert!(err.contains(TOKEN_KEY_VAR), "{err}");
            assert!(!err.contains(bad), "the error echoes the value: {err}");
        }
    }

    #[test]
    fn from_values_configured_uses_port() {
        let key = key_b64();
        let config = Config::from_values(Some(" id "), Some(SECRET), Some(&key), 9123).unwrap();
        let creds = config.credentials().unwrap();
        assert_eq!(creds.client_id, "id");
        assert_eq!(creds.client_secret, SECRET);
        assert_eq!(*creds.token_key, TokenKey::new([42u8; 32]));
        assert_eq!(
            config.redirect_uri,
            "http://127.0.0.1:9123/auth/google/callback"
        );
        assert_eq!(config.token_url, TOKEN_URL);
    }

    #[test]
    fn debug_redacts_secrets() {
        let key = key_b64();
        let config = Config::from_values(Some("id"), Some(SECRET), Some(&key), 8000).unwrap();
        let creds = config.credentials().unwrap();
        for debug in [
            format!("{config:?}"),
            format!("{creds:?}"),
            format!("{:?}", config.token_key),
        ] {
            assert!(!debug.contains(SECRET), "{debug}");
            assert!(!debug.contains(&key), "{debug}");
            assert!(!debug.contains("42"), "{debug}");
        }
    }
}
