# Task 11: Repo prepare + mod — `repo/prepare.rs` and `repo/mod.rs`

**Files:**
- Create: `pds/src/repo/mod.rs`
- Create: `pds/src/repo/prepare.rs`
- Modify: `pds/src/xrpc/com/atproto/repo/mod.rs` (add `assert_repo_availability` + route table; replace Task 1 placeholder)
- Test: in `pds/src/repo/prepare.rs` (in-file unit tests)

- [ ] **Step 1: Write the failing tests**

```rust
// appended to pds/src/repo/prepare.rs
#[cfg(test)]
mod tests {
    use super::*;

    fn blob_json(cid: &str, mime_type: &str, size: i64) -> JsonValue {
        json!({
            "$type": "blob",
            "ref": { "$link": cid },
            "mimeType": mime_type,
            "size": size,
        })
    }

    fn image_post_record() -> RepoRecord {
        serde_json::from_value(json!({
            "$type": "app.bsky.feed.post",
            "text": "image post",
            "createdAt": "2026-01-01T00:00:00.000Z",
            "embed": {
                "$type": "app.bsky.embed.images",
                "images": [
                    { "image": blob_json("bafkreihdwdcefgh4dqkjv67uzcmw7ojee6xedzdetojuzjevtenxquvyku", "image/png", 512), "alt": "" }
                ],
            },
        }))
        .expect("record deserializes")
    }

    #[test]
    fn finds_blob_refs_in_image_embed() {
        let refs = find_blob_refs(Lex::Map(image_post_record()), None, None);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].path.join("/"), "embed/images/image");
    }

    #[test]
    fn asserts_valid_record_requires_type() {
        let record: RepoRecord = serde_json::from_value(json!({ "text": "no type" })).unwrap();
        assert!(assert_valid_record(&record).is_err());
        let record: RepoRecord =
            serde_json::from_value(json!({ "$type": "app.bsky.feed.post", "text": "x" })).unwrap();
        assert!(assert_valid_record(&record).is_ok());
    }
}
```

Run: `cargo test -p pds repo::prepare::tests`
Expected: FAIL — module missing.

- [ ] **Step 2: Implement `repo/prepare.rs`**

Port of `the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/repo/prepare.rs`. The `CONSTRAINTS` lexicon lookup (app.bsky blob size/mime limits) is dropped — `blobs_for_write` returns unconstrained `PreparedBlobRef`s (decision stated in the header).

```rust
// pds/src/repo/prepare.rs
use anyhow::bail;
use lexicon_cid::Cid;
use rsky_common::ipld::cid_for_cbor;
use rsky_common::tid::Ticker;
use rsky_lexicon::blob_refs::{BlobRef, JsonBlobRef};
use rsky_repo::storage::Ipld;
use rsky_repo::types::{
    BlobConstraint, Lex, PreparedBlobRef, PreparedCreateOrUpdate, PreparedDelete, RepoRecord,
    WriteOpAction,
};
use rsky_repo::util::{cbor_to_lex, lex_to_ipld};
use rsky_syntax::aturi::AtUri;
use serde_json::{json, Value as JsonValue};

#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct FoundBlobRef {
    pub r#ref: BlobRef,
    pub path: Vec<String>,
}

pub struct PrepareCreateOpts {
    pub did: String,
    pub collection: String,
    pub rkey: Option<String>,
    pub swap_cid: Option<Cid>,
    pub record: RepoRecord,
    pub validate: Option<bool>,
}

pub struct PrepareUpdateOpts {
    pub did: String,
    pub collection: String,
    pub rkey: String,
    pub swap_cid: Option<Cid>,
    pub record: RepoRecord,
    pub validate: Option<bool>,
}

pub struct PrepareDeleteOpts {
    pub did: String,
    pub collection: String,
    pub rkey: String,
    pub swap_cid: Option<Cid>,
}

pub fn blobs_for_write(record: RepoRecord, _validate: bool) -> anyhow::Result<Vec<PreparedBlobRef>> {
    let refs = find_blob_refs(Lex::Map(record.clone()), None, None);
    for r#ref in refs.clone() {
        if matches!(r#ref.r#ref.original, JsonBlobRef::Untyped(_)) {
            bail!("Legacy blob ref at `{}`", r#ref.path.join("/"))
        }
    }
    refs.into_iter()
        .map(|FoundBlobRef { r#ref, path: _ }| {
            // cacos does not port the app.bsky lexicon constraint table
            // (generated lexicons.toml); blob size/mime constraints are None.
            Ok(PreparedBlobRef {
                cid: r#ref.get_cid()?,
                mime_type: r#ref.get_mime_type().to_string(),
                constraints: BlobConstraint {
                    max_size: None,
                    accept: None,
                },
            })
        })
        .collect::<anyhow::Result<Vec<PreparedBlobRef>>>()
}

pub fn find_blob_refs(val: Lex, path: Option<Vec<String>>, layer: Option<u8>) -> Vec<FoundBlobRef> {
    let layer = layer.unwrap_or(0);
    let path = path.unwrap_or_default();
    if layer > 32 {
        return vec![];
    }
    match val {
        Lex::List(list) => list
            .into_iter()
            .flat_map(|item| find_blob_refs(item, Some(path.clone()), Some(layer + 1)))
            .collect::<Vec<FoundBlobRef>>(),
        Lex::Blob(blob) => vec![FoundBlobRef { r#ref: blob, path }],
        Lex::Ipld(Ipld::Json(JsonValue::Array(list))) => list
            .into_iter()
            .flat_map(|item| match serde_json::from_value::<RepoRecord>(item) {
                Ok(item) => find_blob_refs(Lex::Map(item), Some(path.clone()), Some(layer + 1)),
                Err(_) => vec![],
            })
            .collect::<Vec<FoundBlobRef>>(),
        Lex::Ipld(Ipld::Json(json)) => match serde_json::from_value::<JsonBlobRef>(json.clone()) {
            Ok(blob) => vec![FoundBlobRef {
                r#ref: BlobRef { original: blob },
                path,
            }],
            Err(_) => match serde_json::from_value::<RepoRecord>(json) {
                Ok(record) => record
                    .into_iter()
                    .flat_map(|(key, item)| {
                        find_blob_refs(
                            item,
                            Some([path.as_slice(), [key].as_slice()].concat()),
                            Some(layer + 1),
                        )
                    })
                    .collect::<Vec<FoundBlobRef>>(),
                Err(_) => vec![],
            },
        },
        Lex::Ipld(Ipld::Map(map)) => match serde_json::to_value(Ipld::Map(map.clone()))
            .ok()
            .and_then(|json| serde_json::from_value::<JsonBlobRef>(json).ok())
        {
            Some(blob) => vec![FoundBlobRef {
                r#ref: BlobRef { original: blob },
                path,
            }],
            None => map
                .into_iter()
                .flat_map(|(key, item)| {
                    find_blob_refs(
                        Lex::Ipld(item),
                        Some([path.as_slice(), [key].as_slice()].concat()),
                        Some(layer + 1),
                    )
                })
                .collect::<Vec<FoundBlobRef>>(),
        },
        Lex::Ipld(Ipld::List(list)) => list
            .into_iter()
            .flat_map(|item| find_blob_refs(Lex::Ipld(item), Some(path.clone()), Some(layer + 1)))
            .collect::<Vec<FoundBlobRef>>(),
        Lex::Ipld(_) => vec![],
        Lex::Map(map) => map
            .into_iter()
            .flat_map(|(key, item)| {
                find_blob_refs(
                    item,
                    Some([path.as_slice(), [key].as_slice()].concat()),
                    Some(layer + 1),
                )
            })
            .collect::<Vec<FoundBlobRef>>(),
    }
}

pub fn assert_valid_record(record: &RepoRecord) -> anyhow::Result<()> {
    match record.get("$type") {
        Some(Lex::Ipld(Ipld::String(_))) => Ok(()),
        _ => bail!("No $type provided"),
    }
}

pub fn set_collection_name(
    collection: &String,
    mut record: RepoRecord,
    validate: bool,
) -> anyhow::Result<RepoRecord> {
    if !record.contains_key("$type") {
        record.insert(
            "$type".to_string(),
            Lex::Ipld(Ipld::Json(JsonValue::String(collection.clone()))),
        );
    }
    if let Some(Lex::Ipld(Ipld::Json(JsonValue::String(record_type)))) = record.get("$type") {
        if validate && record_type != collection {
            bail!("Invalid $type: expected {collection}, got {record_type}")
        }
    }
    Ok(record)
}

pub async fn cid_for_safe_record(record: RepoRecord) -> anyhow::Result<Cid> {
    let lex = lex_to_ipld(Lex::Map(record));
    let block = serde_ipld_dagcbor::to_vec(&lex)?;
    // Confirm whether Block properly transforms between lex and cbor
    let _ = cbor_to_lex(block)?;
    cid_for_cbor(&lex)
}

pub async fn prepare_create(opts: PrepareCreateOpts) -> anyhow::Result<PreparedCreateOrUpdate> {
    let PrepareCreateOpts {
        did,
        collection,
        rkey,
        swap_cid,
        validate,
        ..
    } = opts;
    let validate = validate.unwrap_or(true);

    let record = set_collection_name(&collection, opts.record, validate)?;
    if validate {
        assert_valid_record(&record)?;
    }

    let next_rkey = Ticker::new().next(None);
    let rkey = rkey.unwrap_or(next_rkey.to_string());
    let uri = AtUri::make(did, Some(collection), Some(rkey))?;
    Ok(PreparedCreateOrUpdate {
        action: WriteOpAction::Create,
        uri: uri.to_string(),
        cid: cid_for_safe_record(record.clone()).await?,
        swap_cid,
        record: record.clone(),
        blobs: blobs_for_write(record, validate)?,
    })
}

pub async fn prepare_update(opts: PrepareUpdateOpts) -> anyhow::Result<PreparedCreateOrUpdate> {
    let PrepareUpdateOpts {
        did,
        collection,
        rkey,
        swap_cid,
        validate,
        ..
    } = opts;
    let validate = validate.unwrap_or(true);

    let record = set_collection_name(&collection, opts.record, validate)?;
    if validate {
        assert_valid_record(&record)?;
    }
    let uri = AtUri::make(did, Some(collection), Some(rkey))?;
    Ok(PreparedCreateOrUpdate {
        action: WriteOpAction::Update,
        uri: uri.to_string(),
        cid: cid_for_safe_record(record.clone()).await?,
        swap_cid,
        record: record.clone(),
        blobs: blobs_for_write(record, validate)?,
    })
}

pub fn prepare_delete(opts: PrepareDeleteOpts) -> anyhow::Result<PreparedDelete> {
    let PrepareDeleteOpts {
        did,
        collection,
        rkey,
        swap_cid,
    } = opts;
    let uri = AtUri::make(did, Some(collection), Some(rkey))?;
    Ok(PreparedDelete {
        action: WriteOpAction::Delete,
        uri: uri.to_string(),
        swap_cid,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blob_json(cid: &str, mime_type: &str, size: i64) -> JsonValue {
        json!({
            "$type": "blob",
            "ref": { "$link": cid },
            "mimeType": mime_type,
            "size": size,
        })
    }

    fn image_post_record() -> RepoRecord {
        serde_json::from_value(json!({
            "$type": "app.bsky.feed.post",
            "text": "image post",
            "createdAt": "2026-01-01T00:00:00.000Z",
            "embed": {
                "$type": "app.bsky.embed.images",
                "images": [
                    { "image": blob_json("bafkreihdwdcefgh4dqkjv67uzcmw7ojee6xedzdetojuzjevtenxquvyku", "image/png", 512), "alt": "" }
                ],
            },
        }))
        .expect("record deserializes")
    }

    #[test]
    fn finds_blob_refs_in_image_embed() {
        let refs = find_blob_refs(Lex::Map(image_post_record()), None, None);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].path.join("/"), "embed/images/image");
    }

    #[test]
    fn asserts_valid_record_requires_type() {
        let record: RepoRecord = serde_json::from_value(json!({ "text": "no type" })).unwrap();
        assert!(assert_valid_record(&record).is_err());
        let record: RepoRecord =
            serde_json::from_value(json!({ "$type": "app.bsky.feed.post", "text": "x" })).unwrap();
        assert!(assert_valid_record(&record).is_ok());
    }
}
```

- [ ] **Step 3: Implement `repo/mod.rs` and the repo route table + `assert_repo_availability`**

```rust
// pds/src/repo/mod.rs
pub mod prepare;
```

Replace `pds/src/xrpc/com/atproto/repo/mod.rs` (was the Task 1 placeholder):

```rust
// pds/src/xrpc/com/atproto/repo/mod.rs
pub mod apply_writes;
pub mod create_record;
pub mod delete_record;
pub mod describe_repo;
pub mod get_record;
pub mod import_repo;
pub mod list_missing_blobs;
pub mod list_records;
pub mod put_record;
pub mod upload_blob;

use crate::account::helpers::account::{ActorAccount, AvailabilityFlags};
use crate::account::AccountManager;
use anyhow::{bail, Result};

pub async fn assert_repo_availability(
    did: &String,
    is_admin_of_self: bool,
    account_manager: &AccountManager,
) -> Result<ActorAccount> {
    let account = account_manager
        .get_account(
            did,
            Some(AvailabilityFlags {
                include_deactivated: Some(true),
                include_taken_down: Some(true),
            }),
        )
        .await?;
    match account {
        None => bail!("RepoNotFound: Could not find repo for DID: {did}"),
        Some(account) => {
            if is_admin_of_self {
                return Ok(account);
            }
            if account.takedown_ref.is_some() {
                bail!("RepoTakendown: Repo has been takendown: {did}");
            }
            if account.deactivated_at.is_some() {
                bail!("RepoDeactivated: Repo has been deactivated: {did}");
            }
            Ok(account)
        }
    }
}

pub fn routes() -> poem::Route {
    use poem::get;
    use poem::post;
    poem::Route::new()
        .at("/applyWrites", post(apply_writes::apply_writes))
        .at("/createRecord", post(create_record::create_record))
        .at("/deleteRecord", post(delete_record::delete_record))
        .at("/describeRepo", get(describe_repo::describe_repo))
        .at("/getRecord", get(get_record::get_record))
        .at("/importRepo", post(import_repo::import_repo))
        .at("/listMissingBlobs", get(list_missing_blobs::list_missing_blobs))
        .at("/listRecords", get(list_records::list_records))
        .at("/putRecord", post(put_record::put_record))
        .at("/uploadBlob", post(upload_blob::upload_blob))
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p pds repo::prepare::tests`
Expected: PASS (2 tests). Handler files are created in Tasks 12–17; create stub handlers returning `ApiError::RuntimeError` for not-yet-ported routes to keep the build green between tasks (or implement Tasks 12–17 before running `cargo check`).

- [ ] **Step 5: Commit**

```bash
git add pds/src/repo pds/src/xrpc/com/atproto/repo/mod.rs
git commit -m "feat(repo): prepare helpers and assert_repo_availability"
```
