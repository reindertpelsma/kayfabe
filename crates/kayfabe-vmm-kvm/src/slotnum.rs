//! ★★★ Kernel memslot **numbers**, and the one policy question the two VMM backends
//! answered in opposite directions.
//!
//! # The divergence, and how it was settled
//!
//! Until this module existed, `kayfabe-vmm-kvm` allocated memslot numbers by a bare
//! `next_memslot += n`, documented as *"Numbers are not recycled: a recycled slot number is
//! indistinguishable from a stale one in a kernel log, and the ceiling is in the
//! hundreds."* One crate over, [`kayfabe_vmm_qemu::slots::SlotAllocator`] does recycle, and
//! records **why**: the C artifact's first allocator did not, and
//! (`C: nvkvm_mmap_host.c:382-389`) it *"exhausted the pool after a few CUDA processes"* —
//! which is why the C's second allocator (`:404-421`) is a LIFO free stack.
//!
//! Neither doc cited the other, so one backend was running the policy the other's
//! documentation calls a known, **measured** failure mode. The adjudication:
//!
//! - **Never-recycling loses.** The C's exhaustion is a datum, not a prediction, and it is
//!   a property of the *workload* — window install/remove churn — not of the hypervisor.
//!   Every argument that made it fatal in the C applies here unchanged: this backend's
//!   `install_window`/`remove_window` pair is the same churn, and "the ceiling is in the
//!   hundreds" is precisely the sentence the C wrote before measuring a few hundred windows.
//! - **The debuggability argument survives, at its real weight.** A recycled number really
//!   is ambiguous in a kernel log — but the answer to that is
//!   [`MemslotNumbers::release`]'s contract (a number returns to the pool only *after* the
//!   kernel has been told the slot is gone), not refusing to reuse numbers at all. An
//!   ambiguous log line costs an investigation; an exhausted pool costs the guest.
//! - **What legitimately still differs is the DIRECTION, and that is a hypervisor
//!   property.** In `kayfabe-vmm-qemu` we are a device *inside* a hypervisor that allocates
//!   memslot numbers densely upward from zero (`qemu: kvm-all.c:250-262`), so our numbers
//!   must descend from `KVM_CAP_NR_MEMSLOTS` to stay disjoint from a namespace we do not
//!   own — and a collision there is not an error but a silent **replace** of QEMU's own
//!   mapping. Here we *are* the VMM: the whole number space is ours, nobody else allocates
//!   into it, so ascending from zero is correct and no budget floor is needed. That
//!   asymmetry follows from who owns the namespace, which is exactly the "property of the
//!   hypervisor, not an accident of who wrote it second" test.
//!
//! The two allocators are therefore *deliberately* different types with the same release
//! contract, and the duplication is named rather than hidden — see the crate docs' seam map.

/// Refusal: every number in the kernel's memslot space is live at once.
///
/// ★ Note what it now takes to reach this: `OUR_BUDGET`-many windows **live
/// simultaneously**. Before recycling, install/remove churn reached it too, which is the
/// C's measured failure and is no longer a way to get here.
pub const MEMSLOT_CEILING_REACHED: &str = "the kernel's memslot ceiling — §6.7's frequency rule is what keeps a data plane away \
     from it";

/// ★★ Memslot numbers for a VM **this process owns**, allocated ascending from zero with a
/// free list.
///
/// See the module docs for why this is not the same type as the QEMU adapter's.
#[derive(Debug)]
pub struct MemslotNumbers {
    /// The kernel's ceiling. Numbers are `< ceiling`.
    ceiling: u32,
    /// The next fresh number, ascending. Equals `0` before the first allocation.
    next: u32,
    /// Numbers whose slot has been **cleared in the kernel** and may be re-issued.
    free: Vec<u32>,
    /// Cumulative reuses — the non-vacuity witness for the free list. A suite in which this
    /// stays zero has not exercised recycling at all, and would pass identically against
    /// the never-recycling allocator this replaced.
    recycled: u64,
}

impl MemslotNumbers {
    /// An allocator over `[0, ceiling)`.
    #[must_use]
    pub fn new(ceiling: u32) -> Self {
        MemslotNumbers {
            ceiling,
            next: 0,
            free: Vec::new(),
            recycled: 0,
        }
    }

    /// The kernel's ceiling this allocator was built from.
    #[must_use]
    pub fn ceiling(&self) -> u32 {
        self.ceiling
    }

    /// How many numbers have been re-issued from the free list.
    #[must_use]
    pub fn recycled(&self) -> u64 {
        self.recycled
    }

    /// How many numbers are currently handed out.
    #[must_use]
    pub fn live(&self) -> u32 {
        self.next - u32::try_from(self.free.len()).unwrap_or(u32::MAX)
    }

    /// Take `n` numbers, or refuse **without taking any**.
    ///
    /// All-or-nothing because the caller installs a window's spans as a unit: handing out
    /// two of three and refusing the third would leave the caller unwinding numbers it had
    /// not recorded anywhere yet.
    ///
    /// ★ The free list is drained **first**, so a churning workload never advances `next`
    /// at all and the ceiling stops being a function of how long the machine has been up.
    ///
    /// # Errors
    /// [`MEMSLOT_CEILING_REACHED`].
    pub fn alloc(&mut self, n: usize) -> Result<Vec<u32>, &'static str> {
        let fresh_available = (self.ceiling - self.next) as usize;
        if n > self.free.len() + fresh_available {
            return Err(MEMSLOT_CEILING_REACHED);
        }
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            if let Some(s) = self.free.pop() {
                self.recycled += 1;
                out.push(s);
            } else {
                out.push(self.next);
                self.next += 1;
            }
        }
        Ok(out)
    }

    /// Return a number to the pool.
    ///
    /// # ★★★ CONTRACT
    ///
    /// **The kernel must already have been told the slot is gone.** Call this only after the
    /// [`kayfabe_linux_raw::KvmMemslot`] holding `slot` has been dropped — its `Drop` issues
    /// the clearing ioctl. Re-issuing a number the kernel still has a live mapping for turns
    /// the next install from an ADD into a **REPLACE**, which does not fail and says
    /// nothing. That contract, not a refusal to recycle, is what makes recycling safe.
    ///
    /// # Panics
    /// If `slot` was never handed out by this allocator, or is already free. Both are
    /// bookkeeping bugs that would otherwise surface much later as a double-installed slot.
    pub fn release(&mut self, slot: u32) {
        assert!(
            slot < self.next,
            "memslot number {slot} was never handed out by this allocator (live range \
             0..{next})",
            next = self.next,
        );
        assert!(
            !self.free.contains(&slot),
            "memslot number {slot} is already free; releasing it twice would hand the same \
             number to two live windows"
        );
        self.free.push(slot);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ★★★ **The C's measured failure, as a test.** `nvkvm_mmap_host.c:382-389` recorded
    /// that a never-recycling allocator *"exhausted the pool after a few CUDA processes"*.
    /// Churn — install, remove, install — must therefore be free of the ceiling entirely.
    ///
    /// This is written as *many times the ceiling* deliberately: with the old
    /// `next_memslot += n` this loop refuses on iteration `ceiling`, which is the whole
    /// point of writing the bound as a multiple rather than as a number.
    #[test]
    fn churn_never_reaches_the_ceiling_which_is_the_c_s_measured_failure() {
        let ceiling = 32u32;
        let mut a = MemslotNumbers::new(ceiling);
        for i in 0..ceiling * 10 {
            let n = a
                .alloc(1)
                .unwrap_or_else(|e| panic!("iteration {i} exhausted a pool it churns: {e}"));
            a.release(n[0]);
        }
        assert_eq!(a.live(), 0, "every number went back");
        assert_eq!(
            a.next, 1,
            "★ the free list, not the fresh cursor, served every reuse — a passing test \
             with `next == ceiling * 10` would mean the allocator merely had a bigger pool"
        );
        assert_eq!(
            a.recycled(),
            u64::from(ceiling * 10 - 1),
            "★ NON-VACUITY: recycling really happened"
        );
    }

    /// ★ The ceiling is still real — it now takes that many numbers **live at once**.
    #[test]
    fn the_ceiling_binds_on_simultaneously_live_numbers() {
        let mut a = MemslotNumbers::new(8);
        let held = a.alloc(8).expect("exactly the ceiling");
        assert_eq!(held, vec![0, 1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(a.live(), 8);
        assert_eq!(a.alloc(1), Err(MEMSLOT_CEILING_REACHED));
        // …and freeing exactly one makes exactly one available again.
        a.release(3);
        assert_eq!(a.alloc(1), Ok(vec![3]));
        assert_eq!(a.alloc(1), Err(MEMSLOT_CEILING_REACHED));
    }

    /// ★ All-or-nothing: a refused multi-span request takes **no** numbers.
    #[test]
    fn a_refused_request_takes_nothing() {
        let mut a = MemslotNumbers::new(4);
        let _held = a.alloc(3).expect("three of four");
        assert_eq!(a.alloc(2), Err(MEMSLOT_CEILING_REACHED));
        assert_eq!(a.live(), 3, "the refused request consumed nothing");
        assert_eq!(a.alloc(1), Ok(vec![3]), "the last number is still there");
    }

    /// ★★ Releasing a number twice is a **panic**, not a silent double-hand-out. This is
    /// the guard that makes the release contract enforceable rather than merely written.
    #[test]
    #[should_panic(expected = "is already free")]
    fn releasing_twice_panics_rather_than_handing_one_number_to_two_windows() {
        let mut a = MemslotNumbers::new(4);
        let n = a.alloc(1).expect("one");
        a.release(n[0]);
        a.release(n[0]);
    }

    /// ★ And so is releasing a number that was never handed out.
    #[test]
    #[should_panic(expected = "was never handed out")]
    fn releasing_a_number_never_handed_out_panics() {
        let mut a = MemslotNumbers::new(4);
        a.release(2);
    }

    /// ★ A zero-length request is legal and takes nothing — `memslot_spans` can produce a
    /// one-span window, and the all-or-nothing arm must not turn `n == 0` into a refusal at
    /// a full pool.
    #[test]
    fn zero_is_legal_even_at_the_ceiling() {
        let mut a = MemslotNumbers::new(2);
        let _held = a.alloc(2).expect("the whole pool");
        assert_eq!(a.alloc(0), Ok(vec![]));
    }
}
