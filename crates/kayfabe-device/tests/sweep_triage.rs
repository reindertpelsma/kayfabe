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
//! serving it, and [`an_unsurvivable_amputation_may_not_be_left_unserved`] goes red naming
//! the control. That is the boot this port does not have to spend.

use kayfabe_device::inittables::WantedTable;
use kayfabe_device::sweep::{
    SWEEP_TRIAGE, SweepDisposition, triage_for, unsurvivable_and_unserved,
};

#[test]
fn an_unsurvivable_amputation_may_not_be_left_unserved() {
    // ★★★ The gate. Derived on both sides — from `SWEEP_TRIAGE`'s dispositions and from
    // `WantedTable::from_cmd` — so shortening either list cannot make it agree.
    let gap = unsurvivable_and_unserved();
    assert!(
        gap.is_empty(),
        "these controls would amputate an engine RM then dereferences, and nothing serves \
         them: {:#?}",
        gap
    );
}

#[test]
fn the_gate_is_not_vacuous_because_the_unsurvivable_class_is_not_empty() {
    // ⊘ `is_empty()` on an empty universe passes on a port that triaged nothing. Two
    // separate non-vacuity facts: the class has a member, and that member is served.
    let unsurvivable: Vec<_> = SWEEP_TRIAGE
        .iter()
        .filter(|c| c.disposition == SweepDisposition::AmputationUnsurvivable)
        .collect();
    assert_eq!(
        unsurvivable.len(),
        2,
        "two controls are currently known to be unsurvivable if refused"
    );
    let unsurvivable_cmds: Vec<u32> = unsurvivable.iter().map(|c| c.cmd).collect();
    assert_eq!(unsurvivable_cmds, vec![0x2080_0a1c, 0x2080_0a40]);
    let engines: Vec<&str> = unsurvivable.iter().map(|c| c.engine).collect();
    assert_eq!(engines, vec!["KernelMemorySystem", "KernelFifo"]);
    assert_eq!(
        WantedTable::from_cmd(0x2080_0a1c),
        Some(WantedTable::MemorySystemStaticConfig),
        "and it is served, which is the only reason the gate above is green"
    );
    assert_eq!(
        WantedTable::from_cmd(0x2080_0a40),
        Some(WantedTable::InternalDeviceInfo),
        "and so is the second, for the same reason"
    );
}

#[test]
fn the_triage_universe_is_pinned_so_shortening_it_is_a_red_test() {
    // ★★ A gate quantified over a list is only as strong as the list. Pin its size and its
    // membership here, so removing an entry to make something pass is itself a failure.
    assert_eq!(SWEEP_TRIAGE.len(), 4, "the triaged universe's size");
    let cmds: Vec<u32> = SWEEP_TRIAGE.iter().map(|c| c.cmd).collect();
    assert_eq!(
        cmds,
        vec![0x2080_0a1c, 0x2080_0a40, 0x2080_0a4b, 0x2080_0a87]
    );
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
    // ⊘ `None` is the dangerous answer, not a neutral one — an untriaged control reached
    // from the sweep is exactly `t134a`'s defect, and the lookup says so rather than
    // defaulting to something that reads as safe.
    assert_eq!(triage_for(0xdead_beef), None);
}
