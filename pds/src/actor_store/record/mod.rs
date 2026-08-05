//! Per-actor record storage — ported from rsky-pds/src/actor_store/record/mod.rs.
//!
//! Uses sea-orm entities (`record`, `backlink`) for declarative queries.

use crate::actor_store::db::{backlink, record};
use crate::error::{PdsError, Result};
use lexicon_cid::Cid;
use sea_orm::{
    sea_query::OnConflict, ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait,
    PaginatorTrait, QueryFilter, Set, Statement,
};
use std::collections::BTreeSet;

/// Public return type for `RecordReader::get_record`.
#[derive(Debug, Clone, PartialEq)]
pub struct GetRecord {
    pub uri: String,
    pub cid: String,
    pub collection: String,
    pub rkey: String,
    pub repo_rev: String,
    pub indexed_at: time::OffsetDateTime,
    pub takedown_ref: Option<String>,
    pub value: Option<Vec<u8>>,
}

/// Reader over per-actor `record` + `backlink` tables.
#[derive(Clone)]
pub struct RecordReader {
    pub did: String,
    pub db: sea_orm::DatabaseConnection,
}

impl RecordReader {
    pub fn new(did: String, db: sea_orm::DatabaseConnection) -> Self {
        Self { did, db }
    }

    pub async fn record_count(&self) -> Result<usize> {
        let count = record::Entity::find().count(&self.db).await.map_err(|e| {
            PdsError::internal(
                "RecordReader::record_count: find().count failed",
                anyhow::Error::from(e),
            )
        })?;
        Ok(count as usize)
    }

    pub async fn list_collections(&self) -> Result<Vec<String>> {
        let stmt = Statement::from_string(
            DatabaseBackend::Sqlite,
            "SELECT DISTINCT collection FROM record ORDER BY collection".to_string(),
        );
        let rows = self.db.query_all_raw(stmt).await.map_err(|e| {
            PdsError::internal(
                "RecordReader::list_collections: query_all_raw failed",
                anyhow::Error::from(e),
            )
        })?;
        Ok(rows
            .iter()
            .map(|r| r.try_get_by_index::<String>(0).unwrap_or_default())
            .collect())
    }

    pub async fn get_record(
        &self,
        uri: &rsky_syntax::aturi::AtUri,
        _rev: Option<String>,
        _include_dead: Option<bool>,
    ) -> Result<Option<GetRecord>> {
        let row = record::Entity::find_by_id(uri.to_string())
            .one(&self.db)
            .await
            .map_err(|e| {
                PdsError::internal(
                    "RecordReader::get_record: find_by_id failed",
                    anyhow::Error::from(e),
                )
            })?;
        Ok(row.map(|m| GetRecord {
            uri: m.uri,
            cid: m.cid,
            collection: m.collection,
            rkey: m.rkey,
            repo_rev: m.repo_rev,
            indexed_at: m.indexed_at,
            takedown_ref: m.takedown_ref,
            value: None,
        }))
    }

    pub async fn has_record(
        &self,
        collection: String,
        rkey: String,
        _include_dead: Option<bool>,
    ) -> Result<bool> {
        let count = record::Entity::find()
            .filter(record::Column::Collection.eq(collection))
            .filter(record::Column::Rkey.eq(rkey))
            .count(&self.db)
            .await
            .map_err(|e| {
                PdsError::internal(
                    "RecordReader::has_record: count failed",
                    anyhow::Error::from(e),
                )
            })?;
        Ok(count > 0)
    }

    pub async fn get_current_record_cid(&self, collection: String, rkey: String) -> Result<Option<Cid>> {
        let row = record::Entity::find()
            .filter(record::Column::Collection.eq(collection))
            .filter(record::Column::Rkey.eq(rkey))
            .one(&self.db)
            .await
            .map_err(|e| {
                PdsError::internal(
                    "RecordReader::get_current_record_cid: find failed",
                    anyhow::Error::from(e),
                )
            })?;
        match row {
            None => Ok(None),
            Some(m) => {
                let cid = Cid::try_from(m.cid.as_str()).map_err(|e| {
                    PdsError::internal(
                        "RecordReader::get_current_record_cid: Cid::try_from failed",
                        anyhow::Error::from(e),
                    )
                })?;
                Ok(Some(cid))
            }
        }
    }

    pub async fn get_record_backlinks(
        &self,
        collection: String,
        path: String,
        link_to: String,
    ) -> Result<Vec<backlink::Model>> {
        let rows = backlink::Entity::find()
            .filter(backlink::Column::LinkTo.eq(link_to))
            .filter(backlink::Column::Path.eq(path))
            .all(&self.db)
            .await
            .map_err(|e| {
                PdsError::internal(
                    "RecordReader::get_record_backlinks: find failed",
                    anyhow::Error::from(e),
                )
            })?;
        let _ = collection;
        Ok(rows)
    }

    pub async fn get_backlink_conflicts(
        &self,
        _uri: &rsky_syntax::aturi::AtUri,
        _record: &rsky_repo::types::RepoRecord,
    ) -> Result<Vec<rsky_syntax::aturi::AtUri>> {
        // Walking the record's CID and inspecting block contents is deferred —
        // a stub is sufficient for Plan 03 since no Plan 03 consumer uses this.
        // Plan 08 fleshes it out.
        Ok(vec![])
    }

    pub async fn index_record(
        &self,
        uri: rsky_syntax::aturi::AtUri,
        cid: Cid,
        _value: Option<rsky_repo::types::RepoRecord>,
        _action: Option<rsky_repo::types::WriteOpAction>,
        repo_rev: String,
        _prev: Option<String>,
    ) -> Result<()> {
        let now = rsky_common::now();
        let now_dt = time::OffsetDateTime::parse(
            &now,
            &time::format_description::well_known::Rfc3339,
        )
        .unwrap_or_else(|_| time::OffsetDateTime::now_utc());
        let collection = uri.get_collection().to_string();
        let rkey = uri.get_rkey().to_string();
        let am = record::ActiveModel {
            uri: Set(uri.to_string()),
            cid: Set(cid.to_string()),
            collection: Set(collection),
            rkey: Set(rkey),
            repo_rev: Set(repo_rev),
            indexed_at: Set(now_dt),
            takedown_ref: Set(None),
        };
        record::Entity::insert(am)
            .on_conflict(
                OnConflict::column(record::Column::Uri)
                    .update_column(record::Column::Cid)
                    .update_column(record::Column::RepoRev)
                    .update_column(record::Column::IndexedAt)
                    .to_owned(),
            )
            .exec(&self.db)
            .await
            .map_err(|e| {
                PdsError::internal(
                    "RecordReader::index_record: insert failed",
                    anyhow::Error::from(e),
                )
            })?;
        Ok(())
    }

    pub async fn delete_record(&self, uri: &rsky_syntax::aturi::AtUri) -> Result<()> {
        record::Entity::delete_by_id(uri.to_string())
            .exec(&self.db)
            .await
            .map_err(|e| {
                PdsError::internal(
                    "RecordReader::delete_record: delete_by_id failed",
                    anyhow::Error::from(e),
                )
            })?;
        Ok(())
    }

    pub async fn remove_backlinks_by_uri(&self, uri: &rsky_syntax::aturi::AtUri) -> Result<()> {
        backlink::Entity::delete_many()
            .filter(backlink::Column::Uri.eq(uri.to_string()))
            .exec(&self.db)
            .await
            .map_err(|e| {
                PdsError::internal(
                    "RecordReader::remove_backlinks_by_uri: delete_many failed",
                    anyhow::Error::from(e),
                )
            })?;
        Ok(())
    }

    pub async fn add_backlinks(
        &self,
        uri: &rsky_syntax::aturi::AtUri,
        links: BTreeSet<String>,
    ) -> Result<()> {
        if links.is_empty() {
            return Ok(());
        }
        let uri_string = uri.to_string();
        let mut ams: Vec<backlink::ActiveModel> = Vec::with_capacity(links.len());
        for path in links {
            ams.push(backlink::ActiveModel {
                uri: Set(uri_string.clone()),
                path: Set(path.clone()),
                link_to: Set(uri_string.clone()),
            });
        }
        backlink::Entity::insert_many(ams)
            .on_conflict_do_nothing()
            .exec(&self.db)
            .await
            .map_err(|e| {
                PdsError::internal(
                    "RecordReader::add_backlinks: insert_many failed",
                    anyhow::Error::from(e),
                )
            })?;
        Ok(())
    }
}

/// Free function stub: walking a record's CID to the block storage and
/// extracting backlinks (paths referencing the record's collection) lands with
/// Plan 07. Plan 03 leaves this as an empty stub.
pub async fn get_backlinks(
    _uri: &rsky_syntax::aturi::AtUri,
    _record: &rsky_repo::types::RepoRecord,
) -> Result<Vec<backlink::Model>> {
    Ok(vec![])
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup() -> (camino_tempfile::Utf8TempDir, RecordReader, String) {
        let dir = camino_tempfile::Utf8TempDir::new().unwrap();
        let db = crate::db::DatabaseKind::Actor
            .open(dir.path().join("store.sqlite"))
            .await
            .unwrap();
        let did = "did:example:alice".to_string();
        let reader = RecordReader::new(did.clone(), db);
        (dir, reader, did)
    }

    fn make_uri(did: &str, collection: &str, rkey: &str) -> rsky_syntax::aturi::AtUri {
        rsky_syntax::aturi::AtUri::new(
            format!("{did}/{collection}/{rkey}"),
            None,
        )
        .expect("valid AtUri")
    }

    fn cid_for(value: &[u8]) -> Cid {
        use sha2::{Digest, Sha256};
        rsky_common::ipld::sha256_to_cid(Sha256::digest(value).to_vec())
    }

    #[tokio::test]
    async fn empty_record_count_and_collections() {
        let (_dir, reader, _) = setup().await;
        assert_eq!(reader.record_count().await.unwrap(), 0);
        assert!(reader.list_collections().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn index_get_and_delete_record() {
        let (_dir, reader, did) = setup().await;
        let uri = make_uri(&did, "app.bsky.feed.post", "abc123");
        let cid = cid_for(b"{\"text\":\"hello\"}");

        reader
            .index_record(
                uri.clone(),
                cid,
                None,
                None,
                "rev-1".to_string(),
                None,
            )
            .await
            .unwrap();

        let got = reader.get_record(&uri, None, None).await.unwrap().unwrap();
        assert_eq!(got.cid, cid.to_string());
        assert_eq!(got.collection, "app.bsky.feed.post");
        assert_eq!(got.rkey, "abc123");
        assert_eq!(got.repo_rev, "rev-1");
        assert_eq!(reader.record_count().await.unwrap(), 1);

        let collections = reader.list_collections().await.unwrap();
        assert_eq!(collections, vec!["app.bsky.feed.post"]);

        assert!(reader
            .has_record("app.bsky.feed.post".into(), "abc123".into(), None)
            .await
            .unwrap());

        reader.delete_record(&uri).await.unwrap();
        assert_eq!(reader.record_count().await.unwrap(), 0);
        assert!(reader.get_record(&uri, None, None).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn index_record_upserts_on_conflict() {
        let (_dir, reader, did) = setup().await;
        let uri = make_uri(&did, "app.bsky.feed.like", "k1");
        let cid1 = cid_for(b"v1");
        let cid2 = cid_for(b"v2");

        reader
            .index_record(uri.clone(), cid1, None, None, "rev-1".into(), None)
            .await
            .unwrap();
        reader
            .index_record(uri.clone(), cid2, None, None, "rev-2".into(), None)
            .await
            .unwrap();

        let got = reader.get_record(&uri, None, None).await.unwrap().unwrap();
        assert_eq!(got.cid, cid2.to_string());
        assert_eq!(got.repo_rev, "rev-2");
    }

    #[tokio::test]
    async fn maintains_backlinks_for_follows_and_likes() {
        let (_dir, reader, did) = setup().await;
        let alice = make_uri(&did, "app.bsky.actor.profile", "self");

        let mut links = BTreeSet::new();
        links.insert("app.bsky.feed.post/abc".to_string());
        links.insert("app.bsky.feed.like/xyz".to_string());

        reader.add_backlinks(&alice, links).await.unwrap();

        let back = reader
            .get_record_backlinks(
                "app.bsky.actor.profile".into(),
                "app.bsky.feed.post/abc".into(),
                alice.to_string(),
            )
            .await
            .unwrap();
        assert!(back.iter().any(|b| b.uri == alice.to_string()));

        reader.remove_backlinks_by_uri(&alice).await.unwrap();
        let back_after = reader
            .get_record_backlinks(
                "app.bsky.actor.profile".into(),
                "app.bsky.feed.post/abc".into(),
                alice.to_string(),
            )
            .await
            .unwrap();
        assert!(back_after.is_empty());
    }

    #[tokio::test]
    async fn current_record_cid_returns_latest() {
        let (_dir, reader, did) = setup().await;
        let uri = make_uri(&did, "app.bsky.feed.post", "k1");
        let cid1 = cid_for(b"v1");
        let cid2 = cid_for(b"v2");

        reader
            .index_record(uri.clone(), cid1, None, None, "rev-1".into(), None)
            .await
            .unwrap();
        reader
            .index_record(uri.clone(), cid2, None, None, "rev-2".into(), None)
            .await
            .unwrap();

        let current = reader
            .get_current_record_cid("app.bsky.feed.post".into(), "k1".into())
            .await
            .unwrap();
        assert_eq!(current, Some(cid2));
    }

    #[tokio::test]
    async fn get_record_missing_returns_none() {
        let (_dir, reader, did) = setup().await;
        let uri = make_uri(&did, "app.bsky.feed.post", "missing");
        assert!(reader.get_record(&uri, None, None).await.unwrap().is_none());
        assert!(!reader
            .has_record("app.bsky.feed.post".into(), "missing".into(), None)
            .await
            .unwrap());
    }
}
