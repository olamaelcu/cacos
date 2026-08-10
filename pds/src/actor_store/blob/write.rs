//! Upload/track/promote/associate + write-batch GC for blob metadata.

use crate::actor_store::blob::types::{BLOB_SELECT, BlobMetadata, BlobReader, row_to_blob};
use anyhow::{Result, bail};
use lexicon_cid::Cid;
use rsky_common::ipld::sha256_to_cid;
use rsky_common::now;
use rsky_lexicon::blob_refs::BlobRef;
use rsky_repo::types::{PreparedBlobRef, PreparedWrite};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement, Value};
use sha2::{Digest, Sha256};
use std::str::FromStr;

pub async fn accepted_mime(mime: String, accepted: Vec<String>) -> bool {
    if accepted.contains(&"*/*".to_owned()) {
        return true;
    }
    let globs = accepted.iter().filter_map(|a| a.strip_suffix("/*"));
    for glob in globs {
        if mime.starts_with(&format!("{glob}/")) {
            return true;
        }
    }
    accepted.contains(&mime)
}

pub async fn verify_blob(blob: &PreparedBlobRef, found: &super::types::BlobRow) -> Result<()> {
    if let Some(max_size) = blob.constraints.max_size
        && found.size as usize > max_size
    {
        bail!(
            "BlobTooLarge: This file is too large. It is {:?} but the maximum size is {:?}",
            found.size,
            max_size
        )
    }
    if blob.mime_type != found.mime_type {
        bail!(
            "InvalidMimeType: Referenced MimeType does not match stored blob. Expected: {:?}, Got: {:?}",
            found.mime_type,
            blob.mime_type
        )
    }
    if let Some(ref accept) = blob.constraints.accept
        && !accepted_mime(blob.mime_type.clone(), accept.clone()).await
    {
        bail!(
            "Wrong type of file. It is {:?} but it must match {:?}.",
            blob.mime_type,
            accept
        )
    }
    Ok(())
}

pub async fn sha256_stream(to_hash: Vec<u8>) -> Result<Vec<u8>> {
    let digest = Sha256::digest(&*to_hash);
    let hash: &[u8] = digest.as_ref();
    Ok(hash.to_vec())
}

/// `?,?,...` for an `IN (...)` clause.
fn in_placeholders(n: usize) -> String {
    std::iter::repeat_n("?", n).collect::<Vec<_>>().join(",")
}

impl BlobReader {
    pub async fn upload_blob_and_get_metadata(
        &self,
        user_suggested_mime: String,
        bytes: Vec<u8>,
    ) -> Result<BlobMetadata> {
        let size = bytes.len() as i64;
        let temp_key = self.blobstore.put_temp(bytes.clone()).await?;
        let sha256 = sha256_stream(bytes).await?;
        let cid = sha256_to_cid(sha256);
        // Image sniffing is deferred (follow-up): use the user-suggested
        // mime as-is and leave width/height unpopulated.
        Ok(BlobMetadata {
            temp_key,
            size,
            cid,
            mime_type: user_suggested_mime,
            width: None,
            height: None,
        })
    }

    pub async fn track_untethered_blob(&self, metadata: BlobMetadata) -> Result<BlobRef> {
        let BlobMetadata {
            temp_key,
            size,
            cid,
            mime_type,
            width,
            height,
        } = metadata;
        let mime_type_clone = mime_type.clone();
        let stmt = Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            format!("{BLOB_SELECT} WHERE cid = ?"),
            vec![cid.to_string().into()],
        );
        if let Some(row) = self.db.query_one_raw(stmt).await? {
            let found = row_to_blob(&row)?;
            if found.takedown_ref.is_some() {
                bail!("Blob has been takendown, cannot re-upload")
            }
        }
        self.db
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "INSERT INTO blob (cid, \"mimeType\", size, \"tempKey\", width, height, \"createdAt\") \
                 VALUES (?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT (cid) DO UPDATE SET \"tempKey\" = excluded.\"tempKey\" \
                 WHERE blob.\"tempKey\" IS NOT NULL",
                vec![
                    cid.to_string().into(),
                    mime_type_clone.into(),
                    size.into(),
                    temp_key.into(),
                    width.into(),
                    height.into(),
                    now().into(),
                ],
            ))
            .await?;
        Ok(BlobRef::new(cid, mime_type, size, None))
    }

    pub async fn verify_blob_and_make_permanent(&self, blob: PreparedBlobRef) -> Result<()> {
        let cid = blob.cid;
        let stmt = Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            format!("{BLOB_SELECT} WHERE cid = ? AND \"takedownRef\" IS NULL"),
            vec![cid.to_string().into()],
        );
        let found: Option<super::types::BlobRow> = match self.db.query_one_raw(stmt).await? {
            Some(row) => Some(row_to_blob(&row)?),
            None => None,
        };
        if let Some(found) = found {
            verify_blob(&blob, &found).await?;
            if let Some(temp_key) = found.temp_key {
                self.blobstore
                    .make_permanent(temp_key.clone(), blob.cid)
                    .await?;
                self.db
                    .execute_raw(Statement::from_sql_and_values(
                        DatabaseBackend::Sqlite,
                        "UPDATE blob SET \"tempKey\" = NULL WHERE \"tempKey\" = ?",
                        vec![temp_key.clone().into()],
                    ))
                    .await?;
            }
            Ok(())
        } else {
            bail!("Could not find blob: {:?}", blob.cid.to_string())
        }
    }

    pub async fn associate_blob(&self, blob: PreparedBlobRef, record_uri: String) -> Result<()> {
        self.db
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "INSERT INTO record_blob (\"blobCid\", \"recordUri\") VALUES (?, ?) ON CONFLICT DO NOTHING",
                vec![blob.cid.to_string().into(), record_uri.into()],
            ))
            .await?;
        Ok(())
    }

    pub async fn process_write_blobs(&self, writes: Vec<PreparedWrite>) -> Result<()> {
        self.delete_dereferenced_blobs(writes.clone()).await?;
        for write in writes {
            match write {
                PreparedWrite::Create(w) | PreparedWrite::Update(w) => {
                    for blob in w.blobs {
                        self.verify_blob_and_make_permanent(blob.clone()).await?;
                        self.associate_blob(blob, w.uri.clone()).await?;
                    }
                }
                _ => (),
            }
        }
        Ok(())
    }

    pub async fn delete_dereferenced_blobs(&self, writes: Vec<PreparedWrite>) -> Result<()> {
        let uris: Vec<String> = writes
            .iter()
            .filter_map(|w| match w {
                PreparedWrite::Delete(w) => Some(w.uri.clone()),
                PreparedWrite::Update(w) => Some(w.uri.clone()),
                _ => None,
            })
            .collect();
        if uris.is_empty() {
            return Ok(());
        }

        // 1) Drop the record_blob rows for the touched records, returning the
        //    cids they referenced.
        let deleted_repo_blob_cids: Vec<String> = {
            let sql = format!(
                "DELETE FROM record_blob WHERE \"recordUri\" IN ({}) RETURNING \"blobCid\"",
                in_placeholders(uris.len())
            );
            let values = uris.into_iter().map(Value::from).collect::<Vec<Value>>();
            let stmt = Statement::from_sql_and_values(DatabaseBackend::Sqlite, sql, values);
            let rows = self.db.query_all_raw(stmt).await?;
            rows.iter()
                .map(|row| Ok(row.try_get_by_index::<String>(0)?))
                .collect::<Result<Vec<String>>>()?
        };
        if deleted_repo_blob_cids.is_empty() {
            return Ok(());
        }

        // 2) Which of those cids are still referenced by other records?
        let duplicated_cids: Vec<String> = {
            let sql = format!(
                "SELECT \"blobCid\" FROM record_blob WHERE \"blobCid\" IN ({})",
                in_placeholders(deleted_repo_blob_cids.len())
            );
            let values = deleted_repo_blob_cids
                .iter()
                .map(|c| Value::from(c.clone()))
                .collect::<Vec<Value>>();
            let stmt = Statement::from_sql_and_values(DatabaseBackend::Sqlite, sql, values);
            let rows = self.db.query_all_raw(stmt).await?;
            rows.iter()
                .map(|row| Ok(row.try_get_by_index::<String>(0)?))
                .collect::<Result<Vec<String>>>()?
        };

        // 3) Cids referenced by this same write batch also survive.
        let new_blob_cids: Vec<String> = writes
            .into_iter()
            .flat_map(|w| match w {
                PreparedWrite::Create(w) | PreparedWrite::Update(w) => w.blobs,
                PreparedWrite::Delete(_) => Vec::new(),
            })
            .map(|b| b.cid.to_string())
            .collect();

        let cids_to_delete: Vec<String> = deleted_repo_blob_cids
            .into_iter()
            .filter(|cid| !duplicated_cids.contains(cid) && !new_blob_cids.contains(cid))
            .collect();
        if cids_to_delete.is_empty() {
            return Ok(());
        }

        // 4) Delete the metadata rows, then delete the bytes in the background.
        let sql = format!(
            "DELETE FROM blob WHERE cid IN ({})",
            in_placeholders(cids_to_delete.len())
        );
        let values = cids_to_delete
            .iter()
            .map(|c| Value::from(c.clone()))
            .collect::<Vec<Value>>();
        self.db
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                sql,
                values,
            ))
            .await?;

        let blobstore = self.blobstore.clone();
        self.background_queue.add(async move {
            let cids = cids_to_delete
                .into_iter()
                .map(|cid| Cid::from_str(&cid).map_err(anyhow::Error::new))
                .collect::<Result<Vec<Cid>>>()?;
            blobstore.delete_many(cids).await
        });
        Ok(())
    }
}
