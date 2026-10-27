//! SA-06. FALSIFIER: admission can exceed the per-aperture placement budget,
//! perform a host operation after refusal, or lose capacity after a successful unmap.
use kf_host::MapPerm;
use kf_mem::cpuwin::{CpuWindow, ViewOps};
use kf_mem::ledger::{Desired, MapTarget};
use std::cell::Cell;

const BUDGET: usize = 8192;
#[derive(Default)]
struct Ops {
    arms: Cell<usize>,
    ram: Cell<usize>,
    views: Cell<usize>,
    releases: Cell<usize>,
}
impl ViewOps for Ops {
    type View = u64;
    fn arm_store(&self, off: u64, _: u64) -> Result<u64, String> {
        self.arms.set(self.arms.get() + 1);
        Ok(off)
    }
    fn place_view(&self, _: u64, _: u64, _: &u64) -> Result<(), String> {
        self.views.set(self.views.get() + 1);
        Ok(())
    }
    fn place_ram(&self, _: u64, _: u64, _: u64) -> Result<(), String> {
        self.ram.set(self.ram.get() + 1);
        Ok(())
    }
    fn sink(&self, _: u64, _: u64) -> Result<(), String> {
        Ok(())
    }
    fn release(&self, _: u64) -> Result<(), String> {
        self.releases.set(self.releases.get() + 1);
        Ok(())
    }
}
fn row(i: usize, ram: bool) -> Desired {
    Desired {
        va: i as u64 * 0x2000,
        off: i as u64 * 0x1000,
        len: 0x1000,
        ram,
        kind: 0,
        perm: MapPerm::READ_WRITE,
        leaf: 0,
    }
}
fn fill(ram: bool) -> CpuWindow<Ops> {
    let w = CpuWindow::new(Ops::default(), 1 << 30);
    for i in 0..BUDGET {
        w.map(&row(i, ram), false).unwrap();
    }
    w
}
#[test]
fn store_views_refuse_before_arming_a_host_view_past_the_budget() {
    let w = fill(false);
    let result = w.map(&row(BUDGET, false), false);
    assert!(
        result.is_err(),
        "the guest must not allocate another host fd/VMA"
    );
    assert_eq!(w.placements(), BUDGET);
    assert_eq!(w.ops().arms.get(), BUDGET);
    assert_eq!(w.ops().views.get(), BUDGET);
}
#[test]
fn guest_ram_placements_cannot_bypass_the_vma_budget() {
    let w = fill(true);
    assert!(w.map(&row(BUDGET, true), false).is_err());
    assert_eq!(w.placements(), BUDGET);
    assert_eq!(w.ops().ram.get(), BUDGET);
    assert_eq!(w.ops().arms.get(), 0);
}
#[test]
fn unmapping_releases_capacity_without_eviction_or_a_global_reset() {
    let w = fill(false);
    assert!(w.map(&row(BUDGET, false), false).is_err());
    w.unmap(0, false).unwrap();
    assert_eq!(w.ops().releases.get(), 1);
    w.map(&row(BUDGET, false), false).unwrap();
    assert_eq!(w.placements(), BUDGET);
    assert_eq!(w.ops().arms.get(), BUDGET + 1);
}
#[test]
fn ram_and_store_share_one_placement_budget() {
    let w = CpuWindow::new(Ops::default(), 1 << 30);
    for i in 0..BUDGET {
        w.map(&row(i, i % 2 == 0), false).unwrap();
    }
    assert!(w.map(&row(BUDGET, false), false).is_err());
    assert!(w.map(&row(BUDGET, true), false).is_err());
    assert_eq!(w.placements(), BUDGET);
}
