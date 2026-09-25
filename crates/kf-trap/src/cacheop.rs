//! ★ w827 — **the guest's L2 cache-maintenance registers, per family.**
//!
//! `[measured vh, f8cfe8d7, every CUDA rung in the fat guest]` at each CUDA process's teardown the
//! guest RM spun four seconds on `NV_UFLUSH_L2_FLUSH_DIRTY` and gave up:
//!
//! ```text
//! NVRM: kmemsysDoCacheOp_GM107: - timeout error waiting for reg 0x70010 update cnt=7122881
//! NVRM: kmemsysDoCacheOp_GM107: - timeout error waiting for reg 0x70004 update cnt=1
//! NVRM: … Call timed out [NV_ERR_TIMEOUT] … kmemsysCacheOp_HAL(… FB_CACHE_SYSTEM_MEMORY, FB_CACHE_INVALIDATE) @ mem_desc.c:1512
//! ```
//!
//! BAR0 reads are served from the shadow, and a write stores its own value there — so the
//! `PENDING` bit the guest wrote was read back forever. On hardware the bit is the op in flight
//! (`kmemsysDoCacheOp_GM107`, `kern_mem_sys_gm107.c:90-146`: write `PENDING_BUSY`, poll
//! `PENDING | OUTSTANDING` to clear).
//!
//! The op itself is the HOST GPU's (the guest's L2 IS the host L2): it is performed as the authored,
//! unprivileged host verb `NV2080_CTRL_CMD_FB_FLUSH_GPU_CACHE` (flags `0x10118`, `NON_PRIVILEGED`),
//! whose `(aperture, writeback, invalidate)` host RM maps back onto exactly these registers
//! (`kmemsysCacheOp_GM200`, `kern_mem_sys_gm200.c:75-110`). Completion is that host call returning.
//!
//! ## Families
//! - **Turing, Ampere, Ada** — the `GM107` pending-bit protocol: `NV_UFLUSH_L2_FLUSH_DIRTY`
//!   (`0x70010`, `maxwell/gm200/dev_flush.h:47`) and the VF-priv invalidates
//!   (`NV_VIRTUAL_FUNCTION_FULL_PHYS_OFFSET 0xB80000` + `PRIV_L2_SYSMEM_INVALIDATE 0xF00` /
//!   `PRIV_L2_PEERMEM_INVALIDATE 0xF04`, `turing/tu102/dev_vm.h:28-30`, written through
//!   `GPU_VREG_WR32` by `kmemsysWriteL2SysmemInvalidateReg_TU102`).
//! - ⊘ **Hopper, Blackwell** — a different protocol (`kmemsysDoCacheOp_GH100`): the op is started
//!   by READING `NV_XAL_EP_UFLUSH_L2_FLUSH_DIRTY` (`0x10f810`, a token) and completes when
//!   `…_COMPLETED` (`0x10f814`) reaches that token. A read that starts work is a read side effect
//!   the shadow cannot express; this table is EMPTY for those families, stated rather than
//!   guessed (named in the w827 report as an open item).
use kf_chip::Family;

/// Which host cache op a register asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheOp {
    /// `L2_FLUSH_DIRTY` — write back dirty lines (host: `SYSTEM_MEMORY` + `WRITE_BACK`).
    FlushDirty,
    /// `L2_SYSMEM_INVALIDATE` — invalidate sysmem lines (host: `SYSTEM_MEMORY` + `INVALIDATE`,
    /// which host RM promotes to an evict — stronger, never weaker).
    SysmemInvalidate,
    /// `L2_PEERMEM_INVALIDATE` (host: `PEER_MEMORY` + `INVALIDATE`).
    PeermemInvalidate,
}

impl CacheOp {
    /// Index into a per-op array.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            CacheOp::FlushDirty => 0,
            CacheOp::SysmemInvalidate => 1,
            CacheOp::PeermemInvalidate => 2,
        }
    }
    /// All ops, by index.
    pub const ALL: [CacheOp; 3] = [CacheOp::FlushDirty, CacheOp::SysmemInvalidate, CacheOp::PeermemInvalidate];
}

/// `_PENDING` (bit 0) — the guest's request bit, and with `_OUTSTANDING` (bit 1) what it polls.
pub const PENDING: u32 = 1;

/// The BAR0 offset of each cache-op register of `family` (empty where the protocol differs).
#[must_use]
pub fn registers(family: Family) -> &'static [(u32, CacheOp)] {
    const GM107_STYLE: &[(u32, CacheOp)] = &[
        (0x0007_0010, CacheOp::FlushDirty),
        (0x00B8_0F00, CacheOp::SysmemInvalidate),
        (0x00B8_0F04, CacheOp::PeermemInvalidate),
    ];
    match family {
        Family::Turing | Family::Ampere | Family::Ada => GM107_STYLE,
        Family::Hopper | Family::Blackwell => &[],
    }
}

/// The cache op a 32-bit write of `val` at `off` requests, if any.
#[must_use]
pub fn decode(family: Family, off: u64, val: u32) -> Option<CacheOp> {
    if val & PENDING == 0 {
        return None;
    }
    registers(family).iter().find(|(o, _)| u64::from(*o) == off).map(|(_, op)| *op)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ga10x_flush_dirty_and_invalidates_decode_only_with_pending() {
        assert_eq!(decode(Family::Ampere, 0x70010, 1), Some(CacheOp::FlushDirty));
        assert_eq!(decode(Family::Ampere, 0x70010, 0), None);
        assert_eq!(decode(Family::Turing, 0xB80F00, 1), Some(CacheOp::SysmemInvalidate));
        assert_eq!(decode(Family::Ada, 0xB80F04, 1), Some(CacheOp::PeermemInvalidate));
        assert_eq!(decode(Family::Ampere, 0x70004, 1), None, "0x70004 is the pre-Turing sysmem invalidate");
    }

    #[test]
    fn hopper_and_blackwell_are_stated_empty() {
        assert!(registers(Family::Hopper).is_empty());
        assert!(registers(Family::Blackwell).is_empty());
    }
}
