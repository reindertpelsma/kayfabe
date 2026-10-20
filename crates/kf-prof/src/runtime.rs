//! Opt-in runtime. Setup owns all allocation; publication writes bounded private
//! atomic storage and its mmap output mirror. The mapped file is never read back.
use crate::{
    clock::Calibration,
    histogram::Histogram,
    layout::*,
    ledger::{Ledger, Token},
    publication::{WordStorage, read_record, write_record},
    tables::{Kind, Site, State},
    thread::{SpanToken, ThreadState},
};
use kf_linux_raw::{Backing, CachePolicy, HostOffset, HostPageSize, VolatileRegion, prof_clock};
use serde_json::{Value, json};
use std::{
    boxed::Box,
    cell::RefCell,
    format,
    fs::{File, OpenOptions},
    io::{self, Write},
    marker::PhantomData,
    os::fd::AsFd,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    rc::Rc,
    string::String,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering, fence},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
    vec::Vec,
};

const KINDS: usize = Kind::ALL.len();
const LEDGER_SLOTS: usize = KINDS * 16;
const SOURCES: usize = 4096;
const SOURCE_BYTES: usize = 256;

static ON: AtomicBool = AtomicBool::new(false);
static OWNERS: AtomicU64 = AtomicU64::new(1);
std::thread_local! {static LOCAL:RefCell<Option<Context>>=const {RefCell::new(None)};}
fn error(s: &str) -> io::Error {
    io::Error::other(s)
}
fn raw_error(e: impl core::fmt::Debug) -> io::Error {
    io::Error::other(format!("{e:?}"))
}
struct Mirror {
    map: VolatileRegion,
    private: Box<[AtomicU64]>,
}
impl WordStorage for Mirror {
    fn len(&self) -> usize {
        self.private.len()
    }
    fn load(&self, i: usize, o: Ordering) -> u64 {
        self.private.get(i).map_or(0, |w| w.load(o))
    }
    fn store(&self, i: usize, v: u64, o: Ordering) {
        if let Some(w) = self.private.get(i) {
            if matches!(o, Ordering::Release | Ordering::SeqCst) {
                fence(Ordering::Release);
            }
            let _ = self.map.store_u64(HostOffset::new((i as u64) * 8), v);
            w.store(v, o);
        }
    }
}
struct Engine {
    words: Mirror,
    clock: Calibration,
    diagnostic_clock: bool,
    owner: u64,
    claimed: [AtomicBool; THREADS],
    thread_count: AtomicU64,
    source_count: AtomicU64,
    source_dropped: AtomicU64,
    stop: AtomicBool,
    origin: prof_clock::ClockPair,
    #[cfg(feature = "prof-inject")]
    delay_ms: AtomicU64,
    #[cfg(feature = "prof-inject")]
    paused: AtomicBool,
}
impl Engine {
    fn now(&self) -> u64 {
        #[cfg(feature = "prof-inject")]
        if self.diagnostic_clock {
            return prof_clock::monotonic_ns().unwrap_or(self.origin.monotonic_ns);
        }
        prof_clock::counter()
            .and_then(|t| self.clock.to_ns(t).ok())
            .unwrap_or(self.origin.monotonic_ns)
    }
    fn snapshot(&self) -> Value {
        let now = self.now();
        let mut threads = Vec::new();
        let mut oldest = Vec::new();
        let mut kinds = [0u64; KINDS];
        let mut expected_threads = 0usize;
        for slot in 0..THREADS {
            if !self.claimed[slot].load(Ordering::Acquire) {
                continue;
            }
            expected_threads += 1;
            let base = (THREAD_BASE + slot * THREAD_BYTES) / 8;
            let seq_before = self.words.load(base, Ordering::Acquire);
            let old_threads = threads.len();
            let old_entries = oldest.len();
            let old_kinds = kinds;
            let Ok(v) = read_record::<13>(&self.words, base) else {
                continue;
            };
            if v[0] == 0 {
                continue;
            }
            let mut states = Vec::new();
            let mut sites = Vec::new();
            for (offset, n, out, key) in [
                (69632, 32, &mut states, "state"),
                (90112, 64, &mut sites, "site"),
            ] {
                for index in 0..n {
                    if let Ok(h) = read_record::<79>(&self.words, base + (offset + index * 640) / 8)
                    {
                        let active = key == "state" && index as u64 == v[2];
                        if h[0] == 0 && h[5] == 0 && !active {
                            continue;
                        }
                        let mut row = json!({"count":h[0],"total_ns":h[1],"max_ns":h[2],"max_arg":h[3],"flags":h[4],"exclusive_ns":h[5],"histogram":&h[7..72],"scope":"cumulative"});
                        row[key] = json!(index);
                        out.push(row);
                    }
                }
            }
            threads.push(json!({"slot":slot,"tid":v[0],"role":v[1],"state":v[2],"arg":v[3],"since_ns":v[4],"site":v[5],"dropped":v[6],"states":states,"sites":sites}));
            for kind in Kind::ALL {
                for i in 0..16 {
                    let off = base + (172032 + (*kind as usize * 16 + i) * 64) / 8;
                    if let Ok(e) = read_record::<7>(&self.words, off)
                        && e[5] == 1
                    {
                        kinds[*kind as usize] += 1;
                        let age = now.saturating_sub(e[3]);
                        oldest.push(json!({"thread":slot,"kind":*kind as u16,"generation":e[1],"id":e[2],"birth_ns":e[3],"age_ns":age,"overdue":age>kind.bound_ns()}));
                    }
                }
            }
            let observed = self.now();
            fence(Ordering::Acquire);
            if seq_before != self.words.load(base, Ordering::Relaxed) {
                threads.truncate(old_threads);
                oldest.truncate(old_entries);
                kinds = old_kinds;
            } else {
                let t = &mut threads[old_threads];
                t["observed_ns"] = json!(observed);
                t["birth_ns"] = json!(v[11]);
                t["retired_ns"] = json!(v[12]);
                t["active"] = json!(v[12] == 0);
                let residence_until = if v[12] == 0 { observed } else { v[12] };
                if let Some(states) = t["states"].as_array_mut() {
                    for state in states {
                        if state["state"].as_u64() == Some(v[2]) {
                            state["exclusive_ns"] = json!(
                                state["exclusive_ns"]
                                    .as_u64()
                                    .unwrap_or(0)
                                    .saturating_add(residence_until.saturating_sub(v[10]))
                            );
                        }
                    }
                }
                for entry in &mut oldest[old_entries..] {
                    let age =
                        observed.saturating_sub(entry["birth_ns"].as_u64().unwrap_or(observed));
                    entry["age_ns"] = json!(age);
                    let k = entry["kind"].as_u64().unwrap_or(KINDS as u64) as usize;
                    entry["overdue"] =
                        json!(Kind::ALL.get(k).is_some_and(|kind| age > kind.bound_ns()));
                }
            }
        }
        let mut relays = Vec::new();
        let registered = self
            .source_count
            .load(Ordering::Acquire)
            .min(SOURCES as u64);
        for index in 0..registered as usize {
            let base = (LEDGER_BASE + index * SOURCE_BYTES) / 8;
            if let Ok(v) = read_record::<31>(&self.words, base) {
                if v[0] != 1 {
                    continue;
                }
                if v[9] != 0 && v[23] != 0 {
                    kinds[Kind::RelayDrain as usize] =
                        kinds[Kind::RelayDrain as usize].saturating_add(1);
                    let age = now.saturating_sub(v[27]);
                    oldest.push(json!({"source":index,"kind":Kind::RelayDrain as u16,"generation":v[1],"id":v[2],"birth_ns":v[27],"age_ns":age,"observed_ns":v[4],"last_known":true,"overdue":age>Kind::RelayDrain.bound_ns()}));
                }
                relays.push(relay_row(index, &v));
            }
        }
        oldest.sort_by_key(|e| core::cmp::Reverse(e["age_ns"].as_u64().unwrap_or(0)));
        oldest.truncate(16);
        let now = self.now();
        json!({"clock_mode":if self.diagnostic_clock {"diagnostic_os_clock"} else {"calibrated_tsc"},"schema":1,"start_ns":self.origin.monotonic_ns,"end_ns":now,"threads":threads,
            "relays":relays,"relay_registered":registered,"relay_dropped":self.source_dropped.load(Ordering::Relaxed),"ledger":{"counts":kinds,"oldest":oldest},"counters":{"rmcontrol_seen":null,"host_rm_calls":null,"display_new_frames":null,"broker_presented_new_frames":null},
            "coverage":{"registered_threads_only":true,"production_sources":false,"thread_samples_expected":expected_threads,"thread_samples_complete":threads.len()==expected_threads},"aggregator_lag_ns":0})
    }
}
struct Context {
    engine: Arc<Engine>,
    slot: usize,
    tid: u64,
    role: u64,
    born: u64,
    retired: u64,
    state: ThreadState,
    seq: u64,
    state_seq: [u64; 32],
    site_seq: [u64; 64],
    sites: [Histogram; 64],
    ledger: Ledger<KINDS, 16>,
    entries: [[u64; 7]; LEDGER_SLOTS],
    entry_seq: [u64; LEDGER_SLOTS],
}
impl Context {
    fn base(&self) -> usize {
        (THREAD_BASE + self.slot * THREAD_BYTES) / 8
    }
    fn publish(&mut self) {
        let v = self.state.snapshot();
        let payload = [
            self.tid,
            self.role,
            v.state as u64,
            v.arg,
            v.enter_ns,
            v.site as u64,
            self.state.dropped().saturating_add(self.ledger.dropped()),
            0,
            0,
            1,
            self.state.segment_since_ns(),
            self.born,
            self.retired,
        ];
        let base = self.base();
        let _ = write_record(&self.engine.words, base, &payload, &mut self.seq);
    }
    fn begin(&self) {
        if let Some(even) = self.seq.checked_add(2) {
            self.engine
                .words
                .store(self.base(), even - 1, Ordering::Relaxed);
            fence(Ordering::Release);
        }
    }
    fn histogram(&mut self, state: State, site: Option<Site>, duration: Option<(u64, u64)>) {
        let mut h = *self.state.histogram(state);
        let mut exclusive = self.state.exclusive_ns(state);
        let (offset, index) = if let Some(site) = site {
            let index = site as usize;
            if let Some((ns, arg)) = duration {
                self.sites[index].record(ns, arg);
            }
            h = self.sites[index];
            exclusive = 0;
            (90112, index)
        } else {
            (69632, state as usize)
        };
        // Only six metadata words and the changed bucket need stores. Existing
        // buckets remain producer-owned and do not require any shared RMW.
        let base = self.base() + (offset + index * 640) / 8;
        let seq = if site.is_some() {
            &mut self.site_seq[index]
        } else {
            &mut self.state_seq[index]
        };
        let Some(even) = seq.checked_add(2) else {
            return;
        };
        self.engine.words.store(base, even - 1, Ordering::Relaxed);
        fence(Ordering::Release);
        for (i, v) in [
            h.count,
            h.total_ns,
            h.max_ns,
            h.max_arg,
            h.saturated as u64,
            exclusive,
        ]
        .into_iter()
        .enumerate()
        {
            self.engine.words.store(base + 1 + i, v, Ordering::Relaxed);
        }
        if let Some((ns, _)) = duration {
            let b = Histogram::bucket(ns);
            self.engine
                .words
                .store(base + 8 + b, h.buckets[b], Ordering::Relaxed);
        }
        self.engine.words.store(base, even, Ordering::Release);
        *seq = even;
    }
    fn ledger_publish(&mut self, index: usize) {
        let base = self.base() + (172032 + index * 64) / 8;
        let _ = write_record(
            &self.engine.words,
            base,
            &self.entries[index],
            &mut self.entry_seq[index],
        );
    }
}
/// A span is thread-affine; moving it to another thread cannot silently misclose.
pub struct SpanGuard {
    token: Option<SpanToken>,
    owner: u64,
    site: Site,
    state: State,
    birth: u64,
    arg: u64,
    _local: PhantomData<Rc<()>>,
}
impl SpanGuard {
    fn empty() -> Self {
        Self {
            token: None,
            owner: 0,
            site: Site::Unknown,
            state: State::Unknown,
            birth: 0,
            arg: 0,
            _local: PhantomData,
        }
    }
}
/// Enter an already-registered thread's state. Disabled runtime is one relaxed load.
#[inline]
pub fn site(site: Site, state: State, arg: u64) -> SpanGuard {
    if !ON.load(Ordering::Relaxed) {
        return SpanGuard::empty();
    }
    LOCAL.with(|local| {
        let Ok(mut local) = local.try_borrow_mut() else {
            return SpanGuard::empty();
        };
        let Some(c) = local.as_mut() else {
            return SpanGuard::empty();
        };
        let now = c.engine.now();
        let previous = c.state.snapshot().state;
        let Ok(token) = c.state.enter(state, site, arg, now) else {
            c.publish();
            return SpanGuard::empty();
        };
        c.begin();
        c.histogram(previous, None, None);
        #[cfg(feature = "prof-inject")]
        {
            let ms = c.engine.delay_ms.swap(0, Ordering::AcqRel);
            if ms > 0 {
                c.engine.paused.store(true, Ordering::Release);
                thread::sleep(Duration::from_millis(ms));
                c.engine.paused.store(false, Ordering::Release);
            }
        }
        c.publish();
        SpanGuard {
            token: Some(token),
            owner: c.engine.owner,
            site,
            state,
            birth: now,
            arg,
            _local: PhantomData,
        }
    })
}
impl Drop for SpanGuard {
    fn drop(&mut self) {
        let Some(token) = self.token.take() else {
            return;
        };
        LOCAL.with(|local| {
            if let Ok(mut local) = local.try_borrow_mut()
                && let Some(c) = local.as_mut()
            {
                if c.engine.owner != self.owner {
                    return;
                }
                let now = c.engine.now();
                if c.state.leave(token, now) {
                    c.begin();
                    let elapsed = now.saturating_sub(self.birth);
                    c.histogram(self.state, None, Some((elapsed, self.arg)));
                    c.histogram(self.state, Some(self.site), Some((elapsed, self.arg)));
                    c.publish();
                }
            }
        });
    }
}
/// Open a bounded obligation on the registered calling thread.
#[inline]
pub fn ledger_open(kind: Kind, id: u64) -> Option<Token> {
    if !ON.load(Ordering::Relaxed) {
        return None;
    }
    LOCAL.with(|local| {
        let mut local = local.try_borrow_mut().ok()?;
        let c = local.as_mut()?;
        let now = c.engine.now();
        let Some(token) = c.ledger.open(kind as usize, id, now) else {
            c.publish();
            return None;
        };
        let index = token.kind() * 16 + token.slot();
        c.begin();
        c.entries[index] = [kind as u64, token.generation(), id, now, 0, 1, 0];
        c.ledger_publish(index);
        c.publish();
        Some(token)
    })
}
/// Close only the exact producer/kind/slot/generation that opened this obligation.
#[inline]
pub fn ledger_close(token: Token) -> bool {
    if !ON.load(Ordering::Relaxed) {
        return false;
    }
    LOCAL.with(|local| {
        let Ok(mut local) = local.try_borrow_mut() else {
            return false;
        };
        let Some(c) = local.as_mut() else {
            return false;
        };
        let now = c.engine.now();
        if !c.ledger.close(token, now) {
            return false;
        }
        let index = token.kind() * 16 + token.slot();
        c.begin();
        c.entries[index][5] = 0;
        c.entries[index][6] = now;
        c.ledger_publish(index);
        c.publish();
        true
    })
}
/// Registered lifetime, created/dropped outside any hot operation.
pub struct Binding {
    owner: u64,
    _local: PhantomData<Rc<()>>,
}
impl Drop for Binding {
    fn drop(&mut self) {
        LOCAL.with(|l| {
            let mut l = l.borrow_mut();
            if l.as_ref().is_some_and(|c| c.engine.owner == self.owner) {
                if let Some(c) = l.as_mut() {
                    c.retired = c.engine.now();
                    c.publish();
                }
                *l = None;
            }
        });
    }
}
/// Owns output and the dedicated aggregator. No initialization is performed by span calls.
pub struct Session {
    engine: Arc<Engine>,
    thread: Option<JoinHandle<io::Result<()>>>,
}
impl Session {
    /// Create new owner-only files in an existing trusted directory; never overwrite.
    pub fn create(path: &Path) -> io::Result<Self> {
        Self::create_inner(path, false)
    }
    /// Test/diagnostic-only OS clock. Marks its file; absent from normal prof builds.
    #[cfg(feature = "prof-inject")]
    pub fn create_diagnostic(path: &Path) -> io::Result<Self> {
        Self::create_inner(path, true)
    }
    fn create_inner(path: &Path, diagnostic_clock: bool) -> io::Result<Self> {
        Self::create_paths(
            &path.join("prof.mmap"),
            &path.join("prof.jsonl"),
            diagnostic_clock,
        )
    }
    /// Create an explicit mmap or JSONL filename with its adjacent companion file.
    pub fn create_file(path: &Path) -> io::Result<Self> {
        let (map, json) = output_paths(path);
        Self::create_paths(&map, &json, false)
    }
    /// Explicit filename variant of the marked diagnostic clock fixture.
    #[cfg(feature = "prof-inject")]
    pub fn create_diagnostic_file(path: &Path) -> io::Result<Self> {
        let (map, json) = output_paths(path);
        Self::create_paths(&map, &json, true)
    }
    fn create_paths(map_path: &Path, json_path: &Path, diagnostic_clock: bool) -> io::Result<Self> {
        if !diagnostic_clock && !prof_clock::counter_available() {
            return Err(error("invariant TSC unavailable"));
        }
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(map_path)?;
        let mut json_file = JsonSink::create(json_path, 32 * 1024 * 1024)?;
        let zero = [0u8; 65536];
        for _ in 0..FILE_BYTES / zero.len() {
            file.write_all(&zero)?;
        }
        let map = VolatileRegion::map(
            Backing::SharedFile {
                fd: file.as_fd(),
                offset: 0,
            },
            FILE_BYTES as u64,
            CachePolicy::WriteBack,
            HostPageSize::query(),
        )
        .map_err(raw_error)?;
        let private = (0..FILE_BYTES / 8)
            .map(|_| AtomicU64::new(0))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let pair = || -> io::Result<prof_clock::ClockPair> {
            if diagnostic_clock {
                let n = prof_clock::monotonic_ns()
                    .ok_or_else(|| error("diagnostic clock unavailable"))?;
                return Ok(prof_clock::ClockPair {
                    before: n,
                    after: n,
                    monotonic_ns: n,
                    raw_ns: 0,
                });
            }
            prof_clock::pair().ok_or_else(|| error("clock unavailable"))
        };
        let a = pair()?;
        thread::sleep(Duration::from_millis(10));
        let b = pair()?;
        let ca = a.before + (a.after - a.before) / 2;
        let cb = b.before + (b.after - b.before) / 2;
        let bracket = u128::from((a.after - a.before).max(b.after - b.before))
            * u128::from(b.monotonic_ns - a.monotonic_ns)
            / u128::from(cb - ca)
            + 1;
        let clock = Calibration::from_pair(
            ca,
            a.monotonic_ns,
            cb,
            b.monotonic_ns,
            u64::try_from(bracket).map_err(raw_error)?,
        )
        .map_err(raw_error)?;
        let engine = Arc::new(Engine {
            words: Mirror { map, private },
            clock,
            diagnostic_clock,
            owner: OWNERS.fetch_add(1, Ordering::Relaxed),
            claimed: core::array::from_fn(|_| AtomicBool::new(false)),
            thread_count: AtomicU64::new(0),
            source_count: AtomicU64::new(0),
            source_dropped: AtomicU64::new(0),
            stop: AtomicBool::new(false),
            origin: a,
            #[cfg(feature = "prof-inject")]
            delay_ms: AtomicU64::new(0),
            #[cfg(feature = "prof-inject")]
            paused: AtomicBool::new(false),
        });
        let mut revision = [0; 40];
        if let Some(text) = option_env!("KF_PROF_BUILD_REV").filter(|s| {
            s.len() == 40
                && s.bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        }) {
            revision.copy_from_slice(text.as_bytes());
        }
        let header = Header {
            revision,
            pid: u64::from(std::process::id()),
            start_ns: a.monotonic_ns,
            flags: diagnostic_clock as u64,
        }
        .encode();
        for (i, b) in header.as_chunks::<8>().0.iter().enumerate() {
            engine
                .words
                .store(i, u64::from_le_bytes(*b), Ordering::Relaxed);
        }
        // Record the integer fit and both clock anchors; recalibration diagnostics
        // never alter an already emitted timestamp's conversion epoch.
        for (offset, v) in [
            (64, 1),
            (72, ca),
            (80, a.monotonic_ns),
            (88, a.raw_ns),
            (96, engine.clock.parts()[2]),
            (104, engine.clock.parts()[3]),
            (112, engine.clock.parts()[4]),
        ] {
            engine.words.store(offset / 8, v, Ordering::Relaxed);
        }
        let e = Arc::clone(&engine);
        let thread = thread::Builder::new()
            .name(String::from("kf-prof"))
            .spawn(move || aggregate(&e, &mut json_file))?;
        ON.store(true, Ordering::Relaxed);
        Ok(Self {
            engine,
            thread: Some(thread),
        })
    }
    /// Register/bind off the hot path. Allocation and the one-time claim are setup work.
    pub fn bind_current(&self, role: u64) -> io::Result<Binding> {
        if LOCAL.with(|l| l.borrow().is_some()) {
            return Err(error("thread already registered"));
        }
        let slot = self
            .engine
            .claimed
            .iter()
            .position(|c| {
                c.compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
                    .is_ok()
            })
            .ok_or_else(|| error("thread capacity exhausted"))?;
        let now = self.engine.now();
        let mut c = Context {
            engine: Arc::clone(&self.engine),
            slot,
            tid: prof_clock::thread_id(),
            role,
            born: now,
            retired: 0,
            state: ThreadState::new(now),
            seq: 0,
            state_seq: [0; 32],
            site_seq: [0; 64],
            sites: [Histogram::new(); 64],
            ledger: Ledger::new(self.engine.owner.wrapping_mul(64).wrapping_add(slot as u64)),
            entries: [[0; 7]; LEDGER_SLOTS],
            entry_seq: [0; LEDGER_SLOTS],
        };
        c.publish();
        self.engine
            .thread_count
            .fetch_max((slot + 1) as u64, Ordering::Release);
        LOCAL.with(|l| *l.borrow_mut() = Some(c));
        Ok(Binding {
            owner: self.engine.owner,
            _local: PhantomData,
        })
    }
    /// Reserve a source slot during channel birth, off every vCPU/drainer operation.
    pub fn register_relay(&self, key: u64) -> io::Result<RelayProbe> {
        let slot = self
            .engine
            .source_count
            .try_update(Ordering::AcqRel, Ordering::Relaxed, |n| {
                (n < SOURCES as u64).then(|| n + 1)
            });
        let slot = match slot {
            Ok(n) => n as usize,
            Err(_) => {
                let _ = self.engine.source_dropped.try_update(
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                    |n| Some(n.saturating_add(1)),
                );
                return Err(error("relay source capacity exhausted"));
            }
        };
        let mut probe = RelayProbe {
            engine: Arc::clone(&self.engine),
            slot,
            seq: 0,
            gauge: crate::relay::RelayGauge::new(slot as u64 + 1, key, self.engine.now()),
        };
        probe.publish();
        Ok(probe)
    }
    /// Arm a bounded mid-publication diagnostic delay, absent from normal prof builds.
    #[cfg(feature = "prof-inject")]
    pub fn inject_publication_delay_ms(&self, ms: u64) -> io::Result<()> {
        if ms > 1000 {
            return Err(error("diagnostic delay exceeds bound"));
        }
        self.engine.delay_ms.store(ms, Ordering::Release);
        Ok(())
    }
    /// Whether the armed publication fixture is currently paused.
    #[cfg(feature = "prof-inject")]
    pub fn publication_paused(&self) -> bool {
        self.engine.paused.load(Ordering::Acquire)
    }
    /// Read trusted in-process publication, not mutable file bytes.
    pub fn snapshot(&self) -> Value {
        self.engine.snapshot()
    }
    /// Stop and join the aggregator off every vCPU/drainer/production lock path.
    pub fn stop(mut self) -> io::Result<()> {
        self.finish()
    }
    fn finish(&mut self) -> io::Result<()> {
        self.engine.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            t.join().map_err(|_| error("aggregator panicked"))??;
        }
        Ok(())
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}
fn aggregate(e: &Engine, file: &mut JsonSink) -> io::Result<()> {
    let mut last = e.origin.monotonic_ns;
    let mut record = 0u64;
    let mut header_seq = 0u64;
    let mut elapsed = Instant::now();
    let mut sink_ok = true;
    let mut previous = (0..THREADS).map(|_| Prior::new()).collect::<Vec<_>>();
    loop {
        let stopping = e.stop.load(Ordering::Acquire);
        // Live sample only. Exact totals come from guard publication, never sample counts.
        for slot in 0..THREADS {
            if e.claimed[slot].load(Ordering::Acquire) {
                let _ = read_record::<7>(&e.words, (THREAD_BASE + slot * THREAD_BYTES) / 8);
            }
        }
        if elapsed.elapsed() >= Duration::from_secs(1) || stopping {
            let mut row = e.snapshot();
            let now = row["end_ns"].as_u64().unwrap_or(last);
            row["start_ns"] = json!(last);
            intervals(&mut row, &mut previous);
            row["aggregator_lag_ns"] =
                json!(now.saturating_sub(last).saturating_sub(1_000_000_000));
            row["json_sink_ok"] = json!(sink_ok);
            let mut body = bounded_record(&mut row)?;
            record += 1;
            let base = (RECORD_BASE + ((record - 1) as usize % RECORDS) * RECORD_BYTES) / 8;
            e.words.store(base, record * 2 - 1, Ordering::Relaxed);
            fence(Ordering::Release);
            let meta = [last, now, body.len() as u64, record, 0, 0, 0];
            for (i, v) in meta.into_iter().enumerate() {
                e.words.store(base + 1 + i, v, Ordering::Relaxed);
            }
            for (i, b) in body.chunks(8).enumerate() {
                let mut w = [0; 8];
                w[..b.len()].copy_from_slice(b);
                e.words
                    .store(base + 8 + i, u64::from_le_bytes(w), Ordering::Relaxed);
            }
            e.words.store(base, record * 2, Ordering::Release);
            if sink_ok {
                body.push(b'\n');
                if file.append(&body).is_err() {
                    sink_ok = false;
                }
            }
            header_seq += 2;
            e.words.store(24 / 8, header_seq - 1, Ordering::Relaxed);
            fence(Ordering::Release);
            for (offset, v) in [
                (56, now),
                (168, e.thread_count.load(Ordering::Acquire)),
                (176, record),
                (200, e.source_count.load(Ordering::Acquire)),
                (208, e.source_dropped.load(Ordering::Relaxed)),
            ] {
                e.words.store(offset / 8, v, Ordering::Relaxed);
            }
            if !e.diagnostic_clock
                && let Some(p) = prof_clock::pair()
            {
                let c = p.before + (p.after - p.before) / 2;
                let converted = e.clock.to_ns(c).unwrap_or(p.monotonic_ns);
                e.words.store(
                    120 / 8,
                    converted.abs_diff(p.monotonic_ns),
                    Ordering::Relaxed,
                );
            }
            e.words.store(24 / 8, header_seq, Ordering::Release);
            last = now;
            elapsed = Instant::now();
        }
        if stopping {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}

/// Aggregator-only bounded JSONL sink. Two owned chunks, no hot-path I/O.
pub struct JsonSink {
    file: File,
    path: PathBuf,
    bytes: usize,
    limit: usize,
    previous_owned: bool,
}
impl JsonSink {
    /// Create a new output, with a positive limit no larger than 32 MiB.
    pub fn create(path: &Path, limit: usize) -> io::Result<Self> {
        if limit == 0 || limit > 32 * 1024 * 1024 {
            return Err(error("invalid diagnostic file limit"));
        }
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        Ok(Self {
            file,
            path: path.to_path_buf(),
            bytes: 0,
            limit,
            previous_owned: false,
        })
    }
    /// One buffered record append; rotation retains the preceding owned chunk.
    /// Refuse a foreign previous chunk instead of replacing or removing it.
    pub fn append(&mut self, line: &[u8]) -> io::Result<()> {
        if line.len() > self.limit || line.len() > RECORD_BYTES - 63 {
            return Err(error("diagnostic record limit"));
        }
        if self.bytes + line.len() > self.limit {
            let previous = self.path.with_extension("jsonl.previous");
            if self.previous_owned {
                std::fs::remove_file(&previous)?;
            } else if entry_exists(&previous)? {
                return Err(error("previous diagnostic file already exists"));
            }
            std::fs::rename(&self.path, &previous)?;
            self.previous_owned = true;
            self.file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&self.path)?;
            self.bytes = 0;
        }
        self.file.write_all(line)?;
        self.bytes += line.len();
        Ok(())
    }
}

fn entry_exists(path: &Path) -> io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

struct Prior {
    cut: u64,
    exclusive: [u64; 32],
    counts: [u64; 32],
    totals: [u64; 32],
    site_counts: [u64; 64],
    site_totals: [u64; 64],
}
impl Prior {
    fn new() -> Self {
        Self {
            cut: 0,
            exclusive: [0; 32],
            counts: [0; 32],
            totals: [0; 32],
            site_counts: [0; 64],
            site_totals: [0; 64],
        }
    }
}
fn intervals(row: &mut Value, previous: &mut [Prior]) {
    let Some(threads) = row["threads"].as_array_mut() else {
        return;
    };
    for t in threads {
        let Some(slot) = t["slot"].as_u64().and_then(|i| usize::try_from(i).ok()) else {
            continue;
        };
        let Some(p) = previous.get_mut(slot) else {
            continue;
        };
        let observed = t["observed_ns"].as_u64().unwrap_or(0);
        let born = t["birth_ns"].as_u64().unwrap_or(observed);
        let retired = t["retired_ns"].as_u64().unwrap_or(0);
        let until = if retired == 0 { observed } else { retired };
        let start = if p.cut == 0 { born } else { p.cut };
        let exposure = until.saturating_sub(start);
        t["exposure_start_ns"] = json!(start);
        t["exposure_ns"] = json!(exposure);
        p.cut = until;
        if let Some(states) = t["states"].as_array_mut() {
            for state in states {
                let Some(index) = state["state"]
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .filter(|&i| i < 32)
                else {
                    continue;
                };
                let ex = state["exclusive_ns"].as_u64().unwrap_or(0);
                let count = state["count"].as_u64().unwrap_or(0);
                let total = state["total_ns"].as_u64().unwrap_or(0);
                let delta = ex.checked_sub(p.exclusive[index]);
                state["interval_exclusive_ns"] = json!(delta);
                state["time_share"] = json!(
                    delta
                        .filter(|_| exposure > 0)
                        .map(|d| d as f64 / exposure as f64)
                );
                state["interval_count"] = json!(count.checked_sub(p.counts[index]));
                state["interval_total_ns"] = json!(total.checked_sub(p.totals[index]));
                p.exclusive[index] = ex;
                p.counts[index] = count;
                p.totals[index] = total;
            }
        }
        if let Some(sites) = t["sites"].as_array_mut() {
            for site in sites.iter_mut() {
                let Some(index) = site["site"]
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .filter(|&i| i < 64)
                else {
                    continue;
                };
                let count = site["count"].as_u64().unwrap_or(0);
                let total = site["total_ns"].as_u64().unwrap_or(0);
                site["interval_count"] = json!(count.checked_sub(p.site_counts[index]));
                site["interval_total_ns"] = json!(total.checked_sub(p.site_totals[index]));
                p.site_counts[index] = count;
                p.site_totals[index] = total;
            }
            sites.sort_by_key(|r| core::cmp::Reverse(r["interval_total_ns"].as_u64().unwrap_or(0)));
        }
    }
}

fn output_paths(path: &Path) -> (PathBuf, PathBuf) {
    if path.extension().is_some_and(|e| e == "jsonl") {
        (path.with_extension("mmap"), path.to_path_buf())
    } else {
        (path.to_path_buf(), path.with_extension("jsonl"))
    }
}
static GLOBAL: std::sync::OnceLock<Option<Session>> = std::sync::OnceLock::new();
/// Realize-time optional startup. Failure is diagnostic only and never prevents boot.
pub fn init_from_env() -> bool {
    GLOBAL
        .get_or_init(|| {
            let path = std::env::var_os("KF3_PROF_FILE")?;
            let path = PathBuf::from(path);
            let result = if path.is_dir() {
                Session::create(&path)
            } else {
                Session::create_file(&path)
            };
            match result {
                Ok(s) => Some(s),
                Err(e) => {
                    std::eprintln!("kf-prof unavailable: {e}");
                    None
                }
            }
        })
        .is_some()
}
/// Register a worker at its thread entry, outside all hot operations and locks.
pub fn bind_current_global(role: u64) -> Option<Binding> {
    GLOBAL.get()?.as_ref()?.bind_current(role).ok()
}

/// One relay's single-writer output. It can move from birth setup to its worker;
/// callers serialize observations through the relay's existing private exclusion.
pub struct RelayProbe {
    engine: Arc<Engine>,
    slot: usize,
    seq: u64,
    gauge: crate::relay::RelayGauge,
}
impl RelayProbe {
    fn publish(&mut self) {
        let _ = write_record(
            &self.engine.words,
            (LEDGER_BASE + self.slot * SOURCE_BYTES) / 8,
            self.gauge.words(),
            &mut self.seq,
        );
    }
    /// Publish cached observations only; no hardware reads or completion decisions.
    pub fn observe(&mut self, o: crate::relay::Observation) {
        if self.gauge.observe(self.engine.now(), o).is_ok() {
            self.publish();
        }
    }
}
fn relay_row(index: usize, v: &[u64; 31]) -> Value {
    let known = |mask, index| {
        if v[5] & mask != 0 {
            Some(v[index])
        } else {
            None
        }
    };
    let get = known(1, 6);
    let put = known(2, 7);
    let entries = v[24];
    let depth = get
        .zip(put)
        .filter(|&(g, p)| entries > 0 && g < entries && p < entries)
        .map(|(g, p)| (p + entries - g) % entries);
    json!({"source_slot":index,"generation":v[1],"key":v[2],"token":v[3],"observed_ns":v[4],"private_get":get,"private_put":put,"reference":known(4,8),"private_depth":depth,"wait":{"kind":v[9],"target":v[10],"birth_ns":v[27]},"doorbell_count":known(32,11),"last_doorbell_observed_ns":v[12],"wake_reason":v[13],"wake_observed_ns":v[14],"private_get_changed_ns":v[15],"guest_get":known(8,16),"guest_put":known(16,17),"guest_entries":v[18],"guest_observed_ns":v[19],"issued":v[20],"fetched":v[21],"markers_issued":v[22],"active":v[23]!=0,"private_entries":entries,"private_observed_ns":v[25],"private_put_observed_ns":v[26],"birth_ns":v[28]})
}
/// Bound the exact wire payload before publishing any byte of a circular record.
/// Full live source gauges remain in mmap when summaries omit rows.
pub fn bounded_record(row: &mut Value) -> io::Result<Vec<u8>> {
    if let Some(rows) = row["relays"].as_array_mut()
        && rows.len() > 64
    {
        let n = rows.len() - 64;
        rows.truncate(64);
        row["coverage"]["relay_rows_omitted"] = json!(n);
        row["coverage"]["payload_truncated"] = json!(true);
    }
    let mut body = serde_json::to_vec(row).map_err(raw_error)?;
    if body.len() > RECORD_BYTES - 64 {
        if let Some(threads) = row["threads"].as_array_mut() {
            for t in threads {
                t["states"] = json!([]);
                t["sites"] = json!([]);
            }
        }
        row["coverage"]["payload_truncated"] = json!(true);
        body = serde_json::to_vec(row).map_err(raw_error)?;
    }
    for _ in 0..128 {
        if body.len() <= RECORD_BYTES - 64 {
            return Ok(body);
        }
        if let Some(rows) = row["relays"].as_array_mut()
            && rows.pop().is_some()
        {
            let omitted = row["coverage"]["relay_rows_omitted"].as_u64().unwrap_or(0);
            row["coverage"]["relay_rows_omitted"] = json!(omitted + 1);
        } else if let Some(threads) = row["threads"].as_array_mut()
            && threads.pop().is_some()
        {
        } else {
            return Err(error("irreducible profiler record exceeds wire budget"));
        }
        body = serde_json::to_vec(row).map_err(raw_error)?;
    }
    Err(error("profiler record reduction bound exhausted"))
}
/// Channel-birth registration only; no lazy registration on a producer path.
pub fn register_relay_global(key: u64) -> Option<RelayProbe> {
    GLOBAL.get()?.as_ref()?.register_relay(key).ok()
}

/// Runtime-disabled instrumentation is one relaxed load.
#[inline(always)]
pub fn is_on() -> bool {
    ON.load(Ordering::Relaxed)
}
