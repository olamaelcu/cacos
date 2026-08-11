//! Auth module: the three process-wide service-signing keypairs
//! (canonical home in `keypairs`), plus the split `verifier` submodule
//! that holds the JWT/DPoP/admin/service-JWT logic.

pub mod keypairs;
pub mod verifier;

pub use keypairs::{PDS_JWT_KEYPAIR, PDS_PLC_ROTATION_KEYPAIR, PDS_REPO_SIGNING_KEYPAIR};
