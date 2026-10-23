//! SECURITY AUDIT 2026-10-21, finding SA-03 (docs/audits/2026-10-21-security-audit.md): EXPECTED RED.
//!
//! A guest with more than `MAX_SPACES_PER_WALK` (64) rooted VA spaces (each unprivileged guest process that
//! opens the GPU roots one) makes an `ALL_PDB` MMU invalidate unservable: `VaManager::on_invalidate` refuses
//! the whole batch ("N objects in one walk; the walk kernel carries 64"), `refuse_batch` only bumps the
//! `unreconciled` counter, and the trigger stays armed. The guest kernel polls the trigger bit with its RM
//! locks held (the run-223 hang class). The expected behaviour is that the invalidate is split into walks of at
//! most 64 spaces and cleared when the last one completes, or at least that the VA manager does not leave the
//! trigger armed for ever.
use kf_mem::ledger::{Desired, MapTarget, Mapped};
use kf_mem::vasmgr::{VaManager, VasKey, WalkDone, Walker};
use kf_trap::PdbAperture;
use kf_trap::mmuinval::{Invalidate, InvalidatePort, InvalidateRegs, PortWrite, TRIGGER_BIT};

struct Nop;
impl MapTarget for Nop {
    fn map(&self, _: &Desired, _: bool) -> Result<Mapped, String> {
        Ok(Mapped::Placed)
    }
    fn unmap(&self, _: u64, _: bool) -> Result<(), String> {
        Ok(())
    }
    fn invalidate(&self) -> Result<(), String> {
        Ok(())
    }
}

/// A walker that accepts any walk of at most 64 entries (the real kernel's `KF_MAX_PDB`) and records the sizes.
#[derive(Default)]
struct Rec {
    submitted: Vec<usize>,
    pending: Option<Vec<kf_cuda::WalkEntry>>,
    generation: u64,
}
impl Walker for Rec {
    fn submit(&mut self, entries: &[kf_cuda::WalkEntry]) -> Result<(), String> {
        assert!(
            entries.len() <= 64,
            "the manager must never exceed the kernel's capacity"
        );
        assert!(self.pending.is_none());
        self.submitted.push(entries.len());
        self.pending = Some(entries.to_vec());
        Ok(())
    }
    fn poll(&mut self) -> Result<Option<WalkDone>, String> {
        let Some(entries) = self.pending.take() else {
            return Ok(None);
        };
        self.generation += 1;
        Ok(Some(WalkDone {
            generation: self.generation,
            entries: entries
                .into_iter()
                .map(|e| kf_mem::vasmgr::EntryDiff {
                    pdb: e.pdb,
                    slot: e.slot,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }))
    }
    fn ack(&mut self, _: u64, _: Vec<u8>) -> Result<(), String> {
        Ok(())
    }
    fn reset(&mut self, _: u32) -> Result<(), String> {
        Ok(())
    }
    fn slots(&self) -> u32 {
        128
    }
}

#[test]
fn an_all_pdb_invalidate_over_more_than_64_rooted_spaces_is_not_left_armed_for_ever() {
    let port = InvalidatePort::new(InvalidateRegs::from_usermode_base(0xBB_0000).unwrap());
    let mut va = VaManager::new(Rec::default(), 1 << 32, Box::new(|gpa, _| Some(gpa)));
    for k in 0..70u64 {
        va.table.insert(VasKey(k), Nop);
        va.table
            .set_root(VasKey(k), 0x10_0000 * (k + 1), PdbAperture::Vidmem)
            .unwrap();
    }
    // The guest kernel's ALL_PDB invalidate, through the register port as a vCPU does it.
    let r = port.regs();
    let (lo, hi) = Invalidate::encode_pdb(0, PdbAperture::Vidmem);
    port.write(r.pdb, lo);
    port.write(r.upper_pdb, hi);
    let PortWrite::Publish(req) = port.write(r.trigger, TRIGGER_BIT | 1 | 0b10) else {
        panic!("the trigger did not publish");
    };
    va.on_invalidate(req, port.trigger());
    assert_eq!(va.walker().submitted, vec![64]);
    assert_ne!(port.trigger().read(), 0);
    let first = va.on_walk_ready(port.trigger());
    assert!(first.completed.is_empty() && first.unreconciled.is_empty());
    assert_eq!(va.walker().submitted, vec![64, 6]);
    assert_eq!(va.stats.spaces_reconciled, 64);
    assert_ne!(port.trigger().read(), 0, "six spaces are still owed");
    let last = va.on_walk_ready(port.trigger());
    assert_eq!(
        last.completed,
        vec![(req.seq, kf_trap::ClearOutcome::Cleared)]
    );
    assert!(last.unreconciled.is_empty());
    assert_eq!(va.walker().submitted.iter().sum::<usize>(), 70);
    assert_eq!(va.stats.spaces_reconciled, 70);
    assert_eq!(va.stats.cleared, 1);
    assert_eq!(va.stats.unreconciled, 0);
    assert_eq!(port.trigger().read(), 0);
    assert!(!va.in_flight());
}
