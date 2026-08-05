//! `RemoteCreateAccount` seam: the headless-consent `create-account`
//! endpoint delegates account creation here.
//!
//! Plan 08 wires the real implementation (ActorStore::create + repo +
//! PLC + sequencing). Until then, tests use [`MockRemoteCreateAccount`].

use async_trait::async_trait;
use rsky_oauth::OAuthError;

#[derive(Debug, Clone)]
pub struct CreateAccountInput {
    pub rqid: String,
    pub request_uri: String,
    pub client_id: String,
    pub device_id: String,
    pub handle: String,
    pub email: String,
    pub password: String,
    pub invite_code: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum CreateAccountError {
    #[error("oauth: {0}")]
    OAuth(#[source] OAuthError),
    #[error("internal: {0}")]
    Internal(String),
}

#[async_trait]
pub trait RemoteCreateAccount: Send + Sync + 'static {
    async fn create_account(&self, input: CreateAccountInput) -> Result<String, CreateAccountError>;
}

/// Test double: returns a canned result from its `next` slot.
pub struct MockRemoteCreateAccount {
    pub next: tokio::sync::Mutex<Option<Result<String, CreateAccountError>>>,
}

impl Default for MockRemoteCreateAccount {
    fn default() -> Self {
        Self {
            next: tokio::sync::Mutex::new(Some(Ok("did:plc:mock".to_string()))),
        }
    }
}

#[async_trait]
impl RemoteCreateAccount for MockRemoteCreateAccount {
    async fn create_account(&self, _input: CreateAccountInput) -> Result<String, CreateAccountError> {
        self.next
            .lock()
            .await
            .take()
            .unwrap_or_else(|| Err(CreateAccountError::Internal("no mock response".into())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_returns_canned_result() {
        let mock = MockRemoteCreateAccount::default();
        let input = CreateAccountInput {
            rqid: "rq-1".into(),
            request_uri: "urn:ietf:params:oauth:request_uri:rq-1".into(),
            client_id: "https://app.example.com/client".into(),
            device_id: "dev-1".into(),
            handle: "alice.test".into(),
            email: "alice@example.com".into(),
            password: "password123".into(),
            invite_code: None,
        };
        let did = mock.create_account(input).await.unwrap();
        assert_eq!(did, "did:plc:mock");
    }
}
