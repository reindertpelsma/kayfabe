//! The gate that makes `t134a`'s defect unencodable.
//!
//! ★★★ `t134a`'s failure was not a wrong decision — it was **no decision**. "This port does
//! not serve `0x20800a1c`" was the *absence* of a `WantedTable` variant, and an absence is
//! not something a test can quantify over. `kayfabe_device::sweep::SWEEP_TRIAGE` turns each
//! such absence into a statement with a disposition and an argument, and this file is what
//! stops the statement and the served set from drifting apart.
//!
//! ⊘ The bite is deliberately easy to describe: delete `MemorySystemStaticConfig` from
//! `WantedTable::ALL`, or flip its `SweepDisposition` to `AmputationIntended` without
//! serving it, and [`a_refusal_this_port_may_not_make_is_never_left_unserved`] goes red
//! naming the control. That is the boot this port does not have to spend.
//!
//! ⚠ The *universe* this file quantifies over is pinned here, but it is **derived** in
//! `crates/kayfabe-crec/tests/cap1b_differential.rs`, which reads the controls the C oracle
//! is observed to ask out of the capture and demands each be served or triaged. Shortening
//! `SWEEP_TRIAGE` therefore fails twice: here on the pin, and there on the derivation.

use kayfabe_device::inittables::WantedTable;
use kayfabe_device::sweep::{SWEEP_TRIAGE, SweepDisposition, must_serve_and_unserved, triage_for};

#[test]
fn a_refusal_this_port_may_not_make_is_never_left_unserved() {
    // ★★★ The gate. Derived on both sides — from `SWEEP_TRIAGE`'s dispositions (through
    // `SweepDisposition::must_be_served`, an exhaustive `match`) and from
    // `WantedTable::from_cmd` — so shortening either list cannot make it agree.
    let gap = must_serve_and_unserved();
    assert!(
        gap.is_empty(),
        "these controls would leave the guest damaged or silently defaulted, and nothing \
         serves them: {:#?}",
        gap
    );
}

#[test]
fn the_gate_is_not_vacuous_because_the_must_serve_classes_are_not_empty() {
    // ⊘ `is_empty()` on an empty universe passes on a port that triaged nothing. Two
    // separate non-vacuity facts per class: it has members, and each member is served.
    let must: Vec<_> = SWEEP_TRIAGE
        .iter()
        .filter(|c| c.disposition.must_be_served())
        .collect();
    assert_eq!(
        must.len(),
        5,
        "three unsurvivable amputations and two silent fail-opens"
    );
    let cmds: Vec<u32> = must.iter().map(|c| c.cmd).collect();
    assert_eq!(
        cmds,
        vec![
            0x2080_0a40,
            0x2080_0a1c,
            0x2080_0af3,
            0x2080_0aac,
            0x2080_0a59
        ]
    );
    let engines: Vec<&str> = must.iter().map(|c| c.engine).collect();
    assert_eq!(
        engines,
        vec![
            "KernelFifo",
            "KernelMemorySystem",
            "ConfidentialCompute",
            "KernelBif",
            "KernelGmmu"
        ]
    );
    for cmd in cmds {
        assert!(
            WantedTable::from_cmd(cmd).is_some(),
            "{cmd:#x} must be served, and it is the only reason the gate above is green"
        );
    }
}

#[test]
fn the_unsurvivable_class_still_names_the_three_measured_crashes() {
    // ★ The class the two named boots produced, kept separately from the fail-open class
    // so that widening the second cannot quietly shrink the first.
    let unsurvivable: Vec<u32> = SWEEP_TRIAGE
        .iter()
        .filter(|c| c.disposition == SweepDisposition::AmputationUnsurvivable)
        .map(|c| c.cmd)
        .collect();
    assert_eq!(
        unsurvivable,
        vec![0x2080_0a40, 0x2080_0a1c, 0x2080_0a59],
        "t135a's KernelFifo, t134a's KernelMemorySystem, and KernelGmmu's freed pStaticInfo"
    );
}

#[test]
fn the_triage_universe_is_pinned_so_shortening_it_is_a_red_test() {
    // ★★ A gate quantified over a list is only as strong as the list. Pin its size and its
    // membership here, so removing an entry to make something pass is itself a failure.
    //
    // ⊘ The order is the oracle's own `rpc.sequence` order, so a reader can line this list
    // up against `cargo run -p kayfabe-crec --example cap1b_report` directly.
    assert_eq!(SWEEP_TRIAGE.len(), 23, "the triaged universe's size");
    let cmds: Vec<u32> = SWEEP_TRIAGE.iter().map(|c| c.cmd).collect();
    assert_eq!(
        cmds,
        vec![
            0x2080_0a87, // seq 7   KernelNvlink
            0x2080_0a40, // seq 8   KernelFifo — DEVICE_INFO2
            0x2080_0a1c, // seq 11  KernelMemorySystem
            0x2080_0af3, // seq 13, 44  ConfidentialCompute
            0x2080_0aac, // seq 14  KernelBif
            0x2080_0a61, // seq 15, 16, 17, 34  KernelFifo — channel count
            0x2080_2a08, // seq 18  KernelCE — fault method buffer size
            0x2080_0afe, // seq 19  RM user shared data
            0x2080_0aff, // seq 20  RM user shared data poll
            0x2080_0301, // seq 25  event set notification
            0x2080_0a59, // seq 26  KernelGmmu
            0x2080_0a70, // seq 28, 29  KernelBus — sysmembar
            0x2080_0a6c, // seq 30, 31, 32  KernelMemorySystem — L2 invalidate
            0x2080_0a80, // seq 33, 38  KernelPerf — SLI GPU boost
            0x2080_2a0f, // seq 39  KernelCE — PCE config
            0x2080_2a06, // seq 40, 42  KernelCE — class DB
            0x2080_2a0d, // seq 41  KernelCE — PCE/LCE mappings
            0x2080_017e, // seq 43  VMMU segment size
            0x2080_0a9f, // seq 45  GVASPACE reserved PDEs
            0x2080_0a1f, // seq 49  KernelGraphics — caps
            0x2080_0a2a, // seq 50  KernelGraphics — info
            0x2080_0a26, // seq 51  KernelGraphics — floorsweeping
            0x2080_0a4b, // NOT in the oracle's prefix — KernelDisplay
        ]
    );
    // ⊘ And no duplicates: a repeated id would make `triage_for` answer with whichever row
    // came first and leave the other unreachable, including by this file's own assertions.
    let mut sorted = cmds.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), cmds.len(), "an id is triaged at most once");
}

#[test]
fn every_triaged_control_carries_an_argument_and_cites_something() {
    // ★ A disposition with no reason is the shape this table exists to outlaw. Not a style
    // check: the `why` is the only place the *sanctioned* removal arm or the caller's own
    // tolerance is recorded, and a refusal without one is indistinguishable from a control
    // nobody looked at.
    for c in SWEEP_TRIAGE {
        assert!(
            c.why.len() > 60,
            "{:#x} ({}) has no argument behind its disposition",
            c.cmd,
            c.engine
        );
        assert!(
            c.why.contains("ogkm-580:") || c.why.contains("C:"),
            "{:#x} ({}) cites no source",
            c.cmd,
            c.engine
        );
        assert!(!c.engine.is_empty(), "{:#x} names no engine", c.cmd);
    }
}

#[test]
fn a_control_whose_refusal_is_invisible_must_cite_the_oracles_own_reply() {
    // ★★★ The class that is easiest to get wrong in the flattering direction. "Refusing
    // changes nothing" is only a finding if the thing it is compared against is the
    // oracle's own answer — otherwise it is a hope. So the argument must name the C
    // artifact, not merely `ogkm-580`.
    let invisible: Vec<_> = SWEEP_TRIAGE
        .iter()
        .filter(|c| c.disposition == SweepDisposition::RefusalIsInvisible)
        .collect();
    assert_eq!(invisible.len(), 2, "non-vacuity: the class has members");
    for c in invisible {
        assert!(
            c.why.contains("C:"),
            "{:#x} ({}) claims a refusal is invisible without naming the ORACLE reply it \
             was compared against",
            c.cmd,
            c.engine
        );
    }
}

#[test]
fn a_deliberate_amputation_is_a_control_this_port_does_not_serve() {
    // ★★ The other direction, and it is the one that would rot quietly. An entry marked
    // `AmputationIntended` that has *also* been given a `WantedTable` variant is a
    // contradiction: the table says "we refuse this on purpose" and the dispatch says "we
    // answer it". One of the two is stale, and this says which commit to look at.
    for c in SWEEP_TRIAGE {
        if c.disposition == SweepDisposition::AmputationIntended {
            assert_eq!(
                WantedTable::from_cmd(c.cmd),
                None,
                "{:#x} ({}) is triaged as a deliberate refusal but the policy serves it",
                c.cmd,
                c.engine
            );
        }
    }
}

#[test]
fn a_halting_refusal_may_be_served_or_not_and_the_table_says_which() {
    // ★ `RefusalHalts` is the one disposition that is *orthogonal* to whether we serve: it
    // describes what happens if we do not, and this port may still spend the rung. Pin
    // which of them is currently served so that serving or unserving one is a visible
    // change rather than a silent one.
    let halts: Vec<(u32, bool)> = SWEEP_TRIAGE
        .iter()
        .filter(|c| c.disposition == SweepDisposition::RefusalHalts)
        .map(|c| (c.cmd, WantedTable::from_cmd(c.cmd).is_some()))
        .collect();
    let served: Vec<u32> = halts.iter().filter(|(_, s)| *s).map(|(c, _)| *c).collect();
    assert_eq!(
        served,
        vec![0x2080_0a61],
        "the only halting refusal this port has spent a rung on"
    );
    assert_eq!(halts.len(), 13, "non-vacuity: the class is the roadmap");
}

#[test]
fn the_lookup_finds_a_triaged_control_and_admits_an_untriaged_one() {
    assert_eq!(
        triage_for(0x2080_0a1c).map(|c| c.engine),
        Some("KernelMemorySystem")
    );
    assert_eq!(
        triage_for(0x2080_0a87).map(|c| c.disposition),
        Some(SweepDisposition::AmputationIntended)
    );
    assert_eq!(
        triage_for(0x2080_0a40).map(|c| c.engine),
        Some("KernelFifo"),
        "the control `t135a`'s first LEVEL_ERROR named is triaged, not merely known"
    );
    assert_eq!(
        triage_for(0x2080_0a59).map(|c| c.disposition),
        Some(SweepDisposition::AmputationUnsurvivable),
        "the freed-but-not-NULLed pStaticInfo, which no boot has yet reached"
    );
    // ⊘ `None` is the dangerous answer, not a neutral one — an untriaged control reached
    // from the sweep is exactly `t134a`'s defect, and the lookup says so rather than
    // defaulting to something that reads as safe.
    assert_eq!(triage_for(0xdead_beef), None);
}
