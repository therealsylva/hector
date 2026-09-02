use anyhow::{Context, Result, bail};
use reqwest::{Method, RequestBuilder, StatusCode, header};
use serde::{Serialize, de::DeserializeOwned};
use url::Url;

use crate::config::Settings;

const MAX_ERROR_BODY: usize = 512;

#[derive(Clone, Debug)]
pub struct SportyClient {
    http: reqwest::Client,
    settings: Settings,
}

impl SportyClient {
    /// Builds a client with the configured timeout and TLS backend.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying HTTP client cannot be initialized.
    pub fn new(settings: Settings) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(settings.timeout)
            .user_agent(concat!("hector/", env!("CARGO_PKG_VERSION")))
            .build()
            .context("failed to build HTTP client")?;
        Ok(Self { http, settings })
    }

    #[must_use]
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Sends a GET request and decodes its JSON response.
    ///
    /// # Errors
    ///
    /// Returns an error for transport, HTTP, WAF, or JSON decoding failures.
    pub async fn get<T>(&self, path: &str) -> Result<T>
    where
        T: DeserializeOwned,
    {
        self.send(self.request(Method::GET, path)?).await
    }

    /// Sends a GET request with a serialized query and decodes its JSON response.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid paths, transport failures, non-success responses, or invalid JSON.
    pub async fn get_with_query<T, Q>(&self, path: &str, query: &Q) -> Result<T>
    where
        T: DeserializeOwned,
        Q: Serialize + ?Sized,
    {
        self.send(self.request(Method::GET, path)?.query(query))
            .await
    }

    /// Sends a JSON POST request and decodes its JSON response.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid paths, transport failures, non-success responses, or invalid JSON.
    pub async fn post_json<T, B>(&self, path: &str, body: &B) -> Result<T>
    where
        T: DeserializeOwned,
        B: Serialize + ?Sized,
    {
        self.send(self.request(Method::POST, path)?.json(body))
            .await
    }

    /// Sends a plain-text POST request and decodes its JSON response.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid paths, transport failures, non-success responses, or invalid JSON.
    pub async fn post_text<T>(&self, path: &str, body: String) -> Result<T>
    where
        T: DeserializeOwned,
    {
        self.send(
            self.request(Method::POST, path)?
                .header(header::CONTENT_TYPE, "text/plain")
                .body(body),
        )
        .await
    }

    /// Sends an encrypted protected-endpoint payload and returns its raw response.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid paths, transport failures, non-success responses,
    /// or a response body that is not UTF-8.
    pub async fn post_ciphertext(
        &self,
        path: &str,
        trans_id: &str,
        body: String,
    ) -> Result<String> {
        let bytes = self
            .send_bytes(
                self.request(Method::POST, path)?
                    .header(header::CONTENT_TYPE, "application/json;charset=UTF-8")
                    .header("transId", trans_id)
                    .body(body),
            )
            .await?;
        String::from_utf8(bytes).context("encrypted response was not UTF-8")
    }

    /// Resolves an API-relative path against the configured base URL.
    ///
    /// # Errors
    ///
    /// Returns an error when `path` cannot be resolved as a URL.
    pub fn url(&self, path: &str) -> Result<Url> {
        self.settings
            .base_url
            .join(path.trim_start_matches('/'))
            .with_context(|| format!("invalid API path: {path}"))
    }

    fn request(&self, method: Method, path: &str) -> Result<RequestBuilder> {
        let url = self.url(path)?;
        let mut request = self
            .http
            .request(method, url)
            .header(header::ACCEPT, "application/json")
            .header(header::ACCEPT_LANGUAGE, &self.settings.locale)
            .header("OperId", &self.settings.oper_id)
            .header("clientid", &self.settings.client_id)
            .header("platform", &self.settings.platform);

        if let Some(cookie) = &self.settings.cookie {
            request = request.header(header::COOKIE, cookie.expose());
        }
        if let Some(device_id) = &self.settings.device_id {
            request = request.header("DeviceId", device_id);
        }
        if let Some(fingerprint) = &self.settings.fingerprint {
            request = request.header("Fingerprint", fingerprint);
        }

        Ok(request)
    }

    async fn send<T>(&self, request: RequestBuilder) -> Result<T>
    where
        T: DeserializeOwned,
    {
        let body = self.send_bytes(request).await?;
        serde_json::from_slice(&body).context("SportyBet returned invalid JSON")
    }

    async fn send_bytes(&self, request: RequestBuilder) -> Result<Vec<u8>> {
        let response = request.send().await.context("SportyBet request failed")?;
        let status = response.status();
        let server = response
            .headers()
            .get(header::SERVER)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let body = response.bytes().await.context("failed to read response")?;

        if !status.is_success() {
            let preview = String::from_utf8_lossy(&body[..body.len().min(MAX_ERROR_BODY)]);
            if status == StatusCode::FORBIDDEN && server.eq_ignore_ascii_case("CloudFront") {
                bail!(
                    "SportyBet/CloudFront rejected the request (403); refresh the browser session and retry"
                )
            }
            bail!("SportyBet returned HTTP {status}: {preview}");
        }

        Ok(body.to_vec())
    }
}
