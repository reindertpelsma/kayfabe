//! ★★★★★ **THE TWO KINDS A CHANNEL HAS** — the owner's 2026-08-11 split, as types.
//!
//! > *"I think you first need to make the distinction of any channel we present to the
//! > guest, has 2 types: **passthrough** (unprivileged userspace) / **emulated**
//! > (privileged kernel); and any channel we use from the host allocated through ioctls
//! > also have 2 types: **passthrough** (unprivileged guest userspace channels, isolated)
//! > / **managed** (usually scratchpad channels, does not have to be isolated). I would
//! > ensure this abstraction is clear."*
//!
//! # ⊘⊘ LEAD WITH WHAT THIS MODULE REFUTES ABOUT THE BRIEF THAT ASKED FOR IT
//!
//! **The two are not independent axes, and modelling them as two fields would make three
//! quarters of the product space representable and wrong.** The owner's own model states
//! the coupling in the same breath as the split: *"Real GPU work derived from a kernel
//! command runs on a separate scratchpad channel with different VAs, ours end to end."*
//! That is a **function**, not a second free choice —
//!
//! | guest-facing kind | host channel that may back it |
//! |---|---|
//! | [`GuestChannelKind::Emulated`] | [`HostChannelKind::Scratchpad`] — ours end to end |
//! | [`GuestChannelKind::Passthrough`] | [`HostChannelKind::Shadow`] — that guest process's, isolated |
//!
//! — so **two of the four cells are uninhabited**, and the pair
//! `(Emulated, Shadow)` — a guest-*kernel* channel whose host backing is a guest
//! process's isolated channel — is exactly the confused deputy `#14` was designed out
//! of. It is therefore not represented and not checked: [`GuestChannelKind::hosted_by`]
//! is the **only** way to obtain a [`HostChannelKind`] for a channel that has a guest
//! side, and it is total. `prefer unrepresentability over a runtime check`, taken
//! literally.
//!
//! ⊘ The host kind is nonetheless its own type and not a method's return alias, because
//! it is inhabited by channels with **no guest side at all** — the isolate's own
//! executor channel (`kayfabe_isolate_host`'s `alloc_channel_for_isolate` over an
//! `ExecutorVas`), which no guest kind maps to and which is [`HostChannelKind::Scratchpad`]
//! by construction. The set of host kinds is not the image of `hosted_by`.
//!
//! # ★ Where this already existed, and in what form — measured, so nothing is re-derived
//!
//! `[measured 2026-08-11, `git grep` from the consuming crates]` the guest-facing split
//! is **already a dated owner ruling and a design rule**, and was already the *routing*
//! reality. What it was not is a declared per-channel fact:
//!
//! - `docs/design/ce_executor_tree.md` (owner, 2026-08-07) §*"Scope: this governs
//!   KERNEL-originated CE only"* — *"Guest **userspace** pushbuffers are mapped straight
//!   into the GPU and are **passthrough** — we do not inspect them."*
//! - `docs/design/execution_plane.md` §2.3 — *"Userspace channels are non-privileged …
//!   **The parser runs only where the core is already the mediator:** on the
//!   kernel/CeUtils/scrubber channels (the `system` `Proc`)."*
//! - [`crate::project::SYSTEM_ANCHOR`] / [`crate::project::Boundaries::system`] — the
//!   projection has always separated *every declared kernel client* from the user
//!   components, and `Boundaries::by_vchid`'s own doc already says the owning component
//!   *"may be `SYSTEM_ANCHOR` (**a guest-kernel channel**)"*.
//!
//! ⇒ This module adds **no new fact**. It gives the fact that was already being
//! re-derived at each consumer a **name, one derivation, and a carrier**, so that a
//! consumer reads it instead of re-computing it. The cost of it having been re-derived
//! is on the record: `kayfabe_qemu_raw::shim::forwarding_plane_owns_ce`'s system-proc
//! term is this axis inlined into one gate, and its **absence cost 12 boots** of
//! `RmInitAdapter failed! (0x25:0x65:1249)` before `6fcedac`.
//!
//! # ⚠ NAMING — two of the owner's four words are already taken in this tree
//!
//! - ⊘ **`Managed` is NOT used**, though it is the owner's word for the host side.
//!   *"GSP-**managed**"* is load-bearing house vocabulary meaning **`hVASpace == 0`** —
//!   *the channel declared no VA space* — and it is written on
//!   [`crate::gpu::Channel::vas_pdb`], the **sibling field** of the one this kind lands
//!   next to, as well as on `ChannelFacts::vas_pdb`, `AllocFacts::h_vaspace` and
//!   `kayfabe_rt::ceutils`. A `Managed` arm one line from a `vas_pdb` doc that says
//!   "GSP-managed" is a `same_flag_opposite_polarity` waiting to happen. (Second
//!   collision: `kayfabe_arch`'s *managed memory* = `cudaMallocManaged`.) The arm is
//!   [`HostChannelKind::Scratchpad`] — also the owner's own word in the same sentence.
//! - ★ **`Shadow` is the house word for the other host arm**, not an invention: `rm.rs`
//!   already says *"`GP_PUT` is the one 32-bit cursor **a shadow channel** exists to
//!   advance"*, *"on a **shadow channel** they are the **guest's** and not this file's
//!   constants"*, and `kayfabe_rt`'s `DoorbellRoute::HostGr` says a GR doorbell *"still
//!   needs a host channel that **SHADOWS** the guest's"*. Introducing "passthrough" for
//!   that same object would have given one concept two names.
//! - `Passthrough` is kept for the **guest** side, where it is both the owner's word and
//!   `ce_executor_tree.md`'s. It does not collide: `kayfabe_vmm_qemu`'s `Tier::Passthrough`
//!   is a KVM memslot tier and `kayfabe_abi`'s `PassthroughRule` is a control-command
//!   forwarding rule; neither is a channel and neither shares this type name.
//! - ⊘ **"Axis A" / "Axis B" are deliberately never used here.** Both are already defined
//!   and load-bearing across ~40 doc references: Axis A = the **guest driver version**
//!   (`kayfabe-abi`), Axis B = the **GPU architecture** (`kayfabe-arch`). These two are
//!   *"the guest-facing kind"* and *"the host-facing kind"*, spelled out every time.

/// ★★★★★ **What a channel we PRESENT TO THE GUEST is** — the privilege level of the
/// software on the other side of it.
///
/// Derived **once**, at [`crate::project::ProcBoundary::channel_kind`], from the
/// component the channel's owning client namespace projects into; carried on
/// [`crate::gpu::Channel::kind`]; read by consumers. ⊘ Never re-derived at a consumer —
/// that is the whole point, and `two_projections_of_one_fact_disagreeing` is what
/// re-derivation costs in this tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GuestChannelKind {
    /// **EMULATED** — the guest's *privileged kernel* drives it.
    ///
    /// We own its USERD, ring, pushbuffer and semaphore in the emulated framebuffer, and
    /// the guest kernel believes it is driving a real GPU while it is driving us. Real
    /// GPU work derived from a command on such a channel runs somewhere else entirely —
    /// on a [`HostChannelKind::Scratchpad`] channel with different VAs, ours end to end.
    ///
    /// ⚠ **It is not "a channel we understand".** `execution_plane.md` §2.3's rule is
    /// that the pushbuffer parser runs *only* here, because here the core is already the
    /// mediator; that is a licence to inspect, not a claim of a completed decode.
    Emulated,
    /// **PASSTHROUGH** — *unprivileged guest userspace* drives it.
    ///
    /// `ce_executor_tree.md` (owner, 2026-08-07): *"Guest userspace pushbuffers are
    /// mapped straight into the GPU and are passthrough — we do not inspect them, and CE
    /// there is always real."* Its host backing is a [`HostChannelKind::Shadow`] channel
    /// inside **that guest process's own isolate** — per-process separation is `#14`'s
    /// proven fix and it is what makes this arm safe to not inspect.
    Passthrough,
    /// ★★★★★ **TRANSLATED** — the *guest kernel* drives it, and **every entry is inspected
    /// and rewritten before hardware sees it**.
    ///
    /// > **Owner, 2026-09-19:** *"a virtual channel always has a host channel backing it like
    /// > a passthrough one; the ring of the host channel does NOT reside on GPGA, it's instead
    /// > in scratchpad va outside it… each entry is a guest entry, but each entry has the
    /// > ability to translate before its being forwarded."*
    ///
    /// # ⊘ Why two kinds was the wrong number
    ///
    /// [`Self::Emulated`] and [`Self::Passthrough`] answer *"may hardware run these bytes
    /// untouched?"* at **channel** granularity. For a guest-kernel copy-engine channel that is
    /// the wrong granularity: privilege varies **per entry**. `ogkm-580:
    /// channel_utils.c:1053-1091` emits `LAUNCH_DMA.SRC_TYPE=_VIRTUAL` when the channel was
    /// built with `bUseVasForCeCopy` and `_PHYSICAL` otherwise — and a second emitter,
    /// `ogkm-580: mem_utils_gm107.c:2098-2100`, hard-codes `_PHYSICAL` and never consults the
    /// flag. **One driver, one part, two answers.**
    ///
    /// ⊘ A `_PHYSICAL` operand **bypasses the MMU**, so passing it through unmediated would
    /// hand the engine a guest-authored number as a **host** physical address. That is the
    /// escape class, not the guest-internal question §45 defers to ogkm.
    ///
    /// # ★★★ What makes it safe, and both are STRUCTURAL
    ///
    /// **(a) The VAS is the bound.** Entries are rewritten into a VA space that maps *only*
    /// the single store, so a translated operand **cannot** name memory outside the guest's
    /// own. There is no validation to forget.
    ///
    /// **(b) The completion stays hardware's.** We translate the *address* of the semaphore
    /// release and never produce its *value* — the guest spins on a word a real engine wrote.
    /// That is the line between this and `citing_the_c_where_it_forges`.
    ///
    /// ⊘ It is §39(a)-shaped: **copy, then check, then use**. The guest mutating its own
    /// pushbuffer after the copy changes nothing.
    ///
    /// ⚠ **Not a licence to translate everything.** The per-entry copy is real cost (`w315`
    /// measured 86.7 ms/launch when per-launch work ran inline). This kind is for channels
    /// whose privilege cannot be decided per channel — the CeUtils scrub and kernel CE. Hot
    /// userspace channels stay [`Self::Passthrough`].
    ///
    /// See `docs/design/the_three_channel_kinds.md`.
    Translated,
}

/// ★★★★★ **What a channel we ALLOCATE ON THE HOST through RM ioctls is** — whose work it
/// carries, and therefore whether it must be isolated per guest process.
///
/// ⊘ **Not a statement about the ring's provenance.** See
/// [`HostChannelKind::Shadow`] for the measured divergence from
/// `kayfabe_isolate_host`'s `RingOwner`, which answers a strictly finer question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HostChannelKind {
    /// **SHADOW** — this host channel exists to carry **one guest passthrough channel's**
    /// work, and lives in that guest process's own isolate.
    ///
    /// # ⊘⊘ This is NOT `kayfabe_isolate_host`'s `RingOwner`, and collapsing them would
    /// rename a true statement into a false one
    ///
    /// `RingOwner::{Ours, HandedIn}` answers *"did this file allocate the object the
    /// GPFIFO lives in, and must this file therefore free it"*. Its write set is three
    /// sites in one file — the alloc, the CPU map, the teardown. This kind's write set is
    /// in the **core**, one hop from the guest's own `NV01_ROOT` declaration.
    ///
    /// `[measured 2026-08-11, `git grep` from the consuming crates]` the two **disagree
    /// today, on every channel that exists**: `RmBackend::alloc_channel` — the only
    /// channel verb the core can reach — lowers to `RingSource::Ours(None)`
    /// unconditionally, so **every** host channel is `RingOwner::Ours`, including every
    /// `Shadow` one. The `RingSource::Guest` arm has exactly one caller in the workspace
    /// and it is the `rmladder` R31 **diagnostic binary**, not the core. ⇒ ring
    /// provenance is a *detail beneath* this kind, on its way to agreeing with it, and
    /// naming today's `Ours` "scratchpad" would be false of the majority of channels.
    ///
    /// ⊘⊘ **SUPERSEDED 2026-09-09 (w393).** The paragraph above is a dated measurement and
    /// is no longer true: `RmBackend::alloc_channel_declared` — reached from
    /// `kayfabe_fwd::plan_channel_birth` at the guest's own channel alloc — lowers a
    /// `Passthrough` channel to `RingSource::Guest(..)` with the adoption **mandatory by
    /// type**, and `plan_doorbell` refuses to birth a `Passthrough` channel over ours by
    /// name (`FwdFault::PassthroughDoorbellBirth`). ⇒ `Shadow` and `RingOwner::HandedIn`
    /// now agree on every channel born at the alloc; the engine-object latch still admits
    /// `Ours` for a `Passthrough` channel whose ring was not adoptable (cup3's measured
    /// path), which is the one remaining disagreement and is recorded, not hidden.
    Shadow,
    /// **SCRATCHPAD** — ours end to end. No guest channel is bound to it and the guest
    /// cannot name its address space.
    ///
    /// Two populations reach this arm and only one of them has a guest side at all:
    /// the host backing of a [`GuestChannelKind::Emulated`] channel (the owner's *"a
    /// separate scratchpad channel with different VAs"*), and the isolate's **own**
    /// executor channel, allocated over an `ExecutorVas` — *"a host address space NO
    /// GUEST CHANNEL IS EVER BOUND TO"* — which no guest kind maps to.
    ///
    /// ⚠ *"Does not have to be isolated"* (the owner's phrase) is about **us**, never
    /// about tenants. `ce_executor_tree.md`'s scratch-VAS ruling is explicit the other
    /// way: *"⊘ Scope it PER-ISOLATE, not per-device. A shared scratch VAS would be a
    /// cross-tenant channel."* Nothing here licenses sharing one across guests.
    Scratchpad,
}

/// ★★★★★ **What the DOORBELL TRAP may do, by kind** — the owner's ruling of 2026-08-11,
/// carried in the abstraction rather than in one implementation.
///
/// > *"The emulated arm must not block the vCPU"* → owner: **"yes schedule work
/// > asynchronously not during the trap"**.
///
/// # ⊘⊘ WHAT THIS TYPE IS, AND — SAY IT FIRST — WHAT IT IS NOT
///
/// It is a **declared contract**, total in [`GuestChannelKind`]. It is **not an
/// enforcement**, and the reason is precise rather than an omission:
///
/// # ⊘⊘⊘ CORRECTED `[w323, 2026-08-14]` — THE PARAGRAPH BELOW IS TRUE OF A **TYPE ALONE**
///
/// It is now **built**, and it is built the way this very paragraph predicted: a witness
/// token whose obtaining is checked. [`kayfabe_util::trapwitness::OffTrap`] is required by
/// `kayfabe_isolate::Worker::execute` — **the one door to a host RM verb** — and it carries
/// thread identity as a composition of three facts, no two of which suffice:
/// **(1)** a private field with no struct literal ⇒ minted by the constructor;
/// **(2)** `OffTrap::claim` panics when `trapwitness::in_trap()` ⇒ the minting thread was
/// off-trap; **(3)** `!Send` + `!Sync` ⇒ it is **still that thread**.
///
/// ★ The paragraph's own condition for building it — *"that token is NOT built here because
/// it would have nothing to guard: the emulated arm's handler is not a separable object
/// yet"* — was **precisely right**, and `w323` is the rung that gives it something to guard
/// (`kayfabe_device::pubqueue`, the deferred publication lane). ⇒ this is a ruling that
/// **expired on schedule**, not one that was wrong.
///
/// ⊘ **The ceiling is unchanged and is stated here so nobody over-credits it:**
/// `OffTrap::at_a_host_verb` still mints a **counted** exception on a trap thread, so the
/// guarantee remains `VerbPlan::gated_doorbell`'s — *omission → commission* — plus a
/// census. See `docs/design/publication_off_the_bql.md` §7.
///
/// ⇒ **Rust cannot express *"this call is not on the vCPU thread"*.** Thread identity is
/// not in any type here, and the only shape that would carry it is a witness token whose
/// constructor lives on the worker side — the `ExecutorVas` / `DeclaredCompletion` idiom
/// this tree already uses (private field, no public constructor, pinned by a compile-fail
/// UI test). ⊘ That token is not built here **because it would have nothing to guard**:
/// the emulated arm's handler is not a separable object yet, so a witness parameter would
/// be an orphan — a `pub` item with no caller, which `the_orphan_gate` refuses and which
/// `alloc_engine_object`'s own docs warn about one method up.
///
/// ★ And the tree has already written down exactly how strong this class of guarantee can
/// get. `VerbPlan::gated_doorbell`: *"Rust's privacy unit is the crate, so 'only
/// `kayfabe-fwd` may call this' is not expressible in the type system. What changed is the
/// failure mode: bypassing the gate is no longer **omission** … but **commission**."* A
/// thread-affinity witness would buy the same thing and no more.
///
/// # ★ The mechanism the ruling asks for EXISTS and is NAMED — do not rebuild it
///
/// `kayfabe_rt::completion_watch`'s module docs state the split as a table, and it is the
/// shape the emulated arm needs:
///
/// | phase | thread |
/// |---|---|
/// | **declare** — decode the operand, resolve the VA once, register | the **vCPU**, inside locks it already holds |
/// | **observe** — read the word, compare, verdict | the **reactor** thread |
///
/// — *"which is what keeps the vCPU path from gaining a tenth blocking site: declaring is a
/// `BTreeMap` insert under a leaf mutex and nothing else."*
///
/// # ⊘ COMPLETION OBSERVATION IS **NOT** A PROPERTY OF THE CHANNEL, and putting it here
/// would be the mistake this module was written to stop
///
/// The ruling asks whether the abstraction can distinguish *"completion is observed by
/// polling"* from *"completion must be announced"*. It can not, **and must not**:
/// `[measured 2026-08-11]` that fact is `AWAKEN_ENABLE`, `D[20:20]` of the guest's own
/// `SET_REPORT_SEMAPHORE_D`, decoded per **submission** into
/// `kayfabe_rt::completion_watch`'s `CompletionDecl::awaken`. One channel may carry
/// submissions with either value; a per-channel field would be a *third* projection of a
/// per-submission fact, disagreeing with the guest's own words on whichever submission
/// went the other way.
///
/// ★ What IS true, and is the finding worth carrying: `awaken` is **decoded, printed, and
/// branched on by nothing** — `git grep` finds one decode, two prints, one test assertion
/// and **zero** conditions. That is the same shape as the axis this module exists for:
/// *"the record existed, was printed, was correct, and no code read it"*. The polled/
/// announced split therefore does not need inventing; it needs a **decision point**, and
/// that belongs where `awaken` already lives, one layer down from a channel's kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TrapContract {
    /// **RING AND RETURN** — resolve the guest token to its host token, ring it, return to
    /// VM entry.
    ///
    /// ⊘ **No inspection and no work**, which is not a performance choice: a passthrough
    /// channel's pushbuffer is *"mapped straight into the GPU"* and the whole correctness
    /// argument for that arm is that we did not touch it (`ce_executor_tree.md` §Scope).
    /// Bounded by construction, so non-blocking needs no separate argument.
    RingAndReturn,
    /// **SCHEDULE AND RETURN** — hand the channel's handler to a worker and return to VM
    /// entry. ⚠ The handler **must not run on the vCPU thread**.
    ///
    /// The emulated arm is the one that *does* work — decode the ring, translate the
    /// intent, run it somewhere real — and every part of that is unbounded in a way the
    /// trap is not allowed to be. ⚠ `l1_concurrency.md` R1 counts an `eprintln!` as a
    /// blocking site; two shipping violations were found under that rule before.
    ScheduleAndReturn,
}

impl TrapContract {
    /// Every contract — see [`GuestChannelKind::ALL`].
    pub const ALL: [TrapContract; 2] =
        [TrapContract::RingAndReturn, TrapContract::ScheduleAndReturn];

    /// ★ **May the work this contract governs run on the trapping thread?**
    ///
    /// The one predicate the ruling reduces to, so a caller asks it by name instead of
    /// matching an enum and deciding for itself what each arm implies.
    #[must_use]
    pub const fn may_run_on_the_vcpu_thread(self) -> bool {
        match self {
            // ⊘ `true` here is not a licence to do work: this arm's *whole content* is a
            // token lookup and a doorbell write. It says the trap may finish, not that it
            // may compute.
            TrapContract::RingAndReturn => true,
            TrapContract::ScheduleAndReturn => false,
        }
    }

    /// The name a diagnostic prints.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            TrapContract::RingAndReturn => "ring-and-return",
            TrapContract::ScheduleAndReturn => "schedule-and-return",
        }
    }
}

impl std::fmt::Display for TrapContract {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl GuestChannelKind {
    /// Every kind, so a gate can quantify over the enum rather than over a hand-written
    /// list that shrinks in one place with nothing going red.
    pub const ALL: [GuestChannelKind; 3] =
        [
            GuestChannelKind::Emulated,
            GuestChannelKind::Passthrough,
            GuestChannelKind::Translated,
        ];

    /// ★★★★★ **The owner's 2026-08-11 ruling, as a total function** — what the doorbell
    /// trap is permitted to do for a channel of this kind. See [`TrapContract`] for what it
    /// enforces (nothing) and why that is stated rather than papered over.
    #[must_use]
    pub const fn trap_contract(self) -> TrapContract {
        match self {
            GuestChannelKind::Emulated => TrapContract::ScheduleAndReturn,
            GuestChannelKind::Passthrough => TrapContract::RingAndReturn,
            // ⊘ **`ScheduleAndReturn`, the same contract as `Emulated`, and for the same
            // reason** — not because the two kinds are alike downstream, but because the
            // vCPU's obligation is identical: §41 lets an MMIO write update a queue and wake,
            // and a Translated doorbell must do exactly that. The entry has to be READ,
            // COPIED and REWRITTEN before hardware sees it, and none of that may happen in the
            // trap. ★ The vCPU deliberately cannot tell Translated from Emulated, which is the
            // property `route_of_engine`'s docs call "the vCPU has no business knowing which".
            GuestChannelKind::Translated => TrapContract::ScheduleAndReturn,
        }
    }

    /// ★★★ **The owner's model, as a total function: which host channel kind may back a
    /// channel of this guest kind.**
    ///
    /// This is the whole of the coupling between the two kinds, and it is the reason
    /// they are not two independent fields — see the module docs' table. Being the only
    /// route from a guest kind to a host kind is what makes `(Emulated, Shadow)`
    /// unrepresentable rather than merely forbidden.
    ///
    /// ⊘ **A permission, not an observation.** It says which host channel is *allowed*
    /// to carry this guest channel's work; it does not claim one has been allocated.
    /// `[measured]` an `Emulated` channel's CE work is served today by the shell's own
    /// CPU executor and has **no** host channel at all — `hosted_by` still answers
    /// `Scratchpad`, because the question is *"what could host it"* and the answer is
    /// *"never a guest process's isolated channel"*.
    #[must_use]
    pub const fn hosted_by(self) -> HostChannelKind {
        match self {
            GuestChannelKind::Emulated => HostChannelKind::Scratchpad,
            GuestChannelKind::Passthrough => HostChannelKind::Shadow,
            // ★★★ **`Shadow`, NOT `Scratchpad`** — a Translated channel **always has a host
            // channel backing it, like a passthrough one** (owner, 2026-09-19). Its work is
            // one guest channel's and must be isolated as such; what differs from
            // `Passthrough` is not WHERE the host channel lives but that **its ring is ours,
            // in scratchpad VA outside GPGA**, so the guest cannot author what hardware
            // fetches. ⊘ `Scratchpad` would say the work is the VMM's own, which is exactly
            // what `Emulated` means and exactly what this kind is not.
            GuestChannelKind::Translated => HostChannelKind::Shadow,
        }
    }

    /// The name a diagnostic prints. Exhaustive by construction.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            GuestChannelKind::Emulated => "emulated",
            GuestChannelKind::Passthrough => "passthrough",
            GuestChannelKind::Translated => "translated",
        }
    }
}

impl HostChannelKind {
    /// Every kind — see [`GuestChannelKind::ALL`].
    pub const ALL: [HostChannelKind; 2] = [HostChannelKind::Shadow, HostChannelKind::Scratchpad];

    /// The name a diagnostic prints.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            HostChannelKind::Shadow => "shadow",
            HostChannelKind::Scratchpad => "scratchpad",
        }
    }
}

impl std::fmt::Display for GuestChannelKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl std::fmt::Display for HostChannelKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ★★★ **THE COUPLING, stated as the property the module exists for.** Every guest
    /// kind has exactly one permitted host kind, and the two guest kinds do not share
    /// one — an injective total map. If both arms ever answered the same host kind the
    /// distinction would have stopped doing work while every call site still compiled.
    /// ⊘⊘⊘ **w806 — THIS TEST ASSERTED INJECTIVITY, AND INJECTIVITY WAS A PROXY.**
    ///
    /// It read *"no two guest kinds share a host kind"*, justified as: *"the moment two guest
    /// kinds share a host kind, `forwarding_plane_owns_ce`'s `hosted_by(..) == Shadow` term
    /// stops separating anything."* That was exactly right while the map was 2 → 2.
    ///
    /// ⊘ [`GuestChannelKind::Translated`] makes it 3 → 2, **deliberately**: its host channel
    /// carries one guest channel's work and lives in that guest process's own isolate, which
    /// is what [`HostChannelKind::Shadow`] means — *"translated still uses isolates as
    /// normal"* (owner, 2026-09-19). What differs from `Passthrough` is the RING (ours,
    /// outside GPGA), and `HostChannelKind` does not classify rings; it classifies **whose
    /// work it carries and therefore whether it must be isolated per guest process.**
    ///
    /// ⚠ And the term the old test was protecting is **not** weakened by the collision — it is
    /// corrected. `forwarding_plane_owns_ce` asks *"does the forwarding plane own this CE
    /// work?"*, and for a Translated channel hardware **does** own it; we only rewrite the
    /// operands first. `Shadow` including Translated is the right answer, not a lost
    /// distinction.
    ///
    /// ⇒ What is asserted instead is the property injectivity was standing in for, and it is
    /// the safety-relevant one: **every host channel that carries a GUEST channel's work is
    /// per-process isolated, and only a channel that is the VMM's own may be `Scratchpad`.**
    #[test]
    fn only_a_vmm_owned_channel_may_escape_per_process_isolation() {
        for k in GuestChannelKind::ALL {
            let h = k.hosted_by();
            match k {
                // ⊘ The VMM's own work: there is no guest channel to isolate from anything.
                GuestChannelKind::Emulated => assert_eq!(
                    h,
                    HostChannelKind::Scratchpad,
                    "★ an emulated channel runs OUR function bodies; hosting it in a guest \
                     process's isolate would attribute the VMM's work to that process"
                ),
                // ★★★ Both carry ONE GUEST CHANNEL'S work ⇒ both must be isolated per guest
                // process. `#14`'s proven fix, and the reason a hostile guest process cannot
                // reach another's channel.
                GuestChannelKind::Passthrough | GuestChannelKind::Translated => assert_eq!(
                    h,
                    HostChannelKind::Shadow,
                    "★★★★★ {k} carries one guest channel's work and MUST live in that guest \
                     process's own isolate. A `Scratchpad` here would put one guest process's \
                     submissions in a channel shared with every other — the exact separation \
                     `hostile_guest_isolation_is_the_value_proposition` sells"
                ),
            }
        }
        // ⊘ The map stays TOTAL and its image stays exactly the two host kinds — a third host
        // kind appearing without a guest kind reaching it would be an unreachable arm.
        let mut image: Vec<HostChannelKind> =
            GuestChannelKind::ALL.into_iter().map(|k| k.hosted_by()).collect();
        image.sort_unstable();
        image.dedup();
        assert_eq!(
            image.len(),
            HostChannelKind::ALL.len(),
            "★ every host kind must be reachable from some guest kind: image={image:?}"
        );
    }

    /// ⊘ **The uninhabited cell, named.** A guest-KERNEL channel must never be hosted by
    /// a `Shadow` — that is a guest process's isolated channel carrying the guest
    /// kernel's work, which is the confused deputy `#14` designed out. It is
    /// unrepresentable because `hosted_by` is the only constructor of a host kind for a
    /// channel with a guest side; this test is the statement of *what* is unrepresentable,
    /// which a type cannot say out loud.
    #[test]
    fn an_emulated_channel_is_never_hosted_by_a_guest_processs_shadow() {
        assert_eq!(
            GuestChannelKind::Emulated.hosted_by(),
            HostChannelKind::Scratchpad
        );
        assert_ne!(
            GuestChannelKind::Emulated.hosted_by(),
            HostChannelKind::Shadow,
            "★ the guest KERNEL's channel is hosted by a channel belonging to a guest \
             PROCESS's isolate. `l1_concurrency.md` §12.26: the SYSTEM proc has no data \
             plane and its work is FORGED, never forwarded."
        );
    }

    /// ★ Non-vacuity for the test above: the other arm really does reach `Shadow`, so
    /// *"nothing is ever hosted by a Shadow"* would fail here.
    #[test]
    fn a_passthrough_channel_is_hosted_by_a_shadow() {
        assert_eq!(
            GuestChannelKind::Passthrough.hosted_by(),
            HostChannelKind::Shadow
        );
    }

    /// ★★★★★ **THE OWNER'S 2026-08-11 RULING, both halves, and the second is the load-
    /// bearing one.** Exactly one kind may finish its work at the trap, and it is the
    /// PASSTHROUGH one; the emulated arm must schedule.
    ///
    /// ⊘ Written as *"exactly one"* rather than as two literal assertions on purpose: a
    /// contract that let **both** kinds run inline would satisfy a pair of one-sided
    /// assertions if somebody flipped only the arm they were editing.
    #[test]
    fn exactly_one_kind_may_finish_its_work_on_the_vcpu_thread_and_it_is_the_passthrough_one() {
        let inline: Vec<GuestChannelKind> = GuestChannelKind::ALL
            .into_iter()
            .filter(|k| k.trap_contract().may_run_on_the_vcpu_thread())
            .collect();
        assert_eq!(
            inline,
            vec![GuestChannelKind::Passthrough],
            "★ the set of kinds whose trap may finish inline is {inline:?}, not exactly \
             `[Passthrough]`. Owner, 2026-08-11: *\"yes schedule work asynchronously not \
             during the trap\"* — an emulated channel's handler decodes a ring and runs \
             work, and `l1_concurrency.md` R1 forbids every part of that on the vCPU path \
             (it counts an `eprintln!`)."
        );
        assert_eq!(
            GuestChannelKind::Emulated.trap_contract(),
            TrapContract::ScheduleAndReturn
        );
        assert_eq!(
            GuestChannelKind::Passthrough.trap_contract(),
            TrapContract::RingAndReturn
        );
    }

    /// ⊘⊘ **w806 — THE "DIFFERENT CONTRACTS" TEST WAS ALSO AN INJECTIVITY PROXY.**
    ///
    /// It asserted no two kinds share a `TrapContract`. True while there were two kinds and
    /// two contracts; **false, and rightly, now that there are three kinds.**
    /// [`GuestChannelKind::Translated`] shares `ScheduleAndReturn` with
    /// [`GuestChannelKind::Emulated`] **because the vCPU's obligation is identical** — §41
    /// lets an MMIO write update a queue and wake, and both kinds need exactly that. The vCPU
    /// deliberately cannot tell them apart; what differs happens later, on the worker.
    ///
    /// ⇒ The non-degeneracy this was really protecting is that the contract **partitions**
    /// rather than labels: both contracts must be reached, and the inline one must stay a
    /// singleton. A collapse in either direction still lands here.
    #[test]
    fn the_trap_contract_partitions_the_kinds_and_the_inline_class_is_a_singleton() {
        let mut image: Vec<TrapContract> =
            GuestChannelKind::ALL.into_iter().map(|k| k.trap_contract()).collect();
        image.sort_unstable();
        image.dedup();
        assert_eq!(
            image.len(),
            TrapContract::ALL.len(),
            "★ every trap contract must be reached by some kind, or an arm is unreachable \
             and the enum is claiming a distinction nothing makes: image={image:?}"
        );
        let inline: Vec<GuestChannelKind> = GuestChannelKind::ALL
            .into_iter()
            .filter(|k| k.trap_contract() == TrapContract::RingAndReturn)
            .collect();
        assert_eq!(
            inline,
            vec![GuestChannelKind::Passthrough],
            "★★★★★ exactly ONE kind may be rung inline and it is the passthrough one: \
             {inline:?}. A Translated or Emulated channel's entry must be read, copied and \
             rewritten before hardware sees it, and §41 forbids every part of that in the trap"
        );
    }

    /// ★★ **The contracts agree with the host backings** — coherence between two independent
    /// `match`es, so an edit to either alone lands here.
    ///
    /// ⊘⊘ **w806 — THE BICONDITIONAL IS NOW A ONE-WAY IMPLICATION, AND THAT IS THE TRUTH.**
    /// It read `ScheduleAndReturn ⟺ Scratchpad`. [`GuestChannelKind::Translated`] is
    /// `ScheduleAndReturn` **and** `Shadow`: we must schedule it because its entries need
    /// translating, not because we own its backing. ⇒ owning the backing still **implies**
    /// scheduling, but scheduling no longer implies owning the backing — and the reverse
    /// direction, *"rung inline ⇒ hosted by a Shadow"*, is the one that carries safety.
    #[test]
    fn scheduling_is_required_exactly_where_the_host_backing_is_our_own_scratchpad() {
        for k in GuestChannelKind::ALL {
            // ⊘ Owning the backing IMPLIES scheduling: our channel is the one we must drive,
            // and driving it is what may not happen on the vCPU thread.
            if k.hosted_by() == HostChannelKind::Scratchpad {
                assert_eq!(
                    k.trap_contract(),
                    TrapContract::ScheduleAndReturn,
                    "★ {k} runs on a channel of OURS and must be driven off the vCPU thread"
                );
            }
            // ★★★ And the direction that carries safety: a doorbell rung INLINE, with no
            // worker between the guest's store and hardware, is only ever legal on a channel
            // that is one guest process's own. Anything else would put un-inspected bytes on
            // a channel the guest does not exclusively own.
            if k.trap_contract() == TrapContract::RingAndReturn {
                assert_eq!(
                    k.hosted_by(),
                    HostChannelKind::Shadow,
                    "★★★★★ {k} may be rung INLINE on the vCPU, so nothing inspects its bytes \
                     before hardware fetches them. That is only ever safe on a channel that \
                     is one guest process's own"
                );
            }
        }
    }

    /// ⊘ Names are distinct and stable — a diagnostic that printed one word for both
    /// kinds would make every boot log ambiguous about the exact split this module adds.
    #[test]
    fn every_kinds_name_is_distinct() {
        let g: Vec<&str> = GuestChannelKind::ALL.iter().map(|k| k.name()).collect();
        assert_ne!(g[0], g[1]);
        let h: Vec<&str> = HostChannelKind::ALL.iter().map(|k| k.name()).collect();
        assert_ne!(h[0], h[1]);
        let t: Vec<&str> = TrapContract::ALL.iter().map(|k| k.name()).collect();
        assert_ne!(t[0], t[1]);
    }
}

#[cfg(test)]
mod translated_tests {
    use super::*;

    /// ★★★★★ **w806 — THE KIND THAT CARRIES THE GUEST'S USERD AND OUR RING.**
    ///
    /// The three facts that define [`GuestChannelKind::Translated`] and that no other test
    /// states together. Each is a design commitment from
    /// `docs/design/the_three_channel_kinds.md`, and each has a different consequence if it
    /// silently flips.
    #[test]
    fn translated_is_isolated_like_passthrough_and_scheduled_like_emulated() {
        let t = GuestChannelKind::Translated;

        // (1) ISOLATED PER GUEST PROCESS. It carries one guest channel's work.
        // ⊘ A `Scratchpad` here would put one guest process's submissions on a channel shared
        // with every other — owner, 2026-09-19: *"translated still uses isolates as normal"*.
        assert_eq!(t.hosted_by(), HostChannelKind::Shadow);
        assert_eq!(t.hosted_by(), GuestChannelKind::Passthrough.hosted_by());

        // (2) NEVER RUNG INLINE. Its entries must be read, copied and rewritten first, and
        // §41 forbids all of that in the trap.
        assert_eq!(t.trap_contract(), TrapContract::ScheduleAndReturn);
        assert_eq!(t.trap_contract(), GuestChannelKind::Emulated.trap_contract());
        assert!(!t.trap_contract().may_run_on_the_vcpu_thread());

        // (3) IT IS ITS OWN KIND. ⊘ The whole rung exists because a channel-level
        // Emulated/Passthrough answer is the wrong granularity: a guest-kernel CE channel's
        // privilege varies PER ENTRY (`ogkm-580: channel_utils.c:1053-1091` emits `_VIRTUAL`
        // or `_PHYSICAL` for the same channel, and `mem_utils_gm107.c:2098` hard-codes
        // `_PHYSICAL` and never consults the flag).
        assert_ne!(t, GuestChannelKind::Emulated);
        assert_ne!(t, GuestChannelKind::Passthrough);
        assert_eq!(t.name(), "translated");
    }

    /// ⊘ **The kind is in `ALL`, because every gate in this crate quantifies over it.**
    ///
    /// `gates_quantified_over_a_list.md`: a check written against named constants keeps
    /// passing when a member is added. A `Translated` missing from `ALL` would make every
    /// such gate silently exclude it — the shape that let five arms default to a superseded
    /// architecture in w760.
    #[test]
    fn translated_is_reachable_from_the_quantified_list() {
        assert!(GuestChannelKind::ALL.contains(&GuestChannelKind::Translated));
        assert_eq!(GuestChannelKind::ALL.len(), 3);
    }
}
