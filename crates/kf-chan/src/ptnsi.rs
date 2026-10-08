//! ★★★ **Host non-stall edges → guest vectors: wake every subscriber, never drop** (2026-10-08,
//! owner ruling §X in `docs/OWNER_RULINGS.md`).
//!
//! A Passthrough twin's pushbuffer runs on the real engine unparsed, so the completion interrupt
//! it asks for (`NON_STALL_INTERRUPT`, a copy's `LAUNCH_DMA` with `INTERRUPT_TYPE_NON_BLOCKING`)
//! is raised by the HOST GPU and serviced by the HOST RM. What reaches kayfabe is one of the host
//! RM's GPU-wide non-stall notifiers, which carry no identity:
//!
//! - the engine's own notifier (`engineNonStallIntrNotify(RM_ENGINE_TYPE_COPY(n))`,
//!   `ogkm-595.91.07: kernel_ce.c:717`), when the host services that engine's notification;
//! - `NV2080_NOTIFIERS_FIFO_EVENT_MTHD` (RM's HOST notifier), which the host RM fires for EVERY
//!   host-driven engine's non-stall service under `bDefaultNonstallNotify`
//!   (`ogkm-595.91.07: intr.c:1210-1214`).
//!
//! ⊘ CORRECTION (2026-10-08, to the 9925108e sentence after it): GR0's notifier was not watched
//! at 9925108e, so "`FIFO_EVENT_MTHD` ONLY" means "of the watched files". `[measured 2026-10-08,
//! rawclient --ce-interrupt at f589ab23 with GR0 watched, bare metal, 2 runs,
//! traces/passthrough_nsi_nogate_20261008/]` a COPY0 copy's interrupt lands on GR0 AND
//! `FIFO_EVENT_MTHD` in 50/50 iterations (CE0 0/50); COPY2's on CE2 and `FIFO_EVENT_MTHD` (GR0
//! 0/50) — the owner's GRCE hypothesis (COPY0 is a graphics CE, serviced as GR0) survived its
//! falsifier. Behaviour does not depend on it.
//!
//! `[measured 2026-10-08, rawclient --ce-interrupt at 9925108e, RTX 4070, bare metal,
//! traces/rawclient_ce_interrupt_20261008/bare_ce-interrupt_9925108e_run{1,2,3}.log]` a COPY0
//! copy's interrupt arrives on `FIFO_EVENT_MTHD` ONLY (CE0's own notifier stays silent); a COPY2
//! copy's on both CE2 and `FIFO_EVENT_MTHD`.
//!
//! ## The rule (owner, 2026-10-08): follow NVIDIA — an interrupt wakes everyone subscribed
//!
//! RM wakes EVERY client registered on an engine's non-stall list
//! (`_gpuEngineEventNotificationListNotify`, `ogkm-595.84: event_notification.c:330-452`), whoever's work it
//! was. kayfabe does the same, one level up: a host edge is forwarded to every VM whose guest has
//! **armed** that event — a live guest event (userspace's `NV01_EVENT_OS_EVENT` or the guest
//! kernel's `NV01_EVENT_KERNEL_CALLBACK[_EX]`) with `NV01_EVENT_NONSTALL_INTR` on that notifier (`kf_rm::osevent::NonstallArms`, a host-recorded fact about the guest's own
//! subscription, never a doorbell, never "has a live twin", never "has work outstanding"):
//!
//! - an engine-notifier edge → that engine's guest vector, if the guest armed that engine;
//! - a `FIFO_EVENT_MTHD` edge → a guest vector whose service fires the guest's own HOST notifier
//!   (every engine's does), chosen with the least collateral by [`host_notify_vector`]: one no
//!   armed engine shares, else GR0's — if the guest armed `FIFO_EVENT_MTHD` (libcuda does,
//!   `kf_rm::osevent`), and so does the guest KERNEL's CeUtils (`NV01_EVENT_KERNEL_CALLBACK_EX`,
//!   `mem_utils.c:1905-1906`). ⊘ CORRECTION (2026-10-09, review of 1f083ac4, to the measurement
//!   after it): the kernel-callback classes were not counted as armed at 15a400b5, so the 131
//!   unarmed edges below include edges the guest kernel had subscribed to.
//!   `[measured 2026-10-08, kf3 15a400b5, fast guest]` 131 host `FIFO_EVENT_MTHD` edges arrived
//!   while the slot was unarmed (`NotArmed`, nothing raised; whose work they were is not
//!   established), and the quiet window and controls stayed clean.
//!
//! `[measured 2026-10-08, traces/passthrough_nsi_nogate_20261008/]` in the kf3 guest at 15a400b5
//! every `--ce-interrupt` leg lands exactly where it lands on bare metal (COPY0: GR0 and
//! `FIFO_EVENT_MTHD` 50/50; COPY2: CE2 and `FIFO_EVENT_MTHD` 50/50; controls 0/10).
//!
//! ⊘ **INVARIANT: an edge may be delayed, never dropped.** Losing a wake for relevant work is a
//! correctness bug; a cross-tenant wake is a minor denial of service, accepted (the guest's
//! waiter re-checks its semaphore and sleeps again). So there is no gate and no dropping bucket.
//! Merged raises cannot lose a wake either: the guest's leaf pending bit is a level that stays set
//! until the guest's write-1-to-clear (`kf_trap::cpuintr`), and the guest clears it BEFORE it
//! services the engine (`intr_nonstall_tu102.c:365-375`), so an edge after the clear re-pends.
//!
//! ## Optional pacing — off by default, loss-free when on
//!
//! [`Pacer`] with a non-zero minimum interval (`KF3_PT_NSI_MIN_INTERVAL_US`, default 0 = off)
//! raises a vector at most once per interval. An edge inside the interval sets the vector's
//! **pending** flag instead; the worker's tick ([`Pacer::flush`], run on every worker loop and,
//! while anything is owed, at least every millisecond) raises it once the interval has passed —
//! so a coalesced edge is delivered even if no further edge ever arrives. ⊘ No allocation, no
//! lock: atomics only; nothing here runs on a vCPU.

use std::sync::atomic::{AtomicU64, Ordering};

/// How many guest vectors the relay can name (`0..256`); a larger vector is refused.
pub const VECTORS: usize = 256;

/// The largest minimum interval a setting may ask for (1 s).
pub const MAX_MIN_INTERVAL_US: u64 = 1_000_000;

/// The pacing interval from `KF3_PT_NSI_MIN_INTERVAL_US` in ns: unset, junk or `0` = OFF (the
/// default); otherwise clamped to [`MAX_MIN_INTERVAL_US`].
#[must_use]
pub fn min_interval_ns_from(setting: Option<&str>) -> u64 {
    setting
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map_or(0, |us| us.min(MAX_MIN_INTERVAL_US).saturating_mul(1000))
}

/// What [`Pacer::offer`] did with one edge for one vector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Offer {
    /// Raise the vector now.
    Raise,
    /// Inside the interval: the vector is now pending, and the tick will raise it.
    Pending,
    /// Inside the interval and already pending: merged into that later raise.
    Merged,
    /// The vector is out of range (`>= VECTORS`): refused, counted.
    Refused,
}

/// Per-vector state and counters. Bounded: five words.
#[derive(Debug, Default)]
struct Slot {
    /// The earliest time (ns, the caller's clock) the next raise may go out (pacing only).
    next_ok_ns: AtomicU64,
    /// Raises delivered (immediate and late).
    raised: AtomicU64,
    /// Edges coalesced into a later raise, counted when that raise went out.
    coalesced_raised: AtomicU64,
    /// Edges coalesced into the raise still owed.
    absorbing: AtomicU64,
    /// Late raises delivered by the tick.
    late: AtomicU64,
}

/// Per-VM, per-guest-vector delivery: every offered edge raises, unless pacing is on, in which
/// case it raises now or is owed — never dropped.
#[derive(Debug)]
pub struct Pacer {
    interval_ns: u64,
    slots: Box<[Slot]>,
    /// One bit per vector: a raise is owed.
    owed: [AtomicU64; VECTORS / 64],
    /// Offers refused for an out-of-range vector.
    pub refused: AtomicU64,
}

impl Pacer {
    /// A pacer raising each vector at most once per `interval_ns` (`0` = no pacing: every offer
    /// raises). Allocates once, here.
    #[must_use]
    pub fn new(interval_ns: u64) -> Pacer {
        Pacer {
            interval_ns,
            slots: (0..VECTORS).map(|_| Slot::default()).collect(),
            owed: std::array::from_fn(|_| AtomicU64::new(0)),
            refused: AtomicU64::new(0),
        }
    }

    /// The pacing interval (ns; `0` = off).
    #[must_use]
    pub fn interval_ns(&self) -> u64 {
        self.interval_ns
    }

    /// One edge for vector `v` at `now_ns`. Safe from several workers at once.
    pub fn offer(&self, v: u32, now_ns: u64) -> Offer {
        let Some(s) = self.slots.get(v as usize) else {
            self.refused.fetch_add(1, Ordering::Relaxed);
            return Offer::Refused;
        };
        if self.interval_ns == 0 {
            s.raised.fetch_add(1, Ordering::Relaxed);
            return Offer::Raise;
        }
        let (w, bit) = ((v / 64) as usize, 1u64 << (v % 64));
        // Raise now if nothing is owed and the interval has passed (one CAS winner per slot).
        if self.owed[w].load(Ordering::Acquire) & bit == 0 {
            let next = s.next_ok_ns.load(Ordering::Acquire);
            if now_ns >= next
                && s.next_ok_ns
                    .compare_exchange(
                        next,
                        now_ns.saturating_add(self.interval_ns),
                        Ordering::AcqRel,
                        Ordering::Relaxed,
                    )
                    .is_ok()
            {
                s.raised.fetch_add(1, Ordering::Relaxed);
                return Offer::Raise;
            }
        }
        // ⊘ Owed, never dropped. The read-modify-write is what makes it loss-free: if a flush
        // cleared the bit after the load above, this sets a NEW owed raise.
        s.absorbing.fetch_add(1, Ordering::Relaxed);
        if self.owed[w].fetch_or(bit, Ordering::AcqRel) & bit == 0 {
            Offer::Pending
        } else {
            Offer::Merged
        }
    }

    /// ★ **The tick** (worker, every loop and at least every millisecond while
    /// [`Pacer::owes`]): raise every owed vector whose interval has passed, via `deliver`.
    /// Returns whether anything is still owed. With pacing off it does nothing.
    pub fn flush(&self, now_ns: u64, mut deliver: impl FnMut(u32)) -> bool {
        if self.interval_ns == 0 {
            return false;
        }
        let mut still = false;
        for (w, word) in self.owed.iter().enumerate() {
            let mut bits = word.load(Ordering::Acquire);
            while bits != 0 {
                let b = bits.trailing_zeros();
                bits &= bits - 1;
                let v = w as u32 * 64 + b;
                let Some(s) = self.slots.get(v as usize) else {
                    continue;
                };
                let next = s.next_ok_ns.load(Ordering::Acquire);
                if now_ns < next
                    || s.next_ok_ns
                        .compare_exchange(
                            next,
                            now_ns.saturating_add(self.interval_ns),
                            Ordering::AcqRel,
                            Ordering::Relaxed,
                        )
                        .is_err()
                {
                    still = true;
                    continue;
                }
                // The CAS winner owns the late raise: clear the bit, THEN raise, so an edge after
                // the clear is owed again rather than lost.
                if word.fetch_and(!(1u64 << b), Ordering::AcqRel) & (1u64 << b) != 0 {
                    let n = s.absorbing.swap(0, Ordering::AcqRel);
                    s.coalesced_raised.fetch_add(n, Ordering::Relaxed);
                    s.late.fetch_add(1, Ordering::Relaxed);
                    s.raised.fetch_add(1, Ordering::Relaxed);
                    deliver(v);
                }
            }
        }
        still || self.owes()
    }

    /// Whether any raise is owed (one relaxed load per word).
    #[must_use]
    pub fn owes(&self) -> bool {
        self.owed.iter().any(|w| w.load(Ordering::Relaxed) != 0)
    }

    /// Raises delivered on vector `v`.
    #[must_use]
    pub fn raised(&self, v: u32) -> u64 {
        self.slots
            .get(v as usize)
            .map_or(0, |s| s.raised.load(Ordering::Relaxed))
    }

    /// Edges on vector `v` coalesced into a raise that has gone out.
    #[must_use]
    pub fn coalesced_raised(&self, v: u32) -> u64 {
        self.slots
            .get(v as usize)
            .map_or(0, |s| s.coalesced_raised.load(Ordering::Relaxed))
    }

    /// `vN[raised=… coalesced_later_raised=… late=… owed=…]` for every vector with activity.
    #[must_use]
    pub fn summary(&self) -> String {
        let o = Ordering::Relaxed;
        let mut out = Vec::new();
        for (v, s) in self.slots.iter().enumerate() {
            let raised = s.raised.load(o);
            let absorbing = s.absorbing.load(o);
            if raised == 0 && absorbing == 0 {
                continue;
            }
            let owed = self.owed[v / 64].load(o) & (1 << (v % 64)) != 0;
            out.push(format!(
                "v{v}[raised={raised} coalesced_later_raised={} late={} owed={owed}]",
                s.coalesced_raised.load(o),
                s.late.load(o)
            ));
        }
        format!(
            "pacing={} refused={} {}",
            if self.interval_ns == 0 {
                "off".to_string()
            } else {
                format!("{}us", self.interval_ns / 1000)
            },
            self.refused.load(o),
            out.join(" ")
        )
    }
}

/// ★ The guest vector a host `FIFO_EVENT_MTHD` edge is raised on: one whose service fires the
/// guest's own HOST notifier (every engine's does — `bDefaultNonstallNotify`, no engine waives it,
/// `ogkm-595.84: intr.c:1210-1214`) with the LEAST collateral — the first vector, in `engines`
/// order, that no engine the guest ARMED is announced on, so the guest fires `FIFO_EVENT_MTHD` and
/// no engine notification anyone waits on. If every vector carries an armed engine, `fallback`
/// (GR0's): a spurious engine wake, accepted (owner ruling §X). `engines` yields `(vector, armed)`.
///
/// `[measured 2026-10-08, kf3 f589ab23, guest --ce-interrupt]` raising GR0's vector for every
/// `FIFO_EVENT_MTHD` edge made the guest fire GR0's notifier on 50/50 COPY2 iterations, which bare
/// metal never does (`traces/passthrough_nsi_nogate_20261008/`) — the reason for this choice.
#[must_use]
pub fn host_notify_vector(
    engines: impl Iterator<Item = (Option<u32>, bool)> + Clone,
    fallback: Option<u32>,
) -> Option<u32> {
    let mut armed = [0u64; VECTORS / 64];
    for (v, a) in engines.clone() {
        if let (Some(v), true) = (v, a)
            && let Some(w) = armed.get_mut(v as usize / 64)
        {
            *w |= 1 << (v % 64);
        }
    }
    engines
        .filter_map(|(v, _)| v)
        .find(|&v| {
            armed
                .get(v as usize / 64)
                .is_some_and(|w| w & (1 << (v % 64)) == 0)
        })
        .or(fallback)
}

/// Which host notifier an edge came on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    /// `NV2080_NOTIFIERS_FIFO_EVENT_MTHD` (the host notifier; the session completion fd).
    Fifo,
    /// An engine's own non-stall notifier.
    Engine,
}

/// What the relay did with one host edge for this VM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Raise this guest vector now.
    Raise(u32),
    /// Pacing is on and the vector's window is open: the raise is owed (the tick delivers it).
    Owed,
    /// The guest has not armed this event: nothing to wake.
    NotArmed,
    /// Armed, but the served table gives the engine no guest vector (counted; healthy value 0).
    Unvectored,
    /// `KF3_PT_NSI_RELAY=0` and a `FIFO_EVENT_MTHD` edge: counted only (the falsifier mode).
    RelayOff,
    /// The vector is out of range: refused (counted by the pacer).
    Refused,
}

/// ★ One VM's relay: every host edge for an event its guest armed raises the event's guest vector
/// (now, or — pacing on — owed and raised by the tick). Never drops one.
#[derive(Debug)]
pub struct Relay {
    pacer: Pacer,
    relay_fifo: bool,
    /// Edges judged.
    pub edges: AtomicU64,
    /// Edges the guest had not armed.
    pub not_armed: AtomicU64,
    /// Armed edges whose engine has no guest vector.
    pub unvectored: AtomicU64,
    /// `FIFO_EVENT_MTHD` edges counted only (`KF3_PT_NSI_RELAY=0`).
    pub relay_off: AtomicU64,
}

impl Relay {
    /// `interval_ns`: the optional pacing (`0` = off, the default); `relay_fifo`: `false` only
    /// for the `KF3_PT_NSI_RELAY=0` falsifier run (FIFO edges counted, not raised; engine edges
    /// still raised).
    #[must_use]
    pub fn new(interval_ns: u64, relay_fifo: bool) -> Relay {
        Relay {
            pacer: Pacer::new(interval_ns),
            relay_fifo,
            edges: AtomicU64::new(0),
            not_armed: AtomicU64::new(0),
            unvectored: AtomicU64::new(0),
            relay_off: AtomicU64::new(0),
        }
    }

    /// Whether `FIFO_EVENT_MTHD` edges are relayed (not the falsifier mode).
    #[must_use]
    pub fn relays_fifo(&self) -> bool {
        self.relay_fifo
    }

    /// The pacer (its interval and per-vector counters).
    #[must_use]
    pub fn pacer(&self) -> &Pacer {
        &self.pacer
    }

    /// ★ **Worker**, holding a REAL host edge of `kind`: `armed` = this VM's guest has a live
    /// non-stall subscription on the event (`kf_rm::osevent::NonstallArms`), `vector` = the guest
    /// vector that event is announced on. Safe from several workers at once.
    pub fn edge(&self, kind: EdgeKind, armed: bool, vector: Option<u32>, now_ns: u64) -> Verdict {
        self.edges.fetch_add(1, Ordering::Relaxed);
        if !armed {
            self.not_armed.fetch_add(1, Ordering::Relaxed);
            return Verdict::NotArmed;
        }
        let Some(v) = vector else {
            self.unvectored.fetch_add(1, Ordering::Relaxed);
            return Verdict::Unvectored;
        };
        if kind == EdgeKind::Fifo && !self.relay_fifo {
            self.relay_off.fetch_add(1, Ordering::Relaxed);
            return Verdict::RelayOff;
        }
        match self.pacer.offer(v, now_ns) {
            Offer::Raise => Verdict::Raise(v),
            Offer::Pending | Offer::Merged => Verdict::Owed,
            Offer::Refused => Verdict::Refused,
        }
    }

    /// ★ **The tick** — see [`Pacer::flush`].
    pub fn flush(&self, now_ns: u64, deliver: impl FnMut(u32)) -> bool {
        self.pacer.flush(now_ns, deliver)
    }

    /// `edges=… not_armed=… unvectored=… relay_off=… relay=on|OFF pacing=… vN[…]…`.
    #[must_use]
    pub fn summary(&self) -> String {
        let o = Ordering::Relaxed;
        format!(
            "edges={} not_armed={} unvectored={} relay_off={} relay={} {}",
            self.edges.load(o),
            self.not_armed.load(o),
            self.unvectored.load(o),
            self.relay_off.load(o),
            if self.relay_fifo { "on" } else { "OFF" },
            self.pacer.summary()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const US: u64 = 1000;

    #[test]
    fn with_pacing_off_every_edge_raises() {
        let p = Pacer::new(0);
        for t in 0..1000 {
            assert_eq!(p.offer(3, t), Offer::Raise);
        }
        assert_eq!(p.raised(3), 1000);
        assert!(!p.flush(0, |_| panic!("nothing owed")));
        assert!(!p.owes());
    }

    #[test]
    fn hostile_vectors_are_refused_and_counted() {
        for p in [Pacer::new(0), Pacer::new(100 * US)] {
            assert_eq!(p.offer(256, 0), Offer::Refused);
            assert_eq!(p.offer(u32::MAX, 0), Offer::Refused);
            assert_eq!(p.refused.load(Ordering::Relaxed), 2);
            assert_eq!(p.raised(256), 0);
        }
    }

    /// ★ The owner's test: ONE edge inside the pacing window, and no later edge ever — it is
    /// still raised, later, by the tick alone.
    #[test]
    fn a_single_edge_inside_the_window_with_no_later_edge_still_raises_later() {
        let p = Pacer::new(100 * US);
        assert_eq!(p.offer(5, 1_000 * US), Offer::Raise);
        assert_eq!(
            p.offer(5, 1_010 * US),
            Offer::Pending,
            "inside the window: owed"
        );
        let mut got = Vec::new();
        assert!(
            p.flush(1_050 * US, |v| got.push(v)),
            "before the window ends: still owed"
        );
        assert!(got.is_empty());
        assert!(!p.flush(1_100 * US, |v| got.push(v)), "nothing owed after");
        assert_eq!(
            got,
            vec![5],
            "raised once, by the tick, with no further edge"
        );
        assert_eq!(p.raised(5), 2);
        assert_eq!(p.coalesced_raised(5), 1);
    }

    #[test]
    fn a_burst_inside_the_window_is_merged_into_one_later_raise() {
        let p = Pacer::new(100 * US);
        assert_eq!(p.offer(7, 0), Offer::Raise);
        assert_eq!(p.offer(7, 1), Offer::Pending);
        for t in 2..50 {
            assert_eq!(p.offer(7, t), Offer::Merged);
        }
        let mut got = Vec::new();
        p.flush(100 * US, |v| got.push(v));
        assert_eq!(got, vec![7]);
        assert_eq!(p.coalesced_raised(7), 49);
        // The late raise restarted the window: the next edge right after is owed again…
        assert_eq!(p.offer(7, 100 * US + 1), Offer::Pending);
        // …and an edge after the window raises at once only once nothing is owed.
        p.flush(200 * US, |v| got.push(v));
        assert_eq!(got, vec![7, 7]);
        assert_eq!(p.offer(7, 400 * US), Offer::Raise);
    }

    #[test]
    fn vectors_are_paced_independently() {
        let p = Pacer::new(100 * US);
        assert_eq!(p.offer(0, 0), Offer::Raise);
        assert_eq!(
            p.offer(65, 0),
            Offer::Raise,
            "another vector has its own window"
        );
        assert_eq!(p.offer(0, 1), Offer::Pending);
        let mut got = Vec::new();
        p.flush(100 * US, |v| got.push(v));
        assert_eq!(got, vec![0]);
    }

    #[test]
    fn the_interval_setting_is_parsed_and_clamped_and_off_by_default() {
        assert_eq!(min_interval_ns_from(None), 0);
        assert_eq!(min_interval_ns_from(Some("junk")), 0);
        assert_eq!(min_interval_ns_from(Some("0")), 0);
        assert_eq!(min_interval_ns_from(Some(" 250 ")), 250_000);
        assert_eq!(
            min_interval_ns_from(Some("99999999999999")),
            MAX_MIN_INTERVAL_US * 1000
        );
    }

    /// Two workers offering and flushing at once: every edge is raised now or later, and no
    /// edge after the last raise is left unowed.
    #[test]
    fn two_workers_never_lose_an_edge() {
        use std::sync::atomic::AtomicU64 as A;
        let p = Pacer::new(10 * US);
        let clock = A::new(0);
        let delivered = A::new(0);
        std::thread::scope(|sc| {
            for _ in 0..2 {
                sc.spawn(|| {
                    for _ in 0..20_000 {
                        let t = clock.fetch_add(US / 4, Ordering::Relaxed);
                        let _ = p.offer(9, t);
                        p.flush(t, |_| {
                            delivered.fetch_add(1, Ordering::Relaxed);
                        });
                    }
                });
            }
        });
        // After the edges stop, the tick alone drains what is owed.
        let end = clock.load(Ordering::Relaxed) + 20 * US;
        p.flush(end, |_| {
            delivered.fetch_add(1, Ordering::Relaxed);
        });
        assert!(!p.owes(), "nothing left owed after the tick");
        let immediate = p.raised(9) - p.slots[9].late.load(Ordering::Relaxed);
        assert_eq!(
            immediate + p.coalesced_raised(9),
            40_000,
            "every edge raised at once or coalesced into a raise that went out"
        );
        assert_eq!(
            delivered.load(Ordering::Relaxed),
            p.slots[9].late.load(Ordering::Relaxed)
        );
    }

    #[test]
    fn an_armed_vm_gets_every_edge() {
        let r = Relay::new(0, true);
        for t in 0..500 {
            assert_eq!(r.edge(EdgeKind::Fifo, true, Some(0), t), Verdict::Raise(0));
            assert_eq!(
                r.edge(EdgeKind::Engine, true, Some(4), t),
                Verdict::Raise(4)
            );
        }
        assert_eq!(r.pacer().raised(0), 500);
        assert_eq!(r.pacer().raised(4), 500);
        assert_eq!(r.not_armed.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn an_unarmed_vm_gets_nothing_and_counts_it() {
        let r = Relay::new(0, true);
        for kind in [EdgeKind::Fifo, EdgeKind::Engine] {
            assert_eq!(r.edge(kind, false, Some(0), 1), Verdict::NotArmed);
        }
        assert_eq!(r.not_armed.load(Ordering::Relaxed), 2);
        assert_eq!(r.pacer().raised(0), 0);
    }

    #[test]
    fn two_vms_one_armed_one_not_see_the_same_gpu_wide_edge_differently() {
        let (a, b) = (Relay::new(0, true), Relay::new(0, true));
        // The same host edge reaches both VMs' workers; only B's guest armed the event.
        assert_eq!(
            a.edge(EdgeKind::Engine, false, Some(2), 7),
            Verdict::NotArmed
        );
        assert_eq!(
            b.edge(EdgeKind::Engine, true, Some(2), 7),
            Verdict::Raise(2)
        );
    }

    #[test]
    fn hostile_or_missing_vectors_are_refused_not_raised() {
        let r = Relay::new(0, true);
        assert_eq!(r.edge(EdgeKind::Engine, true, None, 0), Verdict::Unvectored);
        assert_eq!(
            r.edge(EdgeKind::Engine, true, Some(256), 0),
            Verdict::Refused
        );
        assert_eq!(
            r.edge(EdgeKind::Fifo, true, Some(u32::MAX), 0),
            Verdict::Refused
        );
        assert_eq!(r.unvectored.load(Ordering::Relaxed), 1);
        assert_eq!(r.pacer().refused.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn the_falsifier_mode_counts_fifo_edges_and_still_raises_engine_edges() {
        let r = Relay::new(0, false);
        assert_eq!(r.edge(EdgeKind::Fifo, true, Some(0), 0), Verdict::RelayOff);
        assert_eq!(
            r.edge(EdgeKind::Engine, true, Some(1), 0),
            Verdict::Raise(1)
        );
        assert_eq!(r.relay_off.load(Ordering::Relaxed), 1);
        assert!(r.summary().contains("relay=OFF"));
    }

    #[test]
    fn with_pacing_on_an_armed_edge_is_owed_then_raised_by_the_tick() {
        let r = Relay::new(100 * US, true);
        assert_eq!(r.edge(EdgeKind::Fifo, true, Some(0), 0), Verdict::Raise(0));
        assert_eq!(
            r.edge(EdgeKind::Fifo, true, Some(0), 10 * US),
            Verdict::Owed
        );
        let mut got = Vec::new();
        assert!(!r.flush(100 * US, |v| got.push(v)));
        assert_eq!(got, vec![0]);
    }

    #[test]
    fn the_host_notify_vector_avoids_every_vector_an_armed_engine_shares() {
        // AD104's served table: GR0, CE0, CE1 on 0; CE2 on 1; CE3 on 2; NVENC1 4; NVDEC0 3; OFA 5.
        let table = |armed: &[bool; 8]| {
            let armed = *armed;
            let v = [0, 0, 0, 1, 2, 4, 3, 5];
            (0..8).map(move |i| (Some(v[i]), armed[i]))
        };
        // Only the guest kernel's FIFO_EVENT_MTHD is armed: the first vector (GR0's) is clean.
        assert_eq!(host_notify_vector(table(&[false; 8]), Some(0)), Some(0));
        // CE1 armed shares vector 0 with GR0: vector 0 is out, CE2's 1 is next.
        let a = [false, false, true, false, false, false, false, false];
        assert_eq!(host_notify_vector(table(&a), Some(0)), Some(1));
        // GR0 and every CE armed (the --ce-interrupt client): a video engine's vector.
        let a = [true, true, true, true, true, false, false, false];
        assert_eq!(host_notify_vector(table(&a), Some(0)), Some(4));
        // Everything armed: the fallback (a spurious engine wake, accepted).
        assert_eq!(host_notify_vector(table(&[true; 8]), Some(0)), Some(0));
        // Unvectored engines are skipped; hostile vectors never index out of range.
        let e = [(None, false), (Some(u32::MAX), true), (Some(7), false)];
        assert_eq!(host_notify_vector(e.into_iter(), None), Some(7));
        assert_eq!(host_notify_vector([(None, false)].into_iter(), None), None);
    }

    /// Two workers judging edges on one VM at once (pacing off): every armed edge raises.
    #[test]
    fn two_workers_raise_every_armed_edge() {
        let r = Relay::new(0, true);
        let raised = AtomicU64::new(0);
        std::thread::scope(|sc| {
            for kind in [EdgeKind::Fifo, EdgeKind::Engine] {
                let (r, raised) = (&r, &raised);
                sc.spawn(move || {
                    for t in 0..10_000 {
                        if let Verdict::Raise(_) = r.edge(kind, true, Some(0), t) {
                            raised.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                });
            }
        });
        assert_eq!(raised.load(Ordering::Relaxed), 20_000);
        assert_eq!(r.edges.load(Ordering::Relaxed), 20_000);
    }
}
