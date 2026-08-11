//! Process-wide service-signing keypairs for the PDS account crate.
//!
//! All three keypairs are populated from environment variables at first
//! access and never change for the lifetime of the process:
//!
//! - `PDS_JWT_KEYPAIR` -- ES256K signing key for access/refresh/service JWTs,
//!   loaded from `PDS_JWT_KEY_K256_PRIVATE_KEY_HEX`.
//! - `PDS_REPO_SIGNING_KEYPAIR` -- secp256k1 keypair used to sign repo
//!   commits and to construct the service DID; loaded from
//!   `PDS_REPO_SIGNING_KEY_K256_PRIVATE_KEY_HEX`.
//! - `PDS_PLC_ROTATION_KEYPAIR` -- secp256k1 keypair used as the legacy
//!   shared server-side PLC rotation key while actors are still being
//!   migrated to per-DID rotation keys; loaded from
//!   `PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX`. Will become vestigial
//!   once the per-DID migration (`cacos-pds-migrate plc-rotation-keys`)
//!   finishes for every actor.

use jwt_simple::algorithms::ES256kKeyPair;
use secp256k1::{Keypair, Secp256k1, SecretKey};
use secrecy::{ExposeSecret, ExposeSecretMut, SecretBox};
use std::env;
use std::sync::LazyLock;
use zeroize::Zeroize;

/// Canonical ES256K signing key for access/refresh/service JWTs, from
/// `PDS_JWT_KEY_K256_PRIVATE_KEY_HEX`.
pub static PDS_JWT_KEYPAIR: LazyLock<ES256kKeyPair> = LazyLock::new(|| {
    let secp = secp256k1::Secp256k1::new();
    let private_key = env::var("PDS_JWT_KEY_K256_PRIVATE_KEY_HEX").unwrap();
    let mut secret_bytes = SecretBox::new(Box::new(hex::decode(private_key.as_bytes()).unwrap()));
    let secret_key = secp256k1::SecretKey::from_slice(secret_bytes.expose_secret()).unwrap();
    let jwt_key = secp256k1::Keypair::from_secret_key(&secp, &secret_key);
    secret_bytes.expose_secret_mut().zeroize();
    ES256kKeyPair::from_bytes(jwt_key.secret_bytes().as_slice()).unwrap()
});

/// Signs service JWTs and is exposed as the PDS's repo signing key by the
/// server handlers.
pub static PDS_REPO_SIGNING_KEYPAIR: LazyLock<Keypair> = LazyLock::new(|| {
    let secp = Secp256k1::new();
    let private_key = env::var("PDS_REPO_SIGNING_KEY_K256_PRIVATE_KEY_HEX").unwrap();
    let mut secret_bytes = SecretBox::new(Box::new(hex::decode(private_key.as_bytes()).unwrap()));
    let secret_key = SecretKey::from_slice(secret_bytes.expose_secret()).unwrap();
    let keypair = Keypair::from_secret_key(&secp, &secret_key);
    secret_bytes.expose_secret_mut().zeroize();
    keypair
});

/// Shared server-side PLC rotation key used by legacy actors during the
/// per-DID migration. Once every actor lists only its per-DID rotation
/// key, this static can be removed (along with the operator-side
/// `PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX` env var).
pub static PDS_PLC_ROTATION_KEYPAIR: LazyLock<Keypair> = LazyLock::new(|| {
    let secp = Secp256k1::new();
    let private_key = env::var("PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX").unwrap();
    let mut secret_bytes = SecretBox::new(Box::new(hex::decode(private_key.as_bytes()).unwrap()));
    let secret_key = SecretKey::from_slice(secret_bytes.expose_secret()).unwrap();
    let keypair = Keypair::from_secret_key(&secp, &secret_key);
    secret_bytes.expose_secret_mut().zeroize();
    keypair
});
