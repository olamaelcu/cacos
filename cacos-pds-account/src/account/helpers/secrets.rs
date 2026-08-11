//! Secret loading helpers: backend-agnostic reads and password-strength
//! validation.
//!
//! The actual lookup lives behind
//! [`crate::account::helpers::secret_provider::SecretProvider`], selected by
//! `PDS_SECRET_BACKEND`. The default `env` backend keeps the standard
//! Docker/Kubernetes/Vault convention: each secret env var `FOO` can be set
//! directly OR via `FOO_FILE` pointing to a file containing the value, so
//! operators can mount secrets as files without putting them in process env.

use std::env;

use crate::account::helpers::secret_provider::provider;

/// Reads a secret by name from the configured secret backend.
pub fn read_secret(name: &str) -> Result<String, String> {
    provider().and_then(|p| p.read(name).map_err(|e| e.to_string()))
}

/// Validates password strength: min 16 chars, ≥3 of {lower, upper, digit,
/// symbol} classes, and not in a small deny-list.
pub fn validate_password_strength(name: &str, value: &str) -> Result<(), String> {
    if value.len() < 16 {
        return Err(format!("{name} must be at least 16 characters"));
    }
    let has_lower = value.chars().any(|c| c.is_ascii_lowercase());
    let has_upper = value.chars().any(|c| c.is_ascii_uppercase());
    let has_digit = value.chars().any(|c| c.is_ascii_digit());
    let has_symbol = value.chars().any(|c| !c.is_alphanumeric());
    let classes = [has_lower, has_upper, has_digit, has_symbol]
        .iter()
        .filter(|x| **x)
        .count();
    if classes < 3 {
        return Err(format!(
            "{name} must contain at least 3 of: lowercase, uppercase, digit, symbol"
        ));
    }
    const DENY_LIST: &[&str] = &[
        "password",
        "changeme",
        "admin123",
        "secret123",
        "letmein",
        "welcome",
        "1234567890",
        "qwertyuiop",
        "passw0rd",
    ];
    let lower = value.to_lowercase();
    if DENY_LIST.iter().any(|w| lower.contains(w)) {
        return Err(format!("{name} is too common; choose a stronger password"));
    }
    Ok(())
}

/// Reads PDS_ADMIN_PASSWORD with `*_FILE` support and strength validation.
/// `PDS_ALLOW_INSECURE_ADMIN=true` bypasses strength check.
pub fn read_admin_password() -> Result<String, String> {
    let value = read_secret("PDS_ADMIN_PASSWORD")?;
    if env::var("PDS_ALLOW_INSECURE_ADMIN").as_deref() == Ok("true") {
        return Ok(value);
    }
    validate_password_strength("PDS_ADMIN_PASSWORD", &value)?;
    Ok(value)
}
