//! GPU-free observation probe for B' c452ca7cb, not a product correctness gate.
//! Overlay in kf-chan/tests and run with chaos-hooks, one test thread.
//! A legitimate wake arrives AFTER epoll returned timeout but BEFORE the next
//! pass. Both production counter branches count it despite an already-ready fd.
use kf_chan::user_loop::{Peek, Stats};
use kf_chan::worker::{self, Inflight, RelayLoop, WorkerStats};
use kf_core::{HostOps, HostSlice, Owner, Plane, Translatable, VmCaps, Vmm};
use kf_linux_raw::{Notifier, Poller};
use kf_trap::Route;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::SeqCst};
use std::time::{Duration, Instant};

static ARM: AtomicBool = AtomicBool::new(false);
static PAUSED: AtomicBool = AtomicBool::new(false);
static GO: AtomicBool = AtomicBool::new(false);

fn hook(p: kf_trap::chaos::Point) {
    if p == kf_trap::chaos::Point::WorkerAfterWait && ARM.swap(false, SeqCst) {
        PAUSED.store(true, SeqCst);
        while !GO.load(SeqCst) { std::thread::yield_now(); }
    }
}

struct Host(AtomicU64);
impl HostOps for Host {
    fn ring_host(&self, _: u32) { panic!("unexpected native ring") }
    fn run_translated(&self, _: u32, _: u64) -> bool { self.0.fetch_add(1, SeqCst); true }
    fn run_emulated(&self, _: u32, _: u64) { panic!("unexpected emulation") }
    fn apply_register(&self, _: u8, _: u32, _: u64, _: u8) {}
    fn operands_translatable(&self, _: u32, _: u64) -> Translatable { Translatable::Yes }
    fn forge_completion(&self, _: u32) { panic!("no forged completion") }
    fn refuse_and_poison(&self, _: u32) { panic!("unexpected refusal") }
    fn fault_channel(&self, _: u32) { panic!("unexpected fault") }
    fn map_guest_slice(&self, _: HostSlice) {}
    fn teardown_step(&self, _: kf_core::Step) {}
}
struct NoneInflight;
impl Inflight for NoneInflight { fn for_each_inflight(&self, _: &mut dyn FnMut(u32)) {} }
struct Fresh<'a> { pending: AtomicBool, plane: &'a Plane<'a>, wake: &'a Notifier }
impl RelayLoop for Fresh<'_> {
    fn waiting(&self) -> bool { self.pending.load(SeqCst) }
    fn unsourced(&self) -> bool { false } // the producer always signals the watched fd
    fn peek(&self, _: Peek) -> bool {
        if self.pending.swap(false, SeqCst) {
            self.plane.ring_internal(0);
            self.wake.signal().unwrap();
            true
        } else { false }
    }
}
fn wait(mut f: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(3);
    while Instant::now() < end { if f() { return true } std::thread::yield_now(); }
    false
}
fn observe(via_look: bool) {
    kf_trap::chaos::set_hook(hook);
    worker::set_park_net(true);
    ARM.store(true, SeqCst); PAUSED.store(false, SeqCst); GO.store(false, SeqCst);
    let vmm = Vmm::default();
    let plane = Plane::new(&vmm, 16, 15);
    let mut caps = VmCaps::from_declared(16, 8, 8, 8);
    plane.allocate_channel(&mut caps, 0, Route::Translated, 0, Owner::User).unwrap();
    let wake = Notifier::create().unwrap();
    let stats = Stats::default();
    let ws = WorkerStats::default();
    let host = Host(AtomicU64::new(0));
    let stop = AtomicBool::new(false);
    let relay = Fresh { pending: AtomicBool::new(false), plane: &plane, wake: &wake };
    std::thread::scope(|s| {
        let worker = s.spawn(|| {
            let poll = Poller::create().unwrap();
            poll.watch(wake.as_source_fd(), worker::WORKER_EFD_TAG).unwrap();
            worker::run_with_relay(&plane, &host, &poll, &wake, &NoneInflight, &ws, &stop,
                                   &|_| {}, &|| {}, &|| false, &relay, &stats);
        });
        let paused = wait(|| PAUSED.load(SeqCst));
        if via_look { relay.pending.store(true, SeqCst); }
        else { plane.ring_internal(0); }
        // A real registered eventfd is ready before the worker resumes. There
        // is no Marker/Popped/Contended wait or lost notification in this probe.
        wake.signal().unwrap();
        GO.store(true, SeqCst);
        let served = wait(|| host.0.load(SeqCst) > 0);
        stop.store(true, SeqCst); wake.signal().unwrap(); worker.join().unwrap();
        assert!(paused && served);
    });
    println!("via_look={via_look} served={} backstop_found_work={} timeouts_with_work={} doorbell_dispatches={}",
             host.0.load(SeqCst), stats.backstop_found_work.load(SeqCst),
             ws.timeouts_with_work.load(SeqCst), stats.wake_doorbell.load(SeqCst));
    assert!(stats.wake_doorbell.load(SeqCst) > 0, "the notification was dispatched");
    assert!(stats.backstop_found_work.load(SeqCst) > 0, "B' counts normal fresh work in this window");
    assert!(ws.timeouts_with_work.load(SeqCst) > 0);
}
#[test]
fn fresh_token_with_ready_wake_is_counted_as_backstop_work() { observe(false); }
#[test]
fn fresh_relay_demand_with_ready_wake_is_counted_as_backstop_work() { observe(true); }
