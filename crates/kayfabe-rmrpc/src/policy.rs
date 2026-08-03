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
use kayfabe_core::gpu::{Gpu, GpuError};
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
}

/// ★★★ **The refusal census as a handle the composition root can still read after it has
/// given the policy away** — the instrument the 2026-08-01 `alloc1` boot did without.
///
/// # Why this type exists, and what it costs to not have it
///
/// `[measured]` boot `alloc1` at rev `2ced035` (`docs/design/boot_measured_2026_08_01.md`
/// §6): every `GSP_RM_ALLOC` was refused `ParamsSizeExceedsPayload` **inside the bridge**,
/// and the only way anyone could tell was that `fn 103` was *missing* from the unserviced
/// ledger's six lines. A bridge refusal answers the command — with a non-zero
/// `rpc_result` — so it never reaches [`crate::policy`]'s terminal recorders, and the port
/// had no channel that said *"the bridge refused something"*. The diagnosis was
/// **by absence**, which is precisely what `kayfabe_device::unserviced::UnservicedLedger`
/// was built to abolish for the other half of the chain.
///
/// ⊘ The obstruction was ownership, not instrumentation. `ObjectPolicy` **owns** its
/// `Gpu`, is installed as a `Box<dyn CommandPolicy>`, and is therefore unreachable from
/// the composition root the moment it is boxed — so a census that lived only behind
/// `&self` could be read by a test and by nothing else. This handle is clonable and is
/// kept by the root, exactly as `UnservicedLog` is.
///
/// ★ **One store, not two.** [`Bridge`] records here and nowhere else, and
/// [`ObjectPolicy::census`] is a *snapshot* taken from this. A mirror kept beside the
/// original would be the "two lists that agree today" shape this repository has been
/// bitten by repeatedly; there is nothing here to drift from.
///
/// The bound argument in [`RefusalCensus`]'s docs is unweakened: the key is still a
/// [`FaultTag`] from a fixed finite set and still nothing the guest supplies.
#[derive(Debug, Clone, Default)]
pub struct SharedRefusalCensus(std::sync::Arc<std::sync::Mutex<BTreeMap<FaultTag, usize>>>);

impl SharedRefusalCensus {
    /// A fresh, empty census.
    #[must_use]
    pub fn new() -> SharedRefusalCensus {
        SharedRefusalCensus::default()
    }

    /// A point-in-time copy — the value form every existing reader asserts against.
    #[must_use]
    pub fn snapshot(&self) -> RefusalCensus {
        let g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        RefusalCensus(g.clone())
    }

    fn record(&self, r: &BridgeRefusal) {
        let mut g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        *g.entry(r.fault_tag()).or_default() += 1;
    }
}

/// ★★★ **What the guest said its GPFIFO rings live at** — the §8.2.2 measurement, as a
/// value the composition root can still read after the policy is boxed.
///
/// # What this answers, and what it deliberately does not
///
/// `kayfabe_arch::PushRange::gpa` is fed to `Vmm::gpa_read` with no walk, while a GA10x
/// GPFIFO entry names a GPU **virtual** address. Whether that is a *live* defect or a
/// latent one turns on one number nobody had ever looked at: the address the guest itself
/// names for a ring, at the wall this port stops at. This census is that number, carried
/// out of a boot.
///
/// ⊘ It cannot say *"and here is the GPA it corresponds to"*, and that absence is a
/// finding rather than a gap in the instrument: the binding that would answer it is a
/// `MAP_MEMORY_DMA`, and that RPC is a HAL stub on every GSP-client part, so it never
/// reaches this port at all (crate docs, §2.7). There is no second number to compare
/// against because the guest never tells us one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RingCensus {
    /// Channel allocs that reached the graph carrying a decoded `gpFifoOffset` —
    /// including the ones that declared `0`.
    pub declarations: u64,
    /// Of those, how many named a **non-zero** ring address. ⚠ Not the same number:
    /// `gpFifoOffset = 0` is a real declaration the driver makes on purpose
    /// (`ogkm-580: kernel_graphics.c:2420-2424`), so folding the two would report a
    /// golden-context channel as *"no ring seen"*.
    pub nonzero: u64,
    /// The **first** non-zero ring the guest declared: `(va, entries)`.
    ///
    /// ⊘ First, not last, for `KayfabeRegAudit::doorbell_refusal`'s reason: a boot that
    /// declares many rings must not be able to push the first observation out of the one
    /// line a teardown report has room for.
    pub first_nonzero: Option<(u64, u32)>,
}

/// [`RingCensus`] as a handle the composition root keeps after handing the policy away —
/// same shape, and the same ownership argument, as [`SharedRefusalCensus`].
#[derive(Debug, Clone, Default)]
pub struct SharedRingCensus(std::sync::Arc<std::sync::Mutex<RingCensus>>);

impl SharedRingCensus {
    /// A fresh, empty census.
    #[must_use]
    pub fn new() -> SharedRingCensus {
        SharedRingCensus::default()
    }

    /// A point-in-time copy.
    #[must_use]
    pub fn snapshot(&self) -> RingCensus {
        *self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Record one channel alloc's declared ring.
    fn record(&self, r: kayfabe_core::rmgraph::GpFifoRing) {
        let mut g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        g.declarations = g.declarations.saturating_add(1);
        if r.va != 0 {
            g.nonzero = g.nonzero.saturating_add(1);
            if g.first_nonzero.is_none() {
                g.first_nonzero = Some((r.va, r.entries));
            }
        }
    }
}

/// ★★★ **The object model, as the two mutations a delivered command can make** — the seam
/// that lets one bridge declare into a bare [`Gpu`] *or* into a shell that owns it behind
/// ranked locks.
///
/// # Why this exists, in the words of the type it un-blocks
///
/// [`ObjectPolicy`]'s own docs have said, since it was written:
///
/// > Owns its [`Gpu`] because a `CommandPolicy` is installed as `Box<dyn CommandPolicy>`
/// > … ⚠ That is a **stage fact, not a design**: `GraphPolicy`'s doc explains why
/// > borrowing is right the moment the doorbell path and the projection also want it, and
/// > the day either exists this type hands its `Gpu` to whatever owns them.
///
/// `docs/design/execution_plane_increments.md` **E2** is that day. A guest MMIO write to
/// the usermode doorbell aperture has to reach `kayfabe_rt::SharedDevice::doorbell`, and
/// it must reach **the same object model** the guest's own `GSP_RM_ALLOC`s populated — a
/// second `Gpu` behind the doorbell would be a routing table that can never resolve, i.e. a
/// green transport over a permanently-wrong answer. So the model becomes a port, one
/// implementation is the plain `Gpu` this crate already had, and the composition root
/// supplies the other.
///
/// # ★ Two methods, and they are the two `Bridge::deliver` calls
///
/// Not a general "device" interface: exactly the mutations a translated command performs,
/// so the surface cannot grow by accident. [`ObjectModel::isolate_census`] is here for the
/// third thing [`ObjectPolicy`] does after every delivery (E1's republication) and reads
/// nothing else.
///
/// `Send` because a `CommandPolicy` is held by the GSP FSM across vCPU threads
/// (`kayfabe_gsp::boot` asserts the trait object is `Send`).
pub trait ObjectModel: Send {
    /// Apply one RM protocol event.
    ///
    /// # Errors
    /// [`kayfabe_core::gpu::GpuError`] — the graph's own refusal, unchanged.
    fn apply(&mut self, ev: kayfabe_core::rmgraph::RmEvent) -> Result<(), GpuError>;

    /// Apply one context promotion.
    ///
    /// # Errors
    /// [`kayfabe_core::promote::PromoteFault`], by variant.
    fn promote_ctx(
        &mut self,
        p: &kayfabe_core::promote::CtxPromotion,
    ) -> Result<kayfabe_core::promote::PromoteJoin, kayfabe_core::promote::PromoteFault>;

    /// Publish the isolate plane's health — increment E1's census — into `to`.
    ///
    /// ★ A **publish** and not a getter, so this trait never names `IsolateCensus` and this
    /// crate needs no `kayfabe-isolate` dependency to hold the port. It also puts the two
    /// implementations' locking where it belongs: a bare `Gpu` walks its own maps, a shell
    /// takes its device read lock and each proc lock in turn, and neither has to hand out a
    /// value across a guard.
    fn publish_isolate_census(&self, to: &kayfabe_core::gpu::SharedIsolateCensus);

    /// ★★★ **#177** — perform the guest's `NVA06F_CTRL_CMD_GPFIFO_SCHEDULE`.
    ///
    /// # Errors
    /// [`kayfabe_core::gpu::ScheduleFault`], by variant.
    fn schedule_channel(
        &mut self,
        client: kayfabe_arch::ids::HClient,
        object: kayfabe_arch::ids::HObject,
        enable: bool,
    ) -> Result<kayfabe_core::gpu::ScheduleAck, kayfabe_core::gpu::ScheduleFault>;

    /// The model as a plain [`Gpu`], **mutably**, if it is one. Same contract and same
    /// `None` as [`Self::as_gpu`].
    fn as_gpu_mut(&mut self) -> Option<&mut Gpu>;

    /// The model as a plain [`Gpu`], if it **is** one.
    ///
    /// ⊘ `None` is the honest answer for a model behind ranked locks: there is no `&Gpu`
    /// to hand out, because the state lives inside a device lock and a proc lock and a
    /// borrow of it would outlive both guards. Callers that projected the graph directly
    /// (tests, the differential) hold a bare `Gpu` and get `Some`; the shipped composition
    /// root does not project and does not ask.
    fn as_gpu(&self) -> Option<&Gpu>;
}

/// The bare-[`Gpu`] implementation — the one this crate had inline before E2 made the
/// model a port. Every arm is the call `Bridge::deliver` used to make directly.
impl ObjectModel for Gpu {
    fn apply(&mut self, ev: kayfabe_core::rmgraph::RmEvent) -> Result<(), GpuError> {
        Gpu::apply(self, ev)
    }

    fn promote_ctx(
        &mut self,
        p: &kayfabe_core::promote::CtxPromotion,
    ) -> Result<kayfabe_core::promote::PromoteJoin, kayfabe_core::promote::PromoteFault> {
        Gpu::promote_ctx(self, p)
    }

    fn publish_isolate_census(&self, to: &kayfabe_core::gpu::SharedIsolateCensus) {
        to.publish(Gpu::isolate_census(self));
    }

    fn schedule_channel(
        &mut self,
        client: kayfabe_arch::ids::HClient,
        object: kayfabe_arch::ids::HObject,
        enable: bool,
    ) -> Result<kayfabe_core::gpu::ScheduleAck, kayfabe_core::gpu::ScheduleFault> {
        Gpu::schedule_channel(self, client, object, enable)
    }

    fn as_gpu(&self) -> Option<&Gpu> {
        Some(self)
    }

    fn as_gpu_mut(&mut self) -> Option<&mut Gpu> {
        Some(self)
    }
}

/// Everything a bridge policy is *besides* the object model it declares into.
///
/// ★ Extracted 2026-08-01 so that [`GraphPolicy`] (which **borrows** a `Gpu`) and
/// [`ObjectPolicy`] (which **owns** one) are one implementation and not two. The
/// alternative — a second `deliver` — would be a second copy of the reassembly ordering,
/// the four counters and the census-vs-outcome bookkeeping, i.e. exactly the shape that
/// drifts silently: a fix applied to one and not the other is invisible to every test that
/// drives only the other.
///
/// `gpu` is a **parameter** of [`Bridge::deliver`] rather than a field, which is what makes
/// the split possible at all: the holder decides the ownership, the bridge decides the
/// meaning.
struct Bridge {
    /// Axis A: which driver's wire layouts to decode with. Selected once at realize.
    ///
    /// ★ By value, not by reference — [`DriverAbiTable`] is `Copy`. The public
    /// constructors still take `&DriverAbiTable` (their callers hold one), so this is an
    /// internal change with no API surface.
    abi: DriverAbiTable,
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
    /// ★ B6. The crate's one piece of state, and the only thing here that is not either
    /// the graph's or a counter. Bounded two ways and keyed by nothing the guest supplies
    /// — see [`Reassembler`].
    reasm: Reassembler,
    census: SharedRefusalCensus,
    /// ★ §8.2.2 — the GPFIFO ring addresses the guest declared. Recorder-only.
    rings: SharedRingCensus,
    applied: u64,
    inert: u64,
    held: u64,
    promoted: u64,
}

impl Bridge {
    fn new(abi: DriverAbiTable, guest_os: GuestOs, limits: ReasmLimits) -> Bridge {
        Bridge {
            abi,
            guest_os,
            reasm: Reassembler::with_limits(limits),
            census: SharedRefusalCensus::default(),
            rings: SharedRingCensus::default(),
            applied: 0,
            inert: 0,
            held: 0,
            promoted: 0,
        }
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
///
/// ⚠ **It claims EVERY command** — `respond` never returns `None` — so it is *the* policy
/// or it is the end of a chain, never a link with anything after it. That is right for the
/// differential and wrong for a device chain that still wants its own recorders to see
/// what nobody answered; [`ObjectPolicy`] is the composable form, and the two differ in
/// exactly that one property.
pub struct GraphPolicy<'a> {
    bridge: Bridge,
    gpu: &'a mut Gpu,
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
            bridge: Bridge::new(*abi, guest_os, limits),
            gpu,
        }
    }

    /// The reassembler, for a caller that wants to see whether a fragmented message is
    /// still in flight.
    ///
    /// ★ Exposed as a **reference**, not as a `&mut`: a test may observe the held state,
    /// and nothing outside `deliver` may advance it.
    #[must_use]
    pub fn reassembler(&self) -> &Reassembler {
        &self.bridge.reasm
    }

    /// The refusal census, as a point-in-time snapshot — see [`RefusalCensus`].
    ///
    /// ★ By value since the census became a [`SharedRefusalCensus`]: the store is behind a
    /// handle the composition root also holds, so there is no `&` to hand out that would
    /// still be a single owner's. A snapshot is what every reader wanted anyway.
    #[must_use]
    pub fn census(&self) -> RefusalCensus {
        self.bridge.census.snapshot()
    }

    /// The census as a **handle**, for the composition root that must keep reading it after
    /// boxing this policy — see [`SharedRefusalCensus`].
    #[must_use]
    pub fn refusal_census(&self) -> SharedRefusalCensus {
        self.bridge.census.clone()
    }

    /// ★ §8.2.2 — the GPFIFO-ring census as a **handle**, for the same reason
    /// [`Self::refusal_census`] is one: the composition root keeps reading it after this
    /// policy is boxed. See [`SharedRingCensus`].
    #[must_use]
    pub fn ring_census(&self) -> SharedRingCensus {
        self.bridge.rings.clone()
    }

    /// How many commands produced an `RmEvent` the graph **accepted**.
    ///
    /// The non-vacuity instrument for every "no refusals" assertion: zero refusals over a
    /// run that also applied nothing is a policy that was never reached, which is the
    /// green-instrument-on-an-unexercised-path failure the doctrine is about.
    #[must_use]
    pub fn applied(&self) -> u64 {
        self.bridge.applied
    }

    /// How many commands were known-and-inert.
    ///
    /// ★ Counted separately from [`Self::applied`] on purpose: "this RPC carries no
    /// object-model content" and "this RPC declared a fact" are different observations,
    /// and a single total would let a regression that turned every alloc inert still
    /// report the same number.
    #[must_use]
    pub fn inert(&self) -> u64 {
        self.bridge.inert
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
        self.bridge.held
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
        self.bridge.promoted
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
        self.bridge.deliver(self.gpu, cmd)
    }
}

impl Bridge {
    /// Translate one command and apply whatever it declared, into the caller's `gpu`.
    ///
    /// ★★ The `gpu` is a **parameter**, and that is the whole of the borrow/own split:
    /// [`GraphPolicy`] passes a `&mut Gpu` it borrowed, [`ObjectPolicy`] passes the
    /// [`ObjectModel`] it owns, and neither can have a different idea of what a command
    /// means.
    ///
    /// ★★★ Since E2 the parameter is `&mut dyn ObjectModel` rather than `&mut Gpu`, and
    /// that is the *only* change: a shell whose object model lives behind ranked locks
    /// declares into it through the same two calls, in the same order, with the same
    /// counters and the same census. A second `deliver` for the shell would be the drift
    /// this function was extracted to prevent.
    fn deliver(
        &mut self,
        gpu: &mut dyn ObjectModel,
        cmd: &RpcCommand,
    ) -> Result<Translation, BridgeRefusal> {
        // ★ B6 — reassembly runs FIRST and decides nothing. It either hands `translate`
        // the message that arrived, or the message the guest actually meant, or holds a
        // fragment. Every rule `translate` owns is then applied exactly ONCE, to the
        // whole — a reassembler that pre-judged fragments would be a second, weaker copy
        // of the translator running on partial bytes.
        //
        // ★ Cloned out ahead of the chain (it is an `Arc`) so the recorder below borrows
        // this handle and not `self`, which `reasm` already holds mutably.
        let rings = self.rings.clone();
        let outcome = self
            .reasm
            .accept(&self.abi, cmd)
            .and_then(|r| match r {
                Reassembled::Whole => translate(&self.abi, self.guest_os, cmd),
                Reassembled::Held => Ok(Translation::Held),
                Reassembled::Complete(full) => translate(&self.abi, self.guest_os, &full),
            })
            .and_then(|t| match t {
                // ★ The bridge does not pre-empt the graph's MISS/DEFER/FAULT taxonomy — it
                // resolves nothing and asks nothing — and it must not swallow the answer
                // either. `Gpu::apply`'s refusal becomes a named `BridgeRefusal` here, which
                // is what turns it into a non-zero `rpc_result` on the wire. The C's
                // behaviour is the opposite: it accepted everything and answered `NV_OK`
                // (`nvidia-gpu-passthrough/src/qemu/nvkvm_gpu_emul.c:3326`).
                Translation::Event(ev) => {
                    // ★★★ §8.2.2 — recorded on the TRANSLATION, before the graph is
                    // asked, and that ordering is the whole of the instrument's honesty.
                    // The channel alloc at this port's wall is one the projection has
                    // been observed to refuse (`boot_measured_2026_08_01.md`: hClass
                    // 0xc56f with a `GpuError::Projection`), so a census taken on the
                    // `Ok` arm below would report *"no ring was ever declared"* about a
                    // boot in which the guest declared one and we said no. The question
                    // is what the GUEST named, not what we accepted.
                    if let kayfabe_core::rmgraph::RmEvent::Alloc {
                        facts:
                            kayfabe_core::rmgraph::AllocFacts {
                                gp_fifo_ring: Some(r),
                                ..
                            },
                        ..
                    } = ev
                    {
                        rings.record(r);
                    }
                    gpu.apply(ev)
                        .map(|()| Translation::Event(ev))
                        .map_err(BridgeRefusal::Graph)
                }
                // ★★ The ADDRESS-plane apply, and it is deliberately here rather than
                // left to the caller. A `Translation` variant nothing consumes is the
                // C's `NV_OK` echo with a Rust type on it — the same argument
                // `BridgeRefusal::UnknownControl` already makes about a `Forward` arm.
                // The join routes on the ADDRESS SPACE the promotion names, so it is
                // legal to run against `&mut Gpu` regardless of which proc's RPC this
                // was; the sharded shell runs the same two functions under its own locks.
                Translation::CtxPromotion(p) => gpu
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

    /// The `Debug` body both policies share, so the two cannot describe themselves
    /// differently — `name` is the only thing that varies.
    fn debug_as(&self, name: &'static str, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct(name)
            .field("driver", &self.abi.version())
            .field("applied", &self.applied)
            .field("inert", &self.inert)
            .field("held", &self.held)
            .field("in_flight", &self.reasm.in_flight())
            .field("census", &self.census)
            .finish_non_exhaustive()
    }
}

/// Hand-written because `Gpu` is not `Debug` — and it should not become `Debug` for a
/// policy's convenience: the whole object model in a panic message is unreadable, and the
/// two numbers below plus the census are what a failure here is actually about.
impl core::fmt::Debug for GraphPolicy<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.bridge.debug_as("GraphPolicy", f)
    }
}

impl CommandPolicy for GraphPolicy<'_> {
    /// Answer one command.
    ///
    /// **Accepted, inert or held → `Some(Reply)`** carrying `NV_OK` and the request's own
    /// body: the C-baseline acknowledgement (`C: src/qemu/nvkvm_gpu_emul.c:2410-2416`),
    /// asked for **explicitly**. Reply *bodies* are the device data model's job
    /// (`mode2_device_data_model.md` class C) and are named out of scope by
    /// `gsp_core_bridge.md` §6, so B2 owes the guest the acknowledgement and nothing more.
    ///
    /// ## ★★★ This used to be `None`, and the change is not cosmetic (task #127)
    ///
    /// `None` used to mean *"let the FSM post its own `cmd.ack(0)`"*, so this doc argued
    /// that spelling it out would be *"the identical wire bytes with a second place to get
    /// the status wrong"*. That argument died with the default. **`None` now means "I
    /// decline", and the FSM answers a declined command with a named refusal** — because
    /// echoing by default was measured to hand a guest its own uninitialised kernel stack
    /// and fault it (`kayfabe_gsp::GspFsm::answer`). One word therefore had two meanings:
    /// *"I accepted this, acknowledge it"* and *"I have nothing for this"*. They are now
    /// two answers, and this is the first.
    ///
    /// ⚠ **What that leaves, stated plainly.** The echo is gone as a default; it survives
    /// here, for the commands this policy has **accepted** — an allowlisted, modelled set
    /// with a decoded body, which is a categorically different thing from reflecting a
    /// command nobody looked at. The bytes are unchanged on purpose: the fragmented-control
    /// arithmetic below consumes each fragment's own body and length, so changing them is a
    /// separate, measurable step and not a side effect of moving the default. Subtracting
    /// it — authoring the reply body instead of reflecting it — belongs with the device
    /// data model that owns reply bodies.
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
    /// ## ★★★ …and the argument above is about REFUSALS ONLY. Here is the accepted path
    ///
    /// ⊘ **The envelope short-circuit says nothing about a command this policy ACCEPTS.**
    /// An accepted control leaves here with `rpc_result: 0` and the request's own body, so
    /// `_issueRpcAndWait` returns `NV_OK`, `rpcRmApiControl_GSP`'s post-RPC block runs in
    /// full, and the control cache is live for it. That gap sat unexamined until
    /// 2026-08-01: every sticky-answer sentence in this crate was attached to a refusal.
    ///
    /// What discharges it is a property of the **body**, read out of the guest's send path
    /// rather than assumed. `rpcRmApiControl_GSP` writes `rpc_params->rmctrlFlags = 0;
    /// rpc_params->rmctrlAccessRight = 0;` into every request it sends
    /// (`ogkm-580: rpc.c:10994-10995`, `ogkm-610: :10799-10800`), and the GSS-legacy cache
    /// branch is guarded by `rmapiControlIsCacheable(rpc_params->rmctrlFlags,
    /// rpc_params->rmctrlAccessRight, NV_TRUE)`, whose first test is
    /// `!(flags & RMCTRL_FLAGS_CACHEABLE_ANY) -> NV_FALSE`
    /// (`ogkm-580: rmapi_cache.c:152-158`, `ogkm-610: :152-158`). Reflecting the request
    /// therefore reflects **zero** into both fields, and zero is *"do not remember this"*.
    ///
    /// ⚠ **That is an accident of the echo, not a decision this type makes, and it holds
    /// only for a stock sender.** `rmctrlFlags` is a field on a message the *guest* wrote;
    /// a guest that pre-sets `RMCTRL_FLAGS_CACHEABLE` in a bit-15 request gets it handed
    /// straight back under `NV_OK`. This type has no guard against that because nothing
    /// installs it in the port — `kayfabe_device::served_policy` is the one production
    /// chain and it is wrapped in `kayfabe_device::sticky::StickyAnswerGuard`, which zeroes
    /// both fields on every accepted control reply. ⊘ **The day this policy is installed,
    /// it must go inside that wrapper**, and `tests/tests/sticky_answer.rs` is where the
    /// claim that it is not installed is checked rather than asserted.
    ///
    /// ★★ **The `Held` arm is load-bearing on the wire, not a convenience.** For a
    /// fragmented `GSP_RM_CONTROL` the driver awaits one reply per fragment — the head at
    /// `(expectedFunc, firstSequence)`, then each continuation at
    /// `(CONTINUATION_RECORD, firstSequence + i)` — and reads `rpc_result` from the
    /// **last** one it received (`ogkm-610: rpc.c:2156-2241`, `ogkm-580: :2135-2220` —
    /// the same loop and the same final read). So the accepted arm is the right
    /// answer for the head and every intermediate fragment (an `NV_OK` ack, echoing that
    /// fragment's own body and length, which is what the loop's `entryLength` arithmetic
    /// consumes), and the reassembly completes on the final fragment — the very one whose
    /// reply the guest will read the status off. The two facts line up without either
    /// side arranging it, which is worth saying because it means a change to *either*
    /// breaks a guest silently.
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        match self.deliver(cmd) {
            // ★ Explicit, not `None`. See this method's docs: `None` is a refusal now.
            Ok(_) => Some(Reply {
                rpc_result: 0, // NV_OK
                body: cmd.payload.clone(),
            }),
            Err(r) => Some(Reply {
                rpc_result: r.rpc_result(),
                body: Vec::new(),
            }),
        }
    }
}

/// ★★★ The **object-declaring verbs**, as a chain link a device policy chain can hold —
/// the form of this bridge that a port installs.
///
/// # Why this exists next to [`GraphPolicy`], and what the one difference is
///
/// `GraphPolicy` claims **every** command: its `respond` never returns `None`. That is
/// right for the C differential (it *is* the policy) and wrong for a port, because
/// `kayfabe_device::served_chain` ends in two recorders that only ever see what the links
/// above them declined. A link that answers everything makes the unserviced ledger — the
/// one instrument that can say *"what has this port not built yet"* — permanently empty.
///
/// So this one claims a **declared, closed set of RPC functions** ([`OBJECT_VERBS`]) and
/// returns `None` for everything else, byte for byte leaving every other arm of the chain
/// exactly as it was.
///
/// # ⊘ What it does NOT claim, and why that is a decision rather than an omission
///
/// - **`GSP_RM_CONTROL`.** `translate` maps two controls to facts (`SetPageDir`,
///   `PromoteCtx`), and claiming the function here would take `GSP_RM_CONTROL` away from
///   `kayfabe_device::inittables::InitTablePolicy`, which answers six of them. Controls
///   reach the object model through the device chain's own links or not at all until
///   somebody measures that they must.
/// - **`DUP_OBJECT`.** `translate_dup` exists and works; no boot has produced one. Serving
///   a verb nothing has sent is the *"is it already done / is it actually needed"* trap in
///   its other direction, and the cost of being wrong is one named refusal on the next
///   boot — which is exactly how this rung was found. It is [`OBJECT_VERBS`]'s most likely
///   next member and it is not one today.
///
/// # ★ Ownership
///
/// Owns its [`Gpu`] because a `CommandPolicy` is installed as `Box<dyn CommandPolicy>`
/// (`'static`), and there is nothing else in this port holding the object model to borrow
/// it from. ⚠ That is a **stage fact, not a design**: `GraphPolicy`'s doc explains why
/// borrowing is right the moment the doorbell path and the projection also want it, and
/// the day either exists this type hands its `Gpu` to whatever owns them.
pub struct ObjectPolicy {
    bridge: Bridge,
    /// ★★★ **E2** — the object model as a **port**, not as a `Gpu` by value. See
    /// [`ObjectModel`] for why the sentence two paragraphs up ("that is a stage fact, not a
    /// design") came due. [`ObjectPolicy::new`] still takes a `Gpu` and boxes it, so every
    /// existing call site is unchanged; [`ObjectPolicy::over`] is the shell's constructor.
    gpu: Box<dyn ObjectModel>,
    /// ★★★ E1 — the isolate plane's health, republished after every delivered command so
    /// the composition root can read it after this policy is boxed. Same handle shape,
    /// same reason, as [`SharedRefusalCensus`]; see
    /// [`kayfabe_core::gpu::SharedIsolateCensus`].
    isolates: kayfabe_core::gpu::SharedIsolateCensus,
}

/// The RPC functions [`ObjectPolicy`] claims. **Closed, and public, so a test can quantify
/// over it rather than restate it** — `gates_quantified_over_a_list`'s rule: a gate that
/// spells its own universe is a gate that shrinks silently.
pub const OBJECT_VERBS: &[kayfabe_gsp::RpcFunction] = &[
    kayfabe_gsp::RpcFunction::RmAlloc,
    kayfabe_gsp::RpcFunction::Free,
];

/// ★★★ **#177** — the `RpcFunction::RmControl` command ids this policy claims, and the
/// **only** ones.
///
/// # Why a second list instead of putting `RmControl` in [`OBJECT_VERBS`]
///
/// `RmControl` is one RPC function carrying hundreds of different commands. Adding it to
/// `OBJECT_VERBS` would make this policy answer **every** control in the port — including
/// every one nobody has decided about — and because `PolicyChain::respond` is a `find_map`,
/// the first `Some` terminates the chain. The `UnservicedLedger` sits at the end of that
/// chain, so the cost would be exact and total: the ledger goes permanently silent, and the
/// ledger is this port's primary instrument for *"what has the guest asked for that we do
/// not answer"*. `GraphPolicy` carries the same warning for the same reason
/// (`kayfabe_device::served_chain`).
///
/// ⊘ So the claim is by **command id**, quantified over this list, and the list is public
/// so a test asks the type rather than restating it (`gates_quantified_over_a_list`).
pub const OBJECT_CONTROLS: &[u32] = &[kayfabe_abi::submit::NVA06F_CTRL_CMD_GPFIFO_SCHEDULE];

impl ObjectPolicy {
    /// A policy that owns `gpu`, decoding with `abi`, serving a `guest_os`.
    #[must_use]
    pub fn new(abi: &DriverAbiTable, guest_os: GuestOs, gpu: Gpu) -> ObjectPolicy {
        ObjectPolicy::with_limits(abi, guest_os, gpu, ReasmLimits::default())
    }

    /// As [`ObjectPolicy::new`], with explicit continuation bounds.
    #[must_use]
    pub fn with_limits(
        abi: &DriverAbiTable,
        guest_os: GuestOs,
        gpu: Gpu,
        limits: ReasmLimits,
    ) -> ObjectPolicy {
        ObjectPolicy::over(abi, guest_os, Box::new(gpu), limits)
    }

    /// ★★★ **E2** — a policy over an object model somebody else owns.
    ///
    /// The composition root's constructor: the model is a shell that also serves the
    /// doorbell path, so the `Gpu` cannot live here. See [`ObjectModel`] for the argument
    /// and `execution_plane_increments.md` E2 for the increment that needed it.
    ///
    /// ⊘ Not a fallback for [`ObjectPolicy::new`] and not a default: a caller that holds a
    /// bare `Gpu` should keep using `new`, because `Box<dyn ObjectModel>` erases the
    /// `as_gpu` answer for nobody's benefit.
    #[must_use]
    pub fn over(
        abi: &DriverAbiTable,
        guest_os: GuestOs,
        gpu: Box<dyn ObjectModel>,
        limits: ReasmLimits,
    ) -> ObjectPolicy {
        let isolates = kayfabe_core::gpu::SharedIsolateCensus::new();
        // ★ Published ONCE at construction, before any command, so the handle is never
        // "empty because nothing has happened yet" in a way a reader could mistake for
        // "empty because the plane is fine". After E0b a freshly realized device really
        // does hold zero isolates, and this is the value that says so.
        gpu.publish_isolate_census(&isolates);
        ObjectPolicy {
            bridge: Bridge::new(*abi, guest_os, limits),
            gpu,
            isolates,
        }
    }

    /// Whether this policy claims `f` — the predicate `respond` gates on, exposed so the
    /// chain-composition test asks the type rather than a copy of its list.
    #[must_use]
    pub fn claims(f: kayfabe_gsp::RpcFunction) -> bool {
        OBJECT_VERBS.contains(&f)
    }

    /// The refusal census, as a point-in-time snapshot — see [`RefusalCensus`].
    ///
    /// ★ By value since the census became a [`SharedRefusalCensus`]: the store is behind a
    /// handle the composition root also holds, so there is no `&` to hand out that would
    /// still be a single owner's. A snapshot is what every reader wanted anyway.
    #[must_use]
    pub fn census(&self) -> RefusalCensus {
        self.bridge.census.snapshot()
    }

    /// The census as a **handle**, for the composition root that must keep reading it after
    /// boxing this policy — see [`SharedRefusalCensus`].
    #[must_use]
    pub fn refusal_census(&self) -> SharedRefusalCensus {
        self.bridge.census.clone()
    }

    /// ★ §8.2.2 — the GPFIFO-ring census as a **handle**, for the same reason
    /// [`Self::refusal_census`] is one: the composition root keeps reading it after this
    /// policy is boxed. See [`SharedRingCensus`].
    #[must_use]
    pub fn ring_census(&self) -> SharedRingCensus {
        self.bridge.rings.clone()
    }

    /// How many commands produced an `RmEvent` the graph **accepted** — the non-vacuity
    /// instrument for every "no refusals" claim about a boot.
    #[must_use]
    pub fn applied(&self) -> u64 {
        self.bridge.applied
    }

    /// The object model, for a caller that wants to project it — if it **is** a plain
    /// [`Gpu`].
    ///
    /// ⊘ `None` since **E2**, and the `Option` is the honest half of that change: a policy
    /// built with [`ObjectPolicy::over`] declares into a shell whose state is behind ranked
    /// locks, and there is no `&Gpu` in existence to return. A method that panicked, or
    /// that handed back an empty stand-in, would let a projection read as *"the guest
    /// declared nothing"* when it means *"you asked the wrong object".*
    #[must_use]
    pub fn gpu(&self) -> Option<&Gpu> {
        self.gpu.as_gpu()
    }

    /// The object model **mutably**, if it is a plain [`Gpu`] — the `&mut` twin of
    /// [`Self::gpu`], with the same `None` and for the same reason.
    ///
    /// ⊘ Test-facing. The shipped composition root builds this policy with
    /// [`ObjectPolicy::over`] and gets `None`; a caller that needs to act on a sharded
    /// shell must go through the shell's own ranked verbs, not through here.
    #[must_use]
    pub fn gpu_mut(&mut self) -> Option<&mut Gpu> {
        self.gpu.as_gpu_mut()
    }

    /// Translate one command and apply whatever it declared — the `Result` form, for a
    /// test that asserts an exact [`BridgeRefusal`] variant.
    ///
    /// ⊘ Unlike [`CommandPolicy::respond`] this does **not** gate on [`OBJECT_VERBS`]: it
    /// is the bridge, not the chain link. A test that wants to know what the *chain* does
    /// with a function must call `respond`.
    ///
    /// # Errors
    ///
    /// [`BridgeRefusal`], by variant.
    pub fn deliver(&mut self, cmd: &RpcCommand) -> Result<Translation, BridgeRefusal> {
        let out = self.bridge.deliver(&mut *self.gpu, cmd);
        // ★★★ E1/E0b — republished on BOTH arms, deliberately. A refused command can
        // still be the one that materialized an isolate through an earlier accepted
        // event's proc set, and — more to the point — a report that only refreshed on
        // success would show the last *good* state while the plane was failing, which is
        // the exact shape of "the instrument agreed with the claim it was checking".
        self.gpu.publish_isolate_census(&self.isolates);
        out
    }

    /// The isolate census as a **handle**, for the composition root that must keep reading
    /// it after boxing this policy — see [`kayfabe_core::gpu::SharedIsolateCensus`].
    #[must_use]
    pub fn isolate_census(&self) -> kayfabe_core::gpu::SharedIsolateCensus {
        self.isolates.clone()
    }

    /// ★★★ **#177** — answer `NVA06F_CTRL_CMD_GPFIFO_SCHEDULE`, and **only** it.
    ///
    /// The three answers, and why each is the one it is:
    ///
    /// - **`None`** — any control not in [`OBJECT_CONTROLS`], and any payload this policy
    ///   cannot even classify as a control. The chain (and therefore the unserviced
    ///   ledger) is untouched. ⊘ This is the arm that must stay large: it is what keeps
    ///   the ledger honest.
    /// - **`NV_OK` with the request's own three params bytes** — the transition was
    ///   performed. The body is what a real GA106's GSP sends
    ///   (`kayfabe_abi::submit::encode_gpfifo_schedule`), not what the C's empty capture
    ///   row implies.
    /// - **`GPFIFO_SCHEDULE_REFUSED_STATUS` (`NV_ERR_INVALID_STATE`) with an empty body**
    ///   — the decode or the route said no. ⚠ Never `NV_ERR_NOT_SUPPORTED`: that is the
    ///   FSM's signature for *"nobody claimed this"*, and the guest prints the raw hex, so
    ///   reusing it would erase the difference between "refused" and "unimplemented" in
    ///   the only place anyone reads it.
    fn respond_control(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        let req = self.bridge.abi.decode_rpc_control(&cmd.payload).ok()?;
        if !OBJECT_CONTROLS.contains(&req.cmd) {
            return None;
        }
        let refuse = || {
            Some(Reply {
                rpc_result: kayfabe_abi::submit::GPFIFO_SCHEDULE_REFUSED_STATUS,
                body: Vec::new(),
            })
        };
        // The guest's own two assertions about its params, both checked — the same pair
        // `InitTablePolicy` checks, for the same reason.
        let want = kayfabe_abi::submit::GpfifoScheduleParams::SIZE;
        if kayfabe_abi::rpc_params_are_serialized(req.rmapi_rpc_flags)
            || req.params_size as usize != want
            || cmd.payload.len() < req.params_at + want
        {
            return refuse();
        }
        let Ok(params) = kayfabe_abi::submit::decode_gpfifo_schedule(
            &cmd.payload[req.params_at..req.params_at + want],
        ) else {
            return refuse();
        };
        let ack = self.gpu.schedule_channel(
            kayfabe_arch::ids::HClient(req.client),
            kayfabe_arch::ids::HObject(req.object),
            params.b_enable != 0,
        );
        self.gpu.publish_isolate_census(&self.isolates);
        if ack.is_err() {
            return refuse();
        }
        // ★ The reply carries the request's params back, because the GSP transport copies
        // the reply's params over the caller's own struct whenever `paramsSize != 0`
        // (`ogkm-580: rpc.c:11085-11090`). A zero-filled body would clear the caller's
        // `bEnable` behind its back.
        let mut body = cmd.payload.clone();
        let params_out = kayfabe_abi::submit::encode_gpfifo_schedule(&params);
        body[req.params_at..req.params_at + want].copy_from_slice(&params_out);
        Some(Reply {
            rpc_result: 0, // NV_OK
            body,
        })
    }
}

impl core::fmt::Debug for ObjectPolicy {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.bridge.debug_as("ObjectPolicy", f)
    }
}

impl CommandPolicy for ObjectPolicy {
    /// Answer one command **if it is an object-declaring verb**, else decline.
    ///
    /// The accepted/refused shapes are [`GraphPolicy`]'s, verbatim and for its reasons —
    /// an acknowledgement carrying the request's own body, or
    /// [`BridgeRefusal::rpc_result`] in the **envelope** with an empty body that
    /// `RpcCommand::reply` zero-fills. Read that method's docs; they are the argument, and
    /// duplicating it here would let the two drift.
    ///
    /// ★★ `None` for anything not in [`OBJECT_VERBS`] — which in a chain means *"ask the
    /// next link"*, and at the end of a chain means the FSM's own named refusal
    /// (`kayfabe_gsp::GspFsm::answer`). Both are correct answers for a verb this port does
    /// not model; an `NV_OK` would not be.
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        if cmd.function == kayfabe_gsp::RpcFunction::RmControl {
            // ★★★ #177 — the narrow control claim. `None` for every control not in
            // `OBJECT_CONTROLS`, so the chain and the unserviced ledger are untouched.
            return self.respond_control(cmd);
        }
        if !ObjectPolicy::claims(cmd.function) {
            return None;
        }
        match self.deliver(cmd) {
            Ok(_) => Some(Reply {
                rpc_result: 0, // NV_OK
                body: cmd.payload.clone(),
            }),
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
// ★ `ObjectPolicy` OWNS its `Gpu`, so this asserts something `GraphPolicy`'s bound cannot:
// that the whole object model — spine, procs, isolate factory — is `Send` when it is moved
// into a policy the FSM holds across vCPU threads. The `Gpu` behind a `&mut` was already
// somebody else's problem; here it is this type's.
kayfabe_util::assert_send!(ObjectPolicy);
