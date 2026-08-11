//! cacos PDS library crate.
//!
//! Post-crate-split: this crate is a thin shell that hosts the
//! `pds/tests/*.rs` integration suite. Every meaningful unit lives in a
//! dedicated workspace member:
//!
//! | Workspace member        | What it owns                                      |
//! |-------------------------|---------------------------------------------------|
//! | cacos-migration         | SQLite migrators + entities                       |
//! | cacos-pds-core          | error / config / observability / db / background  |
//! | cacos-pds-account       | AccountManager + auth verifier (5 submodules)    |
//! | cacos-pds-actor-store   | per-DID storage, blob, record, repo               |
//! | cacos-pds-sequencer     | firehose sequencer + apalis worker               |
//! | cacos-pds-blobstore     | blob trait + OpenDAL backend                      |
//! | cacos-pds-plc           | PLC operations + PlcClient trait                  |
//! | cacos-pds-identity      | DID-document cache                                |
//! | cacos-pds-handle        | handle normalization + validation                |
//! | cacos-pds-mailer        | templated mailers                                  |
//! | cacos-pds-oauth         | OAuth provider + remote API + rate-limit         |
//! | cacos-pds-server        | XRPC HTTP surface + auth_extractors (lib + bin)   |
//! | cacos-pds-migrate       | operator migration binary                         |
//!
//! Integration tests under `pds/tests/` consume those crates directly.
