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
    // ⚠ 5 -> 11 at the state-load rung. Five of the six new members are GR's mandatory
    // static-info controls; the sixth is `0x20800a9f`, which MOVED here out of
    // `RefusalHalts` because the gmmu1 boot showed `gpuStatePostLoad` swallows its status.
    // ⊘ The count is not the point — the membership below is. See `sweep.rs`.
    assert_eq!(
        must.len(),
        13,
        "ten unsurvivable amputations and three silent fail-opens"
    );
    let cmds: Vec<u32> = must.iter().map(|c| c.cmd).collect();
    assert_eq!(
        cmds,
        vec![
            0x2080_0a40,
            0x2080_0a1c,
            0x2080_0af3,
            0x2080_0aac,
            0x2080_2a08,
            0x2080_0a59,
            0x2080_0a9f,
            0x2080_0a1f,
            0x2080_0a26,
            0x2080_0a22,
            0x2080_0a3d,
            0x2080_0a48,
            0x2080_0a32,
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
            "KernelCE",
            "KernelGmmu",
            "OBJGVASPACE",
            "KernelGraphics",
            "KernelGraphics",
            "KernelGraphics",
            "KernelGraphics",
            "KernelGraphics",
            "KernelGraphics",
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
fn the_unsurvivable_class_still_names_the_measured_crashes() {
    // ★ The class the named boots produced, kept separately from the fail-open class so
    // that widening the second cannot quietly shrink the first.
    //
    // ⚠ 3 -> 8 at the state-load rung, and the five new ones are ONE crash rather than
    // five: `gmmu1` at `12b001f` failed `RmInitAdapter (0x25:0x40:1249)` because GR's
    // static info was NULL, and all five of `kgraphicsLoadStaticInfo_KERNEL`'s
    // `NV_CHECK_OK_OR_GOTO(cleanup)` controls reach that same `cleanup:` label. They are
    // listed separately because each is separately refusable, not because each was
    // separately measured — only `0x20800a1f`, the first, was ever the one that fired.
    let unsurvivable: Vec<u32> = SWEEP_TRIAGE
        .iter()
        .filter(|c| c.disposition == SweepDisposition::AmputationUnsurvivable)
        .map(|c| c.cmd)
        .collect();
    assert_eq!(
        unsurvivable,
        vec![
            0x2080_0a40,
            0x2080_0a1c,
            0x2080_2a08,
            0x2080_0a59,
            0x2080_0a1f,
            0x2080_0a26,
            0x2080_0a22,
            0x2080_0a3d,
            0x2080_0a48,
            0x2080_0a32,
        ],
        "t135a's KernelFifo, t134a's KernelMemorySystem, KernelGmmu's freed pStaticInfo, \
         gmmu1's five GR static-info controls plus stateload1's sixth, and irq1's \
         0x20802a08 — whose zero-length fault method buffer is the ONLY member measured \
         against a real GA106 rather than reasoned out of the tree"
    );
}

#[test]
fn the_triage_universe_is_pinned_so_shortening_it_is_a_red_test() {
    // ★★ A gate quantified over a list is only as strong as the list. Pin its size and its
    // membership here, so removing an entry to make something pass is itself a failure.
    //
    // ⊘ The order is the oracle's own `rpc.sequence` order, so a reader can line this list
    // up against `cargo run -p kayfabe-crec --example cap1b_report` directly.
    assert_eq!(SWEEP_TRIAGE.len(), 32, "the triaged universe's size");
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
            0x2080_0a1f, // seq 49  KernelGraphics — caps                   SERVED
            0x2080_0a2a, // seq 50  KernelGraphics — info
            0x2080_0a26, // seq 51  KernelGraphics — floorsweeping           SERVED
            // ── past the oracle's closure limit: gpuStateLoad's own GR static-info run.
            // ⊘ These are NOT in cap1b and so have no reply-plane coverage from it; their
            // corroboration is the C's captured init-control table instead, which is a
            // different artifact of the same real GA106. See kayfabe_abi::grstatic.
            0x2080_0a22, // KernelGraphics — global SM order                 SERVED
            0x2080_0a30, // KernelGraphics — PPC masks
            0x2080_0a2c, // KernelGraphics — zcull info
            0x2080_0a2e, // KernelGraphics — ROP info
            0x2080_0a3d, // KernelGraphics — FECS record size                SERVED
            0x2080_0a3f, // KernelGraphics — FECS trace defines
            0x2080_0a48, // KernelGraphics — PDB properties                  SERVED
            0x2080_0a32, // KernelGraphics — context buffers info              SERVED
            0x2080_0a38, // KernelGraphics — FECS trace HW enable (teardown)
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
    // ⚠ 2 -> 3 at the L2-evict rung: `0x20800a70` moved here from `RefusalHalts` after
    // its stated reason was found FALSE on the GA106 HAL path — `kbusFlush_GM107`
    // overwrites its status only for `NV_ERR_TIMEOUT`, so a refused sysmembar is
    // swallowed even at `kbusVerifyBar2_GM107:4218-4221`. See its `why`.
    //
    // ⚠⚠ 3 -> 2 at the `irq1`/`fmb` rung, and the departure is a WARNING ABOUT THIS GATE,
    // not just a smaller number. `0x20802a08` left for `AmputationUnsurvivable` because a
    // real GA106 was asked and answered `20480` where the C's captured row carries an
    // EMPTY body. The row satisfied this test — it cited `C:` exactly as demanded — and it
    // was still wrong, because **citing the oracle is not the same as the oracle being
    // right**. Six `dlen = 0` rows of `mode2_initctrl_ga106.h` are now measured to be
    // contradicted by hardware (`kayfabe_abi::fmbsize`), so a `C:` citation of an *empty*
    // body is worth nothing at all.
    //
    // ⊘ The two survivors were re-checked against the same real-GA106 transcript rather
    // than grandfathered: `0x20800a80`'s captured body is non-empty and matches hardware
    // byte for byte, and `0x20800a70` has `psize = 0` — no body exists to disagree about.
    // A future member whose citation is an empty `ctl_` array must be re-measured, not
    // admitted because this assertion is green.
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
    // ★★★ `0x20800301` joined this list at the event-notification rung, and it is the
    // FIRST entry whose triage row argued AGAINST serving it. That row said accepting a
    // notification registration "would promise an interrupt nothing raises"; the argument
    // conflated registering an arming with delivering an event, and the cost of an
    // undelivered notification is only paid for an event that can occur. See
    // `kayfabe_abi::eventnotify::SILENT_NOTIFIERS`, which scopes the promise to exactly the
    // notifiers this device can defend it for.
    // ★★★ `0x20800a6c` joined at the L2-evict rung, and it is the SECOND entry whose row
    // argued against serving it — for a reason of a new kind. The row said an L2
    // invalidate/evict "is an ACTION on the cache, and an NV_OK this port cannot back would
    // tell the guest its framebuffer view is coherent when nothing made it so". The premise
    // holds; the conclusion assumed the coherence had to be MADE. See
    // `kayfabe_abi::l2evict`, which argues the postcondition already holds structurally and
    // names the three futures that would falsify that.
    assert_eq!(
        served,
        vec![0x2080_0a61, 0x2080_0301, 0x2080_0a6c],
        "the halting refusals this port has spent a rung on, in SWEEP_TRIAGE's own order"
    );
    // ⚠ 13 -> 12: `0x20800a70` LEFT this class (to `RefusalIsInvisible`) at the same rung,
    // and it left because its argument was wrong rather than because the port changed.
    // ⚠⚠ 12 -> 8 at the state-load rung, and FOUR MORE left for the same reason: their
    // rows claimed a local `NV_ASSERT_OK_OR_RETURN` / `NV_CHECK_OK_OR_GOTO` "halts the
    // boot at a named statement", and `gmmu1` at `12b001f` showed it does not — every one
    // of those statuses lands in `gpuStatePostLoad`, which maps `NV_ERR_NOT_SUPPORTED` to
    // `NV_OK` at `gpu.c:3438`. ⊘ Four rows in this class were therefore claims about a
    // FUNCTION dressed as claims about a BOOT. `0x20800a9f` went to `RefusalFailsOpen`;
    // `0x20800a1f` and `0x20800a26` to `AmputationUnsurvivable`; `0x20800a2a` to
    // `AmputationIntended`.
    assert_eq!(halts.len(), 8, "non-vacuity: the class is the roadmap");
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
