# Task 5: Server module helpers — `server/mod.rs`, describeServer, getServiceAuth, reserveSigningKey

**Files:**
- Create: `pds/src/xrpc/com/atproto/server/mod.rs` (helpers + route table; replace the Task 1 placeholder)
- Create: `pds/src/xrpc/com/atproto/server/describe_server.rs`
- Create: `pds/src/xrpc/com/atproto/server/get_service_auth.rs`
- Create: `pds/src/xrpc/com/atproto/server/reserve_signing_key.rs`
- Test: `pds/src/xrpc/com/atproto/server/mod.rs` (in-file unit tests for helpers)

- [ ] **Step 1: Write the failing tests**

```rust
// appended to pds/src/xrpc/com/atproto/server/mod.rs
#[cfg(test)]
mod tests {
    use super::*;

    fn domains() -> Vec<String> {
        vec![
            ".pds.example.com".to_string(),
            "alt.example.net".to_string(),
        ]
    }

    #[test]
    fn accepts_direct_child_of_service_domain() {
        assert!(validate_handle("alice.pds.example.com", &domains()));
    }

    #[test]
    fn accepts_direct_child_of_secondary_domain() {
        assert!(validate_handle("bob.alt.example.net", &domains()));
    }

    #[test]
    fn rejects_evil_suffix_domain() {
        assert!(!validate_handle("alice.evilpds.example.com", &domains()));
        assert!(!validate_handle("evilpds.example.com", &domains()));
    }

    #[test]
    fn rejects_multi_label_handles() {
        assert!(!validate_handle("a.b.pds.example.com", &domains()));
    }

    #[test]
    fn rejects_bare_service_domain() {
        assert!(!validate_handle("pds.example.com", &domains()));
        assert!(!validate_handle("alt.example.net", &domains()));
    }
}
```

Run: `cargo test -p pds server::tests`
Expected: FAIL — helpers missing.

- [ ] **Step 2: Implement `server/mod.rs` helpers + route table**

Port of `the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/apis/com/atproto/server/mod.rs` (helpers `validate_handle`, `gen_invite_code`, `gen_invite_codes`, `get_random_token`, `safe_resolve_did_doc`, `assert_valid_did_documents_for_service`, `is_valid_did_doc_for_service`, `assert_valid_doc_contents`) plus the `PDS_PLC_ROTATION_KEYPAIR` static. `safe_resolve_did_doc`/`assert_valid_did_documents_for_service` use `SharedIdResolver` instead of rocket `State`.

```rust
// pds/src/xrpc/com/atproto/server/mod.rs
pub mod confirm_email;
pub mod create_account;
pub mod create_app_password;
pub mod create_invite_code;
pub mod create_invite_codes;
pub mod create_session;
pub mod deactivate_account;
pub mod delete_account;
pub mod delete_session;
pub mod describe_server;
pub mod get_account_invite_codes;
pub mod get_service_auth;
pub mod get_session;
pub mod list_app_passwords;
pub mod refresh_session;
pub mod request_account_delete;
pub mod request_email_confirmation;
pub mod request_email_update;
pub mod request_password_reset;
pub mod reserve_signing_key;
pub mod reset_password;
pub mod revoke_app_password;
pub mod update_email;

pub mod activate_account;
pub mod check_account_status;

use crate::config::ServerConfig;
use crate::context::PDS_REPO_SIGNING_KEYPAIR;
use crate::plc::PlcClient;
use crate::xrpc::types::SharedIdResolver;
use anyhow::{bail, Result};
use rsky_crypto::utils::encode_did_key;
use rsky_identity::types::DidDocument;
use secp256k1::{Keypair, Secp256k1, SecretKey};
use std::env;
use std::sync::LazyLock;

pub static PDS_PLC_ROTATION_KEYPAIR: LazyLock<Keypair> = LazyLock::new(|| {
    let secp = Secp256k1::new();
    let private_key = env::var("PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX").unwrap();
    let secret_key = SecretKey::from_slice(&hex::decode(private_key.as_bytes()).unwrap()).unwrap();
    Keypair::from_secret_key(&secp, &secret_key)
});

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct AssertionContents {
    pub signing_key: Option<String>,
    pub pds_endpoint: Option<String>,
    pub rotation_keys: Option<Vec<String>>,
}

/// Formatted xxxxx-xxxxx
pub fn get_random_token() -> String {
    let token: String = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(50)
        .map(char::from)
        .collect();
    // Bluesky Client doesn't support 1,8,9,0 in the email verification tokens
    let allowed_token = token.replace(&['1', '8', '9', '0'][..], "");
    allowed_token[0..5].to_owned() + "-" + &allowed_token[5..10]
}

pub async fn safe_resolve_did_doc(
    id_resolver: &SharedIdResolver,
    did: &String,
    force_refresh: Option<bool>,
) -> Result<Option<DidDocument>> {
    let lock = id_resolver.id_resolver.write().await;
    match lock.did.resolve(did.clone(), force_refresh).await {
        Ok(did_doc) => Ok(did_doc),
        Err(err) => {
            tracing::error!("failed to resolve did doc for `{did}`: {err}");
            Ok(None)
        }
    }
}

/// generate an invite code preceded by the hostname with '.'s replaced by '-'s
pub fn gen_invite_code() -> String {
    env::var("PDS_HOSTNAME")
        .unwrap_or("localhost".to_owned())
        .replace('.', "-")
        + "-"
        + &get_random_token().to_lowercase()
}

pub fn gen_invite_codes(count: i32) -> Vec<String> {
    let mut codes = Vec::new();
    for _i in 0..count {
        codes.push(gen_invite_code());
    }
    codes
}

pub fn validate_handle(handle: &str, service_handle_domains: &[String]) -> bool {
    service_handle_domains.iter().any(|domain| {
        let suffix = if domain.starts_with('.') {
            domain.clone()
        } else {
            format!(".{domain}")
        };
        handle
            .strip_suffix(suffix.as_str())
            .is_some_and(|front| !front.is_empty() && !front.contains('.'))
    })
}

pub async fn is_valid_did_doc_for_service(did: String, plc: &dyn PlcClient) -> Result<bool> {
    match assert_valid_did_documents_for_service(did, plc).await {
        Ok(()) => Ok(true),
        Err(_) => Ok(false),
    }
}

pub async fn assert_valid_did_documents_for_service(
    did: String,
    plc: &dyn PlcClient,
) -> Result<()> {
    if did.starts_with("did:plc") {
        let resolved = plc.get_document_data(&did).await?;
        let pds_endpoint = resolved
            .services
            .get("atproto_pds")
            .map(|service| service.endpoint.clone());
        let signing_key = resolved.verification_methods.get("atproto").cloned();
        assert_valid_doc_contents(AssertionContents {
            pds_endpoint,
            signing_key,
            rotation_keys: Some(resolved.rotation_keys),
        })
        .await?;
    } else {
        bail!("Not yet supporting did:web")
    }
    Ok(())
}

pub async fn assert_valid_doc_contents(contents: AssertionContents) -> Result<()> {
    let AssertionContents {
        signing_key,
        pds_endpoint,
        rotation_keys,
    } = contents;
    let plc_rotation_key = encode_did_key(&PDS_PLC_ROTATION_KEYPAIR.public_key());

    if let Some(rotation_keys) = rotation_keys {
        if !rotation_keys.contains(plc_rotation_key) {
            bail!("Server rotation key not included in PLC DID data")
        }
    }
    let port = std::env::var("PDS_PORT")
        .ok()
        .and_then(|p| p.parse::<usize>().ok())
        .unwrap_or(2583);
    let hostname = std::env::var("PDS_HOSTNAME").unwrap_or("localhost".to_owned());
    let public_url = if hostname == "localhost" {
        format!("http://localhost:{port}")
    } else {
        format!("https://{hostname}")
    };

    if pds_endpoint.is_none() || pds_endpoint.unwrap() != public_url {
        bail!("DID document atproto_pds service endpoint does not match PDS public url")
    }

    if signing_key.is_none()
        || signing_key.unwrap() != encode_did_key(&PDS_REPO_SIGNING_KEYPAIR.public_key())
    {
        bail!("DID document verification method does not match expected signing key")
    }
    Ok(())
}

pub fn routes() -> poem::Route {
    use poem::post;
    use poem::get;
    poem::Route::new()
        .at("/createSession", post(create_session::create_session))
        .at("/refreshSession", post(refresh_session::refresh_session))
        .at("/deleteSession", post(delete_session::delete_session))
        .at("/getSession", get(get_session::get_session))
        .at("/createAccount", post(create_account::server_create_account))
        .at("/activateAccount", post(activate_account::activate_account))
        .at("/deactivateAccount", post(deactivate_account::deactivate_account))
        .at("/deleteAccount", post(delete_account::delete_account))
        .at("/checkAccountStatus", get(check_account_status::check_account_status))
        .at("/confirmEmail", post(confirm_email::confirm_email))
        .at("/updateEmail", post(update_email::update_email))
        .at("/requestEmailConfirmation", post(request_email_confirmation::request_email_confirmation))
        .at("/requestEmailUpdate", post(request_email_update::request_email_update))
        .at("/requestPasswordReset", post(request_password_reset::request_password_reset))
        .at("/resetPassword", post(reset_password::reset_password))
        .at("/requestAccountDelete", post(request_account_delete::request_account_delete))
        .at("/createAppPassword", post(create_app_password::create_app_password))
        .at("/listAppPasswords", get(list_app_passwords::list_app_passwords))
        .at("/revokeAppPassword", post(revoke_app_password::revoke_app_password))
        .at("/createInviteCode", post(create_invite_code::create_invite_code))
        .at("/createInviteCodes", post(create_invite_codes::create_invite_codes))
        .at("/getAccountInviteCodes", get(get_account_invite_codes::get_account_invite_codes))
        .at("/describeServer", get(describe_server::describe_server))
        .at("/getServiceAuth", get(get_service_auth::get_service_auth))
        .at("/reserveSigningKey", post(reserve_signing_key::reserve_signing_key))
}
```

The `routes()` table references all handlers; Tasks 6–10 create each file. Keep `routes()` in sync as the files land.

- [ ] **Step 3: Implement `describe_server.rs`**

Port of `the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/apis/com/atproto/server/describe_server.rs` (reads env directly).

```rust
// pds/src/xrpc/com/atproto/server/describe_server.rs
use crate::xrpc::ApiResult;
use poem::web::Json;
use rsky_lexicon::com::atproto::server::{
    DescribeServerOutput, DescribeServerRefContact, DescribeServerRefLinks,
};

/// GET /xrpc/com.atproto.server.describeServer
#[poem::handler]
pub async fn describe_server() -> ApiResult<Json<DescribeServerOutput>> {
    let available_user_domains = crate::config::env_list("PDS_SERVICE_HANDLE_DOMAINS");
    let invite_code_required = crate::config::env_bool("PDS_INVITE_REQUIRED");
    let privacy_policy = crate::config::env_str("PDS_PRIVACY_POLICY_URL");
    let terms_of_service = crate::config::env_str("PDS_TERMS_OF_SERVICE_URL");
    let contact_email_address = crate::config::env_str("PDS_CONTACT_EMAIL_ADDRESS");

    Ok(Json(DescribeServerOutput {
        did: crate::config::env_str("PDS_SERVICE_DID").unwrap(),
        available_user_domains,
        invite_code_required,
        phone_verification_required: None,
        links: DescribeServerRefLinks {
            privacy_policy,
            terms_of_service,
        },
        contact: DescribeServerRefContact {
            email: contact_email_address,
        },
    }))
}
```

> `crate::config::{env_list, env_bool, env_str}` are thin wrappers over `std::env` (as in `rsky_common::env`); add them to `config.rs` if Plan 01 did not.

- [ ] **Step 4: Implement `get_service_auth.rs`**

Port of `the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/apis/com/atproto/server/get_service_auth.rs`. `PRIVILEGED_METHODS`/`PROTECTED_METHODS` move here as consts (pipethrough is deferred).

```rust
// pds/src/xrpc/com/atproto/server/get_service_auth.rs
use crate::account::helpers::auth::{create_service_jwt, ServiceJwtParams};
use crate::xrpc::auth_extractors::AccessFull;
use crate::xrpc::{ApiError, ApiResult};
use anyhow::{bail, Result};
use chrono::offset::Utc as UtcOffset;
use chrono::DateTime;
use poem::web::Json;
use rsky_common::time::{from_micros_to_utc, HOUR, MINUTE};
use rsky_lexicon::com::atproto::server::GetServiceAuthOutput;
use std::time::SystemTime;

/// Methods that may not be proxied or service-authed (pipethrough consts,
/// ported here because cacos defers the pipethrough module).
pub static PROTECTED_METHODS: &[&str] = &[
    "com.atproto.admin.sendEmail",
    "com.atproto.identity.requestPlcOperationSignature",
    "com.atproto.identity.signPlcOperation",
    "com.atproto.identity.updateHandle",
    "com.atproto.server.activateAccount",
    "com.atproto.server.confirmEmail",
    "com.atproto.server.createAppPassword",
    "com.atproto.server.deactivateAccount",
    "com.atproto.server.getAccountInviteCodes",
    "com.atproto.server.listAppPasswords",
    "com.atproto.server.requestAccountDelete",
    "com.atproto.server.requestEmailConfirmation",
    "com.atproto.server.requestEmailUpdate",
    "com.atproto.server.revokeAppPassword",
    "com.atproto.server.updateEmail",
];

pub static PRIVILEGED_METHODS: &[&str] = &[
    "chat.bsky.actor.deleteAccount",
    "chat.bsky.actor.exportAccountData",
    "chat.bsky.convo.deleteMessageForSelf",
    "chat.bsky.convo.getConvo",
    "chat.bsky.convo.getConvoForMembers",
    "chat.bsky.convo.getLog",
    "chat.bsky.convo.getMessages",
    "chat.bsky.convo.leaveConvo",
    "chat.bsky.convo.listConvos",
    "chat.bsky.convo.muteConvo",
    "chat.bsky.convo.sendMessage",
    "chat.bsky.convo.sendMessageBatch",
    "chat.bsky.convo.unmuteConvo",
    "chat.bsky.convo.updateRead",
    "com.atproto.server.createAccount",
];

pub async fn inner_get_service_auth(
    aud: String,
    exp: Option<u64>,
    lxm: Option<String>,
    auth: AccessFull,
) -> Result<String> {
    let credentials = auth.access.credentials.unwrap();
    let did = credentials.clone().did.unwrap();
    let exp = exp.map(|exp| exp * 1000);
    if let Some(exp) = exp {
        let system_time = SystemTime::now();
        let now: DateTime<UtcOffset> = system_time.into();
        let diff = from_micros_to_utc(exp as i64) - now;
        if diff.num_milliseconds() < 0 {
            bail!("BadExpiration: expiration is in past");
        } else if diff.num_milliseconds() > HOUR as i64 {
            bail!("BadExpiration: cannot request a token with an expiration more than an hour in the future");
        } else if lxm.is_none() && diff.num_milliseconds() > MINUTE as i64 {
            bail!("BadExpiration: cannot request a method-less token with an expiration more than a minute in the future");
        }
    }
    if let Some(ref lxm) = lxm {
        if PROTECTED_METHODS.contains(&lxm.as_str()) {
            bail!("cannot request a service auth token for the following protected method: {lxm}");
        }
        if credentials.is_privileged.unwrap_or(false) && PRIVILEGED_METHODS.contains(&lxm.as_str()) {
            bail!("insufficient access to request a service auth token for the following method: {lxm}");
        }
    }
    create_service_jwt(ServiceJwtParams {
        iss: did,
        aud,
        exp: None,
        lxm,
        jti: None,
    })
    .await
}

/// GET /xrpc/com.atproto.server.getServiceAuth?<aud>&<exp>&<lxm>
#[poem::handler]
pub async fn get_service_auth(
    poem::web::Query(query): poem::web::Query<GetServiceAuthQuery>,
    auth: AccessFull,
) -> ApiResult<Json<GetServiceAuthOutput>> {
    let GetServiceAuthQuery { aud, exp, lxm } = query;
    match inner_get_service_auth(aud, exp, lxm, auth).await {
        Ok(token) => Ok(Json(GetServiceAuthOutput { token })),
        Err(error) => {
            tracing::error!("Internal Error: {error}");
            Err(ApiError::RuntimeError)
        }
    }
}

#[derive(serde::Deserialize)]
pub struct GetServiceAuthQuery {
    pub aud: String,
    pub exp: Option<u64>,
    pub lxm: Option<String>,
}
```

- [ ] **Step 5: Implement `reserve_signing_key.rs`**

Port of `the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/apis/com/atproto/server/reserve_signing_key.rs` (the lexicon does not define these types; the reference declares them inline).

```rust
// pds/src/xrpc/com/atproto/server/reserve_signing_key.rs
use crate::actor_store::ActorStore;
use crate::xrpc::{ApiError, ApiResult};
use poem::web::Json;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct ReserveSigningKeyInput {
    pub did: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReserveSigningKeyOutput {
    pub signing_key: String,
}

/// POST /xrpc/com.atproto.server.reserveSigningKey
#[poem::handler]
pub async fn reserve_signing_key(
    body: Json<ReserveSigningKeyInput>,
    state: poem::State<crate::xrpc::SharedState>,
) -> ApiResult<Json<ReserveSigningKeyOutput>> {
    let ReserveSigningKeyInput { did } = body.0;
    match state.actor_store.reserve_keypair(did.as_deref()).await {
        Ok(signing_key) => Ok(Json(ReserveSigningKeyOutput { signing_key })),
        Err(error) => {
            tracing::error!("{error}");
            Err(ApiError::RuntimeError)
        }
    }
}
```

- [ ] **Step 6: Run the tests**

Run: `cargo test -p pds server::tests`
Expected: PASS (5 helper tests). Handlers referencing not-yet-created sibling modules (`create_session` etc.) will not compile until Tasks 6–10 land — the agent should implement the remaining server files before running the full suite; to keep each commit green, create empty stub files with a `#[poem::handler] pub async fn` returning `ApiError::RuntimeError` for the not-yet-ported handlers in this commit (Task 6–10 replace them).

- [ ] **Step 7: Commit**

```bash
git add pds/src/xrpc/com/atproto/server
git commit -m "feat(server): mod helpers, describeServer, getServiceAuth, reserveSigningKey"
```
