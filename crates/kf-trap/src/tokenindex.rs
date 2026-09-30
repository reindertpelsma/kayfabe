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
                Some(
                    (((value >> Self::RUNLIST_SHIFT) & Self::RUNLIST_MASK) << Self::CHID_BITS)
                        | chid,
                )
            }
        }
    }

    /// The index of a channel the guest allocated on `runlist` with `chid`, or `None` when it cannot
    /// have one (a chid past the declared count, a runlist past the field).
    #[must_use]
    pub const fn of_channel(self, runlist: u32, chid: u32) -> Option<u32> {
        match self {
            TokenIndex::Vector { mask } => {
                if chid & mask == chid {
                    Some(chid)
                } else {
                    None
                }
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

/// ★★★ 2026-09-30 — **the exact 32-bit value the GUEST writes to the doorbell for its channel
/// `(runlist, chid)`** (`docs/design/V3_DOORBELL_IOEVENTFD.md` §3): what a KVM `DATAMATCH`
/// ioeventfd must equal, byte for byte, for the fast path to take that channel's doorbells.
///
/// [`TokenIndex`] only needs to *find a slot* from a value (it masks, as hardware does). A datamatch
/// needs the reverse, and exactly: the guest RM builds the token itself —
/// `kchannelCtrlCmdGpfifoGetWorkSubmitToken_IMPL` → `kfifoGenerateWorkSubmitTokenHal_*`, never the
/// GSP — as `RUNLIST_ID | VECTOR`, plus `RUNLIST_DOORBELL = _ENABLE` on GB20x. The HAL a die group
/// binds is a code fact (`ogkm-580: generated/g_kernel_fifo_nvoc.c:636-656`: TU10x → `_TU102`;
/// GA100…AD10x and GH100 → `_GA100`; GB20x → `_GB202`; the rest → `_GB100`), and each body names
/// its fields through ONE header family (`kernel_fifo_tu102.c:116-117`, `kernel_fifo_ga100.c:226-227`
/// use `NV_CTRL_VF_DOORBELL_*`; `kernel_fifo_gb100.c:114-115`, `kernel_fifo_gb202.c:73-76` use
/// `NV_VIRTUAL_FUNCTION_DOORBELL_*`) — so the per-die-group row below is `(HAL, header prefix,
/// sets RUNLIST_DOORBELL)` and every bit position comes from the generated hwref table.
///
/// ⊘ A value this predicts wrongly costs only speed, never correctness: the guest's real value then
/// matches no ioeventfd and takes the trapped path, which masks it into the same slot. That is why
/// a registration is checked with [`GuestTokenFormat::value`] ∘ [`TokenIndex::of_doorbell`] ==
/// [`TokenIndex::of_channel`] before it is made — a value that would land in ANOTHER slot is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestTokenFormat {
    /// `RUNLIST_ID` `(hi, lo)`.
    runlist: (u32, u32),
    /// `VECTOR` (the chid) `(hi, lo)`.
    vector: (u32, u32),
    /// Bits the HAL sets on every token (GB202's `RUNLIST_DOORBELL_ENABLE`).
    always: u32,
}

/// Which token HAL each die group's guest RM binds, and how that body names its fields.
/// `(die group, HAL, header prefix, sets RUNLIST_DOORBELL = _ENABLE)`.
const TOKEN_HALS: [(kf_chip::hwref::DieGroup, &str, &str, bool); 7] = {
    use kf_chip::hwref::DieGroup as G;
    [
        (G::Tu10x, "TU102", "NV_CTRL_VF_DOORBELL", false),
        (G::Ga100, "GA100", "NV_CTRL_VF_DOORBELL", false),
        (G::Ga10x, "GA100", "NV_CTRL_VF_DOORBELL", false),
        (G::Ad10x, "GA100", "NV_CTRL_VF_DOORBELL", false),
        (G::Gh100, "GA100", "NV_CTRL_VF_DOORBELL", false),
        (G::Gb10x, "GB100", "NV_VIRTUAL_FUNCTION_DOORBELL", false),
        (G::Gb20x, "GB202", "NV_VIRTUAL_FUNCTION_DOORBELL", true),
    ]
};

impl GuestTokenFormat {
    /// The guest's token layout for die group `g`, every field from the hwref table.
    ///
    /// # Errors
    /// The name that did not resolve (an ambiguous or absent field is a refusal, never a guess —
    /// the caller then leaves every doorbell on the trapped path).
    pub fn for_die_group(g: kf_chip::hwref::DieGroup) -> Result<GuestTokenFormat, String> {
        let t = kf_chip::hwref::table();
        let &(_, hal, prefix, sets_rd) = TOKEN_HALS
            .iter()
            .find(|(d, ..)| *d == g)
            .ok_or_else(|| format!("{g:?}: no token HAL row"))?;
        let range = |field: &str| -> Result<(u32, u32), String> {
            let name = format!("{prefix}_{field}");
            let (hi, lo) = t
                .range(g, &name)
                .map_err(|r| format!("{g:?} ({hal}): {name} does not resolve: {r:?}"))?;
            match (u32::try_from(hi), u32::try_from(lo)) {
                (Ok(hi), Ok(lo)) if hi < 32 && lo <= hi => Ok((hi, lo)),
                _ => Err(format!("{g:?}: {name} = {hi}:{lo} is not a 32-bit field")),
            }
        };
        let runlist = range("RUNLIST_ID")?;
        let vector = range("VECTOR")?;
        let always = if sets_rd {
            let (hi, lo) = range("RUNLIST_DOORBELL")?;
            let name = format!("{prefix}_RUNLIST_DOORBELL_ENABLE");
            let en = t
                .value(g, &name)
                .map_err(|r| format!("{g:?} ({hal}): {name} does not resolve: {r:?}"))?;
            let width = hi - lo + 1;
            let en = u32::try_from(en)
                .ok()
                .filter(|v| width == 32 || *v < (1 << width))
                .ok_or_else(|| format!("{g:?}: {name} = {en:#x} does not fit {hi}:{lo}"))?;
            en << lo
        } else {
            0
        };
        Ok(GuestTokenFormat {
            runlist,
            vector,
            always,
        })
    }

    const fn fits(v: u32, (hi, lo): (u32, u32)) -> bool {
        let width = hi - lo + 1;
        width >= 32 || v < (1 << width)
    }

    /// The value the guest writes for its channel `(runlist, chid)`, or `None` when either does
    /// not fit its field (the guest cannot have produced such a token).
    #[must_use]
    pub const fn value(self, runlist: u32, chid: u32) -> Option<u32> {
        if !Self::fits(runlist, self.runlist) || !Self::fits(chid, self.vector) {
            return None;
        }
        Some((runlist << self.runlist.1) | (chid << self.vector.1) | self.always)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ★ Every die group: the predicted value is what its HAL body computes (hand-written here from
    /// the four ogkm bodies, independently of the hwref derivation above), and the device's own
    /// index finds the channel's slot from it — the precondition for registering it at all.
    #[test]
    fn the_guest_token_is_runlist_or_chid_and_gb20x_sets_bit_30() {
        use kf_chip::hwref::DieGroup as G;
        for g in G::ALL {
            let f = GuestTokenFormat::for_die_group(g).expect("resolves");
            let bit30 = if g == G::Gb20x { 1 << 30 } else { 0 };
            for (rl, chid) in [
                (0u32, 0u32),
                (0, 1),
                (13, 1),
                (2, 0x7FF),
                (0x7F, 0x7FF),
                (9, 0x42),
            ] {
                let v = f.value(rl, chid).expect("fits");
                assert_eq!(v, (rl << 16) | chid | bit30, "{g:?} ({rl}, {chid:#x})");
                let r = TokenIndex::RunlistVector;
                assert_eq!(
                    r.of_doorbell(v),
                    r.of_channel(rl, chid),
                    "{g:?}: the guest's own value must find the slot its channel was born in"
                );
            }
            assert_eq!(f.value(0x80, 1), None, "{g:?}: RUNLIST_ID is 7 bits");
            assert_eq!(f.value(0, 0x1000), None, "{g:?}: VECTOR is 12 bits");
        }
    }

    #[test]
    fn vector_is_the_old_mask_byte_for_byte() {
        let v = TokenIndex::Vector { mask: 0xFFF };
        for x in [0u32, 1, 0x7, 0x0001_0007, 0x4000_0002, 0xFFFF_FFFF] {
            assert_eq!(v.of_doorbell(x), Some(x & 0xFFF));
        }
        assert_eq!(
            v.of_channel(9, 5),
            Some(5),
            "the runlist is ignored: chids are device-unique"
        );
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
        assert_eq!(
            (r.chid_of(scrub), r.chid_of(uvm)),
            (1, 1),
            "RC_TRIGGERED names the guest's chid"
        );
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

    /// ★ The same index on Turing … Hopper (the device's, `kf_core::Plane::for_device`): the GA100 /
    /// TU102 HAL token is `RUNLIST_ID` 22:16 | `VECTOR` 11:0 with no bit 30. `[measured 610.57.04 on
    /// GA102]` the guest allocated chid 1 on two runlists (per-runlist CHRAM from 610.43.02): two
    /// slots, each found by its own token. And a ≤ 595 guest's device-unique chids stay distinct.
    #[test]
    fn a_610_ampere_guests_per_runlist_chids_do_not_collide() {
        let r = TokenIndex::RunlistVector;
        let token = |rl: u32, chid: u32| (rl << 16) | chid;
        let (ceutils, user) = (r.of_channel(13, 1).unwrap(), r.of_channel(0, 1).unwrap());
        assert_ne!(ceutils, user);
        assert_eq!(
            (r.of_doorbell(token(13, 1)), r.of_doorbell(token(0, 1))),
            (Some(ceutils), Some(user))
        );
        let (a, b) = (r.of_channel(13, 1).unwrap(), r.of_channel(0, 2).unwrap());
        assert_ne!(
            a, b,
            "a 580 guest's global chids 1 and 2 on different runlists"
        );
        assert_eq!(
            (r.chid_of(ceutils), r.chid_of(user)),
            (1, 1),
            "RC_TRIGGERED names the guest's chid"
        );
    }
}

/// ★ The token fields, held to each die group's header (`kf_chip::hwref`,
/// `docs/design/V3_HW_BOUNDARY_INVENTORY.md`): Turing … Hopper tokens are `NV_CTRL_VF_DOORBELL`
/// (`kfifoGenerateWorkSubmitTokenHal_TU102/_GA100`), Blackwell's `NV_VIRTUAL_FUNCTION_DOORBELL`
/// (`_GB100`, `_GB202`); both carry `RUNLIST_ID` 22:16 and `VECTOR` 11:0.
#[cfg(test)]
mod hwref_check {
    use super::*;
    use kf_chip::hwref::DieGroup;
    use kf_chip::hwref::expect::{mask, range, val};

    #[test]
    fn the_token_fields_are_each_die_groups_header() {
        for g in [
            DieGroup::Tu10x,
            DieGroup::Ga100,
            DieGroup::Ga10x,
            DieGroup::Ad10x,
            DieGroup::Gh100,
        ] {
            assert_eq!(
                mask(g, "NV_CTRL_VF_DOORBELL_VECTOR"),
                u64::from(TokenIndex::VECTOR_MASK),
                "{g:?}"
            );
            assert_eq!(
                range(g, "NV_CTRL_VF_DOORBELL_RUNLIST_ID"),
                (22, 16),
                "{g:?}"
            );
        }
        for g in [DieGroup::Gb10x, DieGroup::Gb20x] {
            let (hi, lo) = range(g, "NV_VIRTUAL_FUNCTION_DOORBELL_RUNLIST_ID");
            assert_eq!(lo, u64::from(TokenIndex::RUNLIST_SHIFT), "{g:?}");
            assert_eq!(
                (1u64 << (hi - lo + 1)) - 1,
                u64::from(TokenIndex::RUNLIST_MASK),
                "{g:?}"
            );
            assert_eq!(
                mask(g, "NV_VIRTUAL_FUNCTION_DOORBELL_VECTOR"),
                u64::from(TokenIndex::VECTOR_MASK),
                "{g:?}"
            );
            // `CHID_BITS` is the per-runlist channel RAM: `NV_CHRAM_CHANNEL__SIZE_1` = 2048.
            assert_eq!(
                1u64 << TokenIndex::CHID_BITS,
                val(g, "NV_CHRAM_CHANNEL__SIZE_1"),
                "{g:?}"
            );
        }
        // ★ The RUNLIST_DOORBELL bit is where the two Blackwell die groups differ: 30:30 = 1 on GB20x;
        // on GB10x it is 22:22 with `_ENABLE` = 0 — INSIDE RUNLIST_ID and never set. Neither reaches
        // the index (`RUNLIST_MASK` stops at bit 22 and a GB10x runlist id < 64 leaves bit 22 clear).
        assert_eq!(
            range(
                DieGroup::Gb20x,
                "NV_VIRTUAL_FUNCTION_DOORBELL_RUNLIST_DOORBELL"
            ),
            (30, 30)
        );
        assert_eq!(
            val(
                DieGroup::Gb20x,
                "NV_VIRTUAL_FUNCTION_DOORBELL_RUNLIST_DOORBELL_ENABLE"
            ),
            1
        );
        assert_eq!(
            range(
                DieGroup::Gb10x,
                "NV_VIRTUAL_FUNCTION_DOORBELL_RUNLIST_DOORBELL"
            ),
            (22, 22)
        );
        assert_eq!(
            val(
                DieGroup::Gb10x,
                "NV_VIRTUAL_FUNCTION_DOORBELL_RUNLIST_DOORBELL_ENABLE"
            ),
            0
        );
    }
}
