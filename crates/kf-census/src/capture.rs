//! The KFGSPCAP capture format: what a Windows capture module writes
//! (`docs/design/V3_GSP_CAPTURE_FORMAT.md`, STATUS: DESIGN-ONLY, 2026-10-13).
//!
//! Everything is little-endian and fixed-size so that a kernel module can emit it with
//! `memcpy`s and no parsing: a 128-byte file header, then records of a 32-byte header plus
//! `payload_bytes` of raw queue bytes. The module never interprets a queue element; all
//! interpretation is this crate's job, offline.
//!
//! The format has **no** field for anything board-specific. [`FILE_FIELDS`] and
//! [`RECORD_FIELDS`] are the complete field tables (they tile the headers exactly) and a test
//! pins that no name in them is a board identity. The payload bytes are the driver's own
//! queue content and can still contain board data inside a message body; that is a policy
//! matter documented in the design page (capture files are host-only), not something the
//! module may filter without parsing.

use std::fmt;
use std::io::{self, Read, Write};

/// File magic.
pub const MAGIC: [u8; 8] = *b"KFGSPCAP";
/// Format version this crate reads and writes.
pub const VERSION: u32 = 1;
/// Size of the file header.
pub const FILE_HEADER_BYTES: usize = 128;
/// Size of every record header.
pub const RECORD_HEADER_BYTES: usize = 32;
/// Record magic, `"KREC"` little-endian.
pub const RECORD_MAGIC: u32 = 0x4345_524b;
/// Reader bound on one record's payload (a module chunk is at most one ring: 1 MiB).
pub const MAX_PAYLOAD_BYTES: u32 = 1 << 20;
/// `entry_count` value of a loss record whose lost-entry count is not known.
pub const UNKNOWN_COUNT: u32 = u32::MAX;
/// Width of the two string fields, including the mandatory terminating NUL.
pub const STRING_BYTES: usize = 32;
/// File-header `flags` bit 0: the file was written by `kf-census scrub` (`scrub.rs`). A module
/// never sets it; a reader that sees it knows that message bodies of controls not on the
/// allow-list were replaced by the fill pattern `scrub::FILL`. Every other bit must be zero.
pub const FLAG_SCRUBBED: u32 = 1;

/// One header field: its place in the byte layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldSpec {
    /// Field name (also the denylist test's input).
    pub name: &'static str,
    /// Byte offset.
    pub off: usize,
    /// Byte length.
    pub len: usize,
    /// True if the field is reserved/padding and must be zero.
    pub zero: bool,
}

const fn fs(name: &'static str, off: usize, len: usize, zero: bool) -> FieldSpec {
    FieldSpec {
        name,
        off,
        len,
        zero,
    }
}

/// The complete file-header layout. Tiles `0..FILE_HEADER_BYTES` with no gap.
pub const FILE_FIELDS: &[FieldSpec] = &[
    fs("magic", 0, 8, false),
    fs("version", 8, 4, false),
    fs("header_bytes", 12, 4, false),
    fs("record_header_bytes", 16, 4, false),
    fs("flags", 20, 4, false),
    fs("os_id", 24, 4, false),
    fs("module_version", 28, 4, false),
    fs("pci_vendor_id", 32, 2, false),
    fs("pci_device_id", 34, 2, false),
    fs("pci_revision_id", 36, 1, false),
    fs("reserved0", 37, 3, true),
    fs("ts_hz", 40, 8, false),
    fs("ts_start", 48, 8, false),
    fs("driver_version", 56, 32, false),
    fs("decode_tag", 88, 32, false),
    fs("reserved1", 120, 8, true),
];

/// The complete record-header layout. Tiles `0..RECORD_HEADER_BYTES` with no gap.
pub const RECORD_FIELDS: &[FieldSpec] = &[
    fs("magic", 0, 4, false),
    fs("kind", 4, 1, false),
    fs("dir", 5, 1, false),
    fs("queue_id", 6, 2, false),
    fs("payload_bytes", 8, 4, false),
    fs("counter", 12, 4, false),
    fs("ts", 16, 8, false),
    fs("first_entry", 24, 4, false),
    fs("entry_count", 28, 4, false),
];

/// Operating system the module ran on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    /// Windows (the WDK module).
    Windows,
    /// Linux.
    Linux,
}

impl Os {
    /// Wire value.
    #[must_use]
    pub fn id(self) -> u32 {
        match self {
            Os::Windows => 1,
            Os::Linux => 2,
        }
    }
    fn from_id(v: u32) -> Option<Os> {
        match v {
            1 => Some(Os::Windows),
            2 => Some(Os::Linux),
            _ => None,
        }
    }
}

/// Which queue of the pair a record belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Dir {
    /// Command queue: driver (CPU) to GSP. The observer calls this `direction 0`, "request".
    Cmd,
    /// Status queue: GSP to driver. The observer's `direction 1`, "reply".
    Status,
}

impl Dir {
    /// Wire value.
    #[must_use]
    pub fn id(self) -> u8 {
        match self {
            Dir::Cmd => 0,
            Dir::Status => 1,
        }
    }
    fn from_id(v: u8) -> Option<Dir> {
        match v {
            0 => Some(Dir::Cmd),
            1 => Some(Dir::Status),
            _ => None,
        }
    }
}

/// Record kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Payload: the 32 raw bytes of the queue's `msgqTxHeader`, copied verbatim when the
    /// module first sees the queue. Defines entry size and ring size for `(queue_id, dir)`.
    Bind,
    /// Payload: `entry_count` consecutive ring entries starting at ring index `first_entry`.
    Elements,
    /// Entries were dropped because the module's own buffer was full.
    LossOverflow,
    /// A range could not be read (fault, or the snapshot was unstable).
    LossUnreadable,
    /// The module lost track for another reason (paused, restarted, record write failed).
    LossUnknown,
    /// Clean close. `entry_count` = number of records written before this one.
    End,
}

impl Kind {
    /// Wire value.
    #[must_use]
    pub fn id(self) -> u8 {
        match self {
            Kind::Bind => 1,
            Kind::Elements => 2,
            Kind::LossOverflow => 3,
            Kind::LossUnreadable => 4,
            Kind::LossUnknown => 5,
            Kind::End => 6,
        }
    }
    fn from_id(v: u8) -> Option<Kind> {
        match v {
            1 => Some(Kind::Bind),
            2 => Some(Kind::Elements),
            3 => Some(Kind::LossOverflow),
            4 => Some(Kind::LossUnreadable),
            5 => Some(Kind::LossUnknown),
            6 => Some(Kind::End),
            _ => None,
        }
    }
    /// True for the three loss kinds.
    #[must_use]
    pub fn is_loss(self) -> bool {
        matches!(
            self,
            Kind::LossOverflow | Kind::LossUnreadable | Kind::LossUnknown
        )
    }
}

/// The metadata a cell needs. No board identity: only the die, the driver and the clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileHeader {
    /// Module's operating system.
    pub os: Os,
    /// Version of the capture module (its own counter, not a driver version).
    pub module_version: u32,
    /// PCI vendor id (0x10de).
    pub pci_vendor_id: u16,
    /// PCI device id: the die.
    pub pci_device_id: u16,
    /// PCI revision id.
    pub pci_revision_id: u8,
    /// Timestamp ticks per second.
    pub ts_hz: u64,
    /// Value of the monotonic clock when the file was opened (informational).
    pub ts_start: u64,
    /// Driver version string as read from the guest, verbatim (printable ASCII, at most 31).
    pub driver_version: String,
    /// The ogkm tag the capture will be decoded with (may be empty: the decoder chooses).
    pub decode_tag: String,
    /// `flags` bit 0 ([`FLAG_SCRUBBED`]): set only by the scrub stage, never by a module.
    pub scrubbed: bool,
}

/// One record header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordHeader {
    /// Kind.
    pub kind: Kind,
    /// Queue of the pair.
    pub dir: Dir,
    /// Queue-pair instance: the module increments it each time a queue pair is (re)created.
    pub queue_id: u16,
    /// Payload length in bytes.
    pub payload_bytes: u32,
    /// Local record counter: 0 for the first record of the file, +1 per record.
    pub counter: u32,
    /// Monotonic timestamp in ticks (`ts_hz`).
    pub ts: u64,
    /// Ring index of the first entry (Elements), or where the loss begins (Loss).
    pub first_entry: u32,
    /// Entries in the payload (Elements), entries lost (Loss) or records written (End).
    pub entry_count: u32,
}

/// One record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// Header.
    pub header: RecordHeader,
    /// Raw bytes, exactly as read from the queue.
    pub payload: Vec<u8>,
}

/// Why a capture stream could not be read further.
#[derive(Debug)]
pub enum CaptureError {
    /// Underlying read failed.
    Io(io::Error),
    /// The first 8 bytes are not [`MAGIC`].
    BadMagic,
    /// Unsupported version.
    UnsupportedVersion(u32),
    /// A header length field disagrees with this crate's.
    BadHeaderSizes {
        /// `header_bytes` found.
        header_bytes: u32,
        /// `record_header_bytes` found.
        record_header_bytes: u32,
    },
    /// The file ended inside the file header.
    TruncatedFileHeader {
        /// Bytes available.
        have: usize,
    },
    /// The file ended inside a record (header or payload).
    TruncatedRecord {
        /// Bytes the record needed (header or header + payload).
        need: usize,
        /// Bytes available.
        have: usize,
    },
    /// Record magic wrong.
    BadRecordMagic(u32),
    /// Unknown record kind.
    UnknownKind(u8),
    /// Unknown direction.
    UnknownDir(u8),
    /// Payload larger than [`MAX_PAYLOAD_BYTES`].
    PayloadTooLarge(u32),
    /// A reserved field is not zero.
    ReservedNotZero(&'static str),
    /// Unknown operating system id.
    UnknownOs(u32),
    /// A string field is not NUL-terminated, not printable ASCII, or has data after the NUL.
    BadString(&'static str),
}

impl fmt::Display for CaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CaptureError::Io(e) => write!(f, "io error: {e}"),
            CaptureError::BadMagic => write!(f, "not a KFGSPCAP file (bad magic)"),
            CaptureError::UnsupportedVersion(v) => write!(f, "unsupported format version {v}"),
            CaptureError::BadHeaderSizes {
                header_bytes,
                record_header_bytes,
            } => write!(
                f,
                "header sizes {header_bytes}/{record_header_bytes} (expected {FILE_HEADER_BYTES}/{RECORD_HEADER_BYTES})"
            ),
            CaptureError::TruncatedFileHeader { have } => {
                write!(f, "file ends inside the file header ({have} bytes)")
            }
            CaptureError::TruncatedRecord { need, have } => {
                write!(f, "file ends inside a record (need {need}, have {have})")
            }
            CaptureError::BadRecordMagic(m) => write!(f, "bad record magic {m:#010x}"),
            CaptureError::UnknownKind(k) => write!(f, "unknown record kind {k}"),
            CaptureError::UnknownDir(d) => write!(f, "unknown direction {d}"),
            CaptureError::PayloadTooLarge(n) => write!(f, "payload of {n} bytes exceeds the bound"),
            CaptureError::ReservedNotZero(n) => write!(f, "reserved field {n} is not zero"),
            CaptureError::UnknownOs(o) => write!(f, "unknown os id {o}"),
            CaptureError::BadString(n) => write!(f, "string field {n} is malformed"),
        }
    }
}

impl std::error::Error for CaptureError {}

impl From<io::Error> for CaptureError {
    fn from(e: io::Error) -> Self {
        CaptureError::Io(e)
    }
}

fn put_str(out: &mut [u8], s: &str, name: &'static str) -> Result<(), CaptureError> {
    let b = s.as_bytes();
    if b.len() >= out.len() || !b.iter().all(|c| (0x20..0x7f).contains(c)) {
        return Err(CaptureError::BadString(name));
    }
    out[..b.len()].copy_from_slice(b);
    Ok(())
}

fn get_str(raw: &[u8], name: &'static str) -> Result<String, CaptureError> {
    let n = raw
        .iter()
        .position(|&c| c == 0)
        .ok_or(CaptureError::BadString(name))?;
    if !raw[n..].iter().all(|&c| c == 0) || !raw[..n].iter().all(|c| (0x20..0x7f).contains(c)) {
        return Err(CaptureError::BadString(name));
    }
    Ok(String::from_utf8_lossy(&raw[..n]).into_owned())
}

fn le32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

fn le64(b: &[u8], off: usize) -> u64 {
    let mut x = [0u8; 8];
    x.copy_from_slice(&b[off..off + 8]);
    u64::from_le_bytes(x)
}

impl FileHeader {
    /// Serialise to the fixed 128-byte layout.
    ///
    /// # Errors
    ///
    /// [`CaptureError::BadString`] if a string does not fit or is not printable ASCII.
    pub fn encode(&self) -> Result<[u8; FILE_HEADER_BYTES], CaptureError> {
        let mut b = [0u8; FILE_HEADER_BYTES];
        b[0..8].copy_from_slice(&MAGIC);
        b[8..12].copy_from_slice(&VERSION.to_le_bytes());
        b[12..16].copy_from_slice(&(FILE_HEADER_BYTES as u32).to_le_bytes());
        b[16..20].copy_from_slice(&(RECORD_HEADER_BYTES as u32).to_le_bytes());
        b[20..24].copy_from_slice(&(if self.scrubbed { FLAG_SCRUBBED } else { 0 }).to_le_bytes());
        b[24..28].copy_from_slice(&self.os.id().to_le_bytes());
        b[28..32].copy_from_slice(&self.module_version.to_le_bytes());
        b[32..34].copy_from_slice(&self.pci_vendor_id.to_le_bytes());
        b[34..36].copy_from_slice(&self.pci_device_id.to_le_bytes());
        b[36] = self.pci_revision_id;
        b[40..48].copy_from_slice(&self.ts_hz.to_le_bytes());
        b[48..56].copy_from_slice(&self.ts_start.to_le_bytes());
        put_str(&mut b[56..88], &self.driver_version, "driver_version")?;
        put_str(&mut b[88..120], &self.decode_tag, "decode_tag")?;
        Ok(b)
    }

    /// Parse and validate a 128-byte file header.
    ///
    /// # Errors
    ///
    /// Any of the header-level [`CaptureError`]s.
    pub fn decode(b: &[u8]) -> Result<FileHeader, CaptureError> {
        if b.len() < FILE_HEADER_BYTES {
            return Err(CaptureError::TruncatedFileHeader { have: b.len() });
        }
        if b[0..8] != MAGIC {
            return Err(CaptureError::BadMagic);
        }
        let version = le32(b, 8);
        if version != VERSION {
            return Err(CaptureError::UnsupportedVersion(version));
        }
        let (hb, rb) = (le32(b, 12), le32(b, 16));
        if hb as usize != FILE_HEADER_BYTES || rb as usize != RECORD_HEADER_BYTES {
            return Err(CaptureError::BadHeaderSizes {
                header_bytes: hb,
                record_header_bytes: rb,
            });
        }
        for f in FILE_FIELDS.iter().filter(|f| f.zero) {
            if b[f.off..f.off + f.len].iter().any(|&x| x != 0) {
                return Err(CaptureError::ReservedNotZero(f.name));
            }
        }
        let flags = le32(b, 20);
        if flags & !FLAG_SCRUBBED != 0 {
            return Err(CaptureError::ReservedNotZero("flags"));
        }
        let os_id = le32(b, 24);
        Ok(FileHeader {
            os: Os::from_id(os_id).ok_or(CaptureError::UnknownOs(os_id))?,
            module_version: le32(b, 28),
            pci_vendor_id: u16::from_le_bytes([b[32], b[33]]),
            pci_device_id: u16::from_le_bytes([b[34], b[35]]),
            pci_revision_id: b[36],
            ts_hz: le64(b, 40),
            ts_start: le64(b, 48),
            driver_version: get_str(&b[56..88], "driver_version")?,
            decode_tag: get_str(&b[88..120], "decode_tag")?,
            scrubbed: flags & FLAG_SCRUBBED != 0,
        })
    }
}

impl RecordHeader {
    /// Serialise to the fixed 32-byte layout.
    #[must_use]
    pub fn encode(&self) -> [u8; RECORD_HEADER_BYTES] {
        let mut b = [0u8; RECORD_HEADER_BYTES];
        b[0..4].copy_from_slice(&RECORD_MAGIC.to_le_bytes());
        b[4] = self.kind.id();
        b[5] = self.dir.id();
        b[6..8].copy_from_slice(&self.queue_id.to_le_bytes());
        b[8..12].copy_from_slice(&self.payload_bytes.to_le_bytes());
        b[12..16].copy_from_slice(&self.counter.to_le_bytes());
        b[16..24].copy_from_slice(&self.ts.to_le_bytes());
        b[24..28].copy_from_slice(&self.first_entry.to_le_bytes());
        b[28..32].copy_from_slice(&self.entry_count.to_le_bytes());
        b
    }

    /// Parse and validate a 32-byte record header.
    ///
    /// # Errors
    ///
    /// Record-level [`CaptureError`]s.
    pub fn decode(b: &[u8]) -> Result<RecordHeader, CaptureError> {
        if b.len() < RECORD_HEADER_BYTES {
            return Err(CaptureError::TruncatedRecord {
                need: RECORD_HEADER_BYTES,
                have: b.len(),
            });
        }
        let magic = le32(b, 0);
        if magic != RECORD_MAGIC {
            return Err(CaptureError::BadRecordMagic(magic));
        }
        let payload_bytes = le32(b, 8);
        if payload_bytes > MAX_PAYLOAD_BYTES {
            return Err(CaptureError::PayloadTooLarge(payload_bytes));
        }
        Ok(RecordHeader {
            kind: Kind::from_id(b[4]).ok_or(CaptureError::UnknownKind(b[4]))?,
            dir: Dir::from_id(b[5]).ok_or(CaptureError::UnknownDir(b[5]))?,
            queue_id: u16::from_le_bytes([b[6], b[7]]),
            payload_bytes,
            counter: le32(b, 12),
            ts: le64(b, 16),
            first_entry: le32(b, 24),
            entry_count: le32(b, 28),
        })
    }
}

/// Appends records to any [`Write`], assigning the local counter.
///
/// This is the reference writer: the kernel module's emit path is the same few `memcpy`s.
#[derive(Debug)]
pub struct Writer<W: Write> {
    w: W,
    counter: u32,
}

impl<W: Write> Writer<W> {
    /// Write the file header.
    ///
    /// # Errors
    ///
    /// [`CaptureError`] for a malformed header string or a failed write.
    pub fn new(mut w: W, header: &FileHeader) -> Result<Self, CaptureError> {
        w.write_all(&header.encode()?)?;
        Ok(Writer { w, counter: 0 })
    }

    fn push(&mut self, mut h: RecordHeader, payload: &[u8]) -> Result<(), CaptureError> {
        h.payload_bytes =
            u32::try_from(payload.len()).map_err(|_| CaptureError::PayloadTooLarge(u32::MAX))?;
        if h.payload_bytes > MAX_PAYLOAD_BYTES {
            return Err(CaptureError::PayloadTooLarge(h.payload_bytes));
        }
        h.counter = self.counter;
        self.w.write_all(&h.encode())?;
        self.w.write_all(payload)?;
        self.counter = self.counter.wrapping_add(1);
        Ok(())
    }

    fn base(kind: Kind, dir: Dir, queue_id: u16, ts: u64) -> RecordHeader {
        RecordHeader {
            kind,
            dir,
            queue_id,
            payload_bytes: 0,
            counter: 0,
            ts,
            first_entry: 0,
            entry_count: 0,
        }
    }

    /// Record the queue's raw 32-byte `msgqTxHeader`.
    ///
    /// # Errors
    ///
    /// Write failures.
    pub fn bind(
        &mut self,
        queue_id: u16,
        dir: Dir,
        ts: u64,
        raw_tx_header: &[u8; 32],
    ) -> Result<(), CaptureError> {
        self.push(Self::base(Kind::Bind, dir, queue_id, ts), raw_tx_header)
    }

    /// Record `entry_count` consecutive entries starting at ring index `first_entry`.
    ///
    /// # Errors
    ///
    /// Write failures; payloads over the bound.
    pub fn elements(
        &mut self,
        queue_id: u16,
        dir: Dir,
        ts: u64,
        first_entry: u32,
        entry_count: u32,
        payload: &[u8],
    ) -> Result<(), CaptureError> {
        let mut h = Self::base(Kind::Elements, dir, queue_id, ts);
        h.first_entry = first_entry;
        h.entry_count = entry_count;
        self.push(h, payload)
    }

    /// Record an explicit loss marker. `kind` must be one of the three loss kinds.
    ///
    /// # Errors
    ///
    /// Write failures; [`CaptureError::UnknownKind`] if `kind` is not a loss kind.
    pub fn loss(
        &mut self,
        kind: Kind,
        queue_id: u16,
        dir: Dir,
        ts: u64,
        first_entry: u32,
        entries_lost: u32,
    ) -> Result<(), CaptureError> {
        if !kind.is_loss() {
            return Err(CaptureError::UnknownKind(kind.id()));
        }
        let mut h = Self::base(kind, dir, queue_id, ts);
        h.first_entry = first_entry;
        h.entry_count = entries_lost;
        self.push(h, &[])
    }

    /// Write the clean-close record and flush. The record carries queue instance 0 and the
    /// command queue; see [`Writer::end_at`] for a writer that stamps the queue it last saw.
    ///
    /// # Errors
    ///
    /// Write failures.
    pub fn end(self, ts: u64) -> Result<W, CaptureError> {
        self.end_at(ts, 0, Dir::Cmd)
    }

    /// Like [`Writer::end`], with the `queue_id` and `dir` fields of the End record chosen by
    /// the caller. The format does not interpret them (the decoder ignores both); the Windows
    /// capture module stamps the queue pair it was following when it closed, so a byte-exact
    /// comparison with its output needs this form (`tests/golden.rs`).
    ///
    /// # Errors
    ///
    /// Write failures.
    pub fn end_at(mut self, ts: u64, queue_id: u16, dir: Dir) -> Result<W, CaptureError> {
        let mut h = Self::base(Kind::End, dir, queue_id, ts);
        h.entry_count = self.counter;
        self.push(h, &[])?;
        self.w.flush()?;
        Ok(self.w)
    }

    /// Finish **without** an End record (what a crashed module leaves behind).
    #[must_use]
    pub fn abandon(self) -> W {
        self.w
    }
}

/// Reads records from any [`Read`], validating framing but not meaning.
#[derive(Debug)]
pub struct Reader<R: Read> {
    r: R,
    header: FileHeader,
}

fn read_full<R: Read>(r: &mut R, buf: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}

impl<R: Read> Reader<R> {
    /// Read and validate the file header.
    ///
    /// # Errors
    ///
    /// Header-level [`CaptureError`]s.
    pub fn new(mut r: R) -> Result<Self, CaptureError> {
        let mut b = [0u8; FILE_HEADER_BYTES];
        let n = read_full(&mut r, &mut b)?;
        if n < FILE_HEADER_BYTES {
            // A short file that is not even our magic is "not this format", not "truncated".
            if n >= 8 && b[0..8] != MAGIC {
                return Err(CaptureError::BadMagic);
            }
            return Err(CaptureError::TruncatedFileHeader { have: n });
        }
        let header = FileHeader::decode(&b)?;
        Ok(Reader { r, header })
    }

    /// The file header.
    #[must_use]
    pub fn header(&self) -> &FileHeader {
        &self.header
    }

    /// Next record; `Ok(None)` at a clean end of file (between records).
    ///
    /// # Errors
    ///
    /// Framing errors, including [`CaptureError::TruncatedRecord`] for a file that ends
    /// inside a record.
    pub fn next_record(&mut self) -> Result<Option<Record>, CaptureError> {
        let mut hb = [0u8; RECORD_HEADER_BYTES];
        let n = read_full(&mut self.r, &mut hb)?;
        if n == 0 {
            return Ok(None);
        }
        if n < RECORD_HEADER_BYTES {
            return Err(CaptureError::TruncatedRecord {
                need: RECORD_HEADER_BYTES,
                have: n,
            });
        }
        let header = RecordHeader::decode(&hb)?;
        let mut payload = vec![0u8; header.payload_bytes as usize];
        let got = read_full(&mut self.r, &mut payload)?;
        if got < payload.len() {
            return Err(CaptureError::TruncatedRecord {
                need: RECORD_HEADER_BYTES + payload.len(),
                have: RECORD_HEADER_BYTES + got,
            });
        }
        Ok(Some(Record { header, payload }))
    }
}
