//! Shared sequencer state used across helpers and HTTP handlers.
//!
//! The service-signing keypairs (`PDS_REPO_SIGNING_KEYPAIR`,
//! `PDS_PLC_ROTATION_KEYPAIR`, `PDS_JWT_KEYPAIR`) live in
//! `cacos_pds_account::auth::*` since Plan 00 Step 3.
//!
//! `SharedSequencer` was extracted into `cacos_pds_sequencer::SharedSequencer`
//! in Step 4 (commit "Step 4: extract cacos-pds-sequencer from cacos-pds").
//! Callers in `xrpc/` now reach for it via `cacos_pds_sequencer::SharedSequencer`
//! (and `cacos_pds_sequencer::Sequencer`); this file is kept as a thin doc
//! placeholder so `pds::context` keeps resolving until Step 9 retires it.
