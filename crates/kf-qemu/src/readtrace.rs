//! ★ DIAGNOSTIC, default OFF (`KF3_BAR0_READ_TRACE=1`): the BAR0 access trace in the VFIO
//! reference's own record format (owner ruling 2026-10-09, `docs/OWNER_RULINGS.md` §X;
//! `docs/design/V3_BAR0_TRACE_MODE.md`).
//!
//! The VFIO reference (`scripts/bench/windows/vfio_dvi_reference.sh`, patched QEMU with
//! `x-gsp-observer`) records every BAR0 access through QEMU's own trace events
//! `vfio_region_read` / `vfio_region_write` and every interrupt through `vfio_msi_interrupt`
//! (`-trace events=…,file=…` with `-msg timestamp=on`). With this mode on, the kf3 C device emits
//! the SAME trace events through the SAME QEMU trace code (`trace_vfio_region_read` …), so one
//! analyser reads both boots. This module is only the policy the C device asks before each record:
//!
//! - **what is selected** — read ranges (the display range `0x610000..=0x6fffff` plus
//!   `KF3_READ_TRACE_RANGES`, or `all`) and write ranges (`KF3_WRITE_TRACE_RANGES`, default `all`);
//!   which shadow pieces stop serving reads from RAM ([`AccessTrace::piece_traps`]);
//! - **the hard caps** — records and bytes (`KF3_READ_TRACE_MAX_RECORDS`,
//!   `KF3_READ_TRACE_MAX_BYTES`, default 512 MiB), counted with the exact length of the line QEMU's
//!   log backend writes (the timestamp at its longest), so the file cannot outgrow the byte cap
//!   because of kf3's records. The first refusal SPENDS the trace: every later record is dropped
//!   and counted ([`AccessTrace::admit`]);
//! - **the report** at exit (records, bytes, drops, whether the cap was hit).
//!
//! ⊘ Off (the default), nothing here is consulted: the C device asks [`AccessTrace::on`] once at
//! realize and, when it is false, builds BAR0, the read path and the interrupt path exactly as
//! before (ROMD on, irqfd MSI, no trace call). Lock-free: atomics only, no allocation, no I/O.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// The display range the read trace selects by default (NV_PDISP and its neighbours).
pub const DISPLAY_RANGE: Range = Range {
    lo: 0x0061_0000,
    hi: 0x006f_ffff,
};
/// Default byte cap: 512 MiB of trace lines.
pub const DEFAULT_MAX_BYTES: u64 = 512 << 20;
/// Default record cap.
pub const DEFAULT_MAX_RECORDS: u64 = 8_000_000;
/// The largest byte cap accepted (a typo must not ask for a disk's worth).
pub const MAX_MAX_BYTES: u64 = 16 << 30;
/// At most this many ranges per list.
pub const MAX_RANGES: usize = 32;
/// The longest device name accepted in a record.
pub const MAX_NAME: usize = 32;
/// The timestamp QEMU's log backend prefixes with `-msg timestamp=on`, at its longest:
/// `2026-10-08T20:10:01.265690Z ` (`g_date_time_format_iso8601` + one space; util/log.c).
pub const TIMESTAMP_BYTES: u64 = 28;

/// An inclusive BAR0 offset range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    /// First byte.
    pub lo: u64,
    /// Last byte (inclusive).
    pub hi: u64,
}

/// A selection: nothing, everything, or up to [`MAX_RANGES`] ranges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ranges {
    /// Nothing selected.
    None,
    /// The whole BAR.
    All,
    /// These ranges (inclusive).
    List(Vec<Range>),
}

impl Ranges {
    /// Whether the access `[off, off + width)` touches the selection.
    #[must_use]
    pub fn hits(&self, off: u64, width: u64) -> bool {
        let last = off.saturating_add(width.max(1) - 1);
        match self {
            Ranges::None => false,
            Ranges::All => true,
            Ranges::List(v) => v.iter().any(|r| off <= r.hi && last >= r.lo),
        }
    }

    /// Parse a list: `all`, `none`, or comma-separated `A-B` (inclusive) / `A` (one 4-byte
    /// register), hex with or without `0x`. Refuses by name: empty items, `A > B`, more than
    /// [`MAX_RANGES`] items.
    pub fn parse(s: &str) -> Result<Ranges, String> {
        let s = s.trim();
        if s.eq_ignore_ascii_case("all") {
            return Ok(Ranges::All);
        }
        if s.eq_ignore_ascii_case("none") || s.is_empty() {
            return Ok(Ranges::None);
        }
        let mut v = Vec::new();
        for item in s.split(',') {
            let item = item.trim();
            if item.eq_ignore_ascii_case("all") {
                return Ok(Ranges::All);
            }
            let (lo, hi) = match item.split_once('-') {
                Some((a, b)) => (hex(a)?, hex(b)?),
                None => {
                    let a = hex(item)?;
                    (a, a.saturating_add(3))
                }
            };
            if lo > hi {
                return Err(format!("range {item:?}: start above end"));
            }
            v.push(Range { lo, hi });
            if v.len() > MAX_RANGES {
                return Err(format!("more than {MAX_RANGES} ranges"));
            }
        }
        Ok(Ranges::List(v))
    }

    /// The union with `r`.
    #[must_use]
    pub fn with(self, r: Range) -> Ranges {
        match self {
            Ranges::All => Ranges::All,
            Ranges::None => Ranges::List(vec![r]),
            Ranges::List(mut v) => {
                if !v.contains(&r) {
                    v.insert(0, r);
                }
                Ranges::List(v)
            }
        }
    }

    fn label(&self) -> String {
        match self {
            Ranges::None => "none".into(),
            Ranges::All => "all".into(),
            Ranges::List(v) => v
                .iter()
                .map(|r| format!("{:#x}-{:#x}", r.lo, r.hi))
                .collect::<Vec<_>>()
                .join(","),
        }
    }
}

fn hex(s: &str) -> Result<u64, String> {
    let t = s.trim();
    let t = t
        .strip_prefix("0x")
        .or_else(|| t.strip_prefix("0X"))
        .unwrap_or(t);
    if t.is_empty() || t.len() > 16 {
        return Err(format!("{s:?} is not a hex offset"));
    }
    u64::from_str_radix(t, 16).map_err(|_| format!("{s:?} is not a hex offset"))
}

/// A size: decimal with an optional `K`, `M` or `G` suffix (binary).
fn size(s: &str) -> Result<u64, String> {
    let t = s.trim();
    let (num, mul) = match t.chars().last() {
        Some('k' | 'K') => (&t[..t.len() - 1], 1u64 << 10),
        Some('m' | 'M') => (&t[..t.len() - 1], 1 << 20),
        Some('g' | 'G') => (&t[..t.len() - 1], 1 << 30),
        _ => (t, 1),
    };
    num.parse::<u64>()
        .ok()
        .and_then(|n| n.checked_mul(mul))
        .ok_or_else(|| format!("{s:?} is not a size"))
}

/// The configuration, read from the environment once at realize.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceConfig {
    /// `KF3_BAR0_READ_TRACE=1`. False = the trace does not exist.
    pub on: bool,
    /// Reads that are logged.
    pub reads: Ranges,
    /// Writes that are logged.
    pub writes: Ranges,
    /// Byte cap.
    pub max_bytes: u64,
    /// Record cap.
    pub max_records: u64,
    /// `KF3_TRACE_NAME`: the device name in the records (default: the host GPU's PCI address,
    /// which is what the VFIO reference prints for the same card).
    pub name: Option<String>,
}

impl TraceConfig {
    /// Off: what every default, production and performance configuration gets.
    #[must_use]
    pub fn off() -> TraceConfig {
        TraceConfig {
            on: false,
            reads: Ranges::None,
            writes: Ranges::None,
            max_bytes: 0,
            max_records: 0,
            name: None,
        }
    }

    /// Parse from a variable lookup (the environment in production; a map in tests). Unset or
    /// `0` = off; `1`/`on` = on; anything else is refused by name, as is any malformed knob while
    /// it is on (a typo never silently turns a diagnostic off or unbounded).
    pub fn parse(get: impl Fn(&str) -> Option<String>) -> Result<TraceConfig, String> {
        match get("KF3_BAR0_READ_TRACE").as_deref().map(str::trim) {
            None | Some("" | "0" | "off") => return Ok(TraceConfig::off()),
            Some("1" | "on") => {}
            Some(v) => return Err(format!("KF3_BAR0_READ_TRACE={v:?}: the values are 0 and 1")),
        }
        let reads = match get("KF3_READ_TRACE_RANGES") {
            Some(s) => Ranges::parse(&s)
                .map_err(|e| format!("KF3_READ_TRACE_RANGES: {e}"))?
                .with(DISPLAY_RANGE),
            None => Ranges::List(vec![DISPLAY_RANGE]),
        };
        let writes = match get("KF3_WRITE_TRACE_RANGES") {
            Some(s) => Ranges::parse(&s).map_err(|e| format!("KF3_WRITE_TRACE_RANGES: {e}"))?,
            None => Ranges::All,
        };
        let max_bytes = match get("KF3_READ_TRACE_MAX_BYTES") {
            Some(s) => size(&s).map_err(|e| format!("KF3_READ_TRACE_MAX_BYTES: {e}"))?,
            None => DEFAULT_MAX_BYTES,
        };
        if max_bytes == 0 || max_bytes > MAX_MAX_BYTES {
            return Err(format!(
                "KF3_READ_TRACE_MAX_BYTES={max_bytes}: must be 1..={MAX_MAX_BYTES}"
            ));
        }
        let max_records = match get("KF3_READ_TRACE_MAX_RECORDS") {
            Some(s) => size(&s).map_err(|e| format!("KF3_READ_TRACE_MAX_RECORDS: {e}"))?,
            None => DEFAULT_MAX_RECORDS,
        };
        if max_records == 0 {
            return Err("KF3_READ_TRACE_MAX_RECORDS=0: must be at least 1".into());
        }
        let name = match get("KF3_TRACE_NAME") {
            Some(n) if valid_name(&n) => Some(n),
            Some(n) => {
                return Err(format!(
                    "KF3_TRACE_NAME={n:?}: 1..={MAX_NAME} characters of [0-9A-Za-z:._-]"
                ));
            }
            None => None,
        };
        Ok(TraceConfig {
            on: true,
            reads,
            writes,
            max_bytes,
            max_records,
            name,
        })
    }

    /// From the process environment.
    pub fn from_env() -> Result<TraceConfig, String> {
        TraceConfig::parse(|k| std::env::var(k).ok())
    }
}

/// A device name the records may carry: no space, parenthesis or comma (the analysers split on
/// them), bounded.
#[must_use]
pub fn valid_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= MAX_NAME
        && n.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'.' | b'_' | b'-'))
}

/// What a record is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A BAR0 read: `(off, width, value)`.
    Read,
    /// A BAR0 write: `(off, width, value)`.
    Write,
    /// An MSI-X raise: `(vector, data, address)`.
    Msi,
}

impl Kind {
    /// From the C device's word (0 read, 1 write, 2 MSI).
    #[must_use]
    pub fn from_abi(k: u32) -> Option<Kind> {
        match k {
            0 => Some(Kind::Read),
            1 => Some(Kind::Write),
            2 => Some(Kind::Msi),
            _ => None,
        }
    }
}

fn hex_digits(v: u64) -> u64 {
    if v == 0 {
        1
    } else {
        u64::from((64 - v.leading_zeros()).div_ceil(4))
    }
}

fn dec_digits(v: u64) -> u64 {
    let mut n = 1;
    let mut v = v;
    while v >= 10 {
        v /= 10;
        n += 1;
    }
    n
}

/// The exact length of the line QEMU's log backend writes for this record (timestamp at its
/// longest), from the event's format in `hw/vfio/trace-events` (QEMU 10.2.4):
/// - `vfio_region_read  (%s:region%d+0x%PRIx64, %d) = 0x%PRIx64`
/// - `vfio_region_write  (%s:region%d+0x%PRIx64, 0x%PRIx64, %d)`
/// - `vfio_msi_interrupt  (%s) vector %d 0x%PRIx64/0x%x` (data is an `int`, printed `%x`)
#[must_use]
pub fn record_len(kind: Kind, name_len: u64, a: u64, b: u64, c: u64) -> u64 {
    TIMESTAMP_BYTES
        + 1 // '\n'
        + match kind {
            Kind::Read => {
                "vfio_region_read  (".len() as u64
                    + name_len
                    + ":region0+0x".len() as u64
                    + hex_digits(a)
                    + 2
                    + dec_digits(b)
                    + ") = 0x".len() as u64
                    + hex_digits(c)
            }
            Kind::Write => {
                "vfio_region_write  (".len() as u64
                    + name_len
                    + ":region0+0x".len() as u64
                    + hex_digits(a)
                    + ", 0x".len() as u64
                    + hex_digits(c)
                    + 2
                    + dec_digits(b)
                    + 1
            }
            Kind::Msi => {
                "vfio_msi_interrupt  (".len() as u64
                    + name_len
                    + ") vector ".len() as u64
                    + dec_digits(a)
                    + " 0x".len() as u64
                    + hex_digits(c)
                    + "/0x".len() as u64
                    + hex_digits(b & 0xffff_ffff)
            }
        }
}

/// The line QEMU writes for this record, given its timestamp (tests and documentation; the
/// device never formats — QEMU's trace code does).
#[must_use]
pub fn render(kind: Kind, ts: &str, name: &str, a: u64, b: u64, c: u64) -> String {
    match kind {
        Kind::Read => format!("{ts} vfio_region_read  ({name}:region0+{a:#x}, {b}) = {c:#x}\n"),
        Kind::Write => format!("{ts} vfio_region_write  ({name}:region0+{a:#x}, {c:#x}, {b})\n"),
        Kind::Msi => format!(
            "{ts} vfio_msi_interrupt  ({name}) vector {a} {c:#x}/{:#x}\n",
            b & 0xffff_ffff
        ),
    }
}

/// The trace's state: the configuration and lock-free counters.
#[derive(Debug)]
pub struct AccessTrace {
    cfg: TraceConfig,
    name: String,
    bytes: AtomicU64,
    records: AtomicU64,
    reads: AtomicU64,
    writes: AtomicU64,
    msis: AtomicU64,
    dropped: AtomicU64,
    unselected: AtomicU64,
    spent: AtomicBool,
}

impl AccessTrace {
    /// The trace for `cfg`, naming the device `default_name` unless `KF3_TRACE_NAME` overrides it
    /// (an invalid default falls back to `kf3`).
    #[must_use]
    pub fn new(cfg: TraceConfig, default_name: &str) -> AccessTrace {
        let name = cfg.name.clone().unwrap_or_else(|| {
            if valid_name(default_name) {
                default_name.to_string()
            } else {
                "kf3".to_string()
            }
        });
        AccessTrace {
            cfg,
            name,
            bytes: AtomicU64::new(0),
            records: AtomicU64::new(0),
            reads: AtomicU64::new(0),
            writes: AtomicU64::new(0),
            msis: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            unselected: AtomicU64::new(0),
            spent: AtomicBool::new(false),
        }
    }

    /// Whether the trace mode is on. False = the C device takes none of the trace paths.
    #[must_use]
    pub fn on(&self) -> bool {
        self.cfg.on
    }

    /// The device name the records carry.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether a shadow piece `[base, base + len)` must serve its reads by exit (it overlaps a
    /// selected read range). Always false when off.
    #[must_use]
    pub fn piece_traps(&self, base: u64, len: u64) -> bool {
        self.cfg.on && len > 0 && self.cfg.reads.hits(base, len)
    }

    /// ★ Asked by the C device before each record, on the vCPU (or main loop, for an MSI): log
    /// it? Selection first, then the caps: the first record that would cross either cap spends the
    /// trace, and from then on every record is dropped and counted. Lock-free (a CAS on the byte
    /// count), never blocks, never allocates.
    pub fn admit(&self, kind: Kind, a: u64, b: u64, c: u64) -> bool {
        if !self.cfg.on {
            return false;
        }
        let selected = match kind {
            Kind::Read => self.cfg.reads.hits(a, b),
            Kind::Write => self.cfg.writes.hits(a, b),
            Kind::Msi => true,
        };
        if !selected {
            self.unselected.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        if self.spent.load(Ordering::Acquire) {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        let len = record_len(kind, self.name.len() as u64, a, b, c);
        let mut cur = self.bytes.load(Ordering::Relaxed);
        loop {
            if cur.saturating_add(len) > self.cfg.max_bytes {
                return self.spend();
            }
            match self.bytes.compare_exchange_weak(
                cur,
                cur + len,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(v) => cur = v,
            }
        }
        if self.records.fetch_add(1, Ordering::AcqRel) >= self.cfg.max_records {
            self.records.fetch_sub(1, Ordering::AcqRel);
            self.bytes.fetch_sub(len, Ordering::AcqRel);
            return self.spend();
        }
        match kind {
            Kind::Read => &self.reads,
            Kind::Write => &self.writes,
            Kind::Msi => &self.msis,
        }
        .fetch_add(1, Ordering::Relaxed);
        true
    }

    fn spend(&self) -> bool {
        self.spent.store(true, Ordering::Release);
        self.dropped.fetch_add(1, Ordering::Relaxed);
        false
    }

    /// Records dropped after the cap.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Records admitted.
    #[must_use]
    pub fn records(&self) -> u64 {
        self.records.load(Ordering::Relaxed)
    }

    /// Bytes admitted (an upper bound of what QEMU wrote for them).
    #[must_use]
    pub fn bytes(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }

    /// The exit report (one line). `"off"` when the mode is off.
    #[must_use]
    pub fn report(&self) -> String {
        if !self.cfg.on {
            return "kf3: BAR0-TRACE off".into();
        }
        let o = Ordering::Relaxed;
        format!(
            "kf3: BAR0-TRACE report name={} records={} (cap {}) bytes<={} (cap {}) reads={} writes={} msi={} dropped={} cap_hit={} unselected_exits={} read_ranges=[{}] write_ranges=[{}]",
            self.name,
            self.records.load(o),
            self.cfg.max_records,
            self.bytes.load(o),
            self.cfg.max_bytes,
            self.reads.load(o),
            self.writes.load(o),
            self.msis.load(o),
            self.dropped.load(o),
            if self.spent.load(o) { "yes" } else { "no" },
            self.unselected.load(o),
            self.cfg.reads.label(),
            self.cfg.writes.label(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn cfg(vars: &[(&str, &str)]) -> Result<TraceConfig, String> {
        let m: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        TraceConfig::parse(|k| m.get(k).cloned())
    }

    fn on(extra: &[(&str, &str)]) -> AccessTrace {
        let mut v = vec![("KF3_BAR0_READ_TRACE", "1")];
        v.extend_from_slice(extra);
        AccessTrace::new(cfg(&v).unwrap(), "0000:01:00.0")
    }

    #[test]
    fn the_default_configuration_is_off_and_gates_every_path() {
        for vars in [
            &[][..],
            &[("KF3_BAR0_READ_TRACE", "0")][..],
            &[("KF3_BAR0_READ_TRACE", "")][..],
            // knobs without the switch do nothing
            &[
                ("KF3_READ_TRACE_RANGES", "all"),
                ("KF3_WRITE_TRACE_RANGES", "all"),
            ][..],
        ] {
            let c = cfg(vars).unwrap();
            assert_eq!(c, TraceConfig::off());
            let t = AccessTrace::new(c, "0000:01:00.0");
            assert!(!t.on());
            // no shadow piece is ever made to exit on read
            assert!(!t.piece_traps(0, 0x70_0000));
            assert!(!t.piece_traps(0x61_0000, 0x1000));
            // and nothing is ever admitted
            assert!(!t.admit(Kind::Read, 0x61_1ec0, 4, 1));
            assert!(!t.admit(Kind::Write, 0x11_0c00, 4, 1));
            assert!(!t.admit(Kind::Msi, 0, 0, 0));
            assert_eq!((t.records(), t.bytes(), t.dropped()), (0, 0, 0));
            assert_eq!(t.report(), "kf3: BAR0-TRACE off");
        }
    }

    #[test]
    fn a_typo_is_refused_by_name_never_silently_off_or_unbounded() {
        assert!(cfg(&[("KF3_BAR0_READ_TRACE", "yes")]).is_err());
        for (k, v) in [
            ("KF3_READ_TRACE_RANGES", "0x10-0x5"),
            ("KF3_READ_TRACE_RANGES", "zz"),
            ("KF3_READ_TRACE_RANGES", "0x1,,0x2"),
            ("KF3_WRITE_TRACE_RANGES", "0x1-"),
            ("KF3_READ_TRACE_MAX_BYTES", "0"),
            ("KF3_READ_TRACE_MAX_BYTES", "17G"),
            ("KF3_READ_TRACE_MAX_BYTES", "lots"),
            ("KF3_READ_TRACE_MAX_RECORDS", "0"),
            ("KF3_TRACE_NAME", "a b"),
            ("KF3_TRACE_NAME", "x(1)"),
        ] {
            assert!(
                cfg(&[("KF3_BAR0_READ_TRACE", "1"), (k, v)]).is_err(),
                "{k}={v} accepted"
            );
        }
        let many = (0..=MAX_RANGES)
            .map(|i| format!("{:#x}", i * 16))
            .collect::<Vec<_>>()
            .join(",");
        assert!(
            cfg(&[
                ("KF3_BAR0_READ_TRACE", "1"),
                ("KF3_READ_TRACE_RANGES", &many)
            ])
            .is_err()
        );
    }

    #[test]
    fn range_parsing() {
        assert_eq!(Ranges::parse("all").unwrap(), Ranges::All);
        assert_eq!(Ranges::parse("ALL").unwrap(), Ranges::All);
        assert_eq!(Ranges::parse("0x0-0xfff,all").unwrap(), Ranges::All);
        assert_eq!(Ranges::parse("none").unwrap(), Ranges::None);
        assert_eq!(
            Ranges::parse(" 0x0-0xfff , 611ec0 ,0X110000-0X110FFF").unwrap(),
            Ranges::List(vec![
                Range { lo: 0, hi: 0xfff },
                Range {
                    lo: 0x61_1ec0,
                    hi: 0x61_1ec3
                },
                Range {
                    lo: 0x11_0000,
                    hi: 0x11_0fff
                },
            ])
        );
        let r = Ranges::parse("0x100-0x1ff").unwrap();
        assert!(r.hits(0x100, 4) && r.hits(0x1fc, 4) && r.hits(0x1ff, 1));
        assert!(r.hits(0xfe, 4), "an access straddling the start touches it");
        assert!(!r.hits(0xfc, 4) && !r.hits(0x200, 4));
        assert!(!Ranges::None.hits(0, 8) && Ranges::All.hits(u64::MAX, 8));
    }

    #[test]
    fn the_default_selection_is_the_display_range_and_a_list_adds_to_it() {
        let t = on(&[]);
        assert!(t.on());
        for off in [
            0x61_1ec0u64,
            0x61_1c00,
            0x61_1800,
            0x61_2078,
            0x68_a218,
            0x69_0a2c,
            0x68_0220,
        ] {
            assert!(t.admit(Kind::Read, off, 4, 0), "{off:#x}");
        }
        assert!(
            !t.admit(Kind::Read, 0, 4, 0x1940_00a1),
            "PMC_BOOT_0 is not display"
        );
        assert!(t.piece_traps(0, 0x70_0000) && !t.piece_traps(0x80_0000, 0x10_0000));
        // writes default to all
        assert!(t.admit(Kind::Write, 0x11_0c00, 4, 7));
        let t = on(&[("KF3_READ_TRACE_RANGES", "0x0-0xfff")]);
        assert!(t.admit(Kind::Read, 0, 4, 0x1940_00a1));
        assert!(
            t.admit(Kind::Read, 0x61_1ec0, 4, 0),
            "display stays selected"
        );
        assert!(!t.admit(Kind::Read, 0x10_0000, 4, 0));
        let t = on(&[("KF3_WRITE_TRACE_RANGES", "none")]);
        assert!(!t.admit(Kind::Write, 0x11_0c00, 4, 7));
        assert!(t.report().contains("unselected_exits=1"));
    }

    /// Lines copied verbatim from the VFIO reference's QEMU trace
    /// (`traces/vfio_dvi_reference_20261008/boot3-qemu-trace.log.gz`, branch
    /// `claude/vfio-dvi-reference-20261008`): kf3's records must be the same text for the same
    /// access, so `mmio_window.py`, `vfio_msi_vectors.py` and the like read both boots.
    #[test]
    fn records_are_the_vfio_reference_lines_and_their_length_is_exact() {
        let ts = "2026-10-08T20:10:01.265690Z";
        let n = "0000:01:00.0";
        for (kind, a, b, c, line) in [
            (
                Kind::Read,
                0x32_4c00,
                4,
                0x4e56,
                "2026-10-08T20:10:01.265690Z vfio_region_read  (0000:01:00.0:region0+0x324c00, 4) = 0x4e56\n",
            ),
            (
                Kind::Read,
                0x39_9a80,
                1,
                0x1,
                "2026-10-08T20:10:01.265690Z vfio_region_read  (0000:01:00.0:region0+0x399a80, 1) = 0x1\n",
            ),
            (
                Kind::Read,
                0x32_4d78,
                4,
                0x2ec0_8b2e,
                "2026-10-08T20:10:01.265690Z vfio_region_read  (0000:01:00.0:region0+0x324d78, 4) = 0x2ec08b2e\n",
            ),
        ] {
            assert_eq!(render(kind, ts, n, a, b, c), line);
            assert_eq!(record_len(kind, n.len() as u64, a, b, c), line.len() as u64);
        }
        // the write and MSI shapes, from the same events' format strings
        let w = render(Kind::Write, ts, n, 0x11_0c00, 4, 0x1f);
        assert_eq!(
            w,
            "2026-10-08T20:10:01.265690Z vfio_region_write  (0000:01:00.0:region0+0x110c00, 0x1f, 4)\n"
        );
        let m = render(Kind::Msi, ts, n, 0, 0x4023, 0xfee0_0000);
        assert_eq!(
            m,
            "2026-10-08T20:10:01.265690Z vfio_msi_interrupt  (0000:01:00.0) vector 0 0xfee00000/0x4023\n"
        );
        // exhaustive-ish length check across digit-count boundaries
        let vals = [
            0u64,
            1,
            9,
            10,
            0xf,
            0x10,
            0xffff,
            0x1_0000,
            0xffff_ffff,
            0x1_0000_0000,
            u64::MAX,
        ];
        for kind in [Kind::Read, Kind::Write, Kind::Msi] {
            for &a in &vals {
                for &b in &[1u64, 2, 4, 8, 10, 0x4023, 0xffff_ffff] {
                    for &c in &vals {
                        let l = render(kind, ts, n, a, b, c);
                        assert_eq!(
                            record_len(kind, n.len() as u64, a, b, c),
                            l.len() as u64,
                            "{l}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_record_cap_spends_the_trace_and_counts_drops() {
        let t = on(&[("KF3_READ_TRACE_MAX_RECORDS", "3")]);
        assert!(t.admit(Kind::Write, 0x10, 4, 1));
        assert!(t.admit(Kind::Read, 0x61_0000, 4, 1));
        assert!(t.admit(Kind::Msi, 0, 0x4023, 0xfee0_0000));
        assert!(!t.admit(Kind::Write, 0x10, 4, 1));
        assert!(!t.admit(Kind::Read, 0x61_0000, 4, 1));
        assert_eq!((t.records(), t.dropped()), (3, 2));
        let r = t.report();
        assert!(
            r.contains("records=3 (cap 3)") && r.contains("dropped=2") && r.contains("cap_hit=yes"),
            "{r}"
        );
        assert!(r.contains("reads=1 writes=1 msi=1"), "{r}");
    }

    #[test]
    fn the_byte_cap_is_never_crossed_and_spends_the_trace() {
        let one = record_len(Kind::Read, 12, 0x61_1ec0, 4, 0xdead_beef);
        let t = on(&[(
            "KF3_READ_TRACE_MAX_BYTES",
            &format!("{}", one * 2 + one / 2),
        )]);
        assert!(t.admit(Kind::Read, 0x61_1ec0, 4, 0xdead_beef));
        assert!(t.admit(Kind::Read, 0x61_1ec0, 4, 0xdead_beef));
        assert!(!t.admit(Kind::Read, 0x61_1ec0, 4, 0xdead_beef));
        // spent: even a record that would still fit is dropped from now on
        assert!(!t.admit(Kind::Read, 0x61_0000, 1, 0));
        assert_eq!(t.bytes(), one * 2);
        assert!(t.bytes() <= one * 2 + one / 2);
        assert_eq!((t.records(), t.dropped()), (2, 2));
        assert!(t.report().contains("cap_hit=yes"));
    }

    #[test]
    fn concurrent_admission_never_crosses_the_caps() {
        let t = std::sync::Arc::new(on(&[
            ("KF3_READ_TRACE_RANGES", "all"),
            ("KF3_READ_TRACE_MAX_RECORDS", "1000"),
        ]));
        let hs: Vec<_> = (0..8)
            .map(|_| {
                let t = t.clone();
                std::thread::spawn(move || {
                    for i in 0..500u64 {
                        t.admit(Kind::Read, i * 4, 4, i);
                    }
                })
            })
            .collect();
        for h in hs {
            h.join().unwrap();
        }
        assert_eq!(t.records(), 1000);
        assert_eq!(t.records() + t.dropped(), 8 * 500);
    }

    #[test]
    fn names() {
        assert_eq!(on(&[]).name(), "0000:01:00.0");
        assert_eq!(on(&[("KF3_TRACE_NAME", "kf3-a")]).name(), "kf3-a");
        let t = AccessTrace::new(cfg(&[("KF3_BAR0_READ_TRACE", "1")]).unwrap(), "bad name");
        assert_eq!(t.name(), "kf3");
        assert!(!valid_name("") && !valid_name(&"a".repeat(MAX_NAME + 1)));
    }

    #[test]
    fn sizes() {
        assert_eq!(size("512M").unwrap(), 512 << 20);
        assert_eq!(size("4k").unwrap(), 4096);
        assert_eq!(size("1G").unwrap(), 1 << 30);
        assert_eq!(size("12345").unwrap(), 12345);
        assert!(size("").is_err() && size("M").is_err() && size("99999999999999G").is_err());
    }
}
