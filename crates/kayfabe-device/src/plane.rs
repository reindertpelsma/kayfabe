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

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, RwLock};

use kayfabe_arch::gsp::GspModel;
use kayfabe_arch::{Aperture, GmmuFmt};
use kayfabe_gsp::{CommandPolicy, GspAbi, GspFault, GspFsm, GuestRam, RamRefused};
use kayfabe_mmu::walker::{FbRead, translate_from_entry};
use kayfabe_trace::Faulted;

use crate::bar2::{BarPdeLog, BarPdes};
use crate::cpuintr::CpuIntrTree;
use crate::doorbell::{DoorbellPort, DoorbellRefused, DoorbellReport, RefusingDoorbell};
use crate::fbwin::{Bar0Window, FbRefused, FbStore, RefusingFb};
use crate::gvaspub::{GvasPubLog, GvasPubSnapshot};
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
///
/// ---
/// ⚠⚠⚠ **THIS PORT IS A BOOT-ONLY STOPGAP. It is not the finished design, and it must not
/// be read as intent** (`#128`, `docs/design/register_plane_read_native.md` §4).
///
/// It was necessary and it is correct for what it fixed: RM's `gpuCheckTimeout` needs
/// elapsed time to *exist*, and answering the counter with a defaulted zero produced an
/// **unkillable silent D-state with no dmesg** on a stock 580.159.04 guest, cleared by
/// `5b54494` in the same session. But a **synthetic CPU-side clock is the wrong answer for
/// two independent reasons**:
///
/// 1. **Accuracy, not performance.** A vmexit costs microseconds with high variance, so a
///    *nanosecond* counter read through a trap carries jitter three orders of magnitude
///    larger than its own resolution. It stops being a nanosecond timer the instant it is
///    trapped — however rarely it is read. ⊘ Trap volume is irrelevant to this.
/// 2. **Timebase.** Compute is forwarded to a real host GPU, so every timestamp the GPU
///    produces (event semaphores, `%globaltimer`) is in the **host GPU's** timebase. This
///    clock is unrelated to it, so anything correlating the two —
///    `cudaEventElapsedTime`, every profiler — gets a **wrong answer, not a slow one**.
///
/// The finished design is *read-native, write-trap*: the guest's timer page is backed by a
/// host GPU register page under `KVM_MEM_READONLY`, so reads never exit and correlation is
/// correct by construction. ★ **MEASURED 2026-08-02 on a real GA106 (RTX 3060,
/// 580.159.04):** an unprivileged, capability-less process *can* map the host counter and
/// read it live — `docs/reference/bench_evidence/timer-mappability-9087090.out`, decoded in
/// `docs/design/read_native_timer_measured.md`. What is not built is the memslot caller.
///
/// ⊘ **The standing rule this port must keep whatever replaces it:** *never answer a
/// free-running counter with a constant.* Zero is **plausible**, so the driver believes it
/// and spins forever with no diagnostic; a refusal is not plausible and surfaces in one
/// boot. That is why [`kayfabe_abi::submit::ptimer_sample`] refuses rather than guessing,
/// and why there is no `Default` impl here.
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
    /// ★ Writes to the free-running nanosecond counter, **refused by name** (`#128`).
    ///
    /// Its own counter rather than a share of [`Self::unclaimed_writes`], because the two
    /// mean opposite things: unclaimed is *"this port does not model that offset yet"*,
    /// this is *"this port models it and says no"*. A guest RM issuing
    /// `tmrSetCurrentTime` should show up here and nowhere else.
    pub ptimer_writes_refused: u64,
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
    /// ★★★ Reads **served through the GMMU** from the translated instance/`BAR2` window.
    ///
    /// ★ Counted apart from [`Counters::fb_reads`] even though the bytes come out of the
    /// same store, because the two say different things: `fb_reads` says a window
    /// resolved an address arithmetically, this says a **page walk succeeded**. A boot in
    /// which this is zero and `kbusVerifyBar2` still fails has an unrooted aperture; a
    /// boot in which it is large and the test still fails has a translation that resolves
    /// to the wrong byte. Merging them would make those two indistinguishable.
    pub bar2_reads: u64,
    /// ★★★ Writes **served through the GMMU** into the translated instance/`BAR2` window.
    pub bar2_writes: u64,
    /// ★★★ Translated accesses this port **refused, by name** — an unrooted aperture, an
    /// unmapped virtual address, a leaf in an aperture this port cannot serve, or a
    /// format that is not installed.
    ///
    /// ⊘ **This is what a dropped translated write looks like**, and it must never be
    /// zero-with-a-shrug: `kbusVerifyBar2` detects exactly one such drop, ninety lines
    /// and one L2 evict later, as `NV_ERR_MEMORY_ERROR`.
    pub bar2_faults: u64,
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
    /// Accesses to the `CPU_INTR` tree (`crate::cpuintr`) — reads and writes together.
    ///
    /// ★ One counter for both directions on purpose: the diagnostic question is *"did the
    /// guest drive this block at all"*, and a boot in which it is zero means the driver
    /// never reached `_osVerifyInterrupts`, whatever else went wrong.
    pub cpu_intr_accesses: u64,
    /// ★★★ `CPU_INTR_LEAF_TRIGGER` writes that latched a vector — i.e. the number of
    /// message-signalled interrupts this device asked the shell to deliver.
    ///
    /// `[measured]` the C oracle's `cap1` has **exactly one** across 359 062 records
    /// (`crates/kayfabe-crec/tests/cap1_differential.rs:31`), so a healthy boot's expected
    /// value here is 1, not 0 and not many.
    pub cpu_intr_raises: u64,
    /// ★★★ Of those, how many real silicon would have **masked** — see
    /// [`crate::cpuintr::TriggerOutcome::would_be_masked`].
    ///
    /// ⊘ This device raises anyway, deliberately. A non-zero value here is not a fault; it
    /// is the evidence that the enable bookkeeping and the guest's expectations disagree,
    /// and it is the only thing that could ever justify turning gating on.
    pub cpu_intr_masked: u64,
    /// ★★★ **§14.18 — completions this device ANNOUNCED with a non-stall vector.**
    ///
    /// One per [`crate::DoorbellReport::ServedLocally`] whose channel's bound engine
    /// resolved to a `vectorNonStall` ([`crate::nonstall::non_stall_vector`]). ⊘ A
    /// *separate* number from [`Counters::cpu_intr_raises`], which counts the guest asking
    /// the device to interrupt the guest: they are two causes and a boot that delivered the
    /// wrong number of vectors must be diagnosable as to which.
    ///
    /// ★ `nonstall_raises + nonstall_unvectored` is the number of completions, so the two
    /// together say *"every copy we served, we either announced or could not"* — the
    /// invariant that makes a silent completion impossible to hide.
    pub nonstall_raises: u64,
    /// ★★★ Completions this device could **not** announce, with the reason available in
    /// the serving's own note.
    ///
    /// ⊘ **This is the one number that must be zero in a healthy boot**, and it is the
    /// whole reason serving notifier index 35 is honest: it counts work that really
    /// happened and was never notified. `[measured 2026-08-08, boot cebind_p35 at
    /// 5a035e0]` the scrubber binds COPY2, whose row publishes `vectorNonStall = 0x07`, so
    /// the expected value is 0. A non-zero one is not a crash — it is the promise being
    /// broken quietly, printed.
    pub nonstall_unvectored: u64,
    /// ★★ Of the [`Counters::nonstall_raises`], how many real silicon would have **masked**
    /// — see [`crate::cpuintr::TriggerOutcome::would_be_masked`].
    ///
    /// ⚠ Sharper here than for the loopback trigger, and worth reading before concluding a
    /// delivery worked: the guest's own non-stall scan is
    /// `intrReadRegLeaf(j) & intrReadRegLeafEnSet(j)`
    /// (`ogkm-580: intr_nonstall_tu102.c:344-346`), so a vector latched while its
    /// `LEAF_EN` bit is clear is **invisible to the ISR** even though the message was
    /// delivered. A non-zero value here with a hung scrubber is that exact diagnosis, and
    /// without the counter the two are the same silence.
    pub nonstall_masked: u64,
    /// ★★★ **E2** — guest MMIO writes that landed on the usermode doorbell register
    /// ([`crate::doorbell_reg`]), i.e. work-submit tokens the guest rang.
    ///
    /// ★ The **arrival** count, incremented before the port is consulted, so it is a
    /// statement about the guest and not about the core: `doorbells == doorbells_served +
    /// doorbells_refused` at all times, and neither of the two can absorb the other.
    ///
    /// ⊘ A boot in which this is zero is not evidence that the doorbell path is broken —
    /// see `docs/design/execution_plane_increments.md` §7: at the current wall the guest
    /// driver never reaches `kfifoUpdateUsermodeDoorbell` because the channel *schedule*
    /// before it fails.
    pub doorbells: u64,
    /// Of those, the ones the installed [`crate::DoorbellPort`] **served** — a
    /// `DoorbellOutcome` came back from the core.
    pub doorbells_served: u64,
    /// Of those, the ones the port **refused, by name**. Includes the refusal of the
    /// default [`crate::RefusingDoorbell`], which is what a build with no forwarding plane
    /// answers every ring with.
    pub doorbells_refused: u64,
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
    /// ★★★ The control census — what WAS answered, and with what result, including the
    /// served-but-refused class the `unserviced` list cannot see (`crate::census`).
    /// Guest-driven observation state, in the residue for `fb_window`'s reason.
    pub census: crate::census::CensusSnapshot,
    /// ★ The replayable-fault-buffer registrations the guest asked for and this port
    /// declined. Guest-driven, and — like `fb_window` — in no snapshot before `#130`.
    pub fault_buffers: Vec<crate::faultbuffer::FaultBufferNote>,
    /// How many such registrations arrived in total, including repeats and anything past
    /// the sample bound. A count and a sample fail differently: a sample that saturates
    /// stops moving, and the count does not.
    pub fault_buffers_registered: u64,
    /// How many **client shadow** fault-buffer registrations (`0x20800a9d`) arrived.
    ///
    /// ⊘ Separate from [`Self::fault_buffers_registered`] because the two controls carry
    /// different promises — see `crate::faultbuffer`.
    pub shadow_fault_buffers_registered: u64,
    /// How many **access-counter** buffer registrations (`0x20800a1d`) arrived.
    pub access_cntr_buffers_registered: u64,
    /// ★★★ The BAR0 moving window's register, as the guest last wrote it.
    ///
    /// Device state, so it is in the residue: a reloaded device whose window still pointed
    /// where the previous guest left it is not indistinguishable from a cold one, and the
    /// register's reset value is a documented silicon fact (`_BASE_0`, `_TARGET_VID_MEM`).
    pub bar0_window: Bar0Window,
    /// ★★★ The bus apertures' published page-table roots.
    ///
    /// Device state, so it is in the residue, and for the sharpest reason in this struct:
    /// a reloaded device that still held the previous guest's BAR2 root would follow it
    /// into framebuffer the new guest has not written and answer the **previous** guest's
    /// mappings through the new one's aperture.
    pub bar_pdes: BarPdes,
    /// ★★★ The VA spaces' published page-directory roots (`crate::gvaspub`).
    ///
    /// In the residue for `bar_pdes`' reason, one level up: a reloaded device that still
    /// held the previous guest's VAS roots would attribute that guest's address spaces to
    /// this one's channels the moment anything consumes them.
    pub gvas_pub: GvasPubSnapshot,
    /// ★★★ How many bytes of framebuffer the store is holding.
    ///
    /// ⊘ Content, not identity — this is a *size*, and it is here because the alternative
    /// was leaving device memory out of the residue entirely. A device life that ended
    /// holding the previous guest's page tables and instance blocks is exactly what
    /// [`FbStore::device_reset`] exists to prevent, and a property quantified over "all of
    /// the device's state" that silently skipped its **memory** would be `#130`'s own
    /// shrinking-universe failure.
    pub fb_resident_bytes: u64,
    /// ★★★ **E2** — what the usermode doorbell aperture saw: the last token and the first
    /// refusal ([`DoorbellLog`]).
    ///
    /// In the residue rather than only in the counters because it is *state*, and because
    /// the property `#130` exists for is the one that matters here: a device life that
    /// ended still reporting the **previous** guest's work-submit token would attribute a
    /// ring to whoever booted next.
    pub doorbell: DoorbellLog,
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
    /// ★★★ The `CPU_INTR` tree — pending and enable bits for the Turing-or-later interrupt
    /// block. Under the same lock as everything else here, because the guest's ISR reads
    /// `TOP` and `LEAF` from one vCPU while another may still be inside the trigger write
    /// that set them, and a torn view of that pair is a lost interrupt. See
    /// [`crate::cpuintr`].
    cpu_intr: CpuIntrTree,
    /// The framebuffer this device advertises, as a port. [`RefusingFb`] until a shell
    /// installs one — the exact shape [`PlaneState::ram`] already has.
    fb: Box<dyn FbStore>,
    /// ★★★ The page-table format this chip's GMMU uses, as a port. `None` until a shell
    /// installs one, and `None` is a **refusal**: a translated aperture with no format
    /// answers *"this port has no page-table format"* by name rather than guessing a
    /// stride. See [`RegPlane::set_mmu`].
    mmu: Option<Box<dyn GmmuFmt>>,
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
    /// ★★★ A `CPU_INTR` tree register — pending bits, enable bits, or the write-only
    /// trigger reading as zero. See [`crate::cpuintr`].
    ///
    /// A variant of its own, and not folded into [`ReadOutcome::Gsp`], because the guest's
    /// **interrupt service routine** is the reader: `osWaitForInterrupt` decides whether an
    /// interrupt happened at all from one of these reads (`ogkm-580: os_sanity.c:57,101`),
    /// so a boot in which this count is zero and a vector was raised is a boot in which the
    /// ISR never looked — a completely different diagnosis from "we answered wrong".
    CpuIntr(u64),
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
    /// ★★★ A **translated** window this port would not resolve — with the virtual
    /// address and the reason.
    ///
    /// ⊘ A different variant from [`ReadOutcome::FbRefused`] because the two carry
    /// different kinds of address: that one has a *physical* address the store refused,
    /// this one has a *virtual* one that never became physical. Reporting a VA in a field
    /// named `phys` is the aliasing defect `kayfabe_arch::Aperture` exists to prevent,
    /// one layer up.
    TranslationRefused {
        /// Which window.
        window: FbWindow,
        /// The virtual address, i.e. the aperture offset the guest accessed.
        va: u64,
        /// Why it did not resolve.
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
            | Self::Bar0Window(v)
            | Self::CpuIntr(v) => v,
            Self::Fb { value, .. } => value,
            Self::GspFault(_)
            | Self::FbWindow(_)
            | Self::FbRefused { .. }
            | Self::TranslationRefused { .. }
            | Self::Unclaimed => 0,
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
    /// ★★★ A **translated** write this port would not resolve — the virtual address, the
    /// length and the reason.
    ///
    /// ⊘ There is deliberately no success-shaped answer beside it: a translated write is
    /// either landed ([`WriteOutcome::fb_landed`], with the *physical* address the walk
    /// produced) or refused here. `kbusVerifyBar2_GM107:4163-4200` is the caller that
    /// notices, and it notices ninety lines later.
    pub bar2_refusal: Option<Bar2Refused>,
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
    /// ★★★ A vector is now pending: **deliver a message-signalled interrupt**.
    ///
    /// ★★ **TWO producers since §14.18, and one field**, which is the opposite of the
    /// split argued for below — deliberately, because they are the same *action* on the
    /// same tree: either the guest wrote `CPU_INTR_LEAF_TRIGGER` (the `_osVerifyInterrupts`
    /// loopback) or this device latched an engine's `vectorNonStall` because a completion
    /// it witnessed really happened ([`RegPlane::announce_completion`]). The guest's ISR
    /// cannot tell them apart — it reads TOP/LEAF and demultiplexes — so a second flag
    /// would be a distinction the wire could not carry. Which one it was is in
    /// [`Counters::cpu_intr_raises`] vs [`Counters::nonstall_raises`], where the question
    /// is actually asked.
    ///
    /// ⚠ A second field beside [`WriteOutcome::raise_status_irq`] and not the same one,
    /// even though today both end in one `msix_notify`. They are two different *causes* —
    /// one is the emulated GSP announcing that it put something on the status queue, the
    /// other is the guest asking the device to interrupt the guest — and a boot that
    /// delivered the wrong number of vectors must be diagnosable as to which. Folding them
    /// would make `Counters::irq_requests` and `Counters::cpu_intr_raises` two names for
    /// one number and lose exactly that.
    pub raise_cpu_intr: bool,
    /// ★★★ **E2** — this write was a **usermode doorbell**, and here is what the core
    /// answered: a served outcome, or a refusal with a kind and a sentence.
    ///
    /// `None` means the write was not a doorbell at all — which is the *control* every E2
    /// acceptance needs, expressed in the type rather than in a counter comparison.
    ///
    /// ⊘ There is deliberately no `doorbell: bool` beside it. Two fields for one fact is
    /// how a *"was a doorbell"* and a *"went somewhere"* come to disagree; see
    /// [`crate::DoorbellReport`], which has the same shape for the same reason.
    pub doorbell: Option<DoorbellReport>,
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
            bar2_refusal: None,
            fb_refusal: None,
            raise_status_irq: false,
            raise_cpu_intr: false,
            doorbell: None,
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

/// The fault tag a write to the **free-running nanosecond counter** carries (`#128`).
///
/// ★★★ It is a REFUSAL, not a drop and not an emulated set. The guest's own RM issues
/// this write — `tmrSetCurrentTime_GV100` (`ogkm-580:
/// src/nvidia/src/kernel/gpu/timer/arch/volta/timer_gv100.c:56-82`) — so it will be seen,
/// and it must be seen as a *decision*. Under `register_plane_read_native.md` the guest
/// reads this counter natively off the host GPU, and a guest that could also *move* it
/// would destroy the single property that buys: that a guest timestamp and a host GPU
/// timestamp are in one timebase.
pub const PTIMER_WRITE_REFUSED: &str = "a write to the free-running nanosecond counter was refused: the guest reads the \
     HOST GPU's own counter and may not move it; the value did NOT change";

/// The fault tag a refused **translated** write carries out to the shell.
///
/// ★ Its own sentence, not [`FB_WRITE_REFUSED`] reused, because the two failures have
/// different fixes: that one means the store would not take bytes at an address that had
/// already been resolved, this one means no address was resolved at all.
pub const BAR2_WRITE_REFUSED: &str = "the GMMU would not translate this write through the instance/BAR2 window; the \
     bytes did NOT land anywhere, and the guest will not be told";

/// ★★★ A translated access this port refused, whole.
///
/// The twin of [`FbRefused`] on the other side of the walk: address, length, reason. The
/// address is **virtual** and the type says so in its field name, because a virtual and a
/// physical address are different facts and one struct that could carry either would let
/// a reader mistake one for the other at exactly the moment it matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bar2Refused {
    /// The aperture offset the guest accessed, i.e. the virtual address in the bus VAS.
    pub va: u64,
    /// How many bytes were asked for.
    pub len: usize,
    /// One sentence.
    pub why: &'static str,
}

/// [`RegPlane`]'s refusal when no page-table format has been installed.
pub const NO_MMU_PORT: &str = "the register plane has no page-table format installed; the shell never called \
     set_mmu, so a translated aperture has no way to resolve an address and this port \
     will not invent a stride";

/// The aperture has no published root.
pub const BAR2_UNROOTED: &str = "the guest has not published a root page-directory entry for this aperture \
     (UPDATE_BAR_PDE); on the firmware-offload model that entry is the ONLY fact this \
     port ever receives about that page-table tree";

/// The published root names a level this format does not have.
pub const BAR2_UNKNOWN_ROOT_LEVEL: &str = "the published root entry declares a level shift this chip's page-table format \
     does not have a level for; decoding it at a guessed level would read the wrong \
     fields of the entry";

/// The access falls outside the one root slot the guest published.
pub const BAR2_OUTSIDE_PUBLISHED_SLOT: &str = "this aperture offset indexes a root slot the guest never published; on the \
     firmware-offload model the guest owns root slot 0 and the firmware owns the rest, \
     and this port has published none of its own";

/// The walk resolved to an aperture this port cannot serve.
pub const BAR2_FOREIGN_APERTURE: &str = "the guest's page tables map this address into an aperture this port does not \
     serve through the bus window; answering it out of the framebuffer store would \
     alias two different memories onto one address";

/// The walk resolved to a leaf the guest marked read-only.
pub const BAR2_READ_ONLY: &str = "the guest's own page tables mark this mapping read-only; letting a write through \
     would give the guest a mapping stronger than the one it published";

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
    /// ★★★ The control census — seen-and-served and seen-and-refused, the two positive
    /// states without which the unserviced list's absences discriminate nothing
    /// (`crate::census`). Held here for [`RegPlane::unserviced`]'s two reasons: readable
    /// without the FSM's lock, and it survives [`RegPlane::set_policy`].
    census: crate::census::ControlCensusLog,
    /// ★ Step 5a's whole deliverable: where the guest said its replayable fault buffer is
    /// (`crate::faultbuffer`). Recorded, never answered.
    fault_buffer: crate::faultbuffer::FaultBufferLog,
    /// ★★★ The bus apertures' published root page-directory entries (`crate::bar2`). Held
    /// here as well as inside the chain link that writes it, for the same two reasons
    /// [`RegPlane::unserviced`] is: reading it must not take the FSM's lock behind a
    /// doorbell, and a caller that replaced the policy still gets to read what the guest
    /// published.
    bar_pdes: BarPdeLog,
    /// ★★★ **The VA-space page-directory publications** (`crate::gvaspub`) — the guest
    /// telling us where its page directories live, with the `hObject` that names which VA
    /// space they root. Held here for the two reasons `bar_pdes` is: reading it must not
    /// take the FSM's lock behind a doorbell, and a caller that replaced the policy still
    /// gets to read what the guest published.
    gvas_pub: GvasPubLog,
    /// ★★★ **E2 — the usermode doorbell seam** (`crate::doorbell`).
    ///
    /// ⚠ **Outside [`RegPlane::state`], and that is a requirement rather than a
    /// preference.** `kayfabe_rt::SharedDevice::doorbell` takes the core's ranked locks and
    /// can *block* on the isolate pool's backpressure gate; running it under the FSM mutex
    /// would put a guest-controlled wait beneath the lock every other vCPU needs to read a
    /// register. The classification in [`RegPlane::write`] therefore runs before that mutex
    /// is taken, and this port has a lock of its own.
    ///
    /// ★ A [`RwLock`] and not a [`Mutex`]: rings are the hot path and sibling vCPUs must be
    /// able to ring concurrently; only [`RegPlane::set_doorbell`] — one call, at
    /// composition — takes it for writing.
    doorbell: RwLock<Box<dyn DoorbellPort>>,
    /// What the doorbell aperture has seen, beyond the counts. Its own small lock, taken
    /// *after* the port has answered and never held across the call.
    doorbell_log: Mutex<DoorbellLog>,
}

/// ★★★ **E2** — what this device life's doorbell aperture saw, beyond the counters.
///
/// ⊘ The **first** refusal is kept and later ones are not. A flood of identical rings must
/// not be able to push the diagnosis out of a one-line report, and the first one is the one
/// that happened before anything else could have changed the device's state.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DoorbellLog {
    /// The most recent token the guest stored, or `None` if it never rang.
    ///
    /// ★ An `Option` and not a bare `0`: token zero is a legal work-submit token (runlist
    /// 0, channel 0), so a sentinel could not tell *"rang channel 0"* from *"never rang"* —
    /// the same two-fields-for-one-fact argument `KayfabeRegWrite::fb_landed` already
    /// carries.
    pub last_token: Option<u64>,
    /// The first ring the port refused — kind and sentence — or `None` if none was.
    pub first_refusal: Option<DoorbellRefused>,
    /// ★★★ **E10e — the MOST RECENT ring the SHELL served itself**, whole
    /// ([`DoorbellReport::ServedLocally::note`]): what the CPU copy-engine executor
    /// actually did — spans run, bytes moved, where the finishPayload landed.
    ///
    /// ⊘ **Last, where [`Self::first_refusal`] is first, and the asymmetry is the point.**
    /// A refusal is a *diagnosis* and a flood of identical rings must not push it out of
    /// the one line a teardown report has room for; a serving is *progress*, and the last
    /// one is how far the guest got. `memmgrTestCeUtils` issues a `MemSet` and then a
    /// `MemCopy`, so "which one was the last to land" is the whole question the report is
    /// asked (`ogkm-580: mem_mgr.c:463` vs `:467`).
    pub last_local_serving: Option<String>,
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
    ptimer_writes_refused: AtomicU64,
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
    bar2_reads: AtomicU64,
    bar2_writes: AtomicU64,
    bar2_faults: AtomicU64,
    bar0_window_reads: AtomicU64,
    bar0_window_writes: AtomicU64,
    commands: AtomicU64,
    faults: AtomicU64,
    ram_refusals: AtomicU64,
    irq_requests: AtomicU64,
    cpu_intr_accesses: AtomicU64,
    cpu_intr_raises: AtomicU64,
    cpu_intr_masked: AtomicU64,
    nonstall_raises: AtomicU64,
    nonstall_unvectored: AtomicU64,
    nonstall_masked: AtomicU64,
    doorbells: AtomicU64,
    doorbells_served: AtomicU64,
    doorbells_refused: AtomicU64,
}

/// How many level rows [`RegPlane::bar2_phys`] will ask a format about when it maps a
/// published root's level shift back to a level index.
///
/// ★ A bound rather than `u8::MAX`, and it is a bound on **our** loop rather than a claim
/// about any format: `kayfabe_arch::GmmuFmt::level_shift` answers `None` for a row it does
/// not have, so scanning further finds nothing and only costs calls. Sixteen is already
/// three times the deepest regime this project has read.
/// How many format levels a root's `pageShift` is searched against.
///
/// ⊘ `pub(crate)`: [`crate::ceresolve`] derives a published root's level with the exact
/// same search `bar2_phys` does for BAR2's, and two constants would be two answers to one
/// question.
pub(crate) const MAX_FORMAT_LEVELS: u8 = 16;

/// Why a windowed access did not resolve to a framebuffer address.
///
/// ★ Two variants and not one string, because the two are reported differently: one is a
/// window this port has never modelled (counted as a dropped *window* access, which is
/// what it has always been), and the other is a window that IS modelled and refused this
/// particular address (counted as a translation fault, with the address).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowRefusal {
    /// No address model for this window at all.
    NoAddressModel,
    /// A translated window that would not resolve this virtual address.
    Translated {
        /// The aperture offset asked for.
        va: u64,
        /// One sentence.
        why: &'static str,
    },
}

/// ★★★ **The join.** The device's one [`FbStore`], wearing the page-table walker's
/// byte-source trait.
///
/// # Why this is five lines and not a second store
///
/// `kayfabe_mmu::walker::FbRead` and [`FbStore`] are two views of the same bytes and they
/// exist separately for a reason that is about *direction*, not about content: `FbRead`
/// deliberately has no method that hands content **in**, which is what makes *"the core
/// acquired a store of device memory"* unrepresentable in the pure crates. A borrow is
/// therefore the only correct adapter — it cannot outlive the store, cannot copy it, and
/// cannot become a second one.
///
/// ⊘ `false`, never zeros. [`FbStore::read`] already answers `Ok` with zeros for an
/// address inside the advertised framebuffer that nobody has written (which is a true
/// statement about memory this device owns); it returns `Err` only for a range it does not
/// back at all, and *that* is what the walker must see as a miss.
struct FbStoreReader<'a> {
    fb: &'a mut dyn FbStore,
}

impl FbRead for FbStoreReader<'_> {
    fn read(&mut self, phys: u64, buf: &mut [u8]) -> bool {
        self.fb.read(phys, buf).is_ok()
    }
}

/// Why a [`RegPlane::read_published_va`] produced no bytes.
///
/// ⊘ Two arms, not one string: *"the walk refused"* and *"the walk resolved and the store
/// refused"* are different findings about different components, and a caller that reported
/// them identically would spend a boot telling them apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishedVaRead {
    /// The walk did not produce an address; its own finding, whole.
    Unresolved(crate::ceresolve::CeResolve),
    /// The address resolved and the store that owns that aperture refused it, by name.
    Store(&'static str),
}

impl PublishedVaRead {
    /// One line naming what happened, for a boot report.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            PublishedVaRead::Unresolved(r) => r.describe(),
            PublishedVaRead::Store(why) => format!("RESOLVED but the store refused: {why}"),
        }
    }
}

/// ★★★ **One CE submission's borrowed view of the register plane** — see
/// [`RegPlane::ce_session`], which is the only way to obtain one.
///
/// It is deliberately three capabilities and no more:
/// [`CePlane::resolve`] (the guest's own page-table walk, from the root the guest
/// published), [`CePlane::fb`] (the emulated framebuffer, which is both where the page
/// tables live and where a vidmem operand's bytes go) and [`CePlane::root`] (for the
/// report). ⊘ It exposes **no guest-RAM port**: the executor's guest-RAM side is the
/// shell's own `Vmm`, so that a copy's bytes and a completion's bytes cannot travel by two
/// different descriptions of one memory plane.
pub struct CePlane<'a> {
    state: &'a mut PlaneState,
    chip: &'static ChipProfile,
    root: crate::ceresolve::VasRoot,
    demand: crate::ceresolve::Demand,
}

impl core::fmt::Debug for CePlane<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CePlane")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

impl CePlane<'_> {
    /// The page-directory root this session walks from — `levels[0]` of the publication
    /// the guest made for its `(hClient, hVASpace)` pair.
    #[must_use]
    pub fn root(&self) -> crate::ceresolve::VasRoot {
        self.root
    }

    /// ★★★ Resolve one GPU virtual address in this session's VA space, on the demand the
    /// session was opened with.
    ///
    /// ⊘ Nothing is cached, here or anywhere below (`gmmu_publication_discipline.md` §7
    /// rule 6): every call is a fresh descent, which is what a TLB does and what makes the
    /// answer valid for this demand and no other.
    pub fn resolve(&mut self, va: u64) -> crate::ceresolve::CeResolve {
        resolve_locked(self.state, self.chip, &self.root, va, self.demand)
    }

    /// The emulated framebuffer this device backs.
    pub fn fb(&mut self) -> &mut dyn FbStore {
        self.state.fb.as_mut()
    }

    /// The framebuffer length this chip advertises — the bound a vidmem address is legal
    /// against, taken from the chip rather than from the store so the two cannot disagree.
    #[must_use]
    pub fn fb_len(&self) -> u64 {
        self.chip.fb_length
    }
}

/// The walk, with the plane's state already locked — the one body
/// [`RegPlane::resolve_published_va`] and [`RegPlane::read_published_va`] share, so the two
/// entry points cannot come to disagree about which format or which framebuffer answers.
fn resolve_locked(
    s: &mut PlaneState,
    chip: &'static ChipProfile,
    root: &crate::ceresolve::VasRoot,
    va: u64,
    demand: crate::ceresolve::Demand,
) -> crate::ceresolve::CeResolve {
    let limits = crate::ceresolve::CeAddrLimits {
        // ★ The chip's own advertised length — the same number `SparseFb` was sized from
        // and the emulated GSP answers `FB_GET_INFO` with, so the §7 rule-5 bound cannot
        // disagree with the store it bounds.
        fb_len: chip.fb_length,
        // See `CeAddrLimits`: this port has no query for the guest's RAM extent, and a
        // fabricated bound would be worse than a stated absence.
        gpa_limit: None,
    };
    let PlaneState { mmu, fb, .. } = s;
    let Some(fmt) = mmu.as_deref() else {
        return crate::ceresolve::CeResolve::NoMmuPort;
    };
    let mut src = FbStoreReader { fb: fb.as_mut() };
    crate::ceresolve::resolve(fmt, &mut src, root, va, limits, demand)
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
        RegPlane::with_objects(
            chip,
            abi,
            clock,
            kayfabe_abi::eventnotify::ProbeArmSet::default(),
            crate::ObjectLinks::default(),
        )
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
    /// `links` is [`crate::ObjectLinks`] — the object model's **two** seats in the chain
    /// (see that type for why they are two, and why only one of them can answer). This
    /// crate cannot name either link's concrete type: it has no `kayfabe-core` dependency,
    /// deliberately (*"a GSP FSM that can see the RM graph starts firing on graph state"*),
    /// so what crosses is a pair of trait objects and the port owns the choice. See
    /// [`crate::served_chain`] for where in the chain each lands and what it must not claim.
    ///
    /// # Errors
    ///
    /// As [`RegPlane::new`].
    pub fn with_objects(
        chip: &'static ChipProfile,
        abi: GspAbi,
        clock: Box<dyn NanoClock>,
        probe_arm: kayfabe_abi::eventnotify::ProbeArmSet,
        links: crate::ObjectLinks,
    ) -> Result<RegPlane, ChipError> {
        let rom = crate::rom_for(chip)?;
        let model = (chip.gsp_model)();
        assert_disjoint(chip, model.as_ref())?;
        let unserviced = crate::unserviced::UnservicedLog::new();
        let census = crate::census::ControlCensusLog::new();
        let fault_buffer = crate::faultbuffer::FaultBufferLog::new();
        let bar_pdes = BarPdeLog::new();
        let gvas_pub = GvasPubLog::new();
        // ★ The probe set is recorded into the census FIRST, and `served_policy` reads it
        // back out of the same log — so "the set the report prints" and "the set the
        // event-plane arm reads" are one stored value, not two values this line promises
        // to keep equal. The probe's history is three boots that ran without it while
        // looking armed from the launching shell; the report proving what it ran with is
        // the fix.
        census.set_probe_arm(probe_arm);
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
                    // ★ The SAME handles the plane keeps below — see `ChainLogs`' own docs
                    // for why this is a struct and not four positional arguments.
                    crate::ChainLogs {
                        unserviced: unserviced.clone(),
                        fault_buffer: fault_buffer.clone(),
                        bar_pdes: bar_pdes.clone(),
                        gvas_pub: gvas_pub.clone(),
                    },
                    census.clone(),
                    links,
                ),
                unclaimed: Vec::new(),
                fb_window: Vec::new(),
                bar0_window: Bar0Window::new(),
                cpu_intr: CpuIntrTree::new(),
                fb: Box::new(RefusingFb),
                mmu: None,
            }),
            c: PlaneCounters::default(),
            unserviced,
            census,
            fault_buffer,
            bar_pdes,
            gvas_pub,
            // ⊘ The default is a REFUSAL, not an empty sink — see `crate::RefusingDoorbell`.
            doorbell: RwLock::new(Box::new(RefusingDoorbell)),
            doorbell_log: Mutex::new(DoorbellLog::default()),
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

    /// ★★★ Install the chip's **page-table format**, replacing the refusal.
    ///
    /// # Why a port and not a [`ChipProfile`] row
    ///
    /// A `GmmuFmt` is an Axis-B seam, and the crate that implements one for a real
    /// generation is an *arch adapter* (`kayfabe_chips`), not this one. Making it a chip
    /// row would mean this crate naming that crate — the wrong direction, and the second
    /// copy of a chip's page-table format the moment a second consumer appears. As a port
    /// it is installed by the one composition root that already holds both, and the format
    /// the plane walks with is **the same type** `kayfabe_arch::Arch::mmu` answers with.
    ///
    /// ⊘ Not installing it is a **refusal**, not a default: [`NO_MMU_PORT`] says so by
    /// name at every translated access, and no arithmetic is invented in its absence.
    ///
    /// ★ Takes `&self` and the plane's lock, like [`RegPlane::set_fb`] and
    /// [`RegPlane::set_ram`].
    pub fn set_mmu(&self, mmu: Box<dyn GmmuFmt>) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.mmu = Some(mmu);
    }

    /// ★★★ **E2** — install the **usermode doorbell port**, replacing
    /// [`crate::RefusingDoorbell`].
    ///
    /// This is the seam a guest work-submit token crosses on its way to
    /// `kayfabe_rt::SharedDevice::doorbell`. Until a shell calls this, every ring is
    /// **counted and refused by name** ([`crate::NO_DOORBELL_PORT`]) rather than dropped —
    /// see [`crate::doorbell`] for why silently swallowing it is the one answer this seam
    /// must not have.
    ///
    /// ⚠ Takes the port's **own** lock, not [`RegPlane::state`]'s. See the field's docs:
    /// the port is called with no plane lock held because it can block.
    pub fn set_doorbell(&self, port: Box<dyn DoorbellPort>) {
        let mut p = self.doorbell.write().unwrap_or_else(|e| e.into_inner());
        *p = port;
    }

    /// What this device life's doorbell aperture has seen — see [`DoorbellLog`].
    #[must_use]
    pub fn doorbell_log(&self) -> DoorbellLog {
        self.doorbell_log
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Where this plane's chip puts its usermode doorbell, or `None` if it names no
    /// usermode register group. Derived — see [`crate::doorbell_reg`].
    #[must_use]
    pub fn doorbell_reg(&self) -> Option<u64> {
        crate::doorbell::doorbell_reg(self.chip)
    }

    /// What the guest has published about the bus apertures' page-table roots.
    ///
    /// ⊘ The only channel: the guest ignores this command's status
    /// (`crate::bar2`), so *"did the root arrive?"* is not answerable from a guest log.
    #[must_use]
    pub fn bar_pdes(&self) -> BarPdes {
        self.bar_pdes.pdes()
    }

    /// How many roots the guest published, and how many bodies were refused.
    #[must_use]
    pub fn bar_pde_counts(&self) -> (u64, u64) {
        (self.bar_pdes.updates(), self.bar_pdes.refusals())
    }

    /// ★★★ The publication latch itself, as a shared handle.
    ///
    /// # ⊘ Why this hands out the writable handle rather than a copy
    ///
    /// [`RegPlane::set_policy`] replaces the **whole** command chain, and the link that
    /// decodes `UPDATE_BAR_PDE` off the wire lives in that chain
    /// ([`crate::bar2::BarPdePolicy`]). A caller that replaced the chain without being able
    /// to reach this latch would **silently unroot the aperture**: the guest would go on
    /// publishing its root, nothing would receive it, and every translated access would
    /// refuse — with a correct-looking diagnosis pointing at the guest. So the latch is
    /// part of the plane's surface and not a private field, exactly as
    /// [`crate::unserviced::UnservicedLog`] is shared with the chain that writes it.
    ///
    /// ★ Read-mostly in practice. [`RegPlane::bar_pdes`] is what a *reporter* wants.
    #[must_use]
    pub fn bar_pde_log(&self) -> BarPdeLog {
        self.bar_pdes.clone()
    }

    /// ★★★ What the guest has published about its **VA spaces'** page-directory roots
    /// (`crate::gvaspub`) — every publication, with the `hClient`/`hObject` it arrived on
    /// and every `PdeLevel` it carried.
    ///
    /// ⊘ The only channel, and for a sharper reason than `bar_pdes`': these two controls
    /// are answered `NV_OK` by `crate::inittables::InitTablePolicy` and the decoded value
    /// was, until this log existed, discarded at the arm. `[measured 2026-08-08]`
    /// (`traces/real_ga106/rpc_transcript_real_ga106.txt`, the §14.9 census) they are the
    /// **only** page-directory root the boot ever sends — `SET_PAGE_DIRECTORY` occurs
    /// zero times — so a boot that does not print this prints nothing about its own
    /// address spaces.
    #[must_use]
    pub fn gvas_publications(&self) -> GvasPubSnapshot {
        self.gvas_pub.snapshot()
    }

    /// ★★★ **Resolve one GPU virtual address in a VA space the guest published** — the
    /// `crate::ceresolve` walk, over **this plane's own** framebuffer and page-table
    /// format.
    ///
    /// # ⊘ The trigger discipline is the argument, not a comment
    ///
    /// `demand` is `gmmu_publication_discipline.md` §7 rule 1: the guest event that
    /// authorised the walk. There is no constructor for a [`crate::ceresolve::Demand`]
    /// that does not name one, so this method cannot be used to prefetch — which is §6.2's
    /// refused case, and the one whose three interleavings all produce a *plausible-looking
    /// wrong physical page* rather than an error.
    ///
    /// # Why it belongs on the plane
    ///
    /// The page tables were written by the guest **through this plane's BAR2 window**, into
    /// the same [`crate::FbStore`] `bar2_phys` reads and the BAR0 moving window writes. A
    /// resolver reaching a second store would answer out of a framebuffer the guest never
    /// wrote — the exact disagreement `bar2_phys`'s own docs refuse. So the byte source is
    /// not a parameter: it is this plane's, or the answer is about a different device.
    ///
    /// ★ The framebuffer bound for §7 rule 5 comes from the **chip's own advertised
    /// length** — the same number the emulated GSP answers `FB_GET_INFO` with and the same
    /// one `SparseFb` was sized from, so the limit cannot disagree with the store.
    #[must_use]
    pub fn resolve_published_va(
        &self,
        client: u32,
        vaspace: u32,
        va: u64,
        demand: crate::ceresolve::Demand,
    ) -> crate::ceresolve::CeResolve {
        let Some(root) =
            crate::ceresolve::published_root(&self.gvas_pub.snapshot(), client, vaspace)
        else {
            return crate::ceresolve::CeResolve::NoPublication;
        };
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        resolve_locked(&mut s, self.chip, &root, va, demand)
    }

    /// ★★ **Resolve a published GPU VA and READ the bytes behind it** — resolution and
    /// access in one lock acquisition, routed to the store the **leaf's own aperture**
    /// names.
    ///
    /// # ⊘ The aperture is the router, and it is not optional
    ///
    /// A vidmem leaf is an offset into this device's framebuffer; a sysmem leaf is a guest
    /// physical address. The two are different number spaces that collide freely — at 8 GiB
    /// a GPU VA is itself a legal GPA (`execution_plane_increments.md` §8.2.3's measured
    /// warning) — so serving one out of the other's store is the *silent wrong bytes*
    /// failure the whole address plane exists to refuse. `c_ceutils_ring_resolution.md` §4
    /// measured a CeUtils finishPayload in **each** aperture within one run, so neither
    /// answer is the safe default.
    ///
    /// ⊘ Peer memory is refused: this device backs none.
    ///
    /// # Errors
    /// The resolution itself, whenever it was not [`crate::ceresolve::CeResolve::Resolved`]
    /// — returned whole so a caller reports the walk's own finding rather than "read
    /// failed" — or the store's refusal sentence.
    pub fn read_published_va(
        &self,
        client: u32,
        vaspace: u32,
        va: u64,
        buf: &mut [u8],
        demand: crate::ceresolve::Demand,
    ) -> Result<crate::ceresolve::CeResolve, PublishedVaRead> {
        let Some(root) =
            crate::ceresolve::published_root(&self.gvas_pub.snapshot(), client, vaspace)
        else {
            return Err(PublishedVaRead::Unresolved(
                crate::ceresolve::CeResolve::NoPublication,
            ));
        };
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let r = resolve_locked(&mut s, self.chip, &root, va, demand);
        let crate::ceresolve::CeResolve::Resolved { phys, aperture, .. } = r else {
            return Err(PublishedVaRead::Unresolved(r));
        };
        match aperture {
            Aperture::Vidmem => {
                s.fb.read(phys, buf)
                    .map(|()| r)
                    .map_err(|e| PublishedVaRead::Store(e.why))
            }
            Aperture::SysmemCoherent | Aperture::SysmemNonCoherent => s
                .ram
                .read(phys, buf)
                .map(|()| r)
                .map_err(|e| PublishedVaRead::Store(e.why)),
            Aperture::Peer => Err(PublishedVaRead::Store(
                "the leaf names PEER memory and this device backs no peer aperture",
            )),
        }
    }

    /// ★★★★ **The PER-LEVEL descent trace for one published VA space and one address** —
    /// [`crate::ceresolve::walk_trace`], over this plane's own format and framebuffer.
    ///
    /// ⊘ The byte source and the format are **this plane's**, exactly as
    /// [`RegPlane::resolve_published_va`] argues: a trace read through a second store would
    /// describe a different device. No [`crate::ceresolve::Demand`] — this reads page-table
    /// pages the guest already published and it produces no address any caller may act on;
    /// it produces a **sentence**.
    #[must_use]
    pub fn published_walk_trace(&self, client: u32, vaspace: u32, va: u64) -> String {
        let Some(root) =
            crate::ceresolve::published_root(&self.gvas_pub.snapshot(), client, vaspace)
        else {
            return " walk=NO-PUBLICATION".to_string();
        };
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let PlaneState { mmu, fb, .. } = &mut *s;
        let Some(fmt) = mmu.as_deref() else {
            return " walk=NO-MMU-PORT".to_string();
        };
        let mut src = FbStoreReader { fb: fb.as_mut() };
        crate::ceresolve::walk_trace(fmt, &mut src, &root, va)
    }

    /// ★★★ **Read RAW framebuffer bytes at a framebuffer-physical address** — no walk, no
    /// translation, no [`crate::ceresolve::Demand`], and **strictly an observation**.
    ///
    /// # ⊘ Why this exists, and why it is not a second data path
    ///
    /// `execution_plane_increments.md` §16.8's question is *"what does OUR framebuffer
    /// actually hold at the address the guest published?"*, and that is a question about
    /// **our own store**, not about the guest's address space. Every other reader here goes
    /// through the guest's page tables, which is exactly the thing under suspicion: the
    /// walk from the failing root descends successfully and lands on an unwritten page, and
    /// telling *"a real page-table pool we are addressing from the wrong base"* from
    /// *"noise the walk descended into"* needs the bytes themselves.
    ///
    /// ⊘ **It reads and it changes nothing** — no allocation in the store beyond what a
    /// read already implies, no cache, no effect the guest can observe. It is the same
    /// `read` the BAR0 moving window serves, addressed directly.
    ///
    /// ⚠ `buf` is caller-bounded and the caller is a diagnostic. An unwritten address
    /// inside the advertised framebuffer reads **zero and succeeds** ([`FbStore::read`]),
    /// which is precisely the fact §16.8 must be able to distinguish from *refused*.
    ///
    /// # Errors
    /// The store's own sentence when it does not back the range at all.
    pub fn fb_peek(&self, phys: u64, buf: &mut [u8]) -> Result<(), &'static str> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.fb.read(phys, buf).map_err(|e| e.why)
    }

    /// The published page-directory root of one `(hClient, hVASpace)` pair, for a report
    /// that wants to state the root it walked from. ⊘ Reading a root is not walking one;
    /// no [`crate::ceresolve::Demand`] is needed and none is implied.
    #[must_use]
    pub fn published_root(&self, client: u32, vaspace: u32) -> Option<crate::ceresolve::VasRoot> {
        crate::ceresolve::published_root(&self.gvas_pub.snapshot(), client, vaspace)
    }

    /// ★★★ **One CE submission's worth of this plane, under ONE lock acquisition**
    /// (`execution_plane_increments.md` §14.15 obstacle 3, the *"the plane hands out its
    /// stores"* half of the owner's two options).
    ///
    /// # Why a session and not more `resolve_published_va`-shaped calls
    ///
    /// Serving one guest submission needs the plane **many times**: resolve the ring, read
    /// it, resolve and read each pushbuffer range, resolve every operand run of every
    /// `LAUNCH_DMA`, write the destination bytes, resolve the finishPayload and write it.
    /// Doing that through per-call locking would take and drop the FSM mutex dozens of
    /// times *inside one guest-visible event*, so a second vCPU could re-point the BAR0
    /// window or replace the format between two steps of a single copy. A submission is
    /// one atomic thing to the guest; this makes it one to us.
    ///
    /// ⊘ **The root is resolved ONCE, before the lock**, and a channel whose
    /// `(client, vaspace)` published nothing gets `None` rather than a session that will
    /// refuse every question — the absence of a publication is a fact about the guest, not
    /// about a walk.
    ///
    /// ⚠ **Lock order.** This is the plane's own mutex and the caller must hold **no core
    /// lock** when it calls: the command-policy chain already takes the core's ranked locks
    /// *under* this mutex (`kayfabe_rmrpc::policy::Bridge`, boxed into `PlaneState::policy`),
    /// so plane→core is the established order and core→plane is its inversion. That is why
    /// the CE executor runs off `DoorbellPort::ring` — which `RegPlane::write` documents as
    /// being called with no plane lock held — rather than from inside
    /// `kayfabe_fwd::apply_pushbuffer`, which runs under a proc lock.
    pub fn ce_session<R>(
        &self,
        client: u32,
        vaspace: u32,
        demand: crate::ceresolve::Demand,
        f: impl FnOnce(&mut CePlane<'_>) -> R,
    ) -> Option<R> {
        let root = crate::ceresolve::published_root(&self.gvas_pub.snapshot(), client, vaspace)?;
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut ce = CePlane {
            state: &mut s,
            chip: self.chip,
            root,
            demand,
        };
        Some(f(&mut ce))
    }

    /// The publication latch itself, as a shared handle — for
    /// [`RegPlane::bar_pde_log`]'s reason: [`RegPlane::set_policy`] replaces the whole
    /// chain, and the link that decodes these publications lives in it.
    #[must_use]
    pub fn gvas_pub_log(&self) -> GvasPubLog {
        self.gvas_pub.clone()
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
            ptimer_writes_refused,
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
            bar2_reads,
            bar2_writes,
            bar2_faults,
            bar0_window_reads,
            bar0_window_writes,
            commands,
            faults,
            ram_refusals,
            irq_requests,
            cpu_intr_accesses,
            cpu_intr_raises,
            cpu_intr_masked,
            nonstall_raises,
            nonstall_unvectored,
            nonstall_masked,
            doorbells,
            doorbells_served,
            doorbells_refused,
        } = &self.c;
        Counters {
            reads: g(reads),
            writes: g(writes),
            boot_reg_reads: g(boot_reg_reads),
            ptimer_reads: g(ptimer_reads),
            ptimer_writes_refused: g(ptimer_writes_refused),
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
            bar2_reads: g(bar2_reads),
            bar2_writes: g(bar2_writes),
            bar2_faults: g(bar2_faults),
            bar0_window_reads: g(bar0_window_reads),
            bar0_window_writes: g(bar0_window_writes),
            commands: g(commands),
            commands_unserviced: self.unserviced.total(),
            faults: g(faults),
            ram_refusals: g(ram_refusals),
            irq_requests: g(irq_requests),
            cpu_intr_accesses: g(cpu_intr_accesses),
            cpu_intr_raises: g(cpu_intr_raises),
            cpu_intr_masked: g(cpu_intr_masked),
            nonstall_raises: g(nonstall_raises),
            nonstall_unvectored: g(nonstall_unvectored),
            nonstall_masked: g(nonstall_masked),
            doorbells: g(doorbells),
            doorbells_served: g(doorbells_served),
            doorbells_refused: g(doorbells_refused),
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
            census,
            fault_buffer,
            bar_pdes,
            gvas_pub,
            // ★ The PORT is the shell's wiring, like `ram` and `policy`; what this device
            // life SAW through it is carried out as `doorbell` just below.
            doorbell: _,
            doorbell_log,
        } = self;
        let counters = self.counters();
        let doorbell = doorbell_log
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let s = state.lock().unwrap_or_else(|e| e.into_inner());
        // ★★★ EXHAUSTIVE, for the same reason.
        let PlaneState {
            fsm,
            ram: _,
            policy: _,
            unclaimed,
            fb_window,
            bar0_window,
            // ★ Carried out as counters rather than as state: *"how many vectors did this
            // life ask for, how many could it not announce, and would any have been
            // masked"* is the question a teardown report is asked, and a snapshot of the
            // arrays would answer none of it.
            //
            // ⚠ **The old reason given here was wrong and is corrected**: it said the
            // arrays are *transient*, because *"every bit the guest sets it clears again in
            // the same ISR"*. Since §14.18 this device sets bits the guest may never clear
            // — see [`RegPlane::device_reset`], which now rebuilds the tree for exactly
            // that reason. The field is excluded because it is reported as counters, not
            // because it cannot hold anything across a life.
            cpu_intr: _,
            // ★ The PORT is the shell's wiring, like `ram` and `policy`; what it HOLDS is
            // device state and is carried out as `fb_resident_bytes` just below.
            fb,
            // ★ Also the shell's wiring: an immutable encoding table installed once
            // through `set_mmu`. Two lives of the same chip cannot differ here, and it
            // holds no guest bytes.
            mmu: _,
        } = &*s;
        PlaneResidue {
            counters,
            gsp: fsm.clone(),
            unclaimed: unclaimed.clone(),
            fb_window: fb_window.clone(),
            unserviced: unserviced.sample(),
            census: census.snapshot(),
            fault_buffers: fault_buffer.sample(),
            fault_buffers_registered: fault_buffer.total(),
            shadow_fault_buffers_registered: fault_buffer.shadow_total(),
            access_cntr_buffers_registered: fault_buffer.access_cntr_total(),
            bar0_window: *bar0_window,
            bar_pdes: bar_pdes.pdes(),
            gvas_pub: gvas_pub.snapshot(),
            fb_resident_bytes: fb.resident_bytes(),
            doorbell,
        }
    }

    /// ★★ The distinct commands nothing answered, up to
    /// [`crate::unserviced::UNSERVICED_SAMPLE_MAX`] — see that module for why the guest
    /// cannot be asked this question.
    #[must_use]
    pub fn unserviced_sample(&self) -> Vec<crate::unserviced::UnservicedCommand> {
        self.unserviced.sample()
    }

    /// ★★★ **How many distinct commands went unserviced — the truth, past
    /// [`crate::unserviced::UNSERVICED_SAMPLE_MAX`].**
    ///
    /// ⊘ Separate from [`Self::unserviced_sample`]`().len()` on purpose: that length is
    /// clamped by the sample's own cap, and reporting it as a distinct count is what let a
    /// saturated ledger read as a complete one. See
    /// [`crate::unserviced::UnservicedLog::distinct`].
    #[must_use]
    pub fn unserviced_distinct(&self) -> u64 {
        self.unserviced.distinct()
    }

    /// ★★★ The control census — what was **answered**, and with what result, including the
    /// served-but-refused class the unserviced list structurally cannot see
    /// (`crate::census`).
    #[must_use]
    pub fn control_census(&self) -> crate::census::CensusSnapshot {
        self.census.snapshot()
    }

    /// How many times the guest registered a **replayable hardware** fault buffer
    /// (`NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER`, `0x20800a9b`).
    ///
    /// ⊘⊘ **The old doc said "the recorder declines the command, so this number rising means
    /// the guest's UVM reached the registration and was REFUSED."** That stopped being true
    /// at §14.41: `crate::inittables` now answers the control and the recorder is an observer.
    /// The number still counts *asks*, but a rising count now means **served**, which is the
    /// opposite reading — and a stale sentence like that is how a report gets misread.
    #[must_use]
    pub fn fault_buffers_registered(&self) -> u64 {
        self.fault_buffer.total()
    }

    /// How many times the guest registered a **client shadow** fault buffer
    /// (`NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_CLIENT_SHADOW_FAULT_BUFFER`, `0x20800a9d`).
    #[must_use]
    pub fn shadow_fault_buffers_registered(&self) -> u64 {
        self.fault_buffer.shadow_total()
    }

    /// How many times the guest registered an **access-counter notification** buffer
    /// (`NV2080_CTRL_CMD_INTERNAL_UVM_REGISTER_ACCESS_CNTR_BUFFER`, `0x20800a1d`).
    #[must_use]
    pub fn access_cntr_buffers_registered(&self) -> u64 {
        self.fault_buffer.access_cntr_total()
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
        // ★★★ **AND THE INTERRUPT TREE** — `#151`'s state, and §14.18 is what made leaving
        // it out dangerous rather than merely untidy.
        //
        // ⚠ The old argument for skipping it (it survives in this method's absence, and in
        // `RegPlane::residue`'s note) was that *"every bit the guest sets it clears again in
        // the same ISR"*. That was true while the ONLY producer was `_osVerifyInterrupts`'
        // loopback, which clears its own bit before returning. It is **false** now: a
        // non-stall completion vector is latched by THIS device
        // ([`RegPlane::announce_completion`]) and cleared only by a guest that lives long
        // enough to run `_intrServiceNonStallLeaf_TU102`. A guest that resets between the
        // two leaves a CE completion bit set, and the next guest's very first non-stall
        // scan finds `MC_ENGINE_IDX_CE2` pending for a copy that never happened in its
        // life — a fabricated completion notification, across a device life, from the one
        // path this whole increment exists to keep honest.
        //
        // ⊘ And it is the register block's own stated reset:
        // `NV_VIRTUAL_FUNCTION_PRIV_CPU_INTR_LEAF_VALUE_INIT` and both `_EN_*_VALUE_INIT`
        // are `0x00000000` (`ogkm-580: ampere/ga102/dev_vm.h:52,56,60`), so a device that
        // came back from a power-on reset with pending bits is not modelling silicon.
        s.cpu_intr = CpuIntrTree::new();
        // ★★★ And the published roots. See `crate::bar2::BarPdeLog::device_reset` for why
        // this one is a cross-life *information* leak and not merely stale state.
        self.bar_pdes.device_reset();
        // ★★★ And the VA-space publications, for `BarPdeLog::device_reset`'s reason one
        // level up: a root that survived a device life belongs to the PREVIOUS guest's
        // address space, and the whole purpose of recording it is that something will
        // eventually follow it.
        self.gvas_pub.device_reset();
        // ★★ E2 — and the doorbell log, for the same reason: a token the PREVIOUS guest
        // rang, reported after a reset as this life's, is a false attribution in exactly
        // the direction that would make an E2 acceptance run pass on somebody else's ring.
        // ⊘ The PORT is not touched: it is the shell's wiring, and a reset is the guest's
        // event, not the composition root's.
        *self.doorbell_log.lock().unwrap_or_else(|e| e.into_inner()) = DoorbellLog::default();
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
            ReadOutcome::CpuIntr(_) => self.c.cpu_intr_accesses.fetch_add(1, Ordering::Relaxed),
            ReadOutcome::FbWindow(w) => {
                self.note_fb_window(w, off);
                self.c.fb_window_reads.fetch_add(1, Ordering::Relaxed)
            }
            ReadOutcome::Fb { window, .. } => {
                if window == FbWindow::InstanceWindow {
                    self.c.bar2_reads.fetch_add(1, Ordering::Relaxed);
                }
                self.c.fb_reads.fetch_add(1, Ordering::Relaxed)
            }
            ReadOutcome::TranslationRefused { window, .. } => {
                self.note_fb_window(window, off);
                self.c.bar2_faults.fetch_add(1, Ordering::Relaxed)
            }
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
        // ★★★ THE INTERRUPT TREE, asked before the GSP model for the same reason the
        // window latch is: `0x00B8_1000`-`0x00B8_1643` is a *register block the chip
        // declares*, and a defaulted zero out of it is not a harmless unknown — it is the
        // ISR being told "no vector is pending" at the one instant the answer decides
        // whether the adapter initialises at all. See `crate::cpuintr`.
        if bar == kayfabe_abi::pcibars::bus_bar::REGS as u8
            && let Some(reg) = crate::cpuintr::decode(off)
        {
            let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            return ReadOutcome::CpuIntr(mask(u64::from(s.cpu_intr.read(reg)), size));
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
    fn window_phys(
        &self,
        w: FbWindow,
        off: u64,
        write: bool,
        s: &mut PlaneState,
    ) -> Result<u64, WindowRefusal> {
        match w {
            FbWindow::Pramin => Ok(s.bar0_window.fb_addr(off - self.chip.pramin_window.base)),
            // ⊘ Still no address model. BAR1 is GMMU-translated like BAR2 and its root
            // arrives through the same command — but nothing in this port serves an
            // access through it, and building half of it would mean publishing a
            // translation nobody has ever exercised.
            FbWindow::FbAperture => Err(WindowRefusal::NoAddressModel),
            FbWindow::InstanceWindow => self.bar2_phys(off, write, s),
        }
    }

    /// ★★★ **THE GMMU TRANSLATION** — one aperture offset, one page walk, one framebuffer
    /// address.
    ///
    /// # ★★★ ONE STORE, TWO APERTURES — the whole point of this rung
    ///
    /// The page-table pages this walk reads come out of [`PlaneState::fb`], **the same
    /// [`FbStore`] the BAR0 moving window writes into** — because that is where the guest
    /// put them. `kbusSetupBar2GpuVaSpace_GM107` builds the whole BAR2 tree through the
    /// BAR0 window before it publishes the root, and the leaf this walk resolves is read
    /// back through that same window by `kbusVerifyBar2`. If this method reached a
    /// *second* store, the two apertures would disagree about one physical byte, which is
    /// exactly the defect the guest's own test is written to catch. [`FbStoreReader`] is
    /// the whole of the join: it is a borrow of the one store wearing the walker's trait.
    ///
    /// # What each refusal means
    ///
    /// Every arm is a **named** refusal and none of them reads zero, because a zero-filled
    /// page-table entry is a well-formed *"this maps nothing"* and a zero-filled data read
    /// is a well-formed *"the guest wrote nothing"*. Both are answers a guest acts on.
    fn bar2_phys(&self, va: u64, write: bool, s: &mut PlaneState) -> Result<u64, WindowRefusal> {
        // ★ Destructured so the format and the byte store can be borrowed at once. They
        // are different fields and the borrow checker knows it; a method call on `s`
        // would not let it.
        let PlaneState { mmu, fb, .. } = s;
        let Some(fmt) = mmu.as_deref() else {
            return Err(WindowRefusal::Translated {
                va,
                why: NO_MMU_PORT,
            });
        };
        let Some(root) = self.bar_pdes.pdes().bar2 else {
            return Err(WindowRefusal::Translated {
                va,
                why: BAR2_UNROOTED,
            });
        };
        // ★★ The level is DERIVED from the shift the guest sent beside the entry, never
        // assumed to be zero. `kbusPatchBar2Pdb_GSPCLIENT` sends
        // `pRootFmt->virtAddrBitLo` (`ogkm-580: kern_bus.c:880`) precisely because the
        // entry alone does not say which format row it belongs to, and the fields of a
        // dual entry and a single one do not overlap.
        let Some((level, geo)) = (0..MAX_FORMAT_LEVELS)
            .filter_map(|l| fmt.level_shift(l).map(|g| (l, g)))
            .find(|(_, g)| u64::from(g.shift) == root.level_shift)
        else {
            return Err(WindowRefusal::Translated {
                va,
                why: BAR2_UNKNOWN_ROOT_LEVEL,
            });
        };
        // ★★★ The guest published exactly ONE slot of that level — its own, slot 0. The
        // other slots belong to the firmware's half of the address space
        // (`ogkm-580: kern_bus.c:810-820`) and this port has published none, so an offset
        // that indexes past slot 0 has no root at all. Refused by name rather than
        // resolved against the one root we happen to hold, which would silently answer a
        // foreign address space out of this one.
        if geo.entries == 0 || (va >> geo.shift) & u64::from(geo.entries.saturating_sub(1)) != 0 {
            return Err(WindowRefusal::Translated {
                va,
                why: BAR2_OUTSIDE_PUBLISHED_SLOT,
            });
        }
        let mut src = FbStoreReader { fb: fb.as_mut() };
        let t = translate_from_entry(fmt, &mut src, level, u128::from(root.entry), va)
            .map_err(|f| WindowRefusal::Translated { va, why: f.why() })?;
        // ⊘ Vidmem only, and by NAME. The guest's page tables can legitimately name
        // system memory, and this port has a guest-RAM port that could serve it — but
        // nothing on this boot path maps sysmem through the bus window (every memdesc
        // `kbusVerifyBar2` and `kbusSetupCpuPointerForBusFlush` allocate is `ADDR_FBMEM`,
        // `ogkm-580: kern_bus_gm107.c:4050`, `kern_bus_gv100.c:66-72`), so serving it
        // would be an untested path answering a case nobody has observed.
        if t.aperture != Aperture::Vidmem {
            return Err(WindowRefusal::Translated {
                va,
                why: BAR2_FOREIGN_APERTURE,
            });
        }
        if write && t.read_only {
            return Err(WindowRefusal::Translated {
                va,
                why: BAR2_READ_ONLY,
            });
        }
        Ok(t.phys)
    }

    /// Serve one framebuffer-window read.
    fn fb_read(&self, w: FbWindow, off: u64, size: u8) -> ReadOutcome {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let phys = match self.window_phys(w, off, false, &mut s) {
            Ok(p) => p,
            Err(WindowRefusal::NoAddressModel) => return ReadOutcome::FbWindow(w),
            Err(WindowRefusal::Translated { va, why }) => {
                return ReadOutcome::TranslationRefused { window: w, va, why };
            }
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
        let phys = match self.window_phys(w, off, true, &mut s) {
            Ok(p) => p,
            Err(WindowRefusal::NoAddressModel) => {
                drop(s);
                self.note_fb_window(w, off);
                self.c.fb_window_writes.fetch_add(1, Ordering::Relaxed);
                return WriteOutcome {
                    fb_window: Some(w),
                    ..WriteOutcome::nothing()
                };
            }
            Err(WindowRefusal::Translated { va, why }) => {
                drop(s);
                self.note_fb_window(w, off);
                self.c.bar2_faults.fetch_add(1, Ordering::Relaxed);
                return WriteOutcome {
                    // ★ A FAULT, so the shell prints it at the instant the write is lost
                    // rather than leaving it to be inferred from an NV_ERR_MEMORY_ERROR
                    // ninety lines of guest code later.
                    fault: Some(BAR2_WRITE_REFUSED),
                    bar2_refusal: Some(Bar2Refused {
                        va,
                        len: usize::from(size.clamp(1, 8)),
                        why,
                    }),
                    ..WriteOutcome::nothing()
                };
            }
        };
        if w == FbWindow::InstanceWindow {
            self.c.bar2_writes.fetch_add(1, Ordering::Relaxed);
        }
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
        // ★★★ **THE USERMODE DOORBELL** (E2). Classified here — before the FSM's lock is
        // taken, before the framebuffer windows and before `decode_reg` — for two separate
        // reasons, and both of them are load-bearing.
        //
        // 1. ⚠ **No plane lock may be held.** The port calls into the core, which takes
        //    ranked locks and can block on the isolate pool's backpressure gate. Every
        //    arm below this one is inside `self.state.lock()`; a doorbell classified there
        //    would put a guest-controlled wait under the lock every other vCPU needs to
        //    read a register. This is the only classification in `write` whose POSITION is
        //    a concurrency requirement rather than a precedence one.
        // 2. ★ A write that fell through to `unclaimed_writes` would be **dropped**, and
        //    the dropped write would be the guest's entire submission — the same argument
        //    the window latch and the interrupt tree make, one aperture over.
        //
        // ⊘ `bar` is checked, not assumed. The register aperture's offset `0x00bb0090` and
        // the framebuffer aperture's offset `0x00bb0090` are the same number and different
        // facts (`PlaneState::unclaimed`'s own note), and only one of them is a ring.
        if bar == kayfabe_abi::pcibars::bus_bar::REGS as u8
            && let Some(reg) = self.doorbell_reg()
            && off == reg
        {
            return self.ring_doorbell(mask(val, size));
        }
        // ★★★ THE INTERRUPT TREE. Classified here, beside the window latch and before the
        // model's `decode_reg` gate, because the model does not own these offsets and a
        // write that fell through would be DROPPED — and the dropped write would be
        // `CPU_INTR_LEAF_TRIGGER` itself, i.e. the entire request. See `crate::cpuintr`.
        if bar == kayfabe_abi::pcibars::bus_bar::REGS as u8
            && let Some(reg) = crate::cpuintr::decode(off)
        {
            self.c.cpu_intr_accesses.fetch_add(1, Ordering::Relaxed);
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let fired = s.cpu_intr.write(reg, val as u32);
            drop(s);
            // ⊘ `out_of_range` raises NOTHING: the guest named a leaf row this chip does
            // not have, so no pending bit was latched, and a vector delivered with nothing
            // pending would send the guest's ISR looking for an interrupt that is not
            // there. It is counted as an access and no more.
            let raise = fired.is_some_and(|t| !t.out_of_range);
            if raise {
                self.c.cpu_intr_raises.fetch_add(1, Ordering::Relaxed);
            }
            if fired.is_some_and(|t| t.would_be_masked && !t.out_of_range) {
                self.c.cpu_intr_masked.fetch_add(1, Ordering::Relaxed);
            }
            return WriteOutcome {
                claimed: true,
                raise_cpu_intr: raise,
                ..WriteOutcome::nothing()
            };
        }
        // ★★★ **A WRITE TO THE FREE-RUNNING COUNTER IS REFUSED BY NAME** (`#128`).
        //
        // `tmrSetCurrentTime_GV100` writes `NV_PTIMER_TIME_0/_1`
        // (`ogkm-580: src/nvidia/src/kernel/gpu/timer/arch/volta/timer_gv100.c:56-82`), so
        // this is a write the guest's own RM really issues, not a hypothetical.
        //
        // ⚠ Before this arm the write fell through to `unclaimed_writes` and was dropped.
        // The *effect* was already right — nothing reached any counter — but the DECISION
        // was never made: it was indistinguishable from the hundreds of offsets this port
        // simply does not model yet, and it would have stayed indistinguishable when the
        // page went read-native. `register_plane_read_native.md` §5 asks for this policy
        // to be **decided explicitly**, and refuse-by-name is the house form.
        //
        // ★★★ **AND IT IS NOT REDUNDANT TODAY** — an earlier draft of this comment said it
        // was, and the claim was wrong in the flattering direction. Under `#128` the second
        // holder is `KVM_MEM_READONLY` on the backing memslot, and **that is not built yet**
        // (`read_native_timer_measured.md` §6). A third was claimed and does not exist at
        // all: RM's `NV_PROTECT_READABLE` grant covers the PTIMER page, and §2 of that
        // document is precisely that the PTIMER page CANNOT be the backing page — the page
        // that can is the usermode window, which the same whitelist leaves READ-WRITE
        // because a CUDA process has to ring its doorbell through it
        // (`ogkm-580: src/nvidia/src/kernel/gpu/subdevice/subdevice_ctrl_gpu_kernel.c:2903,
        // :2919-2926`; measured 2026-08-02 on a GA106 at revision 9087090,
        // `docs/reference/bench_evidence/timer-mappability-9087090.out`).
        //
        // ⇒ Right now this arm is the ONLY thing refusing the write. A comment that counts
        // a mechanism guarding a different page is not conservative; it invites the next
        // reader to delete the one that is load-bearing.
        //
        // ⊘ It refuses rather than emulating a settable clock. A guest that could move its
        // own counter would break the one property `#128` buys — that guest timestamps and
        // host GPU timestamps are in the same timebase.
        if bar == kayfabe_abi::pcibars::bus_bar::REGS as u8
            && (off == self.chip.ptimer.lo_off || off == self.chip.ptimer.hi_off)
        {
            self.c.ptimer_writes_refused.fetch_add(1, Ordering::Relaxed);
            return WriteOutcome {
                claimed: true,
                fault: Some(PTIMER_WRITE_REFUSED),
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

    /// ★★★ **E2** — hand one guest work-submit token to the installed port, count what
    /// came back, and report it whole.
    ///
    /// Split out of [`RegPlane::write`] so the *"no plane lock is held here"* obligation is
    /// visible in one place: nothing in this function takes [`RegPlane::state`], and the
    /// only lock it holds across the port call is the port's own [`RwLock`] read guard.
    fn ring_doorbell(&self, token: u64) -> WriteOutcome {
        // ★ Counted BEFORE the port is consulted, so `doorbells` is a statement about what
        // the GUEST did and cannot be reduced by anything the core decides.
        self.c.doorbells.fetch_add(1, Ordering::Relaxed);
        let report = {
            let port = self.doorbell.read().unwrap_or_else(|e| e.into_inner());
            port.ring(token)
        };
        match &report {
            // ★ Both servings count as served — `doorbells == served + refused` is the
            // invariant [`Counters::doorbells`] states, and a third arm that counted as
            // neither would break it silently. Which serving it was is in the report.
            DoorbellReport::Served { .. } | DoorbellReport::ServedLocally { .. } => {
                self.c.doorbells_served.fetch_add(1, Ordering::Relaxed);
            }
            DoorbellReport::Refused { .. } => {
                self.c.doorbells_refused.fetch_add(1, Ordering::Relaxed);
            }
        }
        {
            let mut log = self.doorbell_log.lock().unwrap_or_else(|e| e.into_inner());
            log.last_token = Some(token);
            if log.first_refusal.is_none()
                && let DoorbellReport::Refused { refusal, .. } = &report
            {
                log.first_refusal = Some(refusal.clone());
            }
            if let DoorbellReport::ServedLocally { note, .. } = &report {
                log.last_local_serving = Some(note.clone());
            }
        }
        // ★★★ **§14.18 — THE COMPLETION IS ANNOUNCED**, and only a completion is.
        let raise_cpu_intr = match &report {
            DoorbellReport::ServedLocally { engine, .. } => self.announce_completion(*engine),
            // ⊘ Not `Served`: a forwarded doorbell's work finishes on a HOST engine, at an
            // instant this device is not standing at, and the host's own completion path is
            // `kayfabe_completion`'s. Announcing here would be a notification for work whose
            // end we did not witness — the doorbell doctrine's own prohibition, one field
            // over. ⊘ And not `Refused`, which is the same claim with the sign flipped.
            DoorbellReport::Served { .. } | DoorbellReport::Refused { .. } => false,
        };
        WriteOutcome {
            // ★ `claimed`, because this device DOES own the offset — whatever the core then
            // decided. A doorbell reported as unclaimed would tell an operator the aperture
            // is unmodelled, which is the opposite of what happened.
            claimed: true,
            doorbell: Some(report),
            raise_cpu_intr,
            ..WriteOutcome::nothing()
        }
    }

    /// ★★★ **§14.18 — latch the non-stall vector that says an engine finished the guest's
    /// work**, and answer whether a message should be delivered.
    ///
    /// # ⊘ The precondition is the CALLER's, and it is the only one that matters
    ///
    /// This must be reached **only** from a [`DoorbellReport::ServedLocally`], i.e. after
    /// the shell's CPU copy-engine executor moved the bytes and advanced the finishPayload
    /// (`execution_plane_increments.md` §14.17). *"The completion and the notification are
    /// the same truth claim"* — a latch on any other path would be this device announcing
    /// work it did not witness, which is the exact shape [`crate::DoorbellReport`]'s own
    /// doctrine makes unrepresentable one field over.
    ///
    /// # ★★★ Why a failed lookup is a LOUD zero and not a silent one
    ///
    /// Three things can go wrong and all three mean the same thing to the guest: work
    /// happened and nothing told it. So every one of them lands in
    /// [`Counters::nonstall_unvectored`] — the counter whose healthy value is zero — rather
    /// than being folded into "did not raise". ⊘ There is deliberately no fallback vector:
    /// the obvious one would be CE0's, whose row publishes `INTR_VECTOR_INVALID`, so a
    /// default would be inventing the one answer hardware explicitly refuses to give.
    ///
    /// # ⚠ Masked is recorded, not acted on
    ///
    /// `crate::cpuintr`'s standing decision — raise regardless of the enable bits, count
    /// the disagreement — applies unchanged, and it matters more here than for the loopback
    /// trigger: the guest's non-stall scan ANDs `LEAF` with `LEAF_EN_SET`
    /// (`ogkm-580: intr_nonstall_tu102.c:344-346`), so a masked latch is a message the ISR
    /// will not attribute. [`Counters::nonstall_masked`] is what makes that one boot's
    /// finding instead of an argument.
    fn announce_completion(&self, engine: Option<u32>) -> bool {
        let Some(rm_engine_type) = engine else {
            // The guest never sent `NVA06F_CTRL_CMD_BIND` for this channel, so there is no
            // engine to name a vector — counted, never guessed.
            self.c.nonstall_unvectored.fetch_add(1, Ordering::Relaxed);
            return false;
        };
        let Ok(vector) = crate::nonstall::non_stall_vector(self.chip.intr_table, rm_engine_type)
        else {
            self.c.nonstall_unvectored.fetch_add(1, Ordering::Relaxed);
            return false;
        };
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let outcome = s.cpu_intr.latch(vector);
        drop(s);
        if outcome.out_of_range {
            // A vector the captured table publishes but this chip's `LEAF` family has no
            // row for. Nothing was latched, so nothing may be delivered — and it is the
            // same broken promise as the two above, so it is counted the same way.
            self.c.nonstall_unvectored.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        self.c.nonstall_raises.fetch_add(1, Ordering::Relaxed);
        if outcome.would_be_masked {
            self.c.nonstall_masked.fetch_add(1, Ordering::Relaxed);
        }
        true
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
    // ★★★ **E2 — THE USERMODE DOORBELL, checked against every other source.**
    //
    // It is classified FIRST in `RegPlane::write` (earlier even than the window latch,
    // because its port may block and therefore may hold no plane lock), so an overlap here
    // makes the loser **unreachable** rather than merely mis-ordered — and the loser would
    // be answered by a call into the forwarding core instead of by the register model. That
    // is the strongest form of the argument the BAR0 window register's block above makes.
    //
    // ⊘ The offset is DERIVED from the usermode register base this chip advertises to the
    // guest (`crate::doorbell_reg`), so this check also has a second edge: it refuses a
    // chip row whose advertised usermode window collides with something it also decodes,
    // which is a row the guest driver would have been TOLD to map over a live register.
    if let Some(off) = crate::doorbell::doorbell_reg(chip) {
        if off >= chip.regs_aperture_len {
            return Err(ChipError::OutsideAperture {
                off,
                aperture: chip.regs_aperture_len,
            });
        }
        if chip.boot_regs.iter().any(|r| r.off == off) {
            return Err(ChipError::OverlappingSources {
                off,
                a: "the usermode doorbell register",
                b: "a silicon-constant register",
            });
        }
        if off == chip.ptimer.lo_off || off == chip.ptimer.hi_off {
            return Err(ChipError::OverlappingSources {
                off,
                a: "the usermode doorbell register",
                b: "the free-running nanosecond counter",
            });
        }
        if off == chip.bar0_window_reg && chip.bar0_window_reg != 0 {
            return Err(ChipError::OverlappingSources {
                off,
                a: "the usermode doorbell register",
                b: "the BAR0 moving window's own register",
            });
        }
        if chip.rom_window.contains(off) {
            return Err(ChipError::OverlappingSources {
                off,
                a: "the usermode doorbell register",
                b: "the ROM window",
            });
        }
        if chip.pramin_window.contains(off) {
            return Err(ChipError::OverlappingSources {
                off,
                a: "the usermode doorbell register",
                b: "the PRAMIN framebuffer window",
            });
        }
        if model.decode_reg(0, off).is_some() {
            return Err(ChipError::OverlappingSources {
                off,
                a: "the usermode doorbell register",
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
