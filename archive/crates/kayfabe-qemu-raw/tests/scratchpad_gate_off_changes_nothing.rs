//! ★★★★★ **THE DISARMED ARM IS THE CONTROL** — and it is now selected BY NAME, not by silence.
//!
//! # ⊘⊘⊘ w763f — this file's TITLE CLAIM was the superseded default
//!
//! It used to open *"THE DISARMED ARM IS WHAT SHIPPED … with `KAYFABE_SCRATCHPAD` unset,
//! nothing about this device is different"*, and every assertion below read `None` as `off`.
//! `SINGLE_STORE_PLAN.md` increment 1's method was *"add behind an env-var gate, boot,
//! measure, then decide"* — **the decision was made, and the armed arm won** (THE_CONSTRAINTS
//! §42). Absence now means the design.
//!
//! ⊘ What survives is the more important half and it survives UNCHANGED: **the control arm
//! must still change nothing**, and it is still reachable — now by `off`, spelled out. A
//! control you can only get by forgetting to configure anything is not a control.
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

use kayfabe_qemu_raw::scratchpad::{ScratchpadArm, scratchpad_cuda_from, scratchpad_from};
use kayfabe_qemu_raw::shim::Regs;

/// The device the shipped configuration realizes, with no environment set.
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

fn disarmed() -> Regs {
    // ⊘ The gates are read by `Regs::create` from the process environment and there is no
    // per-call override for them (unlike the framebuffer arm). This binary is the only one
    // that writes them, the write happens once, and EVERY device in this file is built
    // through here — so the write is ordered before every read of it.
    // SAFETY: a single `OnceLock`-guarded write, before any `Regs::create` in this process.
    static SET: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    SET.get_or_init(|| {
        let set = kayfabe_qemu_raw::testenv_unsafe::set_var_in_single_threaded_test_setup;
        set("KAYFABE_SCRATCHPAD", "off");
        set("KAYFABE_SCRATCHPAD_CUDA", "off");
        set("KAYFABE_DEVICE_VIEW", "off");
    });
    Regs::create_probed_in_a_process_with_no_guest(0, "")
        .expect("the shipped chip row realizes on the control arm")
}

/// ⊘ **The absence, asserted.** With the gate unset there is no VM-lifetime isolate — so no
/// child process is forked at realize, and nothing of the host's video memory is held.
#[test]
fn with_the_gate_off_no_scratchpad_isolate_exists() {
    let regs = disarmed();
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
fn with_the_gate_off_the_advertised_framebuffer_is_the_compiled_one() {
    let regs = disarmed();
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
    // ⊘ The default lives in `defaults_are_the_new_design`; this test owns the ARMS.
    assert_eq!(scratchpad_from(Some("off")), Ok(ScratchpadArm::Off));
    assert_eq!(scratchpad_from(Some("on")), Ok(ScratchpadArm::Measure));
    // ★ `require` is the design's own rule — "if that fails, the VM does not start" — and it
    // is a SEPARATE arm from `on`, because a device that refuses to realize produces no
    // teardown census and therefore no diagnosis of why the reservation failed.
    assert_eq!(scratchpad_from(Some("require")), Ok(ScratchpadArm::Require));
    // ★ A typo is refused, in both directions: defaulted to `off` it would run the control
    // arm on a boot the operator believes is armed; defaulted to `on` it would reserve the
    // host's whole framebuffer on a boot nobody asked for it on.
    for junk in ["1", "0", "true", "ON", "yes", "required", "REQUIRE", ""] {
        assert!(
            scratchpad_from(Some(junk)).is_err(),
            "`{junk}` must be refused, never defaulted"
        );
    }
}

/// ★★★ **INCREMENT 4'S GATE IS A PEER, AND IT IS OFF.**
///
/// ⊘ Arming it makes ONE isolate dynamically linked and sandboxed **late** — a change to a
/// security boundary. `THE_CONSTRAINTS.md` §w724d states the cost plainly, and a boundary that
/// could move because of a typo would not be one.
#[test]
fn the_cuda_gate_has_a_control_arm_and_refuses_anything_that_is_not_a_state() {
    // ⊘ Default asserted in `defaults_are_the_new_design`; the arms are this test's.
    assert_eq!(scratchpad_cuda_from(Some("off")), Ok(false));
    assert_eq!(scratchpad_cuda_from(Some("on")), Ok(true));
    for junk in ["1", "0", "true", "ON", "yes", "require", ""] {
        assert!(
            scratchpad_cuda_from(Some(junk)).is_err(),
            "`{junk}` must be refused, never defaulted — arming this moves a sandbox"
        );
    }
}

/// ⊘ **The two gates are INDEPENDENT**, and this is what stops a later edit folding the CUDA
/// arm into `KAYFABE_SCRATCHPAD` as a third value: the reservation is about video memory and
/// the CUDA arm is about a process's build and its sandbox ordering. A boot must be able to
/// arm either alone — and in particular the reservation, which the raw client depends on, must
/// not start dragging a dynamically-linked isolate along with it.
#[test]
fn the_two_gates_do_not_read_each_other() {
    // ⊘ Independence is about the PARSERS not reading each other, so every arm here is
    // named. Reading the default to prove independence made this test restate the default.
    assert_eq!(scratchpad_from(Some("on")), Ok(ScratchpadArm::Measure));
    assert_eq!(scratchpad_cuda_from(Some("off")), Ok(false));
    assert_eq!(scratchpad_from(Some("off")), Ok(ScratchpadArm::Off));
    assert_eq!(scratchpad_cuda_from(Some("on")), Ok(true));
}

/// ⊘ With the scratchpad gate unset there is no isolate at all, so there is no CUDA report —
/// and the accessor must say EMPTY rather than inventing a "not run" string that a grader
/// could mistake for a measured absence.
#[test]
fn with_the_gate_off_there_is_no_cuda_report_to_read() {
    let regs = disarmed();
    assert!(regs.scratchpad().is_none());
}

/// ★★★ **THE TWO SIDES OF THE SEAM AGREE ABOUT WHICH ISOLATE IS THE SCRATCHPAD.**
///
/// `kayfabe-isolate-host` decides which image to spawn from `id.proc() == u32::MAX`, and it
/// cannot import this crate (it sits below it), so the constant is stated twice.
/// ⊘ Two statements of one fact is the defect this tree keeps paying for; this is the check
/// that makes them one. If they ever differ, the scratchpad silently gets the STATIC image,
/// `dlopen` returns "Dynamic loading not supported", and the census reports it as a host
/// problem.
/// ⊘ `host-isolates` only: without it there is no `kayfabe-isolate-host` to compare against,
/// and a test that could not compile in the default configuration would break the ordinary
/// `cargo test -p kayfabe-qemu-raw` for a check that has nothing to say there.
#[cfg(feature = "host-isolates")]
#[test]
fn the_scratchpad_proc_id_agrees_across_the_seam() {
    assert_eq!(
        kayfabe_qemu_raw::scratchpad::SCRATCHPAD_PROC,
        kayfabe_isolate_host::SCRATCHPAD_ISOLATE_PROC,
        "the two sides of the isolate seam disagree about which IsolateId is the VM-lifetime \
         scratchpad; the CUDA image would go to the wrong isolate, or to none"
    );
}

/// ★★★ **THE DEVICE-VIEW CROSSING'S GATE IS A THIRD PEER, AND IT IS OFF.**
///
/// ⊘ Arming it makes this process hold a `/dev/nvidia<N>` descriptor — transiently, and only
/// under the owner's **conditional** ruling of 2026-09-14
/// (`bar1_passthrough_device_local_host_visible.md` §4 item 1). A boundary that could move
/// because of a typo would not be one, so a value naming neither state is refused.
#[test]
fn the_device_view_gate_has_a_control_arm_and_refuses_anything_that_is_not_a_state() {
    use kayfabe_qemu_raw::scratchpad::device_view_from;
    // ⊘ Default asserted in `defaults_are_the_new_design`; the arms are this test's.
    assert_eq!(device_view_from(Some("off")), Ok(false));
    assert_eq!(device_view_from(Some("probe")), Ok(true));
    for junk in ["on", "1", "true", "PROBE", "yes", "require", ""] {
        assert!(
            device_view_from(Some(junk)).is_err(),
            "`{junk}` must be refused, never defaulted — arming this hands this process a \
             descriptor with an RM escape handler behind it"
        );
    }
}

/// ⊘ **Three independent gates, and none reads another.** The reservation is about video
/// memory, the CUDA arm is about a process's build and its sandbox ordering, and this one is
/// about a descriptor crossing a process boundary. A boot must be able to arm any one alone,
/// and folding them would make a single typo move three things.
#[test]
fn the_three_gates_are_independent() {
    use kayfabe_qemu_raw::scratchpad::device_view_from;
    assert_eq!(scratchpad_from(Some("off")), Ok(ScratchpadArm::Off));
    assert_eq!(device_view_from(Some("off")), Ok(false));
    assert_eq!(scratchpad_from(Some("on")), Ok(ScratchpadArm::Measure));
    assert_eq!(device_view_from(Some("probe")), Ok(true));
}
