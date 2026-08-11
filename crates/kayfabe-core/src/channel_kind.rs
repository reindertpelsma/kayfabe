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

impl GuestChannelKind {
    /// Every kind, so a gate can quantify over the enum rather than over a hand-written
    /// list that shrinks in one place with nothing going red.
    pub const ALL: [GuestChannelKind; 2] =
        [GuestChannelKind::Emulated, GuestChannelKind::Passthrough];

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
        }
    }

    /// The name a diagnostic prints. Exhaustive by construction.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            GuestChannelKind::Emulated => "emulated",
            GuestChannelKind::Passthrough => "passthrough",
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
    #[test]
    fn each_guest_kind_permits_exactly_one_host_kind_and_no_two_share_it() {
        let mut seen: Vec<HostChannelKind> = Vec::new();
        for k in GuestChannelKind::ALL {
            let h = k.hosted_by();
            assert!(
                !seen.contains(&h),
                "★ {k} and a previous guest kind both map to {h}. The map is meant to be \
                 INJECTIVE: the moment two guest kinds share a host kind, \
                 `forwarding_plane_owns_ce`'s `hosted_by(..) == Shadow` term stops \
                 separating anything and every call site still compiles."
            );
            seen.push(h);
        }
        assert_eq!(seen.len(), GuestChannelKind::ALL.len());
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

    /// ⊘ Names are distinct and stable — a diagnostic that printed one word for both
    /// kinds would make every boot log ambiguous about the exact split this module adds.
    #[test]
    fn every_kinds_name_is_distinct() {
        let g: Vec<&str> = GuestChannelKind::ALL.iter().map(|k| k.name()).collect();
        assert_ne!(g[0], g[1]);
        let h: Vec<&str> = HostChannelKind::ALL.iter().map(|k| k.name()).collect();
        assert_ne!(h[0], h[1]);
    }
}
