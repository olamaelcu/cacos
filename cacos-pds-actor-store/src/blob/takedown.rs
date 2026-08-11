//! Takedown/quarantine transactor and status reporter.

use super::types::BlobReader;
use anyhow::Result;
use lexicon_cid::Cid;
use rsky_common::now;
use rsky_lexicon::com::atproto::admin::StatusAttr;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

impl BlobReader {
    pub async fn get_blob_takedown_status(&self, cid: Cid) -> Result<Option<StatusAttr>> {
        let stmt = Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT \"takedownRef\" FROM blob WHERE cid = ?",
            vec![cid.to_string().into()],
        );
        let res: Option<Option<String>> = match self.db.query_one_raw(stmt).await? {
            Some(row) => Some(row.try_get_by_index::<Option<String>>(0)?),
            None => None,
        };
        match res {
            None => Ok(None),
            Some(Some(takedown_ref)) => Ok(Some(StatusAttr {
                applied: true,
                r#ref: Some(takedown_ref),
            })),
            Some(None) => Ok(Some(StatusAttr {
                applied: false,
                r#ref: None,
            })),
        }
    }

    // Transactors
    // -------------------

    pub async fn update_blob_takedown_status(&self, blob: Cid, takedown: StatusAttr) -> Result<()> {
        let takedown_ref: Option<String> = match takedown.applied {
            true => match takedown.r#ref {
                Some(takedown_ref) => Some(takedown_ref),
                None => Some(now()),
            },
            false => None,
        };
        self.db
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "UPDATE blob SET \"takedownRef\" = ? WHERE cid = ?",
                vec![takedown_ref.into(), blob.to_string().into()],
            ))
            .await?;

        let res = match takedown.applied {
            true => self.blobstore.quarantine(blob).await,
            false => self.blobstore.unquarantine(blob).await,
        };
        if let Err(err) = res {
            tracing::error!(?err, cid = %blob, "could not update blob takedown status in blobstore");
        }
        Ok(())
    }
}
