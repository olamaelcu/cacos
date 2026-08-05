//! Account management: `AccountManager` plus the account helpers under
//! `helpers/`.

pub mod helpers;
#[cfg(test)]
pub mod test_util;
#[cfg(test)]
mod tests;

pub use crate::account::helpers::account::{
    AccountStatus, ActorAccount, AvailabilityFlags, GetAccountAdminStatusOutput,
};
pub use crate::account::helpers::auth::{
    AuthHelperError, AuthScope, CreateTokensOpts, RefreshGracePeriodOpts,
};
pub use crate::account::helpers::email_token::EmailTokenPurpose;
pub use crate::account::helpers::invite::{CodeDetail, DisableInviteCodesOpts};
pub use crate::account::helpers::password::UpdateUserPasswordOpts;
pub use crate::account::helpers::repo;

use crate::account::helpers::{account, auth, email_token, invite, password};
use anyhow::Result;
use chrono::DateTime;
use lexicon_cid::Cid;
use rsky_common::RFC3339_VARIANT;
use rsky_lexicon::com::atproto::admin::StatusAttr;
use sea_orm::DatabaseConnection;
use std::env;

#[allow(dead_code)] // used by AccountManager::rotate_refresh_token (Task 7)
fn format_micros(micros: i64) -> Result<String> {
    let dt = DateTime::from_timestamp_micros(micros)
        .ok_or_else(|| anyhow::anyhow!("timestamp out of range: {micros}"))?;
    Ok(format!("{}", dt.format(RFC3339_VARIANT)))
}

/// Helps with readability when calling create_account()
pub struct CreateAccountOpts {
    pub did: String,
    pub handle: String,
    pub email: Option<String>,
    pub password: Option<String>,
    pub repo_cid: Cid,
    pub repo_rev: String,
    pub invite_code: Option<String>,
    pub deactivated: Option<bool>,
}

pub struct ConfirmEmailOpts<'em> {
    pub did: &'em String,
    pub token: &'em String,
}

pub struct ResetPasswordOpts {
    pub password: String,
    pub token: String,
}

pub struct UpdateAccountPasswordOpts {
    pub did: String,
    pub password: String,
}

pub struct UpdateEmailOpts {
    pub did: String,
    pub email: String,
}

#[derive(Clone, Debug)]
pub struct AccountManager {
    pub db: DatabaseConnection,
}

impl AccountManager {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    pub async fn get_account(
        &self,
        handle_or_did: &str,
        flags: Option<AvailabilityFlags>,
    ) -> Result<Option<ActorAccount>> {
        account::get_account(handle_or_did, flags, &self.db).await
    }

    pub async fn get_account_by_email(
        &self,
        email: &str,
        flags: Option<AvailabilityFlags>,
    ) -> Result<Option<ActorAccount>> {
        account::get_account_by_email(email, flags, &self.db).await
    }

    pub async fn is_account_activated(&self, did: &str) -> Result<bool> {
        let account = self
            .get_account(
                did,
                Some(AvailabilityFlags {
                    include_taken_down: None,
                    include_deactivated: Some(true),
                }),
            )
            .await?;
        if let Some(account) = account {
            Ok(account.deactivated_at.is_none())
        } else {
            Ok(false)
        }
    }

    pub async fn get_did_for_actor(
        &self,
        handle_or_did: &str,
        flags: Option<AvailabilityFlags>,
    ) -> Result<Option<String>> {
        match self.get_account(handle_or_did, flags).await {
            Ok(Some(got)) => Ok(Some(got.did)),
            _ => Ok(None),
        }
    }

    pub async fn create_account(&self, opts: CreateAccountOpts) -> Result<(String, String)> {
        let CreateAccountOpts {
            did,
            handle,
            email,
            password,
            repo_cid,
            repo_rev,
            invite_code,
            deactivated,
        } = opts;
        let password_encrypted: Option<String> = match password {
            Some(password) => Some(password::gen_salt_and_hash(password)?),
            None => None,
        };

        let (access_jwt, refresh_jwt) = auth::create_tokens(CreateTokensOpts {
            did: did.clone(),
            service_did: env::var("PDS_SERVICE_DID").unwrap(),
            scope: Some(AuthScope::Access),
            jti: None,
            expires_in: None,
        })?;
        let refresh_payload = auth::decode_refresh_token(refresh_jwt.clone())?;
        let now = rsky_common::now();

        if let Some(invite_code) = invite_code.clone() {
            invite::ensure_invite_is_available(invite_code, &self.db).await?;
        }
        account::register_actor(did.clone(), handle, deactivated, &self.db).await?;
        if let (Some(email), Some(password_encrypted)) = (email, password_encrypted) {
            account::register_account(did.clone(), email, password_encrypted, &self.db).await?;
        }
        invite::record_invite_use(did.clone(), invite_code.clone(), now, &self.db).await?;
        auth::store_refresh_token(refresh_payload, None, &self.db).await?;
        repo::update_root(did, repo_cid, repo_rev, &self.db).await?;

        // Metrics (Plan 02 constants; no-ops without a recorder)
        metrics::counter!(crate::observability::metrics::SIGNUPS_TOTAL).increment(1);
        if invite_code.is_some() {
            metrics::counter!(crate::observability::metrics::INVITE_USAGE_TOTAL).increment(1);
        }
        Ok((access_jwt, refresh_jwt))
    }

    pub async fn get_account_admin_status(
        &self,
        did: &str,
    ) -> Result<Option<GetAccountAdminStatusOutput>> {
        account::get_account_admin_status(did, &self.db).await
    }

    pub async fn update_repo_root(&self, did: String, cid: Cid, rev: String) -> Result<()> {
        repo::update_root(did, cid, rev, &self.db).await
    }

    pub async fn delete_account(&self, did: &str) -> Result<()> {
        account::delete_account(did, &self.db).await
    }

    pub async fn takedown_account(&self, did: &str, takedown: StatusAttr) -> Result<()> {
        let (_, _) = tokio::try_join!(
            account::update_account_takedown_status(did, takedown, &self.db),
            auth::revoke_refresh_tokens_by_did(did, &self.db)
        )?;
        Ok(())
    }

    // @NOTE should always be paired with a sequenceHandle().
    pub async fn update_handle(&self, did: &str, handle: &str) -> Result<()> {
        account::update_handle(did, handle, &self.db).await
    }

    pub async fn deactivate_account(&self, did: &str, delete_after: Option<String>) -> Result<()> {
        account::deactivate_account(did, delete_after, &self.db).await
    }

    pub async fn activate_account(&self, did: &str) -> Result<()> {
        account::activate_account(did, &self.db).await
    }

    pub async fn get_account_status(&self, handle_or_did: &str) -> Result<AccountStatus> {
        let got = account::get_account(
            handle_or_did,
            Some(AvailabilityFlags {
                include_deactivated: Some(true),
                include_taken_down: Some(true),
            }),
            &self.db,
        )
        .await?;
        let res = account::format_account_status(got);
        match res.active {
            true => Ok(AccountStatus::Active),
            false => Ok(res.status.expect("Account status not properly formatted.")),
        }
    }

    pub async fn update_email(&self, opts: UpdateEmailOpts) -> Result<()> {
        let UpdateEmailOpts { did, email } = opts;
        let (_, _) = tokio::try_join!(
            account::update_email(&did, &email, &self.db),
            email_token::delete_all_email_tokens(&did, &self.db)
        )?;
        Ok(())
    }

    pub async fn create_email_token(
        &self,
        did: &str,
        purpose: EmailTokenPurpose,
    ) -> Result<String> {
        email_token::create_email_token(did, purpose, &self.db).await
    }
}
