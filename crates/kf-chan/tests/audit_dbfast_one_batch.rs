//! PR-2. FALSIFIER: a source re-armed by delivery monopolizes service_ready,
//! or a notification left ready does not survive to a subsequent drainer turn.
use kf_chan::dbfast::{DbFast, Delivered, Ioeventfd, Sink};
use kf_linux_raw::{MAX_READY_BATCH, Notifier};
use std::io::Write;
use std::os::fd::BorrowedFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
struct Fake(Arc<Mutex<Vec<std::fs::File>>>);
impl Ioeventfd for Fake {
    fn set(&self, _: u64, _: u32, fd: BorrowedFd<'_>, assign: bool) -> Result<(), i32> {
        if assign {
            self.0.lock().unwrap().push(std::fs::File::from(
                fd.try_clone_to_owned().map_err(|_| 24)?,
            ));
        }
        Ok(())
    }
}
struct Refill {
    files: Arc<Mutex<Vec<std::fs::File>>>,
    stop: Arc<AtomicBool>,
}
impl Refill {
    fn publish(&self) {
        if !self.stop.load(Ordering::Acquire) {
            for file in self.files.lock().unwrap().iter_mut() {
                file.write_all(&1u64.to_ne_bytes()).unwrap();
            }
        }
    }
}
impl Sink for Refill {
    fn deliver(&self, _: u32, _: u32, _: u64) -> Delivered {
        self.publish();
        Delivered::Handed
    }
}
fn check(n: usize) {
    let kick: &'static Notifier = Box::leak(Box::new(Notifier::create().unwrap()));
    let f = Arc::new(DbFast::new(1 << 20, kick).unwrap());
    let files = Arc::new(Mutex::new(Vec::new()));
    assert!(f.enable(Box::new(Fake(files.clone()))));
    f.site_add(0xfd00_0000);
    for i in 1..=n as u32 {
        let _ = f.register(i, 0x1000 + i);
    }
    let sink = Arc::new(Refill {
        files,
        stop: Arc::new(AtomicBool::new(false)),
    });
    assert_eq!(sink.files.lock().unwrap().len(), n);
    sink.publish(); // ready before service begins; no producer-thread startup race
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = {
        let f = f.clone();
        let sink = sink.clone();
        std::thread::spawn(move || {
            tx.send(f.service_ready(&*sink)).unwrap();
        })
    };
    let first = rx.recv_timeout(Duration::from_secs(1)); // test watchdog only
    sink.stop.store(true, Ordering::Release);
    worker.join().unwrap();
    let deliveries = first
        .as_ref()
        .copied()
        .unwrap_or_else(|_| rx.recv().unwrap());
    assert!(
        first.is_ok(),
        "one readiness snapshot must return despite deterministic refill"
    );
    assert!(
        deliveries <= MAX_READY_BATCH,
        "one turn delivered {deliveries}, above its readiness array capacity"
    );
    assert!(
        f.service_ready(&*sink) > 0,
        "refilled fds must remain ready for the next turn"
    );
}
#[test]
fn sixty_four_hot_tokens_cannot_hold_one_drainer_call() {
    check(MAX_READY_BATCH);
}
#[test]
fn two_ready_batches_cannot_hold_one_drainer_call() {
    check(MAX_READY_BATCH * 2);
}
