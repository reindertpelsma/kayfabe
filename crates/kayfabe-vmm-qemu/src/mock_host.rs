//! ★★ `MockQemuHost` — the other implementor of [`QemuHost`], and the reason stage Q1
//! needs no hypervisor.
//!
//! It is deliberately shaped like the **awkward** host rather than the easy one: every
//! refusal arm is reachable by construction, because a mock whose only behaviour is
//! success is a mock that can only test the happy path — the failure this project has
//! caught most often is a green instrument on an unexercised path, and a mock is an
//! instrument.
//!
//! ## What it is not
//!
//! Not a simulation of the hypervisor. It models exactly the observable contract
//! [`QemuHost`] states — regions with bounded backings, reference counts, an
//! interrupt-vector log and an ordered call log — and nothing else. In particular it does
//! **not** model the global lock: §4's whole claim is that the adapter never takes it,
//! and a mock that pretended to hold one would let a test "prove" a property the real
//! host cannot.
//!
//! ## ★ It is a normal module, not `#[cfg(test)]`
//!
//! Integration tests live in their own crate and cannot see a `cfg(test)` item, and the
//! suite that matters here is the integration one. The cost is one public type in a
//! shipping build; the alternative is a self-referential dev-dependency with a feature
//! flag, which would compile this crate twice and give a mutation run two copies of every
//! function to score.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use crate::host::{BarPlacement, BlockerId, HostError, MrHandle, QemuHost, SectionDesc};
use crate::slots::{KERNEL_EEXIST, KERNEL_EINVAL, LiveSlot, SlotPlane};
use kayfabe_linux_raw::{GuestWindow, HostPageSize, RawError, geometry};
use kayfabe_vmm::BarId;

/// One call the adapter made into the host, in order.
///
/// ★ The **ordered** log is the instrument, not a set of counters: §8.1's realize order
/// is load-bearing (the migration blocker before anything is mapped, the discard refusal
/// before the reservation exists) and a set of counters cannot see an order at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostCall {
    /// The runtime version was read.
    Version,
    /// The accelerator was queried.
    KvmEnabled,
    /// A migration blocker was added.
    AddBlocker(BlockerId),
    /// A migration blocker was withdrawn.
    DelBlocker(BlockerId),
    /// Guest-driven RAM discard was disabled (`true`) or re-enabled (`false`).
    DiscardDisable(bool),
    /// The topology listener was registered.
    RegisterListener,
    /// A BAR was asked whether it is an unbacked reservation.
    ReservationShape(BarId),
    /// A BAR's current base was read — the latch check.
    BarBase(BarId),
    /// A reference to a reported region was taken.
    RefRegion(MrHandle),
    /// A reference to a reported region was released.
    UnrefRegion(MrHandle),
    /// A bounded read out of a region.
    ReadRegion(MrHandle, u64, u64),
    /// A bounded write into a region.
    WriteRegion(MrHandle, u64, u64),
    /// A message-signalled vector was raised.
    SignalMsix(u16),
}

/// One region the mock host owns.
#[derive(Debug, Clone)]
struct Region {
    bytes: Vec<u8>,
    refs: u64,
    live: bool,
}

/// How the mock should behave when the adapter asks it for something.
///
/// Every field defaults to the cooperative answer, so a test names only the arm it is
/// about — and every arm is reachable, which is the point.
#[derive(Debug, Clone)]
pub struct MockPolicy {
    /// What [`QemuHost::version`] reports.
    pub version: (u32, u32),
    /// What [`QemuHost::kvm_enabled`] reports.
    pub kvm_enabled: bool,
    /// If set, [`QemuHost::migrate_add_blocker`] refuses with it.
    pub blocker_refuses: Option<HostError>,
    /// If set, [`QemuHost::ram_block_discard_disable`] refuses with it.
    pub discard_refuses: Option<HostError>,
    /// If set, [`QemuHost::register_listener`] refuses with it.
    pub listener_refuses: Option<HostError>,
    /// BARs the shim registered as unbacked pure-MMIO reservations. A window placed in a
    /// BAR outside this set is refused at install — see
    /// [`crate::host::QemuHost::bar_is_unbacked_reservation`].
    pub reservation_bars: Vec<BarId>,
    /// If set, [`QemuHost::signal_msix`] refuses with it.
    pub msix_refuses: Option<HostError>,
    /// Vectors the device was realized with. A vector outside this is refused.
    pub msix_vectors: u16,
}

impl Default for MockPolicy {
    fn default() -> Self {
        MockPolicy {
            version: (10, 2),
            kvm_enabled: true,
            blocker_refuses: None,
            discard_refuses: None,
            listener_refuses: None,
            reservation_bars: vec![BarId::Bar0, BarId::Bar1, BarId::Bar2],
            msix_refuses: None,
            msix_vectors: 8,
        }
    }
}

/// A [`QemuHost`] with no hypervisor behind it.
#[derive(Debug)]
pub struct MockQemuHost {
    policy: MockPolicy,
    state: Mutex<MockState>,
    next_mr: AtomicU64,
    next_blocker: AtomicU64,
}

#[derive(Debug, Default)]
struct MockState {
    regions: BTreeMap<MrHandle, Region>,
    log: Vec<HostCall>,
    irqs: Vec<u16>,
    discard_disabled: bool,
    blockers: Vec<BlockerId>,
    listening: bool,
    /// ★ Where each BAR is **currently** programmed. A test moves a BAR by writing here,
    /// which is exactly what a guest reprogramming the base register does to the
    /// hypervisor's own bookkeeping (`C: virtio_nvgpu_pci.c:71-76` reads that field and
    /// nothing else).
    bar_bases: Vec<(BarId, Option<u64>)>,
}

impl MockQemuHost {
    /// A cooperative host.
    #[must_use]
    pub fn new() -> Self {
        Self::with_policy(MockPolicy::default())
    }

    /// A host that behaves as `policy` says.
    #[must_use]
    pub fn with_policy(policy: MockPolicy) -> Self {
        MockQemuHost {
            policy,
            state: Mutex::new(MockState::default()),
            next_mr: AtomicU64::new(1),
            next_blocker: AtomicU64::new(1),
        }
    }

    /// The ordered call log.
    #[must_use]
    pub fn log(&self) -> Vec<HostCall> {
        self.state.lock().expect("mock state").log.clone()
    }

    /// The vectors raised, in order.
    #[must_use]
    pub fn irqs(&self) -> Vec<u16> {
        self.state.lock().expect("mock state").irqs.clone()
    }

    /// Whether guest-driven RAM discard is currently disabled.
    #[must_use]
    pub fn discard_disabled(&self) -> bool {
        self.state.lock().expect("mock state").discard_disabled
    }

    /// The migration blockers currently outstanding. **Must be empty after unrealize**,
    /// or a device that failed to realize left the machine unmigratable forever.
    #[must_use]
    pub fn blockers(&self) -> Vec<BlockerId> {
        self.state.lock().expect("mock state").blockers.clone()
    }

    /// Whether the topology listener is registered.
    #[must_use]
    pub fn listening(&self) -> bool {
        self.state.lock().expect("mock state").listening
    }

    /// Regions still published, with their outstanding reference counts.
    ///
    /// ★ The conservation instrument: at quiescence every region we published must be
    /// gone and every reference we took must have been released. A leaked reference is a
    /// region the hypervisor can never finalize, and nothing else in the suite would see
    /// it.
    #[must_use]
    pub fn live_regions(&self) -> Vec<(MrHandle, u64)> {
        self.state
            .lock()
            .expect("mock state")
            .regions
            .iter()
            .filter(|(_, r)| r.live)
            .map(|(h, r)| (*h, r.refs))
            .collect()
    }

    /// The bytes currently in a region — how a test sees what a write actually landed on.
    #[must_use]
    pub fn region_bytes(&self, mr: MrHandle) -> Option<Vec<u8>> {
        self.state
            .lock()
            .expect("mock state")
            .regions
            .get(&mr)
            .map(|r| r.bytes.clone())
    }

    /// Mint a region the adapter did not publish — the foreign guest RAM the topology
    /// listener reports. Returns a [`SectionDesc`] ready to hand to
    /// [`crate::QemuMachine::region_add`].
    pub fn mint_foreign(
        &self,
        gpa: u64,
        len: u64,
        facts: crate::host::SectionFacts,
    ) -> SectionDesc {
        self.mint_foreign_slice(gpa, len, 0, len, facts)
    }

    /// ★★ The same, but reporting a section that is a **slice** of a larger region.
    ///
    /// A reported section carries an offset *within its region* as well as its
    /// guest-physical base, and the two are independent: a region aliased into the
    /// address space at a partial offset, or mapped in two pieces, produces exactly this.
    /// Every section the rest of this suite mints starts at offset zero, which makes the
    /// adapter's `region_off + span.offset` and a plain `span.offset` indistinguishable —
    /// a bite-check found precisely that survivor, and this constructor is what closes it.
    ///
    /// # Panics
    /// If the section does not fit inside the region it claims to be a slice of.
    pub fn mint_foreign_slice(
        &self,
        gpa: u64,
        len: u64,
        offset_within_region: u64,
        region_len: u64,
        facts: crate::host::SectionFacts,
    ) -> SectionDesc {
        assert!(
            offset_within_region + len <= region_len,
            "a section must fit inside the region it is a slice of"
        );
        let mr = MrHandle(self.next_mr.fetch_add(1, Ordering::SeqCst));
        self.state.lock().expect("mock state").regions.insert(
            mr,
            Region {
                bytes: vec![0; usize::try_from(region_len).expect("a test-sized region")],
                refs: 0,
                live: true,
            },
        );
        SectionDesc {
            mr,
            gpa,
            len,
            offset_within_region,
            facts,
            // ⊘ `None`, and deliberately so: a mock that invented a backing identity would
            // let every existing test state a layout by accident, and the layout tests would
            // then be asserting over rows nobody wrote. Use [`Self::mint_backed`] to state
            // one on purpose.
            backing: None,
        }
    }

    /// ★★★ Mint a section that also names the **file** behind it — what a hypervisor
    /// reports for RAM it backs with a descriptor.
    ///
    /// This is the only constructor in the mock that causes a run to be stated, because
    /// stating a layout must be something a test *asks for* rather than something it gets by
    /// calling the ordinary helper.
    ///
    /// # Panics
    /// If the section does not fit inside the region it claims to be a slice of.
    pub fn mint_backed(
        &self,
        gpa: u64,
        len: u64,
        facts: crate::host::SectionFacts,
        backing: crate::host::SectionBacking,
    ) -> SectionDesc {
        SectionDesc {
            backing: Some(backing),
            ..self.mint_foreign(gpa, len, facts)
        }
    }

    /// ★★ Program a BAR at `base` — what firmware does at enumeration.
    ///
    /// A window cannot be installed into a BAR that has never been programmed, which is
    /// the C's own guard (`C: nvkvm_mmap_host.c:194-207`: a base of 0 means *"the guest
    /// hasn't programmed the BAR yet"* and the install is deferred rather than cached at a
    /// fallback).
    pub fn place_bar(&self, bar: BarId, base: u64) {
        set_bar_base(
            &mut self.state.lock().expect("mock state").bar_bases,
            bar,
            Some(base),
        );
    }

    /// ★★★ **Move a BAR out from under a live memslot.** The whole subject of
    /// `host_execution_plane.md` §1.5's latent-bug finding: the hypervisor's region follows
    /// the guest's write and our memslot does not.
    ///
    /// `None` unmaps it, which is the transient state PCI BAR sizing passes through.
    pub fn move_bar(&self, bar: BarId, base: Option<u64>) {
        set_bar_base(
            &mut self.state.lock().expect("mock state").bar_bases,
            bar,
            base,
        );
    }

    fn record(&self, c: HostCall) {
        self.state.lock().expect("mock state").log.push(c);
    }
}

impl Default for MockQemuHost {
    fn default() -> Self {
        Self::new()
    }
}

impl QemuHost for MockQemuHost {
    fn version(&self) -> (u32, u32) {
        self.record(HostCall::Version);
        self.policy.version
    }

    fn kvm_enabled(&self) -> bool {
        self.record(HostCall::KvmEnabled);
        self.policy.kvm_enabled
    }

    fn migrate_add_blocker(&self, _reason: &'static str) -> Result<BlockerId, HostError> {
        if let Some(e) = &self.policy.blocker_refuses {
            return Err(e.clone());
        }
        let id = BlockerId(self.next_blocker.fetch_add(1, Ordering::SeqCst));
        let mut st = self.state.lock().expect("mock state");
        st.blockers.push(id);
        st.log.push(HostCall::AddBlocker(id));
        Ok(id)
    }

    fn migrate_del_blocker(&self, id: BlockerId) {
        let mut st = self.state.lock().expect("mock state");
        st.blockers.retain(|b| *b != id);
        st.log.push(HostCall::DelBlocker(id));
    }

    fn ram_block_discard_disable(&self, disable: bool) -> Result<(), HostError> {
        if disable && let Some(e) = &self.policy.discard_refuses {
            return Err(e.clone());
        }
        let mut st = self.state.lock().expect("mock state");
        st.discard_disabled = disable;
        st.log.push(HostCall::DiscardDisable(disable));
        Ok(())
    }

    fn register_listener(&self) -> Result<(), HostError> {
        if let Some(e) = &self.policy.listener_refuses {
            return Err(e.clone());
        }
        let mut st = self.state.lock().expect("mock state");
        st.listening = true;
        st.log.push(HostCall::RegisterListener);
        Ok(())
    }

    fn bar_is_unbacked_reservation(&self, bar: BarId) -> bool {
        self.record(HostCall::ReservationShape(bar));
        self.policy.reservation_bars.contains(&bar)
    }

    fn bar_base(&self, bar: BarId) -> Option<u64> {
        let mut st = self.state.lock().expect("mock state");
        st.log.push(HostCall::BarBase(bar));
        st.bar_bases
            .iter()
            .find(|(b, _)| *b == bar)
            .and_then(|(_, base)| *base)
    }

    fn ref_region(&self, mr: MrHandle) -> Result<(), HostError> {
        let mut st = self.state.lock().expect("mock state");
        let Some(r) = st.regions.get_mut(&mr) else {
            return Err(HostError::Refused {
                what: "taking a reference to an unknown region",
                errno: None,
            });
        };
        r.refs += 1;
        st.log.push(HostCall::RefRegion(mr));
        Ok(())
    }

    fn unref_region(&self, mr: MrHandle) {
        let mut st = self.state.lock().expect("mock state");
        if let Some(r) = st.regions.get_mut(&mr) {
            r.refs = r.refs.saturating_sub(1);
        }
        st.log.push(HostCall::UnrefRegion(mr));
    }

    fn read_region(&self, mr: MrHandle, off: u64, dst: &mut [u8]) -> Result<(), HostError> {
        let mut st = self.state.lock().expect("mock state");
        let Some(r) = st.regions.get(&mr) else {
            return Err(HostError::Refused {
                what: "a read out of an unknown region",
                errno: None,
            });
        };
        let start = usize::try_from(off).map_err(|_| HostError::Refused {
            what: "a read past the end of a region",
            errno: None,
        })?;
        let end = start
            .checked_add(dst.len())
            .filter(|e| *e <= r.bytes.len())
            .ok_or(HostError::Refused {
                what: "a read past the end of a region",
                errno: None,
            })?;
        dst.copy_from_slice(&r.bytes[start..end]);
        st.log.push(HostCall::ReadRegion(mr, off, dst.len() as u64));
        Ok(())
    }

    fn write_region(&self, mr: MrHandle, off: u64, src: &[u8]) -> Result<(), HostError> {
        let mut st = self.state.lock().expect("mock state");
        let Some(r) = st.regions.get_mut(&mr) else {
            return Err(HostError::Refused {
                what: "a write into an unknown region",
                errno: None,
            });
        };
        let start = usize::try_from(off).map_err(|_| HostError::Refused {
            what: "a write past the end of a region",
            errno: None,
        })?;
        let end = start
            .checked_add(src.len())
            .filter(|e| *e <= r.bytes.len())
            .ok_or(HostError::Refused {
                what: "a write past the end of a region",
                errno: None,
            })?;
        r.bytes[start..end].copy_from_slice(src);
        st.log
            .push(HostCall::WriteRegion(mr, off, src.len() as u64));
        Ok(())
    }

    fn signal_msix(&self, vector: u16) -> Result<(), HostError> {
        if let Some(e) = &self.policy.msix_refuses {
            return Err(e.clone());
        }
        if vector >= self.policy.msix_vectors {
            return Err(HostError::Unsupported(
                "a vector this device was not realized with",
            ));
        }
        let mut st = self.state.lock().expect("mock state");
        st.irqs.push(vector);
        st.log.push(HostCall::SignalMsix(vector));
        Ok(())
    }
}

/// `BarId` is deliberately not ordered in the port's vocabulary, so the mock's own base
/// table is an association list rather than a map. Three BARs; the linear scan is the
/// honest shape.
fn set_bar_base(table: &mut Vec<(BarId, Option<u64>)>, bar: BarId, base: Option<u64>) {
    match table.iter_mut().find(|(b, _)| *b == bar) {
        Some(slot) => slot.1 = base,
        None => table.push((bar, base)),
    }
}

/// A BAR placement a test can hand to [`crate::MachineConfig`] without repeating itself.
#[must_use]
pub fn bar(bar: BarId, base: u64, len: u64) -> BarPlacement {
    BarPlacement { bar, base, len }
}

// =====================================================================================
// The memslot plane, without a kernel
// =====================================================================================

/// One memslot the mock kernel is holding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MockSlotRecord {
    /// The slot number.
    pub slot: u32,
    /// Guest-physical base.
    pub gpa: u64,
    /// Length in bytes.
    pub len: u64,
    /// Whether the guest may only read it.
    pub readonly: bool,
}

/// ★★★ A [`SlotPlane`] with no kernel — **and it refuses where the kernel refuses**.
///
/// `host_execution_plane.md` §2.1's rule, applied to the memory plane: *"the mock must lie
/// where the real host lies"*. A `BTreeMap::insert` would accept a misaligned base, an
/// overlapping range and a number past the ceiling, so a suite built on one would assert
/// only that the adapter's bookkeeping is self-consistent. The three arms modelled here are
/// the three the real kernel has, and each is measured in
/// `kayfabe_linux_raw::kvm_unsafe`'s own tests:
///
/// | request | kernel | modelled |
/// |---|---|---|
/// | a base that is not page-aligned | `EINVAL` | yes |
/// | a range another live slot covers | `EEXIST` | yes |
/// | a number at or past the ceiling | `EINVAL` | yes |
///
/// ★ The **fourth** arm is deliberately absent and named instead: a number that is already
/// live is a *replace*, not a refusal, and the kernel says nothing. That is the hazard
/// [`crate::slots::SlotAllocator`] exists for, so this mock reproduces the silence — see
/// [`MockSlotPlane::replaces`], which counts it. A mock that refused a duplicate number
/// would make the allocator's whole argument untestable by making the failure impossible.
#[derive(Debug)]
pub struct MockSlotPlane {
    ceiling: u32,
    page: u64,
    state: Arc<Mutex<MockSlotState>>,
}

#[derive(Debug, Default)]
struct MockSlotState {
    live: Vec<MockSlotRecord>,
    installs: u64,
    clears: u64,
    replaces: u64,
    read_only_installs: u64,
}

/// The handle a mock install returns. Dropping it clears the slot.
#[derive(Debug)]
pub struct MockLiveSlot {
    state: Weak<Mutex<MockSlotState>>,
    slot: u32,
}

impl LiveSlot for MockLiveSlot {}

impl Drop for MockLiveSlot {
    fn drop(&mut self) {
        if let Some(st) = self.state.upgrade() {
            let mut st = st.lock().expect("mock slot state");
            st.live.retain(|r| r.slot != self.slot);
            st.clears += 1;
        }
    }
}

impl MockSlotPlane {
    /// A plane whose kernel reports `ceiling` memslots and a `page`-byte page.
    #[must_use]
    pub fn new(ceiling: u32, page: u64) -> Self {
        MockSlotPlane {
            ceiling,
            page,
            state: Arc::new(Mutex::new(MockSlotState::default())),
        }
    }

    /// The slots currently live, sorted by guest-physical base.
    ///
    /// ★ This is how a test sees the **tiering**: which ranges have a slot at all
    /// (passthrough or read-native) and which do not (observe-everything). Nothing else in
    /// the adapter can distinguish "no slot" from "a slot nobody looked at".
    #[must_use]
    pub fn live(&self) -> Vec<MockSlotRecord> {
        let mut v = self.state.lock().expect("mock slot state").live.clone();
        v.sort_unstable_by_key(|r| r.gpa);
        v
    }

    /// Cumulative installs — the memslot-frequency numerator (§9.3).
    #[must_use]
    pub fn installs(&self) -> u64 {
        self.state.lock().expect("mock slot state").installs
    }

    /// Cumulative clears. At quiescence this must equal [`MockSlotPlane::installs`], or a
    /// slot is live in a kernel nobody is going to tell.
    #[must_use]
    pub fn clears(&self) -> u64 {
        self.state.lock().expect("mock slot state").clears
    }

    /// Installs that carried the read-only flag — the read-native tier's own counter.
    #[must_use]
    pub fn read_only_installs(&self) -> u64 {
        self.state
            .lock()
            .expect("mock slot state")
            .read_only_installs
    }

    /// ★★★ Installs that **silently replaced** a live slot with the same number.
    ///
    /// Must be zero. The kernel does not report this and neither does this mock; it is
    /// counted so a test can assert the absence, which is the only form the assertion can
    /// take. A non-zero value means the allocator handed out a number that was still live —
    /// exactly the failure counting down from the ceiling is meant to make unreachable.
    #[must_use]
    pub fn replaces(&self) -> u64 {
        self.state.lock().expect("mock slot state").replaces
    }
}

impl SlotPlane for MockSlotPlane {
    fn ceiling(&self) -> Result<u32, RawError> {
        Ok(self.ceiling)
    }

    fn install(
        &self,
        slot: u32,
        gpa: u64,
        window: &Arc<GuestWindow>,
        window_offset: u64,
        len: u64,
        guest_readonly: bool,
    ) -> Result<Box<dyn LiveSlot>, RawError> {
        // ★★★ THE PRE-SYSCALL CHECKS, IN THE REAL ONE'S ORDER — and the order is the
        // finding. The real install asks the window for the host address of
        // `window_offset..+len`, and that accessor refuses **before any syscall happens**:
        // first the two alignments, then the bounds. So a misaligned *length* is
        // `RawError::Misaligned`, NOT the kernel's `EINVAL` — a distinction the first draft
        // of this mock got wrong and the real-kernel differential caught, which is exactly
        // what that differential exists for.
        //
        // Getting this wrong in the other direction would be worse than untidy: a mock that
        // reported `EINVAL` where the real path reports `Misaligned` teaches every test
        // above it to expect the kernel's answer for a condition the kernel never sees.
        geometry::require_aligned(window_offset, HostPageSize::query(), "memslot offset")?;
        geometry::require_aligned(len, HostPageSize::query(), "memslot length")?;
        if len == 0 {
            return Err(RawError::ZeroLength {
                what: "memslot length",
            });
        }
        if window_offset.checked_add(len).is_none() {
            return Err(RawError::LengthOverflow {
                offset: window_offset,
                len,
            });
        }
        if window_offset + len > window.len_bytes() {
            return Err(RawError::OutOfRange {
                offset: window_offset,
                len,
                object_len: window.len_bytes(),
            });
        }
        // Only now the kernel's own refusals.
        if slot >= self.ceiling || !gpa.is_multiple_of(self.page) {
            return Err(RawError::Syscall {
                call: "KVM_SET_USER_MEMORY_REGION",
                errno: Some(KERNEL_EINVAL),
            });
        }
        let mut st = self.state.lock().expect("mock slot state");
        if st
            .live
            .iter()
            .any(|r| r.slot != slot && gpa < r.gpa + r.len && r.gpa < gpa + len)
        {
            return Err(RawError::Syscall {
                call: "KVM_SET_USER_MEMORY_REGION",
                errno: Some(KERNEL_EEXIST),
            });
        }
        if st.live.iter().any(|r| r.slot == slot) {
            // The silence, reproduced. See the type docs.
            st.replaces += 1;
            st.live.retain(|r| r.slot != slot);
        }
        st.live.push(MockSlotRecord {
            slot,
            gpa,
            len,
            readonly: guest_readonly,
        });
        st.installs += 1;
        if guest_readonly {
            st.read_only_installs += 1;
        }
        Ok(Box::new(MockLiveSlot {
            state: Arc::downgrade(&self.state),
            slot,
        }))
    }
}
