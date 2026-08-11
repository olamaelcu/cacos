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

use cacos_pds_account::account::helpers::admin_tokens::{
    AdminScope, AdminScopeSet, AdminTokenRegistry, cached_admin_token_registry,
    reset_admin_token_registry,
};
use cacos_pds_account::account::helpers::secret_provider::{
    EnvSecretProvider, FileSecretProvider, KmsSecretProvider, SecretError, SecretProvider,
    reset_provider,
};
use cacos_pds_account::account::helpers::secrets::{
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
        // Restore the default (`env`) secret backend so a provider test
        // cannot leak its backend selection into the rest of the binary.
        std::env::remove_var("PDS_SECRET_BACKEND");
        std::env::remove_var("PDS_KMS_CONFIG");
        // Clear the secrets-test fixture env vars.
        for var in [
            "PDS_TEST_SECRET_FILE_ONLY",
            "PDS_TEST_SECRET_FILE_ONLY_FILE",
            "PDS_TEST_SECRET_ENV_ONLY",
            "PDS_TEST_SECRET_NEITHER",
            "PDS_TEST_SECRET_NEITHER_FILE",
            "PDS_TEST_SECRET_TRIM",
            "PDS_TEST_SECRET_TRIM_FILE",
            "PDS_TEST_PROVIDER_ENV",
            "PDS_TEST_PROVIDER_SWAP",
            "PDS_TEST_PROVIDER_HEX",
        ] {
            std::env::remove_var(var);
            std::env::remove_var(format!("{var}_FILE"));
        }
    }
    // The cached provider is rebuilt from the environment on next use.
    reset_provider();
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

// ---------------------------------------------------------------------------
// R11: SecretProvider backends.
//
// `read_secret` now delegates to the backend selected by
// `PDS_SECRET_BACKEND`. These tests exercise each impl directly and then
// the process-wide provider swap.
// ---------------------------------------------------------------------------

#[test]
fn env_provider_reads_env_and_file_fallback() {
    let _guard = lock_env();
    clear_test_env();
    let provider = EnvSecretProvider;

    // Plain env var.
    // SAFETY: serialised by ENV_LOCK above.
    unsafe {
        std::env::set_var("PDS_TEST_PROVIDER_ENV", "env-value");
    }
    assert_eq!(provider.read("PDS_TEST_PROVIDER_ENV").unwrap(), "env-value");

    // `_FILE` indirection wins over the plain var, and the trailing EOL is
    // stripped (secret files are conventionally newline-terminated).
    let dir = camino_tempfile::Utf8TempDir::new().unwrap();
    let path = dir.path().join("value.txt");
    std::fs::write(&path, "file-value\n").unwrap();
    // SAFETY: serialised by ENV_LOCK above.
    unsafe {
        std::env::set_var("PDS_TEST_PROVIDER_ENV_FILE", path.as_str());
    }
    assert_eq!(
        provider.read("PDS_TEST_PROVIDER_ENV").unwrap(),
        "file-value"
    );

    // Neither set -> NotFound naming both accepted vars.
    let err = provider.read("PDS_TEST_PROVIDER_MISSING").unwrap_err();
    assert!(matches!(err, SecretError::NotFound(_)));
    let msg = err.to_string();
    assert!(msg.contains("PDS_TEST_PROVIDER_MISSING"), "got: {msg}");
    assert!(msg.contains("_FILE"), "got: {msg}");

    clear_test_env();
}

#[test]
fn file_provider_reads_root_slash_name() {
    let dir = camino_tempfile::Utf8TempDir::new().unwrap();
    std::fs::write(dir.path().join("PDS_TEST_FILE_SECRET"), "on-disk\n").unwrap();
    let provider = FileSecretProvider {
        root: std::path::PathBuf::from(dir.path().as_str()),
    };

    assert_eq!(provider.read("PDS_TEST_FILE_SECRET").unwrap(), "on-disk");

    // Missing file surfaces the I/O failure with the path attached.
    let err = provider.read("PDS_TEST_ABSENT").unwrap_err();
    assert!(matches!(err, SecretError::Io(_, _)));
    assert!(err.to_string().contains("PDS_TEST_ABSENT"));

    // A name that is not a single path component must not escape the root.
    assert!(provider.read("../outside").is_err());
}

#[test]
fn kms_provider_returns_unavailable_error() {
    let provider = KmsSecretProvider {
        config: String::new(),
    };
    let err = provider.read("PDS_TEST_KMS_SECRET").unwrap_err();
    assert!(matches!(err, SecretError::KmsUnavailable(_)));
    let msg = err.to_string();
    assert!(msg.contains("KMS"), "got: {msg}");
    // Must state that it refuses to silently fall back to the environment.
    assert!(msg.contains("PDS_TEST_KMS_SECRET"), "got: {msg}");
}

#[test]
fn read_hex32_decodes_exactly_32_bytes() {
    let _guard = lock_env();
    clear_test_env();
    let provider = EnvSecretProvider;

    // SAFETY: serialised by ENV_LOCK above.
    unsafe {
        std::env::set_var("PDS_TEST_PROVIDER_HEX", "42".repeat(32));
    }
    assert_eq!(
        provider.read_hex32("PDS_TEST_PROVIDER_HEX").unwrap(),
        [0x42; 32]
    );

    // Right hex, wrong length.
    // SAFETY: serialised by ENV_LOCK above.
    unsafe {
        std::env::set_var("PDS_TEST_PROVIDER_HEX", "42".repeat(31));
    }
    assert!(matches!(
        provider.read_hex32("PDS_TEST_PROVIDER_HEX").unwrap_err(),
        SecretError::InvalidHex(_)
    ));

    // Not hex at all.
    // SAFETY: serialised by ENV_LOCK above.
    unsafe {
        std::env::set_var("PDS_TEST_PROVIDER_HEX", "zz");
    }
    assert!(matches!(
        provider.read_hex32("PDS_TEST_PROVIDER_HEX").unwrap_err(),
        SecretError::InvalidHex(_)
    ));

    clear_test_env();
}

#[test]
fn provider_can_be_swapped_at_runtime() {
    let _guard = lock_env();
    clear_test_env();

    // Default/explicit `env` backend.
    // SAFETY: serialised by ENV_LOCK above.
    unsafe {
        std::env::set_var("PDS_SECRET_BACKEND", "env");
        std::env::set_var("PDS_TEST_PROVIDER_SWAP", "env-value");
    }
    reset_provider();
    assert_eq!(read_secret("PDS_TEST_PROVIDER_SWAP").unwrap(), "env-value");

    // `file:<dir>` backend reads from disk and ignores the env var that is
    // still set — proving the swap actually took effect.
    let dir = camino_tempfile::Utf8TempDir::new().unwrap();
    std::fs::write(dir.path().join("PDS_TEST_PROVIDER_SWAP"), "file-value\n").unwrap();
    // SAFETY: serialised by ENV_LOCK above.
    unsafe {
        std::env::set_var("PDS_SECRET_BACKEND", format!("file:{}", dir.path()));
    }
    reset_provider();
    assert_eq!(read_secret("PDS_TEST_PROVIDER_SWAP").unwrap(), "file-value");

    // `kms` backend refuses rather than falling back to env.
    // SAFETY: serialised by ENV_LOCK above.
    unsafe {
        std::env::set_var("PDS_SECRET_BACKEND", "kms");
    }
    reset_provider();
    let err = read_secret("PDS_TEST_PROVIDER_SWAP").unwrap_err();
    assert!(err.contains("KMS"), "got: {err}");

    // An unknown backend is a hard error, not a silent fallback to env.
    // SAFETY: serialised by ENV_LOCK above.
    unsafe {
        std::env::set_var("PDS_SECRET_BACKEND", "totally-bogus");
    }
    reset_provider();
    let err = read_secret("PDS_TEST_PROVIDER_SWAP").unwrap_err();
    assert!(err.contains("unknown PDS_SECRET_BACKEND"), "got: {err}");

    // A bad backend is not cached: correcting the env recovers without a
    // restart.
    // SAFETY: serialised by ENV_LOCK above.
    unsafe {
        std::env::remove_var("PDS_SECRET_BACKEND");
    }
    assert_eq!(read_secret("PDS_TEST_PROVIDER_SWAP").unwrap(), "env-value");

    clear_test_env();
}

// ---------------------------------------------------------------------------
// R7: Admin scope gating.
//
// The `RequireInviteAdmin` / `RequireAccountAdmin` / `RequireTakedownAdmin`
// extractors in `pds::xrpc::auth_extractors` compose an [`AdminToken`]
// check with a scope gate on `Credentials.admin_scopes`, which is populated
// by `auth_verifier::verify_admin_token` from the [`AdminTokenRegistry`]
// built via `PDS_ADMIN_TOKEN_<NAME>_*` env vars.
//
// The tests spin up a minimal Route so we exercise the actual poem
// extractor wiring — no `SharedState` needed because the admin extractors
// only read the request header.
// ---------------------------------------------------------------------------

mod admin_scope_extractors {
    use super::{AdminScope, clear_test_env, lock_env};
    use base64ct::Encoding;
    use cacos_pds_account::auth::verifier::verify_admin_token;
    use cacos_pds::xrpc::auth_extractors::{
        RequireAccountAdmin, RequireInviteAdmin, RequireTakedownAdmin,
    };
    use poem::Route;
    use poem::http::StatusCode;
    use poem::test::TestClient;
    use poem::web::Json;
    use serde_json::json;

    fn basic_auth_header(name: &str, secret: &str) -> String {
        let encoded = base64ct::Base64::encode_string(format!("{name}:{secret}").as_bytes());
        format!("Basic {encoded}")
    }

    #[poem::handler]
    async fn needs_invite_admin(_auth: RequireInviteAdmin) -> Json<serde_json::Value> {
        Json(json!({ "ok": true }))
    }

    #[poem::handler]
    async fn needs_account_admin(_auth: RequireAccountAdmin) -> Json<serde_json::Value> {
        Json(json!({ "ok": true }))
    }

    #[poem::handler]
    async fn needs_takedown_admin(_auth: RequireTakedownAdmin) -> Json<serde_json::Value> {
        Json(json!({ "ok": true }))
    }

    fn app() -> poem::Route {
        Route::new()
            .at("/invite", poem::get(needs_invite_admin))
            .at("/account", poem::get(needs_account_admin))
            .at("/takedown", poem::get(needs_takedown_admin))
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn require_invite_admin_accepts_token_with_scope() {
        let _guard = lock_env();
        clear_test_env();
        // SAFETY: serialised by ENV_LOCK above.
        unsafe {
            std::env::set_var("PDS_ADMIN_TOKEN_OPS_SECRET", "ops-shared-secret");
            std::env::set_var("PDS_ADMIN_TOKEN_OPS_SCOPES", "InviteAdmin");
            std::env::set_var("PDS_ADMIN_TOKEN_OPS_NAME", "ops-team");
        }

        let access = verify_admin_token(Some(&basic_auth_header("ops-team", "ops-shared-secret")))
            .await
            .unwrap();
        let creds = access.credentials.as_ref().unwrap();
        assert!(
            creds
                .admin_scopes
                .as_ref()
                .unwrap()
                .contains(AdminScope::InviteAdmin),
            "registry should grant InviteAdmin to the ops token"
        );

        let cli = TestClient::new(app());
        let resp = cli
            .get("/invite")
            .header(
                "Authorization",
                basic_auth_header("ops-team", "ops-shared-secret"),
            )
            .send()
            .await;
        assert_eq!(resp.0.status(), StatusCode::OK);

        clear_test_env();
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn require_invite_admin_rejects_token_without_scope() {
        let _guard = lock_env();
        clear_test_env();
        // SAFETY: serialised by ENV_LOCK above.
        unsafe {
            // Token is granted only AccountAdmin; InviteAdmin must be refused.
            std::env::set_var("PDS_ADMIN_TOKEN_OPS_SECRET", "ops-shared-secret");
            std::env::set_var("PDS_ADMIN_TOKEN_OPS_SCOPES", "AccountAdmin");
            std::env::set_var("PDS_ADMIN_TOKEN_OPS_NAME", "ops-team");
        }

        let cli = TestClient::new(app());
        let resp = cli
            .get("/invite")
            .header(
                "Authorization",
                basic_auth_header("ops-team", "ops-shared-secret"),
            )
            .send()
            .await;
        assert_eq!(resp.0.status(), StatusCode::UNAUTHORIZED);

        let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
        assert_eq!(body["error"], "AuthRequiredError");
        assert!(
            body["message"]
                .as_str()
                .unwrap_or_default()
                .contains("InviteAdmin"),
            "expected scope hint in message, got: {body}"
        );

        // Sanity: the same token IS sufficient for AccountAdmin.
        let resp = cli
            .get("/account")
            .header(
                "Authorization",
                basic_auth_header("ops-team", "ops-shared-secret"),
            )
            .send()
            .await;
        assert_eq!(resp.0.status(), StatusCode::OK);

        clear_test_env();
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn require_account_admin_accepts_token_with_scope() {
        let _guard = lock_env();
        clear_test_env();
        // SAFETY: serialised by ENV_LOCK above.
        unsafe {
            std::env::set_var("PDS_ADMIN_TOKEN_OPS_SECRET", "ops-shared-secret");
            std::env::set_var("PDS_ADMIN_TOKEN_OPS_SCOPES", "AccountAdmin");
            std::env::set_var("PDS_ADMIN_TOKEN_OPS_NAME", "ops-team");
        }

        let cli = TestClient::new(app());
        let resp = cli
            .get("/account")
            .header(
                "Authorization",
                basic_auth_header("ops-team", "ops-shared-secret"),
            )
            .send()
            .await;
        assert_eq!(resp.0.status(), StatusCode::OK);

        clear_test_env();
    }

    #[test]
    fn verify_admin_token_propagates_registry_scopes() {
        // Pure unit assertion (no poem roundtrip) that the
        // `Credentials.admin_scopes` field is populated for a token whose
        // scope set is narrower than Wildcard — the field is the wire that
        // the RequireInviteAdmin / RequireAccountAdmin / RequireTakedownAdmin
        // extractors read.
        let _guard = lock_env();
        clear_test_env();
        // SAFETY: serialised by ENV_LOCK above.
        unsafe {
            std::env::set_var("PDS_ADMIN_TOKEN_NARROW_SECRET", "narrow-secret");
            std::env::set_var("PDS_ADMIN_TOKEN_NARROW_SCOPES", "InviteAdmin");
            std::env::set_var("PDS_ADMIN_TOKEN_NARROW_NAME", "narrow");
        }
        let header = basic_auth_header("narrow", "narrow-secret");
        let access = futures::executor::block_on(verify_admin_token(Some(&header))).unwrap();
        let creds = access.credentials.unwrap();
        assert_eq!(creds.r#type, "admin_token");
        let scopes = creds.admin_scopes.expect("admin_scopes must be populated");
        assert!(scopes.contains(AdminScope::InviteAdmin));
        assert!(!scopes.contains(AdminScope::AccountAdmin));
        assert!(!scopes.contains(AdminScope::TakedownAdmin));
        clear_test_env();
    }
}

// ---------------------------------------------------------------------------
// R3: `init_required_keys` fail-fast boot-time validation.
//
// Every test restores the env vars it touches so the rest of the binary
// (which shares the same process) sees the same starting state.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod init_required_keys_tests {
    use super::{lock_env, strong_password};
    use cacos_pds_account::account::helpers::init_required_keys::init_required_keys;

    // 32-byte hex -> 64 chars
    const KEY_HEX: &str = "9d5907143471e8f0e8df0f8b9512a8c5377878ee767f18fcf961055ecfc071cd";
    const REPO_KEY_HEX: &str = "71cfcf4882a6cff494c3d0affadd3858eb3a5838e7b5e15170e696a590a4fa01";
    const PLC_KEY_HEX: &str = "e7763b0a2d1a4f8e9f9d2d6f3f9a5d0c6b2e4a8c1f7d3e9b5a0c2f4d6e8a0b1c";

    fn valid_env() {
        // SAFETY: serialised by ENV_LOCK in the calling test.
        unsafe {
            std::env::set_var("PDS_JWT_KEY_K256_PRIVATE_KEY_HEX", KEY_HEX);
            std::env::set_var("PDS_ACCESS_JWT_KEY_K256_PRIVATE_KEY_HEX", KEY_HEX);
            std::env::set_var("PDS_OAUTH_SIGNING_KEY_K256_PRIVATE_KEY_HEX", KEY_HEX);
            std::env::set_var("PDS_REPO_SIGNING_KEY_K256_PRIVATE_KEY_HEX", REPO_KEY_HEX);
            std::env::set_var("PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX", PLC_KEY_HEX);
            std::env::set_var("PDS_DPOP_SECRET", KEY_HEX);
            std::env::set_var("PDS_OAUTH_TRUSTED_CLIENTS", "");
            std::env::set_var("PDS_ADMIN_PASSWORD", strong_password());
        }
    }

    fn wipe_bootstrap_env() {
        // SAFETY: serialised by ENV_LOCK in the calling test.
        unsafe {
            for var in [
                "PDS_JWT_KEY_K256_PRIVATE_KEY_HEX",
                "PDS_ACCESS_JWT_KEY_K256_PRIVATE_KEY_HEX",
                "PDS_OAUTH_SIGNING_KEY_K256_PRIVATE_KEY_HEX",
                "PDS_REPO_SIGNING_KEY_K256_PRIVATE_KEY_HEX",
                "PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX",
                "PDS_DPOP_SECRET",
                "PDS_OAUTH_TRUSTED_CLIENTS",
                "PDS_ADMIN_PASSWORD",
            ] {
                std::env::remove_var(var);
            }
        }
    }

    #[test]
    fn init_required_keys_succeeds_with_valid_config() {
        let _guard = lock_env();
        wipe_bootstrap_env();
        valid_env();
        init_required_keys().expect("a fully populated configuration must succeed");
        // Restore (best-effort; the binary ends after this).
        wipe_bootstrap_env();
    }

    #[test]
    fn init_required_keys_errors_when_access_jwt_key_missing() {
        let _guard = lock_env();
        wipe_bootstrap_env();
        // SAFETY: serialised by ENV_LOCK above.
        unsafe {
            std::env::remove_var("PDS_JWT_KEY_K256_PRIVATE_KEY_HEX");
            std::env::remove_var("PDS_ACCESS_JWT_KEY_K256_PRIVATE_KEY_HEX");
        }
        let err = init_required_keys().expect_err("missing access JWT key must fail");
        assert!(
            err.contains("PDS_ACCESS_JWT_KEY_K256_PRIVATE_KEY_HEX"),
            "error must name the missing variable, got: {err}"
        );
        wipe_bootstrap_env();
    }

    #[test]
    fn init_required_keys_errors_when_dpop_secret_wrong_length() {
        let _guard = lock_env();
        wipe_bootstrap_env();
        valid_env();
        // 32 chars but not hex; the length check fires first so we expect
        // a message about length rather than the hex validator.
        // SAFETY: serialised by ENV_LOCK above.
        unsafe {
            std::env::set_var("PDS_DPOP_SECRET", "a".repeat(32));
        }
        let err =
            init_required_keys().expect_err("32-char non-hex DPoP secret must fail length check");
        assert!(
            err.contains("PDS_DPOP_SECRET") && err.contains("32-byte hex"),
            "error must describe the DPoP secret length requirement, got: {err}"
        );
        wipe_bootstrap_env();
    }

    #[test]
    fn init_required_keys_errors_when_admin_password_missing() {
        let _guard = lock_env();
        wipe_bootstrap_env();
        valid_env();
        // SAFETY: serialised by ENV_LOCK above.
        unsafe {
            std::env::remove_var("PDS_ADMIN_PASSWORD");
        }
        let err = init_required_keys().expect_err("missing admin password must fail validation");
        assert!(
            err.contains("PDS_ADMIN_PASSWORD"),
            "error must name PDS_ADMIN_PASSWORD, got: {err}"
        );
        wipe_bootstrap_env();
    }
}
