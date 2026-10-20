//! ★ 2026-10-20 MEASUREMENT ONLY -- the glue of `KF3_DIAG_USER_STALL=<ms>` (default OFF).
//!
//! STATUS: LIVE, 2026-10-20 (DIAGNOSTIC; never a decision input, never needed to boot). Design:
//! `docs/design/V3_USER_RING_EVENT_LOOP_20261014.md`, section "KF3_DIAG_USER_STALL".
//!
//! The pure half is `kf_chan::user_stall` (detector, pushbuffer method decoder, segment lookup, bounded
//! hex-only renderer, counters). This file only gathers facts and prints what the renderer made:
//!
//! * WHERE it runs: on the relay worker, from the worker's tick (`ChanPlane::user_ring_refresh`, after each
//!   wait of the worker loop ends), never on a vCPU, the drainer or the act thread. Two plain field writes
//!   (`Pump::diag_wake`, `Pump::diag_turn`) record the last look that rang a relay and the last turn.
//! * WHAT it takes: `try_lock` only, on the relay map, each relay's record and its private storage. A
//!   record the act thread holds is skipped and counted. Nothing is waited for, nothing allocates on a hot path
//!   (a report allocates, once per condition, after the threshold).
//! * WHAT it reads: the private `USERD` (which the relay already reads), the private GP ring entries, and,
//!   for the segment the PBDMA is inside, the guest pushbuffer and the semaphores it names, through the
//!   relay's own placement view (`userread::snapshot`). The store reader is passive: it never arms,
//!   evicts or releases a CPU view, so a semaphore in video memory that is not already mapped reads as
//!   `UNREAD(vidmem)` rather than disturbing the relay's working set.
//! * WHAT it prints: lines the pure renderer made, every integer in hex, bounded in count and length.
//! * COST when off: one relaxed load of [`THRESHOLD_MS`] per worker tick, and a `None` test per look/turn.

use super::*;
use kf_chan::user_stall as stall;
use std::sync::atomic::{AtomicU64, Ordering};

/// The threshold in milliseconds; 0 = off. Written once by [`init`], read relaxed.
static THRESHOLD_MS: AtomicU64 = AtomicU64::new(0);
/// When the cumulative counter line was last printed (ms), and the counters' sum at that time.
static PERIOD_AT_MS: AtomicU64 = AtomicU64::new(0);
static PERIOD_SUM: AtomicU64 = AtomicU64::new(0);

/// Read the switch ONCE, when the first relay is created (`Pump::new`); never per turn or per look.
pub(super) fn init() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let value = std::env::var(kf_chan::user_stall::ENV).ok();
        match stall::parse_setting(value.as_deref()) {
            stall::Setting::Off => {}
            stall::Setting::On(ms) => {
                THRESHOLD_MS.store(ms, Ordering::Relaxed);
                eprintln!(
                    "kf3: USER_RING_STALL_DIAG ARMED threshold_ms={ms:#x} ({}; measurement only: reads, never decides; one report block per channel per condition)",
                    stall::ENV
                );
            }
            stall::Setting::Invalid => eprintln!(
                "kf3: USER_RING_STALL_DIAG IGNORED: {} is not a number of milliseconds; the diagnostic stays off",
                stall::ENV
            ),
        }
    });
}

/// The threshold; 0 when off. One relaxed load.
pub(super) fn threshold_ms() -> u64 {
    THRESHOLD_MS.load(Ordering::Relaxed)
}

/// A relay's diagnostic state: present only when the switch is on.
pub(super) fn channel_state() -> Option<Box<stall::ChannelDiag>> {
    (threshold_ms() != 0).then(|| Box::new(stall::ChannelDiag::new()))
}

/// Milliseconds since the first call. Only reached with the diagnostic on.
fn now_ms() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    u64::try_from(
        START
            .get_or_init(std::time::Instant::now)
            .elapsed()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}

/// The name printed for a refused read (a short literal; nothing the guest chose is printed).
pub(super) fn refusal_name(why: userread::Refusal) -> &'static str {
    match why {
        userread::Refusal::Bounds => "bounds",
        userread::Refusal::Busy => "busy",
        userread::Refusal::Changed => "changed",
        userread::Refusal::Missing(_) => "unmapped",
        userread::Refusal::Topology => "topology",
        // a store (video memory) window that is not already armed
        userread::Refusal::Store => "vidmem",
        userread::Refusal::Allocation => "alloc",
        userread::Refusal::Poisoned => "poisoned",
    }
}

/// The relay's own store cache, read-only: a window is used only if it is already armed.
struct Passive<'a>(&'a crate::userstore::Cache);

impl userread::Store for Passive<'_> {
    fn prepare(&mut self, spans: &[kf_chan::tmode::Span]) -> Result<(), userread::Refusal> {
        self.0.passive_prepare(spans)
    }
    fn copy(&self, off: u64, bytes: &mut [u8]) -> Result<(), userread::Refusal> {
        userread::Store::copy(self.0, off, bytes)
    }
}

/// The record's fields the look needs, borrowed apart from the pump.
struct View<'a> {
    key: (u32, u32),
    host: u32,
    idx: u32,
    userd: &'a userrelay::UserdSource,
    allocation: &'a Arc<()>,
    lease: &'a Arc<crate::userva::Lease>,
    mirror: &'a Mirror,
}

impl Pump {
    /// A look rang this relay (`ChanPlane::user_ring_peek_one`). A field write when on, a `None` test when off.
    pub(super) fn diag_wake(&mut self, why: kf_chan::user_loop::Peek) {
        if let Some(d) = self.diag.as_mut() {
            d.note_wake(now_ms(), why);
        }
    }

    /// The relay had a turn (`ChanPlane::user_ring_serve`).
    pub(super) fn diag_turn(&mut self) {
        if let Some(d) = self.diag.as_mut() {
            d.note_turn(now_ms());
        }
    }

    /// One look at this relay's private ring; a block of lines when it has not drained for `threshold` ms.
    fn diag_look(&mut self, me: &ChanPlane, v: &View<'_>, now: u64, threshold: u64) {
        use std::sync::atomic::Ordering::Relaxed;
        let stats = &me.relay_stats.life.stall;
        let Some(d) = self.diag.as_mut() else {
            return;
        };
        d.note_doorbells(now, me.queued(v.idx));
        let cursors = match v.lease.storage.try_lock() {
            Ok(storage) => storage.gpu_cursors().ok(),
            Err(_) => {
                stats.skipped_busy.fetch_add(1, Relaxed);
                None
            }
        };
        let Some((get, put)) = cursors else {
            return;
        };
        let kf_chan::user_stall::Verdict::Fire { stalled_ms } =
            d.detector
                .observe(now, get, put, kf_chan::user_storage::ENTRIES, threshold)
        else {
            return;
        };
        let (doorbells, wake, turn_ago_ms) = (d.doorbells(now), d.last_wake(now), d.last_turn(now));
        // the private USERD and the ring rows around GP_GET, then the lock is dropped
        let (userd, rows) = match v.lease.storage.try_lock() {
            Ok(storage) => {
                let Ok(userd) = storage.userd_words() else {
                    return;
                };
                let (first, count) =
                    stall::ring_window(userd[34], userd[35], kf_chan::user_storage::ENTRIES);
                let rows: Vec<stall::Row> = (0..count)
                    .filter_map(|i| {
                        let slot = first.wrapping_add(i) & (kf_chan::user_storage::ENTRIES - 1);
                        storage
                            .read_gp(slot)
                            .ok()
                            .map(|entry| stall::Row { slot, entry })
                    })
                    .collect();
                (userd, rows)
            }
            Err(_) => {
                stats.skipped_busy.fetch_add(1, Relaxed);
                return;
            }
        };
        let guest = |offset: u64| {
            let mut b = [0u8; 4];
            v.userd
                .read(me, v.key, v.allocation, offset, &mut b)
                .ok()
                .map(|()| u32::from_le_bytes(b))
        };
        let life = self.core.life.counts();
        let inputs = stall::Inputs {
            key: v.key,
            host: v.host,
            stalled_ms,
            threshold_ms: threshold,
            guest_put: guest(kf_abi::submit::USERD_GP_PUT),
            guest_get: guest(kf_abi::submit::USERD_GP_GET),
            userd,
            entries: kf_chan::user_storage::ENTRIES,
            rows,
            wait: self.wait,
            doorbells,
            wake,
            turn_ago_ms,
            receipts: (
                self.core.fetch.issued(),
                self.core.fetch.fetched(),
                self.core.marks.issued(),
                self.core.marks.seen(),
            ),
            cursor: self.core.cursor,
            turns: self.core.lazy.counts.turns,
            entries_by_mode: (life.entries_scan, life.entries_pointer, life.entries_window),
            totals: me.relay_stats.life.stall.summary(),
        };
        let (cache, generation) = (&self.cache, self.ram_generation);
        let mut read = |va: u64, bytes: usize| -> Result<Vec<u8>, &'static str> {
            userread::snapshot(
                &v.mirror.rows,
                &v.mirror.log,
                me.ram,
                generation,
                va,
                bytes,
                &mut Passive(cache),
            )
            .map(|s| s.bytes)
            .map_err(refusal_name)
        };
        let report = kf_chan::user_stall::build_report(inputs, &mut read, stats);
        for line in kf_chan::user_stall::render(&report) {
            eprintln!("{line}");
        }
    }
}

impl ChanPlane {
    /// The diagnostic's entry, called from the worker tick (`user_ring_refresh`) on the relay worker. When the
    /// switch is off it is one relaxed load and a return.
    pub(super) fn user_ring_diag(&self) {
        let ms = threshold_ms();
        if ms == 0 {
            return;
        }
        let now = now_ms();
        let Ok(rings) = self.user_rings.try_lock() else {
            return;
        };
        for record in rings.values() {
            if record.is_canceled() {
                continue;
            }
            let Ok(mut guard) = record.try_lock() else {
                // the act thread holds it: skipped, counted, nothing waited for
                self.relay_stats
                    .life
                    .stall
                    .skipped_busy
                    .fetch_add(1, Ordering::Relaxed);
                continue;
            };
            let r = &mut *guard;
            if r.retired || r.free_attempted || r.dead.is_some() {
                continue;
            }
            let Some(host) = r.chan.map(|c| c.token) else {
                continue;
            };
            let userrelay::Record {
                pump,
                userd,
                allocation,
                lease,
                mirror,
                key,
                idx,
                ..
            } = r;
            let Some(p) = pump.as_mut() else {
                continue;
            };
            let view = View {
                key: *key,
                host,
                idx: *idx,
                userd,
                allocation,
                lease,
                mirror,
            };
            p.diag_look(self, &view, now, ms);
        }
        drop(rings);
        // the cumulative counters, once per period and only when they moved (the lifecycle counters line)
        let stall = &self.relay_stats.life.stall;
        if stall.any() {
            let sum = stall.sum();
            let (last, last_sum) = (
                PERIOD_AT_MS.load(Ordering::Relaxed),
                PERIOD_SUM.load(Ordering::Relaxed),
            );
            if stall::period_due(now, last, sum, last_sum) {
                PERIOD_AT_MS.store(now, Ordering::Relaxed);
                PERIOD_SUM.store(sum, Ordering::Relaxed);
                eprintln!(
                    "kf3: USER_RING_LIFECYCLE PERIOD {}",
                    self.relay_stats.life.summary()
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refused_reads_have_short_stable_names_and_a_store_refusal_is_vidmem() {
        use userread::Refusal as R;
        let t = [
            (R::Bounds, "bounds"),
            (R::Busy, "busy"),
            (R::Changed, "changed"),
            (R::Missing(0x1000), "unmapped"),
            (R::Topology, "topology"),
            (R::Store, "vidmem"),
            (R::Allocation, "alloc"),
            (R::Poisoned, "poisoned"),
        ];
        for (r, name) in t {
            assert_eq!(refusal_name(r), name, "{r:?}");
            assert!(name.chars().all(|c| c.is_ascii_lowercase()));
        }
        // a missing row never prints its address: the guest chose it
        assert_eq!(
            refusal_name(R::Missing(0xdead_beef)),
            refusal_name(R::Missing(1))
        );
    }
}
