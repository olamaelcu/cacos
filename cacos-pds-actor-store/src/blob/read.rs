//! Read-only blob queries (single-row lookups, counts, and paged listing).

use super::types::{BLOB_SELECT, BlobReader, row_to_blob};
use anyhow::{Result, bail};
use lexicon_cid::Cid;
use rsky_lexicon::com::atproto::repo::ListMissingBlobsRefRecordBlob;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement, Value};
use std::str::FromStr;

impl BlobReader {
    pub async fn get_blob_metadata(&self, cid: Cid) -> Result<super::types::GetBlobMetadataOutput> {
        let stmt = Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            format!("{BLOB_SELECT} WHERE cid = ? AND \"takedownRef\" IS NULL"),
            vec![cid.to_string().into()],
        );
        let found = match self.db.query_one_raw(stmt).await? {
            Some(row) => Some(row_to_blob(&row)?),
            None => None,
        };
        match found {
            None => bail!("Blob not found"),
            Some(found) => Ok(super::types::GetBlobMetadataOutput {
                size: found.size,
                mime_type: Some(found.mime_type),
            }),
        }
    }

    pub async fn get_blob(&self, cid: Cid) -> Result<super::types::GetBlobOutput> {
        let metadata = self.get_blob_metadata(cid).await?;
        let blob_stream = self.blobstore.get_stream(cid).await?;
        Ok(super::types::GetBlobOutput {
            size: metadata.size,
            mime_type: metadata.mime_type,
            stream: blob_stream,
        })
    }

    pub async fn get_records_for_blob(&self, cid: Cid) -> Result<Vec<String>> {
        let stmt = Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT \"recordUri\" FROM record_blob WHERE \"blobCid\" = ?",
            vec![cid.to_string().into()],
        );
        let rows = self.db.query_all_raw(stmt).await?;
        rows.iter()
            .map(|row| Ok(row.try_get_by_index::<String>(0)?))
            .collect()
    }

    pub async fn blob_count(&self) -> Result<i64> {
        let stmt = Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT count(*) FROM blob",
            vec![],
        );
        let row = self
            .db
            .query_one_raw(stmt)
            .await?
            .expect("count query always returns a row");
        Ok(row.try_get_by_index::<i64>(0)?)
    }

    pub async fn record_blob_count(&self) -> Result<i64> {
        let stmt = Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT count(DISTINCT \"blobCid\") FROM record_blob",
            vec![],
        );
        let row = self
            .db
            .query_one_raw(stmt)
            .await?
            .expect("count query always returns a row");
        Ok(row.try_get_by_index::<i64>(0)?)
    }

    pub async fn get_blob_cids(&self) -> Result<Vec<Cid>> {
        let stmt =
            Statement::from_sql_and_values(DatabaseBackend::Sqlite, "SELECT cid FROM blob", vec![]);
        let rows = self.db.query_all_raw(stmt).await?;
        rows.iter()
            .map(|row| {
                let cid: String = row.try_get_by_index(0)?;
                Cid::from_str(&cid).map_err(anyhow::Error::new)
            })
            .collect()
    }

    pub async fn list_missing_blobs(
        &self,
        opts: super::types::ListMissingBlobsOpts,
    ) -> Result<Vec<ListMissingBlobsRefRecordBlob>> {
        let super::types::ListMissingBlobsOpts { cursor, limit } = opts;
        if limit > 1000 {
            bail!("Limit too high. Max: 1000.");
        }
        let mut sql = String::from(
            "SELECT \"blobCid\", \"recordUri\" FROM record_blob \
             WHERE NOT EXISTS (SELECT 1 FROM blob WHERE blob.cid = record_blob.\"blobCid\")",
        );
        let mut values: Vec<Value> = Vec::new();
        if let Some(cursor) = &cursor {
            sql.push_str(" AND \"blobCid\" > ?");
            values.push(cursor.clone().into());
        }
        sql.push_str(" GROUP BY \"blobCid\" ORDER BY \"blobCid\" ASC LIMIT ?");
        values.push((limit as i64).into());
        let stmt = Statement::from_sql_and_values(DatabaseBackend::Sqlite, sql, values);
        let rows = self.db.query_all_raw(stmt).await?;
        rows.iter()
            .map(|row| {
                Ok(ListMissingBlobsRefRecordBlob {
                    cid: row.try_get_by_index::<String>(0)?,
                    record_uri: row.try_get_by_index::<String>(1)?,
                })
            })
            .collect()
    }

    pub async fn list_blobs(&self, opts: super::types::ListBlobsOpts) -> Result<Vec<String>> {
        let super::types::ListBlobsOpts {
            since,
            cursor,
            limit,
        } = opts;
        let mut sql = String::from("SELECT DISTINCT \"blobCid\" FROM record_blob");
        let mut values: Vec<Value> = Vec::new();
        if let Some(since) = &since {
            sql.push_str(
                " INNER JOIN record ON record.uri = record_blob.\"recordUri\" \
                 WHERE record.\"repoRev\" > ?",
            );
            values.push(since.clone().into());
        } else {
            sql.push_str(" WHERE 1 = 1");
        }
        if let Some(cursor) = &cursor {
            sql.push_str(" AND \"blobCid\" > ?");
            values.push(cursor.clone().into());
        }
        sql.push_str(" ORDER BY \"blobCid\" ASC LIMIT ?");
        values.push((limit as i64).into());
        let stmt = Statement::from_sql_and_values(DatabaseBackend::Sqlite, sql, values);
        let rows = self.db.query_all_raw(stmt).await?;
        rows.iter()
            .map(|row| Ok(row.try_get_by_index::<String>(0)?))
            .collect()
    }
}
