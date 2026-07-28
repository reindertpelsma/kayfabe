//! **S4 — RPC dispatch.** Envelope → an abstract function, a disposition, and an intent.
//!
//! ## The numbering is Axis A, so it arrives as a value
//!
//! `NV_VGPU_MSG_FUNCTION_*` / `NV_VGPU_MSG_EVENT_*` are driver constants
//! (`ogkm: src/nvidia/inc/kernel/vgpu/rpc_global_enums.h`, an X-macro table where every
//! id is explicit — *"Deprecated RPC's numbers cannot be reused in order to not break
//! compatibility"*, `:4`). Per the quarantine rule they belong in `kayfabe-abi`, whose
//! established shape for exactly this is `client_kind_from_process_id`: the constant in
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
//! | must reply, synchronously | `UNLOADING_GUEST_DRIVER` (47) | `_issueRpcAndWait` (`ogkm: src/nvidia/src/kernel/vgpu/rpc.c:9146-9170`) — an unanswered fn-47 blocks `rmmod` for the whole RPC timeout |
//! | reply expected | everything else the guest sends as a command | the reply is matched on `(function, sequence)` (`ogkm: kernel_gsp.c:1824-1828`) |
//! | **no** reply | `GSP_SET_SYSTEM_INFO` (72), `SET_REGISTRY` (73) | both end in `_issueRpcAsync` (`ogkm: rpc.c:10466`, and `:10507`'s own comment *"SET_REGISTRY is async RPC"*); echoing them shows up in the driver as an unexpected event and desyncs the seqNum (`C: src/qemu/nvkvm_gpu_emul.c:2410-2416`) |
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
/// Every field is one explicit enum entry. Values for reference (610.43.02, and stable by
/// the header's own no-reuse rule): 1, 10, 21, 47, 65, 71, 72, 73, 76, 103, `0x1001`,
/// `0x1003` (`ogkm: rpc_global_enums.h:11, 20, 31, 57, 75, 81, 82, 83, 86, 113, 254, 256`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FunctionCodes {
    /// `SET_GUEST_SYSTEM_INFO` — the first synchronous RPC after `GSP_INIT_DONE`.
    pub set_guest_system_info: u32,
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
    /// `CONTINUATION_RECORD` — the large-message carrier.
    pub continuation_record: u32,
    /// `GSP_SET_SYSTEM_INFO` — init RPC, no reply.
    pub gsp_set_system_info: u32,
    /// `SET_REGISTRY` — init RPC, no reply.
    pub set_registry: u32,
    /// `GSP_RM_CONTROL`.
    pub gsp_rm_control: u32,
    /// `GSP_RM_ALLOC`.
    pub gsp_rm_alloc: u32,
    /// `GSP_INIT_DONE` — the event the guest's boot poll waits for.
    pub gsp_init_done: u32,
    /// `POST_EVENT` — the completion carrier. **Not** on the bootup allowlist.
    pub post_event: u32,
}

impl FunctionCodes {
    /// Every id, for the distinctness check.
    fn all(&self) -> [u32; 12] {
        [
            self.set_guest_system_info,
            self.free,
            self.dup_object,
            self.unloading_guest_driver,
            self.get_gsp_static_info,
            self.continuation_record,
            self.gsp_set_system_info,
            self.set_registry,
            self.gsp_rm_control,
            self.gsp_rm_alloc,
            self.gsp_init_done,
            self.post_event,
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
            c if c == self.free => RpcFunction::Free,
            c if c == self.dup_object => RpcFunction::DupObject,
            c if c == self.unloading_guest_driver => RpcFunction::UnloadingGuestDriver,
            c if c == self.get_gsp_static_info => RpcFunction::GetGspStaticInfo,
            c if c == self.continuation_record => RpcFunction::ContinuationRecord,
            c if c == self.gsp_set_system_info => RpcFunction::GspSetSystemInfo,
            c if c == self.set_registry => RpcFunction::SetRegistry,
            c if c == self.gsp_rm_control => RpcFunction::RmControl,
            c if c == self.gsp_rm_alloc => RpcFunction::RmAlloc,
            c if c == self.gsp_init_done => RpcFunction::InitDone,
            c if c == self.post_event => RpcFunction::PostEvent,
            other => RpcFunction::Other(other),
        }
    }
}

/// The Axis-A facts the RPC envelope needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RpcAbi {
    /// `header_version` — `0x0300_0000` is MAJOR 3 / MINOR 0
    /// (`ogkm: src/nvidia/inc/kernel/vgpu/rpc_headers.h:56-59`).
    ///
    /// ★ The guest does **not** check it on receive: `NV_VGPU_MSG_SIGNATURE_VALID`
    /// appears exactly once in the whole tree, in the *send* path
    /// (`ogkm: src/nvidia/src/kernel/rmapi/rpc_common.c:154-184`). We emit both anyway,
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
    /// `FREE` — one object (or, when it names a client root, one namespace) goes away.
    Free,
    /// `DUP_OBJECT` — alias an object into another client's namespace.
    DupObject,
    /// `UNLOADING_GUEST_DRIVER` — synchronous; a reply is a liveness obligation.
    UnloadingGuestDriver,
    /// `GET_GSP_STATIC_INFO`.
    GetGspStaticInfo,
    /// `CONTINUATION_RECORD`.
    ContinuationRecord,
    /// `GSP_SET_SYSTEM_INFO` — init RPC, asynchronous.
    GspSetSystemInfo,
    /// `SET_REGISTRY` — init RPC, asynchronous.
    SetRegistry,
    /// `GSP_RM_CONTROL`.
    RmControl,
    /// `GSP_RM_ALLOC`.
    RmAlloc,
    /// The `GSP_INIT_DONE` event (we send it; the guest never does).
    InitDone,
    /// The `POST_EVENT` event (we send it).
    PostEvent,
    /// An id this table does not name. The guest logs and ignores unknown *events*
    /// (`ogkm: kernel_gsp.c:1587-1599`); an unknown *command* still gets a reply, because
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
            RpcFunction::GspSetSystemInfo | RpcFunction::SetRegistry => Disposition::NoReply,
            RpcFunction::UnloadingGuestDriver => Disposition::ReplyRequired,
            _ => Disposition::Reply,
        }
    }

    /// May this function be posted to the guest as an unsolicited event during its
    /// **bootup poll**?
    ///
    /// The driver's poll runs without the API lock and hard-asserts on anything outside
    /// an eight-entry allowlist (`ogkm: kernel_gsp.c:1419-1440`). `GSP_INIT_DONE` is on
    /// it; `POST_EVENT` is **not**. This is the predicate §7-G7 turns into a state
    /// requirement — events only once the FSM is `Running`.
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
