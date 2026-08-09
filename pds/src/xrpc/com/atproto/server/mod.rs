pub mod activate_account;
pub mod check_account_status;
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

use crate::context::PDS_REPO_SIGNING_KEYPAIR;
use crate::plc::PlcClient;
use crate::xrpc::types::SharedIdResolver;
use anyhow::{bail, Result};
use rand::Rng;
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

    if let Some(rotation_keys) = rotation_keys
        && !rotation_keys.contains(&plc_rotation_key)
    {
        bail!("Server rotation key not included in PLC DID data")
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

pub fn routes(route: poem::Route) -> poem::Route {
    use poem::get;
    use poem::post;
    route
        .at("/createSession", post(create_session::create_session))
        .at("/refreshSession", post(refresh_session::refresh_session))
        .at("/deleteSession", post(delete_session::delete_session))
        .at("/getSession", get(get_session::get_session))
        .at(
            "/createAccount",
            post(create_account::server_create_account),
        )
        .at(
            "/activateAccount",
            post(activate_account::activate_account),
        )
        .at(
            "/deactivateAccount",
            post(deactivate_account::deactivate_account),
        )
        .at("/deleteAccount", post(delete_account::delete_account))
        .at(
            "/checkAccountStatus",
            get(check_account_status::check_account_status),
        )
        .at("/confirmEmail", post(confirm_email::confirm_email))
        .at("/updateEmail", post(update_email::update_email))
        .at(
            "/requestEmailConfirmation",
            post(request_email_confirmation::request_email_confirmation),
        )
        .at(
            "/requestEmailUpdate",
            post(request_email_update::request_email_update),
        )
        .at(
            "/requestPasswordReset",
            post(request_password_reset::request_password_reset),
        )
        .at("/resetPassword", post(reset_password::reset_password))
        .at(
            "/requestAccountDelete",
            post(request_account_delete::request_account_delete),
        )
        .at(
            "/createAppPassword",
            post(create_app_password::create_app_password),
        )
        .at(
            "/listAppPasswords",
            get(list_app_passwords::list_app_passwords),
        )
        .at(
            "/revokeAppPassword",
            post(revoke_app_password::revoke_app_password),
        )
        .at(
            "/createInviteCode",
            post(create_invite_code::create_invite_code),
        )
        .at(
            "/createInviteCodes",
            post(create_invite_codes::create_invite_codes),
        )
        .at(
            "/getAccountInviteCodes",
            get(get_account_invite_codes::get_account_invite_codes),
        )
        .at(
            "/describeServer",
            get(describe_server::describe_server),
        )
        .at(
            "/getServiceAuth",
            get(get_service_auth::get_service_auth),
        )
        .at(
            "/reserveSigningKey",
            post(reserve_signing_key::reserve_signing_key),
        )
}

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