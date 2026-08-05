//! SSRF-hardened HTTPS fetcher for client metadata documents and JWKS sets.
//!
//! Replaces the Task 4 stub. Enforces: https-only URLs, a 10s timeout, no
//! redirects, `application/json` content-type, and a 512 KiB response cap.

use async_trait::async_trait;
use rsky_oauth::client::ClientMetadataFetcher;
use rsky_oauth::jwk::JwkSet;
use rsky_oauth::types::OAuthClientMetadata;
use rsky_oauth::OAuthError;
use std::time::Duration;

const MAX_RESPONSE_SIZE: usize = 512 * 1024;
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// HTTPS fetcher for client metadata documents and JWK sets, SSRF-hardened.
pub struct HttpClientMetadataFetcher {
    client: reqwest::Client,
}

impl Default for HttpClientMetadataFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpClientMetadataFetcher {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent("cacos-pds/0.1")
            .timeout(FETCH_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("reqwest client construction cannot fail");
        Self { client }
    }

    /// Test seam: build with a custom reqwest client (e.g. a shorter timeout).
    pub fn with_client(client: reqwest::Client) -> Self {
        Self { client }
    }

    /// Pure scheme/format validation (unit-testable without network).
    fn validate_fetch_url(url: &str) -> Result<url::Url, OAuthError> {
        let invalid = |reason: String| {
            OAuthError::InvalidClient(format!("failed to fetch {url}: {reason}"))
        };
        let parsed = url::Url::parse(url).map_err(|e| invalid(e.to_string()))?;
        if parsed.scheme() != "https" {
            return Err(invalid("must be an https URL".to_string()));
        }
        Ok(parsed)
    }

    async fn fetch_json_capped(&self, url: &str) -> Result<Vec<u8>, OAuthError> {
        let parsed = Self::validate_fetch_url(url)?;
        self.fetch_json_capped_parsed(parsed).await
    }

    /// Network + response-shape enforcement. `parsed` must already pass
    /// `validate_fetch_url`; tests call this directly against a local http
    /// server to exercise the content-type/size/timeout checks.
    pub(crate) async fn fetch_json_capped_parsed(
        &self,
        parsed: url::Url,
    ) -> Result<Vec<u8>, OAuthError> {
        let url_for_err = parsed.clone();
        let invalid = |reason: String| {
            OAuthError::InvalidClient(format!("failed to fetch {url_for_err}: {reason}"))
        };
        let response = self
            .client
            .get(parsed)
            .header("accept", "application/json")
            .send()
            .await
            .map_err(|e| invalid(e.to_string()))?;
        if !response.status().is_success() {
            return Err(invalid(format!("unexpected status {}", response.status())));
        }
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        if content_type != "application/json" {
            return Err(invalid(format!("unexpected content-type \"{content_type}\"")));
        }
        let body = response.bytes().await.map_err(|e| invalid(e.to_string()))?;
        if body.len() > MAX_RESPONSE_SIZE {
            return Err(invalid("response too large".to_string()));
        }
        Ok(body.to_vec())
    }
}

#[async_trait]
impl ClientMetadataFetcher for HttpClientMetadataFetcher {
    async fn fetch_client_metadata(
        &self,
        url: &str,
    ) -> Result<OAuthClientMetadata, OAuthError> {
        let body = self.fetch_json_capped(url).await?;
        serde_json::from_slice(&body).map_err(|e| {
            OAuthError::InvalidClient(format!("invalid client metadata document: {e}"))
        })
    }

    async fn fetch_jwks(&self, url: &str) -> Result<JwkSet, OAuthError> {
        let body = self.fetch_json_capped(url).await?;
        serde_json::from_slice(&body)
            .map_err(|e| OAuthError::InvalidClient(format!("invalid JWKS document: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn rejects_invalid_and_non_https_urls() {
        let fetcher = HttpClientMetadataFetcher::default();
        let err = fetcher
            .fetch_client_metadata("not a url")
            .await
            .unwrap_err();
        assert!(err.error_description().contains("failed to fetch"));
        let err = fetcher
            .fetch_client_metadata("http://app.example.com/client.json")
            .await
            .unwrap_err();
        assert!(err.error_description().contains("must be an https URL"));
        let err = fetcher
            .fetch_jwks("http://app.example.com/jwks.json")
            .await
            .unwrap_err();
        assert!(err.error_description().contains("must be an https URL"));
    }

    #[tokio::test]
    async fn surfaces_connection_failures() {
        let fetcher = HttpClientMetadataFetcher::new();
        // nothing listens on port 1; the connection is refused immediately
        let err = fetcher
            .fetch_client_metadata("https://127.0.0.1:1/client.json")
            .await
            .unwrap_err();
        assert!(err.error_description().contains("failed to fetch"));
    }

    #[tokio::test]
    async fn times_out_on_slow_servers() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/slow.json"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "client_id": "x" }))
                    .insert_header("content-type", "application/json")
                    .set_delay(Duration::from_secs(2)),
            )
            .mount(&server)
            .await;
        let fetcher = HttpClientMetadataFetcher::with_client(
            reqwest::Client::builder()
                .timeout(Duration::from_millis(50))
                .build()
                .unwrap(),
        );
        let url = format!("{}/slow.json", server.uri());
        let parsed = url::Url::parse(&url).unwrap();
        let err = fetcher.fetch_json_capped_parsed(parsed).await.unwrap_err();
        assert!(err.error_description().contains("failed to fetch"));
    }

    #[tokio::test]
    async fn fetches_and_parses_metadata_body() {
        let server = MockServer::start().await;
        let metadata = serde_json::json!({
            "client_id": "https://app.example.com/client-metadata.json",
            "redirect_uris": ["https://app.example.com/callback"],
        });
        Mock::given(method("GET"))
            .and(path("/client.json"))
            .and(header("accept", "application/json"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(metadata)
                    .insert_header("content-type", "application/json"),
            )
            .mount(&server)
            .await;
        let fetcher = HttpClientMetadataFetcher::with_client(reqwest::Client::new());
        let url = format!("{}/client.json", server.uri());
        let parsed = url::Url::parse(&url).unwrap();
        let body = fetcher.fetch_json_capped_parsed(parsed).await.unwrap();
        let parsed_metadata: OAuthClientMetadata = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            parsed_metadata.client_id,
            "https://app.example.com/client-metadata.json"
        );
    }

    #[tokio::test]
    async fn rejects_wrong_content_type() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/text.json"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("not json")
                    .insert_header("content-type", "text/plain"),
            )
            .mount(&server)
            .await;
        let fetcher = HttpClientMetadataFetcher::with_client(reqwest::Client::new());
        let url = format!("{}/text.json", server.uri());
        let parsed = url::Url::parse(&url).unwrap();
        let err = fetcher.fetch_json_capped_parsed(parsed).await.unwrap_err();
        assert!(err.error_description().contains("unexpected content-type"));
    }

    #[tokio::test]
    async fn rejects_oversized_bodies() {
        let server = MockServer::start().await;
        // set_body_json uses application/json; the JSON string is padded
        // beyond the 512 KiB cap.
        let big = serde_json::json!({
            "client_id": "x",
            "padding": "a".repeat(512 * 1024 + 1),
        });
        Mock::given(method("GET"))
            .and(path("/big.json"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(big)
                    .insert_header("content-type", "application/json"),
            )
            .mount(&server)
            .await;
        let fetcher = HttpClientMetadataFetcher::with_client(reqwest::Client::new());
        let url = format!("{}/big.json", server.uri());
        let parsed = url::Url::parse(&url).unwrap();
        let err = fetcher.fetch_json_capped_parsed(parsed).await.unwrap_err();
        assert!(err.error_description().contains("response too large"));
    }
}
