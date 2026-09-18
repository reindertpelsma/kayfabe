//! ★★★★★ **§3's GATE — `KAYFABE_FB_STORE`, and the two preconditions of its armed arm.**
//!
//! §w727's third rule: *"**Refuse at startup, loudly, never silently clamp.** A guest booted
//! with a BAR too small for its driver fails somewhere unrecognisable."* The same applies to
//! a store whose host-side half is not built: each precondition below, unmet, produces a boot
//! that fails later and elsewhere, with a symptom that names the wrong subsystem.
//!
//! ⊘ These are pure-function tests on purpose. A rule that can only be exercised by booting
//! is a rule whose every arm but one is unmeasured — and the arms that matter here are the
//! **failing** ones, which a successful boot never visits.

use kayfabe_qemu_raw::deviceview::{FbStoreArm, enforce_device_store, fb_store_from};

/// ★ **The default is the SINGLE STORE, stated once and asserted here.**
///
/// # ⊘⊘⊘ w763d — the FIFTH green test found pinning a superseded default
///
/// This file's own header says the gate exists so that *"a boot that fails later and
/// elsewhere, with a symptom that names the wrong subsystem"* cannot happen — and this
/// assertion was holding the arm that produces exactly that, by name: on the arena a guest
/// leaf must be JOINED to be host-nameable, so every ring adoption refuses
/// `ADOPT-WHY (6) the binding EXISTS but carries NO HOST OBJECT`.
///
/// > Owner, 2026-09-18: *"in the single store joining is dead right? that idea of islands of
/// > framebuffers in gpga is removed?"*
///
/// ⊘ `arena` stays reachable BY NAME as the control arm. §42(a): the default names an
/// ARCHITECTURE, and this is the line that says which one.
#[test]
fn absent_is_the_single_store_and_is_not_an_error() {
    assert_eq!(
        fb_store_from(None).unwrap(),
        FbStoreArm::Device,
        "★★★★★ a boot that names no framebuffer store must be the SINGLE STORE.          Defaulting to `arena` means guest vidmem is fabricated per leaf and every leaf needs          a join to become host-nameable — the two-worlds machinery the single store replaces"
    );
    assert_eq!(fb_store_from(Some("arena")).unwrap(), FbStoreArm::Arena);
    assert_eq!(fb_store_from(Some("device")).unwrap(), FbStoreArm::Device);
}

/// ★★ **A typo is REFUSED, not defaulted** — and the refusal says why both directions of a
/// silent default would be wrong.
#[test]
fn a_value_naming_neither_store_is_refused_and_says_why() {
    let why = fb_store_from(Some("Device")).expect_err("case matters; a near miss is a typo");
    assert!(why.contains("arena"), "{why}");
    assert!(why.contains("device"), "{why}");
    assert!(
        why.contains("opposite ways"),
        "the refusal must carry the reason it is not defaulted, or the next reader will \
         'fix' it by adding one: {why}"
    );
}

/// ⊘ **The arena arm has no preconditions**, whatever the rest of the configuration is. The
/// control must be reachable from any boot, or it is not a control.
#[test]
fn the_arena_arm_is_never_refused() {
    for port in [false, true] {
        for defer in [false, true] {
            assert!(
                enforce_device_store(FbStoreArm::Arena, port, defer).is_ok(),
                "the default arm was refused for port={port} defer={defer}"
            );
        }
    }
}

/// ★★★★★ **NO PORT ⇒ REFUSED AT STARTUP.**
///
/// Without one, every page the store names is refused `NO-DEVICE-PORT` by the mirror — one
/// per access — and the boot reads as a translation failure. ⊘ That is a symptom naming the
/// wrong subsystem, which is this tree's most expensive recurring shape.
#[test]
fn the_device_arm_without_a_port_is_refused_by_name() {
    let why = enforce_device_store(FbStoreArm::Device, false, true)
        .expect_err("a device store with nothing able to arm a view must not start");
    assert!(why.contains("NO DEVICE-VIEW PORT"), "{why}");
    assert!(
        why.contains("KAYFABE_SCRATCHPAD") && why.contains("KAYFABE_DEVICE_VIEW"),
        "a refusal that does not say which gates to set costs the reader the diagnosis: {why}"
    );
}

/// ★★★★★ **THE vCPU PRECONDITION, AND IT IS THE ONE A WITNESS WOULD NOT CATCH.**
///
/// With deferred revalidation off, `BarMirror::fill` runs `fill_now` **synchronously on the
/// vCPU**, and `fill_now`'s device branch is an IPC round trip to the scratchpad isolate.
/// `assert_lock_free` asks what the thread *holds*, not where it is, and its
/// `assert_not_on_vcpu` half only **reports** unless `KAYFABE_VCPU_BLOCK_FATAL` is set. ⇒ the
/// vCPU would block inside an MMIO exit, silently, and be found later as latency.
#[test]
fn the_device_arm_with_inline_revalidation_is_refused_because_it_would_block_a_vcpu() {
    let why = enforce_device_store(FbStoreArm::Device, true, false)
        .expect_err("a configuration that arms views on the vCPU must not start");
    assert!(why.contains("vCPU"), "{why}");
    assert!(
        why.contains("assert_not_on_vcpu") && why.contains("REPORTS"),
        "★ the refusal must name WHY the witness does not save us, or a reader will conclude \
         the assertion covers it and delete this check: {why}"
    );
}

/// ⊘ **Both preconditions met ⇒ armed.** The control for the two refusals above, so a green
/// there cannot come from a function that refuses everything.
#[test]
fn the_device_arm_with_both_preconditions_met_is_allowed() {
    assert!(enforce_device_store(FbStoreArm::Device, true, true).is_ok());
}

/// ★★★★★ **THE TWO GATES DO NOT READ EACH OTHER** — and this one is a defect caught in review.
///
/// `KAYFABE_DEVICE_VIEW` arms the **port**; `KAYFABE_FB_STORE` chooses the **store**. w734's
/// census boot ran the first with the second at its default, and that is a legitimate
/// configuration.
///
/// ⇒ a site that picks a backing by asking *"is there a port?"* puts **that one aperture** on
/// the reserved object while every other framebuffer path serves the arena memfd — **two
/// memories for one address, on the control arm**, which is the exact defect
/// `two_worlds_split::a_framebuffer_page_written_through_bar1_is_the_page_bar2_reads` exists
/// to catch and which no unit test of the arena arm would ever see. `install_pramin_window`
/// was written that way.
#[test]
fn a_port_without_the_device_store_never_chooses_a_device_backing() {
    use kayfabe_qemu_raw::deviceview::backing_is_device;
    assert!(
        !backing_is_device(FbStoreArm::Arena, true),
        "★★★ THE PORT ALONE MUST NOT DECIDE. This is w734's own census configuration — the \
         crossing probe armed, the default store — and answering `true` here puts one \
         aperture on real video memory while the rest of the framebuffer is a host memfd."
    );
    assert!(!backing_is_device(FbStoreArm::Arena, false));
    assert!(
        !backing_is_device(FbStoreArm::Device, false),
        "and the store alone cannot act: without a port there is nothing to arm a view with, \
         which `enforce_device_store` refuses at startup rather than discovering here"
    );
    assert!(backing_is_device(FbStoreArm::Device, true));
}

// =====================================================================================
// w763z — the page-table sweep skip
// =====================================================================================

/// ★★★ **The skip is the default and a typo is still refused** (THE_CONSTRAINTS §42).
#[test]
fn the_sweep_skip_is_the_default_and_its_control_is_reachable_by_name() {
    use kayfabe_qemu_raw::shim::pt_sweep_skip_from;
    assert_eq!(
        pt_sweep_skip_from(None),
        Ok(true),
        "★ absent is the design: a window where the guest's CPU wrote no page table cannot \
         produce a different sweep answer, and `[measured w763]` 271 of 369 sweeps found \
         nothing while costing 7.22 ms of a 23 ms invalidate hold"
    );
    assert_eq!(pt_sweep_skip_from(Some("on")), Ok(true));
    assert_eq!(
        pt_sweep_skip_from(Some("off")),
        Ok(false),
        "⊘ the control arm must stay reachable BY NAME: this is the page-table plane, and a \
         wrong skip is a stale GPU translation"
    );
    for typo in ["ON", "1", "true", "yes", "", "Off", " on"] {
        assert!(
            pt_sweep_skip_from(Some(typo)).is_err(),
            "`{typo}` must be refused, never defaulted — an evidence run and its own control \
             must not be spelled alike"
        );
    }
}
