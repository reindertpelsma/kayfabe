//! ★★★ EVERY POLICY MODULE MUST BE ON A LIVE PATH — not merely `pub`.
//!
//! `[owner, 2026-09-21]` asked whether v3 was *wired*. It was not: seven modules were correct,
//! tested, and **called by nothing outside their own file and the tests**. A component nothing
//! calls enforces nothing — `Submission::decide` refusing to fault a kernel channel is worth
//! exactly zero until `worker_pass` calls it.
//!
//! ⊘ This tree has paid for that shape repeatedly: *the passthrough verb is built and orphaned*,
//! *the new design was unreachable by default*, and *the orphan gate asks VISIBILITY, not
//! REACHABILITY*. ⇒ This gate asks reachability: it reads the composed files and requires each
//! policy module to be named there.

const PLANE: &str = include_str!("../src/plane.rs");
const TRAP: &str = include_str!("../src/trap.rs");

/// Each policy module, and the §-rule it would stop enforcing if it fell off the path.
/// ⊘⊘⊘ `[fable w823, HIGH H2]` the first version asked `contains("use crate::shadow")`, which is
/// satisfied by importing the enum `WriteSemantics` while `Cell` and `Trigger` are never called.
/// **The gate that claimed to ask reachability asked visibility — the exact failure its own
/// docstring names.** ⇒ Name the SYMBOL that must be called, not the module that must be imported.
const MUST_CALL: [(&str, &str); 4] = [
    ("Submission", "§7 — the kernel-channel refusal"),
    ("Completion::for_route", "§8 — the forge licence"),
    ("VmCaps", "§9.1 — per-VM twin caps"),
    ("read_traps.policy", "§5 — the read-trap allowlist"),
];

const MUST_BE_REACHED: [(&str, &str); 8] = [
    ("channel", "§7 — an untranslatable operand on a KERNEL channel must refuse, never fault"),
    ("completion", "§8 — a forge is licensed only where no GPU work ran"),
    ("readtrap", "§5 — the read-trap allowlist, and its phase scoping"),
    ("caps", "§9.1 — per-VM twin caps, the only thing stopping one guest starving another"),
    ("wake", "§5.3 — the single wakeup word"),
    ("leaf", "§6.4 — the system-memory bound; a guest leaf may never name OUR memslots"),
    ("lifetime", "§9 — the teardown order; close() is not a synchronous free"),
    ("shadow", "§5.5 — write semantics; a plain store is wrong for four of the five kinds"),
];

#[test]
fn every_policy_module_is_reached_from_a_composed_path() {
    let body = format!("{PLANE}\n{TRAP}");
    let mut orphaned = Vec::new();
    for (m, why) in MUST_BE_REACHED {
        let used = body.contains(&format!("crate::{m}::")) || body.contains(&format!("use crate::{m}"));
        if !used {
            orphaned.push(format!("{m} — would stop enforcing: {why}"));
        }
    }
    for (sym, why) in MUST_CALL {
        if !body.contains(sym) {
            orphaned.push(format!("{sym} is never CALLED — would stop enforcing: {why}"));
        }
    }
    assert!(
        orphaned.is_empty(),
        "⊘ ORPHANED POLICY MODULE(S) — correct, tested, and called by nothing:\n  {}",
        orphaned.join("\n  ")
    );
}
