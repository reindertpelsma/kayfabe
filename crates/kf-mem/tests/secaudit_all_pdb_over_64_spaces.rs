//! SECURITY AUDIT 2026-10-21, finding SA-03 (docs/audits/2026-10-21-security-audit.md): EXPECTED RED.
//!
//! A guest with more than `MAX_SPACES_PER_WALK` (64) rooted VA spaces (each unprivileged guest process that
//! opens the GPU roots one) makes an `ALL_PDB` MMU invalidate unservable: `VaManager::on_invalidate` refuses
//! the whole batch ("N objects in one walk; the walk kernel carries 64"), `refuse_batch` only bumps the
//! `unreconciled` counter, and the trigger stays armed. The guest kernel polls the trigger bit with its RM
//! locks held (the run-223 hang class). The expected behaviour is that the invalidate is split into walks of at
//! most 64 spaces and cleared when the last one completes, or at least that the VA manager does not leave the
//! trigger armed for ever.
use kf_mem::ledger::{Desired, Mapped, MapTarget};
use kf_mem::vasmgr::{VaManager, VasKey, WalkDone, Walker};
use kf_trap::mmuinval::{Invalidate, InvalidatePort, InvalidateRegs, PortWrite, TRIGGER_BIT};
use kf_trap::PdbAperture;

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
}
impl Walker for Rec {
    fn submit(&mut self, entries: &[kf_cuda::WalkEntry]) -> Result<(), String> {
        assert!(entries.len() <= 64, "the manager must never exceed the kernel's capacity");
        self.submitted.push(entries.len());
        Ok(())
    }
    fn poll(&mut self) -> Result<Option<WalkDone>, String> {
        Ok(None)
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
    // Either the manager submitted walks that cover all 70 spaces (in chunks of at most 64) ...
    let covered: usize = va.walker().submitted.iter().sum();
    // ... or it has to have cleared the trigger. What it must not do is refuse and stay armed.
    assert!(
        covered >= 70 || va.stats.unreconciled == 0,
        "ALL_PDB over 70 rooted spaces: submitted {:?}, unreconciled={}, refusals={:?}",
        va.walker().submitted,
        va.stats.unreconciled,
        va.stats.refusals
    );
}
