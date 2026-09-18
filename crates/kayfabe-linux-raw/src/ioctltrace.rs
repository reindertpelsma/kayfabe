//! ★★★★★ **THE IOCTL TRACE — every RM ioctl this process issued, dumped when it matters.**
//!
//! > Owner, 2026-09-18: *"Run in verbose mode so it prints the ioctls. At timeout trace dump
//! > the whole thing."*
//!
//! # Why it is HERE and nowhere else
//!
//! [`crate::CharDev::ioctl`] is the single point every RM ioctl passes through. Recording there
//! makes the trace **complete by construction** — a new call site cannot forget to be traced,
//! because there is no other way to issue one. ⊘ The alternative, `eprintln!` at interesting
//! call sites, is how a trace comes to be complete for the paths someone was debugging and
//! silent for the one that broke.
//!
//! # Off by default, and expensive when on — deliberately
//!
//! > Owner, 2026-09-18: *"Debug that impacts perf is fine, because sometimes you need expensive
//! > inspection, as long as its off by default."*
//!
//! `KF_IOCTL_TRACE` selects: unset/`off` — nothing, not even the timestamp; `ring` — a bounded
//! in-memory ring, printed only on [`dump`]; `verbose` — the ring **and** a line per ioctl as
//! it happens. `ring` is what a timing run wants (a few ns per call); `verbose` is what a hang
//! wants, and it is slow enough to change what it measures. That is the trade, said out loud.
//!
//! # ⚠ The ring is BOUNDED and says when it dropped
//!
//! A trace that silently forgets its beginning is worse than none: the interesting ioctl in a
//! hang is usually the FIRST unusual one, not the last. [`dump`] prints `dropped=N` so a reader
//! can tell "this is everything" from "this is the tail".

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};

/// One issued ioctl, as much of it as is cheap to keep.
#[derive(Debug, Clone, Copy)]
pub struct Entry {
    /// Microseconds since the first traced ioctl — a relative clock, so a dump is readable
    /// without knowing when the process started.
    pub at_us: u64,
    /// The request number. ⊘ Kept raw: its `_IOC_SIZE` field is load-bearing and a decoded
    /// name would hide a request whose size field is the surprise.
    pub request: u64,
    /// Bytes the caller handed over.
    pub arg_len: u32,
    /// `ioctl(2)`'s own return. ⚠ NOT the RM status: RM puts that **in the parameter struct**
    /// while returning 0, which this tree has paid for before (`failed_zero_is_not_nothing_refused`).
    pub rc: i32,
    /// The first four bytes of the argument AFTER the call — for `NVOS*` shapes this is where
    /// a handle or a status usually lands. ⊘ Four bytes, not the struct: a trace that copies
    /// every argument is a trace nobody can afford to leave on.
    pub head_after: u32,
}

const OFF: u8 = 0;
const RING: u8 = 1;
const VERBOSE: u8 = 2;

static MODE: AtomicU8 = AtomicU8::new(u8::MAX);
static DROPPED: AtomicU64 = AtomicU64::new(0);
static T0: AtomicU64 = AtomicU64::new(0);
static RING_BUF: Mutex<VecDeque<Entry>> = Mutex::new(VecDeque::new());

/// How many entries the ring holds before it starts forgetting. ⊘ 4096 covers a full raw-client
/// bring-up with room to spare; a hang's interesting prefix is what must survive.
const CAP: usize = 4096;

fn mode() -> u8 {
    let m = MODE.load(Ordering::Relaxed);
    if m != u8::MAX {
        return m;
    }
    let m = match std::env::var("KF_IOCTL_TRACE").as_deref() {
        Ok("verbose") => VERBOSE,
        Ok("ring" | "on" | "1") => RING,
        _ => OFF,
    };
    MODE.store(m, Ordering::Relaxed);
    m
}

/// Is the trace armed at all? ⊘ Checked before any clock read: `off` must cost one relaxed load.
#[must_use]
pub fn armed() -> bool {
    mode() != OFF
}

fn now_us() -> u64 {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_micros() as u64);
    let base = T0.load(Ordering::Relaxed);
    if base == 0 {
        // ⊘ A race here costs one entry a wrong relative stamp and nothing else.
        T0.store(t, Ordering::Relaxed);
        return 0;
    }
    t.saturating_sub(base)
}

/// Record one issued ioctl. Called from [`crate::CharDev::ioctl`] and nowhere else.
pub fn record(request: u64, arg: &[u8], rc: i32) {
    let m = mode();
    if m == OFF {
        return;
    }
    let head_after = u32::from_le_bytes([
        arg.first().copied().unwrap_or(0),
        arg.get(1).copied().unwrap_or(0),
        arg.get(2).copied().unwrap_or(0),
        arg.get(3).copied().unwrap_or(0),
    ]);
    let e = Entry {
        at_us: now_us(),
        request,
        arg_len: u32::try_from(arg.len()).unwrap_or(u32::MAX),
        rc,
        head_after,
    };
    if m == VERBOSE {
        eprintln!(
            "IOCTL {:>9}us req={:#010x} len={:<5} rc={:<3} head={:#010x}",
            e.at_us, e.request, e.arg_len, e.rc, e.head_after
        );
    }
    let mut g = RING_BUF.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if g.len() == CAP {
        g.pop_front();
        DROPPED.fetch_add(1, Ordering::Relaxed);
    }
    g.push_back(e);
}

/// Print the whole ring. ⊘ Safe to call from a signal handler's thread: it takes a lock and
/// writes to stderr, and a poisoned lock is recovered rather than panicked on — a dump that
/// panics is a dump you do not get.
pub fn dump(why: &str) {
    if !armed() {
        eprintln!("IOCTL-TRACE ⊘ NOT ARMED — set KF_IOCTL_TRACE=ring or =verbose. ({why})");
        return;
    }
    let g = RING_BUF.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dropped = DROPPED.load(Ordering::Relaxed);
    eprintln!(
        "IOCTL-TRACE ★★★ {why}: {} entries, dropped={dropped} (cap {CAP})",
        g.len()
    );
    if dropped > 0 {
        eprintln!(
            "IOCTL-TRACE ⚠ THE BEGINNING IS GONE — {dropped} entries were forgotten, and in a \
             hang the interesting ioctl is usually the FIRST unusual one, not the last."
        );
    }
    for e in g.iter() {
        eprintln!(
            "IOCTL {:>9}us req={:#010x} len={:<5} rc={:<3} head={:#010x}",
            e.at_us, e.request, e.arg_len, e.rc, e.head_after
        );
    }
    eprintln!("IOCTL-TRACE ★★★ end ({why})");
}

/// ★★★★★ **DUMP THE TRACE BEFORE THE HARNESS KILLS US.**
///
/// > Owner, 2026-09-18: *"At timeout trace dump the whole thing."*
///
/// Arms a watchdog thread that dumps at `KF_SELF_DEADLINE_MS` and then aborts the process.
/// Returns whether it armed.
///
/// # ⊘ Why a THREAD and not a SIGTERM handler
///
/// Two reasons, and the second is the load-bearing one:
///
/// 1. A handler would need `unsafe` (`libc::signal`), and the crates that would call this
///    `forbid(unsafe_code)`. Putting the `unsafe` here and the call there would work, but:
/// 2. **`eprintln!` and locking a `Mutex` are not async-signal-safe.** If the signal lands while
///    the ring's lock is held, the handler deadlocks and you get NO dump — precisely in the case
///    the dump exists for. A separate thread takes the lock normally and cannot deadlock against
///    a holder that is still running.
///
/// ★ And it dumps even when the main thread is stuck in an **uninterruptible** ioctl, which is
/// the shape this tree's hangs actually take: `[measured w760]` an arm spun 400 s in `STAT=R`
/// and survived `SIGKILL`, because a task in a kernel busy-loop never reaches a
/// signal-delivery point. A signal-based dump would have produced nothing there.
///
/// ⚠ Set the deadline BELOW the harness's own budget, or the harness kills the process first
/// and the trace dies with it. `run_fast_guest.sh` budgets the whole boot; this should fire a
/// couple of seconds earlier.
pub fn arm_self_deadline() -> bool {
    let Ok(ms) = std::env::var("KF_SELF_DEADLINE_MS") else {
        return false;
    };
    let Ok(ms) = ms.parse::<u64>() else {
        eprintln!("IOCTL-TRACE ⊘ KF_SELF_DEADLINE_MS does not parse as milliseconds: {ms:?}");
        return false;
    };
    std::thread::Builder::new()
        .name("kf-deadline".into())
        .spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(ms));
            eprintln!(
                "IOCTL-TRACE ⊘⊘⊘ SELF-DEADLINE {ms}ms EXPIRED — the process did not finish. \
                 What follows is every ioctl it issued; the LAST one is where it stopped."
            );
            dump("self-deadline");
            // ⊘ `abort`, not `exit`: an orderly exit runs destructors that may themselves block
            // on whatever hung us, and then the dump we just printed is followed by silence.
            std::process::abort();
        })
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ⊘ The default must be OFF: an unset variable is the shipping configuration, and a trace
    /// that is on by default is a perf change nobody asked for.
    #[test]
    fn unset_means_off_and_costs_nothing() {
        // SAFETY of the assertion, not of code: `mode()` caches, so this only holds in a
        // process where nothing armed it. The env is not set under `cargo test`.
        if std::env::var("KF_IOCTL_TRACE").is_err() {
            assert!(!armed(), "the ioctl trace must be OFF unless asked for");
        }
    }
}
