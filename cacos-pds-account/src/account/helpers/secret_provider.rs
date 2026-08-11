//! Pluggable secret backend behind [`SecretProvider`].
//!
//! [`crate::account::helpers::secrets::read_secret`] delegates every read to
//! the process-wide provider selected by `PDS_SECRET_BACKEND`:
//!
//! | `PDS_SECRET_BACKEND` | Provider | Notes |
//! |---|---|---|
//! | unset / `env` | [`EnvSecretProvider`] | Default. `FOO` or `FOO_FILE` indirection. |
//! | `file:<dir>` | [`FileSecretProvider`] | Reads `<dir>/<NAME>`. |
//! | `kms` | [`KmsSecretProvider`] | Currently refuses every read (see below). |
//!
//! The provider is cached behind a `OnceLock<Mutex<Option<_>>>`; tests can
//! swap backends at runtime via [`reset_provider`], mirroring the
//! [`crate::account::helpers::admin_tokens`] registry idiom.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

/// Why a secret could not be produced.
#[derive(Debug)]
pub enum SecretError {
    /// The secret is not configured. Carries the secret name.
    NotFound(String),
    /// The secret is configured but is not the expected hex encoding.
    InvalidHex(String),
    /// The KMS backend was selected but cannot serve reads.
    KmsUnavailable(String),
    /// Backing store I/O failed. `.0` describes the source that failed
    /// (a secret name, or `NAME_FILE=<path>`) so operators can locate it.
    Io(String, std::io::Error),
}

impl std::fmt::Display for SecretError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // Keeps the pre-trait wording so callers (and the operator
            // runbook) still see which env vars satisfy the secret.
            Self::NotFound(name) => write!(f, "{name} (or {name}_FILE) must be set"),
            Self::InvalidHex(name) => write!(f, "secret not valid hex: {name}"),
            Self::KmsUnavailable(msg) => write!(f, "KMS unavailable: {msg}"),
            Self::Io(source, err) => write!(f, "failed to read secret {source}: {err}"),
        }
    }
}

impl std::error::Error for SecretError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(_, err) => Some(err),
            _ => None,
        }
    }
}

/// A backend that can produce secret material by name.
pub trait SecretProvider: Send + Sync {
    /// Reads the secret `name`, with any trailing newline stripped.
    fn read(&self, name: &str) -> Result<String, SecretError>;

    /// Reads `name` and decodes it as exactly 32 bytes of hex.
    fn read_hex32(&self, name: &str) -> Result<[u8; 32], SecretError> {
        let value = self.read(name)?;
        let bytes = hex::decode(&value).map_err(|_| SecretError::InvalidHex(name.to_string()))?;
        <[u8; 32]>::try_from(bytes).map_err(|_| SecretError::InvalidHex(name.to_string()))
    }

    /// Forward-compatibility hook for envelope-encrypted secrets. The
    /// default ignores `kms_key_id` and defers to [`SecretProvider::read`];
    /// a KMS-backed provider overrides it to decrypt under that key.
    fn read_with_kms(&self, name: &str, _kms_key_id: Option<&str>) -> Result<String, SecretError> {
        self.read(name)
    }
}

/// Default backend: `NAME_FILE` (path indirection) takes precedence over
/// `NAME`, matching the Docker/Kubernetes/Vault secret-mount convention.
pub struct EnvSecretProvider;

impl SecretProvider for EnvSecretProvider {
    fn read(&self, name: &str) -> Result<String, SecretError> {
        let file_var = format!("{name}_FILE");
        if let Ok(path) = std::env::var(&file_var) {
            let value = std::fs::read_to_string(&path)
                .map_err(|e| SecretError::Io(format!("{file_var}={path}"), e))?;
            return Ok(trim_trailing_newline(&value));
        }
        std::env::var(name)
            .map(|v| v.to_string())
            .map_err(|_| SecretError::NotFound(name.to_string()))
    }
}

/// Reads each secret from `<root>/<NAME>`.
pub struct FileSecretProvider {
    pub root: PathBuf,
}

impl SecretProvider for FileSecretProvider {
    fn read(&self, name: &str) -> Result<String, SecretError> {
        // `name` is an internal constant today, but refuse anything that
        // could escape `root` so a future caller-supplied name cannot
        // traverse out of the secret directory.
        if !is_plain_component(name) {
            return Err(SecretError::NotFound(name.to_string()));
        }
        let path = self.root.join(name);
        let value = std::fs::read_to_string(&path)
            .map_err(|e| SecretError::Io(format!("{}", path.display()), e))?;
        Ok(trim_trailing_newline(&value))
    }
}

/// Placeholder for a KMS/Vault backend. Every read fails loudly: silently
/// falling back to the environment would defeat the point of selecting a
/// managed backend.
pub struct KmsSecretProvider {
    pub config: String,
}

impl SecretProvider for KmsSecretProvider {
    fn read(&self, name: &str) -> Result<String, SecretError> {
        Err(SecretError::KmsUnavailable(format!(
            "KMS backend not configured (PDS_SECRET_BACKEND=kms); requires a \
             follow-up change that wires an AWS/GCP/Vault client. Refusing to \
             fall back to env for {name}."
        )))
    }
}

/// Secret values are frequently mounted as files ending in a newline; strip
/// only the trailing EOL so values with meaningful inner whitespace survive.
fn trim_trailing_newline(value: &str) -> String {
    value.trim_end_matches(['\n', '\r']).to_string()
}

/// True when `name` is a single path component (no separators, no `..`).
fn is_plain_component(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\'])
        && !name.contains('\0')
}

static PROVIDER: OnceLock<Mutex<Option<Arc<dyn SecretProvider>>>> = OnceLock::new();

fn provider_slot() -> &'static Mutex<Option<Arc<dyn SecretProvider>>> {
    PROVIDER.get_or_init(|| Mutex::new(None))
}

/// Builds the backend named by `PDS_SECRET_BACKEND`.
fn build_from_env() -> Result<Arc<dyn SecretProvider>, String> {
    let backend = std::env::var("PDS_SECRET_BACKEND").unwrap_or_else(|_| "env".to_string());
    if let Some(dir) = backend.strip_prefix("file:") {
        return Ok(Arc::new(FileSecretProvider {
            root: PathBuf::from(dir),
        }));
    }
    match backend.as_str() {
        "env" => Ok(Arc::new(EnvSecretProvider)),
        "kms" => {
            let config = std::env::var("PDS_KMS_CONFIG").unwrap_or_default();
            tracing::warn!(
                kms_config = %config,
                "PDS_SECRET_BACKEND=kms is selected; KmsSecretProvider refuses every read until a real KMS client (AWS/GCP/Vault) is wired. See ADR-0012."
            );
            Ok(Arc::new(KmsSecretProvider { config }))
        }
        other => Err(format!("unknown PDS_SECRET_BACKEND: {other}")),
    }
}

/// Returns the process-wide secret provider, building it on first use.
///
/// A misconfigured `PDS_SECRET_BACKEND` is *not* cached, so fixing the env
/// and retrying works without a restart.
pub fn provider() -> Result<Arc<dyn SecretProvider>, String> {
    let mut guard = provider_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(existing) = guard.as_ref() {
        return Ok(Arc::clone(existing));
    }
    let built = build_from_env()?;
    *guard = Some(Arc::clone(&built));
    Ok(built)
}

/// Test-only: drop the cached provider so the next [`provider`] call
/// rebuilds it from the (possibly mutated) environment.
pub fn reset_provider() {
    if let Some(slot) = PROVIDER.get() {
        let mut guard = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kms_provider_refuses_every_read() {
        let provider = KmsSecretProvider {
            config: String::new(),
        };
        let err = provider
            .read("PDS_UNIT_KMS")
            .expect_err("kms backend must not serve reads");
        assert!(matches!(err, SecretError::KmsUnavailable(_)));
        assert!(err.to_string().contains("PDS_UNIT_KMS"));
    }

    #[test]
    fn file_provider_rejects_path_traversal() {
        let provider = FileSecretProvider {
            root: PathBuf::from("/var/run/secrets"),
        };
        for name in ["../escape", "nested/name", ".."] {
            assert!(
                provider.read(name).is_err(),
                "{name} must not resolve outside the secret root"
            );
        }
    }

    #[test]
    fn not_found_message_names_both_env_vars() {
        let msg = SecretError::NotFound("PDS_X".to_string()).to_string();
        assert!(msg.contains("PDS_X"));
        assert!(msg.contains("_FILE"));
    }

    #[test]
    fn kms_selection_emits_startup_warning() {
        // SAFETY: tests run sequentially within a single test binary.
        unsafe { std::env::set_var("PDS_SECRET_BACKEND", "kms") };
        // SAFETY: tests run sequentially within a single test binary.
        unsafe { std::env::set_var("PDS_KMS_CONFIG", "test-config") };
        reset_provider();

        struct OwnedBufWriter {
            buf: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
        }
        impl std::io::Write for OwnedBufWriter {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                let mut g = self.buf.lock().expect("buffer lock poisoned");
                std::io::Write::write(&mut *g, buf)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let captured: std::sync::Arc<std::sync::Mutex<Vec<u8>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let make_writer = {
            let captured = std::sync::Arc::clone(&captured);
            move || OwnedBufWriter {
                buf: std::sync::Arc::clone(&captured),
            }
        };

        let subscriber = tracing_subscriber::fmt::Subscriber::builder()
            .with_writer(make_writer)
            .with_max_level(tracing::level_filters::LevelFilter::WARN)
            .with_ansi(false)
            .with_target(false)
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            build_from_env().expect("kms backend should build");
        });

        let output = String::from_utf8(captured.lock().unwrap().clone()).unwrap();
        assert!(
            output.contains("kms"),
            "warning should mention kms, got: {output}"
        );
        assert!(
            output.contains("PDS_SECRET_BACKEND"),
            "warning should mention PDS_SECRET_BACKEND, got: {output}"
        );
        assert!(
            output.contains("test-config"),
            "warning should surface PDS_KMS_CONFIG value, got: {output}"
        );

        // SAFETY: tests run sequentially within a single test binary.
        unsafe { std::env::remove_var("PDS_SECRET_BACKEND") };
        // SAFETY: tests run sequentially within a single test binary.
        unsafe { std::env::remove_var("PDS_KMS_CONFIG") };
        reset_provider();
    }
}
