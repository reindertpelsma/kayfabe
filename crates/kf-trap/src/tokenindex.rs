//! ★★★ 2026-09-26 — **the doorbell token → token-table index, per family** (`V3_FAMILY_PORT_BLACKWELL.md` §4).
//!
//! The guest computes its channels' work-submit tokens itself (`kchannelCtrlCmdGpfifoGetWorkSubmitToken`
//! never reaches the GSP), and a doorbell write carries one. Our token table is indexed by a function
//! of that token, and until Blackwell the function was `VECTOR` (11:0) = the chid: on Turing … Hopper
//! the guest RM runs ONE global `CHID_MGR`, so the chid is device-unique.
//!
//! `[measured GB203, bws4]` **Blackwell allocates chids PER RUNLIST.** `bUsePerRunlistChram` is a HAL
//! field that defaults `NV_TRUE` for GB100/GB102/GB10B/GB110/GB112 and every GB20x
//! (`ogkm-580: generated/g_kernel_fifo_nvoc.c:226-236`; Turing … Hopper default `FALSE` and only an
//! SR-IOV host turns it on, `kernel_fifo_init.c:197-222`). The PMA scrubber (runlist 1, chid 1) and
//! UVM's first channel (another runlist, chid 1) then both indexed slot 1, and UVM's channel was
//! refused `OverDeclaredCap { cap: 0 }` → `UVM_REGISTER_GPU` `0x1a`. The token carries the runlist:
//! `kfifoGenerateWorkSubmitTokenHal_GB202` = `RUNLIST_ID` 22:16 | `VECTOR` (chid) 11:0 |
//! `RUNLIST_DOORBELL` 30:30 (`kernel_fifo_gb202.c:58-78`; GB100's arm the same fields, no bit 30).
//!
//! ⇒ [`TokenIndex::RunlistVector`]: index = `runlist << CHID_BITS | chid`. `CHID_BITS` is the width
//! of the per-runlist channel count we DECLARE to the guest (`kf_rm::hostquery::AUTHORED_FIFO_CHANNELS`
//! = 2048 per runlist), so every chid the guest RM can allocate has a slot and none aliases; a token
//! outside that range names no slot (the doorbell does nothing, as for any unknown token).

/// How a doorbell value and a channel's `(runlist, chid)` become a token-table index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenIndex {
    /// Turing … Hopper: the chid is device-unique — `index = value & mask`.
    Vector {
        /// The table's mask (`len - 1`).
        mask: u32,
    },
    /// Blackwell: chids are per runlist — `index = RUNLIST_ID << CHID_BITS | VECTOR`.
    RunlistVector,
}

impl TokenIndex {
    /// Chid bits per runlist: the 2048 channels per runlist we declare.
    pub const CHID_BITS: u32 = 11;
    /// `NV_VIRTUAL_FUNCTION_DOORBELL_RUNLIST_ID` 22:16 (`blackwell/gb202/dev_vm.h:29`).
    const RUNLIST_SHIFT: u32 = 16;
    const RUNLIST_MASK: u32 = 0x7F;
    /// `NV_VIRTUAL_FUNCTION_DOORBELL_VECTOR` 11:0.
    const VECTOR_MASK: u32 = 0xFFF;

    /// The table length this index needs.
    #[must_use]
    pub const fn table_len(self) -> usize {
        match self {
            TokenIndex::Vector { mask } => mask as usize + 1,
            TokenIndex::RunlistVector => 1 << (7 + Self::CHID_BITS),
        }
    }

    /// The index a doorbell write of `value` names, or `None` (a value naming no slot does nothing).
    /// Lock-free and pure: this runs on the vCPU.
    #[inline]
    #[must_use]
    pub const fn of_doorbell(self, value: u32) -> Option<u32> {
        match self {
            TokenIndex::Vector { mask } => Some(value & mask),
            TokenIndex::RunlistVector => {
                let chid = value & Self::VECTOR_MASK;
                if chid >= 1 << Self::CHID_BITS {
                    return None;
                }
                Some((((value >> Self::RUNLIST_SHIFT) & Self::RUNLIST_MASK) << Self::CHID_BITS) | chid)
            }
        }
    }

    /// The index of a channel the guest allocated on `runlist` with `chid`, or `None` when it cannot
    /// have one (a chid past the declared count, a runlist past the field).
    #[must_use]
    pub const fn of_channel(self, runlist: u32, chid: u32) -> Option<u32> {
        match self {
            TokenIndex::Vector { mask } => {
                if chid & mask == chid { Some(chid) } else { None }
            }
            TokenIndex::RunlistVector => {
                if chid >= 1 << Self::CHID_BITS || runlist > Self::RUNLIST_MASK {
                    return None;
                }
                Some((runlist << Self::CHID_BITS) | chid)
            }
        }
    }

    /// The guest's chid inside an index (what an `RC_TRIGGERED` names — with the engine, which
    /// the guest turns back into the runlist).
    #[must_use]
    pub const fn chid_of(self, index: u32) -> u32 {
        match self {
            TokenIndex::Vector { mask } => index & mask,
            TokenIndex::RunlistVector => index & ((1 << Self::CHID_BITS) - 1),
        }
    }

    /// An internal index (already computed by one of the above) clamped into the table — the
    /// identity on [`TokenIndex::RunlistVector`] (the caller bounds-checks), `& mask` otherwise,
    /// exactly as before for Turing … Hopper.
    #[inline]
    #[must_use]
    pub const fn clamp(self, index: u32) -> u32 {
        match self {
            TokenIndex::Vector { mask } => index & mask,
            TokenIndex::RunlistVector => index,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_is_the_old_mask_byte_for_byte() {
        let v = TokenIndex::Vector { mask: 0xFFF };
        for x in [0u32, 1, 0x7, 0x0001_0007, 0x4000_0002, 0xFFFF_FFFF] {
            assert_eq!(v.of_doorbell(x), Some(x & 0xFFF));
        }
        assert_eq!(v.of_channel(9, 5), Some(5), "the runlist is ignored: chids are device-unique");
        assert_eq!(v.of_channel(0, 0x1000), None);
        assert_eq!(v.table_len(), 4096);
    }

    /// The GB203 collision, measured: the scrubber (runlist 1, chid 1) and UVM's channel
    /// (runlist 2, chid 1) must be two slots; the guest's own tokens (bit 30 set) find them.
    #[test]
    fn per_runlist_chids_do_not_collide_and_the_guest_token_finds_its_slot() {
        let r = TokenIndex::RunlistVector;
        let scrub = r.of_channel(1, 1).unwrap();
        let uvm = r.of_channel(2, 1).unwrap();
        assert_ne!(scrub, uvm);
        assert_eq!((r.chid_of(scrub), r.chid_of(uvm)), (1, 1), "RC_TRIGGERED names the guest's chid");
        let token = |rl: u32, chid: u32| (1 << 30) | (rl << 16) | chid;
        assert_eq!(r.of_doorbell(token(1, 1)), Some(scrub));
        assert_eq!(r.of_doorbell(token(2, 1)), Some(uvm));
        // GB100 (no bit 30): the same slot.
        assert_eq!(r.of_doorbell((2 << 16) | 1), Some(uvm));
        // A chid past the declared 2048 names nothing.
        assert_eq!(r.of_doorbell(token(0, 0x800)), None);
        assert_eq!(r.of_channel(0, 0x800), None);
        assert!((r.of_doorbell(0xFFFF_F7FF).unwrap() as usize) < r.table_len());
    }
}
