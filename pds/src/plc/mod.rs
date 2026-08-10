pub mod operations;
pub mod types;

use crate::plc::operations::update_handle_op;
use crate::plc::types::{CompatibleOp, CompatibleOpOrTombstone, DocumentData, OpOrTombstone};
use anyhow::{Result, bail};
use rsky_common::encode_uri_component;
use secp256k1::SecretKey;
use serde::de::DeserializeOwned;
use url::Url;

pub static APP_USER_AGENT: &str = concat!("cacos-pds/", env!("CARGO_PKG_VERSION"),);

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
        let client = reqwest::Client::builder()
            .user_agent(APP_USER_AGENT)
            .build()?;
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
        let client = reqwest::Client::builder()
            .user_agent(APP_USER_AGENT)
            .build()?;
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
        self.send_operation(did, &OpOrTombstone::Operation(op))
            .await
    }
}

/// SSRF-hardened HTTPS PLC client. Resolves the configured hostname and
/// rejects private/loopback/link-local addresses before any request is
/// issued. Use [`plc_client_from_env`] to construct one from a
/// [`ServerConfig`].
pub struct HttpPlcClient {
    pub url: String,
    http: reqwest::Client,
}

impl HttpPlcClient {
    pub fn new(url: String) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(APP_USER_AGENT)
            .timeout(std::time::Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self { url, http })
    }

    /// Validates that the configured endpoint URL does not target a
    /// private/loopback/link-local address. Run this at construction
    /// time (or before the first request) so SSRF attempts fail fast.
    pub fn check_endpoint_ssrf(&self) -> Result<()> {
        let parsed = Url::parse(&self.url)
            .map_err(|e| anyhow::anyhow!("invalid PLC endpoint URL {}: {e}", self.url))?;
        if parsed.scheme() != "https" && parsed.scheme() != "http" {
            anyhow::bail!("PLC URL must use http or https");
        }
        let host = parsed
            .host_str()
            .ok_or_else(|| anyhow::anyhow!("PLC URL missing host"))?
            .to_string();
        // Strip IPv6 brackets so the parser can read the literal.
        let host_for_lookup = host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_string();
        let url_for_err = self.url.clone();
        let ip: std::net::IpAddr = match host_for_lookup.parse() {
            Ok(ip) => ip,
            Err(_) => {
                // Hostname — resolve via system resolver. If the resolver
                // returns any private/loopback/link-local address, reject.
                let addrs: Vec<std::net::IpAddr> = (host_for_lookup.clone(), 0u16)
                    .to_socket_addrs()
                    .map_err(|e| {
                        anyhow::anyhow!("DNS resolution failed for {host_for_lookup}: {e}")
                    })?
                    .map(|sa| sa.ip())
                    .collect();
                if addrs.is_empty() {
                    anyhow::bail!("no addresses resolved for {host_for_lookup}");
                }
                for ip in &addrs {
                    if is_denied_ip(*ip) {
                        anyhow::bail!("PLC URL {url_for_err} resolves to denied IP {ip}");
                    }
                }
                addrs[0]
            }
        };
        if is_denied_ip(ip) {
            anyhow::bail!("PLC URL {url_for_err} points to denied IP {ip}");
        }
        Ok(())
    }

    async fn make_get_req<T: DeserializeOwned>(&self, url: String) -> Result<T> {
        let response = self.http.get(&url).send().await?;
        let status = response.status();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !content_type.starts_with("application/json") {
            anyhow::bail!("unexpected content-type from PLC: {content_type}");
        }
        let body = response.bytes().await?;
        if body.len() > 1024 * 1024 {
            anyhow::bail!("PLC response too large: {} bytes", body.len());
        }
        if !status.is_success() {
            anyhow::bail!("PLC HTTP {status}: {}", String::from_utf8_lossy(&body));
        }
        Ok(serde_json::from_slice(&body)?)
    }
}

fn is_denied_ip(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()           // 127.0.0.0/8
                || v4.is_private()    // 10/8, 172.16/12, 192.168/16
                || v4.is_link_local() // 169.254/16
                || v4.is_unspecified() // 0.0.0.0
                || v4.is_broadcast() // 255.255.255.255
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()           // ::1
                || v6.is_unspecified() // ::
                || (v6.segments()[0] & 0xfe00) == 0xfc00 // fc00::/7
                || (v6.segments()[0] & 0xffc0) == 0xfe80 // fe80::/10
        }
    }
}

#[async_trait::async_trait]
impl PlcClient for HttpPlcClient {
    async fn send_operation(&self, did: &str, op: &OpOrTombstone) -> Result<()> {
        self.check_endpoint_ssrf()?;
        let url = format!("{0}/{1}", self.url, encode_uri_component(&did.to_string()));
        let response = self.http.post(&url).json(op).send().await?;
        if !response.status().is_success() {
            anyhow::bail!(response.text().await.unwrap_or_default());
        }
        Ok(())
    }

    async fn get_document_data(&self, did: &str) -> Result<DocumentData> {
        self.check_endpoint_ssrf()?;
        let url = format!(
            "{0}/{1}/data",
            self.url,
            encode_uri_component(&did.to_string())
        );
        self.make_get_req(url).await
    }

    async fn get_last_op(&self, did: &str) -> Result<CompatibleOpOrTombstone> {
        self.check_endpoint_ssrf()?;
        let url = format!(
            "{0}/{1}/log/last",
            self.url,
            encode_uri_component(&did.to_string())
        );
        self.make_get_req(url).await
    }

    async fn update_handle(&self, did: &str, signer: &SecretKey, handle: &str) -> Result<()> {
        self.check_endpoint_ssrf()?;
        let last_op: CompatibleOp = match self.get_last_op(did).await? {
            CompatibleOpOrTombstone::CreateOpV1(last_op) => CompatibleOp::CreateOpV1(last_op),
            CompatibleOpOrTombstone::Operation(last_op) => CompatibleOp::Operation(last_op),
            CompatibleOpOrTombstone::Tombstone(_) => {
                bail!("Cannot apply op to tombstone")
            }
        };
        let op = update_handle_op(last_op, signer, handle.to_owned()).await?;
        self.send_operation(did, &OpOrTombstone::Operation(op))
            .await
    }
}

use crate::config::ServerConfig;
use std::net::ToSocketAddrs;

/// Selects the production PLC client (HTTP with SSRF guard) by default,
/// or the mock for tests when `PDS_PLC_CLIENT_MODE=mock` is set.
pub fn plc_client_from_env(cfg: &ServerConfig) -> std::sync::Arc<dyn PlcClient> {
    let mode = std::env::var("PDS_PLC_CLIENT_MODE").unwrap_or_else(|_| "http".to_string());
    if mode == "mock" {
        #[cfg(any(test, feature = "test-utils"))]
        {
            return std::sync::Arc::new(MockPlcClient::default());
        }
        #[cfg(not(any(test, feature = "test-utils")))]
        {
            panic!("PDS_PLC_CLIENT_MODE=mock is only available with --features test-utils");
        }
    }
    let url = cfg.identity.plc_url.clone();
    let url_for_err = url.clone();
    HttpPlcClient::new(url)
        .and_then(|c| {
            c.check_endpoint_ssrf()?;
            Ok(c)
        })
        .map(|c| std::sync::Arc::new(c) as std::sync::Arc<dyn PlcClient>)
        .unwrap_or_else(|err| {
            panic!(
                "failed to construct HttpPlcClient for {url_for_err}: {err}; \
                 verify PDS_PLC_URL points to a reachable public PLC directory"
            );
        })
}

/// Hermetic mock used by tests. `get_document_data` returns a document whose
/// `atproto_pds` endpoint matches the test public URL and whose signing key
/// and rotation key are hardcoded placeholders. `get_last_op` returns a
/// canned signed create operation built from a hardcoded secret key so the
/// mock works without `PDS_PLC_ROTATION_KEYPAIR` (added in Task 5).
#[cfg(any(test, feature = "test-utils"))]
pub struct MockPlcClient {
    pub did: String,
}

#[cfg(any(test, feature = "test-utils"))]
impl Default for MockPlcClient {
    fn default() -> Self {
        Self {
            did: "did:plc:mock".to_string(),
        }
    }
}

#[cfg(any(test, feature = "test-utils"))]
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
        use crate::plc::operations::CreateAtprotoOpInput;
        use crate::plc::operations::create_op;
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
