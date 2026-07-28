//! **Stage B6** — continuation-record reassembly, the last stage of the build order.
//!
//! `gsp_core_bridge.md` §2.6. A message larger than the guest's `maxRpcSize` is split by
//! `_issueRpcLarge`: the **first** element carries the real function with
//! `length = maxRpcSize`, and each subsequent element carries
//! `function = NV_VGPU_MSG_FUNCTION_CONTINUATION_RECORD` with
//! `length = entryLength + sizeof(rpc_message_header_v)` and a raw payload slice
//! (`ogkm-610: src/nvidia/src/kernel/vgpu/rpc.c:2074-2145`, notably `:2108`
//! `entryLength = maxRpcSize - sizeof(rpc_message_header_v)`; `ogkm-580: rpc.c:2053-2124`,
//! notably `:2087` — the same statement, and the same fragmenting loop).
//!
//! ## ★★ Which functions can actually fragment — measured, and it is **one**
//!
//! `[src]` The whole large-RPC surface is three call sites, and only one of them is a
//! function this bridge translates:
//!
//! | producer | wrapper | `bBidirectional` / `bWait` | our `RpcFunction` |
//! |---|---|---|---|
//! | `rpcRmApiControl_GSP` (`ogkm-610: rpc.c:10856`, `ogkm-580: :11051`) | `_issueRpcAndWaitLarge` | `NV_TRUE` / wait | **`RmControl`** |
//! | `rpcSetRegistry` (`ogkm-610: rpc.c:10533`, `ogkm-580: :10728`) | `_issueRpcAsyncLarge` | `NV_FALSE` / **no wait** | `SetRegistry` |
//! | `_issuePteDescRpc` (`ogkm-610: rpc.c:2323`, `ogkm-580: :2302`) | `_issueRpcAndWaitLarge` | `NV_FALSE` / wait | `Other` (fn `ALLOC_MEMORY`) |
//!
//! ★ **`GSP_RM_ALLOC` is not on that list and cannot be.** `rpcRmApiAlloc_GSP` copies the
//! params into the single message buffer with an explicit remaining-space bound and
//! returns `NV_ERR_BUFFER_TOO_SMALL` rather than fragmenting
//! (`ogkm-610: rpc.c:11024-11029`, `ogkm-580: :11218-11223` — same code in both trees). So a
//! fn-103 whose `paramsSize` exceeds its payload is **not** the head of a large RPC; it
//! stays [`BridgeRefusal::ParamsSizeExceedsPayload`], exactly as B1 wrote it.
//!
//! ⇒ [`Reassembler`] recognises a head **only** for [`RpcFunction::RmControl`], and that
//! is not a simplification: it is the complete set of fragmenting functions this bridge
//! has a declared total for. The other two are named in the crate docs as what they are.
//!
//! ## The declared total, and why there has to be one
//!
//! There is **no total-length field on the wire.** The head's `length` is `maxRpcSize` and
//! each continuation's is its own fragment size; nothing anywhere says how many follow.
//! §2.6's rule — *"translate when the declared total is complete"* — therefore needs a
//! total that comes from the head's **own body**, and for fn 76 there is exactly one:
//!
//! ```text
//! total_size    = fixed_param_size + paramsSize   (ogkm-610: rpc.c:10785, ogkm-580: :10981)
//! fixed_param_size = sizeof(rpc_message_header_v) + sizeof(rpc_gsp_rm_control_v03_00)
//!                                                 (ogkm-610: rpc.c:10678, ogkm-580: :10874,
//!                                                  which spells the second term
//!                                                  `sizeof(*rpc_params)` — same value)
//! ```
//!
//! and `RpcCommand::payload` is the body **after** the 32-byte envelope, so the
//! reassembled payload length is `total_size - 32` = `params_at + paramsSize` — the two
//! numbers [`kayfabe_abi::view::RpcControlReq`] already carries. Nothing is guessed and no
//! fragment count is inferred.
//!
//! ## What this module deliberately does not do
//!
//! It **concatenates and bounds; it decides nothing.** Serialization bits, class tables,
//! params-size agreement, the reserved client, the implicit VA space — every one of those
//! is [`crate::translate`]'s, applied **once**, to the reassembled whole. A reassembler
//! that pre-judged fragments would be a second, weaker copy of the translator running on
//! partial bytes.
//!
//! ★ And it holds **no handle**. The head buffer is bytes plus four numbers; it is keyed
//! by nothing the guest supplies, it is dropped the moment the message completes or
//! refuses, and there is at most one of it. The crate's standing rule — no handle table,
//! no seen-set, no dedup cache — is untouched, and
//! [`crate::translate`] is still a free function of one message.

use kayfabe_abi::versions::DriverAbiTable;
use kayfabe_gsp::{RpcCommand, RpcFunction};

use crate::BridgeRefusal;

/// The largest reassembled **payload** this bridge will hold, in bytes.
///
/// ★ A **policy** bound, not a driver constant, and it is named as one because the
/// driver's own bound is one we cannot see: `rpcSetRegistry` asserts
/// `totalSize < pRpc->pMessageQueueInfo->commandQueueSize`
/// (`ogkm-610: rpc.c:10510`, `ogkm-580: :10703`), and the
/// command queue's size lives in `kayfabe-gsp`'s geometry, which
/// `CommandPolicy::respond` is deliberately not given (§1.3 — the signature must not
/// widen).
///
/// The number is chosen against what it must not refuse: the largest control params
/// struct this port models is
/// [`kayfabe_abi::generated::ctrl::Nv0080CtrlDmaSetPageDirectoryParams`] at 32 bytes, and
/// every control the table does not model is already
/// [`BridgeRefusal::UnknownControl`] — so nothing reachable today comes close, and the
/// bound exists purely to cap a hostile guest. [`ReasmLimits`] makes it settable so the
/// day a real control needs more, raising it is a value change and not a code change.
pub const MAX_REASSEMBLED_BODY: usize = 64 * 1024;

/// The largest number of continuation records one head may absorb.
///
/// ★ **Not redundant with [`MAX_REASSEMBLED_BODY`].** A continuation carrying a
/// **zero-length** payload makes no progress towards the size bound at all, so a size
/// bound alone lets a guest hold a head open forever with an unbounded number of empty
/// fragments — unbounded *work*, even though the memory stays bounded. This is the bound
/// that refuses that, and `a_head_cannot_be_held_open_by_empty_continuations` is the test
/// that pins the distinction.
///
/// For a conforming guest the size bound binds first by a wide margin: fragments are
/// `maxRpcSize - sizeof(rpc_message_header_v)` = 4064 bytes
/// (`ogkm-610: rpc.c:1002` `maxRpcSize = RM_PAGE_SIZE`, `:2108`; `ogkm-580: rpc.c:1000`,
/// `:2087`), so 64 of them carry ~254
/// KiB, four times [`MAX_REASSEMBLED_BODY`].
pub const MAX_CONTINUATIONS: u32 = 64;

/// The two bounds §2.6 makes mandatory, as a value.
///
/// Carried rather than hard-coded so the hostile-length matrix can drive **small** limits
/// and see the same arms a real guest would take at 64 KiB — a bound that can only be
/// tested at its production value is a bound whose off-by-one nobody ever sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReasmLimits {
    /// Largest reassembled payload, in bytes.
    pub max_body: usize,
    /// Largest number of continuation records per head.
    pub max_continuations: u32,
}

impl Default for ReasmLimits {
    fn default() -> ReasmLimits {
        ReasmLimits {
            max_body: MAX_REASSEMBLED_BODY,
            max_continuations: MAX_CONTINUATIONS,
        }
    }
}

/// What one message meant to the reassembler.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub enum Reassembled {
    /// Not fragmented, and nothing is in flight: translate the message as it stands.
    Whole,
    /// The message was a fragment and was consumed. There is no object-model content
    /// yet — and *"no content yet"* is emphatically not *"no content"*, which is why it
    /// is a distinct answer rather than [`crate::Translation::Inert`].
    Held,
    /// The last fragment arrived. Translate **this** instead of the message that carried
    /// it: it is the head's function, the head's `(code, sequence)`, and the whole
    /// concatenated body.
    Complete(RpcCommand),
}

/// The one message being reassembled, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Head {
    function: RpcFunction,
    code: u32,
    sequence: u32,
    /// The head's own declared total payload length — `params_at + paramsSize`.
    needed: usize,
    /// Bytes so far. Never larger than `needed`, which is never larger than
    /// [`ReasmLimits::max_body`].
    body: Vec<u8>,
    /// Continuation records absorbed so far.
    continuations: u32,
    /// Ring elements consumed by the whole message so far, carried onto the reassembled
    /// [`RpcCommand`] so `elements` still measures the transport cost of the fact.
    elements: u32,
}

/// Continuation-record reassembly: **at most one** message in flight, bounded two ways.
///
/// ## The four rules, all of them §2.6's
///
/// 1. a continuation arriving with **no head in flight** is
///    [`BridgeRefusal::ContinuationWithoutHead`] — never a new head, because a
///    continuation record carries no function of its own to become one;
/// 2. a **new head while one is in flight** is [`BridgeRefusal::ContinuationInterleaved`].
///    `[inferred]` the driver issues one large RPC at a time under the GPU lock
///    (`_issueRpcLarge` runs to completion before returning), so interleaving is not a
///    legal trace;
/// 3. a declared total beyond [`ReasmLimits::max_body`] is
///    [`BridgeRefusal::ContinuationOverflow`], **refused at the head** and before a single
///    byte is reserved;
/// 4. more continuations than [`ReasmLimits::max_continuations`] is
///    [`BridgeRefusal::ContinuationCountExceeded`].
///
/// ★ Plus one the design did not name and the arithmetic requires:
/// [`BridgeRefusal::ContinuationOverrun`], a guest whose fragments carry **more** than its
/// own head declared. The alternative — take the first `needed` bytes — is
/// `abi_struct_truncation` with extra steps, and this project's rule is that a guest's
/// declared number and its actual bytes disagreeing is a refusal, never a clamp.
///
/// ## ★★ Every refusal **drops** the head
///
/// A reassembler that kept a head across a refusal could be wedged permanently by one
/// hostile message: every later fragment would then be attributed to a message the guest
/// abandoned. Dropping it costs a conforming guest nothing (it never interleaves) and
/// bounds a hostile one to a single message's damage. `a_refused_fragment_does_not_wedge`
/// is that property, asserted by exact content on the *next* message rather than by the
/// refusal itself.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Reassembler {
    limits: ReasmLimits,
    head: Option<Head>,
}

impl Reassembler {
    /// A reassembler with the production bounds.
    #[must_use]
    pub fn new() -> Reassembler {
        Reassembler::default()
    }

    /// A reassembler with explicit bounds — the hostile-length matrix's constructor.
    #[must_use]
    pub fn with_limits(limits: ReasmLimits) -> Reassembler {
        Reassembler { limits, head: None }
    }

    /// The bounds in force.
    #[must_use]
    pub fn limits(&self) -> ReasmLimits {
        self.limits
    }

    /// Is a fragmented message currently in flight?
    ///
    /// The instrument every "it was dropped" assertion needs: a test that could only see
    /// the *next* message's outcome cannot tell "the head was dropped" from "the head was
    /// kept and happened not to matter".
    #[must_use]
    pub fn in_flight(&self) -> bool {
        self.head.is_some()
    }

    /// How many bytes of a fragmented message are held right now.
    ///
    /// Zero when nothing is in flight. Bounded by [`ReasmLimits::max_body`], which is the
    /// whole of this type's memory footprint.
    #[must_use]
    pub fn held_bytes(&self) -> usize {
        self.head.as_ref().map_or(0, |h| h.body.len())
    }

    /// Offer one command to the reassembler.
    ///
    /// # Errors
    ///
    /// The five continuation refusals, by variant. Every one of them drops whatever head
    /// was in flight.
    pub fn accept(
        &mut self,
        abi: &DriverAbiTable,
        cmd: &RpcCommand,
    ) -> Result<Reassembled, BridgeRefusal> {
        if cmd.function == RpcFunction::ContinuationRecord {
            return self.absorb(cmd);
        }
        // A non-continuation while a head is in flight: refuse **this** message and drop
        // the head. Rule 2, and the drop is rule ★.
        if self.head.take().is_some() {
            return Err(BridgeRefusal::ContinuationInterleaved { code: cmd.code });
        }
        match declared_total(abi, cmd) {
            // The whole message arrived in one piece. `>` and not `>=`: a message whose
            // declared total equals what arrived is complete, and holding it would wait
            // forever for a continuation the guest has no reason to send.
            Some(needed) if needed > cmd.payload.len() => {
                if needed > self.limits.max_body {
                    // ★ Refused **before** anything is reserved: `needed` is a
                    // guest-declared u32 plus 40, so a hostile `paramsSize` is a
                    // four-gigabyte allocation if this test runs after the buffer.
                    return Err(BridgeRefusal::ContinuationOverflow {
                        declared: needed,
                        max: self.limits.max_body,
                    });
                }
                self.head = Some(Head {
                    function: cmd.function,
                    code: cmd.code,
                    sequence: cmd.sequence,
                    needed,
                    body: cmd.payload.clone(),
                    continuations: 0,
                    elements: cmd.elements,
                });
                Ok(Reassembled::Held)
            }
            _ => Ok(Reassembled::Whole),
        }
    }

    /// One continuation record.
    fn absorb(&mut self, cmd: &RpcCommand) -> Result<Reassembled, BridgeRefusal> {
        // Taken, not borrowed: every arm below either completes the head or refuses, and
        // both drop it. There is no path that puts a refused head back.
        let Some(mut head) = self.head.take() else {
            return Err(BridgeRefusal::ContinuationWithoutHead { code: cmd.code });
        };
        let continuations = head.continuations + 1;
        if continuations > self.limits.max_continuations {
            return Err(BridgeRefusal::ContinuationCountExceeded {
                continuations,
                max: self.limits.max_continuations,
            });
        }
        let have = head.body.len() + cmd.payload.len();
        if have > head.needed {
            // Never `truncate(needed)`. The guest declared a size and sent a different
            // one; taking the prefix would manufacture a struct it did not send.
            return Err(BridgeRefusal::ContinuationOverrun {
                have,
                declared: head.needed,
            });
        }
        head.continuations = continuations;
        head.elements = head.elements.saturating_add(cmd.elements);
        head.body.extend_from_slice(&cmd.payload);
        if head.body.len() < head.needed {
            self.head = Some(head);
            return Ok(Reassembled::Held);
        }
        Ok(Reassembled::Complete(RpcCommand {
            // ★ The head's, all four of them. The reassembled fact belongs to the
            // function the guest actually invoked, at the sequence it invoked it on —
            // `_issueRpcLarge`'s `expectedFunc`/`firstSequence` pair — the one it hands
            // `rpcRecvPoll` for the head (`ogkm-610: rpc.c:2156-2158`,
            // `ogkm-580: :2135-2137`). The *reply*, by contrast, is posted by the FSM
            // against the fragment that arrived last, which is where the driver reads
            // the status from; see [`crate::GraphPolicy`].
            function: head.function,
            code: head.code,
            sequence: head.sequence,
            payload: head.body,
            elements: head.elements,
        }))
    }
}

/// The total payload length a message's **own body** declares, when its function has one.
///
/// [`None`] means *"this function cannot fragment"*, which is a statement about the
/// driver and not about this message: see the module docs' table. It is deliberately not
/// `Some(payload.len())`, because "the total is whatever arrived" would make every
/// truncated message look complete.
fn declared_total(abi: &DriverAbiTable, cmd: &RpcCommand) -> Option<usize> {
    match cmd.function {
        RpcFunction::RmControl => {
            let h = abi.decode_rpc_control(&cmd.payload).ok()?;
            // Checked: `params_at` is a constant 40 and `params_size` is a guest-declared
            // `u32`, so on a 32-bit target the sum can wrap. A wrap here would let a
            // hostile `paramsSize` produce a *small* total and slip the bound.
            h.params_at.checked_add(h.params_size as usize)
        }
        // ★ Every other function, including `RmAlloc`. Not an omission — `rpcRmApiAlloc_GSP`
        // has no large path (`ogkm-610: rpc.c:11024-11029`, `ogkm-580: :11218-11223`), so
        // a short fn-103 is malformed
        // rather than fragmented and must keep reaching
        // `BridgeRefusal::ParamsSizeExceedsPayload`.
        _ => None,
    }
}

kayfabe_util::assert_send_sync!(Reassembler, Reassembled, ReasmLimits);
