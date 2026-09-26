//! ★★★★★ **The kf3-gpu device, composed** — safe code over `kf-core`'s plane, `kf-gsp`'s FSM and
//! `kf-rm`'s answers, with the host session underneath.
//!
//! Threads (THE_ARCHITECTURE_v3.md §1): the guest's vCPUs call [`Device::bar0_write`] (lock-free:
//! a shadow store, `Plane::trap_write`, at most one eventfd write); ONE register drainer thread
//! applies the privileged ring IN ORDER to the GSP state machine, services deferred command
//! doorbells, and re-publishes the GSP registers into the BAR0 read shadow. BAR0 reads never exit:
//! they read the shadow QEMU maps as a ROM device.
//!
//! ⊘ Nothing here executes engine work on the CPU, and nothing forges a completion: GPU work runs
//! on the real engine through `kf-chan` (P5), and the plane's §8 rule governs completions.

use crate::raw_unsafe::RawRegion;
use kf_arch::gsp::{GspModel, GspReg};
use kf_chip::bar0::{Bar0Facts, BootReg, ConfigWord, boot_regs, config_words, fb_layout, pcie_link_caps, vbios_profile};
use kf_chip::Family;
use kf_core::{HostOps, HostSlice, Plane, Step, Translatable, Vmm};
use kf_gsp::{CommandPolicy, GspFsm, GuestRam, RamRefused};
use kf_linux_raw::{Notifier, PollTimeout, Poller, ReadyTokens};
use kf_trap::{Action, Class, WriteSemantics};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// `NV_PROM_DATA(i) = 0x300000 + i`, 1 MiB — where RM streams the VBIOS from.
pub const PROM_BASE: u64 = 0x0030_0000;
/// The PROM window's size.
pub const PROM_BYTES: u64 = 0x0010_0000;

/// Realize-time configuration (the QEMU device's properties).
#[derive(Debug, Clone)]
pub struct Config {
    /// Host GPU device minor (`/dev/nvidia<N>`).
    pub gpu_minor: u32,
    /// The framebuffer the guest gets, in MiB — reserved as ONE store object. ⊘ If the host refuses
    /// the reservation, the VM does not start (constraint 15: the size is what we could reserve).
    pub fb_mb: u64,
    /// The BAR1 (framebuffer window) aperture the C device decodes, in bytes.
    pub bar1_bytes: u64,
    /// The BAR2/BAR3 (instance window) aperture the C device decodes, in bytes.
    pub bar2_bytes: u64,
    /// The GUEST driver's version (its GSP wire layout), e.g. `580.159.04`. Defaults to the host's.
    pub guest_driver: Option<String>,
}

/// What the C device needs to present the PCI function.
#[derive(Debug, Clone, Copy)]
pub struct Identity {
    /// PCI identity (the host's own).
    pub pci: crate::hostfacts::HostPci,
    /// BAR0 bytes.
    pub bar0_bytes: u64,
}

/// A registered BAR0 shadow piece.
#[derive(Debug, Clone, Copy)]
struct Piece {
    base: u64,
    mem: RawRegion,
}

/// ★ w828: one `Disposition::Hole` page's register shadow (4 KiB of 32-bit words).
struct HoleShadow {
    base: u64,
    words: Box<[std::sync::atomic::AtomicU32]>,
}

impl HoleShadow {
    fn new(base: u64, boot: &[BootReg]) -> HoleShadow {
        let words: Box<[std::sync::atomic::AtomicU32]> =
            (0..kf_trap::memmap::PAGE / 4).map(|_| std::sync::atomic::AtomicU32::new(0)).collect();
        for r in boot.iter().filter(|r| (base..base + kf_trap::memmap::PAGE).contains(&r.off)) {
            words[((r.off - base) / 4) as usize].store(r.value, Ordering::Relaxed);
        }
        HoleShadow { base, words }
    }
    fn word(&self, off: u64) -> Option<&std::sync::atomic::AtomicU32> {
        off.checked_sub(self.base).and_then(|r| self.words.get((r / 4) as usize))
    }
}

/// The GSP side the drainer owns.
struct Gsp {
    fsm: GspFsm,
    model: std::sync::Arc<dyn GspModel>,
    policy: Box<dyn CommandPolicy>,
    /// The value last PUBLISHED per BAR0 offset — [`Device::publish`] stores only what changed.
    published: std::collections::HashMap<u64, u64>,
}

/// Counters, for the boot log — never a decision input.
#[derive(Debug, Default)]
pub struct Counters {
    /// Privileged BAR0 writes applied by the drainer.
    pub applied: AtomicU64,
    /// GSP command doorbells serviced.
    pub serviced: AtomicU64,
    /// Guest-RAM accesses refused (no block covers them).
    pub ram_refused: AtomicU64,
    /// Writes to a BAR0 offset with no shadow piece (P4 regions, or outside the aperture).
    pub unshadowed_writes: AtomicU64,
    /// ★ w828: BAR0 read exits served (`Hole` pages only).
    pub read_exits: AtomicU64,
    /// Every BAR0 write the vCPU delivered (before classification).
    pub trapped: AtomicU64,
    /// Writes the plane refused by name or answered with poison.
    pub refused: AtomicU64,
    /// The BAR0 offset of the most recent write.
    pub last_off: AtomicU64,
    /// ★ P5c: `RC_TRIGGERED` events posted to the guest.
    pub rc_posted: AtomicU64,
}

/// Interrupt counters — boot log only.
#[derive(Debug, Default)]
pub struct IrqCounts {
    /// Messages sent (eventfd writes).
    pub raised: AtomicU64,
    /// Latches that stayed pending (a leaf or top enable clear).
    pub held: AtomicU64,
    /// Vectors outside this family's leaves.
    pub out_of_range: AtomicU64,
    /// Writes to the tree.
    pub writes: AtomicU64,
}

/// The MSI-X vector count the C device exposes (its `msix-vectors` default).
pub const MSIX_VECTORS: usize = 32;

/// ★ The device.
pub struct Device {
    /// The host RM session (process-lifetime: the memory plane's views borrow it).
    pub rm: &'static kf_host::HostRm,
    /// The host's family.
    pub family: Family,
    /// What the C device presents.
    pub identity: Identity,
    /// The store (the guest's whole framebuffer, one object).
    pub store: kf_host::Reservation,
    /// ★ P4: the memory plane's shared half (windows, PRAMIN pool, invalidate port, inbox).
    pub mem: crate::mem::MemPlane,
    /// ★ P4: the VA manager, until its thread takes it ([`Device::va_loop`]).
    va: Mutex<Option<crate::mem::Manager>>,
    /// The VA manager's counters, copied out by its thread for the boot log.
    va_stats: Mutex<kf_mem::vasmgr::VaStats>,
    /// Realize time — the boot log's clock for the memory plane's lines.
    born: std::time::Instant,
    /// The plane (trap → workers / drainer).
    pub plane: &'static Plane<'static>,
    /// The host die's facts the served chain answers from (`kf_rm::hostfacts::PROVENANCE`).
    pub host_facts: std::sync::Arc<kf_rm::HostFacts>,
    /// The served chain's latches — the unserviced ledger, fault-buffer and os-event records —
    /// shared with the chain, for the boot log and later planes.
    pub chain_logs: kf_rm::ChainLogs,
    /// The served chain's control census (what the guest read back, per control).
    pub census: kf_rm::census::ControlCensusLog,
    pieces: OnceLock<Vec<Piece>>,
    staged: Mutex<Vec<Piece>>,
    /// ★ w828: the BAR0 `Hole` pages' own shadows — a hole has no memory behind it, so every
    /// register on the page that is NOT a read side effect is served from here by the read exit,
    /// with the read-back-what-was-written semantics a `B` page has.
    holes: Vec<HoleShadow>,
    /// ★ 2026-09-26: the FSP families' RM EMEM channel (`kf_trap::fspemem`) — its page is a hole,
    /// and its data port and cursor are served here, on the vCPU. `None` on the falcon families.
    fsp: Option<kf_trap::fspemem::FspEmem>,
    /// The NVDM type of the last command FSP acknowledged (status line).
    fsp_replies_type: std::sync::atomic::AtomicU32,
    ram: &'static crate::mem::RamMap,
    gsp: Mutex<Gsp>,
    /// ★ w828: the SAME register model the drainer's FSM answers from, shared lock-free with the
    /// vCPU for ONE question — [`GspModel::answer_on_store`] (what a register reads back the
    /// instant the guest's write lands, when the write alone decides it).
    store_model: std::sync::Arc<dyn GspModel>,
    boot: Vec<BootReg>,
    /// ★ 2026-09-26: config-space words read by config cycle (Hopper+; `kf_chip::bar0::config_words`).
    pub config_words: Vec<ConfigWord>,
    vbios: Vec<u8>,
    worker_efd: &'static Notifier,
    drainer_efd: &'static Notifier,
    /// ★ P5: the channel plane (the guest kernel's CE channels, Translated).
    pub chans: &'static crate::chan::ChanPlane,
    /// The workers' counters.
    pub worker_stats: kf_chan::worker::WorkerStats,
    /// ★ P5 §2.7: the CPU interrupt tree (atomic; applied synchronously on the vCPU).
    pub intr: kf_trap::cpuintr::CpuIntr,
    /// ★ P5 §2.7: one eventfd per MSI-X vector, registered by the C device as a KVM irqfd — a
    /// raise is ONE `write(2)` from any thread, no BQL (`THE_TRANSLATED_PLANE.md` §7).
    irq_lines: Vec<Notifier>,
    /// Interrupt counters for the boot log.
    pub irq_counts: IrqCounts,
    stop: AtomicBool,
    /// Boot-log counters.
    pub counters: Counters,
    /// ★ P5c: the VA timing at the previous heartbeat (the heartbeat prints the window).
    vat_prev: Mutex<kf_mem::vasmgr::VaTiming>,
    /// ★ w827: the attribution instruments (`KF3_PROF=1`; off = one relaxed load per hook).
    pub prof: Box<crate::prof::Prof>,
    /// ★ Hopper+: the C device's BAR1 overlay verb and its counters (`V3_BAR1_DOORBELL.md`).
    pub bar1_overlay: std::sync::Arc<crate::mem::Bar1Overlay>,
    /// The GSP command-queue head (the RPC doorbell) as a BAR0 offset — for [`crate::prof`].
    qhead_off: u64,
    /// Held replies' queue-head stamps, oldest first (drainer only) — for [`crate::prof`].
    held_stamps: Mutex<std::collections::VecDeque<u64>>,
}

impl Device {
    /// ★ Realize: host session → family → store → derived BAR0 → GSP FSM + answers → plane.
    ///
    /// # Errors
    /// Any refusal, by name — the VM must not start on a guessed device.
    pub fn realize(cfg: &Config) -> Result<Device, String> {
        let dev = kf_linux_raw::DevDir::open(c"/dev").map_err(|e| format!("open /dev: {e:?}"))?;
        let rm: &'static kf_host::HostRm = Box::leak(Box::new(
            kf_host::HostRm::open(&dev, kf_arch::ids::GpuId(cfg.gpu_minor), &kf_chip::choose_host_classes)
                .map_err(|e| e.to_string())?,
        ));
        let (architecture, implementation, revision) = rm.arch_info();
        let family = Family::from_arch(architecture, implementation).map_err(|e| format!("family: {e:?}"))?;
        // ★ P3: the host die's facts, each from the source `kf_rm::hostfacts::PROVENANCE` names —
        // asked before anything is reserved, so a refusal costs nothing. What the host cannot
        // state is authored as OUR device's (`kf_rm::authored`, cited per value). ⊘ A control the
        // host refuses, or a family an authored rule has no number for, refuses REALIZE, listing
        // every such field: never a default, never a GA106 row. (Hopper's PBDMA fault ids were such
        // a field until 2026-09-26; UVM's hwref states HOST0 = 64 — `V3_HW_BOUNDARY_INVENTORY.md`.)
        let host = std::sync::Arc::new(
            crate::rmfacts::host_facts(rm, family).map_err(|e| format!("host facts: {e}"))?,
        );
        // ★ ONE identity for the host GPU: the PCI address the frontend's `CARD_INFO` states
        // for our minor (`HostRm::card`). The RM device instance was resolved from it; the
        // sysfs facts and the CUDA device below are selected by it too — never by ordinal.
        let card = rm.card();
        let bdf = card.bdf();
        let sysfs = crate::hostfacts::sysfs_dir_for_minor(cfg.gpu_minor)?;
        if sysfs.file_name().and_then(|n| n.to_str()) != Some(bdf.as_str()) {
            return Err(format!(
                "host GPU identity disagrees: CARD_INFO puts minor {} at {bdf}, procfs at {} — \
                 refused by name, never a guess",
                cfg.gpu_minor,
                sysfs.display()
            ));
        }
        let pci = crate::hostfacts::read_host_pci(&sysfs)?;
        // ★ The per-host-card budget, summed over this process's kf3 devices on the same card —
        // before anything is reserved, so the refusal costs nothing (`crate::cardbudget`).
        let (store_neighbours, n_neighbours) = crate::cardbudget::store_held(&bdf);
        crate::cardbudget::admit(
            &bdf,
            pci.bar1_bytes,
            crate::cardbudget::Demand::of(cfg.bar1_bytes, cfg.bar2_bytes, cfg.fb_mb << 20),
        )?;

        let fb_length = cfg.fb_mb << 20;
        let layout = fb_layout(fb_length).ok_or(format!("a {} MiB store cannot hold the firmware carve-out", cfg.fb_mb))?;
        // ★ P4 realize order (`V3_P4_PORT_MAP.md` §2.0): the GPU walker's CUDA context comes up
        // BEFORE the reservation, then the store is exported into it. ⊘ No walker, no device:
        // BAR2 and every invalidate are walked by it, and there is no CPU fallback to fall to.
        let fmt = match family.mmu_format() {
            kf_chip::MmuFormat::Ver2 => kf_cuda::abi::kf_format_ver2(),
            kf_chip::MmuFormat::Ver3 => kf_cuda::abi::kf_format_ver3(),
        };
        // ★★★ v3-roperm: the permission policy — ONE value for the walker's diff key and the host
        // map. ATOMIC_DISABLE is carried only under `KF3_CARRY_ATOMIC_DISABLE=1`: enable it once
        // replayable-fault delivery exists (`kf_mem::apply::PermPolicy`).
        let perm = kf_mem::apply::PermPolicy { carry_atomic_disable: std::env::var("KF3_CARRY_ATOMIC_DISABLE").is_ok_and(|v| v == "1") };
        eprintln!(
            "kf3: permission policy: READ_ONLY+VOLATILE carried to the host map, PRIVILEGED leaves withheld from user twins, ATOMIC_DISABLE {} (key_perm={:#x})",
            if perm.carry_atomic_disable { "CARRIED (KF3_CARRY_ATOMIC_DISABLE=1)" } else { "not carried (KF3_CARRY_ATOMIC_DISABLE=1 enables it once fault delivery exists)" },
            perm.key_perm()
        );
        let mut kernel = kf_cuda::walk::WalkKernel::bring_up_on(
            kf_cuda::walk::WalkCfg { key_perm: perm.key_perm(), ..kf_cuda::walk::WalkCfg::default() },
            fmt,
            kf_cuda::walk::WalkDevice::PciBusId(&bdf),
        )
        .map_err(|e| format!("GPU walker bring-up on {bdf}: {e}"))?;
        let store = rm.reserve_gpga(fb_length).map_err(|e| {
            format!(
                "store of {} MiB refused: {e:?} (host card {bdf}: {n_neighbours} other kf3 device(s) \
                 of this process already hold {} MiB of store on it)",
                cfg.fb_mb,
                store_neighbours >> 20
            )
        })?;
        let export = rm.export_to_new_fd(store.handle).map_err(|e| format!("store export: {e:?}"))?;
        kernel
            .import_store(export.fd_number(), fb_length)
            .map_err(|e| format!("store import into the walker: {e}"))?;
        // The export node stays open for the process (CUDA holds the import).
        std::mem::forget(export);
        // ★ The identity line a multi-GPU run is graded on: minor → PCI → RM instance → CUDA.
        eprintln!(
            "kf3: host GPU minor={} bdf={bdf} gpuId={:#x} rm_device_instance={} cuda_device={:?} store={} MiB",
            cfg.gpu_minor,
            card.gpu_id,
            rm.device_instance(),
            kernel.device_name,
            cfg.fb_mb
        );
        // ★ Our two roots, zeroed on the GPU (the pages are ours: no CPU read, no guest table).
        let zero = vec![0u8; kf_chip::bar0::ROOT_PAGE_BYTES as usize];
        for root in [layout.bar1_pde_base, layout.bar2_pde_base] {
            kernel.write_store(root, &zero).map_err(|e| format!("zeroing our root @{root:#x}: {e}"))?;
        }
        let ram: &'static crate::mem::RamMap = Box::leak(Box::default());
        let inbox = std::sync::Arc::new(crate::mem::Inbox::new()?);

        let facts = Bar0Facts {
            architecture,
            implementation,
            revision,
            fb_mb: cfg.fb_mb,
            pcie_link_caps: pcie_link_caps(pci.max_gen, pci.max_width)
                .ok_or(format!("host link width x{} is not a PCIe width", pci.max_width))?,
        };
        let boot = boot_regs(family, &facts);
        let config_words = config_words(family, &facts);

        // ★ The GUEST driver axis (`docs/design/V3_DRIVER_MATRIX.md` §4): the version every guest-facing
        // layout is selected for. DECLARED by `guest-driver=`; unset, it defaults to the host's own
        // version (the thin guest's host mode boots the host's modules) — and either way the guest's
        // own fn-1 string is checked against it (`kf_rm::guestsysinfo`), so a wrong declaration is a
        // named refusal, never a guest answered with another release's layouts.
        let (guest, source) = match &cfg.guest_driver {
            Some(v) => (v.clone(), "guest-driver="),
            None => (rm.driver_version().to_string(), "defaulted to the host's"),
        };
        let version = kf_abi::DriverVersion::parse(&guest)
            .ok_or(format!("guest driver version {guest:?} does not parse (want major.minor[.patch])"))?;
        let table = kf_abi::versions::table_for(version).map_err(|e| format!("guest driver {version}: {e}"))?;
        let abi = kf_rm::abi::gsp_abi_for(version).map_err(|e| format!("GSP ABI for {version}: {e:?}"))?;
        eprintln!(
            "kf3: guest driver {version} ({source}); measured ABI: static-info {:?}, element {:?}, \
             init-args {:?}, rm-control params@{}, vgx {:?}",
            table.gsp_static_info_wire(),
            table.gsp_element_wire(),
            table.gsp_init_args_wire(),
            table.rm_control_wire().params_off,
            table.vgx_version().map(|v| (v.major, v.minor)),
        );

        let id = kf_chip::bar0::PciIdentity {
            vendor: pci.vendor,
            device: pci.device,
            class: [(pci.class & 0xFF) as u8, ((pci.class >> 8) & 0xFF) as u8, ((pci.class >> 16) & 0xFF) as u8],
        };
        // ★ Cosmetic: a host that did not answer BIOS_GET_INFO_V2 does not stop the VM — the ROM
        // declares the named neutral version and the guest's own ask is refused. Said by name.
        let vbios_version = host.vbios_version.unwrap_or_else(|| {
            eprintln!(
                "kf3: host BIOS_GET_INFO_V2 (0x20800810) not answered: synthetic ROM declares \
                 NEUTRAL_VBIOS_VERSION {:?}; the guest's BIOS_GET_INFO_V2 will be refused",
                kf_abi::vbios::NEUTRAL_VBIOS_VERSION
            );
            kf_abi::vbios::NEUTRAL_VBIOS_VERSION
        });
        let vbios = vbios_profile(family, id, vbios_version)
            .map_err(|e| format!("{e:?}"))
            .and_then(|p| kf_abi::vbios::build(&p, table.vbios_wire()).map_err(|e| format!("VBIOS: {e:?}")))?;

        let board = std::sync::Arc::new(kf_rm::BoardFacts {
            fb_regions: layout.regions.clone(),
            fb_length,
            bar1_pde_base: layout.bar1_pde_base,
            bar2_pde_base: layout.bar2_pde_base,
            pci_vendor_id: pci.vendor,
            pci_device_id: pci.device,
            pci_revision: pci.revision,
            pci_subsystem_vendor_id: pci.subsystem_vendor,
            pci_subsystem_id: pci.subsystem,
            // ★ The apertures THIS device decodes (never the host card's): BAR0 is the host's
            // register span, BAR1/BAR2 are the C device's properties, and there is no I/O BAR.
            pci_bars: vec![
                kf_abi::pcibars::PciBarRow { name: "registers", size_bytes: pci.bar0_bytes },
                kf_abi::pcibars::PciBarRow { name: "framebuffer-window", size_bytes: cfg.bar1_bytes },
                kf_abi::pcibars::PciBarRow { name: "instance-window", size_bytes: cfg.bar2_bytes },
                kf_abi::pcibars::PciBarRow { name: "io", size_bytes: 0 },
            ],
        });
        // The plane lives for the process (a device is realized once): leaked so the vCPU path
        // holds a plain `&'static`, never a lock or a refcount.
        let vmm: &'static Vmm = Box::leak(Box::new(Vmm::new()));
        let bar0_bytes = if pci.bar0_bytes != 0 { pci.bar0_bytes } else { 16 << 20 };
        let plane: &'static Plane<'static> = Box::leak(Box::new(Plane::for_family(
            vmm,
            1 << 12,
            0xFFF,
            family,
            u32::try_from(bar0_bytes).map_err(|_| "BAR0 larger than 4 GiB")?,
        )));
        // ★ P5: the channel plane — built BEFORE the served chain, which carries the guest's channel
        // statements to it. Its completion fd is opened here (host ioctls at realize, never a vCPU).
        // ⊘ The token table is indexed by the doorbell's VECTOR (11:0) — the guest's chid, which is
        // device-unique on the GSP families (one global CHID_MGR, `kf_arch::DoorbellTarget`).
        let worker_efd: &'static Notifier = Box::leak(Box::new(Notifier::create().map_err(|e| format!("eventfd: {e:?}"))?));
        // ★ P5b: the drainer's wake exists before the channel plane: an act that resolves a held
        // reply signals it.
        let drainer_efd: &'static Notifier = Box::leak(Box::new(Notifier::create().map_err(|e| format!("eventfd: {e:?}"))?));
        let mirrors = crate::mem::Mirrors::default();
        let chans: &'static crate::chan::ChanPlane = Box::leak(Box::new(crate::chan::ChanPlane::new(
            rm,
            plane,
            store.handle,
            ram,
            mirrors.clone(),
            inbox.clone(),
            worker_efd,
            drainer_efd,
            plane.tokens.len(),
            family,
            &host.intr_table,
            &host.engines,
        )?));
        chans.start()?;
        // ★ The served chain (census → sticky guard → init tables, static info, guest sys info,
        // inert, the object seat, the unserviced ledger). The object seat is the host-free graph
        // (`GraphObjects`): it answers ALLOC/FREE/DUP from the object model and refuses every
        // page-directory statement by name (`NotModelled`) until the memory plane (P4). Host twins
        // are P5.
        // ⚠ The guest OS is DECLARED, never sniffed (it is a `#define` in the guest driver's build,
        // invisible on the wire); this device answers as a Linux guest.
        let chain_logs = kf_rm::ChainLogs::default();
        let census = kf_rm::census::ControlCensusLog::new();
        // ★ The chain is built through a RECIPE (`V3_DRIVER_MATRIX.md` §4.2): every table-dependent
        // link is constructed from the table handed in, so `ReselectAtFn1` can rebuild it for the
        // guest's own version at fn 1 when the version was defaulted. The shared state (logs,
        // census, the memory inbox, the channel plane) is the SAME across a rebuild — only the
        // links that read layouts are new.
        let build = {
            let (board, host, chain_logs, census, inbox) =
                (board.clone(), host.clone(), chain_logs.clone(), census.clone(), inbox.clone());
            Box::new(move |t: kf_abi::versions::DriverAbiTable| {
                let objects = kf_rm::rmrpc::ObjectPolicy::over(
                    &t,
                    kf_abi::GuestOs::Linux,
                    Box::new(kf_rm::rmrpc::GraphObjects::new(family)),
                    kf_rm::rmrpc::ReasmLimits::default(),
                );
                kf_rm::served_policy(
                    board.clone(),
                    host.clone(),
                    t,
                    chain_logs.clone(),
                    census.clone(),
                    kf_rm::ObjectLinks {
                        objects: Some(Box::new(objects)),
                        // ★ P4: fn 70 and the page-directory statements go to the VA thread's inbox;
                        // their replies are held until it has settled them.
                        memory: Some(kf_rm::MemoryLink {
                            sink: {
                                let inbox = inbox.clone();
                                std::sync::Arc::new(move |st| inbox.push(st))
                            },
                            guest_os: kf_abi::GuestOs::Linux,
                        }),
                        // ★ P5: channel allocs, GPFIFO_SCHEDULE, the token and frees reach the plane,
                        // on the drainer; each answer IS the plane's act.
                        channels: Some(std::sync::Arc::new(move |st| chans.statement(st))),
                    },
                )
            })
        };
        let source = if cfg.guest_driver.is_some() {
            kf_rm::GuestDriverSource::Declared
        } else {
            kf_rm::GuestDriverSource::Defaulted
        };
        let policy: Box<dyn kf_gsp::CommandPolicy> = Box::new(kf_rm::ReselectAtFn1::new(*table, source, build));
        let model: std::sync::Arc<dyn GspModel> =
            std::sync::Arc::from(family.gsp_model(implementation, cfg.fb_mb).map_err(|e| format!("{e:?}"))?);
        let store_model = model.clone();
        crate::prof::init();
        let qhead_off = match model.at(GspReg::GspQueueHead(0)) {
            Some((0, off)) => off,
            _ => u64::MAX,
        };
        let gsp = Gsp { fsm: GspFsm::new(abi), model, policy, published: std::collections::HashMap::new() };


        // ★ P4: the windows, their scratch, the PRAMIN views (armed HERE, off every vCPU), the
        // invalidate port; and the VA manager with OUR BAR2 aperture as its first object.
        let (mem, bar1_ops, bar2_ops) = crate::mem::MemPlane::build(
            rm,
            family,
            store.handle,
            &layout,
            cfg.bar1_bytes,
            cfg.bar2_bytes,
            ram,
            inbox,
            mirrors,
        )?;
        let walker = kf_mem::vasmgr::GpuWalker { kernel, perm };
        // ★ 2026-09-26 (`V3_BAR1_DOORBELL.md` §7 T1's NEGATIVE CONTROL, `V3_FAMILY_PORT_BLACKWELL.md`):
        // `KF3_NEGCTL_NO_BAR1_DOORBELL=1` runs a Hopper+ family WITHOUT the BAR1 usermode-view
        // classification — the pre-`v3-bar1db` behaviour, where the view's leaf maps guest RAM at
        // `0x30000` and every BAR1 doorbell is a silent store. A detector that cannot fail under
        // this switch measures nothing. ⚠ Never for a real run; it is announced on every realize.
        let usermode_mmio = if std::env::var_os("KF3_NEGCTL_NO_BAR1_DOORBELL").is_some() {
            eprintln!("kf3: ⚠ NEGATIVE CONTROL KF3_NEGCTL_NO_BAR1_DOORBELL: BAR1 usermode views are NOT classified (family {family:?})");
            None
        } else {
            family.usermode_mmio()
        };
        // ★ P6b (b): coverage at the family's smallest GMMU page.
        let mut va: crate::mem::Manager = kf_mem::vasmgr::VaManager::new(
            walker,
            fb_length,
            Box::new(move |gpa, len| ram.file_range(gpa, len).map(|(_, off)| off)),
        )
        .with_page_grain(family.mmu_format().small_page_bytes())
        // ★ Hopper+: internal-MMIO usermode views are classified, never mapped as guest RAM
        // (`V3_BAR1_DOORBELL.md`). `None` on Turing … Ada: unchanged.
        .with_usermode_mmio(usermode_mmio);
        va.table.insert(
            crate::mem::K_BAR2,
            crate::mem::Target::Window(kf_mem::cpuwin::CpuWindow::new(bar2_ops, cfg.bar2_bytes)),
        );
        va.table
            .set_root(crate::mem::K_BAR2, layout.bar2_pde_base, kf_trap::PdbAperture::Vidmem)
            .map_err(|e| format!("our BAR2 root: {e:?}"))?;
        // ★ P5 (P4 row 6): BAR1 is walked from OUR BAR1 root like BAR2 — the guest writes its BAR1
        // PDEs straight into that page (no RPC) and invalidates it; the walk places store views.
        let bar1_overlay = std::sync::Arc::new(crate::mem::Bar1Overlay::default());
        let bar1_win = kf_mem::cpuwin::CpuWindow::new(bar1_ops, cfg.bar1_bytes);
        va.table.insert(
            crate::mem::K_BAR1,
            // ★ Hopper+: the guest places BAR1 usermode views where ITS allocator chooses; they
            // become write-trapped overlays there (`V3_BAR1_DOORBELL.md` §4).
            match usermode_mmio {
                Some(u) => crate::mem::Target::Bar1(crate::mem::Bar1Target::new(
                    bar1_win,
                    cfg.bar1_bytes,
                    u.vf_len,
                    bar1_overlay.clone(),
                )),
                None => crate::mem::Target::Window(bar1_win),
            },
        );
        va.table
            .set_root(crate::mem::K_BAR1, layout.bar1_pde_base, kf_trap::PdbAperture::Vidmem)
            .map_err(|e| format!("our BAR1 root: {e:?}"))?;
        eprintln!(
            "kf3: P4 memory plane: store {} MiB (imported into the walker), walker pools {} MiB of host GPU memory ({}), roots bar1={:#x} bar2={:#x}, PRAMIN one map+mmap per move, trigger @{:#x}",
            cfg.fb_mb,
            va.walker().kernel.pool_bytes() >> 20,
            va.walker().kernel.capacity_census(),
            layout.bar1_pde_base,
            layout.bar2_pde_base,
            mem.port.regs().trigger,
        );

        Ok(Device {
            rm,
            family,
            identity: Identity { pci, bar0_bytes },
            store,
            mem,
            va: Mutex::new(Some(va)),
            va_stats: Mutex::new(kf_mem::vasmgr::VaStats::default()),
            born: std::time::Instant::now(),
            plane,
            host_facts: host,
            chain_logs,
            census,
            pieces: OnceLock::new(),
            staged: Mutex::new(Vec::new()),
            ram,
            gsp: Mutex::new(gsp),
            store_model,
            holes: kf_trap::memmap::holes_for(family).iter().map(|(p, _)| HoleShadow::new(*p, &boot)).collect(),
            fsp: (family.boot_style() == kf_chip::BootStyle::Fsp).then(kf_trap::fspemem::FspEmem::new),
            fsp_replies_type: std::sync::atomic::AtomicU32::new(0),
            boot,
            config_words,
            vbios,
            worker_efd,
            chans,
            worker_stats: kf_chan::worker::WorkerStats::default(),
            intr: kf_trap::cpuintr::CpuIntr::new(family, kf_trap::memmap::VF_USERMODE_PAGE)
                .ok_or("CPU interrupt tree: the usermode base is below the PRIV delta")?,
            irq_lines: (0..MSIX_VECTORS)
                .map(|_| Notifier::create().map_err(|e| format!("irq eventfd: {e:?}")))
                .collect::<Result<Vec<_>, _>>()?,
            irq_counts: IrqCounts::default(),
            drainer_efd,
            stop: AtomicBool::new(false),
            counters: Counters::default(),
            vat_prev: Mutex::new(kf_mem::vasmgr::VaTiming::default()),
            prof: Box::default(),
            bar1_overlay,
            qhead_off,
            held_stamps: Mutex::new(std::collections::VecDeque::new()),
        })
    }

    /// The BAR0 memory map for the C device to build (family-scoped: shadow / plain RAM /
    /// passthrough / hole).
    #[must_use]
    pub fn memory_map(&self, bar1_bytes: u64, bar2_bytes: u64) -> kf_trap::memmap::MemoryMap {
        kf_trap::memmap::memory_map(
            self.family,
            self.plane.doorbell,
            self.identity.bar0_bytes,
            bar1_bytes,
            bar2_bytes,
        )
    }

    /// Register a BAR0 shadow piece QEMU allocated (a ROM device's RAM) at BAR0 offset `base`, and
    /// fill it: zeros, the boot registers, the PROM window's VBIOS bytes, the timer PLM.
    pub fn attach_shadow(&self, base: u64, mem: RawRegion) {
        let end = base + mem.len() as u64;
        for r in &self.boot {
            if (base..end).contains(&r.off) {
                mem.store_u32((r.off - base) as usize, r.value);
            }
        }
        if base < PROM_BASE + PROM_BYTES && PROM_BASE < end {
            let lo = base.max(PROM_BASE);
            let hi = end.min(PROM_BASE + PROM_BYTES);
            let img_lo = (lo - PROM_BASE) as usize;
            let img_hi = ((hi - PROM_BASE) as usize).min(self.vbios.len());
            if img_lo < img_hi {
                mem.write_from((lo - base) as usize, &self.vbios[img_lo..img_hi]);
            }
        }
        if let Some((off, v)) = self.plane.timer.plm_shadow() {
            let off = u64::from(off);
            if (base..end).contains(&off) {
                mem.store_u32((off - base) as usize, v);
            }
        }
        if let Ok(mut s) = self.staged.lock() {
            s.push(Piece { base, mem });
        }
    }

    /// Seal the shadow (after every piece is attached) and publish the GSP registers' initial
    /// values. ⊘ From here the vCPU path reads the piece list lock-free.
    pub fn seal_shadow(&self) {
        let mut v = self.staged.lock().map(|mut s| std::mem::take(&mut *s)).unwrap_or_default();
        v.sort_by_key(|p| p.base);
        let _ = self.pieces.set(v);
        if let Ok(mut g) = self.gsp.lock() {
            self.publish(&mut g);
        }
    }

    fn piece_for(&self, off: u64) -> Option<(Piece, usize)> {
        let pieces = self.pieces.get()?;
        let i = pieces.partition_point(|p| p.base <= off).checked_sub(1)?;
        let p = pieces[i];
        let rel = (off - p.base) as usize;
        (rel < p.mem.len()).then_some((p, rel))
    }

    fn shadow_store(&self, off: u64, val: u64, width: u8) {
        match self.piece_for(off) {
            Some((p, rel)) => {
                p.mem.store(rel, val, width);
            }
            None => {
                if !self.hole_store(off, val, width) {
                    self.counters.unshadowed_writes.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }

    /// ★ w828: a store into a `Hole` page's shadow (sub-word widths merge into the word).
    fn hole_store(&self, off: u64, val: u64, width: u8) -> bool {
        let Some(h) = self.holes.iter().find(|h| (h.base..h.base + kf_trap::memmap::PAGE).contains(&off)) else {
            return false;
        };
        let aligned = off & !3;
        let shift = ((off & 3) * 8) as u32;
        #[allow(clippy::cast_possible_truncation)]
        let (mask, v) = match width {
            1 => (0xffu32 << shift, ((val as u32) & 0xff) << shift),
            2 => (0xffffu32 << shift, ((val as u32) & 0xffff) << shift),
            _ => (u32::MAX, val as u32),
        };
        if let Some(w) = h.word(aligned) {
            let _ = w.fetch_update(Ordering::AcqRel, Ordering::Acquire, |old| Some((old & !mask) | v));
        }
        if width == 8
            && let Some(w) = h.word(aligned + 4)
        {
            #[allow(clippy::cast_possible_truncation)]
            w.store((val >> 32) as u32, Ordering::Release);
        }
        true
    }

    /// ★★★ w828 — **THE vCPU PATH for a BAR0 READ EXIT.** Reached only for a `Hole` page
    /// (`kf_trap::memmap::holes_for`): a register whose read has a side effect, and the other
    /// registers that share its page. Lock-free: atomics and at most one eventfd write.
    ///
    /// - a Hopper+ memop START register (`kf_trap::cacheop::TokenRead::Start`): the op is issued —
    ///   its request counter moves and the VA thread is woken to run the host verb — and the read
    ///   returns the op's token;
    /// - its `…_COMPLETED` register: `BUSY` until the VA thread has stored the host verb's return
    ///   (`cache_done`), then `IDLE` — ⊘ never on the read itself: a completion is a host event;
    /// - anything else on the page: the page shadow (what was last written, or the boot value).
    ///
    /// ⊘ The FSP/SEC2 EMEM ports that are the other holes are NOT served here beyond the shadow —
    /// their auto-increment needs the boot sequence's state, which lives under the GSP lock a vCPU
    /// may not take (named open item, unchanged by w828).
    pub fn bar0_read(&self, off: u64, width: u8) -> u64 {
        use kf_trap::cacheop::{TokenRead, completed_word, start_token, token_read};
        self.counters.read_exits.fetch_add(1, Ordering::Relaxed);
        if width == 4 {
            match token_read(self.family, off) {
                Some(TokenRead::Start(op)) => {
                    let n = self.mem.inbox.cache_req[op.index()].fetch_add(1, Ordering::AcqRel) + 1;
                    let _ = self.mem.inbox.wake.signal();
                    return u64::from(start_token(n));
                }
                Some(TokenRead::Completed(op)) => {
                    // `done` first: it only ever trails `req`, so a later `req` can only say BUSY.
                    let done = self.mem.inbox.cache_done[op.index()].load(Ordering::Acquire);
                    let req = self.mem.inbox.cache_req[op.index()].load(Ordering::Acquire);
                    return u64::from(completed_word(req, done));
                }
                None => {}
            }
        }
        // ★ The FSP EMEM channel: its data port moves on a READ (`kf_trap::fspemem`).
        if width == 4
            && let Some(f) = &self.fsp
            && let Some(v) = f.read(off)
        {
            return u64::from(v);
        }
        let Some(h) = self.holes.iter().find(|h| (h.base..h.base + kf_trap::memmap::PAGE).contains(&off)) else {
            return 0;
        };
        let aligned = off & !3;
        let lo = h.word(aligned).map_or(0, |w| w.load(Ordering::Acquire));
        let v = if width == 8 {
            u64::from(lo) | (u64::from(h.word(aligned + 4).map_or(0, |w| w.load(Ordering::Acquire))) << 32)
        } else {
            u64::from(lo >> ((off & 3) * 8))
        };
        match width {
            1 => v & 0xff,
            2 => v & 0xffff,
            4 => v & 0xffff_ffff,
            _ => v,
        }
    }

    /// ★ THE vCPU PATH for a BAR0 write. Lock-free: a shadow store (so a plain register reads
    /// back what was written, as hardware does), the plane's trap, and at most one eventfd write
    /// when a waiter is parked. ⊘ Never blocks, never services.
    pub fn bar0_write(&self, off: u64, val: u64, width: u8) {
        if !crate::prof::on() {
            return self.bar0_write_inner(off, val, width);
        }
        let t0 = crate::prof::now_ns();
        if off == self.qhead_off {
            self.prof.qhead_ns.store(t0, Ordering::Relaxed);
        }
        if off == crate::prof::MARK_OFF && val == crate::prof::MARK_VALUE {
            // The drainer prints the snapshot (never I/O on a vCPU): one eventfd write.
            self.prof.marks.fetch_add(1, Ordering::Relaxed);
            let _ = self.drainer_efd.signal();
        }
        self.bar0_write_inner(off, val, width);
        let ns = crate::prof::now_ns().saturating_sub(t0);
        self.prof.bar0.add(off, ns);
        self.prof.bar0_handler.add(ns);
    }

    /// A human name for a BAR0 offset, for the `PROF` census.
    #[must_use]
    pub fn reg_name(&self, off: u64) -> &'static str {
        use kf_trap::cpuintr::Reg;
        if off == self.qhead_off {
            return "GSP_QUEUE_HEAD0(rpc-doorbell)";
        }
        if (0x0011_0c00..0x0011_0c40).contains(&off) {
            return "GSP_QUEUE_HEAD/TAIL(n)";
        }
        if off == kf_trap::memmap::VF_USERMODE_PAGE + self.plane.doorbell.offset() {
            return "USERMODE_DOORBELL";
        }
        if off == self.mem.pramin_reg.offset {
            return "PRAMIN_WINDOW(BAR0_WINDOW)";
        }
        if self.mem.port.read(off).is_some() {
            return "MMU_INVALIDATE(pdb/upper/trigger)";
        }
        match self.intr.decode(off) {
            Some(Reg::Leaf(_)) => "CPU_INTR_LEAF(w1c)",
            Some(Reg::LeafEnSet(_)) => "CPU_INTR_LEAF_EN_SET",
            Some(Reg::LeafEnClear(_)) => "CPU_INTR_LEAF_EN_CLEAR",
            Some(Reg::Top) => "CPU_INTR_TOP",
            Some(Reg::TopEnSet) => "CPU_INTR_TOP_EN_SET",
            Some(Reg::TopEnClear) => "CPU_INTR_TOP_EN_CLEAR",
            Some(Reg::Trigger) => "CPU_INTR_LEAF_TRIGGER",
            None => "other",
        }
    }

    fn bar0_write_inner(&self, off: u64, val: u64, width: u8) {
        self.counters.trapped.fetch_add(1, Ordering::Relaxed);
        self.counters.last_off.store(off, Ordering::Relaxed);
        // ★ The BAR0 doorbell is live on EVERY family — on Hopper+ too: RM rings kernel channels
        // through its own BAR0 mapping (`kfifoRingChannelDoorBell_GH100` → GV100 →
        // `GPU_VREG_WR32`), and a client without `bBar1Mapping` gets the BAR0 view
        // (`V3_BAR1_DOORBELL.md` §1). ⊘ Before 2026-09-26 Hopper+ never recognised it here.
        let doorbell = off == kf_trap::memmap::VF_USERMODE_PAGE + self.plane.doorbell.offset();
        // ⊘ The WHOLE 64 KiB window guest userspace maps, not its first page (2026-09-26,
        // `kf_trap::memmap::VF_USERMODE_LEN`): pages 1..15 reached the privileged ring before.
        let in_usermode = kf_trap::memmap::in_usermode_window(off);
        // ★ P4: the three MMU_INVALIDATE registers. The port arms FIRST, then the shadow takes the
        // word the guest's spin will read (busy), then one wake — never the privileged ring: the
        // VA thread, not the drainer, completes it (§49.1).
        if !doorbell
            && !in_usermode
            && width == 4
            && let Some(v) = self.mem.invalidate_write(off, val as u32)
        {
            self.shadow_store(off, u64::from(v), 4);
            return;
        }
        // ★ P5 §2.7: the CPU interrupt tree — applied HERE, synchronously and lock-free (§5.5: the
        // ISR writes a mask then reads pending), its read-back published before any message.
        if !doorbell
            && !in_usermode
            && width == 4
            && let Some(r) = self.intr.decode(off)
        {
            self.irq_counts.writes.fetch_add(1, Ordering::Relaxed);
            let raise = self.intr.write(r, val as u32);
            self.intr.shadow(r, |o, v| self.shadow_store(o, u64::from(v), 4));
            self.deliver(raise);
            return;
        }
        // ★ w827: an L2 cache op (`kf_trap::cacheop`). The shadow takes the busy word the guest
        // will poll, the request counter moves AFTER it, and one wake — the VA thread performs the
        // host op and publishes idle. ⊘ Nothing blocks here. The write also goes on to the plane
        // below (a plain privileged register), exactly as before.
        // ★ 2026-09-26: the FSP EMEM channel moves its cursor NOW (RM reads it back right after
        // the burst) and posts FSP's reply at the queue HEAD write. The write ALSO goes on to the
        // plane below, unchanged: the GSP FSM reads the COT from its own copy of the window.
        if width == 4
            && let Some(f) = &self.fsp
            && let kf_trap::fspemem::FspWrite::Replied { nvdm_type } = f.write(off, val as u32)
        {
            self.fsp_replies_type.store(nvdm_type, Ordering::Relaxed);
        }
        let cache_op = if !doorbell && !in_usermode && width == 4 {
            kf_trap::cacheop::decode(self.family, off, val as u32)
        } else {
            None
        };
        let class = if doorbell {
            Class::Doorbell
        } else if in_usermode {
            Class::UserspaceMappable
        } else {
            // ★★★ w828: a register whose read-back the write alone decides answers NOW, not
            // after the drainer — `[measured 8dd2bbdf]` a guest that had spent its poll budget
            // read its own `DMATRFCMD` word twice, FWSEC-SB and Booter Unload never started, and
            // WPR2 outlived the adapter (`kf_arch::gsp::GspModel::answer_on_store`). Pure, no
            // lock: a decode and a match on the shared model.
            let shown = self
                .store_model
                .decode_reg(0, off)
                .and_then(|r| self.store_model.answer_on_store(r, val))
                .unwrap_or(val);
            self.shadow_store(off, shown, width);
            // ★ P4: the PRAMIN window base — re-pointed HERE, synchronously, from pre-armed views
            // (§53.1: the register is B, the window A). The word also goes on to the plane.
            if off == self.mem.pramin_reg.offset && width == 4 {
                self.mem.pramin_write(val as u32);
            }
            Class::Privileged { readable: true, semantics: WriteSemantics::Plain }
        };
        if let Some(op) = cache_op {
            self.mem.inbox.cache_req[op.index()].fetch_add(1, Ordering::Release);
            let _ = self.mem.inbox.wake.signal();
        }
        let off32 = u32::try_from(off).unwrap_or(u32::MAX);
        match self.plane.trap_write(class, 0, off32, val, width) {
            Action::RingHostInline { host_token } => {
                // ★ P5b: a Passthrough token — ONE fenced store into the host's doorbell, and two
                // relaxed counters for the per-token ledger. Nothing else on the vCPU.
                let reached = self.rm.doorbell(host_token).is_ok();
                if let Some(tok) = self.plane.token_index.of_doorbell(val as u32) {
                    self.chans.note_inline(tok, reached);
                }
            }
            Action::WakeWorker => {
                let _ = self.worker_efd.signal();
            }
            Action::WakeDrainer => {
                if crate::prof::on() {
                    self.prof.drainer_signal_ns.store(crate::prof::now_ns(), Ordering::Relaxed);
                }
                let _ = self.drainer_efd.signal();
            }
            Action::RefusedByName | Action::PoisonDevice => {
                self.counters.refused.fetch_add(1, Ordering::Relaxed);
            }
            Action::None => {}
        }
    }

    /// ★ The main loop applied BAR1 overlay change `seq` (`rc` 0 = done): post it and wake the VA
    /// thread, which releases the invalidate clear it was holding (ruling 2026-09-26 (5)).
    /// Main-loop thread; never blocks beyond one uncontended push and one eventfd write.
    pub fn bar1_overlay_done(&self, seq: u64, rc: i32) {
        self.bar1_overlay.post(seq, rc);
        let _ = self.mem.inbox.wake.signal();
    }

    /// ★★★ **A write into a Hopper+ BAR1 usermode view** (`V3_BAR1_DOORBELL.md` §3) — the vCPU
    /// path, from the C device's overlay; `vf_rel` is the offset inside the 64 KiB usermode page,
    /// decoded by the overlay itself (no lookup, no lock).
    ///
    /// The GMMU routes a write through such a PTE to the VF register at `vf_rel` — the SAME
    /// register the BAR0 usermode page exposes — so page 0 enters exactly as that BAR0 write:
    /// `+0x90` is the doorbell (token → our table, never the trap's identity), every other
    /// offset is `Class::UserspaceMappable` and does nothing. ⊘ Pages 1..15 of the view do
    /// nothing either: they are never routed onto BAR0's privileged path (a BAR1 view is an
    /// unprivileged client's).
    pub fn bar1_usermode_write(&self, vf_rel: u64, val: u64, width: u8) {
        if !self.plane.doorbell.follows_guest_bar1() || vf_rel >= kf_trap::memmap::PAGE {
            return;
        }
        if vf_rel == self.plane.doorbell.offset() {
            self.bar1_overlay.rings.fetch_add(1, Ordering::Relaxed);
        }
        self.bar0_write(kf_trap::memmap::VF_USERMODE_PAGE + vf_rel, val, width);
    }

    /// ★ Send what a tree write or latch asked for: ONE eventfd write on MSI-X vector 0 (RM reads
    /// TOP/LEAF to demultiplex — `intr_tu102.c:729-744`; the C artifact used one vector too).
    /// ⊘ Never a BQL-taking notify: the C device registered the eventfd as a KVM irqfd.
    fn deliver(&self, raise: kf_trap::cpuintr::Raise) {
        match raise {
            kf_trap::cpuintr::Raise::Message => {
                let _ = self.irq_lines[0].signal();
                self.irq_counts.raised.fetch_add(1, Ordering::Relaxed);
            }
            kf_trap::cpuintr::Raise::None => {}
            kf_trap::cpuintr::Raise::OutOfRange => {
                self.irq_counts.out_of_range.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// ★ Latch `vector` (a completion this device announces) and deliver — any thread.
    pub fn latch_and_deliver(&self, vector: u32) {
        let raise = self.intr.latch(vector);
        self.intr.shadow_all(|o, v| self.shadow_store(o, u64::from(v), 4));
        if raise == kf_trap::cpuintr::Raise::None {
            self.irq_counts.held.fetch_add(1, Ordering::Relaxed);
        }
        self.deliver(raise);
    }

    /// The eventfd of MSI-X vector `v`, for the C device to register as a KVM irqfd.
    #[must_use]
    pub fn irq_fd(&self, v: usize) -> Option<i32> {
        use std::os::fd::AsRawFd;
        self.irq_lines.get(v).map(|n| n.as_source_fd().as_raw_fd())
    }

    /// Register guest RAM: guest-physical `[gpa, gpa+len)` at `mem`, backed by `fd` at `fd_off`
    /// (`fd < 0`: the backend has none, and sysmem placements are then refused by name).
    pub fn ram_add(&self, gpa: u64, mem: RawRegion, fd: Option<crate::raw_unsafe::BackendFd>, fd_off: u64) {
        self.ram.add(crate::mem::RamBlock { gpa, mem, fd, fd_off });
    }

    /// ★ P4: the host address of `[base, base+len)` of a disposition-A region — PRAMIN (BAR0),
    /// BAR1 or BAR2 (`bar` 2 = PCI BAR3). `None` when no window covers it.
    #[must_use]
    pub fn window_address(&self, bar: u32, base: u64, len: u64) -> Option<kf_linux_raw::HostSpan> {
        let (win, rel) = match bar {
            0 => (self.mem.pramin_win, base.checked_sub(kf_trap::trappolicy::PRAMIN_BASE)?),
            1 => (self.mem.bar1_win, base),
            2 => (self.mem.bar2_win, base),
            _ => return None,
        };
        win.host_span(usize::try_from(rel).ok()?, usize::try_from(len).ok()?)
    }

    /// Unregister the guest RAM block at `gpa`.
    pub fn ram_del(&self, gpa: u64) {
        self.ram.del(gpa);
    }

    /// ★ P4: re-publish the invalidate trigger's word into the BAR0 read shadow — after the VA
    /// thread cleared it. ⊘ Loops until the shadow and the port agree: a vCPU re-arming between
    /// our read and our store has already stored "busy", and our stale "idle" must not stay.
    fn publish_trigger(&self) {
        let off = self.mem.port.regs().trigger;
        for _ in 0..8 {
            let Some(v) = self.mem.port.read(off) else { return };
            self.shadow_store(off, u64::from(v), 4);
            if self.mem.port.read(off) == Some(v) {
                return;
            }
        }
    }

    /// ★ w827 — **serve the guest's L2 cache ops** (`kf_trap::cacheop`) on the VA thread: for each
    /// op whose request counter moved, perform the host op (the authored, unprivileged
    /// `FB_FLUSH_GPU_CACHE` on OUR subdevice), then publish idle (`0`) into the shadow.
    ///
    /// ⊘ Correct against a fresh request: the counter is read BEFORE the host op, so the op covers
    /// every request up to it; the guest RM issues its next write to the same register only after
    /// it has read idle (it polls under its GPU lock, `kmemsysDoCacheOp_GM107`), so a request that
    /// arrives after our idle store re-stores busy and bumps the counter, and is served next pass.
    /// A refused host op is NAMED and the register still goes idle — the guest's alternative is a
    /// 4 s spin to `NV_ERR_TIMEOUT` with the same (unflushed) outcome.
    ///
    /// ★ w828: the same pass serves the Hopper+ READ-started ops (`kf_trap::cacheop::token_registers`):
    /// `cache_req` is then the tokens a vCPU's read exit issued, and `cache_done` — stored here, after
    /// the host verb returned — is what their `…_COMPLETED` register reports. The sysmembar
    /// (`FbFlush`) exists only there.
    fn serve_cache_ops(&self, done: &mut [u64; kf_trap::cacheop::CacheOp::COUNT]) {
        use kf_trap::cacheop::{CacheOp, registers};
        for op in CacheOp::ALL {
            let i = op.index();
            let want = self.mem.inbox.cache_req[i].load(Ordering::Acquire);
            if want == done[i] {
                continue;
            }
            let t = std::time::Instant::now();
            let r = match op {
                CacheOp::FlushDirty => self.rm.flush_gpu_cache(1, true, false),
                CacheOp::SysmemInvalidate => self.rm.flush_gpu_cache(1, false, true),
                CacheOp::PeermemInvalidate => self.rm.flush_gpu_cache(2, false, true),
                CacheOp::FbFlush => self.rm.fb_flush(),
            };
            self.mem.counters.cache_ops.fetch_add(1, Ordering::Relaxed);
            if let Err(e) = &r {
                eprintln!("kf3: L2 cache op {op:?} REFUSED by the host: {e:?} — the register is released anyway");
            } else if done[i] == 0 {
                eprintln!("kf3: L2 cache op {op:?} served by host FB_FLUSH_GPU_CACHE in {} us (first of this op)", t.elapsed().as_micros());
            }
            done[i] = want;
            // ★ w828: the completion edge of a read-started op — AFTER the host verb returned.
            self.mem.inbox.cache_done[i].store(want, Ordering::Release);
            for (o, _) in registers(self.family).iter().filter(|(_, x)| *x == op) {
                self.shadow_store(u64::from(*o), 0, 4);
            }
        }
    }

    /// ★★★ **The VA-manager thread** (`V3_P4_PORT_MAP.md` §2.1(c), Q6): the ONE thread that
    /// owns the GPU walker. It waits in `epoll` on two fds only — the inbox wake (statements from
    /// the drainer, invalidates from a vCPU) and the walker's completion eventfd — and never
    /// blocks on the GPU: a walk is submitted and collected on its fd.
    pub fn va_loop(&self) {
        let Some(mut m) = self.va.lock().ok().and_then(|mut g| g.take()) else { return };
        if let Err(e) = m.walker().kernel.make_current() {
            eprintln!("kf3: VA manager: walker context: {e} — the memory plane is DOWN");
            return;
        }
        const WAKE: u64 = 1;
        const WALK: u64 = 2;
        let Ok(poller) = Poller::create() else { return };
        if poller.watch(self.mem.inbox.wake.as_source_fd(), WAKE).is_err()
            || poller.watch(std::os::fd::AsFd::as_fd(m.walker().kernel.completion_fd()), WALK).is_err()
        {
            eprintln!("kf3: VA manager: epoll watch refused — the memory plane is DOWN");
            return;
        }
        let trigger = self.mem.port.trigger();
        let mut last_seq: Option<u64> = None;
        let mut taken = 0u64;
        let mut logged = 0u32;
        let mut refusals_seen = 0usize;
        let mut armed_seen: Option<u64> = None;
        // ★ Cold-box fix (`crate::mem::prewarm`): the first mirror's one-time host cost is paid
        // here, before the guest runs, as soon as QEMU has registered guest RAM.
        // ★ w827: `PREWARM_SPARES` spares, one per idle tick (the first also pins the guest-RAM
        // object) — never while a statement, a walk or an armed invalidate is waiting on us.
        let mut prewarmed = 0u64;
        let mut va_busy_from = crate::prof::now_ns();
        let mut cache_done = [0u64; kf_trap::cacheop::CacheOp::COUNT];
        while !self.stop.load(Ordering::Acquire) {
            if prewarmed < crate::mem::PREWARM_SPARES
                && (prewarmed == 0
                    || (!m.in_flight()
                        && m.pending() == 0
                        && self.mem.inbox.all_settled()
                        && self.mem.port.armed_request().is_none()))
                && let Some(line) = crate::mem::prewarm(&self.mem, self.rm, self.store.handle)
            {
                prewarmed += 1;
                eprintln!("kf3: mem t={:.3}s [{prewarmed}/{}] {line}", self.born.elapsed().as_secs_f64(), crate::mem::PREWARM_SPARES);
            }
            let mut ready = ReadyTokens::new();
            let prof = crate::prof::on();
            let tw = if prof { crate::prof::now_ns() } else { 0 };
            if prof {
                self.prof.vamgr.busy(tw.saturating_sub(va_busy_from));
            }
            let got = poller.wait(&mut ready, PollTimeout::Millis(50));
            if prof {
                va_busy_from = crate::prof::now_ns();
                self.prof.vamgr.waited(va_busy_from.saturating_sub(tw), matches!(got, Ok(0)));
            }
            let _ = self.mem.inbox.wake.drain();
            self.serve_cache_ops(&mut cache_done);
            for st in self.mem.inbox.take() {
                taken += 1;
                let line = crate::mem::apply_statement(&mut m, &self.mem, self.rm, self.store.handle, st, trigger);
                if kf_mem::maplog::on() {
                    eprintln!("kf3: maplog t={:.6} STATEMENT {line}", kf_mem::maplog::t());
                }
                if logged < 256 {
                    logged += 1;
                    eprintln!("kf3: mem t={:.3}s {line}", self.born.elapsed().as_secs_f64());
                }
            }
            if let Some(req) = self.mem.port.armed_request()
                && last_seq != Some(req.seq)
            {
                last_seq = Some(req.seq);
                m.on_invalidate(req, trigger);
            }
            // ★ P6: a Translated channel's `MEM_OP` split — walked with the invalidates, never a
            // wait on the worker that asked.
            for (ticket, pdb) in self.mem.inbox.take_split_requests() {
                m.on_split(pdb, ticket, trigger);
            }
            let r = m.on_walk_ready(trigger);
            // ★ Ruling 2026-09-26 (5): a BAR1 doorbell overlay the main loop has now made live (or
            // removed) releases the invalidate clear it was holding. Never a wait: with nothing
            // deferred this asks no target anything.
            let t = m.on_targets(trigger);
            if (!t.completed.is_empty() || !t.unreconciled.is_empty()) && logged < 256 {
                logged += 1;
                eprintln!(
                    "kf3: mem t={:.3}s deferred clears released completed={:?} unreconciled={:?}",
                    self.born.elapsed().as_secs_f64(),
                    t.completed,
                    t.unreconciled
                );
            }
            for (ticket, res) in m.take_splits() {
                if let Err(e) = &res {
                    eprintln!("kf3: mem t={:.3}s split ticket {ticket} REFUSED: {e}", self.born.elapsed().as_secs_f64());
                }
                if kf_mem::maplog::on() {
                    eprintln!("kf3: maplog t={:.6} SPLIT-DONE ticket={ticket} ok={}", kf_mem::maplog::t(), res.is_ok());
                }
                // Ring the channel's own token: its next pump resumes after the split.
                if let Some(tok) = self.mem.inbox.finish_split(ticket, res)
                    && self.plane.ring_internal(tok)
                {
                    let _ = self.worker_efd.signal();
                }
            }
            if r.collected && kf_mem::maplog::on() {
                // ★ `KF3_MAPLOG`: for every space this walk changed, the doorbells rung so far on each
                // passthrough twin in it — so a later RC can say whether work was submitted AFTER
                // the change (the counts moved) or only before it.
                for (k, a) in &r.applied {
                    if a.mapped + a.unmapped == 0 {
                        continue;
                    }
                    let space = self.mem.mirrors.lock().ok().and_then(|mm| mm.get(k).map(|mi| mi.space.space));
                    if let Some(sp) = space {
                        eprintln!(
                            "kf3: maplog t={:.6} APPLIED {k:?} +{} -{} host space {sp:#x} doorbells {}",
                            kf_mem::maplog::t(),
                            a.mapped,
                            a.unmapped,
                            self.chans.pt_doorbells(sp)
                        );
                    }
                }
            }
            if r.collected && logged < 256 {
                logged += 1;
                let applied: Vec<String> =
                    r.applied.iter().map(|(k, a)| format!("{:#x}:+{}-{}r{}", k.0, a.mapped, a.unmapped, a.refused)).collect();
                eprintln!(
                    "kf3: mem t={:.3}s walk reconciled [{}] completed={:?} unreconciled={:?}",
                    self.born.elapsed().as_secs_f64(),
                    applied.join(" "),
                    r.completed,
                    r.unreconciled
                );
            }
            for why in m.stats.refusals.iter().skip(refusals_seen) {
                eprintln!("kf3: mem t={:.3}s REFUSED {why}", self.born.elapsed().as_secs_f64());
            }
            // ★ An armed trigger we are NOT working on (its walk failed, so it stays armed by
            // design) is the guest spinning to its own timeout: say when it arms and when it goes.
            let armed = self.mem.port.armed_request().map(|r| r.seq);
            if armed != armed_seen {
                if m.stats.unreconciled > 0 {
                    eprintln!(
                        "kf3: mem t={:.3}s trigger {} (seq {:?}; unreconciled so far {})",
                        self.born.elapsed().as_secs_f64(),
                        if armed.is_some() { "armed" } else { "idle" },
                        armed.or(armed_seen),
                        m.stats.unreconciled
                    );
                }
                armed_seen = armed;
            }
            refusals_seen = m.stats.refusals.len();
            self.publish_trigger();
            // ★ Everything received so far is applied and nothing is walking: held replies go.
            if !m.in_flight() && m.pending() == 0 && self.mem.inbox.settle(taken) {
                let _ = self.drainer_efd.signal();
            }
            if let Ok(mut s) = self.va_stats.lock() {
                *s = m.stats.clone();
            }
        }
    }

    /// Re-publish every GSP register's current answer into the shadow.
    /// ★★★ Publish the FSM's registers into the shadow — DATA FIRST, then the registers the guest
    /// POLLS as a completion edge.
    ///
    /// ⊘ `[measured f67c9dde]` publishing in `GspReg::FIXED` order stored the falcon's HALTED bit
    /// before `WPR2_ADDR_HI`: the guest saw FWSEC halt, read WPR2 in the gap, and failed
    /// *"no initialized WPR2 found"*. When reads trapped, the FSM answered each read in order and
    /// this could not happen; with shadow reads, **publication order IS the guest-visible order**.
    ///
    /// ⊘⊘ **Only what CHANGED.** `[measured p4b1, 1 boot in 5]` re-storing every register after
    /// every applied write raced the vCPU: a guest write the vCPU had already put in the shadow,
    /// but which the drainer had not yet applied, was overwritten with the FSM's older answer —
    /// and the FWSEC handshake failed *"no initialized WPR2 found"* intermittently. So we store
    /// the ones the FSM moved, PLUS the register just applied (`apply_register` forgets its
    /// published value): its shadow holds the guest's write, and the FSM's answer replaces it.
    fn publish(&self, g: &mut Gsp) {
        const EDGES: [GspReg; 4] =
            [GspReg::GspFalconCpuctl, GspReg::GspRiscvCpuctl, GspReg::Sec2FalconCpuctl, GspReg::GspFalconIrqstat];
        let Gsp { fsm, model, published, .. } = g;
        let mut store = |reg: GspReg| {
            let Some((0, off)) = model.at(reg) else { return };
            if let Some(Ok(v)) = fsm.mmio_read_with(model.as_ref(), 0, off) {
                if published.insert(off, v) != Some(v) {
                    self.shadow_store(off, v, 4);
                }
            }
        };
        for reg in GspReg::FIXED.into_iter().chain((0..8u8).map(GspReg::GspQueueHead)) {
            if !EDGES.contains(&reg) {
                store(reg);
            }
        }
        // The data must be visible before any edge that announces it.
        std::sync::atomic::fence(Ordering::Release);
        for reg in EDGES {
            store(reg);
        }
    }

    /// ★ The register drainer's loop: apply the privileged ring in order, park on its own wake
    /// word when empty. Runs on ONE thread (an ordered ring drained by many is not ordered).
    pub fn drainer_loop(&self) {
        let poller = match Poller::create() {
            Ok(p) => p,
            Err(_) => return,
        };
        if poller.watch(self.drainer_efd.as_source_fd(), 0).is_err() {
            return;
        }
        let mut beat = (std::time::Instant::now(), String::new());
        let mut prof_beat = std::time::Instant::now();
        let mut marks_seen = 0u64;
        let mut busy_from = crate::prof::now_ns();
        // ★ w827: the last wait ended by TIMEOUT — did the pass after it find work?
        let mut after_timeout = false;
        while !self.stop.load(Ordering::Acquire) {
            // A heartbeat for the boot log, on the drainer (never a vCPU): printed only on change.
            if beat.0.elapsed() >= std::time::Duration::from_secs(2) {
                let now = self.status_line();
                if now != beat.1 {
                    eprintln!("{now}");
                }
                beat = (std::time::Instant::now(), now);
            }
            if crate::prof::on() {
                let m = self.prof.marks.load(Ordering::Relaxed);
                if m != marks_seen {
                    marks_seen = m;
                    eprintln!("kf3: PROF MARK {m} t={:.3}s", crate::prof::now_ns() as f64 / 1e9);
                    self.prof_print();
                    eprintln!("kf3: PROF MARK-END {m}");
                } else if prof_beat.elapsed() >= std::time::Duration::from_secs(5) {
                    prof_beat = std::time::Instant::now();
                    self.prof_print();
                }
            }
            let seen = self.plane.drainer_wake.seen();
            if self.plane.drainer_pass(self, 256) > 0 {
                if after_timeout {
                    self.prof.drainer.timeouts_with_work.fetch_add(1, Ordering::Relaxed);
                    after_timeout = false;
                }
                continue;
            }
            let released = self.release_settled();
            if released && after_timeout {
                self.prof.drainer.timeouts_with_work.fetch_add(1, Ordering::Relaxed);
            }
            after_timeout = false;
            self.deliver_rc();
            if !self.plane.drainer_wake.try_park(seen) {
                continue;
            }
            let mut ready = ReadyTokens::new();
            let prof = crate::prof::on();
            let tw = if prof { crate::prof::now_ns() } else { 0 };
            if prof {
                self.prof.drainer.busy(tw.saturating_sub(busy_from));
            }
            let got = poller.wait(&mut ready, PollTimeout::Millis(50));
            if prof {
                busy_from = crate::prof::now_ns();
                let timed_out = matches!(got, Ok(0));
                self.prof.drainer.waited(busy_from.saturating_sub(tw), timed_out);
                after_timeout = timed_out;
                let sig = self.prof.drainer_signal_ns.swap(0, Ordering::Relaxed);
                if !timed_out && sig != 0 && sig >= tw {
                    self.prof.drainer_wake.add(busy_from.saturating_sub(sig));
                }
            }
            self.plane.drainer_wake.unpark();
            let _ = self.drainer_efd.drain();
        }
    }

    /// One line of counters and the GSP phase — for the boot log, never a decision input.
    #[must_use]
    pub fn status_line(&self) -> String {
        let c = &self.counters;
        let o = Ordering::Relaxed;
        let (phase, refusals) = self
            .gsp
            .try_lock()
            .map(|g| (format!("{:?}", g.fsm.phase()), g.fsm.refusals().summary()))
            .unwrap_or_else(|_| ("busy".into(), "busy".into()));
        // The ledger's DISTINCT set names the control ids the RPC code alone hides.
        let unserviced: Vec<String> = self
            .chain_logs
            .unserviced
            .sample()
            .iter()
            .map(|c| c.cmd.map_or_else(|| format!("fn{}", c.function), |cmd| format!("{cmd:#010x}")))
            .collect();
        let mc = &self.mem.counters;
        let va = self.va_stats.lock().map(|v| v.clone()).unwrap_or_default();
        let (recv, settled) = self.mem.inbox.counts();
        // ★ P5c: the window since the last heartbeat (the cumulative mean hides growth).
        let tm = &match self.vat_prev.lock() {
            Ok(mut p) => {
                let d = va.timing.since(&p);
                *p = va.timing.clone();
                d
            }
            Err(_) => va.timing.clone(),
        };
        let avg = |sum: u64, n: u64| if n == 0 { 0 } else { sum / n / 1000 };
        let nm = self.mem.counters.mirrors.load(Ordering::Relaxed);
        let timing = format!(
            " mirrors={} reused={} prewarmed={} mirror_avg_us={} mirror_max_us={} vat[invals={} arrive->clear_avg_us={} max_us={} walks={} walk_avg_us={} gpu_avg_us={} plan_avg_us={} apply_avg_us={} leaves={} host_calls={}]",
            nm,
            self.mem.counters.mirrors_reused.load(Ordering::Relaxed),
            self.mem.counters.prewarmed.load(Ordering::Relaxed),
            avg(self.mem.counters.mirror_ns.load(Ordering::Relaxed), nm),
            self.mem.counters.mirror_ns_max.load(Ordering::Relaxed) / 1000,
            tm.invals,
            avg(tm.inval_ns, tm.invals),
            tm.inval_ns_max / 1000,
            tm.walks,
            avg(tm.walk_ns, tm.walks),
            if tm.walks == 0 { 0 } else { tm.gpu_us / tm.walks },
            avg(tm.plan_ns, tm.walks),
            avg(tm.apply_ns, tm.walks),
            tm.leaves_last,
            tm.host_calls,
        );
        let mem = format!(
            " mem[inval={} walks={}/{} cleared={} superseded={} named_missed={} unreconciled={} mapped={} unmapped={} clipped={:#x} held={} vmm_overlaps={} priv_withheld={} priv_withheld_bytes={:#x} priv_mirrored={} fn70={} roots={} root_moves={} stmts={recv}/{settled} refused={} pramin_repoints={} pramin_miss={} last_miss={:#x} pramin_worst_us={} (map {} mmap {}) pramin_maps={} pramin_mmaps={} inline_opens={} reaped={} cache_ops={} sysmembars={} root_unsets={}]",
            mc.invalidates.load(o),
            va.walks_reconciled,
            va.walks_submitted,
            va.cleared,
            va.superseded,
            va.named_missed,
            va.unreconciled,
            va.mapped,
            va.unmapped,
            va.clipped_bytes,
            va.held,
            va.vmm_overlaps,
            va.priv_withheld,
            va.priv_withheld_bytes,
            va.priv_mirrored,
            mc.bar_pdes.load(o),
            mc.roots.load(o),
            mc.root_moves.load(o),
            mc.refused.load(o) + va.refusals.len() as u64,
            self.mem.pramin.repoints.load(o),
            self.mem.pramin.missed.load(o),
            mc.pramin_last_miss.load(o),
            self.mem.pramin.worst_ns.load(o) / 1000,
            self.mem.pramin.worst_map_ns.load(o) / 1000,
            self.mem.pramin.worst_mmap_ns.load(o) / 1000,
            self.mem.pramin.maps.load(o),
            self.mem.pramin.mmaps.load(o),
            self.mem.pramin_trap.inline_opens.load(o),
            self.mem.pramin_trap.reaped.load(o),
            mc.cache_ops.load(o),
            mc.sysmembars.load(o),
            mc.root_unsets.load(o),
        ) + &timing
            // ★ Hopper+ only (Turing … Ada's line is unchanged): the BAR1 doorbell views.
            + &if self.plane.doorbell.follows_guest_bar1() {
                let b = &self.bar1_overlay;
                format!(
                    " bar1db[trapped={} unmirrored={} deferred_clears={} installed={} removed={} refused={} rings={}]",
                    va.usermode_trapped,
                    va.usermode_unmirrored,
                    va.deferred_clears,
                    b.installed.load(o),
                    b.removed.load(o),
                    b.refused.load(o),
                    b.rings.load(o)
                )
            } else {
                String::new()
            }
            // ★ FSP families only: FSP's replies over the RM EMEM channel (`kf_trap::fspemem`).
            + &self.fsp.as_ref().map_or_else(String::new, |f| {
                format!(" fsp[replies={} last_nvdm={:#x}]", f.replies(), self.fsp_replies_type.load(o))
            });
        let ws = &self.worker_stats;
        let toks: Vec<String> = self
            .chans
            .counts()
            .iter()
            .map(|t| {
                format!(
                    "{:#x}:fwd={},subs={},serves={},put={:?},gp_get={:?}{}",
                    t.token,
                    t.forwarded,
                    t.submissions,
                    t.serves,
                    t.last_put,
                    t.gp_get,
                    if t.dead.is_some() { ",DEAD" } else { "" }
                )
            })
            .collect();
        let nsi: Vec<String> = self
            .chans
            .engines
            .iter()
            .filter(|e| e.wakes.load(o) > 0)
            .map(|e| format!("{}:{}/{}raised", e.name, e.wakes.load(o), e.raised.load(o)))
            .collect();
        let (va, vr, vx) = self.rm.view_counts();
        let rc = format!(
            " views[armed={va} released={vr} refused={vx} held={}] rc[armed={} unarmed={} wakes={} seen={} posted={}]",
            va.saturating_sub(vr),
            self.chans.rc_armed.load(o),
            self.chans.rc_unarmed.load(o),
            self.chans.rc_wakes.load(o),
            self.chans.rc_seen.load(o),
            self.counters.rc_posted.load(o)
        );
        let chan = format!(
            " chan[births={} pt_births={} acts={}/{}refused worst_act_us={} nsi=[{}] served={} parks={} host_rings={} contended={} poisoned={} tokens=[{}]]",
            self.chans.births.load(o),
            self.chans.pt_births.load(o),
            self.chans.acts_run.load(o),
            self.chans.acts_refused.load(o),
            self.chans.act_worst_us.load(o),
            nsi.join(" "),
            ws.served.load(o),
            ws.parks.load(o),
            ws.host_rings.load(o),
            self.chans.contended.load(o),
            self.chans.poisoned.load(o),
            toks.join(" ")
        );
        let ic = &self.irq_counts;
        let irq = format!(
            " irq[writes={} raised={} held={} oor={}]",
            ic.writes.load(o),
            ic.raised.load(o),
            ic.held.load(o),
            ic.out_of_range.load(o)
        );
        format!(
            "kf3: family={:?} phase={phase} trapped={} applied={} refused={} serviced={} ram_refused={} unshadowed_writes={} read_exits={} last_off={:#x}{mem}{chan}{rc}{irq} unserviced=[{}] gsp_refusals[{refusals}]",
            self.family,
            c.trapped.load(o),
            c.applied.load(o),
            c.refused.load(o),
            c.serviced.load(o),
            c.ram_refused.load(o),
            c.unshadowed_writes.load(o),
            c.read_exits.load(o),
            c.last_off.load(o),
            unserviced.join(","),
        )
    }

    fn log_report(&self, r: &kf_gsp::ServiceReport) {
        for c in &r.commands {
            eprintln!("kf3: GSP rpc {:?} seq={}", c.function, c.sequence);
        }
        for u in &r.unserviced {
            eprintln!("kf3: GSP rpc UNSERVICED {u:?}");
        }
    }

    /// ★ P4, on the drainer: deliver held replies whose statements the VA thread has settled.
    fn release_settled(&self) -> bool {
        if !self.mem.inbox.all_settled() {
            return false;
        }
        let Ok(mut g) = self.gsp.lock() else { return false };
        if g.fsm.held_len() == 0 {
            return false;
        }
        let g = &mut *g;
        let mut ram = Ram(self);
        let released = g.fsm.release_held(&mut ram);
        log_fresh_refusals(&mut g.fsm);
        match released {
            Ok(n) if n > 0 => {
                self.publish(g);
                if crate::prof::on() {
                    self.prof.held_released_late.fetch_add(n as u64, Ordering::Relaxed);
                    self.prof_held_released(n);
                }
                true
            }
            Ok(_) => false,
            Err(e) => {
                eprintln!("kf3: held reply post REFUSED: {e:?}");
                false
            }
        }
    }

    /// ★ w827: `n` held replies were just posted — close their stamps (oldest first).
    fn prof_held_released(&self, n: usize) {
        let now = crate::prof::now_ns();
        if let Ok(mut q) = self.held_stamps.lock() {
            for _ in 0..n {
                match q.pop_front() {
                    Some(t) if t != 0 => self.prof.rpc_held.add(now.saturating_sub(t)),
                    _ => {}
                }
            }
        }
    }

    /// ★ w827: print the `PROF` lines (drainer heartbeat, `KF3_PROF=1` only).
    fn prof_print(&self) {
        for l in self.prof.lines(&|o| self.reg_name(o)) {
            eprintln!("{l}");
        }
        let ws = &self.worker_stats;
        eprintln!(
            "kf3: PROF workers busy_ms={:.1} wait_ms={:.1} max_busy_us={:.1} waits={} timeouts={} timeouts_with_work={} served={}",
            ws.busy_ns.load(Ordering::Relaxed) as f64 / 1e6,
            ws.wait_ns.load(Ordering::Relaxed) as f64 / 1e6,
            ws.max_busy_ns.load(Ordering::Relaxed) as f64 / 1000.0,
            ws.waits.load(Ordering::Relaxed),
            ws.timeouts.load(Ordering::Relaxed),
            ws.timeouts_with_work.load(Ordering::Relaxed),
            ws.served.load(Ordering::Relaxed),
        );
        let va = self.va_stats.lock().map(|v| v.timing.clone()).unwrap_or_default();
        let avg = |sum: u64, n: u64| sum.checked_div(n).unwrap_or(0) / 1000;
        eprintln!(
            "kf3: PROF va invals={} arrive_to_clear_sum_ms={} arrive_to_clear_avg_us={} max_us={} walks={} walk_sum_ms={} walk_avg_us={} gpu_avg_us={} plan_avg_us={} apply_avg_us={} host_calls={}",
            va.invals,
            va.inval_ns / 1_000_000,
            avg(va.inval_ns, va.invals),
            va.inval_ns_max / 1000,
            va.walks,
            va.walk_ns / 1_000_000,
            avg(va.walk_ns, va.walks),
            va.gpu_us.checked_div(va.walks).unwrap_or(0),
            avg(va.plan_ns, va.walks),
            avg(va.apply_ns, va.walks),
            va.host_calls,
        );
        eprintln!(
            "kf3: PROF acts run={} worst_us={} total_us={} irq_raised={} irq_writes={}",
            self.chans.acts_run.load(Ordering::Relaxed),
            self.chans.act_worst_us.load(Ordering::Relaxed),
            self.chans.act_total_us.load(Ordering::Relaxed),
            self.irq_counts.raised.load(Ordering::Relaxed),
            self.irq_counts.writes.load(Ordering::Relaxed),
        );
    }

    /// ★ P5c, on the drainer (the GSP queue's owner): post each host RC event to the guest as
    /// `RC_TRIGGERED` — what a real GSP sends when its RC path has run (`kernel_gsp.c:541-545`:
    /// *"RC error handling … is executed in GSP-RM. Client notifications … happen in CPU-RM"*) —
    /// then publish the registers and raise the GSP's stall vector. The notifier RECORD is already
    /// in the guest's memory: the host wrote it there (the twin's error context is the guest's
    /// record), as a GSP does before it sends this. ⊘ Nothing here writes guest memory but the
    /// message queue; nothing is forged: every event is a record the HOST wrote.
    fn deliver_rc(&self) {
        let evs = self.chans.take_rc();
        if evs.is_empty() {
            return;
        }
        let Ok(mut guard) = self.gsp.lock() else {
            self.chans.requeue_rc(evs);
            return;
        };
        let g = &mut *guard;
        let mut ram = Ram(self);
        let mut back = Vec::new();
        let mut posted = 0usize;
        for e in evs {
            let Some(engine) = kf_abi::rc::EngineRoute::declared(e.engine) else {
                eprintln!("kf3: RC chid {:#x}: engine type 0 — no route; NOT posted", e.chid);
                continue;
            };
            let ev = kf_abi::rc::RcTriggered {
                engine,
                chid: e.chid,
                except_type: e.except_type,
                // ⊘ CHANNEL, not TSG: our twin is its own host TSG, so only its record was
                // written (a guest TSG's other members keep running on their own twins).
                // ★ v3-promote (owner, TSG fault scope): this is CONSISTENT with the guest's view
                // because the guest's CPU-RM does nothing on RC_TRIGGERED but notify exactly the
                // scope we post (`_kgspRpcRCTriggered`, `kernel_gsp.c:548-676` →
                // `krcErrorSendEventNotificationsCtxDma_FWCLIENT`, `kernel_rc_notification.c:385-410`:
                // the TSG's channel list ONLY for `RC_NOTIFIER_SCOPE_TSG`). With CHANNEL scope the
                // guest considers that one channel dead and its siblings alive — which they are.
                // Group death the guest DECIDES (free of the channel/TSG/device/client, or TSG
                // `GPFIFO_SCHEDULE` disable) reaches every twin of the group (`ChanScope::freed_by`,
                // the schedule arm's `tsg == Some(object)`). ⊘ Posting TSG scope would require every
                // sibling's notifier record, which only the host may write, and no unprivileged
                // host verb RCs a sibling with a record — so a hardware-exact TSG-wide RC is an owner
                // call (V3_P5_PORT_MAP item 24), not something to forge here.
                scope: kf_abi::rc::RC_NOTIFIER_SCOPE_CHANNEL,
                // ⊘ Not read by the receiver (`_kgspRpcRCTriggered` uses engine, chid, gfid, the
                // exception, its level and scope); we hold no fault address, so none is invented.
                mmu_fault_addr: 0,
                mmu_fault_type: 0,
            };
            match g.fsm.post_rc_triggered(&mut ram, ev.encode()) {
                Ok(()) => {
                    posted += 1;
                    self.counters.rc_posted.fetch_add(1, Ordering::Relaxed);
                    eprintln!("kf3: RC_TRIGGERED posted: guest chid {:#x} engine {:#x} except_type {:#x} (host {:#x})", e.chid, e.engine, e.except_type, e.host_token);
                }
                Err(kf_gsp::GspFault::QueueFull { .. }) => back.push(e),
                Err(f) => eprintln!("kf3: RC_TRIGGERED for chid {:#x} REFUSED by the queue: {f:?}", e.chid),
            }
        }
        if posted > 0 {
            self.publish(g);
        }
        drop(guard);
        // ★ The interrupt goes AFTER the message and its registers are visible, and outside the
        // GSP lock (one eventfd write).
        if posted > 0 {
            self.latch_and_deliver(kf_rm::authored::GSP_STALL_VECTOR);
        }
        if !back.is_empty() {
            self.chans.requeue_rc(back);
        }
    }

    /// Stop the device's threads.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.drainer_efd.signal();
        let _ = self.worker_efd.signal();
        let _ = self.mem.inbox.wake.signal();
        self.chans.stop();
    }

    /// ★ P5: one WORKER thread — `kf_chan::worker::run` (the ONE worker loop) over this device's
    /// plane: a doorbell or a host completion wakes it; it claims a token and pumps the channel.
    /// ⊘ Never a vCPU; never waits on the GPU (a completion is an fd in its epoll set).
    pub fn worker_loop(&self) {
        self.worker_stats.prof.store(crate::prof::on(), Ordering::Relaxed);
        let Ok(poller) = Poller::create() else {
            eprintln!("kf3: worker: epoll refused — the channel plane is DOWN");
            return;
        };
        if poller.watch(self.worker_efd.as_source_fd(), kf_chan::worker::WORKER_EFD_TAG).is_err()
            || poller.watch(self.chans.completions.event_fd(), kf_chan::worker::COMPLETIONS_TAG).is_err()
        {
            eprintln!("kf3: worker: epoll watch refused — the channel plane is DOWN");
            return;
        }
        // ★ P5b §2.7: every host engine's non-stall event, in the same poller — its readiness is a
        // completion on that engine, announced to the guest on the engine's authored vector.
        for (i, e) in self.chans.engines.iter().enumerate() {
            if poller.watch(e.ev.as_fd(), kf_chan::worker::OTHER_TAG_BASE + i as u64).is_err() {
                eprintln!("kf3: worker: epoll watch of {} refused — its completions are not announced", e.name);
            }
        }
        // ★ P5c: the RC fd — a host twin's error context was written (its host RC path ran).
        let rc_tag = kf_chan::worker::OTHER_TAG_BASE + RC_TAG_OFFSET;
        if poller.watch(self.chans.rc_ev.as_fd(), rc_tag).is_err() {
            eprintln!("kf3: worker: epoll watch of the RC fd refused — guest channel faults will be SILENT");
        }
        let on_other = |tag: u64| {
            if tag == rc_tag {
                self.chans.rc_scan();
                return;
            }
            let Some(e) = tag.checked_sub(kf_chan::worker::OTHER_TAG_BASE).and_then(|i| self.chans.engines.get(i as usize)) else {
                return;
            };
            e.wakes.fetch_add(1, Ordering::Relaxed);
            // ⊘ Only an engine a guest twin runs on: the notifier is GPU-wide, and a wake with no
            // twin there is our own ring's, the walker's or another tenant's — not guest work.
            let raise = e.live.load(Ordering::Relaxed) > 0 && e.vector.is_some();
            if kf_mem::maplog::on() {
                eprintln!("kf3: maplog t={:.6} NSI host {} wake #{} raised={raise}", kf_mem::maplog::t(), e.name, e.wakes.load(Ordering::Relaxed));
            }
            if raise
                && let Some(v) = e.vector
            {
                e.raised.fetch_add(1, Ordering::Relaxed);
                self.latch_and_deliver(v);
            }
        };
        kf_chan::worker::run(self.plane, self, &poller, self.worker_efd, &self.chans.completions, &self.worker_stats, &self.stop, &on_other);
    }
}

/// ★ P5c: the RC fd's poller tag, past every engine's (`OTHER_TAG_BASE + engine index`).
const RC_TAG_OFFSET: u64 = 1 << 16;

/// Guest RAM as the GSP FSM reads it — through the blocks QEMU registered.
struct Ram<'a>(&'a Device);

impl GuestRam for Ram<'_> {
    fn read(&mut self, gpa: u64, buf: &mut [u8]) -> Result<(), RamRefused> {
        if let Some(b) = self.0.ram.block_for(gpa, buf.len() as u64)
            && b.mem.read_into((gpa - b.gpa) as usize, buf)
        {
            return Ok(());
        }
        self.0.counters.ram_refused.fetch_add(1, Ordering::Relaxed);
        Err(RamRefused { gpa, len: buf.len(), why: "no guest-RAM block QEMU registered covers this range" })
    }
    fn write(&mut self, gpa: u64, bytes: &[u8]) -> Result<(), RamRefused> {
        if let Some(b) = self.0.ram.block_for(gpa, bytes.len() as u64)
            && b.mem.write_from((gpa - b.gpa) as usize, bytes)
        {
            return Ok(());
        }
        self.0.counters.ram_refused.fetch_add(1, Ordering::Relaxed);
        Err(RamRefused { gpa, len: bytes.len(), why: "no guest-RAM block QEMU registered covers this range" })
    }
}

/// ★ The host side of the plane, as the device serves it. P2: the register drainer's writes go to
/// the GSP FSM; channel verbs arrive with P5 (`kf-chan`).
impl HostOps for Device {
    fn ring_host(&self, host_token: u32) {
        let _ = self.rm.doorbell(host_token);
    }
    fn run_translated(&self, host_token: u32, _up_to_seq: u64) -> bool {
        self.chans.serve(host_token)
    }
    fn run_emulated(&self, _host_token: u32, _up_to_seq: u64) {}
    fn apply_register(&self, bar: u8, offset: u32, value: u64, _width: u8) {
        let prof = crate::prof::on();
        let t_apply = if prof { crate::prof::now_ns() } else { 0 };
        let is_qhead = prof && bar == 0 && u64::from(offset) == self.qhead_off;
        let t_trap = if is_qhead { self.prof.qhead_ns.load(Ordering::Relaxed) } else { 0 };
        let Ok(mut g) = self.gsp.lock() else { return };
        let g = &mut *g;
        let held_before = g.fsm.held_len();
        let mut n_cmds = 0usize;
        let n = self.counters.applied.fetch_add(1, Ordering::Relaxed);
        // The first writes ARE the boot sequence; logged on the drainer (never a vCPU).
        if n < 512 {
            eprintln!("kf3: w#{n} bar{bar} @{offset:#08x} = {value:#x} phase={:?}", g.fsm.phase());
        }
        // ★ The register just applied is re-published unconditionally: the vCPU already stored
        // the GUEST's value there, and the FSM's answer may equal what was last published
        // (`[measured edfff3a9, 3/3 boots]` DMATRFCMD read back 'busy' forever: FWSEC timed out).
        if bar == 0 {
            g.published.remove(&u64::from(offset));
        }
        let mut ram = Ram(self);
        let before = g.fsm.phase();
        match g.fsm.mmio_write_with(&mut ram, g.model.as_ref(), g.policy.as_mut(), bar, u64::from(offset), value) {
            Ok(r) => {
                n_cmds += r.commands.len();
                self.log_report(&r);
            }
            Err(e) => eprintln!("kf3: GSP write @{offset:#x}={value:#x} REFUSED: {e:?}"),
        }
        while g.fsm.pending_command_doorbells() > 0 {
            self.counters.serviced.fetch_add(1, Ordering::Relaxed);
            match g.fsm.service_one_deferred_command(&mut ram, g.policy.as_mut()) {
                Ok(r) => {
                    n_cmds += r.commands.len();
                    self.log_report(&r);
                }
                Err(e) => {
                    eprintln!("kf3: GSP command service REFUSED: {e:?}");
                    break;
                }
            }
        }
        let after = g.fsm.phase();
        if after != before {
            // ★ The P2 gate's observable: the boot phase, logged by the drainer (never a vCPU).
            eprintln!("kf3: GSP phase {before:?} -> {after:?}");
        }
        // ★ P4: a held reply (fn 70, a page-directory statement) goes only once the VA thread has
        // settled every statement received — its root written and walked (§49.1 for the RPC
        // path). Otherwise `release_settled` delivers it when the VA thread wakes this thread.
        let held_mid = g.fsm.held_len();
        let released = if self.mem.inbox.all_settled() { g.fsm.release_held(&mut ram).unwrap_or(0) } else { 0 };
        log_fresh_refusals(&mut g.fsm);
        let t_pub = if prof { crate::prof::now_ns() } else { 0 };
        self.publish(g);
        if prof {
            let done = crate::prof::now_ns();
            self.prof.publish.add(done.saturating_sub(t_pub));
            // Newly held replies take this doorbell's trap stamp; released ones close theirs.
            if held_mid > held_before
                && let Ok(mut q) = self.held_stamps.lock()
            {
                for _ in held_before..held_mid {
                    q.push_back(t_trap);
                }
            }
            if released > 0 {
                self.prof_held_released(released);
            }
            if is_qhead {
                self.prof.rpc_doorbells.fetch_add(1, Ordering::Relaxed);
                self.prof.rpc_commands.fetch_add(n_cmds as u64, Ordering::Relaxed);
                self.prof.rpc_service.add(done.saturating_sub(t_apply));
                if t_trap != 0 {
                    self.prof.rpc_trap_to_apply.add(t_apply.saturating_sub(t_trap));
                    if held_mid <= held_before {
                        self.prof.rpc_immediate.add(done.saturating_sub(t_trap));
                    }
                }
            } else {
                self.prof.other_applies.add(done.saturating_sub(t_apply));
            }
        }
    }
    fn operands_translatable(&self, host_token: u32, _up_to_seq: u64) -> Translatable {
        // A channel whose pump REFUSED is dead: the plane then poisons (kernel), never faults.
        if self.chans.alive(host_token) { Translatable::Yes } else { Translatable::No }
    }
    fn forge_completion(&self, host_token: u32) {
        // ⊘ §8: never reached for a Translated route (`Completion::for_route`); counted if it is.
        eprintln!("kf3: FORGE requested for host token {host_token:#x} — refused (no Emulated channel exists)");
    }
    fn refuse_and_poison(&self, host_token: u32) {
        // §7: a kernel channel is NEVER faulted — its work is refused and the device counted
        // poisoned, visibly (a translation miss on the scrubber is not the guest's fault to take).
        if self.chans.poisoned.fetch_add(1, Ordering::Relaxed) < 8 {
            eprintln!("kf3: kernel channel host {host_token:#x} REFUSED-AND-POISONED (§7)");
        }
        self.counters.refused.fetch_add(1, Ordering::Relaxed);
    }
    fn fault_channel(&self, _host_token: u32) {}
    fn map_guest_slice(&self, _slice: HostSlice) {}
    fn teardown_step(&self, _step: Step) {}
}

/// ★★★ v3-refusals: one line per refusal row the FSM posted for the FIRST time — a non-OK status
/// the guest read (`kf_gsp::refusal`). The heartbeat's `gsp_refusals[...]` carries the counts.
/// ⊘ Printed on the drainer (never a vCPU), once per distinct `(function, detail, status)`, so a
/// hot refusal cannot flood the log; bare metal returns non-OK once in 613 RM records, so every
/// line here is a divergence until proven harmless.
fn log_fresh_refusals(fsm: &mut kf_gsp::GspFsm) {
    for r in fsm.take_fresh_refusals() {
        eprintln!(
            "kf3: GSP REFUSED {} ({}) first_seq={} — the guest read a non-OK status",
            r.key(),
            kf_rm::rpc::name_of(r.function).unwrap_or("?"),
            r.first_sequence
        );
    }
}
