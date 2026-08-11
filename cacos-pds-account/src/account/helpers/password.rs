//! Password helper: argon2 hashes, app passwords, account password verification.
//! All SQL goes through sea-orm's raw `Statement` builder.

use crate::account::helpers::sql;
use anyhow::{Result, anyhow, bail};
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use base64ct::{Base64, Encoding};
use rsky_common::{get_random_str, now};
use rsky_lexicon::com::atproto::server::CreateAppPasswordOutput;
use sea_orm::{ConnectionTrait, DatabaseConnection, QueryResult, Value};
use sha2::{Digest, Sha256};

pub struct UpdateUserPasswordOpts {
    pub did: String,
    pub password_encrypted: String,
}

pub async fn verify_account_password(
    did: &str,
    password: &String,
    db: &DatabaseConnection,
) -> Result<bool> {
    let found: Option<String> = db
        .query_one_raw(sql(
            "SELECT password FROM account WHERE did = ?1",
            vec![Value::from(did.to_owned())],
        ))
        .await?
        .map(|row| row.try_get_by_index::<String>(0))
        .transpose()?;
    if let Some(stored_hash) = found {
        verify(password, &stored_hash)
    } else {
        Ok(false)
    }
}

pub async fn verify_app_password(
    did: &str,
    password: &str,
    db: &DatabaseConnection,
) -> Result<Option<String>> {
    let password_encrypted = hash_app_password(&did.to_owned(), &password.to_owned()).await?;
    let found: Option<String> = db
        .query_one_raw(sql(
            "SELECT name FROM app_password WHERE did = ?1 AND password = ?2",
            vec![Value::from(did.to_owned()), Value::from(password_encrypted)],
        ))
        .await?
        .map(|row| row.try_get_by_index::<String>(0))
        .transpose()?;
    Ok(found)
}

// We use Argon because it's 3x faster than scrypt.
pub fn gen_salt_and_hash(password: String) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    // Hash password to PHC string
    let argon2 = Argon2::default();
    let password_hash = argon2
        .hash_password(password.as_ref(), &salt)
        .map_err(|error| anyhow!(error.to_string()))?
        .to_string();
    Ok(password_hash)
}

pub fn hash_with_salt(password: &String, salt: &str) -> Result<String> {
    let salt = SaltString::from_b64(salt).map_err(|error| anyhow!(error.to_string()))?;
    let argon2 = Argon2::default();
    let password_hash = argon2
        .hash_password(password.as_ref(), &salt)
        .map_err(|error| anyhow!(error.to_string()))?
        .to_string();
    Ok(password_hash)
}

pub fn verify(password: &String, stored_hash: &str) -> Result<bool> {
    let parsed_hash = PasswordHash::new(stored_hash).map_err(|error| anyhow!(error.to_string()))?;
    Ok(Argon2::default()
        .verify_password(password.as_ref(), &parsed_hash)
        .is_ok())
}

pub async fn hash_app_password(did: &String, password: &String) -> Result<String> {
    let hash = Sha256::digest(did);
    let salt = Base64::encode_string(&hash).replace("=", "");
    hash_with_salt(password, &salt)
}

/// create an app password with format:
/// 1234-abcd-5678-efgh
pub async fn create_app_password(
    did: String,
    name: String,
    db: &DatabaseConnection,
) -> Result<CreateAppPasswordOutput> {
    let str = &get_random_str()[0..16].to_lowercase();
    let chunks = [&str[0..4], &str[4..8], &str[8..12], &str[12..16]];
    let password = chunks.join("-");
    let password_encrypted = hash_app_password(&did, &password).await?;

    let created_at = now();

    let got: Option<QueryResult> = db
        .query_one_raw(sql(
            "INSERT INTO app_password (did, name, password, \"createdAt\") \
             VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT (did, name) DO NOTHING \
             RETURNING name",
            vec![
                Value::from(did),
                Value::from(name.clone()),
                Value::from(password_encrypted),
                Value::from(created_at.clone()),
            ],
        ))
        .await?;
    if got.is_some() {
        Ok(CreateAppPasswordOutput {
            name,
            password,
            created_at,
        })
    } else {
        bail!("could not create app-specific password")
    }
}

pub async fn list_app_passwords(
    did: &str,
    db: &DatabaseConnection,
) -> Result<Vec<(String, String)>> {
    let rows: Vec<QueryResult> = db
        .query_all_raw(sql(
            "SELECT name, \"createdAt\" FROM app_password WHERE did = ?1",
            vec![Value::from(did.to_owned())],
        ))
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push((
            row.try_get_by_index::<String>(0)?,
            row.try_get_by_index::<String>(1)?,
        ));
    }
    Ok(out)
}

pub async fn update_user_password(
    opts: UpdateUserPasswordOpts,
    db: &DatabaseConnection,
) -> Result<()> {
    db.execute_raw(sql(
        "UPDATE account SET password = ?1 WHERE did = ?2",
        vec![Value::from(opts.password_encrypted), Value::from(opts.did)],
    ))
    .await?;
    Ok(())
}

pub async fn delete_app_password(did: &str, name: &str, db: &DatabaseConnection) -> Result<()> {
    db.execute_raw(sql(
        "DELETE FROM app_password WHERE did = ?1 AND name = ?2",
        vec![Value::from(did.to_owned()), Value::from(name.to_owned())],
    ))
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::helpers::account::{register_account, register_actor};
    use crate::account::test_util::test_db;

    #[test]
    fn gen_and_verify_roundtrip() {
        let hash = gen_salt_and_hash("secret".to_owned()).unwrap();
        assert!(verify(&"secret".to_owned(), &hash).unwrap());
        assert!(!verify(&"other".to_owned(), &hash).unwrap());
        assert!(verify(&"secret".to_owned(), "not-a-phc-hash").is_err());
        assert!(hash_with_salt(&"secret".to_owned(), "!invalid salt!").is_err());
    }

    #[tokio::test]
    async fn app_password_format_and_deterministic_salt() {
        let (_dir, db) = test_db().await;
        register_actor(
            "did:plc:grace".to_owned(),
            "grace.test".to_owned(),
            None,
            &db,
        )
        .await
        .unwrap();
        register_account(
            "did:plc:grace".to_owned(),
            "grace.test@example.com".to_owned(),
            "phc-hash".to_owned(),
            None,
            &db,
        )
        .await
        .unwrap();

        let created = create_app_password("did:plc:grace".to_owned(), "My App".to_owned(), &db)
            .await
            .unwrap();
        // format xxxx-xxxx-xxxx-xxxx: 16 alnum chars in 4 groups of 4
        assert_eq!(created.password.len(), 19);
        let chars: Vec<char> = created.password.chars().collect();
        assert_eq!(chars[4], '-');
        assert_eq!(chars[9], '-');
        assert_eq!(chars[14], '-');
        assert!(
            created
                .password
                .chars()
                .filter(|c| *c != '-')
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        );

        // the per-did salt makes the hash deterministic per did
        let once = hash_app_password(&"did:plc:grace".to_owned(), &created.password)
            .await
            .unwrap();
        let twice = hash_app_password(&"did:plc:grace".to_owned(), &created.password)
            .await
            .unwrap();
        assert_eq!(once, twice);
        let other = hash_app_password(&"did:plc:someone-else".to_owned(), &created.password)
            .await
            .unwrap();
        assert_ne!(once, other);
    }

    #[tokio::test]
    async fn create_app_password_rejects_duplicate_name() {
        let (_dir, db) = test_db().await;
        create_app_password("did:plc:grace".to_owned(), "My App".to_owned(), &db)
            .await
            .unwrap();
        let err = create_app_password("did:plc:grace".to_owned(), "My App".to_owned(), &db)
            .await
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("could not create app-specific password")
        );
    }

    #[tokio::test]
    async fn list_and_verify_app_passwords() {
        let (_dir, db) = test_db().await;
        let created = create_app_password("did:plc:grace".to_owned(), "My App".to_owned(), &db)
            .await
            .unwrap();
        let listed = list_app_passwords("did:plc:grace", &db).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].0, "My App");

        assert_eq!(
            verify_app_password("did:plc:grace", &created.password, &db)
                .await
                .unwrap(),
            Some("My App".to_owned())
        );
        assert_eq!(
            verify_app_password("did:plc:grace", "1111-2222-3333-4444", &db)
                .await
                .unwrap(),
            None
        );
        // a wrong did never matches (deterministic per-did salt)
        assert_eq!(
            verify_app_password("did:plc:nobody", &created.password, &db)
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn update_user_password_changes_verification() {
        let (_dir, db) = test_db().await;
        // Reference upstream uses create_test_account; the plan's helper-level
        // test omits the register calls, which makes the `account` row missing
        // and the verify assertion below unreachable. Insert both rows here.
        register_actor(
            "did:plc:grace".to_owned(),
            "grace.test".to_owned(),
            None,
            &db,
        )
        .await
        .unwrap();
        register_account(
            "did:plc:grace".to_owned(),
            "grace.test@example.com".to_owned(),
            gen_salt_and_hash("oldsecret".to_owned()).unwrap(),
            None,
            &db,
        )
        .await
        .unwrap();
        update_user_password(
            UpdateUserPasswordOpts {
                did: "did:plc:grace".to_owned(),
                password_encrypted: gen_salt_and_hash("newsecret".to_owned()).unwrap(),
            },
            &db,
        )
        .await
        .unwrap();
        assert!(
            verify_account_password("did:plc:grace", &"newsecret".to_owned(), &db,)
                .await
                .unwrap()
        );
        assert!(
            !verify_account_password("did:plc:grace", &"oldsecret".to_owned(), &db)
                .await
                .unwrap()
        );
        // missing account -> false (never an error)
        assert!(
            !verify_account_password("did:plc:missing", &"newsecret".to_owned(), &db)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn delete_app_password_removes_row() {
        let (_dir, db) = test_db().await;
        let created = create_app_password("did:plc:grace".to_owned(), "My App".to_owned(), &db)
            .await
            .unwrap();
        delete_app_password("did:plc:grace", "My App", &db)
            .await
            .unwrap();
        assert!(
            list_app_passwords("did:plc:grace", &db)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            verify_app_password("did:plc:grace", &created.password, &db)
                .await
                .unwrap(),
            None
        );
    }
}
