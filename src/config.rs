use std::{env, fmt, time::Duration};

use anyhow::{Context, Result, bail};
use url::Url;

const DEFAULT_BASE_URL: &str = "https://www.sportybet.com/api/ng/";
const DEFAULT_TIMEOUT_MS: u64 = 10_000;

#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    fn new(value: String) -> Result<Self> {
        if value.contains(['\r', '\n']) {
            bail!("session cookie contains a newline");
        }
        Ok(Self(value))
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted>")
    }
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub base_url: Url,
    pub cookie: Option<Secret>,
    pub device_id: Option<String>,
    pub fingerprint: Option<String>,
    pub locale: String,
    pub oper_id: String,
    pub currency: String,
    pub timeout: Duration,
}

impl Settings {
    /// Loads runtime settings from the process environment.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid base URL, unsafe cookie value, or invalid timeout.
    pub fn from_env() -> Result<Self> {
        let base_url =
            env::var("SPORTYBET_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_owned());
        let base_url = normalize_base_url(&base_url)?;

        let cookie = optional_env("SPORTYBET_COOKIE")
            .map(Secret::new)
            .transpose()?;
        let timeout_ms = env::var("HECTOR_TIMEOUT_MS")
            .ok()
            .map(|value| value.parse::<u64>())
            .transpose()
            .context("HECTOR_TIMEOUT_MS must be an unsigned integer")?
            .unwrap_or(DEFAULT_TIMEOUT_MS);

        if timeout_ms == 0 {
            bail!("HECTOR_TIMEOUT_MS must be greater than zero");
        }

        Ok(Self {
            base_url,
            cookie,
            device_id: optional_env("SPORTYBET_DEVICE_ID"),
            fingerprint: optional_env("SPORTYBET_FINGERPRINT"),
            locale: env::var("SPORTYBET_LOCALE").unwrap_or_else(|_| "en".to_owned()),
            oper_id: env::var("SPORTYBET_OPER_ID").unwrap_or_else(|_| "2".to_owned()),
            currency: env::var("SPORTYBET_CURRENCY").unwrap_or_else(|_| "NGN".to_owned()),
            timeout: Duration::from_millis(timeout_ms),
        })
    }

    /// Returns the imported browser cookie header.
    ///
    /// # Errors
    ///
    /// Returns an error when `SPORTYBET_COOKIE` was not configured.
    pub fn require_cookie(&self) -> Result<&str> {
        self.cookie
            .as_ref()
            .map(Secret::expose)
            .context("SPORTYBET_COOKIE is required; import the full browser Cookie request header")
    }
}

fn optional_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn normalize_base_url(value: &str) -> Result<Url> {
    let normalized = if value.ends_with('/') {
        value.to_owned()
    } else {
        format!("{value}/")
    };
    Url::parse(&normalized).with_context(|| format!("invalid SPORTYBET_BASE_URL: {value}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_debug_is_redacted() {
        let secret = Secret::new("accessToken=super-secret".to_owned()).unwrap();
        assert_eq!(format!("{secret:?}"), "<redacted>");
    }

    #[test]
    fn secret_rejects_header_injection() {
        assert!(Secret::new("ok=true\r\nInjected: yes".to_owned()).is_err());
    }

    #[test]
    fn base_url_gets_a_trailing_slash() {
        let url = normalize_base_url("https://example.com/api/ng").unwrap();
        assert_eq!(url.as_str(), "https://example.com/api/ng/");
    }
}
