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

/// ⊘ **The default is `arena`, stated once and asserted here.**
#[test]
fn absent_is_the_arena_and_is_not_an_error() {
    assert_eq!(fb_store_from(None).unwrap(), FbStoreArm::Arena);
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
