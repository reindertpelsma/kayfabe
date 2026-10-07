//! ★ DIAGNOSTIC, default off (`KF3_BAR0_TRACE=1`): a bounded BAR0 read/write trace of the window
//! after a guest-kernel channel's `GPFIFO_SCHEDULE` (owner-approved 2026-10-07,
//! `docs/OWNER_RULINGS.md` §S, "diagnostic BAR0-read trap exception").
//!
//! kayfabe's BAR0 reads never exit (`qemu/hw/misc/kf3/kf3.c`, the shadow pieces are ROM devices in
//! ROMD mode). This diagnostic turns ROMD off for the shadow pieces — so every BAR0 read of a
//! shadowed register exits and is answered from the SAME shadow word ROMD would have shown — only
//! inside a window:
//!
//! - **opens** on the drainer, right after the reply to a Translated guest-kernel channel's
//!   `GPFIFO_SCHEDULE` is published ([`Bar0Trace::schedule_served`] arms it,
//!   [`Bar0Trace::after_publish`] opens it);
//! - **closes** on the vCPU at the guest's next RPC doorbell (the GSP command-queue head write) —
//!   so a window is a subset of "from that schedule to the next RPC", which for the run30-35 abort
//!   is the first `Free`; or when the cap is reached;
//! - **caps** the whole run at [`CAP`] recorded accesses; once reached, no window opens again.
//!
//! Each window's records (BAR0 reads with the value served, BAR0 writes — the usermode doorbell
//! included — with the value written) are printed by the drainer when the next RPC is logged, with
//! that RPC's function. When that RPC is the first `Free`, the channel's USERD (64 words) and error
//! notifier (4 words) are dumped once ([`crate::chan::ChanPlane::bar0trace_dump`]).
//!
//! ⊘ No answer changes: a trapped read returns the shadow word ROMD would have returned. Nothing is
//! forwarded and nothing reaches the host. The vCPU path is lock-free: one atomic load when the
//! flag is off, and atomics only when it is on (never I/O, never a lock).

use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering};

/// The run's cap on recorded accesses (owner, 2026-10-07: "bounded (cap 4096 accesses)").
pub const CAP: u32 = 4096;

const IDLE: u8 = 0;
/// A schedule was served on the drainer; the window opens once its reply is published.
const ARMED: u8 = 1;
/// Reads trap and every access is recorded.
const OPEN: u8 = 2;
/// The vCPU saw the next RPC doorbell (or the cap); the drainer prints and restores ROMD.
const CLOSED: u8 = 3;
/// The cap was reached and the last window printed: nothing opens again.
const SPENT: u8 = 4;

const WRITE: u64 = 1 << 40;
const VALID: u64 = 1 << 41;

/// Whether `KF3_BAR0_TRACE=1` is set (read once).
#[must_use]
pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("KF3_BAR0_TRACE").is_some_and(|v| v == "1"))
}

/// The channel whose schedule opened the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceChan {
    /// Guest client.
    pub client: u32,
    /// Guest object (the channel, or its group for a TSG schedule).
    pub object: u32,
    /// Our host token for it.
    pub host: u32,
}

/// One record: kind, offset, width, value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Access {
    /// A write (else a read).
    pub write: bool,
    /// BAR0 offset.
    pub off: u64,
    /// Access width in bytes.
    pub width: u8,
    /// The value served (read) or written.
    pub value: u64,
}

/// The diagnostic's state: a lock-free record ring for the vCPU, the rest for the drainer.
pub struct Bar0Trace {
    on: bool,
    state: AtomicU8,
    /// Records reserved (may pass [`CAP`] by the number of racing vCPUs; only `< CAP` are stored).
    used: AtomicU32,
    /// First record index of the open window.
    first: AtomicU32,
    /// Windows opened.
    windows: AtomicU32,
    meta: Box<[AtomicU64]>,
    vals: Box<[AtomicU64]>,
    /// Drainer only (never taken on a vCPU).
    chan: Mutex<Option<TraceChan>>,
    /// The USERD/notifier dump was made.
    dumped: AtomicBool,
    hook: OnceLock<crate::raw_unsafe::ReadTrapHook>,
}

impl std::fmt::Debug for Bar0Trace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Bar0Trace(on={}, state={}, used={})",
            self.on,
            self.state.load(Ordering::Relaxed),
            self.used.load(Ordering::Relaxed)
        )
    }
}

/// What the drainer prints when a window closes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Closed {
    /// The window's number (1-based).
    pub window: u32,
    /// Its channel.
    pub chan: Option<TraceChan>,
    /// Its records, in order.
    pub records: Vec<Access>,
    /// Whether the run's cap is spent.
    pub spent: bool,
}

impl Bar0Trace {
    /// The diagnostic from the environment (off: no memory beyond the struct).
    #[must_use]
    pub fn from_env() -> Bar0Trace {
        Bar0Trace::new(enabled())
    }

    /// A trace, on or off.
    #[must_use]
    pub fn new(on: bool) -> Bar0Trace {
        let n = if on { CAP as usize } else { 0 };
        Bar0Trace {
            on,
            state: AtomicU8::new(IDLE),
            used: AtomicU32::new(0),
            first: AtomicU32::new(0),
            windows: AtomicU32::new(0),
            meta: (0..n).map(|_| AtomicU64::new(0)).collect(),
            vals: (0..n).map(|_| AtomicU64::new(0)).collect(),
            chan: Mutex::new(None),
            dumped: AtomicBool::new(false),
            hook: OnceLock::new(),
        }
    }

    /// Whether the flag is on.
    #[must_use]
    pub fn on(&self) -> bool {
        self.on
    }

    /// The C device's ROMD verb (registered once at realize; ignored when the flag is off).
    pub fn set_hook(&self, h: crate::raw_unsafe::ReadTrapHook) -> bool {
        self.hook.set(h).is_ok()
    }

    fn trap(&self, on: bool) {
        if let Some(h) = self.hook.get() {
            h.request(on);
        }
    }

    /// ★ Drainer, inside the served chain: a Translated guest-kernel channel's `GPFIFO_SCHEDULE`
    /// (enable) was answered. Arms the window; it opens once the reply is published.
    pub fn schedule_served(&self, c: TraceChan) {
        if !self.on {
            return;
        }
        if self
            .state
            .compare_exchange(IDLE, ARMED, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
            && let Ok(mut g) = self.chan.lock()
        {
            *g = Some(c);
        }
    }

    /// ★ Drainer, after `publish`: an armed window opens — ROMD off is requested now.
    pub fn after_publish(&self) {
        if !self.on || self.state.load(Ordering::Acquire) != ARMED {
            return;
        }
        self.first
            .store(self.used.load(Ordering::Acquire), Ordering::Release);
        self.windows.fetch_add(1, Ordering::AcqRel);
        self.state.store(OPEN, Ordering::Release);
        self.trap(true);
    }

    /// ★ THE vCPU PATH (read exit or write trap): record one access while a window is open.
    /// `qhead`: the access is the GSP command-queue head write — the next RPC — which closes it.
    /// Lock-free: atomics only.
    pub fn note(&self, a: Access, qhead: bool) {
        if !self.on || self.state.load(Ordering::Acquire) != OPEN {
            return;
        }
        let i = self.used.fetch_add(1, Ordering::AcqRel);
        if i >= CAP {
            let _ = self
                .state
                .compare_exchange(OPEN, CLOSED, Ordering::AcqRel, Ordering::Acquire);
            return;
        }
        let i = i as usize;
        self.vals[i].store(a.value, Ordering::Relaxed);
        let meta = VALID
            | if a.write { WRITE } else { 0 }
            | (u64::from(a.width) << 32)
            | (a.off & 0xffff_ffff);
        self.meta[i].store(meta, Ordering::Release);
        if qhead {
            let _ = self
                .state
                .compare_exchange(OPEN, CLOSED, Ordering::AcqRel, Ordering::Acquire);
        }
    }

    /// ★ Drainer, for each RPC it logs: a closed window is taken (ROMD back on), returned for
    /// printing with the RPC that closed it. `None` otherwise.
    pub fn take_closed(&self) -> Option<Closed> {
        if !self.on || self.state.load(Ordering::Acquire) != CLOSED {
            return None;
        }
        self.trap(false);
        let first = self.first.load(Ordering::Acquire) as usize;
        let end = (self.used.load(Ordering::Acquire) as usize).min(CAP as usize);
        let records = (first..end)
            .filter_map(|i| {
                let m = self.meta[i].load(Ordering::Acquire);
                (m & VALID != 0).then(|| Access {
                    write: m & WRITE != 0,
                    off: m & 0xffff_ffff,
                    width: ((m >> 32) & 0xff) as u8,
                    value: self.vals[i].load(Ordering::Relaxed),
                })
            })
            .collect();
        let spent = self.used.load(Ordering::Acquire) >= CAP;
        let chan = self.chan.lock().ok().and_then(|mut g| g.take());
        self.state
            .store(if spent { SPENT } else { IDLE }, Ordering::Release);
        Some(Closed {
            window: self.windows.load(Ordering::Acquire),
            chan,
            records,
            spent,
        })
    }

    /// Whether the one-time USERD/notifier dump is still owed (true once, then false).
    pub fn take_dump(&self) -> bool {
        self.on && !self.dumped.swap(true, Ordering::AcqRel)
    }
}

/// The printed form of a window's records: consecutive identical records collapse to one line
/// with a repeat count (the log stays bounded; the records themselves were bounded by [`CAP`]).
#[must_use]
pub fn render(c: &Closed, closed_by: &str) -> Vec<String> {
    let mut out = vec![format!(
        "kf3: BAR0-TRACE window #{} chan={} closed by the next RPC ({closed_by}): {} access(es){}",
        c.window,
        c.chan.map_or("?".to_string(), |t| format!(
            "{:#x}:{:#x} (host {:#x})",
            t.client, t.object, t.host
        )),
        c.records.len(),
        if c.spent {
            " — the run's cap is spent, no window opens again"
        } else {
            ""
        }
    )];
    let mut i = 0;
    while i < c.records.len() {
        let r = c.records[i];
        let mut n = 1;
        while i + n < c.records.len() && c.records[i + n] == r {
            n += 1;
        }
        out.push(format!(
            "kf3: BAR0-TRACE #{} {} {:#08x} w{} = {:#x}{}",
            c.window,
            if r.write { "W" } else { "R" },
            r.off,
            r.width,
            r.value,
            if n > 1 {
                format!(" x{n}")
            } else {
                String::new()
            }
        ));
        i += n;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rd(off: u64, v: u64) -> Access {
        Access {
            write: false,
            off,
            width: 4,
            value: v,
        }
    }

    const C: TraceChan = TraceChan {
        client: 0xc1d0_0020,
        object: 0xff04_0009,
        host: 0x1_0038,
    };

    #[test]
    fn off_records_nothing() {
        let t = Bar0Trace::new(false);
        t.schedule_served(C);
        t.after_publish();
        t.note(rd(0x110094, 0), false);
        assert!(t.take_closed().is_none());
        assert!(!t.take_dump());
    }

    #[test]
    fn nothing_is_recorded_before_the_reply_is_published() {
        let t = Bar0Trace::new(true);
        t.note(rd(0x0, 1), false);
        t.schedule_served(C);
        t.note(rd(0x0, 2), false); // armed, not open
        t.after_publish();
        t.note(rd(0x9400, 3), false);
        t.note(
            Access {
                write: true,
                off: 0x110c00,
                width: 4,
                value: 0,
            },
            true,
        );
        t.note(rd(0x0, 4), false); // after the close
        let c = t.take_closed().expect("closed by the queue-head write");
        assert_eq!(c.window, 1);
        assert_eq!(c.chan, Some(C));
        assert_eq!(c.records.len(), 2);
        assert_eq!(c.records[0], rd(0x9400, 3));
        assert!(c.records[1].write);
        assert!(t.take_closed().is_none());
        // a second schedule opens a second window
        t.schedule_served(C);
        t.after_publish();
        t.note(rd(0x88068, 5), true);
        assert_eq!(
            t.take_closed().map(|c| (c.window, c.records.len())),
            Some((2, 1))
        );
    }

    #[test]
    fn the_cap_bounds_the_run_and_spends_it() {
        let t = Bar0Trace::new(true);
        t.schedule_served(C);
        t.after_publish();
        for i in 0..(CAP + 10) {
            t.note(rd(0x110094, u64::from(i)), false);
        }
        let c = t.take_closed().expect("closed by the cap");
        assert_eq!(c.records.len(), CAP as usize);
        assert!(c.spent);
        t.schedule_served(C);
        t.after_publish();
        t.note(rd(0x0, 0), true);
        assert!(t.take_closed().is_none(), "nothing opens once spent");
    }

    #[test]
    fn render_collapses_repeats() {
        let c = Closed {
            window: 3,
            chan: Some(C),
            records: vec![rd(0x110094, 0), rd(0x110094, 0), rd(0xb81010, 0x8000_0000)],
            spent: false,
        };
        let l = render(&c, "Free");
        assert_eq!(l.len(), 3);
        assert!(l[1].ends_with("= 0x0 x2"), "{}", l[1]);
        assert!(l[0].contains("0xc1d00020:0xff040009"));
    }

    #[test]
    fn the_dump_is_one_time() {
        let t = Bar0Trace::new(true);
        assert!(t.take_dump());
        assert!(!t.take_dump());
    }
}
