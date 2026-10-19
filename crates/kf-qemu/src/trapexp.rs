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
use kf_trap::trapexp::{Delays, Regs, SpinState, parse_delay_us};

/// The parsed flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Cfg {
    /// Injected delays.
    pub delays: Delays,
    /// The interrupt-page spinner.
    pub spin: bool,
    /// The core the spinner pins itself to.
    pub spin_cpu: Option<usize>,
}

impl Cfg {
    /// Read the flags through `get` (the environment, or a table in a test).
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Cfg {
        Cfg {
            delays: Delays {
                intr_us: parse_delay_us(get("KF3_DIAG_TRAP_DELAY_INTR_US").as_deref()),
                db_us: parse_delay_us(get("KF3_DIAG_TRAP_DELAY_DB_US").as_deref()),
            },
            spin: get("KF3_DIAG_INTR_SPIN").is_some_and(|v| v.trim() == "1"),
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
            " trapexp[delay_intr_us={} delay_db_us={} intr_delayed={} db_delayed={} spin={} polls={} stored={} lost={} trigger_raises={} worker_raises={} oor={}{mask}]",
            cfg.delays.intr_us,
            cfg.delays.db_us,
            self.intr_delayed.load(o),
            self.db_delayed.load(o),
            u8::from(cfg.spin),
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
        for (v, want) in [("1", true), (" 1 ", true), ("0", false), ("", false), ("yes", false), ("2", false), ("true", false)] {
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
}
