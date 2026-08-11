use super::*;
use cacos_pds_core::background::BackgroundQueue;
use cacos_pds_blobstore::BlobStore;
use cacos_pds_blobstore::BoxedBlobStream;
use cacos_pds_blobstore::MemoryBlobStore;
use cacos_pds_blobstore::opendal::OpenDALBlobStore;
use cacos_pds_core::db::DatabaseKind;
use futures::TryStreamExt;
use lexicon_cid::Cid;
use rsky_common::ipld::sha256_to_cid;
use rsky_lexicon::blob_refs::BlobRef;
use rsky_lexicon::com::atproto::admin::StatusAttr;
use rsky_repo::types::{
    BlobConstraint, PreparedBlobRef, PreparedCreateOrUpdate, PreparedDelete, PreparedWrite,
    WriteOpAction,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use sha2::{Digest, Sha256};
use std::sync::Arc;

fn cid_for(bytes: &[u8]) -> Cid {
    sha256_to_cid(Sha256::digest(bytes).to_vec())
}

#[tokio::test]
async fn sha256_stream_hashes_input() {
    let hash = sha256_stream(b"hello sha".to_vec()).await.unwrap();
    assert_eq!(hash, Sha256::digest(b"hello sha").to_vec());
    assert_eq!(hash.len(), 32);
}

#[tokio::test]
async fn accepted_mime_globs() {
    assert!(accepted_mime("image/png".to_owned(), vec!["*/*".to_owned()]).await);
    assert!(accepted_mime("image/png".to_owned(), vec!["image/*".to_owned()]).await);
    assert!(!accepted_mime("text/plain".to_owned(), vec!["image/*".to_owned()]).await);
    assert!(accepted_mime("text/plain".to_owned(), vec!["text/plain".to_owned()]).await);
    assert!(!accepted_mime("text/plain".to_owned(), vec!["image/png".to_owned()]).await);
}

struct TestBlobReader {
    reader: BlobReader,
    store: Arc<MemoryBlobStore>,
    _dir: camino_tempfile::Utf8TempDir,
}

async fn test_reader() -> TestBlobReader {
    let dir = camino_tempfile::tempdir().unwrap();
    let db_path = dir.path().join("store.sqlite");
    let db = DatabaseKind::Actor.open(&db_path).await.unwrap();
    let store = Arc::new(MemoryBlobStore::default());
    let reader = BlobReader::new(store.clone(), db, BackgroundQueue::default());
    TestBlobReader {
        reader,
        store,
        _dir: dir,
    }
}

/// Build a per-DID OpenDAL handle backed by an in-memory operator.
/// Used by `per_did_isolation` to assert that two actors sharing the
/// same operator stay isolated by DID.
fn per_did_opendal_handle(did: &str) -> Arc<dyn BlobStore<Stream = BoxedBlobStream>> {
    Arc::new(OpenDALBlobStore::new_memory(did).unwrap())
}

async fn upload(t: &TestBlobReader, bytes: &[u8]) -> BlobRef {
    let metadata = t
        .reader
        .upload_blob_and_get_metadata("text/plain".to_owned(), bytes.to_vec())
        .await
        .unwrap();
    t.reader.track_untethered_blob(metadata).await.unwrap()
}

fn prepared_ref(blob: &BlobRef) -> PreparedBlobRef {
    PreparedBlobRef {
        cid: blob.get_cid().unwrap(),
        mime_type: blob.get_mime_type().to_string(),
        constraints: BlobConstraint {
            max_size: None,
            accept: None,
        },
    }
}

#[tokio::test]
async fn upload_track_and_promote_blob() {
    let t = test_reader().await;
    let blob = upload(&t, b"some blob bytes").await;
    let cid = blob.get_cid().unwrap();
    assert_eq!(blob.get_mime_type(), "text/plain");
    assert_eq!(t.reader.blob_count().await.unwrap(), 1);
    // still in temp storage
    assert!(!t.store.has_stored(cid).await.unwrap());

    // re-tracking the same blob refreshes the temp key rather than failing
    let metadata = t
        .reader
        .upload_blob_and_get_metadata("text/plain".to_owned(), b"some blob bytes".to_vec())
        .await
        .unwrap();
    t.reader.track_untethered_blob(metadata).await.unwrap();

    t.reader
        .verify_blob_and_make_permanent(prepared_ref(&blob))
        .await
        .unwrap();
    assert!(t.store.has_stored(cid).await.unwrap());
    let metadata = t.reader.get_blob_metadata(cid).await.unwrap();
    assert_eq!(metadata.size, 15);
    assert_eq!(metadata.mime_type.as_deref(), Some("text/plain"));
    let output = t.reader.get_blob(cid).await.unwrap();
    // Plan 08's handler shape: try_collect BoxedBlobStream -> Vec<Bytes> -> Vec<u8>.
    let chunks: Vec<bytes::Bytes> = output.stream.try_collect().await.unwrap();
    let bytes: Vec<u8> = chunks.iter().flat_map(|b| b.iter().copied()).collect();
    assert_eq!(bytes, b"some blob bytes");
    // promoting again is a no-op
    t.reader
        .verify_blob_and_make_permanent(prepared_ref(&blob))
        .await
        .unwrap();
}

#[tokio::test]
async fn per_did_isolation() {
    let alice = per_did_opendal_handle("did:example:alice");
    let bob = per_did_opendal_handle("did:example:bob");
    let bytes = b"shared base, disjoint dids".to_vec();
    let cid = cid_for(&bytes);
    alice.put_permanent(cid, bytes.clone()).await.unwrap();
    assert!(alice.has_stored(cid).await.unwrap());
    assert!(!bob.has_stored(cid).await.unwrap());

    // Alice's per-DID `delete_all` returns `Some(future)` and
    // wipes only her directory.
    let alice_delete = alice.delete_all().expect("per-DID delete_all returns Some");
    alice_delete.await.unwrap();
    assert!(!alice.has_stored(cid).await.unwrap());
}

#[tokio::test]
async fn associates_blobs_with_records() {
    let t = test_reader().await;
    let blob = upload(&t, b"blob for record").await;
    let cid = blob.get_cid().unwrap();
    let record_uri = "at://did:example:alice/app.bsky.feed.post/3jt5vlkoraa2a";
    t.reader
        .associate_blob(prepared_ref(&blob), record_uri.to_owned())
        .await
        .unwrap();
    // idempotent
    t.reader
        .associate_blob(prepared_ref(&blob), record_uri.to_owned())
        .await
        .unwrap();
    assert_eq!(
        t.reader.get_records_for_blob(cid).await.unwrap(),
        [record_uri]
    );
    assert_eq!(t.reader.record_blob_count().await.unwrap(), 1);
    assert_eq!(t.reader.get_blob_cids().await.unwrap(), [cid]);
}

#[tokio::test]
async fn verify_blob_enforces_constraints() {
    let t = test_reader().await;
    let blob = upload(&t, b"constrained").await;
    let mut wrong_mime = prepared_ref(&blob);
    wrong_mime.mime_type = "image/png".to_owned();
    assert!(
        t.reader
            .verify_blob_and_make_permanent(wrong_mime)
            .await
            .is_err()
    );

    let mut too_large = prepared_ref(&blob);
    too_large.constraints.max_size = Some(1);
    assert!(
        t.reader
            .verify_blob_and_make_permanent(too_large)
            .await
            .is_err()
    );

    let mut wrong_accept = prepared_ref(&blob);
    wrong_accept.constraints.accept = Some(vec!["image/*".to_owned()]);
    assert!(
        t.reader
            .verify_blob_and_make_permanent(wrong_accept)
            .await
            .is_err()
    );

    let mut accept_any = prepared_ref(&blob);
    accept_any.constraints.accept = Some(vec!["*/*".to_owned()]);
    t.reader
        .verify_blob_and_make_permanent(accept_any)
        .await
        .unwrap();

    let missing = PreparedBlobRef {
        cid: cid_for(b"missing"),
        mime_type: "text/plain".to_owned(),
        constraints: BlobConstraint {
            max_size: None,
            accept: None,
        },
    };
    assert!(
        t.reader
            .verify_blob_and_make_permanent(missing)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn verify_blob_accepts_within_max_size() {
    let t = test_reader().await;
    let blob = upload(&t, b"small").await;
    let mut sized = prepared_ref(&blob);
    sized.constraints.max_size = Some(1024);
    t.reader
        .verify_blob_and_make_permanent(sized)
        .await
        .unwrap();
}

#[tokio::test]
async fn takedown_lifecycle() {
    let t = test_reader().await;
    let blob = upload(&t, b"takedown me").await;
    let cid = blob.get_cid().unwrap();
    t.reader
        .verify_blob_and_make_permanent(prepared_ref(&blob))
        .await
        .unwrap();

    let status = t
        .reader
        .get_blob_takedown_status(cid)
        .await
        .unwrap()
        .unwrap();
    assert!(!status.applied && status.r#ref.is_none());
    t.reader
        .update_blob_takedown_status(
            cid,
            StatusAttr {
                applied: true,
                r#ref: Some("ref-1".to_owned()),
            },
        )
        .await
        .unwrap();
    assert!(t.store.has_quarantined(&cid));
    let status = t
        .reader
        .get_blob_takedown_status(cid)
        .await
        .unwrap()
        .unwrap();
    assert!(status.applied);
    assert_eq!(status.r#ref.as_deref(), Some("ref-1"));
    // metadata is hidden while taken down
    assert!(t.reader.get_blob_metadata(cid).await.is_err());
    assert!(t.reader.get_blob(cid).await.is_err());
    // taken-down blob cannot be re-uploaded
    let metadata = t
        .reader
        .upload_blob_and_get_metadata("text/plain".to_owned(), b"takedown me".to_vec())
        .await
        .unwrap();
    assert!(t.reader.track_untethered_blob(metadata).await.is_err());

    // takedown without a ref defaults to a timestamp
    t.reader
        .update_blob_takedown_status(
            cid,
            StatusAttr {
                applied: false,
                r#ref: None,
            },
        )
        .await
        .unwrap();
    assert!(!t.store.has_quarantined(&cid));
    t.reader
        .update_blob_takedown_status(
            cid,
            StatusAttr {
                applied: true,
                r#ref: None,
            },
        )
        .await
        .unwrap();
    let status = t
        .reader
        .get_blob_takedown_status(cid)
        .await
        .unwrap()
        .unwrap();
    assert!(status.applied && status.r#ref.is_some());
    // unknown blob has no status
    let unknown = cid_for(b"unknown");
    assert!(
        t.reader
            .get_blob_takedown_status(unknown)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn takedown_logs_missing_blobstore_entry() {
    let t = test_reader().await;
    // row exists but blob bytes were never promoted to storage
    let blob = upload(&t, b"row only").await;
    let cid = blob.get_cid().unwrap();
    t.reader
        .update_blob_takedown_status(
            cid,
            StatusAttr {
                applied: true,
                r#ref: Some("ref-x".to_owned()),
            },
        )
        .await
        .unwrap();
    let status = t
        .reader
        .get_blob_takedown_status(cid)
        .await
        .unwrap()
        .unwrap();
    assert!(status.applied);
}

#[tokio::test]
async fn lists_blobs_and_missing_blobs() {
    let t = test_reader().await;
    let blob = upload(&t, b"listed blob").await;
    let cid = blob.get_cid().unwrap();
    let record_uri = "at://did:example:alice/app.bsky.feed.post/3jt5vlkoraa2a";
    t.reader
        .associate_blob(prepared_ref(&blob), record_uri.to_owned())
        .await
        .unwrap();
    // record row for the `since` join
    t.reader
        .db
        .execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "INSERT INTO record (uri, cid, collection, rkey, \"repoRev\", \"indexedAt\") \
                 VALUES (?, 'bafyfake', 'app.bsky.feed.post', '3jt5vlkoraa2a', 'rev-2', 'now')",
            vec![record_uri.into()],
        ))
        .await
        .unwrap();

    let all = t
        .reader
        .list_blobs(ListBlobsOpts {
            since: None,
            cursor: None,
            limit: 10,
        })
        .await
        .unwrap();
    assert_eq!(all, [cid.to_string()]);
    let since = t
        .reader
        .list_blobs(ListBlobsOpts {
            since: Some("rev-1".to_owned()),
            cursor: None,
            limit: 10,
        })
        .await
        .unwrap();
    assert_eq!(since, [cid.to_string()]);
    let since_after = t
        .reader
        .list_blobs(ListBlobsOpts {
            since: Some("rev-2".to_owned()),
            cursor: None,
            limit: 10,
        })
        .await
        .unwrap();
    assert!(since_after.is_empty());
    let cursored = t
        .reader
        .list_blobs(ListBlobsOpts {
            since: None,
            cursor: Some(cid.to_string()),
            limit: 10,
        })
        .await
        .unwrap();
    assert!(cursored.is_empty());

    // a record_blob row without a blob row is a missing blob
    let missing_uri = "at://did:example:alice/app.bsky.feed.post/3jt5vlkorbb2b";
    t.reader
        .db
        .execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "INSERT INTO record_blob (\"blobCid\", \"recordUri\") VALUES ('bafymissing', ?)",
            vec![missing_uri.into()],
        ))
        .await
        .unwrap();
    let missing = t
        .reader
        .list_missing_blobs(ListMissingBlobsOpts {
            cursor: None,
            limit: 10,
        })
        .await
        .unwrap();
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].cid, "bafymissing");
    assert_eq!(missing[0].record_uri, missing_uri);
    let missing_cursored = t
        .reader
        .list_missing_blobs(ListMissingBlobsOpts {
            cursor: Some("bafymissing".to_owned()),
            limit: 10,
        })
        .await
        .unwrap();
    assert!(missing_cursored.is_empty());
    assert!(
        t.reader
            .list_missing_blobs(ListMissingBlobsOpts {
                cursor: None,
                limit: 1001,
            })
            .await
            .is_err()
    );
}

#[tokio::test]
async fn process_write_blobs_deletes_dereferenced() {
    let t = test_reader().await;
    let blob = upload(&t, b"dereference me").await;
    let cid = blob.get_cid().unwrap();
    let record_uri = "at://did:example:alice/app.bsky.feed.post/3jt5vlkoraa2a".to_owned();

    let create = PreparedWrite::Create(PreparedCreateOrUpdate {
        action: WriteOpAction::Create,
        uri: record_uri.clone(),
        cid,
        swap_cid: None,
        record: serde_json::from_value(serde_json::json!({
            "$type": "app.bsky.feed.post",
            "text": "with blob",
            "createdAt": "2023-01-01T00:00:00.000Z",
        }))
        .unwrap(),
        blobs: vec![prepared_ref(&blob)],
    });
    t.reader.process_write_blobs(vec![create]).await.unwrap();
    assert!(t.store.has_stored(cid).await.unwrap());
    assert_eq!(
        t.reader.get_records_for_blob(cid).await.unwrap(),
        vec![record_uri.as_str()]
    );

    // deleting the record dereferences and deletes the blob
    let delete = PreparedWrite::Delete(PreparedDelete {
        action: WriteOpAction::Delete,
        uri: record_uri.clone(),
        swap_cid: None,
    });
    t.reader.process_write_blobs(vec![delete]).await.unwrap();
    t.reader.background_queue.process_all().await;
    assert_eq!(t.reader.blob_count().await.unwrap(), 0);
    assert!(!t.store.has_stored(cid).await.unwrap());
    assert!(t.reader.get_records_for_blob(cid).await.unwrap().is_empty());

    // deleting a record with no blobs is a no-op
    let delete_again = PreparedWrite::Delete(PreparedDelete {
        action: WriteOpAction::Delete,
        uri: record_uri,
        swap_cid: None,
    });
    t.reader
        .process_write_blobs(vec![delete_again])
        .await
        .unwrap();
}

#[tokio::test]
async fn process_write_blobs_handles_updates() {
    let t = test_reader().await;
    let old_blob = upload(&t, b"old media").await;
    let old_cid = old_blob.get_cid().unwrap();
    let record_uri = "at://did:example:alice/app.bsky.feed.post/3jt5vlkoraa2a".to_owned();
    t.reader
        .verify_blob_and_make_permanent(prepared_ref(&old_blob))
        .await
        .unwrap();
    t.reader
        .associate_blob(prepared_ref(&old_blob), record_uri.clone())
        .await
        .unwrap();

    let new_blob = upload(&t, b"new media").await;
    let new_cid = new_blob.get_cid().unwrap();
    let update = PreparedWrite::Update(PreparedCreateOrUpdate {
        action: WriteOpAction::Update,
        uri: record_uri.clone(),
        cid: new_cid,
        swap_cid: None,
        record: serde_json::from_value(serde_json::json!({
            "$type": "app.bsky.feed.post",
            "text": "updated",
            "createdAt": "2023-01-01T00:00:00.000Z",
        }))
        .unwrap(),
        blobs: vec![prepared_ref(&new_blob)],
    });
    t.reader.process_write_blobs(vec![update]).await.unwrap();
    t.reader.background_queue.process_all().await;

    // old blob dereferenced and deleted, new blob permanent and associated
    assert!(!t.store.has_stored(old_cid).await.unwrap());
    assert!(t.store.has_stored(new_cid).await.unwrap());
    assert_eq!(
        t.reader.get_records_for_blob(new_cid).await.unwrap(),
        [record_uri]
    );
    assert_eq!(t.reader.blob_count().await.unwrap(), 1);
}

#[tokio::test]
async fn mixed_delete_and_create_keeps_new_blobs() {
    let t = test_reader().await;
    let old_blob = upload(&t, b"replaced media").await;
    let old_uri = "at://did:example:alice/app.bsky.feed.post/3jt5vlkoraa2a".to_owned();
    t.reader
        .verify_blob_and_make_permanent(prepared_ref(&old_blob))
        .await
        .unwrap();
    t.reader
        .associate_blob(prepared_ref(&old_blob), old_uri.clone())
        .await
        .unwrap();

    // re-upload the same bytes for a new record while deleting the old one
    let new_blob = upload(&t, b"replaced media").await;
    let new_uri = "at://did:example:alice/app.bsky.feed.post/3jt5vlkorbb2b".to_owned();
    let delete = PreparedWrite::Delete(PreparedDelete {
        action: WriteOpAction::Delete,
        uri: old_uri,
        swap_cid: None,
    });
    let create = PreparedWrite::Create(PreparedCreateOrUpdate {
        action: WriteOpAction::Create,
        uri: new_uri.clone(),
        cid: new_blob.get_cid().unwrap(),
        swap_cid: None,
        record: serde_json::from_value(serde_json::json!({
            "$type": "app.bsky.feed.post",
            "text": "recreated",
            "createdAt": "2023-01-01T00:00:00.000Z",
        }))
        .unwrap(),
        blobs: vec![prepared_ref(&new_blob)],
    });
    t.reader
        .process_write_blobs(vec![delete, create])
        .await
        .unwrap();
    t.reader.background_queue.process_all().await;
    // blob is kept because the create in the same commit references it
    let cid = new_blob.get_cid().unwrap();
    assert_eq!(t.reader.blob_count().await.unwrap(), 1);
    assert!(t.store.has_stored(cid).await.unwrap());
    assert_eq!(t.reader.get_records_for_blob(cid).await.unwrap(), [new_uri]);
}

#[tokio::test]
async fn keeps_blobs_still_referenced_elsewhere() {
    let t = test_reader().await;
    let blob = upload(&t, b"shared blob").await;
    let cid = blob.get_cid().unwrap();
    let uri_one = "at://did:example:alice/app.bsky.feed.post/3jt5vlkoraa2a".to_owned();
    let uri_two = "at://did:example:alice/app.bsky.feed.post/3jt5vlkorbb2b".to_owned();
    t.reader
        .verify_blob_and_make_permanent(prepared_ref(&blob))
        .await
        .unwrap();
    t.reader
        .associate_blob(prepared_ref(&blob), uri_one.clone())
        .await
        .unwrap();
    t.reader
        .associate_blob(prepared_ref(&blob), uri_two)
        .await
        .unwrap();

    let delete = PreparedWrite::Delete(PreparedDelete {
        action: WriteOpAction::Delete,
        uri: uri_one,
        swap_cid: None,
    });
    t.reader.process_write_blobs(vec![delete]).await.unwrap();
    t.reader.background_queue.process_all().await;
    // still referenced by uri_two, so blob row and bytes remain
    assert_eq!(t.reader.blob_count().await.unwrap(), 1);
    assert!(t.store.has_stored(cid).await.unwrap());
}
