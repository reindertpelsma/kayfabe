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
    fn all(&self) -> [u32; 16] {
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
    /// The declared body length, after the 32-byte envelope.
    pub payload: Vec<u8>,
    /// How many ring elements the command occupied.
    pub elements: u32,
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
