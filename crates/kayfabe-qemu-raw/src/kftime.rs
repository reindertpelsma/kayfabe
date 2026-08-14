//! ★★★★★ **w315 — THE SEGMENT TIMER. It exists to ATTRIBUTE a latency, not to measure one.**
//!
//! # What it is for
//!
//! `[measured 2026-08-14, w311, `docs/design/w311_throughput_ratio_and_the_llm_question.md`]`
//! a guest kernel launch costs **~100 ms of FIXED per-launch time**, it survives batching
//! (so it is per-SUBMIT, not per-SYNC), and **its mechanism is unattributed**. w311's own
//! recommendation is the whole of this module's brief: *"the next rung should timestamp the
//! doorbell path directly rather than inferring it from a fit."*
//!
//! ⇒ The deliverable is a **breakdown**, never a total. *"100 ms, of which N is X"* is the
//! answer; *"100 ms"* is what we already had.
//!
//! # ⊘⊘ THE FAILURE THIS MODULE IS SHAPED AROUND — it already happened once, on this metric
//!
//! w311 nearly shipped the wrong mechanism. The device's `SEMA-WRITE` lines arrive on a hard
//! **251 ms** cadence, and `251/2 = 125.5 ms` matched the fitted fixed cost `C ≈ 115–132 ms`
//! to within 0.4 %. It **arrived pre-corroborated**. It was refuted only by the guest's own
//! latency distribution — a continuous 102.9–138.1 ms band, neither multiples of 251 nor
//! spread over `[0, 251]` — and the 251 ms turned out to be
//! [`crate::shim::OBSERVER_TICK_MS`], *the observer thread's own epoll timeout*: the
//! instrument's clock impersonating the measured quantity.
//!
//! ⇒ Three design consequences, each a rule this file follows:
//!
//! 1. **Every segment is a bracketed interval on ONE clock**, `Instant` on the thread that
//!    does the work. Nothing here is inferred from a cadence, a fit, or a coincidence of
//!    magnitudes.
//! 2. **The residual is NAMED, not distributed.** [`Segs::line`] prints the sum of the
//!    marked segments beside the bracket total, so a gap between them is visible as a gap.
//!    ⊘ An unattributed millisecond must never be silently folded into the nearest segment —
//!    that is precisely how a wrong mechanism acquires a number.
//! 3. **It can be made to lie on purpose.** [`Arm::inject_us`] adds a *known* delay to a
//!    *named* segment. An instrument that has never mis-attributed on purpose has not been
//!    shown to attribute at all, and `census` must show the injected microseconds landing in
//!    the segment they were injected into and nowhere else.
//!
//! # ⚠ The instrument is not free, and its cost is a reported number
//!
//! Timing under instrumentation is not timing without it. Every hook here is inside the
//! vCPU's MMIO trap **under the QEMU BQL**, so a `Mutex` acquisition and an `eprintln!` per
//! doorbell are charged to the guest. That is why:
//!
//! - the whole module is **OFF by default** ([`Arm::off`]) and armed only by
//!   `KAYFABE_KFTIME`, so an un-armed boot is byte-for-byte master's;
//! - the per-event line is separately gated (`KAYFABE_KFTIME=census` keeps the aggregate and
//!   drops the ~900 lines), so the aggregate can be taken at nearly zero print cost;
//! - the rung reports the **same workload with the instrument off** as its baseline, and the
//!   difference IS the instrument's cost.
//!
//! # ⊘ What a green breakdown does NOT say
//!
//! These are host-side intervals on the host's `CLOCK_MONOTONIC`. The guest measures with
//! its own `CLOCK_MONOTONIC`, and **no offset between the two is computed here or anywhere
//! else in this rung**. The one correspondence that needs no offset is *nesting*: an MMIO
//! write is a vmexit, so the guest is stopped for the whole of it, and every interval this
//! module reports is therefore **contained inside** the guest's own launch window. That
//! licenses `Σ segments ≤ launch_ms` as an arithmetic check and licenses nothing else.

use std::sync::Mutex;
use std::sync::OnceLock;
use std::time::Instant;

/// How many marks one event may carry.
///
/// ⊘ A fixed array rather than a `Vec`: this is allocated on the vCPU's MMIO trap under the
/// BQL, and a heap allocation per doorbell is a cost the measurement would then have to
/// explain. Overflow is **counted and named** ([`Segs::dropped`]) rather than silently
/// truncated — a breakdown missing a segment nobody mentioned is the exact shape of a wrong
/// attribution.
pub const MAX_MARKS: usize = 24;

/// Bucket edges for the per-event total, in microseconds.
///
/// ★ The point of a histogram rather than a mean: w311's floor is a *band* (102.9–138.1 ms),
/// and a mean cannot tell a floor from an occasional stall. The top bucket is open.
const HIST_EDGES_US: [u64; 10] = [
    10, 100, 1_000, 10_000, 30_000, 60_000, 100_000, 200_000, 500_000, 1_000_000,
];

/// The environment variable that arms everything in this module.
pub const KFTIME_ENV: &str = "KAYFABE_KFTIME";
/// Microseconds of *known* delay to inject, for the known-positive.
pub const KFTIME_INJECT_US_ENV: &str = "KAYFABE_KFTIME_INJECT_US";
/// Which segment name the injected delay is added to.
pub const KFTIME_INJECT_SEG_ENV: &str = "KAYFABE_KFTIME_INJECT_SEG";
/// How many events between automatic census prints.
pub const KFTIME_CENSUS_EVERY_ENV: &str = "KAYFABE_KFTIME_CENSUS_EVERY";

/// How this module is armed for the life of the process.
///
/// ⊘ Read **once**, from the environment, and never re-read: an instrument whose arming can
/// change under it cannot say what a run measured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Arm {
    /// Anything at all is recorded.
    pub on: bool,
    /// One line per event, in addition to the aggregate.
    pub per_event: bool,
    /// Microseconds of deliberate delay, for the known-positive. Zero = none.
    pub inject_us: u64,
    /// The segment the delay is charged to. Empty = injection disabled.
    pub inject_seg: &'static str,
    /// Print the running census every this many events. Zero = only at close.
    pub census_every: u64,
}

impl Arm {
    /// The disarmed default — and the *only* configuration a boot that did not ask for this
    /// module can be in.
    #[must_use]
    pub const fn off() -> Self {
        Self {
            on: false,
            per_event: false,
            inject_us: 0,
            inject_seg: "",
            census_every: 0,
        }
    }

    /// Parse an arming out of the three environment values, without touching the
    /// environment. ⊘ Split out from [`arm`] precisely so it is testable: an arming parser
    /// that can only be exercised by setting a process-global is a parser with no test.
    #[must_use]
    pub fn parse(mode: Option<&str>, inject_us: Option<&str>, seg: Option<&str>, every: Option<&str>) -> Self {
        let on;
        let per_event;
        match mode.unwrap_or("").trim() {
            "" | "off" | "0" | "no" => return Self::off(),
            "census" => {
                on = true;
                per_event = false;
            }
            // ★ Anything else that is not a refusal arms fully. The alternative — refusing an
            // unrecognised value — would silently disarm a boot whose operator meant to arm
            // it, and a disarmed boot looks exactly like an armed one that measured nothing.
            _ => {
                on = true;
                per_event = true;
            }
        }
        let inject_us = inject_us.and_then(|s| s.trim().parse::<u64>().ok()).unwrap_or(0);
        // ⊘ `&'static str` because the census keys on segment names by pointer-free equality
        // and the names in the code are literals. An injected name that matches no segment is
        // NOT an error here — it is reported by the census showing zero injected microseconds
        // anywhere, which is a louder failure than a refusal at startup.
        let inject_seg: &'static str = match seg.map(str::trim).unwrap_or("") {
            "" => "",
            s => Box::leak(s.to_owned().into_boxed_str()),
        };
        let census_every = every.and_then(|s| s.trim().parse::<u64>().ok()).unwrap_or(200);
        Self {
            on,
            per_event,
            inject_us,
            inject_seg,
            census_every,
        }
    }
}

static ARM: OnceLock<Arm> = OnceLock::new();

/// The process-wide time origin every `t_ms=` on a `KFTIME` line is measured from.
///
/// ★★★ It exists so the host's lines can be ALIGNED to the guest's `t0_mono_ms` by
/// CORRELATION — matching the two sequences' shapes — rather than by an offset nobody
/// measured. ⊘ It is NOT a shared clock and no arithmetic here converts between the two.
static ORIGIN: OnceLock<Instant> = OnceLock::new();

/// Milliseconds since [`ORIGIN`], which is set on first use.
fn t_ms() -> f64 {
    ORIGIN.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.0
}

/// The arming, read from the environment exactly once.
#[must_use]
pub fn arm() -> Arm {
    *ARM.get_or_init(|| {
        let mode = std::env::var(KFTIME_ENV).ok();
        let ius = std::env::var(KFTIME_INJECT_US_ENV).ok();
        let seg = std::env::var(KFTIME_INJECT_SEG_ENV).ok();
        let every = std::env::var(KFTIME_CENSUS_EVERY_ENV).ok();
        let a = Arm::parse(mode.as_deref(), ius.as_deref(), seg.as_deref(), every.as_deref());
        if a.on {
            eprintln!(
                "kayfabe: KFTIME ARMED per_event={} inject_us={} inject_seg={} census_every={} \
                 ⇒ every number this module prints is a HOST-side interval on the host's \
                 CLOCK_MONOTONIC, taken on the vCPU thread inside the MMIO trap. ⊘ No offset \
                 to the guest's clock is computed anywhere; the only correspondence claimed \
                 is NESTING (the guest is stopped for the whole trap).{}",
                a.per_event,
                a.inject_us,
                if a.inject_seg.is_empty() { "<none>" } else { a.inject_seg },
                a.census_every,
                if a.inject_us > 0 {
                    " ⚠⚠ A DELAY IS BEING INJECTED — this boot is the KNOWN-POSITIVE and its \
                     latencies are NOT a measurement of the plane."
                } else {
                    ""
                },
            );
        }
        a
    })
}

/// Sleep for the injected delay if `seg` is the segment under test.
///
/// ★★★ **The known-positive's whole mechanism.** It is a `sleep` and not a spin because what
/// it models is a *blocking* host call on the vCPU thread, which is the shape every candidate
/// mechanism in this path has (an IPC round trip, a semaphore poll, a per-row pin). A spin
/// would additionally load the CPU and change the thing being measured.
pub fn maybe_inject(seg: &str) {
    let a = arm();
    if a.inject_us == 0 || a.inject_seg.is_empty() || a.inject_seg != seg {
        return;
    }
    std::thread::sleep(std::time::Duration::from_micros(a.inject_us));
}

/// One event's segmentation.
pub struct Segs {
    t0: Instant,
    last: Instant,
    n: usize,
    names: [&'static str; MAX_MARKS],
    us: [u64; MAX_MARKS],
    dropped: u32,
    armed: bool,
    /// ★★★ **NESTED sub-totals — measured INSIDE a marked segment, never beside it.**
    ///
    /// `(name, us, count)`. These are reported and censused, and are deliberately **excluded
    /// from [`Segs::marked_us`]**: `core_rm_ipc` is time already charged to `core`, so adding
    /// it to the sum would double-count and would make the residual — the one number this
    /// module exists to keep honest — read as negative or as zero when it is neither.
    nested: Vec<(&'static str, u64, u64)>,
}

impl Segs {
    /// Open a bracket. Cheap and side-effect-free when disarmed.
    #[must_use]
    pub fn start() -> Self {
        let now = Instant::now();
        Self {
            t0: now,
            last: now,
            n: 0,
            names: [""; MAX_MARKS],
            us: [0; MAX_MARKS],
            dropped: 0,
            armed: arm().on,
            nested: Vec::new(),
        }
    }

    /// Is anything being recorded? Callers use this to skip work that only the instrument
    /// needs, so a disarmed boot pays nothing.
    #[must_use]
    pub const fn armed(&self) -> bool {
        self.armed
    }

    /// Close the interval since the previous mark and charge it to `name`.
    ///
    /// ⊘ Marks are **not** merged by name here. Two marks with one name appear twice in the
    /// line and are summed only by the census — so a reader of a single event's line can see
    /// that a segment ran twice, which a pre-summed number would hide.
    pub fn mark(&mut self, name: &'static str) {
        if !self.armed {
            return;
        }
        let now = Instant::now();
        let d = now.duration_since(self.last);
        self.last = now;
        if self.n >= MAX_MARKS {
            self.dropped += 1;
            return;
        }
        self.names[self.n] = name;
        self.us[self.n] = u64::try_from(d.as_micros()).unwrap_or(u64::MAX);
        self.n += 1;
    }

    /// Record a sub-total that lies INSIDE an already-marked segment.
    ///
    /// ⊘ Not a mark. See [`Segs::nested`] for why it is excluded from the sum.
    pub fn note_nested(&mut self, name: &'static str, us: u64, count: u64) {
        if !self.armed {
            return;
        }
        self.nested.push((name, us, count));
    }

    /// The bracket total — wall time from [`Segs::start`] to now.
    #[must_use]
    pub fn total_us(&self) -> u64 {
        u64::try_from(self.t0.elapsed().as_micros()).unwrap_or(u64::MAX)
    }

    /// The sum of the marked segments.
    #[must_use]
    pub fn marked_us(&self) -> u64 {
        self.us[..self.n].iter().copied().sum()
    }

    /// Marks that did not fit.
    #[must_use]
    pub const fn dropped(&self) -> u32 {
        self.dropped
    }

    /// The per-event line.
    ///
    /// ★★★ It carries `total`, `marked` and `UNMARKED` as three separate numbers. A
    /// breakdown that does not sum to its bracket is a FINDING — the missing time is real —
    /// and this line is what makes such a gap impossible to miss.
    #[must_use]
    pub fn line(&self, head: &str) -> String {
        let total = self.total_us();
        let marked = self.marked_us();
        let mut s = format!(
            "kayfabe: KFTIME {head} t_ms={:.3} total_us={total} marked_us={marked}",
            t_ms()
        );
        // ⊘ Saturating: the bracket is closed AFTER the last mark, so `total >= marked`
        // always holds in practice, but an arithmetic assumption in an instrument is exactly
        // the thing that has bitten this campaign, so it is expressed and not assumed.
        let unmarked = total.saturating_sub(marked);
        s.push_str(&format!(" unmarked_us={unmarked}"));
        if self.dropped > 0 {
            s.push_str(&format!(
                " ⚠ dropped_marks={} (MAX_MARKS={MAX_MARKS} — the breakdown below is INCOMPLETE)",
                self.dropped
            ));
        }
        s.push_str(" |");
        for i in 0..self.n {
            s.push_str(&format!(" {}={}", self.names[i], self.us[i]));
        }
        if !self.nested.is_empty() {
            s.push_str(" | NESTED(inside a segment above, NOT in marked_us):");
            for (n, us, c) in &self.nested {
                s.push_str(&format!(" {n}={us}us/n={c}"));
            }
        }
        s
    }
}

/// Per-segment totals across a whole run.
#[derive(Default)]
struct Census {
    events: u64,
    /// `(name, count, total_us, max_us)`, in first-seen order. ⊘ A `Vec` and not a map: the
    /// ORDER segments were first seen in is the order they run in, and a breakdown printed
    /// in alphabetical order is a breakdown nobody can read as a pipeline.
    segs: Vec<(&'static str, u64, u64, u64)>,
    total_us: u64,
    marked_us: u64,
    hist: [u64; HIST_EDGES_US.len() + 1],
    max_us: u64,
    /// `(name, calls, total_us)` for nested sub-totals. Kept apart from `segs` so a reader
    /// cannot sum the two columns together by accident.
    nested: Vec<(&'static str, u64, u64)>,
}

impl Census {
    fn record(&mut self, s: &Segs) {
        self.events += 1;
        let t = s.total_us();
        self.total_us += t;
        self.marked_us += s.marked_us();
        if t > self.max_us {
            self.max_us = t;
        }
        let mut b = HIST_EDGES_US.len();
        for (i, e) in HIST_EDGES_US.iter().enumerate() {
            if t < *e {
                b = i;
                break;
            }
        }
        self.hist[b] += 1;
        for i in 0..s.n {
            let (name, us) = (s.names[i], s.us[i]);
            if let Some(row) = self.segs.iter_mut().find(|r| r.0 == name) {
                row.1 += 1;
                row.2 += us;
                if us > row.3 {
                    row.3 = us;
                }
            } else {
                self.segs.push((name, 1, us, us));
            }
        }
        for (name, us, calls) in &s.nested {
            if let Some(row) = self.nested.iter_mut().find(|r| r.0 == *name) {
                row.1 += calls;
                row.2 += us;
            } else {
                self.nested.push((name, *calls, *us));
            }
        }
    }

    fn report(&self, kind: &str, why: &str) -> String {
        if self.events == 0 {
            // ⊘ An empty census SAYS SO. A silent instrument and an instrument that ran and
            // saw nothing are the two answers this whole rung exists to keep apart, and an
            // absent line reads as the benign one.
            return format!(
                "kayfabe: KFTIME-CENSUS kind={kind} why={why} events=0 ⊘ NOTHING WAS RECORDED \
                 — this is a statement about the instrument, NOT about the plane"
            );
        }
        let mean = self.total_us / self.events;
        let unmarked = self.total_us.saturating_sub(self.marked_us);
        let mut s = format!(
            "kayfabe: KFTIME-CENSUS kind={kind} why={why} t_ms={:.3} events={} total_ms={:.3} \
             mean_us={mean} max_us={} marked_ms={:.3} UNMARKED_ms={:.3} ({:.1}%)",
            t_ms(),
            self.events,
            self.total_us as f64 / 1000.0,
            self.max_us,
            self.marked_us as f64 / 1000.0,
            unmarked as f64 / 1000.0,
            if self.total_us == 0 {
                0.0
            } else {
                100.0 * unmarked as f64 / self.total_us as f64
            },
        );
        // ★ Ranked by TOTAL, because the question is "where did the run's time go", and
        // printed with the mean beside it, because a segment that is slow once and a segment
        // that is slow always are different fix targets.
        let mut rows: Vec<_> = self.segs.clone();
        rows.sort_by_key(|r| std::cmp::Reverse(r.2));
        for (name, n, tot, mx) in rows {
            s.push_str(&format!(
                "\n    KFTIME-SEG {name:<16} n={n:<7} total_ms={:<10.3} mean_us={:<9} max_us={mx:<9} share={:.1}%",
                tot as f64 / 1000.0,
                tot.checked_div(n).unwrap_or(0),
                if self.total_us == 0 {
                    0.0
                } else {
                    100.0 * tot as f64 / self.total_us as f64
                },
            ));
        }
        for (name, calls, tot) in &self.nested {
            s.push_str(&format!(
                "\n    KFTIME-NESTED {name:<16} calls={calls:<7} total_ms={:<10.3} mean_us={:<9}                  ⊘ INSIDE a segment above — do NOT add this row to the segment column",
                *tot as f64 / 1000.0,
                tot.checked_div(*calls).unwrap_or(0),
            ));
        }
        s.push_str("\n    KFTIME-HIST us:");
        let mut lo = 0u64;
        for (i, count) in self.hist.iter().enumerate() {
            let label = if i < HIST_EDGES_US.len() {
                let hi = HIST_EDGES_US[i];
                let l = format!("[{lo},{hi})");
                lo = hi;
                l
            } else {
                format!("[{lo},∞)")
            };
            if *count > 0 {
                s.push_str(&format!(" {label}={count}"));
            }
        }
        s
    }
}

/// One census per event kind. ⊘ A leaf mutex with nothing under it: it is taken on the vCPU
/// path, so anything blocking beneath it would be charged to the guest by the very
/// instrument that is supposed to explain the guest's latency.
static CENSUS: OnceLock<Mutex<Vec<(&'static str, Census)>>> = OnceLock::new();

fn census_table() -> &'static Mutex<Vec<(&'static str, Census)>> {
    CENSUS.get_or_init(|| Mutex::new(Vec::new()))
}

/// Record one finished event, and print its line if per-event printing is armed.
pub fn record(kind: &'static str, s: &Segs) {
    if !s.armed {
        return;
    }
    let a = arm();
    if a.per_event {
        eprintln!("{}", s.line(kind));
    }
    let due = {
        let mut t = census_table().lock().unwrap_or_else(|e| e.into_inner());
        let row = match t.iter_mut().find(|r| r.0 == kind) {
            Some(r) => r,
            None => {
                t.push((kind, Census::default()));
                let n = t.len() - 1;
                &mut t[n]
            }
        };
        row.1.record(s);
        a.census_every > 0 && row.1.events.is_multiple_of(a.census_every)
    };
    // ★★★ The periodic census, and it is not decoration. `143` (killed) and a truncated log
    // both leave an artefact that READS AS PRESENT; a run whose only census is at teardown
    // has no numbers at all if teardown never happens. Printing every N events means any
    // prefix of the log carries a complete, self-describing breakdown.
    if due {
        report(kind, "periodic");
    }
}

/// Print the census for one kind.
pub fn report(kind: &'static str, why: &str) {
    if !arm().on {
        return;
    }
    let t = census_table().lock().unwrap_or_else(|e| e.into_inner());
    match t.iter().find(|r| r.0 == kind) {
        Some((_, c)) => eprintln!("{}", c.report(kind, why)),
        None => eprintln!(
            "kayfabe: KFTIME-CENSUS kind={kind} why={why} ⊘ NO SUCH KIND WAS EVER RECORDED"
        ),
    }
}

/// Print every census. Called at teardown.
pub fn report_all(why: &str) {
    if !arm().on {
        return;
    }
    let t = census_table().lock().unwrap_or_else(|e| e.into_inner());
    if t.is_empty() {
        eprintln!(
            "kayfabe: KFTIME-CENSUS why={why} ⊘ THE INSTRUMENT WAS ARMED AND RECORDED NOTHING \
             — no hook fired. ⚠ Read this as a statement about the hooks, not about the plane."
        );
        return;
    }
    for (kind, c) in t.iter() {
        eprintln!("{}", c.report(kind, why));
    }
}

// =========================================================================================
// ★★★ THE INSTRUMENT'S OWN TESTS — including the one that makes it mis-attribute on purpose.
// =========================================================================================
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disarmed_records_nothing_and_allocates_no_marks() {
        let mut s = Segs {
            armed: false,
            ..Segs::start()
        };
        s.mark("a");
        s.mark("b");
        assert_eq!(s.n, 0, "a disarmed Segs must not accumulate marks");
        assert_eq!(s.marked_us(), 0);
    }

    #[test]
    fn parse_refuses_only_the_named_refusals() {
        assert_eq!(Arm::parse(None, None, None, None), Arm::off());
        assert_eq!(Arm::parse(Some("off"), None, None, None), Arm::off());
        assert_eq!(Arm::parse(Some("0"), None, None, None), Arm::off());
        let c = Arm::parse(Some("census"), None, None, None);
        assert!(c.on && !c.per_event, "census arms the aggregate only");
        let f = Arm::parse(Some("on"), None, None, None);
        assert!(f.on && f.per_event);
        // ★ An unrecognised value arms rather than disarms — see the parser's own note.
        let u = Arm::parse(Some("yes-please"), None, None, None);
        assert!(u.on && u.per_event);
    }

    #[test]
    fn parse_reads_the_injection() {
        let a = Arm::parse(Some("on"), Some("30000"), Some("vaspublish"), Some("50"));
        assert_eq!(a.inject_us, 30_000);
        assert_eq!(a.inject_seg, "vaspublish");
        assert_eq!(a.census_every, 50);
    }

    /// A hand-built `Segs` so the census arithmetic is exercised without a clock.
    fn segs_of(pairs: &[(&'static str, u64)], total_us: u64) -> Segs {
        let mut s = Segs::start();
        s.armed = true;
        for (n, us) in pairs {
            s.names[s.n] = n;
            s.us[s.n] = *us;
            s.n += 1;
        }
        // Force the bracket total by rewinding `t0`. ⊘ This is the ONLY place the clock is
        // faked, and it is faked so the sum check can be tested at all.
        s.t0 = Instant::now() - std::time::Duration::from_micros(total_us);
        s
    }

    #[test]
    fn the_unmarked_residual_is_reported_and_never_distributed() {
        let mut c = Census::default();
        c.record(&segs_of(&[("a", 10), ("b", 20)], 100));
        let r = c.report("k", "test");
        assert!(r.contains("events=1"), "{r}");
        // 100 total, 30 marked ⇒ 70 unmarked, and it must be VISIBLE, not folded into a or b.
        assert!(r.contains("UNMARKED_ms"), "{r}");
        assert!(r.contains("KFTIME-SEG a"), "{r}");
        assert!(r.contains("KFTIME-SEG b"), "{r}");
        // ⊘ The residual must not have been added to any segment.
        assert!(
            r.contains("total_ms=0.010") && r.contains("total_ms=0.020"),
            "a segment absorbed the residual: {r}"
        );
    }

    #[test]
    fn an_empty_census_says_so_rather_than_printing_zeros() {
        let c = Census::default();
        let r = c.report("k", "test");
        assert!(r.contains("NOTHING WAS RECORDED"), "{r}");
        assert!(
            r.contains("statement about the instrument"),
            "an empty census must not read as a measurement of the plane: {r}"
        );
    }

    #[test]
    fn segments_are_ranked_by_total_not_by_first_seen() {
        let mut c = Census::default();
        c.record(&segs_of(&[("small", 1), ("big", 1000)], 1001));
        let r = c.report("k", "test");
        let ib = r.find("KFTIME-SEG big").expect("big");
        let is = r.find("KFTIME-SEG small").expect("small");
        assert!(ib < is, "the dominant segment must be printed first: {r}");
    }

    /// ★★★★★ **THE KNOWN-POSITIVE, OFFLINE.** A delay of a known size is added to ONE named
    /// segment and the census must attribute it THERE and nowhere else. An instrument that
    /// has never mis-attributed on purpose has not been shown to attribute at all.
    ///
    /// ⊘ This proves the *census arithmetic* attributes. It does NOT prove the hooks are in
    /// the right places or that the guest sees the delay — only a boot can say that, and the
    /// rung runs one (`KAYFABE_KFTIME_INJECT_US`).
    #[test]
    fn a_known_delay_lands_in_the_segment_it_was_injected_into() {
        const INJECT_US: u64 = 40_000;
        let mut base = Census::default();
        let mut hot = Census::default();
        for _ in 0..5 {
            base.record(&segs_of(&[("cheap", 100), ("victim", 200), ("other", 300)], 600));
            hot.record(&segs_of(
                &[("cheap", 100), ("victim", 200 + INJECT_US), ("other", 300)],
                600 + INJECT_US,
            ));
        }
        let find = |c: &Census, n: &str| c.segs.iter().find(|r| r.0 == n).expect("seg").2;
        assert_eq!(find(&hot, "cheap"), find(&base, "cheap"), "an innocent segment moved");
        assert_eq!(find(&hot, "other"), find(&base, "other"), "an innocent segment moved");
        assert_eq!(
            find(&hot, "victim") - find(&base, "victim"),
            5 * INJECT_US,
            "the injected microseconds did not land in the injected segment"
        );
        // And the bracket total moved by exactly the same amount ⇒ the residual is unchanged.
        assert_eq!(hot.total_us - base.total_us, 5 * INJECT_US);
        assert_eq!(
            hot.total_us - hot.marked_us,
            base.total_us - base.marked_us,
            "the injection changed the UNMARKED residual, so the breakdown is not closed"
        );
    }

    #[test]
    fn maybe_inject_is_a_no_op_when_the_segment_does_not_match() {
        // ⊘ `arm()` is process-global and this test must not depend on the environment, so
        // it asserts only the branch that is decidable without arming: a zero injection.
        let a = Arm::parse(Some("on"), Some("0"), Some("x"), None);
        assert_eq!(a.inject_us, 0);
    }

    #[test]
    fn the_histogram_puts_a_hundred_millisecond_event_in_the_right_bucket() {
        let mut c = Census::default();
        c.record(&segs_of(&[("a", 1)], 120_000));
        let r = c.report("k", "test");
        // 120 000 µs falls in [100000,200000).
        assert!(r.contains("[100000,200000)=1"), "{r}");
    }

    #[test]
    fn dropped_marks_are_named_rather_than_silently_truncated() {
        let mut s = Segs::start();
        s.armed = true;
        for _ in 0..(MAX_MARKS + 3) {
            s.mark("x");
        }
        assert_eq!(s.dropped(), 3);
        assert!(s.line("k").contains("dropped_marks=3"), "{}", s.line("k"));
        assert!(s.line("k").contains("INCOMPLETE"));
    }
}
