//! w825 — a doorbell deferred on the trap and forwarded by the worker must read `forwarded>0`.
//! `[measured w825base]` without it five arms whose client passed were graded FAIL.
use kayfabe_device::dbtable::{DoorbellClass, DoorbellLedger};

fn row(l: &DoorbellLedger, tok: &str) -> String {
    l.render().lines().find(|r| r.contains(tok)).unwrap().to_string()
}

#[test]
fn a_worker_forward_clears_the_stranded_reading() {
    let l = DoorbellLedger::default();
    l.record(0x3, DoorbellClass::Emulated, false);
    assert!(row(&l, "tok=0x00000003").ends_with("emulated=1 other=0 forwarded=0"));
    l.record_deferred_forward(0x3);
    assert!(row(&l, "tok=0x00000003").ends_with("emulated=1 other=0 forwarded=1"));
}

#[test]
fn a_locally_served_deferral_stays_stranded() {
    // Known-positive for the gate: nothing but a real forward may clear it.
    let l = DoorbellLedger::default();
    l.record(0x4, DoorbellClass::Emulated, false);
    assert!(row(&l, "tok=0x00000004").ends_with("forwarded=0"));
}
