//! ADVERSARY (breaker, 2026-10-26): what a sparse admission FAILURE does to the walker loop.
//! EXPECTED RED against 3e52e0161.
//!
//! `apply/sparse.rs:118-121` turns an `Err` from `MapTarget::map_sparse` into `failed[i] = true`
//! and a FAILED acknowledgement; `vasmgr.rs:1493-1495` documents the consequence: "anything not
//! applied below stays FAILED (0): the next diff re-emits it". So a sparse run whose admission
//! keeps failing is handed to the host on EVERY walk, paying the host's tree-wide admission cost
//! (kf-host `breaker_i4_*`) each time, with the VA thread (and the VM-wide walk gate) busy
//! for ever. Declines are meant to be HELD (`Ok(false)`); only a genuinely transient fault may
//! be FAILED, and even that must back off.

use kf_cuda::abi::{KFWR_ACK_FAILED, KfMapRun};
use kf_mem::apply::{ApplyCfg, PermPolicy, apply_entry};
use kf_mem::ledger::{Desired, MapTarget, Mapped, OwnView};
use std::cell::Cell;

const P: u64 = 4096;
const L: u64 = 65536;
const VA: u64 = 0x20_0000;
const SPARSE: u32 = 1 << 7;

/// A host whose sparse admission always answers the hard refusal of kf-host
/// `Reservations::admit` for a run in a populated coarse block (`Err(NoMemory)`, no counter).
struct Refusing {
    admissions: Cell<u32>,
    invalidates: Cell<u32>,
}

impl MapTarget for Refusing {
    fn own_view(&self, _: u64, _: u64) -> Option<OwnView> {
        Some(OwnView::default())
    }
    fn sparse_twin(&self) -> bool {
        true
    }
    fn gpu_space(&self) -> bool {
        true
    }
    fn map_sparse(&self, _: u64, _: u64, _: bool) -> Result<Mapped, String> {
        self.admissions.set(self.admissions.get() + 1);
        Err("sparse admit: NoMemory".into())
    }
    fn unmap_sparse(&self, _: u64, _: u64, _: bool, _: bool) -> Result<u64, String> {
        Ok(0)
    }
    fn map(&self, _: &Desired, _: bool) -> Result<Mapped, String> {
        Ok(Mapped::Placed)
    }
    fn unmap(&self, _: u64, _: bool) -> Result<(), String> {
        Ok(())
    }
    fn unmap_range(&self, _: u64, _: u64, _: bool) -> Result<(), String> {
        Ok(())
    }
    fn invalidate(&self) -> Result<(), String> {
        self.invalidates.set(self.invalidates.get() + 1);
        Ok(())
    }
}

fn sparse_run() -> kf_mem::apply::DiffRun {
    PermPolicy::default().diff_run(&KfMapRun {
        va: VA,
        len: L,
        gpga: 0,
        flags: SPARSE,
        op: 1,
        pdb_index: 0,
    })
}

/// I3/I4: ten consecutive walks of an unchanged sparse leaf whose admission is refused must not
/// hand the same run to the host ten times. Minimal sequence: one sparse leaf, walk x10.
#[test]
fn breaker_i4_a_refused_sparse_run_is_re_admitted_on_every_walk() {
    let t = Refusing {
        admissions: Cell::new(0),
        invalidates: Cell::new(0),
    };
    let cfg = ApplyCfg {
        store_bytes: 1 << 30,
        grain: P,
        ram_offset: &|g, _| Some(g),
        usermode: None,
        per_map_kind: true,
        carve: 1 << 30,
        carve_refuse: false,
    };
    let mut failed_walks = 0;
    for _ in 0..10 {
        let out = apply_entry(&t, &[sparse_run()], &cfg);
        failed_walks += u32::from(out.codes == [KFWR_ACK_FAILED]);
    }
    assert!(
        t.admissions.get() <= 2,
        "{} host admissions in 10 walks ({} ACKed FAILED): the walker re-emits a FAILED run every \
         walk, and each admission clones and rescans the host tree",
        t.admissions.get(),
        failed_walks
    );
}
