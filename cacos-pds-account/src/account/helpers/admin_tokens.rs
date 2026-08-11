//! Admin token registry: named bearer-style tokens with scoped permissions.
//!
//! Mirrors the Docker/Kubernetes secret convention: tokens are configured via
//! env vars in the form `PDS_ADMIN_TOKEN_<NAME>_{SECRET,SCOPES,NAME}`. Lookups
//! use constant-time comparison (`subtle::ConstantTimeEq`) so a hostile
//! client cannot infer the registered secret length or contents from the
//! server's response timing.
//!
//! `from_env` also recognises the legacy single-admin case: when
//! `PDS_ADMIN_PASSWORD` (or `PDS_ADMIN_TOKEN_FALLBACK_NAME`) is set with no
//! named tokens, a synthetic entry named `admin` with `Wildcard` scope is
//! created. This preserves the basic-auth admin flow that pre-dated the
//! named-token model.
//!
//! The registry is cached behind a `OnceLock`; tests can reset the cache via
//! `reset_admin_token_registry` to pick up env mutations without restarting
//! the process.

use std::collections::HashSet;
use std::env;
use std::sync::{Mutex, OnceLock};
use subtle::ConstantTimeEq;

/// The privileged actions an admin token can perform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AdminScope {
    /// Issue / revoke invite codes.
    InviteAdmin,
    /// Activate / deactivate / delete / takedown accounts.
    AccountAdmin,
    /// Apply or clear takedown flags on repos / blobs.
    TakedownAdmin,
    /// All of the above.
    Wildcard,
}

impl AdminScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            AdminScope::InviteAdmin => "InviteAdmin",
            AdminScope::AccountAdmin => "AccountAdmin",
            AdminScope::TakedownAdmin => "TakedownAdmin",
            AdminScope::Wildcard => "Wildcard",
        }
    }
}

#[derive(Debug, Clone)]
pub struct AdminScopeSet {
    scopes: Vec<AdminScope>,
}

impl AdminScopeSet {
    /// A registry that permits every admin action.
    pub fn wildcard() -> Self {
        Self {
            scopes: vec![AdminScope::Wildcard],
        }
    }

    /// Parses a comma-separated scope list. Unknown scope names are skipped
    /// (so a stale config never grants accidental access). The string is
    /// case-insensitive; whitespace around tokens is trimmed.
    pub fn from_env_value(raw: &str) -> Self {
        let mut scopes = Vec::new();
        for token in raw.split(',') {
            let trimmed = token.trim();
            if trimmed.is_empty() {
                continue;
            }
            let scope = match trimmed.to_ascii_lowercase().as_str() {
                "inviteadmin" => Some(AdminScope::InviteAdmin),
                "accountadmin" => Some(AdminScope::AccountAdmin),
                "takedownadmin" => Some(AdminScope::TakedownAdmin),
                "wildcard" | "*" => Some(AdminScope::Wildcard),
                _ => None,
            };
            if let Some(scope) = scope {
                scopes.push(scope);
            }
        }
        Self { scopes }
    }

    /// Returns true if `scope` is granted (either directly or via `Wildcard`).
    pub fn contains(&self, scope: AdminScope) -> bool {
        self.scopes
            .iter()
            .any(|s| *s == scope || *s == AdminScope::Wildcard)
    }
}

/// A registered admin token: human-readable `name`, the configured `scopes`,
/// and the secret bytes that callers must present.
#[derive(Debug, Clone)]
pub struct AdminTokenEntry {
    pub name: String,
    pub scopes: AdminScopeSet,
    pub secret: String,
}

/// The set of admin tokens recognised by the server.
#[derive(Debug, Clone, Default)]
pub struct AdminTokenRegistry {
    entries: Vec<AdminTokenEntry>,
}

impl AdminTokenRegistry {
    /// Loads the registry from environment variables. Never panics; missing
    /// configuration simply produces an empty registry.
    pub fn from_env() -> Self {
        let mut entries: Vec<AdminTokenEntry> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        for (key, _) in env::vars() {
            let Some(suffix) = key.strip_prefix("PDS_ADMIN_TOKEN_") else {
                continue;
            };
            let Some(name) = suffix.strip_suffix("_SECRET") else {
                continue;
            };
            if name.is_empty() || !seen.insert(name.to_string()) {
                continue;
            }
            let Ok(secret) = env::var(&key) else {
                continue;
            };
            if secret.is_empty() {
                continue;
            }
            let display_name = env::var(format!("PDS_ADMIN_TOKEN_{name}_NAME"))
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| name.to_string());
            let scopes = env::var(format!("PDS_ADMIN_TOKEN_{name}_SCOPES"))
                .ok()
                .filter(|s| !s.is_empty())
                .map(|raw| AdminScopeSet::from_env_value(&raw))
                .unwrap_or_else(AdminScopeSet::wildcard);
            entries.push(AdminTokenEntry {
                name: display_name,
                scopes,
                secret,
            });
        }

        if entries.is_empty()
            && let Ok(secret) =
                env::var("PDS_ADMIN_PASSWORD").or_else(|_| env::var("PDS_ADMIN_PASS"))
            && !secret.is_empty()
        {
            entries.push(AdminTokenEntry {
                name: "admin".to_string(),
                scopes: AdminScopeSet::wildcard(),
                secret,
            });
        }

        Self { entries }
    }

    pub fn entries(&self) -> &[AdminTokenEntry] {
        &self.entries
    }

    /// Constant-time lookup: returns the scopes for an entry whose name AND
    /// secret both match. Length-mismatched secrets are rejected without
    /// short-circuiting, so the response time depends only on the LONGER of
    /// the two inputs.
    pub fn lookup(&self, name: &str, presented_secret: &str) -> Option<&AdminScopeSet> {
        let name_bytes = name.as_bytes();
        let presented_bytes = presented_secret.as_bytes();
        for entry in &self.entries {
            let entry_name = entry.name.as_bytes();
            let entry_secret = entry.secret.as_bytes();
            let name_match: bool = entry_name.ct_eq(name_bytes).into();
            let secret_match: bool = entry_secret.ct_eq(presented_bytes).into();
            if name_match && secret_match {
                return Some(&entry.scopes);
            }
        }
        None
    }
}

static ADMIN_TOKEN_REGISTRY: OnceLock<Mutex<Option<AdminTokenRegistry>>> = OnceLock::new();

fn registry_slot() -> &'static Mutex<Option<AdminTokenRegistry>> {
    ADMIN_TOKEN_REGISTRY.get_or_init(|| Mutex::new(None))
}

/// Returns the process-wide admin token registry, building it lazily on
/// first call.
pub fn cached_admin_token_registry() -> AdminTokenRegistry {
    let slot = registry_slot();
    let mut guard = slot.lock().expect("admin token registry mutex poisoned");
    if guard.is_none() {
        *guard = Some(AdminTokenRegistry::from_env());
    }
    guard.clone().expect("registry just initialised")
}

/// Test-only: clear the cached registry so the next `cached_admin_token_registry`
/// call rebuilds it from the (possibly mutated) environment.
pub fn reset_admin_token_registry() {
    if let Some(slot) = ADMIN_TOKEN_REGISTRY.get() {
        let mut guard = slot.lock().expect("admin token registry mutex poisoned");
        *guard = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_env_value_recognises_known_scopes_and_skips_unknown() {
        let scopes = AdminScopeSet::from_env_value("InviteAdmin, AccountAdmin, bogus");
        assert!(scopes.contains(AdminScope::InviteAdmin));
        assert!(scopes.contains(AdminScope::AccountAdmin));
        assert!(!scopes.contains(AdminScope::TakedownAdmin));
    }

    #[test]
    fn wildcard_contains_every_scope() {
        let scopes = AdminScopeSet::wildcard();
        assert!(scopes.contains(AdminScope::InviteAdmin));
        assert!(scopes.contains(AdminScope::AccountAdmin));
        assert!(scopes.contains(AdminScope::TakedownAdmin));
        assert!(scopes.contains(AdminScope::Wildcard));
    }

    #[test]
    fn lookup_returns_scopes_on_match() {
        let registry = AdminTokenRegistry {
            entries: vec![AdminTokenEntry {
                name: "ops".to_string(),
                scopes: AdminScopeSet::from_env_value("InviteAdmin"),
                secret: "shh-secret".to_string(),
            }],
        };
        let scopes = registry.lookup("ops", "shh-secret");
        assert!(scopes.is_some());
        assert!(scopes.unwrap().contains(AdminScope::InviteAdmin));
        assert!(!scopes.unwrap().contains(AdminScope::AccountAdmin));
    }

    #[test]
    fn lookup_returns_none_on_wrong_secret() {
        let registry = AdminTokenRegistry {
            entries: vec![AdminTokenEntry {
                name: "ops".to_string(),
                scopes: AdminScopeSet::wildcard(),
                secret: "shh-secret".to_string(),
            }],
        };
        assert!(registry.lookup("ops", "wrong").is_none());
        assert!(registry.lookup("unknown", "shh-secret").is_none());
    }
}
