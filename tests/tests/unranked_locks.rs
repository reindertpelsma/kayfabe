//! # ★★★ The locks the R1 witness cannot see are ENUMERATED (`l1_concurrency.md` §3.3.1)
//!
//! ## ★★★★★ WHICH CLAUSE THIS GATE ENFORCES, AND WHICH IT CANNOT SEE `[w300, 2026-08-13]`
//!
//! `blocking_and_completion_model.md` §1 states the predicate a guest-trap site must satisfy:
//!
//! > **INLINE-SAFE(site) ⇔ (a)** completes **without the guest running**, **and (b)** completes
//! > **within the shortest guest-side timeout covering it**, **and (c)** holds **no lock
//! > another vCPU's trap path takes**.
//!
//! - **(c) — ENFORCED for RANKED locks**, by `kayfabe_util::lock::check_acquire`: a total order,
//!   checked *before* the OS-level acquire, always-on (not a `debug_assert`). An inversion
//!   panics by name instead of occasionally deadlocking.
//! - **(c) — ENUMERATED, NOT ENFORCED, for the unranked locks in this file.** Nothing fires when
//!   one is taken in a bad order. What this gate buys is that the *set* cannot grow silently:
//!   a new unranked lock goes RED until someone writes the ruling down.
//! - **(a) and (b) — NOT VISIBLE HERE AT ALL.** Neither has a mechanism anywhere in the tree
//!   (that doc's §4 says so). A lock on this list that is held across a wait for something the
//!   *guest* must do is an (a) violation this gate is structurally unable to report, and every
//!   row's ruling is therefore about (c) and about blocking-in-general, never about (a).
//!
//! ⚠ **And the stakes are higher than "a stalled thread"**: every guest MMIO write arrives with
//! the **QEMU BQL** held (`shim.rs:4877`, `:6146`, `:6046`), so blocking in a trap handler
//! freezes **every vCPU and QEMU's main loop** — the whole VM, not the ringing thread.
//!
//! ★ This block exists because of *how* (c) stayed violated for months: `assert_lock_free`
//! masked only ranked locks and **nothing said so at the point of use**. Saying it in prose in
//! one file was not enough once — so it is said here, in the file a reader lands in.
//!
//! R1 says *"no blocking call under ANY lock, ever"*. `kayfabe_util::lockwitness` enforces it
//! over a mask of **ranked** locks — device = 0, proc = 1, leaf = 2. A plain `std::sync::Mutex`
//! that nobody ranked is invisible to that mask, so `assert_lock_free` returns cleanly while
//! one is held.
//!
//! ## ⊘⊘ CORRECTED `[w236, 2026-08-11]` — THE HEADLINE ROW IS GONE, because it was RANKED
//!
//! Everything below described `Mutex<PlaneState>` as this list's worst entry. **It is no longer
//! on the list**: it is [`kayfabe_util::lock::LockRank::Plane`] (rank 0) as of w236, so
//! `check_acquire` now refuses `core → plane` deterministically and `assert_lock_free` fires
//! beneath it. ⇒ **the 2026-08-06 paragraph below is a HISTORICAL account, not a live hazard**,
//! and the "review obligation" it hands off is discharged for that lock by
//! `plane_lock_is_visible_to_the_witness.rs`.
//!
//! ★ Why it was ranked **below** `Device` rather than above: the shipping order is
//! **plane → core**, and ranks are acquired in strictly increasing order, so the plane must
//! sort first or every vCPU MMIO trap would panic. §16.87.
//!
//! ⚠ **What did NOT change**: every other row here is still unranked and still carries only a
//! review obligation. Ranking one lock does not rank the rest.
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
/// ⊘⊘ **CORRECTED `[w236, 2026-08-11]`, above the sentence it corrects.** This doc used to
/// say *"the entry that matters is `plane.rs`'s `Mutex<PlaneState>`"*. **That row is gone** —
/// the lock is now `LockRank::Plane` and is enforced, not merely enumerated (§16.87).
/// ⇒ ★ The rows that remain are the ones nobody has ranked **yet**, and none of them is held
/// across a policy chain on the MMIO trap. That is a weaker list than it used to be, which is
/// the point: **the strongest entry left this table by being fixed, not by being reworded.**
const UNRANKED_VCPU_PATH_LOCKS: &[(&str, &str, &str)] = &[
    (
        "crates/kayfabe-qemu-raw/src/kftime.rs",
        "Mutex<Vec<(&'static str, Census)>>",
        "★★ FOUND BY THIS GATE, 2026-08-14, the day w315 added it — and the gate was right to \
         stop the commit. This is the segment timer's per-kind census, and it is taken on the \
         vCPU thread inside EVERY MMIO trap, which is the hottest path this table describes. \
         ⊘ NOTHING MAY BLOCK BENEATH IT, and nothing does: the guard's whole body is a linear \
         scan of a <10-element `Vec`, a few `u64` adds and a comparison. The `eprintln!` that \
         a periodic census triggers is deliberately OUTSIDE the guard (`record_inner` computes \
         `due` inside a block and prints after it drops) — printing beneath it would put a \
         file write under a lock on the vCPU's trap. ⚠ And the instrument exists to explain \
         the guest's latency, so a wait here would be the measurement causing the thing it \
         measures. It is also OFF by default: `record` returns before touching this lock \
         unless `KAYFABE_KFTIME` armed it.",
    ),
    (
        "crates/kayfabe-qemu-raw/src/kftime.rs",
        "Mutex<Vec<(&'static str, HotCensus)>>",
        "The hot-offset census, beside the one above and with the same discipline: taken on \
         the vCPU inside every MMIO trap, held for one bounded linear scan (capped at \
         `HOT_OFFSETS = 96` rows) and one counter update, with no call of any kind beneath \
         it. ⊘ The cap is what makes the scan bounded rather than guest-controlled — an \
         unbounded row set would let a guest touching fresh offsets grow the critical \
         section it holds this lock across. Off by default, like its neighbour.",
    ),
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
    // =================================================================================
    // ★★★★★ THE NINE BELOW WERE INVISIBLE UNTIL `[w300, 2026-08-13]` — every one of them
    // is spelled `Arc<Mutex<…>>`, and the scanner's field test rejected a lock preceded by
    // `<` instead of by its field's own colon. See `collect_locks` step 3 and
    // `the_scanner_finds_every_spelling_a_lock_has_ever_been_written_in`.
    // ⊘ Note WHICH locks the blind spot hid: `Arc<Mutex<…>>` is how a lock shared between
    // the vCPU and another thread is written, so the spelling this gate could not see was
    // exactly the spelling the dangerous entries use.
    // =================================================================================
    (
        "crates/kayfabe-qemu-raw/src/shim.rs",
        "Mutex<Vec<GrCursorWatch>>",
        "★★★★★ `CeShellState::gr_cursors`, and the most load-bearing of the nine: like \
         `completion_watch`'s `Mutex<Inner>` it is held by TWO threads — the vCPU pushes one \
         row per GR channel at the first declare (:5252), the observer's reactor clones it \
         every tick (:3766). ⊘ NOTHING may block beneath it, and today nothing does: the \
         reader CLONES the `Vec` and DROPS the guard before it touches the plane (:3762-3768, \
         whose own comment gives the reason — holding it across a plane read *'would nest two \
         unranked locks on two threads in two orders'*), and the vCPU writer drops the guard \
         before the `eprintln!` on its miss arm (:5263). ⚠ Its doc at :3526 already CLAIMED a \
         classification in this file; there was none, because this gate could not see the \
         field. A doc asserting a ruling that does not exist is worse than silence.",
    ),
    (
        "crates/kayfabe-device/src/osevent.rs",
        "Mutex<Vec<OsEventRegistration>>",
        "`OsEventLog::live` — the live `(hClient, hEvent)` registrations. Taken for a \
         scan-then-push (:156), a `retain` (:192), a `map(post).collect()` (:213) and a \
         `clone` (:220). ⊘ No call beneath it may block and none does: \
         `OsEventRegistration::post` (:115-121) copies three `Copy` fields into a `PostEvent` \
         and calls nothing. Every counter beside it is an `AtomicU64` outside the guard.",
    ),
    (
        "crates/kayfabe-device/src/osevent.rs",
        "Mutex<JoinPoint>",
        "`OsEventLog::last_join` — one `Copy` struct recording the last delivery join. Taken to \
         overwrite it (:352) and to copy it out (:384). No call of any kind runs beneath it, so \
         nothing can block; the `woke_with_nothing` tally beside it is a relaxed atomic.",
    ),
    (
        "crates/kayfabe-device/src/setpagedir.rs",
        "Mutex<Option<SetPageDirRecord>>",
        "`SetPageDirLog::latest` — the most recent accepted `SET_PAGE_DIRECTORY`, a single \
         `Copy` record. Taken to read it out (:259), to overwrite it (:292) and to clear it \
         (:307). No call runs beneath it and nothing may block there; the totals are atomics.",
    ),
    (
        "crates/kayfabe-device/src/bar2.rs",
        "Mutex<BarPdes>",
        "`BarPdeLog::pdes` — the published BAR1/BAR2 PDEs, two `Option<u64>`s. Taken to copy \
         them out (:159), for a single field assignment (:181) and to reset (:206). ⊘ No call \
         beneath it, so nothing can block; `updates`/`refusals` are atomics outside the guard.",
    ),
    (
        "crates/kayfabe-device/src/unserviced.rs",
        "Mutex<Vec<UnservicedCommand>>",
        "`UnservicedLog::seen` — the de-duplicated sample of commands we did not service. \
         Taken for a `clone` (:262) and for the contains-then-push (:274-280), deliberately in \
         ONE guard so the distinct counter and the membership set cannot disagree. ⊘ Nothing \
         blocks beneath it and nothing may: the push is capped at `UNSERVICED_SAMPLE_MAX`, so \
         a hostile guest cannot make the critical section grow.",
    ),
    (
        "crates/kayfabe-device/src/faultbuffer.rs",
        "Mutex<Vec<FaultBufferNote>>",
        "`FaultBufferLog::seen` — the sample of fault-buffer notes. Taken for a `clone` (:159) \
         and a capped push (:186-189). ⊘ No call runs beneath it and none may block. Unlike \
         the unserviced ledger it does NOT de-duplicate, so the cap \
         (`FAULT_BUFFER_SAMPLE_MAX`) is the only bound on the section — which is why the push \
         is guarded by the length test rather than trimmed afterwards.",
    ),
    (
        "crates/kayfabe-device/src/gvaspub.rs",
        "Mutex<GvasPubInner>",
        "`GvasPubLog::inner` — the VA-space publication table and its report sample. Taken for \
         the table insert plus the capped sample push (:270), a single `+= 1` (:316), a \
         `clone` into a snapshot (:323) and a reset (:343). ⊘ Nothing blocks beneath it and \
         nothing may; the table is keyed last-write-wins on `(client, object)` so it is \
         bounded by the guest's live objects rather than by its call count.",
    ),
    (
        "crates/kayfabe-device/src/census.rs",
        "Mutex<CensusInner>",
        "`ControlCensus::inner` — the served/arming/bind tallies. Taken for a field set (:244) \
         and for three find-then-push-or-increment paths (:250, :272, :292), each of which \
         does a linear scan of a distinct-row vector under the guard, plus a `clone` into a \
         snapshot (:312). ⊘ No call beneath it may block and none does. ⚠ The linear scans \
         are the one thing to watch: they are bounded by the number of DISTINCT rows, not by \
         guest call count, so this stays O(small) only while the row keys stay coarse.",
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
            //
            // ★★★★★ **…possibly through one or more WRAPPER generics — the THIRD spelling
            // this scanner was blind to** `[w300, 2026-08-13]`. A lock inside an `Arc` is
            // preceded by `<`, not by its field's colon:
            //
            // ```text
            //     gr_cursors: std::sync::Arc<std::sync::Mutex<Vec<GrCursorWatch>>>,
            //                               ^ the byte before `std::sync::Mutex`
            // ```
            //
            // so the bare `b[k-1] != b':'` test rejected it and **NINE** unranked vCPU-path
            // locks were never classified — the gate reporting `unclassified.is_empty()`
            // the whole time. `Arc<Mutex<…>>` is the *normal* way to write a lock shared
            // between the vCPU and the observer's reactor thread, so this blind spot was
            // aimed squarely at the entries that matter most.
            // ⊘ Stepping out is bounded and structural: skip back over the wrapper's own
            // path name and look again. A `<` with no name before it is not a wrapper, and
            // a generic ARGUMENT (`BTreeMap<K, Mutex<V>>` — preceded by `,`) still does not
            // match, which is right: the lock there is not this field's to hold.
            let mut s = s0;
            let is_field = loop {
                let mut k = s;
                while k > 0 && b[k - 1].is_ascii_whitespace() {
                    k -= 1;
                }
                if k == 0 {
                    break false;
                }
                if b[k - 1] == b':' {
                    // A single `:` is the field's; a `::` means we are still inside a path.
                    break !(k >= 2 && b[k - 2] == b':');
                }
                if b[k - 1] != b'<' {
                    break false;
                }
                let mut w = k - 1;
                while w > 0
                    && (b[w - 1].is_ascii_alphanumeric() || b[w - 1] == b'_' || b[w - 1] == b':')
                {
                    w -= 1;
                }
                if w == k - 1 {
                    break false;
                }
                s = w;
            };
            if !is_field {
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

/// ★★★★★ **THE KNOWN-POSITIVES — the scanner is run against shapes it MUST find, before any
/// zero it reports is believed.**
///
/// # ⊘ Why `!actual.is_empty()` is not this check, and never was
///
/// The gate above already refuses an empty scan. That floor proves the walker found
/// *something*; it cannot distinguish *"the tree declares eleven locks and we found eleven"*
/// from *"the tree declares twenty and we found the eleven written in the one spelling this
/// scanner understands."* Both report **zero unclassified**, and the second is the state this
/// file has now been in **three times**:
///
/// | shape | found by | rows it hid |
/// |---|---|---|
/// | `std::sync::Mutex<…>` fully qualified | `[2026-08-06]` `Mutex<ParkState>` | 1 |
/// | field broken after the colon by rustfmt | `[2026-08-09]` `CeShellState::cursors` | 2 |
/// | **a lock nested inside `Arc<…>`** | `[w300, 2026-08-13]` **this test** | **9** |
///
/// ★★★ Each time the gate was **green** while blind, each time the fix was to the scanner,
/// and each time the blindness was found by someone reading the code rather than by the gate.
/// ⇒ **this tree's rule, applied to this file: a census zero needs a known-positive.** The
/// fixtures below are that known-positive, and they are checked against the scanner
/// **directly**, so a regression fails here — naming the shape — instead of silently
/// shortening the list the gate above compares.
///
/// ⊘ **Fixtures, not the live tree, and deliberately.** A known-positive that reads the real
/// source dies the moment someone refactors the field it names, and its death looks like a
/// pass. These strings cannot rot.
#[test]
fn the_scanner_finds_every_spelling_a_lock_has_ever_been_written_in() {
    /// `(fixture, what must be found, why this shape is here)`
    const KNOWN_POSITIVES: &[(&str, &str, &str)] = &[
        (
            "struct S { plain: Mutex<u64>, }",
            "Mutex<u64>",
            "the baseline spelling — if this fails the scanner is broken outright",
        ),
        (
            "struct S { sysproc_kept: std::sync::Mutex<u64>, }",
            "Mutex<u64>",
            "★★★ `CeShellState::sysproc_kept` AS IT WAS DECLARED at `b6c5442` — the gate's \
             third catch and its first TRUE POSITIVE (a guard held across an `eprintln!` and \
             a `String` build, on the vCPU's own MMIO trap). The first scanner could not see \
             this spelling at all.",
        ),
        (
            "struct S {\n    cursors:\n        std::sync::Mutex<std::collections::BTreeMap<(u32, u32), GpCursor>>,\n}",
            "Mutex<std::collections::BTreeMap<(u32, u32), GpCursor>>",
            "`CeShellState::cursors`' shape — rustfmt breaks a long field AFTER the colon, so \
             the line carrying the name has no type and the line carrying the type has no \
             field colon. `[measured 2026-08-09]` this hid TWO locks.",
        ),
        (
            "struct S {\n    /// ★★★★★ ⊘ ⚠ a doc comment of multi-byte characters\n    after_unicode: Mutex<u8>,\n}",
            "Mutex<u8>",
            "the byte-vs-char bug: every ★/⊘/⚠ is 3 bytes and 1 char, and indexing a \
             `Vec<char>` with a byte offset made `plane.rs`' three locks vanish from the scan.",
        ),
        (
            "struct S { gr_cursors: std::sync::Arc<std::sync::Mutex<Vec<GrCursorWatch>>>, }",
            "Mutex<Vec<GrCursorWatch>>",
            "★★★★★ **THE THIRD BLIND SPELLING, and it is the reason this test exists.** \
             `[w300, 2026-08-13]` A lock inside an `Arc` is preceded by `<`, not by the \
             field's colon, so the field test rejected it and NINE unranked vCPU-path locks \
             were invisible — including `shim.rs`' `gr_cursors`, which is held by TWO \
             threads (the vCPU pushes, the observer's reactor reads) and whose own doc \
             claimed a classification in this file that did not exist.",
        ),
    ];

    for (src, want, why) in KNOWN_POSITIVES {
        let mut got = BTreeSet::new();
        collect_locks(src, "fixture.rs", &mut got);
        let found: BTreeSet<&str> = got.iter().map(|(_, t)| t.as_str()).collect();
        assert!(
            found.contains(want),
            "★★★★★ THE SCANNER IS BLIND TO A SPELLING IT HAS TO SEE. Expected `{want}`, got \
             {found:?}.\n  why this fixture is here: {why}\n  ⊘ Every `unclassified.is_empty()` \
             this scanner reports is only as wide as the spellings it can parse — a shape it \
             cannot see is a lock nobody was ever asked to classify, and the gate stays GREEN \
             while it happens.\n  fixture:\n{src}"
        );
    }
}

/// ★★★★ **THE NEGATIVE KNOWN-POSITIVE — a RANKED lock must NOT appear on this list.**
///
/// ⊘ Without this arm the scanner could be "fixed" by matching anything containing `Mutex`,
/// which would pass every fixture above and then demand a classification for the rank system
/// itself. `RankedMutex<PlaneState>` is the live case and the brief's second known-positive:
/// `kayfabe_device::RegPlane::state` **was** this table's worst entry and left it by being
/// ranked (`LockRank::Plane`, w236 §16.87), not by being reworded. It must be absent for
/// that reason — and a gate that cannot tell "absent because ranked" from "absent because
/// unparsed" is the exact failure the positives above guard.
#[test]
fn a_ranked_lock_is_not_reported_as_unranked() {
    for src in [
        "struct RegPlane { state: RankedMutex<PlaneState>, }",
        "struct S { spine: kayfabe_rt::lock::RankedRwLock<Spine>, }",
        "struct S { held: std::sync::Arc<RankedMutex<PlaneState>>, }",
    ] {
        let mut got = BTreeSet::new();
        collect_locks(src, "fixture.rs", &mut got);
        assert!(
            got.is_empty(),
            "⊘ a RANKED lock was reported as unranked — `check_acquire` already enforces its \
             order, so demanding a row for it pads the list with entries that are not review \
             obligations, and a padded list is a list nobody reads. Got {got:?} from:\n{src}"
        );
    }
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
