//! Integration tests for the actor store.

use super::*;
use crate::blobstore::opendal::OpenDALBlobStore;
use rsky_crypto::utils::encode_did_key;
use rsky_repo::block_map::BlockMap;
use rsky_repo::cid_set::CidSet;
use rsky_repo::types::{
    CommitAction, CommitData, PreparedCreateOrUpdate, PreparedDelete, PreparedWrite, WriteOpAction,
};
use rsky_syntax::aturi::AtUri;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};

const TEST_DID: &str = "did:example:alice";

/// Per-test in-memory OpenDAL blobstore. The shared `OpenDALBlobStore`
/// constructed per-test keeps each test isolated (no `OnceLock`).
fn test_blobstore(did: &str) -> Arc<dyn BlobStore<Stream = BoxedBlobStream>> {
    Arc::new(OpenDALBlobStore::new_memory(did).unwrap())
}
// A well-known secp256k1 secret (deterministic test key).
const TEST_SECRET_HEX: &str = "1d2f8064213bd212453fa93943c084dbbf42104d02f1f02b23a638f9a48f925a";

fn cid(value: &str) -> Cid {
    Cid::from_str(value).expect("test CID parses")
}

fn current() -> Cid {
    cid("bafkreihdwdcefgh4dqkjv67uzcmw7ojee6xedzdetojuzjevtenxquvyku")
}

fn other() -> Cid {
    cid("bafkreibjfgx2gprinfvicegelk5kosd6y2frmqpqzwqkg7usac74l3t2v4")
}

fn test_keypair() -> Keypair {
    import_keypair(&hex::decode(TEST_SECRET_HEX).expect("valid hex")).expect("valid test secret")
}

fn test_store(cache_size: usize) -> (camino_tempfile::Utf8TempDir, ActorStore) {
    let dir = camino_tempfile::Utf8TempDir::new().expect("tempdir");
    let cfg = ActorStoreConfig {
        directory: dir.path().join("actors").to_string(),
        cache_size,
    };
    let store = ActorStore::new(&cfg);
    (dir, store)
}

fn post_write(rkey: &str, text: &str) -> PreparedCreateOrUpdate {
    let record: rsky_repo::types::RepoRecord = serde_json::from_value(serde_json::json!({
        "$type": "app.bsky.feed.post",
        "text": text,
        "createdAt": "2023-01-01T00:00:00.000Z",
    }))
    .expect("valid record");
    let cid = rsky_common::ipld::cid_for_cbor(&record).expect("cid for cbor");
    PreparedCreateOrUpdate {
        action: WriteOpAction::Create,
        uri: format!("at://{TEST_DID}/app.bsky.feed.post/{rkey}"),
        cid,
        swap_cid: None,
        record,
        blobs: vec![],
    }
}

#[test]
fn rejects_unsafe_path_parts() {
    assert!(assert_safe_path_part("did:example:alice").is_ok());
    assert!(assert_safe_path_part("").is_err());
    assert!(assert_safe_path_part(".hidden").is_err());
    assert!(assert_safe_path_part("a/b").is_err());
    assert!(assert_safe_path_part("a\\b").is_err());
    assert!(assert_safe_path_part("a..b").is_err());
}

#[test]
fn no_swap_cid_skips_checks() {
    assert!(check_record_swap(&WriteOpAction::Update, &Some(current()), &None).is_ok());
    assert!(check_record_swap(&WriteOpAction::Delete, &None, &None).is_ok());
    assert!(check_record_swap(&WriteOpAction::Create, &None, &None).is_ok());
}

#[test]
fn create_with_swap_cid_is_rejected() {
    assert!(matches!(
        check_record_swap(&WriteOpAction::Create, &None, &Some(current())),
        Err(FormatCommitError::BadRecordSwap(_))
    ));
}

#[test]
fn matching_swap_cid_is_accepted() {
    assert!(check_record_swap(&WriteOpAction::Update, &Some(current()), &Some(current())).is_ok());
    assert!(check_record_swap(&WriteOpAction::Delete, &Some(current()), &Some(current())).is_ok());
}

#[test]
fn mismatched_or_missing_current_record_is_rejected() {
    assert!(matches!(
        check_record_swap(&WriteOpAction::Update, &Some(current()), &Some(other())),
        Err(FormatCommitError::RecordSwapMismatch(_))
    ));
    assert!(matches!(
        check_record_swap(&WriteOpAction::Delete, &None, &Some(current())),
        Err(FormatCommitError::RecordSwapMismatch(_))
    ));
}

#[test]
fn format_commit_errors_display() {
    assert!(
        FormatCommitError::BadRecordSwap("x".to_owned())
            .to_string()
            .contains("BadRecordSwapError")
    );
    assert!(
        FormatCommitError::RecordSwapMismatch("x".to_owned())
            .to_string()
            .contains("current record")
    );
    assert!(
        FormatCommitError::BadCommitSwap("cid".to_owned())
            .to_string()
            .contains("BadCommitSwapError")
    );
    assert!(
        FormatCommitError::MissingRepoRoot("did".to_owned())
            .to_string()
            .contains("No repo root")
    );
}

#[tokio::test]
async fn location_shards_by_did_hash() {
    let (_dir, store) = test_store(10);
    let location = store.get_location(TEST_DID).unwrap();
    let did_hash = hex::encode(Sha256::digest(TEST_DID.as_bytes()));
    assert!(location.directory.to_string_lossy().contains(&format!(
        "{}/{}",
        &did_hash[0..2],
        TEST_DID
    )));
    assert_eq!(
        location.db_location.file_name().unwrap().to_str().unwrap(),
        "store.sqlite"
    );
    assert_eq!(
        location.key_location.file_name().unwrap().to_str().unwrap(),
        "key"
    );
    assert!(store.get_location("bad/did").is_err());
}

#[tokio::test]
async fn create_open_keypair_destroy_roundtrip() {
    let (_dir, store) = test_store(10);
    let keypair = test_keypair();

    assert!(!store.exists(TEST_DID).await.unwrap());
    assert!(store.keypair(TEST_DID).await.is_err());
    assert!(
        store
            .read(TEST_DID.to_owned(), test_blobstore(TEST_DID))
            .await
            .is_err()
    );
    assert!(
        store
            .transact(TEST_DID.to_owned(), test_blobstore(TEST_DID))
            .await
            .is_err()
    );

    store.create(TEST_DID, &keypair).await.unwrap();
    assert!(store.exists(TEST_DID).await.unwrap());
    // creating again fails
    assert!(store.create(TEST_DID, &keypair).await.is_err());

    let loaded = store.keypair(TEST_DID).await.unwrap();
    assert_eq!(loaded.secret_bytes(), keypair.secret_bytes());

    let reader = store
        .read(TEST_DID.to_owned(), test_blobstore(TEST_DID))
        .await
        .unwrap();
    assert_eq!(reader.did, TEST_DID);
    assert_eq!(
        reader.keypair().await.unwrap().secret_bytes(),
        keypair.secret_bytes()
    );
    assert!(reader.get_repo_root().await.unwrap().is_none());

    store
        .destroy(TEST_DID, test_blobstore(TEST_DID))
        .await
        .unwrap();
    assert!(!store.exists(TEST_DID).await.unwrap());
    // destroying a missing actor is a no-op
    store
        .destroy(TEST_DID, test_blobstore(TEST_DID))
        .await
        .unwrap();
}

#[tokio::test]
async fn concurrent_transactions_serialize_per_did() {
    use std::sync::Arc;
    let (_dir, store) = test_store(10);
    let store = Arc::new(store);
    store.create(TEST_DID, &test_keypair()).await.unwrap();

    let running = Arc::new(AtomicUsize::new(0));
    let max_running = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();
    for _ in 0..4 {
        let store = store.clone();
        let running = running.clone();
        let max_running = max_running.clone();
        handles.push(tokio::spawn(async move {
            let txn = store
                .transact(TEST_DID.to_owned(), test_blobstore(TEST_DID))
                .await
                .unwrap();
            let now = running.fetch_add(1, Ordering::SeqCst) + 1;
            max_running.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            running.fetch_sub(1, Ordering::SeqCst);
            drop(txn);
        }));
    }
    for handle in handles {
        handle.await.unwrap();
    }
    assert_eq!(max_running.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn lru_evicts_least_recently_used_db() {
    let (_dir, store) = test_store(1);
    let keypair = test_keypair();
    let did_bob = "did:example:bob";
    store.create(TEST_DID, &keypair).await.unwrap();
    store.create(did_bob, &keypair).await.unwrap();

    // bob was cached last (cache size 1), so alice must be re-opened from disk
    let alice_location = store.get_location(TEST_DID).unwrap();
    tokio::fs::remove_file(&alice_location.db_location)
        .await
        .unwrap();
    assert!(
        store
            .read(TEST_DID.to_owned(), test_blobstore(TEST_DID))
            .await
            .is_err()
    );
    // bob is still served from the cache even with the file gone
    let bob_location = store.get_location(did_bob).unwrap();
    tokio::fs::remove_file(&bob_location.db_location)
        .await
        .unwrap();
    assert!(
        store
            .read(did_bob.to_owned(), test_blobstore(did_bob))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn reopens_evicted_db_from_disk() {
    let (_dir, store) = test_store(1);
    let keypair = test_keypair();
    store.create(TEST_DID, &keypair).await.unwrap();
    // bob evicts alice from the single-entry cache
    store.create("did:example:bob", &keypair).await.unwrap();
    // alice re-opens from disk
    let reader = store
        .read(TEST_DID.to_owned(), test_blobstore(TEST_DID))
        .await
        .unwrap();
    assert!(reader.get_repo_root().await.unwrap().is_none());
}

#[tokio::test]
async fn reader_keypair_errors_when_key_missing() {
    let (_dir, store) = test_store(10);
    store.create(TEST_DID, &test_keypair()).await.unwrap();
    let location = store.get_location(TEST_DID).unwrap();
    tokio::fs::remove_file(&location.key_location)
        .await
        .unwrap();
    let reader = store
        .read(TEST_DID.to_owned(), test_blobstore(TEST_DID))
        .await
        .unwrap();
    assert!(reader.keypair().await.is_err());
    assert!(
        store
            .transact(TEST_DID.to_owned(), test_blobstore(TEST_DID))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn reserved_keypair_lifecycle() {
    let (_dir, store) = test_store(10);
    // reserving without a did keys the file by the key's own did
    let key_did = store.reserve_keypair(None).await.unwrap();
    assert!(key_did.starts_with("did:key:"));
    let loaded = store.get_reserved_keypair(&key_did).await.unwrap().unwrap();
    assert_eq!(encode_did_key(&loaded.public_key()), key_did);

    // reserving for a did is idempotent
    let for_did = store.reserve_keypair(Some(TEST_DID)).await.unwrap();
    let again = store.reserve_keypair(Some(TEST_DID)).await.unwrap();
    assert_eq!(for_did, again);
    assert!(
        store
            .get_reserved_keypair(TEST_DID)
            .await
            .unwrap()
            .is_some()
    );

    // clearing with a did that has no reserved file is a no-op for that file
    store
        .clear_reserved_keypair(&key_did, Some("did:example:nobody"))
        .await
        .unwrap();
    store
        .clear_reserved_keypair(&for_did, Some(TEST_DID))
        .await
        .unwrap();
    assert!(
        store
            .get_reserved_keypair(&key_did)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .get_reserved_keypair(TEST_DID)
            .await
            .unwrap()
            .is_none()
    );
    // clearing again is a no-op
    store.clear_reserved_keypair(&key_did, None).await.unwrap();
    assert!(store.reserve_keypair(Some("bad/did")).await.is_err());
}

#[tokio::test]
async fn reserved_key_load_errors_on_unreadable_file() {
    let (_dir, store) = test_store(10);
    tokio::fs::create_dir_all(store.reserved_key_dir.join("not-a-file"))
        .await
        .unwrap();
    assert!(store.get_reserved_keypair("not-a-file").await.is_err());
    assert!(store.get_reserved_keypair("bad/path").await.is_err());
}

#[tokio::test]
async fn create_repo_with_initial_writes() {
    let (_dir, store) = test_store(10);
    store.create(TEST_DID, &test_keypair()).await.unwrap();
    let mut txn = store
        .transact(TEST_DID.to_owned(), test_blobstore(TEST_DID))
        .await
        .unwrap();
    let write = post_write("3jt5vlkoraa2a", "first post");
    let commit = txn.create_repo(vec![write.clone()]).await.unwrap();
    assert_eq!(commit.ops.len(), 1);
    assert_eq!(commit.ops[0].cid, Some(write.cid));
    let write_uri: AtUri = write.uri.clone().try_into().unwrap();
    let got = txn
        .record
        .get_record(&write_uri, None, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.cid, write.cid.to_string());

    // the repo loads back from storage and verifies: repo_root exists,
    // the record is indexed, and Repo::load re-derives the commit
    let storage_dyn: std::sync::Arc<tokio::sync::RwLock<dyn RepoStorage>> = txn.storage.clone();
    let repo = rsky_repo::repo::Repo::load(storage_dyn, None)
        .await
        .unwrap();
    assert_eq!(repo.did(), TEST_DID);
    assert_eq!(repo.cid, commit.commit_data.cid);

    // matching swap-commit is accepted
    let root = txn.get_repo_root().await.unwrap().expect("root after init");
    let next = post_write("3jt5vlkorbb2b", "second post");
    txn.process_writes(vec![PreparedWrite::Create(next)], Some(root))
        .await
        .unwrap();
}

#[tokio::test]
async fn create_account_write_and_read_back_records() {
    let (_dir, store) = test_store(10);
    let keypair = test_keypair();
    store.create(TEST_DID, &keypair).await.unwrap();

    // initialize the repo
    let mut txn = store
        .transact(TEST_DID.to_owned(), test_blobstore(TEST_DID))
        .await
        .unwrap();
    let init_commit = txn.create_repo(vec![]).await.unwrap();
    assert!(init_commit.ops.is_empty());
    assert!(init_commit.prev_data.is_none());
    let root = txn.get_repo_root().await.unwrap();
    assert_eq!(root, Some(init_commit.commit_data.cid));

    // create a record
    let create = post_write("3jt5vlkoraa2a", "hello world");
    let create_uri: AtUri = create.uri.clone().try_into().unwrap();
    let commit = txn
        .process_writes(vec![PreparedWrite::Create(create.clone())], None)
        .await
        .unwrap();
    assert_eq!(commit.ops.len(), 1);
    assert!(matches!(commit.ops[0].action, CommitAction::Create));
    assert_eq!(commit.ops[0].cid, Some(create.cid));
    assert!(commit.ops[0].prev.is_none());
    assert!(commit.prev_data.is_some());

    let got = txn
        .record
        .get_record(&create_uri, None, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.cid, create.cid.to_string());

    // swap-commit mismatch is rejected
    let unrelated = post_write("3jt5vlkorbb2b", "swap target");
    let bad_swap = txn
        .process_writes(
            vec![PreparedWrite::Create(unrelated.clone())],
            Some(other()),
        )
        .await;
    assert!(bad_swap.is_err());

    // update the record
    let mut update = post_write("3jt5vlkoraa2a", "hello again");
    update.action = WriteOpAction::Update;
    let update_commit = txn
        .process_writes(vec![PreparedWrite::Update(update.clone())], None)
        .await
        .unwrap();
    assert_eq!(update_commit.ops[0].prev, Some(create.cid));
    let got = txn
        .record
        .get_record(&create_uri, None, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.cid, update.cid.to_string());

    // sync event data and car stream are readable
    let sync_data = txn.get_sync_event_data().await.unwrap();
    assert_eq!(sync_data.cid, update_commit.commit_data.cid);
    drop(txn);

    let reader = store
        .read(TEST_DID.to_owned(), test_blobstore(TEST_DID))
        .await
        .unwrap();
    {
        let storage_guard = reader.storage.read().await;
        let car = storage_guard.get_car_stream(None).await.unwrap();
        assert!(!car.is_empty());
    }

    // delete the record
    let mut txn = store
        .transact(TEST_DID.to_owned(), test_blobstore(TEST_DID))
        .await
        .unwrap();
    let delete = PreparedWrite::Delete(PreparedDelete {
        action: WriteOpAction::Delete,
        uri: create.uri.clone(),
        swap_cid: None,
    });
    let delete_commit = txn.process_writes(vec![delete], None).await.unwrap();
    assert!(matches!(delete_commit.ops[0].action, CommitAction::Delete));
    assert_eq!(delete_commit.ops[0].prev, Some(update.cid));
    assert!(
        txn.record
            .get_record(&create_uri, None, None)
            .await
            .unwrap()
            .is_none()
    );

    // no duplicate cids for empty inputs
    assert!(
        txn.get_duplicate_record_cids(vec![], vec![])
            .await
            .unwrap()
            .is_empty()
    );
    drop(txn);

    store
        .destroy(TEST_DID, test_blobstore(TEST_DID))
        .await
        .unwrap();
    assert!(!store.exists(TEST_DID).await.unwrap());
}

#[tokio::test]
async fn process_writes_requires_repo_root() {
    let (_dir, store) = test_store(10);
    store.create(TEST_DID, &test_keypair()).await.unwrap();
    let mut txn = store
        .transact(TEST_DID.to_owned(), test_blobstore(TEST_DID))
        .await
        .unwrap();
    let res = txn
        .process_writes(
            vec![PreparedWrite::Create(post_write(
                "3jt5vlkoraa2a",
                "no root",
            ))],
            None,
        )
        .await;
    assert!(res.is_err());
}

#[tokio::test]
async fn duplicate_record_cids_are_detected() {
    let (_dir, store) = test_store(10);
    store.create(TEST_DID, &test_keypair()).await.unwrap();
    let mut txn = store
        .transact(TEST_DID.to_owned(), test_blobstore(TEST_DID))
        .await
        .unwrap();
    txn.create_repo(vec![]).await.unwrap();

    // two records with identical content share a cid
    let write_one = post_write("3jt5vlkoraa2a", "same content");
    let write_two = PreparedCreateOrUpdate {
        uri: format!("at://{TEST_DID}/app.bsky.feed.post/3jt5vlkorbb2b"),
        ..write_one.clone()
    };
    txn.process_writes(
        vec![
            PreparedWrite::Create(write_one.clone()),
            PreparedWrite::Create(write_two.clone()),
        ],
        None,
    )
    .await
    .unwrap();

    let write_one_uri: AtUri = write_one.uri.clone().try_into().unwrap();
    let dupes = txn
        .get_duplicate_record_cids(vec![write_one.cid], vec![write_one_uri])
        .await
        .unwrap();
    assert_eq!(dupes, vec![write_two.cid]);

    // deleting only one of the two keeps the shared block for the other
    let delete = PreparedWrite::Delete(PreparedDelete {
        action: WriteOpAction::Delete,
        uri: write_one.uri.clone(),
        swap_cid: None,
    });
    txn.process_writes(vec![delete], None).await.unwrap();
    let write_two_uri: AtUri = write_two.uri.clone().try_into().unwrap();
    let got = txn
        .record
        .get_record(&write_two_uri, None, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.cid, write_two.cid.to_string());
}

#[tokio::test]
async fn process_import_repo_applies_commit_and_writes() {
    let (_dir, store) = test_store(10);
    store.create(TEST_DID, &test_keypair()).await.unwrap();
    let mut txn = store
        .transact(TEST_DID.to_owned(), test_blobstore(TEST_DID))
        .await
        .unwrap();
    let init = txn.create_repo(vec![]).await.unwrap();

    let write = post_write("3jt5vlkoraa2a", "imported");
    let commit = CommitData {
        cid: init.commit_data.cid,
        rev: "3jt5vlkorxx2x".to_owned(),
        since: None,
        prev: None,
        new_blocks: BlockMap::new(),
        relevant_blocks: BlockMap::new(),
        removed_cids: CidSet::new(None),
    };
    txn.process_import_repo(commit, vec![PreparedWrite::Create(write.clone())])
        .await
        .unwrap();
    let write_uri: AtUri = write.uri.try_into().unwrap();
    // process_import_repo index_writes the row but the record body itself
    // comes from the (caller-supplied) commit blocks — when the synthetic
    // commit has empty blocks, only the row index is populated.
    assert!(
        txn.record
            .has_record("app.bsky.feed.post".into(), "3jt5vlkoraa2a".into(), None)
            .await
            .unwrap()
    );
    // the row exists but `get_record` returns it with no value field populated.
    let got = txn.record.get_record(&write_uri, None, None).await.unwrap();
    assert!(got.is_some());
}

#[tokio::test]
async fn moved_record_keeps_shared_blocks() {
    let (_dir, store) = test_store(10);
    store.create(TEST_DID, &test_keypair()).await.unwrap();
    let mut txn = store
        .transact(TEST_DID.to_owned(), test_blobstore(TEST_DID))
        .await
        .unwrap();
    txn.create_repo(vec![]).await.unwrap();

    let original = post_write("3jt5vlkoraa2a", "same content");
    txn.process_writes(vec![PreparedWrite::Create(original.clone())], None)
        .await
        .unwrap();

    // move the record: delete the old rkey and re-create the same content
    // under a new rkey in a single commit (cid stays the same)
    let moved = PreparedCreateOrUpdate {
        uri: format!("at://{TEST_DID}/app.bsky.feed.post/3jt5vlkorbb2b"),
        ..original.clone()
    };
    let delete = PreparedWrite::Delete(PreparedDelete {
        action: WriteOpAction::Delete,
        uri: original.uri.clone(),
        swap_cid: None,
    });
    let commit = txn
        .process_writes(vec![delete, PreparedWrite::Create(moved.clone())], None)
        .await
        .unwrap();
    assert_eq!(commit.ops.len(), 2);

    let moved_uri: AtUri = moved.uri.clone().try_into().unwrap();
    let got = txn
        .record
        .get_record(&moved_uri, None, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.cid, original.cid.to_string());
}
