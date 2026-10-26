//! ADVERSARY (breaker, 2026-10-26): the batch ledger's per-call cost against the number of
//! mappings it already holds (I4). A null host isolates the ledger code. The table is printed by
//! `breaker_ledger_cost_table`; the other tests assert that a single-row map / unmap / view /
//! invalidate does not scale with the number of other rows.

use kf_mem::batch::{BatchedVas, SpaceVerbs};
use kf_mem::ledger::{Desired, Mapped, SkedRow};
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::time::Instant;

const P: u64 = 4096;
const BASE: u64 = 0x2_0000_0000;

struct NullHost {
    next: AtomicU32,
}
impl SpaceVerbs for &NullHost {
    fn invalidate(&self) -> Result<(), String> {
        Ok(())
    }
    fn map_row(&self, _: &Desired, _: bool) -> Result<Mapped, String> {
        Ok(Mapped::Placed)
    }
    fn map_sked_row(&self, _: &SkedRow, _: bool) -> Result<Mapped, String> {
        Ok(Mapped::Placed)
    }
    fn map_scattered(
        &self,
        _: std::os::fd::BorrowedFd<'_>,
        _: &[Desired],
        _: bool,
    ) -> Result<u32, String> {
        Ok(self.next.fetch_add(1, Relaxed))
    }
    fn unmap_whole(&self, _: u64, _: bool) -> Result<(), String> {
        Ok(())
    }
    fn unmap_row(&self, _: u64, _: u64, _: bool) -> Result<(), String> {
        Ok(())
    }
    fn unmap_range(&self, _: u64, _: u64, _: bool) -> Result<(), String> {
        Ok(())
    }
    fn free(&self, _: u32) -> Result<(), String> {
        Ok(())
    }
    fn splits_safely(&self, _: u64, _: u64) -> bool {
        false
    }
    fn reserve(&self, _: u64, _: u64) -> Result<u32, String> {
        Ok(self.next.fetch_add(1, Relaxed))
    }
    fn map_row_in(&self, _: u32, _: &Desired, _: bool) -> Result<Mapped, String> {
        Ok(Mapped::Placed)
    }
    fn map_sked_in(&self, _: u32, _: &SkedRow, _: bool) -> Result<Mapped, String> {
        Ok(Mapped::Placed)
    }
    fn map_scattered_in(
        &self,
        _: u32,
        _: std::os::fd::BorrowedFd<'_>,
        _: &[Desired],
        _: bool,
    ) -> Result<u32, String> {
        Ok(self.next.fetch_add(1, Relaxed))
    }
    fn unmap_in(&self, _: u32, _: u64, _: u64, _: bool) -> Result<(), String> {
        Ok(())
    }
}

fn row(va: u64, len: u64) -> Desired {
    Desired {
        va,
        len,
        off: va,
        ram: true,
        kind: 0,
        perm: kf_host::MapPerm::READ_WRITE,
        leaf: len,
    }
}

/// A ledger holding `n` one-page rows (every other page, so no row merges), plus the timings of
/// one more map, one range unmap, one view and one invalidate with a pending retirement.
fn per_op_us(n: u64, reps: u64) -> [f64; 4] {
    let host = NullHost {
        next: AtomicU32::new(0x100),
    };
    let bv = BatchedVas::with_low_reserve(&host, false);
    for i in 0..n {
        bv.map(&row(BASE + 2 * i * P, P), true).unwrap();
    }
    let top = BASE + 2 * n * P + 64 * P;
    let t = |f: &mut dyn FnMut(u64)| {
        let t0 = Instant::now();
        for k in 0..reps {
            f(k);
        }
        t0.elapsed().as_secs_f64() * 1e6 / reps as f64
    };
    let map = t(&mut |k| {
        bv.map(&row(top + 2 * k * P, P), true).unwrap();
    });
    let unmap = t(&mut |k| {
        bv.unmap_range(top + 2 * k * P, P, true).unwrap();
    });
    let view = t(&mut |k| {
        let _ = bv.own_view(top + 2 * k * P, top + 2 * k * P + 16 * P);
    });
    let inval = t(&mut |_| {
        bv.invalidate().unwrap();
    });
    [map, unmap, view, inval]
}

#[test]
fn breaker_ledger_cost_table() {
    eprintln!("rows   | map us | unmap_range us | own_view us | invalidate us");
    for n in [100u64, 1000, 5000, 20000] {
        let r = per_op_us(n, 50);
        eprintln!(
            "{n:6} | {:6.1} | {:14.1} | {:11.1} | {:13.1}",
            r[0], r[1], r[2], r[3]
        );
    }
}

/// I4: a single-row ledger operation is O(log n), not O(rows held).
#[test]
fn breaker_i4_ledger_ops_do_not_scale_with_the_rows_held() {
    let small = per_op_us(500, 100);
    let large = per_op_us(10_000, 100);
    for (k, name) in ["map", "unmap_range", "own_view", "invalidate"]
        .iter()
        .enumerate()
    {
        let (a, b) = (small[k].max(0.2), large[k]);
        assert!(
            b / a < 4.0,
            "{name}: 20x the rows costs {:.1}x ({a:.1} us -> {b:.1} us)",
            b / a
        );
    }
}
