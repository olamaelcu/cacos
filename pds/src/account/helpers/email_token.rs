//! Email token helper: 15-min TTL confirm/update/reset/delete/PLC tokens,
//! upserted per `(purpose, did)`. Tokens are xxxxx-xxxxx uppercase alphanumeric
//! with `0/1/8/9` stripped (Bluesky Client limitation).

use crate::account::helpers::sql;
use anyhow::{Result, bail};
use rand::{Rng, distributions::Alphanumeric};
use rsky_common::time::{MINUTE, from_str_to_utc, less_than_ago_s};
use sea_orm::{ConnectionTrait, DatabaseConnection, QueryResult, Value};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum EmailTokenPurpose {
    #[default]
    ConfirmEmail,
    UpdateEmail,
    ResetPassword,
    DeleteAccount,
    PlcOperation,
}

impl EmailTokenPurpose {
    pub fn as_str(&self) -> &'static str {
        match self {
            EmailTokenPurpose::ConfirmEmail => "confirm_email",
            EmailTokenPurpose::UpdateEmail => "update_email",
            EmailTokenPurpose::ResetPassword => "reset_password",
            EmailTokenPurpose::DeleteAccount => "delete_account",
            EmailTokenPurpose::PlcOperation => "plc_operation",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Result<Self> {
        match s {
            "confirm_email" => Ok(EmailTokenPurpose::ConfirmEmail),
            "update_email" => Ok(EmailTokenPurpose::UpdateEmail),
            "reset_password" => Ok(EmailTokenPurpose::ResetPassword),
            "delete_account" => Ok(EmailTokenPurpose::DeleteAccount),
            "plc_operation" => Ok(EmailTokenPurpose::PlcOperation),
            _ => bail!("Unable to parse as EmailTokenPurpose: `{s:?}`"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct EmailToken {
    pub purpose: EmailTokenPurpose,
    pub did: String,
    pub token: String,
    #[serde(rename = "requestedAt")]
    pub requested_at: String,
}

pub(crate) fn email_token_from_row(row: &QueryResult) -> Result<EmailToken, sea_orm::DbErr> {
    let purpose: String = row.try_get_by_index(0)?;
    Ok(EmailToken {
        purpose: EmailTokenPurpose::from_str(&purpose).map_err(|_| {
            sea_orm::DbErr::Custom(format!("invalid email token purpose: {purpose}"))
        })?,
        did: row.try_get_by_index(1)?,
        token: row.try_get_by_index(2)?,
        requested_at: row.try_get_by_index(3)?,
    })
}

/// Formatted xxxxx-xxxxx (the Bluesky Client can't render `1`, `8`, `9`, `0`
/// in email verification tokens, so the source pool excludes them).
pub(crate) fn get_random_token() -> String {
    let token: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(50)
        .map(char::from)
        .collect();
    let allowed_token = token.replace(&['1', '8', '9', '0'][..], "");
    allowed_token[0..5].to_owned() + "-" + &allowed_token[5..10]
}

pub async fn create_email_token(
    did: &str,
    purpose: EmailTokenPurpose,
    db: &DatabaseConnection,
) -> Result<String> {
    let token = get_random_token().to_uppercase();
    let now = rsky_common::now();

    db.execute_raw(sql(
        "INSERT INTO email_token (purpose, did, token, \"requestedAt\") \
         VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT (purpose, did) DO UPDATE SET \
         token = excluded.token, \"requestedAt\" = excluded.\"requestedAt\"",
        vec![
            Value::from(purpose.as_str()),
            Value::from(did.to_owned()),
            Value::from(token.clone()),
            Value::from(now),
        ],
    ))
    .await?;
    Ok(token)
}

pub async fn assert_valid_token(
    did: &str,
    purpose: EmailTokenPurpose,
    token: &str,
    expiration_len: Option<i32>,
    db: &DatabaseConnection,
) -> Result<()> {
    let expiration_len = expiration_len.unwrap_or(MINUTE * 15);

    let row: Option<QueryResult> = db
        .query_one_raw(sql(
            "SELECT purpose, did, token, \"requestedAt\" FROM email_token \
             WHERE purpose = ?1 AND did = ?2 AND token = ?3",
            vec![
                Value::from(purpose.as_str()),
                Value::from(did.to_owned()),
                Value::from(token.to_uppercase()),
            ],
        ))
        .await?;
    if let Some(row) = row {
        let res = email_token_from_row(&row)?;
        let requested_at = from_str_to_utc(&res.requested_at)?;
        let expired = !less_than_ago_s(requested_at, expiration_len);
        if expired {
            bail!("Token is expired")
        }
        Ok(())
    } else {
        bail!("Token is invalid")
    }
}

pub async fn assert_valid_token_and_find_did(
    purpose: EmailTokenPurpose,
    token: &str,
    expiration_len: Option<i32>,
    db: &DatabaseConnection,
) -> Result<String> {
    let expiration_len = expiration_len.unwrap_or(MINUTE * 15);

    let row: Option<QueryResult> = db
        .query_one_raw(sql(
            "SELECT purpose, did, token, \"requestedAt\" FROM email_token \
             WHERE purpose = ?1 AND token = ?2",
            vec![
                Value::from(purpose.as_str()),
                Value::from(token.to_uppercase()),
            ],
        ))
        .await?;
    if let Some(row) = row {
        let res = email_token_from_row(&row)?;
        let requested_at = from_str_to_utc(&res.requested_at)?;
        let expired = !less_than_ago_s(requested_at, expiration_len);
        if expired {
            bail!("Token is expired")
        }
        Ok(res.did)
    } else {
        bail!("Token is invalid")
    }
}

pub async fn delete_email_token(
    did: &str,
    purpose: EmailTokenPurpose,
    db: &DatabaseConnection,
) -> Result<()> {
    db.execute_raw(sql(
        "DELETE FROM email_token WHERE did = ?1 AND purpose = ?2",
        vec![Value::from(did.to_owned()), Value::from(purpose.as_str())],
    ))
    .await?;
    Ok(())
}

pub async fn delete_all_email_tokens(did: &str, db: &DatabaseConnection) -> Result<()> {
    db.execute_raw(sql(
        "DELETE FROM email_token WHERE did = ?1",
        vec![Value::from(did.to_owned())],
    ))
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::helpers::sql;
    use crate::account::test_util::test_db;
    use sea_orm::ConnectionTrait;

    #[tokio::test]
    async fn create_assert_delete_roundtrip() {
        let (_dir, db) = test_db().await;
        let token = create_email_token("did:plc:henry", EmailTokenPurpose::ConfirmEmail, &db)
            .await
            .unwrap();
        // formatted xxxxx-xxxxx, uppercased
        assert_eq!(token.len(), 11);
        assert_eq!(token.chars().nth(5), Some('-'));
        assert!(token.chars().all(|c| c == '-' || c.is_ascii_uppercase()));

        assert_valid_token(
            "did:plc:henry",
            EmailTokenPurpose::ConfirmEmail,
            &token,
            None,
            &db,
        )
        .await
        .unwrap();
        delete_email_token("did:plc:henry", EmailTokenPurpose::ConfirmEmail, &db)
            .await
            .unwrap();
        let err = assert_valid_token(
            "did:plc:henry",
            EmailTokenPurpose::ConfirmEmail,
            &token,
            None,
            &db,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("Token is invalid"));
    }

    #[tokio::test]
    async fn second_token_replaces_first() {
        let (_dir, db) = test_db().await;
        let token = create_email_token("did:plc:henry", EmailTokenPurpose::ConfirmEmail, &db)
            .await
            .unwrap();
        let replacement = create_email_token("did:plc:henry", EmailTokenPurpose::ConfirmEmail, &db)
            .await
            .unwrap();
        assert_ne!(token, replacement);
        assert!(
            assert_valid_token(
                "did:plc:henry",
                EmailTokenPurpose::ConfirmEmail,
                &token,
                None,
                &db,
            )
            .await
            .is_err()
        );
        assert_valid_token(
            "did:plc:henry",
            EmailTokenPurpose::ConfirmEmail,
            &replacement,
            None,
            &db,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn expired_token_rejected() {
        let (_dir, db) = test_db().await;
        let token = create_email_token("did:plc:henry", EmailTokenPurpose::ConfirmEmail, &db)
            .await
            .unwrap();
        // backdate the request to the epoch
        db.execute_raw(sql(
            "UPDATE email_token SET \"requestedAt\" = ?1 WHERE did = ?2",
            vec![
                Value::from(rsky_common::time::from_micros_to_str(1_000_000)),
                Value::from("did:plc:henry"),
            ],
        ))
        .await
        .unwrap();
        let err = assert_valid_token(
            "did:plc:henry",
            EmailTokenPurpose::ConfirmEmail,
            &token,
            None,
            &db,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("Token is expired"));
        let err =
            assert_valid_token_and_find_did(EmailTokenPurpose::ConfirmEmail, &token, None, &db)
                .await
                .unwrap_err();
        assert!(err.to_string().contains("Token is expired"));
    }

    #[tokio::test]
    async fn invalid_token_rejected() {
        let (_dir, db) = test_db().await;
        let err = assert_valid_token(
            "did:plc:henry",
            EmailTokenPurpose::ConfirmEmail,
            "BOGUS",
            None,
            &db,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("Token is invalid"));
        let err =
            assert_valid_token_and_find_did(EmailTokenPurpose::ConfirmEmail, "BOGUS", None, &db)
                .await
                .unwrap_err();
        assert!(err.to_string().contains("Token is invalid"));
    }

    #[tokio::test]
    async fn assert_valid_token_and_find_did_returns_did() {
        let (_dir, db) = test_db().await;
        let token = create_email_token("did:plc:henry", EmailTokenPurpose::ResetPassword, &db)
            .await
            .unwrap();
        let did =
            assert_valid_token_and_find_did(EmailTokenPurpose::ResetPassword, &token, None, &db)
                .await
                .unwrap();
        assert_eq!(did, "did:plc:henry");
        // purpose must match
        assert!(
            assert_valid_token_and_find_did(EmailTokenPurpose::ConfirmEmail, &token, None, &db)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn row_mapping_rejects_unknown_purpose() {
        let (_dir, db) = test_db().await;
        let row: Option<QueryResult> = db
            .query_one_raw(sql(
                "SELECT 'bogus_purpose', 'did:plc:x', 'TOKEN', '2023-01-01T00:00:00.000Z'",
                vec![],
            ))
            .await
            .unwrap();
        let row = row.unwrap();
        assert!(email_token_from_row(&row).is_err());
    }

    #[tokio::test]
    async fn delete_all_email_tokens_removes_every_purpose() {
        let (_dir, db) = test_db().await;
        create_email_token("did:plc:henry", EmailTokenPurpose::ConfirmEmail, &db)
            .await
            .unwrap();
        create_email_token("did:plc:henry", EmailTokenPurpose::ResetPassword, &db)
            .await
            .unwrap();
        delete_all_email_tokens("did:plc:henry", &db).await.unwrap();
        assert!(
            assert_valid_token_and_find_did(
                EmailTokenPurpose::ResetPassword,
                "AAAAA-AAAAA",
                None,
                &db,
            )
            .await
            .is_err()
        );
    }
}
