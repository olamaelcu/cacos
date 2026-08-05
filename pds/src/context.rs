//! Process-wide keypairs shared across account helpers and server handlers.
//! Plan 08 imports `crate::context::PDS_REPO_SIGNING_KEYPAIR`.

use secp256k1::{Keypair, Secp256k1, SecretKey};
use std::env;
use std::sync::LazyLock;

/// Signs service JWTs (`helpers::auth::create_service_jwt`) and is exposed as
/// the PDS's repo signing key by Plan 08's server handlers.
pub static PDS_REPO_SIGNING_KEYPAIR: LazyLock<Keypair> = LazyLock::new(|| {
    let secp = Secp256k1::new();
    let private_key = env::var("PDS_REPO_SIGNING_KEY_K256_PRIVATE_KEY_HEX").unwrap();
    let secret_key = SecretKey::from_slice(&hex::decode(private_key.as_bytes()).unwrap()).unwrap();
    Keypair::from_secret_key(&secp, &secret_key)
});
