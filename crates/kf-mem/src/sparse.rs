//! The state a guest leaf word declares: VALID, SPARSE ("there is deliberately nothing here, and
//! an access is absorbed, not faulted") or INVALID. See `docs/design/V3_SPARSE_PTE.md`.

use kf_cuda::abi::KfFormat;

/// What one raw leaf word declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeafState {
    /// A translation.
    Valid,
    /// The sparse encoding.
    Sparse,
    /// Anything else with VALID clear.
    Invalid,
}

/// Classify a raw leaf word under the descriptor `fmt`.
#[must_use]
pub fn classify_leaf(fmt: &KfFormat, raw: u64) -> LeafState {
    if (raw >> fmt.valid_bit) & 1 == 1 {
        LeafState::Valid
    } else {
        LeafState::Invalid
    }
}
