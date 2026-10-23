//! EXPECTED RED (race/correctness audit 2026-10-21, finding PR-2): `DbFast::service_ready` -- which the register
//! drainer calls before EVERY privileged register write (`device.rs` `apply_register`) -- loops for as long as
//! the poll keeps returning a full batch (`MAX_READY_BATCH` = 64). Doorbells arrive through exit-less KVM
//! ioeventfd stores, so a guest that keeps >= 64 distinct tokens' eventfds signalled faster than the drainer
//! delivers them keeps the batch full for ever: the drainer never returns to its privileged ring, the GSP
//! heartbeat, the reset stages or the VA clears (the drainer "may never block").
//!
//! The test registers 128 tokens on fake (no-KVM) placements, lets a "guest" thread re-signal every eventfd in
//! a tight loop, and asks that one `service_ready` call return within 3 s.
use kf_chan::dbfast::{DbFast, Delivered, Ioeventfd, Sink};
use kf_linux_raw::Notifier;
use std::io::Write;
use std::os::fd::BorrowedFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A KVM stand-in: accepts every assignment and keeps a duplicate of each eventfd for the "guest" to write.
struct Fake(Arc<Mutex<Vec<std::fs::File>>>);
impl Ioeventfd for Fake {
    fn set(&self, _gpa: u64, _value: u32, fd: BorrowedFd<'_>, assign: bool) -> Result<(), i32> {
        if assign {
            let dup = fd.try_clone_to_owned().map_err(|_| 24)?;
            self.0.lock().unwrap().push(std::fs::File::from(dup));
        }
        Ok(())
    }
}

/// Delivery costs a little, as ringing a token does.
struct Slow;
impl Sink for Slow {
    fn deliver(&self, _idx: u32, _value: u32, _tag: u64) -> Delivered {
        for _ in 0..200 {
            std::hint::spin_loop();
        }
        Delivered::Handed
    }
}

#[test]
fn service_ready_returns_to_the_drainer_even_while_the_guest_keeps_64_or_more_tokens_signalled() {
    let kick: &'static Notifier = Box::leak(Box::new(Notifier::create().expect("kick")));
    let f = Arc::new(DbFast::new(1 << 20, kick).expect("fast path"));
    let fds = Arc::new(Mutex::new(Vec::new()));
    assert!(f.enable(Box::new(Fake(Arc::clone(&fds)))));
    f.site_add(0xfd00_0000);
    for i in 1..=128u32 {
        let _ = f.register(i, 0x1000 + i);
    }
    assert!(fds.lock().unwrap().len() >= 128, "128 placements captured");

    let stop = Arc::new(AtomicBool::new(false));
    let guest = {
        let (fds, stop) = (Arc::clone(&fds), Arc::clone(&stop));
        std::thread::spawn(move || {
            let one = 1u64.to_ne_bytes();
            while !stop.load(Ordering::Relaxed) {
                for file in fds.lock().unwrap().iter_mut() {
                    // an exit-less ioeventfd store is an eventfd write; EAGAIN (counter saturated) is fine
                    let _ = file.write(&one);
                }
            }
        })
    };

    let (tx, rx) = std::sync::mpsc::channel();
    let drainer = {
        let f = Arc::clone(&f);
        std::thread::spawn(move || {
            let n = f.service_ready(&Slow);
            let _ = tx.send(n);
        })
    };
    let returned = rx.recv_timeout(Duration::from_secs(3));
    stop.store(true, Ordering::Relaxed);
    guest.join().unwrap();
    drainer.join().unwrap(); // returns once the guest stops signalling
    assert!(
        returned.is_ok(),
        "PR-2: one service_ready call did not return in 3 s while 128 tokens were kept signalled: the drainer is held \
         inside the doorbell loop and its privileged ring, GSP heartbeat and reset stages starve"
    );
}
