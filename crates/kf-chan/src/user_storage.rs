//! Private read-only command storage for the restricted Windows relay (§AI).
//!
//! This is a host resource primitive, NOT connected to VM channel birth. The
//! future VA-manager adapter must require WindowsUserRingPermit, reserve its
//! slot in the mirror ownership ledger before calling this, and retain that
//! reservation on any release failure. Linux Passthrough has no caller.
//!
//! Clear every byte before GPU exposure. Only commands/GP entries are mapped,
//! always read-only; private USERD is outside the GPU mapping. There is no gate,
//! writable fence or GPU-work completion. Direct USER descriptors may be copied
//! only after the worker's separate own-space/source-lifetime admission.
//! The caller must quiesce/free its native channel before releasing storage.

use kf_host::{HostRm, MapNode, VaSpace, ViewAccess};
use kf_linux_raw::{Backing, CachePolicy, CharDevice, HostOffset, HostPageSize, VolatileRegion};

/// Trusted per-channel allocation policy, not a per-die fact.
pub const BYTES: u64 = 1 << 20;
/// Pushbuffer region ends here; whole64KiB granules permit RO-only mapping.
pub const GP_OFFSET: u64 = 0xe0000;
/// Trusted source snapshot limit: any intercepted segment must fit completely
/// in the existing private staging region. This is an allocation policy, not
/// the hardware maximum GPFIFO length or a per-die compatibility constant.
pub const MAX_SOURCE_BYTES: u64 = GP_OFFSET;
/// USERD starts here and has NO GPU VA mapping.
pub const USERD_OFFSET: u64 = 0xf0000;
/// Trusted private GP capacity, independent of the guest ring's changing size.
pub const ENTRIES: u32 = 512;
/// The marker cells start this far beyond the descriptor array (after the 4-byte byte witness word,
/// 16-aligned), inside the read-only mapping and below USERD: `ENTRIES` cells of [`MARKER_CELL`].
const MARKER_OFFSET: u64 = 0x10;
/// One marker cell: `[SET_REFERENCE, value, NON_STALL_INTERRUPT, 0]`.
const MARKER_CELL: u64 = 16;
/// Trusted per-command-slot staging cell. One descriptor credit remains empty,
/// so reusing a command slot needs GET beyond its old later witness (see the
/// user_consumption model). Early commitment of GP0 alone cannot reuse its cell.
pub const CELL_BYTES: u64 = 1024;

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
    /// Clear one bounded initialization piece before any GPU mapping exists.
    fn clear_range(&self, _view: &Self::View, _offset: u64, _bytes: u64) -> Result<(), String> {
        Err("USER_STORAGE_INCREMENTAL_CLEAR_UNSUPPORTED".into())
    }
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
    cleared: u64,
}

impl<V> Storage<V> {
    /// Standalone/native convenience; production uses prepare, bounded clear
    /// steps and map_initialized on separate idle VA-worker turns.
    pub fn allocate(
        host: &impl StorageHost<View = V>,
        space: VaSpace,
        va: u64,
    ) -> Result<Self, BuildError<V>> {
        let mut out = Self::prepare(host, space, va)?;
        let build = (|| {
            host.clear(out.view.as_ref().ok_or("USER_STORAGE_VIEW absent")?)?;
            out.cleared = BYTES;
            out.map_initialized(host)
        })();
        match build {
            Ok(()) => Ok(out),
            Err(reason) => Err(out.failed(host, reason)),
        }
    }

    /// Reserve/allocate/arm the CPU view, without a GPU map or usable spec.
    /// Only the VA worker calls this; every partial failure retains ownership.
    pub fn prepare(
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
            cleared: 0,
        };
        let build = (|| {
            if va % 0x10000 != 0 || va.checked_add(BYTES).is_none_or(|end| end > 1 << 40) {
                return Err(
                    "USER_STORAGE_VA: reserved slot is not64KiB-aligned below2^40".to_string(),
                );
            }
            out.reservation = Some(host.reserve(space, va)?);
            let memory = host.alloc()?;
            out.memory = Some(memory);
            out.view = Some(host.view(memory)?);
            Ok(())
        })();
        match build {
            Ok(()) => Ok(out),
            Err(reason) => Err(out.failed(host, reason)),
        }
    }

    fn failed(mut self, host: &impl StorageHost<View = V>, mut reason: String) -> BuildError<V> {
        if let Err(cleanup) = self.release(host) {
            reason.push_str(&format!("; retained: {cleanup}"));
        }
        BuildError {
            reason,
            retained: self,
        }
    }

    /// Clear at most4KiB this VA turn; no GPU map/spec exists until all bytes are
    /// initialized. Failure never advances the initialized prefix.
    pub fn clear_next(&mut self, host: &impl StorageHost<View = V>) -> Result<bool, String> {
        if self.live
            || self.mapped != Mapping::None
            || self.reservation.is_none()
            || self.memory.is_none()
        {
            return Err("USER_STORAGE_INITIALIZATION_STATE".into());
        }
        if self.cleared == BYTES {
            return Ok(true);
        }
        let bytes = 4096.min(BYTES.saturating_sub(self.cleared));
        host.clear_range(
            self.view.as_ref().ok_or("USER_STORAGE_VIEW absent")?,
            self.cleared,
            bytes,
        )?;
        self.cleared += bytes;
        Ok(self.cleared == BYTES)
    }

    /// Install RO PTEs only after the entire object was initialized. A failed
    /// ioctl may have mapped: caller must retain ownership and explicitly clean.
    pub fn map_initialized(&mut self, host: &impl StorageHost<View = V>) -> Result<(), String> {
        if self.live || self.mapped != Mapping::None || self.cleared != BYTES || self.view.is_none()
        {
            return Err("USER_STORAGE_INITIALIZATION_INCOMPLETE".into());
        }
        let reservation = self.reservation.ok_or("USER_STORAGE_NO_RESERVATION")?;
        let memory = self.memory.ok_or("USER_STORAGE_NO_MEMORY")?;
        self.mapped = Mapping::Attempted;
        let actual = match host.map_ro(reservation, memory, self.va) {
            Ok(actual) => actual,
            Err(error) => {
                if let Some(actual) = error.unexpected {
                    self.note_placement(actual);
                }
                return Err(error.reason);
            }
        };
        self.note_placement(actual);
        if actual != self.va {
            return Err(format!(
                "USER_STORAGE_FIXED: got{actual:#x}, expected{:#x}",
                self.va
            ));
        }
        self.mapped = Mapping::Live;
        self.live = true;
        Ok(())
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
    fn clear_range(&self, view: &CpuView, offset: u64, bytes: u64) -> Result<(), String> {
        if bytes > 4096
            || bytes == 0
            || offset % 4 != 0
            || bytes % 4 != 0
            || offset.checked_add(bytes).is_none_or(|end| end > BYTES)
        {
            return Err("USER_STORAGE_CLEAR_BOUND".into());
        }
        let cpu = view.cpu.as_ref().ok_or_else(|| {
            view.map_error
                .clone()
                .unwrap_or_else(|| "USER_STORAGE_CPU_MAP absent".into())
        })?;
        for off in (offset..offset + bytes).step_by(4) {
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
    /// Author one complete command batch and its HOST-consumption barrier.
    /// Caller reserves the three GP credits through user_consumption FIRST.
    /// All words are inspected/host-authored; no raw guest GP word is accepted.
    pub fn write_publication(
        &self,
        publication: crate::user_consumption::Publication,
        words: &[u32],
        sync_wait: bool,
    ) -> Result<(), String> {
        if words.is_empty() || words.len() > (CELL_BYTES / 4) as usize {
            return Err("USER_STORAGE_CELL_BOUND".into());
        }
        let offset = u64::from(publication.command_slot)
            .checked_mul(CELL_BYTES)
            .ok_or("USER_STORAGE_CELL_BOUND")?;
        self.write_authored(offset, words)?;
        self.write_gp(
            publication.command_slot,
            offset,
            words.len() as u64 * 4,
            sync_wait,
        )?;
        self.write_host_barrier(publication.barrier_slot)?;
        self.write_host_witness(publication.witness_slot)
    }

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
        self.write_gp_level(slot, pb_offset, bytes, sync_wait, false)
    }

    /// Preserve the inspected source's public LEVEL progress classification on
    /// an authored staged span. LEVEL1 suppresses TOP_LEVEL_GET; it grants no
    /// execution privilege or receipt. All span checks remain identical.
    pub fn write_gp_level(
        &self,
        slot: u32,
        pb_offset: u64,
        bytes: u64,
        sync_wait: bool,
        subroutine: bool,
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
        if subroutine {
            entry |= 1 << 41;
        }
        self.cpu()?
            .store_u64(HostOffset::new(GP_OFFSET + u64::from(slot) * 8), entry)
            .map_err(|e| format!("USER_STORAGE_GP_WRITE: {e:?}"))
    }

    /// Copy a direct USER descriptor unchanged into the private ring. This
    /// validates format and the trusted scan budget ONLY: the worker must also
    /// resolve the complete source in this channel's own restricted address
    /// space and retain source/channel admission through PUT publication.
    /// Never use this for privileged Translated work or bypass software triggers.
    pub fn write_user_gp(&self, slot: u32, entry: u64) -> Result<(), String> {
        if slot >= ENTRIES {
            return Err("USER_STORAGE_GP_BOUND".into());
        }
        crate::user_stream::inspect_entry(entry, MAX_SOURCE_BYTES)
            .map_err(|why| format!("USER_STORAGE_USER_GP: {why:?}"))?;
        self.cpu()?
            .store_u64(HostOffset::new(GP_OFFSET + u64::from(slot) * 8), entry)
            .map_err(|e| format!("USER_STORAGE_GP_WRITE: {e:?}"))
    }

    /// Opaque diagnostic USER forwarding; caller must retain own restricted USER
    /// eligibility, exact slot/source/allocation admission and no CPU API actions.
    /// This does not validate native GP semantics or grant any receipt.
    pub fn write_shadow_gp(&self, slot: u32, entry: u64) -> Result<(), String> {
        if slot >= ENTRIES {
            return Err("USER_STORAGE_SHADOW_GP_BOUND".into());
        }
        self.cpu()?
            .store_u64(HostOffset::new(GP_OFFSET + u64::from(slot) * 8), entry)
            .map_err(|e| format!("USER_STORAGE_SHADOW_GP_WRITE: {e:?}"))
    }

    /// Author only the public CONTROL NOP/LENGTH0/SYNC_WAIT descriptor.
    /// The caller must insert this after a complete authored packet, then a
    /// nonempty witness. The barrier's own GET commit grants no PB-byte credit.
    ///
    /// # Errors
    /// Invalid slot or retiring/released CPU view.
    pub fn write_host_barrier(&self, slot: u32) -> Result<(), String> {
        if slot >= ENTRIES {
            return Err("USER_STORAGE_BARRIER_BOUND".into());
        }
        // Public595.84 class/clc56f.h: NOP opcode0, LENGTH0, SYNC_WAIT bit63.
        self.cpu()?
            .store_u64(HostOffset::new(GP_OFFSET + u64::from(slot) * 8), 1u64 << 63)
            .map_err(|e| format!("USER_STORAGE_BARRIER_WRITE: {e:?}"))
    }

    /// Author a nonempty LEVEL_SUBROUTINE witness to zero HOST DMA_NOP
    /// bytes just beyond our descriptor array, inside the RO mapping. No CPU
    /// writer exposed by this helper can mutate those bytes after initialization.
    ///
    /// # Errors
    /// Invalid slot or retiring/released CPU view.
    pub fn write_host_witness(&self, slot: u32) -> Result<(), String> {
        if slot >= ENTRIES {
            return Err("USER_STORAGE_WITNESS_BOUND".into());
        }
        let va = self
            .va
            .checked_add(GP_OFFSET + u64::from(ENTRIES) * 8)
            .ok_or("USER_STORAGE_WITNESS_BOUND")?;
        // Synthetic private bytes must not replace TOP_LEVEL_GET monitoring.
        let entry =
            kf_abi::submit::gp_entry(va, 4).ok_or("USER_STORAGE_WITNESS_FORMAT")? | (1 << 41);
        self.cpu()?
            .store_u64(HostOffset::new(GP_OFFSET + u64::from(slot) * 8), entry)
            .map_err(|e| format!("USER_STORAGE_WITNESS_WRITE: {e:?}"))
    }

    /// ★ 2026-10-14 — **the marker** (`crate::user_loop`, `crate::user_gate::Marks`): a private 16-byte
    /// segment `[SET_REFERENCE value, NON_STALL_INTERRUPT 0]` in the read-only mapping, one cell per
    /// ring `slot` (so the value each marker writes is its own and a later marker never rewrites the
    /// bytes of an earlier one still in flight), entered as a `LEVEL_SUBROUTINE` main entry.
    ///
    /// Processing it writes `USERD.REFERENCE = value` (the PROOF that every method before it,
    /// acquires included, was processed -- measured: `GP_GET` runs ahead of processing and proves
    /// nothing about that) and raises the engine's non-stall event the worker's epoll already
    /// watches (the wake). No semaphore, no GPU-written memory: OWNER_RULINGS §AI authorises
    /// read-only commands only; `USERD` is outside the GPU mapping.
    ///
    /// # Errors
    /// Invalid slot or retiring/released CPU view.
    pub fn write_host_marker(&self, slot: u32, value: u32) -> Result<(), String> {
        if slot >= ENTRIES {
            return Err("USER_STORAGE_MARKER_BOUND".into());
        }
        let off = GP_OFFSET + u64::from(ENTRIES) * 8 + MARKER_OFFSET + u64::from(slot) * MARKER_CELL;
        let va = self.va.checked_add(off).ok_or("USER_STORAGE_MARKER_BOUND")?;
        let set_ref = kf_abi::submit::method_header_inc(0, kf_abi::submit::fifo::SET_REFERENCE_C56F, 1)
            .ok_or("USER_STORAGE_MARKER_FORMAT")?;
        let nsi = kf_abi::submit::method_header_inc(0, kf_abi::submit::fifo::NON_STALL_INTERRUPT, 1)
            .ok_or("USER_STORAGE_MARKER_FORMAT")?;
        let entry = kf_abi::submit::gp_entry(va, MARKER_CELL).ok_or("USER_STORAGE_MARKER_FORMAT")? | (1 << 41);
        let cpu = self.cpu()?;
        for (i, w) in [set_ref, value, nsi, 0].into_iter().enumerate() {
            cpu.store_u32(HostOffset::new(off + 4 * i as u64), w)
                .map_err(|e| format!("USER_STORAGE_MARKER_WRITE: {e:?}"))?;
        }
        cpu.store_u64(HostOffset::new(GP_OFFSET + u64::from(slot) * 8), entry)
            .map_err(|e| format!("USER_STORAGE_MARKER_WRITE: {e:?}"))
    }

    /// ★ 2026-10-14 — the SYNC_WAIT gate's witness when the guest stream CARRIES a packet into the
    /// gated entry (RETIRED 2026-10-14: a control entry proves nothing -- `crate::user_gate::Witness::None`): a CONTROL `NOP` GP entry (`LENGTH` 0,
    /// `OPCODE_NOP`, `SYNC_PROCEED`; all 64 bits zero). It fetches no pushbuffer byte, so the
    /// packet the PBDMA is parsing continues untouched into the guest's next segment. It cannot
    /// begin processing before the barrier's wait finishes (public `dev_pbdma.ref`,
    /// `NV_PPBDMA_GP_ENTRY1_SYNC`: "no further GP entries will be processed until the wait
    /// finishes"), and `GP_GET` passes an entry only once it has begun processing
    /// (`NV_PPBDMA_GP_GET`) — the same proof the byte witness gives, without a byte. It grants no
    /// pushbuffer-byte credit (nothing staged depends on it).
    ///
    /// # Errors
    /// Invalid slot or retiring/released CPU view.
    pub fn write_host_control_witness(&self, slot: u32) -> Result<(), String> {
        if slot >= ENTRIES {
            return Err("USER_STORAGE_WITNESS_BOUND".into());
        }
        self.cpu()?
            .store_u64(HostOffset::new(GP_OFFSET + u64::from(slot) * 8), 0)
            .map_err(|e| format!("USER_STORAGE_WITNESS_WRITE: {e:?}"))
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

    /// MEASUREMENT ONLY (`KF3_USER_RING_TRACE`): the first 36 words of our own private USERD as the
    /// hardware last wrote them back (`dev_ram.ref` `NV_RAMUSERD`: `PUT` word 16, `GET` 17, `REF`
    /// 18, `PUT_HI` 19, `TOP_LEVEL_GET` 22, `_HI` 23, `GET_HI` 24, `GP_GET` 34, `GP_PUT` 35). Never
    /// a decision input.
    ///
    /// # Errors
    /// Released CPU view/load failure.
    pub fn userd_words(&self) -> Result<[u32; 36], String> {
        let cpu = self.cpu()?;
        let mut out = [0u32; 36];
        for (i, w) in out.iter_mut().enumerate() {
            *w = cpu
                .load_u32(HostOffset::new(USERD_OFFSET + 4 * i as u64))
                .map_err(|e| format!("USER_STORAGE_USERD_READ: {e:?}"))?;
        }
        Ok(out)
    }

    /// Actual private hardware Reference; never CPU-authored. Public SET_REF
    /// waits for the last-targeted engine, not an arbitrary all-engine receipt. ★ 2026-10-14: the
    /// tagged value of the last PROCESSED marker is the SYNC_WAIT gate's proof
    /// (`crate::user_gate::Marks`) -- not `GP_GET`, which runs ahead of processing; an untagged
    /// value is the guest's own `SET_REFERENCE` and proves nothing.
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

    /// MEASUREMENT ONLY (`KF3_DIAG_USER_STALL`): the private ring's hardware `GP_GET` and the
    /// `GP_PUT` the relay published, two 4-byte loads. Never a decision input.
    ///
    /// # Errors
    /// Released CPU view/load failure.
    pub fn gpu_cursors(&self) -> Result<(u32, u32), String> {
        let cpu = self.cpu()?;
        let get = cpu
            .load_u32(HostOffset::new(USERD_OFFSET + kf_abi::submit::USERD_GP_GET))
            .map_err(|e| format!("USER_STORAGE_GET_READ: {e:?}"))?;
        let put = cpu
            .load_u32(HostOffset::new(USERD_OFFSET + kf_abi::submit::USERD_GP_PUT))
            .map_err(|e| format!("USER_STORAGE_PUT_READ: {e:?}"))?;
        Ok((get, put))
    }

    /// MEASUREMENT ONLY (`KF3_DIAG_USER_STALL`): one private GP ring entry as it stands, 8 bytes.
    ///
    /// # Errors
    /// A slot outside the ring, or a released CPU view/load failure.
    pub fn read_gp(&self, slot: u32) -> Result<u64, String> {
        if slot >= ENTRIES {
            return Err("USER_STORAGE_GP_BOUND".into());
        }
        self.cpu()?
            .load_u64(HostOffset::new(GP_OFFSET + u64::from(slot) * 8))
            .map_err(|e| format!("USER_STORAGE_GP_READ: {e:?}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    // Exercise the production memory accessors on a real CPU mapping without
    // opening NVIDIA or making an RM call. No simulated GET is a GPU result.
    fn anonymous_live_storage() -> Storage<CpuView> {
        Storage {
            va: 1 << 32,
            reservation: Some(11),
            live: true,
            memory: Some(7),
            view: Some(CpuView {
                cpu: Some(
                    VolatileRegion::map(
                        Backing::PrivateAnonymous,
                        BYTES,
                        CachePolicy::WriteBack,
                        HostPageSize::query(),
                    )
                    .unwrap(),
                ),
                _node: CharDevice::openat(&kf_linux_raw::DevDir::open(c"/dev").unwrap(), c"null")
                    .unwrap(),
                cookie: None,
                map_error: None,
            }),
            mapped: Mapping::Live,
            cleared: BYTES,
        }
    }

    #[test]
    fn the_diagnostic_reads_the_private_cursors_and_entries_without_writing() {
        let s = anonymous_live_storage();
        let cpu = s.cpu().unwrap();
        cpu.store_u32(
            HostOffset::new(USERD_OFFSET + kf_abi::submit::USERD_GP_GET),
            0x1a8,
        )
        .unwrap();
        s.publish_put(0x1aa).unwrap();
        assert_eq!(s.gpu_cursors().unwrap(), (0x1a8, 0x1aa));
        let entry = kf_abi::submit::gp_entry(0x3_0472_0000, 500).unwrap() | (1 << 41);
        s.write_user_gp(0x1a7, entry).unwrap();
        s.write_user_gp(ENTRIES - 1, 0x1234_5678_9abc_def0 & !(1 << 63))
            .ok();
        assert_eq!(s.read_gp(0x1a7).unwrap(), entry);
        assert_eq!(
            s.read_gp(0x1a8).unwrap(),
            0,
            "a slot never written reads zero"
        );
        // the slot is an 8-byte stride from the ring base, whatever the slot
        for slot in [0u32, 1, 3, 255, ENTRIES - 1] {
            let e = kf_abi::submit::gp_entry(0x1000 + u64::from(slot) * 0x100, 64).unwrap();
            s.write_user_gp(slot, e).unwrap();
            assert_eq!(s.read_gp(slot).unwrap(), e, "slot {slot}");
        }
        // reading changed nothing
        assert_eq!(s.gpu_cursors().unwrap(), (0x1a8, 0x1aa));
        assert_eq!(s.read_gp(0x1a7).unwrap(), entry);
    }

    #[test]
    fn the_diagnostic_cursors_are_get_then_put_whatever_their_values() {
        let s = anonymous_live_storage();
        let cpu = s.cpu().unwrap();
        for (get, put) in [(1u32, 2u32), (500, 3), (0, 0), (0x1a8, 0x1aa)] {
            cpu.store_u32(
                HostOffset::new(USERD_OFFSET + kf_abi::submit::USERD_GP_GET),
                get,
            )
            .unwrap();
            cpu.store_u32(
                HostOffset::new(USERD_OFFSET + kf_abi::submit::USERD_GP_PUT),
                put,
            )
            .unwrap();
            assert_eq!(s.gpu_cursors().unwrap(), (get, put));
            assert_eq!(s.gpu_get().unwrap(), get, "the same word the relay reads");
        }
    }

    #[test]
    fn the_diagnostic_refuses_a_slot_outside_the_ring_and_a_released_view() {
        let mut s = anonymous_live_storage();
        assert!(s.read_gp(ENTRIES).is_err());
        assert!(s.read_gp(u32::MAX).is_err());
        assert!(s.read_gp(ENTRIES - 1).is_ok());
        s.view = None;
        assert!(s.read_gp(0).is_err());
        assert!(s.gpu_cursors().is_err());
    }

    #[test]
    fn private_ring_publication_keeps_user_addresses_and_never_forges_get_or_reference() {
        let s = anonymous_live_storage();
        let cpu = s.cpu().unwrap();
        let get = HostOffset::new(USERD_OFFSET + kf_abi::submit::USERD_GP_GET);
        let reference = HostOffset::new(USERD_OFFSET + kf_abi::submit::USERD_REFERENCE_C56F);
        cpu.store_u32(get, 17).unwrap();
        cpu.store_u32(reference, 0x1234).unwrap();
        let entry = kf_abi::submit::gp_entry(0x420000, 4096).unwrap() | (1 << 63);
        s.write_user_gp(ENTRIES - 1, entry).unwrap();
        s.write_user_gp(0, 0).unwrap();
        s.publish_put(1).unwrap();
        assert_eq!(
            cpu.load_u64(HostOffset::new(GP_OFFSET + u64::from(ENTRIES - 1) * 8))
                .unwrap(),
            entry
        );
        assert_eq!(cpu.load_u64(HostOffset::new(GP_OFFSET)).unwrap(), 0);
        assert_eq!(
            cpu.load_u32(HostOffset::new(USERD_OFFSET + kf_abi::submit::USERD_GP_PUT))
                .unwrap(),
            1
        );
        assert_eq!(s.gpu_get().unwrap(), 17);
        assert_eq!(s.gpu_reference().unwrap(), 0x1234);
    }

    #[test]
    fn direct_level_one_keeps_original_descriptor_and_real_credit_fields() {
        let s = anonymous_live_storage();
        let entry = kf_abi::submit::gp_entry(0x420000, 128).unwrap() | (1 << 41) | (1 << 63);
        s.write_user_gp(0, entry).unwrap();
        assert_eq!(
            s.cpu()
                .unwrap()
                .load_u64(HostOffset::new(GP_OFFSET))
                .unwrap(),
            entry
        );
        assert_eq!(s.gpu_get().unwrap(), 0);
        assert_eq!(s.gpu_reference().unwrap(), 0);
        let crate::user_stream::Source::Segment(d) =
            crate::user_stream::inspect_entry(entry, 65536).unwrap()
        else {
            panic!("nonempty direct segment");
        };
        assert!(d.subroutine && d.sync_wait);
        assert_eq!(d.gpu_va, 0x420000);
        assert_eq!(d.len_bytes, 128);
    }

    #[test]
    fn large_user_descriptor_fits_staging_policy_and_keeps_real_credit_fields() {
        let s = anonymous_live_storage();
        for bytes in [100 << 10, GP_OFFSET] {
            let entry = kf_abi::submit::gp_entry(0x420000, bytes).unwrap() | (1 << 41);
            s.write_user_gp(0, entry).unwrap();
            assert_eq!(
                s.cpu()
                    .unwrap()
                    .load_u64(HostOffset::new(GP_OFFSET))
                    .unwrap(),
                entry
            );
        }
        let before = s
            .cpu()
            .unwrap()
            .load_u64(HostOffset::new(GP_OFFSET))
            .unwrap();
        let oversized = kf_abi::submit::gp_entry(0x420000, GP_OFFSET + 4).unwrap();
        assert!(s.write_user_gp(0, oversized).is_err());
        assert_eq!(
            s.cpu()
                .unwrap()
                .load_u64(HostOffset::new(GP_OFFSET))
                .unwrap(),
            before
        );
        assert_eq!(s.gpu_get().unwrap(), 0);
        assert_eq!(s.gpu_reference().unwrap(), 0);
    }

    #[test]
    fn source_sync_level_nop_is_kept_without_fabricating_credit() {
        let s = anonymous_live_storage();
        let entry = 0x8000_0200_0000_0000;
        s.write_user_gp(0, entry).unwrap();
        assert_eq!(
            s.cpu()
                .unwrap()
                .load_u64(HostOffset::new(GP_OFFSET))
                .unwrap(),
            entry
        );
        assert_eq!(s.gpu_get().unwrap(), 0);
        assert_eq!(s.gpu_reference().unwrap(), 0);
    }

    #[test]
    fn shadow_writer_preserves_opaque_bits_but_cannot_publish_or_forge_progress() {
        let s = anonymous_live_storage();
        for entry in [
            1 << 32,
            u64::MAX,
            kf_abi::submit::gp_entry(0x420000, MAX_SOURCE_BYTES + 4).unwrap(),
        ] {
            assert!(s.write_user_gp(0, entry).is_err());
            s.write_shadow_gp(ENTRIES - 1, entry).unwrap();
            assert_eq!(
                s.cpu()
                    .unwrap()
                    .load_u64(HostOffset::new(GP_OFFSET + u64::from(ENTRIES - 1) * 8))
                    .unwrap(),
                entry
            );
            assert_eq!(s.gpu_get().unwrap(), 0);
            assert_eq!(s.gpu_reference().unwrap(), 0);
            assert_eq!(
                s.cpu()
                    .unwrap()
                    .load_u32(HostOffset::new(USERD_OFFSET + kf_abi::submit::USERD_GP_PUT))
                    .unwrap(),
                0
            );
            assert!(s.write_shadow_gp(ENTRIES, entry).is_err());
        }
    }

    #[test]
    fn bad_user_descriptor_cannot_modify_private_ring_or_userd() {
        let s = anonymous_live_storage();
        let good = kf_abi::submit::gp_entry(0x420000, 4).unwrap();
        s.write_user_gp(0, good).unwrap();
        for bad in [
            1 << 32, // CONTROL/ILLEGAL, not an unused NOP operand.
            good | 1,
            good | (1 << 40),
            kf_abi::submit::gp_entry(0x420000, MAX_SOURCE_BYTES + 4).unwrap(),
        ] {
            assert!(s.write_user_gp(0, bad).is_err(), "entry={bad:#x}");
            assert_eq!(
                s.cpu()
                    .unwrap()
                    .load_u64(HostOffset::new(GP_OFFSET))
                    .unwrap(),
                good
            );
        }
        assert!(s.write_user_gp(ENTRIES, good).is_err());
        assert!(s.publish_put(ENTRIES).is_err());
        assert_eq!(s.gpu_get().unwrap(), 0);
        assert_eq!(
            s.cpu()
                .unwrap()
                .load_u32(HostOffset::new(USERD_OFFSET + kf_abi::submit::USERD_GP_PUT))
                .unwrap(),
            0
        );
    }

    #[test]
    fn staged_level_and_sync_are_preserved_without_granting_receipts() {
        let s = anonymous_live_storage();
        s.write_authored(0, &[0, 0]).unwrap();
        for (slot, level) in [(0, false), (1, true)] {
            s.write_gp_level(slot, 0, 8, true, level).unwrap();
            let entry = s
                .cpu()
                .unwrap()
                .load_u64(HostOffset::new(GP_OFFSET + u64::from(slot) * 8))
                .unwrap();
            let d = kf_abi::submit::gp_entry_decode_direct_c56f(entry).unwrap();
            assert_eq!(d.subroutine, level);
            assert!(d.sync_wait);
            assert_eq!(d.gpu_va, s.va);
            assert_eq!(d.len_bytes, 8);
        }
        assert!(s.write_gp_level(ENTRIES, 0, 8, false, true).is_err());
        assert!(s.write_gp_level(0, USERD_OFFSET, 4, false, true).is_err());
        assert_eq!(s.gpu_get().unwrap(), 0);
        assert_eq!(s.gpu_reference().unwrap(), 0);
    }

    #[test]
    fn trigger_staging_and_host_witness_do_not_overlap_ring_or_unmapped_userd() {
        let s = anonymous_live_storage();
        s.write_authored(0, &[0, 0]).unwrap();
        s.write_gp(0, 0, 8, false).unwrap();
        s.write_host_barrier(1).unwrap();
        s.write_host_witness(2).unwrap();
        s.publish_put(3).unwrap();
        let cpu = s.cpu().unwrap();
        let read = |slot: u64| cpu.load_u64(HostOffset::new(GP_OFFSET + slot * 8)).unwrap();
        assert_eq!(read(0), kf_abi::submit::gp_entry(s.va, 8).unwrap());
        assert_eq!(read(1), 1 << 63);
        let witness = s.va + GP_OFFSET + u64::from(ENTRIES) * 8;
        assert_eq!(
            read(2),
            kf_abi::submit::gp_entry(witness, 4).unwrap() | (1 << 41)
        );
        assert_eq!(cpu.load_u32(HostOffset::new(witness - s.va)).unwrap(), 0);
        // The carried-packet witness is a CONTROL NOP: no pushbuffer byte, no SYNC.
        s.write_host_control_witness(3).unwrap();
        assert_eq!(read(3), 0);
        assert!(matches!(
            crate::user_stream::inspect_entry(read(3), 4),
            Ok(crate::user_stream::Source::Nop)
        ));
        assert!(s.write_host_control_witness(ENTRIES).is_err());
        assert!(s.write_authored(GP_OFFSET, &[99]).is_err());
        assert!(s.write_gp(3, USERD_OFFSET, 4, false).is_err());
        assert_eq!(
            s.gpu_get().unwrap(),
            0,
            "CPU publication supplies no consumption receipt"
        );
    }

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
        fn clear_range(&self, _: &(), offset: u64, bytes: u64) -> Result<(), String> {
            assert_eq!(bytes, 4096);
            assert!(offset < BYTES && offset % 4096 == 0);
            self.call("clear_range")
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
        assert!(GP_OFFSET + u64::from(ENTRIES) * 8 + 4 <= USERD_OFFSET);
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
    fn sliced_initialization_cannot_expose_a_prefix_or_skip_a_failed_piece() {
        let host = Fake::default();
        let mut storage =
            Storage::prepare(&host, space(), 1 << 32).unwrap_or_else(|e| panic!("{}", e.reason));
        assert!(storage.spec(0).is_err());
        assert!(storage.map_initialized(&host).is_err());
        host.failures.borrow_mut().push("clear_range");
        assert!(storage.clear_next(&host).is_err());
        assert_eq!(storage.cleared, 0);
        host.failures.borrow_mut().clear();
        for i in 0..BYTES / 4096 {
            assert_eq!(storage.clear_next(&host).unwrap(), i + 1 == BYTES / 4096);
            assert!(storage.spec(0).is_err());
            if i + 1 < BYTES / 4096 {
                assert!(storage.map_initialized(&host).is_err());
            }
        }
        assert!(!host.calls.borrow().contains(&"map_ro"));
        storage.map_initialized(&host).unwrap();
        assert!(storage.spec(0).is_ok());
        assert!(storage.clear_next(&host).is_err());
        storage.release(&host).unwrap();
        assert!(storage.map_initialized(&host).is_err());
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

impl Storage<CpuView> {
    /// Bounded double sample of C56F's public USERD TOP_LEVEL_GET fields.
    /// Returns None if the two full samples disagree. This is a raw processing
    /// cursor, not an engine completion or a receipt without lifetime validation.
    ///
    /// # Errors
    /// Retiring/released storage or CPU-load failure.
    pub fn gpu_top_get(&self) -> Result<Option<u64>, String> {
        let cpu = self.cpu()?;
        // Public595.84 class/clc56f.h Nvc56fControl.TopLevelGet/Hi, +0x58/+0x5c.
        let sample = || -> Result<u64, String> {
            let lo = cpu
                .load_u32(HostOffset::new(USERD_OFFSET + 0x58))
                .map_err(|e| format!("USER_STORAGE_TOP_GET: {e:?}"))?;
            let hi = cpu
                .load_u32(HostOffset::new(USERD_OFFSET + 0x5c))
                .map_err(|e| format!("USER_STORAGE_TOP_GET_HI: {e:?}"))?;
            Ok(u64::from(lo) | (u64::from(hi) << 32))
        };
        let first = sample()?;
        let second = sample()?;
        Ok((first == second).then_some(first))
    }
}
