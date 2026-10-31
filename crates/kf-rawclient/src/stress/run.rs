//! The refresh stress arm's orchestration. It calls only [`Backend`], [`Clock`] and [`Progress`], so every
//! decision in it (qualification, the batch loop, the single invalidate, the oracle, the caps and the
//! verdict) is exercised in tests with a fake backend.
//!
//! One refresh, in order (V3_RAW_CLIENT.md §4.1-§4.5):
//!
//! 1. build fragmented memory (pressure, then a seeded random touch order writing each page's signature);
//! 2. **census and qualify**: a workload that fails decides nothing (NOTRUN, rc 3), before anything is pinned;
//! 3. pin each batch (OS descriptor) and map it, deferring the TLB invalidate;
//! 4. **one** invalidate;
//! 5. the oracle over a bounded sample (signature, decoy), then the live-page check;
//! 6. unmap each batch, check the unmap postcondition, free each descriptor, release the memory.
//!
//! Between the last map and the invalidate nothing reads a mapped VA (a deferred PTE may not be live yet),
//! and after the unmap nothing reads an unmapped VA: the arm never provokes a fault.

use super::backend::{Backend, BackendError, Probe, Setup};
use super::calibration::TDR_REFERENCE_NS;
use super::census::{CensusBuilder, PageFrame, Qualification, line as qualify_line, qualify};
use super::oracle::{LIVE_EPOCH, OracleTally, decoy_word, plan_sample, signature};
use super::params::StressParams;
use super::plan::{MemPlan, plan_memory};
use super::rng::{SplitMix64, choose_distinct, mix, permutation};
use super::verdict::{Evidence, PostState, control_passed, gates};
use crate::caps::{self, Needs};
use crate::cell::Cell;
use crate::clock::{Clock, timed};
use crate::hist::LogHist;
use crate::json::Obj;
use crate::ledger::{Check, CheckState, Ledger, Sink};
use crate::mode::Mode;
use crate::outcome::{Outcome, token};
use crate::registry::ArmSpec;
use crate::watchdog::Progress;

/// Touches between deadline checks while building memory.
const DEADLINE_STRIDE: u32 = 8192;
/// Copies between deadline checks in the oracle.
const COPY_STRIDE: usize = 128;

/// What the arm is handed besides its parameters.
pub struct StressEnv<'a> {
    /// The monotonic clock.
    pub clock: &'a dyn Clock,
    /// Phase reporting for the watchdog.
    pub progress: &'a dyn Progress,
    /// Where lines go.
    pub sink: &'a mut dyn Sink,
    /// The declared mode.
    pub mode: Mode,
    /// The declared source revision.
    pub rev: Option<String>,
    /// The seed used when `--stress-seed` was not given.
    pub default_seed: u64,
}

/// Times of one completed refresh, nanoseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RefreshStats {
    /// Sum of the pin calls.
    pub pin_ns: u64,
    /// Sum of the map batches.
    pub map_ns: u64,
    /// The invalidate.
    pub inval_ns: u64,
    /// The whole oracle (copies, reads, live check).
    pub oracle_ns: u64,
    /// Sum of the unmap batches.
    pub unmap_ns: u64,
    /// Sum of the frees.
    pub free_ns: u64,
    /// The slowest single RM call or fence wait.
    pub max_call_ns: u64,
}

impl RefreshStats {
    /// The span a WDDM paging operation is blocked: the maps and the invalidate.
    #[must_use]
    pub fn critical_ns(&self) -> u64 {
        self.map_ns.saturating_add(self.inval_ns)
    }

    fn line(&self, n: usize) -> String {
        let ms = |ns: u64| format!("{:.3}", ns as f64 / 1e6);
        format!(
            "STRESS_REFRESH n={n} pin_ms={} map_ms={} inval_ms={} oracle_ms={} unmap_ms={} free_ms={} critical_ms={} max_call_ms={}",
            ms(self.pin_ns),
            ms(self.map_ns),
            ms(self.inval_ns),
            ms(self.oracle_ns),
            ms(self.unmap_ns),
            ms(self.free_ns),
            ms(self.critical_ns()),
            ms(self.max_call_ns),
        )
    }
}

/// What a run produced.
#[derive(Debug)]
pub struct StressReport {
    /// The verdict.
    pub outcome: Outcome,
    /// The named checks (empty when the run did not reach the gates).
    pub checks: Vec<Check>,
    /// One entry per completed refresh.
    pub refreshes: Vec<RefreshStats>,
    /// Whether the positive control (the decoy) held.
    pub control: Option<bool>,
    /// The seed used.
    pub seed: u64,
    /// Calls that took longer than 2 s (counted, not gated).
    pub stall_over_2s: u64,
}

#[derive(Default)]
struct Hists {
    pin: LogHist,
    map: LogHist,
    unmap: LogHist,
    inval: LogHist,
    copy: LogHist,
}

enum Stop {
    Done(Outcome),
    /// The soft deadline passed mid-refresh: the refresh is dropped, the run continues to the verdict.
    Deadline,
}

/// The verdict a backend error means: an unsupported platform decides nothing, a failed call fails.
fn outcome_of(phase: &str, e: BackendError) -> Outcome {
    match e {
        BackendError::Unsupported(r, d) => Outcome::NotRun(r, format!("{phase}:{d}")),
        BackendError::Failed(d) => Outcome::Fail(format!("{phase}:{d}")),
    }
}

fn stop(phase: &'static str) -> impl Fn(BackendError) -> Stop {
    move |e| Stop::Done(outcome_of(phase, e))
}

/// The slowest single call, overall and in the current refresh, and how many exceeded the TDR reference.
#[derive(Debug, Default, Clone, Copy)]
struct CallStats {
    max_run_ns: u64,
    max_refresh_ns: u64,
    over_tdr: u64,
}

impl CallStats {
    fn note(&mut self, ns: u64) {
        self.max_run_ns = self.max_run_ns.max(ns);
        self.max_refresh_ns = self.max_refresh_ns.max(ns);
        if ns > TDR_REFERENCE_NS {
            self.over_tdr += 1;
        }
    }
}

struct Run<'r, 'e> {
    p: &'r StressParams,
    b: &'r mut dyn Backend,
    env: &'r mut StressEnv<'e>,
    seed: u64,
    plan: MemPlan,
    t_start: u64,
    hist: Hists,
    tally: OracleTally,
    calls: CallStats,
    probed_free: u64,
    still_mapped: u64,
    probe_unsupported: bool,
    teardown_errors: u32,
    pinned: Vec<bool>,
    mapped: Vec<bool>,
}

impl Run<'_, '_> {
    fn note_call(&mut self, ns: u64) {
        self.calls.note(ns);
    }

    fn past_deadline(&self) -> bool {
        self.env.clock.now_ns().saturating_sub(self.t_start)
            >= u64::from(self.p.deadline_s) * 1_000_000_000
    }

    fn check_deadline(&self) -> Result<(), Stop> {
        if self.past_deadline() {
            Err(Stop::Deadline)
        } else {
            Ok(())
        }
    }

    /// Un-map, un-pin and release whatever a refresh left behind. Errors here are counted, not fatal.
    fn cleanup(&mut self) {
        for i in 0..self.p.batches {
            if std::mem::take(&mut self.mapped[i as usize]) && self.b.unmap_batch(i).is_err() {
                self.teardown_errors += 1;
            }
        }
        for i in 0..self.p.batches {
            if std::mem::take(&mut self.pinned[i as usize]) && self.b.free_batch(i).is_err() {
                self.teardown_errors += 1;
            }
        }
        self.b.release_workload();
        self.b.pressure_end();
    }

    /// Build the fragmented workload, census it, qualify it. `Err(Done(NotRun))` when it does not qualify.
    fn build_and_qualify(&mut self, r: u32) -> Result<(), Stop> {
        let p = self.p;
        let total = p.total_pages();
        self.env.progress.enter("pressure", r, 0);
        if p.pressure && self.plan.pressure_bytes > 0 {
            self.b
                .pressure_begin(self.plan.pressure_bytes, mix(self.seed ^ u64::from(r)))
                .map_err(stop("pressure"))?;
        }
        self.env.progress.enter("alloc", r, 0);
        self.b.alloc_workload(total).map_err(stop("alloc"))?;
        self.env.progress.enter("touch", r, 0);
        let order = permutation(total, mix(self.seed ^ 0x70_0000 ^ u64::from(r)));
        for (n, &page) in order.iter().enumerate() {
            if (n as u32).is_multiple_of(DEADLINE_STRIDE) {
                self.check_deadline()?;
            }
            self.b
                .touch(page, signature(self.seed, r, page))
                .map_err(stop("touch"))?;
        }
        self.env.progress.enter("census", r, 0);
        let mut cb = CensusBuilder::new();
        let mut buf = vec![PageFrame::default(); p.pages_per_batch as usize];
        for batch in 0..p.batches {
            self.b
                .pfns(batch * p.pages_per_batch, &mut buf)
                .map_err(stop("census"))?;
            cb.add_batch(&buf);
        }
        let thp = self.b.huge_bytes().map_err(stop("census"))?;
        let census = cb.finish();
        let q = qualify(&census, p.min_run_permille, thp);
        let avail_mib = self.b.mem_available_bytes().unwrap_or(0) >> 20;
        self.env.sink.line(&qualify_line(
            &census,
            thp,
            avail_mib,
            self.seed,
            p.min_run_permille,
            &q,
        ));
        match q {
            Qualification::Qualified => {
                self.b.pressure_end();
                Ok(())
            }
            Qualification::NotRun(reason, detail) => {
                Err(Stop::Done(Outcome::NotRun(reason, detail)))
            }
        }
    }

    /// One refresh from a qualified workload to freed memory.
    fn pin_map_oracle_unmap(&mut self, r: u32) -> Result<RefreshStats, Stop> {
        let p = self.p;
        let clock = self.env.clock;
        let mut st = RefreshStats::default();
        self.calls.max_refresh_ns = 0;

        for batch in 0..p.batches {
            self.check_deadline()?;
            self.env.progress.enter("pin", r, batch);
            let (res, ns) = timed(clock, || self.b.pin_batch(batch));
            res.map_err(stop("pin"))?;
            self.pinned[batch as usize] = true;
            st.pin_ns += ns;
            self.hist.pin.record(ns);
            self.note_call(ns);
        }
        for batch in 0..p.batches {
            self.check_deadline()?;
            self.env.progress.enter("map", r, batch);
            let mut n_calls = 0u64;
            let (res, ns) = timed(clock, || {
                let hist = &mut self.hist.map;
                let calls = &mut self.calls;
                let mut report = |c: u64| {
                    n_calls += 1;
                    hist.record(c);
                    calls.note(c);
                };
                self.b.map_batch(batch, clock, &mut report)
            });
            res.map_err(stop("map"))?;
            self.mapped[batch as usize] = true;
            st.map_ns += ns;
            if n_calls == 0 {
                // A backend that reported no per-call durations: the batch is one call.
                self.hist.map.record(ns);
                self.note_call(ns);
            }
        }

        self.env.progress.enter("invalidate", r, 0);
        let (res, ns) = timed(clock, || self.b.invalidate());
        res.map_err(stop("invalidate"))?;
        st.inval_ns = ns;
        self.hist.inval.record(ns);
        self.note_call(ns);

        self.oracle(r, &mut st)?;

        for batch in 0..p.batches {
            self.env.progress.enter("unmap", r, batch);
            let (res, ns) = timed(clock, || self.b.unmap_batch(batch));
            res.map_err(stop("unmap"))?;
            self.mapped[batch as usize] = false;
            st.unmap_ns += ns;
            self.hist.unmap.record(ns);
            self.note_call(ns);
        }
        self.env.progress.enter("postcondition", r, 0);
        for batch in 0..p.batches {
            match self
                .b
                .probe_free(batch * p.pages_per_batch)
                .map_err(stop("postcondition"))?
            {
                Probe::Free => self.probed_free += 1,
                Probe::Mapped => self.still_mapped += 1,
                Probe::Unsupported => self.probe_unsupported = true,
            }
        }
        for batch in 0..p.batches {
            self.env.progress.enter("free", r, batch);
            let (res, ns) = timed(clock, || self.b.free_batch(batch));
            res.map_err(stop("free"))?;
            self.pinned[batch as usize] = false;
            st.free_ns += ns;
            self.note_call(ns);
        }
        self.b.release_workload();
        st.max_call_ns = self.calls.max_refresh_ns;
        Ok(st)
    }

    fn oracle(&mut self, r: u32, st: &mut RefreshStats) -> Result<(), Stop> {
        let p = self.p;
        let clock = self.env.clock;
        let total = p.total_pages();
        let words_per_page = (p.leaf_bytes / 8) as usize;
        let mut words = vec![0u64; words_per_page];
        let sample = plan_sample(p, self.seed, r);
        let t0 = clock.now_ns();
        for (slot, &page) in sample.iter().enumerate() {
            if slot % COPY_STRIDE == 0 {
                self.check_deadline()?;
            }
            self.env.progress.enter("oracle", r, slot as u32);
            let (res, ns) = timed(clock, || self.b.copy_to_staging(page, slot as u32));
            res.map_err(stop("oracle-copy"))?;
            self.hist.copy.record(ns);
            self.note_call(ns);
            self.b
                .read_staging(slot as u32, &mut words)
                .map_err(stop("oracle-read"))?;
            let want = signature(self.seed, r, page);
            let previous = (r > 0).then(|| signature(self.seed, r - 1, page));
            let decoy = decoy_word(self.seed, r, page, total, want);
            self.tally.judge_page(page, &words, want, previous, decoy);
        }
        // The live-page check: the CPU rewrites K sampled pages after the map; the GPU must see them.
        self.env.progress.enter("oracle-live", r, 0);
        let mut rng = SplitMix64::new(mix(self.seed ^ 0x11_FE ^ u64::from(r)));
        let slots = choose_distinct(0, sample.len() as u32, p.live, &mut rng);
        for slot in slots {
            let page = sample[slot as usize];
            let new = signature(self.seed, LIVE_EPOCH, page);
            self.b
                .rewrite_page(page, new)
                .map_err(stop("live-rewrite"))?;
            let (res, ns) = timed(clock, || self.b.copy_to_staging(page, slot));
            res.map_err(stop("live-copy"))?;
            self.hist.copy.record(ns);
            self.note_call(ns);
            self.b
                .read_staging(slot, &mut words)
                .map_err(stop("live-read"))?;
            self.tally.judge_live(&words, new);
        }
        st.oracle_ns = clock.now_ns().saturating_sub(t0);
        Ok(())
    }
}

/// Run the refresh stress against `b`.
pub fn run_stress(
    p: &StressParams,
    spec: &ArmSpec,
    b: &mut dyn Backend,
    env: &mut StressEnv<'_>,
) -> StressReport {
    let seed = p.seed.unwrap_or(env.default_seed);
    env.sink.line(&p.line(seed));
    let mut ledger = Ledger::default();

    let early = |env: &mut StressEnv<'_>, ledger: &Ledger, cell: Option<&Cell>, o: Outcome| {
        conclude(
            env,
            spec,
            p,
            ledger,
            cell,
            &[],
            &Hists::default(),
            o,
            None,
            seed,
            0,
        )
    };

    env.progress.enter("facts", 0, 0);
    let facts = match b.facts() {
        Ok(f) => f,
        Err(e) => return early(env, &ledger, None, outcome_of("facts", e)),
    };
    let cell = Cell::from_facts(&facts, env.mode, env.rev.as_deref());
    env.sink.line(&cell.line());
    let needs = Needs {
        leaf_bytes: p.leaf_bytes,
        va_bytes: p.workload_bytes() + p.staging_bytes(),
    };
    if let Err(m) = caps::check_all(spec.requires, &facts, &needs) {
        let o = Outcome::NotRun(m.reason, format!("{}:{}", m.cap.word(), m.detail));
        return early(env, &ledger, Some(&cell), o);
    }
    env.progress.enter("plan", 0, 0);
    let avail = match b.mem_available_bytes() {
        Ok(a) => a,
        Err(e) => return early(env, &ledger, Some(&cell), outcome_of("plan", e)),
    };
    let plan = match plan_memory(p, avail) {
        Ok(m) => m,
        Err((r, d)) => return early(env, &ledger, Some(&cell), Outcome::NotRun(r, d)),
    };
    env.progress.enter("setup", 0, 0);
    let setup = Setup {
        batches: p.batches,
        pages_per_batch: p.pages_per_batch,
        leaf_bytes: p.leaf_bytes,
        sample: p.sample,
        shape: p.shape,
    };
    if let Err(e) = b.setup(&setup) {
        b.teardown();
        return early(env, &ledger, Some(&cell), outcome_of("setup", e));
    }

    let t_start = env.clock.now_ns();
    let mut run = Run {
        p,
        b,
        env,
        seed,
        plan,
        t_start,
        hist: Hists::default(),
        tally: OracleTally::default(),
        calls: CallStats::default(),
        probed_free: 0,
        still_mapped: 0,
        probe_unsupported: false,
        teardown_errors: 0,
        pinned: vec![false; p.batches as usize],
        mapped: vec![false; p.batches as usize],
    };
    let mut refreshes: Vec<RefreshStats> = Vec::new();
    let mut stop: Option<Outcome> = None;

    for r in 0..p.refreshes {
        // Refresh 0 always starts; later ones only inside the soft deadline.
        if r > 0 && run.past_deadline() {
            break;
        }
        let step = run
            .build_and_qualify(r)
            .and_then(|()| run.pin_map_oracle_unmap(r));
        match step {
            Ok(st) => {
                run.env.sink.line(&st.line(refreshes.len()));
                refreshes.push(st);
            }
            Err(Stop::Deadline) => {
                run.cleanup();
                break;
            }
            Err(Stop::Done(o)) => {
                run.cleanup();
                stop = Some(o);
                break;
            }
        }
        run.cleanup();
    }
    run.b.teardown();

    if let Some(o) = stop {
        let hist = std::mem::take(&mut run.hist);
        let over = run.calls.over_tdr;
        return conclude(
            run.env,
            spec,
            p,
            &ledger,
            Some(&cell),
            &refreshes,
            &hist,
            o,
            None,
            seed,
            over,
        );
    }

    let post = if run.still_mapped > 0 {
        PostState::StillMapped {
            count: run.still_mapped,
        }
    } else if run.probe_unsupported || run.probed_free == 0 {
        PostState::Unchecked("probe_va-verb-missing")
    } else {
        PostState::Free {
            probed: run.probed_free,
        }
    };
    let evidence = Evidence {
        completed: refreshes.len() as u32,
        tally: run.tally.clone(),
        post,
        max_call_ns: run.calls.max_run_ns,
        stall_limit_ns: u64::from(p.phase_s) * 1_000_000_000,
        teardown_errors: run.teardown_errors,
    };
    let checks = gates(&evidence);
    let control = control_passed(&checks);
    for c in checks.iter().cloned() {
        ledger.record(run.env.sink, c);
    }
    let outcome = if ledger.passed() {
        Outcome::Pass
    } else {
        Outcome::Fail(ledger.failed().join(","))
    };
    let hist = std::mem::take(&mut run.hist);
    let over = run.calls.over_tdr;
    let summary = format!(
        "STRESS_SUMMARY completed={} attempted={} oracle_checked={} oracle_mismatches={} stale={} decoy={}/{} live={}/{} postcondition={} stall_over_2s={over} max_call_ms={:.3} teardown_errors={}",
        refreshes.len(),
        p.refreshes,
        run.tally.checked,
        run.tally.mismatches,
        run.tally.stale,
        run.tally.decoy_tripped,
        run.tally.decoy_checked,
        run.tally.live_checked - run.tally.live_mismatches,
        run.tally.live_checked,
        match &evidence.post {
            PostState::Free { .. } => "free",
            PostState::StillMapped { .. } => "still-mapped",
            PostState::Unchecked(_) => "unchecked",
        },
        run.calls.max_run_ns as f64 / 1e6,
        run.teardown_errors,
    );
    run.env.sink.line(&summary);
    conclude(
        run.env,
        spec,
        p,
        &ledger,
        Some(&cell),
        &refreshes,
        &hist,
        outcome,
        Some(control),
        seed,
        over,
    )
}

/// Print the histograms, the control, the `RUNG_` token and the JSON record; build the report.
#[allow(clippy::too_many_arguments)]
fn conclude(
    env: &mut StressEnv<'_>,
    spec: &ArmSpec,
    p: &StressParams,
    ledger: &Ledger,
    cell: Option<&Cell>,
    refreshes: &[RefreshStats],
    hist: &Hists,
    outcome: Outcome,
    control: Option<bool>,
    seed: u64,
    stall_over_2s: u64,
) -> StressReport {
    if !refreshes.is_empty() {
        for (op, h) in [
            ("pin", &hist.pin),
            ("map", &hist.map),
            ("inval", &hist.inval),
            ("unmap", &hist.unmap),
            ("copy", &hist.copy),
        ] {
            env.sink.line(&h.line(op));
        }
    }
    if let Some(c) = control {
        env.sink.line(&format!(
            "RUNGCTL_{}={}",
            spec.rung,
            if c { "PASS" } else { "FAIL" }
        ));
    }
    env.sink.line(&outcome.rung_line(spec.rung));
    let mut o = Obj::new();
    o.str("arm", spec.name)
        .str("outcome", outcome.word())
        .u64("rc", u64::from(outcome.exit_code()))
        .u64("seed", seed)
        .u64("batches", u64::from(p.batches))
        .u64("pages_per_batch", u64::from(p.pages_per_batch))
        .u64("completed", refreshes.len() as u64)
        .u64("stall_over_2s", stall_over_2s);
    match &outcome {
        Outcome::Fail(w) => {
            o.str("why", w);
        }
        Outcome::NotRun(r, d) => {
            o.str("reason", r.word()).str("detail", &token(d));
        }
        Outcome::Pass => {}
    }
    if let Some(c) = cell {
        let mut co = Obj::new();
        co.str("die", &c.die)
            .str("family", &c.family)
            .str("mmu", &c.mmu)
            .str("drv", &c.drv)
            .str("kmod", &c.kmod)
            .str("mode", c.mode.word())
            .str("rev", &c.rev);
        o.raw("cell", &co.finish());
    }
    let rs = refreshes
        .iter()
        .map(|r| {
            let mut x = Obj::new();
            x.u64("pin_ns", r.pin_ns)
                .u64("map_ns", r.map_ns)
                .u64("inval_ns", r.inval_ns)
                .u64("oracle_ns", r.oracle_ns)
                .u64("unmap_ns", r.unmap_ns)
                .u64("free_ns", r.free_ns)
                .u64("critical_ns", r.critical_ns())
                .u64("max_call_ns", r.max_call_ns);
            x.finish()
        })
        .collect::<Vec<_>>()
        .join(",");
    o.raw("refreshes", &format!("[{rs}]"));
    let cs = ledger
        .checks()
        .iter()
        .map(|c| {
            let mut x = Obj::new();
            x.str("name", c.name)
                .str(
                    "state",
                    match c.state {
                        CheckState::Pass => "PASS",
                        CheckState::Fail => "FAIL",
                        CheckState::Skip => "SKIP",
                    },
                )
                .str("why", &c.why);
            x.finish()
        })
        .collect::<Vec<_>>()
        .join(",");
    o.raw("checks", &format!("[{cs}]"));
    env.sink.line(&format!("RAWCLIENT_JSON {}", o.finish()));
    StressReport {
        outcome,
        checks: ledger.checks().to_vec(),
        refreshes: refreshes.to_vec(),
        control,
        seed,
        stall_over_2s,
    }
}
