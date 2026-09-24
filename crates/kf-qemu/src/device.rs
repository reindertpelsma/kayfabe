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
use std::sync::{Mutex, OnceLock, RwLock};

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

/// A registered guest-RAM block: guest-physical `[gpa, gpa+len)` at `mem`.
#[derive(Debug, Clone, Copy)]
struct RamBlock {
    gpa: u64,
    mem: RawRegion,
}

/// The GSP side the drainer owns.
struct Gsp {
    fsm: GspFsm,
    model: Box<dyn GspModel>,
    policy: Box<dyn CommandPolicy>,
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
}

/// ★ The device.
pub struct Device {
    /// The host RM session.
    pub rm: kf_host::HostRm,
    /// The host's family.
    pub family: Family,
    /// What the C device presents.
    pub identity: Identity,
    /// The store (the guest's whole framebuffer, one object).
    pub store: kf_host::Reservation,
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
    ram: RwLock<Vec<RamBlock>>,
    gsp: Mutex<Gsp>,
    boot: Vec<BootReg>,
    vbios: Vec<u8>,
    worker_efd: Notifier,
    drainer_efd: Notifier,
    stop: AtomicBool,
    /// Boot-log counters.
    pub counters: Counters,
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
        let rm = kf_host::HostRm::open(&dev, kf_arch::ids::GpuId(cfg.gpu_minor), &kf_chip::choose_host_classes)
            .map_err(|e| e.to_string())?;
        let (architecture, implementation, revision) = rm.arch_info();
        let family = Family::from_arch(architecture, implementation).map_err(|e| format!("family: {e:?}"))?;
        // ★ P3: the host die's facts, each from the source `kf_rm::hostfacts::PROVENANCE` names —
        // asked before anything is reserved, so a refusal costs nothing. What the host cannot
        // state is authored as OUR device's (`kf_rm::authored`, cited per value). ⊘ A control the
        // host refuses, or a family an authored rule has no number for (Hopper's PBDMA fault ids),
        // refuses REALIZE, listing every such field: never a default, never a GA106 row.
        let host = std::sync::Arc::new(
            crate::rmfacts::host_facts(&rm, family).map_err(|e| format!("host facts: {e}"))?,
        );
        let sysfs = crate::hostfacts::sysfs_dir_for_minor(cfg.gpu_minor)?;
        let pci = crate::hostfacts::read_host_pci(&sysfs)?;

        let fb_length = cfg.fb_mb << 20;
        let layout = fb_layout(fb_length).ok_or(format!("a {} MiB store cannot hold the firmware carve-out", cfg.fb_mb))?;
        let store = rm.reserve_gpga(fb_length).map_err(|e| format!("store of {} MiB refused: {e:?}", cfg.fb_mb))?;

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
            kf_rm::ObjectLinks { objects: Some(Box::new(objects)) },
        );
        let model = family.gsp_model(cfg.fb_mb).map_err(|e| format!("{e:?}"))?;
        let gsp = Gsp { fsm: GspFsm::new(abi), model, policy };

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

        Ok(Device {
            rm,
            family,
            identity: Identity { pci, bar0_bytes },
            store,
            plane,
            host_facts: host,
            chain_logs,
            census,
            pieces: OnceLock::new(),
            staged: Mutex::new(Vec::new()),
            ram: RwLock::new(Vec::new()),
            gsp: Mutex::new(gsp),
            boot,
            vbios,
            worker_efd: Notifier::create().map_err(|e| format!("eventfd: {e:?}"))?,
            drainer_efd: Notifier::create().map_err(|e| format!("eventfd: {e:?}"))?,
            stop: AtomicBool::new(false),
            counters: Counters::default(),
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
        if let Ok(g) = self.gsp.lock() {
            self.publish(&g);
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
        let class = if doorbell {
            Class::Doorbell
        } else if in_usermode {
            Class::UserspaceMappable
        } else {
            self.shadow_store(off, val, width);
            Class::Privileged { readable: true, semantics: WriteSemantics::Plain }
        };
        let off32 = u32::try_from(off).unwrap_or(u32::MAX);
        match self.plane.trap_write(class, 0, off32, val, width) {
            Action::RingHostInline { host_token } => {
                let _ = self.rm.doorbell(host_token);
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

    /// Register guest RAM: guest-physical `[gpa, gpa+len)` at `mem`.
    pub fn ram_add(&self, gpa: u64, mem: RawRegion) {
        if let Ok(mut r) = self.ram.write() {
            r.retain(|b| b.gpa != gpa);
            r.push(RamBlock { gpa, mem });
            r.sort_by_key(|b| b.gpa);
        }
    }

    /// Unregister the guest RAM block at `gpa`.
    pub fn ram_del(&self, gpa: u64) {
        if let Ok(mut r) = self.ram.write() {
            r.retain(|b| b.gpa != gpa);
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
    fn publish(&self, g: &Gsp) {
        const EDGES: [GspReg; 4] =
            [GspReg::GspFalconCpuctl, GspReg::GspRiscvCpuctl, GspReg::Sec2FalconCpuctl, GspReg::GspFalconIrqstat];
        let store = |reg: GspReg| {
            let Some((0, off)) = g.model.at(reg) else { return };
            if let Some(Ok(v)) = g.fsm.mmio_read_with(g.model.as_ref(), 0, off) {
                self.shadow_store(off, v, 4);
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
        format!(
            "kf3: family={:?} phase={phase} trapped={} applied={} refused={} serviced={} ram_refused={} unshadowed_writes={} last_off={:#x} unserviced=[{}]",
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

    /// Stop the device's threads.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.drainer_efd.signal();
        let _ = self.worker_efd.signal();
    }
}

/// Guest RAM as the GSP FSM reads it — through the blocks QEMU registered.
struct Ram<'a>(&'a Device);

impl GuestRam for Ram<'_> {
    fn read(&mut self, gpa: u64, buf: &mut [u8]) -> Result<(), RamRefused> {
        let blocks = self.0.ram.read().map_err(|_| RamRefused { gpa, len: buf.len(), why: "no guest-RAM block QEMU registered covers this range" })?;
        for b in blocks.iter() {
            if gpa >= b.gpa && gpa - b.gpa + buf.len() as u64 <= b.mem.len() as u64 {
                if b.mem.read_into((gpa - b.gpa) as usize, buf) {
                    return Ok(());
                }
            }
        }
        self.0.counters.ram_refused.fetch_add(1, Ordering::Relaxed);
        Err(RamRefused { gpa, len: buf.len(), why: "no guest-RAM block QEMU registered covers this range" })
    }
    fn write(&mut self, gpa: u64, bytes: &[u8]) -> Result<(), RamRefused> {
        let blocks = self.0.ram.read().map_err(|_| RamRefused { gpa, len: bytes.len(), why: "no guest-RAM block QEMU registered covers this range" })?;
        for b in blocks.iter() {
            if gpa >= b.gpa && gpa - b.gpa + bytes.len() as u64 <= b.mem.len() as u64 {
                if b.mem.write_from((gpa - b.gpa) as usize, bytes) {
                    return Ok(());
                }
            }
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
    fn run_translated(&self, _host_token: u32, _up_to_seq: u64) -> bool {
        false
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
        let _ = g.fsm.release_held(&mut ram);
        self.publish(g);
    }
    fn operands_translatable(&self, _host_token: u32, _up_to_seq: u64) -> Translatable {
        Translatable::No
    }
    fn forge_completion(&self, _host_token: u32) {}
    fn refuse_and_poison(&self, _host_token: u32) {}
    fn fault_channel(&self, _host_token: u32) {}
    fn map_guest_slice(&self, _slice: HostSlice) {}
    fn teardown_step(&self, _step: Step) {}
}
