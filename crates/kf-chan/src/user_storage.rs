//! Private read-only command storage for the restricted Windows relay (§AI).
//!
//! This is a host resource primitive, NOT connected to VM channel birth. The
//! future VA-manager adapter must require WindowsUserRingPermit, reserve its
//! slot in the mirror ownership ledger before calling this, and retain that
//! reservation on any release failure. Linux Passthrough has no caller.
//!
//! Clear every byte before GPU exposure. Only commands/GP entries are mapped,
//! always read-only; private USERD is outside the GPU mapping. There is no gate,
//! writable fence, GPU-work completion or raw guest descriptor submission here.
//! The caller must quiesce/free its native channel before releasing storage.

use kf_host::{HostRm, MapNode, VaSpace, ViewAccess};
use kf_linux_raw::{Backing, CachePolicy, CharDevice, HostOffset, HostPageSize, VolatileRegion};

/// Trusted per-channel allocation policy, not a per-die fact.
pub const BYTES: u64 = 1 << 20;
/// Pushbuffer region ends here; whole64KiB granules permit RO-only mapping.
pub const GP_OFFSET: u64 = 0xe0000;
/// USERD starts here and has NO GPU VA mapping.
pub const USERD_OFFSET: u64 = 0xf0000;
/// Trusted private GP capacity, independent of the guest ring's changing size.
pub const ENTRIES: u32 = 512;

/// Resource verbs, separated so cleanup failures are checkable without a GPU.
/// Production calls must run on the VA-manager side, never a vCPU/trap.
pub trait StorageHost {
    /// CPU-view lifetime kept until explicit revoke; never a guest pointer.
    type View;
    /// Reserve one exclusive fixed VA slot; never route maps through guest ranges.
    fn reserve(&self, space: VaSpace, va: u64) -> Result<u32, String>;
    /// Allocate exactly the trusted object size.
    fn alloc(&self) -> Result<u32, String>;
    /// Arm/map a private CPU view and preserve its release cookie.
    fn view(&self, memory: u32) -> Result<Self::View, String>;
    /// Clear ALL bytes before any GPU map is installed.
    fn clear(&self, view: &Self::View) -> Result<(), String>;
    /// Install exactly `[0, USERD_OFFSET)` read-only at the reserved VA.
    fn map_ro(&self, reservation: u32, memory: u32, va: u64) -> Result<u64, MapFailure>;
    /// Revoke CPU mapping before returning its aperture to RM; retryable.
    fn revoke_view(&self, memory: u32, view: &mut Self::View) -> Result<(), String>;
    /// Remove the GPU map; failure retains the memory/VA ownership.
    fn unmap(&self, reservation: u32, va: u64) -> Result<(), String>;
    /// Free our reservation only after its whole range is unmapped.
    fn free_reservation(&self, reservation: u32) -> Result<(), String>;
    /// Free only after all mappings/views have successfully been revoked.
    fn free(&self, memory: u32) -> Result<(), String>;
}

/// A failed map may have reached RM; absence of a success is not an unmap.
pub struct MapFailure {
    /// Named refusal.
    pub reason: String,
    /// An explicitly reported unexpected placement, if available.
    pub unexpected: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mapping {
    None,
    Attempted,
    Live,
    /// A host placement outside our exclusive slot cannot be reclaimed by a
    /// range unmap of that slot. Retain until the owning RM session is closed.
    Outside(u64),
}

/// Partial construction stays owned on error. An external VA slot is reusable
/// only after `retained.released()`; an error/timer never grants that permission.
pub struct BuildError<V> {
    /// Named failure including any cleanup failure.
    pub reason: String,
    /// Live resources, if cleanup refused; caller/reaper must preserve them.
    pub retained: Storage<V>,
}

/// Private resource ownership, independent of GP progress or guest handle reuse.
pub struct Storage<V> {
    va: u64,
    reservation: Option<u32>,
    live: bool,
    memory: Option<u32>,
    view: Option<V>,
    mapped: Mapping,
}

impl<V> Storage<V> {
    /// Build over a trusted reserved slot. No guest-derived allocation size.
    ///
    /// # Errors
    /// Bounds/host/clear failure; any unreleased resources remain in BuildError.
    pub fn allocate(
        host: &impl StorageHost<View = V>,
        space: VaSpace,
        va: u64,
    ) -> Result<Self, BuildError<V>> {
        let mut out = Self {
            va,
            reservation: None,
            live: false,
            memory: None,
            view: None,
            mapped: Mapping::None,
        };
        let build = (|| {
            if va % 0x10000 != 0 || va.checked_add(BYTES).is_none_or(|end| end > 1 << 40) {
                return Err(
                    "USER_STORAGE_VA: reserved slot is not64KiB-aligned below2^40".to_string(),
                );
            }
            let reservation = host.reserve(space, va)?;
            out.reservation = Some(reservation);
            let memory = host.alloc()?;
            out.memory = Some(memory);
            out.view = Some(host.view(memory)?);
            host.clear(out.view.as_ref().ok_or("USER_STORAGE_VIEW absent")?)?;
            // Set BEFORE the call: an error can follow a successful ioctl.
            out.mapped = Mapping::Attempted;
            let actual = match host.map_ro(reservation, memory, va) {
                Ok(actual) => actual,
                Err(error) => {
                    if let Some(actual) = error.unexpected {
                        out.note_placement(actual);
                    }
                    return Err(error.reason);
                }
            };
            out.note_placement(actual);
            if actual != va {
                return Err(format!(
                    "USER_STORAGE_FIXED: got{actual:#x}, expected{va:#x}"
                ));
            }
            out.mapped = Mapping::Live;
            out.live = true;
            Ok(())
        })();
        match build {
            Ok(()) => Ok(out),
            Err(mut reason) => {
                if let Err(cleanup) = out.release(host) {
                    reason.push_str(&format!("; retained: {cleanup}"));
                }
                Err(BuildError {
                    reason,
                    retained: out,
                })
            }
        }
    }

    /// Resources are gone; only this permits the VA manager to recycle a slot.
    #[must_use]
    pub const fn released(&self) -> bool {
        self.memory.is_none()
            && self.view.is_none()
            && self.reservation.is_none()
            && matches!(self.mapped, Mapping::None)
    }

    /// Stable native ring/USERD specification; no guest descriptor is forwarded.
    ///
    /// # Errors
    /// Construction/release is incomplete.
    pub fn spec(&self, err_notifier: u32) -> Result<kf_host::RingSpec, String> {
        if !self.live || self.view.is_none() || self.mapped != Mapping::Live {
            return Err("USER_STORAGE_NOT_LIVE".into());
        }
        Ok(kf_host::RingSpec {
            gp_fifo_va: self.va + GP_OFFSET,
            gp_fifo_entries: ENTRIES,
            userd_memory: self.memory.ok_or("USER_STORAGE_NOT_LIVE")?,
            userd_offset: USERD_OFFSET,
            err_notifier,
        })
    }

    /// Release after native-channel quiescence/free, on the VA-manager side.
    /// Partial failures keep ownership and permit a later explicit retry.
    /// Dropping this Rust value does NOT free the RM object or its GPU mapping.
    ///
    /// # Errors
    /// First failed revoke/unmap/free; subsequent resources are not freed.
    pub fn release(&mut self, host: &impl StorageHost<View = V>) -> Result<(), String> {
        self.live = false; // cleanup retries never make storage usable again
        if let Some(view) = self.view.as_mut() {
            host.revoke_view(self.memory.ok_or("USER_STORAGE_NO_MEMORY")?, view)?;
            self.view = None;
        }
        if let Mapping::Outside(actual) = self.mapped {
            return Err(format!("USER_STORAGE_QUARANTINED_PLACEMENT: {actual:#x}"));
        }
        if self.mapped != Mapping::None {
            host.unmap(
                self.reservation.ok_or("USER_STORAGE_NO_RESERVATION")?,
                self.va,
            )?;
            self.mapped = Mapping::None;
        }
        if let Some(reservation) = self.reservation {
            host.free_reservation(reservation)?;
            self.reservation = None;
        }
        if let Some(memory) = self.memory {
            host.free(memory)?;
            self.memory = None;
        }
        Ok(())
    }

    fn note_placement(&mut self, actual: u64) {
        // Outside an exclusive reservation violates the RM placement contract.
        // Do not guess another object's ownership or free memory behind it.
        if actual < self.va
            || actual
                .checked_add(USERD_OFFSET)
                .is_none_or(|end| end > self.va + BYTES)
        {
            self.mapped = Mapping::Outside(actual);
        }
    }
}

/// Native CPU mapping plus a retryable RM-view release cookie.
pub struct CpuView {
    cpu: Option<VolatileRegion>,
    _node: CharDevice,
    cookie: Option<u64>,
    map_error: Option<String>,
}

impl StorageHost for HostRm {
    type View = CpuView;
    fn reserve(&self, space: VaSpace, va: u64) -> Result<u32, String> {
        self.reserve_va(space.space, va, BYTES)
            .map_err(|e| format!("USER_STORAGE_RESERVE: {e:?}"))
    }
    fn alloc(&self) -> Result<u32, String> {
        self.alloc_device_local(BYTES)
            .map_err(|e| format!("USER_STORAGE_ALLOC: {e:?}"))
    }
    fn view(&self, memory: u32) -> Result<CpuView, String> {
        let (node, cookie) = self
            .arm_cpu_view(MapNode::Gpu, memory, 0, BYTES, ViewAccess::ReadWrite)
            .map_err(|e| format!("USER_STORAGE_CPU_VIEW: {e:?}"))?;
        let (cpu, map_error) = match VolatileRegion::map(
            Backing::DeviceFile { fd: node.as_fd() },
            BYTES,
            CachePolicy::Uncached,
            HostPageSize::query(),
        ) {
            Ok(cpu) => (Some(cpu), None),
            Err(error) => (None, Some(format!("USER_STORAGE_CPU_MAP: {error:?}"))),
        };
        // Retain the cookie even when mmap fails. clear() reports that failure,
        // then allocate() performs the same retryable cleanup as other phases.
        Ok(CpuView {
            cpu,
            _node: node,
            cookie: Some(cookie),
            map_error,
        })
    }
    fn clear(&self, view: &CpuView) -> Result<(), String> {
        let cpu = view.cpu.as_ref().ok_or_else(|| {
            view.map_error
                .clone()
                .unwrap_or_else(|| "USER_STORAGE_CPU_MAP absent".into())
        })?;
        for off in (0..BYTES).step_by(4) {
            cpu.store_u32(HostOffset::new(off), 0)
                .map_err(|e| format!("USER_STORAGE_CLEAR: {e:?}"))?;
        }
        kf_linux_raw::release_fence();
        Ok(())
    }
    fn map_ro(&self, reservation: u32, memory: u32, va: u64) -> Result<u64, MapFailure> {
        self.map_in_ro_4k(reservation, memory, USERD_OFFSET, va)
            .map_err(|error| MapFailure {
                reason: format!("USER_STORAGE_MAP_RO: {error:?}"),
                unexpected: match error {
                    kf_host::RmError::PlacementRefused { got, .. } => Some(got),
                    _ => None,
                },
            })
    }
    fn revoke_view(&self, memory: u32, view: &mut CpuView) -> Result<(), String> {
        view.cpu = None; // munmap before returning the BAR1 aperture.
        if let Some(cookie) = view.cookie {
            self.release_cpu_view(kf_host::CpuViewRelease {
                h_memory: memory,
                p_linear_address: cookie,
            })
            .map_err(|e| format!("USER_STORAGE_CPU_RELEASE: {e:?}"))?;
            view.cookie = None;
        }
        Ok(())
    }
    fn unmap(&self, reservation: u32, va: u64) -> Result<(), String> {
        // Entire OWN reservation, including a possible uncertain map outcome.
        self.unmap_in(reservation, va, BYTES, false)
            .map_err(|e| format!("USER_STORAGE_UNMAP: {e:?}"))
    }
    fn free_reservation(&self, reservation: u32) -> Result<(), String> {
        HostRm::free(self, reservation).map_err(|e| format!("USER_STORAGE_FREE_RESERVATION: {e:?}"))
    }
    fn free(&self, memory: u32) -> Result<(), String> {
        HostRm::free(self, memory).map_err(|e| format!("USER_STORAGE_FREE: {e:?}"))
    }
}

impl Storage<CpuView> {
    fn cpu(&self) -> Result<&VolatileRegion, String> {
        if !self.live {
            return Err("USER_STORAGE_NOT_LIVE".into());
        }
        self.view
            .as_ref()
            .and_then(|v| v.cpu.as_ref())
            .ok_or_else(|| "USER_STORAGE_CPU_NOT_LIVE".into())
    }

    /// Copy HOST-AUTHORED inspected method words into the private PB region.
    /// A future adapter must hold its own consumption credits; this method grants
    /// none and must not overwrite storage still used by the native channel.
    ///
    /// # Errors
    /// Misalignment/overflow/out-of-region or released CPU view.
    pub fn write_authored(&self, offset: u64, words: &[u32]) -> Result<(), String> {
        let bytes = u64::try_from(words.len())
            .ok()
            .and_then(|n| n.checked_mul(4))
            .ok_or("USER_STORAGE_PB_BOUND")?;
        if offset % 4 != 0 || offset.checked_add(bytes).is_none_or(|end| end > GP_OFFSET) {
            return Err("USER_STORAGE_PB_BOUND".into());
        }
        let cpu = self.cpu()?;
        for (i, word) in words.iter().enumerate() {
            cpu.store_u32(HostOffset::new(offset + 4 * i as u64), *word)
                .map_err(|e| format!("USER_STORAGE_PB_WRITE: {e:?}"))?;
        }
        Ok(())
    }

    /// Author a direct MAIN GP entry pointing only inside OUR command region.
    /// No raw guest descriptor/control/conditional fetch is accepted. Preserve
    /// SYNC_WAIT from the validated inspected source when the adapter requires it.
    ///
    /// # Errors
    /// Invalid slot, empty/out-of-region span or missing CPU view.
    pub fn write_gp(
        &self,
        slot: u32,
        pb_offset: u64,
        bytes: u64,
        sync_wait: bool,
    ) -> Result<(), String> {
        if slot >= ENTRIES
            || bytes == 0
            || pb_offset
                .checked_add(bytes)
                .is_none_or(|end| end > GP_OFFSET)
        {
            return Err("USER_STORAGE_GP_BOUND".into());
        }
        let mut entry = kf_abi::submit::gp_entry(
            self.va
                .checked_add(pb_offset)
                .ok_or("USER_STORAGE_GP_BOUND")?,
            bytes,
        )
        .ok_or("USER_STORAGE_GP_FORMAT")?;
        if sync_wait {
            entry |= 1 << 63;
        }
        self.cpu()?
            .store_u64(HostOffset::new(GP_OFFSET + u64::from(slot) * 8), entry)
            .map_err(|e| format!("USER_STORAGE_GP_WRITE: {e:?}"))
    }

    /// Publish a private PUT after commands and GP descriptors are stored. The
    /// native channel's own doorbell is a separate authored host action.
    ///
    /// # Errors
    /// Out-of-ring PUT or released CPU view.
    pub fn publish_put(&self, put: u32) -> Result<(), String> {
        if put >= ENTRIES {
            return Err("USER_STORAGE_PUT_BOUND".into());
        }
        kf_linux_raw::release_fence();
        self.cpu()?
            .store_u32(
                HostOffset::new(USERD_OFFSET + kf_abi::submit::USERD_GP_PUT),
                put,
            )
            .map_err(|e| format!("USER_STORAGE_PUT_WRITE: {e:?}"))?;
        kf_linux_raw::release_fence();
        Ok(())
    }

    /// Actual private hardware GET. Fetch progress is not engine completion.
    ///
    /// # Errors
    /// Released CPU view/load failure.
    pub fn gpu_get(&self) -> Result<u32, String> {
        self.cpu()?
            .load_u32(HostOffset::new(USERD_OFFSET + kf_abi::submit::USERD_GP_GET))
            .map_err(|e| format!("USER_STORAGE_GET_READ: {e:?}"))
    }

    /// Actual private hardware Reference; never CPU-authored. Public SET_REF
    /// waits for the last-targeted engine, not an arbitrary all-engine receipt.
    ///
    /// # Errors
    /// Released CPU view/load failure.
    pub fn gpu_reference(&self) -> Result<u32, String> {
        self.cpu()?
            .load_u32(HostOffset::new(
                USERD_OFFSET + kf_abi::submit::USERD_REFERENCE_C56F,
            ))
            .map_err(|e| format!("USER_STORAGE_REF_READ: {e:?}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Fake {
        calls: RefCell<Vec<&'static str>>,
        failures: RefCell<Vec<&'static str>>,
        placement: Option<u64>,
        map_error: bool,
    }
    impl Fake {
        fn call(&self, name: &'static str) -> Result<(), String> {
            self.calls.borrow_mut().push(name);
            if self.failures.borrow().contains(&name) {
                Err(name.into())
            } else {
                Ok(())
            }
        }
    }
    impl StorageHost for Fake {
        type View = ();
        fn reserve(&self, _: VaSpace, va: u64) -> Result<u32, String> {
            assert_eq!(va, 1 << 32);
            self.call("reserve")?;
            Ok(11)
        }
        fn alloc(&self) -> Result<u32, String> {
            self.call("alloc")?;
            Ok(7)
        }
        fn view(&self, memory: u32) -> Result<(), String> {
            assert_eq!(memory, 7);
            self.call("view")
        }
        fn clear(&self, _: &()) -> Result<(), String> {
            self.call("clear")
        }
        fn map_ro(&self, reservation: u32, memory: u32, va: u64) -> Result<u64, MapFailure> {
            assert_eq!((reservation, memory), (11, 7));
            let call = self.call("map_ro");
            if self.map_error || call.is_err() {
                Err(MapFailure {
                    reason: "map_ro".into(),
                    unexpected: self.placement,
                })
            } else {
                Ok(self.placement.unwrap_or(va))
            }
        }
        fn revoke_view(&self, memory: u32, _: &mut ()) -> Result<(), String> {
            assert_eq!(memory, 7);
            self.call("revoke")
        }
        fn unmap(&self, reservation: u32, va: u64) -> Result<(), String> {
            assert_eq!((reservation, va), (11, 1 << 32));
            self.call("unmap")
        }
        fn free_reservation(&self, reservation: u32) -> Result<(), String> {
            assert_eq!(reservation, 11);
            self.call("free_reservation")
        }
        fn free(&self, memory: u32) -> Result<(), String> {
            assert_eq!(memory, 7);
            self.call("free")
        }
    }
    fn space() -> VaSpace {
        VaSpace {
            space: 1,
            range: 2,
            guest: Default::default(),
        }
    }
    fn build(host: &Fake) -> Storage<()> {
        Storage::allocate(host, space(), 1 << 32).unwrap_or_else(|e| panic!("{}", e.reason))
    }
    fn failed(host: &Fake, va: u64) -> BuildError<()> {
        match Storage::allocate(host, space(), va) {
            Err(e) => e,
            Ok(_) => panic!("expected refusal"),
        }
    }
    #[test]
    fn initialization_precedes_exposure_and_userd_remains_unmapped() {
        let host = Fake::default();
        let mut storage = build(&host);
        assert_eq!(
            *host.calls.borrow(),
            ["reserve", "alloc", "view", "clear", "map_ro"]
        );
        let spec = storage.spec(19).unwrap();
        assert_eq!(spec.gp_fifo_va, (1 << 32) + GP_OFFSET);
        assert_eq!(
            (
                spec.gp_fifo_entries,
                spec.userd_memory,
                spec.userd_offset,
                spec.err_notifier
            ),
            (ENTRIES, 7, USERD_OFFSET, 19)
        );
        assert!(GP_OFFSET + u64::from(ENTRIES) * 8 <= USERD_OFFSET);
        assert!(USERD_OFFSET + 512 <= BYTES);
        storage.release(&host).unwrap();
        assert!(storage.released());
        assert!(storage.spec(0).is_err());
        assert_eq!(
            &host.calls.borrow()[5..],
            ["revoke", "unmap", "free_reservation", "free"]
        );
    }
    #[test]
    fn failed_map_is_unmapped_even_without_a_successful_reply() {
        for (phase, expected) in [
            ("reserve", vec!["reserve"]),
            ("alloc", vec!["reserve", "alloc", "free_reservation"]),
            (
                "view",
                vec!["reserve", "alloc", "view", "free_reservation", "free"],
            ),
            (
                "clear",
                vec![
                    "reserve",
                    "alloc",
                    "view",
                    "clear",
                    "revoke",
                    "free_reservation",
                    "free",
                ],
            ),
            (
                "map_ro",
                vec![
                    "reserve",
                    "alloc",
                    "view",
                    "clear",
                    "map_ro",
                    "revoke",
                    "unmap",
                    "free_reservation",
                    "free",
                ],
            ),
        ] {
            let host = Fake::default();
            host.failures.borrow_mut().push(phase);
            let error = failed(&host, 1 << 32);
            assert!(error.retained.released(), "{phase}");
            assert_eq!(*host.calls.borrow(), expected, "{phase}");
        }
    }
    #[test]
    fn uncertain_map_plus_failed_unmap_retains_object_and_reservation() {
        let host = Fake {
            map_error: true,
            ..Fake::default()
        };
        host.failures.borrow_mut().push("unmap");
        let mut error = failed(&host, 1 << 32);
        assert!(!error.retained.released());
        assert!(error.retained.spec(0).is_err());
        assert_eq!(error.retained.mapped, Mapping::Attempted);
        assert!(!host.calls.borrow().contains(&"free_reservation"));
        assert!(!host.calls.borrow().contains(&"free"));
        host.failures.borrow_mut().clear();
        error.retained.release(&host).unwrap();
        assert!(error.retained.released());
    }
    #[test]
    fn release_failure_is_retiring_and_retries_only_unfinished_phases() {
        for (i, phase) in ["revoke", "unmap", "free_reservation", "free"]
            .into_iter()
            .enumerate()
        {
            let host = Fake::default();
            let mut storage = build(&host);
            host.failures.borrow_mut().push(phase);
            assert!(storage.release(&host).is_err());
            assert!(!storage.released());
            assert!(storage.spec(0).is_err(), "retiring after {phase}");
            assert_eq!(host.calls.borrow().last(), Some(&phase));
            host.failures.borrow_mut().clear();
            storage.release(&host).unwrap();
            assert!(storage.released());
            for (j, p) in ["revoke", "unmap", "free_reservation", "free"]
                .into_iter()
                .enumerate()
            {
                assert_eq!(
                    host.calls
                        .borrow()
                        .iter()
                        .filter(|&&name| name == p)
                        .count(),
                    if i == j { 2 } else { 1 }
                );
            }
            let n = host.calls.borrow().len();
            storage.release(&host).unwrap();
            assert_eq!(host.calls.borrow().len(), n);
        }
    }
    #[test]
    fn misplaced_map_inside_owned_reservation_uses_whole_range_cleanup() {
        for error_reply in [false, true] {
            let host = Fake {
                placement: Some((1 << 32) + 4096),
                map_error: error_reply,
                ..Fake::default()
            };
            let error = failed(&host, 1 << 32);
            assert!(error.retained.released());
            assert!(host.calls.borrow().contains(&"unmap"));
        }
    }
    #[test]
    fn placement_outside_owned_reservation_is_quarantined_without_guessing() {
        for error_reply in [false, true] {
            let host = Fake {
                placement: Some((1 << 32) + BYTES),
                map_error: error_reply,
                ..Fake::default()
            };
            let mut error = failed(&host, 1 << 32);
            assert!(!error.retained.released());
            assert!(error.retained.spec(0).is_err());
            assert!(error.reason.contains("QUARANTINED"));
            assert!(!host.calls.borrow().contains(&"unmap"));
            assert!(!host.calls.borrow().contains(&"free_reservation"));
            assert!(!host.calls.borrow().contains(&"free"));
            assert!(error.retained.release(&host).is_err());
        }
    }
    #[test]
    fn invalid_slot_never_calls_host() {
        for va in [1, (1 << 40) - 0x10000, u64::MAX] {
            let host = Fake::default();
            let error = failed(&host, va);
            assert!(error.retained.released());
            assert!(host.calls.borrow().is_empty());
        }
    }
}
