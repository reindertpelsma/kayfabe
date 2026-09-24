//! ★★★★★ **The MMU invalidate registers — the trap half of the sync point** (`V3_P4_PORT_MAP.md`
//! §2.1(a); `THE_CONSTRAINTS.md` §49.1; `THE_TRANSLATED_PLANE.md` §5).
//!
//! The guest's RM writes `MMU_INVALIDATE_PDB` (low 28 bits of `pdb >> 12` + aperture) and
//! `MMU_INVALIDATE_UPPER_PDB` (the next 20), then `MMU_INVALIDATE` with `TRIGGER` set, then
//! spin-reads `MMU_INVALIDATE` until `TRIGGER` reads back 0 (`kgmmuCommitTlbInvalidate_TU102`,
//! `ogkm-580 kern_gmmu_tu102.c:112-120`; Blackwell's `kgmmuCommitTlbInvalidate_GB100`,
//! `kern_gmmu_gb100.c:51-59`, is the same sequence). ⇒ The trigger is §5.5's
//! [`WriteSemantics::Trigger`](crate::shadow::WriteSemantics::Trigger): the vCPU **arms and
//! publishes, and returns**; the VA manager walks, reconciles, maps and only then clears.
//! ⊘ The guest must never observe the invalidate complete before the host mapping it describes
//! is committed (§49.1) — the clear is the last thing the reconcile does.
//!
//! ## One layout for every family
//!
//! The three registers and their fields are identical in `turing/tu102/dev_vm.h:120-131`
//! (Turing, Ampere GA10x, Ada and Hopper all run the TU102 HAL for this — `gh100/dev_vm.h`
//! defines none of its own) and in `blackwell/gb100/dev_vm.h` (checked field by field at
//! ogkm-580.159.04). Where they SIT is per chip: at `usermode base − 0x30000` (the PRIV block the
//! usermode window lives in), which is why [`InvalidateRegs::from_usermode_base`] derives them
//! from the advertised base instead of carrying a GA106 constant.
//!
//! The decode is copied from the old tree's `kayfabe-device/src/mmuinval.rs:241-276` (pure, ran
//! on hardware); the recorder/drain half of that file (`note_trigger` / `drain_dirty_pdbs`,
//! `:526-630`) is NOT copied — it fed a table re-sweep, i.e. a mirror.

use crate::shadow::{Cell, Trigger, WriteSemantics};

/// `DRF_BASE(NV_VIRTUAL_FUNCTION)` — how far the usermode window sits above the PRIV block.
pub const USERMODE_ABOVE_PRIV: u64 = 0x0003_0000;
/// `NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE_PDB` (`turing/tu102/dev_vm.h:120`), PRIV-relative.
pub const MMU_INVALIDATE_PDB_OFF: u64 = 0x30A0;
/// `NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE_UPPER_PDB` (`…:128`), PRIV-relative.
pub const MMU_INVALIDATE_UPPER_PDB_OFF: u64 = 0x30A4;
/// `NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE` (`…:131`), PRIV-relative.
pub const MMU_INVALIDATE_OFF: u64 = 0x30B0;
/// `…_PDB_ADDR_ALIGNMENT` — the PDB is stored shifted right by 12 (`…:127`).
pub const PDB_ADDR_ALIGNMENT: u32 = 12;
/// `…_MMU_INVALIDATE_TRIGGER` = bit 31.
pub const TRIGGER_BIT: u32 = 1 << 31;

/// The three BAR0 offsets, for one chip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidateRegs {
    /// `MMU_INVALIDATE` — the trigger and the scope bits.
    pub trigger: u64,
    /// `MMU_INVALIDATE_PDB`.
    pub pdb: u64,
    /// `MMU_INVALIDATE_UPPER_PDB`.
    pub upper_pdb: u64,
}

impl InvalidateRegs {
    /// Derive the offsets from the chip's advertised usermode base. `None` when the base is
    /// below the PRIV delta — refused rather than wrapped into a wrong offset.
    #[must_use]
    pub fn from_usermode_base(usermode: u64) -> Option<InvalidateRegs> {
        let priv_base = usermode.checked_sub(USERMODE_ABOVE_PRIV)?;
        Some(InvalidateRegs {
            trigger: priv_base + MMU_INVALIDATE_OFF,
            pdb: priv_base + MMU_INVALIDATE_PDB_OFF,
            upper_pdb: priv_base + MMU_INVALIDATE_UPPER_PDB_OFF,
        })
    }
}

/// Where the named page directory lives (`MMU_INVALIDATE_PDB_APERTURE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PdbAperture {
    /// `0` — video memory: an offset into the guest's framebuffer, i.e. into the store.
    Vidmem,
    /// `1` — system memory: a guest-physical address.
    Sysmem,
}

/// One decoded `MMU_INVALIDATE` word, with the PDB halves latched before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Invalidate {
    /// The whole word, so a bit this port does not model survives into a log.
    pub raw: u32,
    /// Bit 31. `false` ⇒ the write set scope bits and committed nothing.
    pub trigger: bool,
    /// Bit 0 — the whole VA range of the named PDB. ⊘ RM's register path always sets it
    /// (`rm_cannot_express_a_narrow_invalidate`), so the walk is always of whole spaces.
    pub all_va: bool,
    /// Bit 1 — every PDB. RM then does NOT write the PDB registers (`kern_gmmu_tu102.c:112`),
    /// so [`Self::pdb`] is stale by design.
    pub all_pdb: bool,
    /// Bit 2 — the BAR (hub) VA spaces only.
    pub hubtlb_only: bool,
    /// Bits 5:3 — `REPLAY`.
    pub replay: u32,
    /// The page directory named, reassembled from the latched halves.
    pub pdb: u64,
    /// Its aperture.
    pub pdb_aperture: PdbAperture,
}

impl Invalidate {
    /// Decode `raw` against the latched `pdb_lo` / `pdb_hi` (`kgmmuSetPdbToInvalidate_TU102`,
    /// `kern_gmmu_tu102.c:143-153`: `_PDB_ADDR` 31:4 holds the low 28 bits of `pdb >> 12`,
    /// `_UPPER_PDB_ADDR` 19:0 the next 20).
    #[must_use]
    pub fn decode(raw: u32, pdb_lo: u32, pdb_hi: u32) -> Invalidate {
        let lo28 = u64::from(pdb_lo >> 4);
        let hi20 = u64::from(pdb_hi & 0x000F_FFFF);
        Invalidate {
            raw,
            trigger: raw & TRIGGER_BIT != 0,
            all_va: raw & 0b1 != 0,
            all_pdb: raw & 0b10 != 0,
            hubtlb_only: raw & 0b100 != 0,
            replay: (raw >> 3) & 0b111,
            pdb: ((hi20 << 28) | lo28) << PDB_ADDR_ALIGNMENT,
            pdb_aperture: if (pdb_lo >> 1) & 1 == 0 { PdbAperture::Vidmem } else { PdbAperture::Sysmem },
        }
    }

    /// The inverse, for a harness playing the guest: the two PDB register words RM would write
    /// for `pdb` in `aperture`.
    #[must_use]
    pub fn encode_pdb(pdb: u64, aperture: PdbAperture) -> (u32, u32) {
        let shifted = pdb >> PDB_ADDR_ALIGNMENT;
        let ap = match aperture {
            PdbAperture::Vidmem => 0,
            PdbAperture::Sysmem => 1,
        };
        let lo = (((shifted & 0x0FFF_FFFF) as u32) << 4) | (ap << 1);
        let hi = ((shifted >> 28) & 0x000F_FFFF) as u32;
        (lo, hi)
    }
}

/// ★ What the VA manager is asked to do: walk, reconcile, map — then `complete(seq)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidateRequest {
    /// The trigger's own sequence ([`Trigger::arm_next`]) — what the clear must name.
    pub seq: u64,
    /// The decoded word.
    pub inval: Invalidate,
}

/// ★★★ **The three registers of one GPU, as the vCPU sees them.**
///
/// - PDB lo/hi writes are `Plain` shadow cells (§5.5), latched for the next trigger.
/// - A trigger write with bit 31 set **arms first, then returns the request to publish**; one
///   without it only updates the shadow word.
/// - A read of the trigger register is served from the shadow: the stored word with bit 31
///   reflecting [`Trigger::read`] — disposition B, never a read exit.
///
/// ⊘ No lock, no allocation, no syscall: [`InvalidatePort::write`] is atomics only. The
/// caller (the trap) publishes the returned request and owes one wake.
#[derive(Debug)]
pub struct InvalidatePort {
    regs: InvalidateRegs,
    pdb_lo: Cell,
    pdb_hi: Cell,
    word: Cell,
    trigger: Trigger,
}

/// What a write to one of the three registers did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortWrite {
    /// Not one of the three registers.
    NotOurs,
    /// Shadow updated; nothing to publish.
    Latched,
    /// ★ The trigger was armed; publish this request to the VA manager and wake it.
    Publish(InvalidateRequest),
}

impl InvalidatePort {
    /// A port at `regs`, idle.
    #[must_use]
    pub fn new(regs: InvalidateRegs) -> InvalidatePort {
        InvalidatePort { regs, pdb_lo: Cell::new(), pdb_hi: Cell::new(), word: Cell::new(), trigger: Trigger::new() }
    }

    /// The offsets it answers.
    #[must_use]
    pub fn regs(&self) -> InvalidateRegs {
        self.regs
    }

    /// The trigger — the VA manager clears it through [`Trigger::complete`].
    #[must_use]
    pub fn trigger(&self) -> &Trigger {
        &self.trigger
    }

    /// vCPU, in the trap: a 32-bit write at BAR0 `offset`.
    pub fn write(&self, offset: u64, value: u32) -> PortWrite {
        if offset == self.regs.pdb {
            self.pdb_lo.apply(WriteSemantics::Plain, u64::from(value));
            PortWrite::Latched
        } else if offset == self.regs.upper_pdb {
            self.pdb_hi.apply(WriteSemantics::Plain, u64::from(value));
            PortWrite::Latched
        } else if offset == self.regs.trigger {
            // The readable word keeps the scope bits; bit 31 is the Trigger's, never stored.
            self.word.apply(WriteSemantics::Plain, u64::from(value & !TRIGGER_BIT));
            let inval = Invalidate::decode(value, self.pdb_lo.read() as u32, self.pdb_hi.read() as u32);
            if !inval.trigger {
                return PortWrite::Latched;
            }
            // ★ ARM BEFORE PUBLISH — see `Trigger::arm_next`.
            let seq = self.trigger.arm_next();
            PortWrite::Publish(InvalidateRequest { seq, inval })
        } else {
            PortWrite::NotOurs
        }
    }

    /// ★ P4: the request the trigger is armed for RIGHT NOW, rebuilt from the latched cells —
    /// `None` when idle. The VA manager's thread calls this on its wake instead of receiving a
    /// queued request, so the vCPU's whole cost stays `write` + one wake.
    ///
    /// ⚠ Sound because the latches are written BEFORE the trigger arms (the guest's own order,
    /// `kgmmuCommitTlbInvalidate_TU102`) and the guest issues the next invalidate only after this
    /// one reads idle or times out — in which case this one is `Superseded` and the rebuilt
    /// request names the LATER sequence, which is the one that must be served.
    #[must_use]
    pub fn armed_request(&self) -> Option<InvalidateRequest> {
        let seq = self.trigger.armed_seq()?;
        let raw = (self.word.read() as u32) | TRIGGER_BIT;
        let inval = Invalidate::decode(raw, self.pdb_lo.read() as u32, self.pdb_hi.read() as u32);
        Some(InvalidateRequest { seq, inval })
    }

    /// vCPU, a read at BAR0 `offset` (served from the shadow page in production). `None` when
    /// the offset is not one of the three.
    #[must_use]
    pub fn read(&self, offset: u64) -> Option<u32> {
        if offset == self.regs.pdb {
            Some(self.pdb_lo.read() as u32)
        } else if offset == self.regs.upper_pdb {
            Some(self.pdb_hi.read() as u32)
        } else if offset == self.regs.trigger {
            let busy = if self.trigger.read() != 0 { TRIGGER_BIT } else { 0 };
            Some(self.word.read() as u32 | busy)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shadow::ClearOutcome;

    /// A usermode base of `0xBB_0000` puts the trigger at `0xB8_30B0` — the `bar0+0xb830b0` the
    /// old tree measured on GA106 (`kayfabe-device/src/mmuinval.rs`, w462).
    #[test]
    fn offsets_derive_from_the_usermode_base() {
        let r = InvalidateRegs::from_usermode_base(0xBB_0000).unwrap();
        assert_eq!(r.trigger, 0xB8_30B0);
        assert_eq!(r.pdb, r.trigger - 0x10);
        assert_eq!(r.upper_pdb, r.trigger - 0xC);
        assert_eq!(InvalidateRegs::from_usermode_base(0x1000), None, "below the PRIV delta: refused");
    }

    #[test]
    fn pdb_round_trips_through_the_two_registers() {
        for pdb in [0u64, 0x1000, 0x0100_0000, 0x3_FFFF_F000, 0xFF_FFFF_F000_u64 & !0xFFF, 0x1234_5678_9000] {
            for ap in [PdbAperture::Vidmem, PdbAperture::Sysmem] {
                let (lo, hi) = Invalidate::encode_pdb(pdb, ap);
                let d = Invalidate::decode(TRIGGER_BIT | 1, lo, hi);
                assert_eq!(d.pdb, pdb, "{pdb:#x}");
                assert_eq!(d.pdb_aperture, ap);
                assert!(d.trigger && d.all_va && !d.all_pdb);
            }
        }
    }

    fn port() -> InvalidatePort {
        InvalidatePort::new(InvalidateRegs::from_usermode_base(0xBB_0000).unwrap())
    }

    #[test]
    fn a_trigger_write_arms_and_reads_busy_until_its_own_completion() {
        let p = port();
        let r = p.regs();
        let (lo, hi) = Invalidate::encode_pdb(0x0100_0000, PdbAperture::Vidmem);
        assert_eq!(p.write(r.pdb, lo), PortWrite::Latched);
        assert_eq!(p.write(r.upper_pdb, hi), PortWrite::Latched);
        let PortWrite::Publish(req) = p.write(r.trigger, TRIGGER_BIT | 1) else { panic!("no publish") };
        assert_eq!(req.inval.pdb, 0x0100_0000);
        assert_eq!(p.read(r.trigger).unwrap() & TRIGGER_BIT, TRIGGER_BIT, "busy while armed");
        assert_eq!(p.trigger().complete(req.seq), ClearOutcome::Cleared);
        assert_eq!(p.read(r.trigger).unwrap() & TRIGGER_BIT, 0, "idle after the clear");
        assert_eq!(p.read(r.trigger).unwrap(), 1, "the scope bits read back");
    }

    #[test]
    fn a_write_without_the_trigger_bit_only_latches() {
        let p = port();
        assert_eq!(p.write(p.regs().trigger, 0b11), PortWrite::Latched);
        assert_eq!(p.trigger().read(), 0);
        assert_eq!(p.write(0x1234, 0), PortWrite::NotOurs);
    }

    #[test]
    fn the_armed_request_is_rebuilt_from_the_latches_and_names_the_latest_arm() {
        let p = port();
        let r = p.regs();
        assert_eq!(p.armed_request(), None, "idle");
        let (lo, hi) = Invalidate::encode_pdb(0x2_F339_2000, PdbAperture::Vidmem);
        p.write(r.pdb, lo);
        p.write(r.upper_pdb, hi);
        // `[cap3 #159730]` the guest's BAR2 invalidate word.
        let PortWrite::Publish(a) = p.write(r.trigger, 0x8001_0005) else { panic!() };
        assert_eq!(p.armed_request(), Some(a));
        assert_eq!(a.inval.pdb, 0x2_F339_2000);
        assert!(a.inval.all_va && a.inval.hubtlb_only && !a.inval.all_pdb);
        let PortWrite::Publish(b) = p.write(r.trigger, 0x8001_0005) else { panic!() };
        assert_eq!(p.armed_request().map(|x| x.seq), Some(b.seq), "the later arm");
        assert_eq!(p.trigger().complete(b.seq), ClearOutcome::Cleared);
        assert_eq!(p.armed_request(), None);
    }

    /// §5.5's corruption: A times out in the guest, B is issued, A's work finishes. A's clear
    /// must be `Superseded` and B must still read busy.
    #[test]
    fn a_stale_completion_never_clears_a_later_trigger() {
        let p = port();
        let r = p.regs();
        let PortWrite::Publish(a) = p.write(r.trigger, TRIGGER_BIT | 1) else { panic!() };
        let PortWrite::Publish(b) = p.write(r.trigger, TRIGGER_BIT | 1) else { panic!() };
        assert!(b.seq > a.seq);
        assert_eq!(p.trigger().complete(a.seq), ClearOutcome::Superseded);
        assert_ne!(p.read(r.trigger).unwrap() & TRIGGER_BIT, 0, "B is still pending");
        assert_eq!(p.trigger().complete(b.seq), ClearOutcome::Cleared);
    }

    /// ★ The ordering `arm_next` exists for: a completion that runs before the vCPU would have
    /// armed (under the ring-sequence scheme) must still clear, never strand the trigger.
    #[test]
    fn arm_precedes_publish_so_a_fast_completion_cannot_strand_the_trigger() {
        let t = Trigger::new();
        let seq = t.arm_next();
        assert_eq!(t.complete(seq), ClearOutcome::Cleared);
        assert_eq!(t.read(), 0);
    }
}
