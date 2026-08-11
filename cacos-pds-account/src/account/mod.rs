//! Account management: `AccountManager` plus the account helpers under
//! `helpers/`.

pub mod helpers;
pub mod oauth_store;
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
use chrono::offset::Utc as UtcOffset;
use lexicon_cid::Cid;
use rsky_common::RFC3339_VARIANT;
use rsky_common::time::from_str_to_micros;
use rsky_lexicon::com::atproto::admin::StatusAttr;
use rsky_lexicon::com::atproto::server::CreateAppPasswordOutput;
use sea_orm::DatabaseConnection;
use std::env;
use std::time::SystemTime;

/// Refresh-token grace period in milliseconds. rsky-common's `HOUR` is in
/// milliseconds (SECOND = 1000, MINUTE = 60_000, HOUR = 3_600_000), so this
/// 2-hour window reproduces atproto's `2 * HOUR` reference behavior without
/// the unit ambiguity that the previous `2 * HOUR as i64` inline expression
/// invited. Regression-pinned by `refresh_grace_period_is_two_hours`.
const REFRESH_GRACE_MS: i64 = 2 * 60 * 60 * 1000;

#[allow(dead_code)] // used by AccountManager::rotate_refresh_token (Task 7)
fn format_micros(micros: i64) -> Result<String> {
    let dt = DateTime::from_timestamp_micros(micros)
        .ok_or_else(|| anyhow::anyhow!("timestamp out of range: {micros}"))?;
    Ok(format!("{}", dt.format(RFC3339_VARIANT)))
}

/// Current wall-clock time as unix seconds. Used by lockout bookkeeping.
fn unix_now_secs() -> i64 {
    let system_time = SystemTime::now();
    let dt: DateTime<UtcOffset> = system_time.into();
    dt.timestamp()
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
            account::register_account(did.clone(), email, password_encrypted, None, &self.db)
                .await?;
        }
        invite::record_invite_use(did.clone(), invite_code.clone(), now, &self.db).await?;
        auth::store_refresh_token(refresh_payload, None, &self.db).await?;
        repo::update_root(did, repo_cid, repo_rev, &self.db).await?;

        // Metrics (Plan 02 constants; no-ops without a recorder)
        metrics::counter!(cacos_pds_core::observability::metrics::SIGNUPS_TOTAL).increment(1);
        if invite_code.is_some() {
            metrics::counter!(cacos_pds_core::observability::metrics::INVITE_USAGE_TOTAL)
                .increment(1);
        }
        Ok((access_jwt, refresh_jwt))
    }

    pub async fn get_account_admin_status(
        &self,
        did: &str,
    ) -> Result<Option<GetAccountAdminStatusOutput>> {
        account::get_account_admin_status(did, &self.db).await
    }

    /// Whether `did`'s PLC document has already been rewritten to drop the
    /// shared server-wide rotation key.
    pub async fn is_plc_rotation_keys_migrated(&self, did: &str) -> Result<bool> {
        account::is_plc_rotation_keys_migrated(did, &self.db).await
    }

    /// Records that `did`'s PLC document now lists only its per-DID
    /// rotation key.
    pub async fn mark_plc_rotation_keys_migrated(&self, did: &str) -> Result<()> {
        account::mark_plc_rotation_keys_migrated(did, &self.db).await
    }

    /// DIDs still listing the shared server rotation key in their PLC
    /// document — the work list for `migrate plc-rotation-keys`.
    pub async fn list_unmigrated_plc_rotation_key_dids(&self) -> Result<Vec<String>> {
        account::list_unmigrated_plc_rotation_key_dids(&self.db).await
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

    // Auth
    // ----------
    pub async fn create_session(
        &self,
        did: String,
        app_password_name: Option<String>,
    ) -> Result<(String, String)> {
        if self.is_account_locked(&did).await? {
            return Err(anyhow::Error::new(
                account::AccountHelperError::AccountLocked,
            ));
        }
        let scope = if app_password_name.is_none() {
            AuthScope::Access
        } else {
            AuthScope::AppPass
        };
        let (access_jwt, refresh_jwt) = auth::create_tokens(CreateTokensOpts {
            did,
            service_did: env::var("PDS_SERVICE_DID").unwrap(),
            scope: Some(scope),
            jti: None,
            expires_in: None,
        })?;
        let refresh_payload = auth::decode_refresh_token(refresh_jwt.clone())?;
        auth::store_refresh_token(refresh_payload, app_password_name, &self.db).await?;
        metrics::counter!(cacos_pds_core::observability::metrics::SESSIONS_TOTAL).increment(1);
        Ok((access_jwt, refresh_jwt))
    }

    pub async fn rotate_refresh_token(&self, id: &String) -> Result<Option<(String, String)>> {
        let token = auth::get_refresh_token(id, &self.db).await?;
        if let Some(token) = token {
            let system_time = SystemTime::now();
            let dt: DateTime<UtcOffset> = system_time.into();
            let now = format!("{}", dt.format(RFC3339_VARIANT));

            // take the chance to tidy all of a user's expired tokens
            // does not need to be transactional since this is just best-effort
            auth::delete_expired_refresh_tokens(&token.did, now, &self.db).await?;

            // Shorten the refresh token lifespan down from its
            // original expiration time to its revocation grace period.
            let prev_expires_at = from_str_to_micros(&token.expires_at)?;

            let grace_expires_at = dt.timestamp_micros() + REFRESH_GRACE_MS * 1000;

            let expires_at = if grace_expires_at < prev_expires_at {
                grace_expires_at
            } else {
                prev_expires_at
            };

            if expires_at <= dt.timestamp_micros() {
                return Ok(None);
            }

            // Determine the next refresh token id: upon refresh token
            // reuse you always receive a refresh token with the same id.
            let next_id = token.next_id.unwrap_or_else(auth::get_refresh_token_id);

            let (access_jwt, refresh_jwt) = auth::create_tokens(CreateTokensOpts {
                did: token.did,
                service_did: env::var("PDS_SERVICE_DID").unwrap(),
                scope: Some(if token.app_password_name.is_none() {
                    AuthScope::Access
                } else {
                    AuthScope::AppPass
                }),
                jti: Some(next_id.clone()),
                expires_in: None,
            })?;
            let refresh_payload = auth::decode_refresh_token(refresh_jwt.clone())?;
            match tokio::try_join!(
                auth::add_refresh_grace_period(
                    RefreshGracePeriodOpts {
                        id: id.clone(),
                        expires_at: format_micros(expires_at)?,
                        next_id,
                    },
                    &self.db
                ),
                auth::store_refresh_token(refresh_payload, token.app_password_name, &self.db)
            ) {
                Ok(_) => Ok(Some((access_jwt, refresh_jwt))),
                Err(e) => match e.downcast_ref() {
                    Some(AuthHelperError::ConcurrentRefresh) => {
                        Box::pin(self.rotate_refresh_token(id)).await
                    }
                    _ => Err(e),
                },
            }
        } else {
            Ok(None)
        }
    }

    pub async fn revoke_refresh_token(&self, id: String) -> Result<bool> {
        auth::revoke_refresh_token(id, &self.db).await
    }

    // Invites
    // ----------
    pub async fn create_invite_codes(
        &self,
        to_create: Vec<rsky_lexicon::com::atproto::server::AccountCodes>,
        use_count: i32,
    ) -> Result<()> {
        invite::create_invite_codes(to_create, use_count, &self.db).await
    }

    pub async fn create_account_invite_codes(
        &self,
        for_account: &str,
        codes: Vec<String>,
        expected_total: usize,
        disabled: bool,
    ) -> Result<Vec<CodeDetail>> {
        invite::create_account_invite_codes(for_account, codes, expected_total, disabled, &self.db)
            .await
    }

    pub async fn get_account_invite_codes(&self, did: &str) -> Result<Vec<CodeDetail>> {
        invite::get_account_invite_codes(did, &self.db).await
    }

    pub async fn get_invited_by_for_accounts(
        &self,
        dids: Vec<String>,
    ) -> Result<std::collections::BTreeMap<String, CodeDetail>> {
        invite::get_invited_by_for_accounts(dids, &self.db).await
    }

    pub async fn set_account_invites_disabled(&self, did: &str, disabled: bool) -> Result<()> {
        invite::set_account_invites_disabled(did, disabled, &self.db).await
    }

    pub async fn disable_invite_codes(&self, opts: DisableInviteCodesOpts) -> Result<()> {
        invite::disable_invite_codes(opts, &self.db).await
    }

    // Passwords
    // ----------
    pub async fn create_app_password(
        &self,
        did: String,
        name: String,
    ) -> Result<CreateAppPasswordOutput> {
        password::create_app_password(did, name, &self.db).await
    }

    pub async fn list_app_passwords(&self, did: &str) -> Result<Vec<(String, String)>> {
        password::list_app_passwords(did, &self.db).await
    }

    pub async fn verify_account_password(&self, did: &str, password_str: &String) -> Result<bool> {
        password::verify_account_password(did, password_str, &self.db).await
    }

    pub async fn verify_app_password(
        &self,
        did: &str,
        password_str: &str,
    ) -> Result<Option<String>> {
        password::verify_app_password(did, password_str, &self.db).await
    }

    /// Returns `true` if `did` exists and is currently locked out. An expired
    /// lockout (locked_until <= now) is treated as unlocked.
    pub async fn is_account_locked(&self, did: &str) -> Result<bool> {
        let (_, locked_until) = account::get_account_lockout_state(did, &self.db).await?;
        match locked_until {
            None => Ok(false),
            Some(until) => Ok(until > unix_now_secs()),
        }
    }

    /// Stamp the lockout deadline (unix seconds) for `did`.
    pub async fn lock_account(&self, did: &str, until_unix: i64) -> Result<()> {
        account::set_account_locked_until(did, until_unix, &self.db).await
    }

    /// Increment the failed-login counter; returns the new value. If the
    /// account does not exist the count is 0 (no-op).
    pub async fn record_failed_login(&self, did: &str) -> Result<()> {
        account::increment_failed_login_count(did, &self.db).await?;
        Ok(())
    }

    /// Increment the failed-login counter and apply the lockout policy if
    /// the threshold (5 attempts) is reached. Returns `true` if the account
    /// is now locked, `false` otherwise.
    pub async fn record_failed_login_with_lockout(&self, did: &str) -> Result<bool> {
        account::increment_failed_login_count(did, &self.db).await?;
        let (count, _) = account::get_account_lockout_state(did, &self.db).await?;
        if count >= 5 {
            let until = unix_now_secs() + 900;
            account::set_account_locked_until(did, until, &self.db).await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Clear the failed-login counter and any lockout deadline.
    pub async fn clear_failed_logins(&self, did: &str) -> Result<()> {
        account::clear_account_lockout(did, &self.db).await
    }

    pub async fn reset_password(&self, opts: ResetPasswordOpts) -> Result<()> {
        let did = email_token::assert_valid_token_and_find_did(
            EmailTokenPurpose::ResetPassword,
            &opts.token,
            None,
            &self.db,
        )
        .await?;
        self.update_account_password(UpdateAccountPasswordOpts {
            did,
            password: opts.password,
        })
        .await
    }

    pub async fn update_account_password(&self, opts: UpdateAccountPasswordOpts) -> Result<()> {
        let UpdateAccountPasswordOpts { did, .. } = opts;
        let password_encrypted = password::gen_salt_and_hash(opts.password)?;
        let (_, _, _) = tokio::try_join!(
            password::update_user_password(
                UpdateUserPasswordOpts {
                    did: did.clone(),
                    password_encrypted,
                },
                &self.db
            ),
            email_token::delete_email_token(&did, EmailTokenPurpose::ResetPassword, &self.db),
            auth::revoke_refresh_tokens_by_did(&did, &self.db)
        )?;
        Ok(())
    }

    pub async fn revoke_app_password(&self, did: String, name: String) -> Result<()> {
        let (_, _) = tokio::try_join!(
            password::delete_app_password(&did, &name, &self.db),
            auth::revoke_app_password_refresh_token(&did, &name, &self.db)
        )?;
        Ok(())
    }

    // Email Tokens
    // ----------
    pub async fn confirm_email(&self, opts: ConfirmEmailOpts<'_>) -> Result<()> {
        let ConfirmEmailOpts { did, token } = opts;
        email_token::assert_valid_token(
            did,
            EmailTokenPurpose::ConfirmEmail,
            token,
            None,
            &self.db,
        )
        .await?;
        let now = rsky_common::now();
        let (_, _) = tokio::try_join!(
            email_token::delete_email_token(did, EmailTokenPurpose::ConfirmEmail, &self.db),
            account::set_email_confirmed_at(did, now, &self.db)
        )?;
        Ok(())
    }

    pub async fn assert_valid_email_token(
        &self,
        did: &str,
        purpose: EmailTokenPurpose,
        token: &str,
    ) -> Result<()> {
        email_token::assert_valid_token(did, purpose, token, None, &self.db).await
    }

    pub async fn assert_valid_email_token_and_cleanup(
        &self,
        did: &str,
        purpose: EmailTokenPurpose,
        token: &str,
    ) -> Result<()> {
        email_token::assert_valid_token(did, purpose, token, None, &self.db).await?;
        email_token::delete_email_token(did, purpose, &self.db).await
    }
}
