//! Integration tests for the secret-loading helpers
//! (`account::helpers::secrets`) and the admin-token registry
//! (`account::helpers::admin_tokens`).
//!
//! Each test mutates the process-wide environment directly via
//! `unsafe { std::env::set_var(...) }`. Cargo runs the tests in this
//! binary sequentially within a single process, so the `unsafe` is safe
//! in practice; it mirrors the pattern used elsewhere in this crate
//! (see `pds::auth::auth_verifier::tests`).

use std::sync::Mutex;

use cacos_pds::account::helpers::admin_tokens::{
    AdminScope, AdminScopeSet, AdminTokenRegistry, cached_admin_token_registry,
    reset_admin_token_registry,
};
use cacos_pds::account::helpers::secrets::{
    read_admin_password, read_secret, validate_password_strength,
};

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn lock_env() -> std::sync::MutexGuard<'static, ()> {
    // Tests in this binary mutate the process-wide environment. Holding the
    // lock for the duration of each test prevents Cargo's parallel-test
    // harness from interleaving env writes.
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn strong_password() -> String {
    // 24 chars, mixed case + digits + symbols. Avoids the deny-list.
    "Tr0ub4dor&3lephant-Quail!".to_string()
}

fn clear_test_env() {
    // SAFETY: serialised by ENV_LOCK.
    unsafe {
        // Clear any PDS_ADMIN_TOKEN_* entries that prior tests may have left.
        let keys: Vec<String> = std::env::vars()
            .map(|(k, _)| k)
            .filter(|k| k.starts_with("PDS_ADMIN_TOKEN_"))
            .collect();
        for k in keys {
            std::env::remove_var(&k);
        }
        std::env::remove_var("PDS_ADMIN_PASSWORD");
        std::env::remove_var("PDS_ADMIN_PASS");
        std::env::remove_var("PDS_ALLOW_INSECURE_ADMIN");
        // Clear the secrets-test fixture env vars.
        for var in [
            "PDS_TEST_SECRET_FILE_ONLY",
            "PDS_TEST_SECRET_FILE_ONLY_FILE",
            "PDS_TEST_SECRET_ENV_ONLY",
            "PDS_TEST_SECRET_NEITHER",
            "PDS_TEST_SECRET_NEITHER_FILE",
            "PDS_TEST_SECRET_TRIM",
            "PDS_TEST_SECRET_TRIM_FILE",
        ] {
            std::env::remove_var(var);
            std::env::remove_var(format!("{var}_FILE"));
        }
    }
}

#[test]
fn read_secret_prefers_file_over_env() {
    let _guard = lock_env();
    clear_test_env();
    let dir = camino_tempfile::Utf8TempDir::new().unwrap();
    let path = dir.path().join("value.txt");
    std::fs::write(&path, "from-file").unwrap();
    // SAFETY: serialised by ENV_LOCK above.
    unsafe {
        std::env::set_var("PDS_TEST_SECRET_FILE_ONLY", "from-env");
        std::env::set_var("PDS_TEST_SECRET_FILE_ONLY_FILE", path.as_str());
    }

    let got = read_secret("PDS_TEST_SECRET_FILE_ONLY").unwrap();
    assert_eq!(got, "from-file");

    // SAFETY: serialised by ENV_LOCK above.
    unsafe {
        std::env::remove_var("PDS_TEST_SECRET_FILE_ONLY");
        std::env::remove_var("PDS_TEST_SECRET_FILE_ONLY_FILE");
    }
}

#[test]
fn read_secret_falls_back_to_env() {
    let _guard = lock_env();
    clear_test_env();
    // SAFETY: serialised by ENV_LOCK above.
    unsafe {
        std::env::set_var("PDS_TEST_SECRET_ENV_ONLY", "env-value");
    }
    let got = read_secret("PDS_TEST_SECRET_ENV_ONLY").unwrap();
    assert_eq!(got, "env-value");
    // SAFETY: serialised by ENV_LOCK above.
    unsafe {
        std::env::remove_var("PDS_TEST_SECRET_ENV_ONLY");
    }
}

#[test]
fn read_secret_errors_when_neither_set() {
    let _guard = lock_env();
    clear_test_env();
    let err = read_secret("PDS_TEST_SECRET_NEITHER").unwrap_err();
    assert!(err.contains("PDS_TEST_SECRET_NEITHER"));
    assert!(err.contains("_FILE"));
}

#[test]
fn read_secret_trims_trailing_newlines() {
    let _guard = lock_env();
    clear_test_env();
    let dir = camino_tempfile::Utf8TempDir::new().unwrap();
    let path = dir.path().join("value.txt");
    std::fs::write(&path, "secret-value\n\n\r").unwrap();
    // SAFETY: serialised by ENV_LOCK above.
    unsafe {
        std::env::set_var("PDS_TEST_SECRET_TRIM_FILE", path.as_str());
    }
    let got = read_secret("PDS_TEST_SECRET_TRIM").unwrap();
    assert_eq!(got, "secret-value");
    // SAFETY: serialised by ENV_LOCK above.
    unsafe {
        std::env::remove_var("PDS_TEST_SECRET_TRIM_FILE");
    }
}

#[test]
fn validate_password_strength_accepts_strong_password() {
    let value = strong_password();
    validate_password_strength("PDS_ADMIN_PASSWORD", &value).unwrap();
}

#[test]
fn validate_password_strength_rejects_short() {
    let err = validate_password_strength("PDS_ADMIN_PASSWORD", "short").unwrap_err();
    assert!(err.contains("at least 16 characters"), "got: {err}");
}

#[test]
fn validate_password_strength_rejects_low_diversity() {
    // 16 chars but only one class (lowercase).
    let err = validate_password_strength("PDS_ADMIN_PASSWORD", "sixteenletterstring").unwrap_err();
    assert!(err.contains("at least 3 of"), "got: {err}");
}

#[test]
fn validate_password_strength_rejects_common() {
    // 24 chars (>=16), 3+ classes, but contains "passw0rd" (deny-list entry).
    let err =
        validate_password_strength("PDS_ADMIN_PASSWORD", "Abcdefghi1234567passw0rdXX").unwrap_err();
    assert!(err.contains("too common"), "got: {err}");
}

#[test]
fn read_admin_password_enforces_strength_unless_bypassed() {
    let _guard = lock_env();
    clear_test_env();
    // SAFETY: serialised by ENV_LOCK above.
    unsafe {
        std::env::set_var("PDS_ADMIN_PASSWORD", "short");
    }
    let err = read_admin_password().unwrap_err();
    assert!(err.contains("at least 16 characters"), "got: {err}");

    // SAFETY: serialised by ENV_LOCK above.
    unsafe {
        std::env::set_var("PDS_ALLOW_INSECURE_ADMIN", "true");
    }
    let got = read_admin_password().unwrap();
    assert_eq!(got, "short");

    // SAFETY: serialised by ENV_LOCK above.
    unsafe {
        std::env::set_var("PDS_ADMIN_PASSWORD", strong_password());
        std::env::remove_var("PDS_ALLOW_INSECURE_ADMIN");
    }
    let got = read_admin_password().unwrap();
    assert_eq!(got, strong_password());

    // SAFETY: serialised by ENV_LOCK above.
    unsafe {
        std::env::remove_var("PDS_ADMIN_PASSWORD");
    }
}

#[test]
fn admin_token_registry_discovers_named_tokens() {
    let _guard = lock_env();
    clear_test_env();
    // SAFETY: serialised by ENV_LOCK above.
    unsafe {
        std::env::set_var("PDS_ADMIN_TOKEN_OPS_SECRET", "ops-shared-secret");
        std::env::set_var("PDS_ADMIN_TOKEN_OPS_SCOPES", "InviteAdmin,AccountAdmin");
        std::env::set_var("PDS_ADMIN_TOKEN_OPS_NAME", "ops-team");
        std::env::set_var("PDS_ADMIN_TOKEN_AUDIT_SECRET", "audit-shared-secret");
        std::env::set_var("PDS_ADMIN_TOKEN_AUDIT_SCOPES", "TakedownAdmin");
        std::env::set_var("PDS_ADMIN_TOKEN_AUDIT_NAME", "audit-team");
        // A non-secret var that should NOT be picked up.
        std::env::set_var("PDS_ADMIN_TOKEN_NOSECRET_SCOPES", "InviteAdmin");
    }

    let registry = AdminTokenRegistry::from_env();
    assert_eq!(registry.entries().len(), 2);

    let ops = registry.lookup("ops-team", "ops-shared-secret");
    assert!(ops.is_some(), "ops token must resolve");
    let ops_scopes = ops.unwrap();
    assert!(ops_scopes.contains(AdminScope::InviteAdmin));
    assert!(ops_scopes.contains(AdminScope::AccountAdmin));
    assert!(!ops_scopes.contains(AdminScope::TakedownAdmin));

    let audit = registry.lookup("audit-team", "audit-shared-secret");
    assert!(audit.is_some(), "audit token must resolve");
    assert!(audit.unwrap().contains(AdminScope::TakedownAdmin));

    // Cleanup.
    clear_test_env();
}

#[test]
fn admin_token_registry_resolves_legacy_admin() {
    let _guard = lock_env();
    clear_test_env();
    // SAFETY: serialised by ENV_LOCK above.
    unsafe {
        std::env::set_var("PDS_ADMIN_PASSWORD", "legacy-pw");
    }
    let registry = AdminTokenRegistry::from_env();
    assert_eq!(registry.entries().len(), 1);
    let scopes = registry.lookup("admin", "legacy-pw");
    assert!(scopes.is_some(), "legacy admin must resolve");
    assert!(scopes.unwrap().contains(AdminScope::Wildcard));

    // SAFETY: serialised by ENV_LOCK above.
    unsafe {
        std::env::remove_var("PDS_ADMIN_PASSWORD");
    }
}

#[test]
fn admin_token_registry_lookup_returns_none_for_invalid() {
    let _guard = lock_env();
    clear_test_env();
    // SAFETY: serialised by ENV_LOCK above.
    unsafe {
        std::env::set_var("PDS_ADMIN_TOKEN_OPS_SECRET", "correct-secret");
        std::env::set_var("PDS_ADMIN_TOKEN_OPS_SCOPES", "InviteAdmin");
        std::env::set_var("PDS_ADMIN_TOKEN_OPS_NAME", "ops-team");
    }

    let registry = AdminTokenRegistry::from_env();
    assert!(registry.lookup("ops-team", "wrong-secret").is_none());
    assert!(registry.lookup("unknown", "correct-secret").is_none());
    // Mismatched lengths and contents still rejected.
    assert!(registry.lookup("ops-team", "").is_none());

    // SAFETY: serialised by ENV_LOCK above.
    clear_test_env();
}

#[test]
fn cached_admin_token_registry_resets_after_reset() {
    let _guard = lock_env();
    clear_test_env();
    reset_admin_token_registry();
    // SAFETY: serialised by ENV_LOCK above.
    unsafe {
        std::env::set_var("PDS_ADMIN_TOKEN_CACHE_SECRET", "first-secret");
        std::env::set_var("PDS_ADMIN_TOKEN_CACHE_NAME", "cache");
    }
    let registry = cached_admin_token_registry();
    assert!(registry.lookup("cache", "first-secret").is_some());

    // SAFETY: serialised by ENV_LOCK above.
    unsafe {
        std::env::remove_var("PDS_ADMIN_TOKEN_CACHE_SECRET");
        std::env::set_var("PDS_ADMIN_TOKEN_CACHE_SECRET", "second-secret");
    }
    reset_admin_token_registry();
    let registry = cached_admin_token_registry();
    assert!(registry.lookup("cache", "first-secret").is_none());
    assert!(registry.lookup("cache", "second-secret").is_some());

    clear_test_env();
    reset_admin_token_registry();
}

#[test]
fn admin_scope_set_from_env_value_skips_unknown() {
    let scopes = AdminScopeSet::from_env_value("InviteAdmin, mystery, TakedownAdmin");
    assert!(scopes.contains(AdminScope::InviteAdmin));
    assert!(scopes.contains(AdminScope::TakedownAdmin));
    assert!(!scopes.contains(AdminScope::AccountAdmin));
}
