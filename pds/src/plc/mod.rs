//! PLC (did:plc) directory client.
//!
//! Task 1 ships a minimal trait surface plus a no-op `MockPlcClient` so the
//! rest of the XRPC layer can register an `Arc<dyn PlcClient>` in
//! [`crate::xrpc::SharedState`] without taking a hard dependency on the
//! full PLC HTTP client. Task 18 replaces the trait body with a real
//! reqwest-backed `PlcClientImpl` and a richer mock with canned
//! responses.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Minimal projection of a DID document returned by the PLC directory.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PlcDocumentData {
    /// The DID whose document is being described.
    pub did: String,
}

/// Last-op pointer returned by the PLC directory's log endpoint.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PlcLastOp {
    /// The serialized last operation (CID + payload, opaque to the PDS).
    pub operation: serde_json::Value,
}

/// Result of a PLC operation submission.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PlcOperationResponse {
    /// Whether the directory accepted the operation.
    pub success: bool,
}

/// Trait surface for the PLC directory. All methods are async; tests
/// inject [`MockPlcClient`], production swaps in `PlcClientImpl`.
#[async_trait]
pub trait PlcClient: Send + Sync {
    /// Send a signed `plcOperation` to the directory. Returns the
    /// directory's acknowledgement.
    async fn send_operation(
        &self,
        did: &str,
        operation: serde_json::Value,
    ) -> anyhow::Result<PlcOperationResponse>;

    /// Fetch the current DID document for `did`.
    async fn get_document_data(&self, did: &str) -> anyhow::Result<PlcDocumentData>;

    /// Fetch the last operation applied to `did` (used during handle
    /// resolution and identity event sequencing).
    async fn get_last_op(&self, did: &str) -> anyhow::Result<PlcLastOp>;

    /// Submit a handle update operation for `did`.
    async fn update_handle(
        &self,
        did: &str,
        handle: &str,
    ) -> anyhow::Result<PlcOperationResponse>;
}

/// No-op PLC client. Every method returns `Ok(())` with a default-shaped
/// response. Used in tests where the handler under test never exercises
/// PLC.
#[derive(Debug, Default, Clone)]
pub struct MockPlcClient;

#[async_trait]
impl PlcClient for MockPlcClient {
    async fn send_operation(
        &self,
        _did: &str,
        _operation: serde_json::Value,
    ) -> anyhow::Result<PlcOperationResponse> {
        Ok(PlcOperationResponse { success: true })
    }

    async fn get_document_data(&self, did: &str) -> anyhow::Result<PlcDocumentData> {
        Ok(PlcDocumentData { did: did.to_string() })
    }

    async fn get_last_op(&self, did: &str) -> anyhow::Result<PlcLastOp> {
        Ok(PlcLastOp {
            operation: serde_json::json!({ "did": did }),
        })
    }

    async fn update_handle(
        &self,
        _did: &str,
        _handle: &str,
    ) -> anyhow::Result<PlcOperationResponse> {
        Ok(PlcOperationResponse { success: true })
    }
}

/// Convenience constructor for tests.
pub fn mock() -> Arc<dyn PlcClient> {
    Arc::new(MockPlcClient)
}
