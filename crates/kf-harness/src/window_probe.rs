//! Research USER-channel window probe. A timeout is never evidence of isolation.
//!
//! All writes target the probe's own allocation. An externally supplied window is read for
//! exactly one word, after a trusted host-side fixture has associated it with this VM's canary.
//! This module supplies no physical-mode operands, privileged controls, or Translated injection.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use kf_abi::generated::rpc::ROBUST_CHANNEL_FIFO_ERROR_MMU_ERR_FLT;
use kf_abi::notifier::{ErrorNotification, NOTIFICATION_SIZE, NOTIFIER_STATUS_RC};
use kf_abi::submit::{ENGINE_TYPE_COPY0, USERD_GP_GET, USERD_GP_PUT, gp_entry};
use kf_host::{Channel, HostRm, VaSpace};
use kf_linux_raw::{HostOffset, VolatileRegion};

const BYTES: u64 = 0x1_0000;
const GPFIFO: u64 = 0x1000;
const SEM: u64 = 0x2000;
const FENCE: u64 = 0x2010;
const USERD: u64 = 0x3000;
const ERROR: u64 = 0x4000;
const SOURCE: u64 = 0x5000;
const DEST: u64 = 0x6000;
const POISON: u32 = 0x91f0_25a6;
const MANIFEST_MAX: u64 = 4096;
const WAIT: Duration = Duration::from_secs(3);

/// Which result the host's fixture expects for this exact run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expect {
    /// The legacy window must expose the known canary (positive control).
    Read,
    /// The removed window must cause a channel-local MMU fault.
    Fault,
}

/// A single-read fixture, bounded before any GPU submission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// An opaque run nonce, which prevents stale fixture reuse.
    pub nonce: u64,
    /// The probe's guest-visible VA-space handle from READY.
    pub space: u32,
    /// RM-chosen window base, from this VM's host evidence.
    pub base: u64,
    /// Window extent, from the same host evidence.
    pub len: u64,
    /// Offset of the probe-owned canary in that window.
    pub offset: u64,
    /// Expected result.
    pub expect: Expect,
}

impl Manifest {
    /// Parse a bounded, strict key/value manifest. No unknown or repeated key is accepted.
    pub fn parse(bytes: &[u8], nonce: u64, space: u32) -> Result<Self, String> {
        if bytes.len() > MANIFEST_MAX as usize {
            return Err("manifest exceeds 4096 bytes".into());
        }
        let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
        let mut rows = BTreeMap::new();
        for line in text.lines() {
            let (k, v) = line
                .split_once('=')
                .ok_or("manifest needs key=value lines")?;
            if ![
                "format", "nonce", "space", "base", "len", "offset", "expect",
            ]
            .contains(&k)
                || rows.insert(k, v).is_some()
            {
                return Err(format!("unknown or duplicate manifest key {k}"));
            }
        }
        let get = |k| rows.get(k).copied().ok_or_else(|| format!("missing {k}"));
        let number = |k| -> Result<u64, String> {
            let s = get(k)?;
            let (digits, radix) = s.strip_prefix("0x").map_or((s, 10), |s| (s, 16));
            u64::from_str_radix(digits, radix).map_err(|e| format!("{k}: {e}"))
        };
        if get("format")? != "kf-window-v1" {
            return Err("unknown manifest format".into());
        }
        let m = Self {
            nonce: number("nonce")?,
            space: u32::try_from(number("space")?).map_err(|e| e.to_string())?,
            base: number("base")?,
            len: number("len")?,
            offset: number("offset")?,
            expect: match get("expect")? {
                "read" => Expect::Read,
                "fault" => Expect::Fault,
                _ => return Err("expect must be read or fault".into()),
            },
        };
        if m.nonce != nonce || m.space != space {
            return Err("manifest names another run or VA space".into());
        }
        m.target()?;
        Ok(m)
    }

    /// Validate a naturally aligned four-byte read inside a finite window below 2^40.
    pub fn target(&self) -> Result<u64, String> {
        if self.base == 0
            || !self.base.is_multiple_of(4096)
            || self.len == 0
            || !self.len.is_multiple_of(4096)
            || !self.offset.is_multiple_of(4)
            || self.offset.checked_add(4).is_none_or(|end| end > self.len)
            || self
                .base
                .checked_add(self.len)
                .is_none_or(|end| end > 1 << 40)
        {
            return Err("invalid or overflowing window/canary extent".into());
        }
        self.base
            .checked_add(self.offset)
            .ok_or("target overflow".into())
    }
}

/// Actual GPU observations, retained even for an inconclusive timeout.
#[derive(Debug, Clone, Copy)]
pub struct Observation {
    /// The exact host-WFI fence payload landed.
    pub fence: bool,
    /// The exact CE semaphore payload landed.
    pub ce: bool,
    /// CPU snapshot of the channel's own error notifier.
    pub error: ErrorNotification,
    /// Word in the probe-owned destination.
    pub word: u32,
    /// Hardware's consume cursor (diagnostic, never a completion substitute).
    pub get: u32,
    /// Time actually spent waiting.
    pub elapsed_ms: u128,
}

/// Strict verdict: require a real completion/canary or a fresh MMU notifier and no write.
/// Neighbor liveness and baseline success must independently be true.
pub fn verdict(
    o: Observation,
    expect: Expect,
    canary: u32,
    baseline: bool,
    neighbor: bool,
) -> bool {
    if !baseline || !neighbor {
        return false;
    }
    match expect {
        Expect::Read => o.fence && o.ce && o.error.status == 0 && o.word == canary,
        Expect::Fault => {
            !o.fence
                && !o.ce
                && o.word == POISON
                && o.error.status == NOTIFIER_STATUS_RC
                && o.error.except_type == ROBUST_CHANNEL_FIFO_ERROR_MMU_ERR_FLT
                && u32::from(o.error.engine_type) == ENGINE_TYPE_COPY0
        }
    }
}

struct Rig<'a> {
    rm: &'a HostRm,
    space: VaSpace,
    object: Option<u32>,
    va: Option<u64>,
    alias: Option<u64>,
    cookie: Option<u64>,
    cpu: Option<VolatileRegion>,
    node: Option<kf_linux_raw::CharDevice>,
    context: Option<u32>,
    channel: Option<Channel>,
    put: u32,
    payload: u32,
    canary: u32,
    closed: bool,
}

impl<'a> Rig<'a> {
    fn new(rm: &'a HostRm, canary: u32) -> Result<Self, String> {
        let mut r = Self {
            rm,
            space: rm.alloc_vaspace().map_err(|e| format!("space: {e:?}"))?,
            object: None,
            va: None,
            alias: None,
            cookie: None,
            cpu: None,
            node: None,
            context: None,
            channel: None,
            put: 0,
            // Independent rigs have complementary canaries, hence disjoint payload ranges.
            // Sixteen submissions cannot carry into the upper 24 bits or wrap to zero.
            payload: canary & 0xffff_ff00,
            canary,
            closed: false,
        };
        let object = rm
            .alloc_device_local(BYTES)
            .map_err(|e| format!("allocation: {e:?}"))?;
        r.object = Some(object);
        r.va = Some(
            rm.map(
                r.space,
                object,
                kf_host::MapBacking::Dedicated,
                0,
                BYTES,
                None,
                false,
            )
            .map_err(|e| format!("map: {e:?}"))?,
        );
        let (node, cookie) = rm
            .arm_cpu_view(
                kf_host::MapNode::Gpu,
                object,
                0,
                BYTES,
                kf_host::ViewAccess::ReadWrite,
            )
            .map_err(|e| format!("CPU view: {e:?}"))?;
        r.cookie = Some(cookie);
        r.node = Some(node);
        r.cpu = Some(
            VolatileRegion::map(
                kf_linux_raw::Backing::DeviceFile {
                    fd: r.node.as_ref().unwrap().as_fd(),
                },
                BYTES,
                kf_linux_raw::CachePolicy::WriteCombining,
                kf_linux_raw::HostPageSize::query(),
            )
            .map_err(|e| format!("mmap: {e:?}"))?,
        );
        for off in (0..BYTES).step_by(4) {
            r.store(off, 0)?;
        }
        r.store(SOURCE, canary)?;
        let context = rm
            .alloc_context_dma(object, ERROR, NOTIFICATION_SIZE as u64)
            .map_err(|e| format!("error context: {e:?}"))?;
        r.context = Some(context);
        let chan = rm
            .birth_channel(
                r.space,
                ENGINE_TYPE_COPY0,
                kf_host::RingSpec {
                    gp_fifo_va: r.va.unwrap() + GPFIFO,
                    gp_fifo_entries: 64,
                    userd_memory: object,
                    userd_offset: USERD,
                    err_notifier: context,
                },
            )
            .map_err(|e| format!("USER birth: {e:?}"))?;
        r.channel = Some(chan);
        rm.alloc_ce_object(chan, ENGINE_TYPE_COPY0)
            .map_err(|e| format!("CE: {e:?}"))?;
        rm.schedule(chan).map_err(|e| format!("schedule: {e:?}"))?;
        Ok(r)
    }

    fn store(&self, off: u64, value: u32) -> Result<(), String> {
        self.cpu
            .as_ref()
            .unwrap()
            .store_u32(HostOffset::new(off), value)
            .map_err(|e| format!("store: {e:?}"))
    }
    fn load(&self, off: u64) -> Result<u32, String> {
        self.cpu
            .as_ref()
            .unwrap()
            .load_u32(HostOffset::new(off))
            .map_err(|e| format!("load: {e:?}"))
    }
    fn error(&self) -> Result<ErrorNotification, String> {
        // Status is the publication word. The acquire fence orders the remaining snapshot
        // after it; the driver/GSP writes the record before publishing its nonzero status.
        let tail = self.load(ERROR + 12)?;
        std::sync::atomic::fence(std::sync::atomic::Ordering::Acquire);
        let mut raw = [0u8; NOTIFICATION_SIZE];
        for i in 0..3 {
            raw[i * 4..i * 4 + 4].copy_from_slice(&self.load(ERROR + i as u64 * 4)?.to_le_bytes());
        }
        raw[12..].copy_from_slice(&tail.to_le_bytes());
        ErrorNotification::decode(&raw).map_err(|e| format!("notifier: {e:?}"))
    }

    fn copy_word(&mut self, source: u64) -> Result<Observation, String> {
        if !source.is_multiple_of(4)
            || source.checked_add(4).is_none_or(|end| end > 1 << 40)
            || self.error()?.status != 0
            || self.put >= 16
        {
            return Err("invalid source, dead channel, or submission bound".into());
        }
        let va = self.va.unwrap();
        self.store(DEST, POISON)?;
        self.store(SEM, 0)?;
        self.store(FENCE, 0)?;
        self.payload += 1;
        let mut words = crate::ce_copy_push(
            self.rm.ce_class_id(),
            source,
            va + DEST,
            4,
            va + SEM,
            self.payload,
            false,
        )
        .ok_or("copy encoding")?;
        words.extend(
            crate::host_fence_push(va + FENCE, self.payload, false).ok_or("fence encoding")?,
        );
        if words.len() > 64 {
            return Err("push exceeds slot".into());
        }
        let slot = u64::from(self.put) * 0x100;
        for (i, word) in words.iter().enumerate() {
            self.store(slot + i as u64 * 4, *word)?;
        }
        let entry = gp_entry(va + slot, words.len() as u64 * 4).ok_or("GP entry")?;
        self.store(GPFIFO + u64::from(self.put) * 8, entry as u32)?;
        self.store(GPFIFO + u64::from(self.put) * 8 + 4, (entry >> 32) as u32)?;
        self.put += 1;
        kf_linux_raw::release_fence();
        self.store(USERD + USERD_GP_PUT, self.put)?;
        kf_linux_raw::release_fence();
        let start = Instant::now();
        self.rm
            .doorbell(self.channel.unwrap().token)
            .map_err(|e| format!("doorbell: {e:?}"))?;
        loop {
            let error = self.error()?;
            let fence = self.load(FENCE)? == self.payload;
            if error.status != 0 || fence || start.elapsed() >= WAIT {
                return self.observe(start.elapsed().as_millis());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn observe(&self, elapsed_ms: u128) -> Result<Observation, String> {
        let error = self.error()?;
        let fence = self.load(FENCE)? == self.payload;
        std::sync::atomic::fence(std::sync::atomic::Ordering::Acquire);
        Ok(Observation {
            fence,
            ce: self.load(SEM)? == self.payload,
            error,
            word: self.load(DEST)?,
            get: self.load(USERD + USERD_GP_GET)?,
            elapsed_ms,
        })
    }
    fn baseline(&mut self) -> Result<bool, String> {
        let o = self.copy_word(self.va.unwrap() + SOURCE)?;
        println!("BASELINE space={:#x} {o:?}", self.space.space);
        Ok(verdict(o, Expect::Read, self.canary, true, true))
    }
    fn close(&mut self) -> Result<(), String> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        let mut errors = Vec::new();
        let mut record = |what: &str, result: Result<(), kf_host::RmError>| {
            if let Err(e) = result {
                errors.push(format!("{what}: {e:?}"));
            }
        };
        if let Some(c) = self.channel.take() {
            record("channel", self.rm.free_channel(c));
        }
        if let Some(c) = self.context.take() {
            record("context", self.rm.free(c));
        }
        for va in [self.alias.take(), self.va.take()].into_iter().flatten() {
            record("VA mapping", self.rm.unmap(self.space, va, false));
        }
        drop(self.cpu.take());
        if let (Some(object), Some(cookie)) = (self.object, self.cookie.take()) {
            record(
                "CPU view",
                self.rm.release_cpu_view(kf_host::CpuViewRelease {
                    h_memory: object,
                    p_linear_address: cookie,
                }),
            );
        }
        drop(self.node.take());
        if let Some(object) = self.object.take() {
            record("allocation", self.rm.free(object));
        }
        for g in self.space.guest.iter().filter(|g| g.handle != 0) {
            record("reservation", self.rm.free(g.handle));
        }
        record("VA range", self.rm.free(self.space.range));
        record("VA space", self.rm.free(self.space.space));
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}

impl Drop for Rig<'_> {
    fn drop(&mut self) {
        if let Err(e) = self.close() {
            eprintln!("probe cleanup failed: {e}");
        }
    }
}

/// Run either the native instrumentation self-test or the externally provisioned guest probe.
/// This intentionally performs potentially faulting GPU work; never run alongside other jobs.
pub fn run(rm: &HostRm, manifest: Option<&Path>) -> Result<bool, String> {
    if manifest.is_some_and(Path::exists) {
        return Err("manifest must not pre-exist READY".into());
    }
    let mut bytes = [0u8; 8];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|e| e.to_string())?;
    let nonce = u64::from_le_bytes(bytes);
    let canary = (nonce as u32) ^ 0xa175_083b;
    if [0, u32::MAX, POISON, !POISON].contains(&canary) {
        return Err("degenerate random canary; rerun".into());
    }
    let mut primary = Rig::new(rm, canary)?;
    let mut neighbor = Rig::new(rm, !canary)?;
    let baseline = primary.baseline()? && neighbor.baseline()?;
    if !baseline {
        return Err("positive ordinary-copy baseline failed".into());
    }
    let m = if let Some(path) = manifest {
        println!(
            "WINDOW_READY nonce={nonce:#x} space={:#x} channel={:#x} object={:#x} source_va={:#x} source_offset={SOURCE:#x} canary={canary:#x}",
            primary.space.space,
            primary.channel.unwrap().chan,
            primary.object.unwrap(),
            primary.va.unwrap() + SOURCE
        );
        std::io::stdout().flush().map_err(|e| e.to_string())?;
        let start = Instant::now();
        let data = loop {
            match std::fs::symlink_metadata(path) {
                Ok(meta) => {
                    if !meta.is_file() {
                        return Err("manifest must be a regular file".into());
                    }
                    // The trusted coordinator publishes by atomic rename and never mutates the
                    // resulting file. Reject FIFOs/devices/symlinks before attempting an open.
                    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
                    if !f.metadata().map_err(|e| e.to_string())?.is_file() {
                        return Err("manifest changed type".into());
                    }
                    let mut b = Vec::new();
                    f.take(MANIFEST_MAX + 1)
                        .read_to_end(&mut b)
                        .map_err(|e| e.to_string())?;
                    break b;
                }
                Err(e)
                    if e.kind() == std::io::ErrorKind::NotFound
                        && start.elapsed() < Duration::from_secs(60) =>
                {
                    std::thread::sleep(Duration::from_millis(50))
                }
                Err(e) => return Err(format!("manifest not available: {e}")),
            }
        };
        Manifest::parse(&data, nonce, primary.space.space)?
    } else {
        // Instrumentation control: create then remove an alias of OUR allocation. This proves
        // the notifier/neighbor oracle; it does not exercise QEMU's choice to omit windows.
        let base = rm
            .map_window(primary.space, primary.object.unwrap(), BYTES, false)
            .map_err(|e| format!("alias: {e:?}"))?;
        primary.alias = Some(base);
        let o = primary.copy_word(base.checked_add(SOURCE).ok_or("alias overflow")?)?;
        if !verdict(o, Expect::Read, canary, true, true) {
            return Err(format!("positive alias read failed: {o:?}"));
        }
        println!("ALIAS_POSITIVE base={base:#x} {o:?}");
        rm.unmap(primary.space, base, false)
            .map_err(|e| format!("remove alias: {e:?}"))?;
        primary.alias = None;
        Manifest {
            nonce,
            space: primary.space.space,
            base,
            len: BYTES,
            offset: SOURCE,
            expect: Expect::Fault,
        }
    };
    let target = m.target()?;
    let own = primary.va.unwrap();
    if target >= own && target < own + BYTES {
        return Err("window target aliases ordinary mapped probe memory".into());
    }
    let o = primary.copy_word(target)?;
    println!(
        "WINDOW_OBSERVED target={target:#x} expected={:?} {o:?}",
        m.expect
    );
    let neighbor_alive = neighbor.baseline()?;
    // Read the victim again after the neighbor's GPU fence, so a late destination write or
    // completion cannot be hidden by the first notifier snapshot.
    let final_o = primary.observe(o.elapsed_ms)?;
    let source_unchanged = primary.load(SOURCE)? == canary;
    let good = verdict(o, m.expect, canary, baseline, neighbor_alive)
        && verdict(final_o, m.expect, canary, baseline, neighbor_alive)
        && source_unchanged;
    println!("WINDOW_FINAL {final_o:?}");
    println!(
        "WINDOW_CHECK baseline={baseline} neighbor_after={neighbor_alive} source_unchanged={source_unchanged} bounded_read=4"
    );
    // A successful observation with refused cleanup is not a passing probe.
    let a = primary.close();
    let b = neighbor.close();
    a.and(b)?;
    println!("WINDOW_CLEANUP=PASS");
    Ok(good)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fault() -> Observation {
        Observation {
            fence: false,
            ce: false,
            error: ErrorNotification::rc(
                ROBUST_CHANNEL_FIFO_ERROR_MMU_ERR_FLT,
                ENGINE_TYPE_COPY0 as u16,
                1,
            ),
            word: POISON,
            get: 2,
            elapsed_ms: 3,
        }
    }

    #[test]
    fn only_channel_fault_plus_live_neighbor_and_intact_canary_proves_denial() {
        let good = fault();
        assert!(verdict(good, Expect::Fault, 7, true, true));
        assert!(!verdict(good, Expect::Fault, 7, false, true));
        assert!(!verdict(good, Expect::Fault, 7, true, false));
        let mut bad = good;
        bad.error.status = 0;
        assert!(!verdict(bad, Expect::Fault, 7, true, true), "timeout alone");
        bad = good;
        bad.error.except_type += 1;
        assert!(!verdict(bad, Expect::Fault, 7, true, true), "unrelated RC");
        bad = good;
        bad.error.engine_type += 1;
        assert!(!verdict(bad, Expect::Fault, 7, true, true), "wrong engine");
        bad = good;
        bad.error.status = 1;
        assert!(
            !verdict(bad, Expect::Fault, 7, true, true),
            "unpublished/unknown status"
        );
        bad = good;
        bad.fence = true;
        assert!(
            !verdict(bad, Expect::Fault, 7, true, true),
            "fence after alleged fault"
        );
        bad = good;
        bad.ce = true;
        assert!(!verdict(bad, Expect::Fault, 7, true, true), "CE released");
        bad = good;
        bad.word = 7;
        assert!(!verdict(bad, Expect::Fault, 7, true, true), "target leaked");
    }

    #[test]
    fn positive_control_requires_both_real_completions_and_exact_bytes() {
        let mut o = fault();
        o.fence = true;
        o.ce = true;
        o.error.status = 0;
        o.word = 7;
        assert!(verdict(o, Expect::Read, 7, true, true));
        o.word = 8;
        assert!(!verdict(o, Expect::Read, 7, true, true));
        o.word = 7;
        o.ce = false;
        assert!(!verdict(o, Expect::Read, 7, true, true));
        o.ce = true;
        o.fence = false;
        assert!(!verdict(o, Expect::Read, 7, true, true));
        o.fence = true;
        o.error.status = NOTIFIER_STATUS_RC;
        assert!(!verdict(o, Expect::Read, 7, true, true));
    }

    fn manifest() -> String {
        "format=kf-window-v1\nnonce=9\nspace=3\nbase=0x100000\nlen=0x10000\noffset=0x5000\nexpect=fault\n".into()
    }

    #[test]
    fn fixture_requires_exact_run_strict_keys_and_bounded_read_extent() {
        let good = manifest();
        assert_eq!(
            Manifest::parse(good.as_bytes(), 9, 3)
                .unwrap()
                .target()
                .unwrap(),
            0x105000
        );
        assert!(Manifest::parse(good.as_bytes(), 8, 3).is_err());
        assert!(Manifest::parse(good.as_bytes(), 9, 4).is_err());
        for bad in [
            good.clone() + "space=3\n",
            good.clone() + "physical=1\n",
            good.replace("expect=fault", "expect=timeout"),
            good.replace("len=0x10000", "len=0"),
            good.replace("offset=0x5000", "offset=0x10000"),
            good.replace("offset=0x5000", "offset=3"),
            good.replace("base=0x100000", "base=0xfffffffffffff000"),
            good.replace("base=0x100000", "base=0x10000000000"),
            good.replace("space=3", "space=0x100000000"),
            "x".repeat(4097),
        ] {
            assert!(Manifest::parse(bad.as_bytes(), 9, 3).is_err(), "{bad}");
        }
        let edge = good.replace("offset=0x5000", "offset=0xfffc");
        assert_eq!(
            Manifest::parse(edge.as_bytes(), 9, 3)
                .unwrap()
                .target()
                .unwrap(),
            0x10fffc
        );
    }

    #[test]
    fn notifier_decoder_refuses_every_partial_publication_and_preserves_fields() {
        let n = ErrorNotification::rc(
            ROBUST_CHANNEL_FIFO_ERROR_MMU_ERR_FLT,
            ENGINE_TYPE_COPY0 as u16,
            u64::MAX,
        );
        let b = n.encode();
        assert_eq!(ErrorNotification::decode(&b).unwrap(), n);
        for len in 0..NOTIFICATION_SIZE {
            assert!(ErrorNotification::decode(&b[..len]).is_err());
        }
    }
}
