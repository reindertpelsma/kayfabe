//! **S4 — RPC dispatch.** Envelope → an abstract function, a disposition, and an intent.
//!
//! ## The numbering is Axis A, so it arrives as a value
//!
//! `NV_VGPU_MSG_FUNCTION_*` / `NV_VGPU_MSG_EVENT_*` are driver constants
//! (`ogkm-580:`/`ogkm-610: src/nvidia/inc/kernel/vgpu/rpc_global_enums.h`, an X-macro
//! table where every id is explicit — *"Deprecated RPC's numbers cannot be reused in
//! order to not break compatibility"*, `:4` at **both** tags, same path, same line). Per
//! the quarantine rule they belong in `kayfabe-abi`, whose established shape for exactly
//! this is `client_kind_from_process_id`: the constant in
//! the ABI crate, the abstract type everywhere above it. This module is the "everywhere
//! above it" half — [`FunctionCodes`] is the table it consumes, and this crate declares
//! **no** id of its own.
//!
//! ## What the *ids* are for
//!
//! [`FunctionCodes`] names an id only where something downstream consumes it — the boot
//! FSM here, or the object-model bridge in `kayfabe-rmrpc`. An id NOT in the table classifies
//! as [`RpcFunction::Other`], which the bridge refuses by name; that is the deliberate
//! third state, and it is why `FREE`/`DUP_OBJECT` are in the table while the other ~200
//! ids in `rpc_global_enums.h` are not.
//!
//! ## The three dispositions, and where each comes from
//!
//! | disposition | which functions | source |
//! |---|---|---|
//! | must reply, synchronously | `UNLOADING_GUEST_DRIVER` (47) | `rpcUnloadingGuestDriver_v1F_07` ends in `_issueRpcAndWait` (`ogkm-610: src/nvidia/src/kernel/vgpu/rpc.c:9146-9170`, `ogkm-580: :9168-9192` — same function, both tags, the two differing only in the message-buffer accessor) — an unanswered fn-47 blocks `rmmod` for the whole RPC timeout |
//! | reply expected | everything else the guest sends as a command | the reply is matched on `(function, sequence)` (`ogkm-610: kernel_gsp.c:1824-1828`, `ogkm-580: :1775-1779` — byte-identical) |
//! | **no** reply | `GSP_SET_SYSTEM_INFO` (72), `SET_REGISTRY` (73), `ECC_NOTIFIER_WRITE_ACK` (202) | every `_issueRpcAsync`/`_issueRpcAsyncLarge` call site in the driver, and nothing else — `ogkm-580: rpc.c:10656`, `:10728`/`:10733`, `:11376`; `ogkm-610: :10466`, `:10533`/`:10538`, `:11184`. Echoing one shows up in the driver as an unexpected event and desyncs the seqNum (`C: src/qemu/nvkvm_gpu_emul.c:2410-2416`) |
//!
//! ★★ **That third row was missing, and it was missing because the set was read off our
//! own code.** `tests/tests/rpc_async_set_oracle.rs` now derives it from `rpc.c` instead:
//! it finds every async call site, walks back to each one's `rpcWriteCommonHeader`,
//! resolves the name through the X-macro table, and requires the result to equal this
//! set exactly. A hand-written list cannot detect a shared misreading, and it shortens
//! with no red test.
//!
//! ## What this stage deliberately does not decode
//!
//! The *bodies*. `GSP_RM_CONTROL`/`GSP_RM_ALLOC` payload structs are 213 `#[repr(C)]`
//! layouts that live in `kayfabe-abi` and are mostly not generated yet (that crate's own
//! docs say why: "each needs a consumer first; a broad table with one wrong entry is
//! invisible until a guest trips it"). So an [`RpcCommand`] carries the payload as bytes,
//! and turning those bytes into declared object-model facts is **`kayfabe-rmrpc`**'s job.
//!
//! ★ This paragraph used to say the bridge lived in `kayfabe-fwd`, and
//! `mode2_gsp_port_plan.md` §2/§5 place it in *this* crate. Both are superseded by
//! `docs/design/gsp_core_bridge.md` §1.2, and the reason is not tidiness: this crate has
//! no `kayfabe-core` dependency and must keep none. A GSP FSM that can see the RM graph
//! starts firing on graph state, which is protocol-not-trace violated one level down —
//! exactly the shape the C fell into when a control command's *side effect* on the object
//! model became a transport-level action.

use crate::element::{IncomingRpc, OutgoingRpc};
use crate::fault::GspFault;
use kayfabe_abi::versions::DriverAbiTable;

/// The `NV_VGPU_MSG_*` ids this path needs, supplied by the ABI layer.
///
/// Every field is one explicit enum entry. Values for reference — **identical at both
/// tags**, as the header's own no-reuse rule promises: 1, 10, 21, 47, 65, 71, 72, 73, 76,
/// 103, `0x1001`, `0x1003`.
///
/// The *lines* are not identical, which is the whole point of the tagging rule. The
/// `X(RM, …)` function block sits at the same lines in both trees
/// (`ogkm-610:`/`ogkm-580: rpc_global_enums.h:11, 20, 31, 57, 75, 81, 82, 83, 86, 113`),
/// but the `E(…)` event block is one line earlier at 580: `GSP_INIT_DONE`/`POST_EVENT` are
/// `ogkm-610: :254, 256` and `ogkm-580: :253, 255`. Reading `:254, 256` against 580 yields
/// `GSP_RUN_CPU_SEQUENCER` (`0x1002`) and `RC_TRIGGERED` (`0x1004`) — two plausible,
/// wrong ids, which is exactly the failure the version tag exists to stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FunctionCodes {
    /// `SET_GUEST_SYSTEM_INFO` — the first synchronous RPC after `GSP_INIT_DONE`.
    pub set_guest_system_info: u32,
    /// `SET_GUEST_SYSTEM_INFO_EXT` (0x40) — its **tail call**, and not optional.
    ///
    /// ★ `RmRpcSetGuestSystemInfo` ends by issuing this and **returning its status**
    /// (`ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:8825-8832`,
    /// `ogkm-610: rpc.c:8630-8637`), so a port that answers fn 1 and refuses fn 64 fails
    /// `RmInitAdapter` one line further on with a different message. Named here because
    /// something downstream consumes it — `kayfabe_device::guestsysinfo` — which is this
    /// table's whole membership rule.
    pub set_guest_system_info_ext: u32,
    /// `FREE` (0xa) — RM's object teardown stream.
    ///
    /// ★ It is **the** teardown signal, and `UNLOADING_GUEST_DRIVER` is not:
    /// `[measured]` `docs/reference/mode2_bench_lifecycle.md` §2 — *"`rmmod` emits NO
    /// fn-47"*, the idle release at process exit having already consumed it.
    pub free: u32,
    /// `DUP_OBJECT` (0x15) — the only cross-client transfer edge in the object model,
    /// and therefore the protocol-correct source of process grouping.
    pub dup_object: u32,
    /// `UNLOADING_GUEST_DRIVER` — the synchronous teardown RPC.
    pub unloading_guest_driver: u32,
    /// `GET_GSP_STATIC_INFO` — the second synchronous RPC after `GSP_INIT_DONE`.
    pub get_gsp_static_info: u32,
    /// `INIT_GSP_TRACE_CRASH_BUFFER` (0xE4) — the guest handing its GSP a sysmem buffer
    /// to write crash traces into.
    ///
    /// ★ Named because `kgspInitGspTraceCrashBuffer` sends it from inside
    /// `kgspInitRm_IMPL` and **asserts on the status**
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:3396-3402`, hoisted at
    /// `:4239`), so it is on the critical boot path rather than a diagnostic aside. Its
    /// consumer is `kayfabe_device::inert`.
    pub init_gsp_trace_crash_buffer: u32,
    /// `CONTINUATION_RECORD` — the large-message carrier.
    pub continuation_record: u32,
    /// `GSP_SET_SYSTEM_INFO` — init RPC, no reply.
    pub gsp_set_system_info: u32,
    /// `SET_REGISTRY` — init RPC, no reply.
    pub set_registry: u32,
    /// `ECC_NOTIFIER_WRITE_ACK` (202) — the **third** `_issueRpcAsync` sender, and the one
    /// this port had missed.
    ///
    /// `rpcEccNotifierWriteAck_v23_05` writes the common header and ends in
    /// `_issueRpcAsync` (`ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:11359-11379`, the call
    /// at `:11376`), so nothing awaits a reply. The id is `X(RM, ECC_NOTIFIER_WRITE_ACK,
    /// 202)` (`ogkm-580: src/nvidia/inc/kernel/vgpu/rpc_global_enums.h:212`).
    ///
    /// ★ It is in this table for the same reason every other entry is: something
    /// downstream consumes it — [`RpcFunction::disposition`]. Answering it would post a
    /// reply the guest never awaits, which surfaces as an unsolicited message and hits
    /// `NV_ASSERT(0)` in the bootup poll (`ogkm-580: kernel_gsp.c:1464-1482`).
    pub ecc_notifier_write_ack: u32,
    /// ★★★ `UPDATE_BAR_PDE` — the guest handing its own **bus-aperture root page-directory
    /// entry** to the firmware.
    ///
    /// A GSP-offload guest builds its own BAR2 page tables, reads its own `PDE3[0]` back
    /// out through the BAR0 window and sends that eight-byte value here, because on this
    /// model only the firmware's directory is bound to hardware
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/bus/kern_bus.c:829-881`). In Mode 2 the
    /// firmware is us, so this is the **only** fact this port ever receives about that
    /// tree — and the aperture is unrooted until it arrives.
    ///
    /// ⚠ The sender ignores the status (`kbusPatchBar2Pdb_GSPCLIENT` returns `NV_OK`
    /// whatever comes back), so refusing it is **silent** — the symptom is a translated
    /// write landing nowhere, hundreds of operations later, as `kbusVerifyBar2`'s
    /// `NV_ERR_MEMORY_ERROR`.
    pub update_bar_pde: u32,
    /// `GSP_RM_CONTROL`.
    pub gsp_rm_control: u32,
    /// `GSP_RM_ALLOC`.
    pub gsp_rm_alloc: u32,
    /// `GSP_INIT_DONE` — the event the guest's boot poll waits for.
    pub gsp_init_done: u32,
    /// `POST_EVENT` — the completion carrier. **Not** on the bootup allowlist.
    pub post_event: u32,
    /// `RC_TRIGGERED` — the **robust-channel** event: *this channel has been torn down,
    /// and here is why* (task #111, `docs/design/simulated_gpu_fault.md`).
    ///
    /// ★ It is a GSP→CPU event and the direction is the reason this port needs it at
    /// all. On a GSP-firmware driver the channel teardown runs on the GSP and only the
    /// client notification runs on the CPU — the receiver's own comment says so:
    /// *"RC error handling (\"Channel Teardown sequence\") is executed in GSP-RM.
    /// Client notifications, OS interaction etc happen in CPU-RM (Kernel RM)."*
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:541-545`). In Mode 2 we
    /// **are** the GSP, so there is no other component that could send it.
    ///
    /// ★★ **Not** on either tag's bootup allowlist, exactly like `POST_EVENT` — see
    /// [`RpcFunction::allowed_in_bootup_window`].
    pub rc_triggered: u32,
}

impl FunctionCodes {
    /// Every id, for the distinctness check.
    fn all(&self) -> [u32; 17] {
        [
            self.set_guest_system_info,
            self.set_guest_system_info_ext,
            self.free,
            self.dup_object,
            self.unloading_guest_driver,
            self.get_gsp_static_info,
            self.init_gsp_trace_crash_buffer,
            self.continuation_record,
            self.gsp_set_system_info,
            self.set_registry,
            self.ecc_notifier_write_ack,
            self.update_bar_pde,
            self.gsp_rm_control,
            self.gsp_rm_alloc,
            self.gsp_init_done,
            self.post_event,
            self.rc_triggered,
        ]
    }

    /// Reject a table with a duplicated id.
    ///
    /// A duplicate would make [`FunctionCodes::classify`] silently answer with whichever
    /// arm the `match` reached first — a mis-transcribed table that looks like a working
    /// one, which is the failure mode the whole Axis-A version key exists to prevent.
    ///
    /// # Errors
    ///
    /// [`GspFault::DuplicateFunctionCode`].
    pub fn validated(self) -> Result<FunctionCodes, GspFault> {
        let ids = self.all();
        for (i, &a) in ids.iter().enumerate() {
            for &b in &ids[i + 1..] {
                if a == b {
                    return Err(GspFault::DuplicateFunctionCode { code: a });
                }
            }
        }
        Ok(self)
    }

    /// Which abstract function a wire id names.
    #[must_use]
    pub fn classify(&self, code: u32) -> RpcFunction {
        match code {
            c if c == self.set_guest_system_info => RpcFunction::SetGuestSystemInfo,
            c if c == self.set_guest_system_info_ext => RpcFunction::SetGuestSystemInfoExt,
            c if c == self.free => RpcFunction::Free,
            c if c == self.dup_object => RpcFunction::DupObject,
            c if c == self.unloading_guest_driver => RpcFunction::UnloadingGuestDriver,
            c if c == self.get_gsp_static_info => RpcFunction::GetGspStaticInfo,
            c if c == self.init_gsp_trace_crash_buffer => RpcFunction::InitGspTraceCrashBuffer,
            c if c == self.continuation_record => RpcFunction::ContinuationRecord,
            c if c == self.gsp_set_system_info => RpcFunction::GspSetSystemInfo,
            c if c == self.set_registry => RpcFunction::SetRegistry,
            c if c == self.ecc_notifier_write_ack => RpcFunction::EccNotifierWriteAck,
            c if c == self.update_bar_pde => RpcFunction::UpdateBarPde,
            c if c == self.gsp_rm_control => RpcFunction::RmControl,
            c if c == self.gsp_rm_alloc => RpcFunction::RmAlloc,
            c if c == self.gsp_init_done => RpcFunction::InitDone,
            c if c == self.post_event => RpcFunction::PostEvent,
            c if c == self.rc_triggered => RpcFunction::RcTriggered,
            other => RpcFunction::Other(other),
        }
    }
}

/// The Axis-A facts the RPC envelope needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RpcAbi {
    /// `header_version` — `0x0300_0000` is MAJOR 3 / MINOR 0
    /// (`ogkm-610:`/`ogkm-580: src/nvidia/inc/kernel/vgpu/rpc_headers.h:56-59`, same path
    /// and same lines at both tags).
    ///
    /// ★ The guest does **not** check it on receive: `NV_VGPU_MSG_SIGNATURE_VALID`
    /// appears exactly once in the whole tree, in the *send* path — and that is true of
    /// **both** trees, so it is a protocol fact and not a 610 accident
    /// (`ogkm-610: src/nvidia/src/kernel/rmapi/rpc_common.c:154-184`,
    /// `ogkm-580: :153-183`, both `rpcWriteCommonHeader`). We emit both anyway,
    /// as the C does (`C:1584-1585`), but no test here may assert that a guest rejects a
    /// wrong one — that would assert a behaviour the driver does not have.
    pub header_version: u32,
    /// The function-id table.
    pub codes: FunctionCodes,
}

/// One RPC function, abstractly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RpcFunction {
    /// `SET_GUEST_SYSTEM_INFO`.
    SetGuestSystemInfo,
    /// `SET_GUEST_SYSTEM_INFO_EXT` — the tail call whose status the guest returns.
    SetGuestSystemInfoExt,
    /// `FREE` — one object (or, when it names a client root, one namespace) goes away.
    Free,
    /// `DUP_OBJECT` — alias an object into another client's namespace.
    DupObject,
    /// `UNLOADING_GUEST_DRIVER` — synchronous; a reply is a liveness obligation.
    UnloadingGuestDriver,
    /// `GET_GSP_STATIC_INFO`.
    GetGspStaticInfo,
    /// `INIT_GSP_TRACE_CRASH_BUFFER` — a buffer declaration this port accepts and does
    /// nothing with; see `kayfabe_device::inert`.
    InitGspTraceCrashBuffer,
    /// `CONTINUATION_RECORD`.
    ContinuationRecord,
    /// `GSP_SET_SYSTEM_INFO` — init RPC, asynchronous.
    GspSetSystemInfo,
    /// `SET_REGISTRY` — init RPC, asynchronous.
    SetRegistry,
    /// `ECC_NOTIFIER_WRITE_ACK` — an acknowledgement the guest sends and never awaits.
    EccNotifierWriteAck,
    /// ★★★ `UPDATE_BAR_PDE` — the guest publishing a bus aperture's root page-directory
    /// entry. See [`FunctionCodes::update_bar_pde`].
    UpdateBarPde,
    /// `GSP_RM_CONTROL`.
    RmControl,
    /// `GSP_RM_ALLOC`.
    RmAlloc,
    /// The `GSP_INIT_DONE` event (we send it; the guest never does).
    InitDone,
    /// The `POST_EVENT` event (we send it).
    PostEvent,
    /// The `RC_TRIGGERED` event (we send it; the guest never does). The simulated GPU
    /// fault's carrier — `docs/design/simulated_gpu_fault.md`.
    RcTriggered,
    /// An id this table does not name. The guest logs and ignores unknown *events*
    /// (`ogkm-610: kernel_gsp.c:1587-1599`, `ogkm-580: :1610-1622` — byte-identical
    /// `default:` arm at both tags); an unknown *command* still gets a reply, because
    /// the guest may be polling `(function, sequence)` for it.
    Other(u32),
}

/// What servicing a command owes the guest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// A reply must be posted, and the guest is **blocked** until it is
    /// (`_issueRpcAndWait`). Failing to post one hangs `rmmod`.
    ReplyRequired,
    /// A reply is expected in the ordinary course.
    Reply,
    /// No reply: an echo would surface as an unexpected event and desync the stream.
    NoReply,
}

impl RpcFunction {
    /// What this function owes the guest.
    #[must_use]
    pub fn disposition(self) -> Disposition {
        match self {
            RpcFunction::GspSetSystemInfo
            | RpcFunction::SetRegistry
            | RpcFunction::EccNotifierWriteAck => Disposition::NoReply,
            RpcFunction::UnloadingGuestDriver => Disposition::ReplyRequired,
            _ => Disposition::Reply,
        }
    }

    /// May this function be posted to the guest as an unsolicited event during its
    /// **bootup poll**?
    ///
    /// The driver's poll runs without the API lock and hard-asserts (`NV_ASSERT(0)`) on
    /// any event outside a short allowlist. **The allowlist is a version seam** — it used
    /// to be written here as a version-independent "eight-entry allowlist", which is the
    /// 610 shape asserted as if it were the protocol:
    ///
    /// - `ogkm-580: kernel_gsp.c:1464-1482`, cases at `:1469-1474` — **six** entries:
    ///   `GSP_RUN_CPU_SEQUENCER` (first), `UCODE_LIBOS_PRINT`, `GSP_LOCKDOWN_NOTICE`,
    ///   `GSP_POST_NOCAT_RECORD`, `GSP_INIT_DONE`, `OS_ERROR_LOG`.
    /// - `ogkm-610: kernel_gsp.c:1419-1439`, cases at `:1424-1431` — **eight** entries,
    ///   and a different set: `GSP_RUN_CPU_SEQUENCER` is **gone** (610 dropped the CPU
    ///   sequencer handler entirely), and `PFM_REQ_HNDLR_STATE_SYNC_CALLBACK`,
    ///   `GSP_LOAD_EXEC_GENERIC_BOOTLOADER`, `GSP_LOAD_EXEC_HS_BINARY` are added.
    ///
    /// What survives both tags is a **five-entry intersection** — `UCODE_LIBOS_PRINT`,
    /// `GSP_LOCKDOWN_NOTICE`, `GSP_POST_NOCAT_RECORD`, `GSP_INIT_DONE`, `OS_ERROR_LOG` —
    /// and the two facts this predicate actually rests on hold at **both**:
    /// `GSP_INIT_DONE` is on the list, and `POST_EVENT` is **not**. So this returns
    /// `true` for `InitDone` alone, which is the intersection-safe answer and needs no
    /// version key. Should a caller ever need one of the tag-specific entries, it becomes
    /// a row in an Axis-A profile table — never an `if version ==` in this crate.
    ///
    /// This is the predicate §7-G7 turns into a state requirement — events only once the
    /// FSM is `Running`.
    #[must_use]
    pub fn allowed_in_bootup_window(self) -> bool {
        matches!(self, RpcFunction::InitDone)
    }
}

/// A decoded command from the guest's command queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcCommand {
    /// The abstract function.
    pub function: RpcFunction,
    /// The raw wire id, kept because a reply must echo it.
    pub code: u32,
    /// `rpc.sequence` — the transaction id a reply is matched on.
    pub sequence: u32,
    /// The body as the envelope's own `length` **declares** it, after the 32-byte
    /// envelope.
    ///
    /// ⚠ This is the DECLARED extent and not necessarily everything that arrived — see
    /// [`RpcCommand::delivered`]. It is what a reply is sized against
    /// ([`RpcCommand::reply`]) and what every function whose `length` covers its whole
    /// body should be decoded from.
    pub payload: Vec<u8>,
    /// How many ring elements the command occupied.
    pub elements: u32,
    /// ★★★ **Everything the guest actually put in the element run**, after the envelope —
    /// a superset of [`RpcCommand::payload`], or empty when the transport recorded none.
    ///
    /// # Why this exists: `GSP_RM_ALLOC`'s `length` does not cover its `params[]`
    ///
    /// `[measured]` — the 2026-08-01 boot at rev `2ced035` refused all four of
    /// `RmInitAdapter`'s allocations with `ParamsSizeExceedsPayload`, and reading the
    /// guest's send path says why. `rpcRmApiAlloc_GSP` calls
    /// `rpcWriteCommonHeader(pGpu, pRpc, NV_VGPU_MSG_FUNCTION_GSP_RM_ALLOC,
    /// sizeof(rpc_gsp_rm_alloc_v03_00))` (`ogkm-580: rpc.c:11196-11199`) and
    /// `rpcWriteCommonHeader` sets `length = sizeof(rpc_message_header_v) + paramLength`
    /// (`ogkm-580: rpc_common.c:183`). `rpc_gsp_rm_alloc_v03_00`'s last member is
    /// `NvU8 params[]`, a **flexible array** (`ogkm-580: g_rpc-structures.h:1491-1502`),
    /// so `sizeof` is 32 and the declared `length` stops exactly where `params[]` begins.
    /// The params are then `portMemCopy`'d in afterwards and `length` is never updated.
    ///
    /// They still arrive: `GspMsgQueueSendCommand` copies whole
    /// `GSP_MSG_QUEUE_ELEMENT_SIZE_MIN` (`RM_PAGE_SIZE`) elements into the queue
    /// (`ogkm-580: message_queue_cpu.c:563-565`), so an alloc's params sit in the element
    /// run past the declared length. ⊘ The C artifact — the only implementation a real
    /// driver has ever accepted end to end — reads them exactly that way, at a fixed
    /// element offset with no reference to `length` at all
    /// (`C: src/qemu/nvkvm_gpu_emul.c:6775-6781`, `params = cmd + 112`).
    ///
    /// ⚠ **And they are NOT covered by the queue checksum.** On the non-confidential-
    /// compute path the guest checksums `msgLen` bytes only
    /// (`ogkm-580: message_queue_cpu.c:519`), which is the declared length — so these
    /// bytes have no integrity check on either side. They are still **bounded**: the run
    /// the guest submitted is `elemCount * RM_PAGE_SIZE` and nothing here reads past it.
    /// Treat them as hostile bytes inside a bounded window, which is what
    /// `AllocParams::NoDeclaredFacts` already assumes of every params blob.
    ///
    /// ⊘ **Reply sizing must NEVER use `delivered.len()`** — and note the unit, because
    /// the rule used to be stated in a weaker one.
    ///
    /// # ★ The rule is "never a GUEST-CHOSEN length", not "always `payload.len()`"
    ///
    /// §7-G6 is written as *"clamped to **the request**"*
    /// (`docs/design/mode2_gsp_port_plan.md:1271-1274`), and this doc used to render that
    /// as *"clamped to `payload`"*. Those two coincide **only for `GSP_RM_CONTROL`**,
    /// where `rpcWriteCommonHeader` is given `sizeof(*rpc_params) + paramsSize`
    /// (`ogkm-580: rpc.c:10979-10986`) so the declared length **includes** `params[]`.
    /// For `GSP_RM_ALLOC` it is given plain `sizeof(rpc_gsp_rm_alloc_v03_00)`
    /// (`ogkm-580: rpc.c:11196-11199`) — the declared length **excludes** them. Right
    /// rule, wrong unit, for exactly the one function whose reply has to carry params.
    ///
    /// What the `⊘` genuinely guards is `delivered.len()` = `elemCount * RM_PAGE_SIZE`,
    /// a number the **guest** picked: sizing a reply by it lets the guest choose how many
    /// host bytes come back. [`RpcCommand::reply_alloc`] clamps to the guest's own
    /// *declared* `paramsSize` intersected with what arrived, which never reads
    /// `delivered.len()` as a length — it only bounds a copy by it. No host bytes, no
    /// guest-chosen extent.
    pub delivered: Vec<u8>,
}

impl RpcCommand {
    /// Decode one command from a validated element run.
    #[must_use]
    pub fn from_incoming(abi: &RpcAbi, msg: &IncomingRpc) -> RpcCommand {
        RpcCommand {
            function: abi.codes.classify(msg.envelope.function),
            code: msg.envelope.function,
            sequence: msg.envelope.sequence,
            payload: msg.payload.clone(),
            elements: msg.elements,
            delivered: msg.delivered.clone(),
        }
    }

    /// The bytes a **params decoder** may look at: [`Self::delivered`] when the transport
    /// recorded a longer run, else [`Self::payload`].
    ///
    /// ★ Never the other way round, and never a `max` of two lengths over one buffer:
    /// `delivered` is a superset of `payload` by construction (same start, further end),
    /// so choosing the longer of the two is choosing the same bytes plus more of them.
    ///
    /// ⊘ A caller that wants the *declared* extent — anything sizing a reply — wants
    /// [`Self::payload`] and must not call this.
    #[must_use]
    pub fn wire_body(&self) -> &[u8] {
        if self.delivered.len() > self.payload.len() {
            &self.delivered
        } else {
            &self.payload
        }
    }

    /// The reply this command earns: same `(function, sequence)`, the given result, and a
    /// body **clamped to the request's own size**.
    ///
    /// ★ §7-G6. The C found the unclamped case the expensive way: an over-size control
    /// reply overran the CUDA user library's stack buffer and zeroed a saved frame
    /// pointer (`C: src/qemu/nvkvm_gpu_emul.c:3237-3252`, the M9 clamp). Expressed as a
    /// constructor so the unclamped reply has no way to be built.
    #[must_use]
    pub fn reply(&self, rpc_result: u32, body: &[u8]) -> OutgoingRpc {
        let mut payload = vec![0u8; self.payload.len()];
        let take = body.len().min(payload.len());
        payload[..take].copy_from_slice(&body[..take]);
        OutgoingRpc {
            function: self.code,
            sequence: self.sequence,
            rpc_result,
            rpc_result_private: rpc_result,
            payload,
        }
    }

    /// The reply a `GSP_RM_ALLOC` earns: `body` (this port's 32-byte fixed header)
    /// followed by the request's **own** `params[]` bytes, copied back verbatim.
    ///
    /// # ★★★ Why an alloc reply cannot be sized like every other reply
    ///
    /// [`Self::reply`] sizes the reply payload to `self.payload.len()`, which for
    /// `GSP_RM_ALLOC` is the fixed header **only** — `rpc_gsp_rm_alloc_v03_00`'s last
    /// member is a flexible `params[]` and `rpcWriteCommonHeader` is given plain
    /// `sizeof(...)` (`ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:11196-11199`,
    /// `rpc_common.c:183`). `crate::element::encode_message` then builds
    /// `vec![0u8; total]` and stamps the payload into its front, so everything past the
    /// declared length reaches the guest as **zeros**.
    ///
    /// The guest reads that window back **unconditionally and at a fixed offset**:
    ///
    /// ```text
    /// if (!(flags & RMAPI_ALLOC_FLAGS_SERIALIZED) && (paramsSize > 0))
    ///     portMemCopy(pAllocParams, paramsSize, rpc_params->params, paramsSize);
    /// ```
    ///
    /// (`ogkm-580: rpc.c:11238-11241`, identical at `ogkm-610: :11043-11047`) where
    /// `paramsSize` is a **local**, seeded from `rmapiGetClassAllocParamSize(hClass)` at
    /// `:11178` — the **class** size. ⊘ Not the reply's `paramsSize` field, ⊘ not
    /// `rpc.length`, ⊘ not the element count. Receive copies whole elements
    /// (`message_queue_cpu.c:648-650`) and `rpc.length` is only a checksum extent
    /// (`:680-682, :759-770`), so nothing on the guest side bounds that read by anything
    /// we send. The `NV_ESC_RM_ALLOC` copy-out is class-sized too
    /// (`alloc_free.c:135-142`, `param_copy.c:306`).
    ///
    /// ⇒ a zero-filled window is not "no answer", it is **`memset(caller's params, 0)`**,
    /// performed by the guest kernel on a buffer that for every userspace alloc lives on
    /// **libcuda's stack**. Its symptom is a SIGSEGV inside libcuda with no status
    /// anywhere, which is why no census, ledger or id-diff can see it.
    ///
    /// # ⚠ This is a RESTORE, not a FILL — and the difference is an open item
    ///
    /// Echoing puts the guest's own `[IN]` bytes back where it left them. It does **not**
    /// write the `[OUT]` fields a real GSP writes: `NV_GR_ALLOCATION_PARAMETERS.caps`
    /// (+12) is `[OUT]`, and `0xc56f` / `0x83de` / `0x0079` carry out-fields too. So this
    /// stops the clobber and leaves *stale `[IN]` values where hardware writes results*.
    /// ⊘ Do not record it as "alloc replies are correct now".
    ///
    /// # The clamps, and which of them is the security-relevant one
    ///
    /// `n = min(paramsSize, arrived − params_at, payload_max − params_at)`.
    /// `paramsSize` is the guest's assertion about its own message and `arrived` bounds it
    /// by what the guest actually submitted; the bytes copied are the **guest's own**,
    /// back into the **guest's own** window. ⊘ The reply extent is never
    /// `delivered.len()` — see [`Self::delivered`] for why that, and only that, is the
    /// number §7-G6 forbids.
    ///
    /// # ⊘ Mis-dispatch cannot widen a reply
    ///
    /// Every path that is not *"a `GSP_RM_ALLOC` whose `body` is exactly the fixed
    /// header"* delegates to [`Self::reply`], so calling this for a control, for a
    /// malformed alloc, or for a refusal (`body` empty) is the general clamp and nothing
    /// else. A caller-side inversion here is inert rather than dangerous, which is the
    /// property `a defect in an ARGUMENT is invisible to every test of the callee` says
    /// to build in.
    #[must_use]
    pub fn reply_alloc(
        &self,
        rpc_result: u32,
        body: &[u8],
        abi: &DriverAbiTable,
        payload_max: usize,
    ) -> OutgoingRpc {
        if self.function != RpcFunction::RmAlloc {
            return self.reply(rpc_result, body);
        }
        let wire = self.wire_body();
        let Ok(h) = abi.decode_rpc_alloc(wire) else {
            return self.reply(rpc_result, body);
        };
        // ★ Exactly the fixed header, never "at least": a longer `body` would put the
        // echoed params at the wrong offset, and this port produces no such body. An
        // empty one is the named-refusal shape and takes the general clamp.
        if body.len() != h.params_at {
            return self.reply(rpc_result, body);
        }
        let arrived = wire.len().saturating_sub(h.params_at);
        let room = payload_max.saturating_sub(h.params_at);
        let n = (h.params_size as usize).min(arrived).min(room);
        let mut payload = vec![0u8; h.params_at + n];
        payload[..h.params_at].copy_from_slice(body);
        payload[h.params_at..].copy_from_slice(&wire[h.params_at..h.params_at + n]);
        OutgoingRpc {
            function: self.code,
            sequence: self.sequence,
            rpc_result,
            rpc_result_private: rpc_result,
            payload,
        }
    }

    /// The bare acknowledgement: `(function, sequence)` echoed with a result and the
    /// request's body preserved, which is what the C posts for everything it does not
    /// model (`C: src/qemu/nvkvm_gpu_emul.c:1561-1592`, the `src` template path).
    #[must_use]
    pub fn ack(&self, rpc_result: u32) -> OutgoingRpc {
        OutgoingRpc {
            function: self.code,
            sequence: self.sequence,
            rpc_result,
            rpc_result_private: rpc_result,
            payload: self.payload.clone(),
        }
    }
}

kayfabe_util::assert_send_sync!(FunctionCodes, RpcAbi, RpcFunction, Disposition, RpcCommand);
