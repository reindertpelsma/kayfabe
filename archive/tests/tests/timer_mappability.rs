//! ★★★ `#128` — **the committed hardware capture, read back as assertions.**
//!
//! `read_native_timer_measured.md` rests on one run. This file is what stops that run
//! decaying into a sentence: it parses the capture and re-asserts the four facts the design
//! actually leans on, so a truncated, empty, re-run-wrong or quietly-edited transcript turns
//! the suite red instead of continuing to be cited.
//!
//! ⊘ **The failure this exists for is not "the file is missing".** It is the one
//! `only_live_boots_are_proof` names: *a harness that writes an empty file and exits 0 is
//! worse than none, because the file's existence reads as capture.* Every signal — a
//! freshly-timestamped file, named after the revision, sitting beside real ones — says the
//! evidence is there; only asserting the CONTENT shows whether it is.
//!
//! ⚠ **What this cannot check**, stated so nobody reads more into a green run: that the
//! capture came from the hardware it names. It is a text file, and a text file can be
//! written by anything. What it does check is that the file says what the design says it
//! says — which is the failure mode that has actually happened here (`the C oracle's empty
//! rows`), and it is worth more than trusting a filename.
//!
//! No GPU and no vendored tree are needed: on any machine that can check the repo out, this
//! either runs or the repo is broken. **Absent is a hard failure, never a skip.**

/// The committed capture.
const CAPTURE: &str = "../docs/reference/bench_evidence/timer-mappability-9087090.out";

/// The revision the ladder binary was built from, as it appears in [`CAPTURE`]'s filename
/// and, separately, inside the file's own header. Both must agree — a capture whose
/// filename and contents name different revisions is a capture that was renamed.
const REV: &str = "9087090";

fn capture() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(CAPTURE);
    std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "the #128 timer-mappability capture is missing at {}: {e}. It is a COMMITTED \
             artifact and the only record that the host counter was ever shown to be \
             readable without privilege; a build without it must fail rather than skip.",
            path.display()
        )
    })
}

/// The capture's two arms, split on their banners. Returned as `(measurement, control)`.
/// Both were run 2026-08-02 on a GA106 at revision 9087090, in that order, into one file.
///
/// ★ The split is the whole instrument. Arm B runs as root, and RM returns
/// `NV_PROTECT_READ_WRITE` immediately for `osIsAdministrator()` without ever executing the
/// range walk (`ogkm-580: src/nvidia/arch/nvalloc/unix/src/osapi.c:2023-2054`) — so a root
/// run measures **nothing** about mappability. A test that searched the whole file for a
/// marker would happily be satisfied by the arm that cannot answer the question.
fn arms(text: &str) -> (String, String) {
    let a = text
        .find("===== A. THE MEASUREMENT")
        .expect("the capture must carry arm A, the unprivileged measurement");
    let b = text
        .find("===== B. THE CONTROL")
        .expect("the capture must carry arm B, the root control");
    assert!(a < b, "arm A must precede arm B");
    (text[a..b].to_string(), text[b..].to_string())
}

/// ⊘ **The emptiness check, and it comes first.** Every assertion below is a `contains`,
/// and `contains` on a truncated file fails with "marker not found" — which reads as *the
/// 2026-08-02 GA106 run went the other way*, not as *the file is broken*. Those are
/// opposite conclusions and they must not share a failure message.
#[test]
fn the_capture_is_a_real_capture_before_anything_is_read_out_of_it() {
    let text = capture();
    assert!(
        text.len() > 512,
        "the capture is {} bytes — that is not a run, that is a file that exists",
        text.len()
    );
    assert!(
        text.contains(REV),
        "the capture does not name revision {REV} in its own header, so the filename is \
         the only thing claiming it — and a filename is not provenance"
    );
    assert!(
        text.contains("RTX 3060") && text.contains("580.159.04"),
        "the capture must name the board and the host driver it was taken against"
    );
    // Both arms present, both reached their end. A run killed by the `timeout` would have
    // a banner and no verdict.
    let (a, b) = arms(&text);
    assert!(
        a.contains("done — timer probe only") && b.contains("done — timer probe only"),
        "one of the two arms did not reach its own end line"
    );
}

/// ★★★ **The answer to `#128`'s blocking question, bound to the arm that can give it.**
///
/// The isolate is deliberately capability-less (`guest_blast_radius.md` §3.1), so the only
/// run that bears on *"can it map the host counter?"* is the unprivileged one.
#[test]
fn an_unprivileged_process_mapped_the_host_counter_and_the_root_run_is_only_the_control() {
    let text = capture();
    let (a, b) = arms(&text);

    // Arm A really was unprivileged, and arm B really was root. ⊘ Without this the two
    // arms are just two runs, and "they agree" says nothing.
    assert!(
        a.contains("euid            = 65534"),
        "arm A must have run as an unprivileged uid, or it is not the measurement"
    );
    assert!(
        b.contains("euid            = 0"),
        "arm B must have run as root, or it is not the control"
    );

    // The control that exists expressly "so that clients may map them directly"
    // (`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080tmr.h:107-110`).
    assert!(
        a.contains("TIMER_GET_REGISTER_OFFSET = NV_OK, tmr_offset = 0x9000"),
        "arm A must show the register-offset control answering NV_OK with DRF_BASE(NV_PTIMER)"
    );

    // ★ Both routes, and BOTH matter: the dedicated PTIMER page proves a doorbell-free
    // range is mappable at all, and the usermode mirror is the one whose page offset can
    // actually back the guest (see the alignment test in kayfabe-device).
    assert!(
        a.contains("NV01_TIMER map  = ioctl 0x414 / mmap 0x1000 ACCEPTED"),
        "arm A must show the PTIMER page mapping being accepted"
    );
    assert!(
        a.contains("usermode mirror ="),
        "arm A must show the usermode-window mirror being read"
    );
}

/// ★★ **The counters ADVANCED.** A mapping that reads a frozen value is the exact defect
/// `#128` exists to prevent, and it is indistinguishable from a working one on a single
/// reading — so the capture records a pair across a sleep and this asserts the delta.
///
/// ⊘ It also asserts the delta is not absurd. A counter "advancing" by ten seconds across a
/// 20 ms sleep is not a counter we are reading correctly, and `> 0` would pass on it.
#[test]
fn both_mappings_read_a_counter_that_actually_advanced_across_the_sleep() {
    let text = capture();
    let (a, _) = arms(&text);

    // `… = <x> ns then <y> ns (+<d> ns over a 20 ms sleep)`
    let deltas: Vec<u64> = a
        .lines()
        .filter(|l| l.contains("PTIMER page     =") || l.contains("usermode mirror ="))
        .filter_map(|l| {
            let d = l.split("(+").nth(1)?;
            d.split_whitespace().next()?.parse().ok()
        })
        .collect();

    assert_eq!(
        deltas.len(),
        2,
        "arm A must carry a delta for BOTH mappings; found {deltas:?}"
    );
    for d in &deltas {
        // 20 ms, with room for a descheduled thread on either side. The lower bound is the
        // real assertion; the upper one stops a mis-decoded high word reading as success.
        assert!(
            (19_000_000..60_000_000).contains(d),
            "a 20 ms sleep produced a delta of {d} ns — that is not this counter advancing"
        );
    }
}

/// ★★★ **The two mappings are ONE counter**, which is what licenses the design treating
/// the usermode mirror as a stand-in for the PTIMER page.
///
/// The rung reads PTIMER page `a`, sleeps, reads PTIMER page `b`, then reads the mirror —
/// so a shared counter must give `mirror - b` in the microseconds those two reads are
/// apart. ⚠ The rung's own first version allowed a **one second** slack and passed on it;
/// that bound would have been satisfied by two unrelated clocks that merely happened to be
/// near each other. The capture records the actual gap and this asserts it is small.
#[test]
fn the_two_mappings_agree_closely_enough_to_be_the_same_counter() {
    let text = capture();
    let (a, _) = arms(&text);
    let line = a
        .lines()
        .find(|l| l.contains("same counter?"))
        .expect("arm A must carry the identity check");
    assert!(
        line.trim_start().starts_with('★'),
        "the identity check must have PASSED, not merely be present: {line}"
    );
    let gap: u64 = line
        .split("mirror is ")
        .nth(1)
        .and_then(|s| s.split_whitespace().next())
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| panic!("could not read the gap out of: {line}"));
    assert!(
        gap < 1_000_000,
        "the two mappings were {gap} ns apart — too far to call them one counter"
    );
}

/// ★★★ **The counter reads WALL CLOCK, not uptime** — the fact `guest_blast_radius.md`
/// §5.4 turns into a security statement, and the one that corrected the task's own brief.
///
/// If a future driver or board stopped setting PTIMER from real time, §5.4's exposure would
/// shrink to "host GPU uptime" and the security note would be overstated. That is a change
/// worth being told about, so it is asserted rather than left as prose.
#[test]
fn the_host_counter_is_unix_wall_clock_which_is_what_makes_the_exposure_the_host_clock() {
    let text = capture();
    let (a, _) = arms(&text);
    let reading: u64 = a
        .lines()
        .find(|l| l.contains("PTIMER page     ="))
        .and_then(|l| l.split("= ").nth(1))
        .and_then(|s| s.split_whitespace().next())
        .and_then(|s| s.parse().ok())
        .expect("arm A must carry a PTIMER page reading");

    // Unix seconds for 2026-01-01 and 2036-01-01. A counter reading uptime would be
    // several orders of magnitude below the floor; one reading wall clock cannot help but
    // be inside the window.
    let secs = reading / 1_000_000_000;
    assert!(
        (1_767_225_600..2_082_758_400).contains(&secs),
        "the counter read {reading} ns = {secs} s, which is not Unix wall clock. If this \
         board no longer sets PTIMER from real time, guest_blast_radius.md §5.4 overstates \
         the exposure and must be re-read — do not just widen this window."
    );
}
