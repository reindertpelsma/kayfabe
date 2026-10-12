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

/// Trusted per-worker copy budget, independent of guest PB/ring sizes.
pub(crate) const MAX_BYTES: usize = 64 << 10;
const PIECES: usize = 64;

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
    pub bytes: Vec<u8>,
    pub epoch: u64,
    rows: Weak<RwLock<std::collections::BTreeMap<u64, super::PlacedRow>>>,
    spans: Vec<Span>,
    va: u64,
    end: u64,
    ram: &'a RamMap,
    generation: u64,
}

impl Snapshot<'_> {
    /// Revalidate the exact confirmed source span and birth RAM generation,
    /// retaining both guards across one bounded CPU publication transaction.
    /// The callback MUST NOT perform an RM ioctl, acquire channel/graph locks,
    /// or wait/retry. Channel identity and the VA-manager ordering gate are
    /// separate prerequisites. This does not pin engine operands or complete work.
    #[allow(dead_code)] // Called by the forthcoming runtime USER publisher.
    pub(crate) fn admit<T>(&self, publish: impl FnOnce() -> T) -> Result<T, Refusal> {
        let rows = self.rows.upgrade().ok_or(Refusal::Changed)?;
        let guard = try_read(&rows)?;
        if resolve(&guard, self.va, self.end)? != self.spans {
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

pub(crate) fn mapped<'a>(rows: &PlacedRows, log: &RowsLog, ram: &'a RamMap,
    generation: u64, va: u64, bytes: u64, store_bound: u64)
    -> Result<Mapping<'a>, Refusal> {
    // Allocation policy, not a die/driver fact: at most2^20 source GP entries.
    if bytes == 0 || bytes > 8 << 20 || va & 7 != 0 {
        return Err(Refusal::Bounds);
    }
    let end = va.checked_add(bytes).filter(|end| *end < 1 << 40)
        .ok_or(Refusal::Bounds)?;
    let guard = try_read(rows)?;
    let epoch = log.epoch();
    let spans = resolve(&guard, va, end)?;
    let blocks = try_read(&ram.blocks)?;
    if generation == 0 || ram.generation.load(Ordering::Relaxed) != generation {
        return Err(Refusal::Topology);
    }
    if blocks.len() > PIECES { return Err(Refusal::Bounds); }
    let fd = blocks.iter().find_map(|b| b.fd);
    let mut pieces = 0;
    for span in &spans {
        let end = span.off.checked_add(span.len).ok_or(Refusal::Bounds)?;
        if !span.ram {
            if end > store_bound { return Err(Refusal::Bounds); }
            continue;
        }
        let fd = fd.ok_or(Refusal::Topology)?;
        let mut at = span.off;
        while at < end {
            if pieces == PIECES { return Err(Refusal::Bounds); }
            pieces += 1;
            let mut covers = blocks.iter().enumerate().filter(|(_, b)|
                b.fd == Some(fd) && at >= b.fd_off && b.fd_off
                    .checked_add(b.mem.len() as u64).is_some_and(|e| at < e));
            let (index, block) = covers.next().ok_or(Refusal::Topology)?;
            if covers.next().is_some() { return Err(Refusal::Topology); }
            let until = block.fd_off.checked_add(block.mem.len() as u64)
                .ok_or(Refusal::Bounds)?.min(end);
            if blocks.iter().enumerate().any(|(i, other)| i != index
                && other.fd == Some(fd) && other.fd_off < until
                && other.fd_off.checked_add(other.mem.len() as u64).is_none_or(|e| at < e)) {
                return Err(Refusal::Topology);
            }
            at = until;
        }
    }
    Ok(Mapping(Snapshot { bytes: Vec::new(), epoch, rows: Arc::downgrade(rows),
        spans, va, end, ram, generation }))
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
) -> Result<Vec<Span>, Refusal> {
    let mut spans = Vec::new();
    spans
        .try_reserve_exact(PIECES)
        .map_err(|_| Refusal::Allocation)?;
    let mut at = va;
    while at < end {
        if spans.len() == PIECES {
            return Err(Refusal::Bounds);
        }
        let (&base, &(len, off, ram, perm)) =
            rows.range(..=at).next_back().ok_or(Refusal::Missing(at))?;
        let row_end = base
            .checked_add(len)
            .filter(|e| *e > at)
            .ok_or(Refusal::Missing(at))?;
        let len = (row_end - at).min(end - at);
        let off = off
            .checked_add(at - base)
            .filter(|o| o.checked_add(len).is_some())
            .ok_or(Refusal::Bounds)?;
        spans.push(Span {
            ram,
            off,
            len,
            perm,
        });
        at += len;
    }
    Ok(spans)
}

/// Snapshot one bounded span. No row/RAM pointer escapes; errors expose no
/// partial output. The topology generation comes from this channel's birth,
/// never refreshed after revocation. This is not a VA-manager submission gate.
#[allow(dead_code)] // Runtime USER source consumer is the next adapter.
pub(crate) fn snapshot<'a>(
    rows: &PlacedRows,
    log: &RowsLog,
    ram: &'a RamMap,
    generation: u64,
    va: u64,
    bytes: usize,
    store: &mut impl Store,
) -> Result<Snapshot<'a>, Refusal> {
    if bytes == 0 || bytes > MAX_BYTES || va.checked_add(bytes as u64).is_none() {
        return Err(Refusal::Bounds);
    }
    let end = va + bytes as u64;
    let first = try_read(rows)?;
    let epoch = log.epoch();
    let spans = resolve(&first, va, end)?;
    drop(first);
    // This is the only stage allowed to arm native CPU views. An epoch change
    // during preparation discards this attempt before copying any source bytes.
    store.prepare(&spans)?;
    let mut out = Vec::new();
    out.try_reserve_exact(bytes)
        .map_err(|_| Refusal::Allocation)?;
    out.resize(bytes, 0);
    let rows_guard = try_read(rows)?;
    if log.epoch() != epoch {
        return Err(Refusal::Changed);
    }
    // Steer can remove a row without updating RowsLog. Re-resolve the whole
    // source as well: an epoch is not the sole authority for backing identity.
    if resolve(&rows_guard, va, end)? != spans {
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
        let dest = out.get_mut(done..done + len).ok_or(Refusal::Bounds)?;
        if !span.ram {
            store.copy(span.off, dest)?;
        } else {
            let fd = first_fd.ok_or(Refusal::Topology)?;
            let mut copied = 0usize;
            while copied < len {
                if ram_pieces == PIECES {
                    return Err(Refusal::Bounds);
                }
                ram_pieces += 1;
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
                let pos = usize::try_from(off - b.fd_off).map_err(|_| Refusal::Bounds)?;
                let n = (b.mem.len() - pos).min(len - copied);
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
                if !b.mem.read_into(pos, &mut dest[copied..copied + n]) {
                    return Err(Refusal::Topology);
                }
                copied += n;
            }
        }
        done += len;
    }
    Ok(Snapshot {
        bytes: out,
        epoch,
        rows: Arc::downgrade(rows),
        spans,
        va,
        end,
        ram,
        generation,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kf_host::MapPerm;
    use std::sync::atomic::Ordering;
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
        rows.write().unwrap().insert(0x10000, (0x80000, 0, false, MapPerm::READ_WRITE));
        let source = mapped(&rows, &log, &ram, generation, 0x10000, 0x80000, 0x80000).unwrap();
        assert_eq!(source.admit(|| {
            assert!(rows.try_write().is_err());
            assert!(ram.blocks.try_write().is_err());
            7
        }), Ok(7));
        assert_eq!(mapped(&rows, &log, &ram, generation, 0x10000, 0x80008, 0x100000).err(), Some(Refusal::Missing(0x90000)));
        assert_eq!(mapped(&rows, &log, &ram, generation, 0x10000, 0x80000, 0x7ffff).err(), Some(Refusal::Bounds));
        rows.write().unwrap().clear();
        assert_eq!(source.admit(|| panic!("revoked ring")), Err(Refusal::Missing(0x10000)));
    }
    #[test]
    fn ring_mapping_rejects_ram_holes_aliases_revocation_and_guest_sized_work() {
        let (ram, block, rows, log, generation) = fixture();
        assert!(mapped(&rows, &log, &ram, generation, 0x10000, 0x8000, 0).is_ok());
        rows.write().unwrap().insert(0x10000, (0x10000, 0, true, MapPerm::READ_WRITE));
        assert_eq!(mapped(&rows, &log, &ram, generation, 0x10000, 0x10000, 0).err(), Some(Refusal::Topology));
        assert_eq!(mapped(&rows, &log, &ram, generation, 0x10001, 8, 0).err(), Some(Refusal::Bounds));
        assert_eq!(mapped(&rows, &log, &ram, generation, 0x10000, (8 << 20) + 8, 0).err(), Some(Refusal::Bounds));
        ram.del(block.gpa);
        assert_eq!(mapped(&rows, &log, &ram, generation, 0x10000, 8, 0).err(), Some(Refusal::Topology));
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
