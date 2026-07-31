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
    peek_elem_count, peek_len,
};
use crate::fault::GspFault;
use crate::ram::{GuestRam, RegionMap};
use crate::ring::{MsgqAbi, MsgqGeometry, RxCursor, TxCursor, available_elements, free_elements};
use crate::rpc::{Disposition, RpcAbi, RpcCommand, RpcFunction};
use kayfabe_abi::versions::DriverAbiTable;
use kayfabe_arch::{
    Arch, ArchBootState, BootContext, BootPhase, BootStep, GspObservation, RegWrite,
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

// ★ [`BootPhase`] moved to `kayfabe_arch::gsp` at task #121 and is re-exported below.
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
/// The production policy (`kayfabe_rmrpc::GraphPolicy`) therefore answers an unmodelled
/// control with a non-zero **envelope** `rpc_result`, which short-circuits the guest ahead
/// of both the copy-out and the cache
/// (`ogkm-580: rpc.c:1994`, `ogkm-610: :2012`); its `BridgeRefusal::GspRuleControlUnserviced`
/// carries the whole argument. Keep `EchoOk` for what it is — the C-baseline transport
/// proof, and the fixture a test uses to show that the baseline would be wrong.
#[derive(Debug, Default, Clone, Copy)]
pub struct EchoOk;

impl CommandPolicy for EchoOk {
    fn respond(&mut self, _cmd: &RpcCommand) -> Option<Reply> {
        None
    }
}

/// Several policies, tried in order; the first `Some` wins.
///
/// ★ The seam is *one* policy, and that is right — the FSM must not know how many things
/// have an opinion. But a port grows one answered command at a time, and folding each new
/// one into the previous policy would make an unrelated pair share a type and a state.
/// So composition lives here, as data: a policy that answers nothing returns `None` and
/// the next is asked, and a chain that answers nothing at all is exactly [`EchoOk`].
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

    /// Whether the chain is empty — in which case it behaves as [`EchoOk`].
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
    /// ★ Generation-local boot state, stored here and **never interpreted here**.
    ///
    /// This field replaces `sec2_mailbox0`, which was one boot regime's latch — the
    /// secure-booter argument — living as a named field on the arch-independent value.
    /// Whatever a generation's [`kayfabe_arch::BootSequence`] must remember between
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
            arch_state: ArchBootState::default(),
            init_done_posted: false,
            stat_seq: 0,
            cmd_seq: 0,
            cmd_read_ptr: 0,
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
            stage: self.phase,
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
        model: &dyn kayfabe_arch::GspModel,
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

    /// What a [`kayfabe_arch::BootSequence`] is allowed to look at.
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
        model: &dyn kayfabe_arch::GspModel,
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
        model: &dyn kayfabe_arch::GspModel,
        policy: &mut dyn CommandPolicy,
        step: BootStep,
        report: &mut ServiceReport,
    ) -> Result<(), GspFault> {
        match step {
            BootStep::StartProcessor => {
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
                let mut r = self.publish(ram, model, policy, gpa)?;
                report.transitions.push(Transition::E6);
                report.transitions.append(&mut r.transitions);
                report.commands.extend(r.commands);
                report.raise_status_irq = true;
            }
            BootStep::CommandDoorbell => {
                let (t, mut r) = self.doorbell(ram, policy)?;
                report.transitions.push(t);
                report.transitions.append(&mut r.transitions);
                report.commands.extend(r.commands);
                report.raise_status_irq |= r.raise_status_irq;
            }
            // E10 — the guest's ISR clears the edge before draining the queue
            // (`C: src/qemu/nvkvm_gpu_emul.c:4193-4200`).
            BootStep::ClearStatusIrq => {
                self.swgen0_pending = false;
                report.transitions.push(Transition::E10);
            }
        }
        Ok(())
    }

    /// E1 / E2 / E3 — the GSP falcon STARTCPU, classified on `phase` alone.
    fn gsp_startcpu(&mut self) -> Transition {
        match self.phase {
            BootPhase::Cold | BootPhase::Halted => {
                self.phase = BootPhase::ProtectedRegionUp;
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
        model: &dyn kayfabe_arch::GspModel,
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
        let same_instance = self.region_identity == Some(shared_mem_pa);
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
            let run = self.read_run(ram, &geom, read_ptr, len, &first)?;
            let msg = decode_message(&self.abi.element, &run, len, expect_seq, &self.abi.driver)?;
            let cmd = RpcCommand::from_incoming(&self.abi.rpc, &msg);

            read_ptr = count.slot(read_ptr + run_elements).index();
            expect_seq = expect_seq.wrapping_add(1);
            avail -= run_elements;
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
            // 72/73 are both issued asynchronously and neither is awaited:
            // `GSP_SET_SYSTEM_INFO` calls `_issueRpcAsync`
            // (`ogkm-610: src/nvidia/src/kernel/vgpu/rpc.c:10466`, `ogkm-580: :10656`) and
            // `SET_REGISTRY` takes `_issueRpcAsyncLarge` or `_issueRpcAsync` depending on
            // whether the packed registry table fits one message
            // (`ogkm-610: :10533`/`:10538`, `ogkm-580: :10728`/`:10733`). Identical shape at
            // both tags. Echoing them
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
        // the guest's read pointer at 0 (`ogkm-610: msgq.c:435`, `ogkm-580: :436`), so
        // "the pointer has moved off
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
