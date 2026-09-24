//! v3 harness support: CE pushbuffer encoding and a verdict ledger.
//!
//! ★ A verdict is computed from named checks, never from reaching the end of a program (the old
//! tree's *"the last line is not the verdict"*). Every check prints `CHECK <name> PASS|FAIL <why>`.

use kf_abi::submit::{SET_OBJECT, ce, method_header_inc};

/// The CE subchannel every harness push uses.
pub const CE_SUBCHANNEL: u32 = 4;

/// One named check's outcome.
#[derive(Debug)]
pub struct Check {
    /// The check's name.
    pub name: &'static str,
    /// Whether it passed.
    pub pass: bool,
    /// What was measured.
    pub why: String,
}

/// The run's checks, and the verdict they imply.
#[derive(Debug, Default)]
pub struct Ledger {
    checks: Vec<Check>,
}

impl Ledger {
    /// Record and print one check.
    pub fn check(&mut self, name: &'static str, pass: bool, why: impl Into<String>) {
        let why = why.into();
        println!("CHECK {name} {} {why}", if pass { "PASS" } else { "FAIL" });
        self.checks.push(Check { name, pass, why });
    }

    /// Record a measurement that is NOT a pass/fail (an unknown being measured).
    pub fn measure(&mut self, name: &'static str, value: impl Into<String>) {
        println!("MEASURE {name} {}", value.into());
    }

    /// PASS only if at least one check ran and every check passed. ⊘ Zero checks is a FAIL.
    #[must_use]
    pub fn verdict(&self) -> bool {
        !self.checks.is_empty() && self.checks.iter().all(|c| c.pass)
    }
}

/// A virtual-addressed CE copy of `len` bytes `src → dst`, releasing `payload` at `sem_va`,
/// optionally raising a NON-STALL interrupt when done.
#[must_use]
pub fn ce_copy_push(
    ce_class: u32,
    src: u64,
    dst: u64,
    len: u32,
    sem_va: u64,
    payload: u32,
    interrupt: bool,
) -> Option<Vec<u32>> {
    let sub = CE_SUBCHANNEL;
    let mut flags = ce::LAUNCH_TRANSFER_NON_PIPELINED
        | ce::LAUNCH_FLUSH_ENABLE
        | ce::LAUNCH_SEMAPHORE_RELEASE_ONE_WORD
        | ce::LAUNCH_SRC_PITCH
        | ce::LAUNCH_DST_PITCH
        | ce::LAUNCH_MULTI_LINE_DISABLE
        | ce::LAUNCH_SRC_VIRTUAL
        | ce::LAUNCH_DST_VIRTUAL;
    if interrupt {
        flags |= ce::LAUNCH_INTERRUPT_NON_BLOCKING;
    }
    Some(vec![
        method_header_inc(sub, SET_OBJECT, 1)?,
        ce_class,
        method_header_inc(sub, ce::OFFSET_IN_UPPER, 4)?,
        (src >> 32) as u32,
        (src & 0xFFFF_FFFF) as u32,
        (dst >> 32) as u32,
        (dst & 0xFFFF_FFFF) as u32,
        method_header_inc(sub, ce::LINE_LENGTH_IN, 2)?,
        len,
        1,
        method_header_inc(sub, ce::SET_SEMAPHORE_A, 3)?,
        (sem_va >> 32) as u32,
        (sem_va & 0xFFFF_FFFF) as u32,
        payload,
        method_header_inc(sub, ce::LAUNCH_DMA, 1)?,
        flags,
    ])
}

/// ★ A copy-engine channel on OUR OWN ring in a given host VA space, completing through the host
/// event fd (the host `NON_STALL_INTERRUPT` method → `FIFO_EVENT_MTHD`, measured gate 1).
pub struct CeRig {
    ring_cpu: kf_linux_raw::VolatileRegion,
    _ring_node: kf_linux_raw::CharDevice,
    ring_va: u64,
    chan: kf_host::Channel,
    ev: kf_host::EventFd,
    poller: kf_linux_raw::Poller,
    put: u32,
    payload: u32,
    ce_class: u32,
}

const RIG_RING_BYTES: u64 = 0x1_0000;
const RIG_GPFIFO_OFF: u64 = 0x1000;
const RIG_GPFIFO_ENTRIES: u32 = 64;
const RIG_SEM_OFF: u64 = 0x2000;
const RIG_USERD_OFF: u64 = 0x3000;
const RIG_PB_SLOT: u64 = 0x80;
/// `NV2080_NOTIFIERS_FIFO_EVENT_MTHD` — the host NSI method's edge.
pub const FIFO_EVENT_MTHD: u32 = 35;

/// What one rig submission produced.
#[derive(Debug, Clone, Copy)]
pub struct Submitted {
    /// Event-fd readiness count within the timeout.
    pub ready: usize,
    /// Microseconds from doorbell to readiness (or to timeout).
    pub event_us: u128,
    /// Records drained.
    pub records: usize,
    /// Whether the engine released the expected semaphore payload.
    pub released: bool,
}

impl CeRig {
    /// Build the rig in `space`.
    ///
    /// # Errors
    /// Any step's refusal, by name.
    pub fn new(rm: &kf_host::HostRm, space: kf_host::VaSpace) -> Result<CeRig, String> {
        use kf_abi::submit::ENGINE_TYPE_COPY0;
        let ring = rm.alloc_device_local(RIG_RING_BYTES).map_err(|e| format!("ring obj: {e:?}"))?;
        let ring_va = rm.map(space, ring, 0, RIG_RING_BYTES, None, false).map_err(|e| format!("map ring: {e:?}"))?;
        let (ring_node, ring_cpu) = rm
            .map_cpu(ring, RIG_RING_BYTES, kf_linux_raw::CachePolicy::Uncached)
            .map_err(|e| format!("cpu ring: {e:?}"))?;
        let ev = rm.open_event_fd().map_err(|e| format!("event fd: {e:?}"))?;
        rm.alloc_os_event(rm.subdevice(), FIFO_EVENT_MTHD, true, &ev).map_err(|e| format!("os event: {e:?}"))?;
        rm.set_notification(FIFO_EVENT_MTHD, kf_abi::eventnotify::ACTION_REPEAT)
            .map_err(|e| format!("notify: {e:?}"))?;
        let chan = rm
            .birth_channel(space, ENGINE_TYPE_COPY0, kf_host::RingSpec {
                gp_fifo_va: ring_va + RIG_GPFIFO_OFF,
                gp_fifo_entries: RIG_GPFIFO_ENTRIES,
                userd_memory: ring,
                userd_offset: RIG_USERD_OFF,
                err_notifier: 0,
            })
            .map_err(|e| format!("birth: {e:?}"))?;
        rm.alloc_ce_object(chan, ENGINE_TYPE_COPY0).map_err(|e| format!("ce object: {e:?}"))?;
        rm.schedule(chan).map_err(|e| format!("schedule: {e:?}"))?;
        let poller = kf_linux_raw::Poller::create().map_err(|e| format!("epoll: {e:?}"))?;
        poller.watch(ev.as_fd(), 1).map_err(|e| format!("watch: {e:?}"))?;
        Ok(CeRig {
            ring_cpu,
            _ring_node: ring_node,
            ring_va,
            chan,
            ev,
            poller,
            put: 0,
            payload: 0x1000,
            ce_class: rm.ce_class_id(),
        })
    }

    /// The rig's channel.
    #[must_use]
    pub fn channel(&self) -> kf_host::Channel {
        self.chan
    }

    /// Copy `len` bytes `src_va → dst_va` (VAs in the rig's space) and wait for the event edge.
    ///
    /// # Errors
    /// An encode, store or wait failure.
    pub fn copy(&mut self, rm: &kf_host::HostRm, src_va: u64, dst_va: u64, len: u32) -> Result<Submitted, String> {
        use kf_abi::submit::{USERD_GP_PUT, gp_entry};
        use kf_linux_raw::HostOffset as At;
        self.payload = self.payload.wrapping_add(1);
        let sem_va = self.ring_va + RIG_SEM_OFF;
        let mut words = ce_copy_push(self.ce_class, src_va, dst_va, len, sem_va, self.payload, false)
            .ok_or("push encode")?;
        words.push(kf_abi::submit::method_header_inc(0, 0x20, 1).ok_or("nsi")?);
        words.push(0);
        let slot = u64::from(self.put % 32) * RIG_PB_SLOT;
        if 4 * words.len() as u64 > RIG_PB_SLOT {
            return Err("push larger than a slot".into());
        }
        for (i, w) in words.iter().enumerate() {
            self.ring_cpu.store_u32(At::new(slot + 4 * i as u64), *w).map_err(|e| format!("{e:?}"))?;
        }
        let entry = gp_entry(self.ring_va + slot, 4 * words.len() as u64).ok_or("gp entry")?;
        let gp = RIG_GPFIFO_OFF + u64::from(self.put % RIG_GPFIFO_ENTRIES) * 8;
        self.ring_cpu.store_u32(At::new(gp), entry as u32).map_err(|e| format!("{e:?}"))?;
        self.ring_cpu.store_u32(At::new(gp + 4), (entry >> 32) as u32).map_err(|e| format!("{e:?}"))?;
        self.put = self.put.wrapping_add(1);
        kf_linux_raw::release_fence();
        self.ring_cpu
            .store_u32(At::new(RIG_USERD_OFF + USERD_GP_PUT), self.put % RIG_GPFIFO_ENTRIES)
            .map_err(|e| format!("{e:?}"))?;
        kf_linux_raw::release_fence();
        let t0 = std::time::Instant::now();
        rm.doorbell(self.chan.token).map_err(|e| format!("doorbell: {e:?}"))?;
        let mut ready = kf_linux_raw::ReadyTokens::new();
        let n = self
            .poller
            .wait(&mut ready, kf_linux_raw::PollTimeout::Millis(2000))
            .map_err(|e| format!("wait: {e:?}"))?;
        let event_us = t0.elapsed().as_micros();
        let records = self.ev.drain().map_err(|e| format!("drain: {e:?}"))?.len();
        let sem = self.ring_cpu.load_u32(At::new(RIG_SEM_OFF)).map_err(|e| format!("{e:?}"))?;
        Ok(Submitted { ready: n, event_us, records, released: sem == self.payload })
    }
}
