//! Shared sequencer state used across helpers and HTTP handlers.
//!
//! The service-signing keypairs (`PDS_REPO_SIGNING_KEYPAIR`,
//! `PDS_PLC_ROTATION_KEYPAIR`, `PDS_JWT_KEYPAIR`) live in
//! `cacos_pds_account::auth::*`.
//!
//! `SharedSequencer` was extracted into
//! `cacos_pds_sequencer::shared_sequencer::SharedSequencer`; callers in
//! `xrpc/` now reach for it via
//! `cacos_pds_sequencer::shared_sequencer::SharedSequencer` (and
//! `cacos_pds_sequencer::Sequencer`). This file is kept as a thin doc
//! placeholder so `pds::context` keeps resolving for any remaining
//! callers; remove once the crate split is fully drained.
