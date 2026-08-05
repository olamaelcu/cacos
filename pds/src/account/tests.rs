//! Integration tests for `AccountManager` (part 1: account + email-token flow).

use super::helpers::account::{
    self, AccountHelperError, AccountStatus, AvailabilityFlags, format_account_status,
};
use super::helpers::{account as account_helper, email_token};
use super::*;
use crate::account::helpers::sql;
use crate::account::test_util::{TEST_CID, test_db};
use lexicon_cid::Cid;
use rsky_lexicon::com::atproto::admin::StatusAttr;
use sea_orm::ConnectionTrait;
use std::str::FromStr;

pub(crate) async fn test_manager() -> (camino_tempfile::Utf8TempDir, AccountManager) {
    let (dir, db) = test_db().await;
    (dir, AccountManager::new(db))
}

fn create_opts(did: &str, handle: &str, invite_code: Option<String>) -> CreateAccountOpts {
    CreateAccountOpts {
        did: did.to_owned(),
        handle: handle.to_owned(),
        email: Some(format!("{handle}@example.com")),
        password: Some("password123".to_owned()),
        repo_cid: Cid::from_str(TEST_CID).unwrap(),
        repo_rev: "3jzfcijpj2z2a".to_owned(),
        invite_code,
        deactivated: None,
    }
}

async fn create_test_account(am: &AccountManager, did: &str, handle: &str) -> (String, String) {
    am.create_account(create_opts(did, handle, None))
        .await
        .unwrap()
}

#[tokio::test]
async fn creates_and_fetches_accounts() {
    let (_dir, am) = test_manager().await;
    let (access, refresh) = create_test_account(&am, "did:plc:alice", "alice.test").await;
    assert!(!access.is_empty());
    assert!(!refresh.is_empty());

    // fetch by did and by handle
    let by_did = am
        .get_account("did:plc:alice", None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(by_did.handle, Some("alice.test".to_owned()));
    let by_handle = am.get_account("alice.test", None).await.unwrap().unwrap();
    assert_eq!(by_handle.did, "did:plc:alice");
    assert_eq!(by_handle.email, Some("alice.test@example.com".to_owned()));

    // fetch by email
    let by_email = am
        .get_account_by_email("ALICE.TEST@example.com", None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(by_email.did, "did:plc:alice");
    assert!(
        am.get_account_by_email("missing@example.com", None)
            .await
            .unwrap()
            .is_none()
    );

    assert!(am.is_account_activated("did:plc:alice").await.unwrap());
    assert!(!am.is_account_activated("did:plc:missing").await.unwrap());

    assert_eq!(
        am.get_did_for_actor("alice.test", None).await.unwrap(),
        Some("did:plc:alice".to_owned())
    );
    assert_eq!(am.get_did_for_actor("nope.test", None).await.unwrap(), None);

    // repo root was recorded and can be updated
    am.update_repo_root(
        "did:plc:alice".to_owned(),
        Cid::from_str(TEST_CID).unwrap(),
        "3jzfcijpj2z2b".to_owned(),
    )
    .await
    .unwrap();

    // duplicate did fails
    let err = am
        .create_account(create_opts("did:plc:alice", "alice2.test", None))
        .await
        .unwrap_err();
    assert_eq!(err.to_string(), "UserAlreadyExistsError");
    // duplicate email fails on the account row
    let err = am
        .create_account(CreateAccountOpts {
            email: Some("alice.test@example.com".to_owned()),
            ..create_opts("did:plc:alice3", "alice3.test", None)
        })
        .await
        .unwrap_err();
    assert!(err.to_string().contains("UNIQUE constraint failed"));
}

#[tokio::test]
async fn creates_account_without_credentials_and_deactivated() {
    let (_dir, am) = test_manager().await;
    am.create_account(CreateAccountOpts {
        email: None,
        password: None,
        deactivated: Some(true),
        ..create_opts("did:plc:ghost", "ghost.test", None)
    })
    .await
    .unwrap();

    // hidden by default because it is deactivated
    assert!(
        am.get_account("did:plc:ghost", None)
            .await
            .unwrap()
            .is_none()
    );
    let got = am
        .get_account(
            "did:plc:ghost",
            Some(AvailabilityFlags {
                include_taken_down: None,
                include_deactivated: Some(true),
            }),
        )
        .await
        .unwrap()
        .unwrap();
    assert!(got.deactivated_at.is_some());
    assert!(got.delete_after.is_some());
    assert_eq!(got.email, None);
    assert!(!am.is_account_activated("did:plc:ghost").await.unwrap());
    assert_eq!(
        am.get_account_status("did:plc:ghost").await.unwrap(),
        AccountStatus::Deactivated
    );

    am.activate_account("did:plc:ghost").await.unwrap();
    assert_eq!(
        am.get_account_status("did:plc:ghost").await.unwrap(),
        AccountStatus::Active
    );
}

#[tokio::test]
async fn account_status_and_admin_status() {
    let (_dir, am) = test_manager().await;
    create_test_account(&am, "did:plc:bob", "bob.test").await;

    assert_eq!(
        am.get_account_status("did:plc:bob").await.unwrap(),
        AccountStatus::Active
    );
    assert_eq!(
        am.get_account_status("did:plc:missing").await.unwrap(),
        AccountStatus::Deleted
    );

    let admin_status = am
        .get_account_admin_status("did:plc:bob")
        .await
        .unwrap()
        .unwrap();
    assert!(!admin_status.takedown.applied);
    assert!(!admin_status.deactivated.applied);
    assert!(
        am.get_account_admin_status("did:plc:missing")
            .await
            .unwrap()
            .is_none()
    );

    // takedown with an explicit ref
    am.takedown_account(
        "did:plc:bob",
        StatusAttr {
            applied: true,
            r#ref: Some("mod-action-1".to_owned()),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        am.get_account_status("did:plc:bob").await.unwrap(),
        AccountStatus::Takendown
    );
    let admin_status = am
        .get_account_admin_status("did:plc:bob")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(admin_status.takedown.r#ref, Some("mod-action-1".to_owned()));
    // taken down accounts are hidden by default
    assert!(am.get_account("did:plc:bob", None).await.unwrap().is_none());
    assert!(
        am.get_account(
            "did:plc:bob",
            Some(AvailabilityFlags {
                include_taken_down: Some(true),
                include_deactivated: None,
            }),
        )
        .await
        .unwrap()
        .is_some()
    );

    // takedown without a ref falls back to a timestamp, then reverse it
    am.takedown_account(
        "did:plc:bob",
        StatusAttr {
            applied: true,
            r#ref: None,
        },
    )
    .await
    .unwrap();
    am.takedown_account(
        "did:plc:bob",
        StatusAttr {
            applied: false,
            r#ref: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        am.get_account_status("did:plc:bob").await.unwrap(),
        AccountStatus::Active
    );

    // deactivation with an explicit deleteAfter
    am.deactivate_account("did:plc:bob", Some(rsky_common::now()))
        .await
        .unwrap();
    let admin_status = am
        .get_account_admin_status("did:plc:bob")
        .await
        .unwrap()
        .unwrap();
    assert!(admin_status.deactivated.applied);
    am.activate_account("did:plc:bob").await.unwrap();
}

#[tokio::test]
async fn formats_account_status() {
    assert_eq!(
        format_account_status(None).status,
        Some(AccountStatus::Deleted)
    );
    let base = account::ActorAccount {
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
        format_account_status(Some(account::ActorAccount {
            takedown_ref: Some("ref".to_owned()),
            ..base.clone()
        }))
        .status,
        Some(AccountStatus::Takendown)
    );
    assert_eq!(
        format_account_status(Some(account::ActorAccount {
            deactivated_at: Some(rsky_common::now()),
            ..base
        }))
        .status,
        Some(AccountStatus::Deactivated)
    );
}

#[tokio::test]
async fn updates_handles_and_emails() {
    let (_dir, am) = test_manager().await;
    create_test_account(&am, "did:plc:carol", "carol.test").await;
    create_test_account(&am, "did:plc:dan", "dan.test").await;

    am.update_handle("did:plc:carol", "carol2.test")
        .await
        .unwrap();
    assert_eq!(
        am.get_did_for_actor("carol2.test", None).await.unwrap(),
        Some("did:plc:carol".to_owned())
    );
    // taking another account's handle fails
    let err = am
        .update_handle("did:plc:carol", "dan.test")
        .await
        .unwrap_err();
    assert_eq!(err.to_string(), "UserAlreadyExistsError");

    am.update_email(UpdateEmailOpts {
        did: "did:plc:carol".to_owned(),
        email: "Carol.New@example.com".to_owned(),
    })
    .await
    .unwrap();
    let got = am
        .get_account("did:plc:carol", None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.email, Some("carol.new@example.com".to_owned()));
    // updating to another account's email fails
    let err = am
        .update_email(UpdateEmailOpts {
            did: "did:plc:carol".to_owned(),
            email: "dan.test@example.com".to_owned(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.to_string(), "UserAlreadyExistsError");
}

#[tokio::test]
async fn deletes_accounts() {
    let (_dir, am) = test_manager().await;
    create_test_account(&am, "did:plc:erin", "erin.test").await;
    am.create_email_token("did:plc:erin", EmailTokenPurpose::ConfirmEmail)
        .await
        .unwrap();
    am.delete_account("did:plc:erin").await.unwrap();
    assert!(
        am.get_account(
            "did:plc:erin",
            Some(AvailabilityFlags {
                include_taken_down: Some(true),
                include_deactivated: Some(true),
            }),
        )
        .await
        .unwrap()
        .is_none()
    );
}

#[tokio::test]
async fn register_account_rejects_duplicate_did() {
    let (_dir, am) = test_manager().await;
    create_test_account(&am, "did:plc:dup", "dup.test").await;
    let err = account_helper::register_account(
        "did:plc:dup".to_owned(),
        "dup2@example.com".to_owned(),
        "hash".to_owned(),
        &am.db,
    )
    .await
    .unwrap_err();
    assert_eq!(err.to_string(), "UserAlreadyExistsError");
}

#[tokio::test]
async fn account_helper_edge_cases() {
    let (_dir, am) = test_manager().await;
    // unique violation detection
    assert!(!account_helper::is_unique_violation(&anyhow::anyhow!(
        "boom"
    )));
    // update_email surfaces non-unique-violation errors as-is
    am.db
        .execute_unprepared("DROP TABLE account")
        .await
        .unwrap();
    let err = account_helper::update_email("did:plc:x", "x@example.com", &am.db)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("no such table"));
    assert!(!matches!(
        err.downcast_ref::<AccountHelperError>(),
        Some(AccountHelperError::UserAlreadyExistsError)
    ));
}

#[tokio::test]
async fn email_token_row_mapping_rejects_unknown_purpose() {
    let (_dir, db) = test_db().await;
    let row: Option<sea_orm::QueryResult> = db
        .query_one_raw(sql(
            "SELECT 'bogus_purpose', 'did:plc:x', 'TOKEN', '2023-01-01T00:00:00.000Z'",
            vec![],
        ))
        .await
        .unwrap();
    let row = row.unwrap();
    assert!(email_token::email_token_from_row(&row).is_err());
}
