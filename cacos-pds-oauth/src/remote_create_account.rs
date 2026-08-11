//! `RemoteCreateAccount` seam: the headless-consent `create-account`
//! endpoint delegates account creation here.
//!
//! [`ActorStoreRemoteCreateAccount`] is the production impl; it is wired
//! up in [`crate::bootstrap_oauth_app`]. The mock variant remains
//! available for unit tests behind `#[cfg(any(test, feature = "test-utils"))]`.

use async_trait::async_trait;
use cacos_pds_account::account::CreateAccountOpts;
use cacos_pds_account::account::helpers::account::AccountStatus;
use cacos_pds_account::auth::PDS_REPO_SIGNING_KEYPAIR;
use cacos_pds_blobstore::{BlobStore, BoxedBlobStream};
use cacos_pds_plc::operations::{CreateAtprotoOpInput, create_op};
use cacos_pds_plc::types::{OpOrTombstone, Operation};
use cacos_pds_sequencer::events::sync_evt_data_from_commit;
use rsky_crypto::utils::encode_did_key;
use rsky_oauth::OAuthError;
use email_address::EmailAddress;
use secp256k1::{Keypair, Secp256k1};
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
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("internal: {0}")]
    Internal(String),
}

#[async_trait]
pub trait RemoteCreateAccount: Send + Sync + 'static {
    async fn create_account(&self, input: CreateAccountInput)
    -> Result<String, CreateAccountError>;
}

/// Production impl. Mirrors the canonical
/// `com.atproto.server.createAccount` flow but on the smaller
/// `CreateAccountInput` the OAuth headless-consent route hands us: this
/// PDS always mints a fresh `did:plc` (no `did` / `plc_op` carry-over),
/// skips DID-doc resolution, and returns the new DID without an
/// access/refresh pair (the OAuth route re-fetches the page on its own).
pub struct ActorStoreRemoteCreateAccount {
    pub account_manager: Arc<cacos_pds_account::account::AccountManager>,
    pub actor_store: Arc<cacos_pds_actor_store::ActorStore>,
    pub plc_client: Arc<dyn cacos_pds_plc::PlcClient>,
    pub blobstore: Arc<dyn BlobStore<Stream = BoxedBlobStream>>,
    pub sequencer: cacos_pds_sequencer::shared_sequencer::SharedSequencer,
}

impl ActorStoreRemoteCreateAccount {
    pub fn new(
        account_manager: cacos_pds_account::account::AccountManager,
        actor_store: Arc<cacos_pds_actor_store::ActorStore>,
        plc_client: Arc<dyn cacos_pds_plc::PlcClient>,
        blobstore: Arc<dyn BlobStore<Stream = BoxedBlobStream>>,
        sequencer: cacos_pds_sequencer::shared_sequencer::SharedSequencer,
    ) -> Self {
        Self {
            account_manager: Arc::new(account_manager),
            actor_store,
            plc_client,
            blobstore,
            sequencer,
        }
    }
}

#[async_trait]
impl RemoteCreateAccount for ActorStoreRemoteCreateAccount {
    async fn create_account(
        &self,
        input: CreateAccountInput,
    ) -> Result<String, CreateAccountError> {
        if input.handle.trim().is_empty() {
            return Err(CreateAccountError::InvalidInput("handle must not be empty".into()));
        }
        if input.handle.contains('@') {
            return Err(CreateAccountError::InvalidInput("handle must not contain '@'".into()));
        }
        if input.email.trim().is_empty() || !EmailAddress::is_valid(&input.email) {
            return Err(CreateAccountError::InvalidInput("email is invalid".into()));
        }
        if input.password.is_empty() {
            return Err(CreateAccountError::InvalidInput("password must not be empty".into()));
        }

        let (did, plc_op, plc_rotation_key) = match build_did_and_plc_op(&input.handle).await {
            Ok(v) => v,
            Err(err) => {
                tracing::error!("Failed to build did:plc and PLC operation: {err}");
                return Err(CreateAccountError::Internal(err));
            }
        };

        if let Err(error) = self
            .actor_store
            .create(&did, &PDS_REPO_SIGNING_KEYPAIR)
            .await
        {
            tracing::error!("Failed to create actor store: {error:?}");
            return Err(CreateAccountError::Internal(format!(
                "failed to create actor store: {error:?}"
            )));
        }

        // Persist the per-DID PLC rotation key before the genesis operation
        // is submitted. The op below lists this key as the DID's only
        // rotation key, so losing it after submission would leave the DID
        // document permanently unmanageable — hence a hard failure rather
        // than a warning.
        if let Err(error) = self
            .actor_store
            .store_rotation_keypair(&did, &plc_rotation_key)
            .await
        {
            tracing::error!("Failed to persist per-DID PLC rotation key: {error:?}");
            self.actor_store
                .destroy(&did, self.blobstore.clone())
                .await
                .ok();
            return Err(CreateAccountError::Internal(format!(
                "failed to persist per-DID PLC rotation key: {error:?}"
            )));
        }

        let commit = {
            let actor_txn = match self
                .actor_store
                .transact(did.clone(), self.blobstore.clone())
                .await
            {
                Ok(actor_txn) => actor_txn,
                Err(error) => {
                    tracing::error!("Failed to open actor store: {error:?}");
                    self.actor_store
                        .destroy(&did, self.blobstore.clone())
                        .await
                        .ok();
                    return Err(CreateAccountError::Internal(format!(
                        "failed to open actor store: {error:?}"
                    )));
                }
            };
            match actor_txn.create_repo(Vec::new()).await {
                Ok(commit) => commit,
                Err(error) => {
                    tracing::error!("Failed to create repo: {error:?}");
                    self.actor_store
                        .destroy(&did, self.blobstore.clone())
                        .await
                        .ok();
                    return Err(CreateAccountError::Internal(format!(
                        "failed to create repo: {error:?}"
                    )));
                }
            }
        };

        if let Err(error) = self
            .plc_client
            .send_operation(&did, &OpOrTombstone::Operation(plc_op))
            .await
        {
            tracing::error!("Failed to create did:plc: {error}");
            self.actor_store
                .destroy(&did, self.blobstore.clone())
                .await
                .ok();
            return Err(CreateAccountError::Internal(format!(
                "failed to send PLC operation: {error}"
            )));
        }

        if let Err(error) = self
            .account_manager
            .create_account(CreateAccountOpts {
                did: did.clone(),
                handle: input.handle.clone(),
                email: Some(input.email.clone()),
                password: Some(input.password.clone()),
                repo_cid: commit.commit_data.cid,
                repo_rev: commit.commit_data.rev.clone(),
                invite_code: input.invite_code.clone(),
                deactivated: Some(false),
            })
            .await
        {
            tracing::error!("Error creating account: {error}");
            self.actor_store
                .destroy(&did, self.blobstore.clone())
                .await
                .ok();
            return Err(CreateAccountError::Internal(format!(
                "failed to create account row: {error}"
            )));
        }

        let sync_data = match sync_evt_data_from_commit(commit.clone()) {
            Ok(d) => d,
            Err(error) => {
                tracing::error!("Failed to build sync event data: {error:?}");
                return Err(CreateAccountError::Internal(format!(
                    "failed to build sync event data: {error:?}"
                )));
            }
        };
        let mut sequencer_clone = self
            .sequencer
            .sequencer
            .read()
            .expect("sequencer lock poisoned")
            .clone();
        if sequencer_clone
            .sequence_identity_evt(did.clone(), Some(input.handle.clone()))
            .await
            .is_err()
        {
            tracing::error!("Sequence Identity Event failed");
        }
        let (active, status): (bool, Option<_>) = AccountStatus::Active.into();
        if sequencer_clone
            .sequence_account_evt(did.clone(), active, status)
            .await
            .is_err()
        {
            tracing::error!("Sequence Account Event failed");
        }
        if sequencer_clone
            .sequence_commit(did.clone(), commit.clone())
            .await
            .is_err()
        {
            tracing::error!("Sequence Commit failed");
        }
        if sequencer_clone
            .sequence_sync_evt(did.clone(), sync_data.rev, sync_data.blocks)
            .await
            .is_err()
        {
            tracing::error!("Sequence sync event failed");
        }

        if let Err(error) = self
            .account_manager
            .update_repo_root(did.clone(), commit.commit_data.cid, commit.commit_data.rev)
            .await
        {
            tracing::error!("Failed to update repo root: {error}");
            return Err(CreateAccountError::Internal(format!(
                "failed to update repo root: {error}"
            )));
        }

        Ok(did)
    }
}

/// Build the genesis PLC operation and derive the `did:plc` from it.
/// The DID is the hash of the very operation this code signs, so the
/// rotation key necessarily predates the DID it gets filed under;
/// mirroring the canonical handler, the caller persists the rotation
/// key into the actor store once the DID is known.
async fn build_did_and_plc_op(handle: &str) -> Result<(String, Operation, Keypair), String> {
    let secp = Secp256k1::new();
    let (secret_key, public_key) = secp.generate_keypair(&mut rand::thread_rng());
    let rotation_keypair = Keypair::from_secret_key(&secp, &secret_key);
    let rotation_keys: Vec<String> = vec![encode_did_key(&public_key)];

    let create_op_input = CreateAtprotoOpInput {
        signing_key: encode_did_key(&PDS_REPO_SIGNING_KEYPAIR.public_key()),
        handle: handle.to_string(),
        pds: format!(
            "https://{}",
            std::env::var("PDS_HOSTNAME").unwrap_or("localhost".to_owned())
        ),
        rotation_keys,
    };
    let (did, op) = create_op(create_op_input, secret_key)
        .await
        .map_err(|error| format!("{error}"))?;
    Ok((did, op, rotation_keypair))
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
