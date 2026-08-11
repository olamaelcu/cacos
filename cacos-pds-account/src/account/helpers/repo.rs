//! Repo helper: per-DID repo_root updates (cid/rev/indexedAt).
//!
//! `update_root` writes the (cid, rev) for a DID's repo with a guard against
//! regressions. The semantic is: if no existing row, INSERT; if
//! `existing.rev <= new.rev`, UPDATE (an equal-rev retry is an idempotent
//! no-op); otherwise reject as a regression. The rev ordering is the
//! atproto MST revision string (lexicographically ordered); a true
//! regression (`existing.rev > new.rev`) means a stale writer is trying
//! to clobber a newer winner and must not be allowed.

use crate::account::helpers::sql;
use anyhow::{Error, Result};
use lexicon_cid::Cid;
use sea_orm::{ConnectionTrait, DatabaseConnection, QueryResult, TransactionTrait, Value};
use thiserror::Error;

/// Errors specific to repo-root bookkeeping.
#[derive(Error, Debug)]
pub enum RepoHelperError {
    /// A retry attempted to write a `rev` that is strictly less than the
    /// already-persisted one. The persisted rev is the winner, and the
    /// loser's write was rejected so it cannot clobber state.
    #[error("RevRegression: existing rev `{existing}` rejected attempted `{attempted}`")]
    RevRegression { existing: String, attempted: String },
}

pub async fn update_root(
    did: String,
    cid: Cid,
    rev: String,
    db: &DatabaseConnection,
) -> Result<()> {
    let now = rsky_common::now();
    let cid = cid.to_string();

    let tx = db.begin().await?;
    let existing_rev: Option<String> = {
        let row: Option<QueryResult> = tx
            .query_one_raw(sql(
                "SELECT rev FROM repo_root WHERE did = ?1",
                vec![Value::from(did.clone())],
            ))
            .await?;
        row.map(|r| r.try_get_by_index::<String>(0)).transpose()?
    };

    let (sql_text, values) = match existing_rev {
        None => (
            "INSERT INTO repo_root (did, cid, rev, \"indexedAt\") \
             VALUES (?1, ?2, ?3, ?4)",
            vec![
                Value::from(did),
                Value::from(cid),
                Value::from(rev),
                Value::from(now),
            ],
        ),
        Some(existing) if existing.as_str() <= rev.as_str() => (
            "UPDATE repo_root SET cid = ?2, rev = ?3, \"indexedAt\" = ?4 WHERE did = ?1",
            vec![
                Value::from(did),
                Value::from(cid),
                Value::from(rev),
                Value::from(now),
            ],
        ),
        Some(existing) => {
            tx.rollback().await?;
            return Err(Error::new(RepoHelperError::RevRegression {
                existing,
                attempted: rev,
            }));
        }
    };

    tx.execute_raw(sql(sql_text, values)).await?;
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::test_util::TEST_CID;
    use crate::account::test_util::test_db;
    use std::str::FromStr;

    fn test_cid() -> Cid {
        Cid::from_str(TEST_CID).unwrap()
    }

    async fn read_rev(db: &DatabaseConnection, did: &str) -> Option<String> {
        let row: Option<QueryResult> = db
            .query_one_raw(sql(
                "SELECT rev FROM repo_root WHERE did = ?1",
                vec![Value::from(did.to_owned())],
            ))
            .await
            .unwrap();
        row.map(|r| r.try_get_by_index::<String>(0).unwrap())
    }

    #[tokio::test]
    async fn update_root_rejects_rev_regression() {
        let (_dir, db) = test_db().await;
        let did = "did:plc:repo-root".to_owned();
        let cid = test_cid();

        // Strictly increasing rev writes succeed.
        update_root(did.clone(), cid.clone(), "rev-1".to_owned(), &db)
            .await
            .unwrap();
        assert_eq!(read_rev(&db, &did).await, Some("rev-1".to_owned()));

        update_root(did.clone(), cid.clone(), "rev-2".to_owned(), &db)
            .await
            .unwrap();
        assert_eq!(read_rev(&db, &did).await, Some("rev-2".to_owned()));

        // A retry at a lower rev must be rejected, not silently clobbered.
        let err = update_root(did.clone(), cid.clone(), "rev-1".to_owned(), &db)
            .await
            .unwrap_err();
        let downcast = err.downcast_ref::<RepoHelperError>();
        assert!(
            matches!(downcast, Some(RepoHelperError::RevRegression { .. })),
            "expected RevRegression, got {:?}",
            err
        );
        // The winner's rev is preserved.
        assert_eq!(read_rev(&db, &did).await, Some("rev-2".to_owned()));

        // An equal-rev retry is idempotent: the row already has this rev,
        // so the no-op UPDATE succeeds and does not clobber state.
        update_root(did.clone(), cid.clone(), "rev-2".to_owned(), &db)
            .await
            .unwrap();
        assert_eq!(read_rev(&db, &did).await, Some("rev-2".to_owned()));

        // A stale-rev write after a newer rev is in the row is still a
        // regression: the existing rev must beat the attempted one.
        let err = update_root(did.clone(), cid, "rev-1".to_owned(), &db)
            .await
            .unwrap_err();
        assert!(
            matches!(
                err.downcast_ref::<RepoHelperError>(),
                Some(RepoHelperError::RevRegression { .. })
            ),
            "expected RevRegression on stale retry, got {:?}",
            err
        );
        assert_eq!(read_rev(&db, &did).await, Some("rev-2".to_owned()));
    }
}
