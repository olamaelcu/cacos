//! Admin-scope / admin-token check helpers.
//!
//! Re-exports `AdminScopeSet` / `AdminTokenRegistry` for backwards
//! compatibility with the original flat module.

pub use crate::account::helpers::admin_tokens::{AdminScopeSet, AdminTokenRegistry};

use super::bearer::{AccessOutput, AuthError};
use std::env;

/// Minimal projection of `ActorAccount` used by the pure status checks.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AccountStatus {
    pub deactivated_at: Option<String>,
    pub takedown_ref: Option<String>,
}

/// Returns the configured admin password from the env, preferring the
/// upstream `PDS_ADMIN_PASSWORD` name over the legacy `PDS_ADMIN_PASS`.
pub fn admin_password_from_env() -> Option<String> {
    env::var("PDS_ADMIN_PASSWORD")
        .or_else(|_| env::var("PDS_ADMIN_PASS"))
        .ok()
}

/// Pure account-status check on the pre-fetched [`AccountStatus`].
pub fn check_account_status(
    account: Option<&AccountStatus>,
    check_takedown: bool,
    check_deactivated: bool,
) -> Result<(), AuthError> {
    if !check_takedown && !check_deactivated {
        return Ok(());
    }
    let Some(account) = account else {
        return Err(AuthError::AccountNotFound("Account not found".to_string()));
    };
    if check_takedown && account.takedown_ref.is_some() {
        return Err(AuthError::AccountTakedown(
            "Account has been taken down".to_string(),
        ));
    }
    if check_deactivated && account.deactivated_at.is_some() {
        return Err(AuthError::AccountDeactivated(
            "Account is deactivated".to_string(),
        ));
    }
    Ok(())
}

/// Returns true when the `auth` was an admin token, or when its `did`
/// matches `did`. Used by handlers that accept either the user's own did
/// or any admin token.
pub fn is_user_or_admin(auth: &AccessOutput, did: &String) -> bool {
    match &auth.credentials {
        Some(credentials) if credentials.r#type == "admin_token" => true,
        Some(credentials) => credentials.did == Some(did.to_string()),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::helpers::auth::AuthScope;
    use super::super::bearer::Credentials;

    #[test]
    fn admin_password_prefers_upstream_env_name() {
        // SAFETY: tests run sequentially within a single process; no concurrent
        // env reads.
        unsafe {
            std::env::remove_var("PDS_ADMIN_PASSWORD");
            std::env::remove_var("PDS_ADMIN_PASS");
        }
        assert_eq!(admin_password_from_env(), None);

        // SAFETY: see above.
        unsafe {
            std::env::set_var("PDS_ADMIN_PASS", "legacy");
        }
        assert_eq!(admin_password_from_env(), Some("legacy".to_string()));

        // SAFETY: see above.
        unsafe {
            std::env::set_var("PDS_ADMIN_PASSWORD", "standard");
        }
        assert_eq!(admin_password_from_env(), Some("standard".to_string()));

        // SAFETY: see above.
        unsafe {
            std::env::remove_var("PDS_ADMIN_PASSWORD");
            std::env::remove_var("PDS_ADMIN_PASS");
        }
    }

    #[test]
    fn check_account_status_flags() {
        let err = check_account_status(None, true, false).unwrap_err();
        assert!(matches!(err, AuthError::AccountNotFound(_)));
        assert!(check_account_status(None, false, false).is_ok());

        let taken_down = AccountStatus {
            deactivated_at: None,
            takedown_ref: Some("admin#1".to_string()),
        };
        let err = check_account_status(Some(&taken_down), true, false).unwrap_err();
        assert!(matches!(err, AuthError::AccountTakedown(_)));

        let deactivated = AccountStatus {
            deactivated_at: Some("2026-01-01T00:00:00.000Z".to_string()),
            takedown_ref: None,
        };
        let err = check_account_status(Some(&deactivated), false, true).unwrap_err();
        assert!(matches!(err, AuthError::AccountDeactivated(_)));
        assert!(check_account_status(Some(&deactivated), false, false).is_ok());
    }

    #[test]
    fn is_user_or_admin_matches_by_token_type() {
        let admin = AccessOutput {
            credentials: Some(Credentials {
                r#type: "admin_token".to_string(),
                did: None,
                scope: None,
                audience: None,
                token_id: None,
                aud: None,
                iss: None,
                is_privileged: None,
                admin_scopes: None,
            }),
            artifacts: None,
        };
        assert!(is_user_or_admin(&admin, &"any".to_string()));

        let user = AccessOutput {
            credentials: Some(Credentials {
                r#type: "access".to_string(),
                did: Some("did:plc:alice".to_string()),
                scope: Some(AuthScope::Access),
                audience: None,
                token_id: None,
                aud: None,
                iss: None,
                is_privileged: None,
                admin_scopes: None,
            }),
            artifacts: None,
        };
        assert!(is_user_or_admin(&user, &"did:plc:alice".to_string()));
        assert!(!is_user_or_admin(&user, &"did:plc:bob".to_string()));
        assert!(!is_user_or_admin(
            &AccessOutput {
                credentials: None,
                artifacts: None,
            },
            &"did:plc:alice".to_string(),
        ));
    }
}
