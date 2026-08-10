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

use std::collections::{BTreeMap, BTreeSet};

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
pub struct RefusalCensus {
    counts: BTreeMap<FaultTag, usize>,
    ids: BTreeMap<FaultTag, BTreeSet<u32>>,
}

/// ★★★★ **§16.56 — how many distinct ids are kept per tag.**
///
/// ⊘ A cap rather than a full set, and it is the guest-reachable-allocation rule this
/// module already states for the map itself: the tag set is closed and cannot grow with
/// traffic, but the `hClass` a guest sends is a **guest-supplied value** and an uncapped
/// set of them is an unbounded allocation a hostile guest drives directly.
///
/// ★ The cap is safe *because the count is not capped*: `RefusalCensus::of` still reports
/// every refusal, so a saturated id list can never read as a complete one — the report
/// prints `n` ids beside a larger count, which is a visible truncation rather than a silent
/// one (`a_saturated_instrument_looks_exactly_like_absence`).
pub const REFUSAL_DETAIL_CAP: usize = 8;

impl RefusalCensus {
    /// How many refusals carried `tag`.
    #[must_use]
    pub fn of(&self, tag: FaultTag) -> usize {
        self.counts.get(&tag).copied().unwrap_or(0)
    }

    /// Every tag seen, with its count, in tag order.
    pub fn tags(&self) -> impl Iterator<Item = (FaultTag, usize)> + '_ {
        self.counts.iter().map(|(&t, &n)| (t, n))
    }

    /// ★★★★ **§16.56 — the ids refused under `tag`**, ascending, at most
    /// [`REFUSAL_DETAIL_CAP`] of them. Empty for tags that are not about an id.
    ///
    /// ⊘ This is the answer to *"which class did we refuse?"*, a question no `grep` over
    /// any committed device log could answer before it existed — see
    /// [`crate::BridgeRefusal::fault_id`] for the measurement.
    pub fn ids(&self, tag: FaultTag) -> impl Iterator<Item = u32> + '_ {
        self.ids.get(&tag).into_iter().flatten().copied()
    }

    /// Total refusals, across every tag.
    #[must_use]
    pub fn total(&self) -> usize {
        self.counts.values().sum()
    }

    /// Nothing has been refused.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.counts.is_empty()
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
/// ★★★★ **§16.40 — the first `GPU_PROMOTE_CTX` refusal, latched WITH the address plane's
/// state at the moment it was refused.**
///
/// # Why a sentence and not a counter
///
/// [`SharedRefusalCensus`] counts refusals by [`FaultTag`], which is the right shape for
/// an invariant ("this never happens") and the wrong shape for *this* question.
/// `[measured 2026-08-09, boot `s35_03a7e10_dup`]` the census printed
/// `PromoteFault::ContextVasUndeclared x1` — a true row that names a VA space and
/// **cannot say which one**, because the variant's payload (`client`, `object`) is
/// discarded at the tag. Three rungs in a row then reasoned about *which* VA space from
/// cross-boot correspondence rather than same-boot identity.
///
/// # ★★★ And why it is latched HERE rather than printed at teardown
///
/// The interesting facts are all **lifetime** facts: which channels existed, what each one
/// named, and whether that name had a page-directory base **when the promotion was
/// refused**. The device's exit notifier runs after the CUDA process has exited and its
/// channels are freed, so the same call there returns `NO-LIVE-CHANNELS` — a true sentence
/// about an instant nobody asked about (`a_correct_capture_can_answer_the_wrong_question`).
///
/// # ⊘ FIRST, and only the first
///
/// The same bound as [`crate::KayfabeDoorbellRefusal`]'s: a guest that can drive N
/// refusals must not be able to drive N allocations. The count of *all* promote refusals
/// is already in the tag census, so nothing is lost by keeping one sentence — and the
/// first is the one that matters, because every later one is downstream of it.
#[derive(Debug, Clone, Default)]
pub struct SharedPromoteDiag(std::sync::Arc<std::sync::Mutex<BTreeMap<FaultTag, String>>>);

impl SharedPromoteDiag {
    /// A fresh, unlatched diagnosis.
    #[must_use]
    pub fn new() -> SharedPromoteDiag {
        SharedPromoteDiag::default()
    }

    /// The latched sentences, one per [`FaultTag`], in tag order.
    ///
    /// ⊘ Empty is a **finding**: it means either that every promotion was served or that
    /// none arrived, and the tag census discriminates those two. It never means "the
    /// instrument was off".
    #[must_use]
    pub fn rows(&self) -> Vec<(FaultTag, String)> {
        let g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        g.iter().map(|(t, s)| (*t, s.clone())).collect()
    }

    /// ★★★★ §16.46 — latch `s` for `tag`, **replacing** whatever was there.
    ///
    /// The opposite rule from [`Self::latch`], for a case where the opposite rule is the
    /// correct one. `latch`'s tags are *refusals*, where the first is the one that matters
    /// because every later one is downstream of it. This one's tag is an **acceptance**,
    /// and there the last is the one that matters: promotions arrive in the order
    /// `cuCtxCreate` builds the context, so the final one is taken at the deepest point
    /// the guest reached.
    ///
    /// ⊘ **This exists because closing a bug BLINDED the instrument that measured it.**
    /// `[measured 2026-08-09]` — `s38` reported `census[14 chans, 4 outcomes]` and `s39`,
    /// with the promote guard fixed, reported `census[2 chans, 2 outcomes]`. Nothing
    /// broke: the 14-channel snapshot rode on the `ForeignContextObject` refusal, and that
    /// refusal is what the rung deleted. ★★★★★ That is the SECOND time in two rungs — the
    /// same census was previously reachable only from inside a **doorbell** refusal and
    /// went dark when the doorbell plane started succeeding. ⇒ the rule is not *"look for
    /// disabled instruments"*: **an instrument hung off a refusal path has its own
    /// deletion scheduled by the fix it exists to guide.** Hanging this one off the
    /// *success* path is what makes it survive its own success.
    ///
    /// ⊘ Latching at teardown is not the alternative and never was:
    /// `Gpu::vas_census_string`'s own doc records that by the time the exit notifier runs
    /// the CUDA process has exited and its channels are freed, so a teardown census
    /// returns `NO-LIVE-CHANNELS` — *"a true sentence about the wrong instant"*.
    fn latch_last(&self, tag: FaultTag, s: String) {
        let mut g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        g.insert(tag, s);
    }

    /// Latch `s` for `tag`, if that tag has nothing latched yet.
    ///
    /// ★★★★ **Per TAG, and the first version of this was per BOOT — which measured the
    /// wrong event.** `[measured 2026-08-09, boot `s36_3a0146c_vascensus`]`: a
    /// boot-global "first refusal" latched
    /// `PromoteFault::UnknownContextObject { client: 0xc1d00008, object: 0x31415900 }` —
    /// **kernel RM's** promotion, refused long before `cup2` ran, with a census of the two
    /// CE channels that existed at that instant. The refusal the rung is *about*
    /// (`ContextVasUndeclared`, `x1` in the same boot) was never latched, because it was
    /// not first.
    ///
    /// ⊘ The `doorbell_refusal` precedent that suggested "first" does not transfer: there,
    /// the flood is *identical rings from one guest*, so first is representative. Here the
    /// boot contains **several distinct refusals from different callers**, and first is
    /// simply the earliest — which is the one nobody asked about. Keying on the tag makes
    /// each *kind* of refusal carry its own first, which is what the census counts anyway.
    ///
    /// ★ Still bounded and still guest-independent: [`FaultTag`] comes from a fixed finite
    /// set (`kayfabe_core::promote::PromoteFault` has ten variants), so a hostile guest can
    /// drive the *counts* but never the number of rows.
    fn latch(&self, tag: FaultTag, s: String) {
        let mut g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        g.entry(tag).or_insert(s);
    }
}

/// ★★★★★ §16.48 — **the CUMULATIVE per-`buffer_id` promotion tally**, across every
/// promotion of the run.
///
/// ⊘ **This exists because `latch_last` answers a question about ONE promotion and the
/// two-phase join is a question about ALL of them.** `[measured 2026-08-09, boot
/// `s40_4733730_acceptcensus`]` the accepted row read `declined.promote_only=10
/// declined.initialize_only=0` — and §16.47.4 had to warn, in the same commit that
/// produced it, that the second number describes **the last promotion only** and *"must
/// not be read as 'no promotion ever declared a physical buffer'"*. Eleven promotions were
/// accepted and ten of them left no trace at all.
///
/// ★ So the join cannot be scored from a last-wins row no matter how many fields that row
/// grows: *"does phase 1 ever arrive?"* is a statement about the whole run, and no single
/// promotion can answer it. This accumulates one row per `buffer_id` — how many
/// **physical** halves, how many **virtual** halves, and how many already-**complete**
/// entries that id was ever declared with — so the two phases can be counted against each
/// other instead of assumed to pair up.
///
/// ⊘ It rides the existing [`SharedPromoteDiag`] slots as one more rendered row, so it
/// costs **no ABI change**: `PROMOTE_DIAG_SLOTS` is 4 and `s40` used 2.
#[derive(Clone, Default)]
pub struct SharedPromoteTally {
    per_id: std::sync::Arc<std::sync::Mutex<BTreeMap<u16, [u32; 3]>>>,
    /// ★★★★★ §16.51 — **the CUMULATIVE join outcome**, `[bound, joined, joined_global,
    /// already, globals_added]`, summed over every accepted promotion.
    ///
    /// # ⊘ Why the per-promotion counters were not enough, MEASURED
    ///
    /// `[measured 2026-08-09, rev 21f967b, boot s42_21f967b_gpuscope]`: the join **fired**
    /// — `orphans(awaiting_phys)` fell `10 → 9` and `already` rose `0 → 1` against `s41b`
    /// — and the `ACCEPTED` row still printed `joined_global=0`. That row is latched
    /// **last-wins**, and the cross-address-space join is a **one-shot event that happens
    /// early**: by the last promotion the range it produced is already bound, so it counts
    /// as `already` and the counter that names the mechanism reads zero.
    ///
    /// ★ So the counter was right, on the success path, unconditional — and the **latch**
    /// destroyed it anyway. A new failure class, and a narrow one: *an unconditional
    /// success-path counter on a LAST-WINS latch measures only the last occurrence, and a
    /// one-shot event is invisible at the end of the run.* The falsifier's own outcome-P
    /// row was written as `joined_global>0` and was therefore **unscoreable from the line
    /// it named** — `a_prediction_with_no_readout_was_never_a_test`, one level in.
    ///
    /// ⇒ this accumulates instead, beside the per-id row that already had to exist for
    /// exactly the same reason.
    totals: std::sync::Arc<std::sync::Mutex<[u64; 5]>>,
}

impl SharedPromoteTally {
    /// Fold one promotion's declarations in, keyed by `buffer_id`.
    fn record(&self, p: &kayfabe_core::promote::CtxPromotion) {
        use kayfabe_core::promote::PromoteHalf;
        let mut g = self.per_id.lock().unwrap_or_else(|e| e.into_inner());
        for h in &p.halves {
            let slot = g.entry(h.buffer_id()).or_default();
            match h {
                PromoteHalf::Physical { .. } => slot[0] += 1,
                PromoteHalf::Virtual { .. } => slot[1] += 1,
            }
        }
        for r in &p.ranges {
            g.entry(r.buffer_id).or_default()[2] += 1;
        }
    }

    /// Render as `{bid=0x1 phys=1 va=1 complete=0}` per id, space-joined.
    ///
    /// ★ `buffer_id` is printed in hex and never as a decoded name: the ids are
    /// `NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_*`, and a wrong name on a right number is
    /// much harder to disbelieve than a bare number.
    ///
    /// ★ Bounded and guest-independent for [`RefusalCensus`]'s reason: `buffer_id` is a
    /// 16-bit wire field, but the ids RM emits come from a fixed enum and the row count is
    /// bounded by how many distinct ones ever appear — the guest drives the *counts*, and
    /// the shim's own `[CLIPPED …]` stamp bounds the sentence.
    /// Fold one ACCEPTED promotion's join outcome into the cumulative totals.
    ///
    /// ⊘ Separate from [`Self::record`] on purpose: `record` counts what the **wire
    /// declared** and runs for every accepted promotion's params; this counts what the
    /// **join did**. Merging them would make "ten VA halves arrived" and "one of them
    /// bound" the same number again, which is the confusion `PromoteDeclined` exists to
    /// prevent one layer down.
    fn record_join(&self, j: &kayfabe_core::promote::PromoteJoin) {
        let mut t = self.totals.lock().unwrap_or_else(|e| e.into_inner());
        t[0] += u64::from(j.bound);
        t[1] += u64::from(j.joined);
        t[2] += u64::from(j.joined_global);
        t[3] += u64::from(j.already);
        t[4] += u64::from(j.globals_added);
    }

    fn render(&self) -> String {
        let g = self.per_id.lock().unwrap_or_else(|e| e.into_inner());
        let t = *self.totals.lock().unwrap_or_else(|e| e.into_inner());
        // ★ The cumulative row is emitted even when no id was ever declared: `0 0 0 0 0`
        // is a reading ("no promotion was ever accepted"), and its ABSENCE would be
        // indistinguishable from the render having been skipped.
        let totals = format!(
            " || CUMULATIVE bound={} joined={} joined_global={} already={} globals_added={}",
            t[0], t[1], t[2], t[3], t[4]
        );
        if g.is_empty() {
            return format!("no buffer_id ever declared{totals}");
        }
        let mut out = String::new();
        for (bid, [phys, va, complete]) in g.iter() {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(&format!(
                "{{bid={bid:#x} phys={phys} va={va} complete={complete}}}"
            ));
        }
        out.push_str(&totals);
        out
    }
}

/// The refusal census as a **handle** — see the type's own docs below for why ownership
/// was the obstruction. ⊘ Its inner value is a whole [`RefusalCensus`] since §16.56, not a
/// bare count map: the ids live beside the counts and must snapshot atomically with them.
#[derive(Debug, Clone, Default)]
pub struct SharedRefusalCensus(std::sync::Arc<std::sync::Mutex<RefusalCensus>>);

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
        g.clone()
    }

    fn record(&self, r: &BridgeRefusal) {
        let mut g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let tag = r.fault_tag();
        *g.counts.entry(tag).or_default() += 1;
        // ★★★★ §16.56 — the id beside the tag. ⊘ The COUNT above is incremented
        // unconditionally and the id set below is capped, so truncation can never subtract
        // from the census; it can only stop naming.
        if let Some(id) = r.fault_id() {
            let set = g.ids.entry(tag).or_default();
            if set.len() < REFUSAL_DETAIL_CAP {
                set.insert(id);
            }
        }
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

/// ★★★★ **§16.71 — one declared ring, WITH THE OBJECT THAT DECLARED IT.**
///
/// # ⊘ The row that did not exist, and what it cost
///
/// [`RingCensus`] keeps a count and the **first** non-zero address. `[measured 2026-08-10,
/// boots `w205_227194f_ctl` / `_real`]` both arms printed
/// *"first 0x0000000120064000 (4096 entries)"* while the real arm's forwarded doorbells
/// declared `0x420064000` — so a single boot demonstrably held **both** addresses, and the
/// census could name neither's owner. §16.70.6 was therefore forced to record *"two ring
/// addresses for one token"* as an open question, when the fact needed to close it (are
/// they two channels or one?) was already passing through this recorder and being thrown
/// away.
///
/// ⊘ A count plus a sample of one is not a census of a population; it is the population's
/// first element wearing a census's name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RingRow {
    /// The client namespace the channel was allocated in.
    pub client: u32,
    /// The channel object's own handle.
    pub handle: u32,
    /// `gpFifoOffset` as declared — **including zero**, which is a real declaration.
    pub va: u64,
    /// `gpFifoEntries` as declared.
    pub entries: u32,
}

/// How many [`RingRow`]s the roster keeps.
///
/// ⊘ A cap, and the roster **says when it hit it** — a saturated instrument that looks like
/// a complete one is this campaign's `a_saturated_instrument_looks_exactly_like_absence`.
/// 64 is above every declaration count either `w205` arm reached (26 and 6).
const RING_ROSTER_MAX: usize = 64;

/// [`RingCensus`] as a handle the composition root keeps after handing the policy away —
/// same shape, and the same ownership argument, as [`SharedRefusalCensus`].
#[derive(Debug, Clone, Default)]
pub struct SharedRingCensus(
    std::sync::Arc<std::sync::Mutex<RingCensus>>,
    /// ★ The roster, beside the tally rather than inside it: [`RingCensus`] is `Copy` and
    /// destructured **without `..`** at the C boundary on purpose, and a `Vec` field would
    /// have forced that gate open for a report the C shell does not read.
    std::sync::Arc<std::sync::Mutex<(Vec<RingRow>, u64)>>,
);

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

    /// ★★★ **The roster of declared rings, with owners** — and the number of declarations
    /// that did **not** fit, which is returned rather than hidden.
    ///
    /// ⊘ `(rows, dropped)`. A non-zero `dropped` means this list is a PREFIX of the
    /// population and every conclusion drawn from its absences is void.
    #[must_use]
    pub fn roster(&self) -> (Vec<RingRow>, u64) {
        let g = self.1.lock().unwrap_or_else(|e| e.into_inner());
        g.clone()
    }

    /// Record one channel alloc's declared ring, and **who declared it**.
    fn record(&self, client: u32, handle: u32, r: kayfabe_core::rmgraph::GpFifoRing) {
        let mut g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        g.declarations = g.declarations.saturating_add(1);
        if r.va != 0 {
            g.nonzero = g.nonzero.saturating_add(1);
            if g.first_nonzero.is_none() {
                g.first_nonzero = Some((r.va, r.entries));
            }
        }
        drop(g);
        let mut roster = self.1.lock().unwrap_or_else(|e| e.into_inner());
        // ⊘ Every declaration, zero-VA ones included: the golden-context channel's
        // deliberate `gpFifoOffset = 0` (`ogkm-580: kernel_graphics.c:2420-2424`) is a row
        // a reader must be able to see, not an entry to filter out on our judgement.
        if roster.0.len() < RING_ROSTER_MAX {
            roster.0.push(RingRow {
                client,
                handle,
                va: r.va,
                entries: r.entries,
            });
        } else {
            roster.1 = roster.1.saturating_add(1);
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

    /// ★★★★ **§16.56** — perform the guest's `NVA06C_CTRL_CMD_GPFIFO_SCHEDULE` (the
    /// **TSG** form, `0xa06c0101`) — the wall `s44` named.
    ///
    /// ⊘ A separate method rather than a flag on [`Self::schedule_channel`]: the two take
    /// different objects (a channel group vs one channel), fan out differently, and refuse
    /// with different vocabularies. Conflating them is the mistake
    /// `kayfabe_abi::submit`'s own doc calls out — *"same requirement, three different
    /// objects"*.
    ///
    /// # Errors
    /// [`kayfabe_core::gpu::ScheduleGroupFault`], by variant.
    fn schedule_group(
        &mut self,
        client: kayfabe_arch::ids::HClient,
        object: kayfabe_arch::ids::HObject,
        enable: bool,
    ) -> Result<kayfabe_core::gpu::ScheduleGroupAck, kayfabe_core::gpu::ScheduleGroupFault>;

    /// ★★★★ **§16.59** — verify the guest's
    /// `NV2080_CTRL_CMD_GR_SET_CTXSW_PREEMPTION_MODE` (`0x20801210`) — the wall `s45` and
    /// `s46` both measured at record 331.
    ///
    /// `h_channel` is the request's own `hChannel` **field**, not the control's `hObject`
    /// (which is the subdevice). `[measured]` it carries a **TSG** handle on the
    /// `cuCtxCreate` path.
    ///
    /// ⚠ **A trait method rather than a call through [`Self::as_gpu`]**, for
    /// [`Self::vas_census`]'s measured reason: the shipped composition root installs a
    /// sharded shell whose `as_gpu` returns `None` **by design**, so an arm built on
    /// `as_gpu` refuses on every real boot while passing every test that composes a bare
    /// [`Gpu`] (`skipped_oracle_kills_the_guard`). This arm was written that way first.
    ///
    /// # Errors
    /// [`kayfabe_core::gpu::CtxswPreemptionFault`], by variant.
    fn set_ctxsw_preemption_mode(
        &self,
        client: kayfabe_arch::ids::HClient,
        h_channel: kayfabe_arch::ids::HObject,
    ) -> Result<kayfabe_core::gpu::CtxswPreemptionAck, kayfabe_core::gpu::CtxswPreemptionFault>;

    /// ★★★ **E9/§13.6** — perform the guest's `NVA06F_CTRL_CMD_BIND`.
    ///
    /// `rm_engine_type` is in **RM engine space**: the policy converts the wire ordinal
    /// and checks it against the device's advertised set *before* calling this, so the
    /// model only ever answers the channel question.
    ///
    /// # Errors
    /// [`kayfabe_core::gpu::BindFault`], by variant.
    fn bind_channel(
        &mut self,
        client: kayfabe_arch::ids::HClient,
        object: kayfabe_arch::ids::HObject,
        rm_engine_type: u32,
    ) -> Result<kayfabe_core::gpu::BindAck, kayfabe_core::gpu::BindFault>;

    /// ★★★★ §16.40 — **the live VA-space census, in the model's own locking discipline.**
    ///
    /// ⊘⊘ This is a trait method rather than a call through [`Self::as_gpu`], and the
    /// difference was measured before it was written: the **shipped** composition root
    /// installs a sharded shell whose `as_gpu` returns `None` by design, so a diagnosis
    /// built on `as_gpu` would have printed *"no whole `Gpu`"* on every real boot while
    /// passing every test that composes a bare [`Gpu`]. That is
    /// `skipped_oracle_kills_the_guard` — an instrument that is green in the harness and
    /// blind on the bench.
    ///
    /// Implementors format through `kayfabe_core::gpu::format_vas_census` so the two
    /// sources (a bare `Gpu`'s procs; the shell's lock-ranked walk) cannot drift in shape.
    ///
    /// `mark` is the channel the caller's refusal is about, or `None` when it names a
    /// handle rather than a `ChanId`.
    fn vas_census(&self, mark: Option<kayfabe_core::ChanId>) -> String;

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

    fn schedule_group(
        &mut self,
        client: kayfabe_arch::ids::HClient,
        object: kayfabe_arch::ids::HObject,
        enable: bool,
    ) -> Result<kayfabe_core::gpu::ScheduleGroupAck, kayfabe_core::gpu::ScheduleGroupFault> {
        Gpu::schedule_group(self, client, object, enable)
    }

    fn set_ctxsw_preemption_mode(
        &self,
        client: kayfabe_arch::ids::HClient,
        h_channel: kayfabe_arch::ids::HObject,
    ) -> Result<kayfabe_core::gpu::CtxswPreemptionAck, kayfabe_core::gpu::CtxswPreemptionFault>
    {
        Gpu::set_ctxsw_preemption_mode(self, client, h_channel)
    }

    fn bind_channel(
        &mut self,
        client: kayfabe_arch::ids::HClient,
        object: kayfabe_arch::ids::HObject,
        rm_engine_type: u32,
    ) -> Result<kayfabe_core::gpu::BindAck, kayfabe_core::gpu::BindFault> {
        Gpu::bind_channel(self, client, object, rm_engine_type)
    }

    fn vas_census(&self, mark: Option<kayfabe_core::ChanId>) -> String {
        Gpu::vas_census_string(self, mark)
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
    /// ★★★★ §16.40 — the FIRST `GPU_PROMOTE_CTX` refusal, with its handles and the live
    /// VA-space census taken **at that instant**. See [`SharedPromoteDiag`].
    promote_diag: SharedPromoteDiag,
    /// ★★★★★ §16.48 — the cumulative per-`buffer_id` tally. See [`SharedPromoteTally`]
    /// for why a last-wins row cannot score a two-phase join.
    promote_tally: SharedPromoteTally,
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
            promote_diag: SharedPromoteDiag::default(),
            promote_tally: SharedPromoteTally::default(),
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

    /// ★★★★ §16.40 — the first refused `GPU_PROMOTE_CTX`, with the address plane's state
    /// as it stood at that instant. See [`SharedPromoteDiag`] for why it is a handle and
    /// why the sample is taken at the refusal rather than at teardown.
    #[must_use]
    pub fn promote_diag(&self) -> SharedPromoteDiag {
        self.bridge.promote_diag.clone()
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
        // ★★★★ §16.46 — cloned out ahead of the chain for exactly `rings`'s reason (it is
        // an `Arc`), so the acceptance latch below borrows this handle and not `self`.
        let diag = self.promote_diag.clone();
        // ★★★★★ §16.48 — cloned out for `diag`'s reason (it is an `Arc`). Unlike the
        // acceptance row, the tally is CUMULATIVE, so being overwritten costs nothing:
        // each promotion folds itself in and re-renders, and the surviving render is
        // therefore the whole run rather than the last event.
        let tally = self.promote_tally.clone();
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
                        client,
                        handle,
                        facts:
                            kayfabe_core::rmgraph::AllocFacts {
                                gp_fifo_ring: Some(r),
                                ..
                            },
                        ..
                    } = ev
                    {
                        // ★★★★ §16.71 — the OWNER, recorded with the address. Both were
                        // right here and only the address was kept; see [`RingRow`] for
                        // the boot that then could not say whether two ring addresses
                        // belonged to two channels or one.
                        rings.record(client.0, handle.0, r);
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
                Translation::CtxPromotion(p) => match gpu.promote_ctx(&p) {
                    Ok(join) => {
                        // ★★★★ §16.46 — THE ACCEPTED promotion's own accounting, plus the
                        // census, latched LAST-wins. Two gaps closed by one latch:
                        //
                        //  (1) §16.45.5 — the census used to ride only on a promote
                        //      REFUSAL, so fixing the promote plane switched it off. It
                        //      now rides the success path too and survives its own fix.
                        //  (2) §16.45.4 — an accepted promotion used to contribute one
                        //      anonymous `control 0x2080012b result 0` tick and nothing
                        //      else, so `s39`'s ELEVEN acceptances were eleven opaque
                        //      successes and the rung's own sub-prediction ("this binds
                        //      ZERO ranges") could not be scored either way. ⊘ A claim
                        //      nothing could have contradicted was never a test.
                        //      `PromoteJoin` has carried all three numbers all along; only
                        //      the report threw them away.
                        //
                        // ★ `declined` is printed beside `bound`, not folded into it: a
                        // promotion that declares eight VAs and binds none is a completely
                        // different fact from one that declares none, and a single
                        // "accepted" tick cannot tell them apart — which is the whole of
                        // C defect D3 restated one layer up.
                        diag.latch_last(
                            FaultTag("promote-ctx ACCEPTED (last, with the census AT it)"),
                            format!(
                                "bound={} joined={} joined_global={} \
                                 globals_known={} globals_added={} \
                                 already={} parked={} half_already={} \
                                 half_unusable={} orphans(awaiting_va={},awaiting_phys={}) \
                                 declined.promote_only={} declined.initialize_only={} \
                                 entries={} halves={} \
                                 client={:#x} chan_client={:#x} object={:#x} proc={:?}{}",
                                join.bound,
                                join.joined,
                                // ★★★★★ §16.50 — the three numbers this rung is scored on,
                                // and `globals_known` is the one built for the case where
                                // the fix does nothing: it rides the SUCCESS path and no
                                // refusal, so `joined_global=0` is still legible.
                                // `globals_known=0` beside it says no GPU-scoped physical
                                // was ever published (the question is then allocation-time,
                                // not join-time); `globals_known>0` beside it says the map
                                // filled and nothing drew on it. Those are different rungs.
                                join.joined_global,
                                join.globals_known,
                                join.globals_added,
                                join.already,
                                join.parked,
                                join.half_already,
                                join.half_unusable,
                                join.orphans.0,
                                join.orphans.1,
                                p.declined.promote_only,
                                p.declined.initialize_only,
                                p.ranges.len(),
                                p.halves.len(),
                                p.client.0,
                                p.chan_client.0,
                                p.object.0,
                                join.route.proc,
                                gpu.vas_census(None),
                            ),
                        );
                        // ★★★★★ §16.48 — the CUMULATIVE row, emitted unconditionally on
                        // the SUCCESS path beside the last-wins one.
                        //
                        // ⊘ Not a prettier version of the row above; it answers a
                        // different question. That row says what the LAST promotion did.
                        // This says what ALL of them declared, per `buffer_id` — the only
                        // shape in which *"did phase 1 ever arrive?"* is answerable at all
                        // (§16.47.4), and the reason `s40` could not score its own
                        // two-phase account.
                        tally.record(&p);
                        // ★★★★★ §16.51 — and the JOIN outcome, cumulatively. See
                        // `SharedPromoteTally::totals`: the `ACCEPTED` row above is
                        // last-wins, and the cross-address-space join is a one-shot event
                        // that happens early, so `joined_global` reads 0 there even on the
                        // boot where it fired.
                        tally.record_join(&join);
                        diag.latch_last(
                            FaultTag("promote-ctx TALLY (cumulative, all promotions)"),
                            tally.render(),
                        );
                        Ok(Translation::CtxPromotion(p))
                    }
                    Err(e) => Err(BridgeRefusal::Promote(e)),
                },
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
            Err(r) => {
                self.census.record(r);
                // ★★★★ §16.40 — the address plane's state, sampled AT the refusal.
                //
                // ⊘ Only for a promote fault, and only the first: this is a diagnosis, not
                // a second census. `Gpu::vas_census_string`'s own docs carry the reason it
                // cannot be taken at teardown.
                //
                // ⚠ `as_gpu` returns `None` on the E2 shell port, which owns its state in
                // parts rather than as a `Gpu`. That is recorded rather than papered over:
                // a missing census must not read as an empty one.
                if let BridgeRefusal::Promote(f) = r {
                    // ⊘ Through the PORT, never through `as_gpu()`: the shipped shell
                    // answers `None` there, so an `as_gpu`-based diagnosis is blind on
                    // exactly the boots it exists for. See `ObjectModel::vas_census`.
                    //
                    // `None` marks no channel — promote-ctx names an `hObject`, and
                    // resolving it to a `ChanId` here would be a second resolution of the
                    // question that just failed.
                    let tag = r.fault_tag();
                    self.promote_diag
                        .latch(tag, format!("{f:?}{}", gpu.vas_census(None)));
                }
            }
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
///
/// ⊘ **`DUP_OBJECT` used to be listed here and is no longer** — see [`OBJECT_VERBS`]'s
/// `DupObject` row for why, and for the one thing worth keeping from the old text: this
/// paragraph named its own expiry condition (*"the cost of being wrong is one named refusal
/// on the next boot"*), the refusal arrived on `s31`, and nobody came back to read it.
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
    /// ★★★ **E9/§13.6 option (2)** — the engines THIS DEVICE advertised, i.e. the same
    /// `ChipProfile::engines` slice the device-info path serves the guest
    /// (`kayfabe_device::ga10x::GA106_ENGINES` on the shipped chip). A real GSP answers
    /// `NVA06F_CTRL_CMD_BIND` by linear-scanning its own engine-info list
    /// (`ogkm-580: kernel_fifo_gm107.c:672-759`, `NV_ERR_OBJECT_NOT_FOUND` at `:736`), so
    /// the faithful refusal is *"an engine we never advertised"* — and that question is
    /// answerable only against this slice.
    ///
    /// ⊘ **Required at construction, not a defaulted `Option`** — `None` would have to
    /// mean either "refuse every bind" (silently breaking mock-composed tests) or "accept
    /// every bind" (the `sandbox_unsafe::last_capability` fail-open shape). A gate whose
    /// default is open is not a gate. ⊘ And not on `Arch`: the engine set is per-**chip**
    /// (GA102 and GA106 differ in CE count while sharing `Ga10xArch`), and a second
    /// hand-written description of one silicon is the drift `inittables.rs` forbids by
    /// name.
    engines: &'static [kayfabe_abi::inittables::FifoDeviceEntry],
}

/// The RPC functions [`ObjectPolicy`] claims. **Closed, and public, so a test can quantify
/// over it rather than restate it** — `gates_quantified_over_a_list`'s rule: a gate that
/// spells its own universe is a gate that shrinks silently.
pub const OBJECT_VERBS: &[kayfabe_gsp::RpcFunction] = &[
    kayfabe_gsp::RpcFunction::RmAlloc,
    kayfabe_gsp::RpcFunction::Free,
    // ★★★★ §16.38 — `DUP_OBJECT` (fn 21), added on a MEASUREMENT rather than on a plan.
    //
    // The old text here read *"`translate_dup` exists and works; no boot has produced one …
    // the cost of being wrong is one named refusal on the next boot"*. `[measured
    // 2026-08-09, boot s31_675af4a_echofix]` the refusal arrived, in both places at once:
    //
    //   guest:  NVRM: rpcRmApiDupObject_GSP: GspRmDupObject failed: hClient=0xc1d0000a
    //           hParent=0xcaf00000 hObject=0xcaf00036 hClientSrc=0xc1d00015
    //           hObjectSrc=0x5c000007 flags=0x0 paramsStatus=0x0 status=0x00000056
    //   ours:   nvkvm:   unserviced fn 21            (run_s31_675af4a_echofix_qemu.log:171)
    //
    // `0x5c000007` is libcuda's own `FERMI_VASPACE_A`, and UVM wants it in ITS client so
    // `UVM_REGISTER_GPU_VASPACE` can name the address space libcuda opened
    // (`ogkm-580: nv_gpu_ops.c:2657-2664`, inside `nvGpuOpsDupAddressSpace`). Refusing it
    // is what makes that ioctl return `0x56` and `cuCtxCreate` return 801.
    //
    // ⊘ **Serving it is not a new data path** — that is the whole reason it is safe under
    // `gvaspub`'s rule against served-but-inert links. `translate_dup` → `RmEvent::Dup` →
    // `RmGraph::apply`'s `Dup` arm binds `dst` to the SOURCE'S OWN resource id, so UVM's
    // handle becomes a second name for the resource whose `pdb` the guest already
    // published. Nothing is minted, nothing is faked, and a source we have not observed
    // parks (`pending_dups`) instead of faulting.
    kayfabe_gsp::RpcFunction::DupObject,
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
pub const OBJECT_CONTROLS: &[u32] = &[
    kayfabe_abi::submit::NVA06F_CTRL_CMD_GPFIFO_SCHEDULE,
    // ★★★★ **§16.56 — the TSG form, `0xa06c0101`, and it is the wall `s44` measured.**
    //
    // `[measured 2026-08-10, boot s44_b17381c_rmtrace]` record 196 of 249: libcuda builds
    // the whole context — TSG, 8 channels, 8 compute objects, 8 copy objects, every one
    // `status=0` — asks RM to schedule the **group**, gets `0x56`, and the next record is a
    // `FREE`. ⊘ This id was on the capability allowlist (`capability.rs`) the whole time,
    // which is exactly why nothing saw it: **`admitted` and `served` are different gates**,
    // and clearing the first raises no bridge refusal, builds no `FaultTag` and appears in
    // no refusal census — it falls silently to the `UnservicedLedger`. The gate that now
    // makes that gap impossible to reopen is `tests/tests/admitted_is_served.rs`.
    kayfabe_abi::submit::NVA06C_CTRL_CMD_GPFIFO_SCHEDULE,
    // ★★★ E9/§13.6 — the channel-side bind. Same claim discipline: by id, never by the
    // whole `RmControl` function.
    kayfabe_abi::submit::NVA06F_CTRL_CMD_BIND,
    // ★★★ **§14.25 — the address-plane control, RE-CLAIMED.** It was claimed in §14.21,
    // measured to kill the adapter, and reverted; §14.21's own re-enable condition was
    // *"when the `0x90f10106` publication reaches `Vas::pdb`, promote-ctx SUCCEEDS rather
    // than refuses"*, and §14.24 measured that publication landing (`4 ACCEPTED`) with the
    // milestone reproducing byte for byte. This is that condition being tested.
    //
    // ⚠ The status question does NOT disappear with it — see `Self::respond_promote_ctx`.
    kayfabe_abi::generated::ctrl::NV2080_CTRL_CMD_GPU_PROMOTE_CTX,
    // ★★★★ **§16.59 — `NV2080_CTRL_CMD_GR_SET_CTXSW_PREEMPTION_MODE`, `0x20801210`: the
    // wall `s45` and `s46` both measured at record 331 of 456**, `status=0x56`, with record
    // 332 beginning the `FREE` burst and its `hChannel` naming the very TSG record 196 had
    // just scheduled.
    //
    // ⊘ It is claimed on a **classifier**, not unconditionally, and that distinction is the
    // rung: see `Self::respond_ctxsw_preemption_mode`. `[measured]` the C artifact's guest
    // asked for `COMPUTE_CILP` on this id and the C echoed `NV_OK`; ours asks for
    // `COMPUTE_WFI`. Copying the C would have been honest on our payload by accident.
    kayfabe_abi::submit::NV2080_CTRL_CMD_GR_SET_CTXSW_PREEMPTION_MODE,
];

impl ObjectPolicy {
    /// A policy that owns `gpu`, decoding with `abi`, serving a `guest_os`, answering
    /// engine questions against `engines` — the same `ChipProfile::engines` slice the
    /// device-info path serves (see the field's docs for why it is required, not
    /// defaulted).
    #[must_use]
    pub fn new(
        abi: &DriverAbiTable,
        guest_os: GuestOs,
        gpu: Gpu,
        engines: &'static [kayfabe_abi::inittables::FifoDeviceEntry],
    ) -> ObjectPolicy {
        ObjectPolicy::with_limits(abi, guest_os, gpu, engines, ReasmLimits::default())
    }

    /// As [`ObjectPolicy::new`], with explicit continuation bounds.
    #[must_use]
    pub fn with_limits(
        abi: &DriverAbiTable,
        guest_os: GuestOs,
        gpu: Gpu,
        engines: &'static [kayfabe_abi::inittables::FifoDeviceEntry],
        limits: ReasmLimits,
    ) -> ObjectPolicy {
        ObjectPolicy::over(abi, guest_os, Box::new(gpu), engines, limits)
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
        engines: &'static [kayfabe_abi::inittables::FifoDeviceEntry],
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
            engines,
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

    /// ★★★★ §16.40 — the first refused `GPU_PROMOTE_CTX`, with the address plane's state
    /// as it stood at that instant. See [`SharedPromoteDiag`] for why it is a handle and
    /// why the sample is taken at the refusal rather than at teardown.
    #[must_use]
    pub fn promote_diag(&self) -> SharedPromoteDiag {
        self.bridge.promote_diag.clone()
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

    /// ★★★ **#177 + E9/§13.6** — answer the controls in [`OBJECT_CONTROLS`], and **only**
    /// them: `NVA06F_CTRL_CMD_GPFIFO_SCHEDULE` (`0xa06f0103`) and `NVA06F_CTRL_CMD_BIND`
    /// (`0xa06f0104`).
    ///
    /// The shapes of the answers, and why each is the one it is:
    ///
    /// - **`None`** — any control not in [`OBJECT_CONTROLS`], and any payload this policy
    ///   cannot even classify as a control. The chain (and therefore the unserviced
    ///   ledger) is untouched. ⊘ This is the arm that must stay large: it is what keeps
    ///   the ledger honest.
    /// - **`NV_OK` with the request's own params bytes echoed** — the transition was
    ///   performed. The body is what a real GA106's GSP sends
    ///   (`kayfabe_abi::submit::encode_gpfifo_schedule` / `encode_bind`), not what the
    ///   C's empty capture rows imply.
    /// - **A decided refusal with an empty body** — the decode, the engine check or the
    ///   route said no; per-arm statuses are documented on [`Self::respond_bind`] and the
    ///   schedule arm. ⚠ Never `NV_ERR_NOT_SUPPORTED`: that is the FSM's signature for
    ///   *"nobody claimed this"*, and the guest prints the raw hex, so reusing it would
    ///   erase the difference between "refused" and "unimplemented" in the only place
    ///   anyone reads it.
    fn respond_control(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        let req = self.bridge.abi.decode_rpc_control(&cmd.payload).ok()?;
        if !OBJECT_CONTROLS.contains(&req.cmd) {
            return None;
        }
        // ★ The list gates, the match dispatches, and the two must not drift — a claimed
        // id with no arm here would fall through to the unserviced ledger as a `0x56`
        // while `OBJECT_CONTROLS` says it is decided
        // (`a_table_does_not_decide_behaviour`). The lockstep is tested:
        // `every_claimed_control_is_decided_even_when_malformed` quantifies over the
        // constant and demands `Some` from this function for every member.
        match req.cmd {
            kayfabe_abi::submit::NVA06F_CTRL_CMD_GPFIFO_SCHEDULE => {
                self.respond_gpfifo_schedule(cmd, &req)
            }
            kayfabe_abi::submit::NVA06C_CTRL_CMD_GPFIFO_SCHEDULE => {
                self.respond_gpfifo_schedule_group(cmd, &req)
            }
            kayfabe_abi::submit::NVA06F_CTRL_CMD_BIND => self.respond_bind(cmd, &req),
            kayfabe_abi::generated::ctrl::NV2080_CTRL_CMD_GPU_PROMOTE_CTX => {
                self.respond_promote_ctx(cmd)
            }
            kayfabe_abi::submit::NV2080_CTRL_CMD_GR_SET_CTXSW_PREEMPTION_MODE => {
                self.respond_ctxsw_preemption_mode(cmd, &req)
            }
            _ => None,
        }
    }

    /// ★★★★ **§16.59 — the `NV2080_CTRL_CMD_GR_SET_CTXSW_PREEMPTION_MODE` arm** (`0x20801210`):
    /// the wall `s45` and `s46` both measured at record **331** of 456, two records before
    /// `cuCtxCreate` gives up and one record before the `FREE` burst.
    ///
    /// # ★★★ What is being claimed, in the two words the brief asked for
    ///
    /// **Structurally honest** — every field of
    /// [`kayfabe_abi::submit::CtxswPreemptionRequest`] is `[IN]`
    /// (`ogkm-580: ctrl2080gr.h:836-842`), so there is no `[OUT]` field an echo could get
    /// wrong. **And semantically honest, conditionally** — which is more than the brief
    /// asked for and less than an unconditional echo would claim:
    ///
    /// - the request asks for a **postcondition** (*"this context switches at mode X"*);
    /// - the only `X` this port can be truthful about is wait-for-idle, because it has no
    ///   preemption machinery at all — WFI is not a mode it fails to program, it is the mode
    ///   it is unconditionally in;
    /// - so the arm **classifies the request** ([`kayfabe_abi::submit::CtxswPreemptionRequest::asks_for`]),
    ///   answers `NV_OK` only for wait-for-idle, and refuses everything else **by name**.
    ///
    /// ⇒ The claim in the commit is *"verified, not merely echoed"*. A request for CILP, CTA
    /// or GfxP gets [`kayfabe_core::gpu::CtxswPreemptionFault::PreemptionNotImplemented`],
    /// which is a sentence this port can defend.
    ///
    /// # ⊘⊘⊘ Why the C artifact is NOT the oracle here — it answered a different request
    ///
    /// `[measured 2026-08-10, cap3_matmul_forwarding #453716/#453717 vs boot s46 record 331]`.
    /// This rung was briefed as *"our request bytes match the C's byte-for-byte"*. Three of
    /// the four words do; the fourth is `cilpPreemptMode`, and it is **`2` (`COMPUTE_CILP`)
    /// in the C** against **`0` (`COMPUTE_WFI`) in ours**. That is the only word that decides
    /// whether an `NV_OK` is a true sentence. The C's ack promised instruction-level compute
    /// preemption it had no machinery for and still reached `bad=0 maxerr=0` — because a
    /// short matmul never preempts, so nothing ever read the promise.
    ///
    /// ⇒ A green C oracle is evidence about the C's payload, never about ours. Diffing the
    /// **reply** would have shown a perfect match and taught us to ship the unconditional
    /// echo; only diffing the **request** caught it.
    ///
    /// # ⊘ Why the reply is the request's own bytes
    ///
    /// [`Self::respond_promote_ctx`]'s reason exactly: `paramsSize` is 32, non-zero, so the
    /// GSP transport copies the reply's params over the caller's struct
    /// (`ogkm-580: rpc.c:11085-11090`), and a zero body would clear the caller's `flags` and
    /// `hChannel` behind its back. `[measured]` the C does the same — `cap3` #453702/17/32
    /// are the request element verbatim with only `checkSum`, `seqNum`, `rpc_result` and
    /// `rpc_result_private` rewritten.
    ///
    /// ⊘ **And the echo is therefore NOT the falsifier.** Checking that the reply is the
    /// echo tests this function's `copy_from_slice`. What discriminates is (a) at unit
    /// level, mutating `cilpPreemptMode` and demanding the answer *change*
    /// (`tests/tests/ctxsw_preemption_mode.rs`), and (b) at boot level, **what the guest
    /// does next** — record 332 currently begins the `FREE` burst, and whether it still does
    /// with record 331 at `status=0` is the whole result (§16.59's falsifier).
    ///
    /// # The refusal status
    ///
    /// [`kayfabe_abi::submit::CTXSW_PREEMPTION_REFUSED_STATUS`] = `NV_ERR_NOT_SUPPORTED`,
    /// which is **this control's own documented status for this exact condition** —
    /// *"A value of `NV_ERR_NOT_SUPPORTED` is returned if the target channel does not support
    /// preemption context switch mode changes"* (`ogkm-580: ctrl2080gr.h:791-795`). ⚠ Read
    /// that constant's docs before citing this as a breach of the standing "never reuse
    /// `0x56`" rule: the rule forbids **borrowing** a status that means *absent*, and here
    /// the header supplies it for the meaning we intend.
    fn respond_ctxsw_preemption_mode(
        &mut self,
        cmd: &RpcCommand,
        req: &kayfabe_abi::view::RpcControlReq,
    ) -> Option<Reply> {
        let refuse = || {
            Some(Reply {
                rpc_result: kayfabe_abi::submit::CTXSW_PREEMPTION_REFUSED_STATUS,
                body: Vec::new(),
            })
        };
        // The guest's own two assertions about its params — the same pair every other arm
        // on this list checks, for the same reason.
        let want = kayfabe_abi::submit::CtxswPreemptionRequest::SIZE;
        if kayfabe_abi::rpc_params_are_serialized(req.rmapi_rpc_flags)
            || req.params_size as usize != want
            || cmd.payload.len() < req.params_at + want
        {
            return refuse();
        }
        let Ok(params) = kayfabe_abi::submit::decode_ctxsw_preemption_mode(
            &cmd.payload[req.params_at..req.params_at + want],
        ) else {
            return refuse();
        };
        // THE MODE, first — it is an ABI question over a wire struct and it is answered
        // here rather than in `kayfabe-core`, exactly as `respond_bind`'s engine-space
        // conversion is (`kayfabe-core` does not depend on `kayfabe-abi`).
        match params.asks_for() {
            kayfabe_abi::submit::CtxswPreemptionAsk::WaitForIdle => {}
            kayfabe_abi::submit::CtxswPreemptionAsk::GraphicsPreemption { .. }
            | kayfabe_abi::submit::CtxswPreemptionAsk::ComputePreemption { .. } => {
                return refuse();
            }
        }
        // THE CONTEXT. ⊘ Resolved in the client the request was asked in, against the
        // `hChannel` FIELD and not `req.object` — `req.object` is the subdevice
        // (`[measured 2026-08-10, boot s46_1a9e93c_abi35 record 331]` `hObject=0x5c000003`),
        // and answering about the subdevice would be answering a question nobody asked.
        //
        // ⊘ No private counter and no census call here, deliberately — the same choice
        // `Self::respond_gpfifo_schedule_group` documents. `kayfabe_device::census::ControlCensus`
        // wraps the whole chain and records `(cmd, rpc_result)`, so both outcomes print
        // themselves in the boot report as `control 0x20801210 result 0x00000000|0x00000056`,
        // and a second count kept here would be a number that can disagree with the report's.
        if self
            .gpu
            .set_ctxsw_preemption_mode(
                kayfabe_arch::ids::HClient(req.client),
                kayfabe_arch::ids::HObject(params.h_channel),
            )
            .is_err()
        {
            return refuse();
        }
        let mut body = cmd.payload.clone();
        let params_out = kayfabe_abi::submit::encode_ctxsw_preemption_mode(&params);
        body[req.params_at..req.params_at + want].copy_from_slice(&params_out);
        Some(Reply {
            rpc_result: 0, // NV_OK
            body,
        })
    }

    /// ★★★ **§14.25 — the `NV2080_CTRL_CMD_GPU_PROMOTE_CTX` arm, re-claimed after §14.21
    /// measured its first version killing the adapter.**
    ///
    /// ⊘ **This arm decodes nothing itself, and that is the point.** The other two arms
    /// hand-decode because their params are four and eight bytes; this control's are 560
    /// with a 16-entry array and a three-state classifier, and every piece of that already
    /// exists on the [`Bridge`] path — `translate_promote_ctx` → `Translation::CtxPromotion`
    /// → [`kayfabe_core::gpu::Gpu::promote_ctx`]. Re-deriving it here would be a second
    /// decoder for one wire struct, which is the drift `Bridge::deliver` was extracted to
    /// prevent. So this arm is a **route**, and it inherits reassembly, the `promoted`
    /// counter and the refusal census for free.
    ///
    /// # The reply, and why it is the request's own bytes
    ///
    /// `paramsSize` is 560, non-zero, so the GSP transport copies the reply's params over
    /// the caller's struct (`ogkm-580: rpc.c:11085-11090`) — and RM then **reads its own
    /// struct back**: on `NV_OK` it walks `params.promoteEntry[i].bInitialize` to decide
    /// which context buffers to mark initialized
    /// (`ogkm-580: kernel_graphics_object.c:141-157`). A zero-filled body would therefore
    /// not merely lose information, it would rewrite guest state — which is exactly C
    /// defect **D7**, where a captured foreign-boot blob was replayed into the caller's
    /// buffer under `NV_OK` (`gpu_promote_ctx.md` §3 D7). Echoing the guest's own bytes
    /// unchanged is that defect's documented port: *"a Case-2 ACK writes back nothing"*.
    ///
    /// # ★★★ THE REFUSAL STATUS, and it is `0x56` — the whole of §14.21's lesson
    ///
    /// The first version of this arm answered `NV_ERR_INVALID_OBJECT_HANDLE` (`0x33`) and
    /// `NV_ERR_INVALID_STATE` (`0x40`), per this crate's standing rule that `0x56` is the
    /// FSM's *"nobody claimed this"* signature and must never be reused for a decision.
    /// `[measured 2026-08-08, boot ship2_7c5d74d]` that cost the milestone: the A/B against
    /// `ship_7a881a7` differed in four lines and ended `RmInitAdapter failed! (0x25:0x40:1249)`.
    ///
    /// `gpuStatePostLoad` (`ogkm-580: src/nvidia/src/kernel/gpu/gpu.c:3437-3439`) converts
    /// **only** `NV_ERR_NOT_SUPPORTED` to `NV_OK` and bails on everything else, and this
    /// control's failure reaches it: `kgrobjPromoteContext` → `_kgrAlloc` →
    /// `kgraphicsCreateGoldenImageChannel` → the FIFO engine's `StatePostLoad`
    /// (`kfifoStateLoad_GM107` → `kfifoTriggerPostSchedulingEnableCallback`,
    /// `ogkm-580: kernel_fifo_gm107.c:229-234`, which returns any error verbatim).
    ///
    /// ⇒ The rule needed a **scope**, not a repeal: it is right wherever the guest's error
    /// path *reads* the status (`GPFIFO_SCHEDULE`, `BIND`) and wrong for any control whose
    /// failure propagates into an engine's `StatePostLoad`. ★ So this arm answers
    /// [`BridgeRefusal::rpc_result`], which is `NV_ERR_NOT_SUPPORTED` for **every** variant
    /// — i.e. the refusal rides the envelope exactly as it does for a control this port
    /// never claimed, and the census is where the *variant* is recorded. Asking *"where
    /// does the caller's error go?"* is the question, never *"what does the header
    /// document?"*.
    ///
    /// ⊘ That makes a refused promote-ctx **wire-indistinguishable** from an unserviced
    /// one, and it is worth saying out loud: the difference is visible only in this port's
    /// own census, which is where it belongs. Nothing is bought by telling the guest a
    /// truer status when the truer status is what kills it.
    fn respond_promote_ctx(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        match self.deliver(cmd) {
            // The promotion landed, or this was a fragment the reassembler is holding.
            // Both are `NV_OK` with this message's own body — the held case because the
            // guest reads its status off the LAST fragment and needs each earlier one
            // acknowledged with its own length (`GraphPolicy::respond`'s `Held` docs).
            Ok(Translation::CtxPromotion(_) | Translation::Held) => Some(Reply {
                rpc_result: 0, // NV_OK
                body: cmd.payload.clone(),
            }),
            // ⊘ Not reachable through `translate` for this command id — `ControlParams::PromoteCtx`
            // routes to `translate_promote_ctx`, which returns `CtxPromotion` or an error.
            // Written as a decided refusal rather than `unreachable!()` because this arm is
            // reached from a GUEST-controlled command id: a panic here aborts the whole VM.
            Ok(Translation::Event(_) | Translation::Inert) => Some(Reply {
                rpc_result: kayfabe_abi::NV_ERR_NOT_SUPPORTED,
                body: Vec::new(),
            }),
            Err(r) => Some(Reply {
                rpc_result: r.rpc_result(),
                body: Vec::new(),
            }),
        }
    }

    /// The `NVA06F_CTRL_CMD_GPFIFO_SCHEDULE` arm — see [`Self::respond_control`].
    fn respond_gpfifo_schedule(
        &mut self,
        cmd: &RpcCommand,
        req: &kayfabe_abi::view::RpcControlReq,
    ) -> Option<Reply> {
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

    /// ★★★★ **§16.56 — the `NVA06C_CTRL_CMD_GPFIFO_SCHEDULE` arm** (the TSG form,
    /// `0xa06c0101`): the wall `cuCtxCreate` stopped at, and the one control on this list
    /// that a real `cup2` is measured to issue on its way to first compute.
    ///
    /// # ⊘⊘ Why this is not [`Self::respond_gpfifo_schedule`] with a wider id check
    ///
    /// The decode is shared and **sourced as shared** — the params are a typedef of the
    /// channel form's (`ogkm-580: ctrla06c.h:101`), so `decode_gpfifo_schedule` is the
    /// right decoder rather than a convenient one. Everything after the decode differs:
    /// the object is a channel **group**, the act fans out over the group's members, and
    /// the refusals are about a set rather than about one channel
    /// ([`kayfabe_core::gpu::ScheduleGroupFault`]). Routing a group handle into
    /// `route_schedule_channel` would refuse it `NotAChannel` — correctly, and for a
    /// reason that has nothing to do with what the guest asked.
    ///
    /// # ⊘⊘ Why the `NV_OK` is not a forged one
    ///
    /// The C's comment at `nvkvm_gpu_emul.c:8038` is the standing warning —
    /// *"the host TSG is idle until we schedule it"* — so an ack alone would move the wall
    /// without scheduling anything. It is not an ack alone: the members land in
    /// [`kayfabe_core::gpu::ExecPlane::requested`], which `kayfabe_fwd::plan_doorbell`
    /// **gates** on (`FwdFault::NotScheduled`), and the host-side
    /// `NVA06C_CTRL_CMD_GPFIFO_SCHEDULE` is issued by
    /// `kayfabe_isolate::RmBackend::schedule` at the member's first doorbell. Same
    /// deferral, same argument and same falsifier as the channel form —
    /// `docs/design/gpfifo_schedule.md` §2, and §3 for what is still false.
    ///
    /// The reply is the request's own params bytes, for
    /// [`Self::respond_gpfifo_schedule`]'s reason: `paramsSize != 0`, so the GSP transport
    /// copies the reply's params over the caller's struct
    /// (`ogkm-580: rpc.c:11085-11090`) and a zero body would clear the caller's `bEnable`
    /// behind its back.
    fn respond_gpfifo_schedule_group(
        &mut self,
        cmd: &RpcCommand,
        req: &kayfabe_abi::view::RpcControlReq,
    ) -> Option<Reply> {
        let refuse = || {
            Some(Reply {
                rpc_result: kayfabe_abi::submit::GPFIFO_SCHEDULE_REFUSED_STATUS,
                body: Vec::new(),
            })
        };
        // The guest's own two assertions about its params — the same pair the channel arm
        // checks, because it is the same params struct.
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
        let ack = self.gpu.schedule_group(
            kayfabe_arch::ids::HClient(req.client),
            kayfabe_arch::ids::HObject(req.object),
            params.b_enable != 0,
        );
        self.gpu.publish_isolate_census(&self.isolates);
        let Ok(ack) = ack else {
            return refuse();
        };
        // ⊘ No private counter here, deliberately. `kayfabe_device::census::ControlCensus`
        // wraps the WHOLE chain and records `(cmd, rpc_result)` for every answered control,
        // so this arm's success prints itself in the boot report as
        // `control 0xa06c0101 result 0x00000000 xN` with no new plumbing — and a second
        // count kept here would be a number that can disagree with the report's.
        let _ = ack;
        let mut body = cmd.payload.clone();
        let params_out = kayfabe_abi::submit::encode_gpfifo_schedule(&params);
        body[req.params_at..req.params_at + want].copy_from_slice(&params_out);
        Some(Reply {
            rpc_result: 0, // NV_OK
            body,
        })
    }

    /// ★★★ **E9/§13.6 — the `NVA06F_CTRL_CMD_BIND` arm.** Decode, then the ENGINE, then
    /// the CHANNEL — and the two refusals are different statuses on purpose:
    ///
    /// - **`BIND_UNKNOWN_ENGINE_STATUS` (`0x57`, `NV_ERR_OBJECT_NOT_FOUND`)** — the
    ///   request names an engine this device never advertised. The faithful answer: a
    ///   real GSP linear-scans its own engine-info list and returns exactly this
    ///   (`ogkm-580: kernel_fifo_gm107.c:736`), and that list is built from the very
    ///   device-info table this port serves.
    /// - **`BIND_REFUSED_STATUS` (`0x40`, `NV_ERR_INVALID_STATE`)** — the engine is
    ///   real but the request is malformed or the channel cannot be routed.
    /// - **`NV_OK` with the request's own four bytes echoed** — `[measured]` a real
    ///   GA106's reply body IS the request (`traces/real_ga106/`, `0b 00 00 00`), and a
    ///   zero-filled body would rewrite the caller's `engineType` to `NULL` behind its
    ///   back (`ogkm-580: rpc.c:11085-11090`).
    ///
    /// ⚠ ★ The engine check converts **first** (`nv2080_to_rm_engine_type`): the wire is
    /// `NV2080_ENGINE_TYPE` space, the advertised table is RM space, and the two collide
    /// above `0x12` — raw `0x13` is NVDEC0 in one and COPY10 in the other, so a raw
    /// compare is a silent wrong answer, not a shortcut.
    fn respond_bind(
        &mut self,
        cmd: &RpcCommand,
        req: &kayfabe_abi::view::RpcControlReq,
    ) -> Option<Reply> {
        let refuse = |status: u32| {
            Some(Reply {
                rpc_result: status,
                body: Vec::new(),
            })
        };
        // The guest's own two assertions about its params, both checked — the same pair
        // the schedule arm checks, for the same reason.
        let want = kayfabe_abi::submit::BindParams::SIZE;
        if kayfabe_abi::rpc_params_are_serialized(req.rmapi_rpc_flags)
            || req.params_size as usize != want
            || cmd.payload.len() < req.params_at + want
        {
            return refuse(kayfabe_abi::submit::BIND_REFUSED_STATUS);
        }
        let Ok(params) =
            kayfabe_abi::submit::decode_bind(&cmd.payload[req.params_at..req.params_at + want])
        else {
            return refuse(kayfabe_abi::submit::BIND_REFUSED_STATUS);
        };
        // THE ENGINE — convert first, then ask "did THIS DEVICE advertise it?" against
        // the same slice the device-info path serves the guest (§13.6 option (2)).
        let Some(rm_engine_type) =
            kayfabe_abi::submit::nv2080_to_rm_engine_type(params.engine_type)
        else {
            return refuse(kayfabe_abi::submit::BIND_UNKNOWN_ENGINE_STATUS);
        };
        let advertised = self.engines.iter().any(|e| {
            e.engine_data[kayfabe_abi::inittables::engine_info_type::RM_ENGINE_TYPE]
                == rm_engine_type
        });
        if !advertised {
            return refuse(kayfabe_abi::submit::BIND_UNKNOWN_ENGINE_STATUS);
        }
        // THE CHANNEL — route and record, in the core.
        let ack = self.gpu.bind_channel(
            kayfabe_arch::ids::HClient(req.client),
            kayfabe_arch::ids::HObject(req.object),
            rm_engine_type,
        );
        self.gpu.publish_isolate_census(&self.isolates);
        if ack.is_err() {
            return refuse(kayfabe_abi::submit::BIND_REFUSED_STATUS);
        }
        let mut body = cmd.payload.clone();
        body[req.params_at..req.params_at + want]
            .copy_from_slice(&kayfabe_abi::submit::encode_bind(&params));
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

// =====================================================================================
// ★★★ The PUBLICATION observer — §14.23
// =====================================================================================

/// The `RpcFunction::RmControl` command ids [`PublicationObserver`] carries into the object
/// model, and the **only** ones.
///
/// ⊘ Public and closed so a test quantifies over it rather than restating it
/// (`gates_quantified_over_a_list`), exactly like [`OBJECT_CONTROLS`]. ⊘ And it is a
/// different list from that one on purpose: these two ids are **not answered here**. The
/// device chain's `InitTablePolicy` answers them and must keep answering them; this list
/// says only *which controls carry a fact this port must not drop*.
pub const PUBLICATION_CONTROLS: &[u32] = &[
    kayfabe_abi::gvaspacepdes::NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES,
    kayfabe_abi::gvaspacepdes::NV2080_CTRL_CMD_INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER,
    // ★★★★ §16.42 — `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY`. The THIRD transport by which
    // the guest publishes a page-directory root, and the one `cup2`'s own address space
    // depends on. It was absent from this list for six increments while
    // `kayfabe_device::setpagedir::SetPageDirPolicy` answered it `NV_OK` and wrote it into a
    // report — `⊘ recording is not forwarding`, the same sentence this list's own docs
    // already make about `0x90f10106`.
    //
    // ★★★ THE CHAIN, closed end to end in ONE boot with same-boot identity
    // (`[measured 2026-08-09, boot `s37_0dfe7f7_pertag`]`):
    //
    //   promote-ctx ContextVasUndeclared { client 0xc1d0000c, object 0x5c000019 }
    //     … {1x pdb=N own=not-declared cs=ok(h0x5c000007=>c0xc1d0000c/0x5c000007)
    //            p2/c0:vc7 GrCompute c0xc1d0000c/0x5c000019}
    //
    // - `0x5c000019` is `cup2`'s `AMPERE_CHANNEL_GPFIFO_A` (its own `rmtrace`, same boot).
    // - It declares no `hVASpace` of its own; route 2 commits — the **CtxShare** — and
    //   resolves to `0x5c000007`, libcuda's **FIRST** `FERMI_VASPACE_A`.
    // - `0x5c000007` is exactly the handle UVM dups: `GspRmDupObject … hObject=0xcaf00036;
    //   hClientSrc=…; hObjectSrc=0x5c000007` (`run_s31_675af4a_echofix_probe.log:307`).
    // - And `0x00801813` publishes that root under the alias: `SET_PAGE_DIRECTORY … hVASpace
    //   0xcaf00036 physAddress 0x201000` (every boot since `s35`).
    // - libcuda's *second* VA space, `0x5c000008`, publishes through `0x90f10106` — which is
    //   already on this list. **Two VA spaces, two transports, one of them routed.**
    //
    // ⇒ the PDB the channel's VA space is refused for not having is on the wire, on this
    // transport, and nothing carried it into the graph. `RmEvent::SetPageDir` sets `pdb` on
    // the **resource**, and a `Dup` binds the alias to the source's resource id, so the root
    // published under `0xcaf00036` lands on `0x5c000007`'s resource — the one `ctx_vas`
    // resolves through. No new mechanism; this list was the missing route.
    //
    // ⚠ §14.21's warning does not lapse and is why this is an OBSERVER entry rather than a
    // new answerer: the risk it recorded was *claiming* this control and answering with a
    // status the guest's error path reads. `SetPageDirPolicy` keeps answering it, unchanged;
    // `PublicationObserver` is a `CommandObserver` and **cannot change a reply byte**
    // (`observe` returns nothing to return). Exactly the shape that made `0x90f10106` safe.
    kayfabe_abi::generated::ctrl::NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY,
];

/// ★★★ **The guest's page-directory publication, carried into the object model** — the
/// link that makes `Vas::pdb` a fact instead of a line in a report.
///
/// # Why this is an OBSERVER and not a policy, and why it is a second seat
///
/// `0x90f10106` is **served** — `[measured 2026-08-08, boots ship_7a881a7 / ship3_d5369b5]`
/// `control 0x90f10106 result 0x00000000 x4` — and the link that serves it is
/// `kayfabe_device::inittables::InitTablePolicy`, which is the correct answerer and is
/// pinned as such by two of that crate's tests. A device policy chain is a `find_map`, so
/// a link that needs to *see* this control has to sit **ahead** of the one that answers it.
///
/// A [`CommandPolicy`] in that seat could re-route the reply by returning `Some`; a
/// [`kayfabe_gsp::CommandObserver`] cannot, because it has nothing to return. That trait's
/// docs carry the argument, including the obligation-that-was-never-checked this port
/// already paid for.
///
/// # ⊘ Why it does not reuse `Bridge`, which is the reuse this crate normally insists on
///
/// [`Bridge::deliver`] runs the [`Reassembler`] first, and there must be exactly **one**
/// reassembler over a command stream: a second one seated ahead of [`ObjectPolicy`] would
/// consume the same continuation fragments into a second, independent buffer — two
/// half-messages where the guest sent one. So this link calls
/// [`translate`](crate::translate) directly, on **whole** commands only, and a fragmented
/// publication therefore refuses by name rather than being silently half-absorbed.
///
/// ⊘ That is a real limit, stated: a `GSP_RM_CONTROL` carrying 184 bytes of params in a
/// 36-byte header does not fragment on any transport this port has observed, and if one
/// ever does, [`BridgeRefusal::PublishedPdesMalformed`] is what says so.
///
/// # ★ The census is SHARED, not its own
///
/// It takes a [`SharedRefusalCensus`] at construction — the same handle [`ObjectPolicy`]
/// publishes — so a publication this link refuses appears in the one census the boot
/// report prints. Two censuses would be two answers to *"what did the bridge refuse?"*,
/// and the report prints one.
pub struct PublicationObserver {
    abi: DriverAbiTable,
    guest_os: GuestOs,
    gpu: Box<dyn ObjectModel>,
    refusals: SharedRefusalCensus,
    census: SharedPublicationCensus,
}

/// What [`PublicationObserver`] saw and what the object model did with it.
///
/// ★★★ **Two numbers, and the first is the denominator of the second.** *"No refusals"*
/// over a boot in which `seen == 0` is a link that was never seated, which is this port's
/// most-repeated instrument failure (`skipped_oracle_kills_the_guard`,
/// `gate_read_through_grep_cannot_fail`). A report that printed only `applied` could not
/// tell *"the guest published nothing"* from *"we never looked"*.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PublicationCensus {
    /// Publications that arrived on a command in [`PUBLICATION_CONTROLS`], well-formed or
    /// not.
    pub seen: u64,
    /// ★★★ Publications the object model **accepted** — the number that says `Vas::pdb` was
    /// populated from the guest's own statement.
    ///
    /// ⊘ `seen - applied` is not the refusal count on its own: a publication for a VA space
    /// the guest has not allocated yet **parks** in the graph and is counted here as
    /// applied, because the graph accepted the fact. The refusals are named in
    /// [`SharedRefusalCensus`].
    pub applied: u64,
    /// Translations of a claimed control that were not an `RmEvent`.
    ///
    /// ⊘ Unreachable by construction today (see [`PublicationObserver::observe`]) and
    /// counted anyway rather than asserted: this runs on a vCPU thread inside QEMU, where a
    /// panic is an abort of the whole VM. A number is a better instrument than a hypervisor
    /// that dies to report that a translator arm changed shape.
    pub unexpected: u64,
}

/// [`PublicationCensus`] as a handle the composition root keeps after handing the observer
/// away — same shape, and the same ownership argument, as [`SharedRingCensus`].
#[derive(Debug, Clone, Default)]
pub struct SharedPublicationCensus(std::sync::Arc<std::sync::Mutex<PublicationCensus>>);

impl SharedPublicationCensus {
    /// A fresh, empty census.
    #[must_use]
    pub fn new() -> SharedPublicationCensus {
        SharedPublicationCensus::default()
    }

    /// A point-in-time copy.
    #[must_use]
    pub fn snapshot(&self) -> PublicationCensus {
        *self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn note(&self, f: impl FnOnce(&mut PublicationCensus)) {
        f(&mut self.0.lock().unwrap_or_else(|e| e.into_inner()));
    }
}

impl PublicationObserver {
    /// An observer that declares into an object model somebody else owns, recording its
    /// refusals into `census`.
    ///
    /// ★ `gpu` is a **second** [`ObjectModel`] handle onto the **same** model, which is
    /// what that port exists for (E2: *"a second `Gpu` behind the doorbell would be a
    /// routing table that can never resolve"*). ⊘ Handing this a model of its own would be
    /// precisely that defect: page-directory bases landing in a graph no promotion can see.
    #[must_use]
    pub fn over(
        abi: &DriverAbiTable,
        guest_os: GuestOs,
        gpu: Box<dyn ObjectModel>,
        refusals: SharedRefusalCensus,
    ) -> PublicationObserver {
        PublicationObserver {
            abi: *abi,
            guest_os,
            gpu,
            refusals,
            census: SharedPublicationCensus::new(),
        }
    }

    /// The census as a **handle**, for the composition root that must keep reading it after
    /// boxing this observer — see [`SharedPublicationCensus`].
    #[must_use]
    pub fn census(&self) -> SharedPublicationCensus {
        self.census.clone()
    }

    /// Whether this observer carries `cmd` — the predicate [`Self::observe`] gates on,
    /// exposed so a test asks the type rather than a copy of its list.
    #[must_use]
    pub fn claims(cmd: u32) -> bool {
        PUBLICATION_CONTROLS.contains(&cmd)
    }

    /// A point-in-time copy of what this observer has seen — see [`PublicationCensus`].
    #[must_use]
    pub fn snapshot(&self) -> PublicationCensus {
        self.census.snapshot()
    }
}

impl core::fmt::Debug for PublicationObserver {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PublicationObserver")
            .field("driver", &self.abi.version())
            .field("census", &self.census.snapshot())
            .finish_non_exhaustive()
    }
}

impl kayfabe_gsp::CommandObserver for PublicationObserver {
    /// Gate, translate, apply — and there is no fourth step, because there is nothing to
    /// answer.
    fn observe(&mut self, cmd: &RpcCommand) {
        if cmd.function != kayfabe_gsp::RpcFunction::RmControl {
            return;
        }
        // ⊘ A header this crate cannot even decode is not this link's finding: the chain
        // below still sees the command, and the FSM's own ledger records what nobody
        // answered. Counting it here would double-count a malformed envelope.
        let Ok(req) = self.abi.decode_rpc_control(&cmd.payload) else {
            return;
        };
        if !PublicationObserver::claims(req.cmd) {
            return;
        }
        self.census.note(|c| c.seen = c.seen.saturating_add(1));
        match translate(&self.abi, self.guest_os, cmd) {
            Ok(Translation::Event(ev)) => match self.gpu.apply(ev) {
                Ok(()) => self
                    .census
                    .note(|c| c.applied = c.applied.saturating_add(1)),
                // ★ The graph's own refusal, named — a publication for a VA space the
                // guest has not allocated **parks** rather than refusing
                // (`RmGraph::pending_pdbs`), so reaching this arm means something
                // stronger than an ordering surprise.
                Err(e) => self.refusals.record(&BridgeRefusal::Graph(e)),
            },
            Ok(_) => self
                .census
                .note(|c| c.unexpected = c.unexpected.saturating_add(1)),
            Err(r) => self.refusals.record(&r),
        }
    }
}

// The concurrency contract, compile-time-asserted (decision #17). `GraphPolicy` must be
// `Send` or it cannot be a `CommandPolicy` at all — the FSM takes `&mut dyn CommandPolicy`
// and `kayfabe_gsp::boot` asserts the trait object is `Send`.
kayfabe_util::assert_send_sync!(RefusalCensus);
kayfabe_util::assert_send!(GraphPolicy<'static>);
// ★ Same claim as `ObjectPolicy`'s below, one seat earlier in the chain: this one holds a
// second handle onto the same object model and the FSM holds it across vCPU threads.
kayfabe_util::assert_send!(PublicationObserver);
// ★ `ObjectPolicy` OWNS its `Gpu`, so this asserts something `GraphPolicy`'s bound cannot:
// that the whole object model — spine, procs, isolate factory — is `Send` when it is moved
// into a policy the FSM holds across vCPU threads. The `Gpu` behind a `&mut` was already
// somebody else's problem; here it is this type's.
kayfabe_util::assert_send!(ObjectPolicy);
