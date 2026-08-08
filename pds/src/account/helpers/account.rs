//! Account helper: actors, accounts, handle/email/takedown/deactivation lifecycle.
//! All SQL goes through sea-orm's raw `Statement` builder.

use crate::account::helpers::sql;
use anyhow::Result;
use chrono::DateTime;
use chrono::offset::Utc as UtcOffset;
use rsky_common::RFC3339_VARIANT;
use rsky_lexicon::com::atproto::admin::StatusAttr;
use sea_orm::{ConnectionTrait, DatabaseConnection, QueryResult, TransactionTrait, Value};
use serde::{Deserialize, Serialize};
use std::time::SystemTime;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AccountHelperError {
    #[error("UserAlreadyExistsError")]
    UserAlreadyExistsError,
}

pub struct AvailabilityFlags {
    pub include_taken_down: Option<bool>,
    pub include_deactivated: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub enum AccountStatus {
    Active,
    Takendown,
    Suspended,
    Deleted,
    Deactivated,
    Desynchronized,
    Throttled,
}

impl From<AccountStatus>
    for (
        bool,
        Option<rsky_lexicon::com::atproto::sync::AccountStatus>,
    )
{
    fn from(s: AccountStatus) -> Self {
        use rsky_lexicon::com::atproto::sync::AccountStatus as Lex;
        match s {
            AccountStatus::Active => (true, None),
            AccountStatus::Deactivated => (false, Some(Lex::Deactivated)),
            AccountStatus::Takendown => (false, Some(Lex::Takendown)),
            AccountStatus::Suspended => (false, Some(Lex::Suspended)),
            AccountStatus::Deleted => (false, Some(Lex::Deleted)),
            AccountStatus::Desynchronized => (false, None),
            AccountStatus::Throttled => (true, Some(Lex::Throttled)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct FormattedAccountStatus {
    pub active: bool,
    pub status: Option<AccountStatus>,
}

#[derive(Debug)]
pub struct GetAccountAdminStatusOutput {
    pub takedown: StatusAttr,
    pub deactivated: StatusAttr,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActorAccount {
    pub did: String,
    pub handle: Option<String>,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "takedownRef")]
    pub takedown_ref: Option<String>,
    #[serde(rename = "deactivatedAt")]
    pub deactivated_at: Option<String>,
    #[serde(rename = "deleteAfter")]
    pub delete_after: Option<String>,
    pub email: Option<String>,
    #[serde(rename = "invitesDisabled")]
    pub invites_disabled: Option<i16>,
    #[serde(rename = "emailConfirmedAt")]
    pub email_confirmed_at: Option<String>,
}

const SELECT_ACTOR_ACCOUNT: &str = "\
    SELECT actor.did, actor.handle, actor.\"createdAt\", actor.\"takedownRef\", \
    actor.\"deactivatedAt\", actor.\"deleteAfter\", \
    account.email, account.\"emailConfirmedAt\", account.\"invitesDisabled\" \
    FROM actor LEFT JOIN account ON actor.did = account.did";

fn availability_conditions(flags: Option<AvailabilityFlags>) -> String {
    let AvailabilityFlags {
        include_taken_down,
        include_deactivated,
    } = flags.unwrap_or(AvailabilityFlags {
        include_taken_down: Some(false),
        include_deactivated: Some(false),
    });
    let mut conditions = String::new();
    if !include_taken_down.unwrap_or(false) {
        conditions.push_str(" AND actor.\"takedownRef\" IS NULL");
    }
    if !include_deactivated.unwrap_or(false) {
        conditions.push_str(" AND actor.\"deactivatedAt\" IS NULL");
    }
    conditions
}

fn actor_account_from_row(row: &QueryResult) -> Result<ActorAccount, sea_orm::DbErr> {
    Ok(ActorAccount {
        did: row.try_get_by_index(0)?,
        handle: row.try_get_by_index(1)?,
        created_at: row.try_get_by_index(2)?,
        takedown_ref: row.try_get_by_index(3)?,
        deactivated_at: row.try_get_by_index(4)?,
        delete_after: row.try_get_by_index(5)?,
        email: row.try_get_by_index(6)?,
        email_confirmed_at: row.try_get_by_index(7)?,
        invites_disabled: row.try_get_by_index(8)?,
    })
}

/// True when `err` (anyhow-wrapped) is a SQLite constraint violation. sea-orm
/// wraps sqlx's `DatabaseError`; sqlx reports the SQLite *extended* error code,
/// and every constraint-violation extended code has primary code 19 (0x13).
/// Matches the constraint-violation semantics in the rest of the PDS.
pub fn is_unique_violation(err: &anyhow::Error) -> bool {
    err.chain()
        .any(|cause| match cause.downcast_ref::<sea_orm::DbErr>() {
            Some(sea_orm::DbErr::Exec(runtime)) | Some(sea_orm::DbErr::Query(runtime)) => {
                match runtime {
                    sea_orm::RuntimeErr::SqlxError(sqlx_err_arc) => {
                        if let sea_orm::sqlx::Error::Database(db_err) = &**sqlx_err_arc {
                            matches!(
                                db_err.code().as_deref().and_then(|c| c.parse::<u32>().ok()),
                                Some(extended) if extended & 0xff == 19
                            )
                        } else {
                            false
                        }
                    }
                    _ => false,
                }
            }
            _ => false,
        })
}

pub async fn get_account(
    handle_or_did: &str,
    flags: Option<AvailabilityFlags>,
    db: &DatabaseConnection,
) -> Result<Option<ActorAccount>> {
    let handle_or_did = handle_or_did.to_owned();
    let query = format!(
        "{SELECT_ACTOR_ACCOUNT} WHERE {} = ?1{}",
        if handle_or_did.starts_with("did:") {
            "actor.did"
        } else {
            "actor.handle"
        },
        availability_conditions(flags)
    );
    let row: Option<QueryResult> = db
        .query_one_raw(sql(&query, vec![Value::from(handle_or_did)]))
        .await?;
    match row {
        Some(row) => Ok(Some(actor_account_from_row(&row)?)),
        None => Ok(None),
    }
}

pub async fn get_account_by_email(
    email: &str,
    flags: Option<AvailabilityFlags>,
    db: &DatabaseConnection,
) -> Result<Option<ActorAccount>> {
    let email = email.to_lowercase();
    let query = format!(
        "{SELECT_ACTOR_ACCOUNT} WHERE account.email = ?1{}",
        availability_conditions(flags)
    );
    let row: Option<QueryResult> = db
        .query_one_raw(sql(&query, vec![Value::from(email)]))
        .await?;
    match row {
        Some(row) => Ok(Some(actor_account_from_row(&row)?)),
        None => Ok(None),
    }
}

pub async fn register_actor(
    did: String,
    handle: String,
    deactivated: Option<bool>,
    db: &DatabaseConnection,
) -> Result<()> {
    let system_time = SystemTime::now();
    let dt: DateTime<UtcOffset> = system_time.into();
    let created_at = format!("{}", dt.format(RFC3339_VARIANT));
    let deactivate_at = match deactivated {
        Some(true) => Some(created_at.clone()),
        _ => None,
    };
    // `deleteAfter` is intentionally left null at registration: deactivated
    // accounts created at signup are not scheduled for automatic deletion.
    // An explicit `deleteAfter` is only stamped via the admin deactivate path
    // (`AccountManager::deactivate_account(did, delete_after)`).
    let delete_after: Option<String> = None;

    let registered: Option<QueryResult> = db
        .query_one_raw(sql(
            "INSERT INTO actor (did, handle, \"createdAt\", \"deactivatedAt\", \"deleteAfter\") \
             VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT (did) DO NOTHING \
             RETURNING did",
            vec![
                Value::from(did),
                Value::from(handle),
                Value::from(created_at),
                Value::from(deactivate_at),
                Value::from(delete_after),
            ],
        ))
        .await?;
    match registered {
        Some(_) => Ok(()),
        None => Err(anyhow::Error::new(
            AccountHelperError::UserAlreadyExistsError,
        )),
    }
}

pub async fn register_account(
    did: String,
    email: String,
    password: String,
    db: &DatabaseConnection,
) -> Result<()> {
    let created_at = rsky_common::now();

    // @TODO record recovery key for bring your own recovery key
    let registered: Option<QueryResult> = db
        .query_one_raw(sql(
            "INSERT INTO account (did, email, password, \"createdAt\") \
             VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT (did) DO NOTHING \
             RETURNING did",
            vec![
                Value::from(did),
                Value::from(email),
                Value::from(password),
                Value::from(created_at),
            ],
        ))
        .await?;
    match registered {
        Some(_) => Ok(()),
        None => Err(anyhow::Error::new(
            AccountHelperError::UserAlreadyExistsError,
        )),
    }
}

pub async fn delete_account(did: &str, db: &DatabaseConnection) -> Result<()> {
    let tx = db.begin().await?;
    tx.execute_raw(sql(
        "DELETE FROM repo_root WHERE did = ?1",
        vec![Value::from(did.to_owned())],
    ))
    .await?;
    tx.execute_raw(sql(
        "DELETE FROM email_token WHERE did = ?1",
        vec![Value::from(did.to_owned())],
    ))
    .await?;
    tx.execute_raw(sql(
        "DELETE FROM refresh_token WHERE did = ?1",
        vec![Value::from(did.to_owned())],
    ))
    .await?;
    tx.execute_raw(sql(
        "DELETE FROM account WHERE did = ?1",
        vec![Value::from(did.to_owned())],
    ))
    .await?;
    tx.execute_raw(sql(
        "DELETE FROM actor WHERE did = ?1",
        vec![Value::from(did.to_owned())],
    ))
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn update_account_takedown_status(
    did: &str,
    takedown: StatusAttr,
    db: &DatabaseConnection,
) -> Result<()> {
    let takedown_ref: Option<String> = match takedown.applied {
        true => match takedown.r#ref {
            Some(takedown_ref) => Some(takedown_ref),
            None => Some(rsky_common::now()),
        },
        false => None,
    };
    db.execute_raw(sql(
        "UPDATE actor SET \"takedownRef\" = ?1 WHERE did = ?2",
        vec![Value::from(takedown_ref), Value::from(did.to_owned())],
    ))
    .await?;
    Ok(())
}

pub async fn deactivate_account(
    did: &str,
    delete_after: Option<String>,
    db: &DatabaseConnection,
) -> Result<()> {
    let deactivated_at = rsky_common::now();
    db.execute_raw(sql(
        "UPDATE actor SET \"deactivatedAt\" = ?1, \"deleteAfter\" = ?2 WHERE did = ?3",
        vec![
            Value::from(deactivated_at),
            Value::from(delete_after),
            Value::from(did.to_owned()),
        ],
    ))
    .await?;
    Ok(())
}

pub async fn activate_account(did: &str, db: &DatabaseConnection) -> Result<()> {
    db.execute_raw(sql(
        "UPDATE actor SET \"deactivatedAt\" = NULL, \"deleteAfter\" = NULL WHERE did = ?1",
        vec![Value::from(did.to_owned())],
    ))
    .await?;
    Ok(())
}

pub async fn update_email(did: &str, email: &str, db: &DatabaseConnection) -> Result<()> {
    let res = db
        .execute_raw(sql(
            "UPDATE account SET email = ?1, \"emailConfirmedAt\" = NULL WHERE did = ?2",
            vec![
                Value::from(email.to_lowercase()),
                Value::from(did.to_owned()),
            ],
        ))
        .await;
    match res {
        Ok(_) => Ok(()),
        Err(err) => {
            let any_err = anyhow::Error::new(err);
            if is_unique_violation(&any_err) {
                Err(anyhow::Error::new(
                    AccountHelperError::UserAlreadyExistsError,
                ))
            } else {
                Err(any_err)
            }
        }
    }
}

pub async fn update_handle(did: &str, handle: &str, db: &DatabaseConnection) -> Result<()> {
    let updated = db
        .execute_raw(sql(
            "UPDATE actor SET handle = ?1 \
             WHERE did = ?2 \
             AND NOT EXISTS (SELECT 1 FROM actor actor2 WHERE actor2.handle = ?1)",
            vec![Value::from(handle.to_owned()), Value::from(did.to_owned())],
        ))
        .await?;
    if updated.rows_affected() < 1 {
        return Err(anyhow::Error::new(
            AccountHelperError::UserAlreadyExistsError,
        ));
    }
    Ok(())
}

pub async fn set_email_confirmed_at(
    did: &str,
    email_confirmed_at: String,
    db: &DatabaseConnection,
) -> Result<()> {
    db.execute_raw(sql(
        "UPDATE account SET \"emailConfirmedAt\" = ?1 WHERE did = ?2",
        vec![Value::from(email_confirmed_at), Value::from(did.to_owned())],
    ))
    .await?;
    Ok(())
}

pub async fn get_account_admin_status(
    did: &str,
    db: &DatabaseConnection,
) -> Result<Option<GetAccountAdminStatusOutput>> {
    let row: Option<QueryResult> = db
        .query_one_raw(sql(
            "SELECT \"takedownRef\", \"deactivatedAt\" FROM actor WHERE did = ?1",
            vec![Value::from(did.to_owned())],
        ))
        .await?;
    match row {
        None => Ok(None),
        Some(row) => {
            let takedown_ref: Option<String> = row.try_get_by_index(0)?;
            let deactivated_at: Option<String> = row.try_get_by_index(1)?;
            let takedown = match takedown_ref {
                Some(takedown_ref) => StatusAttr {
                    applied: true,
                    r#ref: Some(takedown_ref),
                },
                None => StatusAttr {
                    applied: false,
                    r#ref: None,
                },
            };
            let deactivated = match deactivated_at {
                Some(_) => StatusAttr {
                    applied: true,
                    r#ref: None,
                },
                None => StatusAttr {
                    applied: false,
                    r#ref: None,
                },
            };
            Ok(Some(GetAccountAdminStatusOutput {
                takedown,
                deactivated,
            }))
        }
    }
}

pub fn format_account_status(account: Option<ActorAccount>) -> FormattedAccountStatus {
    match account {
        None => FormattedAccountStatus {
            active: false,
            status: Some(AccountStatus::Deleted),
        },
        Some(got) if got.takedown_ref.is_some() => FormattedAccountStatus {
            active: false,
            status: Some(AccountStatus::Takendown),
        },
        Some(got) if got.deactivated_at.is_some() => FormattedAccountStatus {
            active: false,
            status: Some(AccountStatus::Deactivated),
        },
        _ => FormattedAccountStatus {
            active: true,
            status: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::helpers::sql;
    use crate::account::test_util::test_db;
    use sea_orm::ConnectionTrait;

    async fn register_full(db: &DatabaseConnection, did: &str, handle: &str) {
        register_actor(did.to_owned(), handle.to_owned(), None, db)
            .await
            .unwrap();
        register_account(
            did.to_owned(),
            format!("{handle}@example.com"),
            "phc-hash".to_owned(),
            db,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn registers_and_fetches_by_did_handle_email() {
        let (_dir, db) = test_db().await;
        register_full(&db, "did:plc:alice", "alice.test").await;

        let by_did = get_account("did:plc:alice", None, &db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(by_did.handle, Some("alice.test".to_owned()));
        assert_eq!(by_did.email, Some("alice.test@example.com".to_owned()));
        assert!(by_did.created_at.ends_with('Z'));

        let by_handle = get_account("alice.test", None, &db).await.unwrap().unwrap();
        assert_eq!(by_handle.did, "did:plc:alice");

        // email lookup is case-insensitive
        let by_email = get_account_by_email("ALICE.TEST@example.com", None, &db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(by_email.did, "did:plc:alice");
        assert!(
            get_account_by_email("missing@example.com", None, &db)
                .await
                .unwrap()
                .is_none()
        );
        assert!(get_account("nope.test", None, &db).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn register_actor_rejects_duplicate_did() {
        let (_dir, db) = test_db().await;
        register_full(&db, "did:plc:dup", "dup.test").await;
        let err = register_actor("did:plc:dup".to_owned(), "dup2.test".to_owned(), None, &db)
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "UserAlreadyExistsError");
    }

    #[tokio::test]
    async fn register_account_rejects_duplicate_did() {
        let (_dir, db) = test_db().await;
        register_full(&db, "did:plc:dup", "dup.test").await;
        let err = register_account(
            "did:plc:dup".to_owned(),
            "dup2@example.com".to_owned(),
            "hash".to_owned(),
            &db,
        )
        .await
        .unwrap_err();
        assert_eq!(err.to_string(), "UserAlreadyExistsError");
    }

    #[tokio::test]
    async fn register_actor_with_deactivated_flag() {
        let (_dir, db) = test_db().await;
        register_actor(
            "did:plc:ghost".to_owned(),
            "ghost.test".to_owned(),
            Some(true),
            &db,
        )
        .await
        .unwrap();
        // hidden by default
        assert!(
            get_account("did:plc:ghost", None, &db)
                .await
                .unwrap()
                .is_none()
        );
        let got = get_account(
            "did:plc:ghost",
            Some(AvailabilityFlags {
                include_taken_down: None,
                include_deactivated: Some(true),
            }),
            &db,
        )
        .await
        .unwrap()
        .unwrap();
        assert!(got.deactivated_at.is_some());
        assert!(got.delete_after.is_none());
    }

    #[tokio::test]
    async fn delete_account_cascades() {
        let (_dir, db) = test_db().await;
        register_full(&db, "did:plc:erin", "erin.test").await;
        delete_account("did:plc:erin", &db).await.unwrap();
        assert!(
            get_account(
                "did:plc:erin",
                Some(AvailabilityFlags {
                    include_taken_down: Some(true),
                    include_deactivated: Some(true),
                }),
                &db,
            )
            .await
            .unwrap()
            .is_none()
        );
        // repo_root row is gone too (the DELETE transaction cascades)
        let row: Option<QueryResult> = db
            .query_one_raw(sql(
                "SELECT did FROM repo_root WHERE did = ?1",
                vec![Value::from("did:plc:erin")],
            ))
            .await
            .unwrap();
        assert!(row.is_none());
    }

    #[tokio::test]
    async fn update_handle_rejects_taken_handle() {
        let (_dir, db) = test_db().await;
        register_full(&db, "did:plc:carol", "carol.test").await;
        register_full(&db, "did:plc:dan", "dan.test").await;
        update_handle("did:plc:carol", "carol2.test", &db)
            .await
            .unwrap();
        assert!(
            get_account("carol2.test", None, &db)
                .await
                .unwrap()
                .is_some()
        );
        let err = update_handle("did:plc:carol", "dan.test", &db)
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "UserAlreadyExistsError");
    }

    #[tokio::test]
    async fn update_email_lowercases_and_rejects_duplicates() {
        let (_dir, db) = test_db().await;
        register_full(&db, "did:plc:carol", "carol.test").await;
        register_full(&db, "did:plc:dan", "dan.test").await;
        update_email("did:plc:carol", "Carol.New@example.com", &db)
            .await
            .unwrap();
        let got = get_account("did:plc:carol", None, &db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got.email, Some("carol.new@example.com".to_owned()));
        assert_eq!(got.email_confirmed_at, None);
        let err = update_email("did:plc:carol", "dan.test@example.com", &db)
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "UserAlreadyExistsError");
    }

    #[tokio::test]
    async fn deactivate_and_activate_flip_availability() {
        let (_dir, db) = test_db().await;
        register_full(&db, "did:plc:bob", "bob.test").await;
        deactivate_account("did:plc:bob", Some(rsky_common::now()), &db)
            .await
            .unwrap();
        assert!(
            get_account("did:plc:bob", None, &db)
                .await
                .unwrap()
                .is_none()
        );
        let got = get_account(
            "did:plc:bob",
            Some(AvailabilityFlags {
                include_taken_down: None,
                include_deactivated: Some(true),
            }),
            &db,
        )
        .await
        .unwrap()
        .unwrap();
        assert!(got.deactivated_at.is_some());
        activate_account("did:plc:bob", &db).await.unwrap();
        assert!(
            get_account("did:plc:bob", None, &db)
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn takedown_status_and_admin_status() {
        let (_dir, db) = test_db().await;
        register_full(&db, "did:plc:bob", "bob.test").await;

        let admin = get_account_admin_status("did:plc:bob", &db)
            .await
            .unwrap()
            .unwrap();
        assert!(!admin.takedown.applied);
        assert!(!admin.deactivated.applied);
        assert!(
            get_account_admin_status("did:plc:missing", &db)
                .await
                .unwrap()
                .is_none()
        );

        update_account_takedown_status(
            "did:plc:bob",
            StatusAttr {
                applied: true,
                r#ref: Some("mod-action-1".to_owned()),
            },
            &db,
        )
        .await
        .unwrap();
        // taken down accounts are hidden by default
        assert!(
            get_account("did:plc:bob", None, &db)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            get_account(
                "did:plc:bob",
                Some(AvailabilityFlags {
                    include_taken_down: Some(true),
                    include_deactivated: None,
                }),
                &db,
            )
            .await
            .unwrap()
            .is_some()
        );
        let admin = get_account_admin_status("did:plc:bob", &db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(admin.takedown.r#ref, Some("mod-action-1".to_owned()));

        // applied with no ref falls back to a timestamp
        update_account_takedown_status(
            "did:plc:bob",
            StatusAttr {
                applied: true,
                r#ref: None,
            },
            &db,
        )
        .await
        .unwrap();
        let admin = get_account_admin_status("did:plc:bob", &db)
            .await
            .unwrap()
            .unwrap();
        assert!(admin.takedown.r#ref.is_some());
        // reversing clears it
        update_account_takedown_status(
            "did:plc:bob",
            StatusAttr {
                applied: false,
                r#ref: None,
            },
            &db,
        )
        .await
        .unwrap();
        let admin = get_account_admin_status("did:plc:bob", &db)
            .await
            .unwrap()
            .unwrap();
        assert!(!admin.takedown.applied);
    }

    #[test]
    fn formats_account_status() {
        assert_eq!(
            format_account_status(None).status,
            Some(AccountStatus::Deleted)
        );
        let base = ActorAccount {
            did: "did:plc:x".to_owned(),
            handle: None,
            created_at: rsky_common::now(),
            takedown_ref: None,
            deactivated_at: None,
            delete_after: None,
            email: None,
            invites_disabled: None,
            email_confirmed_at: None,
        };
        assert!(format_account_status(Some(base.clone())).active);
        assert_eq!(
            format_account_status(Some(ActorAccount {
                takedown_ref: Some("ref".to_owned()),
                ..base.clone()
            }))
            .status,
            Some(AccountStatus::Takendown)
        );
        assert_eq!(
            format_account_status(Some(ActorAccount {
                deactivated_at: Some(rsky_common::now()),
                ..base
            }))
            .status,
            Some(AccountStatus::Deactivated)
        );
    }

    #[test]
    fn account_status_into_formatted_lex() {
        use rsky_lexicon::com::atproto::sync::AccountStatus as Lex;

        let (active, status): (bool, Option<Lex>) = AccountStatus::Active.into();
        assert_eq!(active, true);
        assert_eq!(status, None);

        let (active, status): (bool, Option<Lex>) = AccountStatus::Deactivated.into();
        assert_eq!(active, false);
        assert!(matches!(status, Some(Lex::Deactivated)));

        let (active, status): (bool, Option<Lex>) = AccountStatus::Takendown.into();
        assert_eq!(active, false);
        assert!(matches!(status, Some(Lex::Takendown)));

        let (active, status): (bool, Option<Lex>) = AccountStatus::Suspended.into();
        assert_eq!(active, false);
        assert!(matches!(status, Some(Lex::Suspended)));

        let (active, status): (bool, Option<Lex>) = AccountStatus::Deleted.into();
        assert_eq!(active, false);
        assert!(matches!(status, Some(Lex::Deleted)));

        let (active, status): (bool, Option<Lex>) = AccountStatus::Throttled.into();
        assert_eq!(active, true);
        assert!(matches!(status, Some(Lex::Throttled)));

        // Desynchronized doesn't exist in lexicon; map to (active=false, status=None)
        let (active, status): (bool, Option<Lex>) = AccountStatus::Desynchronized.into();
        assert_eq!(active, false);
        assert_eq!(status, None);
    }

    #[tokio::test]
    async fn is_unique_violation_matches_constraint_errors() {
        let (_dir, db) = test_db().await;
        register_full(&db, "did:plc:alice", "alice.test").await;
        // non-constraint errors are not violations
        assert!(!is_unique_violation(&anyhow::anyhow!("boom")));
        // a genuine unique constraint violation is detected
        let res = db
            .execute_raw(sql(
                "INSERT INTO actor (did, handle, \"createdAt\") \
                 VALUES ('did:plc:alice', 'other.test', '2023-01-01T00:00:00.000Z')",
                vec![],
            ))
            .await;
        let err = res.unwrap_err();
        assert!(is_unique_violation(&anyhow::Error::new(err)));
    }
}
