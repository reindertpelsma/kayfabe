//! The state a guest leaf word declares: VALID, SPARSE ("there is deliberately nothing here, and
//! an access is absorbed, not faulted") or INVALID. See `docs/design/V3_SPARSE_PTE.md`.

use crate::refusal_dump::field;
use kf_cuda::abi::{KF_TBL_VER2, KfFormat};

fn bit(raw: u64, b: u8) -> u64 {
    if b >= 64 { 0 } else { (raw >> b) & 1 }
}

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
///
/// - VER2: SPARSE is VALID clear, VOLATILE set and PRIVILEGE clear (RM's `sparsePte` sets only the
///   first two, `kern_gmmu_gm200.c`). VOLATILE and PRIVILEGE together with VALID clear is NV4K
///   (`kern_gmmu_gv100.c`), and PRIVILEGE alone is the unmapped-big marker: both are INVALID here.
/// - VER3: SPARSE is VALID clear and PCF equal to `PCF_SPARSE` (`fmt.pcf_sparse`).
///
/// A word with VALID set is a translation whatever else it carries.
#[must_use]
pub fn classify_leaf(fmt: &KfFormat, raw: u64) -> LeafState {
    if bit(raw, fmt.valid_bit) == 1 {
        return LeafState::Valid;
    }
    let sparse = if fmt.table_version == KF_TBL_VER2 {
        bit(raw, fmt.bit_volatile) == 1 && bit(raw, fmt.bit_privilege) == 0
    } else {
        field(raw, fmt.pcf) == u64::from(fmt.pcf_sparse)
    };
    if sparse {
        LeafState::Sparse
    } else {
        LeafState::Invalid
    }
}

impl LeafState {
    /// The name the VAHIST probe prints.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Valid => "valid",
            Self::Sparse => "sparse",
            Self::Invalid => "invalid",
        }
    }
}
