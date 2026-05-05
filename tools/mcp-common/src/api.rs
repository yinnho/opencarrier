//! Generic HTTP API client for MCP servers.
//!
//! Wraps `reqwest` with sensible defaults: timeout, default headers,
//! automatic JSON parsing, and error formatting.

use reqwest::{header::HeaderMap, Client, Method};
use serde_json::Value;
use std::time::Duration;

/// A pre-configured HTTP client for a single API base URL.
pub struct ApiClient {
    base_url: String,
    client: Client,
    headers: HeaderMap,
    timeout: Duration,
}

impl ApiClient {
    /// Create a new client for the given base URL (e.g. `https://api.example.com`).
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            client: Client::new(),
            headers: HeaderMap::new(),
            timeout: Duration::from_secs(30),
        }
    }

    /// Add a default header sent with every request.
    pub fn with_header(mut self, key: &str, value: &str) -> Self {
        if let (Ok(h), Ok(v)) = (
            key.parse::<reqwest::header::HeaderName>(),
            value.parse::<reqwest::header::HeaderValue>(),
        ) {
            self.headers.insert(h, v);
        }
        self
    }

    /// Set the request timeout (default: 30 s).
    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.timeout = Duration::from_secs(secs);
        self
    }

    /// Send a request and parse the response as JSON.
    ///
    /// * `path` – API path relative to `base_url` (e.g. `/v1/users`).
    /// * `query` – Optional query string **without leading `?`**.
    /// * `body` – Optional raw body (for POST/PUT). When `Some`, `Content-Type`
    ///   is NOT set automatically; callers should add it via `with_header` if needed.
    pub async fn request(
        &self,
        method: Method,
        path: &str,
        query: Option<&str>,
        body: Option<&str>,
    ) -> Result<Value, String> {
        let url = if let Some(q) = query {
            format!("{}{path}?{q}", self.base_url)
        } else {
            format!("{}{path}", self.base_url)
        };

        let mut req = self
            .client
            .request(method, &url)
            .headers(self.headers.clone())
            .timeout(self.timeout);

        if let Some(b) = body {
            req = req.body(b.to_string());
        }

        let resp = req
            .send()
            .await
            .map_err(|e| format!("API request failed: {e}"))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| format!("API read body failed: {e}"))?;

        if !status.is_success() {
            return Err(format!(
                "API HTTP {status}: {}",
                &text[..text.len().min(500)]
            ));
        }

        let json: Value = serde_json::from_str(&text)
            .map_err(|e| format!("API JSON parse error: {e}"))?;

        Ok(json)
    }

    /// Send a request and return the raw response body as a `String`.
    ///
    /// Useful when the response needs pre-processing (e.g. regex fixes)
    /// before JSON parsing.
    pub async fn request_text(
        &self,
        method: Method,
        path: &str,
        query: Option<&str>,
        body: Option<&str>,
    ) -> Result<String, String> {
        let url = if let Some(q) = query {
            format!("{}{path}?{q}", self.base_url)
        } else {
            format!("{}{path}", self.base_url)
        };

        let mut req = self
            .client
            .request(method, &url)
            .headers(self.headers.clone())
            .timeout(self.timeout);

        if let Some(b) = body {
            req = req.body(b.to_string());
        }

        let resp = req
            .send()
            .await
            .map_err(|e| format!("API request failed: {e}"))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| format!("API read body failed: {e}"))?;

        if !status.is_success() {
            return Err(format!(
                "API HTTP {status}: {}",
                &text[..text.len().min(500)]
            ));
        }

        Ok(text)
    }
}
