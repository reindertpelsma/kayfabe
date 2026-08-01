//! ★★★ **The third answer: accepted, and deliberately without effect.**
//!
//! Task #127 gave the emulated GSP two answers — *serve it from a row*, or *refuse it by
//! name* — and a command that is neither is refused, which is the safe default
//! (`kayfabe_gsp::GspFsm::answer`). This module is the third category, and it exists so
//! that "we acknowledge this and do nothing" is a **decision with a reason attached**
//! rather than something a port drifts into.
//!
//! ⊘ The distinction from an echo is the whole point, and it is not a matter of degree:
//!
//! | | echo (rejected) | inert (here) |
//! |---|---|---|
//! | which commands | **anything** that reached the FSM | a named, enumerated list |
//! | reply body | the request, reflected | zeroed — nothing of the guest's comes back |
//! | why | nobody decided | written down, per entry, with the source |
//! | a new command | silently joins | is refused until someone adds a row |
//!
//! ## ⚠ What makes a command eligible, and what would disqualify one
//!
//! An entry belongs here only if **the guest reads nothing but the status**, and if the
//! observable consequence of our doing nothing is a *true* statement about this device. A
//! command whose reply carries data the guest will act on is not inert — it is unserved,
//! and unserved is a refusal.
//!
//! ## The list
//!
//! ### `INIT_GSP_TRACE_CRASH_BUFFER` (fn 228 / `0xE4`)
//!
//! The guest allocates a page-aligned sysmem buffer, zeroes it, and hands the GSP its
//! physical address and size (`ogkm-580:
//! src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:3378-3402`). `rpcInitGspTraceCrashBuffer_v03_00`
//! sends `{pa, size}` and reads back **only** `_issueRpcAndWait`'s status
//! (`ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:11658-11679`) — there is no `[OUT]` field
//! to get wrong.
//!
//! ★ It is on the **critical boot path**, not a diagnostic aside: `kgspInitRm_IMPL` runs
//! it under `NV_ASSERT_OK_OR_GOTO` (`kernel_gsp.c:4239`), so a refusal ends `RmInitAdapter`.
//! `[measured]` run `t127b`, a stock 580.159.04 guest at `0db7c61` — the boot immediately
//! after the version handshake landed, and again the whole unserviced list was **one**:
//!
//! ```text
//! nvkvm: commands: 5 decoded, 1 UNSERVICED (…), 1 distinct
//! nvkvm:   unserviced fn 228
//! NVRM: nvAssertOkFailedNoLog: … [NV_ERR_NOT_SUPPORTED] (0x00000056) returned from status @ kernel_gsp.c:3402
//! NVRM: nvAssertOkFailedNoLog: … returned from kgspInitGspTraceCrashBuffer(pGpu, pKernelGsp) @ kernel_gsp.c:4239
//! ```
//!
//! ★★ Why inert is the **honest** answer here and not a convenient one: the buffer's whole
//! purpose is for the GSP to write `NV_RATS_GSP_TRACE_RECORD`s into. This port's GSP is not
//! running firmware and produces no trace records, so a guest that later reads the buffer
//! finds it exactly as it left it — zeroed, no records. That is a *true* report of what
//! this device did. ⊘ Accepting it is not a promise to fill it; it is an acknowledgement
//! that we received the declaration, which is all the guest asked for.
//!
//! ### `UNLOADING_GUEST_DRIVER` (fn 47)
//!
//! ★★★ The one whose refusal **escapes the teardown**. `kgspUnloadRm_IMPL` stashes the
//! RPC's status (`ogkm-580: src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:4301`), runs the
//! *whole* unload — suspend poll, log dump, `kgspTeardown_HAL` — and then, at the very
//! end, `if (rpcStatus != NV_OK) return rpcStatus;` (`:4341-4343`) in preference to the
//! teardown's own status. So a refusal here is not absorbed anywhere: it comes back out of
//! `rmmod` as the whole unload's result, after everything has already succeeded.
//!
//! ★★ And it is eligible by this module's own rule, not by convenience.
//! `rpcUnloadingGuestDriver_v1F_07` sends `{bInPMTransition, bGc6Entering, newLevel}` and
//! reads back **only** `_issueRpcAndWait`'s status (`ogkm-580: rpc.c:9168-9192`) — there is
//! no `[OUT]` field to get wrong, exactly as for `INIT_GSP_TRACE_CRASH_BUFFER`.
//!
//! ⊘ *"Inert"* here is a statement about the **reply body**, not about the transport.
//! `GspFsm::answer` fires E9 on fn 47 regardless of what any policy returned — reply
//! first, then `Suspending`, so `MAILBOX0` reports the suspend sentinel the guest's close
//! poll spins on. That is the FSM's business and it is unchanged; what changes is that the
//! guest is no longer told its own teardown failed.
//!
//! ★ And our doing nothing is a *true* report: this port's GSP is not running firmware, so
//! there is no GSP-side driver state to unload. The guest asked us to acknowledge that it
//! is going away, and it is.
//!
//! ## ★★ An inert answer is still an ACCEPTED answer — can the guest keep it forever?
//!
//! No, and not because the body is empty. The guest's control cache is populated from an
//! RPC reply at exactly two call sites in the whole driver —
//! `ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:11097` and `:11102` (`ogkm-610: :10902`,
//! `:10907`) — and both sit inside `rpcRmApiControl_GSP`, reachable only for
//! `NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL`. Fn 228 and fn 47 are not that function, and this
//! policy returns `None` for everything else, so neither site can see a reply of ours.
//! [`crate::sticky`] §1a derives that call-site universe by grep; `tests/tests/sticky_answer.rs`
//! executes the claim against this type rather than leaving it as a reading.
//!
//! ⊘ The address the guest sent is **not recorded**. Nothing here may write to it, so
//! keeping it would be storing a guest-supplied pointer with no consumer — and the day
//! something does want to write GSP traces, it will need the address *and* a guest-RAM
//! port, which is a different design than a field on a stateless policy.

use kayfabe_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};

/// `NV_OK`.
const NV_OK: u32 = 0;

/// Acknowledges the enumerated inert commands, and nothing else.
#[derive(Debug, Clone, Copy, Default)]
pub struct InertPolicy;

impl InertPolicy {
    /// Build it. Stateless by construction — an inert command has nothing to remember.
    #[must_use]
    pub fn new() -> InertPolicy {
        InertPolicy
    }

    /// Whether this port treats `function` as accepted-and-without-effect.
    ///
    /// Exposed so the membership question can be asked without building a wire message,
    /// and so a test can pin the list rather than trusting a `match`.
    #[must_use]
    pub fn is_inert(function: RpcFunction) -> bool {
        matches!(
            function,
            RpcFunction::InitGspTraceCrashBuffer | RpcFunction::UnloadingGuestDriver
        )
    }
}

impl CommandPolicy for InertPolicy {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        if !Self::is_inert(cmd.function) {
            return None;
        }
        // ⊘ An EMPTY body, which `RpcCommand::reply` zero-fills to the request's own
        // length. Not `cmd.payload.clone()`: the guest's declared physical address does
        // not come back at it, because nothing of the guest's ever should.
        Some(Reply {
            rpc_result: NV_OK,
            body: Vec::new(),
        })
    }
}

kayfabe_util::assert_send_sync!(InertPolicy);
