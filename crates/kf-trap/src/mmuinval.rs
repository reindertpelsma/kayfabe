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
use core::sync::atomic::{AtomicU64, Ordering};

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
            pdb_aperture: if (pdb_lo >> 1) & 1 == 0 {
                PdbAperture::Vidmem
            } else {
                PdbAperture::Sysmem
            },
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

impl Invalidate {
    /// `ALL_PDB` + `ALL_VA`: the scope that covers every other. What an arm whose record cannot be
    /// read is treated as.
    #[must_use]
    pub fn everything() -> Invalidate {
        Invalidate::decode(TRIGGER_BIT | 0b11, 0, 0)
    }
}

/// The slot of `seq` in the arm ring.
#[inline]
fn ring_slot(seq: u64) -> usize {
    (seq % ARM_RING as u64) as usize
}

/// One arm's scope in one word: `valid(1) | tag(8) | scope bits(6) | aperture(1) | pdb page(48)`.
/// The tag is `seq / ARM_RING` (mod 256), so a slot that holds another arm's record is recognised.
fn pack_arm(seq: u64, inval: &Invalidate) -> u64 {
    let page = (inval.pdb >> PDB_ADDR_ALIGNMENT) & ((1 << 48) - 1);
    let ap = u64::from(inval.pdb_aperture == PdbAperture::Sysmem);
    1 | (((seq / ARM_RING as u64) & 0xFF) << 1)
        | (u64::from(inval.raw & 0x3F) << 9)
        | (ap << 15)
        | (page << 16)
}

/// The inverse of [`pack_arm`] for `seq`; `None` when the slot is empty or holds another arm.
fn unpack_arm(seq: u64, word: u64) -> Option<Invalidate> {
    if word & 1 == 0 || (word >> 1) & 0xFF != (seq / ARM_RING as u64) & 0xFF {
        return None;
    }
    let scope = ((word >> 9) & 0x3F) as u32;
    Some(Invalidate {
        raw: TRIGGER_BIT | scope,
        trigger: true,
        all_va: scope & 0b1 != 0,
        all_pdb: scope & 0b10 != 0,
        hubtlb_only: scope & 0b100 != 0,
        replay: (scope >> 3) & 0b111,
        pdb: (word >> 16) << PDB_ADDR_ALIGNMENT,
        pdb_aperture: if (word >> 15) & 1 == 1 {
            PdbAperture::Sysmem
        } else {
            PdbAperture::Vidmem
        },
    })
}

/// What [`InvalidatePort::armed_since`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArmedSince {
    /// The newest arm: the sequence the clear must name, with the scope recorded for it.
    pub newest: InvalidateRequest,
    /// Arms between the caller's last and `newest` (oldest first), each with its recorded scope
    /// (`ALL_PDB` when the record is gone). Each is superseded by `newest`; its SCOPE still has
    /// to be refreshed.
    pub earlier: Vec<InvalidateRequest>,
    /// More arms than the ring keeps were skipped: their scopes are unknown, refresh everything.
    pub overflow: bool,
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
    /// ★ What each recent arm was FOR, recorded by the arming vCPU before the arm is visible
    /// (see [`InvalidatePort::armed_since`]). One word per slot, so a reader never sees a torn
    /// record; slot `seq % ARM_RING`.
    ring: [AtomicU64; ARM_RING],
}

/// Arms whose scope is remembered. The manager reads them back by sequence; more than this many
/// unobserved arms (or a slot overwritten or not yet written) are answered by `ALL_PDB`, which
/// covers every scope — never by dropping one.
pub const ARM_RING: usize = 8;

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
        InvalidatePort {
            regs,
            pdb_lo: Cell::new(),
            pdb_hi: Cell::new(),
            word: Cell::new(),
            trigger: Trigger::new(),
            ring: [const { AtomicU64::new(0) }; ARM_RING],
        }
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
            self.word
                .apply(WriteSemantics::Plain, u64::from(value & !TRIGGER_BIT));
            let inval =
                Invalidate::decode(value, self.pdb_lo.read() as u32, self.pdb_hi.read() as u32);
            if !inval.trigger {
                return PortWrite::Latched;
            }
            // ★ ARM BEFORE PUBLISH — see `Trigger::arm_next`. ★ And RECORD BEFORE ARM: the scope
            // of THIS arm is taken from the latches the guest wrote for it, here, in its own
            // trap, and stored under its sequence before the arm is visible. The manager reads
            // the scope of every arm from the record, never from the latches, which the guest
            // may already be rewriting for its NEXT invalidate (a timed-out one, or a hostile
            // one): rebuilt from the latches, an arm could be redirected to another space.
            let seq = self.trigger.mint();
            self.ring[ring_slot(seq)].store(pack_arm(seq, &inval), Ordering::Release);
            self.trigger.arm_minted(seq);
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

    /// ★★ **Every arm the manager has not seen yet** — the newest, with its recorded scope, and the
    /// earlier ones it overwrote. `after` is the newest sequence the caller already took (`None`:
    /// none). `None` when nothing newer than `after` is armed.
    ///
    /// ⊘ Why not [`Self::armed_request`] alone: the trigger names only the NEWEST arm. An earlier
    /// arm that the manager never looked at (the guest's invalidate timed out and it armed again;
    /// or a hostile guest) would be lost, and the newest one rebuilt from the latches could even
    /// name the wrong space. Each arm's scope is recorded in the trap ([`Self::write`]); this reads
    /// them back. A record that is not there (not written yet, or overwritten by a newer arm), and
    /// every arm past the last [`ARM_RING`], is answered by `ALL_PDB`: a global refresh covers any
    /// scope, so the answer is conservative, never a loss.
    ///
    /// Lock-free: three atomic loads per arm; the caller is the VA manager thread.
    #[must_use]
    pub fn armed_since(&self, after: Option<u64>) -> Option<ArmedSince> {
        let newest = self.trigger.armed_seq()?;
        if after.is_some_and(|a| newest <= a) {
            return None;
        }
        let first = after.map_or(0, |a| a + 1);
        let window = (ARM_RING as u64) - 1;
        let lo = first.max(newest.saturating_sub(window));
        let read = |seq: u64| -> InvalidateRequest {
            let word = self.ring[ring_slot(seq)].load(Ordering::Acquire);
            InvalidateRequest {
                seq,
                inval: unpack_arm(seq, word).unwrap_or_else(Invalidate::everything),
            }
        };
        Some(ArmedSince {
            newest: read(newest),
            earlier: (lo..newest).map(read).collect(),
            overflow: lo > first,
        })
    }

    /// ★ Device reset (`docs/design/V3_DEVICE_RESET.md`): the three registers read as a fresh
    /// port's — latches zero, the trigger idle ([`Trigger::abandon`]). Returns whether a request
    /// was armed. ⊘ Only while no vCPU can write the port (the device's reset gate).
    pub fn reset(&self) -> bool {
        self.pdb_lo.apply(WriteSemantics::Plain, 0);
        self.pdb_hi.apply(WriteSemantics::Plain, 0);
        self.word.apply(WriteSemantics::Plain, 0);
        for slot in &self.ring {
            slot.store(0, Ordering::Release);
        }
        self.trigger.abandon()
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
            let busy = if self.trigger.read() != 0 {
                TRIGGER_BIT
            } else {
                0
            };
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

    /// ★ Device reset: a port left armed (the guest rebooted mid-invalidate) reads as a fresh
    /// port's, a late completion of the abandoned request clears nothing, and the next life's
    /// first invalidate arms and completes normally.
    #[test]
    fn a_reset_port_reads_idle_and_a_late_completion_of_the_old_request_is_harmless() {
        let regs = InvalidateRegs::from_usermode_base(0x00BB_0000).expect("regs");
        let fresh = InvalidatePort::new(regs);
        let p = InvalidatePort::new(regs);
        assert_eq!(p.write(regs.pdb, 0x1234_5000), PortWrite::Latched);
        assert_eq!(p.write(regs.upper_pdb, 0x7), PortWrite::Latched);
        let PortWrite::Publish(old) = p.write(regs.trigger, TRIGGER_BIT | 0x5) else {
            panic!("armed");
        };
        let all = [regs.pdb, regs.upper_pdb, regs.trigger];
        assert_ne!(all.map(|o| p.read(o)), all.map(|o| fresh.read(o)));
        assert!(p.reset());
        assert_eq!(all.map(|o| p.read(o)), all.map(|o| fresh.read(o)));
        assert_eq!(p.armed_request(), None);
        assert_eq!(p.trigger().issued(), p.trigger().completed(), "balanced");
        assert_eq!(p.trigger().complete(old.seq), ClearOutcome::AlreadyIdle);
        assert!(!p.reset(), "nothing armed the second time");
        let PortWrite::Publish(new) = p.write(regs.trigger, TRIGGER_BIT) else {
            panic!("armed");
        };
        assert_ne!(new.seq, old.seq, "the sequence never rewinds");
        assert_eq!(p.trigger().complete(new.seq), ClearOutcome::Cleared);
    }

    /// A usermode base of `0xBB_0000` puts the trigger at `0xB8_30B0` — the `bar0+0xb830b0` the
    /// old tree measured on GA106 (`kayfabe-device/src/mmuinval.rs`, w462).
    #[test]
    fn offsets_derive_from_the_usermode_base() {
        let r = InvalidateRegs::from_usermode_base(0xBB_0000).unwrap();
        assert_eq!(r.trigger, 0xB8_30B0);
        assert_eq!(r.pdb, r.trigger - 0x10);
        assert_eq!(r.upper_pdb, r.trigger - 0xC);
        assert_eq!(
            InvalidateRegs::from_usermode_base(0x1000),
            None,
            "below the PRIV delta: refused"
        );
    }

    #[test]
    fn pdb_round_trips_through_the_two_registers() {
        for pdb in [
            0u64,
            0x1000,
            0x0100_0000,
            0x3_FFFF_F000,
            0xFF_FFFF_F000_u64 & !0xFFF,
            0x1234_5678_9000,
        ] {
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
        let PortWrite::Publish(req) = p.write(r.trigger, TRIGGER_BIT | 1) else {
            panic!("no publish")
        };
        assert_eq!(req.inval.pdb, 0x0100_0000);
        assert_eq!(
            p.read(r.trigger).unwrap() & TRIGGER_BIT,
            TRIGGER_BIT,
            "busy while armed"
        );
        assert_eq!(p.trigger().complete(req.seq), ClearOutcome::Cleared);
        assert_eq!(
            p.read(r.trigger).unwrap() & TRIGGER_BIT,
            0,
            "idle after the clear"
        );
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
        let PortWrite::Publish(a) = p.write(r.trigger, 0x8001_0005) else {
            panic!()
        };
        assert_eq!(p.armed_request(), Some(a));
        assert_eq!(a.inval.pdb, 0x2_F339_2000);
        assert!(a.inval.all_va && a.inval.hubtlb_only && !a.inval.all_pdb);
        let PortWrite::Publish(b) = p.write(r.trigger, 0x8001_0005) else {
            panic!()
        };
        assert_eq!(
            p.armed_request().map(|x| x.seq),
            Some(b.seq),
            "the later arm"
        );
        assert_eq!(p.trigger().complete(b.seq), ClearOutcome::Cleared);
        assert_eq!(p.armed_request(), None);
    }

    /// §5.5's corruption: A times out in the guest, B is issued, A's work finishes. A's clear
    /// must be `Superseded` and B must still read busy.
    #[test]
    fn a_stale_completion_never_clears_a_later_trigger() {
        let p = port();
        let r = p.regs();
        let PortWrite::Publish(a) = p.write(r.trigger, TRIGGER_BIT | 1) else {
            panic!()
        };
        let PortWrite::Publish(b) = p.write(r.trigger, TRIGGER_BIT | 1) else {
            panic!()
        };
        assert!(b.seq > a.seq);
        assert_eq!(p.trigger().complete(a.seq), ClearOutcome::Superseded);
        assert_ne!(
            p.read(r.trigger).unwrap() & TRIGGER_BIT,
            0,
            "B is still pending"
        );
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

    /// One guest invalidate through the port: PDB latches, then the trigger.
    fn arm(p: &InvalidatePort, pdb: u64, ap: PdbAperture, word: u32) -> InvalidateRequest {
        let r = p.regs();
        let (lo, hi) = Invalidate::encode_pdb(pdb, ap);
        p.write(r.pdb, lo);
        p.write(r.upper_pdb, hi);
        let PortWrite::Publish(req) = p.write(r.trigger, TRIGGER_BIT | word) else {
            panic!("no publish")
        };
        req
    }

    /// ★ The scope of an arm is recorded in ITS trap: a guest that starts the next invalidate
    /// (writes other PDB latches, no trigger yet) cannot redirect the arm still waiting.
    #[test]
    fn an_arms_scope_is_its_own_not_whatever_the_latches_say_later() {
        let p = port();
        let a = arm(&p, 0x0100_0000, PdbAperture::Vidmem, 1);
        let (lo, hi) = Invalidate::encode_pdb(0x0900_0000, PdbAperture::Sysmem);
        p.write(p.regs().pdb, lo);
        p.write(p.regs().upper_pdb, hi);
        let got = p.armed_since(None).expect("armed");
        assert_eq!(got.newest.seq, a.seq);
        assert_eq!(got.newest.inval.pdb, 0x0100_0000, "A's own pdb");
        assert_eq!(got.newest.inval.pdb_aperture, PdbAperture::Vidmem);
        assert!(got.earlier.is_empty() && !got.overflow);
    }

    /// ★ Arms the manager never looked at are not lost: each comes back with its scope, oldest
    /// first, and the newest is the one that clears.
    #[test]
    fn every_overwritten_arm_comes_back_with_its_own_scope() {
        let p = port();
        let a = arm(&p, 0x0100_0000, PdbAperture::Vidmem, 1);
        let b = arm(&p, 0x0200_0000, PdbAperture::Sysmem, 1);
        let g = arm(&p, 0x0300_0000, PdbAperture::Vidmem, 0b11);
        let got = p.armed_since(None).expect("armed");
        assert_eq!(got.newest.seq, g.seq);
        assert!(got.newest.inval.all_pdb);
        let e: Vec<_> = got
            .earlier
            .iter()
            .map(|r| (r.seq, r.inval.pdb, r.inval.pdb_aperture, r.inval.all_pdb))
            .collect();
        assert_eq!(
            e,
            vec![
                (a.seq, 0x0100_0000, PdbAperture::Vidmem, false),
                (b.seq, 0x0200_0000, PdbAperture::Sysmem, false)
            ]
        );
        assert!(!got.overflow);
        // Having taken `a`, only the later ones are new.
        let got = p.armed_since(Some(a.seq)).expect("armed");
        assert_eq!(got.earlier.len(), 1);
        assert_eq!(got.earlier[0].seq, b.seq);
    }

    #[test]
    fn nothing_newer_than_the_last_taken_arm_is_none_and_idle_is_none() {
        let p = port();
        assert_eq!(p.armed_since(None), None, "idle");
        let a = arm(&p, 0x0100_0000, PdbAperture::Vidmem, 1);
        assert_eq!(p.armed_since(Some(a.seq)), None, "already taken");
        let b = arm(&p, 0x0200_0000, PdbAperture::Vidmem, 1);
        assert_eq!(
            p.armed_since(Some(a.seq)).map(|x| x.newest.seq),
            Some(b.seq)
        );
        assert_eq!(p.armed_since(Some(b.seq)), None, "already taken");
        assert_eq!(p.trigger().complete(b.seq), ClearOutcome::Cleared);
        assert_eq!(p.armed_since(None), None, "cleared");
    }

    /// More unobserved arms than the ring keeps: the overflow flag says refresh everything; the
    /// newest and the last `ARM_RING - 1` earlier ones still come back exactly.
    #[test]
    fn more_unobserved_arms_than_the_ring_keeps_set_the_overflow_flag() {
        let p = port();
        let mut seqs = Vec::new();
        for i in 0..20u64 {
            seqs.push(arm(&p, 0x0100_0000 + i * 0x1000, PdbAperture::Vidmem, 1).seq);
        }
        let got = p.armed_since(None).expect("armed");
        assert!(got.overflow);
        assert_eq!(got.newest.seq, 19);
        assert_eq!(got.earlier.len(), ARM_RING - 1);
        for r in got.earlier.iter().chain([&got.newest]) {
            assert_eq!(
                r.inval.pdb,
                0x0100_0000 + r.seq * 0x1000,
                "exact, not guessed"
            );
        }
        // Exactly the ring's worth of unobserved arms: no overflow.
        let got = p.armed_since(Some(19 - ARM_RING as u64)).expect("armed");
        assert!(!got.overflow);
        assert_eq!(got.earlier.len(), ARM_RING - 1);
        let got = p
            .armed_since(Some(19 - ARM_RING as u64 - 1))
            .expect("armed");
        assert!(got.overflow, "one more than the ring keeps");
    }

    /// A record that is not there (a vCPU minted a sequence and has not stored its scope; or the
    /// slot was reused) is answered by ALL_PDB, never by a guess from the latches.
    #[test]
    fn an_arm_without_a_readable_record_is_treated_as_all_pdb() {
        let p = port();
        for i in 0..9u64 {
            arm(&p, 0x0100_0000 + i * 0x1000, PdbAperture::Vidmem, 1);
        }
        // Sequence 9 armed with no record: its slot still holds sequence 1's.
        let s = p.trigger().mint();
        assert_eq!(s, 9);
        p.trigger().arm_minted(s);
        let got = p.armed_since(Some(7)).expect("armed");
        assert_eq!(got.newest.seq, 9);
        assert!(got.newest.inval.all_pdb, "unreadable => everything");
        assert_eq!(got.earlier.len(), 1);
        assert_eq!(got.earlier[0].inval.pdb, 0x0100_0000 + 8 * 0x1000);
        assert!(!got.earlier[0].inval.all_pdb);
    }

    /// A device reset empties the ring with the trigger.
    #[test]
    fn a_reset_forgets_every_recorded_arm() {
        let p = port();
        arm(&p, 0x0100_0000, PdbAperture::Vidmem, 1);
        arm(&p, 0x0200_0000, PdbAperture::Vidmem, 1);
        assert!(p.reset());
        assert_eq!(p.armed_since(None), None);
        // The next life: a fresh arm is the only one, and its scope is its own.
        let n = arm(&p, 0x0300_0000, PdbAperture::Vidmem, 1);
        let got = p.armed_since(Some(n.seq - 1)).expect("armed");
        assert!(got.earlier.is_empty() && !got.overflow);
        assert_eq!(got.newest.inval.pdb, 0x0300_0000);
    }

    #[test]
    fn a_packed_scope_round_trips() {
        for (pdb, ap, word) in [
            (0u64, PdbAperture::Vidmem, 1u32),
            (0x1000, PdbAperture::Sysmem, 0b11),
            (0x0123_4567_8000_0000, PdbAperture::Sysmem, 0b100_101),
            (
                0xFFFF_FFFF_F000_0000_u64 & !0xFFF,
                PdbAperture::Vidmem,
                0b111_111,
            ),
        ] {
            let (lo, hi) = Invalidate::encode_pdb(pdb, ap);
            let inval = Invalidate::decode(TRIGGER_BIT | word, lo, hi);
            for seq in [0u64, 7, 8, 12345] {
                let back = unpack_arm(seq, pack_arm(seq, &inval)).expect("same seq");
                assert_eq!(back.pdb, inval.pdb);
                assert_eq!(back.pdb_aperture, ap);
                assert_eq!(
                    (back.all_va, back.all_pdb, back.hubtlb_only, back.replay),
                    (inval.all_va, inval.all_pdb, inval.hubtlb_only, inval.replay)
                );
                assert_eq!(
                    unpack_arm(seq + 8, pack_arm(seq, &inval)),
                    None,
                    "other arm"
                );
            }
        }
        assert_eq!(unpack_arm(0, 0), None, "empty slot");
    }

    /// ★ Record BEFORE arm, with a real reader: one writer arms with a different directory each
    /// time, and a reader on another thread looks at the port as fast as it can. Whatever it
    /// sees armed is read back exactly - never an arm whose record is not there yet (which would
    /// come back as ALL_PDB). The writer waits for the reader after every arm, so no slot is ever
    /// reused under the reader and any inexact answer is the window between arm and record.
    #[test]
    fn a_reader_never_sees_an_arm_before_its_record() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU64, Ordering};
        const N: u64 = 150_000;
        let p = Arc::new(port());
        let acked = Arc::new(AtomicU64::new(0));
        let pdb_of = |seq: u64| 0x0100_0000 + (seq % 4096) * 0x1000;
        let reader = {
            let (p, acked) = (p.clone(), acked.clone());
            std::thread::spawn(move || {
                let mut after: Option<u64> = None;
                let mut inexact = 0u64;
                while acked.load(Ordering::Acquire) < N {
                    let Some(a) = p.armed_since(after) else {
                        std::hint::spin_loop();
                        continue;
                    };
                    for r in a.earlier.iter().chain([&a.newest]) {
                        if r.inval.all_pdb || r.inval.pdb != pdb_of(r.seq) {
                            inexact += 1;
                        }
                    }
                    after = Some(a.newest.seq);
                    acked.store(a.newest.seq + 1, Ordering::Release);
                }
                inexact
            })
        };
        for i in 0..N {
            let req = arm(&p, pdb_of(i), PdbAperture::Vidmem, 1);
            assert_eq!(req.seq, i);
            while acked.load(Ordering::Acquire) <= i {
                std::hint::spin_loop();
            }
        }
        let inexact = reader.join().unwrap();
        assert_eq!(
            inexact, 0,
            "{inexact} arms were visible before their record"
        );
    }
}
