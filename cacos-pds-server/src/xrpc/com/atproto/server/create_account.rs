//! `com.atproto.server.createAccount` handler.
//!
//! Port of the git-pinned `olamaelcu/rsky` fork at rev
//! `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky-pds`'s
//! `src/apis/com/atproto/server/create_account.rs`). The PLC-op flow
//! (`format_did_and_plc_op`) is kept, but the generated op is submitted
//! through the injected [`PlcClient`] instead of a directly-constructed
//! `plc::Client`, and account creation runs through the shared state.

use crate::xrpc::auth_extractors::UserDidAuthOptional;
use crate::xrpc::types::SharedIdResolver;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use cacos_pds_account::account::CreateAccountOpts;
use cacos_pds_account::account::helpers::account::AccountStatus;
use cacos_pds_account::auth::PDS_REPO_SIGNING_KEYPAIR;
use cacos_pds_handle::{HandleValidationOpts, normalize_and_validate_handle};
use cacos_pds_plc::operations::{CreateAtprotoOpInput, create_op};
use cacos_pds_plc::types::{OpOrTombstone, Operation};
use cacos_pds_sequencer::events::sync_evt_data_from_commit;
use email_address::EmailAddress;
use poem::web::{Data, Json};
use rsky_common::env::env_bool;
use rsky_crypto::utils::encode_did_key;
use rsky_lexicon::com::atproto::server::{CreateAccountInput, CreateAccountOutput};
use secp256k1::{Keypair, Secp256k1};
use tracing_unwrap::ResultExt;

#[derive(Debug, serde::Deserialize, serde::Serialize, Clone)]
pub struct TransformedCreateAccountInput {
    pub email: String,
    pub handle: String,
    pub did: String,
    pub invite_code: Option<String>,
    pub password: String,
    pub plc_op: Option<Operation>,
    pub deactivated: bool,
}

/// Validated inputs plus the per-DID PLC rotation key minted for a
/// PDS-created `did:plc`.
///
/// The keypair is deliberately kept out of [`TransformedCreateAccountInput`]
/// so secret material never rides along on a `Debug`/`Serialize` type.
pub struct ValidatedCreateAccount {
    pub input: TransformedCreateAccountInput,
    pub plc_rotation_key: Option<Keypair>,
}

async fn inner_create_account(
    body: CreateAccountInput,
    auth: UserDidAuthOptional,
    state: &SharedState,
    id_resolver: &SharedIdResolver,
) -> ApiResult<CreateAccountOutput> {
    tracing::info!("Creating new user account");
    let requester = match auth.access {
        Some(access) if access.credentials.is_some() => access.credentials.unwrap().iss,
        _ => None,
    };
    let ValidatedCreateAccount {
        input:
            TransformedCreateAccountInput {
                email,
                handle,
                did,
                invite_code,
                password,
                deactivated,
                plc_op,
            },
        plc_rotation_key,
    } = validate_inputs_for_local_pds(state, body, requester).await?;

    // Create new actor repo
    if let Err(error) = state
        .actor_store
        .create(&did, &PDS_REPO_SIGNING_KEYPAIR)
        .await
    {
        tracing::error!("Failed to create actor store\n{error:?}");
        return Err(ApiError::RuntimeError);
    }

    // Persist the per-DID PLC rotation key before the genesis operation is
    // submitted. The op below lists this key as the DID's only rotation
    // key, so losing it after submission would leave the DID document
    // permanently unmanageable — hence a hard failure rather than a warning.
    match &plc_rotation_key {
        Some(keypair) => {
            if let Err(error) = state
                .actor_store
                .store_rotation_keypair(&did, keypair)
                .await
            {
                tracing::error!("Failed to persist per-DID PLC rotation key\n{error:?}");
                state
                    .actor_store
                    .destroy(&did, state.blobstore.clone())
                    .await
                    .map_err(|_| ApiError::RuntimeError)?;
                return Err(ApiError::RuntimeError);
            }
        }
        // Imported DID: this PDS did not mint the document, so a rotation
        // key is convenience only. Best-effort.
        None => {
            if let Err(error) = state.actor_store.create_rotation_keypair(&did).await {
                tracing::warn!("Failed to create per-DID PLC rotation key\n{error:?}");
            }
        }
    }
    let commit = {
        let actor_txn = match state
            .actor_store
            .transact(did.clone(), state.blobstore.clone())
            .await
        {
            Ok(actor_txn) => actor_txn,
            Err(error) => {
                tracing::error!("Failed to open actor store\n{error:?}");
                state
                    .actor_store
                    .destroy(&did, state.blobstore.clone())
                    .await
                    .map_err(|_| ApiError::RuntimeError)?;
                return Err(ApiError::RuntimeError);
            }
        };
        match actor_txn.create_repo(Vec::new()).await {
            Ok(commit) => commit,
            Err(error) => {
                tracing::error!("Failed to create repo\n{error:?}");
                state
                    .actor_store
                    .destroy(&did, state.blobstore.clone())
                    .await
                    .map_err(|_| ApiError::RuntimeError)?;
                return Err(ApiError::RuntimeError);
            }
        }
    };

    // Generate a real did with PLC
    match plc_op {
        None => {}
        Some(op) => {
            match state
                .plc_client
                .send_operation(&did, &OpOrTombstone::Operation(op))
                .await
            {
                Ok(_) => tracing::info!("Successfully sent PLC Operation"),
                Err(error) => {
                    tracing::error!("Failed to create did:plc: {error}");
                    state
                        .actor_store
                        .destroy(&did, state.blobstore.clone())
                        .await
                        .map_err(|_| ApiError::RuntimeError)?;
                    return Err(ApiError::RuntimeError);
                }
            }
        }
    }

    let did_doc = match safe_resolve_did_doc(id_resolver, &did, Some(true)).await {
        Ok(res) => res,
        Err(error) => {
            tracing::error!("Error resolving DID Doc\n{error}");
            state
                .actor_store
                .destroy(&did, state.blobstore.clone())
                .await
                .map_err(|_| ApiError::RuntimeError)?;
            return Err(ApiError::RuntimeError);
        }
    };

    let (access_jwt, refresh_jwt);
    match state
        .account_manager
        .create_account(CreateAccountOpts {
            did: did.clone(),
            handle: handle.clone(),
            email: Some(email),
            password: Some(password),
            repo_cid: commit.commit_data.cid,
            repo_rev: commit.commit_data.rev.clone(),
            invite_code,
            deactivated: Some(deactivated),
            service_did: state.config.service.service_did.clone(),
        })
        .await
    {
        Ok(res) => {
            (access_jwt, refresh_jwt) = res;
        }
        Err(error) => {
            tracing::error!("Error creating account\n{error}");
            state
                .actor_store
                .destroy(&did, state.blobstore.clone())
                .await
                .map_err(|_| ApiError::RuntimeError)?;
            return Err(ApiError::RuntimeError);
        }
    }

    if !deactivated {
        let sync_data =
            sync_evt_data_from_commit(commit.clone()).map_err(|_| ApiError::RuntimeError)?;
        let mut sequencer_clone = state
            .sequencer
            .sequencer
            .read()
            .expect_or_log("sequencer lock poisoned")
            .clone();
        if sequencer_clone
            .sequence_identity_evt(did.clone(), Some(handle.clone()))
            .await
            .is_err()
        {
            tracing::error!("Sequence Identity Event failed");
        }
        let (active, status) = {
            let s: AccountStatus = AccountStatus::Active;
            let (a, st) = s.into();
            (a, st)
        };
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
    }
    state
        .account_manager
        .update_repo_root(did.clone(), commit.commit_data.cid, commit.commit_data.rev)
        .await
        .map_err(|_| ApiError::RuntimeError)?;

    let converted_did_doc = match did_doc {
        None => None,
        Some(did_doc) => match serde_json::to_value(did_doc) {
            Ok(res) => Some(res),
            Err(error) => {
                tracing::error!("Did Doc failed conversion\n{error}");
                return Err(ApiError::RuntimeError);
            }
        },
    };

    Ok(CreateAccountOutput {
        access_jwt,
        refresh_jwt,
        handle,
        did,
        did_doc: converted_did_doc,
    })
}

/// POST /xrpc/com.atproto.server.createAccount
#[allow(clippy::too_many_arguments)]
#[poem::handler]
pub async fn server_create_account(
    body: Json<CreateAccountInput>,
    auth: UserDidAuthOptional,
    state: Data<&SharedState>,
) -> ApiResult<Json<CreateAccountOutput>> {
    let id_resolver = SharedIdResolver {
        id_resolver: state.id_resolver.clone(),
    };
    match inner_create_account(body.0, auth, state.0, &id_resolver).await {
        Ok(out) => Ok(Json(out)),
        Err(error) => Err(error),
    }
}

/// Validates Create Account Parameters and builds the PLC Operation if needed.
pub async fn validate_inputs_for_local_pds(
    state: &SharedState,
    input: CreateAccountInput,
    requester: Option<String>,
) -> Result<ValidatedCreateAccount, ApiError> {
    let did: String;
    let plc_op;
    let deactivated: bool;
    let email: String;
    let plc_rotation_key;

    if input.plc_op.is_some() {
        return Err(ApiError::InvalidRequest(
            "Unsupported input: `plcOp`".to_string(),
        ));
    }

    let invites_required = env_bool("PDS_INVITE_REQUIRED").unwrap_or(false);
    let invite_code = if invites_required && input.invite_code.is_none() {
        return Err(ApiError::InvalidInviteCode);
    } else {
        input.invite_code.clone()
    };

    if input.email.is_none() {
        return Err(ApiError::InvalidEmail);
    }
    match input.email {
        None => return Err(ApiError::InvalidEmail),
        Some(ref input_email) => {
            let e_slice: &str = input_email.as_str();
            if !EmailAddress::is_valid(e_slice) {
                return Err(ApiError::InvalidEmail);
            } else {
                email = input_email.clone();
            }
        }
    }

    let handle = normalize_and_validate_handle(
        HandleValidationOpts {
            handle: input.handle.clone(),
            did: requester.clone(),
            allow_reserved: None,
        },
        &state.config.identity.service_handle_domains,
    )
    .await
    .map_err(|e| match e.kind {
        cacos_pds_handle::errors::ErrorKind::InvalidHandle => ApiError::InvalidHandle,
        cacos_pds_handle::errors::ErrorKind::HandleNotAvailable => ApiError::HandleNotAvailable,
        cacos_pds_handle::errors::ErrorKind::UnsupportedDomain => ApiError::UnsupportedDomain,
        cacos_pds_handle::errors::ErrorKind::InternalError => ApiError::RuntimeError,
    })?;
    if !super::validate_handle(&handle, &state.config.identity.service_handle_domains) {
        return Err(ApiError::InvalidHandle);
    }

    let handle_accnt = state
        .account_manager
        .get_account(&handle, None)
        .await
        .map_err(|_| ApiError::RuntimeError)?;
    let email_accnt = state
        .account_manager
        .get_account_by_email(&email, None)
        .await
        .map_err(|_| ApiError::RuntimeError)?;
    if handle_accnt.is_some() {
        return Err(ApiError::HandleNotAvailable);
    } else if email_accnt.is_some() {
        return Err(ApiError::EmailNotAvailable);
    }

    let password = match input.password {
        None => return Err(ApiError::InvalidPassword),
        Some(ref pass) => pass.clone(),
    };

    match input.did {
        Some(input_did) => {
            if Some(&input_did) == requester.as_ref() {
                return Err(ApiError::AuthRequiredError(format!(
                    "Missing auth to create account with did: {input_did}"
                )));
            }
            did = input_did;
            plc_op = None;
            deactivated = true;
            plc_rotation_key = None;
        }
        None => {
            let res = format_did_and_plc_op(input, &state.config.service.hostname).await?;
            did = res.0;
            plc_op = Some(res.1);
            plc_rotation_key = Some(res.2);
            deactivated = false;
        }
    };

    Ok(ValidatedCreateAccount {
        input: TransformedCreateAccountInput {
            email,
            handle,
            did,
            invite_code,
            password,
            plc_op,
            deactivated,
        },
        plc_rotation_key,
    })
}

/// Builds the genesis PLC operation and derives the `did:plc` from it.
///
/// The returned keypair is this DID's own rotation key and is the only
/// rotation key in the operation (besides an optional caller-supplied
/// recovery key). It is generated here rather than read back from the
/// actor store because a `did:plc` is the hash of the very operation this
/// key signs — the key necessarily predates the DID it gets filed under.
/// `inner_create_account` persists it once the DID is known.
///
/// Deliberately no longer uses the shared `PDS_PLC_ROTATION_KEYPAIR`: a
/// single server-wide key in every DID document means one key compromise
/// takes over every account the PDS ever created.
async fn format_did_and_plc_op(
    input: CreateAccountInput,
    hostname: &str,
) -> Result<(String, Operation, Keypair), ApiError> {
    let mut rotation_keys: Vec<String> = Vec::new();

    if let Some(recovery_key) = &input.recovery_key {
        rotation_keys.push(recovery_key.clone());
    }

    let secp = Secp256k1::new();
    let (secret_key, public_key) = secp.generate_keypair(&mut secp256k1::rand::thread_rng());
    let rotation_keypair = Keypair::from_secret_key(&secp, &secret_key);
    rotation_keys.push(encode_did_key(&public_key));

    let create_op_input = CreateAtprotoOpInput {
        signing_key: encode_did_key(&PDS_REPO_SIGNING_KEYPAIR.public_key()),
        handle: input.handle,
        pds: format!("https://{hostname}"),
        rotation_keys,
    };
    let response = match create_op(create_op_input, secret_key).await {
        Ok(res) => res,
        Err(error) => {
            tracing::error!("{error}");
            return Err(ApiError::RuntimeError);
        }
    };

    Ok((response.0, response.1, rotation_keypair))
}

use crate::xrpc::com::atproto::server::safe_resolve_did_doc;
