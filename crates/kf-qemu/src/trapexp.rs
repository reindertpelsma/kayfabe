//! ★ **MEASUREMENT ONLY — feature `diag-trap-exp`, absent from every production build.**
//!
//! `STATUS: RESEARCH, 2026-10-19.` The `kf-qemu` half of the trap experiments
//! (`traces/trap_experiments_20261019/README.md`): the runtime flags, the busy-wait, one spinner
//! poll over a page and the counters. The pure rules are `kf_trap::trapexp`.
//!
//! Flags (all default off, none required to boot):
//! - `KF3_DIAG_TRAP_DELAY_INTR_US=N`, `KF3_DIAG_TRAP_DELAY_DB_US=N` — extra microseconds in the
//!   trapped interrupt-tree / doorbell write (busy wait on a monotonic clock, no sleep);
//! - `KF3_DIAG_INTR_SPIN=1` — the interrupt page is plain RAM and one host thread spins on it
//!   (deliberately breaks the no-polling rule); `KF3_DIAG_INTR_SPIN_CPU=N` pins that thread.

use core::sync::atomic::{AtomicU64, Ordering};
use kf_trap::cpuintr::{CpuIntr, Raise};
use kf_trap::trapexp::{Delays, INTR_PAGE, Regs, SpinState, observed_writes, parse_delay_us};

/// The parsed flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cfg {
    /// Injected delays.
    pub delays: Delays,
    /// The interrupt-page spinner (`KF3_DIAG_INTR_SPIN=1` forward-everything, `=2` model-driven).
    pub spin: bool,
    /// Variant B (`KF3_DIAG_INTR_SPIN=2`): the spinner applies each guest store to the real
    /// interrupt model exactly as a trap would and publishes the model's answer.
    pub model: bool,
    /// Variant B: microseconds a pending bit is held before it acks itself
    /// (`KF3_DIAG_INTR_SPIN_HOLD_US`, default 300).
    pub hold_us: u32,
    /// The core the spinner pins itself to.
    pub spin_cpu: Option<usize>,
}

impl Default for Cfg {
    fn default() -> Cfg {
        Cfg {
            delays: Delays::default(),
            spin: false,
            model: false,
            hold_us: 300,
            spin_cpu: None,
        }
    }
}

impl Cfg {
    /// Read the flags through `get` (the environment, or a table in a test).
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Cfg {
        Cfg {
            delays: Delays {
                intr_us: parse_delay_us(get("KF3_DIAG_TRAP_DELAY_INTR_US").as_deref()),
                db_us: parse_delay_us(get("KF3_DIAG_TRAP_DELAY_DB_US").as_deref()),
            },
            spin: get("KF3_DIAG_INTR_SPIN").is_some_and(|v| matches!(v.trim(), "1" | "2")),
            model: get("KF3_DIAG_INTR_SPIN").is_some_and(|v| v.trim() == "2"),
            hold_us: get("KF3_DIAG_INTR_SPIN_HOLD_US")
                .and_then(|v| v.trim().parse::<u32>().ok())
                .map_or(300, |v| v.min(100_000)),
            spin_cpu: get("KF3_DIAG_INTR_SPIN_CPU").and_then(|v| v.trim().parse().ok()),
        }
    }

    /// From the process environment.
    #[must_use]
    pub fn from_env() -> Cfg {
        Cfg::from_lookup(|k| std::env::var(k).ok())
    }

    /// Is any experiment on?
    #[must_use]
    pub fn any(&self) -> bool {
        self.spin || self.delays.intr_us != 0 || self.delays.db_us != 0
    }
}

/// Spin on a monotonic clock for `us` microseconds (no sleep, no yield).
pub fn busy_wait_us(us: u32) {
    if us == 0 {
        return;
    }
    let t = std::time::Instant::now();
    let want = std::time::Duration::from_micros(u64::from(us));
    while t.elapsed() < want {
        core::hint::spin_loop();
    }
}

/// What one poll of the page did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Poll {
    /// Write the irqfd once (a trigger word was seen). ⊘ Only after the page stores below.
    pub raise: bool,
    /// Words stored.
    pub stored: u32,
    /// Words whose compare-and-swap lost to a guest store (healed by the next poll).
    pub lost: u32,
}

/// ONE poll: read the owned words with `rd(page_offset)`, run the step, store each changed word
/// with `cas(page_offset, old, new)` (false = the guest stored in between).
pub fn poll_once(
    st: &mut SpinState,
    rd: &mut dyn FnMut(usize) -> u32,
    cas: &mut dyn FnMut(usize, u32, u32) -> bool,
) -> Poll {
    let n = st.n_leaf();
    let cur = Regs::load(n, rd);
    let (new, raise) = st.step(&cur);
    let mut out = Poll {
        raise,
        ..Poll::default()
    };
    cur.changes(&new, n, |off, old, nw| {
        if cas(off, old, nw) {
            out.stored += 1;
        } else {
            out.lost += 1;
        }
    });
    out
}

/// What one variant-B poll did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ModelPoll {
    /// Guest stores applied to the model.
    pub writes: u32,
    /// Irqfd writes the model asked for (after the page was published).
    pub raises: u32,
    /// Words published.
    pub stored: u32,
    /// Compare-and-swaps lost to a guest store (the next poll sees that store).
    pub lost: u32,
}

/// When each pending leaf bit was last raised (ns on a monotonic clock; 0 = not pending).
#[derive(Debug, Clone)]
pub struct Ages {
    t: [[u64; 32]; kf_trap::cpuintr::MAX_LEAF],
}

impl Default for Ages {
    fn default() -> Ages {
        Ages {
            t: [[0; 32]; kf_trap::cpuintr::MAX_LEAF],
        }
    }
}

impl Ages {
    /// A worker raised `vector` at `now` (a re-edge on a pending bit restarts its hold).
    pub fn touch(&mut self, vector: u32, now: u64) {
        let (i, b) = ((vector / 32) as usize, (vector % 32) as usize);
        if i < self.t.len() {
            self.t[i][b] = now.max(1);
        }
    }

    /// Given the model's pending words, return per leaf the bits whose hold has run out at `now`
    /// (age >= `hold`), and forget bits that are no longer pending. Newly seen bits start their hold.
    pub fn expire(&mut self, pending: &[u32], now: u64, hold: u64) -> Vec<u32> {
        let mut out = vec![0u32; pending.len()];
        for (i, &w) in pending.iter().enumerate().take(self.t.len()) {
            for b in 0..32 {
                let slot = &mut self.t[i][b];
                if w & (1 << b) == 0 {
                    *slot = 0;
                } else if *slot == 0 {
                    *slot = now.max(1);
                } else if now.saturating_sub(*slot) >= hold {
                    out[i] |= 1 << b;
                    *slot = 0;
                }
            }
        }
        out
    }
}

/// Variant B's state: the words last published, and the pending bits' ages.
#[derive(Debug, Clone, Default)]
pub struct SpinB {
    /// The page as the spinner last left it.
    pub last: Regs,
    /// Pending-bit ages.
    pub ages: Ages,
}

/// Publish the model's words into the page: `put` is `(BAR0 offset, value)` from `CpuIntr::shadow*`.
fn publish(
    n: usize,
    last: &mut Regs,
    cas: &mut dyn FnMut(usize, u32, u32) -> bool,
    out: &mut ModelPoll,
    off_abs: u64,
    v: u32,
) {
    let off = (off_abs - INTR_PAGE) as usize;
    // EN_CLEAR is write-only in practice: its page word stays 0, so that a store of the enabled
    // mask (the usual "disable all") changes the word and is SEEN.
    let v = if (0x400..0x600).contains(&off) { 0 } else { v };
    let Some(old) = last.get(off, n) else { return };
    if old == v {
        return;
    }
    if cas(off, old, v) {
        last.set(off, n, v);
        out.stored += 1;
    } else {
        out.lost += 1;
    }
}

/// ★ Variant B, ONE poll: every word the guest changed since the last publication is applied to the
/// model exactly as a trapped write would be (`CpuIntr::write`), then the model's read-back is
/// published with a compare-and-swap. ⊘ A W1C ack that stores the very value the word holds changes
/// NOTHING in plain RAM and cannot be seen, so a pending bit is also acked by itself after `hold_ns`
/// (the documented approximation of this variant). The caller writes the irqfd `raises` times AFTER
/// this returns.
pub fn model_poll(
    intr: &CpuIntr,
    n_leaf: usize,
    st: &mut SpinB,
    rd: &mut dyn FnMut(usize) -> u32,
    cas: &mut dyn FnMut(usize, u32, u32) -> bool,
    now_ns: u64,
    hold_ns: u64,
) -> ModelPoll {
    let cur = Regs::load(n_leaf, rd);
    let mut out = ModelPoll::default();
    let mut apply = |st: &mut SpinB, r: kf_trap::cpuintr::Reg, val: u32, out: &mut ModelPoll| {
        out.writes += 1;
        if intr.write(r, val) == Raise::Message {
            out.raises += 1;
        }
        let mut puts = Vec::new();
        intr.shadow(r, |o, v| puts.push((o, v)));
        for (o, v) in puts {
            publish(n_leaf, &mut st.last, cas, out, o, v);
        }
    };
    for (off, val) in observed_writes(n_leaf, &st.last, &cur) {
        st.last.set(off, n_leaf, val); // adopt: the page holds the guest's word
        let Some(r) = intr.decode(INTR_PAGE + off as u64) else {
            continue;
        };
        apply(st, r, val, &mut out);
    }
    if hold_ns != 0 {
        let pending: Vec<u32> = (0..n_leaf)
            .map(|i| intr.read(kf_trap::cpuintr::Reg::Leaf(i)))
            .collect();
        for (i, bits) in st.ages.expire(&pending, now_ns, hold_ns).into_iter().enumerate() {
            if bits != 0 {
                apply(st, kf_trap::cpuintr::Reg::Leaf(i), bits, &mut out);
            }
        }
    }
    out
}

/// Variant B, a worker latched a completion edge in the model: publish every word (no guest write).
pub fn model_publish_all(
    intr: &CpuIntr,
    n_leaf: usize,
    st: &mut SpinB,
    cas: &mut dyn FnMut(usize, u32, u32) -> bool,
) -> ModelPoll {
    let mut out = ModelPoll::default();
    let mut puts = Vec::new();
    intr.shadow_all(|o, v| puts.push((o, v)));
    for (o, v) in puts {
        publish(n_leaf, &mut st.last, cas, &mut out, o, v);
    }
    out
}

/// Experiment counters (any thread; relaxed).
#[derive(Debug, Default)]
pub struct Counters {
    /// Interrupt-page writes that were delayed.
    pub intr_delayed: AtomicU64,
    /// Doorbell writes that were delayed.
    pub db_delayed: AtomicU64,
    /// Spinner polls.
    pub polls: AtomicU64,
    /// Words the spinner stored.
    pub stored: AtomicU64,
    /// Compare-and-swaps that lost to a guest store.
    pub lost: AtomicU64,
    /// Irqfd writes caused by a trigger word.
    pub trigger_raises: AtomicU64,
    /// Variant B: guest stores applied to the model by the spinner.
    pub model_writes: AtomicU64,
    /// Irqfd writes caused by a worker completion (unconditional under the spinner).
    pub worker_raises: AtomicU64,
    /// Completion edges whose vector was out of the family's range.
    pub out_of_range: AtomicU64,
}

impl Counters {
    /// One log fragment (` trapexp[...]`), or `""` when `cfg` has no experiment on — so a build
    /// with the feature but no flag prints the line it always printed.
    #[must_use]
    pub fn status(&self, cfg: &Cfg, st: Option<&SpinState>) -> String {
        if !cfg.any() {
            return String::new();
        }
        let o = Ordering::Relaxed;
        let mask = st.map_or(String::new(), |s| {
            let v: Vec<String> = (0..s.n_leaf())
                .filter(|&i| s.mask(i) != 0)
                .map(|i| format!("{i}:{:#x}", s.mask(i)))
                .collect();
            format!(" mask=[{}]", v.join(","))
        });
        format!(
            " trapexp[delay_intr_us={} delay_db_us={} intr_delayed={} db_delayed={} spin={} model={} model_writes={} polls={} stored={} lost={} trigger_raises={} worker_raises={} oor={}{mask}]",
            cfg.delays.intr_us,
            cfg.delays.db_us,
            self.intr_delayed.load(o),
            self.db_delayed.load(o),
            u8::from(cfg.spin),
            u8::from(cfg.model),
            self.model_writes.load(o),
            self.polls.load(o),
            self.stored.load(o),
            self.lost.load(o),
            self.trigger_raises.load(o),
            self.worker_raises.load(o),
            self.out_of_range.load(o),
        )
    }
}

#[allow(dead_code)]
fn _regs_type_is_used(_: &Regs) {}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::AtomicU32;
    use kf_chip::Family;

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| (*v).to_string())
    }

    #[test]
    fn every_flag_defaults_off() {
        let c = Cfg::from_lookup(env(&[]));
        assert_eq!(c, Cfg::default());
        assert!(!c.any());
        assert_eq!(c.delays, Delays::default());
        assert!(!c.spin);
        assert_eq!(c.spin_cpu, None);
    }

    #[test]
    fn the_delay_flags_parse_independently() {
        let c = Cfg::from_lookup(env(&[("KF3_DIAG_TRAP_DELAY_INTR_US", "20")]));
        assert_eq!(c.delays, Delays { intr_us: 20, db_us: 0 });
        assert!(c.any());
        let c = Cfg::from_lookup(env(&[("KF3_DIAG_TRAP_DELAY_DB_US", "10")]));
        assert_eq!(c.delays, Delays { intr_us: 0, db_us: 10 });
        assert!(c.any());
        let c = Cfg::from_lookup(env(&[("KF3_DIAG_TRAP_DELAY_INTR_US", "5000")]));
        assert_eq!(c.delays.intr_us, kf_trap::trapexp::MAX_DELAY_US, "capped");
    }

    #[test]
    fn spin_is_on_only_for_exactly_one() {
        for (v, want) in [("1", true), (" 1 ", true), ("0", false), ("", false), ("yes", false), ("2", true), ("true", false)] {
            let c = Cfg::from_lookup(move |k| (k == "KF3_DIAG_INTR_SPIN").then(|| v.to_string()));
            assert_eq!(c.spin, want, "{v:?}");
            assert_eq!(c.any(), want, "{v:?}");
        }
    }

    #[test]
    fn spin_cpu_parses_a_number_and_nothing_else() {
        let c = Cfg::from_lookup(env(&[("KF3_DIAG_INTR_SPIN_CPU", "23")]));
        assert_eq!(c.spin_cpu, Some(23));
        assert!(!c.any(), "a cpu alone turns nothing on");
        for v in ["", "x", "-1", "1.5"] {
            let c = Cfg::from_lookup(move |k| (k == "KF3_DIAG_INTR_SPIN_CPU").then(|| v.to_string()));
            assert_eq!(c.spin_cpu, None, "{v:?}");
        }
    }

    #[test]
    fn busy_wait_waits_at_least_the_time_and_zero_returns_at_once() {
        let t = std::time::Instant::now();
        busy_wait_us(300);
        assert!(t.elapsed() >= std::time::Duration::from_micros(300));
        assert!(t.elapsed() < std::time::Duration::from_millis(50));
        let t = std::time::Instant::now();
        busy_wait_us(0);
        assert!(t.elapsed() < std::time::Duration::from_micros(200));
    }

    /// A page of RAM words: `rd`, `cas` and a guest-store helper.
    struct Page(Vec<AtomicU32>);
    impl Page {
        fn new() -> Page {
            Page((0..1024).map(|_| AtomicU32::new(0)).collect())
        }
        fn guest(&self, byte_off: usize, v: u32) {
            self.0[byte_off / 4].store(v, Ordering::SeqCst);
        }
        fn word(&self, byte_off: usize) -> u32 {
            self.0[byte_off / 4].load(Ordering::SeqCst)
        }
        fn poll(&self, st: &mut SpinState) -> Poll {
            poll_once(
                st,
                &mut |o| self.0[o / 4].load(Ordering::SeqCst),
                &mut |o, old, new| {
                    self.0[o / 4]
                        .compare_exchange(old, new, Ordering::SeqCst, Ordering::SeqCst)
                        .is_ok()
                },
            )
        }
    }

    const LEAF: usize = 0x000;
    const EN_SET: usize = 0x200;
    const EN_CLEAR: usize = 0x400;
    const TOP: usize = 0x600;
    const TOP_EN_SET: usize = 0x608;
    const TRIGGER: usize = 0x640;

    #[test]
    fn an_idle_page_is_polled_without_a_store_or_a_raise() {
        let page = Page::new();
        let mut st = SpinState::new(Family::Ampere);
        for _ in 0..10 {
            assert_eq!(page.poll(&mut st), Poll::default());
        }
    }

    #[test]
    fn the_loopback_self_test_end_to_end_on_a_page() {
        let page = Page::new();
        let mut st = SpinState::new(Family::Ampere);
        // clear 129, enable it, enable its subtree, trigger it (all plain guest stores)
        page.guest(LEAF + 16, 1 << 1);
        assert_eq!(page.poll(&mut st), Poll { raise: false, stored: 1, lost: 0 });
        assert_eq!(page.word(LEAF + 16), 0, "a clear of a never-enabled vector leaves nothing pending");
        page.guest(EN_SET + 16, 1 << 1);
        page.guest(TOP_EN_SET, 1 << 2);
        page.guest(TRIGGER, 129);
        let p = page.poll(&mut st);
        assert!(p.raise);
        assert_eq!(p.lost, 0);
        // ⊘ the page already shows the vector pending when the raise is reported
        assert_eq!(page.word(LEAF + 16), 1 << 1);
        assert_eq!(page.word(TOP), 1 << 2);
        assert_eq!(page.word(TRIGGER), 0);
        // ISR acks: restored; no raise
        page.guest(LEAF + 16, 1 << 1);
        let p = page.poll(&mut st);
        assert!(!p.raise);
        assert_eq!(page.word(LEAF + 16), 1 << 1);
        // EN_CLEAR is ignored
        page.guest(EN_CLEAR + 16, 0xFFFF_FFFF);
        page.poll(&mut st);
        assert_eq!(page.word(EN_CLEAR + 16), 1 << 1);
        assert_eq!(page.word(LEAF + 16), 1 << 1);
    }

    #[test]
    fn a_lost_compare_and_swap_is_counted_and_healed_by_the_next_poll() {
        let page = Page::new();
        let mut st = SpinState::new(Family::Ampere);
        page.guest(EN_SET, 0b11);
        // the guest stores again between our load and our store: every cas loses
        let p = poll_once(
            &mut st,
            &mut |o| page.0[o / 4].load(Ordering::SeqCst),
            &mut |_, _, _| false,
        );
        assert!(p.lost > 0);
        assert_eq!(p.stored, 0);
        assert_eq!(page.word(LEAF), 0, "nothing was stored");
        // the mask was learned from the snapshot; the next honest poll repairs the page
        let p = page.poll(&mut st);
        assert_eq!(p.lost, 0);
        assert!(p.stored > 0);
        assert_eq!(page.word(LEAF), 0b11);
        assert_eq!(page.word(EN_SET), 0b11);
    }

    #[test]
    fn a_trigger_whose_clear_lost_still_raises_exactly_once_per_poll_that_saw_it() {
        let page = Page::new();
        let mut st = SpinState::new(Family::Ampere);
        page.guest(TRIGGER, 5);
        let p = poll_once(
            &mut st,
            &mut |o| page.0[o / 4].load(Ordering::SeqCst),
            &mut |_, _, _| false,
        );
        assert!(p.raise);
        assert_eq!(page.word(TRIGGER), 5, "the clear lost");
        let p = page.poll(&mut st);
        assert!(p.raise, "seen again, raised again; never silently dropped");
        assert_eq!(page.word(TRIGGER), 0);
        assert!(!page.poll(&mut st).raise);
    }

    #[test]
    fn stored_counts_exactly_the_changed_words() {
        let page = Page::new();
        let mut st = SpinState::new(Family::Ampere);
        page.guest(EN_SET + 4, 1); // leaf 1: stores LEAF[1], EN_SET[1] (already), EN_CLEAR[1], TOP
        let p = page.poll(&mut st);
        assert_eq!(p.stored, 3, "leaf, en_clear, top");
        assert_eq!(page.word(TOP), 1);
        assert_eq!(page.poll(&mut st).stored, 0);
        let _ = (EN_CLEAR, TOP_EN_SET);
    }

    #[test]
    fn the_status_fragment_is_empty_with_no_flag_and_names_the_counters_otherwise() {
        let c = Counters::default();
        assert_eq!(c.status(&Cfg::default(), None), "");
        c.polls.fetch_add(7, Ordering::Relaxed);
        c.trigger_raises.fetch_add(2, Ordering::Relaxed);
        c.worker_raises.fetch_add(3, Ordering::Relaxed);
        c.intr_delayed.fetch_add(4, Ordering::Relaxed);
        c.db_delayed.fetch_add(5, Ordering::Relaxed);
        let cfg = Cfg {
            spin: true,
            model: false,
            hold_us: 300,
            delays: Delays { intr_us: 10, db_us: 20 },
            spin_cpu: None,
        };
        let mut st = SpinState::new(Family::Ampere);
        let mut r = Regs::default();
        r.en_set[4] = 0b10;
        let _ = st.step(&r);
        let s = c.status(&cfg, Some(&st));
        for want in [
            "trapexp[", "delay_intr_us=10", "delay_db_us=20", "intr_delayed=4", "db_delayed=5",
            "spin=1", "polls=7", "trigger_raises=2", "worker_raises=3", "mask=", "4:0x2",
        ] {
            assert!(s.contains(want), "{s} lacks {want}");
        }
        let only_delay = Cfg {
            delays: Delays { intr_us: 10, db_us: 0 },
            ..Cfg::default()
        };
        let s = c.status(&only_delay, None);
        assert!(s.contains("delay_intr_us=10") && s.contains("spin=0") && !s.contains("mask="), "{s}");
    }
    #[test]
    fn the_intr_delay_flag_alone() {
        let c = Cfg::from_lookup(env(&[("KF3_DIAG_TRAP_DELAY_INTR_US", "10")]));
        assert_eq!((c.delays.intr_us, c.delays.db_us), (10, 0));
    }

    #[test]
    fn the_db_delay_flag_alone() {
        let c = Cfg::from_lookup(env(&[("KF3_DIAG_TRAP_DELAY_DB_US", "20")]));
        assert_eq!((c.delays.intr_us, c.delays.db_us), (0, 20));
    }

    #[test]
    fn spin_stays_off_for_other_truthy_words_and_a_delay_flag_does_not_turn_it_on() {
        for v in ["true", "on", "yes", "11", "01x", "enable"] {
            let c = Cfg::from_lookup(move |k| (k == "KF3_DIAG_INTR_SPIN").then(|| v.to_string()));
            assert!(!c.spin && !c.any(), "{v:?}");
        }
        let c = Cfg::from_lookup(env(&[("KF3_DIAG_TRAP_DELAY_DB_US", "1")]));
        assert!(!c.spin);
    }

    #[test]
    fn spin_cpu_garbage_is_none_not_zero() {
        for v in ["abc", "-3", "2.5", "0x3", ""] {
            let c = Cfg::from_lookup(move |k| (k == "KF3_DIAG_INTR_SPIN_CPU").then(|| v.to_string()));
            assert_eq!(c.spin_cpu, None, "{v:?}");
        }
        let c = Cfg::from_lookup(env(&[("KF3_DIAG_INTR_SPIN_CPU", "0")]));
        assert_eq!(c.spin_cpu, Some(0), "an explicit 0 is core 0");
    }

    #[test]
    fn busy_wait_zero_returns_at_once() {
        let t = std::time::Instant::now();
        for _ in 0..1000 {
            busy_wait_us(0);
        }
        assert!(t.elapsed() < std::time::Duration::from_millis(5));
    }

    #[test]
    fn busy_wait_is_never_short() {
        for us in [1u32, 50, 400, 2000] {
            let t = std::time::Instant::now();
            busy_wait_us(us);
            assert!(t.elapsed() >= std::time::Duration::from_micros(u64::from(us)), "{us}");
        }
    }

    #[test]
    fn busy_wait_scales_with_the_request() {
        let t = std::time::Instant::now();
        for _ in 0..5 {
            busy_wait_us(1000);
        }
        assert!(t.elapsed() >= std::time::Duration::from_millis(5));
    }

    #[test]
    fn a_partial_cas_loss_splits_stored_and_lost() {
        let page = Page::new();
        let mut st = SpinState::new(Family::Ampere);
        page.guest(EN_SET + 4, 1);
        let mut calls = 0;
        let p = poll_once(
            &mut st,
            &mut |o| page.0[o / 4].load(Ordering::SeqCst),
            &mut |o, old, new| {
                calls += 1;
                calls != 1
                    && page.0[o / 4]
                        .compare_exchange(old, new, Ordering::SeqCst, Ordering::SeqCst)
                        .is_ok()
            },
        );
        assert_eq!((p.stored, p.lost), (2, 1));
    }

    #[test]
    fn the_status_is_empty_when_nothing_is_on_even_with_counters_and_state() {
        let c = Counters::default();
        c.polls.fetch_add(100, Ordering::Relaxed);
        let mut st = SpinState::new(Family::Ampere);
        let mut r = Regs::default();
        r.en_set[0] = 1;
        let _ = st.step(&r);
        assert_eq!(c.status(&Cfg::default(), Some(&st)), "");
        let only_cpu = Cfg { spin_cpu: Some(3), ..Cfg::default() };
        assert_eq!(c.status(&only_cpu, Some(&st)), "");
    }

    #[test]
    fn the_status_lists_every_learned_leaf() {
        let c = Counters::default();
        let cfg = Cfg { spin: true, ..Cfg::default() };
        let mut st = SpinState::new(Family::Hopper);
        let mut r = Regs::default();
        r.en_set[1] = 0x10;
        r.en_set[15] = 0x3;
        let _ = st.step(&r);
        let s = c.status(&cfg, Some(&st));
        assert!(s.contains("1:0x10") && s.contains("15:0x3"), "{s}");
        assert!(!s.contains("0:0x"), "{s}");
    }

    #[test]
    fn spin_two_is_the_model_variant_and_one_is_forward() {
        let c = Cfg::from_lookup(env(&[("KF3_DIAG_INTR_SPIN", "2")]));
        assert!(c.spin && c.model && c.any());
        let c = Cfg::from_lookup(env(&[("KF3_DIAG_INTR_SPIN", "1")]));
        assert!(c.spin && !c.model);
        for v in ["3", "B", "0", ""] {
            let c = Cfg::from_lookup(move |k| (k == "KF3_DIAG_INTR_SPIN").then(|| v.to_string()));
            assert!(!c.spin && !c.model, "{v:?}");
        }
    }

    /// A page for variant B: words + state, driven like the device does (a fake clock).
    struct MPage {
        p: Page,
        st: SpinB,
        intr: CpuIntr,
        now: u64,
        hold: u64,
    }
    impl MPage {
        fn new(f: Family) -> MPage {
            MPage { p: Page::new(), st: SpinB::default(), intr: CpuIntr::new(f, 0xBB_0000).unwrap(), now: 1_000, hold: 0 }
        }
        fn poll(&mut self) -> ModelPoll {
            let p = &self.p;
            model_poll(
                &self.intr,
                8,
                &mut self.st,
                &mut |o| p.0[o / 4].load(Ordering::SeqCst),
                &mut |o, old, new| p.0[o / 4].compare_exchange(old, new, Ordering::SeqCst, Ordering::SeqCst).is_ok(),
                self.now,
                self.hold,
            )
        }
        fn worker_latch(&mut self, v: u32) -> Raise {
            let r = self.intr.latch(v);
            self.st.ages.touch(v, self.now);
            let p = &self.p;
            model_publish_all(&self.intr, 8, &mut self.st, &mut |o, old, new| {
                p.0[o / 4].compare_exchange(old, new, Ordering::SeqCst, Ordering::SeqCst).is_ok()
            });
            r
        }
    }

    #[test]
    fn model_variant_runs_the_driver_loopback_like_a_trap_would() {
        let mut m = MPage::new(Family::Ampere);
        m.p.guest(LEAF + 16, 1 << 1); // clear (nothing pending: a visible change)
        m.p.guest(EN_SET + 16, 1 << 1);
        m.p.guest(TOP_EN_SET, 1 << 2);
        let p = m.poll();
        assert_eq!(p.raises, 0);
        assert_eq!(m.p.word(EN_SET + 16), 1 << 1, "EN_SET reads back the enable mask");
        assert_eq!(m.p.word(LEAF + 16), 0, "the stray clear was restored to the model's answer");
        m.p.guest(TRIGGER, 129);
        let p = m.poll();
        assert_eq!(p.raises, 1, "the trigger sends one message");
        assert_eq!(m.p.word(LEAF + 16), 1 << 1, "the ISR finds it pending");
        assert_eq!(m.p.word(TOP), 1 << 2);
        assert_eq!(m.p.word(TRIGGER), 0);
    }

    #[test]
    fn model_variant_a_w1c_ack_of_the_exact_pending_value_is_invisible_until_the_hold_expires() {
        let mut m = MPage::new(Family::Ampere);
        m.hold = 300;
        m.p.guest(EN_SET + 16, 1 << 1);
        m.p.guest(TOP_EN_SET, 1 << 2);
        m.poll();
        m.p.guest(TRIGGER, 129);
        m.poll();
        assert_eq!(m.p.word(LEAF + 16), 1 << 1);
        // the ISR acks with the value it read: the word does not change, the poll sees nothing
        m.p.guest(LEAF + 16, 1 << 1);
        m.now += 100;
        let p = m.poll();
        assert_eq!(p.writes, 0, "an identical store is invisible");
        assert_eq!(m.p.word(LEAF + 16), 1 << 1, "so it is still pending inside the hold");
        m.now += 250; // hold (300 ns) has run out
        m.poll();
        assert_eq!(m.p.word(LEAF + 16), 0, "the hold acked it");
        assert_eq!(m.p.word(TOP), 0);
    }

    #[test]
    fn model_variant_pending_comes_only_from_real_edges_and_a_visible_ack_clears_only_its_bits() {
        let mut m = MPage::new(Family::Ampere);
        m.p.guest(EN_SET, 0b111);
        m.p.guest(TOP_EN_SET, 1);
        m.poll();
        assert_eq!(m.p.word(LEAF), 0, "nothing pending without an edge");
        assert_eq!(m.worker_latch(0), Raise::Message);
        assert_eq!(m.worker_latch(2), Raise::Message);
        assert_eq!(m.p.word(LEAF), 0b101, "the worker's edges are published before any raise");
        m.p.guest(LEAF, 0b001); // a store that differs from the word: visible
        m.poll();
        assert_eq!(m.p.word(LEAF), 0b100, "vector 2 stays pending");
        assert_eq!(m.p.word(TOP), 1);
    }

    #[test]
    fn model_variant_enable_clear_is_exact_its_word_reads_zero_and_a_masked_edge_waits() {
        let mut m = MPage::new(Family::Ampere);
        m.p.guest(TOP_EN_SET, 1);
        m.poll();
        assert_eq!(m.worker_latch(5), Raise::None, "masked: held");
        assert_eq!(m.p.word(LEAF), 1 << 5, "but pending in the page");
        m.p.guest(EN_SET, 1 << 5);
        assert_eq!(m.poll().raises, 1, "enabling a pending vector sends it");
        m.p.guest(EN_CLEAR, 1 << 5); // "disable all" stores the very mask: visible because EN_CLEAR reads 0
        m.poll();
        assert_eq!(m.p.word(EN_SET), 0, "EN_CLEAR really clears in this variant");
        assert_eq!(m.p.word(EN_CLEAR), 0, "and its own word is back to 0 for the next store");
        m.p.guest(EN_CLEAR, 1 << 5);
        assert_eq!(m.poll().writes, 1, "a second identical disable is seen again");
    }

    #[test]
    fn model_variant_a_lost_cas_is_adopted_on_the_next_poll() {
        let mut m = MPage::new(Family::Ampere);
        m.p.guest(EN_SET, 1);
        m.p.guest(TOP_EN_SET, 1);
        m.poll();
        m.worker_latch(0);
        m.p.guest(LEAF, 2); // a visible store (acks a bit that is not pending)
        let p = &m.p;
        let r = model_poll(&m.intr, 8, &mut m.st, &mut |o| p.0[o / 4].load(Ordering::SeqCst), &mut |_, _, _| {
            p.guest(LEAF, 4); // the guest stores again between our load and our store
            false
        }, 1_000, 0);
        assert!(r.lost > 0 && r.writes == 1);
        let r = m.poll();
        assert_eq!(m.p.word(LEAF), 1, "the model's pending bit survived and is republished");
        assert_eq!(r.raises, 0);
    }

    #[test]
    fn ages_hold_then_expire_once_and_forget_cleared_bits() {
        let mut a = Ages::default();
        let mut pend = vec![0u32; 8];
        pend[1] = 0b11;
        assert_eq!(a.expire(&pend, 100, 50), vec![0; 8], "first sight starts the hold");
        assert_eq!(a.expire(&pend, 149, 50)[1], 0, "not yet");
        assert_eq!(a.expire(&pend, 150, 50)[1], 0b11, "expired exactly at the hold");
        assert_eq!(a.expire(&pend, 151, 50)[1], 0, "a bit that expired and is still pending starts a new hold");
        pend[1] = 0b01;
        a.expire(&pend, 160, 50);
        pend[1] = 0b11;
        assert_eq!(a.expire(&pend, 300, 50)[1], 0b01, "bit 0 ran out; bit 1 was cleared in between so its hold restarted");
    }

    #[test]
    fn ages_touch_restarts_a_hold() {
        let mut a = Ages::default();
        let mut pend = vec![0u32; 8];
        pend[0] = 1;
        a.expire(&pend, 100, 50);
        a.touch(0, 140);
        assert_eq!(a.expire(&pend, 160, 50)[0], 0, "re-raised at 140: held until 190");
        assert_eq!(a.expire(&pend, 190, 50)[0], 1);
        a.touch(4000, 1); // out of range vector: ignored
    }

    #[test]
    fn hold_flag_defaults_to_300_and_is_bounded() {
        assert_eq!(Cfg::from_lookup(env(&[])).hold_us, 300);
        assert_eq!(Cfg::from_lookup(env(&[("KF3_DIAG_INTR_SPIN_HOLD_US", "50")])).hold_us, 50);
        assert_eq!(Cfg::from_lookup(env(&[("KF3_DIAG_INTR_SPIN_HOLD_US", "9999999")])).hold_us, 100_000);
        assert_eq!(Cfg::from_lookup(env(&[("KF3_DIAG_INTR_SPIN_HOLD_US", "x")])).hold_us, 300);
    }

    #[test]
    fn model_variant_ignores_words_it_does_not_own() {
        let mut m = MPage::new(Family::Ampere);
        m.p.guest(0x800, 0xDEAD);
        m.p.guest(0x604, 5);
        assert_eq!(m.poll(), ModelPoll::default());
    }
    #[test]
    fn ages_a_raise_at_time_zero_still_has_an_age() {
        let mut a = Ages::default();
        let mut pend = vec![0u32; 8];
        pend[0] = 1;
        a.touch(0, 0);
        assert_eq!(a.expire(&pend, 50, 49)[0], 1);
    }

    #[test]
    fn ages_a_bit_that_leaves_and_returns_restarts_its_hold() {
        let mut a = Ages::default();
        let mut pend = vec![0u32; 8];
        pend[3] = 1;
        a.expire(&pend, 100, 50);
        pend[3] = 0;
        a.expire(&pend, 120, 50);
        pend[3] = 1;
        assert_eq!(a.expire(&pend, 160, 50)[3], 0, "it came back at 160: held until 210");
        assert_eq!(a.expire(&pend, 209, 50)[3], 0);
        assert_eq!(a.expire(&pend, 210, 50)[3], 1);
    }

    #[test]
    fn a_worker_publish_keeps_the_en_clear_word_at_zero() {
        let mut m = MPage::new(Family::Ampere);
        m.p.guest(EN_SET, 0b110);
        m.p.guest(TOP_EN_SET, 1);
        m.poll();
        m.worker_latch(1);
        assert_eq!(m.p.word(EN_SET), 0b110);
        assert_eq!(m.p.word(EN_CLEAR), 0);
    }

    #[test]
    fn spin_one_is_never_the_model_variant() {
        let c = Cfg::from_lookup(env(&[("KF3_DIAG_INTR_SPIN", "1")]));
        assert!(!c.model);
        assert!(!Cfg::from_lookup(env(&[])).model);
    }

    #[test]
    fn the_hold_is_what_acks_an_invisible_ack_and_zero_hold_never_does() {
        let mut m = MPage::new(Family::Ampere);
        m.p.guest(EN_SET, 1);
        m.p.guest(TOP_EN_SET, 1);
        m.poll();
        m.worker_latch(0);
        for t in [10u64, 1_000, 1_000_000] {
            m.now += t;
            m.hold = 0;
            m.poll();
            assert_eq!(m.p.word(LEAF), 1, "hold 0: pending stays until a visible ack");
        }
        m.hold = 500;
        m.poll(); // first sight under this clock starts the hold
        m.now += 500;
        m.poll();
        assert_eq!(m.p.word(LEAF), 0);
    }
    #[test]
    fn a_worker_edge_at_time_zero_expires_after_its_hold() {
        let mut m = MPage::new(Family::Ampere);
        m.p.guest(EN_SET, 1);
        m.p.guest(TOP_EN_SET, 1);
        m.poll();
        m.now = 0;
        m.worker_latch(0);
        m.hold = 40;
        m.now = 40;
        m.poll();
        assert_eq!(m.p.word(LEAF), 1);
        m.now = 41;
        m.poll();
        assert_eq!(m.p.word(LEAF), 0);
    }
}
