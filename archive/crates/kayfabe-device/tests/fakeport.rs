//! ⊘ **The byte-port fixture, shared** — cut B's [`kayfabe_device::DeviceFbPort`] without a
//! GPU, an isolate or a descriptor. Lifted into its own file for `tiny.rs`'s reason: a second
//! description of what an armed run is would be a second source of truth, and this tree has
//! paid for that before.
//!
//! # ★★★ WHAT IT IS FAITHFUL TO, AND WHAT IT DELIBERATELY IS NOT
//!
//! Faithful: **the state machine**. A run is armed or it is not; a miss records a want; a
//! drain turns wants into armed runs; a drain may **decline** (the shell declines on a vCPU or
//! inside an MMIO trap, which no `cargo test` can reproduce); an arm may be **refused** (the
//! host BAR1 aperture is full). That is the whole of what cut B's callers branch on, and it is
//! exactly what a test can pin.
//!
//! ⊘ **NOT** faithful to residence. The bytes here are host memory in this process. Whether
//! the real port's bytes are in device-local video memory is `THE_CONSTRAINTS.md` §18's
//! property and is **not a question `cargo test` can ask** — the same disclaimer
//! `two_worlds_split.rs` opens with, restated here so nothing built on this fixture is read as
//! evidence about residence.
//!
//! ⊘ **Sparse, like the object is not.** The real reserved object exists whole, so an address
//! nobody wrote reads **zero** rather than faulting — this fixture reproduces that by
//! materialising a grain on first write and answering zeros for one that was never written. A
//! dense `Vec` would need 12 GiB to hold a GA106's framebuffer and the addresses that matter
//! (`bar1_pde_base` ≈ 11.8 GiB) live at the top of it.

#![allow(dead_code)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use kayfabe_device::{DeviceFbDrained, DeviceFbPort, DeviceFbWant};

/// The fixture's arming grain. ⊘ Deliberately **not** the shell's `ARM_GRAIN`: nothing in the
/// store or in its callers may depend on the number, and a fixture that copied it would hide a
/// dependency on it rather than expose one.
pub const FAKE_GRAIN: u64 = 0x1000;

/// A byte port over a sparse map of grains.
#[derive(Debug)]
pub struct FakePort {
    len: u64,
    grains: Mutex<std::collections::BTreeMap<u64, Vec<u8>>>,
    armed: Mutex<std::collections::BTreeSet<u64>>,
    want: Mutex<std::collections::BTreeSet<u64>>,
    /// When set, every [`DeviceFbPort::drain`] **declines by name** — the shell's vCPU /
    /// in-trap refusal, which is otherwise unreachable offline.
    decline: AtomicBool,
    /// When set, every arm is refused — the host BAR1 aperture being full.
    refuse: AtomicBool,
    drains: AtomicU64,
    declined: AtomicU64,
    armed_total: AtomicU64,
    served_read: AtomicU64,
    served_write: AtomicU64,
    wanted_read: AtomicU64,
    wanted_write: AtomicU64,
    /// ★ w743 — how many times [`DeviceFbPort::probe_or_want`] was asked.
    probes: AtomicU64,
}

impl FakePort {
    /// A port over an object of `len` bytes, with nothing armed and nothing written.
    #[must_use]
    pub fn new(len: u64) -> FakePort {
        FakePort {
            len,
            grains: Mutex::new(std::collections::BTreeMap::new()),
            armed: Mutex::new(std::collections::BTreeSet::new()),
            want: Mutex::new(std::collections::BTreeSet::new()),
            decline: AtomicBool::new(false),
            refuse: AtomicBool::new(false),
            drains: AtomicU64::new(0),
            declined: AtomicU64::new(0),
            armed_total: AtomicU64::new(0),
            served_read: AtomicU64::new(0),
            served_write: AtomicU64::new(0),
            wanted_read: AtomicU64::new(0),
            wanted_write: AtomicU64::new(0),
            probes: AtomicU64::new(0),
        }
    }

    /// How many times the pre-flight asked this port a question.
    #[must_use]
    pub fn probes(&self) -> u64 {
        self.probes.load(Ordering::Relaxed)
    }

    /// Put bytes into the object **without** arming anything — the guest's engines writing
    /// video memory the host has no CPU view of.
    ///
    /// # Panics
    /// If the write leaves the object or crosses a grain, which a fixture caller should not do.
    pub fn poke(&self, at: u64, bytes: &[u8]) {
        assert!(
            at + bytes.len() as u64 <= self.len,
            "poke outside the object"
        );
        let base = at & !(FAKE_GRAIN - 1);
        assert_eq!(
            base,
            (at + bytes.len() as u64 - 1) & !(FAKE_GRAIN - 1),
            "the fixture pokes one grain at a time"
        );
        let mut g = self.grains.lock().unwrap();
        let page = g
            .entry(base)
            .or_insert_with(|| vec![0u8; FAKE_GRAIN as usize]);
        let off = (at - base) as usize;
        page[off..off + bytes.len()].copy_from_slice(bytes);
    }

    /// Read the object directly, past every armed-run check — the oracle a test compares
    /// against.
    #[must_use]
    pub fn peek(&self, at: u64, len: usize) -> Vec<u8> {
        let base = at & !(FAKE_GRAIN - 1);
        let g = self.grains.lock().unwrap();
        match g.get(&base) {
            Some(p) => p[(at - base) as usize..(at - base) as usize + len].to_vec(),
            None => vec![0u8; len],
        }
    }

    /// Arm a run without going through the demand set.
    pub fn arm(&self, at: u64) {
        self.armed.lock().unwrap().insert(at & !(FAKE_GRAIN - 1));
    }

    /// Make every later drain decline by name.
    pub fn set_declining(&self, yes: bool) {
        self.decline.store(yes, Ordering::Relaxed);
    }

    /// Make every later arm refuse.
    pub fn set_refusing(&self, yes: bool) {
        self.refuse.store(yes, Ordering::Relaxed);
    }

    /// `(drains, declined, armed, served_read, served_write, wanted_read, wanted_write)`.
    #[must_use]
    pub fn counts(&self) -> (u64, u64, u64, u64, u64, u64, u64) {
        (
            self.drains.load(Ordering::Relaxed),
            self.declined.load(Ordering::Relaxed),
            self.armed_total.load(Ordering::Relaxed),
            self.served_read.load(Ordering::Relaxed),
            self.served_write.load(Ordering::Relaxed),
            self.wanted_read.load(Ordering::Relaxed),
            self.wanted_write.load(Ordering::Relaxed),
        )
    }

    /// How many runs are wanted and not yet armed.
    #[must_use]
    pub fn wanted_now(&self) -> usize {
        self.want.lock().unwrap().len()
    }

    /// How many runs are armed right now.
    #[must_use]
    pub fn armed_now(&self) -> usize {
        self.armed.lock().unwrap().len()
    }

    fn span(&self, at: u64, len: u64) -> Option<(u64, u64)> {
        let end = at.checked_add(len)?;
        if len == 0 || end > self.len {
            return None;
        }
        Some((at & !(FAKE_GRAIN - 1), (end - 1) & !(FAKE_GRAIN - 1)))
    }

    fn all_armed(&self, first: u64, last: u64) -> bool {
        let a = self.armed.lock().unwrap();
        let mut b = first;
        while b <= last {
            if !a.contains(&b) {
                return false;
            }
            b += FAKE_GRAIN;
        }
        true
    }
}

impl DeviceFbPort for FakePort {
    fn read_armed(&self, at: u64, buf: &mut [u8]) -> bool {
        let Some((first, last)) = self.span(at, buf.len() as u64) else {
            return false;
        };
        if !self.all_armed(first, last) {
            return false;
        }
        let mut done = 0usize;
        let mut base = first;
        while base <= last {
            let start = at.max(base);
            let stop = (at + buf.len() as u64).min(base + FAKE_GRAIN);
            let n = (stop - start) as usize;
            buf[done..done + n].copy_from_slice(&self.peek(start, n));
            done += n;
            base += FAKE_GRAIN;
        }
        self.served_read.fetch_add(1, Ordering::Relaxed);
        true
    }

    fn write_armed(&self, at: u64, bytes: &[u8]) -> bool {
        let Some((first, last)) = self.span(at, bytes.len() as u64) else {
            return false;
        };
        if !self.all_armed(first, last) {
            return false;
        }
        let mut done = 0usize;
        let mut base = first;
        while base <= last {
            let start = at.max(base);
            let stop = (at + bytes.len() as u64).min(base + FAKE_GRAIN);
            let n = (stop - start) as usize;
            self.poke(start, &bytes[done..done + n]);
            done += n;
            base += FAKE_GRAIN;
        }
        self.served_write.fetch_add(1, Ordering::Relaxed);
        true
    }

    fn probe_or_want(&self, at: u64, len: u64, by: DeviceFbWant) -> bool {
        self.probes.fetch_add(1, Ordering::Relaxed);
        let Some((first, last)) = self.span(at, len.max(1)) else {
            return false;
        };
        if self.all_armed(first, last) {
            return true;
        }
        DeviceFbPort::want(self, at, len, by);
        false
    }

    fn want(&self, at: u64, len: u64, by: DeviceFbWant) {
        match by {
            DeviceFbWant::Read => &self.wanted_read,
            DeviceFbWant::Write => &self.wanted_write,
        }
        .fetch_add(1, Ordering::Relaxed);
        let Some((first, last)) = self.span(at, len.max(1)) else {
            return;
        };
        let mut w = self.want.lock().unwrap();
        let mut b = first;
        while b <= last {
            w.insert(b);
            b += FAKE_GRAIN;
        }
    }

    fn drain(&self) -> DeviceFbDrained {
        if self.decline.load(Ordering::Relaxed) {
            self.declined.fetch_add(1, Ordering::Relaxed);
            return DeviceFbDrained::declined();
        }
        self.drains.fetch_add(1, Ordering::Relaxed);
        let batch: Vec<u64> = {
            let mut w = self.want.lock().unwrap();
            let all: Vec<u64> = w.iter().copied().collect();
            w.clear();
            all
        };
        let mut out = DeviceFbDrained::default();
        for b in batch {
            if self.refuse.load(Ordering::Relaxed) {
                out.refused += 1;
                continue;
            }
            if self.armed.lock().unwrap().insert(b) {
                self.armed_total.fetch_add(1, Ordering::Relaxed);
                out.armed += 1;
            }
        }
        out
    }

    fn census_line(&self) -> String {
        format!("FAKE-PORT {:?}", self.counts())
    }
}
