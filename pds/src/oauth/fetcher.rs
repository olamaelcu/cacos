//! Stub. Replaced by the SSRF-hardened reqwest-backed fetcher in Task 6.
//!
//! Currently implements [`rsky_oauth::client::ClientMetadataFetcher`] so
//! that [`SharedOAuthProvider::new`](crate::oauth::SharedOAuthProvider::new)
//! compiles. All calls return a `ServerError`; Task 6 supplies the real
//! reqwest-backed implementation.

use async_trait::async_trait;
use rsky_oauth::client::ClientMetadataFetcher;
use rsky_oauth::jwk::JwkSet as OAuthJwkSet;
use rsky_oauth::types::OAuthClientMetadata;
use rsky_oauth::OAuthError;

pub struct HttpClientMetadataFetcher;

impl HttpClientMetadataFetcher {
    pub fn new() -> Self {
        Self
    }
}

impl Default for HttpClientMetadataFetcher {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ClientMetadataFetcher for HttpClientMetadataFetcher {
    async fn fetch_client_metadata(
        &self,
        url: &str,
    ) -> Result<OAuthClientMetadata, OAuthError> {
        Err(OAuthError::ServerError(format!(
            "client-metadata fetch not implemented yet: {url}"
        )))
    }

    async fn fetch_jwks(&self, url: &str) -> Result<OAuthJwkSet, OAuthError> {
        Err(OAuthError::ServerError(format!(
            "jwks fetch not implemented yet: {url}"
        )))
    }
}
