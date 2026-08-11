//! Fail-fast boot-time validation for every required env var.
//!
//! [`init_required_keys`] runs at process start (called from `main`) and
//! returns an error on the first missing or malformed secret. The point is
//! to refuse to bind the listener rather than crash mid-request when a
//! downstream caller finally dereferences the lazy `PDS_*_KEYPAIR` static
//! or tries to mint a DPoP nonce. Each check uses
//! [`crate::account::helpers::secrets::read_secret`] so the `_FILE`
//! indirection (and the alternative `PDS_SECRET_BACKEND` providers) is
//! honoured uniformly.

fn required(name: &str) -> Result<String, String> {
    crate::account::helpers::secrets::read_secret(name)
        .map_err(|_| format!("{name} is required but not set"))
}

fn required_with_legacy(name: &str, legacy: &str) -> Result<String, String> {
    required(name).or_else(|_| {
        required(legacy).map_err(|_| format!("{name} (or legacy {legacy}) is required but not set"))
    })
}

fn validate_hex(name: &str, value: &str) -> Result<(), String> {
    hex::decode(value).map_err(|_| format!("{name} is not valid hex"))?;
    Ok(())
}

pub fn init_required_keys() -> Result<(), String> {
    let access = required_with_legacy(
        "PDS_ACCESS_JWT_KEY_K256_PRIVATE_KEY_HEX",
        "PDS_JWT_KEY_K256_PRIVATE_KEY_HEX",
    )?;
    validate_hex("PDS_ACCESS_JWT_KEY_K256_PRIVATE_KEY_HEX", &access)?;
    let _ = &*crate::account::helpers::auth::PDS_JWT_KEYPAIR;

    let repo = required("PDS_REPO_SIGNING_KEY_K256_PRIVATE_KEY_HEX")?;
    validate_hex("PDS_REPO_SIGNING_KEY_K256_PRIVATE_KEY_HEX", &repo)?;
    let _ = &*crate::context::PDS_REPO_SIGNING_KEYPAIR;

    let plc = required("PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX")?;
    validate_hex("PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX", &plc)?;
    let _ = &*crate::xrpc::com::atproto::server::PDS_PLC_ROTATION_KEYPAIR;

    let dpop = required("PDS_DPOP_SECRET")?;
    if dpop.len() != 64 {
        return Err("PDS_DPOP_SECRET must be 32-byte hex (64 chars)".to_string());
    }
    validate_hex("PDS_DPOP_SECRET", &dpop)?;

    let _ = required_with_legacy(
        "PDS_OAUTH_SIGNING_KEY_K256_PRIVATE_KEY_HEX",
        "PDS_JWT_KEY_K256_PRIVATE_KEY_HEX",
    )?;

    let _ = required("PDS_OAUTH_TRUSTED_CLIENTS")?;
    let _ = crate::account::helpers::secrets::read_admin_password()?;
    Ok(())
}
