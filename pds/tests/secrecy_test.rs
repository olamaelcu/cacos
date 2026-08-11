//! Smoke tests for the `secrecy::SecretBox` hygiene of process-wide key
//! material.
//!
//! Each test mutates the process-wide environment directly via
//! `unsafe { std::env::set_var(...) }`. Cargo runs the tests in this binary
//! sequentially within a single process, so the `unsafe` is safe in
//! practice; it mirrors the pattern used elsewhere in this crate (see
//! `pds::xrpc::test_utils::init_env`).
//!
//! These tests intentionally only touch the public statics to confirm the
//! `secrecy::SecretBox<Vec<u8>>` wrappers inside the `LazyLock` initializers
//! did not break anything. They do not (and can not) directly inspect the
//! wrapped buffer — `SecretBox` deliberately prevents that without an
//! explicit `expose_secret()`.

use std::sync::Once;

use cacos_pds_account::auth::PDS_JWT_KEYPAIR;
use cacos_pds_account::auth::PDS_REPO_SIGNING_KEYPAIR;
use cacos_pds_account::auth::PDS_PLC_ROTATION_KEYPAIR;

static INIT_ENV: Once = Once::new();

fn init_env() {
    INIT_ENV.call_once(|| {
        let defaults = [
            (
                "PDS_JWT_KEY_K256_PRIVATE_KEY_HEX",
                "9d5907143471e8f0e8df0f8b9512a8c5377878ee767f18fcf961055ecfc071cd",
            ),
            (
                "PDS_REPO_SIGNING_KEY_K256_PRIVATE_KEY_HEX",
                "71cfcf4882a6cff494c3d0affadd3858eb3a5838e7b5e15170e696a590a4fa01",
            ),
            (
                "PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX",
                "e7763b0a2d1a4f8e9f9d2d6f3f9a5d0c6b2e4a8c1f7d3e9b5a0c2f4d6e8a0b1c",
            ),
        ];
        for (key, value) in defaults {
            if std::env::var(key).is_err() {
                // SAFETY: tests run sequentially within a process, no
                // concurrent env reads.
                unsafe {
                    std::env::set_var(key, value);
                }
            }
        }
    });
}

#[test]
fn lazy_keypair_initializes() {
    init_env();
    // Touch the lazy initializer to confirm the SecretBox<Vec<u8>> wrap and
    // zeroize in the closure did not break the resulting `ES256kKeyPair`.
    let _pk = PDS_JWT_KEYPAIR.public_key();
}

#[test]
fn lazy_signing_keypair_initializes() {
    init_env();
    // Touch the lazy initializer to confirm the SecretBox<Vec<u8>> wrap and
    // zeroize in the closure did not break the resulting `secp256k1::Keypair`.
    let _pk = PDS_REPO_SIGNING_KEYPAIR.public_key();
}

#[test]
fn lazy_plc_rotation_keypair_initializes() {
    init_env();
    // Touch the lazy initializer to confirm the SecretBox<Vec<u8>> wrap and
    // zeroize in the closure did not break the resulting `secp256k1::Keypair`.
    let _pk = PDS_PLC_ROTATION_KEYPAIR.public_key();
}
