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
use kf_chip::bar0::{Bar0Facts, BootReg, boot_regs, fb_layout, pcie_link_caps, vbios_profile};
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

/// The GSP side the drainer owns.
struct Gsp {
    fsm: GspFsm,
    model: Box<dyn GspModel>,
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
    ram: &'static crate::mem::RamMap,
    gsp: Mutex<Gsp>,
    boot: Vec<BootReg>,
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
}

fn parse_version(s: &str) -> Option<kf_abi::DriverVersion> {
    let mut it = s.trim().split('.').map(|x| x.parse::<u16>().ok());
    Some(kf_abi::DriverVersion { major: it.next()??, minor: it.next()??, patch: it.next().flatten().unwrap_or(0) })
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
        // host refuses, or a family an authored rule has no number for (Hopper's PBDMA fault ids),
        // refuses REALIZE, listing every such field: never a default, never a GA106 row.
        let host = std::sync::Arc::new(
            crate::rmfacts::host_facts(rm, family).map_err(|e| format!("host facts: {e}"))?,
        );
        let sysfs = crate::hostfacts::sysfs_dir_for_minor(cfg.gpu_minor)?;
        let pci = crate::hostfacts::read_host_pci(&sysfs)?;

        let fb_length = cfg.fb_mb << 20;
        let layout = fb_layout(fb_length).ok_or(format!("a {} MiB store cannot hold the firmware carve-out", cfg.fb_mb))?;
        // ★ P4 realize order (`V3_P4_PORT_MAP.md` §2.0): the GPU walker's CUDA context comes up
        // BEFORE the reservation, then the store is exported into it. ⊘ No walker, no device:
        // BAR2 and every invalidate are walked by it, and there is no CPU fallback to fall to.
        let fmt = match family.mmu_format() {
            kf_chip::MmuFormat::Ver2 => kf_cuda::abi::kf_format_ver2(),
            kf_chip::MmuFormat::Ver3 => kf_cuda::abi::kf_format_ver3(),
        };
        let kernel = kf_cuda::walk::WalkKernel::bring_up(kf_cuda::walk::WalkCfg::default(), fmt)
            .map_err(|e| format!("GPU walker bring-up: {e}"))?;
        let store = rm.reserve_gpga(fb_length).map_err(|e| format!("store of {} MiB refused: {e:?}", cfg.fb_mb))?;
        let export = rm.export_to_new_fd(store.handle).map_err(|e| format!("store export: {e:?}"))?;
        let store_ptr = kernel
            .import_store(export.fd_number(), fb_length)
            .map_err(|e| format!("store import into the walker: {e}"))?;
        // The export node stays open for the process (CUDA holds the import).
        std::mem::forget(export);
        // ★ Our two roots, zeroed on the GPU (the pages are ours: no CPU read, no guest table).
        let zero = vec![0u8; kf_chip::bar0::ROOT_PAGE_BYTES as usize];
        for root in [layout.bar1_pde_base, layout.bar2_pde_base] {
            kernel.write_at(store_ptr + root, &zero).map_err(|e| format!("zeroing our root @{root:#x}: {e}"))?;
        }
        let ram: &'static crate::mem::RamMap = Box::leak(Box::default());
        let inbox = std::sync::Arc::new(crate::mem::Inbox::new()?);

        let facts = Bar0Facts {
            architecture,
            implementation,
            revision,
            fb_mb: cfg.fb_mb,
            pcie_link_caps: pcie_link_caps(pci.max_gen),
        };
        let boot = boot_regs(family, &facts);

        let guest = match &cfg.guest_driver {
            Some(v) => v.clone(),
            None => rm.driver_version().to_string(),
        };
        let version = parse_version(&guest).ok_or(format!("guest driver version {guest:?} does not parse"))?;
        let table = kf_abi::versions::table_for(version).map_err(|e| format!("guest driver {guest}: {e:?}"))?;
        let abi = kf_rm::abi::gsp_abi_for(version).map_err(|e| format!("GSP ABI for {guest}: {e:?}"))?;

        let id = kf_chip::bar0::PciIdentity {
            vendor: pci.vendor,
            device: pci.device,
            class: [(pci.class & 0xFF) as u8, ((pci.class >> 8) & 0xFF) as u8, ((pci.class >> 16) & 0xFF) as u8],
        };
        let vbios = vbios_profile(family, id)
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
        )?));
        chans.start()?;
        // ★ The served chain (census → sticky guard → init tables, static info, guest sys info,
        // inert, the object seat, the unserviced ledger). The object seat is the host-free graph
        // (`GraphObjects`): it answers ALLOC/FREE/DUP from the object model and refuses every
        // page-directory statement by name (`NotModelled`) until the memory plane (P4). Host twins
        // are P5.
        // ⚠ The guest OS is DECLARED, never sniffed (it is a `#define` in the guest driver's build,
        // invisible on the wire); this device answers as a Linux guest.
        let objects = kf_rm::rmrpc::ObjectPolicy::over(
            table,
            kf_abi::GuestOs::Linux,
            Box::new(kf_rm::rmrpc::GraphObjects::new(family)),
            kf_rm::rmrpc::ReasmLimits::default(),
        );
        let chain_logs = kf_rm::ChainLogs::default();
        let census = kf_rm::census::ControlCensusLog::new();
        let policy = kf_rm::served_policy(
            board,
            host.clone(),
            *table,
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
        );
        let model = family.gsp_model(cfg.fb_mb).map_err(|e| format!("{e:?}"))?;
        let gsp = Gsp { fsm: GspFsm::new(abi), model, policy, published: std::collections::HashMap::new() };


        // ★ P4: the windows, their scratch, the PRAMIN views (armed HERE, off every vCPU), the
        // invalidate port; and the VA manager with OUR BAR2 aperture as its first object.
        let (mem, bar1_ops, bar2_ops) = crate::mem::MemPlane::build(
            rm,
            family,
            store.handle,
            store_ptr,
            &layout,
            cfg.bar1_bytes,
            cfg.bar2_bytes,
            ram,
            inbox,
            mirrors,
        )?;
        let walker = kf_mem::vasmgr::GpuWalker { kernel, store_ptr, store_bytes: fb_length };
        // ★ P6b (b): coverage at the family's smallest GMMU page.
        let mut va: crate::mem::Manager = kf_mem::vasmgr::VaManager::new(
            walker,
            fb_length,
            Box::new(move |gpa, len| ram.file_range(gpa, len).map(|(_, off)| off)),
        )
        .with_page_grain(family.mmu_format().small_page_bytes());
        va.table.insert(
            crate::mem::K_BAR2,
            crate::mem::Target::Window(kf_mem::cpuwin::CpuWindow::new(bar2_ops, cfg.bar2_bytes)),
        );
        va.table
            .set_root(crate::mem::K_BAR2, layout.bar2_pde_base, kf_trap::PdbAperture::Vidmem)
            .map_err(|e| format!("our BAR2 root: {e:?}"))?;
        // ★ P5 (P4 row 6): BAR1 is walked from OUR BAR1 root like BAR2 — the guest writes its BAR1
        // PDEs straight into that page (no RPC) and invalidates it; the walk places store views.
        va.table.insert(
            crate::mem::K_BAR1,
            crate::mem::Target::Window(kf_mem::cpuwin::CpuWindow::new(bar1_ops, cfg.bar1_bytes)),
        );
        va.table
            .set_root(crate::mem::K_BAR1, layout.bar1_pde_base, kf_trap::PdbAperture::Vidmem)
            .map_err(|e| format!("our BAR1 root: {e:?}"))?;
        eprintln!(
            "kf3: P4 memory plane: store {} MiB @dev {store_ptr:#x}, roots bar1={:#x} bar2={:#x}, PRAMIN one map+mmap per move, trigger @{:#x}",
            cfg.fb_mb,
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
            boot,
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
                self.counters.unshadowed_writes.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// ★ THE vCPU PATH for a BAR0 write. Lock-free: a shadow store (so a plain register reads
    /// back what was written, as hardware does), the plane's trap, and at most one eventfd write
    /// when a waiter is parked. ⊘ Never blocks, never services.
    pub fn bar0_write(&self, off: u64, val: u64, width: u8) {
        self.counters.trapped.fetch_add(1, Ordering::Relaxed);
        self.counters.last_off.store(off, Ordering::Relaxed);
        let doorbell = match self.plane.doorbell {
            kf_trap::trappolicy::DoorbellPlacement::Bar0 { offset } => {
                off == kf_trap::memmap::VF_USERMODE_PAGE + u64::from(offset)
            }
            kf_trap::trappolicy::DoorbellPlacement::Bar1 { .. } => false,
        };
        let in_usermode = (kf_trap::memmap::VF_USERMODE_PAGE..kf_trap::memmap::VF_USERMODE_PAGE + kf_trap::memmap::PAGE)
            .contains(&off);
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
        let class = if doorbell {
            Class::Doorbell
        } else if in_usermode {
            Class::UserspaceMappable
        } else {
            self.shadow_store(off, val, width);
            // ★ P4: the PRAMIN window base — re-pointed HERE, synchronously, from pre-armed views
            // (§53.1: the register is B, the window A). The word also goes on to the plane.
            if off == self.mem.pramin_reg.offset && width == 4 {
                self.mem.pramin_write(val as u32);
            }
            Class::Privileged { readable: true, semantics: WriteSemantics::Plain }
        };
        let off32 = u32::try_from(off).unwrap_or(u32::MAX);
        match self.plane.trap_write(class, 0, off32, val, width) {
            Action::RingHostInline { host_token } => {
                // ★ P5b: a Passthrough token — ONE fenced store into the host's doorbell, and two
                // relaxed counters for the per-token ledger. Nothing else on the vCPU.
                let reached = self.rm.doorbell(host_token).is_ok();
                self.chans.note_inline((val as u32) & self.plane.token_mask, reached);
            }
            Action::WakeWorker => {
                let _ = self.worker_efd.signal();
            }
            Action::WakeDrainer => {
                let _ = self.drainer_efd.signal();
            }
            Action::RefusedByName | Action::PoisonDevice => {
                self.counters.refused.fetch_add(1, Ordering::Relaxed);
            }
            Action::None => {}
        }
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
        while !self.stop.load(Ordering::Acquire) {
            let mut ready = ReadyTokens::new();
            let _ = poller.wait(&mut ready, PollTimeout::Millis(50));
            let _ = self.mem.inbox.wake.drain();
            for st in self.mem.inbox.take() {
                taken += 1;
                let line = crate::mem::apply_statement(&mut m, &self.mem, self.rm, self.store.handle, st, trigger);
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
            for (ticket, res) in m.take_splits() {
                if let Err(e) = &res {
                    eprintln!("kf3: mem t={:.3}s split ticket {ticket} REFUSED: {e}", self.born.elapsed().as_secs_f64());
                }
                // Ring the channel's own token: its next pump resumes after the split.
                if let Some(tok) = self.mem.inbox.finish_split(ticket, res)
                    && self.plane.ring_internal(tok)
                {
                    let _ = self.worker_efd.signal();
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
        while !self.stop.load(Ordering::Acquire) {
            // A heartbeat for the boot log, on the drainer (never a vCPU): printed only on change.
            if beat.0.elapsed() >= std::time::Duration::from_secs(2) {
                let now = self.status_line();
                if now != beat.1 {
                    eprintln!("{now}");
                }
                beat = (std::time::Instant::now(), now);
            }
            let seen = self.plane.drainer_wake.seen();
            if self.plane.drainer_pass(self, 256) > 0 {
                continue;
            }
            self.release_settled();
            self.deliver_rc();
            if !self.plane.drainer_wake.try_park(seen) {
                continue;
            }
            let mut ready = ReadyTokens::new();
            let _ = poller.wait(&mut ready, PollTimeout::Millis(50));
            self.plane.drainer_wake.unpark();
            let _ = self.drainer_efd.drain();
        }
    }

    /// One line of counters and the GSP phase — for the boot log, never a decision input.
    #[must_use]
    pub fn status_line(&self) -> String {
        let c = &self.counters;
        let o = Ordering::Relaxed;
        let phase = self.gsp.try_lock().map(|g| format!("{:?}", g.fsm.phase())).unwrap_or_else(|_| "busy".into());
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
            " mirrors={} mirror_avg_us={} mirror_max_us={} vat[invals={} arrive->clear_avg_us={} max_us={} walks={} walk_avg_us={} gpu_avg_us={} plan_avg_us={} apply_avg_us={} leaves={} host_calls={}]",
            nm,
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
            " mem[inval={} walks={}/{} cleared={} superseded={} named_missed={} unreconciled={} mapped={} unmapped={} clipped={:#x} held={} vmm_overlaps={} fn70={} roots={} root_moves={} stmts={recv}/{settled} refused={} pramin_repoints={} pramin_miss={} last_miss={:#x} pramin_worst_us={} (map {} mmap {}) pramin_maps={} pramin_mmaps={} inline_opens={} reaped={}]",
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
        ) + &timing;
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
        let rc = format!(
            " rc[armed={} unarmed={} wakes={} seen={} posted={}]",
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
            "kf3: family={:?} phase={phase} trapped={} applied={} refused={} serviced={} ram_refused={} unshadowed_writes={} last_off={:#x}{mem}{chan}{rc}{irq} unserviced=[{}]",
            self.family,
            c.trapped.load(o),
            c.applied.load(o),
            c.refused.load(o),
            c.serviced.load(o),
            c.ram_refused.load(o),
            c.unshadowed_writes.load(o),
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
    fn release_settled(&self) {
        if !self.mem.inbox.all_settled() {
            return;
        }
        let Ok(mut g) = self.gsp.lock() else { return };
        if g.fsm.held_len() == 0 {
            return;
        }
        let g = &mut *g;
        let mut ram = Ram(self);
        match g.fsm.release_held(&mut ram) {
            Ok(n) if n > 0 => self.publish(g),
            Ok(_) => {}
            Err(e) => eprintln!("kf3: held reply post REFUSED: {e:?}"),
        }
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
            if e.live.load(Ordering::Relaxed) > 0
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
        let Ok(mut g) = self.gsp.lock() else { return };
        let g = &mut *g;
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
            Ok(r) => self.log_report(&r),
            Err(e) => eprintln!("kf3: GSP write @{offset:#x}={value:#x} REFUSED: {e:?}"),
        }
        while g.fsm.pending_command_doorbells() > 0 {
            self.counters.serviced.fetch_add(1, Ordering::Relaxed);
            match g.fsm.service_one_deferred_command(&mut ram, g.policy.as_mut()) {
                Ok(r) => self.log_report(&r),
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
        if self.mem.inbox.all_settled() {
            let _ = g.fsm.release_held(&mut ram);
        }
        self.publish(g);
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
