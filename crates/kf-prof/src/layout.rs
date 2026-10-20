//! Fixed wire v1 geometry. Encoding is explicit little endian, never native layout.

/// Full file size, with reserved space included.
pub const FILE_BYTES: usize = 16 * 1024 * 1024;
/// Header byte length.
pub const HEADER_BYTES: usize = 4096;
/// Offset of first producer slice.
pub const THREAD_BASE: usize = 4096;
/// Maximum producer count.
pub const THREADS: usize = 64;
/// Byte length of a producer slice.
pub const THREAD_BYTES: usize = 196608;
/// Offset of reconciled ledger snapshots.
pub const LEDGER_BASE: usize = 12587008;
/// Bytes reserved for reconciled ledger snapshots.
pub const LEDGER_BYTES: usize = 1048576;
/// Offset of per-second records.
pub const RECORD_BASE: usize = 13635584;
/// Size of one per-second record.
pub const RECORD_BYTES: usize = 65536;
/// Number of retained complete per-second records.
pub const RECORDS: usize = 47;

/// Invalid or unsupported file header.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayoutError {
    /// Header is incomplete.
    Truncated,
    /// Magic, version or byte order is unsupported.
    Version,
    /// Geometry disagrees with this fixed wire version.
    Geometry,
    /// Revision contains bytes outside the host-authored hex vocabulary.
    Revision,
}

/// Immutable header identity. Mutable fields are initially zero.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Header {
    /// Host process ID.
    pub pid: u64,
    /// Feature/coverage flags.
    pub flags: u64,
    /// CLOCK_MONOTONIC start time, nanoseconds.
    pub start_ns: u64,
    /// Host build revision, forty lowercase hex bytes (all zero means unknown).
    pub revision: [u8; 40],
}
impl Default for Header {
    fn default() -> Self {
        Self {
            pid: 0,
            flags: 0,
            start_ns: 0,
            revision: [0; 40],
        }
    }
}
impl Header {
    /// Construct a zero-padded header. No native struct representation is exposed.
    pub fn encode(self) -> [u8; HEADER_BYTES] {
        let mut b = [0u8; HEADER_BYTES];
        b[..8].copy_from_slice(b"KFPROF01");
        b[8..12].copy_from_slice(&1u32.to_le_bytes());
        b[12..16].copy_from_slice(&0x01020304u32.to_le_bytes());
        put(&mut b, 16, FILE_BYTES as u64);
        put(&mut b, 32, self.pid);
        put(&mut b, 40, self.flags);
        put(&mut b, 48, self.start_ns);
        b[128..168].copy_from_slice(&self.revision);
        b
    }
    /// Decode a bounded header; no allocation, iteration over header-provided sizes or panic.
    pub fn decode(b: &[u8]) -> Result<Self, LayoutError> {
        let b = b.get(..HEADER_BYTES).ok_or(LayoutError::Truncated)?;
        if &b[..8] != b"KFPROF01"
            || &b[8..12] != 1u32.to_le_bytes().as_slice()
            || &b[12..16] != 0x01020304u32.to_le_bytes().as_slice()
        {
            return Err(LayoutError::Version);
        }
        if get(b, 16) != FILE_BYTES as u64 || get(b, 168) > THREADS as u64 {
            return Err(LayoutError::Geometry);
        }
        let mut revision = [0; 40];
        revision.copy_from_slice(&b[128..168]);
        if revision != [0; 40]
            && !revision
                .iter()
                .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(v))
        {
            return Err(LayoutError::Revision);
        }
        Ok(Self {
            pid: get(b, 32),
            flags: get(b, 40),
            start_ns: get(b, 48),
            revision,
        })
    }
}
fn put(b: &mut [u8; HEADER_BYTES], off: usize, n: u64) {
    b[off..off + 8].copy_from_slice(&n.to_le_bytes());
}
fn get(b: &[u8], off: usize) -> u64 {
    let mut w = [0u8; 8];
    w.copy_from_slice(&b[off..off + 8]);
    u64::from_le_bytes(w)
}
