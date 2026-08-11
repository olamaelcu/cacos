//! Identity caching: SQLite-backed DID document cache.
//!
//! See [`did_cache::DidSqliteCache`].

/// Local copy of `cacos_pds::background::BackgroundQueue`.
///
/// **Temporary:** extracted alongside the DID cache so this crate compiles
/// standalone. `cacos-pds` still owns the original; the two should collapse
/// into one shared crate (or a trait this crate depends on) when the queue
/// itself is extracted. See the crate-split notes in `background`.
pub mod background;
pub mod did_cache;
