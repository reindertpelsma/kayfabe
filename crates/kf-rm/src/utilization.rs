//! ★ **GPU utilisation reported to the guest** — the `report-utilization` device property, default ON
//! (`docs/design/V3_UTILIZATION_FLAG.md`, `docs/OWNER_RULINGS.md` §AV, owner decision 2026-11-17).
//!
//! STATUS: LIVE, 2026-11-17 (GPU-free; **not run on hardware**: every hypothesis and its falsifier is in the
//! design doc, section 9).
//!
//! What a guest asks for, and what answers it:
//!
//! | guest asks | this module |
//! |---|---|
//! | `NV2080_CTRL_CMD_PERF_GET_GPUMON_PERFMON_UTIL_SAMPLES_V2` | a reply rebuilt from zero out of the cached host reading ([`kf_abi::utilization::encode_gpumon_reply`]) |
//! | `INTERNAL_INIT_USER_SHARED_DATA` (the RUSD page) | the page address is remembered, never dereferenced here |
//! | `INTERNAL_USER_SHARED_DATA_SET_DATA_POLL` | the mask is remembered; only the PERF bit is consulted |
//!
//! and the drainer thread (never a vCPU) writes one 56-byte `perfDevUtil` record into the guest's RUSD page
//! ([`UtilizationState::publish`]).
//!
//! ## The rules this module keeps
//!
//! - **One way, read only.** The only host request is a constant ([`kf_abi::utilization::host_request`]) made by
//!   the [`Sampler`] on its own thread. No byte of any guest message reaches it; no guest message makes a host
//!   call (the guest's requests are answered from a cache). What the guest controls is "asking".
//! - **Derived, never typed.** Every id, size and offset comes from the generated, exact-per-tag layout
//!   ([`kf_abi::utilization`]); a guest driver version with no row has no link (the controls reach the ledger
//!   and are refused, as before).
//! - **Unavailable is not zero.** No fresh sample means the control is not answered (refused, as before) and
//!   the RUSD record's timestamp stays `RUSD_TIMESTAMP_INVALID`.
//! - **Bounded.** The reply has the request's own size; GPUMON replies are limited to
//!   [`GPUMON_REPLIES_PER_SEC`]; the host is called at most once per [`SAMPLE_PERIOD_MIN`]; the RUSD write is
//!   one fixed record at one generated offset inside the first page of an address the guest named, through the
//!   guarded guest-RAM writer.
//! - ⚠ **A cross-tenant side channel on a shared GPU.** The numbers are the PHYSICAL GPU's, not this VM's own.
//!   That is why the flag exists; `report-utilization=off` seats nothing here (`ObjectLinks::utilization` is
//!   `None`), starts no thread and makes no host call.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kf_abi::utilization::{self as ku, GuestSample, HostReading, Layout, RusdInit};
use kf_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};

const NV_OK: u32 = 0;

/// The sampler's cadence while a guest asked within [`INTEREST_WINDOW`].
pub const SAMPLE_PERIOD: Duration = Duration::from_millis(250);
/// The sampler's cadence while nobody asked: slow, so that a one-shot query is answerable from a young cache.
pub const IDLE_PERIOD: Duration = Duration::from_secs(1);
/// ★ The floor between two host calls, whatever a caller does (design doc, section 4).
pub const SAMPLE_PERIOD_MIN: Duration = Duration::from_millis(100);
/// A snapshot older than this is not served ("not available" is better than a frozen number).
pub const FRESH_FOR: Duration = Duration::from_secs(3);
/// How long after the last ask the sampler keeps the fast cadence.
pub const INTEREST_WINDOW: Duration = Duration::from_secs(10);
/// GPUMON replies per second (a token bucket). The reply is 55,888 bytes; a flood must not be able to make the
/// drainer build thousands of them.
pub const GPUMON_REPLIES_PER_SEC: u64 = 16;
/// Samples kept for the GPUMON reply (the control's own `NV2080_CTRL_PERF_GPUMON_SAMPLE_COUNT_PERFMON_UTIL`).
pub const RING_MAX: usize = 72;

/// The time source of the state (production: [`MonotonicClock`]).
pub trait Clock: Send + Sync {
    /// Nanoseconds on a monotonic clock, never 0.
    fn now_ns(&self) -> u64;
}

/// The production clock: nanoseconds since the state was built, plus one (so never 0).
#[derive(Debug)]
pub struct MonotonicClock(Instant);

impl MonotonicClock {
    /// A clock starting now.
    #[must_use]
    pub fn new() -> Self {
        MonotonicClock(Instant::now())
    }
}

impl Default for MonotonicClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for MonotonicClock {
    fn now_ns(&self) -> u64 {
        u64::try_from(self.0.elapsed().as_nanos())
            .unwrap_or(u64::MAX - 1)
            .saturating_add(1)
    }
}

/// Why the host could not give a reading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UtilRefusal {
    /// This host has no unprivileged source (the control is not there, not allowed, or the host driver is
    /// outside the layouts derived): permanent. The device then does not serve the feature at all.
    NoSource(String),
    /// A call failed this time (an ioctl error, a status that may pass).
    Failed(String),
    /// The host answered and holds no sample yet.
    Empty,
}

/// ★ **The host seam**: one fixed, host-authored read. Implemented by `kf-qemu` over the host RM session, and by
/// tests over a fake. It takes no argument: nothing the guest sent can reach it.
pub trait HostUtilization: Send {
    /// One reading of the physical GPU's utilisation.
    ///
    /// # Errors
    /// [`UtilRefusal`].
    fn read(&mut self) -> Result<HostReading, UtilRefusal>;
}

/// What the state counted. All monotonic, relaxed.
#[derive(Debug, Default)]
struct Counters {
    gpumon_served: AtomicU64,
    gpumon_no_sample: AtomicU64,
    gpumon_rate_limited: AtomicU64,
    gpumon_refused_shape: AtomicU64,
    init_linked: AtomicU64,
    init_unlinked: AtomicU64,
    init_refused: AtomicU64,
    poll_seen: AtomicU64,
    published: AtomicU64,
    invalidated: AtomicU64,
    publish_refused: AtomicU64,
    host_reads: AtomicU64,
    host_refused: AtomicU64,
    host_stalled: AtomicU64,
    host_empty: AtomicU64,
    clamped: AtomicU64,
}

/// A copy of the counters, for the status line and for tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CounterSnapshot {
    /// GPUMON replies built.
    pub gpumon_served: u64,
    /// GPUMON requests refused because no fresh sample existed.
    pub gpumon_no_sample: u64,
    /// GPUMON requests refused by the token bucket.
    pub gpumon_rate_limited: u64,
    /// GPUMON requests refused for their shape (size, type, bufSize).
    pub gpumon_refused_shape: u64,
    /// RUSD INIT links accepted.
    pub init_linked: u64,
    /// RUSD INIT unlinks accepted.
    pub init_unlinked: u64,
    /// RUSD INIT refused (malformed or bad address).
    pub init_refused: u64,
    /// RUSD POLL requests seen.
    pub poll_seen: u64,
    /// RUSD records published.
    pub published: u64,
    /// RUSD records invalidated because the samples went stale.
    pub invalidated: u64,
    /// RUSD publications the guest-RAM writer refused (address not registered RAM).
    pub publish_refused: u64,
    /// Host reads made.
    pub host_reads: u64,
    /// Host reads refused.
    pub host_refused: u64,
    /// Host reads whose sample timestamp did not advance (dropped).
    pub host_stalled: u64,
    /// Host reads that held no sample.
    pub host_empty: u64,
    /// Raw values clamped to 100 %.
    pub clamped: u64,
}

fn load(c: &AtomicU64) -> u64 {
    c.load(Ordering::Relaxed)
}

fn bump(c: &AtomicU64) {
    c.fetch_add(1, Ordering::Relaxed);
}

/// ★ The shared state of one VM's utilisation reporting: the sample ring (written by the sampler thread, read by
/// the chain and the drainer), the guest's RUSD link, and the counters. Every field is bounded; nothing in it is
/// keyed by a guest-supplied value.
pub struct UtilizationState {
    clock: Arc<dyn Clock>,
    /// At most [`RING_MAX`] samples, oldest first. Held only to copy; never across a host call.
    ring: Mutex<Vec<GuestSample>>,
    /// The clock value of the last ask (0 = never).
    interest_ns: AtomicU64,
    /// The guest's RUSD page (0 = none).
    rusd_phys: AtomicU64,
    /// Whether the last poll mask asked for the utilisation record.
    rusd_wants: AtomicBool,
    /// The RUSD layout of the current chain (set when the link is built; `None` before).
    rusd_layout: Mutex<Option<&'static Layout>>,
    /// A publication is owed now (a poll or a link just changed).
    publish_pending: AtomicBool,
    /// The clock value of the last publication.
    last_published_ns: AtomicU64,
    /// The last timestamp written to the RUSD record (strictly increasing).
    last_ts: AtomicU64,
    /// Whether the guest's page holds a record this module wrote with a valid timestamp (so it must be
    /// invalidated, not left to look current, when the samples go stale).
    record_valid: AtomicBool,
    counters: Counters,
}

impl core::fmt::Debug for UtilizationState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("UtilizationState")
            .field("counters", &self.counters())
            .finish_non_exhaustive()
    }
}

impl UtilizationState {
    /// A state on `clock`.
    #[must_use]
    pub fn new(clock: Arc<dyn Clock>) -> Arc<UtilizationState> {
        Arc::new(UtilizationState {
            clock,
            ring: Mutex::new(Vec::new()),
            interest_ns: AtomicU64::new(0),
            rusd_phys: AtomicU64::new(0),
            rusd_wants: AtomicBool::new(false),
            rusd_layout: Mutex::new(None),
            publish_pending: AtomicBool::new(false),
            last_published_ns: AtomicU64::new(0),
            last_ts: AtomicU64::new(0),
            record_valid: AtomicBool::new(false),
            counters: Counters::default(),
        })
    }

    /// A state on the production clock.
    #[must_use]
    pub fn monotonic() -> Arc<UtilizationState> {
        Self::new(Arc::new(MonotonicClock::new()))
    }

    /// The state's clock, in nanoseconds.
    #[must_use]
    pub fn now_ns(&self) -> u64 {
        self.clock.now_ns()
    }

    /// Record that a guest asked (a GPUMON request, or a poll for the utilisation record).
    pub fn note_interest(&self) {
        self.interest_ns.store(self.now_ns(), Ordering::Relaxed);
    }

    /// Whether the sampler should keep the fast cadence: somebody asked within [`INTEREST_WINDOW`], or a RUSD
    /// page is linked and polled.
    #[must_use]
    pub fn interested(&self) -> bool {
        let at = self.interest_ns.load(Ordering::Relaxed);
        let recent = at != 0
            && self.now_ns().saturating_sub(at)
                <= u64::try_from(INTEREST_WINDOW.as_nanos()).unwrap_or(u64::MAX);
        recent
            || (self.rusd_wants.load(Ordering::Relaxed)
                && self.rusd_phys.load(Ordering::Relaxed) != 0)
    }

    /// Append a sample (the sampler's, under the lock only for the copy).
    pub fn push(&self, s: GuestSample) {
        let mut ring = self
            .ring
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if ring.len() == RING_MAX {
            ring.remove(0);
        }
        ring.push(s);
    }

    /// The ring if its newest sample is at most [`FRESH_FOR`] old; `None` otherwise (and when empty).
    #[must_use]
    pub fn fresh_samples(&self) -> Option<Vec<GuestSample>> {
        let ring = self
            .ring
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let newest = ring.last()?;
        let age = self.now_ns().saturating_sub(newest.timestamp_ns);
        (age <= u64::try_from(FRESH_FOR.as_nanos()).unwrap_or(u64::MAX)).then(|| ring.clone())
    }

    /// The guest's linked RUSD page address, if any (read-only; for the status line and tests).
    #[must_use]
    pub fn rusd_page(&self) -> Option<u64> {
        Some(self.rusd_phys.load(Ordering::Relaxed)).filter(|p| *p != 0)
    }

    /// Whether the guest's last poll mask asked for the utilisation record.
    #[must_use]
    pub fn rusd_wants_utilization(&self) -> bool {
        self.rusd_wants.load(Ordering::Relaxed)
    }

    /// ★ The guest is gone (a device reset): forget its RUSD link, mask and layout at once, keep the samples. The
    /// device calls this on every non-wake reset, besides the chain rebuild, so no path leaves a link.
    pub fn forget_guest(&self) {
        self.reset_guest(None);
    }

    /// A new guest life (the chain was rebuilt): forget the guest's RUSD link and mask, keep the samples.
    fn reset_guest(&self, rusd: Option<&'static Layout>) {
        self.record_valid.store(false, Ordering::Relaxed);
        self.rusd_phys.store(0, Ordering::Relaxed);
        self.rusd_wants.store(false, Ordering::Relaxed);
        self.publish_pending.store(false, Ordering::Relaxed);
        *self
            .rusd_layout
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = rusd;
    }

    /// The counters.
    #[must_use]
    pub fn counters(&self) -> CounterSnapshot {
        let c = &self.counters;
        CounterSnapshot {
            gpumon_served: load(&c.gpumon_served),
            gpumon_no_sample: load(&c.gpumon_no_sample),
            gpumon_rate_limited: load(&c.gpumon_rate_limited),
            gpumon_refused_shape: load(&c.gpumon_refused_shape),
            init_linked: load(&c.init_linked),
            init_unlinked: load(&c.init_unlinked),
            init_refused: load(&c.init_refused),
            poll_seen: load(&c.poll_seen),
            published: load(&c.published),
            invalidated: load(&c.invalidated),
            publish_refused: load(&c.publish_refused),
            host_reads: load(&c.host_reads),
            host_refused: load(&c.host_refused),
            host_stalled: load(&c.host_stalled),
            host_empty: load(&c.host_empty),
            clamped: load(&c.clamped),
        }
    }

    /// One line for the device's status line.
    #[must_use]
    pub fn status(&self) -> String {
        let c = self.counters();
        format!(
            "util[on host_reads={} refused={} stalled={} empty={} gpumon={}/{}/{} rusd={}/{}/{} published={}/{}]",
            c.host_reads,
            c.host_refused,
            c.host_stalled,
            c.host_empty,
            c.gpumon_served,
            c.gpumon_no_sample,
            c.gpumon_rate_limited,
            c.init_linked,
            c.init_unlinked,
            c.poll_seen,
            c.published,
            c.publish_refused
        )
    }

    /// ★ **Publish the RUSD record** if one is due: a page is linked, the guest's mask asked for the utilisation
    /// record, a fresh sample exists, and either a link/poll just changed or [`SAMPLE_PERIOD`] has passed.
    /// `write(gpa, bytes)` is the guarded guest-RAM writer (it must refuse anything that is not registered guest
    /// RAM). Runs on the register drainer, never on a vCPU; it makes no host call and takes no lock a vCPU takes.
    ///
    /// The write is three steps (the seqlock of `cl00de.h`): the timestamp word set to `WRITE_IN_PROGRESS`, the
    /// info block, then a valid timestamp. A refused write ends the attempt for this tick (counted); the
    /// timestamp is left `WRITE_IN_PROGRESS` only if the second write fails, which a reader treats as "retry".
    pub fn publish(&self, write: &mut dyn FnMut(u64, &[u8]) -> bool) -> PublishOutcome {
        let phys = self.rusd_phys.load(Ordering::Relaxed);
        if phys == 0 || !self.rusd_wants.load(Ordering::Relaxed) {
            return PublishOutcome::NotWanted;
        }
        let Some(layout) = *self
            .rusd_layout
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        else {
            return PublishOutcome::NotWanted;
        };
        let now = self.now_ns();
        let pending = self.publish_pending.load(Ordering::Relaxed);
        let last = self.last_published_ns.load(Ordering::Relaxed);
        let period = u64::try_from(SAMPLE_PERIOD.as_nanos()).unwrap_or(u64::MAX);
        if !pending && last != 0 && now.saturating_sub(last) < period {
            return PublishOutcome::NotDue;
        }
        let Some(samples) = self.fresh_samples() else {
            // ★ The samples went stale (the host stopped answering). A record already written must not keep a
            // valid timestamp and a frozen number: write the control's own "not available" (timestamp 0), once.
            if self.record_valid.swap(false, Ordering::Relaxed) {
                let wrote = phys
                    .checked_add(layout.rusd_off_util + layout.util_off_ts)
                    .is_some_and(|at| write(at, &layout.ts_invalid.to_le_bytes()));
                self.publish_pending.store(false, Ordering::Relaxed);
                if !wrote {
                    bump(&self.counters.publish_refused);
                    return PublishOutcome::Refused;
                }
                bump(&self.counters.invalidated);
                return PublishOutcome::Invalidated;
            }
            return PublishOutcome::NoSample;
        };
        let Some(newest) = samples.last() else {
            return PublishOutcome::NoSample;
        };
        // Strictly increasing, non-zero, below the seqlock's reserved range (the encoder checks the range).
        let ts = now.max(self.last_ts.load(Ordering::Relaxed).saturating_add(1));
        let period_us = u32::try_from(SAMPLE_PERIOD.as_micros()).unwrap_or(u32::MAX);
        let Some(w) = ku::encode_rusd_record(layout, &newest.percents, ts, period_us) else {
            return PublishOutcome::NoSample;
        };
        // The guest's page + a generated offset; checked_add so an address near 2^64 cannot wrap (the link
        // refuses anything at or above 2^52, this is the second guard).
        let at = |off: u64| phys.checked_add(off);
        let (Some(a1), Some(a2), Some(a3)) = (at(w.begin.0), at(w.info.0), at(w.finish.0)) else {
            bump(&self.counters.publish_refused);
            return PublishOutcome::Refused;
        };
        if !write(a1, &w.begin.1) || !write(a2, &w.info.1) || !write(a3, &w.finish.1) {
            bump(&self.counters.publish_refused);
            self.publish_pending.store(false, Ordering::Relaxed);
            self.last_published_ns.store(now, Ordering::Relaxed);
            return PublishOutcome::Refused;
        }
        self.last_ts.store(ts, Ordering::Relaxed);
        self.record_valid.store(true, Ordering::Relaxed);
        self.last_published_ns.store(now, Ordering::Relaxed);
        self.publish_pending.store(false, Ordering::Relaxed);
        bump(&self.counters.published);
        PublishOutcome::Written
    }
}

/// What one [`UtilizationState::publish`] call did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishOutcome {
    /// No page linked, or the guest did not ask for the utilisation record, or no layout.
    NotWanted,
    /// Wanted, but the last publication is younger than [`SAMPLE_PERIOD`] and nothing changed.
    NotDue,
    /// Wanted and due, but there is no fresh sample: nothing was written (the record stays invalid).
    NoSample,
    /// The record was written.
    Written,
    /// The samples went stale: the record was invalidated (timestamp `RUSD_TIMESTAMP_INVALID`) instead of left
    /// looking current.
    Invalidated,
    /// The guest-RAM writer refused (the page is not registered RAM), or the address would wrap.
    Refused,
}

/// What one [`Sampler::tick`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TickOutcome {
    /// Not due yet (the period or the [`SAMPLE_PERIOD_MIN`] floor).
    NotDue,
    /// A reading was taken and stored.
    Sampled,
    /// The host's sample did not advance: dropped, the snapshot ages instead of freezing a number.
    Stalled,
    /// The host answered with no sample.
    Empty,
    /// The host refused.
    Refused(UtilRefusal),
}

/// ★ The sampler: owns the host seam and decides WHEN to read. One per device, driven by one thread
/// (`kf-qemu`'s `kf3-util`); `tick` is cheap when nothing is due and is the only place a host call is made.
pub struct Sampler {
    host: Box<dyn HostUtilization>,
    state: Arc<UtilizationState>,
    last_call_ns: u64,
    last_host_ts: u64,
}

impl Sampler {
    /// A sampler reading `host` into `state`.
    #[must_use]
    pub fn new(host: Box<dyn HostUtilization>, state: Arc<UtilizationState>) -> Sampler {
        Sampler {
            host,
            state,
            last_call_ns: 0,
            last_host_ts: 0,
        }
    }

    /// The cadence now: fast while somebody asked, slow otherwise; never below [`SAMPLE_PERIOD_MIN`].
    #[must_use]
    pub fn period(&self) -> Duration {
        if self.state.interested() {
            SAMPLE_PERIOD.max(SAMPLE_PERIOD_MIN)
        } else {
            IDLE_PERIOD.max(SAMPLE_PERIOD_MIN)
        }
    }

    /// Read the host if one is due. The host call is made with NO lock held (the state's locks are taken only
    /// afterwards, to store the result).
    pub fn tick(&mut self) -> TickOutcome {
        let now = self.state.now_ns();
        let need = u64::try_from(self.period().as_nanos()).unwrap_or(u64::MAX);
        if self.last_call_ns != 0 && now.saturating_sub(self.last_call_ns) < need {
            return TickOutcome::NotDue;
        }
        self.last_call_ns = now;
        bump(&self.state.counters.host_reads);
        match self.host.read() {
            Ok(r) => {
                // Equal = the host produced nothing new. A SMALLER one means the host's timer restarted (a host GPU
                // reset): accepted, with the new baseline, instead of dropping every sample until it catches up.
                if r.host_timestamp == self.last_host_ts {
                    bump(&self.state.counters.host_stalled);
                    return TickOutcome::Stalled;
                }
                self.last_host_ts = r.host_timestamp;
                self.state
                    .counters
                    .clamped
                    .fetch_add(u64::from(r.clamped), Ordering::Relaxed);
                self.state.push(GuestSample {
                    timestamp_ns: self.state.now_ns(),
                    percents: r.percents,
                });
                TickOutcome::Sampled
            }
            Err(UtilRefusal::Empty) => {
                bump(&self.state.counters.host_empty);
                TickOutcome::Empty
            }
            Err(e) => {
                bump(&self.state.counters.host_refused);
                TickOutcome::Refused(e)
            }
        }
    }
}

/// What the device hands the chain: the shared state. `None` in `ObjectLinks` is the flag OFF.
#[derive(Clone)]
pub struct UtilizationSeat {
    /// The shared state.
    pub state: Arc<UtilizationState>,
}

impl core::fmt::Debug for UtilizationSeat {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("UtilizationSeat").finish_non_exhaustive()
    }
}

/// A token bucket: [`GPUMON_REPLIES_PER_SEC`] per second, capacity one second's worth.
#[derive(Debug)]
struct Bucket {
    tokens: u64,
    last_ns: u64,
}

impl Bucket {
    fn new(now: u64) -> Bucket {
        Bucket {
            tokens: GPUMON_REPLIES_PER_SEC,
            last_ns: now,
        }
    }

    fn take(&mut self, now: u64) -> bool {
        let elapsed = now.saturating_sub(self.last_ns);
        // GPUMON_REPLIES_PER_SEC tokens per 1e9 ns, whole tokens only (the remainder is kept by not advancing).
        let add = elapsed.saturating_mul(GPUMON_REPLIES_PER_SEC) / 1_000_000_000;
        if add > 0 {
            self.tokens = (self.tokens + add).min(GPUMON_REPLIES_PER_SEC);
            self.last_ns += add * 1_000_000_000 / GPUMON_REPLIES_PER_SEC;
        }
        if self.tokens == 0 {
            return false;
        }
        self.tokens -= 1;
        true
    }
}

/// ★ **The chain link.** Claims only the three ids of the layout row of the guest driver version it was built
/// for; for everything else, and for every request shape it does not serve, it answers `None`, i.e. exactly what
/// the chain did before this link existed.
pub struct UtilizationPolicy {
    driver: kf_abi::versions::DriverAbiTable,
    gpumon: Option<&'static Layout>,
    rusd: Option<&'static Layout>,
    state: Arc<UtilizationState>,
    bucket: Bucket,
}

impl UtilizationPolicy {
    /// A link for the guest driver `driver`, or `None` when that version has neither a GPUMON nor a RUSD row
    /// (exact membership: the controls then reach the ledger and are refused, as before).
    #[must_use]
    pub fn new(
        driver: kf_abi::versions::DriverAbiTable,
        seat: &UtilizationSeat,
    ) -> Option<UtilizationPolicy> {
        let v = driver.driver_version();
        let (gpumon, rusd) = (ku::gpumon_at(v), ku::rusd_at(v));
        // ★ A new chain is a new guest life: its RUSD link and mask start empty, and this happens BEFORE any early
        // return. A rebuild for a driver version with no row must leave NO link behind, or the drainer would keep
        // writing into the memory of the next guest life (security review of PR #459, F1).
        seat.state.reset_guest(rusd);
        if gpumon.is_none() && rusd.is_none() {
            return None;
        }
        let now = seat.state.now_ns();
        Some(UtilizationPolicy {
            driver,
            gpumon,
            rusd,
            state: seat.state.clone(),
            bucket: Bucket::new(now),
        })
    }

    /// An OK reply echoing the request's own body (the guest's bytes go back to the guest only) with the
    /// status word set.
    fn ok_echo(&self, cmd: &RpcCommand) -> Option<Reply> {
        let mut body = cmd.payload.clone();
        let st = self.driver.rm_control_wire().status_off;
        body.get_mut(st..st.checked_add(4)?)?
            .copy_from_slice(&NV_OK.to_le_bytes());
        Some(Reply {
            rpc_result_private: None,
            rpc_result: NV_OK,
            body,
        })
    }

    fn gpumon(
        &mut self,
        l: &Layout,
        cmd: &RpcCommand,
        params_at: usize,
        size: usize,
    ) -> Option<Reply> {
        let params = cmd.payload.get(params_at..params_at.checked_add(size)?)?;
        if !ku::gpumon_request_is_served(l, params) {
            bump(&self.state.counters.gpumon_refused_shape);
            return None;
        }
        // The ask counts even when refused below: it wakes the sampler's fast cadence.
        self.state.note_interest();
        if !self.bucket.take(self.state.now_ns()) {
            bump(&self.state.counters.gpumon_rate_limited);
            return None;
        }
        let Some(samples) = self.state.fresh_samples() else {
            bump(&self.state.counters.gpumon_no_sample);
            return None;
        };
        let reply = ku::encode_gpumon_reply(l, &samples)?;
        if reply.len() != size {
            return None;
        }
        let mut body = cmd.payload.clone();
        body.get_mut(params_at..params_at + size)?
            .copy_from_slice(&reply);
        let st = self.driver.rm_control_wire().status_off;
        body.get_mut(st..st.checked_add(4)?)?
            .copy_from_slice(&NV_OK.to_le_bytes());
        bump(&self.state.counters.gpumon_served);
        Some(Reply {
            rpc_result_private: None,
            rpc_result: NV_OK,
            body,
        })
    }

    fn rusd_init(
        &self,
        l: &Layout,
        cmd: &RpcCommand,
        params_at: usize,
        size: usize,
    ) -> Option<Reply> {
        let params = cmd.payload.get(params_at..params_at.checked_add(size)?)?;
        match ku::parse_init(l, params) {
            Some(RusdInit::Link(phys)) => {
                self.state.rusd_phys.store(phys, Ordering::Relaxed);
                self.state.publish_pending.store(true, Ordering::Relaxed);
                bump(&self.state.counters.init_linked);
            }
            Some(RusdInit::Unlink) => {
                self.state.rusd_phys.store(0, Ordering::Relaxed);
                self.state.publish_pending.store(false, Ordering::Relaxed);
                bump(&self.state.counters.init_unlinked);
            }
            None => {
                bump(&self.state.counters.init_refused);
                return None;
            }
        }
        self.ok_echo(cmd)
    }

    fn rusd_poll(
        &self,
        l: &Layout,
        cmd: &RpcCommand,
        params_at: usize,
        size: usize,
    ) -> Option<Reply> {
        let params = cmd.payload.get(params_at..params_at.checked_add(size)?)?;
        let mask = ku::parse_poll(l, params)?;
        let wants = ku::mask_wants_utilization(l, mask);
        self.state.rusd_wants.store(wants, Ordering::Relaxed);
        if wants {
            self.state.note_interest();
            self.state.publish_pending.store(true, Ordering::Relaxed);
        }
        bump(&self.state.counters.poll_seen);
        self.ok_echo(cmd)
    }
}

impl CommandPolicy for UtilizationPolicy {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        if cmd.function != RpcFunction::RmControl {
            return None;
        }
        let req = self.driver.decode_rpc_control(&cmd.payload).ok()?;
        if kf_abi::rpc_params_are_serialized(req.rmapi_rpc_flags) {
            return None;
        }
        let size = req.params_size as usize;
        if let Some(l) = self.gpumon
            && u64::from(req.cmd) == l.gpumon_cmd
            && size == l.params_len()
        {
            return self.gpumon(l, cmd, req.params_at, size);
        }
        if let Some(l) = self.rusd {
            if u64::from(req.cmd) == l.rusd_init_cmd && size as u64 == l.init_size {
                return self.rusd_init(l, cmd, req.params_at, size);
            }
            if u64::from(req.cmd) == l.rusd_poll_cmd && size as u64 == l.poll_size {
                return self.rusd_poll(l, cmd, req.params_at, size);
            }
        }
        None
    }
}

kf_util::assert_send_sync!(UtilizationPolicy);

#[cfg(test)]
mod tests {
    use super::*;
    use kf_abi::utilization::Percents;
    use std::sync::atomic::AtomicU64;

    /// A hand-wound clock.
    #[derive(Debug, Default)]
    struct FakeClock(AtomicU64);

    impl FakeClock {
        fn at(ns: u64) -> Arc<FakeClock> {
            Arc::new(FakeClock(AtomicU64::new(ns)))
        }
        fn advance(&self, d: Duration) {
            self.0
                .fetch_add(u64::try_from(d.as_nanos()).unwrap(), Ordering::Relaxed);
        }
    }

    impl Clock for FakeClock {
        fn now_ns(&self) -> u64 {
            self.0.load(Ordering::Relaxed)
        }
    }

    /// A scripted host: pops results in order, counts calls.
    struct ScriptedHost {
        script: Vec<Result<HostReading, UtilRefusal>>,
        calls: Arc<AtomicU64>,
    }

    impl HostUtilization for ScriptedHost {
        fn read(&mut self) -> Result<HostReading, UtilRefusal> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            if self.script.is_empty() {
                return Err(UtilRefusal::Empty);
            }
            self.script.remove(0)
        }
    }

    fn reading(ts: u64, gpu: u8) -> HostReading {
        HostReading {
            host_timestamp: ts,
            percents: Percents {
                gpu,
                memory: 5,
                ..Percents::default()
            },
            clamped: 0,
        }
    }

    fn rig(
        script: Vec<Result<HostReading, UtilRefusal>>,
    ) -> (
        Arc<FakeClock>,
        Arc<UtilizationState>,
        Sampler,
        Arc<AtomicU64>,
    ) {
        let clock = FakeClock::at(1_000_000_000);
        let state = UtilizationState::new(clock.clone());
        let calls = Arc::new(AtomicU64::new(0));
        let sampler = Sampler::new(
            Box::new(ScriptedHost {
                script,
                calls: calls.clone(),
            }),
            state.clone(),
        );
        (clock, state, sampler, calls)
    }

    #[test]
    fn the_sampler_reads_slowly_when_nobody_asked_and_fast_when_somebody_did() {
        let (clock, state, mut s, calls) = rig((1..=40).map(|i| Ok(reading(i, 10))).collect());
        assert_eq!(s.tick(), TickOutcome::Sampled, "the first tick reads");
        clock.advance(Duration::from_millis(500));
        assert_eq!(s.tick(), TickOutcome::NotDue, "idle cadence is 1 s");
        clock.advance(Duration::from_millis(500));
        assert_eq!(s.tick(), TickOutcome::Sampled);
        state.note_interest();
        clock.advance(Duration::from_millis(100));
        assert_eq!(
            s.tick(),
            TickOutcome::NotDue,
            "never below 250 ms while asked, and 100 ms is the floor"
        );
        clock.advance(Duration::from_millis(150));
        assert_eq!(s.tick(), TickOutcome::Sampled);
        assert_eq!(calls.load(Ordering::Relaxed), 3);
        // interest lapses after the window
        clock.advance(INTEREST_WINDOW + Duration::from_secs(1));
        assert!(!state.interested());
        assert_eq!(s.period(), IDLE_PERIOD);
    }

    #[test]
    fn however_often_it_is_ticked_the_host_is_called_at_most_once_per_the_floor() {
        let (clock, state, mut s, calls) = rig((1..=1000).map(|i| Ok(reading(i, 10))).collect());
        state.note_interest();
        let mut simulated = Duration::ZERO;
        while simulated < Duration::from_secs(10) {
            let _ = s.tick();
            clock.advance(Duration::from_millis(1));
            simulated += Duration::from_millis(1);
        }
        let n = calls.load(Ordering::Relaxed);
        assert!(
            n <= 10 * 1000 / u64::try_from(SAMPLE_PERIOD_MIN.as_millis()).unwrap(),
            "{n} calls in 10 s"
        );
        assert!(n <= 41, "{n} calls in 10 s at 250 ms");
        assert!(SAMPLE_PERIOD >= SAMPLE_PERIOD_MIN && IDLE_PERIOD >= SAMPLE_PERIOD_MIN);
    }

    #[test]
    fn a_host_sample_that_did_not_advance_is_dropped_and_the_snapshot_ages_out() {
        let (clock, state, mut s, _) = rig(vec![
            Ok(reading(7, 40)),
            Ok(reading(7, 41)),
            Ok(reading(7, 42)),
        ]);
        state.note_interest();
        assert_eq!(s.tick(), TickOutcome::Sampled);
        for _ in 0..2 {
            clock.advance(SAMPLE_PERIOD);
            assert_eq!(s.tick(), TickOutcome::Stalled);
        }
        assert_eq!(state.counters().host_stalled, 2);
        assert_eq!(
            state.fresh_samples().unwrap().len(),
            1,
            "one sample, the first"
        );
        clock.advance(FRESH_FOR + Duration::from_millis(1));
        assert!(
            state.fresh_samples().is_none(),
            "a frozen number is not served"
        );
    }

    #[test]
    fn refusals_and_empty_replies_store_nothing() {
        let (clock, state, mut s, _) = rig(vec![
            Err(UtilRefusal::Failed("ioctl".into())),
            Err(UtilRefusal::Empty),
        ]);
        state.note_interest();
        assert!(matches!(
            s.tick(),
            TickOutcome::Refused(UtilRefusal::Failed(_))
        ));
        clock.advance(SAMPLE_PERIOD);
        assert_eq!(s.tick(), TickOutcome::Empty);
        assert!(state.fresh_samples().is_none());
        let c = state.counters();
        assert_eq!((c.host_refused, c.host_empty, c.host_reads), (1, 1, 2));
    }

    #[test]
    fn the_ring_keeps_72_and_a_stale_ring_is_not_served() {
        let clock = FakeClock::at(10);
        let state = UtilizationState::new(clock.clone());
        for i in 0..100u64 {
            state.push(GuestSample {
                timestamp_ns: 10 + i,
                percents: Percents::default(),
            });
        }
        let s = state.fresh_samples().unwrap();
        assert_eq!(s.len(), RING_MAX);
        assert_eq!(s[0].timestamp_ns, 10 + 28);
    }

    #[test]
    fn the_token_bucket_allows_a_burst_and_then_the_rate() {
        let mut b = Bucket::new(1_000);
        let mut now = 1_000;
        let burst = (0..100).filter(|_| b.take(now)).count();
        assert_eq!(burst as u64, GPUMON_REPLIES_PER_SEC);
        // one second later, one second's worth again
        now += 1_000_000_000;
        assert_eq!(
            (0..100).filter(|_| b.take(now)).count() as u64,
            GPUMON_REPLIES_PER_SEC
        );
        // half a second: half
        now += 500_000_000;
        assert_eq!(
            (0..100).filter(|_| b.take(now)).count() as u64,
            GPUMON_REPLIES_PER_SEC / 2
        );
    }

    // ---- the publisher

    fn linked(
        clock: &Arc<FakeClock>,
        phys: u64,
        layout: &'static Layout,
        wants: bool,
    ) -> Arc<UtilizationState> {
        let state = UtilizationState::new(clock.clone());
        state.reset_guest(Some(layout));
        state.rusd_phys.store(phys, Ordering::Relaxed);
        state.rusd_wants.store(wants, Ordering::Relaxed);
        state.publish_pending.store(true, Ordering::Relaxed);
        state
    }

    fn l580() -> &'static Layout {
        ku::layout(kf_abi::DriverVersion {
            major: 580,
            minor: 159,
            patch: 4,
        })
        .unwrap()
    }

    #[test]
    fn publish_writes_one_record_at_the_generated_offset_in_three_ordered_steps() {
        let clock = FakeClock::at(5_000);
        let state = linked(&clock, 0x40_0000, l580(), true);
        state.push(GuestSample {
            timestamp_ns: 4_000,
            percents: Percents {
                gpu: 77,
                ..Percents::default()
            },
        });
        let mut writes: Vec<(u64, Vec<u8>)> = Vec::new();
        let out = state.publish(&mut |gpa, b| {
            writes.push((gpa, b.to_vec()));
            true
        });
        assert_eq!(out, PublishOutcome::Written);
        assert_eq!(writes.len(), 3);
        assert_eq!(
            writes[0],
            (0x40_0000 + 1976, u64::MAX.to_le_bytes().to_vec()),
            "in progress"
        );
        assert_eq!(writes[1].0, 0x40_0000 + 1984);
        assert_eq!(&writes[1].1[0..4], &77u32.to_le_bytes());
        assert_eq!(writes[2].0, 0x40_0000 + 1976);
        assert_eq!(
            writes[2].1,
            5_000u64.to_le_bytes().to_vec(),
            "a valid timestamp, last"
        );
        assert!(
            writes
                .iter()
                .all(|(a, b)| a + b.len() as u64 <= 0x40_0000 + 4096)
        );
        assert_eq!(state.counters().published, 1);
    }

    #[test]
    fn publish_is_due_only_on_a_change_or_every_period_and_timestamps_strictly_increase() {
        let clock = FakeClock::at(5_000);
        let state = linked(&clock, 0x40_0000, l580(), true);
        state.push(GuestSample {
            timestamp_ns: 5_000,
            percents: Percents::default(),
        });
        let mut ts = Vec::new();
        let mut w = |gpa: u64, b: &[u8]| {
            if gpa == 0x40_0000 + 1976 && b != u64::MAX.to_le_bytes() {
                ts.push(u64::from_le_bytes(b.try_into().unwrap()));
            }
            true
        };
        assert_eq!(state.publish(&mut w), PublishOutcome::Written);
        assert_eq!(state.publish(&mut w), PublishOutcome::NotDue);
        // same clock value, a poll arrived: published again, and the timestamp still moves forward
        state.publish_pending.store(true, Ordering::Relaxed);
        assert_eq!(state.publish(&mut w), PublishOutcome::Written);
        clock.advance(SAMPLE_PERIOD);
        state.push(GuestSample {
            timestamp_ns: clock.now_ns(),
            percents: Percents::default(),
        });
        assert_eq!(state.publish(&mut w), PublishOutcome::Written);
        assert!(ts.windows(2).all(|p| p[0] < p[1]), "{ts:?}");
        assert_eq!(ts.len(), 3);
    }

    #[test]
    fn publish_writes_nothing_without_a_page_a_poll_a_fresh_sample_or_a_layout() {
        let clock = FakeClock::at(5_000);
        let mut never = |_: u64, _: &[u8]| -> bool { panic!("no write may happen") };
        // no page
        let s = linked(&clock, 0, l580(), true);
        s.push(GuestSample {
            timestamp_ns: 5_000,
            percents: Percents::default(),
        });
        assert_eq!(s.publish(&mut never), PublishOutcome::NotWanted);
        // the guest did not ask for the record
        let s = linked(&clock, 0x1000, l580(), false);
        s.push(GuestSample {
            timestamp_ns: 5_000,
            percents: Percents::default(),
        });
        assert_eq!(s.publish(&mut never), PublishOutcome::NotWanted);
        // no sample yet: the record stays invalid
        let s = linked(&clock, 0x1000, l580(), true);
        assert_eq!(s.publish(&mut never), PublishOutcome::NoSample);
        // a stale sample
        s.push(GuestSample {
            timestamp_ns: 1,
            percents: Percents::default(),
        });
        clock.advance(FRESH_FOR + Duration::from_secs(1));
        assert_eq!(s.publish(&mut never), PublishOutcome::NoSample);
        // no layout (a guest version without a RUSD row)
        let s = UtilizationState::new(clock.clone());
        s.rusd_phys.store(0x1000, Ordering::Relaxed);
        s.rusd_wants.store(true, Ordering::Relaxed);
        s.push(GuestSample {
            timestamp_ns: clock.now_ns(),
            percents: Percents::default(),
        });
        assert_eq!(s.publish(&mut never), PublishOutcome::NotWanted);
    }

    #[test]
    fn a_refused_write_is_counted_and_never_retried_in_a_loop() {
        let clock = FakeClock::at(5_000);
        let state = linked(&clock, 0xdead_0000, l580(), true);
        state.push(GuestSample {
            timestamp_ns: 5_000,
            percents: Percents::default(),
        });
        let mut n = 0;
        let out = state.publish(&mut |_, _| {
            n += 1;
            false
        });
        assert_eq!(out, PublishOutcome::Refused);
        assert_eq!(n, 1, "the first refusal ends the attempt");
        assert_eq!(state.counters().publish_refused, 1);
        assert_eq!(
            state.publish(&mut |_, _| panic!("not due until the next period")),
            PublishOutcome::NotDue
        );
    }

    #[test]
    fn an_address_near_the_top_of_u64_cannot_wrap() {
        let clock = FakeClock::at(5_000);
        let state = linked(&clock, u64::MAX - 100, l580(), true);
        state.push(GuestSample {
            timestamp_ns: 5_000,
            percents: Percents::default(),
        });
        assert_eq!(
            state.publish(&mut |_, _| panic!("no write")),
            PublishOutcome::Refused
        );
    }

    #[test]
    fn the_595_layout_is_published_at_its_own_offset() {
        let clock = FakeClock::at(5_000);
        let l = ku::layout(kf_abi::DriverVersion {
            major: 595,
            minor: 84,
            patch: 0,
        })
        .unwrap();
        let state = linked(&clock, 0x8000, l, true);
        state.push(GuestSample {
            timestamp_ns: 5_000,
            percents: Percents::default(),
        });
        let mut first = 0;
        state.publish(&mut |gpa, _| {
            if first == 0 {
                first = gpa;
            }
            true
        });
        assert_eq!(first, 0x8000 + 1992);
    }

    #[test]
    fn a_host_that_repeats_its_timestamp_never_refreshes_the_snapshot() {
        let (clock, state, mut s, _) = rig((0..6).map(|_| Ok(reading(9, 50))).collect());
        state.note_interest();
        let _ = s.tick();
        for _ in 0..5 {
            clock.advance(SAMPLE_PERIOD);
            let _ = s.tick();
        }
        assert_eq!(
            state.fresh_samples().map(|v| v.len()),
            Some(1),
            "five repeats of the same host timestamp stored nothing"
        );
        assert_eq!(state.counters().host_stalled, 5);
    }

    #[test]
    fn publish_refuses_a_clock_in_the_seqlocks_reserved_range() {
        let clock = FakeClock::at(0xFF00_0000_0000_0000);
        let state = linked(&clock, 0x40_0000, l580(), true);
        state.push(GuestSample {
            timestamp_ns: clock.now_ns(),
            percents: Percents::default(),
        });
        assert_eq!(
            state.publish(&mut |_, _| panic!("no write")),
            PublishOutcome::NoSample
        );
    }

    #[test]
    fn an_address_that_wraps_to_zero_at_a_generated_offset_is_refused() {
        let clock = FakeClock::at(5_000);
        for phys in [u64::MAX - 1975, u64::MAX - 1980, u64::MAX - 1000, u64::MAX] {
            let state = linked(&clock, phys, l580(), true);
            state.push(GuestSample {
                timestamp_ns: 5_000,
                percents: Percents::default(),
            });
            assert_eq!(
                state.publish(&mut |g, _| panic!("wrapped write to {g:#x}")),
                PublishOutcome::Refused,
                "{phys:#x}"
            );
        }
    }

    #[test]
    fn a_refused_info_write_stops_before_the_valid_timestamp() {
        let clock = FakeClock::at(5_000);
        let state = linked(&clock, 0x40_0000, l580(), true);
        state.push(GuestSample {
            timestamp_ns: 5_000,
            percents: Percents::default(),
        });
        let mut seen = Vec::new();
        let out = state.publish(&mut |g, b| {
            seen.push((g, b.len()));
            seen.len() < 2
        });
        assert_eq!(out, PublishOutcome::Refused);
        assert_eq!(
            seen.len(),
            2,
            "begin, then the refused info write; the valid timestamp is never written"
        );
    }

    #[test]
    fn the_ring_never_holds_more_than_72_whatever_is_pushed() {
        let clock = FakeClock::at(1);
        let state = UtilizationState::new(clock.clone());
        for i in 0..10_000u64 {
            clock.advance(Duration::from_nanos(1));
            state.push(GuestSample {
                timestamp_ns: clock.now_ns(),
                percents: Percents::default(),
            });
            assert!(
                state.fresh_samples().is_some_and(|v| v.len() <= RING_MAX),
                "after {i} pushes"
            );
        }
        assert_eq!(state.fresh_samples().unwrap().len(), RING_MAX);
    }

    #[test]
    fn a_host_timestamp_that_goes_backwards_is_a_restart_not_a_stall() {
        let (clock, state, mut s, _) = rig(vec![
            Ok(reading(1_000, 40)),
            Ok(reading(5, 60)),
            Ok(reading(5, 61)),
        ]);
        state.note_interest();
        assert_eq!(s.tick(), TickOutcome::Sampled);
        clock.advance(SAMPLE_PERIOD);
        assert_eq!(
            s.tick(),
            TickOutcome::Sampled,
            "a restarted host timer is a new baseline"
        );
        clock.advance(SAMPLE_PERIOD);
        assert_eq!(
            s.tick(),
            TickOutcome::Stalled,
            "and the same value again is a stall"
        );
        assert_eq!(
            state.fresh_samples().unwrap().last().unwrap().percents.gpu,
            60
        );
    }

    #[test]
    fn a_stale_snapshot_invalidates_the_published_record_once() {
        let clock = FakeClock::at(5_000);
        let state = linked(&clock, 0x40_0000, l580(), true);
        state.push(GuestSample {
            timestamp_ns: 5_000,
            percents: Percents::default(),
        });
        let writes = std::cell::RefCell::new(Vec::<(u64, Vec<u8>)>::new());
        let mut w = |g: u64, b: &[u8]| {
            writes.borrow_mut().push((g, b.to_vec()));
            true
        };
        assert_eq!(state.publish(&mut w), PublishOutcome::Written);
        writes.borrow_mut().clear();
        clock.advance(FRESH_FOR + Duration::from_secs(1));
        assert_eq!(state.publish(&mut w), PublishOutcome::Invalidated);
        assert_eq!(
            *writes.borrow(),
            vec![(0x40_0000 + 1976, 0u64.to_le_bytes().to_vec())],
            "the timestamp word only, set to 0"
        );
        clock.advance(Duration::from_secs(1));
        assert_eq!(state.publish(&mut w), PublishOutcome::NoSample, "once");
        assert_eq!(writes.borrow().len(), 1);
        assert_eq!(state.counters().invalidated, 1);
    }

    #[test]
    fn a_refused_invalidation_is_counted_and_not_retried() {
        let clock = FakeClock::at(5_000);
        let state = linked(&clock, 0x40_0000, l580(), true);
        state.push(GuestSample {
            timestamp_ns: 5_000,
            percents: Percents::default(),
        });
        assert_eq!(state.publish(&mut |_, _| true), PublishOutcome::Written);
        clock.advance(FRESH_FOR + Duration::from_secs(1));
        assert_eq!(state.publish(&mut |_, _| false), PublishOutcome::Refused);
        assert_eq!(
            state.publish(&mut |_, _| panic!("not retried")),
            PublishOutcome::NoSample
        );
    }

    #[test]
    fn forgetting_the_guest_leaves_no_link_no_wish_no_layout_and_no_record() {
        let clock = FakeClock::at(5_000);
        let state = linked(&clock, 0x40_0000, l580(), true);
        state.push(GuestSample {
            timestamp_ns: 5_000,
            percents: Percents::default(),
        });
        assert_eq!(state.publish(&mut |_, _| true), PublishOutcome::Written);
        state.forget_guest();
        assert_eq!(state.rusd_page(), None);
        assert!(!state.rusd_wants_utilization());
        clock.advance(FRESH_FOR + Duration::from_secs(1));
        assert_eq!(
            state.publish(&mut |_, _| panic!("a gone guest is never written to")),
            PublishOutcome::NotWanted
        );
    }

    #[test]
    fn a_rebuild_for_a_version_without_a_row_clears_the_link_and_seats_nothing() {
        let clock = FakeClock::at(5_000);
        let state = linked(&clock, 0x40_0000, l580(), true);
        assert_eq!(state.rusd_page(), Some(0x40_0000));
        let no_row = kf_abi::versions::table_for(kf_abi::DriverVersion {
            major: 570,
            minor: 86,
            patch: 15,
        })
        .expect("a wire table");
        assert!(
            UtilizationPolicy::new(
                *no_row,
                &UtilizationSeat {
                    state: state.clone()
                }
            )
            .is_none()
        );
        assert_eq!(
            state.rusd_page(),
            None,
            "no link survives a rebuild that seats nothing"
        );
        assert!(!state.rusd_wants_utilization());
        state.push(GuestSample {
            timestamp_ns: 5_000,
            percents: Percents::default(),
        });
        assert_eq!(
            state.publish(&mut |_, _| panic!("no write")),
            PublishOutcome::NotWanted
        );
    }

    #[test]
    fn after_a_host_timer_restart_the_snapshot_keeps_following_the_host() {
        let (clock, state, mut s, _) = rig(vec![
            Ok(reading(900, 10)),
            Ok(reading(2, 20)),
            Ok(reading(3, 30)),
            Ok(reading(4, 40)),
        ]);
        state.note_interest();
        for _ in 0..4 {
            assert_eq!(s.tick(), TickOutcome::Sampled);
            clock.advance(SAMPLE_PERIOD);
        }
        assert_eq!(state.fresh_samples().unwrap().len(), 4);
    }

    #[test]
    fn interest_follows_asks_and_a_polled_page() {
        let clock = FakeClock::at(1_000);
        let s = UtilizationState::new(clock.clone());
        assert!(!s.interested());
        s.note_interest();
        assert!(s.interested());
        clock.advance(INTEREST_WINDOW + Duration::from_secs(1));
        assert!(!s.interested());
        s.rusd_phys.store(0x1000, Ordering::Relaxed);
        assert!(!s.interested(), "a page without a poll is not interest");
        s.rusd_wants.store(true, Ordering::Relaxed);
        assert!(s.interested(), "a polled page is");
    }
}
