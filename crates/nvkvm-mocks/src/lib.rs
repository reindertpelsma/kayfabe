//! # nvkvm-mocks — the deterministic, in-process fake GPU harness
//!
//! One mock per port (`mode2_rust_testing_strategy.md` §4), so the logic core runs
//! **end-to-end without a GPU, a hypervisor, or an OS**:
//!
//! - [`MockArch`] — a complete fake GPU generation ("Mockingbird"). Its encodings are
//!   **deliberately not NVIDIA's**: any core code that secretly assumes a real bit
//!   layout fails these tests. It is also the standing proof of the anti-duplication
//!   property: it implements `Arch` exactly the way a real `impl Arch for Ampere`
//!   would, with zero core edits.
//! - [`MockVmm`] — a scripted hypervisor: sparse guest RAM as byte maps, recorded
//!   irqs/slots/traps, and a **virtual clock advanced explicitly by the test**
//!   (determinism is load-bearing; no real timers exist anywhere).
//! - [`MockRmBackend`] / [`MockIsolate`] / [`MockIsolateFactory`] — a fake host RM
//!   per isolate: records every verb, hands out synthetic handles from a
//!   **per-isolate namespace** (so cross-isolate handle reach is detectable), and
//!   can be scripted to fail (`fail_next`) for negative paths.
//!
//! All mocks are pure in-memory state machines: no files, no sockets, no wall clock.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};
use std::ops::Range;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nvkvm_arch::ids::{ClassId, EngineClass, VChid};
use nvkvm_arch::{
    Aperture, Arch, DoorbellTarget, GmmuFmt, GmmuVersion, ObjectKind, PageSize, PteDecode,
    UserdModel,
};
use nvkvm_isolate::{
    ControlCmd, HostHandle, Isolate, IsolateFactory, IsolateId, RmBackend, RmError,
};
use nvkvm_util::Instant;
use nvkvm_vmm::{
    BarId, CoreEvent, CoreEventKind, HostRegion, IrqSpec, Prot, RamHandle, SlotId, TrapMode, Vmm,
    VmmError,
};

// ---------------------------------------------------------------------------------
// MockArch — the "Mockingbird" generation
// ---------------------------------------------------------------------------------

/// Fake class-ID plan of the Mockingbird generation. Values are arbitrary and
/// deliberately unlike any real NVIDIA class id.
pub mod mock_classes {
    use nvkvm_arch::ids::ClassId;

    /// Client root.
    pub const CLIENT: ClassId = ClassId(0xF001);
    /// Device.
    pub const DEVICE: ClassId = ClassId(0xF002);
    /// Subdevice.
    pub const SUBDEVICE: ClassId = ClassId(0xF003);
    /// VASpace.
    pub const VASPACE: ClassId = ClassId(0xF010);
    /// TSG / channel group.
    pub const TSG: ClassId = ClassId(0xF020);
    /// CtxShare / subcontext.
    pub const CTXSHARE: ClassId = ClassId(0xF021);
    /// GR GPFIFO channel.
    pub const CHANNEL_GR: ClassId = ClassId(0xF030);
    /// CE GPFIFO channel.
    pub const CHANNEL_CE: ClassId = ClassId(0xF031);
    /// Compute engine object.
    pub const COMPUTE: ClassId = ClassId(0xF040);
    /// Copy-engine object.
    pub const DMA_COPY: ClassId = ClassId(0xF041);
    /// Plain memory object.
    pub const MEMORY: ClassId = ClassId(0xF050);
}

/// A complete fake GPU generation. See crate docs.
#[derive(Debug, Default)]
pub struct MockArch {
    mmu: MockGmmuFmt,
    userd: MockUserd,
}

impl MockArch {
    /// New Mockingbird arch.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inverse of [`Arch::vchid_from_userd_flags`] — lets tests declare a channel
    /// with a chosen vChid through the *encoded* form, like a real guest would.
    #[must_use]
    pub fn userd_flags_for(vchid: VChid) -> u32 {
        // Fake packing: vchid in bits [18:7], marker bits low.
        (u32::from(vchid.0 & 0xfff) << 7) | 0b101
    }

    /// Inverse of [`Arch::decode_doorbell`] — build a valid Mockingbird token.
    #[must_use]
    pub fn token_for(vchid: VChid) -> u64 {
        0xD000_0000_0000_0000 | (u64::from(vchid.0 & 0xfff) << 9)
    }
}

impl Arch for MockArch {
    fn name(&self) -> &'static str {
        "mockingbird"
    }

    fn classify(&self, class: ClassId) -> ObjectKind {
        use mock_classes as c;
        match class {
            c::CLIENT => ObjectKind::Client,
            c::DEVICE => ObjectKind::Device,
            c::SUBDEVICE => ObjectKind::Subdevice,
            c::VASPACE => ObjectKind::VaSpace,
            c::TSG => ObjectKind::Tsg,
            c::CTXSHARE => ObjectKind::CtxShare,
            c::CHANNEL_GR => ObjectKind::Channel { engine: EngineClass::Gr },
            c::CHANNEL_CE => ObjectKind::Channel { engine: EngineClass::Ce },
            c::COMPUTE => ObjectKind::EngineObject { engine: EngineClass::Gr },
            c::DMA_COPY => ObjectKind::EngineObject { engine: EngineClass::Ce },
            c::MEMORY => ObjectKind::Other,
            _ => ObjectKind::Unknown,
        }
    }

    fn vchid_from_userd_flags(&self, flags: u32) -> VChid {
        VChid(((flags >> 7) & 0xfff) as u16)
    }

    fn decode_doorbell(&self, token: u64) -> Option<DoorbellTarget> {
        // Malformed unless the fake marker byte is present (hostile-bytes posture).
        if token >> 56 != 0xD0 {
            return None;
        }
        Some(DoorbellTarget { vchid: VChid(((token >> 9) & 0xfff) as u16) })
    }

    fn mmu(&self) -> &dyn GmmuFmt {
        &self.mmu
    }

    fn userd(&self) -> &dyn UserdModel {
        &self.userd
    }
}

/// Fake MMU format: VER2-shaped geometry, fake entry encoding
/// (bit0 = valid, bit1 = leaf, bit2 = sysmem aperture, phys in bits [51:12]).
#[derive(Debug, Default)]
pub struct MockGmmuFmt;

/// Mockingbird leaf sizes (includes a huge leaf so the #13 "every real page size"
/// discipline is exercised by construction).
pub const MOCK_PAGE_SIZES: [PageSize; 4] =
    [PageSize(0x1000), PageSize(0x10000), PageSize(0x200000), PageSize(0x2000_0000)];

impl MockGmmuFmt {
    /// Encode a fake leaf entry (test helper; inverse of `decode_entry`).
    #[must_use]
    pub fn encode_leaf(phys: u64, sysmem: bool) -> u128 {
        u128::from((phys & 0x000f_ffff_ffff_f000) | 0b011 | if sysmem { 0b100 } else { 0 })
    }
}

impl GmmuFmt for MockGmmuFmt {
    fn version(&self) -> GmmuVersion {
        GmmuVersion::Ver2
    }
    fn page_sizes(&self) -> &[PageSize] {
        &MOCK_PAGE_SIZES
    }
    fn entry_size(&self, _level: u8) -> u8 {
        8
    }
    fn levels(&self) -> u8 {
        5
    }
    fn decode_entry(&self, level: u8, raw: u128) -> PteDecode {
        let raw = raw as u64;
        if raw & 0b1 == 0 {
            return PteDecode::Invalid;
        }
        let phys = raw & 0x000f_ffff_ffff_f000;
        let aperture =
            if raw & 0b100 != 0 { Aperture::SysmemCoherent } else { Aperture::Vidmem };
        if raw & 0b10 != 0 {
            // Fake rule: leaf size depends on the level it appears at.
            let size = MOCK_PAGE_SIZES[usize::from(4u8.saturating_sub(level).min(3))];
            PteDecode::Leaf { phys, aperture, size, read_only: false }
        } else {
            PteDecode::Pde { next: phys, aperture }
        }
    }
}

/// Fake USERD geometry.
#[derive(Debug, Default)]
pub struct MockUserd;

impl UserdModel for MockUserd {
    fn userd_size(&self) -> u64 {
        0x400
    }
    fn gp_get_offset(&self) -> u64 {
        0x110
    }
    fn gp_put_offset(&self) -> u64 {
        0x118
    }
}

// ---------------------------------------------------------------------------------
// MockVmm — the scripted hypervisor with a virtual clock
// ---------------------------------------------------------------------------------

/// A recorded memslot installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotRecord {
    /// Guest-physical start.
    pub gpa: u64,
    /// Length.
    pub len: u64,
    /// Backing named by the core.
    pub backing: HostRegion,
    /// Protection / overlay mode.
    pub prot: Prot,
    /// True if installed via `map_read_native`.
    pub read_native: bool,
    /// Locked by `lock_region`?
    pub locked: Option<CoreEventKind>,
}

/// The scripted hypervisor (testing strategy §4). Guest RAM is a sparse byte map;
/// time only moves when the test calls [`MockVmm::advance`].
#[derive(Debug, Default)]
pub struct MockVmm {
    ram: BTreeMap<u64, u8>,
    /// Installed slots by id (public for assertions).
    pub slots: BTreeMap<SlotId, SlotRecord>,
    /// Every `raise_irq` in order (assert completions were delivered).
    pub irqs: Vec<IrqSpec>,
    /// Every `set_trap` registration in order.
    pub traps: Vec<(BarId, Range<u64>, TrapMode)>,
    /// Every `export_ram` call.
    pub exports: Vec<Option<Range<u64>>>,
    deferred: BinaryHeap<Reverse<(Instant, u64, DeferredEntry)>>,
    now: Instant,
    next_slot: u64,
    next_seq: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct DeferredEntry(CoreEvent);

impl MockVmm {
    /// Fresh VMM at virtual time zero.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Advance the virtual clock by `d`, returning every deferred [`CoreEvent`]
    /// that became due, in order. The test then feeds them to the core — exactly
    /// what a real adapter's serialized executor would do.
    pub fn advance(&mut self, d: Duration) -> Vec<CoreEvent> {
        self.now = self.now.advanced(d);
        let mut due = Vec::new();
        while let Some(Reverse((t, _, _))) = self.deferred.peek() {
            if *t > self.now {
                break;
            }
            let Reverse((_, _, DeferredEntry(ev))) = self.deferred.pop().expect("peeked");
            due.push(ev);
        }
        due
    }

    /// Test helper: read back guest RAM (missing bytes read as 0).
    #[must_use]
    pub fn ram_read(&self, gpa: u64, len: usize) -> Vec<u8> {
        (0..len as u64).map(|i| *self.ram.get(&(gpa + i)).unwrap_or(&0)).collect()
    }
}

impl Vmm for MockVmm {
    fn gpa_read(&mut self, gpa: u64, buf: &mut [u8]) -> Result<(), VmmError> {
        for (i, b) in buf.iter_mut().enumerate() {
            *b = *self.ram.get(&(gpa + i as u64)).unwrap_or(&0);
        }
        Ok(())
    }

    fn gpa_write(&mut self, gpa: u64, buf: &[u8]) -> Result<(), VmmError> {
        for (i, &b) in buf.iter().enumerate() {
            self.ram.insert(gpa + i as u64, b);
        }
        Ok(())
    }

    fn map_guest(
        &mut self,
        gpa: u64,
        len: u64,
        backing: HostRegion,
        prot: Prot,
    ) -> Result<SlotId, VmmError> {
        let id = SlotId(self.next_slot);
        self.next_slot += 1;
        self.slots
            .insert(id, SlotRecord { gpa, len, backing, prot, read_native: false, locked: None });
        Ok(id)
    }

    fn unmap_guest(&mut self, slot: SlotId) -> Result<(), VmmError> {
        self.slots.remove(&slot).map(|_| ()).ok_or(VmmError::BadSlot(slot))
    }

    fn set_trap(&mut self, bar: BarId, range: Range<u64>, mode: TrapMode) -> Result<(), VmmError> {
        self.traps.push((bar, range, mode));
        Ok(())
    }

    fn raise_irq(&mut self, irq: IrqSpec) -> Result<(), VmmError> {
        self.irqs.push(irq);
        Ok(())
    }

    fn export_ram(&mut self, slice: Option<Range<u64>>) -> Result<RamHandle, VmmError> {
        self.exports.push(slice.clone());
        Ok(RamHandle { token: self.exports.len() as u64, covers: slice })
    }

    fn defer(&mut self, after: Duration, event: CoreEvent) {
        let at = self.now.advanced(after);
        let seq = self.next_seq;
        self.next_seq += 1;
        self.deferred.push(Reverse((at, seq, DeferredEntry(event))));
    }

    fn now(&self) -> Instant {
        self.now
    }

    fn map_read_native(
        &mut self,
        gpa: u64,
        len: u64,
        backing: HostRegion,
        write_trap: Option<Range<u64>>,
    ) -> Result<SlotId, VmmError> {
        let id = SlotId(self.next_slot);
        self.next_slot += 1;
        self.slots.insert(
            id,
            SlotRecord { gpa, len, backing, prot: Prot::ReadOnly, read_native: true, locked: None },
        );
        if let Some(r) = write_trap {
            self.traps.push((BarId::Bar0, r, TrapMode::WriteOnly));
        }
        Ok(id)
    }

    fn lock_region(&mut self, slot: SlotId, on_fault: CoreEventKind) -> Result<(), VmmError> {
        match self.slots.get_mut(&slot) {
            Some(rec) => {
                rec.locked = Some(on_fault);
                Ok(())
            }
            None => Err(VmmError::BadSlot(slot)),
        }
    }

    fn unlock_region(&mut self, slot: SlotId) -> Result<(), VmmError> {
        match self.slots.get_mut(&slot) {
            Some(rec) => {
                rec.locked = None;
                Ok(())
            }
            None => Err(VmmError::BadSlot(slot)),
        }
    }
}

// ---------------------------------------------------------------------------------
// MockRmBackend / MockIsolate — a fake host RM with per-isolate namespaces
// ---------------------------------------------------------------------------------

/// One recorded RM verb (the forwarded-op log the tests assert on).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RmVerb {
    /// Generic alloc.
    Alloc {
        /// Parent handle.
        parent: HostHandle,
        /// Class id.
        class: ClassId,
        /// Returned handle.
        handle: HostHandle,
    },
    /// Intent: host VAS allocated.
    AllocVaSpace {
        /// Returned handle.
        handle: HostHandle,
    },
    /// Intent: host sysmem allocated.
    AllocSysmem {
        /// Returned handle.
        handle: HostHandle,
        /// Requested length.
        len: u64,
    },
    /// Intent: host channel allocated on a host VAS.
    AllocChannel {
        /// The host VAS.
        vas: HostHandle,
        /// Returned channel handle.
        handle: HostHandle,
        /// Returned host work-submit token.
        token: u64,
    },
    /// Intent: channel made runnable.
    Schedule {
        /// The channel.
        chan: HostHandle,
    },
    /// Host GPU VA mapped.
    MapGpuVa {
        /// The host VAS mapped into.
        vas: HostHandle,
        /// The memory object.
        memory: HostHandle,
        /// Length.
        len: u64,
        /// Returned host GPU VA.
        va: u64,
    },
    /// Host GPU VA unmapped.
    UnmapGpuVa {
        /// The host VAS.
        vas: HostHandle,
        /// The VA.
        va: u64,
    },
    /// Doorbell rung with a host token.
    RingDoorbell {
        /// The token.
        token: u64,
    },
    /// Object freed.
    Free {
        /// The handle.
        obj: HostHandle,
    },
    /// Control issued.
    Control {
        /// Target object.
        obj: HostHandle,
        /// Command.
        cmd: ControlCmd,
    },
}

/// Shared recorder: `(isolate, verb)` in global order, so tests can assert both
/// per-isolate behavior and cross-isolate separation. Plus a scriptable failure.
#[derive(Debug, Default)]
pub struct RmRecorder {
    /// The global verb log.
    pub log: Vec<(IsolateId, RmVerb)>,
    /// If set, the next verb on ANY isolate fails with this error (then clears).
    pub fail_next: Option<RmError>,
}

/// Handle to the shared recorder, held by the test.
pub type SharedRecorder = Arc<Mutex<RmRecorder>>;

/// A fake host RM connection with a private handle namespace.
///
/// Handle values are namespaced by isolate id (`(id+1) << 32 | n`) so any
/// cross-isolate handle use is *visible* in assertions; validity is still
/// enforced per-backend ([`RmError::BadHandle`]) — the blast-radius property.
#[derive(Debug)]
pub struct MockRmBackend {
    id: IsolateId,
    recorder: SharedRecorder,
    handles: BTreeSet<HostHandle>,
    next: u64,
    next_token: u64,
    next_va: u64,
    retired: bool,
}

impl MockRmBackend {
    fn new(id: IsolateId, recorder: SharedRecorder) -> Self {
        MockRmBackend {
            id,
            recorder,
            handles: BTreeSet::new(),
            next: 1,
            // Namespaced fake host tokens / host VAs: disjoint across isolates by
            // construction, so "disjoint host backing" is directly assertable.
            next_token: (u64::from(id.0) + 1) << 20,
            next_va: 0x7000_0000_0000 + ((u64::from(id.0) + 1) << 36),
            retired: false,
        }
    }

    fn gate(&mut self) -> Result<(), RmError> {
        if self.retired {
            return Err(RmError::Other(0xdead));
        }
        if let Some(e) = self.recorder.lock().expect("recorder").fail_next.take() {
            return Err(e);
        }
        Ok(())
    }

    fn mint(&mut self) -> HostHandle {
        let h = HostHandle(((u64::from(self.id.0) + 1) << 32) | self.next);
        self.next += 1;
        self.handles.insert(h);
        h
    }

    fn check(&self, h: HostHandle) -> Result<(), RmError> {
        if self.handles.contains(&h) { Ok(()) } else { Err(RmError::BadHandle(h)) }
    }

    fn record(&self, verb: RmVerb) {
        self.recorder.lock().expect("recorder").log.push((self.id, verb));
    }
}

impl RmBackend for MockRmBackend {
    fn alloc(
        &mut self,
        parent: HostHandle,
        class: ClassId,
        _params: &[u8],
    ) -> Result<HostHandle, RmError> {
        self.gate()?;
        if parent != HostHandle(0) {
            self.check(parent)?;
        }
        let handle = self.mint();
        self.record(RmVerb::Alloc { parent, class, handle });
        Ok(handle)
    }

    fn alloc_vaspace(&mut self) -> Result<HostHandle, RmError> {
        self.gate()?;
        let handle = self.mint();
        self.record(RmVerb::AllocVaSpace { handle });
        Ok(handle)
    }

    fn alloc_sysmem(&mut self, len: u64) -> Result<HostHandle, RmError> {
        self.gate()?;
        let handle = self.mint();
        self.record(RmVerb::AllocSysmem { handle, len });
        Ok(handle)
    }

    fn alloc_channel(&mut self, vas: HostHandle) -> Result<(HostHandle, u64), RmError> {
        self.gate()?;
        self.check(vas)?;
        let handle = self.mint();
        let token = self.next_token;
        self.next_token += 1;
        self.record(RmVerb::AllocChannel { vas, handle, token });
        Ok((handle, token))
    }

    fn schedule(&mut self, chan: HostHandle) -> Result<(), RmError> {
        self.gate()?;
        self.check(chan)?;
        self.record(RmVerb::Schedule { chan });
        Ok(())
    }

    fn free(&mut self, obj: HostHandle) -> Result<(), RmError> {
        self.gate()?;
        self.check(obj)?;
        self.handles.remove(&obj);
        self.record(RmVerb::Free { obj });
        Ok(())
    }

    fn control(
        &mut self,
        obj: HostHandle,
        cmd: ControlCmd,
        _payload: &mut [u8],
    ) -> Result<(), RmError> {
        self.gate()?;
        self.check(obj)?;
        self.record(RmVerb::Control { obj, cmd });
        Ok(())
    }

    fn map_gpu_va(
        &mut self,
        vas: HostHandle,
        memory: HostHandle,
        len: u64,
    ) -> Result<u64, RmError> {
        self.gate()?;
        self.check(vas)?;
        self.check(memory)?;
        // Fake placement: per-isolate VA range + per-VAS lane, so distinct host
        // VASes yield visibly distinct host VAs.
        let va = self.next_va | ((vas.0 & 0xff) << 28);
        self.next_va += len.next_multiple_of(0x1000);
        self.record(RmVerb::MapGpuVa { vas, memory, len, va });
        Ok(va)
    }

    fn unmap_gpu_va(&mut self, vas: HostHandle, gpu_va: u64) -> Result<(), RmError> {
        self.gate()?;
        self.check(vas)?;
        self.record(RmVerb::UnmapGpuVa { vas, va: gpu_va });
        Ok(())
    }

    fn ring_doorbell(&mut self, host_token: u64) -> Result<(), RmError> {
        self.gate()?;
        self.record(RmVerb::RingDoorbell { token: host_token });
        Ok(())
    }
}

/// A fake per-process isolate wrapping one [`MockRmBackend`].
#[derive(Debug)]
pub struct MockIsolate {
    id: IsolateId,
    rm: MockRmBackend,
    retired: bool,
}

impl Isolate for MockIsolate {
    fn id(&self) -> IsolateId {
        self.id
    }
    fn rm(&mut self) -> &mut dyn RmBackend {
        &mut self.rm
    }
    fn retire(&mut self) {
        self.retired = true;
        self.rm.retired = true;
    }
    fn is_retired(&self) -> bool {
        self.retired
    }
}

/// Spawns [`MockIsolate`]s that all record into one [`SharedRecorder`].
#[derive(Debug, Default)]
pub struct MockIsolateFactory {
    recorder: SharedRecorder,
    /// Every spawned session id, in order (assert isolate-per-proc).
    pub spawned: Vec<IsolateId>,
}

impl MockIsolateFactory {
    /// Create a factory + the recorder handle the test keeps.
    #[must_use]
    pub fn new() -> (Self, SharedRecorder) {
        let recorder: SharedRecorder = Arc::default();
        (MockIsolateFactory { recorder: Arc::clone(&recorder), spawned: Vec::new() }, recorder)
    }
}

impl IsolateFactory for MockIsolateFactory {
    fn spawn(&mut self, id: IsolateId) -> Box<dyn Isolate> {
        self.spawned.push(id);
        Box::new(MockIsolate {
            id,
            rm: MockRmBackend::new(id, Arc::clone(&self.recorder)),
            retired: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_arch_token_roundtrip_and_malformed_rejected() {
        let a = MockArch::new();
        let v = VChid(0x2a);
        assert_eq!(a.decode_doorbell(MockArch::token_for(v)), Some(DoorbellTarget { vchid: v }));
        assert_eq!(a.vchid_from_userd_flags(MockArch::userd_flags_for(v)), v);
        assert_eq!(a.decode_doorbell(0x1234), None, "hostile token must not decode");
    }

    #[test]
    fn mock_vmm_virtual_clock_orders_deferred_events() {
        let mut vmm = MockVmm::new();
        vmm.defer(Duration::from_millis(2), CoreEvent::Deferred(CoreEventKind::DeferredReap));
        vmm.defer(
            Duration::from_millis(1),
            CoreEvent::Deferred(CoreEventKind::CompletionRedeliver),
        );
        assert!(vmm.advance(Duration::from_micros(500)).is_empty(), "nothing due yet");
        let due = vmm.advance(Duration::from_millis(2));
        assert_eq!(
            due,
            vec![
                CoreEvent::Deferred(CoreEventKind::CompletionRedeliver),
                CoreEvent::Deferred(CoreEventKind::DeferredReap),
            ],
            "due in deadline order, deterministically"
        );
    }

    #[test]
    fn mock_rm_handles_are_isolate_scoped() {
        let (mut f, _rec) = MockIsolateFactory::new();
        let mut a = f.spawn(IsolateId(1));
        let mut b = f.spawn(IsolateId(2));
        let ha = a.rm().alloc_vaspace().unwrap();
        // Using isolate A's handle on isolate B is refused: blast-radius containment.
        assert_eq!(b.rm().schedule(ha), Err(RmError::BadHandle(ha)));
    }
}
