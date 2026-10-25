//! Default-off accepted GPU mapping census. Mapping decisions never use it.
//! Counts include aliases and remaps, not unique resident VRAM. Page sizes are
//! the guest's leaf sizes, even when the host splits a row into smaller mappings.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

const BINS: usize = 256 * 256 * 2 * 5;
const SIZES: [&str; 5] = ["4K", "64K", "2M", "unknown", "other"];

fn size(leaf: u64) -> usize {
    match leaf {
        4096 => 0,
        65536 => 1,
        2097152 => 2,
        0 => 3,
        _ => 4,
    }
}

fn index(raw: u8, host: u8, ram: bool, leaf: u64) -> usize {
    usize::from(raw)
        .saturating_mul(256)
        .saturating_add(usize::from(host))
        .saturating_mul(2)
        .saturating_add(usize::from(ram))
        .saturating_mul(5)
        .saturating_add(size(leaf))
}

/// Per-device fixed-bound atomic census; storage exists only with explicit opt-in.
pub struct Census {
    bytes: Box<[AtomicU64]>,
    previous: Box<[AtomicU64]>,
    previous_at: AtomicU64,
    born: Instant,
    finished: AtomicBool,
}

impl Default for Census {
    fn default() -> Self {
        Self::new()
    }
}

impl Census {
    /// No environment access occurs on map paths.
    #[must_use]
    pub fn from_setting(value: Option<&str>) -> Option<Self> {
        (value == Some("1")).then(Self::new)
    }
    /// Arrays are heap-initialized, never large stack temporaries.
    #[must_use]
    pub fn new() -> Self {
        Self {
            bytes: (0..BINS).map(|_| AtomicU64::new(0)).collect(),
            previous: (0..BINS).map(|_| AtomicU64::new(0)).collect(),
            previous_at: AtomicU64::new(0),
            born: Instant::now(),
            finished: AtomicBool::new(false),
        }
    }
    /// Accepted GPU row bytes after final map verdict, excluding CPU views.
    pub fn record_mapping(&self, raw: u8, host: u8, ram: bool, leaf: u64, bytes: u64) {
        if let Some(bin) = self.bytes.get(index(raw, host, ram, leaf)) {
            let _ = bin.try_update(Ordering::Relaxed, Ordering::Relaxed, |old| {
                Some(old.saturating_add(bytes))
            });
        }
    }
    /// Independently atomic snapshot; exact when producers are quiescent.
    #[must_use]
    pub fn snapshot(&self, at_ns: u64) -> Snapshot {
        Snapshot {
            at_ns,
            bytes: self
                .bytes
                .iter()
                .map(|v| v.load(Ordering::Relaxed))
                .collect(),
        }
    }
    /// Cumulative counts and rates since preceding status report, in one bounded JSON line.
    #[must_use]
    pub fn report(&self) -> String {
        let now = u64::try_from(self.born.elapsed().as_nanos()).unwrap_or(u64::MAX);
        let elapsed = now.saturating_sub(self.previous_at.swap(now, Ordering::Relaxed));
        let mut rows = Vec::new();
        let mut total = 0u64;
        let mut compressible = 0u64;
        let mut stripped = 0u64;
        for (i, (bin, previous)) in self.bytes.iter().zip(&self.previous).enumerate() {
            let bytes = bin.load(Ordering::Relaxed);
            let old = previous.swap(bytes, Ordering::Relaxed);
            if bytes == 0 {
                continue;
            }
            let raw = (i / 2560) as u8;
            let host = ((i / 10) % 256) as u8;
            let ram = (i / 5) % 2 != 0;
            let page = SIZES.get(i % 5).copied().unwrap_or("other");
            total = total.saturating_add(bytes);
            if kf_chip::uncompressed_pte_kind(raw).is_some_and(|k| k != raw) {
                compressible = compressible.saturating_add(bytes);
                if raw != host {
                    stripped = stripped.saturating_add(bytes);
                }
            }
            let rate = if elapsed == 0 {
                0.0
            } else {
                bytes.saturating_sub(old) as f64 * 1e9 / elapsed as f64
            };
            rows.push(format!(
                concat!(
                    "{{\"raw\":{},\"host\":{},\"ram\":{},\"page\":\"{}\",",
                    "\"bytes\":{},\"pages4k\":{},\"bytes_per_s\":{:.3},\"pages4k_per_s\":{:.3}}}"
                ),
                raw,
                host,
                ram,
                page,
                bytes,
                bytes / 4096,
                rate,
                rate / 4096.0
            ));
        }
        format!(
            concat!(
                "{{\"at_ns\":{},\"window_ns\":{},\"mapped_bytes\":{},",
                "\"compressible_bytes\":{},\"stripped_bytes\":{},\"rows\":[{}]}}"
            ),
            now,
            elapsed,
            total,
            compressible,
            stripped,
            rows.join(",")
        )
    }
    /// At most one shutdown-request summary.
    #[must_use]
    pub fn shutdown(&self) -> Option<String> {
        (!self.finished.swap(true, Ordering::Relaxed)).then(|| self.report())
    }
}

/// Counts over a monotonic endpoint or explicit elapsed window.
pub struct Snapshot {
    /// Endpoint, or elapsed nanoseconds after subtraction.
    pub at_ns: u64,
    bytes: Vec<u64>,
}

impl Snapshot {
    /// Bytes for a raw/host/aperture/guest-leaf bin.
    #[must_use]
    pub fn bytes(&self, raw: u8, host: u8, ram: bool, leaf: u64) -> u64 {
        self.bytes
            .get(index(raw, host, ram, leaf))
            .copied()
            .unwrap_or(0)
    }
    /// Equivalent 4 KiB pages, independent of run count and guest leaf size.
    #[must_use]
    pub fn pages4k(&self, raw: u8, host: u8, ram: bool, leaf: u64) -> u64 {
        self.bytes(raw, host, ram, leaf) / 4096
    }
    /// Subtract cumulative counts while retaining kind identities.
    #[must_use]
    pub fn since(&self, earlier: &Self) -> Self {
        Self {
            at_ns: self.at_ns.saturating_sub(earlier.at_ns),
            bytes: self
                .bytes
                .iter()
                .zip(&earlier.bytes)
                .map(|(a, b)| a.saturating_sub(*b))
                .collect(),
        }
    }
    /// Rate over this snapshot's elapsed interval.
    #[must_use]
    pub fn bytes_per_second(&self, raw: u8, host: u8, ram: bool, leaf: u64) -> Option<f64> {
        (self.at_ns != 0).then(|| self.bytes(raw, host, ram, leaf) as f64 * 1e9 / self.at_ns as f64)
    }
}
