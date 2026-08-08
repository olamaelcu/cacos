# Task 18: PLC client port — `plc/types.rs`, `plc/operations.rs`, `plc/mod.rs`

**Files:**
- Create: `pds/src/plc/mod.rs`
- Create: `pds/src/plc/types.rs`
- Create: `pds/src/plc/operations.rs`
- Test: `pds/src/plc/operations.rs` (in-file unit tests for op helpers)

- [ ] **Step 1: Write the failing tests**

```rust
// appended to pds/src/plc/operations.rs
#[cfg(test)]
mod tests {
    use super::*;
    use secp256k1::{Secp256k1, SecretKey};

    fn test_key() -> SecretKey {
        SecretKey::from_slice(&hex::decode("9d5907143471e8f0e8df0f8b9512a8c5377878ee767f18fcf961055ecfc071cd").unwrap()).unwrap()
    }

    #[tokio::test]
    async fn creates_and_signs_a_create_operation() {
        let (did, op) = create_op(
            CreateAtprotoOpInput {
                signing_key: "did:key:zQ3shXkXxRqVnGX6fYqPqL4h6F5TpCxYhZJcMvBtNwRpKsUdEiF".to_string(),
                handle: "alice.test".to_string(),
                pds: "http://localhost:2583".to_string(),
                rotation_keys: vec!["did:key:zRotation".to_string()],
            },
            test_key(),
        )
        .await
        .unwrap();
        assert!(did.starts_with("did:plc:"));
        assert_eq!(op.r#type, "plc_operation");
        assert!(op.sig.is_some());
        assert_eq!(op.also_known_as, vec!["at://alice.test"]);
        assert_eq!(
            op.services.get("atproto_pds").unwrap().endpoint,
            "https://localhost:2583"
        );
    }

    #[tokio::test]
    async fn update_handle_op_preserves_fields() {
        let (_, create) = create_op(
            CreateAtprotoOpInput {
                signing_key: "did:key:zKeyA".to_string(),
                handle: "alice.test".to_string(),
                pds: "https://pds.example.com".to_string(),
                rotation_keys: vec!["did:key:zRot".to_string()],
            },
            test_key(),
        )
        .await
        .unwrap();
        let op = update_handle_op(CompatibleOp::Operation(create), &test_key(), "bob.test".to_string())
            .await
            .unwrap();
        assert!(op.also_known_as.contains(&"at://bob.test".to_string()));
        assert_eq!(op.rotation_keys.len(), 1);
        assert!(op.prev.is_some());
        assert!(op.sig.is_some());
    }
}
```

Run: `cargo test -p pds plc::operations::tests`
Expected: FAIL — module missing.

- [ ] **Step 2: Implement `plc/types.rs`**

Port of `the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/plc/types.rs` (full file, 133 lines).

```rust
// pds/src/plc/types.rs
use std::collections::BTreeMap;

#[derive(Debug, serde::Deserialize, serde::Serialize, Clone)]
pub struct Service {
    #[serde(rename = "type")]
    pub r#type: String,
    pub endpoint: String,
}

#[derive(Debug, serde::Deserialize, serde::Serialize, Clone)]
pub struct DocumentData {
    pub did: String,
    #[serde(rename = "rotationKeys")]
    pub rotation_keys: Vec<String>,
    #[serde(rename = "verificationMethods")]
    pub verification_methods: BTreeMap<String, String>,
    #[serde(rename = "alsoKnownAs")]
    pub also_known_as: Vec<String>,
    pub services: BTreeMap<String, Service>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize, Clone)]
pub struct CreateOpV1 {
    #[serde(rename = "type")]
    pub r#type: String,
    #[serde(rename = "signingKey")]
    pub signing_key: String,
    #[serde(rename = "recoveryKey")]
    pub recovery_key: String,
    pub handle: String,
    pub service: String,
    pub prev: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sig: Option<String>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize, Clone)]
pub struct Operation {
    #[serde(rename = "type")]
    pub r#type: String,
    #[serde(rename = "rotationKeys")]
    pub rotation_keys: Vec<String>,
    #[serde(rename = "verificationMethods")]
    pub verification_methods: BTreeMap<String, String>,
    #[serde(rename = "alsoKnownAs")]
    pub also_known_as: Vec<String>,
    pub services: BTreeMap<String, Service>,
    pub prev: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sig: Option<String>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize, Clone)]
pub struct Tombstone {
    #[serde(rename = "type")]
    pub r#type: String,
    pub prev: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sig: Option<String>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize, Clone)]
#[serde(untagged)]
pub enum CompatibleOpOrTombstone {
    CreateOpV1(CreateOpV1),
    Operation(Operation),
    Tombstone(Tombstone),
}

impl CompatibleOpOrTombstone {
    pub fn set_sig(&mut self, sig: String) {
        match self {
            Self::CreateOpV1(create) => create.sig = Some(sig),
            Self::Operation(op) => op.sig = Some(sig),
            Self::Tombstone(tombstone) => tombstone.sig = Some(sig),
        }
    }

    pub fn get_sig(&mut self) -> &Option<String> {
        match self {
            Self::CreateOpV1(create) => &create.sig,
            Self::Operation(op) => &op.sig,
            Self::Tombstone(tombstone) => &tombstone.sig,
        }
    }
}

#[derive(Debug, serde::Deserialize, serde::Serialize, Clone)]
#[serde(untagged)]
pub enum CompatibleOp {
    CreateOpV1(CreateOpV1),
    Operation(Operation),
}

impl CompatibleOp {
    pub fn set_sig(&mut self, sig: String) {
        match self {
            Self::CreateOpV1(create) => create.sig = Some(sig),
            Self::Operation(op) => op.sig = Some(sig),
        }
    }

    pub fn get_sig(&mut self) -> &Option<String> {
        match self {
            Self::CreateOpV1(create) => &create.sig,
            Self::Operation(op) => &op.sig,
        }
    }
}

#[derive(Debug, serde::Deserialize, serde::Serialize, Clone)]
#[serde(untagged)]
pub enum OpOrTombstone {
    Operation(Operation),
    Tombstone(Tombstone),
}

impl OpOrTombstone {
    pub fn set_sig(&mut self, sig: String) {
        match self {
            Self::Operation(op) => op.sig = Some(sig),
            Self::Tombstone(tombstone) => tombstone.sig = Some(sig),
        }
    }

    pub fn get_sig(&mut self) -> &Option<String> {
        match self {
            Self::Operation(op) => &op.sig,
            Self::Tombstone(tombstone) => &tombstone.sig,
        }
    }
}
```

- [ ] **Step 3: Implement `plc/operations.rs`**

Port of `the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/plc/operations.rs` (286 lines). It uses `rsky_common::{ipld::cid_for_cbor, sign::atproto_sign}`, `data_encoding::BASE32`, `indexmap::IndexMap`, `secp256k1::SecretKey`, `sha2`. Port the full file verbatim with `crate::plc::types` imports; only the `CONSTRAINTS`-free part matters. The full file is in the reference; copy it and adjust imports to `crate::plc::types::{...}`.

```rust
// pds/src/plc/operations.rs
use crate::plc::types::{CompatibleOp, CompatibleOpOrTombstone, Operation, Service, Tombstone};
use anyhow::Result;
use data_encoding::BASE32;
use indexmap::IndexMap;
use lexicon_cid::Cid;
use rsky_common::ipld::cid_for_cbor;
use rsky_common::sign::atproto_sign;
use secp256k1::SecretKey;
use serde_json::{Value as JsonValue, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct CreateAtprotoUpdateOpOpts {
    pub signing_key: Option<String>,
    pub handle: Option<String>,
    pub pds: Option<String>,
    pub rotation_keys: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct CreateAtprotoOpInput {
    pub signing_key: String,
    pub handle: String,
    pub pds: String,
    pub rotation_keys: Vec<String>,
}

pub async fn create_op(
    opts: CreateAtprotoOpInput,
    secret_key: SecretKey,
) -> Result<(String, Operation)> {
    let mut create_op = Operation {
        r#type: "plc_operation".to_string(),
        rotation_keys: opts.rotation_keys,
        verification_methods: BTreeMap::from([("atproto".to_string(), opts.signing_key)]),
        also_known_as: vec![ensure_atproto_prefix(opts.handle)],
        services: BTreeMap::from([(
            "atproto_pds".to_string(),
            Service {
                r#type: "AtprotoPersonalDataServer".to_string(),
                endpoint: ensure_http_prefix(opts.pds),
            },
        )]),
        prev: None,
        sig: None,
    };

    create_op = sign(create_op, &secret_key);
    let json = serde_json::to_string(&create_op)?;
    let hashmap_genesis: IndexMap<String, Value> = serde_json::from_str(&json)?;
    let signed_genesis_bytes = serde_ipld_dagcbor::to_vec(&hashmap_genesis)?;
    let mut hasher: Sha256 = Digest::new();
    hasher.update(signed_genesis_bytes.as_slice());
    let hash = hasher.finalize();
    let did_plc = format!("did:plc:{}", BASE32.encode(&hash[..]))[..32].to_lowercase();

    Ok((did_plc, create_op))
}

pub async fn update_handle_op(
    last_op: CompatibleOp,
    signer: &SecretKey,
    handle: String,
) -> Result<Operation> {
    create_atproto_update_op(
        last_op,
        signer,
        CreateAtprotoUpdateOpOpts {
            signing_key: None,
            handle: Some(handle),
            pds: None,
            rotation_keys: None,
        },
    )
    .await
}

pub async fn create_atproto_update_op(
    last_op: CompatibleOp,
    signer: &SecretKey,
    opts: CreateAtprotoUpdateOpOpts,
) -> Result<Operation> {
    create_update_op(last_op, signer, |normalized: Operation| -> Operation {
        let mut updated = normalized.clone();
        if let Some(signing_key) = &opts.signing_key {
            _ = updated
                .verification_methods
                .insert("atproto".to_string(), signing_key.clone());
        }
        if let Some(handle) = &opts.handle {
            let formatted = ensure_atproto_prefix(handle.clone());
            let handle_i = normalized
                .also_known_as
                .iter()
                .position(|h| h.starts_with("at://"));
            match handle_i {
                None => {
                    updated.also_known_as =
                        [[formatted].as_slice(), normalized.also_known_as.as_slice()].concat()
                }
                Some(handle_i) => {
                    updated.also_known_as = [
                        &normalized.also_known_as[0..handle_i],
                        [formatted].as_slice(),
                        &normalized.also_known_as[handle_i + 1..],
                    ]
                    .concat()
                }
            }
        }
        if let Some(pds) = &opts.pds {
            let formatted = ensure_http_prefix(pds.clone());
            _ = updated.services.insert(
                "atproto_pds".to_string(),
                Service {
                    r#type: "AtprotoPersonalDataServer".to_string(),
                    endpoint: formatted,
                },
            )
        }
        if let Some(rotation_keys) = &opts.rotation_keys {
            updated.rotation_keys = rotation_keys.clone();
        }
        updated
    })
    .await
}

pub async fn create_update_op<G>(
    last_op: CompatibleOp,
    signer: &SecretKey,
    func: G,
) -> Result<Operation>
where
    G: Fn(Operation) -> Operation,
{
    let last_op_json = serde_json::to_string(&last_op)?;
    let last_op_index_map: IndexMap<String, JsonValue> = serde_json::from_str(&last_op_json)?;
    let prev = cid_for_cbor(&last_op_index_map)?;
    let mut normalized = normalize_op(last_op);
    normalized.sig = None;

    let mut unsigned = func(normalized);
    unsigned.prev = Some(prev.to_string());

    match add_signature(CompatibleOpOrTombstone::Operation(unsigned), signer).await? {
        CompatibleOpOrTombstone::Operation(op) => Ok(op),
        _ => panic!("Enum type changed"),
    }
}

pub async fn add_signature(
    mut obj: CompatibleOpOrTombstone,
    key: &SecretKey,
) -> Result<CompatibleOpOrTombstone> {
    let sig = atproto_sign(&obj, key)?.to_vec();
    obj.set_sig(base64_url::encode(&sig).replace("=", ""));
    Ok(obj)
}

pub fn normalize_op(op: CompatibleOp) -> Operation {
    match op {
        CompatibleOp::Operation(op) => op,
        CompatibleOp::CreateOpV1(op) => Operation {
            r#type: "plc_operation".to_string(),
            rotation_keys: vec![op.recovery_key, op.signing_key.clone()],
            verification_methods: BTreeMap::from([("atproto".to_string(), op.signing_key)]),
            also_known_as: vec![ensure_atproto_prefix(op.handle)],
            services: BTreeMap::from([(
                "atproto_pds".to_string(),
                Service {
                    r#type: "AtprotoPersonalDataServer".to_string(),
                    endpoint: ensure_http_prefix(op.service),
                },
            )]),
            prev: op.prev,
            sig: op.sig,
        },
    }
}

pub fn ensure_http_prefix(str: String) -> String {
    if str.starts_with("http://") || str.starts_with("https://") {
        return str;
    }
    format!("https://{str}")
}

pub fn ensure_atproto_prefix(str: String) -> String {
    if str.starts_with("at://") {
        return str;
    }
    let stripped = str.replace("http://", "").replace("https://", "");
    format!("at://{stripped}")
}

fn sign(mut op: Operation, private_key: &SecretKey) -> Operation {
    let op_sig = atproto_sign(&op, private_key).unwrap();
    op.sig = Some(base64_url::encode(&op_sig).replace("=", ""));
    op
}

#[cfg(test)]
mod tests {
    use super::*;
    use secp256k1::SecretKey;

    fn test_key() -> SecretKey {
        SecretKey::from_slice(&hex::decode("9d5907143471e8f0e8df0f8b9512a8c5377878ee767f18fcf961055ecfc071cd").unwrap()).unwrap()
    }

    #[tokio::test]
    async fn creates_and_signs_a_create_operation() {
        let (did, op) = create_op(
            CreateAtprotoOpInput {
                signing_key: "did:key:zQ3shXkXxRqVnGX6fYqPqL4h6F5TpCxYhZJcMvBtNwRpKsUdEiF".to_string(),
                handle: "alice.test".to_string(),
                pds: "http://localhost:2583".to_string(),
                rotation_keys: vec!["did:key:zRotation".to_string()],
            },
            test_key(),
        )
        .await
        .unwrap();
        assert!(did.starts_with("did:plc:"));
        assert_eq!(op.r#type, "plc_operation");
        assert!(op.sig.is_some());
        assert_eq!(op.also_known_as, vec!["at://alice.test"]);
        assert_eq!(
            op.services.get("atproto_pds").unwrap().endpoint,
            "https://localhost:2583"
        );
    }

    #[tokio::test]
    async fn update_handle_op_preserves_fields() {
        let (_, create) = create_op(
            CreateAtprotoOpInput {
                signing_key: "did:key:zKeyA".to_string(),
                handle: "alice.test".to_string(),
                pds: "https://pds.example.com".to_string(),
                rotation_keys: vec!["did:key:zRot".to_string()],
            },
            test_key(),
        )
        .await
        .unwrap();
        let op = update_handle_op(
            CompatibleOp::Operation(create),
            &test_key(),
            "bob.test".to_string(),
        )
        .await
        .unwrap();
        assert!(op.also_known_as.contains(&"at://bob.test".to_string()));
        assert_eq!(op.rotation_keys.len(), 1);
        assert!(op.prev.is_some());
        assert!(op.sig.is_some());
    }
}
```

- [ ] **Step 4: Implement `plc/mod.rs` — `PlcClient` trait + reqwest impl + mock**

The trait lets identity handlers run against a mock in tests. `PlcClientImpl` is the ported reqwest client from `the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/plc/mod.rs` (with `APP_USER_AGENT` inlined).

```rust
// pds/src/plc/mod.rs
pub mod operations;
pub mod types;

use crate::plc::operations::update_handle_op;
use crate::plc::types::{CompatibleOp, CompatibleOpOrTombstone, DocumentData, OpOrTombstone};
use anyhow::{bail, Result};
use rsky_common::encode_uri_component;
use secp256k1::SecretKey;
use serde::de::DeserializeOwned;
use types::{CompatibleOpOrTombstone as _T, DocumentData as _D};

pub static APP_USER_AGENT: &str = concat!(
    "cacos-pds/",
    env!("CARGO_PKG_VERSION"),
);

#[async_trait::async_trait]
pub trait PlcClient: Send + Sync {
    async fn send_operation(&self, did: &String, op: &OpOrTombstone) -> Result<()>;
    async fn get_document_data(&self, did: &String) -> Result<DocumentData>;
    async fn get_last_op(&self, did: &String) -> Result<CompatibleOpOrTombstone>;
    async fn update_handle(&self, did: &String, signer: &SecretKey, handle: &str) -> Result<()>;
}

/// Reqwest-backed PLC client (port of rsky's plc::Client).
pub struct PlcClientImpl {
    pub url: String,
}

impl PlcClientImpl {
    pub fn new(url: String) -> Self {
        Self { url }
    }

    fn post_op_url(&self, did: &String) -> String {
        format!("{0}/{1}", self.url, encode_uri_component(did))
    }

    async fn make_get_req<T: DeserializeOwned>(
        &self,
        url: String,
        params: Option<Vec<(&str, String)>>,
    ) -> Result<T> {
        let client = reqwest::Client::builder().user_agent(APP_USER_AGENT).build()?;
        let mut builder = client
            .get(url)
            .header("Connection", "Keep-Alive")
            .header("Keep-Alive", "timeout=5, max=1000");
        if let Some(params) = params {
            builder = builder.query(&params);
        }
        let res = builder.send().await?;
        Ok(res.json().await?)
    }
}

#[async_trait::async_trait]
impl PlcClient for PlcClientImpl {
    async fn send_operation(&self, did: &String, op: &OpOrTombstone) -> Result<()> {
        let client = reqwest::Client::builder().user_agent(APP_USER_AGENT).build()?;
        let response = client
            .post(self.post_op_url(did))
            .json(op)
            .header("Connection", "Keep-Alive")
            .header("Keep-Alive", "timeout=5, max=1000")
            .send()
            .await?;
        match response.error_for_status_ref() {
            Ok(_) => Ok(()),
            Err(_) => bail!(response.text().await?),
        }
    }

    async fn get_document_data(&self, did: &String) -> Result<DocumentData> {
        match self
            .make_get_req(
                format!("{0}/{1}/data", self.url, encode_uri_component(did)),
                None,
            )
            .await
        {
            Ok(res) => Ok(res),
            Err(error) => bail!(error.to_string()),
        }
    }

    async fn get_last_op(&self, did: &String) -> Result<CompatibleOpOrTombstone> {
        match self
            .make_get_req(
                format!("{0}/{1}/log/last", self.url, encode_uri_component(did)),
                None,
            )
            .await
        {
            Ok(res) => Ok(res),
            Err(error) => bail!(error.to_string()),
        }
    }

    async fn update_handle(&self, did: &String, signer: &SecretKey, handle: &str) -> Result<()> {
        let last_op: CompatibleOp = match self.get_last_op(did).await? {
            CompatibleOpOrTombstone::CreateOpV1(last_op) => CompatibleOp::CreateOpV1(last_op),
            CompatibleOpOrTombstone::Operation(last_op) => CompatibleOp::Operation(last_op),
            CompatibleOpOrTombstone::Tombstone(_) => {
                bail!("Cannot apply op to tombstone")
            }
        };
        let op = update_handle_op(last_op, signer, handle.to_owned()).await?;
        self.send_operation(did, &OpOrTombstone::Operation(op)).await
    }
}

/// Hermetic mock used by tests. `get_document_data` returns a document whose
/// `atproto_pds` endpoint and `atproto` verification method match the test
/// public URL and PDS signing key (so activateAccount/checkAccountStatus pass).
pub struct MockPlcClient {
    pub did: String,
}

impl Default for MockPlcClient {
    fn default() -> Self {
        Self {
            did: "did:plc:mock".to_string(),
        }
    }
}

#[async_trait::async_trait]
impl PlcClient for MockPlcClient {
    async fn send_operation(&self, _did: &String, _op: &OpOrTombstone) -> Result<()> {
        Ok(())
    }

    async fn get_document_data(&self, _did: &String) -> Result<DocumentData> {
        use crate::context::PDS_REPO_SIGNING_KEYPAIR;
        use crate::xrpc::com::atproto::server::PDS_PLC_ROTATION_KEYPAIR;
        use rsky_crypto::utils::encode_did_key;
        use std::collections::BTreeMap;
        let hostname = std::env::var("PDS_HOSTNAME").unwrap_or("localhost".to_owned());
        let port = std::env::var("PDS_PORT").ok().and_then(|p| p.parse::<usize>().ok()).unwrap_or(2583);
        let public_url = if hostname == "localhost" {
            format!("http://localhost:{port}")
        } else {
            format!("https://{hostname}")
        };
        Ok(DocumentData {
            did: self.did.clone(),
            rotation_keys: vec![encode_did_key(&PDS_PLC_ROTATION_KEYPAIR.public_key())],
            verification_methods: BTreeMap::from([(
                "atproto".to_string(),
                encode_did_key(&PDS_REPO_SIGNING_KEYPAIR.public_key()),
            )]),
            also_known_as: vec![],
            services: BTreeMap::from([(
                "atproto_pds".to_string(),
                types::Service {
                    r#type: "AtprotoPersonalDataServer".to_string(),
                    endpoint: public_url,
                },
            )]),
        })
    }

    async fn get_last_op(&self, _did: &String) -> Result<CompatibleOpOrTombstone> {
        // A minimal signed create operation built with the PDS rotation key.
        use crate::plc::operations::create_op;
        use crate::plc::operations::CreateAtprotoOpInput;
        use crate::xrpc::com::atproto::server::PDS_PLC_ROTATION_KEYPAIR;
        let (_did_plc, op) = create_op(
            CreateAtprotoOpInput {
                signing_key: "did:key:zQ3shXkXxRqVnGX6fYqPqL4h6F5TpCxYhZJcMvBtNwRpKsUdEiF".to_string(),
                handle: "mock.test".to_string(),
                pds: "https://mock.pds".to_string(),
                rotation_keys: vec![],
            },
            PDS_PLC_ROTATION_KEYPAIR.secret_key(),
        )
        .await?;
        Ok(CompatibleOpOrTombstone::Operation(op))
    }

    async fn update_handle(&self, _did: &String, _signer: &SecretKey, _handle: &str) -> Result<()> {
        Ok(())
    }
}
```

> **Note:** remove the redundant `use types::{... as _T, ... as _D};` line if unused; it exists only to mirror the reference's import style. `rsky_common::encode_uri_component` is assumed from Plan 06's dependency set.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p pds plc::operations::tests`
Expected: PASS (2 tests).

- [ ] **Step 6: Commit**

```bash
git add pds/src/plc
git commit -m "feat(plc): PlcClient trait with reqwest impl and mock"
```
