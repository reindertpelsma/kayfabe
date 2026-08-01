//! # The arch-invariant host classes, pinned (`#156`)
//!
//! `crate::invariant_classes` exists so the host forwarding path can name three class ids
//! by ROLE rather than by generation. A role name is a place a wrong value can hide, so
//! the values are pinned here against a **second, independent transcription** of the
//! NVIDIA headers — not against `crate::generated::classes`, which is what the module
//! under test reads.
//!
//! ⊘ It cannot detect a shared misreading of the headers. It can detect a role wired to
//! the wrong class, a value edited, and a fourth alias arriving without a citation — the
//! three things a role rename actually risks.

use kayfabe_abi::invariant_classes as inv;

// The ids, transcribed a second time, from the class headers rather than from the
// generated module. `ogkm-580: src/common/sdk/nvidia/inc/class/…`
const FERMI_VASPACE_A: u32 = 0x0000_90f1; // cl90f1.h
const FERMI_CONTEXT_SHARE_A: u32 = 0x0000_9067; // cl9067.h
const KEPLER_CHANNEL_GROUP_A: u32 = 0x0000_a06c; // cla06c.h

/// ★★ Each role names the class it says it names.
#[test]
fn each_role_alias_is_the_class_its_documentation_names() {
    assert_eq!(inv::VA_SPACE, FERMI_VASPACE_A, "VA_SPACE");
    assert_eq!(inv::CHANNEL_GROUP, KEPLER_CHANNEL_GROUP_A, "CHANNEL_GROUP");
    assert_eq!(inv::CONTEXT_SHARE, FERMI_CONTEXT_SHARE_A, "CONTEXT_SHARE");

    // Non-vacuity: three distinct values, so a module that aliased all three to one
    // constant could not pass the three assertions above by accident.
    assert_ne!(inv::VA_SPACE, inv::CHANNEL_GROUP);
    assert_ne!(inv::VA_SPACE, inv::CONTEXT_SHARE);
    assert_ne!(inv::CHANNEL_GROUP, inv::CONTEXT_SHARE);
}

/// ★★★ The SET is three, and a fourth member is a decision, not an edit.
///
/// The invariance claim is quantified over this list. Adding an alias without adding a
/// per-chip citation to the module docs would silently widen a claim that three
/// generations were checked to cover a class nobody checked.
#[test]
fn the_invariant_set_is_exactly_the_three_that_were_sourced() {
    let names: Vec<&str> = inv::ARCH_INVARIANT_HOST_CLASSES
        .iter()
        .map(|(n, _)| *n)
        .collect();
    assert_eq!(
        names,
        vec!["VA_SPACE", "CHANNEL_GROUP", "CONTEXT_SHARE"],
        "★ the arch-invariant host-class set changed. Every member is a claim that its \
         id is identical on GA106, AD106 and GH100, read out of \
         g_gpu_class_list.c at a cited line. A new member needs that citation in the \
         module docs' table before it needs this list"
    );
    // …and the list really carries the constants, not a second copy of them.
    for (name, value) in inv::ARCH_INVARIANT_HOST_CLASSES {
        let expect = match *name {
            "VA_SPACE" => inv::VA_SPACE,
            "CHANNEL_GROUP" => inv::CHANNEL_GROUP,
            "CONTEXT_SHARE" => inv::CONTEXT_SHARE,
            other => panic!("unlisted member {other}"),
        };
        assert_eq!(*value, expect, "{name} in the list disagrees with the constant");
    }
}
