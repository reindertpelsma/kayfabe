//! ★★ **The `/dev/kvm` capability gate — skip LOUDLY, never silently.**
//!
//! Stages M2-c and M2-d added ~40 tests that need a real VM file descriptor. GitHub's
//! `ubuntu-latest` runners do not expose `/dev/kvm`, so the `stable` CI job has been red
//! since `1336ce1` — every one of them panicking at the same `open` inside
//! `kvm_unsafe.rs`. This module is the gate that makes the absence a *stated* condition
//! instead of a wall of identical panics.
//!
//! ## The design constraint, which matters more than the mechanism
//!
//! A test that quietly does nothing when the capability is missing is the
//! **green-instrument-on-an-unexercised-path** failure `testing_doctrine.md` §1 exists to
//! prevent, and it is strictly *worse* than the red CI it would replace, because red is
//! honest. So this gate:
//!
//! 1. lives in **one** place — [`kvm_available`] — rather than as a copy per test;
//! 2. **prints a line for every gated test, on both arms**, straight to stderr
//!    (bypassing libtest's capture exactly as `kayfabe_tests::skip_slow!` does, because
//!    capture swallows output on the passing path — which is the arm that matters here);
//! 3. is **never** `#[ignore]`, which is invisible in a summary and impossible to count.
//!
//! ## ★ The non-vacuity half — the part a gate is useless without
//!
//! A capability gate can silently disable an entire suite forever. The defence is that
//! **both arms are counted**, by grepping the two markers this module emits:
//!
//! ```text
//!   KVM-GATE: RAN <test>
//!   KVM-GATE: SKIPPED <test> — /dev/kvm is absent or not permitted
//! ```
//!
//! On a KVM-capable job, `SKIPPED` must be **0** and `RAN` must be **at least the known
//! floor** — a run in which the real-`Vmm` suite quietly stopped executing shows up as a
//! `RAN` count that fell, not as a green build. See the crate-level note in the CI
//! workflow; the counting is deliberately external, because each test binary is its own
//! process and no in-process counter can see across them.
//!
//! This module needs no `unsafe_code` relaxation at all: the probe is a plain
//! `File::open`. (Phrased via the lint name deliberately — the §9.3 `*_unsafe.rs` naming
//! gate greps word-wise for the bare keyword in any file not carrying that suffix, so
//! even prose about it must use the lint name. `unsafe_code` and `*_unsafe.rs` both pass,
//! because `_` is a word character; a hyphenated form does not.)

use std::sync::OnceLock;

/// The device this gate is about.
pub const KVM_DEVICE: &str = "/dev/kvm";

/// ★ Is a real KVM device present **and permitted to this process**?
///
/// Resolved once per process ([`OnceLock`]) so a mid-run permission change cannot make
/// half a binary's tests disagree about the policy — the same discipline
/// `kayfabe_tests::slow_enabled` uses, and for the same reason.
///
/// Deliberately opens read-write: `/dev/kvm` existing but being unreadable by this user
/// is the common container/CI shape, and a probe that only checked for the path would
/// report a capability we do not have.
#[must_use]
pub fn kvm_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(KVM_DEVICE)
            .is_ok()
    })
}

/// Emit this test's gate line. Called by [`crate::require_kvm`]; public because the macro
/// expands at the call site.
///
/// Writes straight to `stderr` rather than through `eprintln!`'s capture-aware path, so
/// the **passing** arm is visible too — a gate whose "it ran" marker only appears on
/// failure cannot be counted, and counting it is the whole non-vacuity argument.
pub fn report(test: &str, available: bool) {
    use std::io::Write as _;
    let mut err = std::io::stderr();
    let _ = if available {
        writeln!(err, "KVM-GATE: RAN {test}")
    } else {
        writeln!(
            err,
            "KVM-GATE: SKIPPED {test} — {KVM_DEVICE} is absent or not permitted \
             (the test asserts nothing; this line is the only record that it did not run)"
        )
    };
}

/// Gate the enclosing `#[test]` on a real `/dev/kvm` — `require_kvm!("test_name")`.
///
/// Prints `KVM-GATE: RAN <name>` and continues, or prints `KVM-GATE: SKIPPED <name> …`
/// and returns. **Both arms print**, which is what makes the gate countable and therefore
/// non-vacuous; see the module docs.
#[macro_export]
macro_rules! require_kvm {
    ($name:expr) => {
        let __kvm = $crate::kvm_gate::kvm_available();
        $crate::kvm_gate::report($name, __kvm);
        if !__kvm {
            return;
        }
    };
}
