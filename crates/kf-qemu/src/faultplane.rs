//! ★★★ **The guest's replayable-fault plane** — `docs/design/V3_UVM_GUEST_FAULT_PLANE.md`
//! §3.1–§3.7 and §4. One per device, and only when `KF3_UVM_EFS=1` asked for it AND the host's b3
//! nvidia-uvm answered the probe (§6); otherwise this module is never built and the device runs the
//! code paths it ran before.
//!
//! ## What it owns
//!
//! - **The guest's buffer** — where the guest registered it (`0x20800a9b`), validated to guest RAM
//!   before anything is written ([`FaultPlane::register`], on the drainer, inside the served RPC:
//!   the guest's UVM reads `GET`/`PUT` once right after, so both restart at 0 before it can).
//! - **The two registers** — [`FaultRing`]: `GET` written by the guest on a vCPU (lock-free, in
//!   `Device::bar0_write_inner`), `PUT` advanced here after an entry is fully written.
//! - **One EFS file per EFS-mode twin space** ([`FaultPlane::add_space`]) and its `kf3-efs-wait`
//!   thread: `UVM_EFS_WAIT` (no lock held) → a packet per record, naming the guest channel that
//!   represents the space (§3.5) → the ring → `PUT` → the interrupt level (§3.3).
//! - **The guest's replay and cancel** ([`FaultPlane::guest_op`], from a worker at a Translated
//!   split) → the `kf3-efs-resolve` thread → `UVM_EFS_RESOLVE` of THIS VM's own delivered records.
//!
//! ## The rules it keeps
//!
//! - ⊘ Every completion the guest sees is a host event: a packet is written only for a record the
//!   host's nvidia-uvm parked; a replay reaches the host only with records we delivered (a guest
//!   spinning on replays reaches the host GPU zero times).
//! - ⊘ No lock is held across an ioctl or a sleep; the vCPU side is atomics only
//!   (`THE_CONSTRAINTS.md` 4). The plane lock guards bookkeeping and guest-RAM writes, never a
//!   syscall; the job queue's lock guards a `VecDeque` push.
//! - ⊘ A packet carries only guest values (its instance block, its VEID, the page) and the host
//!   GPU timer the guest already reads — never a host instance pointer or PDB (the EFS ABI has none).
//! - A record nobody can service (no buffer registered, no guest channel to attribute it to, a
//!   type or GPC the guest could not parse) is cancelled at once, by name — the host would cancel
//!   it at its timeout anyway, and stock UVM would have cancelled it as fatal.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use kf_abi::faultbuffer::FaultBufferRegistration;
use kf_abi::uvmefs::{EfsRecord, UVM_EFS_ACTION_CANCEL, UVM_EFS_ACTION_REPLAY};
use kf_chan::translated::FaultOp;
use kf_chip::fault::{FaultConsts, GuestFaultIdentity, InstLocation, PacketRefusal, packet_for};
use kf_mem::vasmgr::VasKey;
use kf_trap::faultring::{Eval, FaultRing};

/// Bytes per packet (`sizeof(NvC369_BUF_ENTRY)`).
const ENTRY: u64 = 32;
/// The page size the registration's page list is expressed in.
const PAGE: u64 = 4096;
/// `UVM_EFS_WAIT`'s bound per call — short enough that a retired space's waiter exits promptly
/// and a parked-but-undeliverable record is retried often.
const WAIT_US: u32 = 100_000;
/// Records asked for per wait (`UVM_EFS_MAX_WAIT_RECORDS`).
const WAIT_MAX: u32 = kf_abi::uvmefs::UVM_EFS_MAX_WAIT_RECORDS;
/// Ids per resolve (`UVM_EFS_MAX_RESOLVE_RECORDS`).
const RESOLVE_MAX: usize = kf_abi::uvmefs::UVM_EFS_MAX_RESOLVE_RECORDS as usize;

/// ★ One EFS file as the plane uses it — [`kf_host::efs::EfsSession`] on a host, a double in tests.
pub trait EfsFile: Send + Sync {
    /// `UVM_EFS_WAIT`: up to `max` records, blocking at most `timeout_us`.
    ///
    /// # Errors
    /// The host's refusal, by name.
    fn wait(&self, max: u32, timeout_us: u32) -> Result<Vec<EfsRecord>, String>;
    /// `UVM_EFS_RESOLVE(action)` of `ids` → `(resolved, stale)`.
    ///
    /// # Errors
    /// The host's refusal, by name.
    fn resolve(&self, ids: &[u64], action: u32) -> Result<(u32, u32), String>;
}

impl EfsFile for kf_host::efs::EfsSession {
    fn wait(&self, max: u32, timeout_us: u32) -> Result<Vec<EfsRecord>, String> {
        kf_host::efs::EfsSession::wait(self, max, timeout_us).map_err(|e| format!("{e:?}"))
    }
    fn resolve(&self, ids: &[u64], action: u32) -> Result<(u32, u32), String> {
        kf_host::efs::EfsSession::resolve(self, ids, action).map_err(|e| format!("{e:?}"))
    }
}

/// ★ What the plane does to the guest — the device (and a test double). Every method is callable
/// from any thread and blocks on nothing.
pub trait FaultSink: Sync {
    /// Publish a register word the guest reads back (the BAR0 shadow).
    fn shadow(&self, off: u64, val: u32);
    /// Latch the replayable vector in the guest's interrupt tree and signal it.
    fn raise(&self, vector: u32);
    /// Write one packet at guest-physical `gpa`: dwords 0–6, a release fence, then dword 7 (which
    /// carries `VALID`). `false` when guest RAM does not cover `[gpa, gpa+32)`.
    fn write_entry(&self, gpa: u64, words: &[u32; 8]) -> bool;
    /// `[gpa, gpa+len)` is guest RAM QEMU registered (never a kf memslot).
    fn is_guest_ram(&self, gpa: u64, len: u64) -> bool;
}

/// A record the guest was shown, kept until its replay or cancel resolves it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Delivered {
    id: u64,
    page: u64,
    gpc: u32,
    client: u32,
}

/// One EFS-mode twin space.
struct Space {
    file: Arc<dyn EfsFile>,
    /// The guest's page directory for it (a `CancelVa` names it).
    root: Option<u64>,
    /// The live, host-registered GR twins of this space with their GUEST identity, oldest first:
    /// the last is the representative (§3.5).
    twins: Vec<(u32, GuestFaultIdentity)>,
    delivered: Vec<Delivered>,
    undelivered: VecDeque<EfsRecord>,
    stop: Arc<AtomicBool>,
}

/// The registered buffer, validated.
struct Buffer {
    pages: Vec<u64>,
    entries: u32,
}

impl Buffer {
    /// The guest-physical address of entry `i` (entries never straddle a page: 4096 % 32 == 0).
    fn entry_gpa(&self, i: u32) -> Option<u64> {
        if i >= self.entries {
            return None;
        }
        let off = u64::from(i) * ENTRY;
        let page = *self.pages.get(usize::try_from(off / PAGE).ok()?)?;
        Some(page + off % PAGE)
    }
}

#[derive(Default)]
struct Inner {
    buffer: Option<Buffer>,
    spaces: HashMap<VasKey, Space>,
}

/// Work for the resolver thread.
enum Job {
    /// A guest replay or cancel, after everything before it in its stream completed.
    Op(FaultOp),
    /// Records to cancel that nobody will ever service (a replaced buffer's).
    Cancel(Arc<dyn EfsFile>, Vec<u64>),
}

/// Counters — the boot report's, never a decision input.
#[derive(Debug, Default)]
pub struct FaultCounters {
    /// Buffer registrations accepted.
    pub registrations: AtomicU64,
    /// Registrations refused (named at the time).
    pub registration_refused: AtomicU64,
    /// EFS spaces opened / closed.
    pub spaces: AtomicU64,
    /// Of which retired.
    pub spaces_retired: AtomicU64,
    /// Records the host handed us.
    pub records: AtomicU64,
    /// Packets written into the guest's buffer.
    pub delivered: AtomicU64,
    /// Interrupt raises from a `PUT` publication.
    pub put_raises: AtomicU64,
    /// Guest `GET` writes, and the raises they caused.
    pub get_writes: AtomicU64,
    /// See [`FaultCounters::get_writes`].
    pub get_raises: AtomicU64,
    /// Records cancelled because no buffer was registered.
    pub cancelled_no_buffer: AtomicU64,
    /// Records cancelled because no guest GR channel represents their space.
    pub cancelled_no_channel: AtomicU64,
    /// Records cancelled because the packet was refused (type, client, GPC, width).
    pub cancelled_refused: AtomicU64,
    /// Records cancelled because the guest-RAM write was refused.
    pub cancelled_write: AtomicU64,
    /// Guest replay ops, and the host `RESOLVE(REPLAY)` calls / ids they became.
    pub guest_replays: AtomicU64,
    /// See [`FaultCounters::guest_replays`].
    pub host_replays: AtomicU64,
    /// See [`FaultCounters::guest_replays`].
    pub replayed: AtomicU64,
    /// Guest cancel ops, and the ids they cancelled.
    pub guest_cancels: AtomicU64,
    /// See [`FaultCounters::guest_cancels`].
    pub cancelled: AtomicU64,
    /// Guest ops that named nothing of ours (no host call).
    pub ops_empty: AtomicU64,
    /// Ids the host answered stale (already resolved or timed out).
    pub stale: AtomicU64,
    /// Host wait/resolve refusals.
    pub host_errors: AtomicU64,
}

impl FaultCounters {
    /// One line for the heartbeat / end-of-run report.
    #[must_use]
    pub fn line(&self) -> String {
        let g = |a: &AtomicU64| a.load(Ordering::Relaxed);
        format!(
            "regs={}/{}refused spaces={}({} retired) records={} delivered={} raises(put={} get={}) get_writes={} \
             cancelled(no_buffer={} no_channel={} refused={} write={}) replays(guest={} host={} ids={}) \
             cancels(guest={} ids={}) empty_ops={} stale={} host_errors={}",
            g(&self.registrations),
            g(&self.registration_refused),
            g(&self.spaces),
            g(&self.spaces_retired),
            g(&self.records),
            g(&self.delivered),
            g(&self.put_raises),
            g(&self.get_raises),
            g(&self.get_writes),
            g(&self.cancelled_no_buffer),
            g(&self.cancelled_no_channel),
            g(&self.cancelled_refused),
            g(&self.cancelled_write),
            g(&self.guest_replays),
            g(&self.host_replays),
            g(&self.replayed),
            g(&self.guest_cancels),
            g(&self.cancelled),
            g(&self.ops_empty),
            g(&self.stale),
            g(&self.host_errors),
        )
    }
}

/// ★ The plane.
pub struct FaultPlane {
    /// `GET`/`PUT` of the replayable buffer — the vCPU arm reads and writes it lock-free.
    pub ring: FaultRing,
    /// The die group's facts (`kf_chip::fault`).
    pub consts: FaultConsts,
    /// The replayable vector — the host die's own (`MC_ENGINE_IDX_REPLAYABLE_FAULT`'s stall row).
    pub vector: u32,
    /// GPCs the device advertised (the host die's).
    gpcs: u32,
    sink: OnceLock<&'static dyn FaultSink>,
    inner: Mutex<Inner>,
    jobs: Mutex<VecDeque<Job>>,
    jobs_cv: Condvar,
    stop: AtomicBool,
    /// Counters.
    pub counters: FaultCounters,
}

/// Why a registration is refused — the plane then stays unregistered and every record cancels.
fn validate(reg: &FaultBufferRegistration, sink: Option<&dyn FaultSink>) -> Result<Buffer, String> {
    if reg.size == 0 || u64::from(reg.size) % ENTRY != 0 {
        return Err(format!(
            "size {:#x} is not a positive multiple of 32",
            reg.size
        ));
    }
    let need = u64::from(reg.size).div_ceil(PAGE);
    if reg.pages.len() as u64 != need {
        return Err(format!(
            "{} page(s) listed for a {:#x}-byte buffer ({need} needed)",
            reg.pages.len(),
            reg.size
        ));
    }
    let Some(sink) = sink else {
        return Err("the device is not bound yet (guest RAM cannot be checked)".into());
    };
    for (i, &p) in reg.pages.iter().enumerate() {
        if p % PAGE != 0 || !sink.is_guest_ram(p, PAGE) {
            return Err(format!(
                "page {i} at {p:#x} is not a 4 KiB page of guest RAM"
            ));
        }
    }
    Ok(Buffer {
        pages: reg.pages.clone(),
        entries: reg.size / ENTRY as u32,
    })
}

/// The instance block's aperture class the guest's UVM compares (`UVM_APERTURE_VID` vs `_SYS`).
const fn is_vid(aperture: u32) -> bool {
    aperture == kf_abi::faultpacket::INST_APERTURE_VID_MEM
}

/// ★ Which delivered records a guest op resolves, and how — the pure half of the resolver
/// (GPU-free tested). Removes them from `delivered`; returns `(file, ids, action)` per space.
fn plan_op(inner: &mut Inner, op: FaultOp) -> Vec<(Arc<dyn EfsFile>, Vec<u64>, u32)> {
    let mut plan = Vec::new();
    let take = |sp: &mut Space, pick: &dyn Fn(&Delivered) -> bool| -> Vec<u64> {
        let mut ids = Vec::new();
        sp.delivered.retain(|d| {
            if pick(d) {
                ids.push(d.id);
                false
            } else {
                true
            }
        });
        ids
    };
    for sp in inner.spaces.values_mut() {
        let (ids, action) = match op {
            FaultOp::Replay { .. } => (take(sp, &|_| true), UVM_EFS_ACTION_REPLAY),
            FaultOp::CancelVa { pdb, va, .. } => {
                if pdb.is_some() && sp.root != pdb {
                    continue;
                }
                let page = va & !(PAGE - 1);
                (take(sp, &|d| d.page == page), UVM_EFS_ACTION_CANCEL)
            }
            FaultOp::CancelGlobal => (take(sp, &|_| true), UVM_EFS_ACTION_CANCEL),
            FaultOp::CancelTargeted { gpc, client } => (
                take(sp, &|d| d.gpc == gpc && d.client == client),
                UVM_EFS_ACTION_CANCEL,
            ),
            FaultOp::CancelInstance {
                inst,
                aperture,
                global,
                gpc,
                client,
            } => {
                let ours = sp.twins.iter().any(|(_, w)| {
                    w.inst_addr & !(PAGE - 1) == inst & !(PAGE - 1)
                        && is_vid(w.inst.aperture()) == is_vid(aperture)
                });
                if !ours {
                    continue;
                }
                (
                    take(sp, &|d| global || (d.gpc == gpc && d.client == client)),
                    UVM_EFS_ACTION_CANCEL,
                )
            }
        };
        if !ids.is_empty() {
            plan.push((sp.file.clone(), ids, action));
        }
    }
    plan
}

impl FaultPlane {
    /// The plane for a die group's facts, the replayable `vector` and `gpcs` GPCs. `GET`/`PUT` sit
    /// at `priv_base` + the family's PRIV-relative offsets.
    #[must_use]
    pub fn new(consts: FaultConsts, priv_base: u64, vector: u32, gpcs: u32) -> FaultPlane {
        FaultPlane {
            ring: FaultRing::new(
                priv_base + consts.get_off,
                priv_base + consts.put_off,
                consts.ptr_mask,
            ),
            consts,
            vector,
            gpcs,
            sink: OnceLock::new(),
            inner: Mutex::new(Inner::default()),
            jobs: Mutex::new(VecDeque::new()),
            jobs_cv: Condvar::new(),
            stop: AtomicBool::new(false),
            counters: FaultCounters::default(),
        }
    }

    /// Bind the device (once, right after it is leaked — before any guest code runs).
    pub fn bind(&self, sink: &'static dyn FaultSink) {
        let _ = self.sink.set(sink);
    }

    /// Start the resolver thread.
    ///
    /// # Errors
    /// The spawn.
    pub fn start(&'static self) -> Result<(), String> {
        std::thread::Builder::new()
            .name("kf3-efs-resolve".into())
            .spawn(move || self.resolver_loop())
            .map(|_| ())
            .map_err(|e| format!("kf3-efs-resolve: {e}"))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn queue(&self, job: Job) {
        self.jobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push_back(job);
        self.jobs_cv.notify_one();
    }

    /// ★ **Drainer, inside the served `0x20800a9b`**: the guest (re)registered its replayable buffer.
    /// Validated to guest RAM first; `GET = PUT = 0` published before the reply. Whatever the
    /// previous buffer held is cancelled — no one will service it (§3.1).
    pub fn register(&self, reg: &FaultBufferRegistration) {
        let sink = self.sink.get().copied();
        let checked = validate(reg, sink);
        let mut orphans = Vec::new();
        {
            let mut g = self.lock();
            for sp in g.spaces.values_mut() {
                let ids: Vec<u64> = sp
                    .delivered
                    .drain(..)
                    .map(|d| d.id)
                    .chain(sp.undelivered.drain(..).map(|r| r.id))
                    .collect();
                if !ids.is_empty() {
                    orphans.push((sp.file.clone(), ids));
                }
            }
            match checked {
                Ok(b) => {
                    self.ring.register(b.entries);
                    eprintln!(
                        "kf3: fault plane: replayable buffer registered — {} entries over {} guest page(s) (client {:#x} object {:#x}); GET=PUT=0",
                        b.entries,
                        b.pages.len(),
                        reg.h_client,
                        reg.h_object
                    );
                    g.buffer = Some(b);
                    self.counters.registrations.fetch_add(1, Ordering::Relaxed);
                }
                Err(why) => {
                    self.ring.unregister();
                    g.buffer = None;
                    self.counters
                        .registration_refused
                        .fetch_add(1, Ordering::Relaxed);
                    eprintln!(
                        "kf3: fault plane: replayable buffer registration REFUSED: {why} — every fault of this guest is cancelled until a valid one"
                    );
                }
            }
            if let Some(s) = sink {
                s.shadow(self.ring.get_off(), 0);
                s.shadow(self.ring.put_off(), 0);
            }
        }
        for (f, ids) in orphans {
            self.queue(Job::Cancel(f, ids));
        }
    }

    /// ★ **vCPU**: the guest wrote `GET`. Returns the word its read-back shows and whether to raise.
    /// Atomics only.
    pub fn get_write(&self, val: u32) -> (u32, bool) {
        self.counters.get_writes.fetch_add(1, Ordering::Relaxed);
        let (shown, ev) = self.ring.write_get(val);
        let raise = ev == Eval::Pending;
        if raise {
            self.counters.get_raises.fetch_add(1, Ordering::Relaxed);
        }
        (shown, raise)
    }

    /// ★ **VA thread**: an EFS-mode space is open — start its waiter.
    ///
    /// # Errors
    /// The spawn (the space is then not added; the caller tears it down).
    pub fn add_space(&'static self, key: VasKey, file: Arc<dyn EfsFile>) -> Result<(), String> {
        let stop = Arc::new(AtomicBool::new(false));
        self.lock().spaces.insert(
            key,
            Space {
                file: file.clone(),
                root: None,
                twins: Vec::new(),
                delivered: Vec::new(),
                undelivered: VecDeque::new(),
                stop: stop.clone(),
            },
        );
        let spawned = std::thread::Builder::new()
            .name("kf3-efs-wait".into())
            .spawn(move || self.wait_loop(key, file, stop));
        if let Err(e) = spawned {
            self.lock().spaces.remove(&key);
            return Err(format!("kf3-efs-wait for {key:?}: {e}"));
        }
        self.counters.spaces.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// ★ **VA thread**: the space is retired — its waiter stops (within one wait), its records go
    /// with its file (the host cancels whatever is parked when the file closes).
    pub fn remove_space(&self, key: VasKey) {
        if let Some(sp) = self.lock().spaces.remove(&key) {
            sp.stop.store(true, Ordering::Release);
            self.counters.spaces_retired.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Whether `key` is an EFS-mode space.
    #[must_use]
    pub fn has_space(&self, key: VasKey) -> bool {
        self.lock().spaces.contains_key(&key)
    }

    /// ★ **VA thread**: the guest's page directory for `key` (a `CancelVa` names it).
    pub fn set_root(&self, key: VasKey, root: Option<u64>) {
        if let Some(sp) = self.lock().spaces.get_mut(&key) {
            sp.root = root;
        }
    }

    /// ★ **Act thread**: a GR twin of `key` is registered with the host's UVM and runs as the
    /// guest channel `who` — it becomes the space's representative (§3.5).
    pub fn note_twin(&self, key: VasKey, host_chan: u32, who: GuestFaultIdentity) {
        let retry = {
            let mut g = self.lock();
            let Some(sp) = g.spaces.get_mut(&key) else {
                return;
            };
            sp.twins.retain(|(c, _)| *c != host_chan);
            sp.twins.push((host_chan, who));
            !sp.undelivered.is_empty()
        };
        if retry {
            self.deliver(key, Vec::new());
        }
    }

    /// ★ **Act thread**: the twin is gone (unregistered before its free).
    pub fn forget_twin(&self, key: VasKey, host_chan: u32) {
        if let Some(sp) = self.lock().spaces.get_mut(&key) {
            sp.twins.retain(|(c, _)| *c != host_chan);
        }
    }

    /// ★ **Worker, at a Translated split**: the guest's replay or cancel, after everything before it
    /// in its stream completed. Queued for the resolver — never a host call here (the worker holds
    /// its channel's slot lock).
    pub fn guest_op(&self, op: FaultOp) {
        match op {
            FaultOp::Replay { .. } => &self.counters.guest_replays,
            _ => &self.counters.guest_cancels,
        }
        .fetch_add(1, Ordering::Relaxed);
        self.queue(Job::Op(op));
    }

    fn wait_loop(&self, key: VasKey, file: Arc<dyn EfsFile>, stop: Arc<AtomicBool>) {
        let mut errors = 0u64;
        while !stop.load(Ordering::Acquire) && !self.stop.load(Ordering::Acquire) {
            match file.wait(WAIT_MAX, WAIT_US) {
                Ok(recs) => {
                    self.counters
                        .records
                        .fetch_add(recs.len() as u64, Ordering::Relaxed);
                    self.deliver(key, recs);
                }
                Err(e) => {
                    errors += 1;
                    self.counters.host_errors.fetch_add(1, Ordering::Relaxed);
                    if errors <= 3 || errors.is_power_of_two() {
                        eprintln!("kf3: fault plane: {key:?} UVM_EFS_WAIT #{errors} refused: {e}");
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
        }
    }

    /// ★ Deliver `recs` (after any still waiting) for `key`: as many as the guest's ring has room
    /// for, each fully written before `VALID`, then ONE `PUT` publication and the level. Records
    /// nobody can service are cancelled after the lock is released.
    fn deliver(&self, key: VasKey, recs: Vec<EfsRecord>) {
        let (file, cancel) = {
            let mut g = self.lock();
            let Inner { buffer, spaces } = &mut *g;
            let Some(sp) = spaces.get_mut(&key) else {
                return;
            };
            sp.undelivered.extend(recs);
            let cancel = self.drain_into_ring(buffer.as_ref(), sp);
            (sp.file.clone(), cancel)
        };
        if !cancel.is_empty() {
            self.resolve_now(file.as_ref(), &cancel, UVM_EFS_ACTION_CANCEL);
        }
    }

    /// Under the plane lock: move `sp.undelivered` into the ring while it has room. Returns the ids
    /// to cancel.
    fn drain_into_ring(&self, buffer: Option<&Buffer>, sp: &mut Space) -> Vec<u64> {
        let mut cancel = Vec::new();
        if sp.undelivered.is_empty() {
            return cancel;
        }
        let Some(buf) = buffer else {
            self.counters
                .cancelled_no_buffer
                .fetch_add(sp.undelivered.len() as u64, Ordering::Relaxed);
            cancel.extend(sp.undelivered.drain(..).map(|r| r.id));
            return cancel;
        };
        let Some(&(_, who)) = sp.twins.last() else {
            self.counters
                .cancelled_no_channel
                .fetch_add(sp.undelivered.len() as u64, Ordering::Relaxed);
            cancel.extend(sp.undelivered.drain(..).map(|r| r.id));
            return cancel;
        };
        let Some(sink) = self.sink.get() else {
            return cancel;
        };
        let mut room = self.ring.room();
        let start = self.ring.put();
        let mut put = start;
        while room > 0 {
            let Some(rec) = sp.undelivered.pop_front() else {
                break;
            };
            let words = packet_for(&rec, &who, &self.consts, self.gpcs)
                .and_then(|e| e.encode().map_err(PacketRefusal::Overflow));
            let words = match words {
                Ok(w) => w,
                Err(r) => {
                    self.counters
                        .cancelled_refused
                        .fetch_add(1, Ordering::Relaxed);
                    eprintln!(
                        "kf3: fault plane: record {:#x} at {:#x} refused ({r:?}) — cancelled",
                        rec.id, rec.address
                    );
                    cancel.push(rec.id);
                    continue;
                }
            };
            let Some(gpa) = buf.entry_gpa(put) else {
                cancel.push(rec.id);
                continue;
            };
            if !sink.write_entry(gpa, &words) {
                self.counters
                    .cancelled_write
                    .fetch_add(1, Ordering::Relaxed);
                cancel.push(rec.id);
                continue;
            }
            sp.delivered.push(Delivered {
                id: rec.id,
                page: rec.address & !(PAGE - 1),
                gpc: rec.gpc_id,
                client: rec.client_id,
            });
            self.counters.delivered.fetch_add(1, Ordering::Relaxed);
            put = self.ring.next(put).unwrap_or(0);
            room -= 1;
        }
        if put != start {
            // The shadow first, so a raise from either side finds the guest's read-back current.
            std::sync::atomic::fence(Ordering::SeqCst);
            sink.shadow(self.ring.put_off(), put);
            if self.ring.publish_put(put) == Eval::Pending {
                self.counters.put_raises.fetch_add(1, Ordering::Relaxed);
                sink.raise(self.vector);
            }
        }
        cancel
    }

    /// `RESOLVE(action)` of `ids`, in chunks. No lock held.
    fn resolve_now(&self, file: &dyn EfsFile, ids: &[u64], action: u32) {
        for chunk in ids.chunks(RESOLVE_MAX) {
            match file.resolve(chunk, action) {
                Ok((_, stale)) => {
                    self.counters
                        .stale
                        .fetch_add(u64::from(stale), Ordering::Relaxed);
                }
                Err(e) => {
                    self.counters.host_errors.fetch_add(1, Ordering::Relaxed);
                    eprintln!(
                        "kf3: fault plane: UVM_EFS_RESOLVE({}) of {} id(s) refused: {e}",
                        if action == UVM_EFS_ACTION_REPLAY {
                            "REPLAY"
                        } else {
                            "CANCEL"
                        },
                        chunk.len()
                    );
                }
            }
        }
    }

    fn resolver_loop(&self) {
        loop {
            let jobs: Vec<Job> = {
                let mut q = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
                while q.is_empty() && !self.stop.load(Ordering::Acquire) {
                    q = self
                        .jobs_cv
                        .wait_timeout(q, std::time::Duration::from_millis(200))
                        .map_or_else(|e| e.into_inner().0, |r| r.0);
                }
                if self.stop.load(Ordering::Acquire) {
                    return;
                }
                q.drain(..).collect()
            };
            let mut replayed = false;
            for job in jobs {
                match job {
                    Job::Op(op) => {
                        let plan = plan_op(&mut self.lock(), op);
                        if plan.is_empty() {
                            self.counters.ops_empty.fetch_add(1, Ordering::Relaxed);
                        }
                        for (file, ids, action) in plan {
                            if action == UVM_EFS_ACTION_REPLAY {
                                self.counters.host_replays.fetch_add(1, Ordering::Relaxed);
                                self.counters
                                    .replayed
                                    .fetch_add(ids.len() as u64, Ordering::Relaxed);
                                replayed = true;
                            } else {
                                self.counters
                                    .cancelled
                                    .fetch_add(ids.len() as u64, Ordering::Relaxed);
                            }
                            self.resolve_now(file.as_ref(), &ids, action);
                        }
                    }
                    Job::Cancel(file, ids) => {
                        self.counters
                            .cancelled
                            .fetch_add(ids.len() as u64, Ordering::Relaxed);
                        self.resolve_now(file.as_ref(), &ids, UVM_EFS_ACTION_CANCEL);
                    }
                }
            }
            // A replay follows the guest consuming entries: records held back for want of room
            // may fit now.
            if replayed {
                let keys: Vec<VasKey> = self
                    .lock()
                    .spaces
                    .iter()
                    .filter(|(_, s)| !s.undelivered.is_empty())
                    .map(|(k, _)| *k)
                    .collect();
                for k in keys {
                    self.deliver(k, Vec::new());
                }
            }
        }
    }

    /// Records delivered and not yet resolved, over every space (for the report).
    #[must_use]
    pub fn outstanding(&self) -> (usize, usize) {
        let g = self.lock();
        g.spaces.values().fold((0, 0), |(d, u), s| {
            (d + s.delivered.len(), u + s.undelivered.len())
        })
    }
}

/// ★ The guest identity a channel's own allocation stated (`instanceMem`, the context share's
/// `subctxId`) — `None` when the guest sent no instance block this port can name.
#[must_use]
pub fn guest_identity(
    inst: Option<kf_abi::notifier::InstanceMem>,
    veid: Option<u32>,
) -> Option<GuestFaultIdentity> {
    /// `ADDR_SYSMEM` / `ADDR_FBMEM` (`ogkm-580: nvgputypes.h`/`mem_desc.h`).
    const ADDR_SYSMEM: u32 = 1;
    const ADDR_FBMEM: u32 = 2;
    /// `NV_MEMORY_UNCACHED`.
    const UNCACHED: u32 = 0;
    let m = inst?;
    let loc = match m.address_space {
        ADDR_FBMEM => InstLocation::Vid,
        ADDR_SYSMEM if m.cache_attrib == UNCACHED => InstLocation::SysNoncoherent,
        ADDR_SYSMEM => InstLocation::SysCoherent,
        _ => return None,
    };
    (m.base % PAGE == 0).then_some(GuestFaultIdentity {
        inst_addr: m.base,
        inst: loc,
        veid: veid.unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kf_chip::hwref::DieGroup;

    const PRIV: u64 = 0xB8_0000;

    /// A host double: records the resolves it was asked for.
    #[derive(Default)]
    struct FakeFile {
        resolved: Mutex<Vec<(Vec<u64>, u32)>>,
    }
    impl EfsFile for FakeFile {
        fn wait(&self, _max: u32, _timeout_us: u32) -> Result<Vec<EfsRecord>, String> {
            Ok(Vec::new())
        }
        fn resolve(&self, ids: &[u64], action: u32) -> Result<(u32, u32), String> {
            self.resolved.lock().unwrap().push((ids.to_vec(), action));
            Ok((ids.len() as u32, 0))
        }
    }

    /// A guest double: 64 KiB of "guest RAM" at 0x10_0000, a shadow and a raise log.
    struct FakeGuest {
        ram: Mutex<Vec<u8>>,
        shadow: Mutex<HashMap<u64, u32>>,
        raises: AtomicU64,
    }
    const RAM_AT: u64 = 0x10_0000;
    impl FakeGuest {
        fn new() -> &'static FakeGuest {
            Box::leak(Box::new(FakeGuest {
                ram: Mutex::new(vec![0; 64 << 10]),
                shadow: Mutex::new(HashMap::new()),
                raises: AtomicU64::new(0),
            }))
        }
        fn entry(&self, gpa: u64) -> [u32; 8] {
            let r = self.ram.lock().unwrap();
            let o = (gpa - RAM_AT) as usize;
            let mut b = [0u8; 32];
            b.copy_from_slice(&r[o..o + 32]);
            kf_abi::faultpacket::from_bytes(&b)
        }
    }
    impl FaultSink for FakeGuest {
        fn shadow(&self, off: u64, val: u32) {
            self.shadow.lock().unwrap().insert(off, val);
        }
        fn raise(&self, _vector: u32) {
            self.raises.fetch_add(1, Ordering::Relaxed);
        }
        fn write_entry(&self, gpa: u64, words: &[u32; 8]) -> bool {
            if !self.is_guest_ram(gpa, 32) {
                return false;
            }
            let o = (gpa - RAM_AT) as usize;
            self.ram.lock().unwrap()[o..o + 32]
                .copy_from_slice(&kf_abi::faultpacket::to_bytes(words));
            true
        }
        fn is_guest_ram(&self, gpa: u64, len: u64) -> bool {
            gpa >= RAM_AT && gpa + len <= RAM_AT + (64 << 10)
        }
    }

    fn plane() -> (&'static FaultPlane, &'static FakeGuest) {
        let c = FaultConsts::for_group(DieGroup::Ga10x).unwrap();
        let p: &'static FaultPlane = Box::leak(Box::new(FaultPlane::new(c, PRIV, 64, 6)));
        let g = FakeGuest::new();
        p.bind(g);
        (p, g)
    }

    fn reg(size: u32, pages: Vec<u64>) -> FaultBufferRegistration {
        FaultBufferRegistration {
            h_client: 1,
            h_object: 2,
            size,
            pages,
        }
    }

    fn rec(id: u64, addr: u64) -> EfsRecord {
        EfsRecord {
            id,
            address: addr,
            gpu_timestamp: 0x1234,
            divert_ns: 0,
            gpu_uuid: [0; 16],
            access_type: 1, // UVM_FAULT_ACCESS_TYPE_READ
            access_type_mask: 1 << 1,
            fault_type: 1,  // UVM_FAULT_TYPE_INVALID_PTE
            client_type: 0, // GPC
            client_id: 3,
            gpc_id: 2,
            utlb_id: 1,
            ve_id: 0,
            mmu_engine_id: 64,
            num_instances: 1,
        }
    }

    const WHO: GuestFaultIdentity = GuestFaultIdentity {
        inst_addr: 0x7_0000_2000,
        inst: InstLocation::Vid,
        veid: 3,
    };

    /// Insert a space without a waiter thread (the tests drive delivery by hand).
    fn space(p: &FaultPlane, key: VasKey) -> Arc<FakeFile> {
        let f = Arc::new(FakeFile::default());
        p.lock().spaces.insert(
            key,
            Space {
                file: f.clone(),
                root: None,
                twins: Vec::new(),
                delivered: Vec::new(),
                undelivered: VecDeque::new(),
                stop: Arc::new(AtomicBool::new(false)),
            },
        );
        f
    }

    #[test]
    fn a_registration_is_validated_to_guest_ram() {
        let (p, g) = plane();
        p.register(&reg(4096, vec![RAM_AT]));
        assert_eq!(p.ring.entries(), 128);
        assert_eq!(g.shadow.lock().unwrap().get(&(PRIV + 0x302c)), Some(&0));
        // ⊘ A page outside guest RAM, a size that is not whole entries, a short page list.
        for bad in [
            reg(4096, vec![0x9000_0000]),
            reg(100, vec![RAM_AT]),
            reg(8192, vec![RAM_AT]),
            reg(4096, vec![RAM_AT + 8]),
        ] {
            p.register(&bad);
            assert_eq!(p.ring.entries(), 0, "{bad:?}");
        }
        assert_eq!(p.counters.registration_refused.load(Ordering::Relaxed), 4);
    }

    /// ★ The end-to-end contract of one delivery: the packet names the guest's instance block and
    /// VEID, the page and the codes; it is VALID; PUT moved and the level was raised.
    #[test]
    fn a_record_becomes_one_valid_packet_and_a_raise() {
        let (p, g) = plane();
        p.register(&reg(4096, vec![RAM_AT]));
        let key = VasKey(0xc1d0_0001_cafe_0001);
        let f = space(p, key);
        p.note_twin(key, 0x55, WHO);
        p.deliver(key, vec![rec(7, 0x7f00_1234_5000)]);
        let e = kf_abi::faultpacket::FaultEntry::decode(&g.entry(RAM_AT));
        assert!(e.valid && e.replayable && e.replayable_en);
        assert_eq!(e.inst_addr, WHO.inst_addr);
        assert_eq!(e.addr, 0x7f00_1234_5000);
        assert_eq!(e.engine_id, 64 + 3, "GRAPHICS + the guest's VEID");
        assert_eq!(e.gpc_id, 2);
        assert_eq!(p.ring.put(), 1);
        assert_eq!(g.shadow.lock().unwrap().get(&(PRIV + 0x302c)), Some(&1));
        assert_eq!(g.raises.load(Ordering::Relaxed), 1);
        assert!(
            f.resolved.lock().unwrap().is_empty(),
            "nothing resolved yet"
        );
        // The guest consumes it: GET = PUT, no raise.
        assert_eq!(p.get_write(1), (1, false));
    }

    /// ⊘ Nobody to attribute to, or nowhere to write: cancelled at once, never parked for nobody.
    #[test]
    fn an_undeliverable_record_is_cancelled() {
        let (p, _g) = plane();
        let key = VasKey(1);
        let f = space(p, key);
        p.deliver(key, vec![rec(1, 0x1000)]);
        assert_eq!(
            f.resolved.lock().unwrap().as_slice(),
            &[(vec![1], UVM_EFS_ACTION_CANCEL)],
            "no buffer registered"
        );
        p.register(&reg(4096, vec![RAM_AT]));
        p.deliver(key, vec![rec(2, 0x2000)]);
        assert_eq!(
            f.resolved.lock().unwrap().last(),
            Some(&(vec![2], UVM_EFS_ACTION_CANCEL)),
            "no guest channel represents the space"
        );
        // A GPC the device never advertised.
        p.note_twin(key, 9, WHO);
        let mut r = rec(3, 0x3000);
        r.gpc_id = 6;
        p.deliver(key, vec![r]);
        assert_eq!(
            f.resolved.lock().unwrap().last(),
            Some(&(vec![3], UVM_EFS_ACTION_CANCEL))
        );
        assert_eq!(p.ring.put(), 0, "nothing was written");
    }

    /// The ring never overwrites an unconsumed entry: the rest waits, and goes after GET moves.
    #[test]
    fn a_full_ring_holds_records_back() {
        let (p, _g) = plane();
        p.register(&reg(128, vec![RAM_AT])); // 4 entries, room 3
        let key = VasKey(2);
        let _f = space(p, key);
        p.note_twin(key, 9, WHO);
        p.deliver(key, (0..5).map(|i| rec(i, 0x1000 * (i + 1))).collect());
        assert_eq!(p.ring.put(), 3);
        assert_eq!(p.outstanding(), (3, 2));
        p.get_write(3);
        p.deliver(key, Vec::new());
        assert_eq!(p.ring.put(), 1, "wrapped");
        assert_eq!(p.outstanding(), (5, 0));
    }

    /// ★ Ownership: a replay resolves exactly what was delivered (and nothing when nothing was); a
    /// cancel naming another space's PDB, or an instance that is not ours, touches nothing.
    #[test]
    fn guest_ops_resolve_only_what_they_name() {
        let (p, _g) = plane();
        p.register(&reg(4096, vec![RAM_AT]));
        let (ka, kb) = (VasKey(0xa), VasKey(0xb));
        let (fa, fb) = (space(p, ka), space(p, kb));
        p.set_root(ka, Some(0x10_0000));
        p.set_root(kb, Some(0x20_0000));
        p.note_twin(ka, 1, WHO);
        p.note_twin(
            kb,
            2,
            GuestFaultIdentity {
                inst_addr: 0x9000,
                ..WHO
            },
        );
        assert!(
            plan_op(&mut p.lock(), FaultOp::Replay { ack_all: false }).is_empty(),
            "a replay with nothing delivered is no host call"
        );
        p.deliver(ka, vec![rec(1, 0x1000), rec(2, 0x2000)]);
        p.deliver(kb, vec![rec(3, 0x2000)]);
        // A cancel of B's PDB at 0x2000 touches B only.
        let plan = plan_op(
            &mut p.lock(),
            FaultOp::CancelVa {
                pdb: Some(0x20_0000),
                pdb_aperture: 0,
                va: 0x2000,
                access: 7,
                engine: 64,
            },
        );
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].1, vec![3]);
        assert!(Arc::ptr_eq(&plan[0].0, &(fb.clone() as Arc<dyn EfsFile>)));
        // A cancel naming a PDB nobody has, or an instance that is not ours: nothing.
        for op in [
            FaultOp::CancelVa {
                pdb: Some(0x30_0000),
                pdb_aperture: 0,
                va: 0x1000,
                access: 7,
                engine: 64,
            },
            FaultOp::CancelInstance {
                inst: 0xdead_0000,
                aperture: 0,
                global: true,
                gpc: 0,
                client: 0,
            },
            // The right address in the wrong aperture class is not the same instance block.
            FaultOp::CancelInstance {
                inst: WHO.inst_addr,
                aperture: kf_abi::faultpacket::INST_APERTURE_SYS_MEM_COHERENT,
                global: true,
                gpc: 0,
                client: 0,
            },
        ] {
            assert!(plan_op(&mut p.lock(), op).is_empty(), "{op:?}");
        }
        // The replay takes what is left — A's two, in one call on A's file.
        let plan = plan_op(&mut p.lock(), FaultOp::Replay { ack_all: true });
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].1, vec![1, 2]);
        assert_eq!(plan[0].2, UVM_EFS_ACTION_REPLAY);
        assert!(Arc::ptr_eq(&plan[0].0, &(fa as Arc<dyn EfsFile>)));
        assert_eq!(p.outstanding(), (0, 0));
    }

    /// A cancel by instance takes the named channel's space: all of it (global), or one GPC/client.
    #[test]
    fn a_cancel_by_instance_is_scoped_to_its_space() {
        let (p, _g) = plane();
        p.register(&reg(4096, vec![RAM_AT]));
        let k = VasKey(0xc);
        let _f = space(p, k);
        p.note_twin(k, 1, WHO);
        let mut other = rec(2, 0x2000);
        other.gpc_id = 1;
        p.deliver(k, vec![rec(1, 0x1000), other]);
        let plan = plan_op(
            &mut p.lock(),
            FaultOp::CancelInstance {
                inst: WHO.inst_addr,
                aperture: kf_abi::faultpacket::INST_APERTURE_VID_MEM,
                global: false,
                gpc: 1,
                client: 3,
            },
        );
        assert_eq!(plan[0].1, vec![2]);
        let plan = plan_op(&mut p.lock(), FaultOp::CancelGlobal);
        assert_eq!(plan[0].1, vec![1]);
    }

    /// A re-registration cancels what the old buffer held (queued to the resolver) and restarts.
    #[test]
    fn a_reregistration_orphans_the_old_records() {
        let (p, _g) = plane();
        p.register(&reg(4096, vec![RAM_AT]));
        let k = VasKey(0xd);
        let _f = space(p, k);
        p.note_twin(k, 1, WHO);
        p.deliver(k, vec![rec(1, 0x1000)]);
        assert_eq!(p.ring.put(), 1);
        p.register(&reg(4096, vec![RAM_AT + 4096]));
        assert_eq!((p.ring.get(), p.ring.put()), (0, 0));
        assert_eq!(p.outstanding(), (0, 0));
        assert_eq!(
            p.jobs.lock().unwrap().len(),
            1,
            "the old id goes to the resolver"
        );
    }

    #[test]
    fn the_guest_identity_is_the_guests_statement() {
        let m = |base, address_space, cache_attrib| kf_abi::notifier::InstanceMem {
            base,
            size: 4096,
            address_space,
            cache_attrib,
        };
        assert_eq!(
            guest_identity(Some(m(0x2000, 2, 0)), Some(5)),
            Some(GuestFaultIdentity {
                inst_addr: 0x2000,
                inst: InstLocation::Vid,
                veid: 5
            })
        );
        assert_eq!(
            guest_identity(Some(m(0x3000, 1, 0)), None).map(|w| (w.inst, w.veid)),
            Some((InstLocation::SysNoncoherent, 0))
        );
        assert_eq!(
            guest_identity(Some(m(0x3008, 1, 1)), None),
            None,
            "unaligned"
        );
        assert_eq!(
            guest_identity(Some(m(0x3000, 7, 1)), None),
            None,
            "no aperture"
        );
        assert_eq!(guest_identity(None, Some(1)), None);
    }
}
