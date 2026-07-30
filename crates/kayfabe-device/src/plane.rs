//! ★★★ **Stage Q4: the register plane.** One trapped base-address-register access in, one
//! value out.
//!
//! # What was missing, in one sentence
//!
//! `kayfabe_gsp::GspFsm` has had `mmio_read`/`mmio_write` since stage S3 and
//! `kayfabe_crec` has been driving them from a recorded trace for weeks — but a *guest*
//! could not reach them, because the hypervisor shim's register region returned a constant
//! and said so in its own comment. This module is the missing routing, and nothing else:
//! it decides which of four sources answers an offset and it holds the lock that makes the
//! FSM usable from more than one vCPU.
//!
//! # ★★ The four sources, in the order they are asked
//!
//! 1. **The chip's silicon constants** ([`crate::BootReg`]) — exact-offset, stateless.
//! 2. **The ROM window** — the synthetic VBIOS, generated from the same profile the
//!    device's PCI identity comes from.
//! 3. **The GSP register model** — anything [`kayfabe_arch::GspModel::decode_reg`] claims.
//! 4. **Nobody** — and this is where the interesting decision is, below.
//!
//! The order is safe because the three claimants are provably disjoint: `assert_disjoint`
//! checks it for a chip at construction, so a future row whose ROM window swallowed a GSP
//! register is a refusal at realize and not a value nobody can explain.
//!
//! # ★★★ An unclaimed register reads ZERO, and that is a decision with a cost
//!
//! `kayfabe_arch::GspModel::decode_reg` returning `None` means *"a different model owns
//! this offset"*, and plan §11-O1 — every non-GSP register — is still open. So this plane
//! is asked about thousands of offsets it has no model for, and it answers zero, exactly as
//! the C artifact does (`C: src/qemu/nvkvm_gpu_emul.c:1500-1504` and its comment
//! *"everything else reads back 0 for now"*).
//!
//! That is **not** MISS = FAULT and it is worth being honest about why. MISS = FAULT is the
//! address-table rule, where a miss means we would otherwise invent a translation. Here the
//! guest is reading a register a real chip does have; answering zero is a *wrong value*
//! rather than a *fabricated mapping*, and refusing instead would mean the device could not
//! be booted at all until every register in a 16 MiB aperture had a model. What this plane
//! owes instead is **visibility**: [`Counters::unclaimed_reads`] and
//! [`RegPlane::unclaimed_sample`] make "how much of this boot was answered by a defaulted
//! zero, and where" a number an operator can read, rather than a suspicion. When §11-O1
//! closes, this arm becomes a refusal and the counter is how anyone knows what that will
//! cost.
//!
//! # ★ Guest RAM is a NAMED REFUSAL at this stage, not silence
//!
//! [`kayfabe_gsp::GspFsm::mmio_write`] needs a [`GuestRam`] the moment a write is a queue
//! doorbell. Stage Q4 wires *registers*; the memory plane's realize is a separate object
//! with a separate lifetime, and joining them is stage Q5. So the plane is constructed with
//! [`RefusingRam`], every access is counted, and the fault the FSM produces is carried out
//! to the caller as a tag. A guest that rings the doorbell gets
//! `GspFault::GuestRam(RamRefused…)` reported by name — which is a diagnosis, where a
//! zero-filled read would have been a wrong answer nobody could see.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use kayfabe_arch::gsp::GspModel;
use kayfabe_gsp::{CommandPolicy, EchoOk, GspAbi, GspFault, GspFsm, GuestRam, RamRefused};
use kayfabe_trace::Faulted;

use crate::{ChipError, ChipProfile};

/// A [`GuestRam`] that refuses every access, by name.
///
/// ★ Not a stub that returns zeros. A zero-filled read of a message queue produces a
/// well-formed-looking element with a zero checksum, i.e. a *wrong answer the guest acts
/// on*; a refusal produces `GspFault::GuestRam`, which this plane counts and reports. The
/// difference is the whole reason this type exists rather than a `Vec<u8>`.
#[derive(Debug, Default, Clone, Copy)]
pub struct RefusingRam;

impl GuestRam for RefusingRam {
    fn read(&mut self, gpa: u64, buf: &mut [u8]) -> Result<(), RamRefused> {
        Err(RamRefused {
            gpa,
            len: buf.len(),
        })
    }

    fn write(&mut self, gpa: u64, bytes: &[u8]) -> Result<(), RamRefused> {
        Err(RamRefused {
            gpa,
            len: bytes.len(),
        })
    }
}

/// What a register access did, as numbers an acceptance test outside the process can read.
///
/// ★ Every field is a count of a *route taken*, so the four sources of §"the four sources"
/// are separately observable. A boot in which `gsp_reads` is zero looks identical to a boot
/// in which it is thousands, if all you have is "the guest hung".
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Counters {
    /// Register reads dispatched into this plane.
    pub reads: u64,
    /// Register writes dispatched into this plane.
    pub writes: u64,
    /// Reads answered from [`ChipProfile::boot_regs`].
    pub boot_reg_reads: u64,
    /// Reads answered from the ROM window.
    pub rom_reads: u64,
    /// Reads answered by the GSP register model.
    pub gsp_reads: u64,
    /// Writes the GSP register model claimed.
    pub gsp_writes: u64,
    /// ★ Reads no source claimed, answered with a defaulted zero. See the module docs for
    /// why this is a counter and not a refusal.
    pub unclaimed_reads: u64,
    /// Writes no source claimed, dropped.
    pub unclaimed_writes: u64,
    /// Faults the FSM raised on a write.
    pub faults: u64,
    /// Guest-RAM accesses the plane's RAM port refused.
    pub ram_refusals: u64,
    /// Times a write asked for the status-queue interrupt to be announced.
    pub irq_requests: u64,
}

/// The mutable half — everything that needs the lock.
struct PlaneState {
    fsm: GspFsm,
    ram: Box<dyn GuestRam>,
    policy: Box<dyn CommandPolicy>,
    /// The first unclaimed offsets seen, for diagnosis. Bounded, deliberately: an
    /// unbounded set is a guest-driven allocation, and a poller can produce millions.
    unclaimed: Vec<u64>,
}

/// How many distinct unclaimed offsets are remembered.
///
/// ★ Small and fixed. The counter says *how many*; this says *which*, and a guest that
/// polls one register a million times must not be able to grow it.
pub const UNCLAIMED_SAMPLE_MAX: usize = 64;

/// What one read produced, and which source produced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadOutcome {
    /// A chip constant.
    BootReg(u64),
    /// A byte of the ROM window.
    Rom(u64),
    /// The GSP register model's encoding for the FSM's current state.
    Gsp(u64),
    /// The GSP model claimed the offset and could not serve it.
    GspFault(&'static str),
    /// No source claimed the offset.
    Unclaimed,
}

impl ReadOutcome {
    /// The value the guest sees. An unclaimed or faulted register reads zero — see the
    /// module docs.
    #[must_use]
    pub fn value(self) -> u64 {
        match self {
            Self::BootReg(v) | Self::Rom(v) | Self::Gsp(v) => v,
            Self::GspFault(_) | Self::Unclaimed => 0,
        }
    }
}

/// What one write produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteOutcome {
    /// The GSP register model claimed the offset.
    pub claimed: bool,
    /// The fault the FSM raised, by tag, if it raised one.
    pub fault: Option<&'static str>,
    /// The status-queue interrupt should be announced to the guest.
    pub raise_status_irq: bool,
    /// How many transitions fired.
    pub transitions: usize,
    /// How many commands were decoded off the command queue.
    pub commands: usize,
}

/// ★★★ The register plane: the routing stage Q4 adds.
pub struct RegPlane {
    chip: &'static ChipProfile,
    model: Box<dyn GspModel>,
    rom: Vec<u8>,
    state: Mutex<PlaneState>,
    c: PlaneCounters,
}

impl core::fmt::Debug for RegPlane {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RegPlane")
            .field("chip", &self.chip.name)
            .field("rom_len", &self.rom.len())
            .field("counters", &self.counters())
            .finish_non_exhaustive()
    }
}

/// The counters, atomically. Separate from [`PlaneState`] so a read that takes the
/// FSM lock and one that does not both account, and so [`RegPlane::counters`] never
/// blocks behind a doorbell being serviced.
#[derive(Debug, Default)]
struct PlaneCounters {
    reads: AtomicU64,
    writes: AtomicU64,
    boot_reg_reads: AtomicU64,
    rom_reads: AtomicU64,
    gsp_reads: AtomicU64,
    gsp_writes: AtomicU64,
    unclaimed_reads: AtomicU64,
    unclaimed_writes: AtomicU64,
    faults: AtomicU64,
    ram_refusals: AtomicU64,
    irq_requests: AtomicU64,
}

impl RegPlane {
    /// Build a plane for one chip and one guest driver version.
    ///
    /// # Errors
    ///
    /// Whatever [`crate::rom_for`] refuses, or [`ChipError::OverlappingSources`] if the
    /// chip's own declarations put two answers over one offset.
    pub fn new(chip: &'static ChipProfile, abi: GspAbi) -> Result<RegPlane, ChipError> {
        let rom = crate::rom_for(chip)?;
        let model = (chip.gsp_model)();
        assert_disjoint(chip, model.as_ref())?;
        Ok(RegPlane {
            chip,
            model,
            rom,
            state: Mutex::new(PlaneState {
                fsm: GspFsm::new(abi),
                ram: Box::new(RefusingRam),
                policy: Box::new(EchoOk),
                unclaimed: Vec::new(),
            }),
            c: PlaneCounters::default(),
        })
    }

    /// The chip this plane answers as.
    #[must_use]
    pub fn chip(&self) -> &'static ChipProfile {
        self.chip
    }

    /// The ROM image the window serves, for a test that wants to check the bytes the guest
    /// would read against the generator's own output.
    #[must_use]
    pub fn rom(&self) -> &[u8] {
        &self.rom
    }

    /// Install a real guest-RAM port, replacing [`RefusingRam`].
    ///
    /// ★ The seam stage Q5 plugs into. It takes `&self` and the lock, so a plane already
    /// answering registers on one vCPU can acquire RAM without being rebuilt.
    pub fn set_ram(&self, ram: Box<dyn GuestRam>) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.ram = ram;
    }

    /// Install a command policy, replacing the C-baseline echo.
    pub fn set_policy(&self, policy: Box<dyn CommandPolicy>) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.policy = policy;
    }

    /// The counters.
    #[must_use]
    pub fn counters(&self) -> Counters {
        let g = |a: &AtomicU64| a.load(Ordering::Relaxed);
        Counters {
            reads: g(&self.c.reads),
            writes: g(&self.c.writes),
            boot_reg_reads: g(&self.c.boot_reg_reads),
            rom_reads: g(&self.c.rom_reads),
            gsp_reads: g(&self.c.gsp_reads),
            gsp_writes: g(&self.c.gsp_writes),
            unclaimed_reads: g(&self.c.unclaimed_reads),
            unclaimed_writes: g(&self.c.unclaimed_writes),
            faults: g(&self.c.faults),
            ram_refusals: g(&self.c.ram_refusals),
            irq_requests: g(&self.c.irq_requests),
        }
    }

    /// The distinct offsets no source claimed, up to [`UNCLAIMED_SAMPLE_MAX`].
    #[must_use]
    pub fn unclaimed_sample(&self) -> Vec<u64> {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.unclaimed.clone()
    }

    /// The FSM's current boot phase, so a test can assert the guest moved it.
    #[must_use]
    pub fn phase(&self) -> kayfabe_gsp::BootPhase {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.fsm.phase()
    }

    /// Power-on reset: rebuild the FSM. The RAM port and the policy survive, because they
    /// are the *shell's* wiring and not the device's state.
    pub fn device_reset(&self) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.fsm.device_reset();
    }

    /// ★★★ Serve one register read.
    ///
    /// `size` is the access width in bytes; the answer is masked to it, because a guest
    /// reading a byte of a dword register must not be handed the whole thing.
    pub fn read(&self, bar: u8, off: u64, size: u8) -> ReadOutcome {
        self.c.reads.fetch_add(1, Ordering::Relaxed);
        let out = self.read_inner(bar, off, size);
        match out {
            ReadOutcome::BootReg(_) => self.c.boot_reg_reads.fetch_add(1, Ordering::Relaxed),
            ReadOutcome::Rom(_) => self.c.rom_reads.fetch_add(1, Ordering::Relaxed),
            ReadOutcome::Gsp(_) => self.c.gsp_reads.fetch_add(1, Ordering::Relaxed),
            ReadOutcome::GspFault(_) => self.c.faults.fetch_add(1, Ordering::Relaxed),
            ReadOutcome::Unclaimed => {
                self.note_unclaimed(off);
                self.c.unclaimed_reads.fetch_add(1, Ordering::Relaxed)
            }
        };
        out
    }

    fn read_inner(&self, bar: u8, off: u64, size: u8) -> ReadOutcome {
        if bar == 0 {
            if let Some(r) = self.chip.boot_regs.iter().find(|r| r.off == off) {
                return ReadOutcome::BootReg(mask(u64::from(r.value), size));
            }
            if self.chip.rom_window.contains(off) {
                return ReadOutcome::Rom(self.rom_read(off - self.chip.rom_window.base, size));
            }
        }
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        match s.fsm.mmio_read_with(self.model.as_ref(), bar, off) {
            None => ReadOutcome::Unclaimed,
            Some(Ok(v)) => ReadOutcome::Gsp(mask(v, size)),
            Some(Err(f)) => ReadOutcome::GspFault(f.fault_tag().0),
        }
    }

    /// Read `size` bytes out of the ROM image, little-endian.
    ///
    /// ★ Past the end of the image reads zero, exactly as an unprogrammed EEPROM would.
    /// The driver never *needs* those bytes: it computes `biosSize` from the image's own
    /// PCIR/NPDE block count and confines itself to that.
    fn rom_read(&self, off: u64, size: u8) -> u64 {
        let mut v: u64 = 0;
        for i in 0..u64::from(size.min(8)) {
            let b = off.saturating_add(i);
            let byte = usize::try_from(b)
                .ok()
                .and_then(|i| self.rom.get(i))
                .copied()
                .unwrap_or(0);
            v |= u64::from(byte) << (8 * i);
        }
        v
    }

    /// ★★★ Serve one register write.
    pub fn write(&self, bar: u8, off: u64, size: u8, val: u64) -> WriteOutcome {
        let _ = size;
        self.c.writes.fetch_add(1, Ordering::Relaxed);
        let claimed = self.model.decode_reg(bar, off).is_some();
        if !claimed {
            self.note_unclaimed(off);
            self.c.unclaimed_writes.fetch_add(1, Ordering::Relaxed);
            return WriteOutcome {
                claimed: false,
                fault: None,
                raise_status_irq: false,
                transitions: 0,
                commands: 0,
            };
        }
        self.c.gsp_writes.fetch_add(1, Ordering::Relaxed);
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let PlaneState {
            fsm, ram, policy, ..
        } = &mut *s;
        match fsm.mmio_write_with(
            ram.as_mut(),
            self.model.as_ref(),
            policy.as_mut(),
            bar,
            off,
            val,
        ) {
            Ok(report) => {
                if report.raise_status_irq {
                    self.c.irq_requests.fetch_add(1, Ordering::Relaxed);
                }
                WriteOutcome {
                    claimed: true,
                    fault: None,
                    raise_status_irq: report.raise_status_irq,
                    transitions: report.transitions.len(),
                    commands: report.commands.len(),
                }
            }
            Err(f) => {
                self.c.faults.fetch_add(1, Ordering::Relaxed);
                if matches!(f, GspFault::GuestRam(_)) {
                    self.c.ram_refusals.fetch_add(1, Ordering::Relaxed);
                }
                WriteOutcome {
                    claimed: true,
                    fault: Some(f.fault_tag().0),
                    raise_status_irq: false,
                    transitions: 0,
                    commands: 0,
                }
            }
        }
    }

    fn note_unclaimed(&self, off: u64) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.unclaimed.len() < UNCLAIMED_SAMPLE_MAX && !s.unclaimed.contains(&off) {
            s.unclaimed.push(off);
        }
    }
}

/// Mask a value to an access width. A width of 8 or more is the whole value.
fn mask(v: u64, size: u8) -> u64 {
    match size {
        1 => v & 0xFF,
        2 => v & 0xFFFF,
        4 => v & 0xFFFF_FFFF,
        _ => v,
    }
}

/// ★★ Prove a chip's three read sources are disjoint, at construction.
///
/// The read path asks them in a fixed order, so an overlap would be resolved *silently* by
/// that order — a GSP register swallowed by a ROM window would read as ROM bytes forever
/// and the FSM would simply never be consulted. A chip row is data, and data can be wrong;
/// this is the check that makes a wrong row a refusal at realize.
///
/// The ROM window is checked against the GSP model by asking the model to decode every
/// 4-byte-aligned offset in the window. That is 262 144 calls for a 1 MiB window, once, at
/// realize — cheap enough to be exhaustive, and being exhaustive is the point: a sampled
/// check would pass a row whose single overlapping register sat between samples.
fn assert_disjoint(chip: &ChipProfile, model: &dyn GspModel) -> Result<(), ChipError> {
    for r in chip.boot_regs {
        if chip.rom_window.contains(r.off) {
            return Err(ChipError::OverlappingSources {
                off: r.off,
                a: "a silicon-constant register",
                b: "the ROM window",
            });
        }
        if model.decode_reg(0, r.off).is_some() {
            return Err(ChipError::OverlappingSources {
                off: r.off,
                a: "a silicon-constant register",
                b: "the GSP register model",
            });
        }
        if r.off >= chip.regs_aperture_len {
            return Err(ChipError::OutsideAperture {
                off: r.off,
                aperture: chip.regs_aperture_len,
            });
        }
    }
    if chip.rom_window.base.saturating_add(chip.rom_window.len) > chip.regs_aperture_len {
        return Err(ChipError::OutsideAperture {
            off: chip.rom_window.base,
            aperture: chip.regs_aperture_len,
        });
    }
    let mut off = chip.rom_window.base;
    let end = chip.rom_window.base + chip.rom_window.len;
    while off < end {
        if model.decode_reg(0, off).is_some() {
            return Err(ChipError::OverlappingSources {
                off,
                a: "the ROM window",
                b: "the GSP register model",
            });
        }
        off += 4;
    }
    Ok(())
}

kayfabe_util::assert_send_sync!(RegPlane);
kayfabe_util::assert_send_sync!(RefusingRam);
