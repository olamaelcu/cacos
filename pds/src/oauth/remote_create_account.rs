//! `RemoteCreateAccount` seam: the headless-consent `create-account`
//! endpoint delegates account creation here.
//!
//! [`ActorStoreRemoteCreateAccount`] is the production impl; it is wired
//! up in [`crate::oauth::bootstrap_oauth_app`]. The mock variant remains
//! available for unit tests behind `#[cfg(any(test, feature = "test-utils"))]`.

use async_trait::async_trait;
use rsky_oauth::OAuthError;
use std::sync::Arc;

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
    async fn create_account(&self, input: CreateAccountInput)
    -> Result<String, CreateAccountError>;
}

/// Production impl. The current stub returns `Internal("wiring pending
/// phase-e")`; the full integration (account creation + PLC op + per-DID
/// rotation key) is the next wave (Phase E).
pub struct ActorStoreRemoteCreateAccount {
    #[allow(dead_code)]
    pub account_manager: Arc<cacos_pds_account::account::AccountManager>,
    #[allow(dead_code)]
    pub actor_store: Arc<cacos_pds_actor_store::ActorStore>,
    #[allow(dead_code)]
    pub plc_client: Arc<dyn cacos_pds_plc::PlcClient>,
}

impl ActorStoreRemoteCreateAccount {
    pub fn new(
        account_manager: cacos_pds_account::account::AccountManager,
        actor_store: Arc<cacos_pds_actor_store::ActorStore>,
        plc_client: Arc<dyn cacos_pds_plc::PlcClient>,
    ) -> Self {
        Self {
            account_manager: Arc::new(account_manager),
            actor_store,
            plc_client,
        }
    }
}

#[async_trait]
impl RemoteCreateAccount for ActorStoreRemoteCreateAccount {
    async fn create_account(
        &self,
        _input: CreateAccountInput,
    ) -> Result<String, CreateAccountError> {
        // TODO(phase-e): replace with full ActorStoreRemoteCreateAccount impl.
        Err(CreateAccountError::Internal(
            "actor store remote create account not yet wired (phase-e)".into(),
        ))
    }
}

/// Test double: returns a canned result from its `next` slot.
#[cfg(any(test, feature = "test-utils"))]
pub struct MockRemoteCreateAccount {
    pub next: tokio::sync::Mutex<Option<Result<String, CreateAccountError>>>,
}

#[cfg(any(test, feature = "test-utils"))]
impl Default for MockRemoteCreateAccount {
    fn default() -> Self {
        Self {
            next: tokio::sync::Mutex::new(Some(Ok("did:plc:mock".to_string()))),
        }
    }
}

#[cfg(any(test, feature = "test-utils"))]
#[async_trait]
impl RemoteCreateAccount for MockRemoteCreateAccount {
    async fn create_account(
        &self,
        _input: CreateAccountInput,
    ) -> Result<String, CreateAccountError> {
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
            password: "password123".to_owned(),
            invite_code: None,
        };
        let did = mock.create_account(input).await.unwrap();
        assert_eq!(did, "did:plc:mock");
    }
}
