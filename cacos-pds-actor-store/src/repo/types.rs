//! Repo storage types re-exported for downstream plans.

use lexicon_cid::Cid;
use rsky_repo::block_map::BlockMap;

#[derive(Debug, Clone)]
pub struct SyncEvtData {
    pub cid: Cid,
    pub rev: String,
    pub blocks: BlockMap,
}

impl SyncEvtData {
    /// Split into the `(rev, blocks)` pair consumed by
    /// `Sequencer::sequence_sync_evt`. `cid` is dropped — it's the
    /// root CID of the commit block set; not needed for sequencing.
    pub fn into_parts(self) -> (String, BlockMap) {
        (self.rev, self.blocks)
    }
}
