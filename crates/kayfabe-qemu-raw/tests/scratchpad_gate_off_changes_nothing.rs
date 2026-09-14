//! ★★★★★ **THE DISARMED ARM IS WHAT SHIPPED** — `SINGLE_STORE_PLAN.md` increment 1.
//!
//! The increment's method is *"add behind an env-var gate, boot, measure, then decide"*, and
//! the half of that method a test can hold is the first clause: **with `KAYFABE_SCRATCHPAD`
//! unset, nothing about this device is different.**
//!
//! # ⊘ Why this is its own test binary
//!
//! `KAYFABE_SCRATCHPAD` is process-global and `Regs::create` reads it **once**, at the
//! composition root. A test that shared a process with one that sets the variable would be
//! asserting whatever ran first. Cargo gives each integration-test file its own process, so
//! this file's process never sets it and the `None` arm is the one under test.
//!
//! # ⚠ What this test does NOT claim
//!
//! It does not claim the armed arm works — that needs a GPU and a guest, and the evidence
//! for it is a boot's `SCRATCHPAD AT …` census line, not a unit test. It claims exactly one
//! thing: that arming nothing changes nothing. ⊘ That is the claim the sequencing rule
//! ("deletions come last") rests on, and it is the one that would rot silently.

use kayfabe_qemu_raw::scratchpad::scratchpad_from;
use kayfabe_qemu_raw::shim::Regs;

/// The device the shipped configuration realizes, with no environment set.
fn shipped() -> Regs {
    Regs::create(0).expect("the shipped chip row realizes")
}

/// ⊘ **The absence, asserted.** With the gate unset there is no VM-lifetime isolate — so no
/// child process is forked at realize, and nothing of the host's video memory is held.
#[test]
fn with_the_gate_unset_no_scratchpad_isolate_exists() {
    assert!(
        std::env::var_os("KAYFABE_SCRATCHPAD").is_none(),
        "this test binary must not have the gate set; see the module docs"
    );
    let regs = shipped();
    assert!(
        regs.scratchpad().is_none(),
        "the gate is off and an isolate was spawned anyway"
    );
}

/// ★★★ **The advertised framebuffer size is the COMPILED one when nothing was reserved.**
///
/// This is the assertion that matters most, because the armed path rebinds `chip` — and a
/// rebinding that fired unconditionally would change what every guest is told about its own
/// video memory, silently and on every boot.
#[test]
fn with_the_gate_unset_the_advertised_framebuffer_is_the_compiled_one() {
    let regs = shipped();
    let chip = regs.plane().chip();
    assert_eq!(
        chip.fb_length,
        kayfabe_device::ga10x::GA106.fb_length,
        "the disarmed arm must serve GA106's own row, byte for byte"
    );
    // ⊘ Pointer identity, not just equal contents: `ga106_profile` leaks a NEW profile, so a
    // rebinding that happened to produce the same numbers would still be a different static.
    // Asserting identity is what makes "the shipped configuration does not go through that
    // function" a checked fact rather than a comment.
    assert!(
        std::ptr::eq(chip, &kayfabe_device::ga10x::GA106),
        "the disarmed arm must be the static GA106 row itself, not a copy of it"
    );
}

/// ⊘ **Absent is `off`, and `off` is not an error.** The three-way distinction — unset,
/// explicitly off, and a value naming neither — is the whole of the gate's contract.
#[test]
fn the_gates_three_way_contract() {
    assert_eq!(scratchpad_from(None), Ok(false), "absent is off");
    assert_eq!(scratchpad_from(Some("off")), Ok(false));
    assert_eq!(scratchpad_from(Some("on")), Ok(true));
    // ★ A typo is refused, in both directions: defaulted to `off` it would run the control
    // arm on a boot the operator believes is armed; defaulted to `on` it would reserve the
    // host's whole framebuffer on a boot nobody asked for it on.
    for junk in ["1", "0", "true", "ON", "yes", ""] {
        assert!(
            scratchpad_from(Some(junk)).is_err(),
            "`{junk}` must be refused, never defaulted"
        );
    }
}
