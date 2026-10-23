//! SA-14 FALSIFIER: latch-only writes change either aperture or raw scope of an
//! already armed sequence. A snapshot must bind all three, not only the address.
use kf_trap::mmuinval::TRIGGER_BIT;
use kf_trap::{Invalidate, InvalidatePort, InvalidateRegs, PdbAperture, PortWrite};
fn armed() -> (InvalidatePort, kf_trap::InvalidateRequest) {
    let p = InvalidatePort::new(InvalidateRegs::from_usermode_base(0xbb_0000).unwrap());
    let r = p.regs();
    let (lo, hi) = Invalidate::encode_pdb(0x1234_5000, PdbAperture::Vidmem);
    p.write(r.pdb, lo);
    p.write(r.upper_pdb, hi);
    let PortWrite::Publish(request) = p.write(r.trigger, TRIGGER_BIT | 3) else {
        panic!("publish");
    };
    (p, request)
}
#[test]
fn a_later_latch_cannot_change_the_armed_aperture() {
    let (p, request) = armed();
    let (lo, hi) = Invalidate::encode_pdb(request.inval.pdb, PdbAperture::Sysmem);
    p.write(p.regs().pdb, lo);
    p.write(p.regs().upper_pdb, hi);
    assert_eq!(p.armed_request(), Some(request));
}
#[test]
fn an_untriggered_scope_write_cannot_change_the_armed_raw_word() {
    let (p, request) = armed();
    assert_eq!(p.write(p.regs().trigger, 0x1234), PortWrite::Latched);
    assert_eq!(p.armed_request(), Some(request));
}
