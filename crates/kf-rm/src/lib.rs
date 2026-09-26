//! ★★★★★ **v3 `kf-rm` — the emulated GSP's RM: RPC answers, object graph, controls.**
//!
//! The guest's stock driver talks to "GSP" through the message queue `kf-gsp` frames; this crate is
//! what answers. Copied from the old tree's `kayfabe-device` / `kayfabe-doorbell` / `kayfabe-rmrpc`
//! per `docs/design/V3_P2_PORT_MAP.md`, with the per-die `ChipProfile` replaced by
//! [`BoardFacts`] — facts about the board we PRESENT, derived from the store we reserved and the
//! host GPU we run on, never a captured chip row.
//!
//! ★ **P3's served chain** ([`served_policy`], [`ChainLogs`], [`ObjectLinks`]) answers from
//! [`BoardFacts`] (the board we present) and [`HostFacts`] (the host die's facts, each field with
//! its stated source in [`hostfacts::PROVENANCE`]). Its composition-level tests
//! (`tests/control_census.rs`, `tests/inert.rs`'s whole-chain test) run it over the old tree's
//! captured GA106 rows, kept ONLY as a test fixture (`tests/support/ga106.rs`).

pub mod abi;
pub mod authored;
pub mod barpde;
pub mod census;
pub mod chanlink;
pub mod faultbuffer;
pub mod guestsysinfo;
pub mod hostfacts;
pub mod hostquery;
pub mod inert;
pub mod inittables;
pub mod osevent;
pub mod rmgraph;
pub mod rmrpc;
pub mod rpc;
pub mod staticinfo;
pub mod sticky;
pub mod sweep;
pub mod sysmembar;
pub mod zbc;
pub mod unserviced;

pub use hostfacts::HostFacts;

use kf_abi::gspstaticinfo::FbRegion;
use kf_abi::pcibars::{PciBarRow, bus_bar};

/// ★ The board the guest sees — every value derived, none captured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardFacts {
    /// The framebuffer regions RM will manage (from the store's size: constraint 15).
    pub fb_regions: Vec<FbRegion>,
    /// The framebuffer length in bytes — the size the store actually holds.
    pub fb_length: u64,
    /// Where RM's BAR1 page directory lives in the framebuffer (a derivation from `fb_length`).
    pub bar1_pde_base: u64,
    /// ★ P4: where OUR BAR2 root page lives (`kf_chip::bar0::FbLayout::bar2_pde_base`).
    pub bar2_pde_base: u64,
    /// PCI vendor id presented to the guest (the host's own: NVIDIA's).
    pub pci_vendor_id: u16,
    /// PCI device id presented to the guest (the host's own, so the guest driver binds).
    pub pci_device_id: u16,
    /// PCI revision.
    pub pci_revision: u8,
    /// PCI subsystem vendor id.
    pub pci_subsystem_vendor_id: u16,
    /// PCI subsystem id.
    pub pci_subsystem_id: u16,
    /// ★ The PCI BAR table RM is told (`BUS_GET_PCI_BAR_INFO`), indexed by RM's logical
    /// [`bus_bar`] index: BAR0 from the host's sysfs length, BAR1/BAR2 the sized apertures we
    /// present (`THE_CONSTRAINTS.md` §w727), BAR3 absent. ⊘ One statement of each length — the
    /// register aperture's length is READ from here ([`BoardFacts::regs_aperture_len`]), never
    /// stated a second time, so the two can no longer disagree (the old `identity_for`
    /// cross-check existed only because they could).
    pub pci_bars: Vec<PciBarRow>,
}

impl BoardFacts {
    /// The BAR0 register aperture length — `pci_bars[REGS]`, or 0 if the table has no row.
    #[must_use]
    pub fn regs_aperture_len(&self) -> u64 {
        self.pci_bars.get(bus_bar::REGS).map_or(0, |b| b.size_bytes)
    }
}

/// ★★★ **The shared latches a served chain writes into** — named fields, so a latch cannot be
/// seated into the wrong link silently. `Clone` hands out the same latches (`Arc` inside).
///
/// ⊘ v3 carries three of the old six. `bar_pdes` (`bar2`), `gvas_pub` (`gvaspub`) and
/// `set_page_dir` (`setpagedir`) record the guest's page-directory statements; they are P4
/// (`V3_P2_PORT_MAP.md` §1: "defer to P4 — record the guest's statement as an attribute of the
/// VA-space object") and come back with the memory plane, not as logs beside it.
#[derive(Debug, Clone, Default)]
pub struct ChainLogs {
    /// The commands **nothing answered** ([`unserviced`]).
    pub unserviced: unserviced::UnservicedLog,
    /// The replayable-fault-buffer registrations ([`faultbuffer`]).
    pub fault_buffer: faultbuffer::FaultBufferLog,
    /// The os-event registrations a wakeup may be posted to, and the `FREE`s that retire them
    /// ([`osevent`]).
    pub os_events: osevent::OsEventLog,
}

/// ★★★ **The object-model seat in the served chain.**
///
/// | field | seat | job |
/// |---|---|---|
/// | [`Self::objects`] | between the answering links and the ledger | *answer* the object-declaring verbs and the controls it claims by id |
///
/// ⊘ The old second seat, `publications` (a `CommandObserver` at the FRONT carrying the guest's
/// page-directory publication into the object model — `kayfabe_rmrpc::PublicationObserver`), is
/// cut with the observer (`V3_P2_PORT_MAP.md` §1, rmrpc policy.rs: "Drop … PublicationObserver").
///
/// ⚠ The link installed here must be the COMPOSABLE form ([`rmrpc::ObjectPolicy`]), which
/// declines by default: [`rmrpc::GraphPolicy`] answers EVERY command, and installing it here
/// would silence [`unserviced::UnservicedLedger`] permanently.
///
/// `Default` is the seat empty — the register-only configuration, in which the object verbs
/// reach the ledger and are refused by name.
#[derive(Default)]
pub struct ObjectLinks {
    /// The object-model link.
    pub objects: Option<Box<dyn kf_gsp::CommandPolicy>>,
    /// ★ P4: the memory plane's inbox for the guest's address-space statements — fn 70
    /// ([`barpde::BarPdePolicy`]) and the page-directory controls ([`barpde::PageDirPolicy`]).
    /// `None` is the plane absent: fn 70 then reaches the object link (inert) as before, and the
    /// page-directory controls are answered without anything acting on them.
    pub memory: Option<MemoryLink>,
    /// ★ P5: the channel plane's seat — channel allocs, `GPFIFO_SCHEDULE`, the work-submit token
    /// and frees ([`chanlink::ChannelPolicy`]). `None` is the plane absent: those reach the object
    /// seat and the ledger as before (the controls then answer `NV_ERR_NOT_SUPPORTED`).
    pub channels: Option<chanlink::ChanSink>,
}

/// ★ P4: the memory plane's seat — where statements go, and the guest OS the page-directory
/// controls are decoded for (DECLARED by the device, never sniffed; see `ObjectPolicy::over`).
#[derive(Clone)]
pub struct MemoryLink {
    /// The plane's inbox.
    pub sink: barpde::MemSink,
    /// The guest OS.
    pub guest_os: kf_abi::GuestOs,
}

impl core::fmt::Debug for ObjectLinks {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ObjectLinks")
            .field("objects", &self.objects.is_some())
            .field("memory", &self.memory.is_some())
            .field("channels", &self.channels.is_some())
            .finish()
    }
}

/// ★★ **The one command-policy chain this device answers with** — built here so the reply-plane
/// differential (`kf-crec/tests/cap1b_differential.rs`) drives the same chain a guest does.
///
/// Layers, outermost first:
///
/// 1. [`census::ControlCensus`] — answers nothing; records the `rpc_result` the guest reads, so
///    absence means *never seen*.
/// 2. [`sticky::StickyAnswerGuard`] — every accepted `GSP_RM_CONTROL` reply crosses it, and it
///    zeroes the two reply fields that would let the guest cache our answer forever.
/// 3. [`served_chain`] — the links, in precedence order.
///
/// ⊘ v3 change from the old `served_policy(chip: &'static ChipProfile, …)`: the per-die facts
/// are a [`HostFacts`] value the composition root fills from the host, and the board is a
/// [`BoardFacts`] value. The name no longer comes from an env var (`KAYFABE_GPU_NAME`, w337); it
/// is [`HostFacts::gpu_name`].
#[must_use]
pub fn served_policy(
    board: std::sync::Arc<BoardFacts>,
    host: std::sync::Arc<HostFacts>,
    driver: kf_abi::versions::DriverAbiTable,
    logs: ChainLogs,
    census: census::ControlCensusLog,
    links: ObjectLinks,
) -> Box<dyn kf_gsp::CommandPolicy> {
    // ★ The notifier PROBE SET comes out of the census log: "the set the report states" and
    // "the set the event-plane arm consults" are ONE stored value.
    let probe_arm = census.snapshot().probe_arm;
    Box::new(census::ControlCensus::new(
        driver,
        census,
        sticky::StickyAnswerGuard::new(driver, served_chain(board, host, driver, logs, probe_arm, links)),
    ))
}

/// Where the device's guest driver version came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestDriverSource {
    /// The operator declared it (`guest-driver=`): a guest reporting another version is refused
    /// by name at fn 1 — the declaration is checked, never silently overridden.
    Declared,
    /// Defaulted to the host's version: at fn 1 the device RE-SELECTS to the guest's own
    /// version when the pre-fn-1 surface of the pair is identical
    /// (`kf_abi::versions::pre_fn1_surface_differs`), and refuses by name otherwise.
    Defaulted,
}

/// ★★ The served chain, re-selectable at fn 1 (`docs/design/V3_DRIVER_MATRIX.md` §4.2, owner
/// ruling 6, 2026-09-26).
///
/// A device must pick a table at realize, before the guest has said anything; the guest first
/// states its own driver version in `SET_GUEST_SYSTEM_INFO`. For a device whose version was
/// DEFAULTED, this wrapper rebuilds the whole chain for the guest's own version at that point —
/// provided everything the guest consumed before fn 1 is identical between the two (framing,
/// init args, VBIOS path, the pre-fn-1 RPC numbers and fn 1's body). Anything else is refused by
/// name by the chain's own fn-1 check, exactly as for a declared version.
///
/// ⊘ The rebuild discards only chain state, and there is none yet at fn 1: RM allocates its
/// first object after `GET_GSP_STATIC_INFO`, which follows fn 1. A later fn 1 (a GSP re-init)
/// with the same version is a no-op; with a different version it is a new driver load.
pub struct ReselectAtFn1 {
    provisional: kf_abi::versions::DriverAbiTable,
    current: kf_abi::DriverVersion,
    source: GuestDriverSource,
    build: Box<dyn Fn(kf_abi::versions::DriverAbiTable) -> Box<dyn kf_gsp::CommandPolicy> + Send>,
    inner: Box<dyn kf_gsp::CommandPolicy>,
}

impl ReselectAtFn1 {
    /// Build the provisional chain now; keep the recipe for a re-selection.
    pub fn new(
        provisional: kf_abi::versions::DriverAbiTable,
        source: GuestDriverSource,
        build: Box<dyn Fn(kf_abi::versions::DriverAbiTable) -> Box<dyn kf_gsp::CommandPolicy> + Send>,
    ) -> ReselectAtFn1 {
        let inner = build(provisional);
        ReselectAtFn1 { provisional, current: provisional.driver_version(), source, build, inner }
    }

    /// The version the chain currently answers as.
    #[must_use]
    pub fn current(&self) -> kf_abi::DriverVersion {
        self.current
    }

    /// Decide at fn 1. Returns what happened, for the log and for tests.
    pub fn on_fn1(&mut self, payload: &[u8]) -> Reselection {
        let Ok(said) = kf_abi::guestsysinfo::decode_guest_driver_version(payload) else {
            return Reselection::Kept;
        };
        let Some(reported) = kf_abi::DriverVersion::parse(said) else {
            return Reselection::Kept;
        };
        if reported == self.current {
            return Reselection::Kept;
        }
        if self.source == GuestDriverSource::Declared {
            return Reselection::RefusedDeclared { reported };
        }
        let table = match kf_abi::versions::table_for(reported) {
            Ok(t) => *t,
            Err(e) => return Reselection::RefusedUnserved { reported, why: e.to_string() },
        };
        if let Some(what) = kf_abi::versions::pre_fn1_surface_differs(&self.provisional, &table) {
            return Reselection::RefusedSurface { reported, what };
        }
        let from = self.current;
        self.inner = (self.build)(table);
        self.current = reported;
        Reselection::Reselected { from, to: reported }
    }
}

/// The outcome of [`ReselectAtFn1::on_fn1`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reselection {
    /// The guest is the version the chain already answers as (or said nothing parseable — the
    /// chain's own fn-1 check then refuses by name).
    Kept,
    /// Re-selected: the pre-fn-1 surface of the pair is identical.
    Reselected {
        /// The provisional (defaulted) version.
        from: kf_abi::DriverVersion,
        /// The guest's own version, now served.
        to: kf_abi::DriverVersion,
    },
    /// The version was declared; the declaration stands and the chain refuses the guest.
    RefusedDeclared {
        /// What the guest said it is.
        reported: kf_abi::DriverVersion,
    },
    /// The guest's version has no table (unmeasured, or no capability row).
    RefusedUnserved {
        /// What the guest said it is.
        reported: kf_abi::DriverVersion,
        /// Why there is no table.
        why: String,
    },
    /// The guest's version differs from the provisional one in a fact it already consumed.
    RefusedSurface {
        /// What the guest said it is.
        reported: kf_abi::DriverVersion,
        /// The first differing pre-fn-1 fact.
        what: &'static str,
    },
}

impl kf_gsp::CommandPolicy for ReselectAtFn1 {
    fn respond(&mut self, cmd: &kf_gsp::RpcCommand) -> Option<kf_gsp::Reply> {
        if cmd.function == kf_gsp::RpcFunction::SetGuestSystemInfo {
            match self.on_fn1(&cmd.payload) {
                Reselection::Kept => {}
                Reselection::Reselected { from, to } => eprintln!(
                    "kf-rm: guest driver RE-SELECTED at fn 1: {from} (defaulted) -> {to} (the guest's \
                     own; pre-fn-1 surface identical)"
                ),
                Reselection::RefusedDeclared { reported } => eprintln!(
                    "kf-rm: the guest says it is driver {reported}, the device was DECLARED {}; \
                     not re-selecting a declared version",
                    self.current
                ),
                Reselection::RefusedUnserved { reported, why } => eprintln!(
                    "kf-rm: the guest says it is driver {reported}; no table to re-select: {why}"
                ),
                Reselection::RefusedSurface { reported, what } => eprintln!(
                    "kf-rm: the guest says it is driver {reported}; cannot re-select from {}: \
                     {what} differs and the guest already consumed it",
                    self.current
                ),
            }
        }
        self.inner.respond(cmd)
    }

    fn holds_for_refresh(&self, cmd: &kf_gsp::RpcCommand) -> bool {
        self.inner.holds_for_refresh(cmd)
    }

    fn defers(&mut self, cmd: &kf_gsp::RpcCommand) -> Option<kf_gsp::Deferred> {
        self.inner.defers(cmd)
    }
}

/// The chain [`served_policy`] wraps — exposed so a test can drive the links **without** the
/// guard and see the difference the guard makes. Nothing in the port calls this.
///
/// ★ Order is precedence, and the answering links are disjoint by function code:
/// [`inittables::InitTablePolicy`] claims only `GSP_RM_CONTROL`, [`staticinfo::StaticInfoPolicy`]
/// only `GET_GSP_STATIC_INFO`, [`guestsysinfo::GuestSystemInfoPolicy`] only fn 1 and fn 64.
///
/// - **Front, observers** (cannot answer — `CommandObserver::observe` returns nothing): the
///   fault-buffer recorder (`InitTablePolicy` TERMINATES the chain for `0x20800a9b`, so a
///   recorder behind it would be blind) and the os-event registry (the object link terminates
///   `GSP_RM_ALLOC`/`FREE`).
/// - **Answering links**: init tables, static info, guest system info, inert.
/// - **The object seat** ([`ObjectLinks::objects`]): after the answering links (the specific
///   answer wins) and before the ledger (terminal-shaped; anything below it is unreachable).
/// - **The ledger** ([`unserviced::UnservicedLedger`]), last: answers nothing, writes down
///   exactly what every link above declined.
///
/// ★ P4 (w826): with [`ObjectLinks::memory`] seated, [`barpde::PageDirPolicy`] (front: observes
/// the publications, answers `SET_PAGE_DIRECTORY`) and [`barpde::BarPdePolicy`] (fn 70) carry the
/// guest's address-space statements to the memory plane and HOLD their replies for its
/// reconcile. Without it (the register-only configuration and most tests), fn 70 is inert and
/// `0x00801813` reaches the ledger and is refused by name, as before.
#[must_use]
pub fn served_chain(
    board: std::sync::Arc<BoardFacts>,
    host: std::sync::Arc<HostFacts>,
    driver: kf_abi::versions::DriverAbiTable,
    logs: ChainLogs,
    probe_arm: kf_abi::eventnotify::ProbeArmSet,
    links: ObjectLinks,
) -> Box<dyn kf_gsp::CommandPolicy> {
    // ★★★ EXHAUSTIVE: a latch added to `ChainLogs` and not seated below is a compile error.
    let ChainLogs { unserviced, fault_buffer, os_events } = logs;
    let ObjectLinks { objects, memory, channels } = links;
    let mut static_info = staticinfo::StaticInfoPolicy::new(board.clone(), driver)
        .with_engine_caps(authored::engine_caps(&host.engines));
    if let (Some(n), Some(sn)) = (host.gpu_name, host.gpu_short_name.or(host.gpu_name)) {
        static_info = static_info.with_name(n, sn);
    }
    let mut chain: Vec<Box<dyn kf_gsp::CommandPolicy>> = Vec::new();
    // ★ P5: the channel link is FIRST — ahead of the object seat (which terminates the alloc and
    // free it must see) and of the ledger (which would record its controls unserviced).
    if let Some(sink) = channels {
        let guest_os = memory.as_ref().map_or(kf_abi::GuestOs::Linux, |m| m.guest_os);
        chain.push(Box::new(chanlink::ChannelPolicy::new(driver, guest_os, sink)));
    }
    // ★ P4: the page-directory carrier is FIRST — ahead of `InitTablePolicy`, which terminates
    // the chain for the publication ids — and answers nothing; fn 70's link answers only fn 70.
    if let Some(MemoryLink { sink, guest_os }) = memory {
        chain.push(Box::new(barpde::PageDirPolicy::new(driver, guest_os, sink.clone())));
        // ★ v3-refusals: the guest's sysmembar, performed as the host's (`sysmembar.rs`).
        chain.push(Box::new(sysmembar::SysmembarPolicy::new(driver, sink.clone())));
        chain.push(Box::new(barpde::BarPdePolicy::new(sink)));
    }
    chain.extend::<[Box<dyn kf_gsp::CommandPolicy>; 7]>([
        // ★ v3-gfx: the per-VM ZBC table — claims only `0x9096xxxx` controls, answers them from
        // its own state and never forwards (`zbc.rs`).
        Box::new(zbc::ZbcPolicy::new(driver, host.zbc_table_sizes)),
        Box::new(kf_gsp::Observing(Box::new(faultbuffer::FaultBufferRecorder::new(
            driver,
            fault_buffer,
        )))),
        Box::new(kf_gsp::Observing(Box::new(osevent::OsEventRecorder::new(driver, os_events)))),
        Box::new(inittables::InitTablePolicy::with_probe_arm(board, host, driver, probe_arm)),
        Box::new(static_info),
        Box::new(guestsysinfo::GuestSystemInfoPolicy::new(driver)),
        Box::new(inert::InertPolicy::new()),
    ]);
    chain.extend(objects);
    chain.push(Box::new(unserviced::UnservicedLedger::new(driver, unserviced)));
    Box::new(kf_gsp::PolicyChain::new(chain))
}
