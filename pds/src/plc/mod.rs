pub mod operations;
pub mod types;

use crate::plc::operations::update_handle_op;
use crate::plc::types::{CompatibleOp, CompatibleOpOrTombstone, DocumentData, OpOrTombstone};
use anyhow::{bail, Result};
use rsky_common::encode_uri_component;
use secp256k1::SecretKey;
use serde::de::DeserializeOwned;

pub static APP_USER_AGENT: &str = concat!(
    "cacos-pds/",
    env!("CARGO_PKG_VERSION"),
);

#[async_trait::async_trait]
pub trait PlcClient: Send + Sync {
    async fn send_operation(&self, did: &str, op: &OpOrTombstone) -> Result<()>;
    async fn get_document_data(&self, did: &str) -> Result<DocumentData>;
    async fn get_last_op(&self, did: &str) -> Result<CompatibleOpOrTombstone>;
    async fn update_handle(&self, did: &str, signer: &SecretKey, handle: &str) -> Result<()>;
}

/// Reqwest-backed PLC client (port of rsky's plc::Client).
pub struct PlcClientImpl {
    pub url: String,
}

impl PlcClientImpl {
    pub fn new(url: String) -> Self {
        Self { url }
    }

    fn post_op_url(&self, did: &str) -> String {
        format!("{0}/{1}", self.url, encode_uri_component(&did.to_string()))
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
    async fn send_operation(&self, did: &str, op: &OpOrTombstone) -> Result<()> {
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

    async fn get_document_data(&self, did: &str) -> Result<DocumentData> {
        match self
            .make_get_req(
                format!(
                    "{0}/{1}/data",
                    self.url,
                    encode_uri_component(&did.to_string())
                ),
                None,
            )
            .await
        {
            Ok(res) => Ok(res),
            Err(error) => bail!(error.to_string()),
        }
    }

    async fn get_last_op(&self, did: &str) -> Result<CompatibleOpOrTombstone> {
        match self
            .make_get_req(
                format!(
                    "{0}/{1}/log/last",
                    self.url,
                    encode_uri_component(&did.to_string())
                ),
                None,
            )
            .await
        {
            Ok(res) => Ok(res),
            Err(error) => bail!(error.to_string()),
        }
    }

    async fn update_handle(&self, did: &str, signer: &SecretKey, handle: &str) -> Result<()> {
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
/// `atproto_pds` endpoint matches the test public URL and whose signing key
/// and rotation key are hardcoded placeholders. `get_last_op` returns a
/// canned signed create operation built from a hardcoded secret key so the
/// mock works without `PDS_PLC_ROTATION_KEYPAIR` (added in Task 5).
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
    async fn send_operation(&self, _did: &str, _op: &OpOrTombstone) -> Result<()> {
        Ok(())
    }

    async fn get_document_data(&self, _did: &str) -> Result<DocumentData> {
        use std::collections::BTreeMap;
        let hostname = std::env::var("PDS_HOSTNAME").unwrap_or("localhost".to_owned());
        let port = std::env::var("PDS_PORT")
            .ok()
            .and_then(|p| p.parse::<usize>().ok())
            .unwrap_or(2583);
        let public_url = if hostname == "localhost" {
            format!("http://localhost:{port}")
        } else {
            format!("https://{hostname}")
        };
        Ok(DocumentData {
            did: self.did.clone(),
            rotation_keys: vec!["did:key:zRotationKeyPlaceholder".to_string()],
            verification_methods: BTreeMap::from([(
                "atproto".to_string(),
                "did:key:zSigningKeyPlaceholder".to_string(),
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

    async fn get_last_op(&self, _did: &str) -> Result<CompatibleOpOrTombstone> {
        use crate::plc::operations::create_op;
        use crate::plc::operations::CreateAtprotoOpInput;
        let secret_key = secp256k1::SecretKey::from_slice(
            &hex::decode("1111111111111111111111111111111111111111111111111111111111111111")
                .unwrap(),
        )
        .unwrap();
        let (_did_plc, op) = create_op(
            CreateAtprotoOpInput {
                signing_key: "did:key:zQ3shXkXxRqVnGX6fYqPqL4h6F5TpCxYhZJcMvBtNwRpKsUdEiF"
                    .to_string(),
                handle: "mock.test".to_string(),
                pds: "https://mock.pds".to_string(),
                rotation_keys: vec![],
            },
            secret_key,
        )
        .await?;
        Ok(CompatibleOpOrTombstone::Operation(op))
    }

    async fn update_handle(&self, _did: &str, _signer: &SecretKey, _handle: &str) -> Result<()> {
        Ok(())
    }
}
