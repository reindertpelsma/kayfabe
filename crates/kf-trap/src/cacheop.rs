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
//! - ★★★ **Hopper, Blackwell** — w828: a different protocol, and a **READ with a side effect**
//!   (`kmemsysDoCacheOp_GH100`, `ogkm-580: src/nvidia/src/kernel/gpu/mem_sys/arch/hopper/
//!   kern_mem_sys_gh100.c:45-215`, bound for GH100 and every GB1xx/GB20x by
//!   `generated/g_kern_mem_sys_nvoc.c:399-407`): the op is STARTED by reading the op register,
//!   whose value is the token the hardware assigned it, and it is done when the `…_COMPLETED`
//!   register (`STATUS` 31:31, `TOKEN` 30:0) reads IDLE or has passed that token. The sysmembar
//!   (`kbusSendSysmembarSingle_GH100`, `ogkm-580: .../bus/arch/hopper/kern_bus_gh100.c:2908-2990`)
//!   uses the same scheme on `NV_XAL_EP_UFLUSH_FB_FLUSH`. A shadow cannot express a read that
//!   starts work, so these pages are [`crate::memmap::Disposition::Hole`]s on those families and
//!   [`token_registers`] is what the read exit serves. ⊘ `write` ops ([`registers`]) are empty
//!   there: the GM107 registers do not exist on these chips.
//!
//! ⊘ **Not testable on the GA106 bench** — no Hopper/Blackwell part exists there. Covered by unit
//! tests of the token protocol against the driver's own loop (`tests` below).
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
    /// ★ w828: `NV_XAL_EP_UFLUSH_FB_FLUSH` — the Hopper+ sysmembar (host: the same verb's
    /// `FB_FLUSH_YES` alone, `ctrl2080fb.h:664-666`; *"If only the FB flush is needed, only the
    /// _APERTURE and _FB_FLUSH_YES are needed"*, `kern_mem_sys_ctrl.c:1500-1512`). Only reached
    /// through [`token_registers`].
    FbFlush,
}

impl CacheOp {
    /// Index into a per-op array.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            CacheOp::FlushDirty => 0,
            CacheOp::SysmemInvalidate => 1,
            CacheOp::PeermemInvalidate => 2,
            CacheOp::FbFlush => 3,
        }
    }
    /// How many ops there are (the size of a per-op array).
    pub const COUNT: usize = 4;
    /// All ops, by index.
    pub const ALL: [CacheOp; CacheOp::COUNT] =
        [CacheOp::FlushDirty, CacheOp::SysmemInvalidate, CacheOp::PeermemInvalidate, CacheOp::FbFlush];
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

/// ★★★ w828 — one **read-started** cache op of the Hopper+ token protocol: reading `start` begins
/// the op and returns its token; `completed` reports progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenOp {
    /// The register whose READ starts the op (its value: the op's token, `TOKEN` 30:0).
    pub start: u32,
    /// `…_COMPLETED`: `STATUS` 31:31 (`BUSY` = 1) and the last completed `TOKEN` 30:0.
    pub completed: u32,
    /// What the host must do.
    pub op: CacheOp,
}

/// `…_TOKEN` is `30:0` on every token register (`ogkm-580: hopper/gh100/pri_nv_xal_ep.h:29-31`,
/// `hopper/gh100/dev_vm.h:29-31`).
pub const TOKEN_MASK: u32 = 0x7FFF_FFFF;
/// `…_COMPLETED_STATUS_BUSY` — `31:31` = 1 (same citations).
pub const COMPLETED_BUSY: u32 = 1 << 31;
/// `NV_XAL_EP_MEMOP_MAX_OUTSTANDING` — the driver waits only while `completed` is within this many
/// tokens behind `start` (`ogkm-580: hopper/gh100/dev_nv_xal_addendum.h:29`).
pub const MEMOP_MAX_OUTSTANDING: u32 = 140;

/// ★★★ w828 — the read-started cache ops of `family` (empty before Hopper).
///
/// Offsets are GH100's, and they are Blackwell's too: `kmemsysDoCacheOp_GH100` and
/// `kbusSendSysmembarSingle_GH100` are compiled against `published/hopper/gh100/` and bound for
/// every GB1xx/GB20x chip (`g_kern_mem_sys_nvoc.c:399-407`); `blackwell/gb100/pri_nv_xal_ep.h:53-56`
/// and `blackwell/gb100/dev_vm.h:42-54` restate the same offsets. The VF registers are reached
/// through `GPU_VREG_RD32`, i.e. `NV_VIRTUAL_FUNCTION_FULL_PHYS_OFFSET` (`0xB80000`) + `0xF10…`.
///
/// ⊘ `NV_XAL_EP_UFLUSH_L2_CLEAN_COMPTAGS` (`0x10f808`/`0x10f80c`) shares the XAL page and is NOT
/// here: there is no unprivileged host verb that cleans comptags, so it keeps the page's shadow
/// semantics (it read back `0`, i.e. `IDLE`, before this change too) — an owner decision, named.
#[must_use]
pub fn token_registers(family: Family) -> &'static [TokenOp] {
    const GH100_STYLE: &[TokenOp] = &[
        // `NV_XAL_EP_UFLUSH_L2_FLUSH_DIRTY` / `_COMPLETED` (`pri_nv_xal_ep.h:28-33`).
        TokenOp { start: 0x0010_F810, completed: 0x0010_F814, op: CacheOp::FlushDirty },
        // `NV_VIRTUAL_FUNCTION_PRIV_FUNC_L2_SYSMEM_INVALIDATE` / `_COMPLETED` (`dev_vm.h:28-33`).
        TokenOp { start: 0x00B8_0F10, completed: 0x00B8_0F14, op: CacheOp::SysmemInvalidate },
        // `…_L2_PEERMEM_INVALIDATE` / `_COMPLETED` (`dev_vm.h:34-39`).
        TokenOp { start: 0x00B8_0F18, completed: 0x00B8_0F1C, op: CacheOp::PeermemInvalidate },
        // `NV_XAL_EP_UFLUSH_FB_FLUSH` / `_COMPLETED` (`pri_nv_xal_ep.h`, the sysmembar).
        TokenOp { start: 0x0010_F800, completed: 0x0010_F804, op: CacheOp::FbFlush },
    ];
    match family {
        Family::Turing | Family::Ampere | Family::Ada => &[],
        Family::Hopper | Family::Blackwell => GH100_STYLE,
    }
}

/// What a read at `off` means under the token protocol, if anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenRead {
    /// A read of the op's start register: the op begins; answer [`start_token`].
    Start(CacheOp),
    /// A read of the op's `…_COMPLETED`: answer [`completed_word`].
    Completed(CacheOp),
}

/// Classify a read at `off` for `family`.
#[must_use]
pub fn token_read(family: Family, off: u64) -> Option<TokenRead> {
    token_registers(family).iter().find_map(|t| {
        if u64::from(t.start) == off {
            Some(TokenRead::Start(t.op))
        } else if u64::from(t.completed) == off {
            Some(TokenRead::Completed(t.op))
        } else {
            None
        }
    })
}

/// The value a start read returns: the token of the request just issued (`issued` counts every
/// request so far, this one included).
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub const fn start_token(issued: u64) -> u32 {
    (issued as u32) & TOKEN_MASK
}

/// The `…_COMPLETED` word for `issued` requests of which the host has finished `done`
/// (`done <= issued`): `IDLE` with the last token once they are equal, `BUSY` with the last
/// finished token while the host op is in flight.
///
/// ★ Completion is a HOST event: `done` moves only when the host verb returned
/// (`kf_qemu`'s `serve_cache_ops`), never on the read.
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub const fn completed_word(issued: u64, done: u64) -> u32 {
    let tok = (done as u32) & TOKEN_MASK;
    if done >= issued { tok } else { tok | COMPLETED_BUSY }
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
    fn hopper_and_blackwell_have_no_write_started_ops() {
        assert!(registers(Family::Hopper).is_empty());
        assert!(registers(Family::Blackwell).is_empty());
    }

    /// The driver's own wait (`kmemsysDoCacheOp_GH100`, `kern_mem_sys_gh100.c:130-195`), verbatim
    /// in shape: `true` = still waiting.
    fn driver_waits(start: u32, completed: u32) -> bool {
        let busy = completed & COMPLETED_BUSY != 0;
        let tok = completed & TOKEN_MASK;
        busy && (start.wrapping_sub(tok) & TOKEN_MASK) <= MEMOP_MAX_OUTSTANDING
    }

    #[test]
    fn a_read_starts_the_op_and_the_guest_waits_until_the_host_finishes_it() {
        assert_eq!(token_read(Family::Hopper, 0x10F810), Some(TokenRead::Start(CacheOp::FlushDirty)));
        assert_eq!(token_read(Family::Blackwell, 0xB80F14), Some(TokenRead::Completed(CacheOp::SysmemInvalidate)));
        assert_eq!(token_read(Family::Ampere, 0x10F810), None, "GA10x has no token protocol");
        // Idle device: COMPLETED reads IDLE, token 0 (the registers' _INIT values).
        assert_eq!(completed_word(0, 0), 0);
        // The guest READS the start register: request 1 is issued, token 1.
        let issued = 1;
        let start = start_token(issued);
        assert_eq!(start, 1);
        // The host has not run it: BUSY, and within the window, so the driver WAITS.
        assert!(driver_waits(start, completed_word(issued, 0)), "must not pass before the host op");
        // The host verb returned: IDLE — the driver's loop exits on `bMemopBusy == FALSE`.
        assert!(!driver_waits(start, completed_word(issued, 1)));
    }

    #[test]
    fn the_token_wraps_at_31_bits_and_the_wait_still_holds() {
        let issued = u64::from(TOKEN_MASK) + 2; // token wrapped to 1
        let start = start_token(issued);
        assert_eq!(start, 1);
        assert!(driver_waits(start, completed_word(issued, issued - 1)), "done = 0x7fffffff, one behind");
        assert!(!driver_waits(start, completed_word(issued, issued)));
    }

    #[test]
    fn every_token_op_is_on_a_hole_page_of_its_family() {
        for f in [Family::Hopper, Family::Blackwell] {
            let holes = crate::memmap::holes_for(f);
            for t in token_registers(f) {
                for r in [t.start, t.completed] {
                    let page = u64::from(r) & !(crate::memmap::PAGE - 1);
                    assert!(holes.iter().any(|(p, _)| *p == page), "{f:?}: {r:#x} must exit on READ");
                }
            }
        }
    }
}
