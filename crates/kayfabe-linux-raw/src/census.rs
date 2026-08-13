//! ★★★ **The ioctl census — every `ioctl(2)` this process issues, counted at the syscall.**
//!
//! ## Why it lives HERE and not in the RM backend
//!
//! `kayfabe-isolate-host` knows what it *meant* to do. This crate knows what the kernel was
//! *asked* to do, and those are different numbers — the campaign has a recorded instance of a
//! counter that "counts our own INTENT" being read as a count of work. [`CharDevice::ioctl`]
//! is the single funnel through which every RM ioctl in the workspace passes, and a census
//! placed there **cannot be bypassed by a call site that forgot to register**. That is the
//! whole property: an unlabelled ioctl still increments the total, so a phase that under-reports
//! its own work shows up as a gap between the phase subtotals and the grand total.
//!
//! ## ⊘ What it deliberately does NOT record
//!
//! `l1_os_shell.md` §4.7 — **no business logic in this crate.** So the record carries only
//! `ioctl(2)`-ABI facts: the request's `_IOC_NR`, its `_IOC_SIZE`, and the outcome. It does
//! **not** decode `hClass` out of an `NVOS21` buffer or `cmd` out of an `NVOS54` one; that
//! vocabulary belongs to `kayfabe-abi`, and a decoder here would be this crate growing an
//! opinion about NVIDIA's structs.
//! ⇒ Callers that want NV-level names decode `nr` themselves against their own `NV_ESC_*`
//! constants. The **phase label** ([`phase`]) is the seam for anything finer.
//!
//! ## ⚠ It is a diagnostic, and it says so
//!
//! - The **total** is always maintained (one relaxed atomic add per ioctl; nothing measurable).
//! - The **ordered log** is off until [`record_sequence`] turns it on, and it is **bounded**
//!   ([`LOG_CAPACITY`]). A capped log reports its own truncation through [`Census::dropped`],
//!   because a silently truncated sequence is a sequence that reads as complete.
//! - It is **process-global**, so a multi-threaded run interleaves phases. The ladder that
//!   consumes it is single-threaded through the rungs that matter; anything else must read
//!   the totals and ignore the phase attribution.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// How many records the ordered log holds before it starts dropping.
///
/// ★ Sized for the raw-CE-client rung, whose whole point is that it is ~20 ioctls, with four
/// orders of magnitude of headroom so a full ladder run also fits. A run that exceeds it
/// reports [`Census::dropped`] rather than quietly presenting a prefix as the whole.
pub const LOG_CAPACITY: usize = 65_536;

/// One ioctl, as the kernel saw it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IoctlRecord {
    /// Its position in the process-wide sequence, from 1.
    pub seq: u64,
    /// `_IOC_NR(request)` — for the NVIDIA frontend this is the `NV_ESC_*` number.
    pub nr: u8,
    /// `_IOC_TYPE(request)` — the driver magic (`'F'` = `0x46` for the NVIDIA frontend).
    pub magic: u8,
    /// `_IOC_SIZE(request)` — the byte count the driver copies in both directions.
    pub size: u16,
    /// The caller-declared phase in force when it was issued; `""` if none was set.
    pub phase: &'static str,
    /// `0` when the syscall returned `>= 0`, otherwise the `errno`.
    pub errno: i32,
}

/// A snapshot of the census, taken by [`snapshot`].
#[derive(Clone, Debug)]
pub struct Census {
    /// Every ioctl issued since the last [`reset`], counted at the syscall. Always exact —
    /// this number does not depend on the log being enabled or on it having room.
    pub total: u64,
    /// How many failed (`errno != 0`).
    pub failed: u64,
    /// The ordered log, empty unless [`record_sequence`] was on.
    pub log: Vec<IoctlRecord>,
    /// ⚠ How many records the bounded log had to drop. Non-zero means [`Census::log`] is a
    /// **prefix**, not the sequence.
    pub dropped: u64,
}

impl Census {
    /// The distinct `(magic, nr)` pairs seen, each with how many times it occurred and how
    /// many of those failed, in first-appearance order.
    ///
    /// ⊘ Derived from [`Census::log`], so it is only complete when the log was enabled and
    /// [`Census::dropped`] is zero.
    #[must_use]
    pub fn by_request(&self) -> Vec<((u8, u8), u64, u64)> {
        let mut out: Vec<((u8, u8), u64, u64)> = Vec::new();
        for r in &self.log {
            let key = (r.magic, r.nr);
            if let Some(e) = out.iter_mut().find(|e| e.0 == key) {
                e.1 += 1;
                e.2 += u64::from(r.errno != 0);
            } else {
                out.push((key, 1, u64::from(r.errno != 0)));
            }
        }
        out
    }

    /// Per-phase subtotals in first-appearance order.
    ///
    /// ★ Read these against [`Census::total`]. A shortfall is not an accounting error; it is
    /// ioctls issued outside any phase, which is exactly what an uninstrumented call site
    /// looks like.
    #[must_use]
    pub fn by_phase(&self) -> Vec<(&'static str, u64)> {
        let mut out: Vec<(&'static str, u64)> = Vec::new();
        for r in &self.log {
            if let Some(e) = out.iter_mut().find(|e| e.0 == r.phase) {
                e.1 += 1;
            } else {
                out.push((r.phase, 1));
            }
        }
        out
    }
}

static TOTAL: AtomicU64 = AtomicU64::new(0);
static FAILED: AtomicU64 = AtomicU64::new(0);
static DROPPED: AtomicU64 = AtomicU64::new(0);
static RECORDING: AtomicU64 = AtomicU64::new(0);
static LOG: Mutex<Vec<IoctlRecord>> = Mutex::new(Vec::new());
static PHASE: Mutex<&'static str> = Mutex::new("");

/// Turn the ordered log on or off. The running total is unaffected — it is always on.
pub fn record_sequence(on: bool) {
    RECORDING.store(u64::from(on), Ordering::Relaxed);
}

/// Name the phase every subsequent ioctl is attributed to, until the next call.
///
/// ★ Process-global by design: the funnel it annotates is process-global too, and a
/// thread-local label would silently attribute a worker's ioctls to `""` while looking like
/// it worked.
pub fn phase(name: &'static str) {
    if let Ok(mut p) = PHASE.lock() {
        *p = name;
    }
}

/// Clear every counter and the log. Call before the measurement, never during it.
pub fn reset() {
    TOTAL.store(0, Ordering::Relaxed);
    FAILED.store(0, Ordering::Relaxed);
    DROPPED.store(0, Ordering::Relaxed);
    if let Ok(mut l) = LOG.lock() {
        l.clear();
    }
    phase("");
}

/// Read the census out.
#[must_use]
pub fn snapshot() -> Census {
    Census {
        total: TOTAL.load(Ordering::Relaxed),
        failed: FAILED.load(Ordering::Relaxed),
        log: LOG.lock().map(|l| l.clone()).unwrap_or_default(),
        dropped: DROPPED.load(Ordering::Relaxed),
    }
}

/// Record one ioctl. Called from [`CharDevice::ioctl`] and nowhere else.
///
/// [`CharDevice::ioctl`]: crate::CharDevice::ioctl
pub(crate) fn note(request: u64, errno: i32) {
    let seq = TOTAL.fetch_add(1, Ordering::Relaxed) + 1;
    if errno != 0 {
        FAILED.fetch_add(1, Ordering::Relaxed);
    }
    if RECORDING.load(Ordering::Relaxed) == 0 {
        return;
    }
    // ⊘ The phase is read under its own lock and the log under its own; neither is held
    // across the other, and neither is ever held across a syscall — `note` runs strictly
    // after the `ioctl(2)` has returned.
    let phase = PHASE.lock().map(|p| *p).unwrap_or("");
    let Ok(mut log) = LOG.lock() else { return };
    if log.len() >= LOG_CAPACITY {
        DROPPED.fetch_add(1, Ordering::Relaxed);
        return;
    }
    log.push(IoctlRecord {
        seq,
        nr: crate::ioctl::nr_of(request),
        magic: crate::ioctl::magic_of(request),
        size: u16::try_from(crate::ioctl::declared_size(request)).unwrap_or(u16::MAX),
        phase,
        errno,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CharDevice, DevDir, ioctl};

    /// `/dev/null` answers every ioctl with `ENOTTY`, which is what makes it the right
    /// fixture: the census must count a **refused** ioctl exactly like a served one, because
    /// *"how many times did we enter the driver"* is the question it answers.
    ///
    /// ⚠ **ONE test, deliberately, and not two.** Every counter here is process-global and
    /// `cargo test` runs test functions on many threads, so a second test toggling
    /// [`record_sequence`] would flip the log off underneath this one — a flake that reads as
    /// *"the record was not written"*, which is the exact false negative this instrument
    /// exists to avoid. Splitting it is not a style choice; it is a race.
    #[test]
    fn the_census_counts_at_the_syscall_logs_in_order_and_survives_the_log_being_off() {
        let dir = DevDir::open(c"/dev").expect("/dev exists on a Linux host");
        let d = CharDevice::openat(&dir, c"null").expect("/dev/null exists on a Linux host");
        let mut arg = [0u8; 8];

        // (a) with the log ON: the record is written, and it carries every field.
        record_sequence(true);
        phase("census-test");
        let before = snapshot().total;
        let req = ioctl::readwrite(0x46, 200, 8).expect("8 bytes encodes");
        assert!(
            d.ioctl(req, &mut arg, &mut []).is_err(),
            "/dev/null ENOTTYs"
        );
        let after = snapshot();
        assert!(after.total > before, "the total moved");
        let mine: Vec<_> = after
            .log
            .iter()
            .filter(|r| r.phase == "census-test" && r.nr == 200)
            .collect();
        assert_eq!(mine.len(), 1, "exactly one record, and it is in the log");
        assert_eq!(mine[0].magic, 0x46, "the driver magic is carried");
        assert_eq!(mine[0].size, 8, "the copy length is carried");
        assert_ne!(mine[0].errno, 0, "a REFUSAL is recorded AS a refusal");
        assert_eq!(
            mine[0].seq,
            before + 1,
            "the sequence number is the process-wide position"
        );

        // (b) with the log OFF: the total still moves. That is what makes a phase shortfall
        // readable as "ioctls outside any phase" rather than as "the log was off".
        record_sequence(false);
        let before_off = snapshot().total;
        let req = ioctl::readwrite(0x46, 201, 8).expect("8 bytes encodes");
        let _ = d.ioctl(req, &mut arg, &mut []);
        let off = snapshot();
        assert_eq!(off.total, before_off + 1, "counted with the log off");
        assert!(
            !off.log.iter().any(|r| r.nr == 201),
            "and NOT logged — the two switches are independent"
        );
        phase("");
    }
}
