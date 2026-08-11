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
//!
//! ## ★★★★ Its THIRD catch is the first one that was a REAL violation — and it left no row
//!
//! `[measured 2026-08-11, `b6c5442`]` w232 added `CeShellState::sysproc_kept:
//! std::sync::Mutex<u64>` and this gate went RED on `origin/master`. The first two catches
//! (`Mutex<ParkState>`, `Mutex<u32>`) were locks that turned out to be **safe**, so the fix
//! was a row. This one was not: the guard was held across the `eprintln!` that prints the
//! counter *and* across the `String` one of its arguments builds, so the process-global
//! stderr lock, a `write(2)` on it and a heap allocation all ran beneath an unranked lock on
//! the vCPU's own MMIO trap. R1 forbids each of those by name.
//!
//! ⊘ **So the fix deleted the lock rather than ruling on it** — the field is an `AtomicU64`
//! now — and that is why nothing was added to the table below. ★ Note what that means for
//! reading this list: *the gate's catches are not all here.* A row records a lock that was
//! **kept**; a lock that was removed leaves the table exactly as it found it. The gate is a
//! trigger for the review obligation, and the obligation's answer is sometimes "delete it".

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
        "crates/kayfabe-qemu-raw/src/shim.rs",
        "Mutex<u32>",
        "★★ FOUND BY THIS GATE, 2026-08-10, and it had ALREADY SHIPPED: `CeShellState::gr_dumps`,          §16.79's bounded GR-pushbuffer dump counter, arrived at `2f616e2` and was never          classified — so `cargo test --workspace` was RED at `fe65678` and two `BOOTED` commits          were made on top of it. ⊘ The mask is cargo's own: without `--no-fail-fast` the run          stops at the first failing target, so ONE unrelated red hides every gate behind it.          The lock itself is SAFE and deliberately so: `dump_gr_pushbuffer_once` (shim.rs:3371-3377)          takes it inside its own block and DROPS it before the dump does anything — every          `eprintln!`, the `plane.upgrade()`, the root resolution and the memory-plane lock are          outside that scope. Nothing blocks beneath it.",
    ),
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
    (
        "crates/kayfabe-qemu-raw/src/shim.rs",
        "Mutex<Option<QemuVmm>>",
        "★★★ THE SECOND HAZARD, and it was invisible until this gate's scope reached the MMIO          handler. The shell's guest-memory port, taken in `try_ce_submission` and held across          the WHOLE copy-engine submission — the ring walk, the pushbuffer read, every CPU          sub-copy and the completion write all run beneath it, and beneath the plane's          `ce_session` as well. ⊘ NOTHING may block beneath it: a wait here stalls the vCPU          that took the trap while the plane's own FSM mutex is also out. It is bounded today          only because every operation beneath it is a memcpy against a resident store — an          executor that ever waits on a host isolate must take this lock after the wait, not          across it.",
    ),
    (
        "crates/kayfabe-qemu-raw/src/shim.rs",
        "Mutex<std::collections::BTreeMap<(u32, u32), kayfabe_rt::ceutils::GpCursor>>",
        "Per-channel GPFIFO read cursors. Taken for a single map read, and again for a single          insert on the success arm; no call of any kind beneath it, and it is never held          across the submission it belongs to.",
    ),
    (
        "crates/kayfabe-qemu-raw/src/shim.rs",
        "Mutex<std::collections::BTreeMap<(u32, u32), kayfabe_rt::ceutils::MethodState>>",
        "Per-channel method accumulators, keyed and committed exactly like the cursors beside          them. Taken for a single map read, and again for a single insert on the success arm;          no call of any kind beneath it.",
    ),
    (
        "crates/kayfabe-rt/src/completion_watch.rs",
        "Mutex<Inner>",
        "★★★★★ THE COMPLETION OBSERVER'S WATCH LIST, and it is held by TWO threads — the only \
         entry in this table of which that is true. The vCPU takes it for one map insert \
         (`WatchList::declare`) and for one `+= 1` (`attempt`); the observer's reactor thread \
         takes it across a whole `sweep`, which runs the caller-supplied READER over every \
         live watch. ⊘ NOTHING may block beneath it, and the reader is what makes that a real \
         constraint rather than a note: today it is `QemuVmm::gpa_read`, a memcpy off a \
         resident host pointer that takes no BQL and no ranked lock. ⚠ A future reader that \
         waits — a host-isolate round trip, an eventfd read — must be moved OUT of the sweep \
         and its result merged in, because a vCPU declaring a completion would otherwise queue \
         behind it on the MMIO trap. The sweep is deliberately inside the guard so the read \
         and the verdict cannot come from two instants; that choice is what pins the reader's \
         cost.",
    ),
    (
        "crates/kayfabe-qemu-raw/src/shim.rs",
        "Mutex<Option<ObserverThread>>",
        "★★★ The observer thread's handle. Taken for a `Some`/`None` check plus an eventfd \
         `signal` (`poke_observer`), and once at teardown to `take()` the thread out before it \
         is joined. ⊘ NOTHING may block beneath it. The JOIN happens AFTER the guard is dropped — deliberately, and it is \
         the one thing that would be wrong here: joining a thread beneath this lock while that \
         thread's sweep can be woken by a vCPU holding it is a deadlock, not a stall. ⚠ Every \
         vCPU entry is `poke_observer`, which is entered with no ranked lock held (the CE \
         session, the memory-plane port and the rank-0 device read are all released before \
         the declare), and `Notifier::signal` asserts exactly that.",
    ),
    (
        "crates/kayfabe-qemu-raw/src/shim.rs",
        "Mutex<DoorbellCensus>",
        "★★★★ §16.65 — the per-engine doorbell census. Taken for a single `+= 1` on a fixed \
         array and released before the routing decision it counts; taken again, alone, to copy \
         the whole (`Copy`) struct out at audit time. ⊘ NO call of any kind runs beneath it, and \
         it is deliberately NOT held across `try_ce_submission`'s body — the tally must not \
         become a second lock nested inside the guest-memory port two entries up, which is \
         already this file's second named hazard. It is a fixed-size array of counters with no \
         guest-supplied key, so it can neither grow nor allocate while held.",
    ),
];

/// Crates a vCPU thread executes through — the scope of the list above.
///
/// ★★★★ `kayfabe-qemu-raw` **is the MMIO handler**, and it was missing. A vCPU trap enters
/// this port before it reaches either of the other two, so a gate that stopped at
/// `kayfabe-device`/`kayfabe-rt` was honest about what it checked and simply did not check
/// the place the guest arrives. ⊘ Same family as every scoped instrument in this campaign:
/// the scope, not the assertion, was where the hole was.
const VCPU_PATH_CRATES: &[&str] = &["kayfabe-device", "kayfabe-rt", "kayfabe-qemu-raw"];

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
        collect_locks(&text, &rel, out);
    }
}

/// ★★★★ **Whole-FILE, bracket-matched — not line-by-line.**
///
/// # ⊘ The line scanner missed a lock I had just added, in this very campaign
///
/// `rustfmt` breaks a long field after the colon:
///
/// ```text
///     cursors:
///         std::sync::Mutex<std::collections::BTreeMap<(u32, u32), …::GpCursor>>,
/// ```
///
/// so the line carrying the name has no type on it and the line carrying the type has no
/// `": "` on it. `[measured 2026-08-09]` **both** `CeShellState::cursors` and
/// `CeShellState::states` are declared exactly like that, and the old scan reported the
/// crate as carrying **one** lock when it carries three. ⊘ That is the second spelling this
/// scanner has been blind to — the first (`std::sync::Mutex<…>` fully qualified) is recorded
/// at `executor.rs`'s entry — and it is the same defect both times: *the gate was honest
/// about what it checked and did not check the shape the code was actually written in.*
///
/// ⚠ A `//` inside a string literal would truncate that line. No file in scope has one, and
/// the failure direction is **toward a false positive** (a lock reported that is really in a
/// comment), which fails loudly rather than passing quietly.
fn collect_locks(text: &str, rel: &str, out: &mut BTreeSet<(String, String)>) {
    // 1. Strip line comments, then flatten to ONE string so a declaration split across lines
    //    is the same input as one written on a single line.
    let mut flat = String::new();
    for line in text.lines() {
        let code = match line.find("//") {
            Some(i) => &line[..i],
            None => line,
        };
        flat.push_str(code);
        flat.push(' ');
    }
    // ⊘ BYTES throughout, deliberately. The first draft searched with `str::find` (which
    // returns a BYTE offset) and indexed a `Vec<char>` with the result. Every `★`, `⊘` and
    // `⚠` in a string literal in the scanned crates is 3 bytes and 1 char, so the two
    // indices diverge and the walk lands in the middle of an unrelated declaration —
    // `plane.rs`'s three locks vanished from the scan while the crate plainly declares them.
    // The structural characters here (`<`, `>`, `:`) are all ASCII, so every slice boundary
    // taken below is a char boundary by construction.
    let b = flat.as_bytes();
    for kind in ["Mutex", "RwLock"] {
        let k8 = kind.as_bytes();
        let mut from = 0usize;
        while let Some(rel_at) = flat[from..].find(kind) {
            let at = from + rel_at;
            from = at + k8.len();
            // The name must be followed immediately by `<` — `Mutex` alone is a use or a
            // turbofish, not a field type.
            if b.get(at + k8.len()) != Some(&b'<') {
                continue;
            }
            // 2. Walk back over the path qualifier (`std::sync::`) to the start of the type.
            let mut s0 = at;
            while s0 > 0
                && (b[s0 - 1].is_ascii_alphanumeric() || b[s0 - 1] == b'_' || b[s0 - 1] == b':')
            {
                s0 -= 1;
            }
            // 2b. ★★ The last path segment must BE the lock name. ⊘ Without this,
            //     `RankedMutex<Proc>` matches on its `Mutex` tail and the gate demands the
            //     classification of the rank system itself — a false positive, and a gate
            //     that cries wolf gets its list padded rather than read.
            if flat[s0..at + k8.len()].rsplit("::").next() != Some(kind) {
                continue;
            }
            // 3. It is a FIELD only if a `:` (the field's own colon, not a path's `::`)
            //    precedes it. Anything else is a `let`, a return type or a bound.
            let mut k = s0;
            while k > 0 && b[k - 1].is_ascii_whitespace() {
                k -= 1;
            }
            if k == 0 || b[k - 1] != b':' || (k >= 2 && b[k - 2] == b':') {
                continue;
            }
            // 4. BRACKET-MATCH the generic argument, so a nested `BTreeMap<…>` cannot end
            //    the type early. An unbalanced tail is skipped rather than guessed.
            let mut depth = 0i32;
            let mut end = None;
            for (i, c) in b.iter().enumerate().skip(at + k8.len()) {
                match c {
                    b'<' => depth += 1,
                    b'>' => {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(i);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let Some(e) = end else { continue };
            // 5. Normalise whitespace, so how rustfmt broke the line cannot change the key.
            let ty = flat[at..=e]
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            out.insert((rel.to_owned(), ty));
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
