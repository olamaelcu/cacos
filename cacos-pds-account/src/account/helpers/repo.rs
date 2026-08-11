//! Repo helper: per-DID repo_root updates (cid/rev/indexedAt).

use crate::account::helpers::sql;
use anyhow::Result;
use lexicon_cid::Cid;
use sea_orm::{ConnectionTrait, DatabaseConnection, Value};

pub async fn update_root(
    did: String,
    cid: Cid,
    rev: String,
    db: &DatabaseConnection,
) -> Result<()> {
    // @TODO balance risk of a race in the case of a long retry
    let now = rsky_common::now();
    let cid = cid.to_string();

    db.execute_raw(sql(
        "INSERT INTO repo_root (did, cid, rev, \"indexedAt\") \
         VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT (did) DO UPDATE SET cid = excluded.cid, rev = excluded.rev",
        vec![
            Value::from(did),
            Value::from(cid),
            Value::from(rev),
            Value::from(now),
        ],
    ))
    .await?;
    Ok(())
}
