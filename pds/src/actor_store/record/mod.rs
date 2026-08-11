//! Per-actor record storage.
//!
//! Uses sea-orm entities (`record`, `backlink`) for declarative queries.

use crate::actor_store::db::{backlink, record};
use cacos_pds_core::error::{PdsError, Result};
use lexicon_cid::Cid;
use rsky_repo::storage::Ipld;
use rsky_repo::types::Lex;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect, Set, Statement, sea_query::OnConflict,
};
use std::collections::BTreeSet;

/// Recursively walk a `RepoRecord` (= `BTreeMap<String, Lex>`) and collect
/// `(link_to, path)` pairs for every strong-ref object (`{"uri": "<at://… or did:…>"}`)
/// or leaf string starting with `at://` or `did:`.
///
/// The path is `<$type>/<slot>` where `<$type>` is the record's root `$type`
/// value and `<slot>` is the JSON key under which the link is nested
/// (e.g. `app.bsky.feed.post/parent` for `{"reply": {"parent": {"uri": …}}}`).
/// Empty record yields no pairs.
fn extract_links_from_repo_record(
    record: &rsky_repo::types::RepoRecord,
    out: &mut Vec<(String, String)>,
) {
    let collection = record.get("$type").and_then(|lex| match lex {
        Lex::Ipld(Ipld::String(s)) => Some(s.clone()),
        _ => None,
    });
    for (k, v) in record {
        if k == "$type" {
            continue;
        }
        walk_lex(v, k, &collection, out);
    }
}

fn make_path(slot: &str, collection: &Option<String>) -> String {
    match collection {
        Some(c) => format!("{c}/{slot}"),
        None => slot.to_string(),
    }
}

fn walk_lex(value: &Lex, slot: &str, collection: &Option<String>, out: &mut Vec<(String, String)>) {
    match value {
        Lex::Ipld(ipld) => walk_ipld(ipld, slot, collection, out),
        Lex::Map(m) => walk_map_lex(m, slot, collection, out),
        Lex::List(arr) => {
            for v in arr {
                walk_lex(v, slot, collection, out);
            }
        }
        Lex::Blob(_) => {}
    }
}

fn walk_map_lex(
    m: &std::collections::BTreeMap<String, Lex>,
    slot: &str,
    collection: &Option<String>,
    out: &mut Vec<(String, String)>,
) {
    if let Some(Lex::Ipld(Ipld::String(s))) = m.get("uri")
        && (s.starts_with("at://") || s.starts_with("did:"))
    {
        out.push((s.clone(), make_path(slot, collection)));
        return;
    }
    for (k, v) in m {
        walk_lex(v, k, collection, out);
    }
}

fn walk_ipld(
    value: &Ipld,
    slot: &str,
    collection: &Option<String>,
    out: &mut Vec<(String, String)>,
) {
    match value {
        Ipld::Map(m) => walk_map_ipld(m, slot, collection, out),
        Ipld::List(arr) => {
            for v in arr {
                walk_ipld(v, slot, collection, out);
            }
        }
        Ipld::String(s) => {
            if s.starts_with("at://") || s.starts_with("did:") {
                out.push((s.clone(), make_path(slot, collection)));
            }
        }
        Ipld::Link(_) | Ipld::Bytes(_) | Ipld::Json(_) => {}
    }
}

fn walk_map_ipld(
    m: &std::collections::BTreeMap<String, Ipld>,
    slot: &str,
    collection: &Option<String>,
    out: &mut Vec<(String, String)>,
) {
    if let Some(Ipld::String(s)) = m.get("uri")
        && (s.starts_with("at://") || s.starts_with("did:"))
    {
        out.push((s.clone(), make_path(slot, collection)));
        return;
    }
    for (k, v) in m {
        walk_ipld(v, k, collection, out);
    }
}

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

    pub async fn get_current_record_cid(
        &self,
        collection: String,
        rkey: String,
    ) -> Result<Option<Cid>> {
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
        record: &rsky_repo::types::RepoRecord,
    ) -> Result<Vec<rsky_syntax::aturi::AtUri>> {
        let mut links = Vec::new();
        extract_links_from_repo_record(record, &mut links);
        if links.is_empty() {
            return Ok(vec![]);
        }
        let target_ph = "?,".repeat(links.len());
        let target_ph = target_ph.trim_end_matches(',');
        let path_ph = "?,".repeat(links.len());
        let path_ph = path_ph.trim_end_matches(',');
        let mut values: Vec<sea_orm::Value> = Vec::with_capacity(links.len() * 2);
        for (target, _path) in &links {
            values.push(target.clone().into());
        }
        for (_target, path) in &links {
            values.push(path.clone().into());
        }
        let stmt = Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            format!(
                "SELECT DISTINCT uri FROM backlink WHERE \"linkTo\" IN ({target_ph}) AND path IN ({path_ph})"
            ),
            values,
        );
        let rows =
            self.db.query_all_raw(stmt).await.map_err(|e| {
                PdsError::internal("get_backlink_conflicts", anyhow::Error::from(e))
            })?;
        let mut out = Vec::new();
        for r in rows.iter() {
            if let Ok(s) = r.try_get_by_index::<String>(0)
                && let Ok(uri) = rsky_syntax::aturi::AtUri::new(s, None)
            {
                out.push(uri);
            }
        }
        Ok(out)
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
        let now_dt =
            time::OffsetDateTime::parse(&now, &time::format_description::well_known::Rfc3339)
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

    pub async fn get_record_takedown_status(
        &self,
        uri: &rsky_syntax::aturi::AtUri,
    ) -> Result<Option<String>> {
        let row = record::Entity::find_by_id(uri.to_string())
            .one(&self.db)
            .await
            .map_err(|e| {
                PdsError::internal(
                    "RecordReader::get_record_takedown_status: find_by_id failed",
                    anyhow::Error::from(e),
                )
            })?;
        Ok(row.and_then(|m| m.takedown_ref))
    }

    pub async fn update_record_takedown_status(
        &self,
        uri: &rsky_syntax::aturi::AtUri,
        takedown_ref: Option<String>,
    ) -> Result<()> {
        let am = record::ActiveModel {
            uri: Set(uri.to_string()),
            takedown_ref: Set(takedown_ref),
            ..Default::default()
        };
        record::Entity::update(am)
            .exec(&self.db)
            .await
            .map_err(|e| {
                PdsError::internal(
                    "RecordReader::update_record_takedown_status: update failed",
                    anyhow::Error::from(e),
                )
            })?;
        Ok(())
    }

    pub async fn list_records_for_collection(
        &self,
        collection: &str,
        limit: Option<usize>,
        reverse: Option<bool>,
    ) -> Result<Vec<GetRecord>> {
        let mut query = record::Entity::find().filter(record::Column::Collection.eq(collection));
        if reverse.unwrap_or(false) {
            query = query.order_by_desc(record::Column::Rkey);
        } else {
            query = query.order_by_asc(record::Column::Rkey);
        }
        if let Some(limit) = limit {
            query = query.limit(limit as u64);
        }
        let rows = query.all(&self.db).await.map_err(|e| {
            PdsError::internal(
                "RecordReader::list_records_for_collection: find failed",
                anyhow::Error::from(e),
            )
        })?;
        Ok(rows
            .into_iter()
            .map(|m| GetRecord {
                uri: m.uri,
                cid: m.cid,
                collection: m.collection,
                rkey: m.rkey,
                repo_rev: m.repo_rev,
                indexed_at: m.indexed_at,
                takedown_ref: m.takedown_ref,
                value: None,
            })
            .collect())
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

/// Free function: walk a `RepoRecord` and find all existing `backlink` rows
/// where the `link_to` matches any `at://` or `did:` string leaf in the record.
pub async fn get_backlinks(
    reader: &RecordReader,
    _uri: &rsky_syntax::aturi::AtUri,
    record: &rsky_repo::types::RepoRecord,
) -> Result<Vec<backlink::Model>> {
    let mut links = Vec::new();
    extract_links_from_repo_record(record, &mut links);
    if links.is_empty() {
        return Ok(vec![]);
    }
    let target_ph = "?,".repeat(links.len());
    let target_ph = target_ph.trim_end_matches(',');
    let mut values: Vec<sea_orm::Value> = Vec::with_capacity(links.len());
    for (target, _path) in &links {
        values.push(target.clone().into());
    }
    let stmt = Statement::from_sql_and_values(
        DatabaseBackend::Sqlite,
        format!("SELECT * FROM backlink WHERE \"linkTo\" IN ({target_ph})"),
        values,
    );
    let rows = reader
        .db
        .query_all_raw(stmt)
        .await
        .map_err(|e| PdsError::internal("get_backlinks", anyhow::Error::from(e)))?;
    let mut out = Vec::new();
    for r in rows.iter() {
        let uri = r.try_get_by_index::<String>(0).unwrap_or_default();
        let path = r.try_get_by_index::<String>(1).unwrap_or_default();
        let link_to = r.try_get_by_index::<String>(2).unwrap_or_default();
        out.push(backlink::Model { uri, path, link_to });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup() -> (camino_tempfile::Utf8TempDir, RecordReader, String) {
        let dir = camino_tempfile::Utf8TempDir::new().unwrap();
        let db = cacos_pds_core::db::DatabaseKind::Actor
            .open(dir.path().join("store.sqlite"))
            .await
            .unwrap();
        let did = "did:example:alice".to_string();
        let reader = RecordReader::new(did.clone(), db);
        (dir, reader, did)
    }

    fn make_uri(did: &str, collection: &str, rkey: &str) -> rsky_syntax::aturi::AtUri {
        rsky_syntax::aturi::AtUri::new(format!("{did}/{collection}/{rkey}"), None)
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
            .index_record(uri.clone(), cid, None, None, "rev-1".to_string(), None)
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

        assert!(
            reader
                .has_record("app.bsky.feed.post".into(), "abc123".into(), None)
                .await
                .unwrap()
        );

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
        assert!(
            !reader
                .has_record("app.bsky.feed.post".into(), "missing".into(), None)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn takedown_status_roundtrip() {
        let (_dir, reader, did) = setup().await;
        let uri = make_uri(&did, "app.bsky.feed.post", "t1");
        let cid = cid_for(b"v1");
        reader
            .index_record(uri.clone(), cid, None, None, "rev-1".into(), None)
            .await
            .unwrap();

        assert_eq!(reader.get_record_takedown_status(&uri).await.unwrap(), None);

        reader
            .update_record_takedown_status(&uri, Some("mod-1".into()))
            .await
            .unwrap();
        assert_eq!(
            reader.get_record_takedown_status(&uri).await.unwrap(),
            Some("mod-1".to_string())
        );

        reader
            .update_record_takedown_status(&uri, None)
            .await
            .unwrap();
        assert_eq!(reader.get_record_takedown_status(&uri).await.unwrap(), None);
    }

    #[tokio::test]
    async fn list_records_for_collection_filters_and_orders() {
        let (_dir, reader, did) = setup().await;
        let uri_a = make_uri(&did, "app.bsky.feed.post", "aaa");
        let uri_b = make_uri(&did, "app.bsky.feed.post", "bbb");
        let uri_c = make_uri(&did, "app.bsky.feed.like", "ccc");
        let cid_a = cid_for(b"a");
        let cid_b = cid_for(b"b");
        let cid_c = cid_for(b"c");
        reader
            .index_record(uri_a, cid_a, None, None, "rev-1".into(), None)
            .await
            .unwrap();
        reader
            .index_record(uri_b, cid_b, None, None, "rev-2".into(), None)
            .await
            .unwrap();
        reader
            .index_record(uri_c, cid_c, None, None, "rev-3".into(), None)
            .await
            .unwrap();

        let posts = reader
            .list_records_for_collection("app.bsky.feed.post", None, None)
            .await
            .unwrap();
        assert_eq!(posts.len(), 2);
        assert!(posts.iter().any(|r| r.rkey == "aaa"));
        assert!(posts.iter().any(|r| r.rkey == "bbb"));
        assert!(!posts.iter().any(|r| r.collection == "app.bsky.feed.like"));

        // Reverse order
        let posts_rev = reader
            .list_records_for_collection("app.bsky.feed.post", None, Some(true))
            .await
            .unwrap();
        assert_eq!(posts_rev.first().unwrap().rkey, "bbb");

        // Limit
        let posts_lim = reader
            .list_records_for_collection("app.bsky.feed.post", Some(1), None)
            .await
            .unwrap();
        assert_eq!(posts_lim.len(), 1);
    }

    #[tokio::test]
    async fn get_backlink_conflicts_detects_existing_links() {
        use std::collections::BTreeSet;

        let (_dir, reader, did) = setup().await;

        // Pre-existing record claims a backlink to a target
        let target_uri = make_uri(&did, "app.bsky.actor.profile", "self");
        reader
            .add_backlinks(
                &target_uri,
                BTreeSet::from(["app.bsky.feed.post/parent".to_string()]),
            )
            .await
            .unwrap();

        // Now build a new post record that would claim the same target/path
        let repo_record: rsky_repo::types::RepoRecord = serde_json::from_value(serde_json::json!({
            "$type": "app.bsky.feed.post",
            "text": "hi",
            "reply": { "parent": { "uri": target_uri.to_string() } }
        }))
        .unwrap();
        let new_uri = make_uri(&did, "app.bsky.feed.post", "new");
        let conflicts = reader
            .get_backlink_conflicts(&new_uri, &repo_record)
            .await
            .unwrap();
        assert!(!conflicts.is_empty(), "expected at least one conflict");
        assert!(
            conflicts
                .iter()
                .any(|u| u.to_string() == target_uri.to_string())
        );
    }

    #[tokio::test]
    async fn get_backlink_conflicts_empty_record_returns_no_conflicts() {
        let (_dir, reader, did) = setup().await;
        let uri = make_uri(&did, "app.bsky.feed.post", "x");
        let empty: rsky_repo::types::RepoRecord =
            serde_json::from_value(serde_json::json!({})).unwrap();
        let conflicts = reader.get_backlink_conflicts(&uri, &empty).await.unwrap();
        assert!(conflicts.is_empty());
    }

    #[tokio::test]
    async fn free_get_backlinks_returns_links_for_record() {
        use std::collections::BTreeSet;

        let (_dir, reader, did) = setup().await;
        let target_uri = make_uri(&did, "app.bsky.actor.profile", "self");
        reader
            .add_backlinks(
                &target_uri,
                BTreeSet::from(["app.bsky.feed.like/subject".to_string()]),
            )
            .await
            .unwrap();

        let repo_record: rsky_repo::types::RepoRecord = serde_json::from_value(serde_json::json!({
            "$type": "app.bsky.feed.like",
            "subject": { "uri": target_uri.to_string() }
        }))
        .unwrap();
        let rows = get_backlinks(&reader, &target_uri, &repo_record)
            .await
            .unwrap();
        assert!(!rows.is_empty());
    }
}
