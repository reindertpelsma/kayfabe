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
    CeExecutorChoice, DoorbellAsyncArm, FbJoinArm, GuestRamSource, GuestRingArm, IsolatePlane,
    JoinReleaseArm, VasPublishArm, ce_executor_from, dirty_gate_from, doorbell_async_from,
    fb_join_from, fb_trap_from, guest_ram_source_from, guest_ring_from, isolate_plane_from,
    join_release_from, pt_sweep_skip_from, vas_publish_from,
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

    // ⊘⊘⊘ **w811 — THE ROW THIS GATE DID NOT HAVE, AND WHAT ITS ABSENCE COST.**
    //
    // `KAYFABE_GUEST_RAM` defaulted to `none` while its sibling one screen up defaulted to
    // `IsolatePlane::Real`: a real isolate plane, blind to guest memory. `[measured w811]`
    // EVERY `VAS-PUBLISH` of a thin-guest run carried `⊘ NO GUEST-RAM BACKING (no
    // hypervisor layout to resolve a GPA against)` and `published=0 refused=9` — and the
    // harness had been building the precondition (`memory-backend-memfd,share=on`) and
    // documenting why on every boot the whole time.
    //
    // ⚠ Read how it survived w810's audit: that pass quantified over **the selectors I
    // listed**, and this one was not on the list. The list was the bug, which is what
    // `the_gate_itself_quantifies_over_every_selector` below now closes.
    for armed in [IsolatePlane::Loopback, IsolatePlane::Real] {
        assert_eq!(
            guest_ram_source_from(armed, None),
            Ok(GuestRamSource::HypervisorMemfd),
            "★★★★★ w811: with `none`, every isolate is blind to guest RAM and every \
             publication refuses before the VAS is even looked at"
        );
    }
    // ⊘ Not a flat constant: with no isolate there is nobody to hold the grant, and
    // `isolate_factory` refuses the pair by name. The default is the PLANE's.
    assert_eq!(
        guest_ram_source_from(IsolatePlane::Stillborn, None),
        Ok(GuestRamSource::None)
    );

    // ⊘⊘⊘ **w811 — a PERF gate that was also a CORRECTNESS LATCH.** w330 measured this arm
    // worth 8.5x on the median trap and moved the default ON; those numbers stand. What
    // retired it is (a) the owner's ruling that the PTX walker makes the avoided pass cost
    // 205.7 µs, so the premise *"the pass is expensive"* is gone, and (b) a measurement
    // this gate's own absence let through: the skip is keyed on *"unchanged since the last
    // COMPLETED pass"*, and a pass that published nothing changes nothing — so the first
    // failure latches forever. `[measured w811]` 5 880 candidate rows / 11.6 MB skipped on
    // every doorbell of a run, reported as `refused=0` — absent demand, not blocked demand.
    assert_eq!(
        dirty_gate_from(None),
        Ok(false),
        "★★★★★ w811: `on` skips publication whenever the previous pass published nothing, \
         which is exactly when it most needs to run"
    );

    // ⊘ Already the newest arm — w380 moved it `Supersede` → `Alias` on measurement (127
    // takeovers then 28 108 `⊘ SUPERSEDE CAPPED`, and an `Xid 31 FAULT_PDE` from a live VA
    // frozen unbacked). The row exists so that a future move has to come here and say why.
    assert_eq!(join_release_from(None), Ok(JoinReleaseArm::Alias));

    // ★★ **`fb_trap` is classified DELIBERATELY as `Serve`, and it is not an exception to
    // §44.** The armed arm is not an architecture: `refuse` makes the trap path return a
    // named refusal so that one boot converts *"`TRAP_FILLS` reads zero"* into *"the trap
    // path is unreachable"*, which is what licenses deleting the demand-fill mirror. Serving
    // a trapped BAR1/BAR2 access is what the hardware does and what the single store's
    // device views keep doing; what the single store supersedes is the MIRROR BEHIND it, and
    // the mirror is deleted by the expiry below rather than by flipping this.
    // ⚠ **EXPIRY** (restated here so the gate carries it): this selector, `selected_fb_trap`
    // and `kayfabe_device::plane::FbTrapPolicy` are DELETED once the 30-arm guest suite is
    // green with `KAYFABE_FB_TRAP=refuse`. When that lands, this row goes away with them —
    // it must not survive as a permanent blessing of the arm it is scheduled to remove.
    assert_eq!(
        fb_trap_from(None),
        Ok(kayfabe_device::plane::FbTrapPolicy::Serve),
        "a trapped framebuffer access is SERVED; `refuse` is a deletion probe, not a design"
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
    for plane in IsolatePlane::ALL {
        assert_eq!(
            guest_ram_source_from(plane, Some("none")),
            Ok(GuestRamSource::None),
            "the blind arm stays spellable on every plane, including the ones that derive \
             away from it"
        );
    }
    assert_eq!(
        isolate_plane_from(Some("stillborn")),
        Ok(IsolatePlane::Stillborn)
    );
    // ⊘ w330's perf arm, frozen rather than deleted: its 8.5x median was real, and it is the
    // arm to reach for if the publication latch w811 found is ever fixed independently.
    assert_eq!(dirty_gate_from(Some("on")), Ok(true));
    assert_eq!(
        join_release_from(Some("supersede")),
        Ok(JoinReleaseArm::Supersede)
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
    assert!(guest_ram_source_from(IsolatePlane::Real, Some("memfdd")).is_err());
}

/// ★★★★★ **THE GATE MUST QUANTIFY OVER THE SELECTORS, NOT OVER MY MEMORY OF THEM.**
///
/// # ⊘⊘⊘ The list above was itself the defect
///
/// w810 built this file to stop superseded defaults, audited the selectors, and **missed
/// `guest_ram_source_from`** — whose default then refused every single publication of the
/// next run. The gate was green throughout. A hand-written ledger checks the rows someone
/// thought to write down, and the failure mode of *"old default creeps back in"* is
/// precisely **the row nobody thought to write down**; so a ledger is structurally the
/// wrong instrument for it, and was wrong here on its first outing.
///
/// ⇒ This test reads `shim.rs` itself and requires that **every `pub fn *_from` selector in
/// it is named somewhere in this file.** A selector added tomorrow reddens this tonight, and
/// the fix is to give it a row rather than to extend a list of exemptions.
///
/// ⊘ `include_str!` and not a runtime read: the path is resolved by the compiler, so a moved
/// or renamed `shim.rs` is a build error rather than a test that vacuously passes over an
/// empty string. `a_check_that_reports_is_not_a_check_that_gates`, applied to the gate's own
/// input.
#[test]
fn the_gate_itself_quantifies_over_every_selector() {
    const SHIM: &str = include_str!("../src/shim.rs");
    const SELF: &str = include_str!("defaults_are_the_new_design.rs");

    let mut selectors: Vec<&str> = Vec::new();
    for line in SHIM.lines() {
        let line = line.trim_start();
        let Some(rest) = line.strip_prefix("pub fn ") else {
            continue;
        };
        let Some(name) = rest.split('(').next() else {
            continue;
        };
        // ⊘ `_from` is the file's own naming convention for "the pure half of a selector",
        // stated in each one's rustdoc. Matching the convention rather than a list is the
        // entire point of this test.
        if name.ends_with("_from") && !selectors.contains(&name) {
            selectors.push(name);
        }
    }
    assert!(
        selectors.len() >= 7,
        "★ the scan found only {} selectors — the convention moved and this gate went \
         vacuous, which is the one way it can fail silently: {selectors:?}",
        selectors.len()
    );

    let missing: Vec<&str> = selectors
        .iter()
        .copied()
        .filter(|name| !SELF.contains(name))
        .collect();
    assert!(
        missing.is_empty(),
        "⊘⊘⊘ these selectors have a default that this gate never asserts: {missing:?}\n\
         Every one of them decides what a boot with no environment set actually runs. Give \
         each a row in `absent_selects_the_new_design_for_every_arm` naming the new-design \
         arm and what the superseded one costs — do NOT add an exemption list, because a \
         forgotten row is exactly the defect w811 paid for."
    );
}
