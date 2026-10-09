//! ⚠ **PERTURBING DIAGNOSTIC, default OFF** (`KF3_DEBUG_IRQ_FLOOD=<classes>:<period_ms>`,
//! `docs/design/V3_DEBUG_IRQ_FLOOD.md`): a thread that, every `period_ms`, raises the interrupt
//! vectors of the chosen classes that the guest has ENABLED, through the ordinary path
//! ([`crate::device::Device::latch_and_deliver`]: the leaf-pending latch plus the irqfd). The guest
//! sees a normal level-pending leaf bit it must W1C and service.
//!
//! It answers one question: is a missing interrupt NOTIFICATION the reason the Windows guest tears
//! its driver down (TDR 0x116) at the lock screen? If Windows survives a flood, a missing type is
//! the cause and the classes bisect it; if it dies as without the flood, interrupts alone are not.
//!
//! Rules (the module is only policy; the thread is [`crate::device::Device::irqflood_loop`]):
//! - its own thread: never a vCPU, never the register drainer; it serves no input, so sleeping
//!   between ticks leaves nothing deaf (`THE_CONSTRAINTS.md` §35);
//! - only vectors the guest enabled, read from kf's own record of its `LEAF_EN`/`TOP_EN` writes
//!   ([`kf_trap::cpuintr::CpuIntr::vector_enabled`]); the vector sets come from the SERVED kernel
//!   interrupt table (`HostFacts::intr_table`), never a captured one;
//! - it writes NO completion word: no semaphore, no GP_GET, no notifier record, no POST_EVENT, no
//!   GSP message. A bare interrupt is not a forged completion; it announces nothing;
//! - `errors` (every other enabled stall vector) is an explicit opt-in and NEVER part of
//!   `all-completion`.
//!
//! `dispstat` is separate: it changes what the display model PRESENTS, not what is raised
//! ([`dispstat_on`], read by `display.rs`).

use kf_abi::inittables::{INTR_VECTOR_INVALID, IntrTableEntry};
use kf_rm::authored::{MC_ENGINE_IDX_DISP, MC_ENGINE_IDX_GSP};
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// The knob.
pub const ENV: &str = "KF3_DEBUG_IRQ_FLOOD";
/// Shortest and longest period accepted (a typo must not ask for a busy loop or a day).
pub const PERIOD_MS: std::ops::RangeInclusive<u64> = 1..=3_600_000;
/// The longest sleep between two checks of the stop flag.
const STOP_SLICE: Duration = Duration::from_millis(20);

/// Which classes are on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Classes {
    /// Every non-stall engine vector the guest enabled.
    pub nonstall: bool,
    /// The GSP stall vector (`MC_ENGINE_IDX_GSP`).
    pub gsp: bool,
    /// The display stall vector (`MC_ENGINE_IDX_DISP`).
    pub disp: bool,
    /// Every other enabled stall vector. Opt-in; not in `all-completion`.
    pub errors: bool,
    /// The display model presents the head-timing status and a non-zero dispatch while the
    /// guest's enable bit is set. Raises nothing.
    pub dispstat: bool,
}

/// A parsed `KF3_DEBUG_IRQ_FLOOD` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spec {
    /// The classes.
    pub classes: Classes,
    /// The tick period.
    pub period: Duration,
}

impl Spec {
    /// Parse `<classes>:<period_ms>`; classes are comma-separated from `nonstall`, `gsp`, `disp`,
    /// `all-completion` (= `nonstall,gsp,disp`), `errors`, `dispstat`.
    ///
    /// # Errors
    /// The malformed part, by name.
    pub fn parse(s: &str) -> Result<Spec, String> {
        let (cls, ms) = s
            .trim()
            .rsplit_once(':')
            .ok_or("expected <classes>:<period_ms>")?;
        let ms: u64 = ms
            .trim()
            .parse()
            .map_err(|_| format!("period {ms:?} is not a number of milliseconds"))?;
        if !PERIOD_MS.contains(&ms) {
            return Err(format!("period {ms} ms is outside {PERIOD_MS:?}"));
        }
        let mut c = Classes::default();
        for w in cls.split(',').map(str::trim) {
            match w {
                "nonstall" => c.nonstall = true,
                "gsp" => c.gsp = true,
                "disp" => c.disp = true,
                "all-completion" => (c.nonstall, c.gsp, c.disp) = (true, true, true),
                "errors" => c.errors = true,
                "dispstat" => c.dispstat = true,
                other => return Err(format!("unknown class {other:?}")),
            }
        }
        if c == Classes::default() {
            return Err("no class named".into());
        }
        Ok(Spec {
            classes: c,
            period: Duration::from_millis(ms),
        })
    }

    /// The knob's value, when set. `Ok(None)` when unset or empty.
    ///
    /// # Errors
    /// A malformed value refuses realize by name.
    pub fn from_env() -> Result<Option<Spec>, String> {
        match std::env::var(ENV) {
            Ok(v) if !v.trim().is_empty() => Spec::parse(&v)
                .map(Some)
                .map_err(|e| format!("{ENV}={v}: {e}")),
            _ => Ok(None),
        }
    }

    /// `nonstall,gsp,disp,errors,dispstat:100` — the status line's spelling.
    #[must_use]
    pub fn label(&self) -> String {
        let c = &self.classes;
        let names: Vec<&str> = [
            (c.nonstall, "nonstall"),
            (c.gsp, "gsp"),
            (c.disp, "disp"),
            (c.errors, "errors"),
            (c.dispstat, "dispstat"),
        ]
        .into_iter()
        .filter_map(|(on, n)| on.then_some(n))
        .collect();
        format!("{}:{}", names.join(","), self.period.as_millis())
    }
}

/// `dispstat` on? Read once; a malformed knob counts as off here (realize refuses it by name).
#[must_use]
pub fn dispstat_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| matches!(Spec::from_env(), Ok(Some(s)) if s.classes.dispstat))
}

/// The vector sets of each class, from the served kernel interrupt table.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// Every row's non-stall vector.
    pub nonstall: Vec<u32>,
    /// The GSP row's stall vector.
    pub gsp: Option<u32>,
    /// The display row's stall vector.
    pub disp: Option<u32>,
    /// Every other row's stall vector (not GSP, not DISP, not a non-stall vector).
    pub errors: Vec<u32>,
}

impl Plan {
    /// Sort the served table's vectors into classes. A vector that is the STALL vector of any row
    /// is a stall vector, whatever its non-stall column says: the served table repeats the stall
    /// vector as the non-stall one for engines 59-64, 73 and 1, and a stall interrupt with no
    /// cause behind it is a level the guest's ISR cannot clear (measured, run 108: `LEAF(4)` read
    /// `0x30` on 9067 reads per second after one raise of vectors 132 and 133). So `nonstall` holds
    /// only vectors no row uses as a stall vector; those belong to `errors`.
    #[must_use]
    pub fn from_table(table: &[IntrTableEntry]) -> Plan {
        let valid = |v: u32| (v != INTR_VECTOR_INVALID).then_some(v);
        let stall_of = |idx: u16| {
            table
                .iter()
                .find(|e| e.engine_idx == idx)
                .and_then(|e| valid(e.vector_stall))
        };
        let stall: Vec<u32> = table.iter().filter_map(|e| valid(e.vector_stall)).collect();
        let mut p = Plan {
            nonstall: table
                .iter()
                .filter_map(|e| valid(e.vector_non_stall))
                .filter(|v| !stall.contains(v))
                .collect(),
            gsp: stall_of(MC_ENGINE_IDX_GSP),
            disp: stall_of(MC_ENGINE_IDX_DISP),
            errors: stall
                .iter()
                .copied()
                .filter(|v| Some(*v) != stall_of(MC_ENGINE_IDX_GSP))
                .filter(|v| Some(*v) != stall_of(MC_ENGINE_IDX_DISP))
                .collect(),
        };
        p.nonstall.sort_unstable();
        p.nonstall.dedup();
        p.errors.sort_unstable();
        p.errors.dedup();
        p
    }

    /// The vectors the classes select (sorted, distinct).
    #[must_use]
    pub fn select(&self, c: &Classes) -> Vec<u32> {
        let mut v = Vec::new();
        if c.nonstall {
            v.extend(&self.nonstall);
        }
        if c.gsp {
            v.extend(self.gsp);
        }
        if c.disp {
            v.extend(self.disp);
        }
        if c.errors {
            v.extend(&self.errors);
        }
        v.sort_unstable();
        v.dedup();
        v
    }
}

/// What the flood raises through, and asks.
pub trait Sink {
    /// Has the guest enabled `vector`?
    fn enabled(&self, vector: u32) -> bool;
    /// Latch and deliver `vector` — and nothing else.
    fn raise(&self, vector: u32);
}

/// The flood's counters (written by its thread, read by the status line).
#[derive(Debug, Default)]
pub struct Counts {
    ticks: AtomicU64,
    per_vector: Mutex<BTreeMap<u32, u64>>,
}

impl Counts {
    /// `(ticks, raises per vector)`.
    #[must_use]
    pub fn snapshot(&self) -> (u64, BTreeMap<u32, u64>) {
        let m = self
            .per_vector
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default();
        (self.ticks.load(Ordering::Relaxed), m)
    }
}

/// The flood as the device holds it.
#[derive(Debug)]
pub struct Flood {
    /// The parsed knob.
    pub spec: Spec,
    /// The class sets.
    pub plan: Plan,
    /// The counters.
    pub counts: Counts,
}

impl Flood {
    /// The status line's segment: the banner and the per-vector raise counts.
    #[must_use]
    pub fn status(&self) -> String {
        let (ticks, per) = self.counts.snapshot();
        let raised: Vec<String> = per.iter().map(|(v, n)| format!("v{v}={n}")).collect();
        format!(
            " PERTURBING DIAGNOSTIC ON: irq-flood {} ticks={ticks} raised[{}]",
            self.spec.label(),
            raised.join(" ")
        )
    }
}

/// One tick: raise each selected vector the guest has enabled. Returns how many were raised.
pub fn tick(vectors: &[u32], sink: &dyn Sink, counts: &Counts) -> usize {
    counts.ticks.fetch_add(1, Ordering::Relaxed);
    let mut n = 0;
    for &v in vectors {
        if sink.enabled(v) {
            sink.raise(v);
            if let Ok(mut m) = counts.per_vector.lock() {
                *m.entry(v).or_insert(0) += 1;
            }
            n += 1;
        }
    }
    n
}

/// The flood thread's body: tick every period (sleep-until on the monotonic clock, so a slow tick
/// does not stretch the period) until `stop`.
pub fn run(flood: &Flood, sink: &dyn Sink, stop: &AtomicBool) {
    let vectors = flood.plan.select(&flood.spec.classes);
    let mut next = Instant::now() + flood.spec.period;
    while !stop.load(Ordering::Acquire) {
        let now = Instant::now();
        if now >= next {
            tick(&vectors, sink, &flood.counts);
            next += flood.spec.period;
            if next < now {
                next = now + flood.spec.period;
            }
            continue;
        }
        std::thread::sleep((next - now).min(STOP_SLICE));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn row(idx: u16, stall: u32, non_stall: u32) -> IntrTableEntry {
        IntrTableEntry {
            engine_idx: idx,
            pmc_intr_mask: 0,
            vector_stall: stall,
            vector_non_stall: non_stall,
        }
    }

    /// GR0 non-stall 0, CE2 non-stall 1, GSP 155, DISP 154, an error-ish stall row 64, a row whose
    /// stall equals its non-stall (kayfabe's engines 59-64 do this) at 133.
    fn table() -> Vec<IntrTableEntry> {
        let inv = INTR_VECTOR_INVALID;
        vec![
            row(0, inv, 0),
            row(17, inv, 1),
            row(MC_ENGINE_IDX_GSP, 155, inv),
            row(MC_ENGINE_IDX_DISP, 154, inv),
            row(20, 64, inv),
            row(61, 133, 133),
        ]
    }

    #[derive(Default)]
    struct Fake {
        enabled: Mutex<Vec<u32>>,
        raised: Mutex<Vec<u32>>,
    }
    impl Sink for Fake {
        fn enabled(&self, v: u32) -> bool {
            self.enabled.lock().unwrap().contains(&v)
        }
        fn raise(&self, v: u32) {
            self.raised.lock().unwrap().push(v);
        }
    }

    fn flood(s: &str) -> Flood {
        Flood {
            spec: Spec::parse(s).unwrap(),
            plan: Plan::from_table(&table()),
            counts: Counts::default(),
        }
    }

    #[test]
    fn parse_spells_the_knob() {
        let s = Spec::parse("all-completion:10").unwrap();
        assert_eq!(s.period, Duration::from_millis(10));
        assert!(s.classes.nonstall && s.classes.gsp && s.classes.disp);
        assert!(
            !s.classes.errors && !s.classes.dispstat,
            "not in all-completion"
        );
        let s = Spec::parse("disp, dispstat:1000").unwrap();
        assert!(s.classes.disp && s.classes.dispstat && !s.classes.gsp);
        assert_eq!(s.label(), "disp,dispstat:1000");
        for bad in [
            "",
            "gsp",
            "gsp:0",
            "gsp:x",
            "bogus:10",
            ":10",
            "gsp:3600001",
        ] {
            assert!(Spec::parse(bad).is_err(), "{bad:?} must be refused");
        }
    }

    #[test]
    fn the_plan_sorts_the_served_table() {
        let p = Plan::from_table(&table());
        assert_eq!(
            p.nonstall,
            vec![0, 1],
            "133 is also a stall vector, so it is not non-stall"
        );
        assert_eq!((p.gsp, p.disp), (Some(155), Some(154)));
        assert_eq!(p.errors, vec![64, 133]);
    }

    #[test]
    fn only_enabled_vectors_are_raised_and_counted() {
        let f = flood("all-completion:10");
        let sink = Fake::default();
        *sink.enabled.lock().unwrap() = vec![1, 155, 64, 133]; // not 0, 154; 64 and 133 are enabled but are `errors`
        let v = f.plan.select(&f.spec.classes);
        assert_eq!(tick(&v, &sink, &f.counts), 2);
        assert_eq!(tick(&v, &sink, &f.counts), 2);
        assert_eq!(*sink.raised.lock().unwrap(), vec![1, 155, 1, 155]);
        let (ticks, per) = f.counts.snapshot();
        assert_eq!(ticks, 2);
        assert_eq!(per, BTreeMap::from([(1, 2), (155, 2)]));
        // the guest enables the display vector later: the next tick raises it
        sink.enabled.lock().unwrap().push(154);
        assert_eq!(tick(&v, &sink, &f.counts), 3);
    }

    #[test]
    fn classes_select_their_vectors_and_errors_stay_out_of_all_completion() {
        let p = Plan::from_table(&table());
        let sel = |s: &str| p.select(&Spec::parse(s).unwrap().classes);
        assert_eq!(sel("gsp:10"), vec![155]);
        assert_eq!(sel("disp:10"), vec![154]);
        assert_eq!(sel("nonstall:10"), vec![0, 1]);
        assert_eq!(sel("all-completion:10"), vec![0, 1, 154, 155]);
        for e in [64, 133] {
            assert!(
                !sel("all-completion:10").contains(&e),
                "errors NEVER in all-completion"
            );
        }
        assert_eq!(sel("errors:10"), vec![64, 133]);
        assert_eq!(
            sel("all-completion,errors:10"),
            vec![0, 1, 64, 133, 154, 155]
        );
        assert!(sel("dispstat:10").is_empty(), "dispstat raises nothing");
    }

    #[test]
    fn the_thread_ticks_and_stops_on_shutdown() {
        let f = Arc::new(flood("gsp:1"));
        let sink = Arc::new(Fake::default());
        sink.enabled.lock().unwrap().push(155);
        let stop = Arc::new(AtomicBool::new(false));
        let t = {
            let (f, sink, stop) = (f.clone(), sink.clone(), stop.clone());
            std::thread::spawn(move || run(&f, &*sink, &stop))
        };
        let t0 = Instant::now();
        while sink.raised.lock().unwrap().len() < 3 {
            assert!(
                t0.elapsed() < Duration::from_secs(5),
                "the flood never ticked"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        stop.store(true, Ordering::Release);
        t.join().unwrap();
        let n = sink.raised.lock().unwrap().len();
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(
            sink.raised.lock().unwrap().len(),
            n,
            "nothing is raised after shutdown"
        );
        assert!(
            f.status()
                .contains("PERTURBING DIAGNOSTIC ON: irq-flood gsp:1 ")
        );
        assert!(f.status().contains("v155="));
    }

    #[test]
    fn a_long_period_still_stops_promptly() {
        let f = Arc::new(flood("gsp:3600000"));
        let stop = Arc::new(AtomicBool::new(false));
        let t = {
            let (f, stop) = (f.clone(), stop.clone());
            std::thread::spawn(move || run(&f, &Fake::default(), &stop))
        };
        stop.store(true, Ordering::Release);
        let t0 = Instant::now();
        t.join().unwrap();
        assert!(t0.elapsed() < Duration::from_secs(1));
    }
}
