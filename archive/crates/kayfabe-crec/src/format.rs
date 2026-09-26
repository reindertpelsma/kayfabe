//! The C emulator's §6 replay-trace format, decoded.
//!
//! The binary layout is `nvkvm_m2_rec.h` in the C repo, and its reference decoder is
//! `scripts/mode2_diag/rec_dump.py` there. **This is a second instrument, so it is
//! cross-validated against the first before anything is believed of it**
//! (`suspect_the_instrument_first`): `tests/decoder_matches_reference.rs` pins the exact
//! census `rec_dump.py` prints for the committed capture — total, per kind, dense order,
//! header counters — so a decoder bug shows up as a failing test rather than as a
//! divergence in the GSP.
//!
//! ```text
//! [ NvkvmRecHdr, 96 bytes ][ provenance text, hdr_len - 96 ][ records ]
//! ```
//!
//! Each record is a fixed 32-byte entry followed by `len` payload bytes padded up to an
//! 8-byte multiple, so every record starts 8-byte aligned. All integers little-endian.
//!
//! ## Two decoding rules that are not obvious
//!
//! 1. **`hdr_len` is authoritative, `sizeof(hdr)` is not.** The provenance block starts at
//!    `sizeof(NvkvmRecHdr)` = 96 — *including* `reserved1[3]` — and runs to `hdr_len`. The
//!    reference decoder's own comment records getting this wrong the first time.
//! 2. **A zero `n_records` is legal.** The header counters are patched by `pwrite` at
//!    close; a killed QEMU leaves them zero and a usable *dense prefix*. So the record
//!    count is always taken from the scan, and the header's copy is compared to it as a
//!    separate, reportable fact ([`CTrace::closed_cleanly`]).

use kayfabe_arch::ids::Gpa;
use kayfabe_trace::{Bar, IrqSpec, Record, Seq, TraceEvent, Width};

/// `NVKVM_REC_MAGIC` — `"NKVRECRC"` little-endian.
pub const MAGIC: u64 = 0x4352_4345_5256_4B4E;
/// `NVKVM_REC_VERSION`.
pub const VERSION: u32 = 1;
/// `sizeof(NvkvmRecHdr)`.
pub const HEADER_BYTES: usize = 96;
/// `sizeof(NvkvmRecEnt)`.
pub const ENTRY_BYTES: usize = 32;

/// What a record is. The numbering is the C's `NVKVM_REC_*`, and 1..=6 map 1:1 onto
/// [`TraceEvent`]'s wire plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CKind {
    /// `a` = offset, `b` = the value **served**; `bar`/`width` set.
    MmioRead,
    /// `a` = offset, `b` = the value **written**; `bar`/`width` set.
    MmioWrite,
    /// `a` = gpa, payload = the bytes **returned**.
    GuestRead,
    /// `a` = gpa, payload = the bytes **written**.
    GuestWrite,
    /// `a = 0` → MSI-X with `b` = vector; `a = 1` → INTx with `b` = level.
    Irq,
    /// `a` = nanoseconds on `QEMU_CLOCK_VIRTUAL`.
    Clock,
    /// The `m2romregs` overlay page snapshot. **No [`TraceEvent`] counterpart** — it
    /// stands in for register reads that never trapped, so feeding it to a positional
    /// differential against a recorder with no overlay is exactly the error the C's own
    /// header warns about. Absent from the committed capture (`m2romregs=off`).
    OverlaySnap,
}

impl CKind {
    /// Decode the wire byte. `None` for a kind this decoder does not know — never a
    /// silent default, because an unknown kind means the format moved and every later
    /// record is suspect.
    #[must_use]
    pub fn from_wire(v: u8) -> Option<CKind> {
        Some(match v {
            1 => CKind::MmioRead,
            2 => CKind::MmioWrite,
            3 => CKind::GuestRead,
            4 => CKind::GuestWrite,
            5 => CKind::Irq,
            6 => CKind::Clock,
            7 => CKind::OverlaySnap,
            _ => return None,
        })
    }

    /// The name `rec_dump.py` prints, so a census can be compared to the reference
    /// decoder's output literally.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            CKind::MmioRead => "MmioRead",
            CKind::MmioWrite => "MmioWrite",
            CKind::GuestRead => "GuestRead",
            CKind::GuestWrite => "GuestWrite",
            CKind::Irq => "IrqRaise",
            CKind::Clock => "Clock",
            CKind::OverlaySnap => "OverlaySnap",
        }
    }
}

/// One decoded record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CRecord {
    /// The recorder's dense sequence number.
    pub seq: u64,
    /// What happened.
    pub kind: CKind,
    /// MMIO access size in bytes; 0 otherwise.
    pub width: u8,
    /// PCI BAR index for MMIO; `0xFF` otherwise.
    pub bar: u8,
    /// Kind-dependent — see [`CKind`].
    pub a: u64,
    /// Kind-dependent — see [`CKind`].
    pub b: u64,
    /// The variable tail.
    pub payload: Vec<u8>,
}

impl CRecord {
    /// The wire-plane [`TraceEvent`] this record is, or `None` for [`CKind::OverlaySnap`],
    /// which has no counterpart and must not be fed to a positional differential.
    ///
    /// # Errors
    ///
    /// [`CrecError::BadWidth`] if an MMIO record claims an access size no register access
    /// can be — a corrupt width becomes a decode refusal instead of a nonsense number
    /// carried into a comparison.
    pub fn to_event(&self) -> Result<Option<TraceEvent>, CrecError> {
        let width = || {
            Width::from_bytes(self.width).ok_or(CrecError::BadWidth {
                seq: self.seq,
                width: self.width,
            })
        };
        Ok(Some(match self.kind {
            CKind::MmioRead => TraceEvent::MmioRead {
                bar: Bar(self.bar),
                off: self.a,
                size: width()?,
                val: self.b,
            },
            CKind::MmioWrite => TraceEvent::MmioWrite {
                bar: Bar(self.bar),
                off: self.a,
                size: width()?,
                val: self.b,
            },
            CKind::GuestRead => TraceEvent::GuestRead {
                gpa: Gpa(self.a),
                bytes: self.payload.clone(),
            },
            CKind::GuestWrite => TraceEvent::GuestWrite {
                gpa: Gpa(self.a),
                bytes: self.payload.clone(),
            },
            CKind::Irq => TraceEvent::IrqRaise {
                spec: if self.a == 0 {
                    IrqSpec::Msix(self.b as u16)
                } else {
                    IrqSpec::IntxLevel(self.b != 0)
                },
            },
            CKind::Clock => TraceEvent::Clock { ns: self.a },
            CKind::OverlaySnap => return Ok(None),
        }))
    }
}

/// The file header, plus the free-text provenance block that follows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CHeader {
    /// Format version.
    pub version: u32,
    /// Bytes from file start to the first record.
    pub hdr_len: u32,
    /// `sizeof(NvkvmRecEnt)` as the writer saw it.
    pub rec_size: u32,
    /// `NVKVM_REC_P_*` — exactly which `m2*` knobs were on.
    pub props: u64,
    /// `NVKVM_REC_M_*` — the declared filter, as applied.
    pub mask: u64,
    /// Patched at close; 0 means "the writer was killed, scan to EOF".
    pub n_records: u64,
    /// Patched at close.
    pub n_bytes: u64,
    /// Short or failed `write(2)`s. **Must be 0** or the artifact is not trustworthy.
    pub n_errors: u64,
    /// `QEMU_CLOCK_VIRTUAL` at open.
    pub t0_ns: u64,
    /// The free-text block: guest kernel, host driver, VBIOS md5, `nvidia-smi` summary.
    pub provenance: String,
}

/// `NVKVM_REC_P_NONHERMETIC`: the host GPU could DMA into guest RAM behind this recorder.
pub const P_NONHERMETIC: u64 = 1 << 32;
/// `NVKVM_REC_P_M2ROMREGS`: the rom-device overlay was on, so falcon register reads did
/// not trap and only [`CKind::OverlaySnap`] stands in for them.
pub const P_M2ROMREGS: u64 = 1 << 8;

impl CHeader {
    /// Can a replay be **closed** over this trace?
    ///
    /// Only when the host GPU cannot have written guest memory behind the recorder. With
    /// `m2fwd`/`m2exec` on, `nvkvm_m2_share_guest_ram` `MAP_FIXED`s the guest-RAM memfd
    /// into the stub and the GPU DMAs into it directly — bytes that pass through neither
    /// `nvkvm_dmaw` nor any QEMU path (`c_rust_trace_differential.md` §5a, L3).
    #[must_use]
    pub fn hermetic(&self) -> bool {
        self.props & P_NONHERMETIC == 0
    }

    /// Were falcon register reads served from the rom-device overlay (and therefore
    /// invisible as [`CKind::MmioRead`])?
    #[must_use]
    pub fn rom_overlay(&self) -> bool {
        self.props & P_M2ROMREGS != 0
    }
}

/// A refusal to decode. Every variant names the exact thing that was wrong; there is no
/// "invalid file" catch-all, because a differential built on a silently-mangled trace is
/// worse than one that does not run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CrecError {
    /// Shorter than a header.
    ShortHeader {
        /// Bytes actually present.
        got: usize,
    },
    /// Not a `NVKVM_REC_MAGIC` file.
    BadMagic {
        /// What the first eight bytes said.
        got: u64,
    },
    /// A format version this decoder does not implement.
    UnsupportedVersion {
        /// What the file declared.
        got: u32,
    },
    /// The writer's `sizeof(NvkvmRecEnt)` is not ours.
    RecordSizeMismatch {
        /// What the file declared.
        got: u32,
        /// What this decoder implements.
        expected: usize,
    },
    /// `hdr_len` does not even cover the fixed header.
    HeaderTooShort {
        /// What the file declared.
        hdr_len: u32,
    },
    /// The sink reported short or failed writes. The artifact is not trustworthy and this
    /// is a refusal, not a warning.
    SinkErrors {
        /// The header's `n_errors`.
        n_errors: u64,
    },
    /// A record claims more payload than the file holds.
    Truncated {
        /// The record's sequence number.
        seq: u64,
        /// The payload length it claimed.
        len: u32,
    },
    /// A record kind this decoder does not know.
    UnknownKind {
        /// The record's sequence number.
        seq: u64,
        /// The wire byte.
        kind: u8,
    },
    /// An MMIO record claims an access width no register access can be.
    BadWidth {
        /// The record's sequence number.
        seq: u64,
        /// The width it claimed.
        width: u8,
    },
}

/// A decoded capture: the header, the provenance, and every record in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CTrace {
    header: CHeader,
    records: Vec<CRecord>,
}

impl CTrace {
    /// Decode a whole capture.
    ///
    /// # Errors
    ///
    /// A named [`CrecError`]. Note in particular that `n_errors != 0` is a **refusal**:
    /// the C's own header says such a file is not trustworthy, and a differential is
    /// exactly the consumer that must not proceed on one.
    pub fn parse(blob: &[u8]) -> Result<CTrace, CrecError> {
        if blob.len() < HEADER_BYTES {
            return Err(CrecError::ShortHeader { got: blob.len() });
        }
        let u32_at = |o: usize| u32::from_le_bytes(blob[o..o + 4].try_into().unwrap());
        let u64_at = |o: usize| u64::from_le_bytes(blob[o..o + 8].try_into().unwrap());

        let magic = u64_at(0);
        if magic != MAGIC {
            return Err(CrecError::BadMagic { got: magic });
        }
        let version = u32_at(8);
        if version != VERSION {
            return Err(CrecError::UnsupportedVersion { got: version });
        }
        let hdr_len = u32_at(12);
        let rec_size = u32_at(16);
        if rec_size as usize != ENTRY_BYTES {
            return Err(CrecError::RecordSizeMismatch {
                got: rec_size,
                expected: ENTRY_BYTES,
            });
        }
        if (hdr_len as usize) < HEADER_BYTES {
            return Err(CrecError::HeaderTooShort { hdr_len });
        }
        let n_errors = u64_at(56);
        if n_errors != 0 {
            return Err(CrecError::SinkErrors { n_errors });
        }
        // ★ From `sizeof(NvkvmRecHdr)`, not from the end of the last named field: the
        // reserved words are part of the struct and the text starts after them.
        let prov_end = (hdr_len as usize).min(blob.len());
        let prov_raw = &blob[HEADER_BYTES..prov_end];
        let provenance = String::from_utf8_lossy(match prov_raw.iter().position(|&b| b == 0) {
            Some(n) => &prov_raw[..n],
            None => prov_raw,
        })
        .into_owned();

        let header = CHeader {
            version,
            hdr_len,
            rec_size,
            props: u64_at(24),
            mask: u64_at(32),
            n_records: u64_at(40),
            n_bytes: u64_at(48),
            n_errors,
            t0_ns: u64_at(64),
            provenance,
        };

        let mut records = Vec::new();
        let mut off = hdr_len as usize;
        while off + ENTRY_BYTES <= blob.len() {
            let seq = u64::from_le_bytes(blob[off..off + 8].try_into().unwrap());
            let kind_wire = blob[off + 8];
            let width = blob[off + 9];
            let bar = blob[off + 10];
            let len = u32::from_le_bytes(blob[off + 12..off + 16].try_into().unwrap());
            let a = u64::from_le_bytes(blob[off + 16..off + 24].try_into().unwrap());
            let b = u64::from_le_bytes(blob[off + 24..off + 32].try_into().unwrap());
            let kind = CKind::from_wire(kind_wire).ok_or(CrecError::UnknownKind {
                seq,
                kind: kind_wire,
            })?;
            let pstart = off + ENTRY_BYTES;
            let pend = pstart
                .checked_add(len as usize)
                .ok_or(CrecError::Truncated { seq, len })?;
            if pend > blob.len() {
                return Err(CrecError::Truncated { seq, len });
            }
            records.push(CRecord {
                seq,
                kind,
                width,
                bar,
                a,
                b,
                payload: blob[pstart..pend].to_vec(),
            });
            off = pend + ((8 - (len as usize & 7)) & 7);
        }
        Ok(CTrace { header, records })
    }

    /// The header and provenance.
    #[must_use]
    pub fn header(&self) -> &CHeader {
        &self.header
    }

    /// Every record, in file order.
    #[must_use]
    pub fn records(&self) -> &[CRecord] {
        &self.records
    }

    /// Did the writer patch its counters at close? A `false` here is not fatal — it means
    /// QEMU was killed and the file is a **dense prefix**, which is the only kind of
    /// truncation that is not — but it is a fact a consumer must be able to see.
    #[must_use]
    pub fn closed_cleanly(&self) -> bool {
        self.header.n_records == self.records.len() as u64
    }

    /// How many records of each kind — the census `rec_dump.py` prints, in the same
    /// spelling, so the two decoders can be compared literally.
    #[must_use]
    pub fn census(&self) -> Vec<(&'static str, usize)> {
        let mut out: Vec<(&'static str, usize)> = Vec::new();
        for r in &self.records {
            let name = r.kind.name();
            match out.iter_mut().find(|(n, _)| *n == name) {
                Some((_, c)) => *c += 1,
                None => out.push((name, 1)),
            }
        }
        out.sort_unstable();
        out
    }

    /// The stream as [`kayfabe_trace::Record`]s, so [`kayfabe_trace::check_dense_order`]
    /// can be applied to it — the same order check the consumer applies to its own
    /// recorder, rather than a second one written here.
    ///
    /// # Errors
    ///
    /// [`CrecError::BadWidth`] from [`CRecord::to_event`]. [`CKind::OverlaySnap`] records
    /// are **dropped**, which is why this cannot be used as the differential's input on a
    /// capture that has any: it would silently break the density the checker tests. The
    /// committed capture has none, and [`CTrace::has_overlay`] is how a caller checks.
    pub fn to_records(&self) -> Result<Vec<Record>, CrecError> {
        let mut out = Vec::with_capacity(self.records.len());
        for r in &self.records {
            if let Some(ev) = r.to_event()? {
                out.push(Record {
                    seq: Seq(r.seq),
                    ev,
                });
            }
        }
        Ok(out)
    }

    /// Does this capture contain overlay snapshots? If so it carries register reads that
    /// never trapped, and a positional differential against a recorder with no overlay is
    /// invalid.
    #[must_use]
    pub fn has_overlay(&self) -> bool {
        self.records.iter().any(|r| r.kind == CKind::OverlaySnap)
    }
}

kayfabe_util::assert_send_sync!(CTrace, CRecord, CHeader, CKind, CrecError);
