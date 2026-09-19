//! ★★★★★ **A boot that asks for a deleted arm must FAIL, not be quietly ignored (w533).**
//!
//! `THE_PRODUCTION_CONTRACT.md` §6: *"once an arm is gone, refuse to start if its variable is
//! set. A stale script exporting a removed flag must fail loudly, not be ignored while someone
//! reads the results as though it applied."*
//!
//! # Why this is the first thing built, before any arm is cut
//!
//! The bench pins nine variables on **every** graded boot. When one is deleted the scripts
//! still export it. An unknown variable that is ignored produces the worst possible outcome:
//! the run measures the production path while its operator believes it measured an arm, and
//! **nothing in the log says otherwise**. That is a silent wrong attribution, which this tree
//! has paid for repeatedly — `a_flag_is_not_progress`, `the_old_default_was_identical_to_off`.
//!
//! ⊘ A refused boot is loud and costs one minute. A silent misattribution costs a conclusion.

use kayfabe_qemu_raw::shim::Regs;

/// The first arm deleted under the production contract. ⊘ Named as a literal and not imported:
/// the constant is **gone**, and a test that imported it could not compile — which is the point.
const REMOVED: &str = "KAYFABE_PT_SWEEP";

/// ★★★ **Setting a removed arm refuses the boot; not setting it realizes normally.**
///
/// ⊘ Both halves in one test. The refusal half alone would pass for an implementation that
/// refused unconditionally, and this process shares its environment across tests, so the two
/// cannot be separated without racing.
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
fn a_removed_arm_refuses_the_boot_and_its_absence_does_not() {
    // ---- absent: the ordinary path still works.
    // SAFETY: single-threaded test setup; this is the only test in this binary.
    kayfabe_qemu_raw::testenv_unsafe::remove_var_in_single_threaded_test_setup(REMOVED);
    assert!(
        Regs::create_probed_on(0, "", Some(kayfabe_qemu_raw::deviceview::FbStoreArm::Arena)).is_ok(),
        "with no removed arm set, the shipped chip row must still realize — otherwise this \
         test proves the gate refuses everything, not that it refuses the right thing"
    );

    // ---- present: the boot is refused BY NAME.
    // SAFETY: as above.
    kayfabe_qemu_raw::testenv_unsafe::set_var_in_single_threaded_test_setup(REMOVED, "on");
    let refused = Regs::create_probed_on(0, "", Some(kayfabe_qemu_raw::deviceview::FbStoreArm::Arena));
    assert!(
        refused.is_err(),
        "a boot that exports a deleted arm must FAIL. Ignoring it lets the run measure the \
         production path while its operator believes it measured an arm, with nothing in the \
         log to say so"
    );

    // ⊘ And the value it was set to must not matter: `off` is the arm someone would most
    // likely still be exporting, and it is exactly the case where silently ignoring it would
    // invert the operator's belief about what ran.
    // SAFETY: as above.
    kayfabe_qemu_raw::testenv_unsafe::set_var_in_single_threaded_test_setup(REMOVED, "off");
    assert!(
        Regs::create_probed_on(0, "", Some(kayfabe_qemu_raw::deviceview::FbStoreArm::Arena)).is_err(),
        "`off` must refuse too — it is the value a stale script is most likely to carry, and \
         the one where being ignored inverts what the operator thinks ran"
    );

    // SAFETY: as above.
    kayfabe_qemu_raw::testenv_unsafe::remove_var_in_single_threaded_test_setup(REMOVED);
}
