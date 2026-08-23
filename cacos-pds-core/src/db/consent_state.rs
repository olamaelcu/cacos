//! Consent-state (nonce) store for the headless-consent remote API.
//!
//! One rotating one-time nonce per authorization request, owned by the PDS.
//! Created by the `/oauth/authorize` redirect handler, rotated (atomically)
//! on every remote API call, and deleted on accept/reject success.

use crate::db::entities::consent_state;
use anyhow::{Result, bail};
use base64ct::{Base64Url, Encoding};
use rand::Rng;
use rsky_common::now as rfc3339_now;
use rsky_common::time::{from_micros_to_str, from_str_to_micros};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};

/// The nonce row stays valid for this window (matches
/// `rsky_oauth::request::AUTHORIZATION_INACTIVITY_TIMEOUT`, 300s). Sliding:
/// every `rotate` overwrites `expiresAt` to now + window.
const NONCE_TTL_MICROS: i64 = 300 * 1_000_000;

fn expires_at(now_str: &str) -> String {
    match from_str_to_micros(now_str) {
        Ok(micros) => from_micros_to_str(micros + NONCE_TTL_MICROS),
        Err(_) => now_str.to_owned(),
    }
}

/// 32 random bytes, base64url-encoded without padding — the one-time nonce.
pub fn new_nonce() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    Base64Url::encode_string(&bytes).replace('=', "")
}

/// Insert a fresh nonce for `request_id`, returning it.
pub async fn create(db: &sea_orm::DatabaseConnection, request_id: &str) -> Result<String> {
    let state = new_nonce();
    let now_str = rfc3339_now();
    let model = consent_state::ActiveModel {
        request_id: Set(request_id.to_owned()),
        state: Set(state.clone()),
        expires_at: Set(expires_at(&now_str)),
    };
    model.insert(db).await?;
    Ok(state)
}

/// Validate the supplied nonce against the row and rotate to a fresh one,
/// extending `expiresAt`. Returns the new nonce on success; errors on
/// mismatch/expiry.
pub async fn rotate(
    db: &sea_orm::DatabaseConnection,
    request_id: &str,
    supplied: &str,
) -> Result<String> {
    let now_str = rfc3339_now();
    let fresh = new_nonce();
    // Atomic update: match both request_id AND state, AND not-expired, set
    // new state + expiresAt. RFC3339 lexicographic comparison is valid for
    // the ms timestamps we write.
    let result = consent_state::Entity::update_many()
        .set(consent_state::ActiveModel {
            state: Set(fresh.clone()),
            expires_at: Set(expires_at(&now_str)),
            ..Default::default()
        })
        .filter(consent_state::Column::RequestId.eq(request_id))
        .filter(consent_state::Column::State.eq(supplied))
        .filter(consent_state::Column::ExpiresAt.gt(now_str))
        .exec(db)
        .await?;
    if result.rows_affected == 0 {
        bail!("invalid or expired state");
    }
    Ok(fresh)
}

/// Delete the row for `request_id` (used on accept/reject success).
pub async fn delete(db: &sea_orm::DatabaseConnection, request_id: &str) -> Result<()> {
    consent_state::Entity::delete_many()
        .filter(consent_state::Column::RequestId.eq(request_id))
        .exec(db)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DatabaseKind;

    async fn test_db() -> (camino_tempfile::Utf8TempDir, sea_orm::DatabaseConnection) {
        let dir = camino_tempfile::Utf8TempDir::new().unwrap();
        let db = DatabaseKind::Account
            .open(dir.path().join("account.sqlite"))
            .await
            .unwrap();
        (dir, db)
    }

    #[tokio::test]
    async fn create_then_rotate_returns_fresh_and_consumes_old() {
        let (_dir, db) = test_db().await;
        let first = create(&db, "req-1").await.unwrap();
        let second = rotate(&db, "req-1", &first).await.unwrap();
        assert_ne!(first, second);
        // Old nonce is consumed.
        assert!(rotate(&db, "req-1", &first).await.is_err());
        // New nonce rotates again.
        let third = rotate(&db, "req-1", &second).await.unwrap();
        assert_ne!(second, third);
    }

    #[tokio::test]
    async fn rotate_with_wrong_state_fails() {
        let (_dir, db) = test_db().await;
        let first = create(&db, "req-1").await.unwrap();
        assert!(rotate(&db, "req-1", "wrong").await.is_err());
        // Original nonce still valid.
        assert!(rotate(&db, "req-1", &first).await.is_ok());
    }

    #[tokio::test]
    async fn rotate_on_expired_row_fails() {
        let (_dir, db) = test_db().await;
        let first = create(&db, "req-1").await.unwrap();
        // Backdate expiresAt into the past.
        consent_state::Entity::update_many()
            .set(consent_state::ActiveModel {
                expires_at: Set("2000-01-01T00:00:00.000Z".to_owned()),
                ..Default::default()
            })
            .filter(consent_state::Column::RequestId.eq("req-1"))
            .exec(&db)
            .await
            .unwrap();
        assert!(rotate(&db, "req-1", &first).await.is_err());
    }

    #[tokio::test]
    async fn delete_removes_row() {
        let (_dir, db) = test_db().await;
        let first = create(&db, "req-1").await.unwrap();
        delete(&db, "req-1").await.unwrap();
        assert!(rotate(&db, "req-1", &first).await.is_err());
    }
}
