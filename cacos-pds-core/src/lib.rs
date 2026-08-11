//! cacos PDS core: foundation crate containing the cross-cutting error type,
//! process-wide configuration, observability registry, SQLite database openers,
//! and the in-process task drain.
//!
//! Layer-2 in the planned layered dependency graph:
//!
//! ```text
//!       foundation: cacos-migration
//!               |
//!             core       (this crate)
//!               |
//!     +---------+---------+-----+
//!     |                   |     |
//!   account          actor-store  sequencer  ...
//! ```
//!
//! Higher-layer crates (`cacos-pds-account`, `cacos-pds-actor-store`,
//! `cacos-pds-sequencer`, ...) import from this crate; this crate does not
//! import from them. The reverse-dep guardrail is enforced by the workspace
//! dep graph.

pub mod background;
pub mod config;
pub mod db;
pub mod error;
pub mod observability;

pub use error::{PdsError, Result as PdsResult};
pub type Result<T> = error::Result<T>;
