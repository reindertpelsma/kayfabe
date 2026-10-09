//! ★ **The one logging entry point of the VMM crates** — [`klog!`]
//! (`docs/design/V3_NONSTALL_THREADS.md` §3.B).
//!
//! Why not `eprintln!`: it PANICS when the write fails (`ENOSPC`, `EPIPE`), which kills the calling
//! thread — for the register drainer, silently — and it cannot say how long it took. QEMU itself logs
//! synchronously (`qemu_log` = `flockfile` + `fprintf`, a failed write ignored); this does the same,
//! plus a measurement:
//!
//! - the line is formatted and written as ONE `write(2)` to stderr, and a failure is ignored — never
//!   a panic (`let _ = …`);
//! - the duration of every call is recorded in always-on atomics ([`stats`]): the longest call, the
//!   thread class that made it ([`ThreadClass`], set once per thread with [`set_class`]), a count of
//!   calls at or over 1 ms per class. The device prints them in its status line.
//!
//! ## Production is quiet (owner rule 2026-10-09)
//!
//! In steady state no input-serving thread (drainer, act, workers, VA, display) logs anything per
//! RPC, per statement, per doorbell or per act. Three macros say which kind a line is:
//!
//! | macro | prints | use for |
//! |---|---|---|
//! | [`klog!`] | always | boot-phase lines (realize, first-N counters), the rare line that is itself the event (a phase change, a device going DOWN), and lines already behind a default-off diagnostic flag |
//! | [`klog_limited!`] | the first 4 calls of that call site, then each power of two | an ERROR or refusal that can repeat (a guest-triggerable one must not flood the log) |
//! | [`klog_trace!`] | only with `KF3_LOG_VERBOSE=1` | anything that happens per RPC / statement / act / object (default off; the device then prints `PERTURBING_DIAGNOSTIC_ON(KF3_LOG_VERBOSE)`) |
//!
//! ⊘ There is deliberately no logger thread, queue, ring or drop policy. If a real run shows a log
//! call of about a millisecond or more on a serving thread (`stats().worst`), the escalation is a
//! bounded logger thread (§3.B of the design document) — until then it is measured, not guessed.
//!
//! A source scan (`crates/kf-qemu/tests/no_raw_prints.rs`) fails on a raw `eprintln!`/`println!` in the
//! crates that run inside the VMM.

use std::cell::Cell;
use std::fmt;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// What the calling thread does, for attributing a slow log call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadClass {
    /// Anything not named below (QEMU's main loop, realize, probes …).
    Other = 0,
    /// The register drainer.
    Drainer = 1,
    /// The channel plane's act thread.
    Act = 2,
    /// A worker.
    Worker = 3,
    /// The VA manager thread.
    Va = 4,
    /// The display worker.
    Display = 5,
}

impl ThreadClass {
    /// Every class.
    pub const ALL: [ThreadClass; 6] = [
        ThreadClass::Other,
        ThreadClass::Drainer,
        ThreadClass::Act,
        ThreadClass::Worker,
        ThreadClass::Va,
        ThreadClass::Display,
    ];

    /// The name the status line prints.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            ThreadClass::Other => "other",
            ThreadClass::Drainer => "drainer",
            ThreadClass::Act => "act",
            ThreadClass::Worker => "worker",
            ThreadClass::Va => "va",
            ThreadClass::Display => "display",
        }
    }
}

thread_local! {
    static CLASS: Cell<ThreadClass> = const { Cell::new(ThreadClass::Other) };
}

/// Name the calling thread's class (once, at the top of its loop).
pub fn set_class(c: ThreadClass) {
    CLASS.with(|x| x.set(c));
}

const N: usize = ThreadClass::ALL.len();
static CALLS: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];
static MAX_NS: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];
static OVER_1MS: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];
static FAILED: AtomicU64 = AtomicU64::new(0);

/// A call at or over this many nanoseconds (1 ms) is counted in [`ClassStats::over_1ms`].
pub const SLOW_NS: u64 = 1_000_000;

/// One thread class's log-call measurements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassStats {
    /// The class.
    pub class: ThreadClass,
    /// Calls made.
    pub calls: u64,
    /// The longest call, ns.
    pub max_ns: u64,
    /// Calls at or over [`SLOW_NS`].
    pub over_1ms: u64,
}

/// The measurements: per class, plus the number of writes that failed (and were ignored).
#[must_use]
pub fn stats() -> ([ClassStats; N], u64) {
    let o = Ordering::Relaxed;
    (
        std::array::from_fn(|i| ClassStats {
            class: ThreadClass::ALL[i],
            calls: CALLS[i].load(o),
            max_ns: MAX_NS[i].load(o),
            over_1ms: OVER_1MS[i].load(o),
        }),
        FAILED.load(o),
    )
}

/// The longest call of any class: `(class, ns)`; `(Other, 0)` before any call.
#[must_use]
pub fn worst() -> (ThreadClass, u64) {
    let (s, _) = stats();
    s.iter()
        .max_by_key(|c| c.max_ns)
        .map_or((ThreadClass::Other, 0), |c| (c.class, c.max_ns))
}

/// One compact status-line fragment (leading space): the worst call and its class, the count of slow
/// calls on the drainer and act thread, and failed writes.
#[must_use]
pub fn fragment() -> String {
    let (s, failed) = stats();
    let (class, ns) = worst();
    format!(
        " log[max_call_us={} max_call_class={} drainer_calls={} drainer_max_us={} drainer_over1ms={} act_max_us={} act_over1ms={} worker_max_us={} failed_writes={}]",
        ns / 1000,
        class.name(),
        s[ThreadClass::Drainer as usize].calls,
        s[ThreadClass::Drainer as usize].max_ns / 1000,
        s[ThreadClass::Drainer as usize].over_1ms,
        s[ThreadClass::Act as usize].max_ns / 1000,
        s[ThreadClass::Act as usize].over_1ms,
        s[ThreadClass::Worker as usize].max_ns / 1000,
        failed,
    )
}

/// Write `args` and a newline to `out` as ONE `write_all`, ignoring a failure; `true` if it wrote.
pub fn write_line(out: &mut dyn Write, args: fmt::Arguments<'_>) -> bool {
    let mut line = format!("{args}");
    line.push('\n');
    out.write_all(line.as_bytes()).is_ok()
}

/// Log one line to `out` and record the call. The macro's engine; `out` is stderr in production and a
/// failing writer in the no-panic test.
pub fn emit_to(out: &mut dyn Write, args: fmt::Arguments<'_>) {
    let t0 = Instant::now();
    if !write_line(out, args) {
        FAILED.fetch_add(1, Ordering::Relaxed);
    }
    let ns = u64::try_from(t0.elapsed().as_nanos()).unwrap_or(u64::MAX);
    let i = CLASS.with(Cell::get) as usize;
    CALLS[i].fetch_add(1, Ordering::Relaxed);
    MAX_NS[i].fetch_max(ns, Ordering::Relaxed);
    if ns >= SLOW_NS {
        OVER_1MS[i].fetch_add(1, Ordering::Relaxed);
    }
}

/// Log one line to stderr: never a panic, always measured. Use [`klog!`].
pub fn emit(args: fmt::Arguments<'_>) {
    emit_to(&mut std::io::stderr(), args);
}

/// Whether `KF3_LOG_VERBOSE=1` (or `on`) was set: the per-RPC / per-statement / per-act lines print.
/// Read once.
#[must_use]
pub fn verbose() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("KF3_LOG_VERBOSE").is_ok_and(|v| v == "1" || v == "on"))
}

/// Whether the `n`th (1-based) call of a [`klog_limited!`] site prints: the first 4, then each power
/// of two.
#[must_use]
pub fn limited(n: u64) -> bool {
    n <= 4 || n.is_power_of_two()
}

/// A per-RPC / per-statement / per-act line: printed only with `KF3_LOG_VERBOSE=1`.
#[macro_export]
macro_rules! klog_trace {
    ($($t:tt)*) => {
        if $crate::log::verbose() {
            $crate::log::emit(format_args!($($t)*))
        }
    };
}

/// A repeating error line: the first 4 calls of this call site print, then each power of two.
#[macro_export]
macro_rules! klog_limited {
    ($($t:tt)*) => {{
        static N: ::std::sync::atomic::AtomicU64 = ::std::sync::atomic::AtomicU64::new(0);
        let n = N.fetch_add(1, ::std::sync::atomic::Ordering::Relaxed) + 1;
        if $crate::log::limited(n) {
            $crate::log::emit(format_args!($($t)*))
        }
    }};
}

/// `eprintln!`'s replacement: never panics on a failed write, records its own duration. See the
/// module documentation.
#[macro_export]
macro_rules! klog {
    ($($t:tt)*) => {
        $crate::log::emit(format_args!($($t)*))
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stderr whose every write fails (a full disk, a closed pipe).
    struct Broken;
    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::from_raw_os_error(28)) // ENOSPC
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// A stderr that is slow, for the duration witness.
    struct Slow(std::time::Duration);
    impl Write for Slow {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            std::thread::sleep(self.0);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// ★ `eprintln!` panics when stderr is full; this must not (it would kill the drainer silently).
    #[test]
    fn a_failing_sink_is_ignored_and_counted_never_a_panic() {
        let before = stats().1;
        for i in 0..3 {
            emit_to(&mut Broken, format_args!("kf3: line {i}"));
        }
        assert!(stats().1 >= before + 3, "the failed writes are counted");
    }

    /// ★ The witness: a slow write shows up as the max call, under this thread's class.
    #[test]
    fn a_slow_write_is_the_recorded_max_for_its_thread_class() {
        let h = std::thread::spawn(|| {
            set_class(ThreadClass::Display);
            emit_to(
                &mut Slow(std::time::Duration::from_millis(3)),
                format_args!("slow"),
            );
        });
        h.join().unwrap();
        let (s, _) = stats();
        let d = s[ThreadClass::Display as usize];
        assert!(d.calls >= 1);
        assert!(d.max_ns >= 2_000_000, "{d:?}");
        assert!(d.over_1ms >= 1);
        assert!(fragment().contains("max_call_us="));
    }

    #[test]
    fn limited_prints_the_first_four_then_powers_of_two() {
        let printed: Vec<u64> = (1..=40).filter(|n| limited(*n)).collect();
        assert_eq!(printed, vec![1, 2, 3, 4, 8, 16, 32]);
        // The macro counts per call site: 7 of 40 calls print.
        let before = stats().0[ThreadClass::Display as usize].calls;
        let h = std::thread::spawn(|| {
            set_class(ThreadClass::Display);
            for _ in 0..40 {
                crate::klog_limited!("kf-util limited test");
            }
            stats().0[ThreadClass::Display as usize].calls
        });
        let after = h.join().unwrap();
        assert!(after >= before + 7);
    }

    #[test]
    fn trace_is_silent_without_the_verbose_flag() {
        if !verbose() {
            let h = std::thread::spawn(|| {
                set_class(ThreadClass::Va);
                let before = stats().0[ThreadClass::Va as usize].calls;
                crate::klog_trace!("kf-util trace test (must not print)");
                (before, stats().0[ThreadClass::Va as usize].calls)
            });
            let (b, a) = h.join().unwrap();
            assert_eq!(a, b);
        }
    }

    #[test]
    fn the_macro_writes_one_line_without_a_sink() {
        crate::klog!("kf-util log test {x}", x = 4);
        let mut v = Vec::new();
        emit_to(&mut v, format_args!("a {} b", 1));
        assert_eq!(v, b"a 1 b\n");
    }
}
