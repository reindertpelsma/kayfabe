//! A fake [`Backend`] for the GPU-free tests. It models the parts of the world the oracle depends on:
//! what is in each page, which VA maps which page, when a mapping becomes live (at the invalidate), and
//! what the GPU copy returns. It can be told to misbehave in the specific ways the oracle exists to
//! catch, and it **enforces the protocol the arm promises**: a copy before the invalidate, or from an
//! unmapped VA, is an error (on hardware that is a deferred-PTE miss or an Xid 31).

use super::backend::{Backend, BackendError, Probe, Setup};
use super::census::PageFrame;
use crate::caps::{HostFacts, test_facts};
use crate::clock::Clock;
use std::cell::Cell;
use std::rc::Rc;

/// A clock the fake advances.
#[derive(Clone)]
pub struct FakeClock(pub Rc<Cell<u64>>);

impl FakeClock {
    pub fn new() -> Self {
        FakeClock(Rc::new(Cell::new(1_000)))
    }
}

impl Clock for FakeClock {
    fn now_ns(&self) -> u64 {
        self.0.get()
    }
}

/// How physical frames are laid out for the workload.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Frag {
    /// No two neighbours in mapping order are neighbours in memory.
    Scattered,
    /// One contiguous run.
    Contiguous,
    /// Present pages read frame 0.
    Hidden,
}

/// A way for the fake world to be wrong.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Fault {
    /// Behaves.
    None,
    /// Every mapped page shows the NEXT page's content: a map that succeeds but points at the wrong page.
    WrongPage,
    /// From the second refresh on, the GPU still sees the PREVIOUS refresh's content: a stale translation.
    Stale,
    /// The GPU reads a snapshot taken at the invalidate and never sees the CPU's later stores.
    IgnoreRewrite,
    /// The GPU returns zeros.
    Zeros,
    /// The map of this batch is refused.
    MapRefused(u32),
    /// The unmap leaves a mapping behind (the probe reports it).
    LeakMapping,
}

/// Simulated nanoseconds each call costs.
#[derive(Clone, Copy, Debug)]
pub struct Costs {
    pub pin: u64,
    pub map: u64,
    pub inval: u64,
    pub copy: u64,
    pub unmap: u64,
    pub free: u64,
}

impl Default for Costs {
    fn default() -> Self {
        Costs {
            pin: 100_000,
            map: 2_000_000,
            inval: 500_000,
            copy: 50_000,
            unmap: 300_000,
            free: 100_000,
        }
    }
}

pub struct Fake {
    pub clock: FakeClock,
    pub facts: HostFacts,
    pub avail: u64,
    pub frag: Frag,
    pub fault: Fault,
    pub costs: Costs,
    pub thp: Option<u64>,
    pub probe: Probe,
    pub fail_facts: Option<BackendError>,
    /// Every call, in order.
    pub log: Vec<String>,
    setup: Option<Setup>,
    epoch: u32,
    /// content of each page now (one word stands for the whole page)
    word: Vec<u64>,
    /// content of each page in the previous refresh
    prev: Vec<u64>,
    /// the GPU's snapshot at the last invalidate
    snap: Vec<u64>,
    pinned: Vec<bool>,
    mapped: Vec<bool>,
    live: bool,
    staging: Vec<u64>,
    pub teardown_calls: u32,
    pub invalidates_this_refresh: Vec<u32>,
    inval_count: u32,
}

impl Fake {
    pub fn new() -> Self {
        Fake {
            clock: FakeClock::new(),
            facts: test_facts(),
            avail: 8 << 30,
            frag: Frag::Scattered,
            fault: Fault::None,
            costs: Costs::default(),
            thp: Some(0),
            probe: Probe::Free,
            fail_facts: None,
            log: Vec::new(),
            setup: None,
            epoch: 0,
            word: Vec::new(),
            prev: Vec::new(),
            snap: Vec::new(),
            pinned: Vec::new(),
            mapped: Vec::new(),
            live: false,
            staging: Vec::new(),
            teardown_calls: 0,
            invalidates_this_refresh: Vec::new(),
            inval_count: 0,
        }
    }

    fn tick(&self, ns: u64) {
        self.clock.0.set(self.clock.0.get() + ns);
    }

    fn s(&self) -> Setup {
        self.setup.expect("setup was called")
    }

    pub fn count(&self, prefix: &str) -> usize {
        self.log.iter().filter(|l| l.starts_with(prefix)).count()
    }

    fn page_of(&self, batch: u32) -> std::ops::Range<usize> {
        let s = self.s();
        (batch * s.pages_per_batch) as usize..((batch + 1) * s.pages_per_batch) as usize
    }
}

impl Backend for Fake {
    fn facts(&mut self) -> Result<HostFacts, BackendError> {
        self.log.push("facts".into());
        match self.fail_facts.clone() {
            Some(e) => Err(e),
            None => Ok(self.facts.clone()),
        }
    }

    fn mem_available_bytes(&mut self) -> Result<u64, BackendError> {
        Ok(self.avail)
    }

    fn setup(&mut self, s: &Setup) -> Result<(), BackendError> {
        self.log.push("setup".into());
        self.setup = Some(*s);
        self.pinned = vec![false; s.batches as usize];
        self.mapped = vec![false; s.batches as usize];
        self.staging = vec![0; s.sample as usize];
        Ok(())
    }

    fn pressure_begin(&mut self, bytes: u64, _seed: u64) -> Result<(), BackendError> {
        self.log.push(format!("pressure_begin {bytes}"));
        Ok(())
    }

    fn alloc_workload(&mut self, pages: u32) -> Result<(), BackendError> {
        self.log.push(format!("alloc_workload {pages}"));
        self.word = vec![0; pages as usize];
        self.live = false;
        self.invalidates_this_refresh.push(0);
        Ok(())
    }

    fn touch(&mut self, page: u32, word: u64) -> Result<(), BackendError> {
        self.word[page as usize] = word;
        Ok(())
    }

    fn pfns(&mut self, first: u32, out: &mut [PageFrame]) -> Result<(), BackendError> {
        for (i, f) in out.iter_mut().enumerate() {
            let page = u64::from(first) + i as u64;
            *f = PageFrame {
                present: true,
                pfn: match self.frag {
                    Frag::Scattered => 100_000 + page * 3,
                    Frag::Contiguous => 100_000 + page,
                    Frag::Hidden => 0,
                },
            };
        }
        Ok(())
    }

    fn huge_bytes(&mut self) -> Result<Option<u64>, BackendError> {
        Ok(self.thp)
    }

    fn pressure_end(&mut self) {
        self.log.push("pressure_end".into());
    }

    fn pin_batch(&mut self, batch: u32) -> Result<(), BackendError> {
        self.log.push(format!("pin {batch}"));
        self.tick(self.costs.pin);
        self.pinned[batch as usize] = true;
        Ok(())
    }

    fn map_batch(
        &mut self,
        batch: u32,
        _clock: &dyn Clock,
        call_ns: &mut dyn FnMut(u64),
    ) -> Result<(), BackendError> {
        self.log.push(format!("map {batch}"));
        assert!(self.pinned[batch as usize], "map of an unpinned batch");
        if self.fault == Fault::MapRefused(batch) {
            return Err(BackendError::failed("NV_ERR_INVALID_ARGUMENT"));
        }
        let calls = match self.s().shape {
            super::params::Shape::Whole => 1,
            super::params::Shape::Sliced => self.s().pages_per_batch,
        };
        for _ in 0..calls {
            let c = self.costs.map / u64::from(calls);
            self.tick(c);
            call_ns(c);
        }
        self.mapped[batch as usize] = true;
        // A deferred PTE is not live until the invalidate.
        self.live = false;
        Ok(())
    }

    fn invalidate(&mut self) -> Result<(), BackendError> {
        self.log.push("invalidate".into());
        self.tick(self.costs.inval);
        self.live = true;
        self.snap = self.word.clone();
        self.inval_count += 1;
        if let Some(last) = self.invalidates_this_refresh.last_mut() {
            *last += 1;
        }
        Ok(())
    }

    fn copy_to_staging(&mut self, page: u32, slot: u32) -> Result<(), BackendError> {
        self.log.push(format!("copy {page} {slot}"));
        let batch = page / self.s().pages_per_batch;
        if !self.mapped[batch as usize] {
            return Err(BackendError::failed("read of an unmapped VA (host Xid 31)"));
        }
        if !self.live {
            return Err(BackendError::failed(
                "read of a mapped VA before the invalidate (deferred PTE not live)",
            ));
        }
        self.tick(self.costs.copy);
        let total = self.word.len() as u32;
        let seen = match self.fault {
            Fault::WrongPage => self.word[((page + 1) % total) as usize],
            Fault::Stale if self.epoch > 0 && !self.prev.is_empty() => self.prev[page as usize],
            Fault::Zeros => 0,
            Fault::IgnoreRewrite => self.snap[page as usize],
            _ => self.word[page as usize],
        };
        self.staging[slot as usize] = seen;
        Ok(())
    }

    fn read_staging(&mut self, slot: u32, words: &mut [u64]) -> Result<(), BackendError> {
        words.fill(self.staging[slot as usize]);
        Ok(())
    }

    fn rewrite_page(&mut self, page: u32, word: u64) -> Result<(), BackendError> {
        self.log.push(format!("rewrite {page}"));
        self.word[page as usize] = word;
        Ok(())
    }

    fn unmap_batch(&mut self, batch: u32) -> Result<(), BackendError> {
        self.log.push(format!("unmap {batch}"));
        self.tick(self.costs.unmap);
        if self.fault != Fault::LeakMapping {
            self.mapped[batch as usize] = false;
        }
        Ok(())
    }

    fn probe_free(&mut self, page: u32) -> Result<Probe, BackendError> {
        self.log.push(format!("probe {page}"));
        let batch = page / self.s().pages_per_batch;
        if self.fault == Fault::LeakMapping && self.mapped[batch as usize] {
            return Ok(Probe::Mapped);
        }
        Ok(self.probe)
    }

    fn free_batch(&mut self, batch: u32) -> Result<(), BackendError> {
        self.log.push(format!("free {batch}"));
        self.tick(self.costs.free);
        self.pinned[batch as usize] = false;
        self.mapped[batch as usize] = false;
        let _ = self.page_of(batch);
        Ok(())
    }

    fn release_workload(&mut self) {
        if !self.word.is_empty() {
            self.log.push("release_workload".into());
            self.prev = std::mem::take(&mut self.word);
            self.epoch += 1;
        }
    }

    fn teardown(&mut self) {
        self.log.push("teardown".into());
        self.teardown_calls += 1;
    }
}
