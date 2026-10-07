//! ★★ vCPUs and **MMIO exit dispatch** — `l1_os_shell.md` §10 stage M2-d.
//!
//! M2-c registered traps for real and cross-checked them against the memslot map, and then
//! said what was missing: *"the trap **registration** is real and its consistency with the
//! memslot map is asserted; the **dispatch** is M2-d."* This module is that dispatch, and
//! it is the piece that turns a registered trap into a delivered access.
//!
//! ## ★ What a vCPU makes observable that nothing else could
//!
//! §14.8 F7 named two things the harness *"genuinely cannot observe, both needing a vCPU"*.
//! Both are closed here:
//!
//! - **(a) Where a memslot points.** The guest loads a value we placed at a
//!   guest-physical address and stores it through a trapped register. The value that
//!   arrives at [`VcpuRunner`]'s MMIO handler *is* the memslot's placement — an offset
//!   computed wrongly delivers a different number, not a silently-passing test.
//! - **(b) ★★ The ORDER of a teardown — residual N13.** M2-c's own bite-check ("swap the
//!   teardown order: memslot DELETE before undeclare") *did not bite*, because with no
//!   vCPU the `Arc` keeps the mapping alive either way and no test can tell the orders
//!   apart. With a vCPU the two orders are **physically different**:
//!
//!   > A guest-physical range with no memslot **exits to userspace**. So between a
//!   > premature DELETE and the undeclare that should have preceded it, there is a window
//!   > in which the guest's access to that range arrives here as an MMIO exit **while the
//!   > region map still calls it RAM**. That is the map and the kernel disagreeing, from
//!   > the guest's side, and it is what [`AuditReport::ram_declared_exits`] counts.
//!
//!   Under the correct order the window is *exactly zero wide*, and not statistically:
//!   the undeclare completes under the view lock before the DELETE begins, and this
//!   handler consults the same lock, so "the memslot is gone" happens-before "the range
//!   no longer resolves" for every observer. Under the swapped order the window is the
//!   whole duration of a `munmap` plus a memslot DELETE — measured at 230–460 µs p50 with
//!   a 1–4 ms tail (§6.7) — against a guest that touches the range every few tens of
//!   nanoseconds. The counter is a **quantity**, the invariant is `== 0`, and the bite is
//!   not a coin flip.
//!
//! ## The dispatch itself
//!
//! An MMIO exit carries a guest-physical address and nothing else — no BAR, no device.
//! Routing it is this module's whole job, and it is a **positive** classification in the
//! same shape as `Vmm::gpa_read`'s *prove RAM* rule: an address inside a realized BAR
//! dispatches to [`kayfabe_vmm::Device`]; anything else is [`ExitClass::Unclaimed`] and is
//! counted, never guessed at. A read nobody claims is completed with all-ones — the value
//! a real machine returns for an unclaimed access — rather than zero, because zero is a
//! plausible register value and all-ones is the universal "nothing answered".

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use kayfabe_linux_raw::{KvmVcpu, VcpuExit};
use kayfabe_vmm::{BarId, Device, VmmError};

use crate::{KvmMachine, KvmVmm, Plane, host_refused};

/// How one MMIO exit was classified before it was dispatched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitClass {
    /// Inside a realized BAR: dispatched to the device.
    Bar {
        /// Which BAR.
        bar: BarId,
        /// Offset within it.
        off: u64,
    },
    /// ★★ **The N13 contradiction.** The kernel exited for a guest-physical address the
    /// region map calls RAM. On KVM a RAM region *is* a live memslot, so this state is
    /// unreachable unless a memslot was cleared while the map still promised memory — the
    /// teardown-order bug, observed from the guest's side.
    ///
    /// ★ Since #87 this is **sharper**, not weaker. A store into a read-native overlay's
    /// write-trap span is no longer counted here — it is a [`ExitClass::Bar`], because a
    /// read-only memslot exists precisely so that writes exit. What remains is every
    /// *read* that exits from RAM, **including** a read from a read-native span: that one
    /// the old classifier could not even express, since it had no `is_write` to tell the
    /// two directions apart.
    RamDeclared {
        /// The address the map still calls RAM.
        gpa: u64,
    },
    /// Nothing claimed it: not RAM, not one of our BARs. A real machine's unassigned
    /// space, and the arm that must never silently absorb one of the other two.
    Unclaimed {
        /// The unclaimed address.
        gpa: u64,
    },
}

/// Why a [`VcpuRunner::run_until`] stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// The caller's stop flag was observed set between two exits.
    Stopped,
    /// The exit budget was spent. A **bound, not a timeout**: every assertion about a
    /// vCPU in this suite is over a counted number of exits, never over a clock.
    BudgetSpent,
    /// The guest halted.
    Halted,
    /// The guest triple-faulted, or the hardware refused to enter it, or KVM could not
    /// emulate what it did. Carried rather than retried: a guest that has lost its way is
    /// not something to resume.
    GuestFaulted(VcpuExit),
}

/// What one vCPU did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct VcpuReport {
    /// Total exits handled.
    pub exits: u64,
    /// MMIO stores dispatched into the device.
    pub mmio_writes: u64,
    /// MMIO loads dispatched into the device.
    pub mmio_reads: u64,
    /// Exits at an address the region map calls RAM — **must be zero** (see
    /// [`ExitClass::RamDeclared`]).
    pub ram_declared_exits: u64,
    /// Exits nothing claimed.
    pub unclaimed_exits: u64,
    /// Runs cut short by a signal, which is not an exit at all.
    pub interruptions: u64,
}

/// One vCPU, its dispatch target, and the classification that connects them.
///
/// Owned by exactly one thread (a vCPU is entered from one thread; see [`KvmVcpu`]).
///
/// No `Debug`: `dyn Device` has none, and deriving one would mean bounding the port's
/// trait on `Debug` for a runner's convenience — the tail wagging the seam.
pub struct VcpuRunner {
    vcpu: KvmVcpu,
    plane: Arc<Plane>,
    vmm: KvmVmm,
    device: Arc<dyn Device>,
    report: VcpuReport,
    /// The first contradiction seen, kept so a failure says *which* address, not just
    /// that there was one.
    first_ram_declared: Option<u64>,
}

impl KvmMachine {
    /// Create vCPU `id` and point it at `device`.
    ///
    /// The vCPU starts **stopped and unprogrammed**; [`VcpuRunner::enter_at`] puts it into
    /// flat 32-bit protected mode at a guest-physical entry point.
    ///
    /// # Errors
    /// [`VmmError::HostRefused`] if the kernel refuses the vCPU or its shared run
    /// structure; [`VmmError::Unsupported`] if the run structure's layout is not the one
    /// the raw module is written for.
    ///
    /// # Panics
    /// If called with any ranked lock or any adapter lock held (R1).
    pub fn create_vcpu(&self, id: u32, device: Arc<dyn Device>) -> Result<VcpuRunner, VmmError> {
        let p = &self.plane;
        p.about_to_syscall("KVM_CREATE_VCPU");
        // A hypervisor task-state segment is required before the first entry on Intel
        // hosts and does not exist on AMD ones. Queried, never assumed, and never
        // swallowed: "the kernel said no" and "this host has no such concept" are
        // different facts.
        p.vm.set_tss_addr_if_supported()
            .map_err(|e| host_refused("the hypervisor task-state segment", &e))?;
        let vcpu = KvmVcpu::create(&p.kvm, Arc::clone(&p.vm), id)
            .map_err(|e| host_refused("creating a vCPU", &e))?;
        Ok(VcpuRunner {
            vcpu,
            plane: Arc::clone(&self.plane),
            vmm: self.vmm(),
            device,
            report: VcpuReport::default(),
            first_ram_declared: None,
        })
    }
}

impl KvmMachine {
    /// ★ How an MMIO exit at `[gpa, gpa+len)` **would** be classified — the N13 detector,
    /// queryable without a guest.
    ///
    /// This exists because of a bite-check that did **not** bite: neutering
    /// [`Plane::classify_exit`] so it never answers [`ExitClass::RamDeclared`] left the
    /// whole mean run green, because `ram_declared_exits == 0` is exactly as true of a
    /// working detector as of a detector that cannot fire. That is the
    /// witness-that-cannot-fail shape (`l1_concurrency.md` §12.43 N12), one component over,
    /// and an invariant asserted with an instrument nobody tested is a claim, not a test.
    ///
    /// So the classification is a **query with all three answers reachable**, and the suite
    /// asserts each of them positively against a machine whose layout it knows. A live RAM
    /// window answers `RamDeclared` here — correctly: the answer means *"an exit at this
    /// address would be the map and the kernel disagreeing"*, and while the window is live
    /// no exit can happen, which is the whole invariant.
    ///
    /// ## ★★ Why `is_write` is an argument and not an omission
    ///
    /// Over a read-native overlay's write-trap span the two directions have **different**
    /// answers, so a classifier that could not see the direction had to get one of them
    /// wrong. A load there is served by the read-only memslot with no exit, so an exit is
    /// the contradiction; a store there exits *by design* and belongs to the device. #87
    /// was exactly that: the store was counted as the N13 defect **and dropped**, because
    /// only the [`ExitClass::Bar`] arm reaches [`kayfabe_vmm::Device`].
    #[must_use]
    pub fn classify_guest_exit(&self, gpa: u64, len: u64, is_write: bool) -> ExitClass {
        self.plane.classify_exit(gpa, len, is_write)
    }
}

impl VcpuRunner {
    /// Put the guest into flat 32-bit protected mode at `entry`, with `rbx = rbx`.
    ///
    /// `rbx` is how this harness gives each vCPU its own trapped address to store
    /// through, so several vCPUs are distinguishable at the exit without several code
    /// images.
    ///
    /// # Errors
    /// [`VmmError::Unsupported`] on a non-x86 host; [`VmmError::HostRefused`] if the
    /// kernel refuses the register state.
    ///
    /// # Panics
    /// If called with any ranked lock or any adapter lock held (R1).
    pub fn enter_at(&self, entry: u64, rbx: u64) -> Result<(), VmmError> {
        self.plane
            .about_to_syscall("programming a vCPU's entry state");
        self.vcpu
            .enter_flat_protected_mode(entry, rbx)
            .map_err(|e| host_refused("a vCPU's entry state", &e))
    }

    /// What this vCPU has done so far.
    #[must_use]
    pub fn report(&self) -> VcpuReport {
        self.report
    }

    /// The first address that exited while the region map still called it RAM, if any.
    #[must_use]
    pub fn first_ram_declared_exit(&self) -> Option<u64> {
        self.first_ram_declared
    }

    /// Run the guest until `stop` is set, `budget` exits have been handled, or the guest
    /// stops being runnable.
    ///
    /// `stop` is checked **between** exits, which is why every guest image in this suite
    /// is a loop that traps: the vCPU is never more than one instruction away from
    /// returning, so a teardown never waits on a clock.
    ///
    /// # Errors
    /// [`VmmError::HostRefused`] if `KVM_RUN` itself fails.
    ///
    /// # Panics
    /// If entered with any ranked lock or any adapter lock held (R1).
    pub fn run_until(&mut self, stop: &AtomicBool, budget: u64) -> Result<StopReason, VmmError> {
        let mut handled = 0u64;
        loop {
            if stop.load(Ordering::Acquire) {
                return Ok(StopReason::Stopped);
            }
            if handled >= budget {
                return Ok(StopReason::BudgetSpent);
            }
            let exit = self
                .vcpu
                .run()
                .map_err(|e| host_refused("entering the guest", &e))?;
            match exit {
                VcpuExit::Mmio {
                    gpa,
                    len,
                    is_write,
                    data,
                } => {
                    self.report.exits += 1;
                    handled += 1;
                    self.dispatch(gpa, len, is_write, data)?;
                }
                VcpuExit::Interrupted => self.report.interruptions += 1,
                VcpuExit::Halted => return Ok(StopReason::Halted),
                other => return Ok(StopReason::GuestFaulted(other)),
            }
        }
    }

    /// Classify one MMIO exit and hand it to the device.
    fn dispatch(
        &mut self,
        gpa: u64,
        len: u8,
        is_write: bool,
        data: [u8; 8],
    ) -> Result<(), VmmError> {
        self.plane.audit.guest_exits.fetch_add(1, Ordering::SeqCst);
        let class = self.plane.classify_exit(gpa, u64::from(len), is_write);
        if let ExitClass::RamDeclared { gpa } = class {
            self.report.ram_declared_exits += 1;
            self.plane
                .audit
                .ram_declared_exits
                .fetch_add(1, Ordering::SeqCst);
            self.plane
                .audit
                .first_ram_declared_exit
                .compare_exchange(u64::MAX, gpa, Ordering::SeqCst, Ordering::SeqCst)
                .ok();
            self.first_ram_declared.get_or_insert(gpa);
        }
        match class {
            ExitClass::Bar { bar, off } => {
                if is_write {
                    self.report.mmio_writes += 1;
                    self.device
                        .mmio_write(&mut self.vmm, bar, off, len, u64::from_le_bytes(data));
                } else {
                    self.report.mmio_reads += 1;
                    let v = self.device.mmio_read(&mut self.vmm, bar, off, len);
                    self.complete_read(len, v)?;
                }
            }
            // Neither of the other two arms reaches the device. A RAM-declared exit is a
            // state that must not exist and is counted so it fails an assertion, not
            // served so it fails a debugging session; an unclaimed one is a real machine's
            // unassigned space.
            ExitClass::RamDeclared { .. } | ExitClass::Unclaimed { .. } => {
                if let ExitClass::Unclaimed { .. } = class {
                    self.report.unclaimed_exits += 1;
                }
                if !is_write {
                    self.complete_read(len, u64::MAX)?;
                }
            }
        }
        Ok(())
    }

    fn complete_read(&mut self, len: u8, value: u64) -> Result<(), VmmError> {
        self.vcpu
            .complete_mmio_read(len, value)
            .map_err(|e| host_refused("completing an MMIO load", &e))
    }
}

impl Plane {
    /// Classify a guest-physical address that just exited to userspace.
    ///
    /// Takes the **view** lock — the same one `remove_window` undeclares under, which is
    /// what makes the ordering argument in this module's docs a happens-before rather than
    /// a hope.
    pub(crate) fn classify_exit(&self, gpa: u64, len: u64, is_write: bool) -> ExitClass {
        // ★★ Both facts, one critical section. Reading the region map and the write-trap
        // spans under two separate acquisitions would let a `remove_window` land between
        // them, and the answer would then describe two different instants of the plane.
        let (ram, write_trapped) = {
            let (v, _h) = self.view();
            (
                v.regions.resolve(gpa, len.max(1)).is_ok(),
                // Start-address containment, because an MMIO exit reports the address
                // that FAULTED. A store straddling the end of the read-only span faults
                // at its first byte, inside the span, and is the device's to see.
                v.write_traps
                    .values()
                    .any(|t| gpa >= t.start && gpa < t.end),
            )
        };
        // ★★★ Reads and writes are NOT symmetric over a read-only memslot, and this is
        // the whole of #87. A **read** there is served from RAM and cannot exit, so an
        // exit is the contradiction — as it is for any other RAM. A **write** there
        // exits BY DESIGN: that is what `KVM_MEM_READONLY` is for, and it is the
        // read-native overlay's other half. Classifying it `RamDeclared` both slandered
        // a correct event as the teardown-order defect and dropped the guest's store,
        // because only the `Bar` arm reaches the device.
        //
        // Falling through to the BAR lookup is safe *because* `install_window` refuses a
        // write-trap span that no realized BAR covers — otherwise this would land in
        // `Unclaimed` and drop the store anyway, one layer down.
        if ram && !(is_write && write_trapped) {
            return ExitClass::RamDeclared { gpa };
        }
        for b in &self.bars {
            if gpa >= b.base && gpa < b.base + b.len {
                return ExitClass::Bar {
                    bar: b.bar,
                    off: gpa - b.base,
                };
            }
        }
        ExitClass::Unclaimed { gpa }
    }
}

kayfabe_util::assert_send_sync!(ExitClass, StopReason, VcpuReport);
