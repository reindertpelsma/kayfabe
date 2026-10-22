//! Translated RC destinations and allocation lifetime, independent of GPU execution.

use crate::mem::RamMap;
use kf_arch::fault::ErrorNotifier;
use kf_host::{CpuViewRelease, HostRm, MapNode, ViewAccess};
use kf_linux_raw::{Backing, CachePolicy, HostOffset, HostPageSize, VolatileRegion};
use kf_rm::memory_list::{GuestRamAuthority, RamSpan};
use std::sync::{Arc, RwLock, TryLockError};

/// A posting permit serializes event publication with retirement, not just a racy bool check.
#[derive(Debug, Clone)]
pub(crate) struct Life(Arc<RwLock<bool>>);

pub(crate) enum Post<T> {
    Posted(T),
    Retired,
    Busy,
}

impl Life {
    pub(crate) fn new() -> Self {
        Self(Arc::new(RwLock::new(true)))
    }
    // Act thread only; no vCPU ever takes this lock. Holders perform just the queue post.
    pub(crate) fn retire(&self) {
        match self.0.write() {
            Ok(mut live) => *live = false,
            Err(poisoned) => *poisoned.into_inner() = false,
        }
    }
    pub(crate) fn keep_pending(&self) -> bool {
        match self.0.try_read() {
            Ok(live) => *live,
            Err(TryLockError::WouldBlock) => true,
            Err(TryLockError::Poisoned(_)) => false,
        }
    }
    pub(crate) fn post<T>(&self, f: impl FnOnce() -> T) -> Post<T> {
        match self.0.try_read() {
            Ok(live) if *live => {
                let result = f();
                drop(live); // The posting permission covers the actual post.
                Post::Posted(result)
            }
            Ok(_) | Err(TryLockError::Poisoned(_)) => Post::Retired,
            Err(TryLockError::WouldBlock) => Post::Busy,
        }
    }
}

/// Whole-record bounds before any map or store. The view may cover a page boundary.
pub(crate) fn fb_span(
    layout: &kf_chip::bar0::FbLayout,
    off: u64,
) -> Result<(u64, u64, u64), String> {
    if !off.is_multiple_of(4) || !layout.in_usable_heap(off, 16) {
        return Err(format!(
            "RC notifier {off:#x}+0x10 is unaligned or outside the usable heap"
        ));
    }
    let end = off.checked_add(16).ok_or("RC notifier overflow")?;
    let page = off & !0xfff;
    let mapped_end = end.checked_add(0xfff).ok_or("RC notifier page overflow")? & !0xfff;
    let len = mapped_end
        .checked_sub(page)
        .ok_or("RC notifier view underflow")?;
    if mapped_end > layout.fb_length || len > 0x2000 {
        return Err("RC notifier view is outside the store".into());
    }
    Ok((page, len, off.saturating_sub(page)))
}

/// Pins the allocation-time topology; each bounded store holds its RAM authority guard.
struct RamTarget<A> {
    authority: A,
    span: RamSpan,
    generation: u64,
}

impl<A: GuestRamAuthority> RamTarget<A> {
    fn arm(authority: A, span: RamSpan) -> Result<Self, String> {
        let generation = authority
            .validate(span)
            .filter(|g| *g != 0)
            .ok_or("RC notifier is not current writable guest RAM")?;
        Ok(Self {
            authority,
            span,
            generation,
        })
    }

    fn store(&self, offset: u64, value: u32) -> Result<(), String> {
        if !matches!(offset, 0 | 4 | 8 | 12) {
            return Err("RC store outside its 16-byte record".into());
        }
        self.authority
            .try_write(self.span, self.generation, offset, &value.to_le_bytes())
            .map_err(|e| format!("RC RAM authority: {e:?}"))
    }
}

enum View {
    Ram(RamTarget<crate::mem::MemoryListRam>),
    Store {
        region: VolatileRegion,
        _node: kf_linux_raw::CharDevice,
        cookie: u64,
        at: u64,
    },
}

/// A bounded, allocation-time guest destination. It is never a source of host errors.
pub(crate) struct GuestNotifier {
    view: Option<View>,
    rm: &'static HostRm,
    store: u32,
}

impl GuestNotifier {
    pub(crate) fn arm(
        rm: &'static HostRm,
        ram: &'static RamMap,
        store: u32,
        layout: &kf_chip::bar0::FbLayout,
        notifier: Option<ErrorNotifier>,
    ) -> Result<Self, String> {
        let view = match notifier {
            None => None,
            Some(ErrorNotifier::Unreachable) => {
                return Err("RC notifier aperture is unreachable".into());
            }
            Some(ErrorNotifier::Sysmem { gpa }) => {
                if !gpa.is_multiple_of(4) {
                    return Err("RC notifier DMA address is unaligned".into());
                }
                ram.dma_to_file_range(gpa, 16)
                    .ok_or("RC notifier has no complete guest-RAM DMA range")?;
                Some(View::Ram(RamTarget::arm(
                    crate::mem::MemoryListRam(ram),
                    RamSpan {
                        base: gpa,
                        length: 16,
                    },
                )?))
            }
            Some(ErrorNotifier::Framebuffer { off }) => {
                let (page, len, at) = fb_span(layout, off)?;
                let (node, cookie) = rm
                    .arm_cpu_view(MapNode::Gpu, store, page, len, ViewAccess::ReadWrite)
                    .map_err(|e| format!("RC notifier CPU view: {e:?}"))?;
                let region = match VolatileRegion::map(
                    Backing::DeviceFile { fd: node.as_fd() },
                    len,
                    CachePolicy::Uncached,
                    HostPageSize::query(),
                ) {
                    Ok(region) => region,
                    Err(e) => {
                        let _ = rm.release_cpu_view(CpuViewRelease {
                            h_memory: store,
                            p_linear_address: cookie,
                        });
                        return Err(format!("RC notifier mmap: {e:?}"));
                    }
                };
                Some(View::Store {
                    region,
                    _node: node,
                    cookie,
                    at,
                })
            }
        };
        Ok(Self { view, rm, store })
    }

    pub(crate) fn store(&self, offset: u64, value: u32) -> Result<(), String> {
        if !matches!(offset, 0 | 4 | 8 | 12) {
            return Err("RC store outside its 16-byte record".into());
        }
        match self.view.as_ref() {
            None => Ok(()), // The guest explicitly waived its notifier; the event still goes out.
            Some(View::Ram(target)) => target.store(offset, value),
            Some(View::Store { region, at, .. }) => {
                let at = at.checked_add(offset).ok_or("RC store overflow")?;
                region
                    .store_u32(HostOffset::new(at), value)
                    .map_err(|e| format!("RC store: {e:?}"))
            }
        }
    }
}

impl Drop for GuestNotifier {
    fn drop(&mut self) {
        if let Some(View::Store {
            region,
            cookie,
            _node,
            ..
        }) = self.view.take()
        {
            drop(region); // Unmap the CPU address before returning its host aperture.
            let _ = self.rm.release_cpu_view(CpuViewRelease {
                h_memory: self.store,
                p_linear_address: cookie,
            });
            drop(_node);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> kf_chip::bar0::FbLayout {
        kf_chip::bar0::fb_layout(1 << 30).unwrap()
    }

    #[test]
    fn a_retired_allocation_cannot_post_even_with_a_retained_queue_reference() {
        let life = Life::new();
        let queued = life.clone();
        life.retire();
        assert!(!queued.keep_pending());
        assert!(matches!(
            queued.post(|| panic!("retired event posted")),
            Post::Retired
        ));
    }

    #[test]
    fn reusing_a_chid_does_not_revive_the_old_allocations_event() {
        let old = Life::new();
        old.retire();
        let replacement = Life::new();
        assert!(matches!(old.post(|| 1), Post::Retired));
        assert!(matches!(replacement.post(|| 2), Post::Posted(2)));
    }

    #[test]
    fn the_posting_permit_covers_the_actual_queue_write() {
        let life = Life::new();
        assert!(matches!(
            life.post(|| {
                assert!(
                    life.0.try_write().is_err(),
                    "retirement must wait for the queue write"
                );
                7
            }),
            Post::Posted(7)
        ));
        assert!(life.0.try_write().is_ok());
    }

    #[test]
    fn a_busy_retirement_lock_keeps_the_event_for_retry_without_posting() {
        let life = Life::new();
        let lock = life.0.write().unwrap();
        assert!(life.keep_pending(), "contention is not retirement");
        assert!(matches!(
            life.post(|| panic!("busy permit posted")),
            Post::Busy
        ));
        drop(lock);
        assert!(matches!(life.post(|| 9), Post::Posted(9)));
    }

    #[test]
    fn a_notifier_may_cross_a_page_without_truncating_its_record() {
        let l = layout();
        assert_eq!(fb_span(&l, 0x2ffc).unwrap(), (0x2000, 0x2000, 0xffc));
        assert_eq!(fb_span(&l, 0x2000).unwrap(), (0x2000, 0x1000, 0));
    }

    #[test]
    fn the_firmware_carveout_is_never_a_guest_notifier_destination() {
        let l = layout();
        for off in [l.carve(), l.fb_length - 16] {
            assert!(fb_span(&l, off).is_err(), "{off:#x}");
        }
    }

    #[test]
    fn a_record_straddling_the_heap_end_is_refused_whole() {
        let l = layout();
        assert!(fb_span(&l, l.carve() - 4).is_err());
        assert!(fb_span(&l, l.carve() - 8).is_err());
        assert!(fb_span(&l, l.carve() - 16).is_ok());
    }

    #[test]
    fn hostile_overflow_unaligned_and_out_of_store_offsets_are_refused() {
        let l = layout();
        for off in [u64::MAX, u64::MAX - 3, l.fb_length, 0x2001] {
            assert!(fb_span(&l, off).is_err());
        }
    }
}

#[cfg(test)]
mod ram_tests {
    use super::*;
    use kf_rm::memory_list::AccessError;
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    };

    #[derive(Debug, Default)]
    struct Authority {
        generation: AtomicU64,
        busy: AtomicBool,
        bytes: Mutex<[u8; 16]>,
    }
    #[derive(Debug, Clone)]
    struct Port(Arc<Authority>);
    const SPAN: RamSpan = RamSpan {
        base: 0x1000,
        length: 16,
    };
    impl GuestRamAuthority for Port {
        fn validate(&self, span: RamSpan) -> Option<u64> {
            (span == SPAN).then(|| self.0.generation.load(Ordering::Acquire))
        }
        fn read(&self, _: RamSpan, _: u64, _: u64, _: &mut [u8]) -> bool {
            false
        }
        fn write(&self, span: RamSpan, generation: u64, offset: u64, bytes: &[u8]) -> bool {
            self.try_write(span, generation, offset, bytes).is_ok()
        }
        fn try_write(
            &self,
            span: RamSpan,
            generation: u64,
            offset: u64,
            bytes: &[u8],
        ) -> Result<(), AccessError> {
            if self.0.busy.load(Ordering::Acquire) {
                return Err(AccessError::Busy);
            }
            if generation != self.0.generation.load(Ordering::Acquire) {
                return Err(AccessError::Topology);
            }
            if span != SPAN {
                return Err(AccessError::Bounds);
            }
            let mut dst = self.0.bytes.lock().unwrap();
            let end = (offset as usize)
                .checked_add(bytes.len())
                .ok_or(AccessError::Bounds)?;
            dst.get_mut(offset as usize..end)
                .ok_or(AccessError::Bounds)?
                .copy_from_slice(bytes);
            Ok(())
        }
    }
    fn target() -> (Arc<Authority>, RamTarget<Port>) {
        let authority = Arc::new(Authority::default());
        authority.generation.store(1, Ordering::Release);
        let target = RamTarget::arm(Port(authority.clone()), SPAN).unwrap();
        (authority, target)
    }
    fn delivery() -> kf_chan::rc::RcDelivery {
        kf_chan::rc::RcDelivery::armed(
            kf_chan::rc::RcRoute {
                chid: 17,
                engine: 1,
                host_token: 4,
            },
            [0; 4],
        )
    }
    #[test]
    fn ram_revocation_cannot_redirect_a_pending_rc_into_replacement_backing() {
        let (authority, target) = target();
        let mut delivery = delivery();
        authority.generation.store(2, Ordering::Release);
        assert!(
            delivery
                .poll([1, 2, 32, 0xffff_0000], |off, v| target.store(off, v))
                .is_err()
        );
        assert!(delivery.failed());
        assert_eq!(*authority.bytes.lock().unwrap(), [0; 16]);
        assert!(
            delivery
                .poll([0; 4], |off, v| target.store(off, v))
                .is_err(),
            "never refresh old authority"
        );
        assert_eq!(*authority.bytes.lock().unwrap(), [0; 16]);
    }
    #[test]
    fn ram_topology_change_mid_publication_never_writes_status_or_posts_an_event() {
        let (authority, target) = target();
        let mut delivery = delivery();
        let result = delivery.poll([1, 2, 32, 0xffff_0000], |offset, value| {
            if offset == 8 {
                authority.generation.store(2, Ordering::Release);
                *authority.bytes.lock().unwrap() = [0; 16];
            }
            target.store(offset, value)
        });
        assert!(result.is_err());
        assert!(delivery.failed());
        assert_eq!(*authority.bytes.lock().unwrap(), [0; 16]);
    }

    #[test]
    fn transient_ram_contention_retries_the_original_error_before_its_event() {
        let (authority, target) = target();
        let mut delivery = delivery();
        authority.busy.store(true, Ordering::Release);
        assert!(
            delivery
                .poll([1, 2, 32, 0xffff_0000], |off, v| target.store(off, v))
                .is_err()
        );
        assert_eq!(*authority.bytes.lock().unwrap(), [0; 16]);
        authority.busy.store(false, Ordering::Release);
        let event = delivery
            .poll([0; 4], |off, v| target.store(off, v))
            .unwrap()
            .unwrap();
        assert_eq!(event.except_type, 32);
        let bytes = *authority.bytes.lock().unwrap();
        assert_eq!(&bytes[8..12], &32u32.to_le_bytes());
        assert_eq!(&bytes[12..], &0xffff_0001u32.to_le_bytes());
    }
    #[test]
    fn ram_target_rejects_offsets_outside_the_notifier_before_the_authority_is_called() {
        let (authority, target) = target();
        for off in [1, 13, 16, u64::MAX] {
            assert!(target.store(off, 32).is_err())
        }
        assert_eq!(*authority.bytes.lock().unwrap(), [0; 16]);
    }
}
