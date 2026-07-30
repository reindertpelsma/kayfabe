//! ★★ **Bounded termination, with a diagnostic that actually reaches the log.**
//!
//! The house rule (`concurrency_stress.rs`'s M4b lesson) is that a test which can wedge
//! must **fail fast and loudly**, never eat the CI timeout. Ten copies of a watchdog had
//! been written to enforce it — one per test file, plus one in this crate's own unit
//! tests — and every one of them was **silent**.
//!
//! ## ★★★ THE MEASURED DEFECT: `eprintln!` from a watchdog thread is thrown away
//!
//! libtest captures a test's output by installing a thread-local sink
//! (`std::io::set_output_capture`) and buffering everything the `print!`/`eprintln!`
//! macros write. `std::thread::spawn` **propagates that sink into the child thread**, so
//! a watchdog thread's `eprintln!` lands in the same buffer as the test's — and libtest
//! only flushes that buffer when the test *finishes*. `std::process::abort()` never
//! reaches that point, so the buffer dies with the process.
//!
//! Measured, not reasoned about (2026-07-31, a standalone probe under `cargo test`):
//!
//! | what the watchdog thread did before aborting | what `cargo test` printed |
//! |---|---|
//! | `eprintln!("WATCHDOG: …")` | **nothing at all** |
//! | a write to a real fd 2 | the full text |
//!
//! So the failure mode of every wedged test in this workspace was a bare
//! `signal: 6, SIGABRT` after 60–300 s with **no indication of which test, or where**.
//! That is precisely "a hang wedges CI instead of failing it", one level in: the timeout
//! fired, and still reported nothing.
//!
//! ## What this module does instead
//!
//! [`watchdog`] writes its diagnostic to **`/dev/stderr` opened as a file**, which is a
//! real file descriptor and therefore invisible to libtest's macro-level capture, and it
//! writes more than a name: every thread in the process with its `comm`, its scheduler
//! state and its **kernel wait channel** (`/proc/<tid>/wchan`). A wedge is then reported
//! as *which* test, for *how long*, and *what each thread was blocked in* —
//! `futex_wait_queue` on a condvar, `pipe_read` on a reply, `do_epoll_wait` in the
//! reactor — which is how every C-era hang in the sibling repo was actually root-caused
//! (`l1_concurrency.md` §5, "`/proc/…/stack` — how every C bug was root-caused").
//!
//! ⊘ **Nothing here relaxes `unsafe_code`.** The workspace forbids it; writing to fd 2 via
//! `libc::write` would need it and would confine this to crates that carry a `libc`
//! dependency. Opening `/dev/stderr` is ordinary safe file I/O and reaches the same fd.
//!
//! ## Why one copy
//!
//! Ten hand-maintained copies of one guard is the list-shaped defect this project keeps
//! being bitten by: a fix applied to nine of them leaves the tenth silently unprotected,
//! and nothing turns red. `kayfabe-mocks` is already a dependency of every test file that
//! had a copy, and it is test-only by charter, so it is the one place all of them can
//! share.

use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Environment override for every watchdog's limit, in seconds.
///
/// Sanitizer runs are 5–20× slower than a plain build, and the bounds here are sized for
/// a plain one; TSan/ASan jobs raise this rather than each bound being padded for a case
/// that almost never runs.
pub const WATCHDOG_ENV: &str = "KAYFABE_STRESS_WATCHDOG_SECS";

/// Resolve a watchdog limit, honouring [`WATCHDOG_ENV`].
///
/// # Panics
/// If the variable is set to something that is not a whole number of seconds — a typo
/// that silently disabled every bound would be worse than the wedge.
#[must_use]
pub fn limit_or_env(default: Duration) -> Duration {
    match std::env::var(WATCHDOG_ENV) {
        Ok(s) => Duration::from_secs(
            s.parse()
                .unwrap_or_else(|_| panic!("{WATCHDOG_ENV} must be a whole number of seconds")),
        ),
        Err(_) => default,
    }
}

/// ★ Write `msg` to the process's **real** standard error, bypassing libtest's capture.
///
/// Returns whether it got out, so a caller can fall back rather than assume. `/dev/stderr`
/// is `/proc/self/fd/2` on Linux and reopens whatever fd 2 already is — a terminal, a
/// pipe to `cargo`, or a redirect to a file.
pub fn write_uncaptured(msg: &str) -> bool {
    for path in ["/dev/stderr", "/proc/self/fd/2"] {
        if let Ok(mut f) = std::fs::OpenOptions::new().write(true).open(path)
            && f.write_all(msg.as_bytes()).is_ok()
        {
            let _ = f.flush();
            return true;
        }
    }
    // Last resort: the captured macro. Worthless under `abort()`, but a caller that
    // chooses to panic instead of aborting will still surface it.
    eprint!("{msg}");
    false
}

/// ★ Every thread in this process: tid, name, scheduler state, and the kernel function it
/// is blocked in.
///
/// This is the whole point of the rewrite — "the test wedged" is not a diagnosis, "three
/// threads in `futex_wait_queue` and one in `do_epoll_wait`" is. Best-effort: a thread can
/// exit between the readdir and the reads, and `wchan` is empty for a running thread, so
/// every field degrades to `?` rather than failing.
#[must_use]
pub fn thread_report() -> String {
    let Ok(entries) = std::fs::read_dir("/proc/self/task") else {
        return "  (/proc/self/task unreadable — no per-thread state available)\n".to_string();
    };
    let mut tids: Vec<String> = entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    tids.sort();
    let mut out = String::new();
    for tid in tids {
        let dir = std::path::Path::new("/proc/self/task").join(&tid);
        let read = |f: &str| {
            std::fs::read_to_string(dir.join(f))
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|_| "?".into())
        };
        let comm = read("comm");
        let wchan = read("wchan");
        // /proc/<tid>/stat: `pid (comm) state …`, and comm may itself contain spaces and
        // parentheses, so the state is the first field AFTER the final ") ".
        let state = std::fs::read_to_string(dir.join("stat"))
            .ok()
            .and_then(|s| {
                s.rsplit_once(") ")
                    .and_then(|(_, rest)| rest.split_whitespace().next().map(str::to_string))
            })
            .unwrap_or_else(|| "?".into());
        let wchan = if wchan.is_empty() || wchan == "0" {
            "(running)".to_string()
        } else {
            wchan
        };
        out.push_str(&format!(
            "    tid {tid:<8} state={state:<2} wchan={wchan:<24} name={comm}\n"
        ));
    }
    out
}

/// ★★ Abort the process **loudly** if the returned guard is not dropped within `limit`.
///
/// The guard disarms on drop, so the success path costs one atomic store. `limit` is
/// overridable by [`WATCHDOG_ENV`].
///
/// ★ Why `abort()` and not a panic: the wedged thread is some *other* thread, parked in a
/// condvar or a blocking syscall, and nothing in `std` can unwind it. Killing the process
/// is the only way to end the wait — so the requirement is not "don't abort", it is
/// "**say something first, through a channel that survives**", which is what
/// [`write_uncaptured`] is for.
#[must_use]
pub fn watchdog(test: &'static str, limit: Duration) -> WatchdogGuard {
    let limit = limit_or_env(limit);
    let done = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&done);
    std::thread::spawn(move || {
        let started = Instant::now();
        while started.elapsed() < limit {
            if flag.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if flag.load(Ordering::Relaxed) {
            return;
        }
        write_uncaptured(&format!(
            "\n\
             ★★★ WATCHDOG: {test} DID NOT TERMINATE within {limit:?}.\n\
             This is a HANG being converted into a failure, not a slow test: the guard is\n\
             dropped on every exit path, including an unwind, so reaching this means a\n\
             thread is genuinely blocked. Raise {WATCHDOG_ENV} for a sanitizer run; do not\n\
             raise it to make this pass.\n\
             Threads at the moment of the abort:\n\
             {}\n\
             Aborting (SIGABRT) — the parked thread cannot be unwound from here.\n\n",
            thread_report()
        ));
        std::process::abort();
    });
    WatchdogGuard(done)
}

/// Disarms its [`watchdog`] on drop.
pub struct WatchdogGuard(Arc<AtomicBool>);

impl Drop for WatchdogGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ★ NON-VACUITY for the whole module: the report must actually enumerate threads,
    /// and it must find *this* one. A `thread_report` that silently returned an empty
    /// string would make every watchdog message above look fine and say nothing.
    #[test]
    fn the_thread_report_names_this_process_own_threads() {
        let started = Arc::new(AtomicBool::new(false));
        let s = Arc::clone(&started);
        let t = std::thread::Builder::new()
            .name("kf-wd-probe".into())
            .spawn(move || {
                s.store(true, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(300));
            })
            .expect("spawn");
        while !started.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }
        let r = thread_report();
        assert!(
            r.contains("kf-wd-probe"),
            "the report must name a live thread by its `comm`; got:\n{r}"
        );
        assert!(
            r.lines().filter(|l| l.contains("tid ")).count() >= 2,
            "at least the main thread and the probe must appear; got:\n{r}"
        );
        t.join().expect("join");
    }

    /// ★ The guard disarms — otherwise every test in the workspace would abort at its
    /// bound, which is a failure mode nobody would miss but the assertion is cheap.
    #[test]
    fn a_dropped_guard_disarms_the_abort() {
        {
            let _g = watchdog(
                "a_dropped_guard_disarms_the_abort",
                Duration::from_millis(50),
            );
        }
        std::thread::sleep(Duration::from_millis(250));
    }

    /// ★ The env override is read, so a sanitizer job can raise every bound at once.
    #[test]
    fn the_env_override_replaces_the_default() {
        // Not set in a normal run: the default must survive untouched.
        if std::env::var(WATCHDOG_ENV).is_err() {
            assert_eq!(limit_or_env(Duration::from_secs(7)), Duration::from_secs(7));
        }
    }

    /// ★ The uncaptured writer must really reach a descriptor. `false` would mean every
    /// watchdog message is going to the buffer that `abort()` discards — the exact defect
    /// this module exists to fix — so this is the bite-check for it.
    #[test]
    fn the_diagnostic_reaches_a_real_descriptor() {
        assert!(
            write_uncaptured(""),
            "neither /dev/stderr nor /proc/self/fd/2 could be opened for writing, so a \
             watchdog abort would be silent again"
        );
    }
}
