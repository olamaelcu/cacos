//! Auth helper: access/refresh/service JWTs (ES256K), refresh-token storage
//! with grace-period rotation, concurrent-refresh detection.
//!
//! Owns `PDS_JWT_KEYPAIR` and `AuthScope` for the whole crate (see the
//! cross-plan contracts in the plan header — Plan 06's auth_verifier imports
//! both from here).

use crate::account::helpers::sql;
use crate::context::PDS_REPO_SIGNING_KEYPAIR;
use anyhow::{Error, Result, bail};
use chrono::DateTime;
use jwt_simple::prelude::*;
use miette::Diagnostic;
use rsky_common::time::MINUTE;
use rsky_common::{RFC3339_VARIANT, get_random_str, json_to_b64url};
use sea_orm::{ConnectionTrait, DatabaseConnection, QueryResult, Value};
use secp256k1::Message;
use secrecy::{ExposeSecret, ExposeSecretMut, SecretBox};
use sha2::{Digest, Sha256};
use std::env;
use std::sync::LazyLock;
use std::time::SystemTime;
use thiserror::Error;
use zeroize::Zeroize;

/// Canonical ES256K signing key for access/refresh/service JWTs, from
/// `PDS_JWT_KEY_K256_PRIVATE_KEY_HEX`. Owned HERE; Plan 06's auth_verifier
/// imports it (`use crate::account::helpers::auth::PDS_JWT_KEYPAIR;`).
pub static PDS_JWT_KEYPAIR: LazyLock<ES256kKeyPair> = LazyLock::new(|| {
    let secp = secp256k1::Secp256k1::new();
    let private_key = env::var("PDS_JWT_KEY_K256_PRIVATE_KEY_HEX").unwrap();
    let mut secret_bytes = SecretBox::new(Box::new(hex::decode(private_key.as_bytes()).unwrap()));
    let secret_key = secp256k1::SecretKey::from_slice(secret_bytes.expose_secret()).unwrap();
    let jwt_key = secp256k1::Keypair::from_secret_key(&secp, &secret_key);
    secret_bytes.expose_secret_mut().zeroize();
    ES256kKeyPair::from_bytes(jwt_key.secret_bytes().as_slice()).unwrap()
});

#[derive(PartialEq, Clone, Debug)]
pub enum AuthScope {
    Access,
    Refresh,
    AppPass,
    AppPassPrivileged,
    SignupQueued,
}

impl AuthScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            AuthScope::Access => "com.atproto.access",
            AuthScope::Refresh => "com.atproto.refresh",
            AuthScope::AppPass => "com.atproto.appPass",
            AuthScope::AppPassPrivileged => "com.atproto.appPassPrivileged",
            AuthScope::SignupQueued => "com.atproto.signupQueued",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(scope: &str) -> Result<Self> {
        match scope {
            "com.atproto.access" => Ok(AuthScope::Access),
            "com.atproto.refresh" => Ok(AuthScope::Refresh),
            "com.atproto.appPass" => Ok(AuthScope::AppPass),
            "com.atproto.appPassPrivileged" => Ok(AuthScope::AppPassPrivileged),
            "com.atproto.signupQueued" => Ok(AuthScope::SignupQueued),
            _ => bail!("Invalid AuthScope: `{scope:?}` is not a valid auth scope"),
        }
    }
}

pub struct CreateTokensOpts {
    pub did: String,
    pub service_did: String,
    pub scope: Option<AuthScope>,
    pub jti: Option<String>,
    pub expires_in: Option<Duration>,
}

pub struct RefreshGracePeriodOpts {
    pub id: String,
    pub expires_at: String,
    pub next_id: String,
}

#[allow(dead_code)] // defined for reference fidelity; nothing in this plan constructs it
pub struct AuthToken {
    pub scope: AuthScope,
    pub sub: String,
    pub exp: Duration,
}

pub struct RefreshToken {
    pub scope: AuthScope, // AuthScope::Refresh
    pub sub: String,
    pub exp: Duration,
    pub jti: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ServiceJwtPayload {
    pub iss: String,
    pub aud: String,
    pub exp: Option<u64>,
    pub lxm: Option<String>,
    pub jti: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ServiceJwtHeader {
    pub typ: String,
    pub alg: String,
}

pub struct ServiceJwtParams {
    pub iss: String,
    pub aud: String,
    pub exp: Option<u64>,
    pub lxm: Option<String>,
    pub jti: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct CustomClaimObj {
    pub scope: String,
}

#[derive(Error, Debug, Diagnostic)]
pub enum AuthHelperError {
    #[error("ConcurrentRefreshError")]
    #[diagnostic(code(cacos::pds::auth::concurrent_refresh))]
    ConcurrentRefresh,

    #[error("not a refresh token")]
    #[diagnostic(code(cacos::pds::auth::not_a_refresh_token))]
    NotARefreshToken,

    #[error("refresh token missing {0} claim")]
    #[diagnostic(code(cacos::pds::auth::missing_refresh_claim))]
    MissingRefreshClaim(&'static str),
}

pub fn create_tokens(opts: CreateTokensOpts) -> Result<(String, String)> {
    let CreateTokensOpts {
        did,
        service_did,
        scope,
        jti,
        expires_in,
    } = opts;
    let access_jwt = create_access_token(CreateTokensOpts {
        did: did.clone(),
        service_did: service_did.clone(),
        scope,
        expires_in,
        jti: None,
    })?;
    let refresh_jwt = create_refresh_token(CreateTokensOpts {
        did,
        service_did,
        jti,
        expires_in,
        scope: None,
    })?;
    Ok((access_jwt, refresh_jwt))
}

pub fn create_access_token(opts: CreateTokensOpts) -> Result<String> {
    let CreateTokensOpts {
        did,
        service_did,
        scope,
        expires_in,
        ..
    } = opts;
    let scope = scope.unwrap_or(AuthScope::Access);
    let expires_in = expires_in.unwrap_or_else(|| Duration::from_hours(2));
    let claims = Claims::with_custom_claims(
        CustomClaimObj {
            scope: scope.as_str().to_owned(),
        },
        expires_in,
    )
    .with_audience(service_did)
    .with_subject(did);

    PDS_JWT_KEYPAIR.sign(claims)
}

pub fn create_refresh_token(opts: CreateTokensOpts) -> Result<String> {
    let CreateTokensOpts {
        did,
        service_did,
        jti,
        expires_in,
        ..
    } = opts;
    let jti = jti.unwrap_or_else(get_random_str);
    let expires_in = expires_in.unwrap_or_else(|| Duration::from_days(90));
    let claims = Claims::with_custom_claims(
        CustomClaimObj {
            scope: AuthScope::Refresh.as_str().to_owned(),
        },
        expires_in,
    )
    .with_audience(service_did)
    .with_subject(did)
    .with_jwt_id(jti);

    PDS_JWT_KEYPAIR.sign(claims)
}

// Emits a service JWT with `exp` in seconds since the Unix epoch
// (`now_seconds + MINUTE/1000` = now + 1 minute). JWT verifiers interpret
// `exp` as seconds; emitting milliseconds would push the expiry ~54,000
// years into the future and effectively make the token never expire.
pub async fn create_service_jwt(params: ServiceJwtParams) -> Result<String> {
    let ServiceJwtParams { iss, aud, .. } = params;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("timestamp in micros since UNIX epoch")
        .as_micros() as usize;
    let now_secs = (now / 1_000_000) as u64;
    // rsky-common's MINUTE is in milliseconds (SECOND = 1000 ms), so MINUTE / 1000
    // gives 60 seconds — matches atproto's `iat + MINUTE / 1e3`.
    let exp = params.exp.unwrap_or(now_secs + (MINUTE as u64 / 1000));
    let lxm = params.lxm;
    let jti = get_random_str();
    let header = ServiceJwtHeader {
        typ: "JWT".to_string(),
        alg: "ES256K".to_string(),
    };
    let payload = ServiceJwtPayload {
        iss,
        aud,
        exp: Some(exp),
        lxm,
        jti: Some(jti),
    };
    let to_sign_str = format!(
        "{0}.{1}",
        json_to_b64url(&header)?,
        json_to_b64url(&payload)?
    );
    let hash = Sha256::digest(to_sign_str.clone());
    let message = Message::from_digest_slice(hash.as_ref())?;
    let mut sig = PDS_REPO_SIGNING_KEYPAIR.secret_key().sign_ecdsa(message);
    // Convert to low-s
    sig.normalize_s();
    // ASN.1 encoded per decode_dss_signature
    let compact_sig = sig.serialize_compact();
    Ok(format!(
        "{0}.{1}",
        to_sign_str,
        base64_url::encode(&compact_sig).replace("=", "") // Base 64 encode signature bytes
    ))
}

// @NOTE should only be used w/ direct output from createRefreshToken() or createTokens();
// returns Err (not panic) on wrong-scope tokens or missing claims.
pub fn decode_refresh_token(jwt: String) -> Result<RefreshToken> {
    let claims = PDS_JWT_KEYPAIR
        .public_key()
        .verify_token::<CustomClaimObj>(&jwt, None)?;
    if claims.custom.scope != AuthScope::Refresh.as_str() {
        return Err(Error::new(AuthHelperError::NotARefreshToken));
    }
    Ok(RefreshToken {
        scope: AuthScope::from_str(&claims.custom.scope)?,
        sub: claims
            .subject
            .ok_or_else(|| Error::new(AuthHelperError::MissingRefreshClaim("subject")))?,
        exp: claims
            .expires_at
            .ok_or_else(|| Error::new(AuthHelperError::MissingRefreshClaim("expiration")))?,
        jti: claims
            .jwt_id
            .ok_or_else(|| Error::new(AuthHelperError::MissingRefreshClaim("jti")))?,
    })
}

pub async fn store_refresh_token(
    payload: RefreshToken,
    app_password_name: Option<String>,
    db: &DatabaseConnection,
) -> Result<()> {
    let exp = DateTime::from_timestamp_millis(payload.exp.as_millis() as i64)
        .ok_or_else(|| anyhow::anyhow!("token expiry out of range"))?;
    let expires_at = format!("{}", exp.format(RFC3339_VARIANT));

    // ON CONFLICT DO NOTHING e.g. when re-granting during a refresh grace period
    db.execute_raw(sql(
        "INSERT INTO refresh_token (id, did, \"appPasswordName\", \"expiresAt\") \
         VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT (id) DO NOTHING",
        vec![
            Value::from(payload.jti),
            Value::from(payload.sub),
            Value::from(app_password_name),
            Value::from(expires_at),
        ],
    ))
    .await?;
    Ok(())
}

pub async fn revoke_refresh_token(id: String, db: &DatabaseConnection) -> Result<bool> {
    let deleted = db
        .execute_raw(sql(
            "DELETE FROM refresh_token WHERE id = ?1",
            vec![Value::from(id)],
        ))
        .await?;
    Ok(deleted.rows_affected() > 0)
}

pub async fn revoke_refresh_tokens_by_did(did: &str, db: &DatabaseConnection) -> Result<bool> {
    let deleted = db
        .execute_raw(sql(
            "DELETE FROM refresh_token WHERE did = ?1",
            vec![Value::from(did.to_owned())],
        ))
        .await?;
    Ok(deleted.rows_affected() > 0)
}

pub async fn revoke_app_password_refresh_token(
    did: &str,
    app_pass_name: &str,
    db: &DatabaseConnection,
) -> Result<bool> {
    let deleted = db
        .execute_raw(sql(
            "DELETE FROM refresh_token WHERE did = ?1 AND \"appPasswordName\" = ?2",
            vec![
                Value::from(did.to_owned()),
                Value::from(app_pass_name.to_owned()),
            ],
        ))
        .await?;
    Ok(deleted.rows_affected() > 0)
}

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct RefreshTokenRow {
    pub id: String,
    pub did: String,
    #[serde(rename = "expiresAt")]
    pub expires_at: String,
    #[serde(rename = "nextId")]
    pub next_id: Option<String>,
    #[serde(rename = "appPasswordName")]
    pub app_password_name: Option<String>,
}

pub async fn get_refresh_token(
    id: &str,
    db: &DatabaseConnection,
) -> Result<Option<RefreshTokenRow>> {
    let row: Option<QueryResult> = db
        .query_one_raw(sql(
            "SELECT id, did, \"expiresAt\", \"nextId\", \"appPasswordName\" \
             FROM refresh_token WHERE id = ?1",
            vec![Value::from(id.to_owned())],
        ))
        .await?;
    match row {
        Some(row) => Ok(Some(RefreshTokenRow {
            id: row.try_get_by_index(0)?,
            did: row.try_get_by_index(1)?,
            expires_at: row.try_get_by_index(2)?,
            next_id: row.try_get_by_index(3)?,
            app_password_name: row.try_get_by_index(4)?,
        })),
        None => Ok(None),
    }
}

pub async fn delete_expired_refresh_tokens(
    did: &str,
    now: String,
    db: &DatabaseConnection,
) -> Result<()> {
    db.execute_raw(sql(
        "DELETE FROM refresh_token WHERE did = ?1 AND \"expiresAt\" <= ?2",
        vec![Value::from(did.to_owned()), Value::from(now)],
    ))
    .await?;
    Ok(())
}

pub async fn add_refresh_grace_period(
    opts: RefreshGracePeriodOpts,
    db: &DatabaseConnection,
) -> Result<()> {
    let RefreshGracePeriodOpts {
        id,
        expires_at,
        next_id,
    } = opts;
    let updated = db
        .execute_raw(sql(
            "UPDATE refresh_token SET \"expiresAt\" = ?1, \"nextId\" = ?2 \
             WHERE id = ?3 AND (\"nextId\" IS NULL OR \"nextId\" = ?2)",
            vec![
                Value::from(expires_at),
                Value::from(next_id.clone()),
                Value::from(id),
            ],
        ))
        .await?;
    if updated.rows_affected() < 1 {
        return Err(anyhow::Error::new(AuthHelperError::ConcurrentRefresh));
    }
    Ok(())
}

pub fn get_refresh_token_id() -> String {
    get_random_str()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::test_util::{init_env, test_db};

    #[tokio::test]
    async fn create_tokens_roundtrip() {
        init_env();
        let (access_jwt, refresh_jwt) = create_tokens(CreateTokensOpts {
            did: "did:plc:alice".to_owned(),
            service_did: "did:web:localho.st".to_owned(),
            scope: Some(AuthScope::Access),
            jti: None,
            expires_in: None,
        })
        .unwrap();
        assert!(!access_jwt.is_empty());
        assert!(!refresh_jwt.is_empty());

        let payload = decode_refresh_token(refresh_jwt).unwrap();
        assert_eq!(payload.sub, "did:plc:alice");
        assert_eq!(payload.scope, AuthScope::Refresh);
        assert!(!payload.jti.is_empty());
    }

    #[test]
    fn access_token_scope_and_subject() {
        init_env();
        let jwt = create_access_token(CreateTokensOpts {
            did: "did:plc:alice".to_owned(),
            service_did: "did:web:localho.st".to_owned(),
            scope: Some(AuthScope::AppPass),
            jti: None,
            expires_in: None,
        })
        .unwrap();
        let claims = PDS_JWT_KEYPAIR
            .public_key()
            .verify_token::<CustomClaimObj>(&jwt, None)
            .unwrap();
        assert_eq!(claims.custom.scope, "com.atproto.appPass");
        assert_eq!(claims.subject.as_deref(), Some("did:plc:alice"));
        match claims.audiences {
            Some(jwt_simple::claims::Audiences::AsString(aud)) => {
                assert_eq!(aud, "did:web:localho.st")
            }
            _ => panic!("expected a string audience"),
        }
    }

    #[tokio::test]
    async fn store_and_get_refresh_token() {
        let (_dir, db) = test_db().await;
        let payload = RefreshToken {
            scope: AuthScope::Refresh,
            sub: "did:plc:alice".to_owned(),
            exp: Duration::from_days(90),
            jti: "tok-1".to_owned(),
        };
        store_refresh_token(payload, Some("My App".to_owned()), &db)
            .await
            .unwrap();
        let got = get_refresh_token("tok-1", &db).await.unwrap().unwrap();
        assert_eq!(got.did, "did:plc:alice");
        assert_eq!(got.app_password_name, Some("My App".to_owned()));
        assert_eq!(got.next_id, None);
        assert!(got.expires_at.ends_with('Z'));
        assert!(get_refresh_token("missing", &db).await.unwrap().is_none());
        // storing the same id again is a no-op (ON CONFLICT DO NOTHING)
        store_refresh_token(
            RefreshToken {
                scope: AuthScope::Refresh,
                sub: "did:plc:alice".to_owned(),
                exp: Duration::from_days(90),
                jti: "tok-1".to_owned(),
            },
            None,
            &db,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn revoke_refresh_token_test() {
        let (_dir, db) = test_db().await;
        store_refresh_token(
            RefreshToken {
                scope: AuthScope::Refresh,
                sub: "did:plc:alice".to_owned(),
                exp: Duration::from_days(90),
                jti: "tok-1".to_owned(),
            },
            None,
            &db,
        )
        .await
        .unwrap();
        assert!(revoke_refresh_token("tok-1".to_owned(), &db).await.unwrap());
        assert!(!revoke_refresh_token("tok-1".to_owned(), &db).await.unwrap());

        store_refresh_token(
            RefreshToken {
                scope: AuthScope::Refresh,
                sub: "did:plc:bob".to_owned(),
                exp: Duration::from_days(90),
                jti: "tok-bob".to_owned(),
            },
            None,
            &db,
        )
        .await
        .unwrap();
        assert!(
            revoke_refresh_tokens_by_did("did:plc:bob", &db)
                .await
                .unwrap()
        );
        assert!(
            !revoke_refresh_tokens_by_did("did:plc:bob", &db)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn revoke_app_password_refresh_token_test() {
        let (_dir, db) = test_db().await;
        store_refresh_token(
            RefreshToken {
                scope: AuthScope::Refresh,
                sub: "did:plc:alice".to_owned(),
                exp: Duration::from_days(90),
                jti: "tok-app".to_owned(),
            },
            Some("rotator".to_owned()),
            &db,
        )
        .await
        .unwrap();
        assert!(
            revoke_app_password_refresh_token("did:plc:alice", "rotator", &db)
                .await
                .unwrap()
        );
        assert!(
            !revoke_app_password_refresh_token("did:plc:alice", "rotator", &db)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn add_refresh_grace_period_guards_next_id() {
        let (_dir, db) = test_db().await;
        store_refresh_token(
            RefreshToken {
                scope: AuthScope::Refresh,
                sub: "did:plc:x".to_owned(),
                exp: Duration::from_days(90),
                jti: "token-1".to_owned(),
            },
            None,
            &db,
        )
        .await
        .unwrap();
        // first grant wins
        add_refresh_grace_period(
            RefreshGracePeriodOpts {
                id: "token-1".to_owned(),
                expires_at: rsky_common::now(),
                next_id: "next-a".to_owned(),
            },
            &db,
        )
        .await
        .unwrap();
        // a second grant with the SAME next id is allowed (grace re-issue)
        add_refresh_grace_period(
            RefreshGracePeriodOpts {
                id: "token-1".to_owned(),
                expires_at: rsky_common::now(),
                next_id: "next-a".to_owned(),
            },
            &db,
        )
        .await
        .unwrap();
        // a conflicting next id is rejected as a concurrent refresh
        let err = add_refresh_grace_period(
            RefreshGracePeriodOpts {
                id: "token-1".to_owned(),
                expires_at: rsky_common::now(),
                next_id: "next-b".to_owned(),
            },
            &db,
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err.downcast_ref::<AuthHelperError>(),
            Some(AuthHelperError::ConcurrentRefresh)
        ));
    }

    #[tokio::test]
    async fn delete_expired_refresh_tokens_test() {
        let (_dir, db) = test_db().await;
        // already-expired token
        store_refresh_token(
            RefreshToken {
                scope: AuthScope::Refresh,
                sub: "did:plc:alice".to_owned(),
                exp: Duration::from_millis(0),
                jti: "tok-expired".to_owned(),
            },
            None,
            &db,
        )
        .await
        .unwrap();
        // still-valid token (amended: `Duration::from_days(90)` is treated as
        // epoch ms in `store_refresh_token`, so it lands in 1970 — must use a
        // span large enough to exceed the current epoch ms, e.g. ~90 years)
        store_refresh_token(
            RefreshToken {
                scope: AuthScope::Refresh,
                sub: "did:plc:alice".to_owned(),
                exp: Duration::from_days(90 * 365),
                jti: "tok-live".to_owned(),
            },
            None,
            &db,
        )
        .await
        .unwrap();

        delete_expired_refresh_tokens("did:plc:alice", rsky_common::now(), &db)
            .await
            .unwrap();
        assert!(
            get_refresh_token("tok-expired", &db)
                .await
                .unwrap()
                .is_none()
        );
        assert!(get_refresh_token("tok-live", &db).await.unwrap().is_some());
    }

    #[test]
    fn get_refresh_token_id_non_empty() {
        assert!(!get_refresh_token_id().is_empty());
    }

    #[tokio::test]
    async fn service_jwt_three_segments() {
        init_env();
        let service_jwt = create_service_jwt(ServiceJwtParams {
            iss: "did:web:pds.test".to_owned(),
            aud: "did:web:appview.test".to_owned(),
            exp: None,
            lxm: Some("com.atproto.repo.uploadBlob".to_owned()),
            jti: None,
        })
        .await
        .unwrap();
        assert_eq!(service_jwt.split('.').count(), 3);
        // header decodes to typ JWT / alg ES256K
        let header_b64 = service_jwt.split('.').next().unwrap();
        let header_json = base64_url::decode(header_b64).unwrap();
        assert!(String::from_utf8_lossy(&header_json).contains("\"alg\":\"ES256K\""));
    }

    #[tokio::test]
    async fn service_jwt_exp_is_seconds_since_epoch() {
        init_env();
        let service_jwt = create_service_jwt(ServiceJwtParams {
            iss: "did:web:pds.test".to_owned(),
            aud: "did:web:appview.test".to_owned(),
            exp: None,
            lxm: Some("com.atproto.repo.uploadBlob".to_owned()),
            jti: None,
        })
        .await
        .unwrap();
        // Decode the payload segment (index 1) as base64url JSON.
        let payload_b64 = service_jwt.split('.').nth(1).unwrap();
        let payload_json = base64_url::decode(payload_b64).unwrap();
        let payload: serde_json::Value =
            serde_json::from_slice(&payload_json).expect("payload is JSON");
        let exp = payload
            .get("exp")
            .and_then(|v| v.as_u64())
            .expect("payload.exp present as u64");

        // Current epoch seconds (the JWT exp is now + 1 minute, in seconds).
        let now_secs = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        // JWT exp must be in seconds, NOT milliseconds (~1.7e12).
        assert!(
            exp < 10 * now_secs,
            "exp must be seconds since epoch, got {exp} (now_secs={now_secs})"
        );
        assert!(exp > now_secs, "exp must be in the future, got {exp}");
        assert!(
            exp <= now_secs + 120,
            "exp must be within 2 minutes (default MINUTE), got {exp}"
        );
    }

    #[tokio::test]
    async fn decode_refresh_token_rejects_non_refresh_token() {
        init_env();
        // An access token has scope "com.atproto.access", not "com.atproto.refresh".
        // decode_refresh_token must surface that as Err, not panic.
        let access_jwt = create_access_token(CreateTokensOpts {
            did: "did:plc:alice".to_owned(),
            service_did: "did:web:localho.st".to_owned(),
            scope: Some(AuthScope::Access),
            jti: None,
            expires_in: None,
        })
        .unwrap();
        let result = decode_refresh_token(access_jwt);
        let err = match result {
            Ok(_) => panic!("expected scope-rejection error"),
            Err(e) => e,
        };
        assert!(
            err.to_string().contains("not a refresh token"),
            "expected scope-rejection error, got: {err}"
        );
    }

    #[tokio::test]
    async fn decode_refresh_token_rejects_garbage() {
        init_env();
        // A string that isn't a JWT at all must produce Err, not panic.
        let result = decode_refresh_token("not-a-jwt".to_owned());
        assert!(result.is_err(), "expected Err for non-JWT input");
    }
}
