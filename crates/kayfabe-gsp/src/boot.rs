//! **S3 — the boot FSM**, and the transport it gates.
//!
//! States are named for what is *true*, and **every latch is a state component**. The C
//! artifact's eight scattered fields (`C: src/qemu/nvkvm_gpu_emul.c:161-179, 187-196`:
//! `bootargs_dumped, fwsec_ran, gsp_suspended, gsp_reloaded, sec_mbox0, q_ready, mbox0,
//! mbox1`) collapse into [`BootPhase`] × [`QueueState`], and all three of its one-shot
//! latches are deleted:
//!
//! | C latch | what it guarded | here |
//! |---|---|---|
//! | `bootargs_dumped` (`C:161`) | "don't re-read the boot args" | **deleted** — [`GspFsm::publish`] is idempotent by construction: it recomputes the geometry from the guest's *current* header and rebinds, which is exactly what a re-`insmod` needs |
//! | `q_ready` (`C:187`) | "the queue GPA is valid" | **replaced by a type** — [`QueueState::Bound`]. The GPA only exists inside the variant, so a stale GPA is unrepresentable |
//! | `gsp_reloaded` (`C:171`) | "distinguish re-boot from trailing teardown" | **deleted** — [`Transition::E2`]/[`Transition::E3`] distinguish on `phase` alone. The C needed the heuristic because its `gsp_suspended` was cleared unconditionally at `C:4257` *before* the classification could use it a second time |
//!
//! ## ★ The measured failure this fixes (`docs/reference/mode2_bench_lifecycle.md` §3)
//!
//! After fn-47 the teardown STARTCPU arrives with `was_suspended == true`, and the C
//! classifies it as a **re-acquire** (`C:4255-4283`): it re-raises WPR2, re-posts
//! `GSP_INIT_DONE`, and re-latches `bootargs_dumped`/`q_ready`. The next driver life then
//! points at the **previous life's queue GPA**, and because `was_suspended` is now false
//! the re-dump never fires. Observed: `msgqRxLink` timeout, **`-7`**, 71 064 retries —
//! and `-7` has exactly one cause, `rx.size != size` (`ogkm: msgq.c:386-389`), i.e. *the
//! guest's new status queue never received a tx header*. Two independent sources, one
//! mechanism.
//!
//! Here that STARTCPU is [`Transition::E2`]: `Suspending → Halted`, WPR2 stays down, and
//! **the binding is dropped by value**. The next life's mailbox write rebinds against the
//! queue that life actually allocated.
//!
//! ## What `Running` means, and why it is observed rather than assumed
//!
//! §7-G7 forbids posting a non-allowlisted event during the guest's bootup poll, which
//! runs without the API lock and hard-asserts on anything outside eight functions
//! (`ogkm: src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:1419-1440`). The plan proposed
//! gating on `phase == Running` and listed *"`Running` implies not-inside-a-boot-poll"*
//! as inferred (I9). It need not be inferred: with `MSGQ_FLAGS_SWAP_RX` the guest
//! publishes its own consumption of the status queue into the command queue's rx header
//! (`ogkm: msgq.c:416-419`), so **"the guest has drained `GSP_INIT_DONE`" is directly
//! observable** — `peer read pointer == our write pointer`. That is the edge into
//! [`BootPhase::Running`], so the gate rests on a read rather than on a belief.

use crate::element::{
    ElementLayout, IncomingRpc, MsgLen, OutgoingRpc, decode_message, encode_message, peek_len,
};
use crate::fault::GspFault;
use crate::ram::{GuestRam, RegionMap};
use crate::ring::{MsgqAbi, MsgqGeometry, RxCursor, TxCursor, available_elements, free_elements};
use crate::rpc::{Disposition, RpcAbi, RpcCommand, RpcFunction};
use kayfabe_abi::versions::DriverAbiTable;
use kayfabe_arch::{Arch, GspObservation, GspReg};

/// Where the fields of `MESSAGE_QUEUE_INIT_ARGUMENTS` are, for one driver version.
///
/// ★ The struct has **three** shapes across the three available trees, and the C parses
/// one of them: r535 has 6 fields including a lockless queue pair
/// (`nv: r535/nvrm/gsp.h:578-585`), r570 has the first 4 only
/// (`nv: r570/nvrm/gsp.h:497-502`), and 610 has the 4 plus five *geometry* fields
/// (`ogkm: src/nvidia/inc/kernel/gpu/gsp/gsp_init_args.h:32-42`). The C reads 32 bytes as
/// the first four (`C: src/qemu/nvkvm_gpu_emul.c:3411-3425`) — right for r570 and 610,
/// right by luck for r535, and right about the padding (`NvLength` is `size_t`, so there
/// are 4 pad bytes after the `u32` at +8) only because it hard-codes it.
///
/// `element_hdr_size_off` is the **capability** the plan asks for, not an assumption: on
/// 610 the guest hands us `queueElementHdrSize` itself — with the CC tag already folded
/// in (`ogkm: message_queue_cpu.c:80-86`) — so on that version even the element header
/// size is derived rather than keyed on a version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InitArgsLayout {
    /// Offset of `sharedMemPhysAddr` (`u64`).
    pub shared_mem_pa_off: usize,
    /// Offset of `pageTableEntryCount` (`u32`).
    pub pte_count_off: usize,
    /// Offset of `cmdQueueOffset` (`u64`) — a **byte offset into the region**.
    pub cmd_queue_off_off: usize,
    /// Offset of `statQueueOffset` (`u64`) — likewise.
    pub stat_queue_off_off: usize,
    /// Bytes that must be readable for the fields above.
    pub min_size: usize,
    /// Offset of `queueElementHdrSize` (`u64`), on versions that declare it.
    pub element_hdr_size_off: Option<usize>,
}

/// Everything Axis A supplies to the faked GSP.
///
/// One value, so a second driver version is a second value and never a second code path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GspAbi {
    /// The `msgq` library constants.
    pub msgq: MsgqAbi,
    /// The element header shape.
    pub element: ElementLayout,
    /// The RPC envelope + function ids.
    pub rpc: RpcAbi,
    /// `queueElementSizeMax` — `RM_PAGE_SIZE * 16` on 610
    /// (`ogkm: message_queue_cpu.c:88-89`); also declared by the guest on versions whose
    /// init-args struct carries it.
    pub element_size_max: u32,
    /// Where the init-args fields are.
    pub init_args: InitArgsLayout,
    /// The wire-layout table for the detected driver version. Keyed on the **full**
    /// `major.minor.patch`, with `NoTableForVersion` below the floor — never a
    /// nearest-neighbour fallback (`rs: crates/kayfabe-abi/src/versions.rs`).
    pub driver: DriverAbiTable,
}

/// The boot phase — what is *true* about the emulated GSP.
///
/// Ordered so that "WPR2 is up" is a range test, exactly as
/// `mode2_gsp_port_plan.md` §3.2's observable table states it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum BootPhase {
    /// Power-on, or post-reset. WPR2 down.
    #[default]
    Cold,
    /// FWSEC has run: WPR2 is up, the RISC-V core reports active.
    FwsecRan,
    /// The SEC2 Booter Load has run.
    Booted,
    /// `GSP_INIT_DONE` was posted **and drained by the guest**: the RPC plane is live and
    /// unsolicited events are allowed (§7-G7).
    Running,
    /// fn-47 has been serviced: `MAILBOX0` reports the suspend sentinel.
    Suspending,
    /// Torn down. WPR2 down, no binding.
    Halted,
}

impl BootPhase {
    /// Is WPR2 up in this phase? `phase >= FwsecRan && phase < Halted`
    /// (`ogkm: kernel_gsp_tu102.c:1172-1180` is the guest's own test, and
    /// `ogkm: kernel_gsp.c:4805-4809` is the boot gate that reads it).
    #[must_use]
    pub fn wpr2_up(self) -> bool {
        self >= BootPhase::FwsecRan && self < BootPhase::Halted
    }
}

/// A live queue binding: the geometry **and** both cursors, as one value.
///
/// §7-G1. There is no `Default`, no public field and no constructor other than
/// [`GspFsm::publish`]'s. `device_reset` drops it by value, so the GPA, the geometry,
/// both cursors and both sequence numbers die together — there is no field to leave
/// stale, because the fields do not exist outside the variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueBinding {
    geom: MsgqGeometry,
    stat: TxCursor,
    cmd: RxCursor,
    /// The peer's status-queue read pointer **as it read at the moment of the bind**.
    ///
    /// This is what makes *"the guest has linked to the header we published"* observable
    /// rather than inferred. `msgqRxLink` zeroes the read pointer on success
    /// (`ogkm: msgq.c:435-437`), so after a rebind the value in guest memory goes
    /// `previous life's value → 0 → wherever the guest drains to`. Sampling it once at
    /// bind time gives the discriminator: a later reading that **differs** from this one
    /// can only have come from the guest, so the link happened. A zero at bind time needs
    /// no discriminator, because a fresh queue and a just-linked queue read the same.
    ///
    /// Without it, a stale value that happened to equal our write pointer would read as
    /// *"the guest drained everything"* — and that is the gate on posting unsolicited
    /// events, which during the guest's boot poll is an `NV_ASSERT(0)` in the guest
    /// (`ogkm: kernel_gsp.c:1419-1440`).
    peer_read_at_bind: u32,
}

impl QueueBinding {
    /// The bound geometry.
    #[must_use]
    pub fn geometry(&self) -> &MsgqGeometry {
        &self.geom
    }

    /// Our status-queue cursor.
    #[must_use]
    pub fn status_cursor(&self) -> TxCursor {
        self.stat
    }

    /// Our command-queue cursor.
    #[must_use]
    pub fn command_cursor(&self) -> RxCursor {
        self.cmd
    }
}

/// Bound, or not. The only two possibilities, and the GPA lives in exactly one of them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum QueueState {
    /// No queue. Any doorbell is a refusal, and **no guest RAM is read**.
    #[default]
    Unbound,
    /// A validated binding.
    Bound(QueueBinding),
}

/// Which transition fired.
///
/// Recorded on every step, because a replay that never reached the transition it claims
/// to cover is the classic vacuous protocol test (`testing_doctrine.md` §1). A test
/// asserts on this list, not on a downstream side effect that several transitions share.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Transition {
    /// GSP STARTCPU from `Cold`/`Halted` → `FwsecRan`, WPR2 up.
    E1,
    /// GSP STARTCPU while `Suspending` → `Halted`. WPR2 stays down, no rebind, no
    /// re-posted `INIT_DONE` — the trailing teardown STARTCPU.
    E2,
    /// GSP STARTCPU while already booted — a no-op. Idempotent; **[inferred] I1**: ogkm
    /// has no "re-STARTCPU while running" path and the C's `!s->fwsec_ran` guard
    /// (`C:4259`) has the same effect, but neither *states* idempotency.
    E3,
    /// SEC2 STARTCPU with the Unload argument → WPR2 down, `Halted`.
    E4,
    /// SEC2 STARTCPU with a Load argument → `Booted`.
    E5,
    /// Both boot-args mailbox halves seen → publish: decode the region array, bind the
    /// queues, write the status tx header, post `GSP_INIT_DONE`.
    E6,
    /// Doorbell while bound → service the command ring.
    E7,
    /// Doorbell while **unbound** → loud refusal, zero guest-RAM reads.
    E8,
    /// fn-47 serviced → reply first, then `Suspending`.
    E9,
    /// `IRQSCLR` write → the status-queue interrupt edge is cleared.
    E10,
    /// `device_reset` → `Cold`, unbound, every counter zero.
    E11,
    /// The guest drained `GSP_INIT_DONE` → `Running`. Observed, not assumed.
    Running,
}

/// What one MMIO write did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServiceReport {
    /// Transitions that fired, in order.
    pub transitions: Vec<Transition>,
    /// Commands decoded off the command queue.
    pub commands: Vec<RpcCommand>,
    /// True if the status-queue interrupt should be announced to the guest.
    ///
    /// The FSM deliberately does not name an `IrqSpec`: interrupt delivery is the VMM
    /// port's vocabulary and the device shell owns it (see `kayfabe_arch::gsp`'s docs).
    pub raise_status_irq: bool,
}

/// What the caller wants replied to a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    /// `rpc_result`, and `rpc_result_private` with it.
    pub rpc_result: u32,
    /// The reply body. **Clamped** to the request's own body length
    /// ([`RpcCommand::reply`]).
    pub body: Vec<u8>,
}

/// How a command is answered — the seam the forwarding plane implements.
///
/// `Send` (not `Sync`), like every other seam reached exclusively through `&mut`: the
/// policy is the forwarding plane's own state, and its synchronisation is that plane's.
pub trait CommandPolicy: Send {
    /// Answer one command, or `None` to let the FSM post its own acknowledgement.
    ///
    /// Returning `None` for a [`Disposition::ReplyRequired`] command does **not** mean
    /// "no reply": the FSM posts the acknowledgement anyway, because an unanswered fn-47
    /// blocks the guest's `rmmod` for the full RPC timeout (§7-G8).
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply>;
}

/// The C artifact's own policy: echo the request back with `NV_OK`.
///
/// `C: src/qemu/nvkvm_gpu_emul.c:2410-2416` — every command that is not on the async list
/// is echoed. Kept as the default so the port's baseline behaviour is the behaviour that
/// booted a real guest, and so the forwarding plane is a *replacement* for a working
/// policy rather than the only one that exists.
#[derive(Debug, Default, Clone, Copy)]
pub struct EchoOk;

impl CommandPolicy for EchoOk {
    fn respond(&mut self, _cmd: &RpcCommand) -> Option<Reply> {
        None
    }
}

/// The faked GSP: one resettable value.
///
/// Resettability is not a feature bolted on (lesson L12): [`GspFsm::device_reset`]
/// rebuilds the value, so a test drives boot → run → teardown → boot again **inside one
/// process**, with no QEMU restart and no bench slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GspFsm {
    abi: GspAbi,
    phase: BootPhase,
    queue: QueueState,
    swgen0_pending: bool,
    /// What the boot-args mailboxes **read back** — a register shadow, so a guest that
    /// reads one gets what it wrote, exactly as hardware would.
    mailbox_lo: u32,
    mailbox_hi: u32,
    /// Which halves have been written **since the last publish or teardown** — the
    /// protocol trigger, and deliberately not the same thing as the shadow.
    ///
    /// ★ This split is a bug the composed boot test found. With one pair of fields, a new
    /// driver life's *first* mailbox write completed a pair whose other half was the
    /// PREVIOUS life's, and the FSM published a boot-args address assembled from two
    /// different lives — then published again, correctly, on the second write. Two `E6`s,
    /// the first at a mixed address. The C cannot hit this because it keys on `MAILBOX1`
    /// alone (`C:4298-4302`); the conjunction is more robust ([inferred] I4) only if the
    /// terms are cleared when the pair stops meaning anything.
    boot_args_seen: (bool, bool),
    sec2_mailbox0: u32,
    init_done_posted: bool,
    /// The status stream's next per-message sequence number.
    stat_seq: u32,
    /// The sequence number the next command must carry.
    cmd_seq: u32,
    /// `sharedMemPhysAddr` of the region the last bind used, if any — the discriminator
    /// between a re-acquire and a new driver life. See [`GspFsm::publish`].
    region_identity: Option<u64>,
    /// Bound on how many LibOS region-array entries a hostile guest can make us read.
    /// Also bounds the page-table entry count.
    max_entries: u32,
}

impl GspFsm {
    /// A cold device.
    #[must_use]
    pub fn new(abi: GspAbi) -> GspFsm {
        GspFsm {
            abi,
            phase: BootPhase::Cold,
            queue: QueueState::Unbound,
            swgen0_pending: false,
            mailbox_lo: 0,
            mailbox_hi: 0,
            boot_args_seen: (false, false),
            sec2_mailbox0: 0,
            init_done_posted: false,
            stat_seq: 0,
            cmd_seq: 0,
            region_identity: None,
            max_entries: 4096,
        }
    }

    /// The current phase.
    #[must_use]
    pub fn phase(&self) -> BootPhase {
        self.phase
    }

    /// The current queue state.
    #[must_use]
    pub fn queue(&self) -> &QueueState {
        &self.queue
    }

    /// The abstract observation the register model serves from — §3.2's table, and
    /// nothing else.
    #[must_use]
    pub fn observe(&self) -> GspObservation {
        GspObservation {
            wpr2_up: self.phase.wpr2_up(),
            riscv_active: self.phase.wpr2_up(),
            suspended: self.phase == BootPhase::Suspending,
            swgen0_pending: self.swgen0_pending,
            boot_args_lo: self.mailbox_lo,
            boot_args_hi: self.mailbox_hi,
        }
    }

    /// ★ **E11 — the reset, and it is total.**
    ///
    /// Every piece of state is reachable from the value, so the reset rebuilds the value
    /// rather than clearing fields: a field added later cannot be forgotten here. The C's
    /// reset is field-by-field at four separate sites (`C:2471-2475`, `C:4257-4258`,
    /// `C:9393-9399`, `C:3484-3485`) and they disagree.
    pub fn device_reset(&mut self) -> Transition {
        *self = GspFsm::new(self.abi);
        Transition::E11
    }

    /// Serve a register read, if this is a GSP register.
    ///
    /// `None` means the offset is not ours and another model must answer — never a
    /// defaulted zero.
    ///
    /// # Errors
    ///
    /// [`GspFault::NoGspModel`] if the architecture has no register model, or
    /// [`GspFault::RegisterUnserviceable`] if it decodes an offset it cannot serve.
    pub fn mmio_read(&self, arch: &dyn Arch, bar: u8, off: u64) -> Option<Result<u64, GspFault>> {
        let Some(model) = arch.gsp() else {
            return Some(Err(GspFault::NoGspModel));
        };
        let reg = model.decode_reg(bar, off)?;
        Some(
            model
                .encode(reg, &self.observe())
                .ok_or(GspFault::RegisterUnserviceable { reg }),
        )
    }

    /// Drive the FSM from one register write.
    ///
    /// # Errors
    ///
    /// Whatever the transition it triggers refuses with. A refusal is **per-message**:
    /// the register surface keeps answering (§7-G8).
    pub fn mmio_write(
        &mut self,
        ram: &mut dyn GuestRam,
        arch: &dyn Arch,
        policy: &mut dyn CommandPolicy,
        bar: u8,
        off: u64,
        val: u64,
    ) -> Result<ServiceReport, GspFault> {
        let Some(model) = arch.gsp() else {
            return Err(GspFault::NoGspModel);
        };
        let Some(reg) = model.decode_reg(bar, off) else {
            return Ok(ServiceReport::default());
        };
        let mut report = ServiceReport::default();
        match reg {
            GspReg::GspFalconCpuctl if model.is_startcpu(val) => {
                report.transitions.push(self.gsp_startcpu());
            }
            GspReg::Sec2FalconMailbox0 => {
                // Latched, not acted on: Load and Unload differ only by this argument.
                self.sec2_mailbox0 = val as u32;
            }
            GspReg::Sec2FalconCpuctl if model.is_startcpu(val) => {
                if model.is_booter_unload(self.sec2_mailbox0) {
                    // E4 — Booter Unload. WPR2 must read down afterwards, and the guest
                    // asserts exactly that (`ogkm: kernel_gsp_booter_tu102.c:175-187`).
                    self.enter_halted();
                    report.transitions.push(Transition::E4);
                } else if self.phase == BootPhase::FwsecRan {
                    self.phase = BootPhase::Booted;
                    report.transitions.push(Transition::E5);
                }
            }
            GspReg::GspFalconMailbox0 | GspReg::GspFalconMailbox1 => {
                if reg == GspReg::GspFalconMailbox0 {
                    self.mailbox_lo = val as u32;
                    self.boot_args_seen.0 = true;
                } else {
                    self.mailbox_hi = val as u32;
                    self.boot_args_seen.1 = true;
                }
                // ★ [inferred] I4, taking the plan's own robust reading: the pair is
                // complete when **both halves have been written since the last reset**,
                // not when a particular half arrives. The C keys on MAILBOX1
                // (`C:4298-4302`) and ogkm writes lo then hi
                // (`ogkm: kernel_gsp_tu102.c:392-403`) — a write *order*, not a protocol
                // guarantee, so keying on one half would be an assumption where a
                // conjunction costs nothing.
                if self.boot_args_seen == (true, true)
                    && matches!(self.phase, BootPhase::FwsecRan | BootPhase::Booted)
                {
                    let gpa = (u64::from(self.mailbox_hi) << 32) | u64::from(self.mailbox_lo);
                    // Consumed: the next publish needs a fresh pair, so a single later
                    // write cannot re-trigger against a half that is no longer current.
                    self.boot_args_seen = (false, false);
                    self.publish(ram, arch, gpa)?;
                    report.transitions.push(Transition::E6);
                    report.raise_status_irq = true;
                }
            }
            GspReg::GspQueueHead(_) => {
                let (t, mut r) = self.doorbell(ram, policy)?;
                report.transitions.push(t);
                report.transitions.append(&mut r.transitions);
                report.commands = r.commands;
                report.raise_status_irq |= r.raise_status_irq;
            }
            // E10 — the guest's ISR clears the edge before draining the queue
            // (`C: src/qemu/nvkvm_gpu_emul.c:4193-4200`).
            GspReg::GspFalconIrqsclr if model.is_swgen0_clear(val) => {
                self.swgen0_pending = false;
                report.transitions.push(Transition::E10);
            }
            _ => {}
        }
        Ok(report)
    }

    /// E1 / E2 / E3 — the GSP falcon STARTCPU, classified on `phase` alone.
    fn gsp_startcpu(&mut self) -> Transition {
        match self.phase {
            BootPhase::Cold | BootPhase::Halted => {
                self.phase = BootPhase::FwsecRan;
                Transition::E1
            }
            BootPhase::Suspending => {
                // ★ E2 — the trailing teardown STARTCPU. This is the one the C
                // misclassifies as a re-acquire; the binding dies here, so the next
                // driver life cannot inherit this one's queue GPA.
                self.enter_halted();
                Transition::E2
            }
            BootPhase::FwsecRan | BootPhase::Booted | BootPhase::Running => Transition::E3,
        }
    }

    /// Enter `Halted`: WPR2 down and — the load-bearing half — **the binding dropped by
    /// value**.
    fn enter_halted(&mut self) {
        self.phase = BootPhase::Halted;
        self.queue = QueueState::Unbound;
        self.init_done_posted = false;
        self.swgen0_pending = false;
        // The boot-args pair stops meaning anything when the life ends; the register
        // shadow keeps reading back what was written, as hardware would.
        self.boot_args_seen = (false, false);
    }

    /// ★ **E6 — the boot handshake.** Idempotent: it recomputes everything from the
    /// guest's *current* memory, so re-publishing is a correct rebind rather than a
    /// double-init to be latched out.
    ///
    /// Steps, in the guest's own order: walk the LibOS region array for `RMARGS`; read
    /// `MESSAGE_QUEUE_INIT_ARGUMENTS` out of it; load the region's **own page table**;
    /// bind and validate the geometry; publish the status-queue tx header; post
    /// `GSP_INIT_DONE`.
    ///
    /// # Errors
    ///
    /// A named refusal for every guest-supplied scalar that does not check out.
    fn publish(
        &mut self,
        ram: &mut dyn GuestRam,
        arch: &dyn Arch,
        boot_args_gpa: u64,
    ) -> Result<(), GspFault> {
        let model = arch.gsp().ok_or(GspFault::NoGspModel)?;
        let layout = model.libos_region_layout();

        // The region array. Bounded by the descriptor's own declared maximum, and a zero
        // entry is treated as **skip**, not stop: that the array is zero-terminated is
        // [inferred] I8 — the header declares no sentinel
        // (`ogkm: src/common/uproc/os/common/include/libos_init_args.h:31-56`), the C
        // relies on it (`C:3399-3401`), and the mechanism is only "the descriptor was
        // zeroed on allocation". The C also caps its scan at 16 (`C:3388-3407`), which is
        // a parameter it never wrote down — GSP-D9.
        // ★ Guest-OS axis: searching **by id** rather than by index is what keeps this
        // OS-agnostic. The array's contents and ordering are the CPU-side RM's, so a
        // different RM build (or a different OS's) may lay it out differently; the id8 is
        // the protocol. That the id is spelled "RMARGS" is one of the three genuinely
        // ogkm-shaped assumptions in this crate (see the crate docs) — and it fails loudly.
        let mut args_pa = None;
        let mut entry = vec![0u8; layout.entry_stride];
        for i in 0..layout.max_entries {
            let gpa = boot_args_gpa + (i as u64) * (layout.entry_stride as u64);
            ram.read(gpa, &mut entry)?;
            let id = read_u64(&entry, layout.id_offset)?;
            let pa = read_u64(&entry, layout.pa_offset)?;
            if id == layout.rmargs_id {
                args_pa = Some(pa);
                break;
            }
        }
        let args_pa = args_pa.ok_or(GspFault::RmargsRegionAbsent {
            scanned: layout.max_entries,
        })?;

        let il = self.abi.init_args;
        let mut args = vec![0u8; il.min_size];
        ram.read(args_pa, &mut args)?;
        let shared_mem_pa = read_u64(&args, il.shared_mem_pa_off)?;
        let pte_count = read_u32(&args, il.pte_count_off)?;
        let cmd_off = read_u64(&args, il.cmd_queue_off_off)?;
        let stat_off = read_u64(&args, il.stat_queue_off_off)?;
        // GSP-P1, strongest form: on a version that declares the element header size, use
        // the guest's number instead of the version key's.
        if let Some(off) = il.element_hdr_size_off {
            let declared = read_u64(&args, off)?;
            let want = self.abi.element.hdr_size() as u64;
            if declared != want {
                return Err(GspFault::DeclaredHeaderSizeMismatch {
                    declared,
                    expected: want,
                });
            }
        }

        let region = RegionMap::load(
            ram,
            shared_mem_pa,
            pte_count,
            u64::from(self.abi.msgq.region_page_size),
            self.max_entries,
        )?;

        let geom = MsgqGeometry::bind(ram, region, cmd_off, stat_off, &self.abi.msgq)?;

        // Publish the status-queue tx header. The guest's `msgqRxLink` spins on this and
        // nothing else (`ogkm: message_queue_cpu.c:368-390`).
        let hdr = geom.published_header().encode();
        geom.region().write(ram, stat_off, &hdr)?;

        // ★★ Position resets, sequence numbers usually do not — and the exception is
        // **observable**, which is what closes `mode2_gsp_port_plan.md`'s open item O3.
        //
        // `msgqRxLink` sets `rxReadPtr = 0` (`ogkm: msgq.c:435`) and nothing in the GSP
        // tree ever *assigns* `rxSeqNum` — it is only `++`'d
        // (`ogkm: message_queue_cpu.c:836`) — so a re-link resets the POSITION and
        // preserves the SEQUENCE. The C learned this the expensive way (`C:3459-3483`):
        // zeroing the sequence made the re-posted `INIT_DONE` arrive at 0 << N, the guest
        // filed it as an old package (`ogkm: message_queue_cpu.c:762, 768`) and the second
        // context hung in `kgspWaitForRmInitDone`.
        //
        // But `rxSeqNum` **is** zero-initialised once: in `GspMsgQueueInit`, which runs
        // from `kgspConstructEngine` and is torn down only in `kgspDestruct` — module
        // load and module unload. So a true `insmod` starts at 0 while an idle release
        // does not, and preserving unconditionally (the C's behaviour) would hand a fresh
        // guest a sequence number **greater** than its own, for which its receive path has
        // no recovery branch at all (`:768-782` handles only `<`).
        //
        // The discriminator is the **region**: `kgspDestruct` frees the shared memdesc, so
        // a new module load allocates a new one, while an idle release keeps
        // `MESSAGE_QUEUE_INFO` and its allocation alive (the C's own note at `C:3459-3470`
        // records exactly that). Same `sharedMemPhysAddr` ⇒ the same queue instance ⇒
        // preserve. A different one ⇒ a new instance ⇒ start at 0, as the guest did.
        let same_instance = self.region_identity == Some(shared_mem_pa);
        if !same_instance {
            self.stat_seq = 0;
            self.cmd_seq = 0;
        }
        self.region_identity = Some(shared_mem_pa);
        let count = geom.msg_count();
        let peer_read_at_bind = geom.region().read_u32(ram, geom.peer_stat_read_ptr_off())?;
        self.queue = QueueState::Bound(QueueBinding {
            geom,
            stat: TxCursor::fresh(count),
            cmd: RxCursor { read_ptr: 0 },
            peer_read_at_bind,
        });
        self.init_done_posted = false;

        let init_done = OutgoingRpc {
            function: self.abi.rpc.codes.gsp_init_done,
            sequence: 0,
            rpc_result: 0,
            rpc_result_private: 0,
            payload: Vec::new(),
        };
        self.post(ram, &init_done)?;
        self.init_done_posted = true;
        Ok(())
    }

    /// E7 / E8 — a doorbell.
    fn doorbell(
        &mut self,
        ram: &mut dyn GuestRam,
        policy: &mut dyn CommandPolicy,
    ) -> Result<(Transition, ServiceReport), GspFault> {
        // ★ E8 — the security fix, and it is placed *before* anything reads guest RAM.
        // The C parses whatever the stale `q_*` fields point at and answers `NV_OK`
        // (`C:2380-2410`), which a guest reaches by reloading its own driver
        // (`docs/reference/mode2_bench_lifecycle.md` §4: 508 log lines of
        // `cmd fn=1959520414 seq=4055862830 -> echo NV_OK`).
        if matches!(self.queue, QueueState::Unbound) {
            return Err(GspFault::QueueNotBound);
        }
        let report = self.service_command_queue(ram, policy)?;
        Ok((Transition::E7, report))
    }

    /// Drain the command queue: decode every complete message, answer it, and publish our
    /// consumption.
    ///
    /// # Errors
    ///
    /// A named refusal. The cursor is **not** advanced past a message that refused, so a
    /// refusal is visible rather than silently skipped; the loop is bounded by the
    /// available-element count either way.
    pub fn service_command_queue(
        &mut self,
        ram: &mut dyn GuestRam,
        policy: &mut dyn CommandPolicy,
    ) -> Result<ServiceReport, GspFault> {
        let mut report = ServiceReport::default();
        let QueueState::Bound(binding) = &self.queue else {
            return Err(GspFault::QueueNotBound);
        };
        let geom = binding.geom.clone();
        let count = geom.msg_count();
        let element_size = geom.element_size();

        let write_ptr = geom.region().read_u32(ram, geom.cmd_write_ptr_off())?;
        let mut read_ptr = binding.cmd.read_ptr;
        let mut expect_seq = self.cmd_seq;
        let mut avail = available_elements(write_ptr, read_ptr, count)?;

        while avail > 0 {
            let mut first = vec![0u8; element_size as usize];
            geom.region()
                .read(ram, geom.cmd_element_off(count.slot(read_ptr)), &mut first)?;
            let len = peek_len(
                &self.abi.element,
                &first,
                element_size,
                self.abi.element_size_max,
            )?;
            if len.elements() > avail {
                // The producer has not finished writing this message. The driver's own
                // receive treats a short read the same way (`NV_ERR_NOT_READY`,
                // `ogkm: message_queue_cpu.c:660-682`).
                break;
            }
            let run = self.read_run(ram, &geom, read_ptr, len, &first)?;
            let msg = decode_message(&self.abi.element, &run, len, expect_seq, &self.abi.driver)?;
            let cmd = RpcCommand::from_incoming(&self.abi.rpc, &msg);

            read_ptr = count.slot(read_ptr + len.elements()).index();
            expect_seq = expect_seq.wrapping_add(1);
            avail -= len.elements();
            self.commit_command_cursor(read_ptr, expect_seq);

            self.answer(ram, policy, &cmd, &mut report)?;
            report.commands.push(cmd);
        }

        // Publish our consumption of the command queue into the **status** queue's rx
        // header — the swapped location. Writing it to `cmd_base + rxHdrOff` instead left
        // the guest seeing zero free space and reporting *"buffer is full"* once ~63
        // elements had accumulated (`C:3352-3358`).
        geom.region()
            .write_u32(ram, geom.cmd_read_ptr_ack_off(), read_ptr)?;

        if self.maybe_enter_running(ram, &geom)? {
            report.transitions.push(Transition::Running);
        }
        report.raise_status_irq |= self.swgen0_pending;
        Ok(report)
    }

    /// Read the `len.elements()` elements of one message, following the ring's wrap.
    ///
    /// ★ GSP-D6: the C reads exactly one element and *skips* the continuation elements
    /// (`C:3341-3350` advances the read pointer past them without reading them), so a
    /// command whose payload spans elements is truncated. The extent here comes from the
    /// first copy's `rpc.length` and is bounded by `queueElementSizeMax` before any of it
    /// is read — the shape `gl11_region_arguments.md` §2.1 item 4 permits verbatim.
    fn read_run(
        &self,
        ram: &mut dyn GuestRam,
        geom: &MsgqGeometry,
        read_ptr: u32,
        len: MsgLen,
        first: &[u8],
    ) -> Result<Vec<u8>, GspFault> {
        let element_size = geom.element_size() as usize;
        let mut run = vec![0u8; len.elements() as usize * element_size];
        run[..element_size].copy_from_slice(first);
        for i in 1..len.elements() {
            let slot = geom.msg_count().slot(read_ptr + i);
            let at = i as usize * element_size;
            geom.region().read(
                ram,
                geom.cmd_element_off(slot),
                &mut run[at..at + element_size],
            )?;
        }
        Ok(run)
    }

    /// Post the reply a command earns, and run E9 if it was fn-47.
    fn answer(
        &mut self,
        ram: &mut dyn GuestRam,
        policy: &mut dyn CommandPolicy,
        cmd: &RpcCommand,
        report: &mut ServiceReport,
    ) -> Result<(), GspFault> {
        let disposition = cmd.function.disposition();
        if disposition == Disposition::NoReply {
            // 72/73 are `_issueRpcAsync` (`ogkm: rpc.c:10466`, `:10507`). Echoing them
            // surfaces in the driver as an unexpected event and desyncs the sequence.
            return Ok(());
        }
        let out = match policy.respond(cmd) {
            Some(r) => cmd.reply(r.rpc_result, &r.body),
            None => cmd.ack(0),
        };
        self.post(ram, &out)?;

        if cmd.function == RpcFunction::UnloadingGuestDriver {
            // ★ E9 — reply first, **then** suspend. `MAILBOX0` must report the suspend
            // sentinel afterwards or the guest's close poll spins
            // (`ogkm: kernel_gsp_tu102.c:352-357`). Sequence numbers are preserved: the
            // guest sent this fn-47 at the current sequence and polls for its ack there
            // (`C:2465-2472` records what zeroing them cost).
            self.phase = BootPhase::Suspending;
            report.transitions.push(Transition::E9);
        }
        Ok(())
    }

    /// Post one message to the status queue, flow-controlled.
    ///
    /// ★ **GSP-D2 — real flow control**, which the C has none of. `nvkvm_m3_post_status`
    /// (`C:1561-1620`) never reads the guest's status-queue `readPtr` — the file's
    /// complete guest-RAM read set is eight `pci_dma_read` sites and none of them is
    /// `cmdQueueBase + rxHdrOff` — so it posts unconditionally and mitigates with a proxy
    /// (*"one SWGEN0 batch outstanding"*, `C:1697-1703`).
    ///
    /// Over-posting has two distinct failures and the proxy only obscures them: the ring
    /// overwrites unconsumed elements, the guest reads a `seqNum` **greater** than its
    /// `rxSeqNum`, and the recovery branch at `ogkm: message_queue_cpu.c:768-782` handles
    /// only `seqNum < rxSeqNum` — for `>` there is no recovery, and `rxSeqNum++` still
    /// happens at `:836`, so the streams stay one apart forever.
    ///
    /// # Errors
    ///
    /// [`GspFault::QueueFull`] — **retryable**, the completion plane's back-pressure —
    /// or a named refusal of a guest-supplied pointer.
    pub fn post(&mut self, ram: &mut dyn GuestRam, rpc: &OutgoingRpc) -> Result<(), GspFault> {
        let QueueState::Bound(binding) = &self.queue else {
            return Err(GspFault::QueueNotBound);
        };
        let geom = binding.geom.clone();
        let cursor = binding.stat;
        let count = geom.msg_count();
        let element_size = geom.element_size();

        let run = encode_message(
            &self.abi.element,
            self.abi.rpc.header_version,
            element_size,
            self.abi.element_size_max,
            self.stat_seq,
            rpc,
        )?;
        let elements = (run.len() / element_size as usize) as u32;

        // The producer's cached free count first, a live read of the peer's pointer only
        // if the cache is short — `msgqTxSubmitBuffers`' own order (`ogkm: msgq.c:544-547`).
        let mut free = cursor.free_cache;
        if elements > free {
            let peer_read = geom.region().read_u32(ram, geom.peer_stat_read_ptr_off())?;
            free = free_elements(peer_read, cursor.write_ptr, count)?;
            if elements > free {
                return Err(GspFault::QueueFull {
                    needed: elements,
                    free,
                });
            }
        }

        for i in 0..elements {
            let slot = count.slot(cursor.write_ptr + i);
            let at = i as usize * element_size as usize;
            geom.region().write(
                ram,
                geom.stat_element_off(slot),
                &run[at..at + element_size as usize],
            )?;
        }
        let write_ptr = count.slot(cursor.write_ptr + elements).index();
        geom.region()
            .write_u32(ram, geom.stat_write_ptr_off(), write_ptr)?;

        self.stat_seq = self.stat_seq.wrapping_add(1);
        if let QueueState::Bound(b) = &mut self.queue {
            b.stat.write_ptr = write_ptr;
            b.stat.free_cache = free - elements;
        }
        self.swgen0_pending = true;
        Ok(())
    }

    /// Post an unsolicited event.
    ///
    /// ★ §7-G7: only once the guest is past its bootup poll, which asserts on anything
    /// outside an eight-function allowlist (`ogkm: kernel_gsp.c:1419-1440`) —
    /// `POST_EVENT` is **not** on it.
    ///
    /// # Errors
    ///
    /// [`GspFault::NotRunning`] before the guest has drained `GSP_INIT_DONE`, plus
    /// whatever [`GspFsm::post`] refuses with.
    pub fn post_event(
        &mut self,
        ram: &mut dyn GuestRam,
        rpc: &OutgoingRpc,
    ) -> Result<(), GspFault> {
        let f = self.abi.rpc.codes.classify(rpc.function);
        if self.phase != BootPhase::Running && !f.allowed_in_bootup_window() {
            return Err(GspFault::NotRunning);
        }
        self.post(ram, rpc)
    }

    /// Has the guest drained everything we posted — including `GSP_INIT_DONE`?
    fn maybe_enter_running(
        &mut self,
        ram: &mut dyn GuestRam,
        geom: &MsgqGeometry,
    ) -> Result<bool, GspFault> {
        let peer_read = geom.region().read_u32(ram, geom.peer_stat_read_ptr_off())?;
        let QueueState::Bound(b) = &mut self.queue else {
            return Ok(false);
        };
        // The link edge (see `QueueBinding::peer_read_at_bind`): either the pointer was
        // already zero when we bound — in which case a stale value cannot be confused with
        // a linked one — or it has since changed, which only the guest can have done.
        let linked = b.peer_read_at_bind == 0 || peer_read != b.peer_read_at_bind;
        // `GSP_INIT_DONE` is the first thing posted after a bind, and `msgqRxLink` starts
        // the guest's read pointer at 0 (`ogkm: msgq.c:435`), so "the pointer has moved off
        // zero" IS "the guest consumed INIT_DONE". Deliberately not "the guest drained
        // everything": a reply posted in this very service pass is not yet drained, and
        // waiting for it would keep the FSM out of `Running` for exactly as long as it
        // keeps answering.
        let drained = linked && peer_read != 0;
        if self.phase == BootPhase::Booted && self.init_done_posted && drained {
            self.phase = BootPhase::Running;
            return Ok(true);
        }
        Ok(false)
    }

    fn commit_command_cursor(&mut self, read_ptr: u32, seq_num: u32) {
        self.cmd_seq = seq_num;
        if let QueueState::Bound(b) = &mut self.queue {
            b.cmd.read_ptr = read_ptr;
        }
    }
}

fn read_u64(bytes: &[u8], off: usize) -> Result<u64, GspFault> {
    let end = off + 8;
    let slice = bytes.get(off..end).ok_or(GspFault::Truncated {
        need: end,
        have: bytes.len(),
    })?;
    let mut b = [0u8; 8];
    b.copy_from_slice(slice);
    Ok(u64::from_le_bytes(b))
}

fn read_u32(bytes: &[u8], off: usize) -> Result<u32, GspFault> {
    let end = off + 4;
    let slice = bytes.get(off..end).ok_or(GspFault::Truncated {
        need: end,
        have: bytes.len(),
    })?;
    let mut b = [0u8; 4];
    b.copy_from_slice(slice);
    Ok(u32::from_le_bytes(b))
}

kayfabe_util::assert_send_sync!(
    BootPhase,
    QueueState,
    QueueBinding,
    Transition,
    ServiceReport,
    Reply,
    GspAbi,
    InitArgsLayout,
    GspFsm,
    EchoOk,
);
kayfabe_util::assert_send!(dyn CommandPolicy);

/// Compile-time witness that [`IncomingRpc`] stays reachable from this module's public
/// surface: the decode path returns it, and a change that drops it from the API would
/// otherwise only fail at a test.
const _: fn(&IncomingRpc) -> u32 = |m| m.seq_num;
