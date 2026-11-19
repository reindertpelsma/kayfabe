//! SECURITY AUDIT 2026-10-21, finding SA-14 (docs/audits/2026-10-21-security-audit.md): EXPECTED RED.
//!
//! `InvalidatePort::armed_request` rebuilds the request the VA manager serves from the LIVE latch cells, not
//! from what the guest wrote before it armed the trigger. A guest kernel that writes PDB_LO/HI for space B after
//! arming the trigger for space A (a latch-only write, allowed while the trigger is busy) makes the VA thread
//! serve (and clear the trigger over) space B: A's walk is skipped and A's stale translations survive a
//! completed invalidate. Guest userspace cannot write these registers, so this is guest-root-against-itself
//! (in-guest A.9: the guest kernel owns its own page tables anyway); it is recorded because the doc comment's
//! soundness argument ("the guest issues the next invalidate only after this one reads idle") is a guest
//! behaviour, not a host guarantee.
use kf_trap::mmuinval::{Invalidate, InvalidatePort, InvalidateRegs, PortWrite, TRIGGER_BIT};
use kf_trap::PdbAperture;

#[test]
fn a_later_latch_write_does_not_change_the_armed_pdb() {
    let p = InvalidatePort::new(InvalidateRegs::from_usermode_base(0xBB_0000).unwrap());
    let r = p.regs();
    let (la, ha) = Invalidate::encode_pdb(0x0100_0000, PdbAperture::Vidmem);
    let (lb, hb) = Invalidate::encode_pdb(0x0200_0000, PdbAperture::Vidmem);
    p.write(r.pdb, la);
    p.write(r.upper_pdb, ha);
    let PortWrite::Publish(a) = p.write(r.trigger, TRIGGER_BIT | 1) else {
        panic!("the trigger did not publish");
    };
    // latch-only writes while the trigger is armed
    p.write(r.pdb, lb);
    p.write(r.upper_pdb, hb);
    let served = p.armed_request().expect("still armed");
    assert_eq!(
        served.inval.pdb, a.inval.pdb,
        "the request served for sequence {} names a different PDB than the one the guest armed it for",
        served.seq
    );
}
