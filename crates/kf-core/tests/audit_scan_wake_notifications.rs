//! RC-02. FALSIFIER: scan exhaustion loses the current worker's rescan or a
//! parked peer's notification, or consumed stale hints prevent an idle park.
use kf_core::plane::{HostOps, Translatable, Vmm};
use kf_core::{HostSlice, Owner, Plane, Step, VmCaps};
use kf_trap::token::Route;
use kf_trap::trap::Class;
use std::sync::atomic::{AtomicUsize, Ordering};
#[derive(Default)]
struct Host {
    signals: AtomicUsize,
}
impl HostOps for Host {
    fn ring_host(&self, _: u32) {}
    fn run_translated(&self, _: u32, _: u64) -> bool {
        true
    }
    fn run_emulated(&self, _: u32, _: u64) {}
    fn apply_register(&self, _: u8, _: u32, _: u64, _: u8) {}
    fn operands_translatable(&self, _: u32, _: u64) -> Translatable {
        Translatable::Yes
    }
    fn forge_completion(&self, _: u32) {}
    fn refuse_and_poison(&self, _: u32) {}
    fn fault_channel(&self, _: u32) {}
    fn map_guest_slice(&self, _: HostSlice) {}
    fn teardown_step(&self, _: Step) {}
}
fn ring(p: &Plane<'_>, caps: &mut VmCaps, token: u32) {
    p.allocate_channel(caps, token, Route::Translated, 1000 + token, Owner::User)
        .unwrap();
    p.trap_write(Class::Doorbell, 0, 0, u64::from(token), 4);
}
fn peer_and_active_scanner(limit: usize) {
    let vmm = Vmm::new();
    let p = Plane::new(&vmm, 256, 0xff);
    let host = Host::default();
    let mut caps = VmCaps::from_declared(512, 8, 8, 8);
    for token in 0..limit as u32 {
        ring(&p, &mut caps, token);
        assert!(p.retire_channel_token(&mut caps, token));
    }
    ring(&p, &mut caps, limit as u32 + 1);
    let seen = p.worker_wake().seen();
    assert!(p.worker_wake().try_park(seen)); // model a peer registered on the real wake word
    let n = p.worker_pass(&host, &mut Vec::new(), limit);
    let parked = p.worker_wake().try_park(seen);
    if parked {
        p.worker_wake().unpark();
    }
    p.worker_wake().unpark(); // peer
    assert_eq!(n, 0, "stale hints must never count as served work");
    assert!(!parked, "the active scanner still owes a rescan");
    assert_eq!(
        host.signals.load(Ordering::Relaxed),
        1,
        "SignalOne must reach the host notifier seam"
    );
    assert_eq!(p.worker_pass(&host, &mut Vec::new(), limit), 1);
    assert_eq!(p.worker_wake().pollers(), 0);
}
#[test]
fn a_full_stale_word_preserves_the_rescan_and_notifies_a_parked_peer() {
    peer_and_active_scanner(64);
}
#[test]
fn a_stale_bit_preserves_a_same_word_live_bit_and_notifies_a_parked_peer() {
    peer_and_active_scanner(1);
}
#[test]
fn exhausted_stale_hints_eventually_allow_an_idle_park() {
    let vmm = Vmm::new();
    let p = Plane::new(&vmm, 256, 0xff);
    let host = Host::default();
    let mut caps = VmCaps::from_declared(512, 8, 8, 8);
    ring(&p, &mut caps, 0);
    assert!(p.retire_channel_token(&mut caps, 0));
    assert_eq!(p.worker_pass(&host, &mut Vec::new(), 1), 0);
    let seen = p.worker_wake().seen();
    assert_eq!(p.worker_pass(&host, &mut Vec::new(), 1), 0);
    assert!(p.worker_wake().try_park(seen));
    p.worker_wake().unpark();
}
