//! The real [`Backend`]: `kf_host::HostRm` for the RM verbs, `kf_harness::CeRig` for the copy-engine
//! oracle, and `kf_linux_raw` regions for the memory. **Safe code only**: the two OS calls that needed
//! `unsafe` (`madvise(NOHUGEPAGE|DONTNEED)`) live in `kf-linux-raw` (`MappedRegion::advise`, in
//! `mapping_unsafe.rs`), and the `/proc` reads are `std::fs`.
//!
//! ⊘ **STATUS: written against the real APIs and compiled, never run on a GPU.** This agent had no
//! hardware. Everything here is UNSURE until step S3 runs it on a box with `KF3_MAPLOG=1`
//! (V3_RAW_CLIENT.md §4.9 F1): in particular whether `MapBacking::SharedSlice` is right for a fresh OS
//! descriptor (the design's own UNSURE), and whether `HostRm::open` works from guest userspace for a
//! copy/map workload (it has so far only been run in a guest by `kf-syncwait-stress`).
//!
//! What is **stubbed behind the trait** because `kf-host` has no such verb: [`Backend::probe_free`]
//! returns [`Probe::Unsupported`] (no `VASPACE_GET_PAGE_LEVEL_INFO` wrapper exists yet), so the arm
//! reports the unmap postcondition as `SKIP`. Nothing here invents an ioctl.

use super::stress::backend::{Backend, BackendError, Probe, Setup};
use super::stress::census::PageFrame;
use super::stress::params::Shape;
use super::stress::plan::parse_mem_available;
use super::stress::rng::SplitMix64;
use crate::caps::{HostFacts, VaBand};
use crate::clock::Clock;
use crate::outcome::NotRunReason;
use kf_harness::CeRig;
use kf_host::{HostRm, MapBacking, VaSpace};
use kf_linux_raw::pagemap::{PagemapEntry, pfns_visible};
use kf_linux_raw::{
    Backing, CachePolicy, DevDir, HostOffset, HostPageSize, HostProt, MappedRegion, RegionAdvice,
};

/// 2 MiB: the alignment the batch band starts on.
const BAND_ALIGN: u64 = 2 << 20;

/// The hardware backend for one GPU minor.
pub struct HwBackend {
    gpu: u32,
    rm: Option<HostRm>,
    space: Option<VaSpace>,
    rig: Option<CeRig>,
    setup: Option<Setup>,
    band_base: u64,
    staging_va: u64,
    workload: Option<MappedRegion>,
    pressure: Option<MappedRegion>,
    staging: Option<MappedRegion>,
    staging_desc: Option<u32>,
    descs: Vec<Option<u32>>,
    page: HostPageSize,
}

fn unsupported(r: NotRunReason, d: impl Into<String>) -> BackendError {
    BackendError::Unsupported(r, d.into())
}

fn failed(what: &str, e: impl std::fmt::Debug) -> BackendError {
    BackendError::Failed(format!("{what}: {e:?}"))
}

/// Fill `buf` with `word` (little-endian) repeated.
fn fill_words(buf: &mut [u8], word: u64) {
    let (chunks, _) = buf.as_chunks_mut::<8>();
    for c in chunks {
        *c = word.to_le_bytes();
    }
}

impl HwBackend {
    /// A backend for GPU minor `gpu`. Nothing is opened until [`Backend::facts`].
    #[must_use]
    pub fn new(gpu: u32) -> Self {
        HwBackend {
            gpu,
            rm: None,
            space: None,
            rig: None,
            setup: None,
            band_base: 0,
            staging_va: 0,
            workload: None,
            pressure: None,
            staging: None,
            staging_desc: None,
            descs: Vec::new(),
            page: HostPageSize::query(),
        }
    }

    fn cfg(&self) -> Result<Setup, BackendError> {
        self.setup
            .ok_or_else(|| BackendError::failed("setup was not called"))
    }

    fn anon(&self, bytes: u64) -> Result<MappedRegion, BackendError> {
        let r = MappedRegion::map(
            Backing::PrivateAnonymous,
            bytes,
            HostProt::ReadWrite,
            CachePolicy::WriteBack,
            self.page,
        )
        .map_err(|e| failed("mmap", e))?;
        r.advise(HostOffset::ZERO, bytes, RegionAdvice::NoHugePage)
            .map_err(|e| failed("madvise(NOHUGEPAGE)", e))?;
        Ok(r)
    }

    /// Are page frame numbers readable? Map eight pages, touch them, read their pagemap.
    fn pagemap_pfn_visible(&self) -> Option<bool> {
        let bytes = 8 * self.page.bytes();
        let r = self.anon(bytes).ok()?;
        for i in 0..8 {
            r.write_from(HostOffset::new(i * self.page.bytes()), &[1])
                .ok()?;
        }
        let mut e = [PagemapEntry::default(); 8];
        r.pagemap(0, &mut e).ok()?;
        e.iter().all(|x| x.present).then(|| pfns_visible(&e))
    }

    fn kmod() -> String {
        match std::fs::read_to_string("/proc/driver/nvidia/version") {
            Ok(t) if t.contains("Open Kernel Module") => "open".into(),
            Ok(t) if t.contains("Kernel Module") => "proprietary".into(),
            _ => "unknown".into(),
        }
    }

    fn write_page(&mut self, page: u32, word: u64) -> Result<(), BackendError> {
        let s = self.cfg()?;
        let mut buf = vec![0u8; s.leaf_bytes as usize];
        fill_words(&mut buf, word);
        let r = self
            .workload
            .as_ref()
            .ok_or_else(|| BackendError::failed("no workload region"))?;
        r.write_from(HostOffset::new(u64::from(page) * s.leaf_bytes), &buf)
            .map_err(|e| failed("touch", e))
    }
}

impl Backend for HwBackend {
    fn facts(&mut self) -> Result<HostFacts, BackendError> {
        let dev = DevDir::open(c"/dev")
            .map_err(|e| unsupported(NotRunReason::NoRmDevice, format!("open /dev: {e:?}")))?;
        let rm = HostRm::open(
            &dev,
            kf_arch::ids::GpuId(self.gpu),
            &kf_chip::choose_host_classes,
        )
        .map_err(|e| {
            let r = if e.rung.starts_with("R0") || e.rung.starts_with("R1") {
                NotRunReason::NoRmDevice
            } else {
                NotRunReason::HostRefused
            };
            unsupported(r, format!("{} {}", e.rung, e.detail))
        })?;
        let space = rm.alloc_vaspace().map_err(|e| {
            unsupported(NotRunReason::NoGuestVaBand, format!("alloc_vaspace: {e:?}"))
        })?;
        // The first reserved guest range is the band: host-derived, not a constant. Start it on a 2 MiB
        // boundary so no batch window straddles a leaf of a larger page size.
        let band = space.guest.iter().find(|g| g.handle != 0).and_then(|g| {
            let base = g.lo.checked_add(BAND_ALIGN - 1)? & !(BAND_ALIGN - 1);
            (g.hi > base).then_some(VaBand {
                base,
                len: g.hi - base,
            })
        });
        let (architecture, implementation, _) = rm.arch_info();
        let facts = HostFacts {
            architecture,
            implementation,
            driver: rm.driver_version().to_owned(),
            kmod: Self::kmod(),
            host_page_bytes: self.page.bytes(),
            ce_class: Some(rm.ce_class_id()),
            va_band: band,
            pagemap_pfn_visible: self.pagemap_pfn_visible(),
        };
        self.band_base = band.map_or(0, |b| b.base);
        self.space = Some(space);
        self.rm = Some(rm);
        Ok(facts)
    }

    fn mem_available_bytes(&mut self) -> Result<u64, BackendError> {
        let t = std::fs::read_to_string("/proc/meminfo")
            .map_err(|e| unsupported(NotRunReason::ProcUnreadable, format!("meminfo: {e}")))?;
        parse_mem_available(&t)
            .ok_or_else(|| unsupported(NotRunReason::ProcUnreadable, "meminfo has no MemAvailable"))
    }

    fn setup(&mut self, s: &Setup) -> Result<(), BackendError> {
        let (rm, space) = match (self.rm.as_ref(), self.space) {
            (Some(rm), Some(sp)) => (rm, sp),
            _ => return Err(BackendError::failed("facts was not called")),
        };
        self.descs = vec![None; s.batches as usize];
        // The staging buffer sits in the band right after the last batch window.
        let batch_bytes = u64::from(s.pages_per_batch) * s.leaf_bytes;
        self.staging_va = self.band_base + u64::from(s.batches) * batch_bytes;
        let staging_bytes = u64::from(s.sample) * s.leaf_bytes;
        let staging = self.anon(staging_bytes)?;
        let mut poison = vec![0u8; s.leaf_bytes as usize];
        fill_words(&mut poison, 0xDEAD_BEEF_DEAD_BEEF);
        for slot in 0..u64::from(s.sample) {
            staging
                .write_from(HostOffset::new(slot * s.leaf_bytes), &poison)
                .map_err(|e| failed("poison staging", e))?;
        }
        let desc = rm
            .alloc_os_descriptor(&staging, HostOffset::ZERO, staging_bytes)
            .map_err(|e| failed("staging descriptor", e))?;
        self.staging_desc = Some(desc);
        let got = rm
            .map(
                space,
                desc,
                MapBacking::SharedSlice,
                0,
                staging_bytes,
                Some(self.staging_va),
                false,
            )
            .map_err(|e| failed("map staging", e))?;
        if got != self.staging_va {
            return Err(BackendError::failed(format!(
                "staging mapped at {got:#x}, wanted {:#x}",
                self.staging_va
            )));
        }
        self.staging = Some(staging);
        let rig =
            CeRig::new(rm, space).map_err(|e| BackendError::failed(format!("ce rig: {e}")))?;
        self.rig = Some(rig);
        self.setup = Some(*s);
        Ok(())
    }

    fn pressure_begin(&mut self, bytes: u64, seed: u64) -> Result<(), BackendError> {
        let region = self.anon(bytes)?;
        let page = self.page.bytes();
        let pages = bytes / page;
        for i in 0..pages {
            region
                .write_from(HostOffset::new(i * page), &[1])
                .map_err(|e| failed("pressure touch", e))?;
        }
        // Give back a seeded random half, as runs, so the free lists hold holes all over RAM.
        let mut rng = SplitMix64::new(seed);
        let mut run: Option<u64> = None;
        for i in 0..=pages {
            let drop_it = i < pages && rng.next_u64() & 1 == 1;
            match (drop_it, run) {
                (true, None) => run = Some(i),
                (false, Some(start)) => {
                    region
                        .advise(
                            HostOffset::new(start * page),
                            (i - start) * page,
                            RegionAdvice::DontNeed,
                        )
                        .map_err(|e| failed("pressure discard", e))?;
                    run = None;
                }
                _ => {}
            }
        }
        self.pressure = Some(region);
        Ok(())
    }

    fn alloc_workload(&mut self, pages: u32) -> Result<(), BackendError> {
        let s = self.cfg()?;
        self.workload = Some(self.anon(u64::from(pages) * s.leaf_bytes)?);
        Ok(())
    }

    fn touch(&mut self, page: u32, word: u64) -> Result<(), BackendError> {
        self.write_page(page, word)
    }

    fn pfns(&mut self, first: u32, out: &mut [PageFrame]) -> Result<(), BackendError> {
        let r = self
            .workload
            .as_ref()
            .ok_or_else(|| BackendError::failed("no workload region"))?;
        let mut e = vec![PagemapEntry::default(); out.len()];
        r.pagemap(u64::from(first), &mut e)
            .map_err(|er| unsupported(NotRunReason::ProcUnreadable, format!("pagemap: {er:?}")))?;
        for (o, x) in out.iter_mut().zip(&e) {
            *o = PageFrame {
                present: x.present && !x.swapped,
                pfn: x.pfn,
            };
        }
        Ok(())
    }

    fn huge_bytes(&mut self) -> Result<Option<u64>, BackendError> {
        Ok(self
            .workload
            .as_ref()
            .and_then(MappedRegion::anon_huge_bytes))
    }

    fn pressure_end(&mut self) {
        self.pressure = None;
    }

    fn pin_batch(&mut self, batch: u32) -> Result<(), BackendError> {
        let s = self.cfg()?;
        let bytes = u64::from(s.pages_per_batch) * s.leaf_bytes;
        let (Some(rm), Some(w)) = (self.rm.as_ref(), self.workload.as_ref()) else {
            return Err(BackendError::failed("no rm or workload"));
        };
        let h = rm
            .alloc_os_descriptor(w, HostOffset::new(u64::from(batch) * bytes), bytes)
            .map_err(|e| failed("alloc_os_descriptor", e))?;
        self.descs[batch as usize] = Some(h);
        Ok(())
    }

    fn map_batch(
        &mut self,
        batch: u32,
        clock: &dyn Clock,
        call_ns: &mut dyn FnMut(u64),
    ) -> Result<(), BackendError> {
        let s = self.cfg()?;
        let bytes = u64::from(s.pages_per_batch) * s.leaf_bytes;
        let va = self.band_base + u64::from(batch) * bytes;
        let (Some(rm), Some(space), Some(h)) = (
            self.rm.as_ref(),
            self.space,
            self.descs.get(batch as usize).copied().flatten(),
        ) else {
            return Err(BackendError::failed("map before pin"));
        };
        let mut one = |off: u64, len: u64, at: u64| -> Result<u64, BackendError> {
            let t0 = clock.now_ns();
            // `defer = true`: NVOS46 DEFER_TLB_INVALIDATION; the batch ends with ONE invalidate.
            let r = rm.map(space, h, MapBacking::SharedSlice, off, len, Some(at), true);
            call_ns(clock.now_ns().saturating_sub(t0));
            r.map_err(|e| failed("map_memory_dma", e))
        };
        match s.shape {
            Shape::Whole => {
                let got = one(0, bytes, va)?;
                if got != va {
                    return Err(BackendError::failed(format!(
                        "mapped at {got:#x}, wanted {va:#x}"
                    )));
                }
            }
            Shape::Sliced => {
                for i in 0..u64::from(s.pages_per_batch) {
                    one(i * s.leaf_bytes, s.leaf_bytes, va + i * s.leaf_bytes)?;
                }
            }
        }
        Ok(())
    }

    fn invalidate(&mut self) -> Result<(), BackendError> {
        let (Some(rm), Some(space)) = (self.rm.as_ref(), self.space) else {
            return Err(BackendError::failed("no rm"));
        };
        rm.invalidate_tlb(space)
            .map_err(|e| failed("invalidate_tlb", e))
    }

    fn copy_to_staging(&mut self, page: u32, slot: u32) -> Result<(), BackendError> {
        let s = self.cfg()?;
        let src = self.band_base + u64::from(page) * s.leaf_bytes;
        let dst = self.staging_va + u64::from(slot) * s.leaf_bytes;
        let mut poison = vec![0u8; s.leaf_bytes as usize];
        fill_words(&mut poison, 0xDEAD_BEEF_0000_0000 | u64::from(slot));
        let st = self
            .staging
            .as_ref()
            .ok_or_else(|| BackendError::failed("no staging"))?;
        st.write_from(HostOffset::new(u64::from(slot) * s.leaf_bytes), &poison)
            .map_err(|e| failed("poison slot", e))?;
        let (Some(rm), Some(rig)) = (self.rm.as_ref(), self.rig.as_mut()) else {
            return Err(BackendError::failed("no rig"));
        };
        let out = rig
            .copy(
                rm,
                src,
                dst,
                u32::try_from(s.leaf_bytes).unwrap_or(u32::MAX),
            )
            .map_err(|e| BackendError::failed(format!("ce copy: {e}")))?;
        if out.ce_released && out.fence_landed {
            Ok(())
        } else {
            Err(BackendError::failed(format!(
                "ce copy did not complete: ce_released={} fence_landed={} wakes={}",
                out.ce_released, out.fence_landed, out.wakes
            )))
        }
    }

    fn read_staging(&mut self, slot: u32, words: &mut [u64]) -> Result<(), BackendError> {
        let s = self.cfg()?;
        let mut buf = vec![0u8; words.len() * 8];
        self.staging
            .as_ref()
            .ok_or_else(|| BackendError::failed("no staging"))?
            .read_into(HostOffset::new(u64::from(slot) * s.leaf_bytes), &mut buf)
            .map_err(|e| failed("read staging", e))?;
        let (chunks, _) = buf.as_chunks::<8>();
        for (w, c) in words.iter_mut().zip(chunks) {
            *w = u64::from_le_bytes(*c);
        }
        Ok(())
    }

    fn rewrite_page(&mut self, page: u32, word: u64) -> Result<(), BackendError> {
        self.write_page(page, word)
    }

    fn unmap_batch(&mut self, batch: u32) -> Result<(), BackendError> {
        let s = self.cfg()?;
        let bytes = u64::from(s.pages_per_batch) * s.leaf_bytes;
        let va = self.band_base + u64::from(batch) * bytes;
        let (Some(rm), Some(space)) = (self.rm.as_ref(), self.space) else {
            return Err(BackendError::failed("no rm"));
        };
        // Non-deferred: each unmap invalidates itself. UNSURE whether the guest RM defers its unmaps
        // (V3_RAW_CLIENT.md §4.3 records only the maps as deferred).
        let r = match s.shape {
            Shape::Whole => rm.unmap_row(space, va, bytes, false),
            Shape::Sliced => rm.unmap_range(space, va, bytes, false),
        };
        r.map_err(|e| failed("unmap_dma", e))
    }

    fn probe_free(&mut self, _page: u32) -> Result<Probe, BackendError> {
        // No `VASPACE_GET_PAGE_LEVEL_INFO` wrapper in kf-host (V3_RAW_CLIENT.md §2.3.1): nothing to ask.
        Ok(Probe::Unsupported)
    }

    fn free_batch(&mut self, batch: u32) -> Result<(), BackendError> {
        let Some(rm) = self.rm.as_ref() else {
            return Err(BackendError::failed("no rm"));
        };
        match self.descs.get_mut(batch as usize).and_then(Option::take) {
            Some(h) => rm.free(h).map_err(|e| failed("free descriptor", e)),
            None => Ok(()),
        }
    }

    fn release_workload(&mut self) {
        self.workload = None;
    }

    fn teardown(&mut self) {
        self.rig = None;
        if let (Some(rm), Some(h)) = (self.rm.as_ref(), self.staging_desc.take()) {
            let _ = rm.free(h);
        }
        self.staging = None;
        if let (Some(rm), Some(space)) = (self.rm.as_ref(), self.space.take()) {
            rm.free_vaspace(space);
        }
        self.workload = None;
        self.pressure = None;
    }
}
