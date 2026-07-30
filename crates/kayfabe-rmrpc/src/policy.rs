//! **Stage B2** — the adapter that joins the two halves: a [`kayfabe_gsp::CommandPolicy`]
//! whose answer to a command is *what the object model made of it*.
//!
//! [`translate`](crate::translate) is a pure function and stays one. This module is the
//! only thing in the crate that **applies**, and therefore the only thing that holds a
//! `&mut Gpu`. Nothing here decodes a byte.
//!
//! ## What B2 makes true that B1 did not
//!
//! At B1 the two ends were joined **by a test**: a `#[test]` called `translate`, matched
//! the `Translation`, and called `Gpu::apply` itself. That proves the types compose; it
//! does not put the bridge on the guest's own path. [`GraphPolicy`] is what the boot FSM
//! calls, from inside `GspFsm::service_command_queue`, for every command a guest actually
//! posts — so the graph is now driven by the transport rather than by a test body.
//!
//! ## ★ The three places a refusal surfaces, and how each is served here
//!
//! `gsp_core_bridge.md` §4.2 requires all three:
//!
//! 1. **On the wire.** [`GraphPolicy::respond`] returns `Some(Reply)` with a non-zero
//!    `rpc_result` — never `None`, which is what the FSM turns into `cmd.ack(0)`, i.e.
//!    the C's affirmative echo. A refusal is **not** a drop: the guest is blocked in
//!    `_issueRpcAndWait` polling `(function, sequence)`, so an unanswered command hangs
//!    it for the whole RPC timeout.
//! 2. **Countably.** [`GraphPolicy::census`] — a per-[`FaultTag`] tally, so an invariant
//!    can be the *bound* `testing_doctrine.md` §2 rule 4 asks for ("zero refusals over a
//!    clean boot script") rather than an absence.
//! 3. **In the return value.** [`GraphPolicy::deliver`] is the `Result` form, and it is
//!    what a test asserts an exact variant against. `respond` is a thin wrapper over it,
//!    because `Option<Reply>` cannot carry a variant and a test that could only see the
//!    status word would be asserting `0x56 == 0x56`.
//!
//! ★ **Why the trace is a census here and not a `TraceEvent`.** §4.2 item 2 says "a typed
//! `TraceEvent` per refusal". It cannot be one yet, and the obstruction is structural
//! rather than effort: `CommandPolicy::respond` takes no trace argument, `kayfabe_trace`'s
//! `Trace<'r>` wraps `&'r mut dyn Journal` and is therefore **not `Send`**, while
//! `CommandPolicy` **is** `Send` (`kayfabe_gsp::boot` asserts it) — so a `GraphPolicy`
//! holding a `Trace` would not implement the trait it exists to implement. `Gpu::apply`
//! takes no trace either, so no plane emits at this seam today. The census is the same
//! fact in the shape this seam can hold; the day `Gpu::apply` grows a trace argument, this
//! is where the event goes.
//!
//! ## ★ The state question, answered explicitly
//!
//! The crate doc's rule is that the **bridge** mints nothing and remembers nothing, and
//! that rule is unweakened: [`translate`](crate::translate) is still a free function of
//! one message, and this module does not wrap it in a cache. What [`GraphPolicy`] holds
//! is a `&mut Gpu` (which is the graph's state, not the bridge's), three **bounded,
//! handle-free** counters, and — from B6 — one [`Reassembler`]. None of them is keyed by
//! an `hClient` or an `hObject`, so none can refuse, deduplicate or mis-attribute a legal
//! recycle — which is the property [`RefusalCensus`]'s own docs pin, and which
//! `tests/tests/rmrpc_bridge.rs::a_recycled_hclient_survives_the_whole_transport` drives
//! from wire bytes through the FSM.
//!
//! ★★ **B6 is where that rule was most at risk, so state exactly what the reassembler is
//! not.** It is *not* a memo, a seen-set or a dedup cache: it holds **one** partial
//! message, it is looked up by nothing (there is no key — the head is the head), it is
//! dropped the instant the message completes or refuses, and two identical fragmented
//! controls sent back to back produce two identical `RmEvent`s with nothing carried
//! between them. That last property is what keeps the graph's idempotent-retry tolerance
//! reachable, and `two_identical_fragmented_controls_produce_two_identical_events` is the
//! canary for it.

use std::collections::BTreeMap;

use kayfabe_abi::DriverAbi;
use kayfabe_abi::GuestOs;
use kayfabe_abi::versions::DriverAbiTable;
use kayfabe_core::gpu::Gpu;
use kayfabe_gsp::{CommandPolicy, Reply, RpcCommand};
use kayfabe_trace::{FaultTag, Faulted};

use crate::{BridgeRefusal, ReasmLimits, Reassembled, Reassembler, Translation, translate};

/// How many refusals of each kind happened, by [`FaultTag`].
///
/// ★ **Bounded by construction, and that is why it may exist at all.** `fault_tag` is a
/// total function from a refusal to one of a *fixed, finite* set of `&'static str`s — the
/// [`BridgeRefusal`] variants, plus (through the delegating `Graph` arm) the
/// `RmGraphError`/`GpuError` variants. So this map cannot grow past that set however much
/// traffic a hostile guest sends, and it is keyed by **nothing the guest supplies**: no
/// handle, no client, no sequence number. A per-command log would be neither, and would
/// be a guest-reachable unbounded allocation of exactly the shape `GpuError::SpineCapacity`
/// exists to refuse.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RefusalCensus(BTreeMap<FaultTag, usize>);

impl RefusalCensus {
    /// How many refusals carried `tag`.
    #[must_use]
    pub fn of(&self, tag: FaultTag) -> usize {
        self.0.get(&tag).copied().unwrap_or(0)
    }

    /// Every tag seen, with its count, in tag order.
    pub fn tags(&self) -> impl Iterator<Item = (FaultTag, usize)> + '_ {
        self.0.iter().map(|(&t, &n)| (t, n))
    }

    /// Total refusals, across every tag.
    #[must_use]
    pub fn total(&self) -> usize {
        self.0.values().sum()
    }

    /// Nothing has been refused.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn record(&mut self, r: &BridgeRefusal) {
        *self.0.entry(r.fault_tag()).or_default() += 1;
    }
}

/// The GSP command policy that drives the RM object model.
///
/// `respond` = [`translate`](crate::translate) → [`Gpu::apply`] → a reply. It is the
/// whole of stage B2.
///
/// Borrows rather than owns the device: the same `Gpu` is reached by the doorbell path,
/// the completion path and the projection, and a policy that owned it would make itself
/// the only way in. `&'a mut Gpu` is also the exclusivity proof this needs — §1.3's named
/// caveat is that applying an event can mint a `Proc` through the isolate factory, which
/// is why the caller runs `respond` under the device write lock and why **no host verb may
/// be issued from inside it** (R1, no blocking under a lock).
pub struct GraphPolicy<'a> {
    /// Axis A: which driver's wire layouts to decode with. Selected once at realize.
    abi: &'a DriverAbiTable,
    /// ★ The **fourth axis** (`four_axes_of_variation.md` §1): which OS built that
    /// driver. Selected once at realize, beside `abi` and never inside it — the two are
    /// independent keys, and the doc's *"do not collapse guest OS into the version key"*
    /// is exactly the mistake a field on `DriverAbiTable` would be.
    ///
    /// ★★ There is **no default here on purpose.** Both constructors take it, so every
    /// site that builds a policy has to name the guest it is serving. Until 2026-07-29
    /// the answer was an unnamed "Linux", applied to every guest by a free function, and
    /// on a Windows guest it silently folded the guest kernel's RM clients into a guest
    /// process's isolate. A `new()` that quietly meant Linux would be the same silence
    /// one level up.
    guest_os: GuestOs,
    /// The object model this policy declares facts into.
    gpu: &'a mut Gpu,
    /// ★ B6. The crate's one piece of state, and the only thing here that is not either
    /// the graph's or a counter. Bounded two ways and keyed by nothing the guest supplies
    /// — see [`Reassembler`].
    reasm: Reassembler,
    census: RefusalCensus,
    applied: u64,
    inert: u64,
    held: u64,
    promoted: u64,
}

impl<'a> GraphPolicy<'a> {
    /// A policy that declares into `gpu`, decoding with `abi`, serving a `guest_os`.
    #[must_use]
    pub fn new(abi: &'a DriverAbiTable, guest_os: GuestOs, gpu: &'a mut Gpu) -> GraphPolicy<'a> {
        GraphPolicy::with_limits(abi, guest_os, gpu, ReasmLimits::default())
    }

    /// A policy with explicit continuation bounds — the hostile-length matrix's
    /// constructor, so the bound's own arms are reachable without building a 64 KiB
    /// message for every case.
    #[must_use]
    pub fn with_limits(
        abi: &'a DriverAbiTable,
        guest_os: GuestOs,
        gpu: &'a mut Gpu,
        limits: ReasmLimits,
    ) -> GraphPolicy<'a> {
        GraphPolicy {
            abi,
            guest_os,
            gpu,
            reasm: Reassembler::with_limits(limits),
            census: RefusalCensus::default(),
            applied: 0,
            inert: 0,
            held: 0,
            promoted: 0,
        }
    }

    /// The reassembler, for a caller that wants to see whether a fragmented message is
    /// still in flight.
    ///
    /// ★ Exposed as a **reference**, not as a `&mut`: a test may observe the held state,
    /// and nothing outside `deliver` may advance it.
    #[must_use]
    pub fn reassembler(&self) -> &Reassembler {
        &self.reasm
    }

    /// The refusal census — see [`RefusalCensus`].
    #[must_use]
    pub fn census(&self) -> &RefusalCensus {
        &self.census
    }

    /// How many commands produced an `RmEvent` the graph **accepted**.
    ///
    /// The non-vacuity instrument for every "no refusals" assertion: zero refusals over a
    /// run that also applied nothing is a policy that was never reached, which is the
    /// green-instrument-on-an-unexercised-path failure the doctrine is about.
    #[must_use]
    pub fn applied(&self) -> u64 {
        self.applied
    }

    /// How many commands were known-and-inert.
    ///
    /// ★ Counted separately from [`Self::applied`] on purpose: "this RPC carries no
    /// object-model content" and "this RPC declared a fact" are different observations,
    /// and a single total would let a regression that turned every alloc inert still
    /// report the same number.
    #[must_use]
    pub fn inert(&self) -> u64 {
        self.inert
    }

    /// How many commands were **fragments consumed into reassembly** ([`Translation::Held`]).
    ///
    /// ★ A third counter for the same reason there are two: "this fragment was absorbed"
    /// is a different observation from "this RPC declared a fact" and from "this RPC
    /// carried none", and a regression that silently dropped every fragment would leave
    /// `applied` and `inert` untouched. It is also the non-vacuity instrument for the
    /// reassembly tests — a run that completed a large control while holding nothing
    /// never fragmented anything.
    #[must_use]
    pub fn held(&self) -> u64 {
        self.held
    }

    /// How many commands were **context promotions the address plane accepted**
    /// ([`Translation::CtxPromotion`]).
    ///
    /// ★ The non-vacuity instrument for every promote assertion, and a fourth counter for
    /// the reason there are three: a promotion declares *address* facts, not object-model
    /// ones, and a run that folded it into [`Self::applied`] could not tell a regression
    /// that stopped joining anything from a run that had no promotions in it.
    #[must_use]
    pub fn promoted(&self) -> u64 {
        self.promoted
    }

    /// The object model, for a caller that wants to project it.
    #[must_use]
    pub fn gpu(&self) -> &Gpu {
        &*self.gpu
    }

    /// Translate one command and apply whatever it declared — the `Result` form.
    ///
    /// This is what [`CommandPolicy::respond`] is a wrapper over, and the form a test
    /// asserts an **exact variant** against: `Option<Reply>` carries only a status word,
    /// and every refusal currently carries the same one (§4.2's `[open]`).
    ///
    /// # Errors
    ///
    /// [`BridgeRefusal`], by variant — including [`BridgeRefusal::Graph`], which is the
    /// arm B1 could not construct because nothing in that stage applied.
    pub fn deliver(&mut self, cmd: &RpcCommand) -> Result<Translation, BridgeRefusal> {
        // ★ B6 — reassembly runs FIRST and decides nothing. It either hands `translate`
        // the message that arrived, or the message the guest actually meant, or holds a
        // fragment. Every rule `translate` owns is then applied exactly ONCE, to the
        // whole — a reassembler that pre-judged fragments would be a second, weaker copy
        // of the translator running on partial bytes.
        let outcome = self
            .reasm
            .accept(self.abi, cmd)
            .and_then(|r| match r {
                Reassembled::Whole => translate(self.abi, self.guest_os, cmd),
                Reassembled::Held => Ok(Translation::Held),
                Reassembled::Complete(full) => translate(self.abi, self.guest_os, &full),
            })
            .and_then(|t| match t {
                // ★ The bridge does not pre-empt the graph's MISS/DEFER/FAULT taxonomy — it
                // resolves nothing and asks nothing — and it must not swallow the answer
                // either. `Gpu::apply`'s refusal becomes a named `BridgeRefusal` here, which
                // is what turns it into a non-zero `rpc_result` on the wire. The C's
                // behaviour is the opposite: it accepted everything and answered `NV_OK`
                // (`nvidia-gpu-passthrough/src/qemu/nvkvm_gpu_emul.c:3326`).
                Translation::Event(ev) => self
                    .gpu
                    .apply(ev)
                    .map(|()| Translation::Event(ev))
                    .map_err(BridgeRefusal::Graph),
                // ★★ The ADDRESS-plane apply, and it is deliberately here rather than
                // left to the caller. A `Translation` variant nothing consumes is the
                // C's `NV_OK` echo with a Rust type on it — the same argument
                // `BridgeRefusal::UnknownControl` already makes about a `Forward` arm.
                // The join routes on the ADDRESS SPACE the promotion names, so it is
                // legal to run against `&mut Gpu` regardless of which proc's RPC this
                // was; the sharded shell runs the same two functions under its own locks.
                Translation::CtxPromotion(p) => self
                    .gpu
                    .promote_ctx(&p)
                    .map(|_| Translation::CtxPromotion(p))
                    .map_err(BridgeRefusal::Promote),
                Translation::Inert | Translation::Held => Ok(t),
            });
        match &outcome {
            Ok(Translation::Event(_)) => self.applied = self.applied.saturating_add(1),
            Ok(Translation::Inert) => self.inert = self.inert.saturating_add(1),
            Ok(Translation::Held) => self.held = self.held.saturating_add(1),
            // ★ Its own counter, for the reason `held` has one: "this RPC declared
            // address bindings" is a different observation from "this RPC declared a
            // graph fact", and a single total would let a regression that stopped
            // promoting anything report the same number.
            Ok(Translation::CtxPromotion(_)) => self.promoted = self.promoted.saturating_add(1),
            Err(r) => self.census.record(r),
        }
        outcome
    }
}

/// Hand-written because `Gpu` is not `Debug` — and it should not become `Debug` for a
/// policy's convenience: the whole object model in a panic message is unreadable, and the
/// two numbers below plus the census are what a failure here is actually about.
impl core::fmt::Debug for GraphPolicy<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GraphPolicy")
            .field("driver", &self.abi.version())
            .field("applied", &self.applied)
            .field("inert", &self.inert)
            .field("held", &self.held)
            .field("in_flight", &self.reasm.in_flight())
            .field("census", &self.census)
            .finish_non_exhaustive()
    }
}

impl CommandPolicy for GraphPolicy<'_> {
    /// Answer one command.
    ///
    /// **Accepted, inert or held → `None`**, which the FSM turns into `cmd.ack(0)`: the
    /// `(function, sequence)` pair echoed with `NV_OK` and the request's own body
    /// preserved. That is deliberate rather than an omission — reply **bodies** are the
    /// device data model's job (`mode2_device_data_model.md` class C) and are named
    /// out of scope by `gsp_core_bridge.md` §6, so B2 owes the guest the acknowledgement
    /// and nothing more. The alternative, `Some(Reply { rpc_result: 0, body })`, is the
    /// identical wire bytes with a second place to get the status wrong.
    ///
    /// **Refused → `Some(Reply)`** carrying [`BridgeRefusal::rpc_result`] and an **empty**
    /// body, which `RpcCommand::reply` zero-fills to the request's own length (the M9
    /// clamp). Empty rather than the request echoed back: reflecting the guest's own bytes
    /// at it under a failing status is precisely `memcpy(resp, cmd, 4096)`
    /// (`nvidia-gpu-passthrough/src/qemu/nvkvm_gpu_emul.c:2737`). `[open]`, with §4.2's:
    /// which `NV_STATUS` each refusal deserves needs an `NV_STATUS` table that does not
    /// exist, and B4 revisits both together.
    ///
    /// ★★ **The zero-filled body is unreachable by the guest, and the mechanism is not
    /// the one this doc used to name.** It said *"the status we send is the one RM answers
    /// with `SKIP_COPYOUT`"*. `SKIP_COPYOUT` is real
    /// (`ogkm-580: src/nvidia/src/kernel/rmapi/control.c:314-318`) but it is one layer
    /// below and it is **conditional** — it is skipped when the control carries
    /// `RMCTRL_FLAGS_COPYOUT_ON_ERROR`, a property the guest advertises to us on the wire
    /// (`ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:10997-10998`,
    /// `ogkm-610: :10802-10803`). What actually saves us is one level up and
    /// unconditional: the refusal rides the RPC **envelope**'s `rpc_result`, so
    /// `_issueRpcAndWait` returns non-`NV_OK` (`ogkm-580: rpc.c:1994`, `ogkm-610: :2012`)
    /// and `rpcRmApiControl_GSP`'s whole post-RPC block — the copy-out *and* the control
    /// cache — never runs at all.
    ///
    /// That distinction is load-bearing rather than pedantic, because the cache half of
    /// that block would make a wrong answer **sticky**: see
    /// [`BridgeRefusal::GspRuleControlUnserviced`] §3, which reads the branch out of the
    /// guest's own source. A refusal expressed in the reply *body* instead of the envelope
    /// would inherit both hazards.
    ///
    /// ★★ **The `Held` arm is load-bearing on the wire, not a convenience.** For a
    /// fragmented `GSP_RM_CONTROL` the driver awaits one reply per fragment — the head at
    /// `(expectedFunc, firstSequence)`, then each continuation at
    /// `(CONTINUATION_RECORD, firstSequence + i)` — and reads `rpc_result` from the
    /// **last** one it received (`ogkm-610: rpc.c:2156-2241`, `ogkm-580: :2135-2220` —
    /// the same loop and the same final read). So `None` here is the right
    /// answer for the head and every intermediate fragment (an `NV_OK` ack, echoing that
    /// fragment's own body and length, which is what the loop's `entryLength` arithmetic
    /// consumes), and the reassembly completes on the final fragment — the very one whose
    /// reply the guest will read the status off. The two facts line up without either
    /// side arranging it, which is worth saying because it means a change to *either*
    /// breaks a guest silently.
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        match self.deliver(cmd) {
            Ok(_) => None,
            Err(r) => Some(Reply {
                rpc_result: r.rpc_result(),
                body: Vec::new(),
            }),
        }
    }
}

// The concurrency contract, compile-time-asserted (decision #17). `GraphPolicy` must be
// `Send` or it cannot be a `CommandPolicy` at all — the FSM takes `&mut dyn CommandPolicy`
// and `kayfabe_gsp::boot` asserts the trait object is `Send`.
kayfabe_util::assert_send_sync!(RefusalCensus);
kayfabe_util::assert_send!(GraphPolicy<'static>);
