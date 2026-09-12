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
#[test]
fn a_removed_arm_refuses_the_boot_and_its_absence_does_not() {
    // ---- absent: the ordinary path still works.
    // SAFETY: single-threaded test setup; this is the only test in this binary.
    unsafe {
        std::env::remove_var(REMOVED);
    }
    assert!(
        Regs::create(0).is_ok(),
        "with no removed arm set, the shipped chip row must still realize — otherwise this \
         test proves the gate refuses everything, not that it refuses the right thing"
    );

    // ---- present: the boot is refused BY NAME.
    // SAFETY: as above.
    unsafe {
        std::env::set_var(REMOVED, "on");
    }
    let refused = Regs::create(0);
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
    unsafe {
        std::env::set_var(REMOVED, "off");
    }
    assert!(
        Regs::create(0).is_err(),
        "`off` must refuse too — it is the value a stale script is most likely to carry, and \
         the one where being ignored inverts what the operator thinks ran"
    );

    // SAFETY: as above.
    unsafe {
        std::env::remove_var(REMOVED);
    }
}
