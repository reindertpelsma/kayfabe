//! Bounded immutable source snapshots through confirmed placement rows.
//!
//! No guest table/PFN cache or GPU action. The caller retains the exact channel,
//! mirror and USERD lifetimes, and checks the VA-manager pending gate. This seam
//! checks row identity again after CPU-view preparation and holds row/topology
//! read guards across the bounded copy. Store preparation is outside all guards;
//! Store::copy must only read already-armed owned views, without host verbs.

use super::{PlacedRows, RamMap, RowsLog};
use kf_chan::tmode::Span;
use std::sync::{Arc, RwLock, RwLockReadGuard, TryLockError, Weak, atomic::Ordering};

/// Trusted bounded snapshot policy, shared with native descriptor admission and
/// limited by the existing private staging region. All reads remain off-vCPU.
pub(crate) const MAX_BYTES: usize = kf_chan::user_storage::MAX_SOURCE_BYTES as usize;
// Complete maximum span at public4KiB grain, including an unaligned first page.
// Finer hostile fragmentation still refuses; no guest-sized unbounded traversal.
const PIECES: usize = MAX_BYTES / 4096 + 1;
// Trusted declared-ring metadata cap:8MiB at public4KiB pages, plus one for8-byte alignment.
const RING_PIECES: usize = 2049;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    Bounds,
    Busy,
    Changed,
    Missing(u64),
    Topology,
    Store,
    Allocation,
    Poisoned,
}

/// Worker-only store view owner. Preparation may call RM, with no row or RAM
/// guard retained. Copy is bounded CPU access only and must not call RM.
pub(crate) trait Store {
    fn prepare(&mut self, spans: &[Span]) -> Result<(), Refusal>;
    fn copy(&self, off: u64, bytes: &mut [u8]) -> Result<(), Refusal>;
}

#[derive(Debug)]
pub(crate) struct Snapshot<'a> {
    writer: Arc<std::sync::atomic::AtomicU64>,
    pub bytes: Vec<u8>,
    pub epoch: u64,
    rows: Weak<RwLock<std::collections::BTreeMap<u64, super::PlacedRow>>>,
    spans: Vec<Span>,
    va: u64,
    end: u64,
    ram: &'a RamMap,
    generation: u64,
    pieces: usize,
}

impl Snapshot<'_> {
    /// Diagnostic text only: the backing pieces this snapshot read (guest RAM or VRAM store, the
    /// backing offset, the length). Names no guest bytes.
    pub(crate) fn describe(&self) -> String {
        let mut out = String::new();
        for s in self.spans.iter().take(8) {
            out.push_str(&format!(
                "{}@{:#x}+{:#x} ",
                if s.ram { "ram" } else { "vram" },
                s.off,
                s.len
            ));
        }
        if self.spans.len() > 8 {
            out.push_str(&format!("(+{} more)", self.spans.len().saturating_sub(8)));
        }
        out
    }
    /// Revalidate the exact confirmed source span and birth RAM generation,
    /// retaining both guards across one bounded CPU publication transaction.
    /// The callback MUST NOT perform an RM ioctl, acquire channel/graph locks,
    /// or wait/retry. Channel identity and the VA-manager ordering gate are
    /// separate prerequisites. This does not pin engine operands or complete work.
    pub(crate) fn admit<T>(&self, publish: impl FnOnce() -> T) -> Result<T, Refusal> {
        let rows = self.rows.upgrade().ok_or(Refusal::Changed)?;
        let guard = try_rows(&rows, &self.writer)?;
        if resolve(&guard, self.va, self.end, self.pieces)? != self.spans {
            return Err(Refusal::Changed);
        }
        let _ram_guard = try_read(&self.ram.blocks)?;
        if self.generation == 0 || self.ram.generation.load(Ordering::Relaxed) != self.generation {
            return Err(Refusal::Topology);
        }
        Ok(publish())
    }
}

/// Whole-ring mapping admission without reading a guest-sized ring. The trusted
/// ring cap and row/RAM-piece caps bound all work; no CPU store view is armed.
pub(crate) struct Mapping<'a>(Snapshot<'a>);
impl Mapping<'_> {
    pub(crate) fn admit<T>(&self, publish: impl FnOnce() -> T) -> Result<T, Refusal> {
        self.0.admit(publish)
    }
}

pub(crate) fn mapped<'a>(
    rows: &PlacedRows,
    log: &RowsLog,
    ram: &'a RamMap,
    generation: u64,
    va: u64,
    bytes: u64,
    store_bound: u64,
) -> Result<Mapping<'a>, Refusal> {
    // Allocation policy, not a die/driver fact: at most2^20 source GP entries.
    if bytes == 0 || bytes > 8 << 20 || va & 7 != 0 {
        return Err(Refusal::Bounds);
    }
    let end = va
        .checked_add(bytes)
        .filter(|end| *end < 1 << 40)
        .ok_or(Refusal::Bounds)?;
    let guard = try_rows(rows, &log.writer)?;
    let epoch = log.epoch();
    let spans = resolve(&guard, va, end, RING_PIECES)?;
    let blocks = try_read(&ram.blocks)?;
    if generation == 0 || ram.generation.load(Ordering::Relaxed) != generation {
        return Err(Refusal::Topology);
    }
    if blocks.len() > PIECES {
        return Err(Refusal::Bounds);
    }
    let fd = blocks.iter().find_map(|b| b.fd);
    let mut pieces = 0;
    for span in &spans {
        let end = span.off.checked_add(span.len).ok_or(Refusal::Bounds)?;
        if !span.ram {
            if end > store_bound {
                return Err(Refusal::Bounds);
            }
            continue;
        }
        let fd = fd.ok_or(Refusal::Topology)?;
        let mut at = span.off;
        while at < end {
            if pieces == RING_PIECES {
                return Err(Refusal::Bounds);
            }
            pieces = pieces.saturating_add(1); // bounded by the `== RING_PIECES` refusal just above
            let mut covers = blocks.iter().enumerate().filter(|(_, b)| {
                b.fd == Some(fd)
                    && at >= b.fd_off
                    && b.fd_off
                        .checked_add(b.mem.len() as u64)
                        .is_some_and(|e| at < e)
            });
            let (index, block) = covers.next().ok_or(Refusal::Topology)?;
            if covers.next().is_some() {
                return Err(Refusal::Topology);
            }
            let until = block
                .fd_off
                .checked_add(block.mem.len() as u64)
                .ok_or(Refusal::Bounds)?
                .min(end);
            if blocks.iter().enumerate().any(|(i, other)| {
                i != index
                    && other.fd == Some(fd)
                    && other.fd_off < until
                    && other
                        .fd_off
                        .checked_add(other.mem.len() as u64)
                        .is_none_or(|e| at < e)
            }) {
                return Err(Refusal::Topology);
            }
            at = until;
        }
    }
    Ok(Mapping(Snapshot {
        bytes: Vec::new(),
        epoch,
        rows: Arc::downgrade(rows),
        writer: log.writer.clone(),
        spans,
        va,
        end,
        ram,
        generation,
        pieces: RING_PIECES,
    }))
}

fn try_rows<'a, T>(
    lock: &'a RwLock<T>,
    writer: &std::sync::atomic::AtomicU64,
) -> Result<RwLockReadGuard<'a, T>, Refusal> {
    let result = try_read(lock);
    if matches!(result, Err(Refusal::Busy)) {
        kf_host::latency::contention(
            kf_host::latency::Cause::Rows,
            writer.load(Ordering::Acquire),
        );
    }
    result
}

fn try_read<T>(lock: &RwLock<T>) -> Result<RwLockReadGuard<'_, T>, Refusal> {
    lock.try_read().map_err(|e| match e {
        TryLockError::WouldBlock => Refusal::Busy,
        TryLockError::Poisoned(_) => Refusal::Poisoned,
    })
}

fn resolve(
    rows: &std::collections::BTreeMap<u64, super::PlacedRow>,
    va: u64,
    end: u64,
    pieces: usize,
) -> Result<Vec<Span>, Refusal> {
    let mut spans = Vec::new();
    spans
        .try_reserve_exact(pieces)
        .map_err(|_| Refusal::Allocation)?;
    let mut at = va;
    while at < end {
        if spans.len() == pieces {
            return Err(Refusal::Bounds);
        }
        let (&base, &(len, off, ram, perm)) =
            rows.range(..=at).next_back().ok_or(Refusal::Missing(at))?;
        let row_end = base
            .checked_add(len)
            .filter(|e| *e > at)
            .ok_or(Refusal::Missing(at))?;
        // `base <= at` (the range is `..=at`), `at < row_end` (the filter) and `at < end` (the loop condition): never short.
        let len = row_end
            .checked_sub(at)
            .zip(end.checked_sub(at))
            .map(|(row, rest)| row.min(rest))
            .ok_or(Refusal::Missing(at))?;
        let off = off
            .checked_add(at.checked_sub(base).ok_or(Refusal::Missing(at))?)
            .filter(|o| o.checked_add(len).is_some())
            .ok_or(Refusal::Bounds)?;
        spans.push(Span {
            ram,
            off,
            len,
            perm,
        });
        at = at.checked_add(len).ok_or(Refusal::Bounds)?;
    }
    Ok(spans)
}

/// Snapshot one bounded span. No row/RAM pointer escapes; errors expose no
/// partial output. The topology generation comes from this channel's birth,
/// never refreshed after revocation. This is not a VA-manager submission gate.
pub(crate) fn snapshot<'a>(
    rows: &PlacedRows,
    log: &RowsLog,
    ram: &'a RamMap,
    generation: u64,
    va: u64,
    bytes: usize,
    store: &mut impl Store,
) -> Result<Snapshot<'a>, Refusal> {
    if bytes == 0 || bytes > MAX_BYTES {
        return Err(Refusal::Bounds);
    }
    let end = va.checked_add(bytes as u64).ok_or(Refusal::Bounds)?;
    let first = try_rows(rows, &log.writer)?;
    let epoch = log.epoch();
    let spans = resolve(&first, va, end, PIECES)?;
    drop(first);
    // This is the only stage allowed to arm native CPU views. An epoch change
    // during preparation discards this attempt before copying any source bytes.
    store.prepare(&spans)?;
    let mut out = Vec::new();
    out.try_reserve_exact(bytes)
        .map_err(|_| Refusal::Allocation)?;
    out.resize(bytes, 0);
    let rows_guard = try_rows(rows, &log.writer)?;
    if log.epoch() != epoch {
        return Err(Refusal::Changed);
    }
    // Steer can remove a row without updating RowsLog. Re-resolve the whole
    // source as well: an epoch is not the sole authority for backing identity.
    if resolve(&rows_guard, va, end, PIECES)? != spans {
        return Err(Refusal::Changed);
    }
    let blocks = try_read(&ram.blocks)?;
    if generation == 0 || ram.generation.load(Ordering::Relaxed) != generation {
        return Err(Refusal::Topology);
    }
    if blocks.len() > PIECES {
        return Err(Refusal::Bounds);
    }
    let first_fd = blocks.iter().find_map(|b| b.fd);
    let mut done = 0usize;
    let mut ram_pieces = 0usize;
    for span in &spans {
        let len = usize::try_from(span.len).map_err(|_| Refusal::Bounds)?;
        let dest = out
            .get_mut(done..done.checked_add(len).ok_or(Refusal::Bounds)?)
            .ok_or(Refusal::Bounds)?;
        if !span.ram {
            store.copy(span.off, dest)?;
        } else {
            let fd = first_fd.ok_or(Refusal::Topology)?;
            let mut copied = 0usize;
            while copied < len {
                if ram_pieces == PIECES {
                    return Err(Refusal::Bounds);
                }
                ram_pieces = ram_pieces.saturating_add(1); // bounded by the `== PIECES` refusal just above
                let off = span.off.checked_add(copied as u64).ok_or(Refusal::Bounds)?;
                let mut covers = blocks.iter().enumerate().filter(|(_, b)| {
                    b.fd == Some(fd)
                        && off >= b.fd_off
                        && b.fd_off
                            .checked_add(b.mem.len() as u64)
                            .is_some_and(|e| off < e)
                });
                let (index, b) = covers.next().ok_or(Refusal::Topology)?;
                if covers.next().is_some() {
                    return Err(Refusal::Topology);
                }
                // `covers` admits only `fd_off <= off < fd_off + mem.len`, and `copied < len`: neither difference is short.
                let pos = off
                    .checked_sub(b.fd_off)
                    .and_then(|d| usize::try_from(d).ok())
                    .ok_or(Refusal::Bounds)?;
                let n = b
                    .mem
                    .len()
                    .checked_sub(pos)
                    .zip(len.checked_sub(copied))
                    .map(|(block, rest)| block.min(rest))
                    .ok_or(Refusal::Bounds)?;
                let end = off.checked_add(n as u64).ok_or(Refusal::Bounds)?;
                if blocks.iter().enumerate().any(|(i, other)| {
                    i != index
                        && other.fd == Some(fd)
                        && other.fd_off < end
                        && other
                            .fd_off
                            .checked_add(other.mem.len() as u64)
                            .is_none_or(|e| off < e)
                }) {
                    return Err(Refusal::Topology);
                }
                let window = copied
                    .checked_add(n)
                    .and_then(|stop| dest.get_mut(copied..stop))
                    .ok_or(Refusal::Bounds)?;
                if !b.mem.read_into(pos, window) {
                    return Err(Refusal::Topology);
                }
                copied = copied.checked_add(n).ok_or(Refusal::Bounds)?;
            }
        }
        done = done.checked_add(len).ok_or(Refusal::Bounds)?;
    }
    Ok(Snapshot {
        bytes: out,
        epoch,
        rows: Arc::downgrade(rows),
        writer: log.writer.clone(),
        spans,
        va,
        end,
        ram,
        generation,
        pieces: PIECES,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kf_host::MapPerm;
    use std::sync::atomic::Ordering;
    #[test]
    fn staging_sized_fragmented_source_is_bounded_and_rechecks_its_last_page() {
        let bytes = kf_chan::user_storage::GP_OFFSET as usize;
        let ram = RamMap::default();
        let (mem, fd) = crate::raw_unsafe::test_owned_ram(bytes + 4096);
        ram.add(super::super::RamBlock {
            gpa: 0x2000,
            mem,
            fd: Some(fd),
            fd_off: 0,
        });
        let rows = PlacedRows::default();
        for offset in (0..bytes + 4096).step_by(4096) {
            rows.write().unwrap().insert(
                0x100000 + offset as u64,
                (4096, offset as u64, true, MapPerm::READ_WRITE),
            );
        }
        assert!(mem.write_from(4, &[7, 8, 9, 10]));
        assert!(mem.write_from(bytes, &[11, 12, 13, 14]));
        let log = RowsLog::default();
        let generation = ram.generation.load(Ordering::Relaxed);
        let source =
            snapshot(&rows, &log, &ram, generation, 0x100004, bytes, &mut NoStore).unwrap();
        assert_eq!(&source.bytes[..4], &[7, 8, 9, 10]);
        assert_eq!(&source.bytes[bytes - 4..], &[11, 12, 13, 14]);
        source.admit(|| ()).unwrap();
        assert_eq!(
            snapshot(
                &rows,
                &log,
                &ram,
                generation,
                0x100004,
                bytes + 1,
                &mut NoStore
            )
            .err(),
            Some(Refusal::Bounds)
        );
        rows.write().unwrap().remove(&(0x100000 + bytes as u64));
        assert_eq!(
            source.admit(|| ()),
            Err(Refusal::Missing(0x100000 + bytes as u64))
        );
        assert!(snapshot(&rows, &log, &ram, generation, 0x100004, bytes, &mut NoStore).is_err());
    }

    /// A row whose backing offset cannot hold the requested span (offset + length would wrap) is refused,
    /// as is a request that starts past the last row; neither reads, panics or loops.
    #[test]
    fn rows_with_unrepresentable_backing_offsets_refuse() {
        let (ram, _, rows, log, generation) = fixture();
        rows.write().unwrap().clear();
        rows.write().unwrap().insert(
            0x10000,
            (0x2000, u64::MAX - 0x10, true, MapPerm::READ_WRITE),
        );
        assert_eq!(
            snapshot(&rows, &log, &ram, generation, 0x10000, 0x100, &mut NoStore).err(),
            Some(Refusal::Bounds)
        );
        // Starting at the second page of the row: the offset still wraps.
        assert_eq!(
            snapshot(&rows, &log, &ram, generation, 0x11000, 0x100, &mut NoStore).err(),
            Some(Refusal::Bounds)
        );
        // Past the end of the only row: missing, by address.
        assert_eq!(
            snapshot(&rows, &log, &ram, generation, 0x12000, 0x100, &mut NoStore).err(),
            Some(Refusal::Missing(0x12000))
        );
        // A span that runs off the end of the row: the hole is named.
        rows.write().unwrap().clear();
        rows.write()
            .unwrap()
            .insert(0x10000, (0x1000, 0, true, MapPerm::READ_WRITE));
        assert_eq!(
            snapshot(&rows, &log, &ram, generation, 0x10f00, 0x200, &mut NoStore).err(),
            Some(Refusal::Missing(0x11000))
        );
    }

    #[test]
    fn full_eight_mib_ring_at_four_kib_grain_is_bounded_and_admitted() {
        let (ram, _, rows, log, generation) = fixture();
        rows.write().unwrap().clear();
        for i in 0..2048 {
            rows.write().unwrap().insert(
                0x100000 + i * 4096,
                (
                    4096,
                    i * 4096,
                    false,
                    MapPerm {
                        read_only: true,
                        ..MapPerm::READ_WRITE
                    },
                ),
            );
        }
        let mapping = mapped(&rows, &log, &ram, generation, 0x100000, 8 << 20, 8 << 20).unwrap();
        mapping.admit(|| ()).unwrap();
        // Complete span admission still notices a changed last page, even if
        // the current copied descriptor is at the beginning of the source.
        rows.write()
            .unwrap()
            .get_mut(&(0x100000 + 2047 * 4096))
            .unwrap()
            .1 = 0;
        assert_eq!(mapping.admit(|| ()), Err(Refusal::Changed));
        assert_eq!(
            mapped(
                &rows,
                &log,
                &ram,
                generation,
                0x100000,
                (8 << 20) + 8,
                9 << 20
            )
            .err(),
            Some(Refusal::Bounds)
        );
    }
    #[test]
    fn maximum_eight_aligned_ring_may_span_2049_four_kib_rows() {
        let (ram, _, rows, log, generation) = fixture();
        rows.write().unwrap().clear();
        for i in 0..RING_PIECES as u64 {
            rows.write().unwrap().insert(
                0x100000 + i * 4096,
                (
                    4096,
                    i * 4096,
                    false,
                    MapPerm {
                        read_only: true,
                        ..MapPerm::READ_WRITE
                    },
                ),
            );
        }
        let ring = mapped(&rows, &log, &ram, generation, 0x100008, 8 << 20, 9 << 20).unwrap();
        ring.admit(|| ()).unwrap();
    }
    #[test]
    fn actual_source_admission_never_reserves_native_credit_on_busy_or_replaced_rows() {
        use kf_chan::user_scan::Fetch;
        let (ram, _, rows, log, generation) = fixture();
        let snapshot = snapshot(&rows, &log, &ram, generation, 0x10000, 8, &mut NoStore).unwrap();
        let mut fetch = Fetch::new(8).unwrap();
        let mut writes = 0;
        {
            let _writer = rows.write().unwrap();
            assert_eq!(
                snapshot.admit(|| {
                    fetch.publish(Some((1, 1))).unwrap();
                    writes += 1;
                }),
                Err(Refusal::Busy)
            );
        }
        assert_eq!((fetch.issued(), writes), (0, 0));
        // Successful retry of the SAME live snapshot reserves exactly once.
        snapshot
            .admit(|| {
                fetch.publish(Some((1, 1))).unwrap();
                writes += 1;
            })
            .unwrap();
        assert_eq!((fetch.issued(), writes), (1, 1));
        rows.write().unwrap().get_mut(&0x10000).unwrap().1 += 8;
        assert_eq!(
            snapshot.admit(|| {
                fetch.publish(Some((1, 2))).unwrap();
                writes += 1;
            }),
            Err(Refusal::Changed)
        );
        assert_eq!((fetch.issued(), writes), (1, 1));
        assert_eq!(fetch.observe(1).unwrap(), [(1, 1)]);
    }
    #[test]
    fn staged_suffix_keeps_birth_topology_through_put_without_old_source_rows() {
        let (ram, _, _, _, generation) = fixture();
        let mut puts = 0;
        ram.admit_user_generation(generation, || {
            assert!(ram.blocks.try_write().is_err());
            puts += 1;
        })
        .unwrap();
        assert_eq!(puts, 1);
        // A zero-length logical USERD check done before this change cannot
        // authorize the subsequent suffix PUT against a revoked RAM topology.
        ram.generation.fetch_add(1, Ordering::Release);
        assert_eq!(
            ram.admit_user_generation(generation, || {
                puts += 1;
            }),
            Err(Refusal::Topology)
        );
        assert_eq!(puts, 1);
        let _writer = ram.blocks.write().unwrap();
        assert_eq!(
            ram.admit_user_generation(generation, || {
                puts += 1;
            }),
            Err(Refusal::Busy)
        );
        assert_eq!(puts, 1);
    }
    struct NoStore;
    impl Store for NoStore {
        fn prepare(&mut self, spans: &[Span]) -> Result<(), Refusal> {
            assert!(spans.iter().all(|s| s.ram));
            Ok(())
        }
        fn copy(&self, _: u64, _: &mut [u8]) -> Result<(), Refusal> {
            panic!("RAM only")
        }
    }
    fn fixture() -> (RamMap, super::super::RamBlock, PlacedRows, RowsLog, u64) {
        let ram = RamMap::default();
        let (mem, fd) = crate::raw_unsafe::test_owned_ram(0x8000);
        let block = super::super::RamBlock {
            gpa: 0x2000,
            mem,
            fd: Some(fd),
            fd_off: 0,
        };
        ram.add(block);
        let rows = PlacedRows::default();
        rows.write()
            .unwrap()
            .insert(0x10000, (0x8000, 0, true, MapPerm::READ_WRITE));
        let generation = ram.generation.load(Ordering::Relaxed);
        (ram, block, rows, RowsLog::default(), generation)
    }
    #[test]
    fn snapshots_are_owned_bounded_and_never_expose_partial_hole_reads() {
        let (ram, block, rows, log, generation) = fixture();
        block.mem.write_from(0, &[7, 8, 9, 10]);
        let old = snapshot(&rows, &log, &ram, generation, 0x10000, 4, &mut NoStore).unwrap();
        block.mem.write_from(0, &[1, 2, 3, 4]);
        assert_eq!(old.bytes, [7, 8, 9, 10]);
        assert_eq!(old.epoch, 0);
        for (va, len) in [(0x10000, 0), (u64::MAX, 4), (0x10000, MAX_BYTES + 1)] {
            assert_eq!(
                snapshot(&rows, &log, &ram, generation, va, len, &mut NoStore).err(),
                Some(Refusal::Bounds)
            );
        }
        assert_eq!(
            snapshot(&rows, &log, &ram, generation, 0x17ffe, 4, &mut NoStore).err(),
            Some(Refusal::Missing(0x18000))
        );
        let guard = rows.write().unwrap();
        assert_eq!(
            snapshot(&rows, &log, &ram, generation, 0x10000, 4, &mut NoStore).err(),
            Some(Refusal::Busy)
        );
        drop(guard);
        let guard = ram.blocks.write().unwrap();
        assert_eq!(
            snapshot(&rows, &log, &ram, generation, 0x10000, 4, &mut NoStore).err(),
            Some(Refusal::Busy)
        );
        drop(guard);
        ram.del(block.gpa);
        ram.add(block);
        assert_eq!(
            snapshot(&rows, &log, &ram, generation, 0x10000, 4, &mut NoStore).err(),
            Some(Refusal::Topology)
        );
    }
    struct ReplaceDuringPrepare<'a>(&'a PlacedRows, &'a RowsLog, bool);
    impl Store for ReplaceDuringPrepare<'_> {
        fn prepare(&mut self, _: &[Span]) -> Result<(), Refusal> {
            // This succeeds only if the snapshot released its first row guard.
            let mut rows = self.0.try_write().unwrap();
            rows.insert(0x10000, (0x8000, 4, true, MapPerm::READ_WRITE));
            if self.2 {
                self.1.commit(0x10000, 0x18000);
            }
            Ok(())
        }
        fn copy(&self, _: u64, _: &mut [u8]) -> Result<(), Refusal> {
            panic!("must refuse")
        }
    }
    #[test]
    fn preparation_runs_outside_row_guard_and_changed_rows_are_not_copied() {
        let (ram, _, rows, log, generation) = fixture();
        let mut store = ReplaceDuringPrepare(&rows, &log, true);
        assert_eq!(
            snapshot(&rows, &log, &ram, generation, 0x10000, 4, &mut store).err(),
            Some(Refusal::Changed)
        );
    }
    #[test]
    fn unlogged_row_replacement_during_prepare_never_uses_stale_spans() {
        let (ram, _, rows, log, generation) = fixture();
        let mut store = ReplaceDuringPrepare(&rows, &log, false);
        assert_eq!(
            snapshot(&rows, &log, &ram, generation, 0x10000, 4, &mut store).err(),
            Some(Refusal::Changed)
        );
    }
    #[test]
    fn partial_file_alias_starting_inside_copy_is_refused() {
        let (ram, block, rows, log, _) = fixture();
        let (alias, _) = crate::raw_unsafe::test_owned_ram(0x2000);
        ram.add(super::super::RamBlock {
            gpa: 0x100000,
            mem: alias,
            fd: block.fd,
            fd_off: 0x4000,
        });
        let generation = ram.generation.load(Ordering::Relaxed);
        assert_eq!(
            snapshot(&rows, &log, &ram, generation, 0x10000, 0x8000, &mut NoStore).err(),
            Some(Refusal::Topology)
        );
    }
    #[test]
    fn publication_admission_rechecks_exact_source_and_retains_guards() {
        let (ram, block, rows, log, generation) = fixture();
        let s = snapshot(&rows, &log, &ram, generation, 0x10000, 4, &mut NoStore).unwrap();
        assert_eq!(
            s.admit(|| {
                assert!(matches!(rows.try_write(), Err(TryLockError::WouldBlock)));
                assert!(matches!(
                    ram.blocks.try_write(),
                    Err(TryLockError::WouldBlock)
                ));
                7
            }),
            Ok(7)
        );
        let guard = rows.write().unwrap();
        assert_eq!(s.admit(|| panic!("no admission")), Err(Refusal::Busy));
        drop(guard);
        rows.write()
            .unwrap()
            .insert(0x10000, (0x8000, 4, true, MapPerm::READ_WRITE));
        assert_eq!(s.admit(|| panic!("stale snapshot")), Err(Refusal::Changed));
        rows.write()
            .unwrap()
            .insert(0x10000, (0x8000, 0, true, MapPerm::READ_WRITE));
        ram.del(block.gpa);
        ram.add(block);
        assert_eq!(s.admit(|| panic!("old topology")), Err(Refusal::Topology));
        drop(rows);
        assert_eq!(s.admit(|| panic!("gone source")), Err(Refusal::Changed));
    }
    #[test]
    fn complete_ring_mapping_is_checked_without_copying_or_preparing_store_views() {
        let (ram, _, rows, log, generation) = fixture();
        rows.write()
            .unwrap()
            .insert(0x10000, (0x80000, 0, false, MapPerm::READ_WRITE));
        let source = mapped(&rows, &log, &ram, generation, 0x10000, 0x80000, 0x80000).unwrap();
        assert_eq!(
            source.admit(|| {
                assert!(rows.try_write().is_err());
                assert!(ram.blocks.try_write().is_err());
                7
            }),
            Ok(7)
        );
        assert_eq!(
            mapped(&rows, &log, &ram, generation, 0x10000, 0x80008, 0x100000).err(),
            Some(Refusal::Missing(0x90000))
        );
        assert_eq!(
            mapped(&rows, &log, &ram, generation, 0x10000, 0x80000, 0x7ffff).err(),
            Some(Refusal::Bounds)
        );
        rows.write().unwrap().clear();
        assert_eq!(
            source.admit(|| panic!("revoked ring")),
            Err(Refusal::Missing(0x10000))
        );
    }
    #[test]
    fn ring_mapping_rejects_ram_holes_aliases_revocation_and_guest_sized_work() {
        let (ram, block, rows, log, generation) = fixture();
        assert!(mapped(&rows, &log, &ram, generation, 0x10000, 0x8000, 0).is_ok());
        rows.write()
            .unwrap()
            .insert(0x10000, (0x10000, 0, true, MapPerm::READ_WRITE));
        assert_eq!(
            mapped(&rows, &log, &ram, generation, 0x10000, 0x10000, 0).err(),
            Some(Refusal::Topology)
        );
        assert_eq!(
            mapped(&rows, &log, &ram, generation, 0x10001, 8, 0).err(),
            Some(Refusal::Bounds)
        );
        assert_eq!(
            mapped(&rows, &log, &ram, generation, 0x10000, (8 << 20) + 8, 0).err(),
            Some(Refusal::Bounds)
        );
        ram.del(block.gpa);
        assert_eq!(
            mapped(&rows, &log, &ram, generation, 0x10000, 8, 0).err(),
            Some(Refusal::Topology)
        );
    }
    struct CheckedStore<'a> {
        rows: &'a PlacedRows,
        ram: &'a RamMap,
        fail: bool,
        prepared: bool,
    }
    impl Store for CheckedStore<'_> {
        fn prepare(&mut self, spans: &[Span]) -> Result<(), Refusal> {
            assert!(spans.iter().any(|s| !s.ram));
            assert!(self.rows.try_write().is_ok());
            assert!(self.ram.blocks.try_write().is_ok());
            self.prepared = true;
            Ok(())
        }
        fn copy(&self, off: u64, bytes: &mut [u8]) -> Result<(), Refusal> {
            assert!(self.prepared);
            assert!(matches!(
                self.rows.try_write(),
                Err(TryLockError::WouldBlock)
            ));
            assert!(matches!(
                self.ram.blocks.try_write(),
                Err(TryLockError::WouldBlock)
            ));
            if self.fail {
                return Err(Refusal::Store);
            }
            for (i, byte) in bytes.iter_mut().enumerate() {
                *byte = (off + i as u64) as u8;
            }
            Ok(())
        }
    }
    #[test]
    fn mixed_ram_store_copy_holds_guards_and_store_error_returns_no_snapshot() {
        let (ram, block, rows, log, generation) = fixture();
        block.mem.write_from(0, &[7, 8]);
        {
            let mut r = rows.write().unwrap();
            r.insert(0x10000, (2, 0, true, MapPerm::READ_WRITE));
            r.insert(0x10002, (2, 0x20, false, MapPerm::READ_WRITE));
        }
        let mut store = CheckedStore {
            rows: &rows,
            ram: &ram,
            fail: false,
            prepared: false,
        };
        let s = snapshot(&rows, &log, &ram, generation, 0x10000, 4, &mut store).unwrap();
        assert_eq!(s.bytes, [7, 8, 0x20, 0x21]);
        store.fail = true;
        assert_eq!(
            snapshot(&rows, &log, &ram, generation, 0x10000, 4, &mut store).err(),
            Some(Refusal::Store)
        );
    }
    #[test]
    fn fragmented_rows_exceeding_trusted_piece_budget_refuse_before_copy() {
        let (ram, _, rows, log, generation) = fixture();
        {
            let mut r = rows.write().unwrap();
            r.clear();
            for i in 0..=PIECES as u64 {
                r.insert(0x10000 + i, (1, i, true, MapPerm::READ_WRITE));
            }
        }
        assert_eq!(
            snapshot(
                &rows,
                &log,
                &ram,
                generation,
                0x10000,
                PIECES + 1,
                &mut NoStore
            )
            .err(),
            Some(Refusal::Bounds)
        );
    }
}
