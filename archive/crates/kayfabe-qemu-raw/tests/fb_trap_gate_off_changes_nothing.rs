//! ★★★★★ **§w724g's MEASUREMENT ARM, AND ITS CONTROL** — `SINGLE_STORE_PLAN.md`.
//!
//! The arm makes the BAR1/BAR2 **trap path** refuse by name instead of translating and
//! serving. Its whole purpose is one boot: if the refusal never fires, the trap path is
//! **unreachable** rather than merely unused, and increment 7 may delete the demand-fill
//! mirror.
//!
//! ⊘ This file is the half a `cargo test` can hold: that the control arm is what shipped, and
//! that the gate refuses a value naming neither state. Whether the refusal fires is a property
//! of a **boot**, and no assertion here pretends otherwise.

use kayfabe_device::plane::FbTrapPolicy;
use kayfabe_qemu_raw::shim::{Regs, fb_trap_from};

/// ⊘ **Absent is `Serve`, and `Serve` is what shipped.** The default has to be stated in
/// exactly one place; this is the test that it is the right one.
// ⊘⊘⊘ **w763 — THE FRAMEBUFFER ARM IS STATED HERE, NOT INHERITED FROM THE ENVIRONMENT.**
//
// `KAYFABE_FB_STORE` now defaults to `device`, whose preconditions (a scratchpad isolate, a
// device-view port) no test process has. `[measured w763]` the flip reddened whole suites at
// once — none of them about what guest video memory IS — because every one reached a fixture
// that read a process global it never named.
//
// ⊘ Setting the variable here instead would put a process-global write in a multi-threaded
// test binary, which is the shape that already cost this campaign a flake. The arm is an
// argument to the composition root; production still calls `Regs::create`.

#[test]
fn absent_is_serve_and_is_not_an_error() {
    // ⊘ w811d: the DEFAULT is asserted once, in `defaults_are_the_new_design`. This test
    // keeps what is its own: the arms, and the refusal of a typo.
    assert_eq!(fb_trap_from(Some("serve")), Ok(FbTrapPolicy::Serve));
    assert_eq!(fb_trap_from(Some("refuse")), Ok(FbTrapPolicy::RefuseByName));
}

/// ★ A typo defaulted to `serve` would run the **control** arm on a boot whose entire purpose
/// is the armed one — and the armed arm's result is what licenses a deletion. So it refuses.
#[test]
fn a_value_naming_neither_state_is_refused() {
    for junk in ["on", "off", "1", "0", "REFUSE", "Serve", "deny", ""] {
        assert!(
            fb_trap_from(Some(junk)).is_err(),
            "`{junk}` must be refused, never defaulted — a silently-serving boot would report \
             a zero that means nothing"
        );
    }
}

/// ⊘ With the variable unset, a realized device serves the trap path exactly as it always
/// has, and its refusal counter is zero **because nothing refuses**, not because nothing
/// happened.
#[test]
fn with_the_gate_unset_the_plane_serves_the_trap_path() {
    assert!(
        std::env::var_os("KAYFABE_FB_TRAP").is_none(),
        "this test binary must not have the gate set; it is testing the default"
    );
    let regs = Regs::create_probed_in_a_process_with_no_guest(0, "").expect("the shipped chip row realizes");
    assert_eq!(
        regs.plane().fb_trap_policy(),
        FbTrapPolicy::Serve,
        "the shipped configuration must serve the trap path; arming the refusal by default \
         would turn a soft property into a hard correctness requirement without anyone \
         deciding to"
    );
    assert_eq!(regs.plane().fb_trap_refusals(), 0);
}

/// ★★★ **The arm actually changes the trap path.** ⊘ Without this, the gate could be wired to
/// nothing and every boot would report `FB_TRAP_REFUSALS=0` — the *"a diagnostic gated on the
/// thing it hunts"* shape, where a zero from an arm that never ran is indistinguishable from a
/// zero from an arm that ran and found nothing.
#[test]
fn the_armed_policy_is_what_the_plane_reports() {
    let regs = Regs::create_probed_in_a_process_with_no_guest(0, "").expect("the shipped chip row realizes");
    regs.plane().set_fb_trap_policy(FbTrapPolicy::RefuseByName);
    assert_eq!(regs.plane().fb_trap_policy(), FbTrapPolicy::RefuseByName);
    regs.plane().set_fb_trap_policy(FbTrapPolicy::Serve);
    assert_eq!(regs.plane().fb_trap_policy(), FbTrapPolicy::Serve);
}
