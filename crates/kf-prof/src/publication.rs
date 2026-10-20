//! Atomic-word snapshot protocol with bounded readers and an explicit ordering seam.
use core::sync::atomic::{AtomicU64, Ordering, fence};

/// Validated storage of atomic u64 words. Implementations must bounds-check indices.
///
/// The core performs geometry validation too. There are no raw pointers or native
/// layout casts in this interface. One writer owns a record; readers may be concurrent.
pub trait WordStorage {
    /// Number of addressable words.
    fn len(&self) -> usize;
    /// Whether the storage contains no words.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Atomic load at a validated word index.
    fn load(&self, index: usize, order: Ordering) -> u64;
    /// Atomic store at a validated word index.
    fn store(&self, index: usize, value: u64, order: Ordering);
    /// Ordering barrier; exposed for deterministic protocol tests.
    fn fence(&self, order: Ordering) {
        fence(order);
    }
}

/// Allocation-free atomic words, useful for private trusted publication storage.
pub struct AtomicWords<const N: usize> {
    words: [AtomicU64; N],
}
impl<const N: usize> Default for AtomicWords<N> {
    fn default() -> Self {
        Self::new()
    }
}
impl<const N: usize> AtomicWords<N> {
    /// Zero-initialized word array.
    pub const fn new() -> Self {
        Self {
            words: [const { AtomicU64::new(0) }; N],
        }
    }
}
impl<const N: usize> WordStorage for AtomicWords<N> {
    fn len(&self) -> usize {
        N
    }
    fn load(&self, index: usize, order: Ordering) -> u64 {
        self.words.get(index).map_or(0, |w| w.load(order))
    }
    fn store(&self, index: usize, value: u64, order: Ordering) {
        if let Some(w) = self.words.get(index) {
            w.store(value, order);
        }
    }
}
/// Snapshot is unavailable; callers never spin waiting for a writer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadError {
    /// Odd or changing writer sequence.
    Busy,
    /// Record leaves the storage allocation.
    Bounds,
}
/// A rejected publication leaves the record untouched.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriteError {
    /// Record leaves the storage allocation.
    Bounds,
    /// Sequence exhausted; retire this record instead of reusing an ABA identity.
    Exhausted,
}
fn fits(len: usize, base: usize, payload: usize) -> bool {
    base.checked_add(payload)
        .and_then(|n| n.checked_add(1))
        .is_some_and(|end| end <= len)
}
/// Publish one record using a writer-private sequence. No reads or RMWs.
///
/// The owner must serialize publications to this record. This is a logical
/// consistency requirement, not a memory-safety precondition: all payload words
/// are atomic. The release fence before data pairs with the reader's acquire
/// fence through any newly observed payload word; the final sequence validation
/// must then see that publication's odd sequence or a later one.
pub fn write_record(
    s: &impl WordStorage,
    base: usize,
    payload: &[u64],
    sequence: &mut u64,
) -> Result<(), WriteError> {
    if !fits(s.len(), base, payload.len()) {
        return Err(WriteError::Bounds);
    }
    let even = sequence
        .checked_add(2)
        .filter(|n| n % 2 == 0)
        .ok_or(WriteError::Exhausted)?;
    s.store(base, even - 1, Ordering::Relaxed);
    s.fence(Ordering::Release);
    for (i, &v) in payload.iter().enumerate() {
        s.store(base + 1 + i, v, Ordering::Relaxed);
    }
    s.store(base, even, Ordering::Release);
    *sequence = even;
    Ok(())
}
/// Attempt one snapshot; work is bounded by a compile-time payload length.
pub fn read_record<const N: usize>(
    s: &impl WordStorage,
    base: usize,
) -> Result<[u64; N], ReadError> {
    if !fits(s.len(), base, N) {
        return Err(ReadError::Bounds);
    }
    let a = s.load(base, Ordering::Acquire);
    if a & 1 != 0 {
        return Err(ReadError::Busy);
    }
    let mut out = [0; N];
    for (i, v) in out.iter_mut().enumerate() {
        *v = s.load(base + 1 + i, Ordering::Relaxed);
    }
    s.fence(Ordering::Acquire);
    if a != s.load(base, Ordering::Relaxed) {
        return Err(ReadError::Busy);
    }
    Ok(out)
}
