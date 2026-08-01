//! ★★★ **Stage Q4: the register plane.** One trapped base-address-register access in, one
//! value out.
//!
//! # What was missing, in one sentence
//!
//! `kayfabe_gsp::GspFsm` has had `mmio_read`/`mmio_write` since stage S3 and
//! `kayfabe_crec` has been driving them from a recorded trace for weeks — but a *guest*
//! could not reach them, because the hypervisor shim's register region returned a constant
//! and said so in its own comment. This module is the missing routing, and nothing else:
//! it decides which of five sources answers an offset and it holds the lock that makes the
//! FSM usable from more than one vCPU.
//!
//! # ★★ The five sources, in the order they are asked
//!
//! 1. **The chip's silicon constants** ([`crate::BootReg`]) — exact-offset, stateless.
//! 2. **The free-running nanosecond counter** ([`crate::PtimerRegs`]) — the only source
//!    whose answer is a function of neither the chip nor the FSM. See below.
//! 3. **The ROM window** — the synthetic VBIOS, generated from the same profile the
//!    device's PCI identity comes from.
//! 4. **The GSP register model** — anything [`kayfabe_arch::GspModel::decode_reg`] claims.
//! 5. **Nobody** — and this is where the interesting decision is, below.
//!
//! The order is safe because the four claimants are provably disjoint (`assert_disjoint`): `assert_disjoint`
//! checks it for a chip at construction, so a future row whose ROM window swallowed a GSP
//! register is a refusal at realize and not a value nobody can explain.
//!
//! # ★★★ A STOPPED CLOCK IS AN UNKILLABLE HANG, WHICH IS WHY THE CLOCK IS A CONSTRUCTOR
//! ARGUMENT
//!
//! The driver's every bounded wait — falcon reset-ready, memory scrubbing, DMA idle, halt,
//! the message-queue polls — is a bare loop whose only exit besides success is
//! `gpuCheckTimeout`, and on this generation `gpuCheckTimeout` reads the GPU's own
//! nanosecond counter through the virtual-function aperture
//! (`ogkm-580: src/nvidia/src/kernel/gpu/timer/arch/turing/timer_tu102.c:130-155`, whose
//! `tmrReadTimeLoReg_TU102`/`tmrReadTimeHiReg_TU102` read
//! `NV_VIRTUAL_FUNCTION_TIME_0`/`_TIME_1`). Answer that counter with a constant and *every*
//! one of those loops becomes unbounded: the driver spins in kernel context, uninterruptible
//! by any signal, and prints nothing at all, because the print only exists on the timeout
//! arm it can never reach. One example, in full, is
//! `kflcnPreResetWait_GA102`
//! (`ogkm-580: src/nvidia/src/kernel/gpu/falcon/arch/ampere/kernel_falcon_ga102.c:212-224`).
//!
//! So [`NanoClock`] is **not** a port with a default. `set_ram`/`set_policy` can default,
//! because [`RefusingRam`] refuses *by name* and a refusal is a diagnosis. There is no
//! refusing answer for a register read — a wrong number is the only thing a reader can be
//! handed — so the decision is moved to where it cannot be skipped: [`RegPlane::new`] takes
//! the clock, and a caller that has not thought about time does not compile.
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
//! # ★★ Guest RAM is a NAMED REFUSAL — before stage Q5 *and after it*
//!
//! [`kayfabe_gsp::GspFsm::mmio_write`] needs a [`GuestRam`] the moment a write is a queue
//! doorbell. Stage Q4 wired *registers* only; the memory plane's realize is a separate
//! object with a separate lifetime, and joining them is stage **Q5**, which is
//! [`RegPlane::set_ram`].
//!
//! The construction default is still [`RefusingRam`] and that has not moved, because the
//! default is what a shell that forgot to wire gets, and [`NO_RAM_PORT`] says exactly that
//! in one sentence. What Q5 changes is only *which* implementation a realized device
//! carries — and the replacement is a refuser too: the adapter's port resolves through the
//! guest-physical region map, which proves a range is memory before touching it and
//! refuses everything else **by name**. So at both stages the answer to an address nothing
//! backs is `GspFault::GuestRam(RamRefused…)`, carried to the caller with its address, its
//! length and its reason ([`WriteOutcome::ram_refusal`]).
//!
//! That is the property to preserve, and it is worth stating why in the negative: a
//! zero-filled read of a message queue produces a well-formed-looking element with a zero
//! checksum — a *wrong answer the guest acts on* — and the closest cautionary case in this
//! tree is the free-running counter above, where a defaulted zero was a plausible answer
//! and therefore an unkillable silent spin. A refusal is never plausible, which is exactly
//! what makes it a diagnosis.
//!
//! # ★★★ A FRAMEBUFFER WINDOW IS NOT AN UNCLAIMED REGISTER (`#102` stage C)
//!
//! The "unclaimed reads zero" argument two sections up turns on the offset naming a
//! *register*. Three of the offsets this plane is handed do not: the `PRAMIN` window inside
//! the register aperture, the framebuffer aperture and the instance/`BAR2` window are
//! **device memory**, and a page table lives in device memory. So they are classified
//! before the unclaimed arm and counted under their own name
//! ([`crate::FbWindow`], [`Counters::fb_window_reads`], [`RegPlane::fb_window_sample`]).
//!
//! ⊘ **Nothing here serves a framebuffer byte, and nothing here decides where the
//! framebuffer should live** — that is plan §11-O1 and `eight_blockers_resolved.md` §12,
//! which put the bytes in the isolate. This arm exists so that the absence has a name.
//!
//! ★★ It is worth the type because the volume is not small and was not visible.
//! Bucketing every base-address-register write in the committed C reference traces by
//! region (`nvidia-gpu-passthrough/traces/mode2_c_reference/`, decoded 2026-07-31):
//! **211 836** of the cold boot's and **250 041** of the matmul's land in one of these three
//! windows — and under the previous code every one was indistinguishable from an unknown
//! register offset. See [`crate::FbWindow`] for the per-window split.
//!
//! # ★★★ AND ONE OF THE THREE IS NOW SERVED — `PRAMIN`, THE BAR0 MOVING WINDOW (`#146`)
//!
//! The paragraph above was written when **no** framebuffer byte was served and the arm
//! existed so that the absence had a name. That is still true of two of the three windows.
//! It is no longer true of `PRAMIN`: [`crate::fbwin`] gives it a real address model
//! ([`Bar0Window`], the `NV_PBUS_BAR0_WINDOW` latch) and a real byte store
//! ([`FbStore`]), because the boot of 2026-08-01 stopped at `kbusVerifyBar2_GM107`, whose
//! first sub-test is a plain **dword write-then-read through `PRAMIN`** with no BAR2 and no
//! MMU anywhere in it.
//!
//! The other two are unchanged and still refuse, and the difference is not effort: the
//! framebuffer aperture and the instance window are **GMMU-translated**, so serving them
//! needs the page-table format `kayfabe_chips::UnbuiltGmmu` says this port does not have.
//! `PRAMIN` is **untranslated** — the framebuffer address *is* the window base plus the
//! offset — so it needs no MMU at all. ⊘ That asymmetry is the whole of why one window
//! could be built and two could not, and it is why serving this one did not turn
//! `the_shipped_arch_refuses_every_data_plane_seam` red.
//!
//! ★★ The three counters that used to describe every window
//! ([`Counters::fb_window_reads`], [`Counters::fb_window_writes`]) now describe **only the
//! unserved ones**, and three new counters describe this one
//! ([`Counters::fb_reads`], [`Counters::fb_writes`], [`Counters::fb_refusals`]). Merging
//! them would have hidden exactly the fact that matters: *how many framebuffer accesses
//! this boot dropped on the floor*.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use kayfabe_arch::gsp::GspModel;
use kayfabe_gsp::{CommandPolicy, GspAbi, GspFault, GspFsm, GuestRam, RamRefused};
use kayfabe_trace::Faulted;

use crate::fbwin::{Bar0Window, FbRefused, FbStore, RefusingFb};
use crate::{ChipError, ChipProfile, FbWindow};

/// A [`GuestRam`] that refuses every access, by name.
///
/// ★ Not a stub that returns zeros. A zero-filled read of a message queue produces a
/// well-formed-looking element with a zero checksum, i.e. a *wrong answer the guest acts
/// on*; a refusal produces `GspFault::GuestRam`, which this plane counts and reports. The
/// difference is the whole reason this type exists rather than a `Vec<u8>`.
#[derive(Debug, Default, Clone, Copy)]
pub struct RefusingRam;

/// [`RefusingRam`]'s one sentence, as [`RamRefused::why`].
///
/// ★ It names the *absence*, not a failure: a plane whose shell never called
/// [`RegPlane::set_ram`] has no memory to reach, and a reader who sees this has a wiring
/// question, not a guest-behaviour question. Those two are the near neighbours here.
pub const NO_RAM_PORT: &str = "the register plane has no guest-RAM port installed; the shell never called set_ram, \
     so this device cannot reach guest memory at all";

impl GuestRam for RefusingRam {
    fn read(&mut self, gpa: u64, buf: &mut [u8]) -> Result<(), RamRefused> {
        Err(RamRefused {
            gpa,
            len: buf.len(),
            why: NO_RAM_PORT,
        })
    }

    fn write(&mut self, gpa: u64, bytes: &[u8]) -> Result<(), RamRefused> {
        Err(RamRefused {
            gpa,
            len: bytes.len(),
            why: NO_RAM_PORT,
        })
    }
}

/// ★★★ The device's free-running nanosecond counter, as a port.
///
/// The one thing this plane serves that is a function of neither the chip nor the boot
/// state machine. See the module docs for why it is a constructor argument rather than a
/// settable port with a default.
///
/// The contract is narrow and all of it is load-bearing:
///
/// - **Monotonic non-decreasing.** The driver reads the counter as high / low / high and
///   retries when the high half moved; a counter that went backwards would make an elapsed
///   time negative and a timeout fire immediately or never.
/// - **Advancing.** Two calls separated by real work must not return the same value
///   forever. This is the whole point of the type.
/// - **Nanoseconds.** The driver's timeouts are in microseconds and it converts by
///   dividing, so the *unit* is part of the contract even though nothing can check it.
pub trait NanoClock: Send + Sync + core::fmt::Debug {
    /// Nanoseconds since an arbitrary, fixed origin.
    fn now_ns(&self) -> u64;
}

/// A [`NanoClock`] that advances a fixed amount per reading, from zero.
///
/// ★ For tests and for any caller that must be reproducible: it makes the counter a pure
/// function of *how many times it has been read*, so a replay produces bit-identical
/// values. It is deliberately **not** the default for a live device — the driver's timeouts
/// are wall-clock quantities, and a clock whose rate depends on how often the guest happens
/// to poll turns a 4-second timeout into an unpredictable number of iterations.
#[derive(Debug)]
pub struct SteppingClock {
    step_ns: u64,
    now: AtomicU64,
}

impl SteppingClock {
    /// A clock that advances `step_ns` nanoseconds per reading.
    #[must_use]
    pub fn new(step_ns: u64) -> SteppingClock {
        SteppingClock {
            step_ns,
            now: AtomicU64::new(0),
        }
    }
}

impl NanoClock for SteppingClock {
    fn now_ns(&self) -> u64 {
        self.now.fetch_add(self.step_ns, Ordering::Relaxed)
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
    /// Reads answered from the free-running nanosecond counter.
    pub ptimer_reads: u64,
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
    /// ★★★ Reads that landed in a framebuffer window ([`crate::FbWindow`]) this port has
    /// **no address model for** — the GMMU-translated ones. **Not** a subset of
    /// [`Counters::unclaimed_reads`]: an access is classified into exactly one of the two,
    /// because the whole point is that they are different facts.
    ///
    /// ⚠ Since `#146` this excludes `PRAMIN`, which is served — see
    /// [`Counters::fb_reads`].
    pub fb_window_reads: u64,
    /// Writes that landed in an unmodelled framebuffer window and were therefore
    /// **dropped**.
    ///
    /// ★★ The one to watch. A dropped framebuffer write can be a dropped page-table entry,
    /// which does not fail here — it fails much later, as a mapping that is simply absent.
    pub fb_window_writes: u64,
    /// ★★★ Reads **served** from the device's framebuffer through the BAR0 moving window.
    pub fb_reads: u64,
    /// ★★★ Writes **landed** in the device's framebuffer through the BAR0 moving window.
    ///
    /// ★ "Landed", not "attempted": this counter is incremented only after
    /// [`FbStore::write`] returned `Ok`, so `fb_writes + fb_refusals == attempts` and
    /// neither number can absorb the other. A port that counted attempts would report a
    /// healthy boot while dropping every byte.
    pub fb_writes: u64,
    /// ★★★ Framebuffer accesses the store **refused, by name** — see
    /// [`crate::fbwin::FbRefused`].
    ///
    /// ⊘ This is what a dropped framebuffer write looks like now: a number that moves, a
    /// sentence, and a physical address, at the instant it happens. Before `#146` it looked
    /// like nothing at all until `kbusVerifyBar2` reported `NV_ERR_MEMORY_ERROR` hundreds
    /// of operations later.
    pub fb_refusals: u64,
    /// ★★ Reads of `NV_PBUS_BAR0_WINDOW` itself — the guest asking where its own window
    /// points.
    ///
    /// ★ Worth a counter of its own because a boot in which this is **zero** and
    /// [`Counters::fb_reads`] is large means the guest never checked, which is the
    /// condition under which a dropped window write goes unnoticed. RM does both: it
    /// read-modify-writes the register twice per re-point and refreshes its own cache from
    /// it (`ogkm-580: kern_bus_gm107.c:4738-4741`).
    pub bar0_window_reads: u64,
    /// Writes to `NV_PBUS_BAR0_WINDOW` — the guest re-pointing its window.
    pub bar0_window_writes: u64,
    /// Faults the FSM raised on a write.
    pub faults: u64,
    /// Guest-RAM accesses the plane's RAM port refused.
    pub ram_refusals: u64,
    /// Times a write asked for the status-queue interrupt to be announced.
    pub irq_requests: u64,
    /// Commands decoded off the guest's command queue.
    pub commands: u64,
    /// ★★ Of those, the ones **no policy answered** — refused by name by the FSM. See
    /// [`crate::unserviced`]: the refusal is quiet in the guest, so this and
    /// [`RegPlane::unserviced_sample`] are the only place the list is readable.
    pub commands_unserviced: u64,
}

/// ★★★ **One device life's residue, whole.** What [`RegPlane::residue`] answers; see that
/// method for why the enumeration is a compiler obligation and not a list.
///
/// ★ Every member has derived equality, so comparing two lives is `==` on one value. The
/// two members [`RegPlane`] holds that *cannot* have equality — the guest-RAM port and the
/// command policy, both `Box<dyn …>` — are absent by decision rather than by oversight, and
/// [`RegPlane::residue`]'s docs say which and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaneResidue {
    /// Every route counter, as [`RegPlane::counters`] reports them.
    pub counters: Counters,
    /// ★★ The emulated GSP, **whole**: phase, queue binding, both ring cursors, both
    /// sequence numbers, the mailbox shadows, the boot-args conjunction, the region
    /// identity and both latches. `PartialEq` is derived over every field it has.
    pub gsp: GspFsm,
    /// The bounded sample of `(bar, offset)` pairs no source claimed.
    pub unclaimed: Vec<(u8, u64)>,
    /// ★ The bounded sample of framebuffer-window accesses. Added to the residue in
    /// `#130`: it is the exact twin of `unclaimed` and had been outside every snapshot
    /// since it was written.
    pub fb_window: Vec<(FbWindow, u64)>,
    /// The distinct commands no policy answered.
    pub unserviced: Vec<crate::unserviced::UnservicedCommand>,
    /// ★ The replayable-fault-buffer registrations the guest asked for and this port
    /// declined. Guest-driven, and — like `fb_window` — in no snapshot before `#130`.
    pub fault_buffers: Vec<crate::faultbuffer::FaultBufferNote>,
    /// How many such registrations arrived in total, including repeats and anything past
    /// the sample bound. A count and a sample fail differently: a sample that saturates
    /// stops moving, and the count does not.
    pub fault_buffers_registered: u64,
    /// ★★★ The BAR0 moving window's register, as the guest last wrote it.
    ///
    /// Device state, so it is in the residue: a reloaded device whose window still pointed
    /// where the previous guest left it is not indistinguishable from a cold one, and the
    /// register's reset value is a documented silicon fact (`_BASE_0`, `_TARGET_VID_MEM`).
    pub bar0_window: Bar0Window,
    /// ★★★ How many bytes of framebuffer the store is holding.
    ///
    /// ⊘ Content, not identity — this is a *size*, and it is here because the alternative
    /// was leaving device memory out of the residue entirely. A device life that ended
    /// holding the previous guest's page tables and instance blocks is exactly what
    /// [`FbStore::device_reset`] exists to prevent, and a property quantified over "all of
    /// the device's state" that silently skipped its **memory** would be `#130`'s own
    /// shrinking-universe failure.
    pub fb_resident_bytes: u64,
}

/// The mutable half — everything that needs the lock.
struct PlaneState {
    fsm: GspFsm,
    ram: Box<dyn GuestRam>,
    policy: Box<dyn CommandPolicy>,
    /// The first unclaimed accesses seen, as `(bar, offset)`, for diagnosis. Bounded,
    /// deliberately: an unbounded set is a guest-driven allocation, and a poller can produce
    /// millions.
    ///
    /// ★ The **bar** travels with the offset, and that is a fix rather than a decoration:
    /// offset `0x9008c` in the framebuffer aperture and offset `0x9008c` in the register
    /// aperture are the same number and different facts, and a sample that reported only the
    /// number said the second when it meant the first.
    unclaimed: Vec<(u8, u64)>,
    /// The first framebuffer-window accesses seen, as `(window, offset)`. Bounded for the
    /// same reason and by the same constant.
    fb_window: Vec<(FbWindow, u64)>,
    /// ★★★ The BAR0 moving window's register. **Under the same lock as everything the
    /// window resolves**, so a read and a write on two vCPUs cannot observe half of a
    /// re-point.
    bar0_window: Bar0Window,
    /// The framebuffer this device advertises, as a port. [`RefusingFb`] until a shell
    /// installs one — the exact shape [`PlaneState::ram`] already has.
    fb: Box<dyn FbStore>,
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
    /// One half of the free-running nanosecond counter.
    Ptimer(u64),
    /// A byte of the ROM window.
    Rom(u64),
    /// The GSP register model's encoding for the FSM's current state.
    Gsp(u64),
    /// The GSP model claimed the offset and could not serve it.
    GspFault(&'static str),
    /// ★★★ The offset is inside a framebuffer window — **device memory**, which this port
    /// does not model. Reads zero like [`ReadOutcome::Unclaimed`] does and is a separate
    /// variant precisely because the two mean different things: see the module docs.
    ///
    /// ⚠ Since `#146` this is the answer for the **GMMU-translated** windows only; the
    /// BAR0 moving window answers [`ReadOutcome::Fb`].
    FbWindow(FbWindow),
    /// ★★★ The BAR0 moving window's register itself, `NV_PBUS_BAR0_WINDOW` — the last
    /// word the guest wrote there.
    ///
    /// A variant of its own rather than a [`ReadOutcome::BootReg`], because a boot register
    /// is a *constant of the silicon* and this one is a latch the guest owns. See
    /// [`Bar0Window`] for why answering it with anything other than the guest's own word
    /// breaks the guest's read-modify-write.
    Bar0Window(u64),
    /// ★★★ Framebuffer bytes, served through the BAR0 moving window.
    Fb {
        /// Which window (always [`FbWindow::Pramin`] today).
        window: FbWindow,
        /// The framebuffer-physical address the window resolved the offset to.
        phys: u64,
        /// The bytes, as a little-endian value masked to the access width.
        value: u64,
    },
    /// ★★★ The framebuffer store **refused** the resolved address, by name.
    ///
    /// ⊘ It still reads zero, because a trapped register read has no error channel to the
    /// guest — but it is a *different variant*, counted apart ([`Counters::fb_refusals`])
    /// and sampled, so an operator is never left inferring a dropped access from a boot
    /// that failed somewhere else.
    FbRefused {
        /// Which window.
        window: FbWindow,
        /// The address the window resolved to.
        phys: u64,
        /// Why the store would not serve it.
        why: &'static str,
    },
    /// No source claimed the offset.
    Unclaimed,
}

impl ReadOutcome {
    /// The value the guest sees. An unclaimed or faulted register reads zero — see the
    /// module docs.
    #[must_use]
    pub fn value(self) -> u64 {
        match self {
            Self::BootReg(v)
            | Self::Ptimer(v)
            | Self::Rom(v)
            | Self::Gsp(v)
            | Self::Bar0Window(v) => v,
            Self::Fb { value, .. } => value,
            Self::GspFault(_) | Self::FbWindow(_) | Self::FbRefused { .. } | Self::Unclaimed => 0,
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
    /// ★★ The guest-RAM refusal behind a `GspFault::GuestRam`, kept **whole**.
    ///
    /// The tag alone says *which kind* of fault; this says *which address*, *how many
    /// bytes* and *why the port would not serve it*. Stage Q4 shipped without it because
    /// there was exactly one reason a RAM access could fail (there was no RAM port); the
    /// moment a real port was wired in there were several, and a diagnosis that cannot
    /// distinguish them costs a boot each time.
    pub ram_refusal: Option<RamRefused>,
    /// ★★★ The write landed in a framebuffer window this port has no address model for
    /// and was dropped, with the window named. `None` means it did not.
    ///
    /// ⚠ Since `#146` the BAR0 moving window is never reported here — it either lands
    /// ([`WriteOutcome::fb_landed`]) or refuses ([`WriteOutcome::fb_refusal`]). ⊘ **There
    /// is deliberately no third answer**: *"framebuffer write, dropped, success"* is the
    /// shape this whole rung exists to make unrepresentable.
    pub fb_window: Option<FbWindow>,
    /// ★★★ The write **landed** in the device's framebuffer, at this physical address.
    ///
    /// Set only after [`FbStore::write`] returned `Ok`.
    pub fb_landed: Option<u64>,
    /// ★★★ The framebuffer store **refused** the write, whole — address, length, reason.
    ///
    /// ★ Kept as the payload rather than as a tag for the same reason
    /// [`WriteOutcome::ram_refusal`] is: the sentence and the address are the diagnosis,
    /// and a shell that could only report *that* one happened would cost a boot to tell
    /// "the shell forgot to install the store" from "the guest pointed the window past the
    /// end of its own framebuffer".
    pub fb_refusal: Option<FbRefused>,
    /// The status-queue interrupt should be announced to the guest.
    pub raise_status_irq: bool,
    /// How many transitions fired.
    pub transitions: usize,
    /// How many commands were decoded off the command queue.
    pub commands: usize,
}

impl WriteOutcome {
    /// A write that claimed nothing, faulted nowhere and moved no byte.
    ///
    /// ★ The base every arm of the write path builds on, so that **adding a field to
    /// [`WriteOutcome`] is one edit rather than six** — and, more to the point, so that the
    /// field cannot be silently omitted from one arm and set in the other five. That shape
    /// is how a "dropped" arm and a "landed" arm come to differ in a way nobody notices.
    #[must_use]
    pub fn nothing() -> WriteOutcome {
        WriteOutcome {
            claimed: false,
            fault: None,
            ram_refusal: None,
            fb_window: None,
            fb_landed: None,
            fb_refusal: None,
            raise_status_irq: false,
            transitions: 0,
            commands: 0,
        }
    }
}

/// The fault tag a refused framebuffer write carries out to the shell.
///
/// ★ A sentence rather than a name, because the shell prints it verbatim beside the
/// register offset and the reader is an operator staring at one line of a boot log.
pub const FB_WRITE_REFUSED: &str = "the framebuffer store would not take a write through the BAR0 moving window; the \
     bytes did NOT land, and the guest will not be told";

/// ★★★ The register plane: the routing stage Q4 adds.
pub struct RegPlane {
    chip: &'static ChipProfile,
    model: Box<dyn GspModel>,
    rom: Vec<u8>,
    /// ★ Outside the [`Mutex`], and that is the point: the guest reads this counter in the
    /// inner loop of every timeout it has, millions of times per boot, and putting it
    /// behind the FSM's lock would serialize the whole thing behind a doorbell being
    /// serviced. [`NanoClock`] takes `&self` so it needs no lock of ours.
    clock: Box<dyn NanoClock>,
    state: Mutex<PlaneState>,
    c: PlaneCounters,
    /// ★★ The list of commands nothing answered. Held here as well as inside the chain's
    /// terminal link because a caller that replaces the policy with
    /// [`RegPlane::set_policy`] still gets to read what the default one recorded — and
    /// because reading it must not take the FSM's lock behind a doorbell.
    unserviced: crate::unserviced::UnservicedLog,
    /// ★ Step 5a's whole deliverable: where the guest said its replayable fault buffer is
    /// (`crate::faultbuffer`). Recorded, never answered.
    fault_buffer: crate::faultbuffer::FaultBufferLog,
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
    ptimer_reads: AtomicU64,
    rom_reads: AtomicU64,
    gsp_reads: AtomicU64,
    gsp_writes: AtomicU64,
    unclaimed_reads: AtomicU64,
    unclaimed_writes: AtomicU64,
    fb_window_reads: AtomicU64,
    fb_window_writes: AtomicU64,
    fb_reads: AtomicU64,
    fb_writes: AtomicU64,
    fb_refusals: AtomicU64,
    bar0_window_reads: AtomicU64,
    bar0_window_writes: AtomicU64,
    commands: AtomicU64,
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
    pub fn new(
        chip: &'static ChipProfile,
        abi: GspAbi,
        clock: Box<dyn NanoClock>,
    ) -> Result<RegPlane, ChipError> {
        RegPlane::with_objects(chip, abi, clock, None)
    }

    /// Build a plane whose served chain also carries an **object-model link**.
    ///
    /// ★★★ A separate constructor rather than a fifth argument on [`RegPlane::new`], and
    /// the reason is a rule this project already paid for
    /// (`gates_quantified_over_a_list`): `new` has ~25 call sites, every one of them a
    /// test about registers, and threading `None` through all of them would make the
    /// interesting case — *there IS an object model* — look like the default. It is not
    /// the default. It is a decision one composition root makes, and it is spelled out at
    /// the one place that makes it.
    ///
    /// `objects` is the [`kayfabe_gsp::CommandPolicy`] the object model is behind. This
    /// crate cannot name its type: it has no `kayfabe-core` dependency, deliberately (*"a
    /// GSP FSM that can see the RM graph starts firing on graph state"*), so what crosses
    /// is a trait object and the port owns the choice. See [`crate::served_chain`] for
    /// where in the chain it lands and what it must not claim.
    ///
    /// # Errors
    ///
    /// As [`RegPlane::new`].
    pub fn with_objects(
        chip: &'static ChipProfile,
        abi: GspAbi,
        clock: Box<dyn NanoClock>,
        objects: Option<Box<dyn CommandPolicy>>,
    ) -> Result<RegPlane, ChipError> {
        let rom = crate::rom_for(chip)?;
        let model = (chip.gsp_model)();
        assert_disjoint(chip, model.as_ref())?;
        let unserviced = crate::unserviced::UnservicedLog::new();
        let fault_buffer = crate::faultbuffer::FaultBufferLog::new();
        Ok(RegPlane {
            chip,
            model,
            rom,
            clock,
            state: Mutex::new(PlaneState {
                fsm: GspFsm::new(abi),
                ram: Box::new(RefusingRam),
                policy: crate::served_policy(
                    chip,
                    abi.driver,
                    unserviced.clone(),
                    fault_buffer.clone(),
                    objects,
                ),
                unclaimed: Vec::new(),
                fb_window: Vec::new(),
                bar0_window: Bar0Window::new(),
                fb: Box::new(RefusingFb),
            }),
            c: PlaneCounters::default(),
            unserviced,
            fault_buffer,
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

    /// ★★★ Install the device's framebuffer, replacing [`RefusingFb`].
    ///
    /// # Why it is a port and not a `Vec<u8>` in this struct
    ///
    /// Two reasons, and they point the same way. **(1)** The bytes are device *memory*, and
    /// where device memory lives is a decision the owner made at another seam
    /// (`eight_blockers_resolved.md` §12.2: the isolate's mapping of the fabricated
    /// aperture). This crate must not become a second answer to it — see [`FbStore`] for
    /// the three measured reasons that seam cannot serve *this* access today, and for what
    /// convergence looks like when it can. **(2)** A 12 GiB framebuffer is not a `Vec` on
    /// any box this project runs on.
    ///
    /// ★ Takes `&self` and the plane's lock, like [`RegPlane::set_ram`], so a plane already
    /// answering registers acquires memory without being rebuilt and without an interval in
    /// which it answers something else.
    pub fn set_fb(&self, fb: Box<dyn FbStore>) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.fb = fb;
    }

    /// Install a command policy, replacing the C-baseline echo.
    pub fn set_policy(&self, policy: Box<dyn CommandPolicy>) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.policy = policy;
    }

    /// The counters.
    ///
    /// ★★★ **The source is DESTRUCTURED, and the absent `..` is the point.** This is a
    /// projection from one struct to another, i.e. exactly the shape that goes silently
    /// out of date: an atomic added to [`PlaneCounters`] that nobody wires here is a
    /// number the outside world can never read, with no red test anywhere. Binding every
    /// field by name makes `rustc` refuse the pattern (E0027) on the day the field is
    /// added, so *"is this on the wire?"* becomes a question someone must answer rather
    /// than one nobody is asked.
    #[must_use]
    pub fn counters(&self) -> Counters {
        let g = |a: &AtomicU64| a.load(Ordering::Relaxed);
        let PlaneCounters {
            reads,
            writes,
            boot_reg_reads,
            ptimer_reads,
            rom_reads,
            gsp_reads,
            gsp_writes,
            unclaimed_reads,
            unclaimed_writes,
            fb_window_reads,
            fb_window_writes,
            fb_reads,
            fb_writes,
            fb_refusals,
            bar0_window_reads,
            bar0_window_writes,
            commands,
            faults,
            ram_refusals,
            irq_requests,
        } = &self.c;
        Counters {
            reads: g(reads),
            writes: g(writes),
            boot_reg_reads: g(boot_reg_reads),
            ptimer_reads: g(ptimer_reads),
            rom_reads: g(rom_reads),
            gsp_reads: g(gsp_reads),
            gsp_writes: g(gsp_writes),
            unclaimed_reads: g(unclaimed_reads),
            unclaimed_writes: g(unclaimed_writes),
            fb_window_reads: g(fb_window_reads),
            fb_window_writes: g(fb_window_writes),
            fb_reads: g(fb_reads),
            fb_writes: g(fb_writes),
            fb_refusals: g(fb_refusals),
            bar0_window_reads: g(bar0_window_reads),
            bar0_window_writes: g(bar0_window_writes),
            commands: g(commands),
            commands_unserviced: self.unserviced.total(),
            faults: g(faults),
            ram_refusals: g(ram_refusals),
            irq_requests: g(irq_requests),
        }
    }

    /// ★★★ **Everything this device life would leave behind, as ONE value — and the
    /// enumeration is enforced by the COMPILER, not by a reviewer's memory.**
    ///
    /// # The property this exists for (`#130`)
    ///
    /// > After a guest bricks the emulator, unload → reload must yield a device
    /// > **indistinguishable from first boot**.
    ///
    /// "Indistinguishable" is a statement quantified over *all* of the device's state, so
    /// the way it fails is not a wrong assertion — it is a **true assertion about a
    /// shrinking universe**. This repository has been bitten by that shape five separate
    /// times, and it had already happened here: the recovery test shipped on 2026-07-31
    /// named [`RegPlane::unclaimed_sample`] as its one non-derived member, and by the time
    /// the next task looked, [`RegPlane::fb_window_sample`] and
    /// [`RegPlane::fault_buffer_sample`] had been added beside it and were in **no**
    /// snapshot at all. Nothing went red. Nothing could have.
    ///
    /// # ★★ How this is structural rather than another list
    ///
    /// The body **destructures [`RegPlane`] and [`PlaneState`] with no `..`**. A field
    /// added to either is `error[E0027]: pattern does not mention field` — the build stops
    /// on the commit that adds the state, and the author decides *there* whether it is
    /// device state (into the residue) or shell wiring (bound to `_` with a reason). That
    /// is the difference between a guarantee and a reset function someone must remember to
    /// extend.
    ///
    /// ⊘ Six fields are deliberately `_`, in three groups, each with a stated reason:
    /// - `chip`, `model`, `rom` — a `&'static` table, its model and the ROM generated from
    ///   it. Immutable for the plane's whole life; two lives of the same chip cannot
    ///   differ here.
    /// - `clock` — the host's monotonic time, which is *supposed* to keep running across a
    ///   reload. A device whose clock reset would be the bug (see this module's
    ///   "a stopped clock is an unkillable hang").
    /// - `ram`, `policy` — the **shell's** wiring, replaced through [`RegPlane::set_ram`]
    ///   and [`RegPlane::set_policy`], and `Box<dyn …>` with no equality to compare. That
    ///   they survive a [`RegPlane::device_reset`] is asserted separately and by name.
    #[must_use]
    pub fn residue(&self) -> PlaneResidue {
        // ★★★ EXHAUSTIVE. The missing `..` is load-bearing — see this method's docs.
        let RegPlane {
            chip: _,
            model: _,
            rom: _,
            clock: _,
            state,
            // Read through `counters()`, which destructures it in turn.
            c: _,
            unserviced,
            fault_buffer,
        } = self;
        let counters = self.counters();
        let s = state.lock().unwrap_or_else(|e| e.into_inner());
        // ★★★ EXHAUSTIVE, for the same reason.
        let PlaneState {
            fsm,
            ram: _,
            policy: _,
            unclaimed,
            fb_window,
            bar0_window,
            // ★ The PORT is the shell's wiring, like `ram` and `policy`; what it HOLDS is
            // device state and is carried out as `fb_resident_bytes` just below.
            fb,
        } = &*s;
        PlaneResidue {
            counters,
            gsp: fsm.clone(),
            unclaimed: unclaimed.clone(),
            fb_window: fb_window.clone(),
            unserviced: unserviced.sample(),
            fault_buffers: fault_buffer.sample(),
            fault_buffers_registered: fault_buffer.total(),
            bar0_window: *bar0_window,
            fb_resident_bytes: fb.resident_bytes(),
        }
    }

    /// ★★ The distinct commands nothing answered, up to
    /// [`crate::unserviced::UNSERVICED_SAMPLE_MAX`] — see that module for why the guest
    /// cannot be asked this question.
    #[must_use]
    pub fn unserviced_sample(&self) -> Vec<crate::unserviced::UnservicedCommand> {
        self.unserviced.sample()
    }

    /// How many times the guest registered a replayable fault buffer
    /// (`NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER`).
    ///
    /// ⊘ A count of *asks*, not of anything served: the recorder declines the command, so
    /// this number rising means the guest's UVM reached the registration and was refused.
    #[must_use]
    pub fn fault_buffers_registered(&self) -> u64 {
        self.fault_buffer.total()
    }

    /// The fault-buffer registrations remembered, capped at
    /// [`crate::faultbuffer::FAULT_BUFFER_SAMPLE_MAX`].
    #[must_use]
    pub fn fault_buffer_sample(&self) -> Vec<crate::faultbuffer::FaultBufferNote> {
        self.fault_buffer.sample()
    }

    /// The distinct `(bar, offset)` pairs no source claimed, up to
    /// [`UNCLAIMED_SAMPLE_MAX`].
    ///
    /// ★ `bar` is part of the answer. An offset alone names two different accesses in two
    /// different apertures, and reporting only the offset made a framebuffer access read as
    /// a register one.
    #[must_use]
    pub fn unclaimed_sample(&self) -> Vec<(u8, u64)> {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.unclaimed.clone()
    }

    /// ★★★ The distinct framebuffer-window accesses seen, up to [`UNCLAIMED_SAMPLE_MAX`].
    ///
    /// The *which* behind [`Counters::fb_window_reads`] / [`Counters::fb_window_writes`].
    /// An operator reading a boot that ends in a missing mapping wants this list, because
    /// every entry is a byte of device memory this port did not carry.
    #[must_use]
    pub fn fb_window_sample(&self) -> Vec<(FbWindow, u64)> {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.fb_window.clone()
    }

    /// The FSM's current boot phase, so a test can assert the guest moved it.
    #[must_use]
    pub fn phase(&self) -> kayfabe_gsp::BootPhase {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.fsm.phase()
    }

    /// ★★ The emulated GSP's state as a **whole value**, so two device lives can be
    /// compared rather than a field list somebody chose.
    ///
    /// [`RegPlane::phase`] answers one field, which is what a boot test wants and exactly
    /// what a *recovery* test must not rely on: "is this device indistinguishable from a
    /// cold one?" quantified over a hand-written list of getters silently stops covering
    /// the field added next. [`kayfabe_gsp::GspFsm`] derives `PartialEq`, so equality
    /// against `GspFsm::new` is total by construction and stays total for free — the same
    /// argument [`kayfabe_gsp::GspFsm::device_reset`] makes for rebuilding the value
    /// instead of clearing fields, used from the outside.
    ///
    /// ★ It is a **clone**, not a borrow: the value lives behind the plane's lock, and
    /// handing out a guard would let a caller hold the register plane shut.
    #[must_use]
    pub fn gsp_state(&self) -> kayfabe_gsp::GspFsm {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.fsm.clone()
    }

    /// Power-on reset: rebuild the FSM, re-point the BAR0 window at its reset value and
    /// **forget every framebuffer byte**. The RAM port, the framebuffer *port* and the
    /// policy survive, because they are the *shell's* wiring and not the device's state.
    ///
    /// ★★★ The framebuffer clear is not tidiness. Content that survived a device life is
    /// the previous guest's page tables, instance blocks and semaphores, readable by the
    /// next one through this very window — see [`FbStore::device_reset`].
    pub fn device_reset(&self) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.fsm.device_reset();
        s.bar0_window = Bar0Window::new();
        s.fb.device_reset();
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
            ReadOutcome::Ptimer(_) => self.c.ptimer_reads.fetch_add(1, Ordering::Relaxed),
            ReadOutcome::Rom(_) => self.c.rom_reads.fetch_add(1, Ordering::Relaxed),
            ReadOutcome::Gsp(_) => self.c.gsp_reads.fetch_add(1, Ordering::Relaxed),
            ReadOutcome::GspFault(_) => self.c.faults.fetch_add(1, Ordering::Relaxed),
            ReadOutcome::Bar0Window(_) => self.c.bar0_window_reads.fetch_add(1, Ordering::Relaxed),
            ReadOutcome::FbWindow(w) => {
                self.note_fb_window(w, off);
                self.c.fb_window_reads.fetch_add(1, Ordering::Relaxed)
            }
            ReadOutcome::Fb { .. } => self.c.fb_reads.fetch_add(1, Ordering::Relaxed),
            ReadOutcome::FbRefused { window, .. } => {
                self.note_fb_window(window, off);
                self.c.fb_refusals.fetch_add(1, Ordering::Relaxed)
            }
            ReadOutcome::Unclaimed => {
                self.note_unclaimed(bar, off);
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
            if let Some(v) = self.ptimer_read(off) {
                return ReadOutcome::Ptimer(mask(v, size));
            }
            if self.chip.rom_window.contains(off) {
                return ReadOutcome::Rom(self.rom_read(off - self.chip.rom_window.base, size));
            }
        }
        // ★★★ THE WINDOW REGISTER IS A LATCH. Asked here, before the GSP model and before
        // the unclaimed arm, and answered with the guest's own last word — see
        // `crate::fbwin` for the two independent ways a defaulted zero here loses a write
        // permanently (the guest's read-modify-write, and RM's own
        // `cachedBar0WindowVidOffset`).
        if bar == kayfabe_abi::pcibars::bus_bar::REGS as u8
            && self.chip.bar0_window_reg != 0
            && off == self.chip.bar0_window_reg
        {
            let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            return ReadOutcome::Bar0Window(mask(u64::from(s.bar0_window.raw()), size));
        }
        // ★★★ Device memory, not a register — asked BEFORE the GSP model and before the
        // unclaimed arm, because a framebuffer window that some future model happened to
        // claim an offset inside would be served as a register, which is the silent-
        // misattribution this classification exists to prevent. `assert_disjoint` refuses
        // such a chip at realize, so reaching here means the two really are separate.
        if let Some(w) = self.chip.fb_window(bar, off) {
            return self.fb_read(w, off, size);
        }
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        match s.fsm.mmio_read_with(self.model.as_ref(), bar, off) {
            None => ReadOutcome::Unclaimed,
            Some(Ok(v)) => ReadOutcome::Gsp(mask(v, size)),
            Some(Err(f)) => ReadOutcome::GspFault(f.fault_tag().0),
        }
    }

    /// ★★★ **THE ONE RESOLVER.** Which framebuffer-physical address a windowed access at
    /// register offset `off` names, or `None` for a window this port has no address model
    /// for.
    ///
    /// # Why this is a function and not two lines inlined twice
    ///
    /// It is called by [`RegPlane::fb_read`] and by [`RegPlane::fb_write`] and by nothing
    /// else. Two copies of `(base << 16) + (off - window_base)` could disagree about the
    /// mask width, about `+` versus `|`, or about which end the offset is subtracted from —
    /// and a read-after-write that resolved to two different addresses is precisely the
    /// failure `kbusVerifyBar2_GM107` reports, arriving with no clue that the arithmetic
    /// was the cause. One function cannot disagree with itself.
    ///
    /// ⊘ The two `None` arms are honest rather than lazy: the framebuffer aperture and the
    /// instance window are **GMMU-translated**, and this port's `GmmuFmt` says it has no
    /// page-table format (`kayfabe_chips::UnbuiltGmmu`). Answering them would mean
    /// inventing a translation, which is `MISS = FAULT`'s whole prohibition.
    fn window_phys(&self, w: FbWindow, off: u64, s: &PlaneState) -> Option<u64> {
        match w {
            FbWindow::Pramin => Some(s.bar0_window.fb_addr(off - self.chip.pramin_window.base)),
            FbWindow::FbAperture | FbWindow::InstanceWindow => None,
        }
    }

    /// Serve one framebuffer-window read.
    fn fb_read(&self, w: FbWindow, off: u64, size: u8) -> ReadOutcome {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(phys) = self.window_phys(w, off, &s) else {
            return ReadOutcome::FbWindow(w);
        };
        let n = usize::from(size.clamp(1, 8));
        let mut buf = [0u8; 8];
        match s.fb.read(phys, &mut buf[..n]) {
            Ok(()) => ReadOutcome::Fb {
                window: w,
                phys,
                value: mask(u64::from_le_bytes(buf), size),
            },
            Err(e) => ReadOutcome::FbRefused {
                window: w,
                phys,
                why: e.why,
            },
        }
    }

    /// Serve one framebuffer-window write.
    ///
    /// ★★★ **There are exactly three answers and none of them is "dropped, success".**
    /// Either the window has no address model (the two translated apertures — reported as
    /// [`WriteOutcome::fb_window`], which is what "dropped" honestly means there), or the
    /// store took the bytes ([`WriteOutcome::fb_landed`]), or the store refused by name
    /// ([`WriteOutcome::fb_refusal`], with the address and the reason). The `Result` from
    /// [`FbStore::write`] is matched, never discarded.
    fn fb_write(&self, w: FbWindow, off: u64, size: u8, val: u64) -> WriteOutcome {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(phys) = self.window_phys(w, off, &s) else {
            drop(s);
            self.note_fb_window(w, off);
            self.c.fb_window_writes.fetch_add(1, Ordering::Relaxed);
            return WriteOutcome {
                fb_window: Some(w),
                ..WriteOutcome::nothing()
            };
        };
        let n = usize::from(size.clamp(1, 8));
        let bytes = val.to_le_bytes();
        let outcome = match s.fb.write(phys, &bytes[..n]) {
            Ok(()) => {
                self.c.fb_writes.fetch_add(1, Ordering::Relaxed);
                WriteOutcome {
                    fb_landed: Some(phys),
                    ..WriteOutcome::nothing()
                }
            }
            Err(e) => {
                self.c.fb_refusals.fetch_add(1, Ordering::Relaxed);
                WriteOutcome {
                    // ★ It is a FAULT, not merely a note: the shell prints `fault`
                    // unconditionally, so a refused framebuffer write is loud in the same
                    // boot rather than inferable from a later `NV_ERR_MEMORY_ERROR`.
                    fault: Some(FB_WRITE_REFUSED),
                    fb_refusal: Some(e),
                    ..WriteOutcome::nothing()
                }
            }
        };
        drop(s);
        self.note_fb_window(w, off);
        outcome
    }

    /// The free-running counter's two halves, or `None` if `off` is neither.
    ///
    /// ★ Each half samples the clock independently, which is the C artifact's behaviour
    /// (`C: src/qemu/nvkvm_gpu_emul.c:1523-1528`) and is safe because the driver reads
    /// high / low / high and retries whenever the high half moved — so a sample that
    /// straddles a 2^32-nanosecond boundary (about every 4.3 s) is *detected* by the reader
    /// rather than needing to be prevented by us.
    fn ptimer_read(&self, off: u64) -> Option<u64> {
        let p = self.chip.ptimer;
        if off == p.lo_off {
            // The low half's `NSEC` field is bits 31:5, so the bottom five bits of a real
            // one read zero (`ogkm-580:
            // src/common/inc/swref/published/turing/tu102/dev_vm.h:224-225`).
            Some(u64::from(
                (self.clock.now_ns() as u32) & PTIMER_LO_NSEC_MASK,
            ))
        } else if off == p.hi_off {
            Some(self.clock.now_ns() >> 32)
        } else {
            None
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
        self.c.writes.fetch_add(1, Ordering::Relaxed);
        // ★★★ THE WINDOW REGISTER IS A LATCH, and it is classified FIRST — before the
        // framebuffer windows, before the GSP model and before the unclaimed arm. A write
        // here that fell through to `unclaimed_writes` would be dropped, and the guest
        // would go on addressing framebuffer address zero believing it had moved the
        // window. See `crate::fbwin`.
        if bar == kayfabe_abi::pcibars::bus_bar::REGS as u8
            && self.chip.bar0_window_reg != 0
            && off == self.chip.bar0_window_reg
        {
            self.c.bar0_window_writes.fetch_add(1, Ordering::Relaxed);
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            // ⊘ The whole word, truncated to 32 bits and NOT masked to the two fields this
            // port decodes: the guest reads this register back and re-writes it, so a bit
            // we do not understand must survive the round trip. Dropping it would be a
            // read-modify-LOSE at a register whose entire job is to be modified in place.
            s.bar0_window.set_raw(val as u32);
            return WriteOutcome {
                claimed: true,
                ..WriteOutcome::nothing()
            };
        }
        // ★★★ A framebuffer write either LANDS or REFUSES BY NAME — see
        // [`RegPlane::fb_write`], which is where the three-answers rule is written down.
        // It is classified before the register sources for the reason `read_inner` gives.
        if let Some(w) = self.chip.fb_window(bar, off) {
            return self.fb_write(w, off, size, val);
        }
        let claimed = self.model.decode_reg(bar, off).is_some();
        if !claimed {
            self.note_unclaimed(bar, off);
            self.c.unclaimed_writes.fetch_add(1, Ordering::Relaxed);
            return WriteOutcome::nothing();
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
                self.c
                    .commands
                    .fetch_add(report.commands.len() as u64, Ordering::Relaxed);
                WriteOutcome {
                    claimed: true,
                    raise_status_irq: report.raise_status_irq,
                    transitions: report.transitions.len(),
                    commands: report.commands.len(),
                    ..WriteOutcome::nothing()
                }
            }
            Err(f) => {
                self.c.faults.fetch_add(1, Ordering::Relaxed);
                // ★ Matched for its PAYLOAD, not merely for its shape: the counter says
                // how many, and `ram_refusal` carries which address and why out to the
                // shell, which is the only channel an operator ever reads.
                let ram_refusal = match f {
                    GspFault::GuestRam(r) => {
                        self.c.ram_refusals.fetch_add(1, Ordering::Relaxed);
                        Some(r)
                    }
                    _ => None,
                };
                WriteOutcome {
                    claimed: true,
                    fault: Some(f.fault_tag().0),
                    ram_refusal,
                    ..WriteOutcome::nothing()
                }
            }
        }
    }

    fn note_unclaimed(&self, bar: u8, off: u64) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.unclaimed.len() < UNCLAIMED_SAMPLE_MAX && !s.unclaimed.contains(&(bar, off)) {
            s.unclaimed.push((bar, off));
        }
    }

    fn note_fb_window(&self, w: FbWindow, off: u64) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.fb_window.len() < UNCLAIMED_SAMPLE_MAX && !s.fb_window.contains(&(w, off)) {
            s.fb_window.push((w, off));
        }
    }
}

/// The low half's `NSEC` field, bits 31:5.
const PTIMER_LO_NSEC_MASK: u32 = 0xFFFF_FFE0;

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
    // ★ The counter's two halves go first, and are checked against every other source
    // rather than only against the ones added before them. A counter half swallowed by an
    // earlier source is the exact defect this whole check exists for, in its worst form:
    // the guest would read a constant and spin forever with nothing printed.
    for off in [chip.ptimer.lo_off, chip.ptimer.hi_off] {
        if off >= chip.regs_aperture_len {
            return Err(ChipError::OutsideAperture {
                off,
                aperture: chip.regs_aperture_len,
            });
        }
        if chip.boot_regs.iter().any(|r| r.off == off) {
            return Err(ChipError::OverlappingSources {
                off,
                a: "a silicon-constant register",
                b: "the free-running nanosecond counter",
            });
        }
        if chip.rom_window.contains(off) {
            return Err(ChipError::OverlappingSources {
                off,
                a: "the free-running nanosecond counter",
                b: "the ROM window",
            });
        }
        if model.decode_reg(0, off).is_some() {
            return Err(ChipError::OverlappingSources {
                off,
                a: "the free-running nanosecond counter",
                b: "the GSP register model",
            });
        }
    }
    if chip.ptimer.lo_off == chip.ptimer.hi_off {
        return Err(ChipError::OverlappingSources {
            off: chip.ptimer.lo_off,
            a: "the nanosecond counter's low half",
            b: "its own high half",
        });
    }
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
    // ★★★ THE TWO HALVES OF THE BAR0 MOVING WINDOW MUST BOTH BE THERE, OR NEITHER.
    //
    // The register selects which framebuffer page the aperture shows. A row with the
    // aperture and no register serves a FIXED view of framebuffer address zero while the
    // guest believes it has moved the window — every access mis-addressed, nothing logged,
    // and the first symptom hundreds of operations later at `kbusVerifyBar2`. That is the
    // exact failure `#146` was written against, so it is refused at realize rather than
    // discovered in a boot.
    if (chip.pramin_window.len == 0) != (chip.bar0_window_reg == 0) {
        return Err(ChipError::WindowWithoutItsRegister {
            window_len: chip.pramin_window.len,
            reg_off: chip.bar0_window_reg,
        });
    }
    if chip.bar0_window_reg != 0 {
        let off = chip.bar0_window_reg;
        if off >= chip.regs_aperture_len {
            return Err(ChipError::OutsideAperture {
                off,
                aperture: chip.regs_aperture_len,
            });
        }
        // ★★ Checked against every other source, in the order the read path asks them, and
        // for the sharpest version of the disjointness argument: this offset is answered
        // BEFORE all of them, so an overlap would not merely resolve to the wrong source —
        // it would make the other source *unreachable*, and the boot register or GSP
        // register it swallowed would answer a guest's latch value forever.
        if chip.boot_regs.iter().any(|r| r.off == off) {
            return Err(ChipError::OverlappingSources {
                off,
                a: "the BAR0 moving window's own register",
                b: "a silicon-constant register",
            });
        }
        if off == chip.ptimer.lo_off || off == chip.ptimer.hi_off {
            return Err(ChipError::OverlappingSources {
                off,
                a: "the BAR0 moving window's own register",
                b: "the free-running nanosecond counter",
            });
        }
        if chip.rom_window.contains(off) {
            return Err(ChipError::OverlappingSources {
                off,
                a: "the BAR0 moving window's own register",
                b: "the ROM window",
            });
        }
        if chip.pramin_window.contains(off) {
            return Err(ChipError::OverlappingSources {
                off,
                a: "the BAR0 moving window's own register",
                b: "the PRAMIN framebuffer window",
            });
        }
        if model.decode_reg(0, off).is_some() {
            return Err(ChipError::OverlappingSources {
                off,
                a: "the BAR0 moving window's own register",
                b: "the GSP register model",
            });
        }
    }
    // ★★★ The PRAMIN window is checked against every register source, exhaustively, for a
    // reason the other windows do not have: it is classified BEFORE the GSP model, so an
    // overlap here would not read as the wrong source, it would make a register the FSM
    // needs unreachable — and the FSM would then be waiting on a doorbell that can never
    // arrive. That is the stopped-clock failure with a different cause.
    let pramin = chip.pramin_window;
    if pramin.len != 0 {
        if pramin.base.saturating_add(pramin.len) > chip.regs_aperture_len {
            return Err(ChipError::OutsideAperture {
                off: pramin.base,
                aperture: chip.regs_aperture_len,
            });
        }
        for r in chip.boot_regs {
            if pramin.contains(r.off) {
                return Err(ChipError::OverlappingSources {
                    off: r.off,
                    a: "the PRAMIN framebuffer window",
                    b: "a silicon-constant register",
                });
            }
        }
        for off in [chip.ptimer.lo_off, chip.ptimer.hi_off] {
            if pramin.contains(off) {
                return Err(ChipError::OverlappingSources {
                    off,
                    a: "the PRAMIN framebuffer window",
                    b: "the free-running nanosecond counter",
                });
            }
        }
        let mut off = pramin.base;
        let end = pramin.base + pramin.len;
        while off < end {
            if chip.rom_window.contains(off) {
                return Err(ChipError::OverlappingSources {
                    off,
                    a: "the PRAMIN framebuffer window",
                    b: "the ROM window",
                });
            }
            if model.decode_reg(0, off).is_some() {
                return Err(ChipError::OverlappingSources {
                    off,
                    a: "the PRAMIN framebuffer window",
                    b: "the GSP register model",
                });
            }
            off += 4;
        }
    }
    Ok(())
}

kayfabe_util::assert_send_sync!(RegPlane);
kayfabe_util::assert_send_sync!(RefusingRam);
