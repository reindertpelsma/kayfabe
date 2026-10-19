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
//! and `-7` has exactly one cause, `rx.size != size`
//! (`ogkm-610: src/nvidia/src/libraries/msgq/msgq.c:386-389`,
//! `ogkm-580: src/common/shared/msgq/msgq.c:387-390`), i.e. *the guest's new status queue
//! never received a tx header*. Two independent sources, one mechanism.
//!
//! ★ The `msgq` library **moved trees** between the tags — `src/nvidia/src/libraries/msgq/`
//! at 610, `src/common/shared/msgq/` at 580 — but every routine this crate depends on is
//! byte-identical across them, off by one or two lines. `-7` is returned from nowhere else
//! in either tree. Later `msgq.c` citations here give the bare `:line` per tag; the two
//! paths are the ones spelled out above.
//!
//! Here that STARTCPU is [`Transition::E2`]: `Suspending → Halted`, WPR2 stays down, and
//! **the binding is dropped by value**. The next life's mailbox write rebinds against the
//! queue that life actually allocated.
//!
//! ## What `Running` means, and why it is observed rather than assumed
//!
//! §7-G7 forbids posting a non-allowlisted event during the guest's bootup poll, which
//! runs without the API lock and hard-asserts (`NV_ASSERT(0)`) on anything outside a short
//! allowlist. ★ **The allowlist is version-split, and the count differs**: **eight**
//! functions at 610 (`ogkm-610: src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:1419-1439`),
//! **six** at 580 (`ogkm-580: :1464-1482`). 580 admits `GSP_RUN_CPU_SEQUENCER` — and lists
//! it *first* — while 610 drops it and adds `PFM_REQ_HNDLR_STATE_SYNC_CALLBACK`,
//! `GSP_LOAD_EXEC_GENERIC_BOOTLOADER` and `GSP_LOAD_EXEC_HS_BINARY`. The five shared
//! entries are `UCODE_LIBOS_PRINT`, `GSP_LOCKDOWN_NOTICE`, `GSP_POST_NOCAT_RECORD`,
//! `GSP_INIT_DONE`, `OS_ERROR_LOG`; `POST_EVENT` is on **neither**. The plan proposed
//! gating on `phase == Running` and listed *"`Running` implies not-inside-a-boot-poll"*
//! as inferred (I9). It need not be inferred: with `MSGQ_FLAGS_SWAP_RX` the guest
//! publishes its own consumption of the status queue into the command queue's rx header
//! (`ogkm-610: msgq.c:416-419`, `ogkm-580: :417-420`), so **"the guest has drained
//! `GSP_INIT_DONE`" is directly observable** — `peer read pointer == our write pointer`.
//! That is the edge into [`BootPhase::Running`], so the gate rests on a read rather than
//! on a belief.

use crate::element::{
    ElementLayout, IncomingRpc, MsgLen, OutgoingRpc, decode_message, encode_message, max_elements,
    peek_elem_count, peek_len, peek_seq_num,
};
use crate::fault::GspFault;
use crate::ram::{GuestRam, RegionMap};
use crate::ring::{MsgqAbi, MsgqGeometry, RxCursor, TxCursor, available_elements, free_elements};
use crate::rpc::{Disposition, RpcAbi, RpcCommand, RpcFunction};
use kf_abi::NV_ERR_NOT_SUPPORTED;
use kf_abi::versions::DriverAbiTable;
use kf_abi::view::RpcEnvelope;
use kf_arch::{
    AfterSuspend, Arch, ArchBootState, BootContext, BootPhase, BootStep, GspObservation, RegWrite,
};

/// Where the fields of `MESSAGE_QUEUE_INIT_ARGUMENTS` are, for one driver version.
///
/// ★ The struct has **three** shapes across the four available trees, and the C parses
/// one of them: r535 has 6 fields including a lockless queue pair
/// (`nv: r535/nvrm/gsp.h:578-585`); r570 has the first 4 only
/// (`nv: r570/nvrm/gsp.h:497-502`) and **the bench's 580 is that same 4-field shape
/// verbatim** (`ogkm-580: src/nvidia/inc/kernel/gpu/gsp/gsp_init_args.h:29-34`); 610 has
/// those 4 plus five *geometry* fields (`ogkm-610: :32-42`). The C reads 32 bytes as
/// the first four (`C: src/qemu/nvkvm_gpu_emul.c:3411-3425`) — right for r570, 580 and
/// 610, right by luck for r535, and right about the padding (`NvLength` is `size_t`, so
/// there are 4 pad bytes after the `u32` at +8) only because it hard-codes it.
///
/// ⚠ The divergence does not stop at this struct: `MESSAGE_QUEUE_INIT_ARGUMENTS` is the
/// **first** member of `GSP_ARGUMENTS_CACHED` and grows 40 bytes at 610, so *every*
/// subsequent offset differs, and 610 additionally appends `rmStateMonitorBufferArgs` and
/// `bindataArgs` (`ogkm-580: gsp_init_args.h:45-64` vs `ogkm-610: :53-83`). Nothing here
/// reads them today; the first person who needs one must not transcribe a 610 offset.
///
/// `element_hdr_size_off` is the **capability** the plan asks for, not an assumption: on
/// 610 the guest hands us `queueElementHdrSize` itself — declared in its init args
/// (`ogkm-610: gsp_init_args.h:37`) and computed with the CC tag already folded in
/// (`ogkm-610: message_queue_cpu.c:82-86`) — so on that version even the element header
/// size is derived rather than keyed on a version.
///
/// ★ **SEAM: at 580 there is no such field on either side, so this is `None` there.** The
/// header size is the compile-time `NV_OFFSETOF(GSP_MSG_QUEUE_ELEMENT, rpc)` = 48, and the
/// CC `authTagBuffer`/`aadBuffer` sit *inside* that header rather than being appended to
/// it, so there is nothing to fold in (`ogkm-580: message_queue_priv.h:43-51, 93`).
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
    /// `queueElementSizeMax`. A **runtime field** at 610, set to `RM_PAGE_SIZE * 16`
    /// (`ogkm-610: message_queue_cpu.c:88-89`) and additionally declared by the guest in
    /// its init args on that version. At 580 it is a **compile-time constant**,
    /// `GSP_MSG_QUEUE_ELEMENT_SIZE_MIN * 16` (`ogkm-580: message_queue_priv.h:91-92`),
    /// declared nowhere on the wire. Both evaluate to 65 536; only the provenance differs,
    /// which is why this is a field here and not a constant.
    pub element_size_max: u32,
    /// Where the init-args fields are.
    pub init_args: InitArgsLayout,
    /// The wire-layout table for the detected driver version. Keyed on the **full**
    /// `major.minor.patch`, with `NoTableForVersion` below the floor — never a
    /// nearest-neighbour fallback (`rs: crates/kayfabe-abi/src/versions.rs`).
    pub driver: DriverAbiTable,
}

// ★ [`BootPhase`] moved to `kf_arch::gsp` at task #121 and is re-exported below.
// It is now read by TWO planes — the register model, whose answer for a register can
// depend on the stage, and the boot sequence, which decides what the next write means —
// and neither of them may depend on this crate. Its `ProtectedRegionUp` variant was renamed
// `ProtectedRegionUp` in the same move: see that type's docs for why one firmware's name
// for one generation's way of reaching a stage must not be the stage's name.

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
    /// (`ogkm-610: msgq.c:435-437`, `ogkm-580: :436-438`), so after a rebind the value in
    /// guest memory goes
    /// `previous life's value → 0 → wherever the guest drains to`. Sampling it once at
    /// bind time gives the discriminator: a later reading that **differs** from this one
    /// can only have come from the guest, so the link happened. A zero at bind time needs
    /// no discriminator, because a fresh queue and a just-linked queue read the same.
    ///
    /// Without it, a stale value that happened to equal our write pointer would read as
    /// *"the guest drained everything"* — and that is the gate on posting unsolicited
    /// events, which during the guest's boot poll is an `NV_ASSERT(0)` in the guest
    /// (`ogkm-610: kernel_gsp.c:1419-1439`, `ogkm-580: :1464-1482`; the assert itself is
    /// `ogkm-610: :1436`, `ogkm-580: :1479`). The allowlist it guards has eight entries at
    /// 610 and six at 580 — see the crate-level note above.
    peer_read_at_bind: u32,
    /// ★ 2026-10-14 (`docs/design/V3_S3_RESUME.md`): set only by a RESUME
    /// ([`Transition::E15`]) — the peer's status-queue read pointer when the saved binding was
    /// restored. A resume does not re-link (the guest's `rxReadPtr` continues where it stopped,
    /// `ogkm-580: kernel_gsp_tu102.c:563-570`), so "the guest consumed `GSP_INIT_DONE`" is "the
    /// pointer moved off this value", not "off zero".
    resume_mark: Option<u32>,
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
    /// GSP STARTCPU from `Cold`/`Halted` → `ProtectedRegionUp`, WPR2 up.
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
    /// Doorbell while unbound **and a binding has already existed in this device life**
    /// (`phase == Halted`) → loud refusal, zero guest-RAM reads. The stale-binding
    /// signature the bench measured.
    E8,
    /// ★ Doorbell while unbound and **no binding has ever existed in this device life** —
    /// the healthy 580 boot order, not an attack.
    ///
    /// At 580 the two init RPCs are queued by `kgspQueueAsyncInitRpcs_IMPL`
    /// (`ogkm-580: kernel_gsp.c:3753-3777`) from `kgspInitRm_IMPL` (`:4141`) — **before**
    /// `_kgspBootGspRm` (`:4184`), therefore before FWSEC, Booter Load, RISC-V start and
    /// the status-queue link — and `rpcSendMessage` rings `kgspSetCmdQueueHead_HAL`
    /// unconditionally after every submit (`:425`) with no "is the GSP up" gate in
    /// `_kgspRpcSanityCheck` (`:281-321`). So `QUEUE_HEAD(0)` is written twice while
    /// nothing is bound on **every healthy 580 boot**. At 610 the same two RPCs are sent
    /// from inside `kgspBootstrap_TU102` instead (`ogkm-610: kernel_gsp_tu102.c:576-585`,
    /// `kernel_gsp.c:4686-4709`), so the same door never rings early — which is why this
    /// must be an observed-order outcome and not a version branch.
    ///
    /// Reads **zero** guest RAM, exactly like [`Transition::E8`]; what differs is only the
    /// classification, and therefore whether a shell may escalate it.
    E12,
    /// fn-47 serviced → reply first, then `Suspending`.
    E9,
    /// ★ w828: right after [`Transition::E9`], on a regime whose firmware stops itself
    /// ([`AfterSuspend::FirmwareHalts`]) → `Halted`: WPR2 down, binding dropped, the suspend
    /// sentinel still readable. The FSP generations' counterpart of E2/E4.
    E13,
    /// `IRQSCLR` write → the status-queue interrupt edge is cleared.
    E10,
    /// `device_reset` → `Cold`, unbound, every counter zero.
    E11,
    /// ★ 2026-10-14 (`docs/design/V3_S3_RESUME.md`) — the platform's WAKE reset of a device whose
    /// life a PM-transition suspend ended ([`GspFsm::wake_reset`]): every register back to
    /// power-on (`Cold`), and the saved GSP-RM state — the queue binding, both cursors and both
    /// sequence numbers — kept, as Booter Unload's suspend/resume data keeps it on silicon.
    E14,
    /// ★ 2026-10-14 — the resume ([`BootStep::FirmwareResume`] on a saved life): `Booted`, the
    /// saved binding restored **without a publish** (no header write, no re-link, sequence numbers
    /// continue), `GSP_INIT_DONE` posted into it.
    E15,
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
    /// port's vocabulary and the device shell owns it (see `kf_arch::gsp`'s docs).
    pub raise_status_irq: bool,
    /// ★★ Commands no [`CommandPolicy`] answered, which the FSM therefore refused by name.
    ///
    /// The refusal is the right default ([`GspFsm::answer`]) but it is **quiet in the
    /// guest**: RM logs `NV_ERR_NOT_SUPPORTED` at `LEVEL_INFO`. Without this, *"what has
    /// this port not built yet"* is answerable only one boot at a time, which is the
    /// difference between reporting a list and discovering it over a week.
    ///
    /// ⊘ Function ids only. Deciding *which control* a `GSP_RM_CONTROL` carried means
    /// decoding a body, and this stage does not decode bodies (see `crate::rpc`'s module
    /// docs); the crate that owns the control vocabulary attaches that.
    pub unserviced: Vec<Unserviced>,
}

/// ★★★★★ §16.76 — what one call to [`GspFsm::deliver_events`] did.
///
/// ⊘ **Five variants and not an `Option<usize>`**, and the reason is the one this port has
/// been bitten by repeatedly: *"no events were delivered"* is four different findings whose
/// next moves are completely different — nobody registered one, the previous batch is still
/// outstanding, the boot never reached `Running`, or the ring refused us. A single number
/// would make all four read as the same silence, and the gate in particular is a state that
/// can be *stuck*, which is the one an operator must be able to see by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventDelivery {
    /// No os-event has been registered, so there is nobody to wake (`C:1851-1853`).
    NoneRegistered,
    /// ★ The previous batch has not been drained — [`GspFsm::deliver_events`]'s gate held.
    ///
    /// Healthy in steady state (it is what bounds the ring to one batch) and **the symptom
    /// to look for** if delivery stops: a gate opened only by an `IRQSCLR` stays shut
    /// forever if the guest never takes the interrupt.
    Gated,
    /// The guest has not drained `GSP_INIT_DONE`, so `POST_EVENT` — which is on neither
    /// driver tag's bootup-window allowlist — must not be posted yet.
    NotRunning,
    /// A batch was posted and the gate is now closed until the guest clears the edge.
    Delivered {
        /// How many `POST_EVENT` messages landed in the status queue.
        posted: usize,
        /// Set when the batch was cut short — the ring refused the rest. The elements that
        /// did land are still announced; see [`GspFsm::deliver_events`] for why.
        short: Option<GspFault>,
    },
    /// Nothing landed at all. The gate is left OPEN: there is no batch to drain.
    Failed {
        /// Why the first post was refused.
        fault: GspFault,
    },
}

/// One command the FSM had to refuse because nothing offered an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unserviced {
    /// The wire function id, as sent.
    pub code: u32,
    /// `rpc.sequence`, so a refusal can be tied to a recorder's stream position.
    pub sequence: u32,
}

/// What the caller wants replied to a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    /// The envelope `rpc_result`. A non-zero value is a HARD STOP for the guest: `_issueRpcAndWait`
    /// returns it verbatim and nothing else in the reply is read (`ogkm-595.84: rpc.c:2008-2025`).
    pub rpc_result: u32,
    /// ★ The envelope `rpc_result_private`, when it must differ from `rpc_result`. `None` (every
    /// reply but one) means "same value", which is what the guest has always been sent. Only fn 1's
    /// down-negotiation sets it: OK in `rpc_result` and a private status that makes the guest read
    /// the host's pair out of the body and retry (`ogkm-595.84: rpc.c:8947-8968`).
    pub rpc_result_private: Option<u32>,
    /// The reply body. **Clamped** to the request's own body length
    /// ([`RpcCommand::reply`]).
    pub body: Vec<u8>,
}

impl Reply {
    /// The reply as it goes on the wire for `cmd`: body clamped to the request
    /// ([`RpcCommand::reply`]), and the private status split from `rpc_result` only when the policy
    /// asked for it.
    #[must_use]
    pub fn on_wire(&self, cmd: &RpcCommand) -> OutgoingRpc {
        let mut out = cmd.reply(self.rpc_result, &self.body);
        if let Some(private) = self.rpc_result_private {
            out.rpc_result_private = private;
        }
        out
    }
}

/// ★★★ One reply the FSM is holding until the page-table refresh that its command triggered
/// has reached the host. See [`Reply::hold_for_refresh`].
#[derive(Debug, Clone, PartialEq, Eq)]
struct HeldReply {
    rpc: OutgoingRpc,
    /// ★ P5b: the host act this reply's STATUS waits on ([`CommandPolicy::defers`]), if any.
    deferred: Option<Deferred>,
    /// The command was a `GSP_RM_ALLOC` — a failed outcome is stamped into its params status too.
    alloc: bool,
    /// ★ v3-refusals: what the command carried, for [`crate::refusal::RefusalLedger`].
    detail: Option<u32>,
}

/// ★★★ **A reply whose STATUS is the outcome of a host act still in progress** (P5b).
///
/// The answer to a channel's allocation IS its birth (`THE_ARCHITECTURE_v3.md` §7), and a birth
/// is several host ioctls (~4 ms). Run on the register drainer while it holds the GSP lock, that
/// is blocking under a lock (owner invariant). ⇒ The link hands the act to a non-vCPU thread and
/// the FSM HOLDS the reply — exactly the RPC-map sync point's shape — until the act resolves this
/// cell; [`GspFsm::release_held`] then posts it with the act's status: `0` posts the reply as
/// built, anything else posts it as that refusal (envelope result, and for an alloc the params
/// `status` the guest actually reads, `ogkm-580: rpc.c:11236-11241`).
///
/// ⊘ The guest cannot race it: a GSP client's RPCs are synchronous, so nothing the guest sends
/// about this object can arrive before the reply does.
///
/// ★ x11-dispsw review (2026-10-03, LOW): the act runs while the links BEHIND its link are still
/// deciding the same command — the object seat can refuse the alloc (a handle the guest already
/// uses for another object) after the act built a host object for it. The act's owner may attach
/// an UNDO ([`Self::on_orphaned`]); [`GspFsm::release_held`] runs it, once, exactly when the act
/// succeeded and the reply posted is a refusal — the case it used to only log as orphaned.
#[derive(Clone)]
pub struct Deferred {
    outcome: std::sync::Arc<core::sync::atomic::AtomicU64>,
    undo: std::sync::Arc<std::sync::Mutex<Option<OrphanUndo>>>,
    /// ★ EXPERIMENT `KF3_GSS_NATIVE`: the HOST's reply bytes, written into the held reply when the
    /// act succeeded ([`Self::set_reply_patch`]). `None` for every other act.
    patch: std::sync::Arc<std::sync::Mutex<Option<ReplyPatch>>>,
}

/// ★ EXPERIMENT `KF3_GSS_NATIVE`: bytes an act wants in its held reply's payload, at `at`. Applied by
/// [`GspFsm::release_held`] only when the act resolved `NV_OK` and only inside the payload the reply
/// already has (the reply is clamped to the request's own size, `RpcCommand::reply`): a patch never
/// grows a reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyPatch {
    /// Offset into the reply payload.
    pub at: usize,
    /// The bytes.
    pub bytes: Vec<u8>,
}

/// What undoes a deferred act whose command another link refused ([`Deferred::on_orphaned`]).
/// Runs on the register drainer under the GSP lock, so it must not block: hand the work to the
/// act's own thread.
pub type OrphanUndo = Box<dyn FnOnce() + Send>;

impl Deferred {
    const PENDING: u64 = u64::MAX;

    /// A pending outcome.
    #[must_use]
    pub fn new() -> Deferred {
        Deferred {
            outcome: std::sync::Arc::new(core::sync::atomic::AtomicU64::new(Self::PENDING)),
            undo: std::sync::Arc::new(std::sync::Mutex::new(None)),
            patch: std::sync::Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// ★ EXPERIMENT `KF3_GSS_NATIVE`: the bytes the held reply carries when the act succeeds. Set
    /// BEFORE [`Self::resolve`] (the reply may be posted the moment the cell resolves). A second
    /// call replaces the first.
    pub fn set_reply_patch(&self, at: usize, bytes: Vec<u8>) {
        if let Ok(mut p) = self.patch.lock() {
            *p = Some(ReplyPatch { at, bytes });
        }
    }

    /// The reply patch, if one was set (a copy: a post that fails is retried).
    #[must_use]
    pub fn reply_patch(&self) -> Option<ReplyPatch> {
        self.patch.lock().ok().and_then(|p| p.clone())
    }

    /// Resolve with an `NV_STATUS` (`0` = the act succeeded). The first resolution wins.
    pub fn resolve(&self, status: u32) {
        let _ = self.outcome.compare_exchange(
            Self::PENDING,
            u64::from(status),
            core::sync::atomic::Ordering::AcqRel,
            core::sync::atomic::Ordering::Acquire,
        );
    }

    /// The outcome, once resolved.
    #[must_use]
    pub fn outcome(&self) -> Option<u32> {
        match self.outcome.load(core::sync::atomic::Ordering::Acquire) {
            Self::PENDING => None,
            s => Some(s as u32),
        }
    }

    /// Attach the act's undo: run once if the act succeeds and the command's reply is a refusal
    /// another link made ([`GspFsm::release_held`]). A second call replaces the first.
    pub fn on_orphaned(&self, undo: impl FnOnce() + Send + 'static) {
        if let Ok(mut u) = self.undo.lock() {
            *u = Some(Box::new(undo));
        }
    }

    /// Run the attached undo, if any; `true` when one ran. At most once per cell (clones share it).
    pub fn run_orphan_undo(&self) -> bool {
        let undo = self.undo.lock().ok().and_then(|mut u| u.take());
        undo.map(|f| f()).is_some()
    }
}

impl core::fmt::Debug for Deferred {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Deferred")
            .field("outcome", &self.outcome())
            .field(
                "undo",
                &self.undo.lock().map(|u| u.is_some()).unwrap_or(false),
            )
            .finish()
    }
}

impl Default for Deferred {
    fn default() -> Self {
        Self::new()
    }
}

impl PartialEq for Deferred {
    fn eq(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.outcome, &other.outcome)
    }
}
impl Eq for Deferred {}

/// ★ What [`GspFsm::release_held`] does with a held reply's deferred act: `false` = still pending
/// (the reply stays held); `true` = the reply may be posted as `rpc` now reads. A failed act's
/// status becomes the reply's (and an alloc's params `status`); a SUCCEEDED act under a reply
/// another link refused runs its undo ([`Deferred::on_orphaned`]) — or, with none attached, is
/// named orphaned. Idempotent across a post that fails and is retried: the undo runs once.
fn settle_deferred(rpc: &mut OutgoingRpc, d: &Deferred, alloc: bool) -> bool {
    match d.outcome() {
        None => false,
        Some(0) if rpc.rpc_result != 0 => {
            let undone = d.run_orphan_undo();
            eprintln!(
                "kayfabe: HELD-REPLY fn={:#x} seq={}: its deferred act SUCCEEDED but the reply is a refusal ({:#x}) — {}",
                rpc.function,
                rpc.sequence,
                rpc.rpc_result,
                if undone {
                    "its owner's undo was queued"
                } else {
                    "the act's object is orphaned"
                }
            );
            true
        }
        Some(0) => {
            // ★ EXPERIMENT `KF3_GSS_NATIVE`: the host's bytes, only inside the payload the reply has.
            if let Some(p) = d.reply_patch()
                && let Some(end) = p.at.checked_add(p.bytes.len())
                && let Some(dst) = rpc.payload.get_mut(p.at..end)
            {
                dst.copy_from_slice(&p.bytes);
            }
            true
        }
        Some(status) => {
            rpc.rpc_result = status;
            rpc.rpc_result_private = status;
            if alloc {
                stamp_alloc_status(rpc, status);
            }
            true
        }
    }
}

/// `rpc_gsp_rm_alloc_v03_00.status` — the `[OUT]` field a GSP client reads as the alloc's
/// result when the transport status is not `NV_OK` (`ogkm-580: g_rpc-structures.h:1491-1502`,
/// `rpc.c:11236-11241`: `status = rpc_params->status`).
pub const RM_ALLOC_PARAMS_STATUS_AT: usize = 16;

/// ★ Stamp a refusal into an alloc reply's params `status`. ⊘ Without it a refused alloc reads as
/// SUCCESS in the guest: `rpcRmApiAlloc_GSP` replaces the transport status with the params status
/// (`rpc.c:11236-11241`), which an echoed request carries as `0` — `[measured kf3m2]` every
/// refused `NV01_MEMORY_VIRTUAL` "succeeded" in the guest and was later freed as an unknown object.
pub fn stamp_alloc_status(rpc: &mut OutgoingRpc, status: u32) {
    if status == 0 {
        return;
    }
    if rpc.payload.len() < RM_ALLOC_PARAMS_STATUS_AT + 4 {
        rpc.payload.resize(RM_ALLOC_PARAMS_STATUS_AT + 4, 0);
    }
    rpc.payload[RM_ALLOC_PARAMS_STATUS_AT..RM_ALLOC_PARAMS_STATUS_AT + 4]
        .copy_from_slice(&status.to_le_bytes());
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

    /// ★★★★★ **MAY THIS COMMAND'S REPLY BE DELIVERED YET?**
    ///
    /// `true` means *"this command changed the address space; hold its reply until the
    /// page-table refresh has reached the host"*. Owner, 2026-09-10: *"You should only
    /// return from a TLB invalidate, the RPC map call or the kernel emulated channel for
    /// UVM after the PTE/PDB page table refresh function finished."*
    ///
    /// ⊘ **A separate question from [`CommandPolicy::respond`], deliberately.** `respond`
    /// says WHAT the answer is; this says WHEN it may be delivered. Folding it into `Reply`
    /// would have put a delivery concern in 54 construction sites that have no opinion on
    /// it, and every one of them would have had to say `false` to stay silent.
    ///
    /// ⊘ **This is not the vCPU waiting.** The vCPU posts nothing and returns from its trap;
    /// the guest is already spinning in its own `rpcRecvPoll`, which is what an RPC reply is
    /// for. [`GspFsm::release_held`] posts it from the publication worker — the same shape
    /// as the invalidate, which holds `TRIGGER` and completes off-thread.
    ///
    /// ⚠ **Why, measured `[w402]`.** Six host faults in one boot, all `FAULT_PDE`, all at
    /// addresses our table already declared: four worker threads' copy sources, `P3`'s
    /// release semaphore, and the fault this campaign had called a harmless bystander. Every
    /// one IS published — after the engine touched it. Five failures, one cause: a map call
    /// returned before its rows existed on the host.
    ///
    /// The default is `false`: a policy that has not thought about it must not accidentally
    /// hold a reply, because a held reply nobody releases is a guest hang.
    fn holds_for_refresh(&self, _cmd: &RpcCommand) -> bool {
        false
    }

    /// ★ P5b: **does this command's reply STATUS wait on a host act?** ([`Deferred`]). Asked
    /// right after [`CommandPolicy::respond`]; `Some` holds the reply until the cell resolves.
    /// The default is `None`: a policy that has not thought about it holds nothing.
    fn defers(&mut self, _cmd: &RpcCommand) -> Option<Deferred> {
        None
    }
}

/// ★★★ **A link that SEES every command and cannot answer one** — the seam for a fact the
/// guest states inside a control that some *other* link is the correct answerer for.
///
/// # Why this is a second trait and not a `CommandPolicy` that promises to return `None`
///
/// A device policy chain is a `find_map`: the first `Some` terminates it. So a link that
/// must observe a control whose *answer* belongs to a link further down has to be seated
/// **ahead** of the answerer — at which point *"and it always declines"* is the only thing
/// standing between an observer and a silently re-routed reply.
///
/// This port has already paid for the difference between an obligation that is *stated* and
/// one that is *checked*: `kayfabe_device::served_chain`'s object-link seat carries the
/// sentence *"the link must not claim more than it serves … the obligation is stated here
/// and `tests/served_chain_objects.rs` is where it is checked"*, and that file **has never
/// existed** — the obligation was load-bearing, well written, cited by a second crate, and
/// unchecked for its whole life.
///
/// ⇒ This trait has **no return value**. An observer cannot answer, cannot re-route a
/// reply and cannot change a byte the guest reads, because there is nothing for it to
/// return — the property is `rustc`'s rather than a reviewer's. `gates_quantified_over_a_list`
/// in its other direction: the strongest gate is the one that cannot be spelled wrong.
///
/// `Send` for [`CommandPolicy`]'s reason exactly: it is held by the FSM across vCPU threads.
pub trait CommandObserver: Send {
    /// Look at one command. Whatever this learns, it cannot be an answer.
    ///
    /// ⊘ Called for **every** command, before any link has answered — so an implementation
    /// owes its own gate on the ones it means to observe, and must be cheap on the rest.
    fn observe(&mut self, cmd: &RpcCommand);
}

/// The adapter that seats a [`CommandObserver`] in a [`CommandPolicy`] chain.
///
/// ★ `respond` is *literally* `{ self.0.observe(cmd); None }` — the whole type. It is here,
/// beside the two traits, rather than in the crate that builds chains, so that the one
/// place an observer becomes a policy is the one place both traits are declared.
pub struct Observing(pub Box<dyn CommandObserver>);

impl CommandPolicy for Observing {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        self.0.observe(cmd);
        // ⊘ Unconditional, and the only `return` in the type. See [`CommandObserver`].
        None
    }
}

impl core::fmt::Debug for Observing {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Observing").finish_non_exhaustive()
    }
}

/// A safe default policy: echo the request back with `NV_OK`.
///
/// `C: src/qemu/nvkvm_gpu_emul.c:2410-2416` is the C's generic path — every command that is
/// not on the async list is echoed — and this reproduces it, so the port's baseline is the
/// behaviour that booted a real guest and the forwarding plane is a *replacement* for a
/// working policy rather than the only one that exists.
///
/// ★★ **CORRECTED 2026-07-29 by the trace differential — this is NOT a faithful model of
/// the C, and the doc used to claim it was.** Replaying `cap1` against this policy
/// (`kayfabe-crec`) produced two replies that agree with the C on every field a guest
/// matches on — slot, `seqNum`, `function`, `sequence`, `rpc_result` **and `rpc.length`** —
/// and differ in the **body**. The C special-cases at least `GET_GSP_STATIC_INFO` (fn 65),
/// splicing a captured GA106 `GspStaticConfigInfo` blob into the reply
/// (`C: src/qemu/nvkvm_gpu_emul.c:3434-3452`), and its `GSP_RM_CONTROL` (fn 76) replies are
/// sized by the control's own response rather than echoed. Whether a guest *notices* is a
/// question about each control's semantics, not about the transport, and it is
/// [`crate::CommandPolicy`]'s job — but "`EchoOk` is what the C does" was measurably wrong
/// and is now measurably narrower.
///
/// ## ★★★ Not a production policy, and here is the sharpest reason why
///
/// This answers **every** command `NV_OK` with the request's own body reflected. For a
/// control whose params are `[OUT]` the request body is zeros, so the guest is handed a
/// plausible, well-formed, entirely fictional answer — which is the C's measured
/// cudart-initialisation failure exactly: the library read `0` where it expected data and
/// aborted **silently**, the rejection living in the reply *payload* rather than in a
/// status or a log line (`C: src/qemu/nvkvm_gpu_emul.c:3335-3360`).
///
/// On the Mode-2 transport that failure is worse than the C ever observed, for a class of
/// control the guest treats specially. A control the guest's RM considers GSP-implemented
/// gets a dedicated cache path — set on a successful reply
/// (`ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:11098-11103`, `ogkm-610: :10903-10908`)
/// and consulted before the next send, which then returns without reaching the wire
/// (`ogkm-580: rpc.c:10962-10971`, `ogkm-610: :10766-10775`). So an `NV_OK` answer is not
/// one bad reply; it can be **persisted in the guest and served forever**, invisibly to
/// any recorder on this side, because the traffic stops arriving. Whether it persists is
/// decided by flag fields *the replying GSP fills in the reply body* — `EchoOk` avoids it
/// only by the accident of reflecting a request in which the guest zeroed them itself.
///
/// ★★ **The accident, spelled out, because it is the whole of this type's defence and it
/// has two holes.** `rpcRmApiControl_GSP` writes `rpc_params->rmctrlFlags = 0;
/// rpc_params->rmctrlAccessRight = 0;` before every send (`ogkm-580: rpc.c:10994-10995`,
/// `ogkm-610: :10799-10800`) and the GSS-legacy cache branch's first test is
/// `!(flags & RMCTRL_FLAGS_CACHEABLE_ANY) -> NV_FALSE`
/// (`ogkm-580: src/nvidia/src/kernel/rmapi/rmapi_cache.c:152-158`, `ogkm-610: :152-158`).
/// So the echo reflects zero and nothing sticks — **provided the sender is stock**. A guest
/// that sets those bits itself gets them straight back. And the *other* cache branch,
/// `bCacheable`, is decided inside the guest from its own export table before the RPC is
/// even sent (`ogkm-580: rpc.c:10900-10913`), so no reply — echo or otherwise — can prevent
/// it. `kayfabe_device::sticky` carries both readings and closes the half that is ours;
/// this type is outside that guard because nothing installs it in the port.
///
/// ## ★★★ And the guest does NOT always zero them — measured on hardware, 2026-07-31
///
/// The paragraph above assumes an `[OUT]` request arrives zeroed. It does not always.
/// Run `t126b` on the bench (a stock 580.159.04 guest at `f2acb89`, driven with
/// `nvidia-smi`) reached `kbusInitBarsSize_KERNEL`, whose params are an **uninitialised
/// stack struct** — `NV2080_CTRL_BUS_GET_PCI_BAR_INFO_PARAMS params;` with no
/// `portMemSet` (`ogkm-580: src/nvidia/src/kernel/gpu/bus/kern_bus.c:585`). RM serialises
/// it into the RPC as-is, the echo hands it straight back, and the caller then trusts
/// `params.pciBarCount` as a loop bound over an eight-element array
/// (`kern_bus.c:595-598`). The guest kernel took a page fault reading off the end of its
/// own `CONFIG_VMAP_STACK` stack, twice on two fresh boots:
///
/// ```text
/// BUG: unable to handle page fault for address: ffffd4a44035c000
/// RIP: 0010:kbusInitBarsSize_KERNEL+0x90/0x110 [nvidia]
///  kbusStatePreInitLocked_GM107+0x73/0xe0 [nvidia]
///  RmInitAdapter+0x12f2/0x1e80 [nvidia]
/// ```
///
/// So the echo is not merely *uninformative* for an `[OUT]` control; for a control whose
/// caller does not pre-zero, it is an **out-of-bounds read the guest performs on its own
/// kernel stack** at our reply's instruction. Reflecting a request is never safe by
/// default, and this is the measured proof of it.
///
/// ## ★★★ So it stopped being a default — task #127
///
/// It is no longer reachable by *not deciding*. [`GspFsm::answer`] answers a command no
/// policy claimed with a non-zero **envelope** `rpc_result`, which short-circuits the guest
/// ahead of both the copy-out and the control cache (`ogkm-580: rpc.c:1994`,
/// `ogkm-610: :2012`); `kayfabe_rmrpc::BridgeRefusal::GspRuleControlUnserviced` carries the
/// whole argument for why that is the right status.
///
/// ⊘ This type therefore had to change shape: it now returns `Some` **explicitly**, because
/// `None` means *"I decline"* and no longer means *"echo"*. Nothing in the port installs it;
/// it is the C-baseline transport proof (`kayfabe_crec`'s `cap1` replay drives it, and the
/// bytes it produces are unchanged), and the fixture a test uses to show that the baseline
/// would be wrong — `tests/gsp_boot.rs`'s
/// `a_command_no_policy_answers_is_refused_by_name_rather_than_echoed` runs both arms side
/// by side, which is the only way to see that they differ.
#[derive(Debug, Default, Clone, Copy)]
pub struct EchoOk;

impl CommandPolicy for EchoOk {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        // ★★ `Some`, explicitly, and NOT `None`. `None` used to mean "echo" because the
        // FSM's own fallback was one; task #127 made that fallback a named refusal, so a
        // policy that wants the C's behaviour must now say so. That is the whole point of
        // the change: echoing is a thing a caller opts into, in one place, with this
        // type's docs attached — never something a caller gets by not deciding.
        Some(Reply {
            rpc_result_private: None,
            rpc_result: 0,
            body: cmd.payload.clone(),
        })
    }
}

/// Several policies, tried in order; the first `Some` wins.
///
/// ★ The seam is *one* policy, and that is right — the FSM must not know how many things
/// have an opinion. But a port grows one answered command at a time, and folding each new
/// one into the previous policy would make an unrelated pair share a type and a state.
/// So composition lives here, as data: a policy that answers nothing returns `None` and
/// the next is asked, and a chain that answers nothing at all leaves the FSM to refuse by
/// name ([`GspFsm::answer`]) — which is the safe end of the chain, not [`EchoOk`].
///
/// ⚠ Order is precedence, and it is the caller's to declare. Two policies claiming one
/// function is not detectable from here — it is a contradiction in the *port*, and the
/// crate that installs them is where a test can name both.
pub struct PolicyChain {
    links: Vec<Box<dyn CommandPolicy>>,
}

impl PolicyChain {
    /// Build a chain. Earlier links take precedence.
    #[must_use]
    pub fn new(links: Vec<Box<dyn CommandPolicy>>) -> PolicyChain {
        PolicyChain { links }
    }

    /// How many links the chain has.
    #[must_use]
    pub fn len(&self) -> usize {
        self.links.len()
    }

    /// Whether the chain is empty — in which case every command is refused by name.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.links.is_empty()
    }
}

impl core::fmt::Debug for PolicyChain {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PolicyChain")
            .field("links", &self.links.len())
            .finish()
    }
}

impl CommandPolicy for PolicyChain {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        self.links.iter_mut().find_map(|p| p.respond(cmd))
    }

    /// ⊘ ANY link may hold. The chain's `respond` is a `find_map` and stops at the first
    /// answerer, but an observer seated ahead of it can be the one that knows the address
    /// space moved — so this is an `any`, not a "whoever answered".
    fn holds_for_refresh(&self, cmd: &RpcCommand) -> bool {
        self.links.iter().any(|p| p.holds_for_refresh(cmd))
    }

    /// ⊘ Any link may defer (the channel link is seated ahead of the answerer), and at most one
    /// does per command; every link is asked so none keeps a stale cell for the next command.
    fn defers(&mut self, cmd: &RpcCommand) -> Option<Deferred> {
        let mut out = None;
        for p in &mut self.links {
            if let Some(d) = p.defers(cmd) {
                out.get_or_insert(d);
            }
        }
        out
    }
}

/// The faked GSP: one resettable value.
///
/// Resettability is not a feature bolted on (lesson L12): [`GspFsm::device_reset`]
/// rebuilds the value, so a test drives boot → run → teardown → boot again **inside one
/// process**, with no QEMU restart and no bench slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GspFsm {
    // Lifetime budget survives guest device resets; host option only.
    reply_body_trace: crate::replytrace::ReplyBodyTrace,
    /// ★★★★★ **w432 — HOW MANY COMMAND DOORBELLS ARE WAITING FOR A WORKER.**
    ///
    /// `[measured]` servicing the GSP command queue is the **worst trap in the device**:
    /// `at=bar0+0x110c00` — `NV_PGSP_QUEUE_HEAD(0)` — held a vCPU for **1.79 s**, because one
    /// guest register write drains and services every queued RPC, host verbs included, before
    /// the store returns.
    ///
    /// Owner: *"MMIO writes primarily touch a queue to register that a write happened there
    /// and wake a coordinator"*, and *"a trap may not take longer than a millisecond"*. So
    /// [`BootStep::CommandDoorbell`] ALWAYS increments this instead of servicing, and the register
    /// drainer calls [`Self::service_one_deferred_command`].
    ///
    /// ⊘⊘ **v3 (w826): deferral is the ONLY mode.** The old tree had a `defer_commands` flag,
    /// default OFF "so every existing test keeps the synchronous behaviour", which the device
    /// armed at attach — and which a guest power-on reset once silently disarmed (w472b). A flag
    /// whose default is the 1.79 s vCPU trap is the defect with a switch on it; there is no switch.
    ///
    /// ⊘ A COUNT, not a flag. Two writes before the worker runs are two queue states to
    /// drain, and collapsing them to a bool would service once and leave the second guest
    /// submission in a queue nothing looks at again.
    ///
    /// ⚠ The guest misses nothing by waiting: it POLLS the message queue for its reply, which
    /// is what real GSP makes it do. What changes is that it polls in its own loop rather
    /// than inside one held store.
    pending_command_doorbells: u32,
    /// Replies withheld until their rows are on the host — [`Reply::hold_for_refresh`].
    /// ⊘ A `Vec` and not a map: it is ordered, it is single-digit in every measured boot,
    /// and the order replies are posted in is the order the guest asked for them.
    held: Vec<HeldReply>,
    /// ★★★ v3-refusals: every non-`NV_OK` reply posted, keyed by what it answered
    /// ([`crate::refusal`]). Written at the two posting sites only; answers nothing.
    refusals: crate::refusal::RefusalLedger,
    /// ★ w827: the guest's last `NV_PRISCV_RISCV_BCR_CTRL` write ([`GspReg::GspRiscvBcrCtrl`]).
    bcr_ctrl: Option<u32>,
    /// ★ 2026-10-14: the guest's last `NV_PGSP_FALCON_ENGINE` write ([`GspReg::GspFalconEngine`]).
    falcon_engine: Option<u32>,
    /// ★ w828: how the life that is running now STOPS after fn-47 — latched from the regime that
    /// STARTED the processor ([`BootStep::StartProcessor`]), because the FSM holds no model
    /// between writes and the doorbell that carries fn-47 is serviced without one.
    after_suspend: AfterSuspend,
    /// ★ w828: the firmware halted itself after suspending ([`Transition::E13`]) — the suspend
    /// sentinel stays readable in `Halted`, as a register keeps its value until the next reset.
    /// Cleared by the next STARTCPU.
    fw_halted_after_suspend: bool,
    abi: GspAbi,
    phase: BootPhase,
    queue: QueueState,
    /// ★ Ruling 2: a `GSP_RM_CONTROL` larger than one message, joined here before any policy
    /// sees it and answered in as many replies as it arrived in ([`crate::large`]).
    large: crate::large::Assembler,
    swgen0_pending: bool,
    /// ★★★★★ §16.76 — **the os-event flow-control gate**, and it is a SECOND flag beside
    /// [`GspFsm::swgen0_pending`] rather than a reuse of it.
    ///
    /// # What it gates, in the C's own words
    ///
    /// `C: src/qemu/nvkvm_gpu_emul.c:1854-1863`, whose comment calls it CRITICAL:
    ///
    /// > *"the status queue is SHARED with RPC responses and has strictly monotonic
    /// > per-message seqNums in a small ring. Posting an event batch on every doorbell
    /// > (hundreds of times) overflows the ring before the guest drains it → the guest's
    /// > `rpcRecvPoll` sees a seqNum gap (\"Bad sequence number\") and the whole RPC path
    /// > breaks."*
    ///
    /// So the rule is **post-when-drained, post-all, raise-once**, which bounds the
    /// outstanding event messages to one batch. [`GspFsm::deliver_events`] is the rule.
    ///
    /// # ⊘⊘ WHY IT IS NOT `swgen0_pending`, and this is the refutation that mattered
    ///
    /// The brief for this rung said we hold *"the same flag with the opposite polarity"*.
    /// **We do not — we hold a DIFFERENTLY-SCOPED flag with the same name**, and porting
    /// the C's gate onto it would have wedged delivery shut permanently:
    ///
    /// - In the C, `gsp_swgen0_pending` is written in exactly one place,
    ///   `nvkvm_gsp_raise_swgen0` (`C:1830`), which is reachable only from
    ///   `nvkvm_gsp_deliver_events` (`C:1868`). An ordinary RPC reply — `nvkvm_m3_post_status`
    ///   — **never sets it**. It therefore means *"an EVENT BATCH is outstanding"*.
    /// - Here, [`GspFsm::post`] sets `swgen0_pending` on **every** post, and `answer` posts
    ///   every RPC reply. It therefore means *"we have written something to the status
    ///   queue"*, which is true after essentially every doorbell.
    ///
    /// ⇒ Gating on `swgen0_pending` would have refused the FIRST event batch (an RPC reply
    /// always precedes it in the same service pass) and reopened only on an `IRQSCLR` —
    /// which the guest writes only in response to an interrupt we would, by then, never
    /// have raised. A wedge, shipped as a fix, with a green test suite.
    ///
    /// ⊘ And `swgen0_pending` is deliberately left alone: it is the `IRQSTAT` **register
    /// shadow** the guest reads back (`kayfabe_device::ga10x`'s `GspFalconIrqstat` arm), and
    /// narrowing it to event batches would make that register lie about a queue that does
    /// have a message in it.
    ///
    /// Both are cleared by the same opener, [`Transition::E10`], because both describe
    /// state the guest's ISR has just finished acting on.
    events_outstanding: bool,
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
    /// ★ Generation-local boot state, stored here and **never interpreted here**.
    ///
    /// This field replaces `sec2_mailbox0`, which was one boot regime's latch — the
    /// secure-booter argument — living as a named field on the arch-independent value.
    /// Whatever a generation's [`kf_arch::BootSequence`] must remember between
    /// writes goes in here, which means [`GspFsm::device_reset`] clears it for free.
    arch_state: ArchBootState,
    init_done_posted: bool,
    /// The status stream's next per-message sequence number.
    stat_seq: u32,
    /// The sequence number the next command must carry.
    cmd_seq: u32,
    /// ★★ Our position in the **command** ring, kept beside [`GspFsm::cmd_seq`] because
    /// it is the same instance's state and restarts exactly when that does.
    ///
    /// The status queue's positions reset on a rebind and its sequence does not, because
    /// `msgqRxLink` assigns `rxReadPtr = 0` and never assigns `rxSeqNum`
    /// (`ogkm-580: src/common/shared/msgq/msgq.c:436` — the assignment inside `msgqRxLink`
    /// itself, `ogkm-610: src/nvidia/src/libraries/msgq/msgq.c:435`; `rxSeqNum` is only
    /// ever `++`'d, `ogkm-580: message_queue_cpu.c:782`, `ogkm-610: :836`).
    /// **The command queue is the mirror image**:
    /// there is no re-link on the producing side, so nothing resets the guest's tx
    /// `writePtr` short of `msgqTxCreate`, which only runs from `_gspMsgQueueInit` at
    /// module load (`ogkm-580: message_queue_cpu.c:155-161`). An idle-release re-acquire
    /// therefore rebinds against a ring whose producer is at, say, 4 — and a consumer
    /// that restarted at 0 re-reads four already-answered commands and then refuses
    /// forever on [`GspFault::SeqNumGap`], because their sequence numbers are behind.
    ///
    /// Found by B4's drain-on-publish: before it, the first *doorbell* after a re-acquire
    /// hit exactly the same wall, but no test rang one.
    cmd_read_ptr: u32,
    /// ★ 2026-10-14 (`docs/design/V3_S3_RESUME.md`) — the binding a fn-47 suspend ended, kept
    /// **inert** when the life entered `Halted` from `Suspending` (E2/E4/E13): nothing reads guest
    /// RAM through it (a doorbell while `Halted` is still [`Transition::E8`], zero reads). Only a
    /// resume ([`Transition::E15`]) of a life whose fn 47 was a PM transition
    /// ([`GspFsm::note_pm_suspend`]) brings it back; a cold boot (E1), a publish (E6) or a reset
    /// (E11) discards it.
    suspended: Option<QueueBinding>,
    /// ★ 2026-10-14 — the fn 47 that ended this life said `bInPMTransition` (and not GC6): the
    /// device decodes the body and tells the FSM ([`GspFsm::note_pm_suspend`]).
    pm_suspend: bool,
    /// `sharedMemPhysAddr` of the region the last bind used, if any — the discriminator
    /// between a re-acquire and a new driver life. See [`GspFsm::publish`].
    ///
    /// ⚠ **PC-D7, a named trade-off**: an address is not an identity, and the guest's
    /// allocator recycles pages. A destruct → construct cycle onto the same page reads as
    /// *"same instance"* and yields a permanent [`GspFault::SeqNumGap`]. Always resetting
    /// instead is **worse** — `task #64`, and the C's own `C: src/qemu/nvkvm_gpu_emul.c`
    /// at `C:3459-3483`. See [`GspFsm::publish`] for the full statement and for what would
    /// actually dissolve it.
    region_identity: Option<u64>,
    /// Bound on how many LibOS region-array entries a hostile guest can make us read.
    /// Also bounds the page-table entry count.
    max_entries: u32,
    /// ★★★ v3-initrace: the DMEM image of the ucode about to start ([`BootStep::FwsecCommand`]) —
    /// consumed by the next STARTCPU.
    fwsec_image: Option<u64>,
    /// … the image of the FWSEC that raised WPR2, until [`GspFsm::resolve_frts_command`] reads it,
    frts_image: Option<u64>,
    /// … and the FRTS offset its command named, served while WPR2 is up
    /// ([`GspObservation::frts_offset`]). Cleared when WPR2 comes down.
    frts_offset: Option<u64>,
    /// ★ The boot display (`docs/design/V3_DISPLAY.md` §4.11.4 row 2): where fn 72's body is kept
    /// for fn 65's encoder ([`crate::sysinfo`]). `None`: fn 72 is dropped unread, as before the boot
    /// display — kf3's posture with `gop=off`, which since 2026-10-03 (`v3-gop`) attaches no cell
    /// (`kf_qemu::gop::ConsoleWiring`; before that every kf3 device carried one).
    /// ⊘ Survives [`GspFsm::device_reset`]: it is the chain's, not one driver life's.
    system_info: Option<crate::sysinfo::SystemInfoCell>,
}

/// ★★★ v3-initrace — **FWSEC's FRTS command, read out of the DMEM image RM loaded it from.**
///
/// The image is OUR generated ROM's FWSEC (`kf_abi::vbios::GENERATED_FWSEC`, served to every
/// falcon-boot family by `kf_chip::bar0::vbios_profile`), which RM copied to guest RAM and patched
/// (`s_vbiosPatchInterfaceData`, `ogkm-580: kernel_gsp_frts_tu102.c:156-262`): the DMEM mapper's
/// `init_cmd` (+44, `:81-99`) is `CMD_FRTS` (0x15, `:100`) and the command buffer at
/// `cmd_in_buffer_offset` holds `FWSECLIC_FRTS_CMD` — a 24-byte `FWSECLIC_READ_VBIOS_DESC`, then
/// `frtsRegionDesc {version=1, size=20, frtsRegionOffset4K, frtsRegionSize=0x100 (1 MiB),
/// frtsRegionMediaType=FB (2)}` (`:103-130`, filled at `:319-327`).
///
/// `dmem0` is the guest-physical address of DMEM physical offset 0 ([`BootStep::FwsecCommand`]);
/// the image's data is loaded at `dmem_phys_base`. Every word read lies inside the image's DMEM
/// load (`dmem_load_size`), at offsets fixed by the ROM we generated — never at an offset the
/// guest wrote — and the image's own mapper must agree with it. `None` for any other command
/// (FWSEC-SB), a refused read, or a descriptor FWSEC would not accept: the model then derives.
fn read_frts_command(ram: &mut dyn GuestRam, dmem0: u64) -> Option<u64> {
    let g = kf_abi::vbios::GENERATED_FWSEC;
    let image = dmem0.checked_add(u64::from(g.dmem_phys_base))?;
    let mut rd = |off: u32| -> Option<u32> {
        if off.checked_add(4)? > g.dmem_load_size {
            return None;
        }
        let mut b = [0u8; 4];
        ram.read(image.checked_add(u64::from(off))?, &mut b).ok()?;
        Some(u32::from_le_bytes(b))
    };
    // FALCON_APPLICATION_INTERFACE_DMEM_MAPPER_V3: cmd_in_buffer_offset +8, init_cmd +44.
    if rd(g.dmem_mapper_offset.checked_add(8)?)? != g.cmd_in_buffer_offset {
        return None;
    }
    if rd(g.dmem_mapper_offset.checked_add(44)?)? != FWSEC_CMD_FRTS {
        return None;
    }
    let desc = g.cmd_in_buffer_offset.checked_add(24)?;
    let (version, size) = (rd(desc)?, rd(desc + 4)?);
    let (offset_4k, size_4k, media) = (rd(desc + 8)?, rd(desc + 12)?, rd(desc + 16)?);
    (version == 1
        && size == 20
        && size_4k == FRTS_REGION_SIZE_1MB_IN_4K
        && media == FRTS_REGION_MEDIA_FB)
        .then_some(u64::from(offset_4k) << 12)
}

/// `FALCON_APPLICATION_INTERFACE_DMEM_MAPPER_V3_CMD_FRTS` (`ogkm-580: kernel_gsp_frts_tu102.c:100`).
const FWSEC_CMD_FRTS: u32 = 0x15;
/// `FWSECLIC_FRTS_REGION_SIZE_1MB_IN_4K` (`:125`).
const FRTS_REGION_SIZE_1MB_IN_4K: u32 = 0x100;
/// `FWSECLIC_FRTS_REGION_MEDIA_FB` (`:124`).
const FRTS_REGION_MEDIA_FB: u32 = 2;

impl GspFsm {
    /// A cold device.
    #[must_use]
    pub fn new(abi: GspAbi) -> GspFsm {
        GspFsm {
            reply_body_trace: crate::replytrace::ReplyBodyTrace::from_host_env(),
            pending_command_doorbells: 0,
            held: Vec::new(),
            refusals: crate::refusal::RefusalLedger::new(),
            bcr_ctrl: None,
            falcon_engine: None,
            after_suspend: AfterSuspend::AwaitsTeardownUcode,
            fw_halted_after_suspend: false,
            abi,
            phase: BootPhase::Cold,
            queue: QueueState::Unbound,
            large: crate::large::Assembler::default(),
            swgen0_pending: false,
            events_outstanding: false,
            mailbox_lo: 0,
            mailbox_hi: 0,
            boot_args_seen: (false, false),
            arch_state: ArchBootState::default(),
            init_done_posted: false,
            stat_seq: 0,
            cmd_seq: 0,
            cmd_read_ptr: 0,
            region_identity: None,
            suspended: None,
            pm_suspend: false,
            max_entries: 4096,
            fwsec_image: None,
            frts_image: None,
            frts_offset: None,
            system_info: None,
        }
    }

    /// ★ Keep every `GSP_SET_SYSTEM_INFO` (fn 72) body in `cell` ([`crate::sysinfo`]) — it is still
    /// answered with nothing, as an asynchronous RPC must be; the cell is what fn 65's encoder reads
    /// the guest's `consoleMemSize` from.
    #[must_use]
    pub fn with_system_info_cell(mut self, cell: crate::sysinfo::SystemInfoCell) -> GspFsm {
        self.system_info = Some(cell);
        self
    }

    /// ★★★ v3-initrace — **read the FRTS command of the FWSEC that raised WPR2**, once, and serve
    /// WPR2 at the offset it names ([`read_frts_command`]). Returns `Some(what was read)` when a
    /// command was pending, `None` when there was nothing to do.
    ///
    /// ⊘ **A device act, called after the write that started FWSEC and before the registers are
    /// published** (the guest reads `WPR2_ADDR_LO` only after it sees FWSEC halted, and `HALTED`
    /// is published last). It is NOT inside [`GspFsm::mmio_write_with`] on purpose: the C artifact
    /// never read this command, so a replay that must close over the C's own guest-RAM reads
    /// (`kf-crec`'s `cap1`) would stop at it; without it the FSM serves the derivation — exactly
    /// what the C served.
    pub fn resolve_frts_command(&mut self, ram: &mut dyn GuestRam) -> Option<Option<u64>> {
        if !self.phase.wpr2_up() {
            return None;
        }
        let image = self.frts_image.take()?;
        self.frts_offset = read_frts_command(ram, image);
        Some(self.frts_offset)
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

    /// The **command** sequence we next expect from the guest — the other half of the
    /// cursor a service pass may commit.
    ///
    /// ★ Exposed so a test can assert what a *failed* pass committed. PC-D1 is precisely a
    /// question about this number: a pass that consumed a command it never answered has
    /// advanced it, and that is not visible from [`GspFsm::queue`] alone. See
    /// `tests/tests/service_pass_atomicity.rs`.
    #[must_use]
    pub fn command_seq(&self) -> u32 {
        self.cmd_seq
    }

    /// Whether command progress could supply heartbeat metadata (initial test seam).
    #[must_use]
    pub fn has_current_system_info(&self) -> bool {
        self.command_seq() != 0
    }

    /// The abstract observation the register model serves from — §3.2's table, and
    /// nothing else.
    #[must_use]
    pub fn observe(&self) -> GspObservation {
        GspObservation {
            stage: self.phase,
            wpr2_up: self.phase.wpr2_up(),
            riscv_active: self.phase.wpr2_up(),
            suspended: self.phase == BootPhase::Suspending || self.fw_halted_after_suspend,
            swgen0_pending: self.swgen0_pending,
            boot_args_lo: self.mailbox_lo,
            boot_args_hi: self.mailbox_hi,
            riscv_bcr_ctrl: self.bcr_ctrl,
            falcon_engine: self.falcon_engine,
            frts_offset: if self.phase.wpr2_up() {
                self.frts_offset
            } else {
                None
            },
        }
    }

    /// ★ **E11 — the reset, and it is total.**
    ///
    /// Every piece of state is reachable from the value, so the reset rebuilds the value
    /// rather than clearing fields: a field added later cannot be forgotten here. The C's
    /// reset is field-by-field at four separate sites (`C:2471-2475`, `C:4257-4258`,
    /// `C:9393-9399`, `C:3484-3485`) and they disagree.
    pub fn device_reset(&mut self) -> Transition {
        // ⊘ w472b's fix (carrying a `defer_commands` flag across the reset) is gone with the flag:
        // deferral is structural in v3, so a reset cannot disarm it.
        let system_info = self.system_info.take();
        let body_trace = std::mem::take(&mut self.reply_body_trace);
        *self = GspFsm::new(self.abi);
        self.system_info = system_info;
        self.reply_body_trace = body_trace;
        Transition::E11
    }

    /// ★ 2026-10-14 (`docs/design/V3_S3_RESUME.md`) — **the fn 47 that ended this life was a PM
    /// transition** (`bInPMTransition`, not GC6), decoded by the device from the body. Recorded
    /// only while that fn 47's suspend is current: `Suspending` (the falcon regime, before its
    /// teardown STARTCPU) or `Halted` with the binding already saved (a regime whose firmware
    /// halts itself, E13). Returns whether it was recorded.
    pub fn note_pm_suspend(&mut self) -> bool {
        let current = self.phase == BootPhase::Suspending
            || (self.phase == BootPhase::Halted && self.suspended.is_some());
        if current {
            self.pm_suspend = true;
        }
        current
    }

    /// ★ 2026-10-14 — a saved GSP-RM life a resume can restore: a PM-transition fn 47 ended it and
    /// its binding is kept ([`GspFsm::note_pm_suspend`]).
    #[must_use]
    pub fn resumable(&self) -> bool {
        self.pm_suspend && self.suspended.is_some()
    }

    /// ★★ **E14 — the platform's WAKE reset** (QEMU's `RESET_TYPE_WAKEUP`, `hw/i386/pc.c:1723-1727`)
    /// of a device whose life a PM-transition suspend ended and whose firmware has stopped
    /// (`Halted`). `Some(E14)`: every register is back to power-on (`Cold`: WPR2 down, the
    /// processor not running, the mailboxes and latches clear) and the saved GSP-RM state is
    /// kept — the binding, both cursors, both sequence numbers and the region identity — as
    /// Booter Unload's suspend/resume data keeps it across D3cold on silicon
    /// (`_kgspGetBooterUnloadArgs`, `ogkm-580: kernel_gsp_tu102.c:589-606`). `None`: nothing to
    /// keep — the caller resets totally ([`GspFsm::device_reset`], E11). ⊘ Any other phase
    /// (a suspend that never reached its teardown, a running device) is never kept.
    pub fn wake_reset(&mut self) -> Option<Transition> {
        if self.phase != BootPhase::Halted || !self.resumable() {
            return None;
        }
        let suspended = self.suspended.take();
        let (stat_seq, cmd_seq, cmd_read_ptr, region_identity) = (
            self.stat_seq,
            self.cmd_seq,
            self.cmd_read_ptr,
            self.region_identity,
        );
        let refusals = std::mem::take(&mut self.refusals);
        self.device_reset();
        self.suspended = suspended;
        self.pm_suspend = true;
        self.stat_seq = stat_seq;
        self.cmd_seq = cmd_seq;
        self.cmd_read_ptr = cmd_read_ptr;
        self.region_identity = region_identity;
        self.refusals = refusals;
        Some(Transition::E14)
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
        self.mmio_read_with(model, bar, off)
    }

    /// [`GspFsm::mmio_read`], given the register model directly.
    ///
    /// ★★ **Why this variant exists, and it is not convenience.** The register plane needs
    /// a *register model* and nothing else — it never calls `mmu()`, `userd()`,
    /// `pushbuffer()` or `classify()`. Requiring a whole [`Arch`] to serve one register
    /// makes the device shell either carry an architecture it has no implementation for, or
    /// compose one out of `kayfabe-mocks`, whose own manifest says *"never a production
    /// dependency"*. Neither is a thing to ship, and neither is a statement about the GSP.
    ///
    /// [`GspFsm::mmio_read`] is kept, unchanged, as the one-line wrapper — every existing
    /// caller and the whole trace differential still go through it.
    ///
    /// `None` means the offset is not this model's and another must answer — never a
    /// defaulted zero.
    ///
    /// # Errors
    ///
    /// [`GspFault::RegisterUnserviceable`] if the model decodes an offset it cannot serve.
    pub fn mmio_read_with(
        &self,
        model: &dyn kf_arch::GspModel,
        bar: u8,
        off: u64,
    ) -> Option<Result<u64, GspFault>> {
        if let Some(reg) = model.decode_reg(bar, off) {
            return Some(
                model
                    .encode(reg, &self.observe())
                    .ok_or(GspFault::RegisterUnserviceable { reg }),
            );
        }
        // ★ The shared vocabulary declined. Before treating the offset as somebody else's,
        // ask this generation's own boot sequence: a generation whose boot is driven by
        // registers no [`GspReg`] variant names still has to *answer* them, and their
        // answers are functions of the boot state this FSM holds. The order is fixed —
        // model first — so a sequence can never shadow a shared register.
        model
            .boot_sequence()
            .on_read(model, bar, off, &self.boot_context(), &self.arch_state)
            .map(Ok)
    }

    /// What a [`kf_arch::BootSequence`] is allowed to look at.
    fn boot_context(&self) -> BootContext {
        BootContext {
            obs: self.observe(),
            boot_args_seen: self.boot_args_seen,
        }
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
        self.mmio_write_with(ram, model, policy, bar, off, val)
    }

    /// [`GspFsm::mmio_write`], given the register model directly. See
    /// [`GspFsm::mmio_read_with`] for why the variant exists.
    ///
    /// # Errors
    ///
    /// Whatever the transition it triggers refuses with. A refusal is **per-message**:
    /// the register surface keeps answering (§7-G8).
    pub fn mmio_write_with(
        &mut self,
        ram: &mut dyn GuestRam,
        model: &dyn kf_arch::GspModel,
        policy: &mut dyn CommandPolicy,
        bar: u8,
        off: u64,
        val: u64,
    ) -> Result<ServiceReport, GspFault> {
        // ★★★ **The flow, and it is now arch-independent.** Decode with the generation's
        // register model, ask the generation's boot SEQUENCE what the write means, and
        // apply the steps. This function used to be a `match` over `GspReg` whose arms
        // *were* the falcon/secure-booter ordering — so a generation reaching the same
        // truths through different registers could only be added by editing them. It no
        // longer names a register at all.
        //
        // ⚠ `reg` is an `Option` on purpose and the `None` is **not** an early return any
        // more: a generation whose boot is driven by offsets the shared vocabulary cannot
        // name sees them here and nowhere else. Sequences that live inside the vocabulary
        // answer `BootSteps::none()` for a `None`, which is exactly the old early return.
        let w = RegWrite {
            bar,
            off,
            val,
            reg: model.decode_reg(bar, off),
        };
        // ★ w827: a register the model answers from what was written (`GspRiscvBcrCtrl`).
        if w.reg == Some(kf_arch::gsp::GspReg::GspRiscvBcrCtrl) {
            self.bcr_ctrl = Some(val as u32);
        }
        // ★ 2026-10-14: likewise the GSP reset FSM's register (`GspReg::GspFalconEngine`).
        if w.reg == Some(kf_arch::gsp::GspReg::GspFalconEngine) {
            self.falcon_engine = Some(val as u32);
        }
        let ctx = self.boot_context();
        let steps = model
            .boot_sequence()
            .on_write(model, &w, &ctx, &mut self.arch_state);
        if steps.overflowed() {
            return Err(GspFault::BootStepOverflow);
        }
        let mut report = ServiceReport::default();
        for step in steps {
            self.apply(ram, model, policy, step, &mut report)?;
        }
        Ok(report)
    }

    /// Perform one [`BootStep`]. The FSM's whole vocabulary of effects.
    ///
    /// ★ Each arm is the *same* code the corresponding `match reg` arm used to run — the
    /// transitions, their guards and their ordering inside a report are unchanged, which
    /// is what makes "GA10x behaves bit-identically" a claim about a moved body rather
    /// than a rewritten one.
    fn apply(
        &mut self,
        ram: &mut dyn GuestRam,
        model: &dyn kf_arch::GspModel,
        policy: &mut dyn CommandPolicy,
        step: BootStep,
        report: &mut ServiceReport,
    ) -> Result<(), GspFault> {
        match step {
            BootStep::FwsecCommand(dmem0) => {
                // ★ v3-initrace: consumed by the StartProcessor that follows in the same write; the
                // command itself is read by `resolve_frts_command` (a device act — see there).
                self.fwsec_image = Some(dmem0);
            }
            BootStep::StartProcessor => {
                // ★ w828: the regime that starts the processor decides how it stops.
                self.after_suspend = model.boot_sequence().after_suspend();
                report.transitions.push(self.gsp_startcpu());
            }
            BootStep::Teardown => {
                // E4. WPR2 must read down afterwards, and on the falcon/secure-booter
                // regime the guest asserts exactly that
                // (`ogkm-610: kernel_gsp_booter_tu102.c:175-187`, `ogkm-580: :179-191`
                // — identical code at both tags, including the GC6 arm that requires
                // WPR2 to stay *up*).
                self.enter_halted();
                report.transitions.push(Transition::E4);
            }
            BootStep::FirmwareLoaded => {
                // E5. The guard is the FSM's, not the sequence's: reaching `Booted` from
                // anywhere but `ProtectedRegionUp` is a phase-graph question and every
                // generation gets the same answer.
                if self.phase == BootPhase::ProtectedRegionUp {
                    self.phase = BootPhase::Booted;
                    report.transitions.push(Transition::E5);
                }
            }
            BootStep::FirmwareResume => {
                // E15 — only a saved PM-suspended life resumes; every other case is the cold
                // device's own SEC2 traffic (Ada's scrubber) and changes nothing.
                if matches!(self.phase, BootPhase::Cold | BootPhase::Halted) && self.resumable() {
                    let mut r = self.resume(ram, policy)?;
                    report.transitions.push(Transition::E15);
                    report.transitions.append(&mut r.transitions);
                    report.commands.extend(r.commands);
                    report.unserviced.extend(r.unserviced);
                    report.raise_status_irq = true;
                }
            }
            BootStep::BootArgsLo(v) => {
                self.mailbox_lo = v;
                self.boot_args_seen.0 = true;
            }
            BootStep::BootArgsHi(v) => {
                self.mailbox_hi = v;
                self.boot_args_seen.1 = true;
            }
            BootStep::PublishBootArgs(gpa) => {
                // Consumed: the next publish needs a fresh pair, so a single later
                // write cannot re-trigger against a half that is no longer current.
                self.boot_args_seen = (false, false);
                // ★ FSP regime: the address names GSP_FMC_BOOT_PARAMS, which points at the array.
                let gpa = match model.boot_sequence().boot_args_indirection() {
                    None => gpa,
                    Some(off) => {
                        let mut b = [0u8; 8];
                        ram.read(
                            gpa.checked_add(off)
                                .ok_or(GspFault::RmargsRegionAbsent { scanned: 0 })?,
                            &mut b,
                        )?;
                        u64::from_le_bytes(b)
                    }
                };
                let mut r = self.publish(ram, model, policy, gpa)?;
                report.transitions.push(Transition::E6);
                report.transitions.append(&mut r.transitions);
                report.commands.extend(r.commands);
                report.unserviced.extend(r.unserviced);
                report.raise_status_irq = true;
            }
            BootStep::CommandDoorbell => {
                // ★★★★★ **THE WORST TRAP IN THE DEVICE, MOVED OFF THE vCPU.**
                //
                // `[measured]` this arm serviced the whole command queue inside the guest's
                // register store and held a vCPU for **1.79 s** at `bar0+0x110c00`. Owner:
                // *"no blocking calls in vcpu"*, *"a trap may not take longer than a
                // millisecond"*, *"MMIO writes primarily touch a queue … and wake a
                // coordinator"*.
                //
                // ⊘ The register state has ALREADY been stored by the time we get here — this
                // arm is the SERVICING, and only the servicing is deferred. A read-back of the
                // queue head still answers what the guest wrote.
                self.pending_command_doorbells = self.pending_command_doorbells.saturating_add(1);
                return Ok(());
            }
            // E10 — the guest's ISR clears the edge before draining the queue
            // (`C: src/qemu/nvkvm_gpu_emul.c:4193-4200`).
            BootStep::ClearStatusIrq => {
                self.swgen0_pending = false;
                // ★★★★★ §16.76 — **THE OPENER**, and it is the same edge for both flags.
                //
                // The C gates its event batches on `gsp_swgen0_pending` and clears that flag
                // in exactly one place: the `IRQSCLR` write handler (`C:4441`). Its ISR
                // clears the edge before draining, so *"the guest cleared the interrupt"*
                // and *"the guest is about to consume what we posted"* are the same instant.
                //
                // ⊘ **The oracle covers this line NOT AT ALL**, and that is stated rather
                // than discovered: `cap1` contains **zero** `IRQSCLR` writes across 359 062
                // records (`crates/kayfabe-crec/tests/cap1_differential.rs` F-3), because
                // the guest never received an interrupt to clear. A green `cap1` diff says
                // nothing here; only a live boot exercises it. If it never fires, the gate
                // latches shut after one batch — which is the SAFE direction (no ring
                // overflow) and is measurable as
                // `kayfabe_device::osevent::OsEventLog::gated`.
                self.events_outstanding = false;
                report.transitions.push(Transition::E10);
            }
        }
        Ok(())
    }

    /// E1 / E2 / E3 — the GSP falcon STARTCPU, classified on `phase` alone.
    fn gsp_startcpu(&mut self) -> Transition {
        // ★ v3-initrace: the DMEM image loaded for THIS start (if any) is consumed now, whichever way.
        let image = self.fwsec_image.take();
        match self.phase {
            BootPhase::Cold | BootPhase::Halted => {
                self.phase = BootPhase::ProtectedRegionUp;
                self.fw_halted_after_suspend = false;
                // ★ 2026-10-14: FWSEC runs only on a NORMAL boot (`ogkm-580:
                // kernel_gsp_tu102.c:514-532`) — the guest is not resuming the saved life.
                self.suspended = None;
                self.pm_suspend = false;
                self.frts_image = image;
                self.frts_offset = None;
                Transition::E1
            }
            BootPhase::Suspending => {
                // ★ E2 — the trailing teardown STARTCPU. This is the one the C
                // misclassifies as a re-acquire; the binding dies here, so the next
                // driver life cannot inherit this one's queue GPA.
                self.enter_halted();
                Transition::E2
            }
            BootPhase::ProtectedRegionUp | BootPhase::Booted | BootPhase::Running => Transition::E3,
        }
    }

    /// Enter `Halted`: WPR2 down and — the load-bearing half — **the binding dropped by
    /// value**.
    fn enter_halted(&mut self) {
        // ★ 2026-10-14 (`docs/design/V3_S3_RESUME.md`): a life a fn 47 suspended keeps its binding,
        // INERT, for a resume; nothing reads through it until [`Transition::E15`]. Entering from
        // any other phase saves nothing new and leaves a saved life as it is (a Booter Unload
        // after the teardown STARTCPU already halted it changes nothing about it).
        let queue = std::mem::take(&mut self.queue);
        if self.phase == BootPhase::Suspending {
            self.suspended = match queue {
                QueueState::Bound(b) => Some(b),
                QueueState::Unbound => None,
            };
            if self.suspended.is_none() {
                self.pm_suspend = false;
            }
        }
        self.phase = BootPhase::Halted;
        // ★ v3-initrace: WPR2 is down; the next FWSEC-FRTS carries its own command.
        self.frts_offset = None;
        self.frts_image = None;
        self.fwsec_image = None;
        self.queue = QueueState::Unbound;
        self.large.reset();
        self.init_done_posted = false;
        self.swgen0_pending = false;
        // ★ The batch died with the binding: the queue it was posted into is gone, so the
        // next life must not inherit a gate that this one closed. ⊘ The registry itself is
        // NOT cleared here — it lives in the device shell, and the guest's own `FREE`s are
        // what retire its rows (`kayfabe_device::osevent`).
        self.events_outstanding = false;
        // The boot-args pair stops meaning anything when the life ends; the register
        // shadow keeps reading back what was written, as hardware would.
        self.boot_args_seen = (false, false);
        // ⊘⊘ **DO NOT clear `region_identity` here — MEASURED WRONG 2026-08-20.**
        // Clearing it at `enter_halted` was the obvious fix for PC-D7 (see `publish`) and it
        // FAILS `an_idle_release_re_acquire_rebinds_and_preserves_the_sequence_numbers`:
        // `BadSequence { expected: 5, got: 0 }`. `enter_halted` is reached from BOTH E4 (the
        // Booter-Unload STARTCPU — a real driver-life end, guest reconstructs its msgq) and
        // E2 (the trailing teardown STARTCPU, which an IDLE RELEASE also takes — guest KEEPS
        // its queue and its `rxSeqNum`). The two need OPPOSITE treatment, so the reset must be
        // scoped to the transition, not to this function. ⚠ Unfixed: see the PC-D7 block.
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
        model: &dyn kf_arch::GspModel,
        policy: &mut dyn CommandPolicy,
        boot_args_gpa: u64,
    ) -> Result<ServiceReport, GspFault> {
        let layout = model.libos_region_layout();

        // The region array. Bounded by the descriptor's own declared maximum, and a zero
        // entry is treated as **skip**, not stop: that the array is zero-terminated is
        // [inferred] I8 — the header declares no sentinel
        // (`ogkm-580: src/common/uproc/os/common/include/libos_init_args.h:31-56`,
        // `ogkm-610: :31-56` — the file is **byte-identical** between the tags, same lines,
        // so there is no version seam here and no profile to add), the C
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
        // nothing else (`GspStatusQueueInit`'s retry loop,
        // `ogkm-610: message_queue_cpu.c:368-390`, `ogkm-580: :346-368` — identical).
        let hdr = geom.published_header().encode();
        geom.region().write(ram, stat_off, &hdr)?;

        // ★★ Position resets, sequence numbers usually do not — and the exception is
        // **observable**, which is what closes `mode2_gsp_port_plan.md`'s open item O3.
        //
        // `msgqRxLink` sets `rxReadPtr = 0` (`ogkm-610: msgq.c:435`, `ogkm-580: :436`) and
        // nothing in the GSP tree ever *assigns* `rxSeqNum` — it is only `++`'d
        // (`ogkm-610: message_queue_cpu.c:836`, `ogkm-580: :782`; grep of both trees finds
        // no other write) — so a re-link resets the POSITION and preserves the SEQUENCE.
        // The one difference is immaterial here: 610 increments unconditionally at the exit
        // label, 580 only when `msgqRxMarkConsumed` succeeded (`ogkm-580: :774-783`).
        // The C learned this the expensive way (`C:3459-3483`):
        // zeroing the sequence made the re-posted `INIT_DONE` arrive at 0 << N, the guest
        // filed it as an old package (`ogkm-610: message_queue_cpu.c:762, 768`,
        // `ogkm-580: :693, 699`) and the second context hung in `kgspWaitForRmInitDone`.
        //
        // But `rxSeqNum` **is** zero-initialised once: in `GspMsgQueueInit`, which runs
        // from `kgspConstructEngine` and is torn down only in `kgspDestruct` — module
        // load and module unload. So a true `insmod` starts at 0 while an idle release
        // does not, and preserving unconditionally (the C's behaviour) would hand a fresh
        // guest a sequence number **greater** than its own, for which its receive path has
        // no recovery branch at all (`ogkm-610: :768-782`, `ogkm-580: :699-713` — both
        // handle only `<`).
        //
        // The discriminator is the **region**: `kgspDestruct` frees the shared memdesc, so
        // a new module load allocates a new one, while an idle release keeps
        // `MESSAGE_QUEUE_INFO` and its allocation alive (the C's own note at `C:3459-3470`
        // records exactly that). Same `sharedMemPhysAddr` ⇒ the same queue instance ⇒
        // preserve. A different one ⇒ a new instance ⇒ start at 0, as the guest did.
        //
        // ★ The command ring's POSITION travels with the sequence, for the reason
        // [`GspFsm::cmd_read_ptr`] gives: the guest's tx `writePtr` is only zeroed by
        // `msgqTxCreate` at module load, so a same-instance rebind must resume where we
        // stopped, and a new instance starts at 0 because the guest's producer did.
        //
        // ═══ ★★★ PC-D7 — ⊘⊘ **CLOSED 2026-08-20. THE TRADE-OFF BELOW WAS REAL AND ITS**
        // ═══ **RARITY JUDGEMENT WAS WRONG**, and that judgement is what kept it open. ═══
        //
        // The analysis below is correct in every mechanism and wrong in one quantity: it
        // says the wrong-preserve "needs an allocator to return the same page across a
        // module unload/reload". `[measured 2026-08-20, boot `w373`]` **it needs no unload at
        // all.** Running `nvidia-smi` in a loop — no CUDA, no `rmmod`, persistence off —
        // every cycle destroys and reconstructs the guest's `MESSAGE_QUEUE_INFO`, because at
        // 580 `rxSeqNum` lives and dies with the *adapter*, not with the module. The guest
        // printed `Expected 0 got 161` on the **third** `nvidia-smi` of the boot.
        // ⇒ Not rare: a coin flip per cycle on allocator reuse. It fired at cycle 3 in w373
        //   and cycle 5 in w370/w371 — **that nondeterminism is why the wall's ordinal
        //   varied**, which cost a day of hunting a counter that does not exist.
        // ⇒ **Fixed by replacing the discriminator, not by trading one wrong case for the
        //   other.** ⊘ The first attempt — dropping `region_identity` at `enter_halted` —
        //   is REFUTED and must not be retried: `enter_halted` is reached from BOTH E4 (the
        //   Booter-Unload STARTCPU) and E2 (the trailing teardown STARTCPU, which an IDLE
        //   RELEASE also takes), and those two need OPPOSITE answers. It failed
        //   `an_idle_release_re_acquire_rebinds_and_preserves_the_sequence_numbers` with
        //   `BadSequence { expected: 5, got: 0 }` on the first test run.
        //   ⚠ And scoping it to E4 would have been WORSE THAN USELESS: measured from the
        //   580 source, `kgspTeardown_TU102` runs FWSEC-SB *before* Booter Unload, our E2
        //   takes WPR2 down with the phase, and the guest's own gate then skips the SEC2
        //   launch entirely (`ogkm-580: kernel_gsp_booter_tu102.c:148-153`) — so **E4 never
        //   fires on this path at all**. An E4-scoped fix is inert on the bench while
        //   keeping every test green: the exact shape of a change that reads as done.
        //   ⇒ The discriminator is now the guest's own command `seqNum` (see below), which
        //   answers both cases from evidence the guest authored.
        // ★ The lesson worth keeping: **this comment predicted the failure verbatim, named
        //   the exact symptom, and was filed as "a named trade-off, not a defect to be
        //   fixed". Writing the hazard down is not the same as bounding how often it fires**
        //   — and the frequency estimate, not the mechanism, was the load-bearing claim.
        //
        // `sharedMemPhysAddr` is an ADDRESS, and the guest's allocator recycles pages. A
        // destruct → construct cycle that lands the new shared memdesc on the SAME physical
        // page reads here as *"the same instance"*, so we preserve a sequence the guest has
        // just reset to 0 — and it then sees `seqNum > rxSeqNum`, for which its receive path
        // has NO recovery branch at either tag (`ogkm-610: message_queue_cpu.c:768-782`,
        // `ogkm-580: :699-713` handle only `<`, and `rxSeqNum++` happens anyway). The
        // streams stay one apart forever: `GspFault::SeqNumGap`, permanently.
        //
        // ⊘ **The obvious fix is WORSE, and that is a source reading rather than a run of
        // ours**: always resetting is what `task #64` tried, and it is the C's own expensive
        // lesson at `C: src/qemu/nvkvm_gpu_emul.c:3459-3483` — a re-posted `INIT_DONE` at
        // sequence 0 arrives `0 << N`, the guest files it as an old package, and the second
        // context hangs in `kgspWaitForRmInitDone`. That failure is the COMMON case — every
        // idle release/re-acquire — where this one needs an allocator to return the same
        // page across a module unload/reload.
        //
        // So the choice is between a rare wrong-preserve and a routine wrong-reset, and the
        // rare one is taken deliberately. What would actually dissolve it is a discriminator
        // that is not an address — the guest's own `rxSeqNum`, read at bind time, would say
        // which instance it thinks it is — and that is a design change with its own reads,
        // not a one-line swap. Written down here rather than fixed, so the next reader finds
        // a decision instead of a bug.
        // ═══ ★★★★★ REACHABILITY THEOREM (2026-08-21) — EVERY E6 PUBLISH IS FRESH ═══════
        //
        // ⚠ **This whole discriminator may be dead code, and knowing that is worth more than
        // the branch.** The GSP boot-args mailboxes have exactly THREE writers at 580:
        //   1. `kgspBootstrap_TU102` (`kernel_gsp_tu102.c:533`), inside the
        //      `bootMode == KGSP_BOOT_MODE_NORMAL`-only block;
        //   2. `kgspExecuteSequencerCommand_TU102`, `CORE_RESUME` (`:933`);
        //   3. `kgspExecuteSequencerCommand_GA102`, `CORE_RESUME` (`ga102.c:162`).
        // (A fourth grep hit, `tu102.c:363`, is the function's own definition.)
        //
        // **(1) is fresh by construction.** `MESSAGE_QUEUE_INFO` lives exactly one
        // RmInitAdapter↔RmShutdownAdapter span (`kernel_gsp.c:3607` create, `:4353` destroy),
        // which the bench measured directly: the guest printed `Expected 0` on EVERY cycle.
        // A second publish per queue instance needs the `bRetry` loop, and a first attempt
        // that published necessarily ran FWSEC-FRTS first, so WPR2 is up and attempt two dies
        // at the gate (`kernel_gsp.c:3872-3880`) BEFORE publishing.
        // **(2)/(3) are the surviving-queue case — suspend/resume — and they are REAL**
        // (the source says so: *"for resume or GC6 exit, GSP-RM will restore queue state"*,
        // `tu102.c:568-570`). ⊘ But they cannot reach E6: `kflcnResetIntoRiscv_GA102` issues
        // no STARTCPU, so our phase is still `Halted` when the `CORE_RESUME` mailbox pair
        // lands, and E6's guard demands `ProtectedRegionUp | Booted` (`seq.rs:174-180`).
        // ⇒ **Resume is simply unbuilt in our FSM; when it is built it must RESTORE the
        // binding without publishing, and the preserved-sequence contract belongs at THAT
        // door, not this one.**
        //
        // ⊘⊘ **CHIP-CONDITIONAL, AND THIS IS THE SEAM.** The WPR2 gate that fences (1) is
        // bypassed by `PDB_PROP_GPU_PREINITIALIZED_WPR_REGION`, which is NOT setter-less: it
        // is set for **`GB10B | GB20B | GB20C`** (`generated/g_gpu_nvoc.c:191-199`) and false
        // for GA106. ⚠ Adding any of those chips silently un-fences the retry path and this
        // theorem stops holding — with no test to notice, because the theorem lives in a
        // comment. ★ An audit reported this property as having "zero setters in the tree";
        // it does not, and a `src/`-only grep is how that happened.
        //
        // ⇒ **Consequence for the code below: on every reachable path today it takes the
        // reset arm anyway**, so the seq test and an unconditional reset are observationally
        // identical here. The branch is kept because deleting it is a simplification that
        // wants a boot to confirm, and because the `w == 0` hazard below is real in general
        // even though it is unreachable now.
        // ⊘ **`w == 0` DOES NOT MEAN "FRESH".** The guest's write pointer WRAPS
        // (`msgq.c:559-562`: `writePtr += n; if (writePtr >= msgCount) writePtr -= msgCount`),
        // so `W == 0` is equally a same-instance producer that wrapped. Unreachable only
        // because same-instance E6 publishes do not exist — i.e. this is safe **by the
        // theorem above and by nothing else.**
        //
        // ★★★★★ **THE DISCRIMINATOR IS THE GUEST'S OWN COMMAND SEQUENCE, NOT THE ADDRESS.**
        // The guest stamps its private `txSeqNum` into every command element it submits
        // (`ogkm-580: message_queue_cpu.c:481`), and zeroes that counter only at queue
        // construction. So the most recently submitted command answers "is this the same
        // queue instance I was already talking to?" directly, in-band, with no dependence
        // on where the guest's allocator happened to place the region.
        //
        // `S + 1 == self.cmd_seq` — the guest's producer continues exactly where our
        // consumer stopped (`cmd_seq` is the sequence we NEXT expect; see
        // [`GspFsm::command_seq`]). `W == 0` is fresh by definition: a same-instance rebind
        // has necessarily submitted the two async init RPCs plus fn-47 already.
        //
        // ⊘ **Peek, never decode.** No checksum, no validation, no cursor movement — a
        // stale or hostile element must yield a *number*, not a refusal. Both possible
        // answers ("continue", "reset") are safe, so this read cannot fail the boot.
        let seq_evidence = (|| -> Result<Option<u32>, GspFault> {
            let count = geom.msg_count();
            let w = geom.region().read_u32(ram, geom.cmd_write_ptr_off())?;
            if w == 0 {
                return Ok(None);
            }
            let last = count.slot(w.wrapping_sub(1));
            let mut buf = vec![0u8; geom.element_size() as usize];
            geom.region()
                .read(ram, geom.cmd_element_off(last), &mut buf)?;
            Ok(Some(peek_seq_num(&self.abi.element, &buf)?))
        })()
        .unwrap_or(None);
        let same_instance = seq_evidence.is_some_and(|s| s.wrapping_add(1) == self.cmd_seq);

        // ★ The GPA is kept ONLY as a cross-check, because a publish where the two
        // disagree IS PC-D7 happening live — the case that used to wedge the GPU silently
        // and is now merely a counted line. `gpa_same=true seq_same=false` is the recycled
        // page; `gpa_same=false seq_same=true` would mean a live queue moved, which nothing
        // in 580 does and which we would want to hear about immediately.
        let gpa_same = self.region_identity == Some(shared_mem_pa);
        eprintln!(
            "kayfabe: GSP-PUBLISH gpa=0x{shared_mem_pa:x} gpa_same={gpa_same} seq_last={seq_evidence:?} \
             cmd_seq={} ⇒ same_instance={same_instance}{}",
            self.cmd_seq,
            if gpa_same == same_instance {
                ""
            } else {
                "  ★ PC-D7 COLLISION: address and sequence disagree — the address was lying"
            }
        );
        if !same_instance {
            self.stat_seq = 0;
            self.cmd_seq = 0;
            self.cmd_read_ptr = 0;
        }
        self.region_identity = Some(shared_mem_pa);
        let count = geom.msg_count();
        let peer_read_at_bind = geom.region().read_u32(ram, geom.peer_stat_read_ptr_off())?;
        self.queue = QueueState::Bound(QueueBinding {
            geom,
            stat: TxCursor::fresh(count),
            // `slot` is `% msgCount`, so a carried position can never name a slot the new
            // geometry does not have. Same-instance implies the same geometry, so in the
            // reachable case it is the identity.
            cmd: RxCursor {
                read_ptr: count.slot(self.cmd_read_ptr).index(),
            },
            peer_read_at_bind,
            resume_mark: None,
        });
        // ★ 2026-10-14: a publish is a new queue instance — a suspended life is not resumed.
        self.suspended = None;
        self.pm_suspend = false;
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

        // ★★ B4 — drain the backlog the bind inherited, without waiting for a doorbell.
        //
        // At 580 the command ring is **already non-empty** at bind time: the guest's
        // `writePtr` is 2, because `kgspQueueAsyncInitRpcs_IMPL` queued
        // `GSP_SET_SYSTEM_INFO` and `SET_REGISTRY` before `_kgspBootGspRm` ever ran
        // (`ogkm-580: kernel_gsp.c:3753-3777`, called at `:4141`, boot at `:4184`). The
        // cursor this bind installs starts at 0, so without a drain here those two
        // commands sit unread until the guest happens to ring the door again — which it
        // does, with `SET_GUEST_SYSTEM_INFO` right after `INIT_DONE`, so the port
        // *recovered by luck*. Draining on the bind makes the recovery structural, and on
        // a guest whose ring is empty (610's order) it reads the write pointer, finds
        // nothing, and does nothing.
        self.service_command_queue(ram, policy)
    }

    /// ★★ **E15 — the resume** (`docs/design/V3_S3_RESUME.md`). The design intent the
    /// reachability theorem in [`GspFsm::publish`] wrote down: *"when it is built it must RESTORE
    /// the binding without publishing, and the preserved-sequence contract belongs at THAT door"*.
    ///
    /// The guest's `KGSP_BOOT_MODE_SR_RESUME` does not link the status queue again — *"for resume
    /// or GC6 exit, GSP-RM will restore queue state"* — and waits for `GSP_INIT_DONE` on the queue
    /// it kept (`kgspWaitForRmInitDone`, `ogkm-580: kernel_gsp_tu102.c:563-575`,
    /// `kernel_gsp.c:5214-5240`): its `rxReadPtr` and `rxSeqNum` continue, and so do ours. So the
    /// saved binding comes back exactly as the suspend left it — no header write, no peek, no
    /// sequence reset — and `GSP_INIT_DONE` is posted at the status stream's next sequence number.
    ///
    /// ⊘ Reads one word of guest RAM before posting (the peer's status read pointer, the
    /// [`QueueBinding::resume_mark`]), through the geometry validated at the original bind.
    ///
    /// # Errors
    ///
    /// A refusal of a guest-supplied pointer; the life stays saved and the phase unchanged.
    fn resume(
        &mut self,
        ram: &mut dyn GuestRam,
        policy: &mut dyn CommandPolicy,
    ) -> Result<ServiceReport, GspFault> {
        let Some(mut binding) = self.suspended.take() else {
            return Ok(ServiceReport::default());
        };
        let mark = match binding
            .geom
            .region()
            .read_u32(ram, binding.geom.peer_stat_read_ptr_off())
        {
            Ok(v) => v,
            Err(e) => {
                self.suspended = Some(binding);
                return Err(e);
            }
        };
        binding.resume_mark = Some(mark);
        binding.cmd = RxCursor {
            read_ptr: binding.geom.msg_count().slot(self.cmd_read_ptr).index(),
        };
        self.queue = QueueState::Bound(binding);
        self.pm_suspend = false;
        self.fw_halted_after_suspend = false;
        self.phase = BootPhase::Booted;
        self.events_outstanding = false;
        self.init_done_posted = false;
        eprintln!(
            "kayfabe: GSP-RESUME the saved binding restored (no publish): stat_seq={} cmd_seq={} cmd_read_ptr={} peer_stat_read={mark}",
            self.stat_seq, self.cmd_seq, self.cmd_read_ptr
        );
        let init_done = OutgoingRpc {
            function: self.abi.rpc.codes.gsp_init_done,
            sequence: 0,
            rpc_result: 0,
            rpc_result_private: 0,
            payload: Vec::new(),
        };
        self.post(ram, &init_done)?;
        self.init_done_posted = true;
        // As at a publish: whatever the guest queued before the resume is served now.
        self.service_command_queue(ram, policy)
    }

    /// E7 / E8 / E12 — a doorbell.
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
        //
        // ★★ …but only when a binding has already existed. `Halted` is reached exactly by
        // a teardown (E2/E4), so it is precisely the case the bench measured: the guest
        // dropped its queues and is still ringing. Every other unbound phase — `Cold`
        // after power-on or a device reset, `ProtectedRegionUp` before the mailbox pair — is the
        // *healthy* 580 pre-bootstrap doorbell (see [`Transition::E12`]), and reporting
        // that as the stale-binding attack signature would put a false positive in the
        // ledger on every boot. Both arms read zero guest RAM; only the name differs.
        // ★★★★★ w828 — **A SUSPENDED PROCESSOR SERVICES NOTHING, and the queue it was bound to
        // may already be someone else's memory.** After fn-47 the GSP-RM processor has
        // suspended (E9); nothing it would read is live. `[measured 8dd2bbdf, vh,
        // run_cl_U_cup2_3]` a teardown that never reached its STARTCPU left the FSM
        // `Suspending` with the dead life's binding, and the NEXT life's pre-bootstrap
        // doorbells (the healthy 580 order, [`Transition::E12`]) made us read the freed region:
        // `PeerWritePtrOutOfRange { value: 2237667778, count: 63 }` — refused only because the
        // garbage happened to be out of range. An in-range garbage pointer is the C's `cap2b`
        // defect (arbitrary guest RAM parsed as RPCs and answered). ⇒ Refused BEFORE any read.
        if self.phase == BootPhase::Suspending {
            return Err(GspFault::ProcessorSuspended);
        }
        if matches!(self.queue, QueueState::Unbound) {
            return if self.phase == BootPhase::Halted {
                Err(GspFault::QueueNotBound)
            } else {
                Ok((Transition::E12, ServiceReport::default()))
            };
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

        let outcome = self.drain_commands(ram, policy, &geom, &mut report);

        // ★★★ **PC-D2 — the acknowledgement is published however the pass ended.**
        //
        // This write used to sit after the drain's `?`s, so a pass that consumed three
        // commands and then faulted on the fourth left our published `readPtr` at the value
        // from the *previous* pass. The guest computes its own free space from that number,
        // so it sees less room than exists — and a pass that keeps failing never publishes
        // again. `C: src/qemu/nvkvm_gpu_emul.c:3352-3358` is the same symptom arriving by a
        // different door: *"buffer is full"* at ~63 elements.
        //
        // ⊘ The ordering is the point: `drain_commands` commits `self.cmd_read_ptr` message
        // by message, so this always publishes exactly what we actually consumed — never
        // more. Both results are carried and the DRAIN's is reported first, because the
        // drain's fault is the diagnosis and a failure to publish is a consequence.
        let ack = geom
            .region()
            .write_u32(ram, geom.cmd_read_ptr_ack_off(), self.cmd_read_ptr);
        outcome?;
        ack?;

        if self.maybe_enter_running(ram, &geom)? {
            report.transitions.push(Transition::Running);
        }
        report.raise_status_irq |= self.swgen0_pending;
        Ok(report)
    }

    /// The drain itself: every complete message decoded, answered, and only then consumed.
    ///
    /// ★★★ **PC-D1 — answer BEFORE committing the cursor.** This loop used to commit the
    /// read pointer and the expected sequence and *then* call [`GspFsm::answer`]. `answer`
    /// posts, and [`GspFsm::post`] can return [`GspFault::QueueFull`] — whose own
    /// documentation calls it *retryable back-pressure*. On this path there was no retry:
    /// the command had already been consumed, so no reply was ever posted for it, and the
    /// guest blocked on `_issueRpcAndWait` for the whole RPC timeout.
    ///
    /// ⚠ It was unreachable **only** because this guest is synchronous under the GPU lock —
    /// one command in flight, so the status ring cannot fill against it. That is a property
    /// of the guest, not of the protocol, and `cap1b` reaches a real `QueueFull` at txn 1028
    /// with nine elements needed and one free.
    ///
    /// ## ★★ Why answer-then-commit, and what makes the other failure IMPOSSIBLE
    ///
    /// The alternative — hold the reply and re-post on the next pass — needs a queue of
    /// deferred replies, and a deferred reply is state that can be lost, reordered or
    /// double-sent. Answer-then-commit needs no state at all: the cursor *is* the record of
    /// what has been answered.
    ///
    /// Its own failure mode is the mirror image — **double-service**, a command answered
    /// twice because a retry re-reads it after a partially-effective post. Two replies at
    /// one `rpc.sequence` desynchronise the guest exactly as badly as none. That is made
    /// impossible rather than unlikely by [`GspFsm::post`]'s shape, and this is the property
    /// this ordering depends on:
    ///
    /// - the flow-control test returns **before any write**, so a `QueueFull` post leaves
    ///   the ring untouched;
    /// - the elements are written first and `stat_write_ptr` is advanced **last**, so a post
    ///   that faults mid-run leaves bytes in slots the guest is not looking at;
    /// - `stat_seq` and the cached free count advance only after the pointer.
    ///
    /// ⇒ a post either becomes visible to the guest in full, or not at all. A retry of a
    /// failed post rewrites the same slots and publishes once.
    /// `tests/service_pass_atomicity.rs` is what fails if that stops being true.
    fn drain_commands(
        &mut self,
        ram: &mut dyn GuestRam,
        policy: &mut dyn CommandPolicy,
        geom: &MsgqGeometry,
        report: &mut ServiceReport,
    ) -> Result<(), GspFault> {
        let count = geom.msg_count();
        let element_size = geom.element_size();

        let write_ptr = geom.region().read_u32(ram, geom.cmd_write_ptr_off())?;
        let mut read_ptr = self.cmd_read_ptr;
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
            // ★★ B3 — how far the ring moves is the **producer's** number, not ours.
            //
            // The guest advanced its `writePtr` by the `elemCount` it wrote
            // (`ogkm-580: message_queue_cpu.c:482, 578`), so that field is authoritative
            // for how far our `readPtr` must move. On a layout that has no such field
            // (610) the derivation is the only number there is and nothing changes.
            let declared = peek_elem_count(&self.abi.element, &first)?;
            let run_elements = match declared {
                Some(declared) => {
                    // Range first: a count above the staging bound can never become
                    // valid, however much the ring later fills. GSP-S1, at the one place
                    // the value is guest-written rather than derived.
                    let max = max_elements(element_size, self.abi.element_size_max);
                    if declared > max {
                        return Err(GspFault::ElementCountOutOfRange {
                            count: declared,
                            max,
                        });
                    }
                    // Then the cross-check. Deliberately before the availability test:
                    // a disagreement is a protocol violation regardless of how much of
                    // the ring the producer has filled, and deferring it behind
                    // "not ready" would hide it whenever the two happen to race.
                    if declared != len.elements() {
                        return Err(GspFault::ElementCountMismatch {
                            declared,
                            derived: len.elements(),
                        });
                    }
                    declared
                }
                None => len.elements(),
            };
            if run_elements > avail {
                // The producer has not finished writing this message. The driver's own
                // receive treats a short read the same way (`NV_ERR_NOT_READY`,
                // `ogkm-580: message_queue_cpu.c:632-644`,
                // `ogkm-610: message_queue_cpu.c:660-682`).
                break;
            }
            let run = self.read_run(ram, geom, read_ptr, len, &first)?;
            let msg = decode_message(&self.abi.element, &run, len, expect_seq, &self.abi.driver)?;
            let cmd = RpcCommand::from_incoming(&self.abi.rpc, &msg);

            // ★★★ PC-D1: answer FIRST. A `?` here leaves the cursor exactly where it was,
            // so this message is still owed and the next doorbell re-reads it. Consuming
            // first and answering second loses the command on any refusal `post` can raise
            // — `QueueFull` above all, which is the one this method's caller is expected to
            // retry.
            // ★ Ruling 2: a large control is joined first ([`crate::large`]); the assembler's
            // step is committed only once the command is consumed, so a retried pass rebuilds it.
            let declared = self.large_head_declared(&cmd);
            let consumed = match self.large.step(&cmd, declared) {
                crate::large::Step::NotLarge => {
                    self.answer(ram, policy, &cmd, report)?;
                    Some(cmd)
                }
                crate::large::Step::Held => None,
                crate::large::Step::Complete { whole, fragments } => {
                    self.answer_large(ram, policy, &whole, &fragments, report)?;
                    Some(whole)
                }
                crate::large::Step::Refused(why) => {
                    eprintln!(
                        "kf-gsp: LARGE-RPC REFUSED fn {} seq {}: {why}",
                        cmd.code, cmd.sequence
                    );
                    if matches!(why, crate::large::LargeRefusal::Interrupted { .. }) {
                        self.answer(ram, policy, &cmd, report)?;
                    } else {
                        let out = cmd.reply(NV_ERR_NOT_SUPPORTED, &[]);
                        self.post(ram, &out)?;
                        let detail = self.refusal_detail(&cmd);
                        self.refusals
                            .note(out.function, detail, out.rpc_result, out.sequence);
                    }
                    Some(cmd)
                }
            };
            self.large.commit();

            read_ptr = count.slot(read_ptr + run_elements).index();
            expect_seq = expect_seq.wrapping_add(1);
            avail -= run_elements;
            self.commit_command_cursor(read_ptr, expect_seq);
            if let Some(c) = consumed {
                report.commands.push(c);
            }
        }
        Ok(())
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
    ///
    /// # ★★★ What an unanswered command gets, and why it is a refusal
    ///
    /// Three policies were available for the command a [`CommandPolicy`] declines, and the
    /// choice is not a matter of taste — each was measured on a stock 580.159.04 guest.
    ///
    /// **(a) Echo the request back with `NV_OK`** — what the C does
    /// (`C: src/qemu/nvkvm_gpu_emul.c:2410-2416`) and what this method used to do. ⊘
    /// **Rejected: it is not merely uninformative, it is an out-of-bounds read performed
    /// by the guest at our instruction.** `kbusInitBarsSize_KERNEL` declares its `[OUT]`
    /// params on the stack with no initialiser and no `portMemSet`
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/bus/kern_bus.c:585`); RM serialises the
    /// uninitialised struct into the RPC, an echo hands it straight back, and the caller
    /// loops to `params.pciBarCount` over an eight-element array. `[measured]` run
    /// `t126b`, twice on two fresh boots at `f2acb89`: `BUG: unable to handle page fault
    /// for address: ffffd4a44035c000 / RIP: kbusInitBarsSize_KERNEL+0x90/0x110`. ★ This
    /// instance faults loudly; nothing makes the next one loud.
    ///
    /// **(b) Zero the reply body** — ⊘ **rejected: a zero is an answer too.** At the
    /// engine-table rung a zeroed `[OUT]` body made RM read *"this device has nothing"* —
    /// `portMemAllocNonPaged(0)` is `NULL` and `vectReserve(0)` asserts — and RM refused
    /// with `NV_ERR_NO_MEMORY` / `NV_ERR_INVALID_ARGUMENT`
    /// (`kf_abi::inittables`' module docs carry that dmesg). Zeros are not neutral;
    /// they are a small number, and RM branches on small numbers.
    ///
    /// **(c) Refuse by name** — what this does. The reply carries
    /// [`kf_abi::NV_ERR_NOT_SUPPORTED`] in the **envelope**, which short-circuits the
    /// guest *ahead of the copy-out and ahead of the control cache*
    /// (`ogkm-580: rpc.c:1994-2005`, `:11061-11065`): RM never reads the body at all, so
    /// there is no value in it to be believed. It is also the house style everywhere else
    /// in this port ([`crate::fault::GspFault::GuestRam`], `RefusingRam`,
    /// `kayfabe_rmrpc`'s `BridgeRefusal`), and it is why a rung costs one boot: the guest
    /// stops at the *first* thing it needed and names it, instead of proceeding on
    /// fiction and dying somewhere else.
    ///
    /// ## ⚠ The refusal is QUIET IN THE GUEST, and that is why [`ServiceReport`] carries it
    ///
    /// `rpcRmApiControl_GSP` lists `NV_ERR_NOT_SUPPORTED` among the statuses it logs
    /// *quietly* (`ogkm-580: rpc.c:11109-11115`), so a refused control produces **no
    /// dmesg line of its own** on a release module — only the eventual `LEVEL_ERROR` at
    /// whichever caller could not continue. Answering *"which controls did we refuse"*
    /// from the guest therefore costs one boot per control. [`ServiceReport::unserviced`]
    /// is the host-side ledger that answers it in one, and it is not optional
    /// instrumentation.
    ///
    /// ⊘ A [`Disposition::NoReply`] command is still not replied to, refusal included:
    /// posting anything for fn-72/73/202 surfaces in the driver as an unexpected event and
    /// desyncs the sequence.
    /// The whole message's payload length when `cmd` is the HEAD of a large `GSP_RM_CONTROL`
    /// (`_issueRpcAndWaitLarge`): its control header declares `params_off + paramsSize` bytes
    /// and the message is exactly `maxRpcSize` long (the head is always `min(bufSize,
    /// maxRpcSize)`, `rpc.c:2061`). ⊘ Both conditions: a short message that under-carries its
    /// params is NOT a head — holding it would wait for continuations that never come.
    fn large_head_declared(&self, cmd: &RpcCommand) -> Option<usize> {
        let declared = if cmd.function == RpcFunction::RmControl {
            let w = self.abi.driver.rm_control_wire();
            let size = cmd.payload.get(w.params_size_off..w.params_size_off + 4)?;
            w.params_off + u32::from_le_bytes(size.try_into().ok()?) as usize
        } else {
            // ★ 2026-10-14 (`docs/design/V3_S3_RESUME.md`): an `ALLOC_MEMORY` whose PTE list
            // outgrew one message — the suspend buffer GSP-RM's heap is saved into
            // (`_fbsrInitGsp`, `ogkm-580: fbsr_gm107.c:104-138`). The layout is the version's
            // generated MemoryList contract; no contract, no join.
            let cell = kf_abi::memory_list::cell(self.abi.driver.driver_version())?;
            if cmd.function != RpcFunction::Other(cell.function()) {
                return None;
            }
            cell.declared_len(&cmd.payload)?
        };
        let max_rpc =
            (self.abi.element_size_max as usize).saturating_sub(self.abi.element.hdr_size());
        (declared > cmd.payload.len() && cmd.payload.len() + crate::large::RPC_HEADER == max_rpc)
            .then_some(declared)
    }

    /// ★ Ruling 2: answer a joined large command, split at the request's own boundaries
    /// ([`crate::large::split_reply`]) — all replies or none: the room for every one is checked
    /// before the first is written, so a `QueueFull` retry never posts a fragment twice.
    fn answer_large(
        &mut self,
        ram: &mut dyn GuestRam,
        policy: &mut dyn CommandPolicy,
        whole: &RpcCommand,
        fragments: &[crate::large::Fragment],
        report: &mut ServiceReport,
    ) -> Result<(), GspFault> {
        let full = match policy.respond(whole) {
            Some(r) => r.on_wire(whole),
            None => {
                report.unserviced.push(Unserviced {
                    code: whole.code,
                    sequence: whole.sequence,
                });
                whole.reply(NV_ERR_NOT_SUPPORTED, &[])
            }
        };
        let replies = if crate::large::bidirectional(whole.function) {
            crate::large::split_reply(&full, fragments, self.abi.rpc.codes.continuation_record)
        } else {
            // ★ 2026-10-14: one reply, at the head, carrying only the status the guest reads.
            vec![OutgoingRpc {
                payload: Vec::new(),
                ..full.clone()
            }]
        };
        let QueueState::Bound(binding) = &self.queue else {
            return Err(GspFault::QueueNotBound);
        };
        let geom = binding.geom.clone();
        let element_size = geom.element_size();
        let mut needed = 0u32;
        for r in &replies {
            let run = encode_message(
                &self.abi.element,
                self.abi.rpc.header_version,
                element_size,
                self.abi.element_size_max,
                self.stat_seq,
                r,
            )?;
            needed += (run.len() / element_size as usize) as u32;
        }
        let cursor = binding.stat;
        if needed > cursor.free_cache {
            let peer_read = geom.region().read_u32(ram, geom.peer_stat_read_ptr_off())?;
            let free = free_elements(peer_read, cursor.write_ptr, geom.msg_count())?;
            if needed > free {
                return Err(GspFault::QueueFull { needed, free });
            }
        }
        for r in &replies {
            self.post(ram, r)?;
        }
        // ★ v3-refusals: the guest reads the status from the last reply — one ledger row for the command.
        if full.rpc_result != 0 {
            let detail = self.refusal_detail(whole);
            self.refusals
                .note(full.function, detail, full.rpc_result, full.sequence);
        }
        eprintln!(
            "kf-gsp: large RPC fn {} seq {} joined from {} fragment(s), {} bytes, answered in {} replies (rpc_result {:#x})",
            whole.code,
            whole.sequence,
            fragments.len(),
            whole.payload.len(),
            replies.len(),
            full.rpc_result
        );
        Ok(())
    }

    fn answer(
        &mut self,
        ram: &mut dyn GuestRam,
        policy: &mut dyn CommandPolicy,
        cmd: &RpcCommand,
        report: &mut ServiceReport,
    ) -> Result<(), GspFault> {
        let disposition = cmd.function.disposition();
        if disposition == Disposition::NoReply {
            // 72/73/202 are all issued asynchronously and none is awaited:
            // `GSP_SET_SYSTEM_INFO` calls `_issueRpcAsync`
            // (`ogkm-610: src/nvidia/src/kernel/vgpu/rpc.c:10466`, `ogkm-580: :10656`);
            // `SET_REGISTRY` takes `_issueRpcAsyncLarge` or `_issueRpcAsync` depending on
            // whether the packed registry table fits one message
            // (`ogkm-610: :10533`/`:10538`, `ogkm-580: :10728`/`:10733`); and
            // `ECC_NOTIFIER_WRITE_ACK` calls `_issueRpcAsync` (`ogkm-610: :11184`,
            // `ogkm-580: :11376`). Identical shape at both tags, and those are ALL the
            // async call sites in the driver — `tests/tests/rpc_async_set_oracle.rs`
            // derives the set from `rpc.c` rather than trusting this comment, which is how
            // the third one was found. Echoing one surfaces in the driver as an unexpected
            // event and desyncs the sequence.
            // ★ Fn 72's body is KEPT (never answered) for fn 65's encoder: the boot display's
            // `consoleMemSize`, decoded there with the table that serves fn 65 ([`crate::sysinfo`]).
            if cmd.function == RpcFunction::GspSetSystemInfo {
                crate::sysinfo::log_nocat(&self.abi.driver, &cmd.payload, cmd.sequence);
            }
            if cmd.function == RpcFunction::GspSetSystemInfo
                && let Some(cell) = &self.system_info
            {
                cell.store(&cmd.payload, cmd.sequence);
            }
            return Ok(());
        }
        // ★★★ **THE DEFAULT IS A NAMED REFUSAL** (task #127). A policy that has no answer
        // gets one posted for it, and that reply carries a non-zero envelope `rpc_result`
        // with a zeroed body — never the request reflected, and never a fabricated
        // `NV_OK`. See this method's docs for the measurement that made echoing
        // indefensible and for why zeroing alone is not the fix either.
        // ★★★ The one function whose reply may exceed its declared length, and it is a
        // protocol fact rather than a relaxation: `GSP_RM_ALLOC`'s `rpc.length` stops
        // where its flexible `params[]` begins, and the guest copies that window back
        // **unconditionally**, class-sized, from a fixed offset. Sizing an alloc reply
        // like every other reply hands the guest a zeroed params window — i.e. a
        // `memset` of libcuda's stack — with no status anywhere to see it. The whole
        // derivation, the clamps and the ⚠ that this is a RESTORE and not a FILL live on
        // `RpcCommand::reply_alloc`; ⊘ `RpcCommand::reply` is unchanged.
        let payload_max = (self.abi.element_size_max as usize)
            .saturating_sub(self.abi.element.hdr_size())
            .saturating_sub(RpcEnvelope::SIZE);
        let out = match policy.respond(cmd) {
            Some(r) if cmd.function == RpcFunction::RmAlloc => {
                cmd.reply_alloc(r.rpc_result, &r.body, &self.abi.driver, payload_max)
            }
            Some(r) => r.on_wire(cmd),
            None => {
                report.unserviced.push(Unserviced {
                    code: cmd.code,
                    sequence: cmd.sequence,
                });
                // ⊘ An alloc refusal must carry its status IN the params (see
                // `stamp_alloc_status`): an empty body leaves the guest reading `0`.
                if cmd.function == RpcFunction::RmAlloc {
                    cmd.reply_alloc(
                        NV_ERR_NOT_SUPPORTED,
                        &cmd.payload,
                        &self.abi.driver,
                        payload_max,
                    )
                } else {
                    cmd.reply(NV_ERR_NOT_SUPPORTED, &[])
                }
            }
        };
        // ⊘ ASKED **AFTER** `respond`, and the order is the whole point. The question is
        // *"did servicing this command move the address space"*, which is only answerable
        // once it has been serviced. Asked before, a policy could only pattern-match on the
        // function id — and the function that binds rows is `RmControl`, which is also the
        // function for a hundred controls that bind nothing.
        let held_this_command = policy.holds_for_refresh(cmd);
        let deferred = policy.defers(cmd);
        let held_this_command = held_this_command || deferred.is_some();
        // ★★★ THE HOLD. `held` is a queue of ONE-per-command replies whose rows are not on
        // the host yet; `release_held` posts them. ⊘ The `UnloadingGuestDriver` arm below is
        // deliberately not reachable through it: a teardown reply must never wait on a
        // publication, and it changes no address space, so it never sets the flag.
        if held_this_command {
            // ⊘⊘ `[measured w425]` this push was SILENT, and the silence read as absence: a
            // whole boot's log carried no held-reply line and I concluded the mechanism never
            // fired. That is the ledger's *"no counter fired is NOT no record exists"* one
            // level up — nothing here ever wrote a counter to begin with.
            //
            // ⚠ This is the RPC-map synchronization point. Whether a map reply waits for its
            // rows is the difference between the guest submitting work against a backed range
            // and against an unbacked one, and `[measured w425]` the pin of the LLM's faulting
            // range completed in the SAME SECOND as the `FAULT_PDE ACCESS_TYPE_VIRT_WRITE`
            // that hit it. So "did the hold run" must be legible FROM the artefact, never
            // inferred from the absence of some other line.
            eprintln!(
                "kayfabe: HELD-REPLY fn={:?} depth={} — this map reply WAITS for its rows",
                cmd.function,
                self.held.len() + 1
            );
            self.held.push(HeldReply {
                rpc: out,
                deferred,
                alloc: cmd.function == RpcFunction::RmAlloc,
                detail: self.refusal_detail(cmd),
            });
        } else {
            self.post(ram, &out)?;
            // ★ v3-refusals: recorded AFTER a successful post — the status the guest reads.
            if out.rpc_result != 0 {
                let detail = self.refusal_detail(cmd);
                self.refusals
                    .note(out.function, detail, out.rpc_result, out.sequence);
            }
        }

        if cmd.function == RpcFunction::UnloadingGuestDriver {
            // ★ E9 — reply first, **then** suspend. `MAILBOX0` must report the suspend
            // sentinel afterwards or the guest's close poll spins
            // (`kgspWaitForProcessorSuspend_TU102`, `ogkm-610: kernel_gsp_tu102.c:351-359`,
            // `ogkm-580: :1241-1249` — the poll itself is identical).
            //
            // ★★ SEAM in the *predicate* that poll spins on, and it constrains how the
            // register model may encode this: 580 tests `mailbox == 0x80000000`, exact
            // equality with the constant inlined (`ogkm-580: kernel_gsp_tu102.c:1225-1239`),
            // where 610 tests `mailbox & INTERRUPT_PROCESSOR_SUSPENDED_VALUE`, a mask
            // (`ogkm-610: :333, 348`). So the sentinel must be written **whole** and never
            // OR-ed onto a mailbox shadow: a shadow still holding a boot-args low half with
            // bit 31 set reads as suspended at 610 and hangs the teardown poll forever at
            // 580. Sequence numbers are preserved: the
            // guest sent this fn-47 at the current sequence and polls for its ack there
            // (`C:2465-2472` records what zeroing them cost).
            self.suspend(report);
        }
        Ok(())
    }

    /// ★ E9 (and, on a regime whose firmware stops itself, E13) — fn-47 has been answered.
    fn suspend(&mut self, report: &mut ServiceReport) {
        self.phase = BootPhase::Suspending;
        report.transitions.push(Transition::E9);
        // ★★★★★ w828 — **on the FSP regime nothing else will ever end this life.** The driver
        // runs no teardown ucode there; `kgspTeardown_GH100` only waits for the RISC-V core to
        // halt (`ogkm-580: src/nvidia/src/kernel/gpu/gsp/arch/hopper/kernel_gsp_gh100.c:995-1004`)
        // while the GSP-FMC and ACR take WPR2 down. So the firmware halts itself here — WPR2
        // down, binding dropped — and the suspend sentinel stays readable
        // (`fw_halted_after_suspend`). Left `Suspending`, WPR2 stayed up and every
        // Hopper/Blackwell re-open died at `_kgspBootGspRm`'s WPR2 gate
        // (`ogkm-580: kernel_gsp.c:3872-3880`).
        if self.after_suspend == AfterSuspend::FirmwareHalts {
            // ⊘ The status interrupt the fn-47 reply raised survives the halt — IRQSTAT keeps its
            // value until the guest clears it, as it would on silicon.
            let irq = self.swgen0_pending;
            self.enter_halted();
            self.swgen0_pending = irq;
            self.fw_halted_after_suspend = true;
            report.transitions.push(Transition::E13);
        }
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
    /// `rxSeqNum`, and the recovery branch (`ogkm-610: message_queue_cpu.c:768-782`,
    /// `ogkm-580: :699-713`) handles only `seqNum < rxSeqNum` — for `>` there is no
    /// recovery at either tag, and `rxSeqNum++` still happens
    /// (`ogkm-610: :836`, `ogkm-580: :782`), so the streams stay one apart forever.
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
        // if the cache is short — `msgqTxSubmitBuffers`' own order
        // (`ogkm-610: msgq.c:544-547`, `ogkm-580: :545-548` — identical).
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
        // AFTER successful queue publication and any held-reply patch. Never
        // records a failed/full-queue attempt; it cannot alter the posted RPC.
        if let Some(record) =
            self.reply_body_trace
                .posted(&self.abi, rpc, self.stat_seq.wrapping_sub(1))
        {
            eprintln!("{record}");
        }
        Ok(())
    }

    /// Post an unsolicited event.
    ///
    /// ★ §7-G7: only once the guest is past its bootup poll, which asserts on anything
    /// outside a short allowlist — **eight** functions at 610
    /// (`ogkm-610: kernel_gsp.c:1419-1439`) and **six** at 580 (`ogkm-580: :1464-1482`).
    /// `POST_EVENT` is **not** on either, so [`RpcFunction::allowed_in_bootup_window`] is
    /// correct for both tags; the lists differ in `GSP_RUN_CPU_SEQUENCER` (580 only) and in
    /// `PFM_REQ_HNDLR_STATE_SYNC_CALLBACK` / `GSP_LOAD_EXEC_GENERIC_BOOTLOADER` /
    /// `GSP_LOAD_EXEC_HS_BINARY` (610 only), so any *widening* of that predicate is
    /// version-keyed data and must not be written as one set.
    ///
    /// # Errors
    ///
    /// [`GspFault::NotRunning`] before the guest has drained `GSP_INIT_DONE`, plus
    /// whatever [`GspFsm::post`] refuses with.
    /// ★★★★★ **POST THE REPLIES WHOSE REFRESH HAS FINISHED** — the other half of
    /// [`Reply::hold_for_refresh`].
    ///
    /// Called by the publication worker once the rows are on the host, never by a vCPU. That
    /// split is the owner's rule and its constraint at the same time: *"only return from a
    /// TLB invalidate, the RPC map call or the kernel emulated channel for UVM after the
    /// PTE/PDB page table refresh function finished"*, and *"vCPU thread no allowance for
    /// such blocking things"*. The guest spends the interval in its own `rpcRecvPoll`, which
    /// is what it does for every RPC anyway.
    ///
    /// Returns how many were posted.
    ///
    /// ⊘ **No deadline here, and that is not an omission.** This crate has no clock (§8.3),
    /// and a deadline invented from a counter would be a second, disagreeing notion of time.
    /// The caller has the clock and owns the bound: it watches [`GspFsm::held_len`] and is
    /// the one that can say *"this has been held too long"*. ⚠ A held reply nobody releases
    /// is a guest hang, so a non-zero `held_len` at teardown is a defect, not a statistic.
    ///
    /// # Errors
    ///
    /// Whatever [`GspFsm::post`] refuses with. ⊘ A refused post leaves the remaining held
    /// replies in place rather than dropping them: a dropped reply is a guest hang, and the
    /// next call will try again.
    /// ★★★★★ **w432 — DRAIN THE DEFERRED COMMAND DOORBELLS. CALL THIS OFF A vCPU.**
    ///
    /// Services one command doorbell that [`BootStep::CommandDoorbell`] deferred, and returns
    /// what it serviced together with the
    /// merged report — the caller still has to act on `raise_status_irq` and the commands,
    /// exactly as the inline path's caller did.
    ///
    /// ⊘ **Drains to zero rather than servicing one.** A doorbell is a statement that the
    /// queue moved, and two writes before the worker wakes are two queue states; servicing
    /// one and leaving the count at one would strand the guest's second submission behind a
    /// wake that never comes again.
    ///
    /// ⚠ The count is decremented BEFORE each `doorbell` call, not after. If servicing
    /// returns an error we must not re-enter that same doorbell forever on every later wake —
    /// a fault is reported to the caller and the slot is gone, which is the same thing the
    /// inline path did with `?`.
    ///
    /// # Errors
    /// Whatever servicing the queue refused, by name, on the first doorbell that faults.
    pub fn service_one_deferred_command(
        &mut self,
        ram: &mut dyn GuestRam,
        policy: &mut dyn CommandPolicy,
    ) -> Result<ServiceReport, GspFault> {
        let mut report = ServiceReport::default();
        if self.pending_command_doorbells == 0 {
            return Ok(report);
        }
        // ⚠ Decrement BEFORE servicing, not after. If servicing faults we must not re-enter
        // the same doorbell on every later wake; the inline path lost it to `?` the same way.
        self.pending_command_doorbells -= 1;
        let (t, mut r) = self.doorbell(ram, policy)?;
        report.transitions.push(t);
        report.transitions.append(&mut r.transitions);
        report.commands.extend(r.commands);
        report.unserviced.extend(r.unserviced);
        report.raise_status_irq |= r.raise_status_irq;
        Ok(report)
    }

    /// How many command doorbells are waiting for [`Self::service_one_deferred_command`].
    #[must_use]
    pub const fn pending_command_doorbells(&self) -> u32 {
        self.pending_command_doorbells
    }

    /// ★★★★★ **w754 — RECORD DOORBELLS THE SHELL POSTED WITHOUT TAKING THIS LOCK.**
    ///
    /// Constraint 6 asks that an MMIO trap *"only posts the register write to a queue …,
    /// wakes a worker, and returns"*. [`BootStep::CommandDoorbell`]'s deferral already does
    /// the *"does not service"* half, but reaching it costs the shell the plane's rank-0
    /// lock — the one a worker holds across a whole RPC. So the shell may now classify the
    /// queue-head write **before** that lock, count it in an atomic of its own, and hand the
    /// total here on the worker's next pass, under the lock it was going to take anyway.
    ///
    /// ⊘ **A COUNT, not a flag**, for exactly [`Self::pending_command_doorbells`]' reason:
    /// two writes before the worker wakes are two queue states to drain, and collapsing them
    /// would strand the guest's second submission behind a wake that never comes again.
    ///
    /// ⚠ Saturating. The count is a *"the queue moved"* tally and the drain empties it to
    /// zero, so the only way to reach `u32::MAX` is a worker that never runs — in which case
    /// the guest is already parked and losing an increment changes nothing.
    ///
    /// ⊘ There is no arming flag to consult (v3: deferral is the only mode).
    pub const fn note_command_doorbells(&mut self, n: u32) {
        self.pending_command_doorbells = self.pending_command_doorbells.saturating_add(n);
    }

    pub fn release_held(&mut self, ram: &mut dyn GuestRam) -> Result<usize, GspFault> {
        let mut posted = 0usize;
        while let Some(h) = self.held.first() {
            // ⊘ Cloned before the post and removed only AFTER it succeeds. `post` can refuse
            // (an unbound queue, a full ring), and a reply removed by a FAILED post is a
            // reply nobody will ever send — which is a guest hang, not a dropped message.
            let mut rpc = h.rpc.clone();
            // ★ P5b: a reply whose status waits on a host act stays held — and so does every
            // reply behind it (the order posted is the order asked).
            // ⊘ An act that succeeded for a command another link refused: the guest sees the
            // refusal, and what the act built is undone by its owner (or, with no undo attached,
            // orphaned until its owner is freed). Named, never silent (`settle_deferred`).
            if let Some(d) = &h.deferred
                && !settle_deferred(&mut rpc, d, h.alloc)
            {
                break;
            }
            let detail = h.detail;
            self.post(ram, &rpc)?;
            self.held.remove(0);
            // ★ v3-refusals: the FINAL status (after the deferred act resolved it).
            self.refusals
                .note(rpc.function, detail, rpc.rpc_result, rpc.sequence);
            posted += 1;
        }
        if posted > 0 {
            // ★ The pair of the HELD-REPLY line. Two counters that must be read together:
            // a boot with holds and no releases is a PARKED GUEST, and a boot with neither
            // is a guest that never waited for its rows at all. Neither is visible from one.
            eprintln!("kayfabe: HELD-REPLY-POSTED n={posted} — their rows are on the host now");
        }
        Ok(posted)
    }

    /// ★★★ v3-refusals: every non-`NV_OK` reply posted so far ([`crate::refusal`]).
    #[must_use]
    pub fn refusals(&self) -> &crate::refusal::RefusalLedger {
        &self.refusals
    }

    /// ★ v3-refusals: the refusal rows first seen since the previous call — one log line each.
    pub fn take_fresh_refusals(&mut self) -> Vec<crate::refusal::RefusalRow> {
        self.refusals.take_fresh()
    }

    /// What a command carried, for the refusal ledger: the control id of a `GSP_RM_CONTROL`, the
    /// class of a `GSP_RM_ALLOC`, else `None`. A header read only — nothing is judged here.
    fn refusal_detail(&self, cmd: &RpcCommand) -> Option<u32> {
        match cmd.function {
            RpcFunction::RmControl => self
                .abi
                .driver
                .decode_rpc_control(cmd.wire_body())
                .ok()
                .map(|r| r.cmd),
            RpcFunction::RmAlloc => self
                .abi
                .driver
                .decode_rpc_alloc(cmd.wire_body())
                .ok()
                .map(|r| r.class),
            _ => None,
        }
    }

    /// How many replies are waiting on a refresh right now. ⊘ A non-zero value at teardown
    /// means a refresh never completed and the guest was left polling.
    #[must_use]
    pub fn held_len(&self) -> usize {
        self.held.len()
    }

    /// ★ P5c: post one `RC_TRIGGERED` event (`NV_VGPU_MSG_EVENT_RC_TRIGGERED`) whose body is
    /// `payload` (`rpc_rc_triggered_v17_02`, encoded by `kf_abi::rc::RcTriggered`). Unsolicited, so
    /// `sequence` is 0, as for every event ([`GspFsm::deliver_events`]). The caller raises the GSP
    /// interrupt after publishing the registers.
    ///
    /// # Errors
    /// As [`GspFsm::post_event`].
    pub fn post_rc_triggered(
        &mut self,
        ram: &mut dyn GuestRam,
        payload: Vec<u8>,
    ) -> Result<(), GspFault> {
        let rpc = OutgoingRpc {
            function: self.abi.rpc.codes.rc_triggered,
            sequence: 0,
            rpc_result: 0,
            rpc_result_private: 0,
            payload,
        };
        self.post_event(ram, &rpc)
    }

    /// ★ Display step 3c: post one LIST `POST_EVENT` (`NV_VGPU_MSG_EVENT_POST_EVENT` with
    /// `bNotifyList`) whose body is `payload` (`rpc_post_event_v17_00` plus its event data, encoded
    /// by `kf_abi::postevent::SubdeviceNotify`) — what a GSP sends when a subdevice event such as a
    /// hotplug fires. Unsolicited, so `sequence` is 0, as for every event; NOT behind
    /// [`GspFsm::deliver_events`]'s os-event gate (that gate is for the os-event batch). The caller
    /// raises the GSP interrupt after publishing the registers.
    ///
    /// # Errors
    /// As [`GspFsm::post_event`].
    pub fn post_subdevice_event(
        &mut self,
        ram: &mut dyn GuestRam,
        payload: Vec<u8>,
    ) -> Result<(), GspFault> {
        let rpc = OutgoingRpc {
            function: self.abi.rpc.codes.post_event,
            sequence: 0,
            rpc_result: 0,
            rpc_result_private: 0,
            payload,
        };
        self.post_event(ram, &rpc)
    }

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

    /// ★★★★★ §16.76 — **deliver one batch of os-event wakeups**: the gate, then the
    /// action.
    ///
    /// This is `C: src/qemu/nvkvm_gpu_emul.c:1849-1873` (`nvkvm_gsp_deliver_events`), and
    /// the port keeps its four steps in the C's own order because three of them exist to
    /// protect the fourth:
    ///
    /// 1. **nothing registered → return.** `C:1851-1853`.
    /// 2. **previous batch not drained → return.** `C:1861-1863`, gated on
    ///    [`GspFsm::events_outstanding`] — read that field's docs for why it is not
    ///    `swgen0_pending`, which is the trap this rung was briefed to walk into.
    /// 3. **post ALL of them**, one `POST_EVENT` per registered `(hClient, hEvent)`.
    /// 4. **raise once** — here, by *latching the flag*; the interrupt itself belongs to
    ///    the device shell, which owns the interrupt tree and the VMM's vocabulary. The FSM
    ///    deliberately names no `IrqSpec` (see [`ServiceReport::raise_status_irq`]).
    ///
    /// # ⊘ What a partial post does, and why it still raises
    ///
    /// [`GspFsm::post`] can return [`GspFault::QueueFull`], which its own docs call
    /// *retryable back-pressure*. If the ring fills part-way through a batch this stops,
    /// reports how many landed, and **still latches the gate** — because the elements that
    /// DID land are real messages the guest must be told to drain. Raising for a short
    /// batch wakes a waiter that then re-checks and finds nothing new, which costs one
    /// spurious poll; not raising strands posted elements behind a gate that only an
    /// `IRQSCLR` opens, and no `IRQSCLR` arrives without an interrupt. ⊘ The C does not
    /// distinguish these cases at all — `nvkvm_m3_post_status` returns `void` — so this is
    /// strictly more careful than the oracle, in the direction the oracle cannot advise on.
    ///
    /// # ⚠ A wakeup is not a completion
    ///
    /// Every event posted here says *"the event you registered has fired"* and nothing
    /// more; the waiter re-reads its own semaphore and decides. ⊘ This is therefore **not**
    /// a forged completion and must never be read as one — nothing here claims work
    /// finished, and the execution plane still owes every byte it always owed.
    pub fn deliver_events(
        &mut self,
        ram: &mut dyn GuestRam,
        events: &[kf_abi::postevent::PostEvent],
    ) -> EventDelivery {
        // Step 1 (`C:1851`).
        if events.is_empty() {
            return EventDelivery::NoneRegistered;
        }
        // Step 2 (`C:1861`) — THE GATE, before the action.
        if self.events_outstanding {
            return EventDelivery::Gated;
        }
        // ⊘ Asked before the loop and reported by name, because `NotRunning` is a
        // statement about the BOOT and `QueueFull` is one about the ring — an operator's
        // next move differs, and a single "delivered 0" would collapse them.
        if self.phase != BootPhase::Running {
            return EventDelivery::NotRunning;
        }
        let function = self.abi.rpc.codes.post_event;
        let mut posted = 0usize;
        let mut refused = None;
        // Step 3 (`C:1864-1867`) — post ALL of them.
        for ev in events {
            let rpc = OutgoingRpc {
                function,
                // ⊘ Zero, and it is not a placeholder: an EVENT is unsolicited, so it
                // echoes no request's transaction id. The guest's receiver dispatches
                // events on `function` alone (`ogkm-580: kernel_gsp.c:1610-1622`); the
                // per-message ordering the ring cares about is `stat_seq`, which
                // `GspFsm::post` owns.
                sequence: 0,
                rpc_result: 0,
                rpc_result_private: 0,
                payload: ev.encode(),
            };
            match self.post_event(ram, &rpc) {
                Ok(()) => posted += 1,
                Err(f) => {
                    refused = Some(f);
                    break;
                }
            }
        }
        if posted == 0 {
            // Nothing landed, so there is nothing for the guest to drain and no reason to
            // close a gate over an empty batch.
            return EventDelivery::Failed {
                fault: refused.unwrap_or(GspFault::QueueNotBound),
            };
        }
        // Step 4 (`C:1868`) — raise ONCE, per batch, by latching. The shell delivers.
        self.events_outstanding = true;
        EventDelivery::Delivered {
            posted,
            short: refused,
        }
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
        // the guest's read pointer at 0 (`ogkm-610: msgq.c:435`, `ogkm-580: :436`), so
        // "the pointer has moved off
        // zero" IS "the guest consumed INIT_DONE". Deliberately not "the guest drained
        // everything": a reply posted in this very service pass is not yet drained, and
        // waiting for it would keep the FSM out of `Running` for exactly as long as it
        // keeps answering.
        let drained = match b.resume_mark {
            // ★ 2026-10-14: a resumed binding was never re-linked — the pointer continues from
            // where the suspend left it, so moving off that value is consuming `GSP_INIT_DONE`.
            Some(mark) => peer_read != mark,
            None => linked && peer_read != 0,
        };
        if self.phase == BootPhase::Booted && self.init_done_posted && drained {
            self.phase = BootPhase::Running;
            return Ok(true);
        }
        Ok(false)
    }

    fn commit_command_cursor(&mut self, read_ptr: u32, seq_num: u32) {
        self.cmd_seq = seq_num;
        self.cmd_read_ptr = read_ptr;
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

kf_util::assert_send_sync!(
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
kf_util::assert_send!(dyn CommandPolicy);

/// Compile-time witness that [`IncomingRpc`] stays reachable from this module's public
/// surface: the decode path returns it, and a change that drops it from the API would
/// otherwise only fail at a test.
const _: fn(&IncomingRpc) -> u32 = |m| m.seq_num;

#[cfg(test)]
mod holding_a_reply_until_the_refresh_lands {
    //! ★★★★★ **w402 — the mechanism for the owner's ordering rule.**
    //!
    //! > *"You should only return from a TLB invalidate, the RPC map call or the kernel
    //! > emulated channel for UVM after the PTE/PDB page table refresh function finished."*
    //!
    //! The measurement that forced it: six host faults in one boot, every one `FAULT_PDE`,
    //! every one at an address our table already declared — four worker threads' copy
    //! sources, `P3`'s release semaphore, and the fault this campaign had spent weeks calling
    //! a harmless bystander. All of them ARE published, *after* the engine touched them.
    //!
    //! ⚠ These test the part that can hang a guest. A reply that is held and never released
    //! is worse than a wrong reply: the guest sits in `rpcRecvPoll` until the driver's own
    //! RPC timeout, and nothing in our logs says why.
    use super::*;

    struct Never;
    impl CommandPolicy for Never {
        fn respond(&mut self, _cmd: &RpcCommand) -> Option<Reply> {
            None
        }
    }

    struct Holds;
    impl CommandPolicy for Holds {
        fn respond(&mut self, _cmd: &RpcCommand) -> Option<Reply> {
            None
        }
        fn holds_for_refresh(&self, _cmd: &RpcCommand) -> bool {
            true
        }
    }

    fn cmd() -> RpcCommand {
        RpcCommand {
            function: RpcFunction::RmAlloc,
            code: 1,
            sequence: 7,
            payload: Vec::new(),
            elements: 1,
            delivered: Vec::new(),
        }
    }

    /// ⊘ THE DEFAULT IS THE SAFE ONE. A policy that has never heard of this mechanism must
    /// not hold a reply — a held reply nobody releases is a guest hang, so silence has to
    /// mean "deliver now".
    #[test]
    fn a_policy_that_says_nothing_never_holds() {
        assert!(
            !Never.holds_for_refresh(&cmd()),
            "the default must be false: an unheld reply is at worst early, a held one that \
             nobody releases is a hang"
        );
    }

    /// ★★★ ANY LINK MAY HOLD — the chain is a `find_map` for the ANSWER and an `any` for the
    /// HOLD, and conflating them is the bug this pins.
    ///
    /// A link seated ahead of the answerer exists precisely to observe facts the answerer
    /// does not own (`CommandObserver`'s whole rationale). The link that knows the address
    /// space moved is often NOT the link that produces the reply, so asking only the
    /// answerer would silently deliver early.
    #[test]
    fn a_holding_link_ahead_of_the_answerer_still_holds() {
        let chain = PolicyChain::new(vec![Box::new(Holds), Box::new(Never)]);
        assert!(
            chain.holds_for_refresh(&cmd()),
            "the FIRST link holds; it does not matter that a later one would answer"
        );
        let chain = PolicyChain::new(vec![Box::new(Never), Box::new(Holds)]);
        assert!(
            chain.holds_for_refresh(&cmd()),
            "and the other order too — this is an `any`, not 'whoever answered'"
        );
    }

    /// ⊘ And the negative control: a chain of non-holders holds nothing. Without this, a
    /// `holds_for_refresh` hardcoded to `true` would pass the test above.
    #[test]
    fn a_chain_of_non_holders_holds_nothing() {
        let chain = PolicyChain::new(vec![Box::new(Never), Box::new(Never)]);
        assert!(!chain.holds_for_refresh(&cmd()));
    }
}

#[cfg(test)]
mod a_reply_whose_status_is_a_host_act {
    //! ★ P5b — the channel plane's births moved off the GSP lock: the reply is HELD on a
    //! [`Deferred`] and posted with the act's status.
    use super::*;

    struct Defers(Option<Deferred>);
    impl CommandPolicy for Defers {
        fn respond(&mut self, _cmd: &RpcCommand) -> Option<Reply> {
            None
        }
        fn defers(&mut self, _cmd: &RpcCommand) -> Option<Deferred> {
            self.0.take()
        }
    }

    fn cmd() -> RpcCommand {
        RpcCommand {
            function: RpcFunction::RmAlloc,
            code: 103,
            sequence: 9,
            payload: vec![0u8; 32],
            elements: 1,
            delivered: Vec::new(),
        }
    }

    #[test]
    fn a_deferred_is_pending_until_resolved_and_the_first_resolution_wins() {
        let d = Deferred::new();
        assert_eq!(d.outcome(), None);
        let seen = d.clone();
        d.resolve(0x56);
        d.resolve(0);
        assert_eq!(
            seen.outcome(),
            Some(0x56),
            "a clone sees the act's outcome, and a second resolve cannot rewrite it"
        );
    }

    fn held(rpc_result: u32) -> OutgoingRpc {
        OutgoingRpc {
            function: 103,
            sequence: 9,
            rpc_result,
            rpc_result_private: rpc_result,
            payload: vec![0u8; 40],
        }
    }

    /// ★ x11-dispsw review (2026-10-03, LOW): an act that SUCCEEDED for a command another link
    /// refused (the object seat: a handle the guest already uses) runs its owner's undo — once,
    /// however often the post is retried — and an act that failed or a reply that succeeded never
    /// does. Without the call in `settle_deferred` the host object the act built stays until its
    /// channel goes (the review's orphan).
    #[test]
    fn a_succeeded_act_under_a_refused_reply_runs_its_undo_once() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let ran = std::sync::Arc::new(AtomicU32::new(0));
        let hook = |d: &Deferred| {
            let r = ran.clone();
            d.on_orphaned(move || {
                r.fetch_add(1, Ordering::Relaxed);
            });
        };
        // Pending: the reply waits, nothing runs.
        let d = Deferred::new();
        hook(&d);
        let mut rpc = held(0x19);
        assert!(!settle_deferred(&mut rpc, &d, true));
        // Act OK, reply refused by another link: the undo runs, the refusal stands.
        d.resolve(0);
        assert!(settle_deferred(&mut rpc, &d, true));
        assert_eq!(ran.load(Ordering::Relaxed), 1);
        assert_eq!(
            rpc.rpc_result, 0x19,
            "the guest still reads the seat's refusal"
        );
        // A retried post (the first failed) settles again — the undo does not run twice.
        assert!(settle_deferred(&mut rpc, &d, true));
        assert_eq!(ran.load(Ordering::Relaxed), 1);
        // Act OK, reply OK: nothing to undo.
        let ok = Deferred::new();
        hook(&ok);
        ok.resolve(0);
        let mut fine = held(0);
        assert!(settle_deferred(&mut fine, &ok, true));
        assert_eq!(ran.load(Ordering::Relaxed), 1);
        assert!(
            ok.run_orphan_undo(),
            "still attached: it was never an orphan"
        );
        assert_eq!(ran.load(Ordering::Relaxed), 2, "run by hand above, once");
        // Act refused: its status is the reply's (and the alloc params'), and no undo runs.
        let bad = Deferred::new();
        hook(&bad);
        bad.resolve(0x56);
        let mut r = held(0);
        assert!(settle_deferred(&mut r, &bad, true));
        assert_eq!((r.rpc_result, r.rpc_result_private), (0x56, 0x56));
        assert_eq!(
            &r.payload[RM_ALLOC_PARAMS_STATUS_AT..RM_ALLOC_PARAMS_STATUS_AT + 4],
            &0x56u32.to_le_bytes()
        );
        assert_eq!(ran.load(Ordering::Relaxed), 2);
        // No undo attached: named orphaned, nothing runs, and a clone shares the cell.
        let bare = Deferred::new();
        let twin = bare.clone();
        bare.resolve(0);
        let mut refused = held(0x19);
        assert!(settle_deferred(&mut refused, &twin, true));
        assert!(!bare.run_orphan_undo());
    }

    /// ★ EXPERIMENT `KF3_GSS_NATIVE`: the host's bytes land in the held reply when the act succeeded,
    /// only inside the payload the reply has, and never when the act failed or the reply is a refusal.
    #[test]
    fn a_reply_patch_lands_only_on_success_and_only_inside_the_reply() {
        let bytes = vec![0xAB, 0xCD, 0xEF];
        // Success: written at `at`, nothing else moves.
        let d = Deferred::new();
        d.set_reply_patch(10, bytes.clone());
        d.resolve(0);
        let mut rpc = held(0);
        assert!(settle_deferred(&mut rpc, &d, false));
        assert_eq!(&rpc.payload[10..13], &bytes[..]);
        assert_eq!(rpc.payload.len(), 40);
        assert!(rpc.payload[..10].iter().all(|b| *b == 0));
        assert!(rpc.payload[13..].iter().all(|b| *b == 0));
        // A retried post settles again from a fresh clone: the patch is still there.
        let mut again = held(0);
        assert!(settle_deferred(&mut again, &d, false));
        assert_eq!(again.payload, rpc.payload);
        // Failure: the status is the reply's and the bytes are NOT written.
        let bad = Deferred::new();
        bad.set_reply_patch(10, bytes.clone());
        bad.resolve(0x56);
        let mut r = held(0);
        assert!(settle_deferred(&mut r, &bad, false));
        assert_eq!(r.rpc_result, 0x56);
        assert!(r.payload.iter().all(|b| *b == 0));
        // A patch that does not fit the reply is dropped whole (the reply never grows).
        let big = Deferred::new();
        big.set_reply_patch(38, bytes);
        big.resolve(0);
        let mut r = held(0);
        assert!(settle_deferred(&mut r, &big, false));
        assert_eq!(r.payload.len(), 40);
        assert!(r.payload.iter().all(|b| *b == 0));
        // A refused reply (another link) is not patched.
        let refused = Deferred::new();
        refused.set_reply_patch(0, vec![1]);
        refused.resolve(0);
        let mut r = held(0x19);
        assert!(settle_deferred(&mut r, &refused, false));
        assert!(r.payload.iter().all(|b| *b == 0));
    }

    /// ⊘ The default defers nothing, and a chain reports the one link that deferred wherever it
    /// sits — the channel link is seated ahead of the answerer.
    #[test]
    fn a_chain_reports_the_link_that_deferred() {
        let d = Deferred::new();
        let mut chain = PolicyChain::new(vec![
            Box::new(Defers(None)),
            Box::new(Defers(Some(d.clone()))),
        ]);
        assert_eq!(chain.defers(&cmd()), Some(d));
        assert_eq!(
            chain.defers(&cmd()),
            None,
            "a cell is carried for ONE command"
        );
    }

    /// ★ A refused alloc's status is where the guest reads it — the params `status`, not only the
    /// envelope (`rpc.c:11236-11241`). `[measured kf3m2]` without it every refusal was a success.
    #[test]
    fn a_refused_alloc_carries_its_status_in_the_params() {
        let mut rpc = OutgoingRpc {
            function: 103,
            sequence: 9,
            rpc_result: 0x56,
            rpc_result_private: 0x56,
            payload: vec![0u8; 40],
        };
        stamp_alloc_status(&mut rpc, 0x56);
        assert_eq!(
            &rpc.payload[RM_ALLOC_PARAMS_STATUS_AT..RM_ALLOC_PARAMS_STATUS_AT + 4],
            &0x56u32.to_le_bytes()
        );
        let mut empty = OutgoingRpc {
            payload: Vec::new(),
            ..rpc.clone()
        };
        stamp_alloc_status(&mut empty, 0x40);
        assert_eq!(
            &empty.payload[RM_ALLOC_PARAMS_STATUS_AT..],
            &0x40u32.to_le_bytes(),
            "an empty body grows to hold it"
        );
        let mut ok = OutgoingRpc {
            payload: vec![0u8; 40],
            ..rpc
        };
        stamp_alloc_status(&mut ok, 0);
        assert!(
            ok.payload.iter().all(|&b| b == 0),
            "a success stamps nothing"
        );
    }
}

#[cfg(test)]
mod a_life_ends_and_the_next_one_boots {
    //! ★★★★★ **w828 — GSP re-init after the guest's last close.** The unit half; the register
    //! answers per family are `kf-chip/tests/gsp_teardown.rs`, and the bench half is the
    //! reopen stress lane (`scripts/bench/reopen_stress.sh`).
    use super::*;
    use crate::element::{ElementLayout, TransportHdr};
    use crate::ring::MsgqAbi;
    use crate::rpc::{FunctionCodes, RpcAbi};

    /// A GSP Axis-A bundle for the bench driver. Only its shape matters here: these tests never
    /// decode an element.
    pub(super) fn abi() -> GspAbi {
        let table =
            kf_abi::versions::table_for(kf_abi::versions::BENCH_DRIVER).expect("bench driver");
        let wire = table.gsp_element_wire();
        let transport = match wire.transport() {
            None => TransportHdr::None,
            Some(t) => TransportHdr::Mctp {
                header_off: t.header_off,
                header_word: t.header_word,
                nvdm_off: t.nvdm_off,
                nvdm_word: t.nvdm_word,
            },
        };
        let element = ElementLayout::new(
            wire.hdr_size(),
            wire.checksum_off(),
            wire.seqnum_off(),
            wire.elem_count_off(),
            transport,
        )
        .expect("element layout");
        let init = table.gsp_init_args_wire();
        let codes = FunctionCodes {
            set_guest_system_info: 1,
            set_guest_system_info_ext: 64,
            free: 10,
            dup_object: 21,
            unloading_guest_driver: 47,
            get_gsp_static_info: 65,
            init_gsp_trace_crash_buffer: 228,
            continuation_record: 71,
            gsp_set_system_info: 72,
            set_registry: 73,
            ecc_notifier_write_ack: 202,
            update_bar_pde: 70,
            gsp_rm_control: 76,
            gsp_rm_alloc: 103,
            gsp_init_done: 0x1001,
            post_event: 0x1002,
            rc_triggered: 0x1003,
            set_page_directory:
                kf_abi::generated::matrix::RPC_FUNCTIONS_NV_VGPU_MSG_FUNCTION_SET_PAGE_DIRECTORY
                    .everywhere_u32(),
            unset_page_directory:
                kf_abi::generated::matrix::RPC_FUNCTIONS_NV_VGPU_MSG_FUNCTION_UNSET_PAGE_DIRECTORY
                    .everywhere_u32(),
        };
        GspAbi {
            msgq: MsgqAbi {
                version: 0,
                msg_size_min: 4096,
                swap_rx_flag: 1,
                region_page_size: 4096,
            },
            element,
            rpc: RpcAbi {
                header_version: 0x0300_0000,
                codes,
            },
            element_size_max: table.gsp_element_size_max(),
            init_args: InitArgsLayout {
                shared_mem_pa_off: 0,
                pte_count_off: 8,
                cmd_queue_off_off: 16,
                stat_queue_off_off: 24,
                min_size: init.min_size(),
                element_hdr_size_off: init.element_hdr_size_off(),
            },
            driver: *table,
        }
    }

    /// A guest RAM the test proves is never touched.
    pub(super) struct Untouchable;
    impl GuestRam for Untouchable {
        fn read(&mut self, gpa: u64, _buf: &mut [u8]) -> Result<(), crate::fault::RamRefused> {
            panic!("a suspended GSP read guest RAM at {gpa:#x}");
        }
        fn write(&mut self, gpa: u64, _bytes: &[u8]) -> Result<(), crate::fault::RamRefused> {
            panic!("a suspended GSP wrote guest RAM at {gpa:#x}");
        }
    }

    fn running(after: AfterSuspend) -> GspFsm {
        let mut f = GspFsm::new(abi());
        f.phase = BootPhase::Running;
        f.after_suspend = after;
        f.swgen0_pending = true; // the fn-47 reply was just posted
        f
    }

    /// ★ Falcon regime: fn-47 leaves the processor SUSPENDED with WPR2 up; the life ends only at
    /// the teardown ucode's STARTCPU (E2), which is what the guest then does.
    #[test]
    fn a_falcon_life_waits_for_its_teardown_ucode() {
        let mut f = running(AfterSuspend::AwaitsTeardownUcode);
        let mut r = ServiceReport::default();
        f.suspend(&mut r);
        assert_eq!(r.transitions, vec![Transition::E9]);
        assert_eq!(f.phase(), BootPhase::Suspending);
        assert!(
            f.observe().wpr2_up && f.observe().suspended,
            "WPR2 stays up until FWSEC-SB / Booter Unload"
        );
        assert_eq!(
            f.gsp_startcpu(),
            Transition::E2,
            "FWSEC-SB's STARTCPU ends the life"
        );
        assert!(!f.observe().wpr2_up && !f.observe().suspended);
        assert_eq!(f.gsp_startcpu(), Transition::E1, "and the next open boots");
        assert!(f.observe().wpr2_up);
    }

    /// ★★★ FSP regime: nothing the driver writes ends the life, so the firmware halts itself —
    /// WPR2 down, binding dropped, the suspend sentinel still readable, the status IRQ kept.
    #[test]
    fn an_fsp_life_halts_itself_and_the_next_open_boots() {
        let mut f = running(AfterSuspend::FirmwareHalts);
        let mut r = ServiceReport::default();
        f.suspend(&mut r);
        assert_eq!(r.transitions, vec![Transition::E9, Transition::E13]);
        assert_eq!(f.phase(), BootPhase::Halted);
        let o = f.observe();
        assert!(!o.wpr2_up, "the next _kgspBootGspRm must read WPR2 down");
        assert!(
            o.suspended,
            "kgspWaitForProcessorSuspend still reads the sentinel"
        );
        assert!(o.swgen0_pending, "the reply's interrupt survives the halt");
        assert!(
            matches!(f.queue(), QueueState::Unbound),
            "the dead life's binding is dropped by value"
        );
        assert_eq!(
            f.gsp_startcpu(),
            Transition::E1,
            "the next open's FSP boot starts the processor"
        );
        assert!(
            !f.observe().suspended,
            "a boot clears the sentinel, freeing MAILBOX0 for FMC errors"
        );
        assert!(f.observe().wpr2_up);
    }

    /// ★★★★★ `[measured 8dd2bbdf, run_cl_U_cup2_3]` a doorbell while SUSPENDED must read nothing:
    /// the dead life's queue may already be the guest's next allocation.
    #[test]
    fn a_suspended_processor_services_no_doorbell_and_reads_no_ram() {
        let mut f = running(AfterSuspend::AwaitsTeardownUcode);
        f.suspend(&mut ServiceReport::default());
        f.note_command_doorbells(2);
        let mut ram = Untouchable;
        let mut policy = EchoOk;
        assert_eq!(
            f.service_one_deferred_command(&mut ram, &mut policy)
                .unwrap_err(),
            GspFault::ProcessorSuspended
        );
        assert_eq!(
            f.pending_command_doorbells(),
            1,
            "the refused doorbell is consumed, not retried forever"
        );
    }
}

#[cfg(test)]
mod a_retry_after_a_failed_boot_reads_its_own_frts_command {
    //! ★★★ v3-initrace — `V3_HW_BOUNDARY_INVENTORY.md` §5.2 L1: after ONE failed GSP boot RM moves
    //! the FRTS region down by `kgspGetWprEndMargin` (`ogkm-580: kernel_gsp.c:5637-5697`) and checks
    //! `WPR2_ADDR_LO` against the moved offset exactly (`kernel_gsp_frts_tu102.c:514-524`). The FSM
    //! now reads the offset out of the FWSEC-FRTS command RM patched into the DMEM image; the model's
    //! encode of it is `kf-chip/tests/wpr_end_margin.rs`, the sequence's latching of the DMA source
    //! `kf-chip/tests/wpr_end_margin.rs` too.
    use super::*;
    use std::collections::BTreeMap;

    /// Guest RAM: a sparse byte map; an address never written is REFUSED (as unregistered RAM is).
    #[derive(Default)]
    struct Ram(BTreeMap<u64, u8>);
    impl Ram {
        fn put32(&mut self, gpa: u64, v: u32) {
            for (i, b) in v.to_le_bytes().into_iter().enumerate() {
                self.0.insert(gpa + i as u64, b);
            }
        }
    }
    impl GuestRam for Ram {
        fn read(&mut self, gpa: u64, buf: &mut [u8]) -> Result<(), crate::fault::RamRefused> {
            let len = buf.len();
            for (i, o) in buf.iter_mut().enumerate() {
                *o = *self
                    .0
                    .get(&(gpa + i as u64))
                    .ok_or(crate::fault::RamRefused {
                        gpa,
                        len,
                        why: "not registered",
                    })?;
            }
            Ok(())
        }
        fn write(&mut self, _gpa: u64, _bytes: &[u8]) -> Result<(), crate::fault::RamRefused> {
            unreachable!("the FRTS command is read, never written")
        }
    }

    const IMAGE: u64 = 0x7654_3000;

    /// What RM leaves in guest RAM for FWSEC (`s_vbiosPatchInterfaceData` over OUR generated
    /// image): the whole DMEM load present (zeros), the mapper's `cmd_in_buffer_offset`, `init_cmd`,
    /// and the FRTS region descriptor.
    fn image(cmd: u32, off_4k: u32, size_4k: u32, media: u32) -> Ram {
        let g = kf_abi::vbios::GENERATED_FWSEC;
        let base = IMAGE + u64::from(g.dmem_phys_base);
        let mut r = Ram::default();
        for o in (0..g.dmem_load_size).step_by(4) {
            r.put32(base + u64::from(o), 0);
        }
        r.put32(
            base + u64::from(g.dmem_mapper_offset + 8),
            g.cmd_in_buffer_offset,
        );
        r.put32(base + u64::from(g.dmem_mapper_offset + 44), cmd);
        let d = base + u64::from(g.cmd_in_buffer_offset + 24);
        for (i, v) in [1, 20, off_4k, size_4k, media].into_iter().enumerate() {
            r.put32(d + 4 * i as u64, v);
        }
        r
    }

    #[test]
    fn the_frts_offset_is_read_from_the_command_and_only_from_an_acceptable_one() {
        let at = 0x1f6e_0000u64; // a margin-shifted FRTS offset (4 KiB units: 0x1f6e0)
        assert_eq!(
            read_frts_command(&mut image(0x15, 0x1f6e0, 0x100, 2), IMAGE),
            Some(at)
        );
        assert_eq!(
            read_frts_command(&mut image(0x19, 0x1f6e0, 0x100, 2), IMAGE),
            None,
            "FWSEC-SB carries no region"
        );
        assert_eq!(
            read_frts_command(&mut image(0x15, 0x1f6e0, 0x200, 2), IMAGE),
            None,
            "only the 1 MiB region FWSEC builds"
        );
        assert_eq!(
            read_frts_command(&mut image(0x15, 0x1f6e0, 0x100, 1), IMAGE),
            None,
            "only the FB medium"
        );
        let mut other = image(0x15, 0x1f6e0, 0x100, 2);
        let g = kf_abi::vbios::GENERATED_FWSEC;
        other.put32(
            IMAGE + u64::from(g.dmem_phys_base + g.dmem_mapper_offset + 8),
            g.cmd_in_buffer_offset + 0x40,
        );
        assert_eq!(
            read_frts_command(&mut other, IMAGE),
            None,
            "a mapper that disagrees with the ROM we generated"
        );
        assert_eq!(
            read_frts_command(&mut Ram::default(), IMAGE),
            None,
            "unregistered RAM is refused, never zero"
        );
        assert_eq!(
            read_frts_command(&mut image(0x15, 0x1f6e0, 0x100, 2), u64::MAX - 8),
            None,
            "no wrap"
        );
    }

    #[test]
    fn the_command_read_for_a_start_is_served_while_wpr2_is_up_and_dropped_when_it_comes_down() {
        let mut f = GspFsm::new(super::a_life_ends_and_the_next_one_boots::abi());
        let mut ram = image(0x15, 0x1f6e0, 0x100, 2);
        assert_eq!(
            f.resolve_frts_command(&mut ram),
            None,
            "nothing started: nothing to read"
        );
        f.fwsec_image = Some(IMAGE); // `BootStep::FwsecCommand`
        assert_eq!(f.gsp_startcpu(), Transition::E1);
        assert_eq!(
            f.observe().frts_offset,
            None,
            "until the device resolves it"
        );
        assert_eq!(f.resolve_frts_command(&mut ram), Some(Some(0x1f6e_0000)));
        assert_eq!(
            f.observe().frts_offset,
            Some(0x1f6e_0000),
            "the retry's FWSEC-FRTS placed it there"
        );
        assert_eq!(f.resolve_frts_command(&mut ram), None, "read once");
        assert_eq!(
            f.gsp_startcpu(),
            Transition::E3,
            "a start while up changes nothing"
        );
        assert_eq!(f.observe().frts_offset, Some(0x1f6e_0000));
        f.enter_halted();
        assert_eq!(
            f.observe().frts_offset,
            None,
            "WPR2 down: nothing is served"
        );
        assert_eq!(
            f.gsp_startcpu(),
            Transition::E1,
            "a start with no DMEM load of its own"
        );
        assert_eq!(f.resolve_frts_command(&mut ram), None, "nothing to read");
        assert_eq!(
            f.observe().frts_offset,
            None,
            "derives — never the previous life's command"
        );
        f.enter_halted();
        f.fwsec_image = Some(IMAGE);
        assert_eq!(f.gsp_startcpu(), Transition::E1);
        assert_eq!(
            f.resolve_frts_command(&mut Ram::default()),
            Some(None),
            "an unreadable command derives"
        );
        assert_eq!(f.observe().frts_offset, None);
    }
}

#[cfg(test)]
mod fn72_is_kept_for_fn65 {
    //! ★ The boot display (`docs/design/V3_DISPLAY.md` §4.11.4 row 2): fn 72 stays unanswered and
    //! reaches no policy, and its body lands in the shared cell — or, with no cell, nothing changes.
    use super::a_life_ends_and_the_next_one_boots::abi;
    use super::*;
    use crate::sysinfo::{StashedSystemInfo, SystemInfoCell};

    /// Guest RAM nothing may touch: an asynchronous RPC posts no reply.
    struct NoReplyRam;
    impl GuestRam for NoReplyRam {
        fn read(&mut self, gpa: u64, _buf: &mut [u8]) -> Result<(), crate::fault::RamRefused> {
            panic!("an asynchronous RPC read guest RAM at {gpa:#x}");
        }
        fn write(&mut self, gpa: u64, _bytes: &[u8]) -> Result<(), crate::fault::RamRefused> {
            panic!("an asynchronous RPC was answered (a write at {gpa:#x})");
        }
    }

    /// A policy no asynchronous RPC may reach.
    struct NoPolicy;
    impl CommandPolicy for NoPolicy {
        fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
            panic!("fn {} reached a policy", cmd.code);
        }
    }

    fn rpc(function: RpcFunction, code: u32, payload: Vec<u8>, sequence: u32) -> RpcCommand {
        RpcCommand {
            function,
            code,
            sequence,
            payload,
            elements: 1,
            delivered: Vec::new(),
        }
    }

    fn answer(f: &mut GspFsm, cmd: &RpcCommand) -> ServiceReport {
        let mut report = ServiceReport::default();
        f.answer(&mut NoReplyRam, &mut NoPolicy, cmd, &mut report)
            .expect("an asynchronous RPC is consumed");
        report
    }

    #[test]
    fn fn72_lands_in_the_cell_and_is_never_answered() {
        let cell = SystemInfoCell::new();
        let mut f = GspFsm::new(abi()).with_system_info_cell(cell.clone());
        let report = answer(
            &mut f,
            &rpc(RpcFunction::GspSetSystemInfo, 72, vec![7; 936], 3),
        );
        assert_eq!(report, ServiceReport::default(), "nothing posted or logged");
        assert_eq!(
            cell.latest(),
            Some(StashedSystemInfo {
                declared_len: 936,
                bytes: vec![7; 936],
                sequence: 3
            })
        );
        // SET_REGISTRY (73) is asynchronous too, and is not kept
        answer(&mut f, &rpc(RpcFunction::SetRegistry, 73, vec![1; 64], 4));
        assert_eq!(cell.latest().map(|s| s.sequence), Some(3));
    }

    #[test]
    fn the_cell_survives_a_device_reset_and_the_next_life_replaces_it() {
        let cell = SystemInfoCell::new();
        let mut f = GspFsm::new(abi()).with_system_info_cell(cell.clone());
        answer(
            &mut f,
            &rpc(RpcFunction::GspSetSystemInfo, 72, vec![1; 936], 1),
        );
        assert_eq!(f.device_reset(), Transition::E11);
        answer(
            &mut f,
            &rpc(RpcFunction::GspSetSystemInfo, 72, vec![2; 928], 1),
        );
        let s = cell.latest().expect("the next driver life's fn 72");
        assert_eq!((s.declared_len, s.bytes[0]), (928, 2));
    }

    /// ⊘ The default (no cell, `gop=off`): fn 72 is dropped unread and the state machine is
    /// byte-identical to one that never saw it.
    #[test]
    fn without_a_cell_fn72_changes_nothing() {
        let mut f = GspFsm::new(abi());
        answer(
            &mut f,
            &rpc(RpcFunction::GspSetSystemInfo, 72, vec![7; 936], 3),
        );
        assert_eq!(f, GspFsm::new(abi()));
    }
}

#[cfg(test)]
mod device_reset_from_every_phase {
    //! ★★★ E11 is wired to QEMU's PCI device reset, so it must be **total from wherever the guest left
    //! the device**: every [`BootPhase`], with and without live state (held replies, command doorbells
    //! not yet serviced, a recorded refusal, the boot-args shadows, an outstanding event batch, a large
    //! RPC half-joined), with and without the shared system-info cell.
    //!
    //! GPU-free and QEMU-free. The device is driven the way the register plane drives it: through
    //! [`GspFsm::mmio_write_with`] with a minimal in-test register model (a [`GspModel`] and a
    //! [`kf_arch::BootSequence`] that map a handful of made-up offsets onto the real [`BootStep`]s),
    //! and through the guest's side of the queues in a sparse fake guest RAM — the region array, the
    //! init args, the page table and both rings are laid out for real, so `publish`, `post`, the
    //! command drain and the Booted → Running edge are the FSM's own code.
    //!
    //! ⊘ Two things are injected rather than reached, and both only where no queue exists yet:
    //! in `Cold` and `ProtectedRegionUp` a held reply is made by calling the private `answer` (the
    //! function the queue drain calls) directly, because there is no ring to carry a command; and
    //! no refusal can be recorded there at all (a refusal is written only after a reply is posted).
    use super::a_life_ends_and_the_next_one_boots::abi;
    use super::*;
    use crate::ring::TxHeader;
    use crate::sysinfo::SystemInfoCell;
    use kf_abi::postevent::PostEvent;
    use kf_arch::gsp::{
        BootSequence, BootStageDesc, BootSteps, FalconDma, GspModel, GspReg, LibosRegionLayout,
    };
    use std::collections::BTreeMap;

    // ---- the in-test register model ---------------------------------------------------------

    const OFF_START: u64 = 0x10;
    const OFF_FWSEC: u64 = 0x18;
    const OFF_FW_LOADED: u64 = 0x20;
    /// ★ 2026-10-14: the resume's Booter Load ([`BootStep::FirmwareResume`]).
    const OFF_FW_RESUME: u64 = 0x28;
    const OFF_TEARDOWN: u64 = 0x30;
    const OFF_ARGS_LO: u64 = 0x40;
    const OFF_ARGS_HI: u64 = 0x44;
    const OFF_PUBLISH: u64 = 0x48;
    const OFF_DOORBELL: u64 = 0x50;
    const OFF_IRQ_CLEAR: u64 = 0x60;
    const OFF_BCR: u64 = 0x70;
    /// Every offset the model can read back.
    const READ_OFFS: [u64; 7] = [0x70, 0x100, 0x104, 0x108, 0x10c, 0x110, 0x114];
    const RMARGS_ID: u64 = 0x0052_4d41_5247_5300;

    /// Exhaustive on purpose: a new [`BootPhase`] stops this test module compiling until every
    /// driver below has been taught about it.
    fn ordinal(p: BootPhase) -> u64 {
        match p {
            BootPhase::Cold => 0,
            BootPhase::ProtectedRegionUp => 1,
            BootPhase::Booted => 2,
            BootPhase::Running => 3,
            BootPhase::Suspending => 4,
            BootPhase::Halted => 5,
        }
    }

    const PHASES: [BootPhase; 6] = [
        BootPhase::Cold,
        BootPhase::ProtectedRegionUp,
        BootPhase::Booted,
        BootPhase::Running,
        BootPhase::Suspending,
        BootPhase::Halted,
    ];

    struct Seq {
        after: AfterSuspend,
    }

    impl BootSequence for Seq {
        fn stages(&self) -> &'static [BootStageDesc] {
            &[]
        }
        fn on_write(
            &self,
            _model: &dyn GspModel,
            w: &RegWrite,
            _ctx: &BootContext,
            state: &mut ArchBootState,
        ) -> BootSteps {
            match w.off {
                OFF_START => BootSteps::one(BootStep::StartProcessor),
                OFF_FWSEC => {
                    // generation-local state that only `arch_state` holds, so a reset has to clear it
                    state.set_latch(0, w.val);
                    BootSteps::one(BootStep::FwsecCommand(w.val))
                }
                OFF_FW_LOADED => BootSteps::one(BootStep::FirmwareLoaded),
                OFF_FW_RESUME => BootSteps::one(BootStep::FirmwareResume),
                OFF_TEARDOWN => BootSteps::one(BootStep::Teardown),
                OFF_ARGS_LO => BootSteps::one(BootStep::BootArgsLo(w.val as u32)),
                OFF_ARGS_HI => {
                    state.window_write_u32(0, w.val as u32);
                    BootSteps::one(BootStep::BootArgsHi(w.val as u32))
                }
                OFF_PUBLISH => BootSteps::one(BootStep::PublishBootArgs(w.val)),
                OFF_DOORBELL => BootSteps::one(BootStep::CommandDoorbell),
                OFF_IRQ_CLEAR => BootSteps::one(BootStep::ClearStatusIrq),
                _ => BootSteps::none(),
            }
        }
        fn after_suspend(&self) -> AfterSuspend {
            self.after
        }
    }

    struct Mini {
        seq: Seq,
    }

    impl GspModel for Mini {
        fn decode_reg(&self, bar: u8, off: u64) -> Option<GspReg> {
            if bar != 0 {
                return None;
            }
            match off {
                0x70 => Some(GspReg::GspRiscvBcrCtrl),
                0x100 => Some(GspReg::GspFalconCpuctl),
                0x104 => Some(GspReg::GspFalconMailbox0),
                0x108 => Some(GspReg::GspFalconMailbox1),
                0x10c => Some(GspReg::GspFalconIrqstat),
                0x110 => Some(GspReg::Wpr2AddrLo),
                0x114 => Some(GspReg::GspRiscvCpuctl),
                _ => None,
            }
        }
        fn is_startcpu(&self, _value: u64) -> bool {
            false
        }
        fn is_booter_unload(&self, _sec2_mailbox0: u32) -> bool {
            false
        }
        fn is_swgen0_clear(&self, _value: u64) -> bool {
            false
        }
        /// Every observable field reaches some register, so a read-back compares the whole
        /// [`GspObservation`].
        fn encode(&self, reg: GspReg, o: &GspObservation) -> Option<u64> {
            Some(match reg {
                GspReg::GspRiscvBcrCtrl => o.riscv_bcr_ctrl.map_or(0xffff_ffff, u64::from),
                GspReg::GspFalconCpuctl => {
                    ordinal(o.stage)
                        | (u64::from(o.wpr2_up) << 8)
                        | (u64::from(o.riscv_active) << 9)
                        | (u64::from(o.suspended) << 10)
                        | (u64::from(o.swgen0_pending) << 11)
                }
                GspReg::GspFalconMailbox0 => u64::from(o.boot_args_lo),
                GspReg::GspFalconMailbox1 => u64::from(o.boot_args_hi),
                GspReg::GspFalconIrqstat => u64::from(o.swgen0_pending),
                GspReg::Wpr2AddrLo => o.frts_offset.unwrap_or(0xdead),
                GspReg::GspRiscvCpuctl => u64::from(o.riscv_active),
                _ => return None,
            })
        }
        fn libos_region_layout(&self) -> LibosRegionLayout {
            LibosRegionLayout {
                entry_stride: 24,
                id_offset: 0,
                pa_offset: 8,
                size_offset: 16,
                max_entries: 4,
                rmargs_id: RMARGS_ID,
            }
        }
        fn falcon_dma(&self, _bar: u8, _off: u64, _value: u64) -> Option<FalconDma> {
            None
        }
        fn boot_sequence(&self) -> &dyn BootSequence {
            &self.seq
        }
    }

    // ---- the policy: the three outcomes a reset must be able to interrupt ---------------------

    /// `GSP_RM_CONTROL` is unmodelled (a refusal, recorded), `GSP_RM_ALLOC` is held for its
    /// refresh (a held reply), everything else is echoed.
    struct Live;
    impl CommandPolicy for Live {
        fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
            match cmd.function {
                RpcFunction::RmControl => None,
                _ => Some(Reply {
                    rpc_result: 0,
                    rpc_result_private: None,
                    body: cmd.payload.clone(),
                }),
            }
        }
        fn holds_for_refresh(&self, cmd: &RpcCommand) -> bool {
            cmd.function == RpcFunction::RmAlloc
        }
    }

    // ---- sparse fake guest RAM -----------------------------------------------------------------

    const PAGE: u64 = 4096;

    /// Unwritten pages are refused, as unregistered RAM is.
    #[derive(Default, Clone, PartialEq, Eq)]
    struct Ram(BTreeMap<u64, Box<[u8; PAGE as usize]>>);

    impl Ram {
        fn put(&mut self, gpa: u64, bytes: &[u8]) {
            for (i, b) in bytes.iter().enumerate() {
                let at = gpa + i as u64;
                self.0
                    .entry(at / PAGE)
                    .or_insert_with(|| Box::new([0; PAGE as usize]))[(at % PAGE) as usize] = *b;
            }
        }
        fn put32(&mut self, gpa: u64, v: u32) {
            self.put(gpa, &v.to_le_bytes());
        }
        fn put64(&mut self, gpa: u64, v: u64) {
            self.put(gpa, &v.to_le_bytes());
        }
        fn get32(&mut self, gpa: u64) -> u32 {
            let mut b = [0u8; 4];
            self.read(gpa, &mut b).expect("mapped");
            u32::from_le_bytes(b)
        }
    }

    impl GuestRam for Ram {
        fn read(&mut self, gpa: u64, buf: &mut [u8]) -> Result<(), crate::fault::RamRefused> {
            let len = buf.len();
            for (i, o) in buf.iter_mut().enumerate() {
                let at = gpa + i as u64;
                *o = self
                    .0
                    .get(&(at / PAGE))
                    .map(|p| p[(at % PAGE) as usize])
                    .ok_or(crate::fault::RamRefused {
                        gpa,
                        len,
                        why: "not registered",
                    })?;
            }
            Ok(())
        }
        fn write(&mut self, gpa: u64, bytes: &[u8]) -> Result<(), crate::fault::RamRefused> {
            self.put(gpa, bytes);
            Ok(())
        }
    }

    // ---- the guest's memory layout --------------------------------------------------------------

    const ARRAY: u64 = 0x10_0000;
    const ARGS: u64 = 0x11_0000;
    const REGION: u64 = 0x20_0000;
    const IMAGE: u64 = 0x7654_3000;
    const ELEM: u64 = PAGE;
    /// Elements per ring. Large enough to hold one maximum-size RPC and the traffic around it.
    const COUNT: u64 = 64;
    const CMD_OFF: u64 = 0x1000;
    const QSIZE: u64 = ELEM * (1 + COUNT);
    const STAT_OFF: u64 = CMD_OFF + QSIZE;
    const REGION_PAGES: u64 = (STAT_OFF + QSIZE) / PAGE;
    const RX_HDR_OFF: u64 = 32;

    /// One device and the guest around it: the FSM, the guest RAM, the register model, the policy.
    struct Dev {
        fsm: GspFsm,
        ram: Ram,
        model: Mini,
        policy: Live,
        /// The guest's command `seqNum` and the producer's `writePtr`.
        guest_seq: u32,
        guest_wp: u32,
        /// A `GSP_SET_SYSTEM_INFO` body was sent this life.
        sent_sysinfo: bool,
    }

    // GPU-free child fixture exercises the real queue publication path. Process-local
    // environment avoids unsafe set_var and races between parallel Rust tests.
    #[test]
    fn reply_body_trace_fixture() {
        let Ok(mode) = std::env::var("KF_POST_TRACE_FIXTURE") else {
            return;
        };
        let after = AfterSuspend::AwaitsTeardownUcode;
        let mut d = Dev::life(GspFsm::new(abi()), after);
        d.start_with_frts();
        d.firmware_loaded();
        d.publish();
        let w = d.fsm.abi.driver.rm_control_wire();
        let params = if mode == "large" { 5000 } else { 4 };
        let selector: u32 = if mode == "large" {
            0x0073_0245
        } else {
            0x2080_0a58
        };
        let mut payload = vec![0; w.params_off + params];
        payload[8..12].copy_from_slice(&selector.to_le_bytes());
        payload[w.params_size_off..w.params_size_off + 4]
            .copy_from_slice(&(params as u32).to_le_bytes());
        payload[w.params_off..].fill(0xab);
        let rpc = OutgoingRpc {
            function: 76,
            sequence: 9,
            rpc_result: 0,
            rpc_result_private: 0,
            payload,
        };
        // A failed publication must not consume a record or report a sent body.
        let queue = std::mem::replace(&mut d.fsm.queue, QueueState::Unbound);
        assert!(d.fsm.post(&mut d.ram, &rpc).is_err());
        d.fsm.queue = queue;
        // Board/static-info replies and unrelated controls must remain outside
        // the fixed display diagnostic allowlist, even with capture enabled.
        let mut outside = rpc.clone();
        outside.function = 65;
        d.guest_drains_status();
        d.fsm.post(&mut d.ram, &outside).unwrap();
        outside.function = 76;
        outside.payload[8..12].copy_from_slice(&0x2080_0a89u32.to_le_bytes());
        d.guest_drains_status();
        d.fsm.post(&mut d.ram, &outside).unwrap();
        let count = match mode.as_str() {
            "reset" => 514,
            "large" => 100,
            _ => 1,
        };
        for i in 0..count {
            if mode == "reset" && i == 512 {
                d.fsm.device_reset();
                d = Dev::life(d.fsm, after);
                d.start_with_frts();
                d.firmware_loaded();
                d.publish();
            }
            d.guest_drains_status();
            d.fsm
                .post(&mut d.ram, &rpc)
                .expect("fixture reply fits the ring");
        }
    }

    fn reply_body_fixture_output(mode: &str, enabled: bool) -> Vec<String> {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child
            .args([
                "reply_body_trace_fixture",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("KF_POST_TRACE_FIXTURE", mode);
        if enabled {
            child.env("KF3_RPC_BODY_TRACE", "1");
        } else {
            child.env_remove("KF3_RPC_BODY_TRACE");
        }
        let output = child.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stderr)
            .lines()
            .filter(|l| l.starts_with("kf-gsp: reply-body "))
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn posted_reply_body_is_logged_once_and_failed_post_is_absent() {
        let rows = reply_body_fixture_output("one", true);
        assert_eq!(
            rows.len(),
            1,
            "capture only the successfully posted a58 reply"
        );
        assert!(rows[0].contains("selector=0x20800a58"));
        assert!(rows[0].ends_with("abababab"));
    }

    #[test]
    fn guest_reset_does_not_rearm_the_reply_record_budget() {
        assert_eq!(reply_body_fixture_output("reset", true).len(), 512);
    }

    #[test]
    fn large_reply_dumps_obey_both_per_record_and_total_byte_limits() {
        let rows = reply_body_fixture_output("large", true);
        assert_eq!(rows.len(), 64, "256KiB total at4096bytes per record");
        for row in rows {
            assert!(row.contains("saved=4096 truncated=1"));
            assert_eq!(row.split("body=").last().unwrap().len(), 8192);
        }
    }

    #[test]
    fn reply_body_diagnostics_are_disabled_by_default() {
        assert!(reply_body_fixture_output("one", false).is_empty());
    }

    impl Dev {
        /// A guest life with its queues laid out in fresh RAM, against `fsm`.
        fn life(fsm: GspFsm, after: AfterSuspend) -> Dev {
            let il = fsm.abi.init_args;
            let hdr_size = fsm.abi.element.hdr_size() as u64;
            let swap = fsm.abi.msgq.swap_rx_flag;
            let mut ram = Ram::default();
            // the LibOS region array: entry 0 is RMARGS
            ram.put64(ARRAY, RMARGS_ID);
            ram.put64(ARRAY + 8, ARGS);
            // MESSAGE_QUEUE_INIT_ARGUMENTS
            let mut args = vec![0u8; il.min_size.max(64)];
            let mut set64 =
                |off: usize, v: u64| args[off..off + 8].copy_from_slice(&v.to_le_bytes());
            set64(il.shared_mem_pa_off, REGION);
            set64(il.cmd_queue_off_off, CMD_OFF);
            set64(il.stat_queue_off_off, STAT_OFF);
            if let Some(off) = il.element_hdr_size_off {
                set64(off, hdr_size);
            }
            args[il.pte_count_off..il.pte_count_off + 4]
                .copy_from_slice(&(REGION_PAGES as u32).to_le_bytes());
            ram.put(ARGS, &args);
            // the region's page table lives in its own first page
            for i in 0..REGION_PAGES {
                ram.put64(REGION + i * 8, REGION + i * PAGE);
            }
            // the guest's command queue header
            let hdr = TxHeader {
                version: 0,
                size: QSIZE as u32,
                msg_size: ELEM as u32,
                msg_count: COUNT as u32,
                write_ptr: 0,
                flags: swap,
                rx_hdr_off: RX_HDR_OFF as u32,
                entry_off: ELEM as u32,
            };
            ram.put(REGION + CMD_OFF, &hdr.encode());
            // the DMEM image RM patched for FWSEC-FRTS (`a_retry_after_a_failed_boot_…`'s fixture)
            let g = kf_abi::vbios::GENERATED_FWSEC;
            let base = IMAGE + u64::from(g.dmem_phys_base);
            ram.put(base, &vec![0u8; g.dmem_load_size as usize]);
            ram.put32(
                base + u64::from(g.dmem_mapper_offset + 8),
                g.cmd_in_buffer_offset,
            );
            ram.put32(base + u64::from(g.dmem_mapper_offset + 44), 0x15);
            let desc = base + u64::from(g.cmd_in_buffer_offset + 24);
            for (i, v) in [1u32, 20, 0x1f6e0, 0x100, 2].into_iter().enumerate() {
                ram.put32(desc + 4 * i as u64, v);
            }
            Dev {
                fsm,
                ram,
                model: Mini { seq: Seq { after } },
                policy: Live,
                guest_seq: 0,
                guest_wp: 0,
                sent_sysinfo: false,
            }
        }

        fn write(&mut self, off: u64, val: u64) -> String {
            format!(
                "{:?}",
                self.fsm
                    .mmio_write_with(&mut self.ram, &self.model, &mut self.policy, 0, off, val)
            )
        }

        fn read(&self, off: u64) -> Option<Result<u64, GspFault>> {
            self.fsm.mmio_read_with(&self.model, 0, off)
        }

        /// The guest's own command element: the wire format is shared with `post`'s, which is not
        /// what is under test here.
        fn send(&mut self, code: u32, payload: Vec<u8>) {
            let abi = self.fsm.abi;
            let rpc = OutgoingRpc {
                function: code,
                sequence: 100 + self.guest_seq,
                rpc_result: 0,
                rpc_result_private: 0,
                payload,
            };
            let bytes = encode_message(
                &abi.element,
                abi.rpc.header_version,
                ELEM as u32,
                abi.element_size_max,
                self.guest_seq,
                &rpc,
            )
            .expect("a command that fits");
            let n = bytes.len() as u64 / ELEM;
            for i in 0..n {
                let slot = (u64::from(self.guest_wp) + i) % COUNT;
                let from = (i * ELEM) as usize;
                self.ram.put(
                    REGION + CMD_OFF + ELEM + slot * ELEM,
                    &bytes[from..from + ELEM as usize],
                );
            }
            self.guest_wp = ((u64::from(self.guest_wp) + n) % COUNT) as u32;
            self.ram.put32(REGION + CMD_OFF + 16, self.guest_wp);
            self.guest_seq += 1;
        }

        /// The guest ISR: it has read everything the status ring holds (writes its `readPtr`).
        fn guest_drains_status(&mut self) {
            let wp = self.ram.get32(REGION + STAT_OFF + 16);
            self.ram.put32(REGION + CMD_OFF + RX_HDR_OFF, wp);
        }

        /// Ring the command doorbell and let the worker service it.
        fn ring(&mut self) -> String {
            let w = self.write(OFF_DOORBELL, 1);
            let s = format!(
                "{:?}",
                self.fsm
                    .service_one_deferred_command(&mut self.ram, &mut self.policy)
            );
            format!("{w} / {s}")
        }

        fn start_with_frts(&mut self) -> String {
            let a = self.write(OFF_FWSEC, IMAGE);
            let b = self.write(OFF_START, 1);
            let c = format!("{:?}", self.fsm.resolve_frts_command(&mut self.ram));
            format!("{a} / {b} / {c}")
        }

        fn firmware_loaded(&mut self) -> String {
            self.write(OFF_FW_LOADED, 1)
        }

        fn publish(&mut self) -> String {
            let a = self.write(OFF_ARGS_LO, ARRAY & 0xffff_ffff);
            let b = self.write(OFF_ARGS_HI, ARRAY >> 32);
            let c = self.write(OFF_PUBLISH, ARRAY);
            format!("{a} / {b} / {c}")
        }

        fn run(&mut self) -> String {
            self.guest_drains_status();
            self.ring()
        }

        /// A set-system-info (async), an unmodelled control (a refusal) and an alloc (a held reply).
        fn queue_traffic(&mut self) -> String {
            let abi = self.fsm.abi;
            self.send(abi.rpc.codes.gsp_set_system_info, vec![7; 936]);
            self.sent_sysinfo = true;
            self.send(abi.rpc.codes.gsp_rm_control, vec![0; 40]);
            self.send(abi.rpc.codes.gsp_rm_alloc, vec![0; 64]);
            self.ring()
        }

        /// fn 47, the guest's last close.
        fn unload(&mut self) -> String {
            let code = self.fsm.abi.rpc.codes.unloading_guest_driver;
            self.send(code, vec![0; 8]);
            self.ring()
        }

        fn deliver_an_event(&mut self) -> String {
            let ev = PostEvent {
                client: 1,
                event: 2,
                notify_index: 3,
            };
            format!("{:?}", self.fsm.deliver_events(&mut self.ram, &[ev]))
        }

        /// The head of a `GSP_RM_CONTROL` that needs continuations: a maximum-size message whose
        /// declared params are longer — the assembler is left holding it.
        fn open_a_large_control(&mut self) -> String {
            let abi = self.fsm.abi;
            let w = abi.driver.rm_control_wire();
            let len =
                abi.element_size_max as usize - abi.element.hdr_size() - crate::large::RPC_HEADER;
            let mut payload = vec![0u8; len];
            payload[w.params_size_off..w.params_size_off + 4]
                .copy_from_slice(&(1u32 << 16).to_le_bytes());
            self.send(abi.rpc.codes.gsp_rm_control, payload);
            self.ring()
        }

        /// A command held where there is no ring to carry it: the private `answer`, as the drain calls it.
        fn hold_a_reply_directly(&mut self) {
            let cmd = RpcCommand {
                function: RpcFunction::RmAlloc,
                code: 103,
                sequence: 9,
                payload: vec![0u8; 64],
                elements: 1,
                delivered: Vec::new(),
            };
            let mut report = ServiceReport::default();
            self.fsm
                .answer(&mut self.ram, &mut self.policy, &cmd, &mut report)
                .expect("a held reply posts nothing");
        }

        /// Pending doorbells, the boot-args shadows and the BCR shadow: register-plane state with no
        /// queue behind it.
        fn dirty_the_register_plane(&mut self) {
            self.write(OFF_ARGS_LO, 0xaaaa_0001);
            self.write(OFF_ARGS_HI, 0x0000_00bb);
            self.write(OFF_BCR, 0x1234);
            self.write(OFF_DOORBELL, 1);
            self.write(OFF_DOORBELL, 1);
            self.fsm.note_command_doorbells(1);
        }

        /// Everything an outside reader can see of the device.
        fn snapshot(&self) -> String {
            let reads: Vec<_> = READ_OFFS.iter().map(|o| self.read(*o)).collect();
            format!(
                "{:?} {:?} reads={reads:?} held={} pending={} cmd_seq={} refusals={} large_open={}",
                self.fsm.phase(),
                self.fsm.observe(),
                self.fsm.held_len(),
                self.fsm.pending_command_doorbells(),
                self.fsm.command_seq(),
                self.fsm.refusals().total(),
                self.fsm.large.is_open(),
            )
        }
    }

    fn fresh(cell: Option<&SystemInfoCell>) -> GspFsm {
        let f = GspFsm::new(abi());
        match cell {
            Some(c) => f.with_system_info_cell(c.clone()),
            None => f,
        }
    }

    /// Drive a fresh device into `phase`; with `live`, leave as much in flight as that phase can hold.
    ///
    /// ★ The `match` is exhaustive: a new [`BootPhase`] must be given a driver here.
    fn drive(phase: BootPhase, live: bool, cell: Option<&SystemInfoCell>) -> Dev {
        let after = match phase {
            BootPhase::Halted => AfterSuspend::FirmwareHalts,
            BootPhase::Cold
            | BootPhase::ProtectedRegionUp
            | BootPhase::Booted
            | BootPhase::Running
            | BootPhase::Suspending => AfterSuspend::AwaitsTeardownUcode,
        };
        let mut d = Dev::life(fresh(cell), after);
        match phase {
            BootPhase::Cold => {
                if live {
                    d.write(OFF_FWSEC, IMAGE); // a DMEM load nobody started
                    d.dirty_the_register_plane();
                    d.hold_a_reply_directly();
                }
            }
            BootPhase::ProtectedRegionUp => {
                d.start_with_frts();
                if live {
                    d.dirty_the_register_plane();
                    d.hold_a_reply_directly();
                }
            }
            BootPhase::Booted => {
                d.start_with_frts();
                d.firmware_loaded();
                d.publish();
                if live {
                    d.queue_traffic();
                }
            }
            BootPhase::Running => {
                d.start_with_frts();
                d.firmware_loaded();
                d.publish();
                d.run();
                if live {
                    d.queue_traffic();
                    d.deliver_an_event();
                    d.open_a_large_control();
                }
            }
            BootPhase::Suspending | BootPhase::Halted => {
                d.start_with_frts();
                d.firmware_loaded();
                d.publish();
                d.run();
                if live {
                    d.queue_traffic();
                    d.deliver_an_event();
                }
                d.unload();
            }
        }
        if live {
            d.fsm.note_command_doorbells(2);
        }
        d
    }

    /// Booter Unload ends the life instead of the firmware halting itself.
    fn drive_halted_by_booter_unload(live: bool, cell: Option<&SystemInfoCell>) -> Dev {
        let mut d = Dev::life(fresh(cell), AfterSuspend::AwaitsTeardownUcode);
        d.start_with_frts();
        d.firmware_loaded();
        d.publish();
        d.run();
        if live {
            d.queue_traffic();
            d.deliver_an_event();
        }
        d.write(OFF_TEARDOWN, 1);
        if live {
            d.fsm.note_command_doorbells(1);
        }
        d
    }

    // ---- 0. the drivers really do what they claim ---------------------------------------------

    #[test]
    fn every_driver_reaches_its_phase_with_the_live_state_it_claims() {
        let ph = |p: BootPhase| usize::try_from(ordinal(p)).expect("small");
        for phase in PHASES {
            assert_eq!(ph(phase), PHASES.iter().position(|p| *p == phase).unwrap());
            let quiet = drive(phase, false, None);
            assert_eq!(quiet.fsm.phase(), phase, "{phase:?}: quiet driver");
            assert_eq!(quiet.fsm.held_len(), 0, "{phase:?}: nothing held");
            assert_eq!(quiet.fsm.pending_command_doorbells(), 0, "{phase:?}");

            let d = drive(phase, true, None);
            assert_eq!(d.fsm.phase(), phase, "{phase:?}: live driver");
            assert!(d.fsm.held_len() > 0, "{phase:?}: a reply is held");
            assert!(
                d.fsm.pending_command_doorbells() > 0,
                "{phase:?}: a doorbell waits"
            );
            assert_ne!(
                d.fsm.observe().boot_args_lo,
                0,
                "{phase:?}: the shadows were written"
            );
            let has_queue = ordinal(phase) >= 2;
            assert_eq!(
                d.fsm.refusals().total() > 0,
                has_queue,
                "{phase:?}: a refusal is recorded exactly where a ring exists"
            );
            assert_eq!(d.sent_sysinfo, has_queue, "{phase:?}");
            // the outstanding event batch dies with the binding (`enter_halted`), not at fn 47
            assert_eq!(
                d.fsm.events_outstanding,
                matches!(phase, BootPhase::Running | BootPhase::Suspending),
                "{phase:?}: events outstanding"
            );
            assert_eq!(
                d.fsm.large.is_open(),
                phase == BootPhase::Running,
                "{phase:?}: a large control is half-joined only in the Running driver"
            );
            assert_eq!(
                matches!(d.fsm.queue(), QueueState::Bound(_)),
                matches!(
                    phase,
                    BootPhase::Booted | BootPhase::Running | BootPhase::Suspending
                ),
                "{phase:?}: binding"
            );
        }
        let d = drive(BootPhase::Halted, true, None);
        assert!(
            d.fsm.fw_halted_after_suspend,
            "the FSP regime halted itself"
        );
        assert!(d.fsm.observe().swgen0_pending, "and kept its status IRQ");
    }

    // ---- 1. reset from every phase -------------------------------------------------------------

    #[test]
    fn a_device_reset_from_every_phase_is_e11_and_a_fresh_device() {
        for phase in PHASES {
            for live in [false, true] {
                let mut d = drive(phase, live, None);
                if phase != BootPhase::Cold || live {
                    assert_ne!(
                        d.fsm,
                        GspFsm::new(abi()),
                        "{phase:?} live={live}: not vacuous"
                    );
                }
                assert_eq!(
                    d.fsm.device_reset(),
                    Transition::E11,
                    "{phase:?} live={live}"
                );
                assert_eq!(d.fsm, GspFsm::new(abi()), "{phase:?} live={live}");
            }
        }
    }

    #[test]
    fn a_device_reset_from_every_phase_keeps_the_system_info_cell_and_nothing_else() {
        for phase in PHASES {
            for live in [false, true] {
                let cell = SystemInfoCell::new();
                let mut d = drive(phase, live, Some(&cell));
                let stashed = cell.latest();
                assert_eq!(
                    stashed.is_some(),
                    d.sent_sysinfo,
                    "{phase:?} live={live}: fn 72 reached the cell exactly when the ring carried it"
                );
                assert_eq!(
                    d.fsm.device_reset(),
                    Transition::E11,
                    "{phase:?} live={live}"
                );
                assert_eq!(
                    d.fsm,
                    GspFsm::new(abi()).with_system_info_cell(cell.clone()),
                    "{phase:?} live={live}: fresh, with the same cell"
                );
                assert_ne!(
                    d.fsm,
                    GspFsm::new(abi()),
                    "the cell is the one thing that stays"
                );
                assert_eq!(
                    cell.latest(),
                    stashed,
                    "the cell's contents are not the reset's to touch"
                );
            }
        }
    }

    #[test]
    fn a_life_ended_by_booter_unload_resets_to_a_fresh_device() {
        for live in [false, true] {
            let cell = SystemInfoCell::new();
            for c in [None, Some(&cell)] {
                let mut d = drive_halted_by_booter_unload(live, c);
                assert_eq!(d.fsm.phase(), BootPhase::Halted);
                assert!(!d.fsm.fw_halted_after_suspend, "E4, not E13");
                if live {
                    assert!(d.fsm.held_len() > 0 && d.fsm.refusals().total() > 0);
                }
                assert_eq!(d.fsm.device_reset(), Transition::E11);
                assert_eq!(d.fsm, fresh(c), "live={live}");
            }
        }
    }

    /// The compile-time half of "total": every field of the value is named, so a field added to
    /// [`GspFsm`] stops this test compiling until someone decides what a reset does to it.
    #[test]
    fn a_reset_value_is_fresh_field_by_field() {
        let mut d = drive(BootPhase::Running, true, None);
        d.fsm.device_reset();
        let GspFsm {
            reply_body_trace,
            pending_command_doorbells,
            held,
            refusals,
            bcr_ctrl,
            falcon_engine,
            after_suspend,
            fw_halted_after_suspend,
            abi: reset_abi,
            phase,
            queue,
            large,
            swgen0_pending,
            events_outstanding,
            mailbox_lo,
            mailbox_hi,
            boot_args_seen,
            arch_state,
            init_done_posted,
            stat_seq,
            cmd_seq,
            cmd_read_ptr,
            region_identity,
            suspended,
            pm_suspend,
            max_entries,
            fwsec_image,
            frts_image,
            frts_offset,
            system_info,
        } = d.fsm;
        let GspFsm {
            reply_body_trace: bt0,
            pending_command_doorbells: p0,
            held: h0,
            refusals: r0,
            bcr_ctrl: b0,
            falcon_engine: fe0,
            after_suspend: a0,
            fw_halted_after_suspend: w0,
            abi: abi0,
            phase: ph0,
            queue: q0,
            large: l0,
            swgen0_pending: s0,
            events_outstanding: e0,
            mailbox_lo: lo0,
            mailbox_hi: hi0,
            boot_args_seen: bs0,
            arch_state: as0,
            init_done_posted: i0,
            stat_seq: ss0,
            cmd_seq: cs0,
            cmd_read_ptr: crp0,
            region_identity: ri0,
            suspended: su0,
            pm_suspend: pm0,
            max_entries: me0,
            fwsec_image: fi0,
            frts_image: fr0,
            frts_offset: fo0,
            system_info: si0,
        } = GspFsm::new(abi());
        // Default-off diagnostic state remains empty; enabled budget survival
        // is covered separately by guest_reset_does_not_rearm_the_reply_record_budget.
        assert_eq!(reply_body_trace, bt0);
        assert_eq!(pending_command_doorbells, p0);
        assert_eq!(held, h0);
        assert_eq!(refusals, r0);
        assert_eq!(bcr_ctrl, b0);
        assert_eq!(falcon_engine, fe0);
        assert_eq!(after_suspend, a0);
        assert_eq!(fw_halted_after_suspend, w0);
        assert_eq!(reset_abi, abi0);
        assert_eq!(phase, ph0);
        assert_eq!(queue, q0);
        assert_eq!(large, l0);
        assert_eq!(swgen0_pending, s0);
        assert_eq!(events_outstanding, e0);
        assert_eq!(mailbox_lo, lo0);
        assert_eq!(mailbox_hi, hi0);
        assert_eq!(boot_args_seen, bs0);
        assert_eq!(arch_state, as0);
        assert_eq!(init_done_posted, i0);
        assert_eq!(stat_seq, ss0);
        assert_eq!(cmd_seq, cs0);
        assert_eq!(cmd_read_ptr, crp0);
        assert_eq!(region_identity, ri0);
        assert_eq!(suspended, su0);
        assert_eq!(pm_suspend, pm0);
        assert_eq!(max_entries, me0);
        assert_eq!(fwsec_image, fi0);
        assert_eq!(frts_image, fr0);
        assert_eq!(frts_offset, fo0);
        assert_eq!(system_info, si0);
    }

    // ---- 2. reset after reset ------------------------------------------------------------------

    #[test]
    fn a_reset_straight_after_a_reset_is_e11_again_and_still_fresh() {
        for phase in PHASES {
            let cell = SystemInfoCell::new();
            for c in [None, Some(&cell)] {
                let mut d = drive(phase, true, c);
                assert_eq!(d.fsm.device_reset(), Transition::E11, "{phase:?}");
                assert_eq!(d.fsm, fresh(c), "{phase:?}: first reset");
                assert_eq!(d.fsm.device_reset(), Transition::E11, "{phase:?}");
                assert_eq!(d.fsm, fresh(c), "{phase:?}: second reset");
            }
        }
    }

    #[test]
    fn a_reset_of_a_device_nothing_ever_touched_changes_nothing() {
        let mut f = GspFsm::new(abi());
        assert_eq!(f.device_reset(), Transition::E11);
        assert_eq!(f, GspFsm::new(abi()));
        let cell = SystemInfoCell::new();
        let mut f = fresh(Some(&cell));
        assert_eq!(f.device_reset(), Transition::E11);
        assert_eq!(f.device_reset(), Transition::E11);
        assert_eq!(f, fresh(Some(&cell)));
    }

    // ---- 3. the second life is a first life ------------------------------------------------------

    /// Apply `step` to both devices and require everything observable, and the values themselves, to
    /// agree afterwards.
    fn lockstep(
        a: &mut Dev,
        b: &mut Dev,
        label: &str,
        mut step: impl FnMut(&mut Dev) -> String,
    ) -> String {
        let (ra, rb) = (step(a), step(b));
        assert_eq!(ra, rb, "{label}: what the step returned");
        assert_eq!(a.snapshot(), b.snapshot(), "{label}: observable state");
        assert!(a.ram == b.ram, "{label}: guest RAM (every reply posted)");
        assert_eq!(a.fsm, b.fsm, "{label}: the values");
        ra
    }

    /// A step the guest's boot depends on must have succeeded, or the comparison above is two
    /// identical refusals.
    fn ok(label: &str, out: String) {
        assert!(!out.contains("Err("), "{label}: {out}");
    }

    fn second_life_after_a_reset_matches_a_fresh_device(after: AfterSuspend, with_cell: bool) {
        let cell = SystemInfoCell::new();
        let c = with_cell.then_some(&cell);
        // (a) a device that has never been used; (b) one driven to Running with everything in
        // flight, then reset by the guest-visible PCI reset.
        let mut a = Dev::life(fresh(c), after);
        let mut b = Dev::life(fresh(c), after);
        b.start_with_frts();
        b.firmware_loaded();
        b.publish();
        b.run();
        b.queue_traffic();
        b.deliver_an_event();
        b.open_a_large_control();
        b.fsm.note_command_doorbells(3);
        assert_eq!(b.fsm.phase(), BootPhase::Running);
        assert!(b.fsm.held_len() > 0);
        assert_eq!(b.fsm.device_reset(), Transition::E11);
        // the guest's next life: new queues in new RAM, the command stream starts again at 0
        let mut b = Dev::life(b.fsm, after);

        lockstep(&mut a, &mut b, "before anything", |d| d.snapshot());
        ok(
            "FWSEC-FRTS + STARTCPU",
            lockstep(&mut a, &mut b, "FWSEC-FRTS + STARTCPU", |d| {
                d.start_with_frts()
            }),
        );
        ok(
            "firmware loaded",
            lockstep(&mut a, &mut b, "firmware loaded", |d| d.firmware_loaded()),
        );
        ok(
            "boot args published",
            lockstep(&mut a, &mut b, "boot args published", |d| d.publish()),
        );
        assert_eq!(a.fsm.phase(), BootPhase::Booted);
        ok(
            "traffic before INIT_DONE is drained",
            lockstep(&mut a, &mut b, "traffic before INIT_DONE is drained", |d| {
                d.queue_traffic()
            }),
        );
        ok(
            "the guest drains INIT_DONE",
            lockstep(&mut a, &mut b, "the guest drains INIT_DONE", |d| d.run()),
        );
        assert_eq!(a.fsm.phase(), BootPhase::Running);
        ok(
            "traffic while running",
            lockstep(&mut a, &mut b, "traffic while running", |d| {
                d.queue_traffic()
            }),
        );
        lockstep(&mut a, &mut b, "held replies released", |d| {
            format!("{:?}", d.fsm.release_held(&mut d.ram))
        });
        assert_eq!(a.fsm.held_len(), 0);
        ok(
            "an event batch",
            lockstep(&mut a, &mut b, "an event batch", |d| d.deliver_an_event()),
        );
        lockstep(&mut a, &mut b, "a second batch is gated", |d| {
            d.deliver_an_event()
        });
        lockstep(&mut a, &mut b, "the guest clears the status IRQ", |d| {
            d.write(OFF_IRQ_CLEAR, 1)
        });
        ok(
            "a large control opens",
            lockstep(&mut a, &mut b, "a large control opens", |d| {
                d.open_a_large_control()
            }),
        );
        ok("fn 47", lockstep(&mut a, &mut b, "fn 47", |d| d.unload()));
        lockstep(&mut a, &mut b, "a doorbell after the last close", |d| {
            d.ring()
        });
        lockstep(&mut a, &mut b, "the teardown start", |d| {
            d.write(OFF_START, 1)
        });
        lockstep(&mut a, &mut b, "a next open starts the processor", |d| {
            d.start_with_frts()
        });
        assert_eq!(a.fsm.phase(), BootPhase::ProtectedRegionUp);
    }

    #[test]
    fn a_second_life_after_a_reset_is_a_first_life_on_the_falcon_regime() {
        second_life_after_a_reset_matches_a_fresh_device(AfterSuspend::AwaitsTeardownUcode, false);
    }

    #[test]
    fn a_second_life_after_a_reset_is_a_first_life_on_the_fsp_regime() {
        second_life_after_a_reset_matches_a_fresh_device(AfterSuspend::FirmwareHalts, false);
    }

    #[test]
    fn a_second_life_after_a_reset_is_a_first_life_with_a_system_info_cell() {
        second_life_after_a_reset_matches_a_fresh_device(AfterSuspend::AwaitsTeardownUcode, true);
        second_life_after_a_reset_matches_a_fresh_device(AfterSuspend::FirmwareHalts, true);
    }

    // ---- 4. nothing accumulates ------------------------------------------------------------------

    #[test]
    fn fifty_boot_and_reset_cycles_leave_nothing_behind() {
        let cell = SystemInfoCell::new();
        for with_cell in [false, true] {
            let c = with_cell.then_some(&cell);
            let mut fsm = fresh(c);
            for cycle in 0..50 {
                let after = if cycle % 2 == 0 {
                    AfterSuspend::AwaitsTeardownUcode
                } else {
                    AfterSuspend::FirmwareHalts
                };
                let mut d = Dev::life(fsm, after);
                d.start_with_frts();
                d.firmware_loaded();
                d.publish();
                d.run();
                d.queue_traffic();
                d.deliver_an_event();
                d.open_a_large_control();
                d.fsm.note_command_doorbells(2);
                assert_eq!(d.fsm.phase(), BootPhase::Running, "cycle {cycle}");
                assert!(d.fsm.held_len() > 0, "cycle {cycle}: live held replies");
                assert!(d.fsm.refusals().total() > 0, "cycle {cycle}: a refusal");
                assert!(d.fsm.command_seq() > 0, "cycle {cycle}: the stream moved");
                fsm = d.fsm;
                assert_eq!(fsm.device_reset(), Transition::E11, "cycle {cycle}");
                assert_eq!(fsm.held_len(), 0, "cycle {cycle}");
                assert_eq!(fsm.pending_command_doorbells(), 0, "cycle {cycle}");
                assert_eq!(fsm.refusals().total(), 0, "cycle {cycle}");
                assert_eq!(fsm.command_seq(), 0, "cycle {cycle}");
                assert_eq!(fsm, fresh(c), "cycle {cycle}: nothing accumulated");
            }
        }
    }

    // ---- 3. S3: a PM-transition suspend, the platform's wake reset, the resume ---------------
    //
    // ★ 2026-10-14 (`docs/design/V3_S3_RESUME.md`). Measured on the bench before this: the guest's
    // `KGSP_BOOT_MODE_SR_RESUME` found the GSP Cold after the wake (`device reset #2 … Halted ->
    // Cold`), its Booter Load changed nothing, and `kgspBootstrap_TU102` returned NOT_READY (0x55).
    mod s3_resume {
        use super::*;

        /// The guest's status-queue element at `slot`: its transport `seqNum`.
        fn stat_seq_at(d: &mut Dev, slot: u64) -> u32 {
            let mut buf = vec![0u8; ELEM as usize];
            d.ram
                .read(REGION + STAT_OFF + ELEM + slot * ELEM, &mut buf)
                .expect("mapped");
            peek_seq_num(&d.fsm.abi.element, &buf).expect("an element")
        }

        fn stat_wp(d: &mut Dev) -> u32 {
            d.ram.get32(REGION + STAT_OFF + 16)
        }

        /// A running falcon-regime life that answered some traffic, then a fn 47 the device
        /// decoded as a PM transition (or not), then FWSEC-SB's STARTCPU: `Halted`.
        fn suspended(pm: bool) -> Dev {
            let mut d = Dev::life(fresh(None), AfterSuspend::AwaitsTeardownUcode);
            d.start_with_frts();
            d.firmware_loaded();
            d.publish();
            d.run();
            let code = d.fsm.abi.rpc.codes.gsp_set_system_info;
            d.send(code, vec![1; 64]);
            d.ring();
            d.guest_drains_status();
            d.unload();
            d.guest_drains_status();
            assert_eq!(d.fsm.phase(), BootPhase::Suspending);
            if pm {
                assert!(d.fsm.note_pm_suspend(), "recorded while Suspending");
            }
            d.write(OFF_START, 1); // FWSEC-SB: E2
            assert_eq!(d.fsm.phase(), BootPhase::Halted);
            d
        }

        /// ★★ The whole S3 cycle: suspend (PM), wake reset (E14), Booter Load (E15), and the RPC
        /// stream continues — sequence numbers on both queues kept, nothing re-published.
        #[test]
        fn a_pm_suspended_life_survives_the_wake_reset_and_resumes_without_a_publish() {
            let mut d = suspended(true);
            let (stat_seq, cmd_seq) = (d.fsm.stat_seq, d.fsm.command_seq());
            assert!(
                stat_seq > 0 && cmd_seq > 0,
                "the streams moved before the suspend"
            );
            let wp = stat_wp(&mut d);
            let hdr_before = {
                let mut b = vec![0u8; 32];
                d.ram.read(REGION + STAT_OFF, &mut b).expect("mapped");
                b
            };

            assert_eq!(d.fsm.wake_reset(), Some(Transition::E14));
            assert_eq!(d.fsm.phase(), BootPhase::Cold);
            assert_eq!(
                d.fsm.observe(),
                fresh(None).observe(),
                "every register reads power-on after the wake"
            );
            assert!(matches!(d.fsm.queue(), QueueState::Unbound));
            assert!(d.fsm.resumable());

            // SR_RESUME: scrubber/Booter Load only — no FWSEC, no mailbox pair.
            let out = d.write(OFF_FW_RESUME, 1);
            assert!(out.contains("E15"), "{out}");
            assert!(!out.contains("E6"), "a resume never publishes: {out}");
            assert_eq!(d.fsm.phase(), BootPhase::Booted);
            assert!(
                d.fsm.observe().riscv_active,
                "kflcnIsRiscvActive after Booter Load"
            );
            assert!(!d.fsm.resumable(), "the saved life is consumed");

            // GSP_INIT_DONE at the stream's next sequence number, in the slot after the last reply
            assert_eq!(stat_wp(&mut d), (wp + 1) % COUNT as u32);
            assert_eq!(stat_seq_at(&mut d, u64::from(wp)), stat_seq);
            let mut hdr_after = vec![0u8; 32];
            d.ram
                .read(REGION + STAT_OFF, &mut hdr_after)
                .expect("mapped");
            assert_eq!(
                hdr_after[..16],
                hdr_before[..16],
                "the status header is not re-published"
            );

            // the guest consumes INIT_DONE and its next RPC carries the next command sequence
            d.guest_drains_status();
            // `FREE` is answered (fn 72 is not), so the reply's sequence is observable
            let code = d.fsm.abi.rpc.codes.free;
            d.send(code, vec![2; 64]);
            let out = d.ring();
            assert!(out.contains("E7"), "served, not refused: {out}");
            assert!(out.contains("Running"), "{out}");
            assert_eq!(d.fsm.phase(), BootPhase::Running);
            assert_eq!(d.fsm.command_seq(), cmd_seq + 1);
            assert_eq!(
                stat_seq_at(&mut d, u64::from((wp + 1) % COUNT as u32)),
                stat_seq + 1
            );
        }

        /// The same life suspended and resumed three times in one boot.
        #[test]
        fn three_suspend_resume_cycles_keep_one_stream() {
            let mut d = suspended(true);
            for cycle in 0..3 {
                if cycle > 0 {
                    d.unload();
                    d.guest_drains_status();
                    assert!(d.fsm.note_pm_suspend(), "cycle {cycle}");
                    d.write(OFF_START, 1);
                }
                let (stat_seq, cmd_seq) = (d.fsm.stat_seq, d.fsm.command_seq());
                assert_eq!(d.fsm.wake_reset(), Some(Transition::E14), "cycle {cycle}");
                assert!(d.write(OFF_FW_RESUME, 1).contains("E15"), "cycle {cycle}");
                d.guest_drains_status();
                let code = d.fsm.abi.rpc.codes.free;
                d.send(code, vec![3; 64]);
                assert!(d.ring().contains("E7"), "cycle {cycle}");
                assert_eq!(d.fsm.phase(), BootPhase::Running, "cycle {cycle}");
                assert_eq!(d.fsm.command_seq(), cmd_seq + 1, "cycle {cycle}");
                assert_eq!(
                    d.fsm.stat_seq,
                    stat_seq + 2,
                    "cycle {cycle}: INIT_DONE + reply"
                );
                d.guest_drains_status();
            }
        }

        /// ★ A device without power loss (s2idle): `Halted`, no wake reset, the resume still works.
        #[test]
        fn a_resume_without_a_wake_reset_restores_the_binding_too() {
            let mut d = suspended(true);
            assert!(d.write(OFF_FW_RESUME, 1).contains("E15"));
            assert_eq!(d.fsm.phase(), BootPhase::Booted);
        }

        /// ⊘ A reset that does NOT follow a PM suspend still goes Cold.
        #[test]
        fn only_a_halted_pm_suspend_is_kept_by_a_wake_reset() {
            // an unload that was not a PM transition (rmmod, last close)
            let mut d = suspended(false);
            assert!(!d.fsm.resumable());
            assert_eq!(d.fsm.wake_reset(), None, "a teardown is not kept");
            // a PM suspend whose teardown STARTCPU never came
            for phase in [BootPhase::Running, BootPhase::Booted, BootPhase::Cold] {
                let mut d = drive(phase, false, None);
                assert!(!d.fsm.note_pm_suspend(), "{phase:?}: no fn 47 is current");
                assert_eq!(d.fsm.wake_reset(), None, "{phase:?}");
            }
            let mut d = drive(BootPhase::Suspending, false, None);
            assert!(d.fsm.note_pm_suspend());
            assert_eq!(
                d.fsm.wake_reset(),
                None,
                "Suspending: the firmware has not stopped — not kept"
            );
        }

        /// ⊘ After the wake, a guest that boots instead of resuming (FWSEC, a publish) gets a
        /// fresh queue instance: the saved life is gone and never comes back.
        #[test]
        fn a_cold_boot_after_the_wake_discards_the_saved_life() {
            let mut d = suspended(true);
            assert_eq!(d.fsm.wake_reset(), Some(Transition::E14));
            d.start_with_frts();
            assert!(!d.fsm.resumable(), "FWSEC runs only on a NORMAL boot");
            d.firmware_loaded();
            assert!(d.publish().contains("E6"));
            assert!(!d.write(OFF_FW_RESUME, 1).contains("E15"));
            assert_eq!(d.fsm.phase(), BootPhase::Booted);
        }

        /// ⊘ With nothing saved, the resume step reads no RAM and changes nothing (Ada's
        /// scrubber on a cold device takes this path; so does a hostile guest).
        #[test]
        fn a_resume_step_with_nothing_saved_reads_no_ram_and_changes_nothing() {
            let model = Mini {
                seq: Seq {
                    after: AfterSuspend::AwaitsTeardownUcode,
                },
            };
            let mut f = fresh(None);
            let mut ram = super::super::a_life_ends_and_the_next_one_boots::Untouchable;
            let r = f
                .mmio_write_with(&mut ram, &model, &mut Live, 0, OFF_FW_RESUME, 1)
                .expect("no refusal");
            assert!(r.transitions.is_empty());
            assert_eq!(f, fresh(None));
        }

        /// ★★ The saved binding is INERT while `Halted`: a doorbell reads nothing through it.
        #[test]
        fn a_saved_binding_reads_no_ram_until_the_resume() {
            let d = suspended(true);
            let mut f = d.fsm;
            f.note_command_doorbells(1);
            let mut ram = super::super::a_life_ends_and_the_next_one_boots::Untouchable;
            assert_eq!(
                f.service_one_deferred_command(&mut ram, &mut Live)
                    .unwrap_err(),
                GspFault::QueueNotBound,
                "E8, zero guest-RAM reads"
            );
            assert_eq!(f.wake_reset(), Some(Transition::E14));
            f.note_command_doorbells(1);
            assert!(
                f.service_one_deferred_command(&mut ram, &mut Live).is_ok(),
                "Cold after the wake: E12, zero reads"
            );
        }

        /// The status reply at `slot`: `(function, sequence, rpc_result, payload_len)`.
        fn reply_at(d: &mut Dev, slot: u64) -> (u32, u32, u32, usize) {
            let abi = d.fsm.abi;
            let mut first = vec![0u8; ELEM as usize];
            d.ram
                .read(REGION + STAT_OFF + ELEM + slot * ELEM, &mut first)
                .expect("mapped");
            let len =
                peek_len(&abi.element, &first, ELEM as u32, abi.element_size_max).expect("a reply");
            let seq = peek_seq_num(&abi.element, &first).expect("seq");
            let m = decode_message(&abi.element, &first, len, seq, &abi.driver).expect("decodes");
            (
                m.envelope.function,
                m.envelope.sequence,
                m.envelope.rpc_result,
                m.envelope.payload_len,
            )
        }

        /// ★★ The suspend buffer's registration (`ALLOC_MEMORY`, a PTE list longer than one
        /// message, `_issuePteDescRpc` with `bBidirectional = NV_FALSE`) is joined and answered
        /// ONCE, at its head — `[measured b1, 44e9b092]` it was answered twice (a refusal of the
        /// head and one of the continuation), and the guest logged `Unexpected RPC event …
        /// CONTINUATION_RECORD`.
        #[test]
        fn a_large_alloc_memory_is_joined_and_answered_once_at_its_head() {
            let mut d = Dev::life(fresh(None), AfterSuspend::AwaitsTeardownUcode);
            d.start_with_frts();
            d.firmware_loaded();
            d.publish();
            d.run();
            d.guest_drains_status();
            let abi = d.fsm.abi;
            let cell = kf_abi::memory_list::cell(abi.driver.driver_version()).expect("contract");
            let head_len =
                abi.element_size_max as usize - abi.element.hdr_size() - crate::large::RPC_HEADER;
            let pages = head_len / 8 + 10;
            let mut body = vec![0u8; 56 + pages * 8];
            for (off, v) in [
                (0usize, 1u32),
                (4, 2),
                (8, 0x77),
                (12, 0x81),
                (40, pages as u32),
            ] {
                body[off..off + 4].copy_from_slice(&v.to_le_bytes());
            }
            assert_eq!(cell.declared_len(&body[..head_len]), Some(body.len()));
            let head_seq = 100 + d.guest_seq;
            d.send(4, body[..head_len].to_vec());
            d.send(abi.rpc.codes.continuation_record, body[head_len..].to_vec());
            let wp = stat_wp(&mut d);
            let out = d.ring();
            assert!(out.contains("Ok"), "{out}");
            assert_eq!(
                stat_wp(&mut d),
                (wp + 1) % COUNT as u32,
                "exactly one reply"
            );
            assert_eq!(reply_at(&mut d, u64::from(wp)), (4, head_seq, 0, 0));
            // the stream goes on: the next command is answered at its own sequence
            d.guest_drains_status();
            let next_seq = 100 + d.guest_seq;
            d.send(abi.rpc.codes.free, vec![0; 16]);
            d.ring();
            assert_eq!(
                reply_at(&mut d, u64::from((wp + 1) % COUNT as u32)).1,
                next_seq
            );
        }

        /// The FSP regime's firmware halts itself at fn 47 (E13); the device decodes the body
        /// after that, so the PM note must still find the saved binding.
        #[test]
        fn a_firmware_halted_pm_suspend_is_resumable_too() {
            let mut d = Dev::life(fresh(None), AfterSuspend::FirmwareHalts);
            d.start_with_frts();
            d.firmware_loaded();
            d.publish();
            d.run();
            d.unload();
            assert_eq!(d.fsm.phase(), BootPhase::Halted);
            assert!(d.fsm.note_pm_suspend());
            assert_eq!(d.fsm.wake_reset(), Some(Transition::E14));
            assert!(d.write(OFF_FW_RESUME, 1).contains("E15"));
        }
    }
}
