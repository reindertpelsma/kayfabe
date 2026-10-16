//! MEASUREMENT ONLY (perf lifecycle campaign, 2026-10-16, scratch branch, never for merge).
//!
//! Counters for the Windows USER-ring relay, printed as `kf3: LIFECYCLE ...` lines when the process
//! has `KF3_LIFECYCLE_COUNT=1`: every pushbuffer / GP-entry snapshot by call site and by channel
//! lifecycle class, the bytes by backing (guest RAM vs the BAR1 store), the BAR1 copy count / bytes /
//! nanoseconds, GP entries with SYNC_WAIT, turns, triggers and ring growths. Default off: the only
//! cost then is one `OnceLock` read per call. Nothing here changes what the relay does, except the
//! experiment switch [`scan_off`] (`KF3_SCAN_OFF_AFTER_5080=1`), which the report names as such.
//!
//! No unsafe code, no host verbs, no locks a vCPU takes (the registry is read from the worker only).

use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

/// Which read: the 8-byte GP entry, the pushbuffer segment, or anything else (migration, trace, shadow).
#[derive(Clone, Copy, Debug)]
pub(crate) enum Site {
    Gp = 0,
    Pb = 1,
    Other = 2,
}
pub(crate) const SITES: [&str; 3] = ["gp_entry", "pushbuffer", "other"];
/// The channel's lifecycle class at the time of the read.
pub(crate) const CLASSES: [&str; 3] = ["pre5080", "post5080_nopend", "post5080_regpend"];
const LEN_BUCKETS: [&str; 8] = [
    "le16", "le32", "le64", "le128", "le256", "le416", "le1024", "gt1024",
];

fn bucket(bytes: usize) -> usize {
    match bytes {
        0..=16 => 0,
        17..=32 => 1,
        33..=64 => 2,
        65..=128 => 3,
        129..=256 => 4,
        257..=416 => 5,
        417..=1024 => 6,
        _ => 7,
    }
}

pub(crate) fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("KF3_LIFECYCLE_COUNT").is_ok_and(|v| v == "1"))
}

/// EXPERIMENT ONLY (`KF3_SCAN_OFF_AFTER_5080`, scratch, never for merge): stop reading pushbuffers once the
/// channel has seen its 5080 SET_OBJECT. Mode 1: from then on, always (the registration window is NOT handled:
/// the first FIFO_UPDATE_CHANNEL_INFO trigger reaches the hardware unscanned). Mode 2: also scan while a
/// FIFO_UPDATE_CHANNEL_INFO registration is pending (the lifecycle plan's registration window).
pub(crate) fn scan_off_mode() -> u8 {
    static M: OnceLock<u8> = OnceLock::new();
    *M.get_or_init(|| match std::env::var("KF3_SCAN_OFF_AFTER_5080").as_deref() {
        Ok("1") => 1,
        Ok("2") => 2,
        _ => 0,
    })
}

pub(crate) fn scan_off() -> bool {
    scan_off_mode() != 0
}

fn started() -> Instant {
    static T: OnceLock<Instant> = OnceLock::new();
    *T.get_or_init(Instant::now)
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Tl {
    pub vram_copies: u64,
    pub vram_bytes: u64,
    pub vram_ns: u64,
    pub ram_bytes: u64,
}
thread_local! {
    static TL: Cell<Tl> = const { Cell::new(Tl { vram_copies: 0, vram_bytes: 0, vram_ns: 0, ram_bytes: 0 }) };
}
/// One BAR1 / store copy (called from `userstore::Native::copy`).
pub(crate) fn note_vram(bytes: usize, ns: u64) {
    TL.with(|t| {
        let mut v = t.get();
        v.vram_copies += 1;
        v.vram_bytes += bytes as u64;
        v.vram_ns += ns;
        t.set(v);
    });
}
/// Guest-RAM bytes copied by a snapshot.
pub(crate) fn note_ram(bytes: usize) {
    TL.with(|t| {
        let mut v = t.get();
        v.ram_bytes += bytes as u64;
        t.set(v);
    });
}
pub(crate) fn tl_take() -> Tl {
    TL.with(|t| t.replace(Tl::default()))
}

type A3 = [[AtomicU64; 3]; 3];

#[derive(Default)]
pub(crate) struct Chan {
    pub key: (u32, u32),
    calls: A3,
    errs: A3,
    bytes: A3,
    ram_bytes: A3,
    vram_bytes: A3,
    vram_copies: A3,
    vram_snaps: A3,
    vram_ns: A3,
    total_ns: A3,
    /// pushbuffer reads / BAR1 copies by [class][length bucket]
    pb_len_calls: [[AtomicU64; 8]; 3],
    pb_len_vcopies: [[AtomicU64; 8]; 3],
    pub gp_entries: AtomicU64,
    pub gp_sync: AtomicU64,
    pub gp_sync_len0: AtomicU64,
    pub gp_sync_len: AtomicU64,
    pub gp_len0: AtomicU64,
    pub turns: AtomicU64,
    pub turns_published: AtomicU64,
    pub turns_retry: AtomicU64,
    pub rare_segments: AtomicU64,
    pub trig_update_channel_info: AtomicU64,
    pub trig_invalidate_tlb: AtomicU64,
    pub trig_context: AtomicU64,
    pub trig_other: AtomicU64,
    pub migrated: AtomicU64,
    /// pushbuffer reads before the first `post5080` read (the lifecycle's scan-from-birth part)
    pub pb_reads_before_5080: AtomicU64,
    pub scanoff_skipped: AtomicU64,
    pub scanoff_skipped_bytes: AtomicU64,
}

impl Chan {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record(
        &self,
        site: Site,
        class: usize,
        bytes: usize,
        ok: bool,
        total_ns: u64,
        tl: Tl,
    ) {
        let (s, c) = (site as usize, class);
        self.calls[s][c].fetch_add(1, Relaxed);
        if !ok {
            self.errs[s][c].fetch_add(1, Relaxed);
            return;
        }
        self.bytes[s][c].fetch_add(bytes as u64, Relaxed);
        self.ram_bytes[s][c].fetch_add(tl.ram_bytes, Relaxed);
        self.vram_bytes[s][c].fetch_add(tl.vram_bytes, Relaxed);
        self.vram_copies[s][c].fetch_add(tl.vram_copies, Relaxed);
        if tl.vram_copies > 0 {
            self.vram_snaps[s][c].fetch_add(1, Relaxed);
        }
        self.vram_ns[s][c].fetch_add(tl.vram_ns, Relaxed);
        self.total_ns[s][c].fetch_add(total_ns, Relaxed);
        if matches!(site, Site::Pb) {
            let b = bucket(bytes);
            self.pb_len_calls[c][b].fetch_add(1, Relaxed);
            self.pb_len_vcopies[c][b].fetch_add(tl.vram_copies, Relaxed);
        }
    }
}

fn registry() -> &'static Mutex<Vec<Arc<Chan>>> {
    static R: OnceLock<Mutex<Vec<Arc<Chan>>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(Vec::new()))
}

pub(crate) fn new_chan(key: (u32, u32)) -> Option<Arc<Chan>> {
    if !on() {
        return None;
    }
    let c = Arc::new(Chan {
        key,
        ..Chan::default()
    });
    if let Ok(mut r) = registry().lock() {
        r.push(c.clone());
    }
    Some(c)
}

fn g(a: &AtomicU64) -> u64 {
    a.load(Relaxed)
}
fn sum3(a: &A3, s: usize, c: usize, chans: &[Arc<Chan>], f: impl Fn(&Chan) -> &A3) -> u64 {
    let _ = a;
    chans.iter().map(|ch| g(&f(ch)[s][c])).sum()
}

fn period_ms() -> u64 {
    static P: OnceLock<u64> = OnceLock::new();
    *P.get_or_init(|| {
        std::env::var("KF3_LIFECYCLE_PERIOD_S")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(10)
            * 1000
    })
}

static NEXT_MS: AtomicU64 = AtomicU64::new(0);
static DB_TRANSLATED: AtomicU64 = AtomicU64::new(0);
static DB_OTHER: AtomicU64 = AtomicU64::new(0);
/// A doorbell write seen by the drainer / vCPU path (`translated` = a worker-served token).
pub(crate) fn note_doorbell(translated: bool) {
    if translated {
        DB_TRANSLATED.fetch_add(1, Relaxed);
    } else {
        DB_OTHER.fetch_add(1, Relaxed);
    }
}

/// Called at the start of each pump turn: one dump per period, by whichever worker gets there first.
pub(crate) fn tick(me: &super::chan::ChanPlane) {
    if !on() {
        return;
    }
    let now = u64::try_from(started().elapsed().as_millis()).unwrap_or(u64::MAX);
    let next = NEXT_MS.load(Relaxed);
    if now < next {
        return;
    }
    if NEXT_MS
        .compare_exchange(next, now + period_ms(), Relaxed, Relaxed)
        .is_err()
    {
        return;
    }
    dump(me, "tick");
}

/// The totals (and, for `why != "tick"`, every channel), as `kf3: LIFECYCLE` lines.
pub(crate) fn dump(me: &super::chan::ChanPlane, why: &str) {
    if !on() {
        return;
    }
    let chans: Vec<Arc<Chan>> = registry().lock().map(|r| r.clone()).unwrap_or_default();
    let t = started().elapsed().as_secs_f64();
    let wall = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64());
    let dummy: A3 = Default::default();
    for (si, sn) in SITES.iter().enumerate() {
        for (ci, cn) in CLASSES.iter().enumerate() {
            let s = |f: fn(&Chan) -> &A3| sum3(&dummy, si, ci, &chans, f);
            let calls = s(|c| &c.calls);
            if calls == 0 {
                continue;
            }
            eprintln!(
                "kf3: LIFECYCLE t={t:.1} wall={wall:.1} {why} read site={sn} class={cn} calls={calls} errs={} bytes={} ram_bytes={} vram_bytes={} vram_copies={} vram_snaps={} vram_ns={} total_ns={}",
                s(|c| &c.errs),
                s(|c| &c.bytes),
                s(|c| &c.ram_bytes),
                s(|c| &c.vram_bytes),
                s(|c| &c.vram_copies),
                s(|c| &c.vram_snaps),
                s(|c| &c.vram_ns),
                s(|c| &c.total_ns),
            );
        }
    }
    for (ci, cn) in CLASSES.iter().enumerate() {
        let calls: Vec<u64> = (0..8)
            .map(|b| chans.iter().map(|c| g(&c.pb_len_calls[ci][b])).sum())
            .collect();
        if calls.iter().all(|x| *x == 0) {
            continue;
        }
        let vc: Vec<u64> = (0..8)
            .map(|b| chans.iter().map(|c| g(&c.pb_len_vcopies[ci][b])).sum())
            .collect();
        eprintln!(
            "kf3: LIFECYCLE t={t:.1} wall={wall:.1} {why} pb_len class={cn} buckets={LEN_BUCKETS:?} calls={calls:?} vram_copies={vc:?}"
        );
    }
    let sum = |f: fn(&Chan) -> &AtomicU64| -> u64 { chans.iter().map(|c| g(f(c))).sum() };
    eprintln!(
        "kf3: LIFECYCLE t={t:.1} wall={wall:.1} {why} gp gp_entries={} len0={} sync_wait={} sync_wait_len0={} sync_wait_len_gt0={}",
        sum(|c| &c.gp_entries),
        sum(|c| &c.gp_len0),
        sum(|c| &c.gp_sync),
        sum(|c| &c.gp_sync_len0),
        sum(|c| &c.gp_sync_len),
    );
    let (db_q, db_r) = (g(&DB_TRANSLATED), g(&DB_OTHER));
    let reg = me.lifecycle_registry_stats();
    eprintln!(
        "kf3: LIFECYCLE t={t:.1} wall={wall:.1} {why} loop chans={} turns={} turns_published={} turns_retry={} rare_segments={} trig_update_channel_info={} trig_invalidate_tlb={} trig_context={} trig_other={} ring_migrated={} pb_reads_before_5080={} scanoff_skipped={} scanoff_skipped_bytes={} doorbells_translated={db_q} doorbells_other={db_r} registry={reg}",
        chans.len(),
        sum(|c| &c.turns),
        sum(|c| &c.turns_published),
        sum(|c| &c.turns_retry),
        sum(|c| &c.rare_segments),
        sum(|c| &c.trig_update_channel_info),
        sum(|c| &c.trig_invalidate_tlb),
        sum(|c| &c.trig_context),
        sum(|c| &c.trig_other),
        sum(|c| &c.migrated),
        sum(|c| &c.pb_reads_before_5080),
        sum(|c| &c.scanoff_skipped),
        sum(|c| &c.scanoff_skipped_bytes),
    );
    eprintln!(
        "kf3: LIFECYCLE t={t:.1} wall={wall:.1} {why} relay_stats {}",
        me.relay_stats.summary()
    );
    eprintln!("kf3: LIFECYCLE t={t:.1} wall={wall:.1} {why} {}", crate::userstore::summary());
    for c in &chans {
        let pb_post: u64 = (1..3).map(|k| g(&c.calls[1][k])).sum();
        let vc_post: u64 = (1..3).map(|k| g(&c.vram_copies[1][k])).sum();
        let pb_pre = g(&c.calls[1][0]);
        // periodic lines only for channels that have done real work; the final dump lists all
        if why == "tick" && pb_post + pb_pre < 1000 {
            continue;
        }
        eprintln!(
            "kf3: LIFECYCLE_CHAN t={t:.1} wall={wall:.1} {why} {:#x}:{:#x} turns={} gp_entries={} pb_calls_pre={pb_pre} pb_calls_post={pb_post} pb_vram_copies_pre={} pb_vram_copies_post={vc_post} pb_vram_ns={} pb_bytes={} rare_segments={} migrated={} trig_uci={} trig_tlb={} trig_ctx={} trig_other={} scanoff_skipped={}",
            c.key.0,
            c.key.1,
            g(&c.turns),
            g(&c.gp_entries),
            g(&c.vram_copies[1][0]),
            (0..3).map(|k| g(&c.vram_ns[1][k])).sum::<u64>(),
            (0..3).map(|k| g(&c.bytes[1][k])).sum::<u64>(),
            g(&c.rare_segments),
            g(&c.migrated),
            g(&c.trig_update_channel_info),
            g(&c.trig_invalidate_tlb),
            g(&c.trig_context),
            g(&c.trig_other),
            g(&c.scanoff_skipped),
        );
    }
}
