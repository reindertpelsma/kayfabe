//! The real-KVM harness the doorbell fast-path tests share (`dbfast_kvm.rs`, `dbfast_exhaust.rs`,
//! `dbfast_bench.rs`): a flat 32-bit guest, a read-only doorbell page, the device's own trap arm as
//! the delivery, a drainer thread. GPU-free.
#![allow(dead_code)]

use kf_chan::dbfast::{DbFast, Delivered, Ioeventfd, Sink};
use kf_core::{HostOps, HostSlice, Plane, Step, Translatable, VmCaps, Vmm};
use kf_linux_raw::{
    GuestWindow, HostOffset, HostPageSize, Kvm, KvmMemslot, KvmVcpu, KvmVm, Notifier, PollTimeout,
    RawError, ReadyTokens, VcpuExit,
};
use kf_trap::{Action, Class, Route};
use std::collections::{BTreeSet, HashMap};
use std::os::fd::BorrowedFd;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// BAR0 offset of the doorbell (the usermode page `0xBB0000` + `NOTIFY_CHANNEL_PENDING` 0x90) —
/// what the device hands `trap_write`; the harness's guest-physical doorbell is elsewhere.
pub const DB_OFF: u32 = 0x00BB_0090;
/// The drainer's own wake tag in the fast path's poller.
pub const KICK_TAG: u64 = 1;

/// The real kernel as the fast path's KVM verb.
pub struct KvmVerb(pub Arc<KvmVm>);
impl Ioeventfd for KvmVerb {
    fn set(&self, gpa: u64, value: u32, fd: BorrowedFd<'_>, assign: bool) -> Result<(), i32> {
        self.0
            .ioeventfd(gpa, value, fd, assign)
            .map_err(|e| match e {
                RawError::Syscall { errno: Some(e), .. } => e,
                _ => -1,
            })
    }
}

/// A guest: code in RAM at one page, the doorbell page read-only at the next.
pub struct Guest {
    pub kvm: Kvm,
    pub vm: Arc<KvmVm>,
    pub code_gpa: u64,
    pub doorbell: u64,
    _slots: Vec<(Arc<GuestWindow>, KvmMemslot)>,
}

impl Guest {
    /// Code at page 1 (up to 4 pages), the doorbell page read-only at page 8; `image` is given
    /// the doorbell's guest-physical address.
    pub fn new(image: impl FnOnce(u64) -> Vec<u8>) -> Guest {
        let page = HostPageSize::query();
        let kvm = Kvm::open().expect("/dev/kvm");
        let vm = Arc::new(kvm.create_vm().expect("vm"));
        vm.set_tss_addr_if_supported().expect("tss");
        let code_gpa = page.bytes();
        let db_gpa = 8 * page.bytes();
        let doorbell = db_gpa + 0x90;
        let code = image(doorbell);
        let code_len = 4 * page.bytes();
        assert!(code.len() as u64 <= code_len, "guest image too long");
        let cw = Arc::new(GuestWindow::create(code_len, page).expect("code"));
        cw.write_from(HostOffset::ZERO, &code).expect("image");
        let cs = KvmMemslot::install(
            Arc::clone(&vm),
            0,
            code_gpa,
            Arc::clone(&cw),
            0,
            code_len,
            false,
        )
        .expect("code slot");
        let dw = Arc::new(GuestWindow::create(page.bytes(), page).expect("db"));
        let ds = KvmMemslot::install(
            Arc::clone(&vm),
            1,
            db_gpa,
            Arc::clone(&dw),
            0,
            page.bytes(),
            true,
        )
        .expect("read-only doorbell slot");
        Guest {
            kvm,
            vm,
            code_gpa,
            doorbell,
            _slots: vec![(cw, cs), (dw, ds)],
        }
    }

    pub fn vcpu(&self) -> KvmVcpu {
        let v = KvmVcpu::create(&self.kvm, Arc::clone(&self.vm), 0).expect("vcpu");
        v.enter_flat_protected_mode(self.code_gpa, 0).expect("pm");
        v
    }

    /// Run until HLT; every doorbell exit goes to `trap` with its value.
    pub fn run(&self, vcpu: &mut KvmVcpu, mut trap: impl FnMut(u32)) {
        loop {
            match vcpu.run().expect("run") {
                VcpuExit::Halted => return,
                VcpuExit::Interrupted => {}
                VcpuExit::Mmio {
                    gpa,
                    len: 4,
                    is_write: true,
                    data,
                } if gpa == self.doorbell => {
                    trap(u32::from_le_bytes([data[0], data[1], data[2], data[3]]));
                }
                other => panic!("unexpected exit {other:?}"),
            }
        }
    }
}

/// `mov dword [db], v` per token, then HLT.
pub fn stores(db: u64, tokens: &[u32]) -> Vec<u8> {
    let d = u32::try_from(db).expect("below 4 GiB").to_le_bytes();
    let mut v = Vec::new();
    for t in tokens {
        let t = t.to_le_bytes();
        v.extend_from_slice(&[0xC7, 0x05, d[0], d[1], d[2], d[3], t[0], t[1], t[2], t[3]]);
    }
    v.push(0xF4);
    v
}

/// `mov ecx, n; loop: (mov dword [db], v)*; dec ecx; jnz loop; hlt`.
pub fn loop_stores(db: u64, tokens: &[u32], n: u32) -> Vec<u8> {
    let d = u32::try_from(db).expect("below 4 GiB").to_le_bytes();
    let mut v = vec![0xB9];
    v.extend_from_slice(&n.to_le_bytes());
    let top = v.len();
    for t in tokens {
        let t = t.to_le_bytes();
        v.extend_from_slice(&[0xC7, 0x05, d[0], d[1], d[2], d[3], t[0], t[1], t[2], t[3]]);
    }
    v.push(0x49); // dec ecx
    let back = v.len() + 2 - top;
    assert!(back <= 128, "loop body too long for a rel8");
    v.push(0x75); // jnz rel8
    #[allow(clippy::cast_possible_truncation)]
    v.push((256 - back) as u8);
    v.push(0xF4);
    v
}

/// Host-side stand-in for the twins and rings: which host tokens are live, and every ring/serve.
#[derive(Default)]
pub struct Host {
    pub freed: Mutex<BTreeSet<u32>>,
    pub serves: Mutex<Vec<u32>>,
}
impl HostOps for Host {
    fn ring_host(&self, _: u32) {}
    fn run_translated(&self, host_token: u32, _: u64) -> bool {
        self.serves.lock().unwrap().push(host_token);
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

/// ★ The device's delivery, as `kf-qemu` implements it: the trap's own doorbell arm, then the act
/// it asks for. Records every ring by path and registration.
pub struct DevSink {
    pub plane: &'static Plane<'static>,
    pub host: Arc<Host>,
    pub worker: Notifier,
    /// (registration tag, host token) of every fast-path ring.
    pub fast_rings: Mutex<Vec<(u64, u32)>>,
    /// Fast-path rings of a host token already freed — must stay 0.
    pub fast_late: AtomicU64,
    /// Trap-path rings of a host token already freed (the trap's own load→ring window; reported).
    pub trap_late: AtomicU64,
    /// Trap exits handled, per value.
    pub trapped: Mutex<HashMap<u32, u64>>,
}

impl DevSink {
    pub fn new(plane: &'static Plane<'static>, host: Arc<Host>) -> DevSink {
        DevSink {
            plane,
            host,
            worker: Notifier::create().expect("worker efd"),
            fast_rings: Mutex::new(Vec::new()),
            fast_late: AtomicU64::new(0),
            trap_late: AtomicU64::new(0),
            trapped: Mutex::new(HashMap::new()),
        }
    }

    /// What the device does with a doorbell of `value`, on whichever thread holds it.
    pub fn act(&self, value: u32) -> (Action, Delivered) {
        let route = self
            .plane
            .token_index
            .of_doorbell(value)
            .and_then(|i| self.plane.tokens.get(i as usize))
            .map(|w| w.load().route);
        let a = self
            .plane
            .trap_write(Class::Doorbell, 0, DB_OFF, u64::from(value), 4);
        let d = match a {
            Action::RingHostInline { .. } => Delivered::Rang { reached: true },
            Action::WakeWorker => {
                let _ = self.worker.signal();
                Delivered::Handed
            }
            _ if matches!(route, Some(Route::Translated | Route::Emulated)) => Delivered::Handed,
            _ => Delivered::Absorbed,
        };
        (a, d)
    }

    /// The trap path (a vCPU exit).
    pub fn trap(&self, value: u32) {
        *self.trapped.lock().unwrap().entry(value).or_default() += 1;
        if let (Action::RingHostInline { host_token }, _) = self.act(value)
            && self.host.freed.lock().unwrap().contains(&host_token)
        {
            self.trap_late.fetch_add(1, Ordering::Relaxed);
        }
    }
}

impl Sink for DevSink {
    fn deliver(&self, _idx: u32, value: u32, tag: u64) -> Delivered {
        let (a, d) = self.act(value);
        if let Action::RingHostInline { host_token } = a {
            if self.host.freed.lock().unwrap().contains(&host_token) {
                self.fast_late.fetch_add(1, Ordering::Relaxed);
            }
            self.fast_rings.lock().unwrap().push((tag, host_token));
        }
        d
    }
}

pub fn plane() -> &'static Plane<'static> {
    let vmm: &'static Vmm = Box::leak(Box::new(Vmm::new()));
    Box::leak(Box::new(Plane::for_device(
        vmm,
        1 << 12,
        0xFFF,
        kf_chip::Family::Ampere,
        16 << 20,
    )))
}

/// The drainer: waits on the fast path's poller (its kick at [`KICK_TAG`]) and delivers.
pub struct Drainer {
    stop: Arc<AtomicBool>,
    h: Option<std::thread::JoinHandle<()>>,
}
impl Drainer {
    pub fn start(f: &Arc<DbFast>, kick: &'static Notifier, sink: &Arc<DevSink>) -> Drainer {
        f.poller()
            .watch(kick.as_source_fd(), KICK_TAG)
            .expect("watch kick");
        let stop = Arc::new(AtomicBool::new(false));
        let (f, sink, st) = (Arc::clone(f), Arc::clone(sink), Arc::clone(&stop));
        let h = std::thread::spawn(move || {
            while !st.load(Ordering::Acquire) {
                let mut r = ReadyTokens::new();
                let _ = f.poller().wait(&mut r, PollTimeout::Millis(5));
                let t = Instant::now();
                if r.iter().any(|t| t == KICK_TAG) {
                    let _ = kick.drain();
                }
                f.on_ready(&r, t, &*sink);
            }
        });
        Drainer { stop, h: Some(h) }
    }
}
impl Drop for Drainer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(h) = self.h.take() {
            let _ = h.join();
        }
    }
}

pub fn kick() -> &'static Notifier {
    Box::leak(Box::new(Notifier::create().expect("kick")))
}

pub fn caps() -> Mutex<VmCaps> {
    Mutex::new(VmCaps::from_declared(4096, 64, 64, 64))
}
