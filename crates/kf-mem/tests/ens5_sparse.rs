//! ENS5 drives the actual host reservation model through the actual memory apply/ACK path.
#![allow(dead_code)]
pub use kf_host::{RmError, diag};
mod channel {
    pub use kf_host::channel::{GuestVaRange, TWIN_VA_FLOOR, VA_ROW_WRAPS};
}
#[path = "../../kf-host/src/sparse.rs"]
mod host_sparse;
use host_sparse::{Ops, Reservations, SPARSE_GRAIN};
use kf_mem::apply::{ApplyCfg, DiffRun, PermPolicy, apply_entry};
use kf_mem::ledger::{Desired, MapTarget, Mapped};
use std::cell::{Cell, RefCell};
const VA: u64 = 1 << 40;
struct Target {
    tree: RefCell<Reservations>,
    calls: Cell<u64>,
}
impl Ops for Target {
    fn reserve(&self, _: u64, _: u64, _: bool) -> Result<u32, RmError> {
        panic!("closed fallback must not call RM")
    }
    fn release(&self, _: u32) -> Result<(), RmError> {
        panic!("closed fallback must not release a populated block")
    }
}
impl MapTarget for Target {
    fn gpu_space(&self) -> bool {
        true
    }
    fn sparse_twin(&self) -> bool {
        true
    }
    fn map_sparse(&self, va: u64, len: u64, _: bool) -> Result<Mapped, String> {
        self.calls.set(self.calls.get() + 1);
        self.tree
            .borrow_mut()
            .admit(self, va, len)
            .map(|ok| {
                if ok {
                    Mapped::Placed
                } else {
                    Mapped::HeldByHost
                }
            })
            .map_err(|e| format!("{e:?}"))
    }
    fn map(&self, _: &Desired, _: bool) -> Result<Mapped, String> {
        unreachable!()
    }
    fn unmap(&self, _: u64, _: bool) -> Result<(), String> {
        unreachable!()
    }
    fn invalidate(&self) -> Result<(), String> {
        Ok(())
    }
}
fn repeated_refresh(offset: u64) {
    let mut tree = Reservations::new(
        &[channel::GuestVaRange {
            handle: 1,
            lo: VA,
            hi: VA + 16 * 1024 * 1024,
        }],
        8192,
    );
    tree.mark(VA + offset, 4096, true);
    let t = Target {
        tree: RefCell::new(tree),
        calls: Cell::new(0),
    };
    let mut pending = vec![PermPolicy::default().diff_run(&kf_cuda::abi::KfMapRun {
        va: VA + 4 * 1024 * 1024,
        len: SPARSE_GRAIN,
        gpga: 0,
        flags: kf_cuda::abi::KFWR_RF_SPARSE,
        op: kf_cuda::abi::KFWR_OP_MAP,
        pdb_index: 0,
    })];
    for _ in 0..8 {
        let out = apply_entry(
            &t,
            &pending,
            &ApplyCfg {
                store_bytes: 1 << 30,
                grain: 4096,
                ram_offset: &|p, _| Some(p),
                usermode: None,
                per_map_kind: true,
                carve: 1 << 30,
                carve_refuse: false,
            },
        );
        let codes = out.codes;
        pending = pending
            .into_iter()
            .zip(codes)
            .filter_map(|(r, c)| (c == kf_cuda::abi::KFWR_ACK_FAILED).then_some(r))
            .collect::<Vec<DiffRun>>();
    }
    assert!(
        pending.is_empty(),
        "counted decline is committed HELD, not a refresh failure"
    );
    assert_eq!(
        t.calls.get(),
        1,
        "unchanged sparse diff must not be re-admitted every refresh"
    );
}
#[test]
fn ens5_failed_occupied_admission_does_not_reemit_each_refresh() {
    repeated_refresh(0);
}
#[test]
fn ens5_failed_occupied_suffix_does_not_reemit_each_refresh() {
    repeated_refresh(4096);
}
