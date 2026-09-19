//! ★★★★★ **EVERY SELECTOR'S DEFAULT IS THE NEW DESIGN — ASSERTED, NOT REMEMBERED.**
//!
//! > **Owner, 2026-09-19:** *"this is like the tenth time that old code creeped in your
//! > reasoning of superseded. clearly mark those code for superseded/deletion later, and
//! > ensure they are frozen, same for the flags, ensure you debug against the new design flags
//! > and set they to default. Testing with old code is only for revert if the new one is
//! > genuinely broken. But often we switch because components are incompatible."*
//!
//! # ⊘⊘⊘ Why a GATE and not a document
//!
//! This defect has recurred ~10 times in one campaign, and every instance was found the same
//! expensive way: a boot behaved strangely, hours went into the behaviour, and the cause was a
//! default naming a superseded arm. A non-exhaustive list from this session alone:
//!
//! | arm | old default | what it cost |
//! |---|---|---|
//! | `KAYFABE_CE_EXECUTOR` | `local` | **no CE submission reached the GPU at all**; `try_ce_submission` claimed every doorbell by engine before the channel's kind was consulted (w797) |
//! | `KAYFABE_VAS_PUBLISH` | `off` (pre-w290) | `join_one_fb_leaf` never ran ⇒ *"no host object behind them"* ⇒ `CeExecutor::Ours ×10, HostCe ×0` (w809/w810) |
//! | pt-sweep skip | `true` | the guest's page tables were never re-read: `PT-DECODE drained=0` ×831 (w784/w810) |
//! | five arms at w760 | superseded | *"THE NEW DESIGN WAS UNREACHABLE BY DEFAULT"* |
//!
//! ⚠ **The owner's last clause is the load-bearing one:** *"often we switch because components
//! are incompatible."* An old default is not a conservative choice — it is a configuration the
//! rest of the system has moved away from, and debugging against it produces conclusions that
//! are true of nothing we ship. Three separate CE misattributions this session came from
//! exactly that.
//!
//! # What this gate does and does not claim
//!
//! ⊘ It does not claim the new arms are correct. It claims that **what a boot runs with no
//! environment set is the architecture we are developing**, so a measurement taken by default
//! describes the product. Superseded arms stay *spellable* (§42(d) — an opt-in may MOVE but
//! not disappear) and are **frozen**: kept to bisect against, never to develop on.
//!
//! ⚠ A red here after a deliberate flip is **the flip's own paperwork**, not a failure: update
//! the row and say in the commit why the new arm is the design. §42(e) — a test that reddens
//! on a default flip is reporting a hidden argument, and this file is where that argument goes.

use kayfabe_qemu_raw::shim::{
    CeExecutorChoice, DoorbellAsyncArm, FbJoinArm, GuestRingArm, IsolatePlane, VasPublishArm,
    ce_executor_from, doorbell_async_from, fb_join_from, guest_ring_from, isolate_plane_from,
    pt_sweep_skip_from, vas_publish_from,
};

/// ★★★ **The ledger.** One row per selector: absent ⇒ the new-design arm, with the superseded
/// arm named so a reader knows what is frozen rather than merely unused.
#[test]
fn absent_selects_the_new_design_for_every_arm() {
    // ⊘ `Stillborn` is the frozen arm: a plane that can issue no host verb at all.
    assert_eq!(
        isolate_plane_from(None),
        Ok(IsolatePlane::Real),
        "★ w763: with `stillborn` as the default the whole isolate plane was unreachable"
    );

    // ⊘ `Local` is FROZEN by §46: real kernel-channel work is never executed on the CPU.
    assert_eq!(
        ce_executor_from(None),
        Ok(CeExecutorChoice::Host),
        "★★★★★ w797/w800: with `local`, `local_ce_is_the_only_executor` is permanently true, \
         `try_ce_submission` claims every CE doorbell BY ENGINE before the channel's kind is \
         consulted, and NO copy-engine submission reaches the GPU"
    );

    // ⊘ `Off` is the pre-w290 arm — publication silent, `join_one_fb_leaf` never called.
    assert_eq!(
        vas_publish_from(None),
        Ok(VasPublishArm::Drain),
        "★★★★★ w809/w810: with `off` no framebuffer page gets a host object behind it, so \
         every CE span is graded `CeExecutor::Ours` and the copy plane never reaches hardware"
    );

    // ⊘ The skip's premise died with the trapped framebuffer window.
    assert_eq!(
        pt_sweep_skip_from(None),
        Ok(false),
        "★★★★★ w784/w810: the skip is keyed on a CPU witness the single store removed \
         (`PT-DECODE drained=0` ×831), so defaulting it ON means never re-reading the guest's \
         page tables on the architecture we ship"
    );

    assert_eq!(guest_ring_from(None), Ok(GuestRingArm::Ring));
    assert_eq!(fb_join_from(None), Ok(FbJoinArm::Shared));
    assert_eq!(doorbell_async_from(None), Ok(DoorbellAsyncArm::On));
}

/// ⊘ **Every frozen arm is still SPELLABLE** — §42(d): an opt-in may MOVE but may not silently
/// disappear. A frozen arm that stopped parsing would make a bisect impossible and would turn
/// *"revert if the new one is genuinely broken"* into an unavailable option.
#[test]
fn every_frozen_arm_can_still_be_named() {
    assert_eq!(ce_executor_from(Some("local")), Ok(CeExecutorChoice::Local));
    assert_eq!(vas_publish_from(Some("off")), Ok(VasPublishArm::Off));
    assert_eq!(pt_sweep_skip_from(Some("on")), Ok(true));
    assert_eq!(
        isolate_plane_from(Some("stillborn")),
        Ok(IsolatePlane::Stillborn)
    );
}

/// ⊘⊘ **A typo must REFUSE, never fall back to a default.** The reason is §42's and it is
/// measured: a misspelling that silently selected the control would make an evidence run and
/// its own control indistinguishable, and the control's expected result is *"nothing
/// happened"* — which is also what a broken run looks like.
#[test]
fn a_misspelled_arm_is_refused_rather_than_defaulted() {
    assert!(ce_executor_from(Some("hsot")).is_err());
    assert!(vas_publish_from(Some("drian")).is_err());
    assert!(isolate_plane_from(Some("rael")).is_err());
}
