//! ★★★★★ **The worker THREAD** — the OS half around `kf_core::Plane::worker_pass`.
//!
//! `kf-core` owns the plane (scan, claim, submit decision, §8 completion rule, release) and makes no
//! OS call. This is the loop a worker thread runs around it: pass; if nothing was served, park via
//! the wake word's `try_park(seen)` and wait in `epoll` on the worker eventfd and the session
//! completion fd. ★ A completion rings the in-flight channels' OWN tokens (`Plane::ring_internal`),
//! so it reaches the channel through the same claim path as a doorbell — never a direct call.
//! ⊘ There is exactly ONE worker loop in v3; this is it (it replaced a duplicate claim loop that
//! lived here at gate 4).

use crate::completions::Completions;
use crate::user_loop::{self, Peek, Stats as RelayStats};
use kf_core::{HostOps, Plane};
use kf_linux_raw::{Notifier, PollTimeout, Poller, ReadyTokens};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// The poller tag of the worker eventfd.
pub const WORKER_EFD_TAG: u64 = 0;
/// The poller tag of the session's completion fd ([`Completions`]).
pub const COMPLETIONS_TAG: u64 = 1 << 32;
const SCAN_LIMIT: usize = 64;
/// How long a parked worker sleeps before re-checking `stop` and the relays' safety net
/// ([`user_loop::BACKSTOP_MS`]): never a correctness timeout, every firing that finds work is counted.
const PARK_MS: u32 = user_loop::BACKSTOP_MS;
/// ★ 2026-10-08: how long a parked worker sleeps while `on_tick` says a raise is owed (the
/// paced Passthrough relay's trailing raise, `kf_chan::ptnsi::Pacer::flush`).
const OWED_PARK_MS: u32 = 1;

/// Counters, for gates — never read on a hot path.
#[derive(Debug, Default)]
pub struct WorkerStats {
    /// Tokens served by `worker_pass`.
    pub served: AtomicU64,
    /// Times a worker parked in `epoll`.
    pub parks: AtomicU64,
    /// Completion wakes turned into internal rings.
    pub host_rings: AtomicU64,
    /// ★ w827 attribution (only when [`WorkerStats::prof`] is set): ns outside the park wait.
    pub busy_ns: AtomicU64,
    /// ns inside the park wait.
    pub wait_ns: AtomicU64,
    /// The longest single stretch outside a wait.
    pub max_busy_ns: AtomicU64,
    /// Waits entered.
    pub waits: AtomicU64,
    /// Waits that ended by TIMEOUT.
    pub timeouts: AtomicU64,
    /// Timeouts after which the next pass served a token — a lost or late wake.
    pub timeouts_with_work: AtomicU64,
    /// Turns the accounting on (two clock reads per park).
    pub prof: AtomicBool,
}

/// ★ P5b: the first tag a caller may give an EXTRA fd in the same poller (a per-engine host
/// non-stall event, whose readiness `on_other` turns into a guest interrupt). Tags at or above it
/// are handed to `on_other`; nothing else reaches it.
pub const OTHER_TAG_BASE: u64 = 2 << 32;

/// ★ 2026-10-08: the tag `on_other` receives once after EVERY park wait of a worker ends (events or the
/// [`PARK_MS`] timeout) — a bounded-period tick on the worker, never a vCPU. No fd carries it. Used by
/// `KF3_RELAY_GET_REFRESH` (the USERD relay's `GP_GET` refresh while a relayed twin lives); a caller
/// that ignores it loses nothing.
pub const TICK_TAG: u64 = u64::MAX;

/// ★ 2026-10-14 — what the worker needs from the USER-ring relays to be an EVENT loop
/// (`docs/design/V3_USER_RING_EVENT_LOOP_20261014.md`): whether any relay waits for something only a
/// LOOK can find (the private `GP_GET` word is not an fd), and the look itself. Neither blocks.
pub trait RelayLoop {
    /// Some relay is waiting for private-ring progress (the spin is worth doing).
    fn waiting(&self) -> bool;
    /// Look at the waiting relays' not-yet-fd state and ring what is satisfied. Returns whether
    /// anything was rung. `why` names the caller for the counters (a backstop peek that finds work is
    /// a bug counter, never the normal path).
    fn peek(&self, why: Peek) -> bool;
}

/// What the worker loop does after a `worker_pass` that served `served` tokens (`docs/design/V3_USER_RING_EVENT_LOOP_20261014.md`
/// section 10). The decision is a pure function so that the fairness model drives THE SAME code the loop runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AfterPass {
    /// Run the next pass at once.
    Continue,
    /// Look before the next pass: poll the epoll set with timeout 0 (engine events, completions, the doorbell eventfd) and peek the
    /// waiting relays, then run the next pass.
    Look,
    /// Nothing was served: spin, then park.
    Idle,
}

/// The worker loop's per-iteration policy state.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LoopPolicy {
    busy_passes: u32,
    /// Whether a busy worker ever looks (`true` = the bounded-fairness fix; `false` only under the negative control).
    look: bool,
}

impl Default for LoopPolicy {
    /// The fix: a busy worker looks every [`LOOK_EVERY`] passes.
    fn default() -> Self {
        Self {
            busy_passes: 0,
            look: true,
        }
    }
}

/// ★ 2026-10-20 NEGATIVE CONTROL (default off, read once): `KF3_NEGCTL_NO_BUSY_LOOK=1` makes [`LoopPolicy::after_pass`] never answer
/// [`AfterPass::Look`] -- the loop as it was before the bounded-fairness fix -- so ONE binary can run both arms of an A/B by one
/// environment variable.
pub const NEGCTL_NO_BUSY_LOOK: &str = "KF3_NEGCTL_NO_BUSY_LOOK";

/// Whether [`NEGCTL_NO_BUSY_LOOK`]'s value turns the control on: exactly `1`.
#[must_use]
pub fn parse_negctl_no_busy_look(_value: Option<&str>) -> bool {
    todo!("EXPECTED RED")
}

/// The control's setting, read from the environment once.
#[must_use]
pub fn negctl_no_busy_look() -> bool {
    todo!("EXPECTED RED")
}

/// After this many consecutive busy passes (each served something) the loop looks once. A pass serves a hot token up to
/// `REACT_ROUNDS + 1` times, so the delay a busy worker adds to any wait is bounded by `LOOK_EVERY` passes, whatever the neighbours do.
pub const LOOK_EVERY: u32 = 4;

impl LoopPolicy {
    /// The policy of the pre-fix loop: never a busy look (the negative control).
    #[must_use]
    pub fn without_busy_look() -> Self {
        todo!("EXPECTED RED")
    }
    /// The policy the worker runs: the fix, unless the negative control is on.
    #[must_use]
    pub fn from_env() -> Self {
        Self::from_setting(negctl_no_busy_look())
    }
    /// The policy for a given control setting.
    #[must_use]
    pub fn from_setting(no_busy_look: bool) -> Self {
        if no_busy_look {
            Self::without_busy_look()
        } else {
            Self::default()
        }
    }
    /// What follows a pass that served `served` tokens.
    pub fn after_pass(&mut self, served: usize) -> AfterPass {
        if served > 0 {
            self.busy_passes += 1;
            if self.busy_passes >= LOOK_EVERY {
                self.busy_passes = 0;
                AfterPass::Look
            } else {
                AfterPass::Continue
            }
        } else {
            self.busy_passes = 0;
            AfterPass::Idle
        }
    }
}

/// No relays (the harness): the loop is the old one minus the timer-driven refresh.
pub struct NoRelay;
impl RelayLoop for NoRelay {
    fn waiting(&self) -> bool {
        false
    }
    fn peek(&self, _why: Peek) -> bool {
        false
    }
}

/// One worker thread's loop, until `stop`. `poller` watches `efd` at [`WORKER_EFD_TAG`] and
/// `completions` at [`COMPLETIONS_TAG`]; any fd the caller watched at a tag `>=`
/// [`OTHER_TAG_BASE`] is reported to `on_other` (P5b: engine non-stall events → guest MSI-X).
/// ★ 2026-10-08: `on_completion` runs once per readiness of `completions` (a REAL host
/// `FIFO_EVENT_MTHD` edge), after the in-flight tokens were rung — the device's Passthrough relay
/// (`kf_chan::ptnsi`) judges it there. `on_tick` runs once per loop iteration, edge or not (the
/// relay's timer: it raises what pacing owes) and answers whether anything is still owed — the
/// park is then at most [`OWED_PARK_MS`], so an owed raise goes out even if no edge ever comes.
#[allow(clippy::too_many_arguments)]
pub fn run(
    plane: &Plane<'_>,
    host: &dyn HostOps,
    poller: &Poller,
    efd: &Notifier,
    completions: &Completions,
    stats: &WorkerStats,
    stop: &AtomicBool,
    on_other: &dyn Fn(u64),
    on_completion: &dyn Fn(),
    on_tick: &dyn Fn() -> bool,
) {
    run_with_relay(
        plane,
        host,
        poller,
        efd,
        completions,
        stats,
        stop,
        on_other,
        on_completion,
        on_tick,
        &NoRelay,
        &RelayStats::default(),
    );
}

/// [`run`] with the USER-ring relays' event hooks ([`RelayLoop`]) and their counters: with nothing
/// to serve the worker SPINS a bounded [`user_loop::SPIN`] iterations (`epoll_wait(timeout=0)` + a
/// peek) while a relay waits, then parks for at most [`PARK_MS`] ([`user_loop::BACKSTOP_MS`]); a
/// timeout is the relays' safety net (a peek whose findings are counted), never their clock.
#[allow(clippy::too_many_arguments)]
pub fn run_with_relay(
    plane: &Plane<'_>,
    host: &dyn HostOps,
    poller: &Poller,
    efd: &Notifier,
    completions: &Completions,
    stats: &WorkerStats,
    stop: &AtomicBool,
    on_other: &dyn Fn(u64),
    on_completion: &dyn Fn(),
    on_tick: &dyn Fn() -> bool,
    relay: &dyn RelayLoop,
    relay_stats: &RelayStats,
) {
    // The ready tags of one wait, dispatched. Returns (doorbell eventfd, engine/completion/other event).
    let dispatch = |ready: &ReadyTokens| -> (bool, bool) {
        let (mut doorbell, mut event) = (false, false);
        for tag in ready.iter() {
            if tag >= OTHER_TAG_BASE {
                event = true;
                on_other(tag);
            } else if tag == COMPLETIONS_TAG {
                event = true;
                completions.for_each_inflight(|t| {
                    stats.host_rings.fetch_add(1, Ordering::Relaxed);
                    if plane.ring_internal(t) {
                        let _ = efd.signal();
                    }
                });
                on_completion();
            } else {
                doorbell = true;
                let _ = efd.drain();
            }
        }
        (doorbell, event)
    };
    let mut scratch = Vec::with_capacity(SCAN_LIMIT);
    let mut policy = LoopPolicy::from_env();
    let prof = stats.prof.load(Ordering::Relaxed);
    let mut busy_from = std::time::Instant::now();
    let mut after_timeout = false;
    while !stop.load(Ordering::Acquire) {
        let owed = on_tick();
        // §5.3: every `seen` load happens-before the scan.
        let seen = plane.worker_wake().seen();
        let n = plane.worker_pass(host, &mut scratch, SCAN_LIMIT);
        if n > 0 {
            stats.served.fetch_add(n as u64, Ordering::Relaxed);
            if after_timeout {
                stats.timeouts_with_work.fetch_add(1, Ordering::Relaxed);
            }
            after_timeout = false;
            match policy.after_pass(n) {
                AfterPass::Continue | AfterPass::Idle => continue,
                AfterPass::Look => {
                    // ★ BOUNDED FAIRNESS: a busy worker (a hot relay rings its own token after every turn; two workers can hand one
                    // hot token to each other) must still look at the world: the engine's events (the guest's completion
                    // interrupts), the completions, and the waiting relays' private words. Same look as the idle spin's, once.
                    RelayStats::bump(&relay_stats.busy_looks);
                    let mut ready = ReadyTokens::new();
                    if matches!(poller.wait(&mut ready, PollTimeout::Immediate), Ok(k) if k > 0) {
                        let (doorbell, event) = dispatch(&ready);
                        if doorbell {
                            RelayStats::bump(&relay_stats.wake_doorbell);
                        }
                        if event {
                            RelayStats::bump(&relay_stats.wake_event);
                            let _ = relay.peek(Peek::Event);
                        }
                    }
                    if relay.waiting() && relay.peek(Peek::Spin) {
                        RelayStats::bump(&relay_stats.wake_spin);
                    }
                    continue;
                }
            }
        }
        after_timeout = false;
        // ★ The bounded SPIN: only while some relay waits for something that only a look can find,
        // and never instead of the park (a latency optimisation; the model proves liveness with 0).
        if user_loop::idle(relay.waiting(), user_loop::SPIN) == user_loop::Idle::Spin {
            let mut woke = false;
            for _ in 0..user_loop::SPIN {
                let mut ready = ReadyTokens::new();
                if matches!(poller.wait(&mut ready, PollTimeout::Immediate), Ok(n) if n > 0) {
                    let (doorbell, event) = dispatch(&ready);
                    if doorbell {
                        RelayStats::bump(&relay_stats.wake_doorbell);
                    }
                    if event {
                        RelayStats::bump(&relay_stats.wake_event);
                        let _ = relay.peek(Peek::Event);
                    }
                    woke = true;
                    break;
                }
                if relay.peek(Peek::Spin) {
                    RelayStats::bump(&relay_stats.wake_spin);
                    woke = true;
                    break;
                }
                if !relay.waiting() {
                    break;
                }
                std::hint::spin_loop();
            }
            if woke {
                continue;
            }
        }
        if !plane.worker_wake().try_park(seen) {
            continue;
        }
        stats.parks.fetch_add(1, Ordering::Relaxed);
        let mut ready = ReadyTokens::new();
        let tw = prof.then(std::time::Instant::now);
        if let Some(tw) = tw {
            let b = u64::try_from(tw.duration_since(busy_from).as_nanos()).unwrap_or(u64::MAX);
            stats.busy_ns.fetch_add(b, Ordering::Relaxed);
            stats.max_busy_ns.fetch_max(b, Ordering::Relaxed);
        }
        let park = if owed { OWED_PARK_MS } else { PARK_MS };
        let got = poller.wait(&mut ready, PollTimeout::Millis(park));
        if let Some(tw) = tw {
            busy_from = std::time::Instant::now();
            let w = u64::try_from(busy_from.duration_since(tw).as_nanos()).unwrap_or(u64::MAX);
            stats.wait_ns.fetch_add(w, Ordering::Relaxed);
            stats.waits.fetch_add(1, Ordering::Relaxed);
            after_timeout = matches!(got, Ok(0));
            if after_timeout {
                stats.timeouts.fetch_add(1, Ordering::Relaxed);
            }
        }
        plane.worker_wake().unpark();
        if got.is_ok() {
            let (doorbell, event) = dispatch(&ready);
            if doorbell {
                RelayStats::bump(&relay_stats.wake_doorbell);
            }
            if event {
                RelayStats::bump(&relay_stats.wake_event);
                let _ = relay.peek(Peek::Event);
            }
            // A timeout of the long park is the relays' safety net: look at everything once. The
            // short park of a paced raise (`owed`) is the Pacer's, not the net's.
            if matches!(got, Ok(0)) && !owed {
                RelayStats::bump(&relay_stats.wake_backstop);
                let _ = relay.peek(Peek::Backstop);
            }
        }
        on_other(TICK_TAG);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pass that served something continues; every `LOOK_EVERY`-th consecutive busy pass looks first; an empty pass idles and
    /// restarts the count.
    #[test]
    fn a_busy_worker_looks_every_look_every_passes_and_an_empty_pass_idles() {
        let mut p = LoopPolicy::default();
        for round in 0..3 {
            for i in 1..LOOK_EVERY {
                assert_eq!(
                    p.after_pass(3),
                    AfterPass::Continue,
                    "round {round} pass {i}"
                );
            }
            assert_eq!(p.after_pass(3), AfterPass::Look, "round {round}");
        }
        assert_eq!(p.after_pass(1), AfterPass::Continue);
        assert_eq!(p.after_pass(0), AfterPass::Idle);
        for _ in 1..LOOK_EVERY {
            assert_eq!(p.after_pass(1), AfterPass::Continue, "the count restarted");
        }
        assert_eq!(p.after_pass(1), AfterPass::Look);
    }

    /// The default IS the fix: the byte-identical behaviour of the fairness branch.
    #[test]
    fn the_default_policy_is_the_fix() {
        assert_eq!(LoopPolicy::default(), LoopPolicy::from_setting(false));
        let mut p = LoopPolicy::default();
        for _ in 1..LOOK_EVERY {
            assert_eq!(p.after_pass(1), AfterPass::Continue);
        }
        assert_eq!(p.after_pass(1), AfterPass::Look);
    }

    /// The control never looks, however long the worker stays busy; an empty pass still idles (the park is not the control's business).
    #[test]
    fn the_negative_control_never_looks_however_busy_the_worker() {
        let mut p = LoopPolicy::without_busy_look();
        for i in 0..10_000 {
            assert_eq!(p.after_pass(1 + i % 7), AfterPass::Continue, "pass {i}");
        }
        assert_eq!(p.after_pass(0), AfterPass::Idle);
        for _ in 0..(10 * LOOK_EVERY) {
            assert_eq!(p.after_pass(3), AfterPass::Continue);
        }
    }

    /// The control's state is bounded (the model hashes it): it does not count passes.
    #[test]
    fn the_negative_controls_state_does_not_grow() {
        let mut p = LoopPolicy::without_busy_look();
        let before = p.clone();
        for _ in 0..1000 {
            p.after_pass(1);
        }
        assert_eq!(p, before);
    }

    /// The two policies differ in exactly one answer.
    #[test]
    fn the_control_and_the_fix_differ_only_in_the_look() {
        let (mut fix, mut ctl) = (LoopPolicy::default(), LoopPolicy::without_busy_look());
        let mut looks = 0;
        for i in 0..200usize {
            let served = [0usize, 1, 1, 1, 1, 2, 3][i % 7];
            let (a, b) = (fix.after_pass(served), ctl.after_pass(served));
            if a == AfterPass::Look {
                looks += 1;
                assert_eq!(b, AfterPass::Continue);
            } else {
                assert_eq!(a, b, "pass {i}");
            }
        }
        assert!(looks > 0);
    }

    /// OFF unless the value is exactly `1`.
    #[test]
    fn the_negative_control_is_off_unless_exactly_one() {
        assert!(!parse_negctl_no_busy_look(None));
        for off in ["", "0", "true", "yes", "2", " 1", "1 ", "01", "on"] {
            assert!(!parse_negctl_no_busy_look(Some(off)), "{off:?}");
        }
        assert!(parse_negctl_no_busy_look(Some("1")));
    }

    /// The environment is read in one place, once, and the name is the repo's control class.
    #[test]
    fn the_negative_control_is_read_once_from_a_negctl_name() {
        let src = include_str!("worker.rs");
        let prod = src.split("#[cfg(test)]").next().unwrap();
        assert_eq!(NEGCTL_NO_BUSY_LOOK, "KF3_NEGCTL_NO_BUSY_LOOK");
        assert_eq!(
            prod.matches("std::env::var").count(),
            1,
            "one read of the environment in worker.rs"
        );
        assert!(prod.contains("OnceLock"), "read once");
        assert!(
            prod.contains("LoopPolicy::from_env()"),
            "the loop asks for its policy once, outside the loop"
        );
        assert_eq!(prod.matches("LoopPolicy::from_env()").count(), 1);
    }
}
