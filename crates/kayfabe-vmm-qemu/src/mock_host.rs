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
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::host::{BarPlacement, BlockerId, HostError, MrHandle, QemuHost, SectionDesc};
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
    /// One of our reservations was published.
    PublishWindow(MrHandle),
    /// A read-native overlay was published.
    PublishRomOverlay(MrHandle),
    /// A region we published was removed.
    Unpublish(MrHandle),
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
    /// Refuse the `n`-th region publication (0-based), so a **partial** realize can be
    /// constructed on purpose.
    pub publish_refuses_at: Option<u64>,
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
            publish_refuses_at: None,
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
    publishes: AtomicU64,
}

#[derive(Debug, Default)]
struct MockState {
    regions: BTreeMap<MrHandle, Region>,
    log: Vec<HostCall>,
    irqs: Vec<u16>,
    discard_disabled: bool,
    blockers: Vec<BlockerId>,
    listening: bool,
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
            publishes: AtomicU64::new(0),
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
        }
    }

    fn record(&self, c: HostCall) {
        self.state.lock().expect("mock state").log.push(c);
    }

    fn publish(&self, len: u64, tag: fn(MrHandle) -> HostCall) -> Result<MrHandle, HostError> {
        let n = self.publishes.fetch_add(1, Ordering::SeqCst);
        if self.policy.publish_refuses_at == Some(n) {
            return Err(HostError::Refused {
                what: "publishing a region",
                errno: Some(12),
            });
        }
        let mr = MrHandle(self.next_mr.fetch_add(1, Ordering::SeqCst));
        let mut st = self.state.lock().expect("mock state");
        st.regions.insert(
            mr,
            Region {
                bytes: vec![0; usize::try_from(len).expect("a test-sized region")],
                refs: 1,
                live: true,
            },
        );
        st.log.push(tag(mr));
        Ok(mr)
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

    fn publish_window(
        &self,
        _name: &'static str,
        at: BarPlacement,
        bar_offset: u64,
        len: u64,
    ) -> Result<MrHandle, HostError> {
        if bar_offset.saturating_add(len) > at.len {
            return Err(HostError::Unsupported("a subregion outside its BAR"));
        }
        self.publish(len, HostCall::PublishWindow)
    }

    fn publish_rom_overlay(
        &self,
        _name: &'static str,
        at: BarPlacement,
        bar_offset: u64,
        len: u64,
    ) -> Result<MrHandle, HostError> {
        if bar_offset.saturating_add(len) > at.len {
            return Err(HostError::Unsupported("a subregion outside its BAR"));
        }
        self.publish(len, HostCall::PublishRomOverlay)
    }

    fn unpublish(&self, mr: MrHandle) {
        let mut st = self.state.lock().expect("mock state");
        if let Some(r) = st.regions.get_mut(&mr) {
            r.refs = r.refs.saturating_sub(1);
            r.live = false;
        }
        st.log.push(HostCall::Unpublish(mr));
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

/// A BAR placement a test can hand to [`crate::MachineConfig`] without repeating itself.
#[must_use]
pub fn bar(bar: BarId, base: u64, len: u64) -> BarPlacement {
    BarPlacement { bar, base, len }
}
