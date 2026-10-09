//! ★★★★★ **The doorbell fast path — one KVM ioeventfd per live guest token, serviced by the
//! register drainer** (`docs/design/V3_DOORBELL_IOEVENTFD.md`).
//!
//! Only the **transport** of a doorbell changes. A guest store of exactly a live token's value to
//! the doorbell register is matched inside the host kernel (`KVM_IOEVENTFD` with `DATAMATCH`) and
//! signals that token's eventfd **without an exit to userspace**; the register drainer wakes on it
//! and does precisely what the trap would have done with that value ([`Sink::deliver`]): a
//! Passthrough token rings its host twin's doorbell inline on the drainer, a Translated/Emulated
//! token is handed to its channel's existing path (RUNG, bitmap, worker wake), and a dead or
//! unknown token is absorbed. Every other store — any unregistered value, a registration KVM
//! refused, a site not yet placed — still exits and takes the trapped path, unchanged.
//!
//! ## The three threads, and what each may do
//!
//! | thread | does | never does |
//! |---|---|---|
//! | the channel **act** thread (birth/free acts) | creates the eventfd, watches it, places/removes KVM registrations ([`DbFast::register`], [`DbFast::deregister`]) — syscalls that may sleep (an SRCU grace period) | touch the drainer's table |
//! | the **main loop** (the C device's memory listener) | a doorbell register appears/disappears at a GPA ([`DbFast::site_add`] / [`DbFast::site_del`]) | — |
//! | the register **drainer** | drains ready eventfds and delivers ([`DbFast::service_ready`], [`DbFast::on_ready`]), applies the act thread's table commands | a KVM ioctl, a wait on another thread, a timer |
//!
//! ⊘ No vCPU ever touches any of it: the vCPU's part is done by the kernel.
//!
//! ## Why no doorbell is ever lost (the lost-wakeup argument)
//!
//! 1. **Placement.** Before a registration is in KVM at a site, a store there exits and the trap
//!    delivers it from the token word. After, the kernel counts it in the eventfd. There is no
//!    third place for it to go. The eventfd is watched *before* it is placed, so a count that
//!    lands before the drainer has the table entry is still reported (level-triggered), and the
//!    drainer applies pending commands before it declares a ready tag stale.
//! 2. **Drain-before-act.** The drainer reads (and so resets) the counter, THEN delivers. A store
//!    counted after the read leaves the fd ready, and the next poll delivers it. Several stores
//!    counted before one read are covered by the one delivery that follows it — incidental
//!    coalescing only, never a wait to accumulate.
//! 3. **Removal.** [`DbFast::deregister`] removes every KVM placement first. When the kernel's
//!    deassign returns it has finished every signal the registration could make (SRCU grace
//!    period), so the drainer's **final drain** at the `Remove` command sees the last count,
//!    delivers it, and only then acknowledges. After that the token's stores exit again.
//!
//! ## Why a stale eventfd never rings the wrong twin (generation safety)
//!
//! Every registration has its **own** eventfd, created at birth and closed after its removal was
//! acknowledged; a re-born token gets a new one. Counts are never moved between eventfds, so a
//! count of an old registration cannot be delivered for a new one. Delivery reads the token word
//! at that instant, exactly as the trap does, and [`DbFast::deregister`] does not return until the
//! drainer has delivered the registration's last count — so the caller frees the twin (and a later
//! act re-births the token) only after the old eventfd can deliver nothing more.

use crate::stall::{LockId, Stall};
use kf_linux_raw::{MAX_READY_BATCH, Notifier, PollTimeout, Poller, ReadyTokens};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::os::fd::BorrowedFd;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

/// Poller tags at or above this are doorbell eventfds; below it are the drainer's own sources.
pub const DB_TAG_BASE: u64 = 1 << 48;

/// How long [`DbFast::deregister`] waits for the drainer's acknowledgement before it reports the
/// drainer as unresponsive (counted — it must stay 0) and returns without the final ledger.
pub const ACK_TIMEOUT: Duration = Duration::from_secs(2);

/// `ENOSPC`, `EEXIST`, `EMFILE` — the kernel refusals the counters name.
const ENOSPC: i32 = 28;
const EEXIST: i32 = 17;

/// ★ The KVM side: assign or deassign one 4-byte `DATAMATCH` MMIO ioeventfd for `value` at `gpa`,
/// signalling `fd`. `Err` carries the kernel's positive `errno`. Called only from the act thread
/// and the main loop, never from a vCPU or the drainer. Production: the C device's
/// `kvm_vm_ioctl(KVM_IOEVENTFD)`; GPU-free tests: `kf_linux_raw::KvmVm::ioeventfd`.
pub trait Ioeventfd: Send + Sync {
    /// Assign (`assign`) or deassign one registration.
    ///
    /// # Errors
    /// The kernel's `errno`.
    fn set(&self, gpa: u64, value: u32, fd: BorrowedFd<'_>, assign: bool) -> Result<(), i32>;
}

/// What delivering one notification did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivered {
    /// A Passthrough token: the host twin's doorbell was rung inline (`reached`: the store landed).
    Rang {
        /// The host store succeeded.
        reached: bool,
    },
    /// A Translated/Emulated token: handed to its channel's path (RUNG + publish + worker wake,
    /// exactly as the trap does it).
    Handed,
    /// The token was dead or unknown at delivery: absorbed, exactly as the trap absorbs it.
    Absorbed,
}

/// ★ What the drainer does with a delivered doorbell: the device implements it with the trap's
/// own doorbell arm, so the handling is the trapped path's by construction.
pub trait Sink {
    /// One notification for token `idx`, whose guest value is `value` (one or more guest stores),
    /// through registration `tag` (the registration's identity: diagnostics and the generation
    /// tests; the handling must not depend on it — it is the trap's, from the token word).
    fn deliver(&self, idx: u32, value: u32, tag: u64) -> Delivered;
}

/// One registration's delivery ledger — returned by [`DbFast::deregister`] for the per-token
/// `DOORBELL-LEDGER` line (the grader's visibility must not shrink because the trap no longer
/// sees these doorbells).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FastLedger {
    /// Guest stores that arrived through the eventfd (the sum of the counters read).
    pub doorbells: u64,
    /// Deliveries (counter reads that found at least one store).
    pub wakes: u64,
    /// Doorbells whose delivery reached the host twin (Passthrough, store landed) or the
    /// channel's path (Translated/Emulated).
    pub forwarded: u64,
    /// Doorbells delivered while the token was dead/unknown (absorbed, as the trap would).
    pub absorbed: u64,
    /// Passthrough host rings that did NOT land (counted as not forwarded).
    pub failed: u64,
    /// KVM placements this registration held when it was removed.
    pub sites: u32,
    /// Placements refused (budget or kernel): those sites stayed on the trapped path.
    pub refused: u32,
}

impl FastLedger {
    fn add(&mut self, n: u64, d: Delivered) {
        self.doorbells += n;
        self.wakes += 1;
        match d {
            Delivered::Rang { reached: true } | Delivered::Handed => self.forwarded += n,
            Delivered::Rang { reached: false } => self.failed += n,
            Delivered::Absorbed => self.absorbed += n,
        }
    }
}

/// A log-linear histogram (8 sub-buckets per octave of nanoseconds, ~9 % resolution), lock-free.
pub struct Hist {
    b: [AtomicU64; 512],
    n: AtomicU64,
    max: AtomicU64,
}

impl Default for Hist {
    fn default() -> Self {
        Hist {
            b: core::array::from_fn(|_| AtomicU64::new(0)),
            n: AtomicU64::new(0),
            max: AtomicU64::new(0),
        }
    }
}

impl Hist {
    fn bucket(ns: u64) -> usize {
        if ns < 8 {
            return ns as usize;
        }
        let lg = 63 - ns.leading_zeros() as usize; // >= 3
        let sub = ((ns >> (lg - 3)) & 7) as usize;
        (lg * 8 + sub).min(511)
    }

    /// The upper edge of bucket `i`, ns.
    fn upper(i: usize) -> u64 {
        if i < 8 {
            return i as u64;
        }
        let (lg, sub) = (i / 8, (i % 8) as u64);
        ((8 + sub + 1) << (lg - 3)).saturating_sub(1)
    }

    /// Record one sample.
    pub fn add(&self, ns: u64) {
        self.b[Self::bucket(ns)].fetch_add(1, Ordering::Relaxed);
        self.n.fetch_add(1, Ordering::Relaxed);
        self.max.fetch_max(ns, Ordering::Relaxed);
    }

    /// Samples recorded.
    pub fn count(&self) -> u64 {
        self.n.load(Ordering::Relaxed)
    }

    /// The `p`-th percentile (0..=100) as the upper edge of its bucket, ns (0 when empty).
    pub fn pct(&self, p: f64) -> u64 {
        let n = self.count();
        if n == 0 {
            return 0;
        }
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let want = ((n as f64) * p / 100.0).ceil().max(1.0) as u64;
        let mut seen = 0;
        for (i, b) in self.b.iter().enumerate() {
            seen += b.load(Ordering::Relaxed);
            if seen >= want {
                return Self::upper(i).min(self.max.load(Ordering::Relaxed));
            }
        }
        self.max.load(Ordering::Relaxed)
    }

    /// `n= p50= p99= max=` in µs, for a status line.
    pub fn summary_us(&self) -> String {
        format!(
            "n={} p50={:.1}us p99={:.1}us max={:.1}us",
            self.count(),
            self.pct(50.0) as f64 / 1000.0,
            self.pct(99.0) as f64 / 1000.0,
            self.max.load(Ordering::Relaxed) as f64 / 1000.0
        )
    }
}

/// Counters for the status line and the gates — never a decision input.
#[derive(Default)]
pub struct Counters {
    /// Registrations made (token births that got an eventfd).
    pub registered: AtomicU64,
    /// Registrations removed.
    pub deregistered: AtomicU64,
    /// KVM placements made.
    pub placed: AtomicU64,
    /// KVM placements removed.
    pub unplaced: AtomicU64,
    /// Placements refused by OUR budget — those (token, site) pairs stay trapped.
    pub refused_budget: AtomicU64,
    /// Placements refused by the kernel (any errno) — trapped.
    pub refused_kvm: AtomicU64,
    /// … of which `ENOSPC` (the MMIO bus full of non-ioeventfd devices).
    pub refused_enospc: AtomicU64,
    /// … of which `EEXIST` (a `(gpa, value)` collision — must stay 0).
    pub refused_eexist: AtomicU64,
    /// Registrations refused before KVM: no eventfd (`EMFILE`…) or the poller refused it.
    pub refused_fd: AtomicU64,
    /// Deassigns the kernel refused — must stay 0.
    pub deassign_failed: AtomicU64,
    /// A token registered while an older registration of it was still live — must stay 0.
    pub double_register: AtomicU64,
    /// Deregistrations whose drainer acknowledgement timed out — must stay 0.
    pub ack_timeouts: AtomicU64,
    /// Guest stores delivered through eventfds.
    pub doorbells: AtomicU64,
    /// Deliveries.
    pub wakes: AtomicU64,
    /// … to a Passthrough token (host rings).
    pub rang: AtomicU64,
    /// … to a Translated/Emulated token.
    pub handed: AtomicU64,
    /// … absorbed (dead/unknown token).
    pub absorbed: AtomicU64,
    /// Readiness reported for a tag no longer registered (benign: a removal raced a report).
    pub stale_ready: AtomicU64,
    /// Readiness whose counter read found 0 (benign: already drained).
    pub empty_ready: AtomicU64,
    /// Non-blocking polls made by the drainer between its other work.
    pub polls: AtomicU64,
    /// … that found at least one doorbell.
    pub poll_hits: AtomicU64,
    /// The largest number of doorbell fds ready at once.
    pub max_batch: AtomicU64,
    /// Registrations live now.
    pub live_regs: AtomicU64,
    /// KVM placements live now.
    pub live_placed: AtomicU64,
    /// Doorbell sites mapped now. ★ `v3-mc22`: written under the registry lock by the thread that
    /// changes the set, read without it by [`DbFast::status`] — the drainer's heartbeat prints that
    /// line, and the registry lock is held across `KVM_IOEVENTFD` (each one an SRCU grace period in
    /// the kernel), so reading the set there would park the doorbell servicer behind the main loop.
    pub live_sites: AtomicU64,
}

/// One live registration, the act thread's view.
struct Reg {
    tag: u64,
    value: u32,
    efd: Arc<Notifier>,
    placed: BTreeSet<u64>,
    refused: u32,
}

#[derive(Default)]
struct Registry {
    sites: BTreeSet<u64>,
    regs: BTreeMap<u32, Reg>,
    placed: usize,
    next: u64,
}

enum Cmd {
    Add {
        tag: u64,
        idx: u32,
        value: u32,
        efd: Arc<Notifier>,
    },
    Remove {
        tag: u64,
        ack: mpsc::SyncSender<FastLedger>,
        /// Drop the final count instead of delivering it (a registration replaced under a live
        /// token: its count predates the token's current generation).
        discard: bool,
    },
}

struct Live {
    idx: u32,
    value: u32,
    efd: Arc<Notifier>,
    ledger: FastLedger,
}

/// The drainer's own half — only the drainer thread locks it, so it is never contended.
struct Side {
    rx: mpsc::Receiver<Cmd>,
    live: HashMap<u64, Live>,
}

/// What [`DbFast::register`] did — the channel's doorbells take the fast path at the sites it
/// was placed at, and the trapped path everywhere else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegOutcome {
    /// The fast path is off (no KVM verb registered): every doorbell stays trapped.
    Off,
    /// Registered; placed at `placed` of the `sites` currently mapped (`refused` stay trapped).
    Registered {
        /// Its poller tag.
        tag: u64,
        /// Sites placed.
        placed: u32,
        /// Sites refused.
        refused: u32,
        /// Sites mapped now.
        sites: u32,
    },
    /// No eventfd could be created or watched: every doorbell of this token stays trapped.
    Refused,
}

/// ★ The fast path of one device.
pub struct DbFast {
    budget: std::sync::atomic::AtomicUsize,
    ops: OnceLock<Box<dyn Ioeventfd>>,
    poller: Poller,
    kick: &'static Notifier,
    reg: Mutex<Registry>,
    /// Where blocked waits on `reg` are recorded, once the device attaches its instruments.
    stall: OnceLock<&'static Stall>,
    tx: Mutex<mpsc::Sender<Cmd>>,
    side: Mutex<Side>,
    /// Counters.
    pub counters: Counters,
    /// Drainer wake (or poll) → delivery done, per delivery.
    pub wake_to_deliver: Hist,
    /// A deregistration's wait for the drainer's acknowledgement (act thread).
    pub ack_wait: Hist,
    /// Refusals printed so far (the first few are named in the log; all are counted).
    printed: AtomicU64,
}

/// How many refusals are printed by name before only the counters speak.
const PRINT_LIMIT: u64 = 32;

impl DbFast {
    /// A fast path whose drainer waits on its own [`Poller`] (which the drainer also watches its
    /// wake fd in) and is kicked through `kick` when the act thread queues a table command. At most
    /// `budget` KVM placements are held at once; beyond it a token stays on the trapped path
    /// ([`DbFast::enable`] may set it again).
    ///
    /// # Errors
    /// The poller could not be created.
    pub fn new(budget: usize, kick: &'static Notifier) -> Result<DbFast, String> {
        let (tx, rx) = mpsc::channel();
        Ok(DbFast {
            budget: std::sync::atomic::AtomicUsize::new(budget),
            ops: OnceLock::new(),
            poller: Poller::create().map_err(|e| format!("doorbell poller: {e:?}"))?,
            kick,
            reg: Mutex::new(Registry::default()),
            stall: OnceLock::new(),
            tx: Mutex::new(tx),
            side: Mutex::new(Side {
                rx,
                live: HashMap::new(),
            }),
            counters: Counters::default(),
            wake_to_deliver: Hist::default(),
            ack_wait: Hist::default(),
            printed: AtomicU64::new(0),
        })
    }

    /// Attach the device's stall instruments: blocked waits on the registry lock are recorded.
    pub fn attach_stall(&self, st: &'static Stall) {
        let _ = self.stall.set(st);
    }

    /// The registry lock, its blocked wait recorded ([`LockId::DbReg`]). The main loop and the act
    /// thread hold it across `KVM_IOEVENTFD`; the drainer never takes it.
    fn lock_reg(&self) -> std::sync::LockResult<std::sync::MutexGuard<'_, Registry>> {
        match self.reg.try_lock() {
            Ok(g) => Ok(g),
            Err(std::sync::TryLockError::Poisoned(p)) => Err(p),
            Err(std::sync::TryLockError::WouldBlock) => {
                let t = Instant::now();
                let r = self.reg.lock();
                if let Some(st) = self.stall.get() {
                    st.note_lock_wait(
                        LockId::DbReg,
                        u64::try_from(t.elapsed().as_nanos()).unwrap_or(u64::MAX),
                    );
                }
                r
            }
        }
    }

    /// ★ Turn the fast path ON by handing it the KVM verb. Until this is called every token stays
    /// on the trapped path. Returns `false` if a verb was already set.
    pub fn enable(&self, ops: Box<dyn Ioeventfd>) -> bool {
        self.ops.set(ops).is_ok()
    }

    /// Set the placement budget (the C device's `doorbell-ioeventfd-max`). Placements already held
    /// are kept; new ones are refused while at or over it.
    pub fn set_budget(&self, budget: usize) {
        self.budget.store(budget, Ordering::Relaxed);
    }

    /// Whether the fast path is on.
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.ops.get().is_some()
    }

    /// The per-device placement budget.
    #[must_use]
    pub fn budget(&self) -> usize {
        self.budget.load(Ordering::Relaxed)
    }

    /// The drainer's readiness set: the drainer watches its own wake fd here (at a tag below
    /// [`DB_TAG_BASE`]) and blocks in it; doorbell eventfds are added and removed by this type.
    #[must_use]
    pub fn poller(&self) -> &Poller {
        &self.poller
    }

    fn note_refusal(&self, what: std::fmt::Arguments<'_>) {
        if self.printed.fetch_add(1, Ordering::Relaxed) < PRINT_LIMIT {
            kf_util::klog!("kf3: DBFAST {what} — that doorbell stays on the TRAPPED path");
        }
    }

    fn send(&self, c: Cmd) {
        if let Ok(tx) = self.tx.lock() {
            let _ = tx.send(c);
        }
        let _ = self.kick.signal();
    }

    /// Place `reg` at `site` under the budget. Returns whether it was placed.
    fn place(&self, placed: &mut usize, reg: &mut Reg, idx: u32, site: u64) -> bool {
        let Some(ops) = self.ops.get() else {
            return false;
        };
        let c = &self.counters;
        let budget = self.budget();
        if *placed >= budget {
            c.refused_budget.fetch_add(1, Ordering::Relaxed);
            reg.refused += 1;
            self.note_refusal(format_args!(
                "tok={idx:#x} value={:#010x} site={site:#x} REFUSED by the budget ({budget} placements held)",
                reg.value
            ));
            return false;
        }
        match ops.set(site, reg.value, reg.efd.as_source_fd(), true) {
            Ok(()) => {
                reg.placed.insert(site);
                *placed += 1;
                c.placed.fetch_add(1, Ordering::Relaxed);
                c.live_placed.fetch_add(1, Ordering::Relaxed);
                true
            }
            Err(errno) => {
                c.refused_kvm.fetch_add(1, Ordering::Relaxed);
                if errno == ENOSPC {
                    c.refused_enospc.fetch_add(1, Ordering::Relaxed);
                }
                if errno == EEXIST {
                    c.refused_eexist.fetch_add(1, Ordering::Relaxed);
                }
                reg.refused += 1;
                self.note_refusal(format_args!(
                    "tok={idx:#x} value={:#010x} site={site:#x} REFUSED by KVM (errno {errno}{})",
                    reg.value,
                    match errno {
                        ENOSPC => ": the MMIO bus is full",
                        EEXIST => ": the (gpa, value) is already registered",
                        _ => "",
                    }
                ));
                false
            }
        }
    }

    fn unplace(&self, reg: &Reg, idx: u32, site: u64) {
        let Some(ops) = self.ops.get() else { return };
        let c = &self.counters;
        match ops.set(site, reg.value, reg.efd.as_source_fd(), false) {
            Ok(()) => {
                c.unplaced.fetch_add(1, Ordering::Relaxed);
            }
            Err(errno) => {
                // ⊘ Only reachable if our bookkeeping disagrees with the kernel's (ENOENT): then
                // the kernel holds nothing for this (site, value) and signals nothing — safe, but a
                // defect, so it is loud.
                c.deassign_failed.fetch_add(1, Ordering::Relaxed);
                kf_util::klog_limited!(
                    "kf3: DBFAST tok={idx:#x} value={:#010x} site={site:#x} DEASSIGN REFUSED (errno {errno}) — bookkeeping defect",
                    reg.value
                );
            }
        }
        let _ = c
            .live_placed
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_sub(1));
    }

    /// ★ **Act thread — a channel was born** with token index `idx`, whose guest writes `value`
    /// (`kf_trap::tokenindex::GuestTokenFormat::value`). Call AFTER its token word is installed:
    /// until the placement lands the trap serves it, after, the eventfd does.
    pub fn register(&self, idx: u32, value: u32) -> RegOutcome {
        if !self.enabled() {
            return RegOutcome::Off;
        }
        let c = &self.counters;
        let efd = match Notifier::create() {
            Ok(n) => Arc::new(n),
            Err(e) => {
                c.refused_fd.fetch_add(1, Ordering::Relaxed);
                self.note_refusal(format_args!(
                    "tok={idx:#x} value={value:#010x} no eventfd ({e:?})"
                ));
                return RegOutcome::Refused;
            }
        };
        let Ok(mut r) = self.lock_reg() else {
            return RegOutcome::Refused;
        };
        if let Some(old) = r.regs.remove(&idx) {
            // ⊘ A birth over a live registration: the free that should have removed it never
            // came. Remove it now (its fd is never read again: no count of it can reach the new
            // registration), loudly.
            c.double_register.fetch_add(1, Ordering::Relaxed);
            kf_util::klog_limited!(
                "kf3: DBFAST tok={idx:#x} registered again while registration {:#x} is live — removing the old one",
                old.tag
            );
            for site in &old.placed {
                self.unplace(&old, idx, *site);
            }
            r.placed -= old.placed.len();
            let _ = self.poller.unwatch(old.efd.as_source_fd());
            let (ack, _) = mpsc::sync_channel(1);
            // Its last count predates the token word the caller just installed: discarded, never
            // delivered to the new generation.
            self.send(Cmd::Remove {
                tag: old.tag,
                ack,
                discard: true,
            });
            c.live_regs.fetch_sub(1, Ordering::Relaxed);
        }
        r.next += 1;
        let tag = DB_TAG_BASE + r.next;
        // ⊘ Watched BEFORE it is placed: a count KVM makes between the placement and the drainer
        // applying `Add` is still reported, because readiness is level-triggered.
        if let Err(e) = self.poller.watch(efd.as_source_fd(), tag) {
            c.refused_fd.fetch_add(1, Ordering::Relaxed);
            self.note_refusal(format_args!(
                "tok={idx:#x} value={value:#010x} not watchable ({e:?})"
            ));
            return RegOutcome::Refused;
        }
        self.send(Cmd::Add {
            tag,
            idx,
            value,
            efd: Arc::clone(&efd),
        });
        let mut reg = Reg {
            tag,
            value,
            efd,
            placed: BTreeSet::new(),
            refused: 0,
        };
        let sites: Vec<u64> = r.sites.iter().copied().collect();
        let mut placed = r.placed;
        for s in &sites {
            self.place(&mut placed, &mut reg, idx, *s);
        }
        r.placed = placed;
        let out = RegOutcome::Registered {
            tag,
            placed: u32::try_from(reg.placed.len()).unwrap_or(u32::MAX),
            refused: reg.refused,
            sites: u32::try_from(sites.len()).unwrap_or(u32::MAX),
        };
        r.regs.insert(idx, reg);
        c.registered.fetch_add(1, Ordering::Relaxed);
        c.live_regs.fetch_add(1, Ordering::Relaxed);
        out
    }

    /// ★ **Act thread — a channel is being freed.** Removes every KVM placement of `idx`, then
    /// hands the eventfd to the drainer for its final drain and waits for the acknowledgement: when
    /// this returns, no store of the old registration can be delivered any more, so the caller may
    /// free the twin and a later birth may reuse the token. `None` when `idx` had no registration
    /// (the fast path was off, or refused it) or the drainer did not answer (counted).
    pub fn deregister(&self, idx: u32) -> Option<FastLedger> {
        let (reg, sites_held) = {
            let mut r = self.lock_reg().ok()?;
            let reg = r.regs.remove(&idx)?;
            for site in &reg.placed {
                self.unplace(&reg, idx, *site);
            }
            r.placed -= reg.placed.len();
            let held = u32::try_from(reg.placed.len()).unwrap_or(u32::MAX);
            (reg, held)
        };
        let c = &self.counters;
        let _ = self.poller.unwatch(reg.efd.as_source_fd());
        let (ack, done) = mpsc::sync_channel(1);
        let t0 = Instant::now();
        self.send(Cmd::Remove {
            tag: reg.tag,
            ack,
            discard: false,
        });
        c.deregistered.fetch_add(1, Ordering::Relaxed);
        c.live_regs.fetch_sub(1, Ordering::Relaxed);
        match done.recv_timeout(ACK_TIMEOUT) {
            Ok(mut l) => {
                self.ack_wait
                    .add(u64::try_from(t0.elapsed().as_nanos()).unwrap_or(u64::MAX));
                l.sites = sites_held;
                l.refused = reg.refused;
                Some(l)
            }
            Err(_) => {
                c.ack_timeouts.fetch_add(1, Ordering::Relaxed);
                kf_util::klog_limited!(
                    "kf3: DBFAST tok={idx:#x} registration {:#x}: the drainer did not acknowledge its removal within {ACK_TIMEOUT:?}",
                    reg.tag
                );
                None
            }
        }
    }

    /// ★ **Main loop — a doorbell register became visible at guest-physical `gpa`** (the C
    /// device's memory listener: BAR0 mapped, or a Hopper+ BAR1 usermode view placed). Every live
    /// registration is placed there too, under the budget.
    pub fn site_add(&self, gpa: u64) {
        let Ok(mut r) = self.lock_reg() else { return };
        if !r.sites.insert(gpa) {
            return;
        }
        self.counters.live_sites.store(
            u64::try_from(r.sites.len()).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
        let mut placed = r.placed;
        let idxs: Vec<u32> = r.regs.keys().copied().collect();
        for idx in idxs {
            if let Some(reg) = r.regs.get_mut(&idx) {
                self.place(&mut placed, reg, idx, gpa);
            }
        }
        r.placed = placed;
    }

    /// ★ **Main loop — the doorbell register at `gpa` went away** (BAR moved or decode off, a BAR1
    /// view removed): every placement there is removed.
    pub fn site_del(&self, gpa: u64) {
        let Ok(mut r) = self.lock_reg() else { return };
        if !r.sites.remove(&gpa) {
            return;
        }
        self.counters.live_sites.store(
            u64::try_from(r.sites.len()).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
        let idxs: Vec<u32> = r.regs.keys().copied().collect();
        let mut gone = 0;
        for idx in idxs {
            if let Some(reg) = r.regs.get_mut(&idx)
                && reg.placed.remove(&gpa)
            {
                gone += 1;
                let reg = &*reg;
                self.unplace(reg, idx, gpa);
            }
        }
        r.placed -= gone;
    }

    /// The eventfd of live registration `tag` — for measurements and tests only: signalling it is
    /// exactly what the kernel's ioeventfd does on a matching guest store.
    #[must_use]
    pub fn efd_for_bench(&self, tag: u64) -> Option<Arc<Notifier>> {
        self.reg
            .lock()
            .ok()?
            .regs
            .values()
            .find(|r| r.tag == tag)
            .map(|r| Arc::clone(&r.efd))
    }

    /// The sites mapped now. ⊘ Takes the registry lock, which is held across `KVM_IOEVENTFD`: never
    /// on the drainer ([`Self::status`] reads [`Counters::live_sites`] instead).
    #[must_use]
    pub fn sites(&self) -> Vec<u64> {
        self.reg
            .lock()
            .map(|r| r.sites.iter().copied().collect())
            .unwrap_or_default()
    }

    /// `(registrations, placements)` live now.
    #[must_use]
    pub fn live(&self) -> (usize, usize) {
        self.reg
            .lock()
            .map(|r| (r.regs.len(), r.placed))
            .unwrap_or_default()
    }

    // ---- the drainer's half ------------------------------------------------------------------

    /// Apply the act thread's queued commands. Drainer only.
    fn sync(&self, side: &mut Side, sink: &dyn Sink) {
        while let Ok(cmd) = side.rx.try_recv() {
            match cmd {
                Cmd::Add {
                    tag,
                    idx,
                    value,
                    efd,
                } => {
                    side.live.insert(
                        tag,
                        Live {
                            idx,
                            value,
                            efd,
                            ledger: FastLedger::default(),
                        },
                    );
                }
                Cmd::Remove { tag, ack, discard } => {
                    let mut ledger = FastLedger::default();
                    if let Some(mut l) = side.live.remove(&tag) {
                        // ★ The FINAL drain: every KVM placement is gone, so this count is the last
                        // one this registration can ever have. Delivered like any other — unless the
                        // registration was replaced under a live token (then discarded).
                        if discard {
                            let _ = l.efd.drain();
                        } else {
                            let t0 = Instant::now();
                            self.drain_and_deliver(&mut l, tag, sink, t0);
                        }
                        ledger = l.ledger;
                    }
                    let _ = ack.send(ledger);
                }
            }
        }
    }

    /// Read (reset) the counter, then deliver once if it held anything. Returns whether it did.
    fn drain_and_deliver(&self, l: &mut Live, tag: u64, sink: &dyn Sink, t_wake: Instant) -> bool {
        let c = &self.counters;
        let n = match l.efd.drain() {
            Ok(n) => n,
            Err(_) => return false,
        };
        if n == 0 {
            return false;
        }
        let d = sink.deliver(l.idx, l.value, tag);
        self.wake_to_deliver
            .add(u64::try_from(t_wake.elapsed().as_nanos()).unwrap_or(u64::MAX));
        l.ledger.add(n, d);
        c.doorbells.fetch_add(n, Ordering::Relaxed);
        c.wakes.fetch_add(1, Ordering::Relaxed);
        match d {
            Delivered::Rang { .. } => c.rang.fetch_add(1, Ordering::Relaxed),
            Delivered::Handed => c.handed.fetch_add(1, Ordering::Relaxed),
            Delivered::Absorbed => c.absorbed.fetch_add(n, Ordering::Relaxed),
        };
        true
    }

    /// ★ **Drainer — the tags a wait on [`DbFast::poller`] reported** (tags below [`DB_TAG_BASE`]
    /// are the caller's own and are skipped). `t_wake` is when the wait returned. Returns how many
    /// deliveries were made. Never blocks.
    pub fn on_ready(&self, tags: &ReadyTokens, t_wake: Instant, sink: &dyn Sink) -> usize {
        let Ok(mut side) = self.side.lock() else {
            return 0;
        };
        let side = &mut *side;
        // Commands first: a Remove queued before this report is honoured before its tag is read.
        self.sync(side, sink);
        let mut n = 0;
        let mut ready = 0u64;
        for tag in tags.iter().filter(|t| *t >= DB_TAG_BASE) {
            ready += 1;
            if !side.live.contains_key(&tag) {
                self.sync(side, sink);
            }
            match side.live.get_mut(&tag) {
                Some(l) => {
                    if self.drain_and_deliver(l, tag, sink, t_wake) {
                        n += 1;
                    } else {
                        self.counters.empty_ready.fetch_add(1, Ordering::Relaxed);
                    }
                }
                None => {
                    self.counters.stale_ready.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        self.counters.max_batch.fetch_max(ready, Ordering::Relaxed);
        n
    }

    /// ★ **Drainer — deliver the doorbells that are ready NOW: ONE bounded batch** (at the top of its
    /// loop, and before a privileged register write is applied). One non-blocking poll of at most
    /// [`MAX_READY_BATCH`] fds, delivered, and back to the caller; a no-op without a syscall when
    /// nothing is registered. Returns how many deliveries were made.
    ///
    /// ⊘ **Not a loop.** It used to repeat while a poll came back full, so a guest ringing 64+ tokens
    /// continuously could keep the drainer here forever, with privileged writes, held-reply release
    /// and hotplug/RC delivery starved (owner rule 2026-10-09: no unbounded loop on the drainer). A
    /// fd still ready is level-triggered: the next call delivers it. A call that filled the batch is
    /// counted ([`Stall::doorbell_batch_full`]).
    ///
    /// ⚠ **Ordering, as relaxed.** §4's "a doorbell a vCPU stored before a later privileged write is
    /// delivered before that write is applied" held only because this looped until the ready set was
    /// empty. With more than a batch of tokens ready at once, a doorbell beyond the first batch can
    /// now be delivered after a privileged write queued behind it. The trap never ordered doorbells of
    /// DIFFERENT tokens against privileged writes (a worker served them asynchronously), and a
    /// doorbell is never lost (level-triggered); the case that mattered, a free after its own
    /// doorbell, is closed by `deregister`'s final drain. If the owner wants the old guarantee back,
    /// the bounded way is a per-call snapshot (each ready tag at most once, at most `live_regs`
    /// polls) — not built.
    pub fn service_ready(&self, sink: &dyn Sink) -> usize {
        // Commands first (a queued Remove's acknowledgement is someone's wait); then, with nothing
        // registered on this side, there is nothing a poll could report: no syscall.
        match self.side.lock() {
            Ok(mut side) => {
                self.sync(&mut side, sink);
                if side.live.is_empty() {
                    return 0;
                }
            }
            Err(_) => return 0,
        }
        let mut ready = ReadyTokens::new();
        self.counters.polls.fetch_add(1, Ordering::Relaxed);
        let t = Instant::now();
        let got = self
            .poller
            .wait(&mut ready, PollTimeout::Immediate)
            .unwrap_or(0);
        let n = self.on_ready(&ready, t, sink);
        if n > 0 {
            self.counters.poll_hits.fetch_add(1, Ordering::Relaxed);
        }
        if got >= MAX_READY_BATCH
            && let Some(st) = self.stall.get()
        {
            st.doorbell_batch_full.fetch_add(1, Ordering::Relaxed);
        }
        n
    }

    /// One line for the device's status/boot log.
    #[must_use]
    pub fn status(&self) -> String {
        let c = &self.counters;
        let o = Ordering::Relaxed;
        format!(
            "dbfast[{} regs={} placed={}/{} sites={} doorbells={} wakes={} rang={} handed={} absorbed={} refused(budget={} kvm={} enospc={} eexist={} fd={}) deassign_failed={} ack_timeouts={} double={} stale={} empty={} polls={}/{} max_batch={} wake_to_deliver[{}] ack_wait[{}]]",
            if self.enabled() { "ON" } else { "off" },
            c.live_regs.load(o),
            c.live_placed.load(o),
            self.budget(),
            c.live_sites.load(o),
            c.doorbells.load(o),
            c.wakes.load(o),
            c.rang.load(o),
            c.handed.load(o),
            c.absorbed.load(o),
            c.refused_budget.load(o),
            c.refused_kvm.load(o),
            c.refused_enospc.load(o),
            c.refused_eexist.load(o),
            c.refused_fd.load(o),
            c.deassign_failed.load(o),
            c.ack_timeouts.load(o),
            c.double_register.load(o),
            c.stale_ready.load(o),
            c.empty_ready.load(o),
            c.poll_hits.load(o),
            c.polls.load(o),
            c.max_batch.load(o),
            self.wake_to_deliver.summary_us(),
            self.ack_wait.summary_us(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    /// A KVM stand-in: records placements, refuses on demand (so the refusal arms are reachable
    /// deterministically — the real-kernel exhaustion is `tests/dbfast_kvm.rs`).
    #[derive(Default)]
    struct FakeKvm {
        held: Mutex<BTreeSet<(u64, u32)>>,
        refuse_after: Mutex<Option<usize>>,
        refuse_errno: i32,
    }

    impl Ioeventfd for Arc<FakeKvm> {
        fn set(&self, gpa: u64, value: u32, _fd: BorrowedFd<'_>, assign: bool) -> Result<(), i32> {
            let mut h = self.held.lock().unwrap();
            if assign {
                if let Some(n) = *self.refuse_after.lock().unwrap()
                    && h.len() >= n
                {
                    return Err(self.refuse_errno);
                }
                if !h.insert((gpa, value)) {
                    return Err(EEXIST);
                }
            } else if !h.remove(&(gpa, value)) {
                return Err(2);
            }
            Ok(())
        }
    }

    /// A sink that records deliveries and answers from a fake token table.
    #[derive(Default)]
    struct Rec {
        got: Mutex<Vec<(u32, u32)>>,
        dead: Mutex<BTreeSet<u32>>,
    }
    impl Sink for Rec {
        fn deliver(&self, idx: u32, value: u32, _tag: u64) -> Delivered {
            self.got.lock().unwrap().push((idx, value));
            if self.dead.lock().unwrap().contains(&idx) {
                Delivered::Absorbed
            } else {
                Delivered::Rang { reached: true }
            }
        }
    }

    fn kick() -> &'static Notifier {
        Box::leak(Box::new(Notifier::create().unwrap()))
    }

    /// The efd KVM would signal for `idx` (tests only: looked up through the registry).
    fn efd_of(f: &DbFast, idx: u32) -> Arc<Notifier> {
        Arc::clone(&f.reg.lock().unwrap().regs[&idx].efd)
    }

    fn pump(f: &DbFast, s: &dyn Sink) -> usize {
        f.service_ready(s)
    }

    /// ★ `v3-mc22`: the status line never waits on the registry lock. The drainer prints it every
    /// 2 s, and the main loop and the act thread hold that lock across `KVM_IOEVENTFD` (an SRCU
    /// grace period each) — a status line that took it parked the doorbell servicer behind them.
    #[test]
    fn the_status_line_never_waits_on_the_registry_lock() {
        let f = Arc::new(DbFast::new(16, kick()).unwrap());
        assert!(f.enable(Box::new(Arc::new(FakeKvm::default()))));
        f.site_add(0x1000_0090);
        f.site_add(0x2000_0090);
        f.site_add(0x2000_0090); // idempotent
        f.site_del(0x1000_0090);
        // Another thread mid-ioctl, as far as the drainer can tell.
        let held = f.reg.lock().unwrap();
        let (tx, rx) = mpsc::channel();
        let g = Arc::clone(&f);
        std::thread::spawn(move || {
            let _ = tx.send(g.status());
        });
        let line = rx.recv_timeout(Duration::from_secs(5));
        drop(held);
        let line = line.expect("status() waited on the registry lock");
        assert!(line.contains(" sites=1 "), "{line}");
        assert_eq!(
            f.sites(),
            vec![0x2000_0090],
            "the count agrees with the set"
        );
    }

    #[test]
    fn off_until_enabled_and_then_every_site_gets_every_token() {
        let f = DbFast::new(16, kick()).unwrap();
        assert_eq!(f.register(1, 0x1), RegOutcome::Off);
        let kvm = Arc::new(FakeKvm::default());
        assert!(f.enable(Box::new(Arc::clone(&kvm))));
        assert!(!f.enable(Box::new(Arc::clone(&kvm))), "one verb");
        // A registration before any site: live, placed nowhere (its doorbells trap).
        let r = f.register(1, 0x10001);
        assert!(matches!(
            r,
            RegOutcome::Registered {
                placed: 0,
                sites: 0,
                ..
            }
        ));
        f.site_add(0x1000_0090);
        f.site_add(0x1000_0090); // idempotent
        assert!(matches!(
            f.register(2, 0x20002),
            RegOutcome::Registered {
                placed: 1,
                sites: 1,
                ..
            }
        ));
        f.site_add(0x2000_0090);
        assert_eq!(kvm.held.lock().unwrap().len(), 4, "2 tokens x 2 sites");
        assert_eq!(f.live(), (2, 4));
        f.site_del(0x1000_0090);
        assert_eq!(f.live(), (2, 2));
        let s = Arc::new(Rec::default());
        let f = Arc::new(f);
        let d = Drainer::start(&f, &s);
        assert!(f.deregister(1).is_some());
        assert!(f.deregister(1).is_none(), "gone");
        assert_eq!(
            *kvm.held.lock().unwrap(),
            BTreeSet::from([(0x2000_0090, 0x20002)])
        );
        assert!(f.deregister(2).is_some());
        assert!(kvm.held.lock().unwrap().is_empty());
        assert_eq!(f.live(), (0, 0));
        drop(d);
        assert_eq!(pump(&f, &*s), 0);
        assert_eq!(f.counters.deassign_failed.load(Ordering::Relaxed), 0);
    }

    /// ★ The budget and a kernel refusal both leave the token TRAPPED — named, counted, and the
    /// placements that did land still work.
    #[test]
    fn budget_and_kernel_refusals_leave_the_token_trapped_and_are_counted() {
        let f = Arc::new(DbFast::new(3, kick()).unwrap());
        let kvm = Arc::new(FakeKvm {
            refuse_errno: ENOSPC,
            ..FakeKvm::default()
        });
        f.enable(Box::new(Arc::clone(&kvm)));
        let _d = Drainer::start(&f, &Arc::new(Rec::default()));
        f.site_add(0x90);
        for i in 0..5 {
            f.register(i, 0x100 + i);
        }
        assert_eq!(f.counters.refused_budget.load(Ordering::Relaxed), 2);
        assert_eq!(kvm.held.lock().unwrap().len(), 3);
        // The kernel now refuses anything past 2 of its own registrations.
        *kvm.refuse_after.lock().unwrap() = Some(2);
        assert!(f.deregister(0).is_some());
        f.register(9, 0x109); // under OUR budget again, but the kernel says ENOSPC
        f.register(10, 0x10A); // the same
        assert_eq!(f.counters.refused_enospc.load(Ordering::Relaxed), 2);
        assert_eq!(f.live().1, 2, "only the placements that landed are held");
        // Deregistering a refused token touches no KVM state and still acknowledges.
        let l = f.deregister(9).expect("registered, placed nowhere");
        assert_eq!((l.sites, l.refused), (0, 1));
        assert_eq!(f.counters.deassign_failed.load(Ordering::Relaxed), 0);
    }

    /// ★ Drain-before-act, the final drain, and the ledger: a count present at removal is
    /// delivered once, BEFORE the acknowledgement, and never again.
    #[test]
    fn a_count_pending_at_removal_is_delivered_before_the_ack_and_never_after() {
        let f = Arc::new(DbFast::new(8, kick()).unwrap());
        f.enable(Box::new(Arc::new(FakeKvm::default())));
        f.site_add(0x90);
        let s = Arc::new(Rec::default());
        f.register(7, 0x0007);
        let e = efd_of(&f, 7);
        e.signal().unwrap();
        e.signal().unwrap();
        assert_eq!(pump(&f, &*s), 1, "two stores, one delivery");
        e.signal().unwrap(); // lands, never serviced by a poll before the removal
        // The drainer runs on another thread; deregister blocks for its ack.
        let d = Drainer::start(&f, &s);
        let l = f.deregister(7).expect("acked");
        drop(d);
        assert_eq!(
            (l.doorbells, l.wakes, l.forwarded),
            (3, 2, 3),
            "all three stores reached the host, in two deliveries"
        );
        assert_eq!(s.got.lock().unwrap().len(), 2);
        e.signal().unwrap(); // a store after removal (the kernel would not make it) is never read
        assert_eq!(pump(&f, &*s), 0);
        assert_eq!(s.got.lock().unwrap().len(), 2);
    }

    /// A drainer thread for a test: waits on the fast path's poller and delivers, until dropped.
    struct Drainer {
        stop: Arc<AtomicBool>,
        h: Option<std::thread::JoinHandle<()>>,
    }
    impl Drainer {
        fn start(f: &Arc<DbFast>, s: &Arc<Rec>) -> Drainer {
            let stop = Arc::new(AtomicBool::new(false));
            let (f, s, st) = (Arc::clone(f), Arc::clone(s), Arc::clone(&stop));
            let h = std::thread::spawn(move || {
                while !st.load(Ordering::Acquire) {
                    let mut r = ReadyTokens::new();
                    let _ = f.poller().wait(&mut r, PollTimeout::Millis(2));
                    f.on_ready(&r, Instant::now(), &*s);
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

    /// ★ Generation safety: a re-born token has a NEW eventfd; the old one's pending count is
    /// delivered (to whatever the token is at that instant) before its removal completes, and a
    /// count on the new one is never attributed to the old registration's ledger.
    #[test]
    fn a_reborn_token_never_inherits_the_old_registrations_counts() {
        let f = Arc::new(DbFast::new(8, kick()).unwrap());
        f.enable(Box::new(Arc::new(FakeKvm::default())));
        f.site_add(0x90);
        let s = Arc::new(Rec::default());
        let _d = Drainer::start(&f, &s);
        f.register(3, 0x3);
        let old = efd_of(&f, 3);
        s.dead.lock().unwrap().insert(3); // the free statement retired the token word …
        old.signal().unwrap(); // … and a store landed in the old eventfd before the deassign
        let l = f.deregister(3).expect("acked");
        assert_eq!(
            (l.doorbells, l.absorbed),
            (1, 1),
            "delivered to the dead token: absorbed"
        );
        s.dead.lock().unwrap().remove(&3); // re-born
        f.register(3, 0x3);
        let new = efd_of(&f, 3);
        assert!(!Arc::ptr_eq(&old, &new), "a new eventfd per registration");
        new.signal().unwrap();
        // The kernel cannot signal the old eventfd after its deassign; even if something did, it
        // is no longer watched or listed, so nothing reads it.
        old.signal().unwrap();
        let l2 = f.deregister(3).expect("acked");
        assert_eq!(l2.doorbells, 1, "the new registration's own count only");
        assert_eq!(s.got.lock().unwrap().len(), 2, "one old + one new delivery");
        assert_eq!(
            old.drain().unwrap(),
            1,
            "the old eventfd's stray count was never read"
        );
    }

    /// ⊘ A birth over a live registration (its free never came): the old registration is removed
    /// and its pending count DISCARDED — it predates the token word the new birth installed, so
    /// delivering it would ring the new generation for an old store.
    #[test]
    fn a_registration_replaced_under_a_live_token_never_delivers_its_old_count() {
        let f = Arc::new(DbFast::new(8, kick()).unwrap());
        f.enable(Box::new(Arc::new(FakeKvm::default())));
        f.site_add(0x90);
        let s = Arc::new(Rec::default());
        f.register(4, 0x4);
        let old = efd_of(&f, 4);
        old.signal().unwrap();
        f.register(4, 0x4); // no deregister in between
        assert_eq!(f.counters.double_register.load(Ordering::Relaxed), 1);
        assert_eq!(
            pump(&f, &*s),
            0,
            "the old count is discarded, the new fd is quiet"
        );
        assert!(s.got.lock().unwrap().is_empty());
        efd_of(&f, 4).signal().unwrap();
        assert_eq!(pump(&f, &*s), 1, "the new registration delivers");
        assert_eq!(f.live(), (1, 1));
    }

    /// ★ FALSIFIER of the doorbell-servicing stall (D): 100 tokens ring continuously (every delivery
    /// re-arms its eventfd, as a guest storing in a loop does). The old `service_ready` repeated
    /// while a poll came back full and never returned; now one call is one bounded batch, and the
    /// rest is picked up by the next call (the drainer's loop interleaves its other work between).
    #[test]
    fn service_ready_is_one_bounded_batch_even_while_the_guest_keeps_ringing() {
        struct Rering(Arc<DbFast>);
        impl Sink for Rering {
            fn deliver(&self, _idx: u32, _value: u32, tag: u64) -> Delivered {
                if let Some(e) = self.0.efd_for_bench(tag) {
                    let _ = e.signal(); // the guest rings again at once
                }
                Delivered::Rang { reached: true }
            }
        }
        let st = Stall::leak();
        let f = Arc::new(DbFast::new(1 << 20, kick()).unwrap());
        f.attach_stall(st);
        f.enable(Box::new(Arc::new(FakeKvm::default())));
        f.site_add(0x90);
        for i in 0..100u32 {
            f.register(i, 0x1000 + i);
            efd_of(&f, i).signal().unwrap();
        }
        let (tx, rx) = mpsc::channel();
        let g = Arc::clone(&f);
        std::thread::spawn(move || {
            let sink = Rering(Arc::clone(&g));
            let first = g.service_ready(&sink);
            let second = g.service_ready(&sink);
            let _ = tx.send((first, second));
        });
        let (first, second) = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("service_ready looped while the guest kept ringing");
        assert_eq!(first, MAX_READY_BATCH, "100 ready: one full batch, no more");
        assert!(second > 0, "the rest is served by the next call");
        assert_eq!(
            f.counters.polls.load(Ordering::Relaxed),
            2,
            "one poll per call"
        );
        assert_eq!(st.doorbell_batch_full.load(Ordering::Relaxed), 2);
    }

    /// ★ The drainer's contract (D): with 100 tokens ringing continuously AND a privileged register
    /// write queued, `drainer_pass` applies the write — after at most ONE doorbell batch — and
    /// returns. (`apply_register` sweeps doorbells before it applies, exactly as the device's does.)
    #[test]
    fn a_privileged_write_is_applied_within_one_doorbell_batch_while_the_guest_keeps_ringing() {
        use kf_core::{HostOps, HostSlice, Plane, Step, Translatable, Vmm};
        use kf_trap::RegWrite;

        struct Rering(Arc<DbFast>, Arc<Mutex<Vec<&'static str>>>);
        impl Sink for Rering {
            fn deliver(&self, _idx: u32, _value: u32, tag: u64) -> Delivered {
                self.1.lock().unwrap().push("doorbell");
                if let Some(e) = self.0.efd_for_bench(tag) {
                    let _ = e.signal();
                }
                Delivered::Rang { reached: true }
            }
        }
        struct Dev(Arc<DbFast>, Rering, Arc<Mutex<Vec<&'static str>>>);
        impl HostOps for Dev {
            fn ring_host(&self, _: u32) {}
            fn run_translated(&self, _: u32, _: u64) -> bool {
                false
            }
            fn run_emulated(&self, _: u32, _: u64) {}
            fn apply_register(&self, _: u8, _: u32, _: u64, _: u8) {
                self.0.service_ready(&self.1);
                self.2.lock().unwrap().push("APPLIED");
            }
            fn operands_translatable(&self, _: u32, _: u64) -> Translatable {
                Translatable::Yes
            }
            fn forge_completion(&self, _: u32) {}
            fn refuse_and_poison(&self, _: u32) {}
            fn fault_channel(&self, _: u32) {}
            fn map_guest_slice(&self, _: HostSlice) {}
            fn teardown_step(&self, _: Step) {}
        }

        let f = Arc::new(DbFast::new(1 << 20, kick()).unwrap());
        f.enable(Box::new(Arc::new(FakeKvm::default())));
        f.site_add(0x90);
        for i in 0..100u32 {
            f.register(i, 0x1000 + i);
            efd_of(&f, i).signal().unwrap();
        }
        let order = Arc::new(Mutex::new(Vec::new()));
        let dev = Dev(
            Arc::clone(&f),
            Rering(Arc::clone(&f), Arc::clone(&order)),
            Arc::clone(&order),
        );
        let vmm: &'static Vmm = Box::leak(Box::new(Vmm::new()));
        let plane = Plane::new(vmm, 16, 0xFFF);
        plane.ring.push(RegWrite {
            bar: 0,
            offset: 0x1234,
            value: 1,
            width: 4,
        });
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let n = plane.drainer_pass(&dev, 256);
            let _ = tx.send(n);
        });
        let n = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the drainer pass never returned: doorbell servicing starved the write");
        assert_eq!(n, 1, "the privileged write was applied");
        let o = order.lock().unwrap();
        let before = o.iter().take_while(|e| **e == "doorbell").count();
        assert_eq!(o.last(), Some(&"APPLIED"));
        assert_eq!(
            before, MAX_READY_BATCH,
            "exactly one batch preceded the write"
        );
    }

    #[test]
    fn the_histogram_reports_percentiles_within_a_bucket() {
        let h = Hist::default();
        for i in 1..=1000u64 {
            h.add(i * 1000);
        }
        let p50 = h.pct(50.0);
        let p99 = h.pct(99.0);
        assert!((450_000..=560_000).contains(&p50), "{p50}");
        assert!((950_000..=1_000_000).contains(&p99), "{p99}");
        assert_eq!(Hist::default().pct(50.0), 0);
    }
}
