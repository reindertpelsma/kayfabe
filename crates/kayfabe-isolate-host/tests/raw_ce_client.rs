//! ★★★★★ **THE OWNER'S RAW CE CLIENT, AS A TEST** — `GPU-GATE` family.
//!
//! > *"i would definitely add the raw client we have now in a test suite. maybe we should write
//! > some tests now to cover some bugs we had, its fine if some tests are real vmm/gpu only"*
//! > — owner, 2026-08-14
//!
//! `kayfabe-rm-ladder --ce-client` (R33) drives a copy engine end to end through raw RM ioctls
//! with no `libcuda`. `[census, w295]` it is invoked by **no test**: `run_full_suite.sh` runs
//! `--engines` and `--concurrency`, and the `--ce-client` flag is reached only when a human
//! types it or when `scripts/bench/r33_hook_ce_client.sh` pushes a musl build into a guest.
//! Its data path is `alloc_vaspace` → `prove_ce_copy`, both `pub` on [`HostRmBackend`], so the
//! arm that matters is a test as soon as somebody writes one. This is that file.
//!
//! # ⊘⊘ THE ONE RULE THIS FILE EXISTS TO ENFORCE: **NEVER COLLAPSE TO ONE BOOLEAN**
//!
//! The client's own banner states the bar as **four facts** — the bytes moved (read back
//! through a mapping that is not the one written), the semaphore carries the DECLARED payload,
//! `GP_GET` reached `GP_PUT`, and the destination did not already hold the answer. `[measured
//! 2026-08-13, boot `w283c_client`, real GA106]` the verdict implemented **three** and printed
//! *"GP_GET 0 caught GP_PUT 1"* on its ★ success line, returning `R33_RC=0`.
//!
//! ⇒ Every fact below is its own `assert`, in the order a reader needs them, so a red run names
//! **which** one failed. A single `assert!(evidence.met_the_whole_bar())` would be correct and
//! would tell the next reader nothing, and this project has already paid twice for a diagnosis
//! that was true of one fact and printed as if it were true of all four.
//!
//! # The gate
//!
//! One mechanism: the same `GPU-GATE` family `tests/tests/e6_hw_join.rs` uses, decided by an
//! actual [`RmConnection::open`] rather than by `stat`ing a device node — a box can have
//! `/dev/nvidia0` and no usable driver, and a gate that cannot tell those apart reports the
//! wrong kind of zero. **Both arms print**, straight to the `stderr` descriptor, so
//! `run_full_suite.sh`'s `gate-census` and CI's reached-count can both see it.
//!
//! ⊘ It asserts **nothing** off-bench, and says so on the way out. A hardware test that skips
//! silently is worse than a missing one, because the suite then reports green over it.

use kayfabe_arch::ids::GpuId;
// ★ `RmBackend` is in scope for `alloc_vaspace` alone: it is a **trait** method, and the raw
// client reaches it through the same port a production isolate does rather than through a
// private helper. ⊘ A test that called an inherent shortcut would be testing a path the
// product does not have.
use kayfabe_isolate::{IsolateId, RmBackend as _};
use kayfabe_isolate_host::ChildExports;
use kayfabe_isolate_host::rm::{HostRmBackend, RmConnection, userd_offset_refusal};
use kayfabe_linux_raw::DevDir;
use std::sync::Arc;

const GPU: GpuId = GpuId::ZERO;

/// Neither zero nor the sentinel `prove_ce_copy` pre-fills the destination with (`!PATTERN`),
/// so *"the copy happened"* and *"the destination already looked like that"* are distinguishable
/// readings rather than the same one.
const PATTERN: u32 = 0xC0FF_EE33;

/// ⚠ Straight to the `stderr` **descriptor**, never `eprintln!`. `libtest` captures a test
/// thread's output and flushes it only when the test **fails**, so a marker written that way is
/// invisible on exactly the runs a reached-count needs to count. `[measured 2026-08-03, bench,
/// rev a1cdfdd]` `grep -c "GPU-GATE: RAN"` over a full `cargo test --workspace` was **0** while
/// the gated test passed against a real GA106.
fn gate_line(line: &str) {
    use std::io::Write as _;
    let _ = writeln!(std::io::stderr(), "{line}");
}

/// The `GPU-GATE` decision, announced on both arms.
fn gate(test: &str) -> Option<Arc<RmConnection>> {
    let dev = match DevDir::open(c"/dev") {
        Ok(d) => d,
        Err(e) => {
            gate_line(&format!(
                "GPU-GATE: SKIPPED {test} — /dev could not be opened ({e:?}). This test asserts \
                 NOTHING here and nothing is substituted for it."
            ));
            return None;
        }
    };
    match RmConnection::open(&dev, GPU, kayfabe_chips::pinned_host_classes()) {
        Ok(c) => {
            gate_line(&format!("GPU-GATE: RAN {test}"));
            Some(Arc::new(c))
        }
        Err(e) => {
            gate_line(&format!(
                "GPU-GATE: SKIPPED {test} — no NVIDIA RM connection on this box ({e}). This test \
                 asserts NOTHING here and nothing is substituted for it."
            ));
            None
        }
    }
}

/// A backend shaped exactly as `rmladder.rs` builds it for R33 — same `IsolateId`, same
/// pinned host-class profile, its own export table. ⊘ Deliberately identical: a test that
/// constructs the backend differently from the diagnostic is a test of a different path.
fn backend(conn: &Arc<RmConnection>) -> HostRmBackend {
    let id = IsolateId::new(0, GPU);
    HostRmBackend::new(id, Arc::clone(conn), Arc::new(ChildExports::new()))
}

/// ★★★★★ **R33 arm 1 as an acceptance: a copy engine allocated, mapped, submitted and
/// completed through raw RM ioctls, with all four facts asserted SEPARATELY.**
#[test]
fn the_raw_ce_client_moves_bytes_and_each_of_the_four_facts_holds_on_its_own() {
    let Some(conn) = gate("the_raw_ce_client_moves_bytes_and_each_of_the_four_facts_holds_on_its_own")
    else {
        return;
    };
    let mut rm = backend(&conn);

    let vas = rm
        .alloc_vaspace()
        .expect("★ R7: the client needs its own address space before any operand exists");
    let evidence = rm
        .prove_ce_copy(vas, PATTERN)
        .expect("★ R33 arm 1: the copy could not be built at all — this is a bring-up failure, \
                 not a result about the four facts below");

    // ---- FACT 1 — NON-VACUITY. Asserted FIRST, because every fact after it is conditional
    // on the destination not having held the answer already.
    assert_ne!(
        evidence.before, evidence.expect_after,
        "★ FACT 1 (non-vacuity): the destination read back as the expected pattern BEFORE the \
         copy, so nothing below can distinguish a working engine from a pre-filled buffer. \
         {evidence:?}"
    );

    // ---- FACT 2 — THE BYTES, first word and last, read back through an INDEPENDENT mapping
    // (its own device node, its own mmap, a kernel-chosen address).
    assert_eq!(
        evidence.after, evidence.expect_after,
        "★ FACT 2a (the bytes): the destination's FIRST word is not the source's. {evidence:?}"
    );
    assert_eq!(
        evidence.after_last, evidence.expect_after_last,
        "★ FACT 2b (the extent): the first word moved and the LAST did not — a TRUNCATED copy, \
         which is a different defect from a copy that never ran and must not be reported as one. \
         {evidence:?}"
    );

    // ---- FACT 3 — THE RELEASE, carrying the DECLARED payload. Bytes without a release would
    // mean something moved them that we did not ask.
    assert_eq!(
        evidence.submit.semaphore, evidence.payload,
        "★ FACT 3 (the release): the engine semaphore does not carry the payload this \
         submission declared, so the bytes above are not attributable to it. {evidence:?}"
    );

    // ---- FACT 4 — THE CURSOR. ⊘ The one `CeEvidence::copied()` does not check, and the one
    // `w283c` passed on while printing the words "caught" on its success line.
    assert!(
        evidence.cursor_caught_up(),
        "★ FACT 4 (the cursor): `GP_GET {}` did not reach `GP_PUT {}`. That cursor is THIS \
         channel's own USERD — a path that executes the work on a DIFFERENT host channel cannot \
         advance it, which is why this fact is separable from the three above. {evidence:?}",
        evidence.submit.gp_get,
        evidence.submit.gp_put,
    );

    // ---- And the conjunction LAST, as a cross-check on the predicate rather than as the
    // verdict. If this fires while all four rows above passed, the instrument disagrees with
    // itself and that is a bug in the instrument, not a result about the GPU.
    assert!(
        evidence.met_the_whole_bar(),
        "★★★ the four facts each held and `met_the_whole_bar()` says otherwise — the verdict \
         predicate and these assertions disagree. {evidence:?}"
    );

    gate_line(&format!(
        "GPU-GATE: R33 ACCEPTANCE all four facts hold separately — {} bytes, src {:#018x} dst \
         {:#018x}, semaphore {:#010x} == declared {:#010x}, GP_GET {} == GP_PUT {}",
        evidence.bytes,
        evidence.src_va,
        evidence.dst_va,
        evidence.submit.semaphore,
        evidence.payload,
        evidence.submit.gp_get,
        evidence.submit.gp_put,
    ));
}

/// ★★★ **The guest-supplied `userdOffset` rule, checked against the host RM that will NOT
/// check it.**
///
/// The refusal itself is a pure function and is unit-tested in `rm.rs`. What only a real driver
/// can corroborate is the *premise*: that RM accepts a channel at an aligned offset, so the
/// refusal below is ours and not RM's. ⇒ This test asserts the **non-vacuity of the premise**
/// on hardware and the refusal's shape beside it, which is the pair the unit test cannot make.
///
/// ⊘ It deliberately does not try to allocate a channel at a misaligned offset and watch
/// hardware corrupt itself. RM's own behaviour there is a silent truncation
/// (`kernel_channel_gv100.c:208`, `>> 9`, no validation) — an experiment whose failure mode is
/// a wedged channel, and whose result we already know from the source.
#[test]
fn the_offset_this_client_uses_is_one_the_rule_admits_and_a_nudged_one_is_not() {
    let Some(_conn) =
        gate("the_offset_this_client_uses_is_one_the_rule_admits_and_a_nudged_one_is_not")
    else {
        return;
    };
    // The client's own layout constant, reached through the public rule rather than re-spelled
    // here: a second copy of `0x3000` in a test is the "two records of one fact" shape.
    assert_eq!(
        userd_offset_refusal(0x3000),
        None,
        "★ the offset this client actually submits must be placeable, or every misalignment \
         refusal below is a blanket 'no' wearing a specific name"
    );
    assert!(
        userd_offset_refusal(0x3000 + 8).is_some(),
        "★ the same offset nudged by one word must be refused — RM truncates it to a DIFFERENT \
         512-byte slot and reports nothing"
    );
}
