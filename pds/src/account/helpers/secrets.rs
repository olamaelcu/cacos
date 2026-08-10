//! Secret loading helpers: `*_FILE` indirection and password-strength validation.
//!
//! Standard Docker/Kubernetes/Vault convention: each secret env var `FOO`
//! can be set directly OR via `FOO_FILE` pointing to a file containing
//! the value. This lets operators mount secrets as files without putting
//! them in process env.

use std::env;

/// Reads a secret from `name` (env var) or `name_FILE` (path to file).
pub fn read_secret(name: &str) -> Result<String, String> {
    let file_var = format!("{name}_FILE");
    if let Ok(path) = env::var(&file_var) {
        let value = std::fs::read_to_string(&path)
            .map_err(|e| format!("failed to read {file_var}={path}: {e}"))?;
        return Ok(value.trim_end_matches(['\n', '\r']).to_string());
    }
    env::var(name).map_err(|_| format!("{name} (or {file_var}) must be set"))
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
