//! # ★★★ The locks the R1 witness cannot see are ENUMERATED (`l1_concurrency.md` §3.3.1)
//!
//! R1 says *"no blocking call under ANY lock, ever"*. `kayfabe_util::lockwitness` enforces it
//! over a mask of **ranked** locks — device = 0, proc = 1, leaf = 2. A plain `std::sync::Mutex`
//! that nobody ranked is invisible to that mask, so `assert_lock_free` returns cleanly while
//! one is held.
//!
//! ⚠ `[measured]` 2026-08-06 — this is not hypothetical. A control-path host call was being
//! designed against that assert while the caller held `RegPlane`'s unranked FSM mutex
//! (`kayfabe-device/src/plane.rs:802`, taken at `:1922` across the whole policy chain). It
//! would have compiled, passed every assertion, and stalled **every vCPU's MMIO** for the
//! duration of a host round trip — multi-second at RM's timeout, forever if the worker wedged.
//! `plane.rs:818-825` already says the doorbell port lives outside that mutex *"a requirement
//! rather than a preference"* — enforced, until this file, by nothing but that comment.
//!
//! ## What this gate does and does not buy
//!
//! ⊘ **It does not detect the violation.** Nothing here fires when a blocking call runs under
//! an unranked lock; that remains a review obligation. What it does is stop the *set* from
//! growing silently: a new unranked lock in the two crates a vCPU thread runs through goes RED
//! until someone classifies it, which is when the review obligation gets asked for.
//!
//! ★ Same shape as `l1_os_shell.md` §4.2.1.1's residue list and the trybuild `REQUIRED_ROWS`:
//! the failure mode being guarded is a list quietly getting longer without anyone noticing,
//! and the answer is enumeration by name in both directions.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// ★★★ Every unranked lock declared in the crates a **vCPU thread** executes through, with
/// whether a blocking call may run beneath it.
///
/// ⊘ Scoped to `kayfabe-device` and `kayfabe-rt` deliberately. The other ~20 in the workspace
/// are in mocks, or in the isolate **child process** where no vCPU thread exists, so a stall
/// there cannot wedge the guest's register plane. Widening the scope would trade the property
/// this list actually asserts for a longer list nobody reads.
///
/// ⚠ The entry that matters is `plane.rs`'s `Mutex<PlaneState>`: it is held across the whole
/// policy chain on the vCPU's own MMIO trap, so anything blocking beneath it blocks every
/// other vCPU's register access too.
const UNRANKED_VCPU_PATH_LOCKS: &[(&str, &str, &str)] = &[
    (
        "crates/kayfabe-rt/src/device.rs",
        "Mutex<GateState>",
        "PoolGate backpressure. BLOCKING IS ITS PURPOSE — a caller waits here for a worker to \
         come back. Safe because every ranked guard is dropped before entry (device.rs:332-398 \
         re-enters from the top after the wait).",
    ),
    (
        "crates/kayfabe-device/src/plane.rs",
        "Mutex<PlaneState>",
        "★★★ THE HAZARD. The register-plane FSM mutex, taken at :1922 on the vCPU MMIO trap and \
         held across the entire policy chain. ⊘ NOTHING may block beneath it: a wait here stalls \
         every vCPU's register access, and the R1 witness will not say so.",
    ),
    (
        "crates/kayfabe-device/src/plane.rs",
        "RwLock<Box<dyn DoorbellPort>>",
        "The doorbell port, deliberately OUTSIDE `state` (:818-825) precisely so the doorbell's \
         ranked-lock + backpressure path is not run beneath the FSM mutex. Blocking beneath THIS \
         one is the design.",
    ),
    (
        "crates/kayfabe-rt/src/executor.rs",
        "Mutex<ParkState>",
        "★ FOUND BY THIS GATE, 2026-08-06 — it was absent from the hand-built list because the          first scanner missed the qualified `std::sync::Mutex<…>` spelling. `Parker`'s condvar          state: blocking beneath it IS the mechanism — `Condvar::wait` atomically releases this          mutex while it waits, so the mutex is not held across the wait at all. Nothing else is          called beneath it.",
    ),
    (
        "crates/kayfabe-device/src/plane.rs",
        "Mutex<DoorbellLog>",
        "A bounded diagnostic log. Held for a push and released; no call of any kind beneath it.",
    ),
];

/// Crates a vCPU thread executes through — the scope of the list above.
const VCPU_PATH_CRATES: &[&str] = &["kayfabe-device", "kayfabe-rt"];

/// The ranked wrappers' own inners. These ARE the rank system, not holes in it.
const RANKED_WRAPPER_INNERS: &[&str] = &["crates/kayfabe-rt/src/lock.rs"];

fn repo_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p
}

/// Every `(path, lock type)` declared as a struct field in the scoped crates.
fn declared_locks() -> BTreeSet<(String, String)> {
    let root = repo_root();
    let mut out = BTreeSet::new();
    for c in VCPU_PATH_CRATES {
        walk(&root.join("crates").join(c).join("src"), &root, &mut out);
    }
    out
}

fn walk(dir: &Path, root: &Path, out: &mut BTreeSet<(String, String)>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, root, out);
            continue;
        }
        if p.extension().is_none_or(|x| x != "rs") {
            continue;
        }
        let rel = p
            .strip_prefix(root)
            .expect("under the repo root")
            .to_string_lossy()
            .into_owned();
        if RANKED_WRAPPER_INNERS.contains(&rel.as_str()) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&p) else {
            continue;
        };
        for line in text.lines() {
            let t = line.trim();
            if t.starts_with("//") || t.starts_with("///") {
                continue;
            }
            // `name: Mutex<T>,` — but ALSO `name: std::sync::Mutex<T>,`.
            //
            // ⚠ `[measured]` 2026-08-06: the first version of this scan matched only the bare
            // form, so a bite-check mutation using the fully-qualified path was NOT CAUGHT. A
            // scanner that misses a spelling of the thing it scans for is a gate that reports
            // clean while the class walks past it — the instrument-defect shape this whole
            // file is about, found in this file, by breaking it.
            let Some(colon) = t.find(": ") else { continue };
            let rest = t[colon + 2..].trim();
            // Strip any path qualifier: `std::sync::Mutex<T>` -> `Mutex<T>`.
            let bare = match rest.find('<') {
                Some(lt) => match rest[..lt].rfind("::") {
                    Some(sep) => &rest[sep + 2..],
                    None => rest,
                },
                None => rest,
            };
            for kind in ["Mutex<", "RwLock<"] {
                if !bare.starts_with(kind) {
                    continue;
                }
                let ty = bare.trim_end_matches(',').trim().to_owned();
                out.insert((rel.clone(), ty));
            }
        }
    }
}

// =====================================================================================

/// ★★★ The declared set and the actual set are EQUAL, in both directions.
///
/// A new unranked lock on the vCPU path is the silent growth this guards; a stale entry means
/// the list describes code that no longer exists, which makes every other entry less
/// trustworthy.
#[test]
fn every_unranked_lock_a_vcpu_thread_can_hold_is_classified() {
    let actual = declared_locks();
    let declared: BTreeSet<(String, String)> = UNRANKED_VCPU_PATH_LOCKS
        .iter()
        .map(|(f, t, _)| ((*f).to_owned(), (*t).to_owned()))
        .collect();

    assert!(
        !actual.is_empty(),
        "the scan found no locks at all in {VCPU_PATH_CRATES:?} — the walker is broken, and a \
         broken walker makes this gate vacuously satisfiable"
    );

    let unclassified: Vec<&(String, String)> = actual.difference(&declared).collect();
    assert!(
        unclassified.is_empty(),
        "★★★ a new UNRANKED lock appeared on the vCPU path and nobody said whether a blocking \
         call may run beneath it. `lockwitness::assert_lock_free` CANNOT see it — it masks only \
         ranked locks — so a wait beneath this will pass every assertion and stall the register \
         plane. Classify it in UNRANKED_VCPU_PATH_LOCKS: {unclassified:#?}"
    );

    let vanished: Vec<&(String, String)> = declared.difference(&actual).collect();
    assert!(
        vanished.is_empty(),
        "⊘ these are classified but no longer declared — a list describing code that does not \
         exist makes its other rows less believable: {vanished:#?}"
    );
}

/// ★★ Every entry says something about blocking, because the classification IS the artefact.
///
/// ⊘ A row whose note does not mention blocking is a name with no ruling attached, and the
/// whole point of the list is the ruling.
#[test]
fn every_classification_states_whether_blocking_is_permitted_beneath_it() {
    for (file, ty, why) in UNRANKED_VCPU_PATH_LOCKS {
        let w = why.to_lowercase();
        assert!(
            w.contains("block") || w.contains("wait") || w.contains("no call"),
            "★ `{ty}` in `{file}` is listed without saying whether anything may block beneath \
             it — that ruling is the reason the list exists, not decoration"
        );
        assert!(
            why.len() > 60,
            "`{ty}` in `{file}` has a note too short to carry a ruling and its reason"
        );
    }
}
