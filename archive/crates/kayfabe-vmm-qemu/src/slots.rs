//! ★★★ The memslot plane — **our** slots, in a machine we did not create.
//!
//! `host_execution_plane.md` §1: the hypervisor reserves the guest-physical window and does
//! not back it; we install the slots that shadow it. This module is that half, and it is
//! deliberately *not* on [`crate::host::QemuHost`]: installing a memslot is a call to the
//! **kernel**, so putting it on the trait whose subject is the hypervisor's C API would
//! make it untestable for the same reason everything else on that trait is — there is no
//! real implementor.
//!
//! ## The three tiers, and what each one physically is
//!
//! | tier | mechanism | guest read | guest write |
//! |---|---|---|---|
//! | **passthrough** | an ordinary read-write memslot | native | native |
//! | **read-native** | a memslot with the kernel's read-only flag | native | **exits to us** |
//! | **observe-everything** | *no memslot at all* | exits | exits |
//!
//! ★★ **`host_execution_plane.md` §1.5's last box: this taxonomy is NEW CONSTRUCTION, not a
//! port.** Mode 2 in the C artifact has never had a memslot —
//! `C: nvkvm_gpu_emul.c:9743-9779` makes every BAR trapping MMIO and
//! `KVM_SET_USER_MEMORY_REGION` appears nowhere in that file — and the Mode-1 window that
//! *does* have one uses exactly one, read-write, forever: `readonly` is a dead parameter
//! there (`C: nvkvm_mmap_host.c:467-473`, both call sites pass `false`). So the C is
//! precedent for **reservation + one read-write slot** and for nothing else, and the
//! read-only tier and the mixed layout have to earn their own green gate here.
//!
//! ## ★★ Why there is a seam at all, when one implementation is real
//!
//! Because the *other* implementation is what makes the plane testable where the kernel is
//! absent. `/dev/kvm` is not present on this project's CI runner, so a design in which
//! every tiering assertion needs a real machine is a design whose tiering assertions CI
//! never runs (`kvm_gated_tests_ci_blind`). The seam gives both: the whole allocator, the
//! whole span algebra and every refusal are exercised without a kernel, and a **named
//! differential** runs the identical scenario list against [`KvmSlotPlane`] so the mock
//! cannot quietly stop lying where the kernel does.

use std::fmt;
use std::sync::{Arc, Mutex};

use kayfabe_linux_raw::{GuestWindow, KvmMemslot, KvmVm, RawError};

/// `EINVAL` as Linux's `errno.h` defines it.
///
/// ★★ **Retyped, not imported, and that is deliberate.** `kayfabe-vmm-kvm` keeps `libc` a
/// *dev*-dependency on the stated rule that *"the crate itself names no OS constant: every
/// syscall belongs to `kayfabe-linux-raw`, and a dependency here would be the first step of
/// that rule decaying"*. The same rule holds here, and these two numbers are needed in the
/// crate's **src** because [`crate::mock_host::MockSlotPlane`] lives there (an integration
/// test cannot see a `cfg(test)` item).
///
/// The obvious objection — *"a hardcoded errno can be wrong"* — is answered by measurement
/// rather than by care: the QEMU adapter's slot differential runs the identical scenario
/// list against [`KvmSlotPlane`] and asserts the **kernel's own** value equals this one, so
/// a wrong constant is a red test and not a silent divergence.
pub const KERNEL_EINVAL: i32 = 22;

/// `EEXIST` as Linux's `errno.h` defines it. See [`KERNEL_EINVAL`] for why it is retyped.
pub const KERNEL_EEXIST: i32 = 17;

/// `EBUSY` as Linux's `errno.h` defines it — task #97's arm, the number the hypervisor's
/// discard-disable returns when a *requirer* is already present. See [`KERNEL_EINVAL`].
pub const KERNEL_EBUSY: i32 = 16;

/// The number of memslot numbers this device may hold at once.
///
/// ★ Not a resource limit — a **collision floor**. See [`SlotAllocator`] for the whole
/// argument; the short form is that our numbers descend from the kernel's ceiling and the
/// hypervisor's ascend from zero, so the budget is the width of the gap we are prepared to
/// let the hypervisor grow into before we refuse rather than overwrite.
pub const OUR_SLOT_BUDGET: u32 = 64;

/// ★★★★★ **w393 — the BAR mirror's slot budget**, on top of [`OUR_SLOT_BUDGET`].
///
/// The demand-driven BAR1/BAR2 mirror installs **one memslot per guest-touched 4 KiB
/// aperture page** (`docs/design/bar1_passthrough_device_local_host_visible.md` §2.3), so
/// its population is the guest's BAR working set, not a handful of reservations. 4096 slots
/// = 16 MiB of BAR pages mirrored at once; past it the mirror refuses by name
/// (`SLOT-BUDGET`) and the page is served by the trap, correctly and slowly.
///
/// ⊘ Taken only when the kernel's ceiling can hold it with a hypervisor's worth beneath
/// (see [`SlotAllocator::for_machine`]): on a kernel reporting the old 509-slot ceiling the
/// allocator falls back to [`OUR_SLOT_BUDGET`] alone and the mirror's census says so.
pub const MIRROR_SLOT_BUDGET: u32 = 4096;

/// A live memslot. Dropping it **clears the slot in the kernel** before releasing whatever
/// the kernel was pointing at.
///
/// Object-safe and empty on purpose: nothing above this seam may do anything with a live
/// slot except keep it alive and eventually drop it. The lifetime hazard — the kernel holds
/// a host address that no Rust lifetime describes — is closed inside the implementation,
/// not by the caller.
pub trait LiveSlot: Send + Sync + fmt::Debug {}

/// `kayfabe_linux_raw::KvmMemslot` already owns its window by `Arc` and clears the slot in
/// its own `Drop`, which is the entire contract.
impl LiveSlot for KvmMemslot {}

/// The kernel's memslot facility, as this adapter needs it.
pub trait SlotPlane: Send + Sync + fmt::Debug {
    /// How many memslots this machine may hold at once — the kernel's own ceiling.
    ///
    /// # Errors
    /// [`RawError`] if the capability query failed.
    fn ceiling(&self) -> Result<u32, RawError>;

    /// Install one slot. `guest_readonly` is the read-native tier.
    ///
    /// # Errors
    /// [`RawError::Syscall`] carrying the kernel's own `errno` — `EEXIST` for a
    /// guest-physical range another slot already covers, `EINVAL` for a misaligned base or
    /// a number past the ceiling.
    fn install(
        &self,
        slot: u32,
        gpa: u64,
        window: &Arc<GuestWindow>,
        window_offset: u64,
        len: u64,
        guest_readonly: bool,
    ) -> Result<Box<dyn LiveSlot>, RawError>;

    /// ★★★★★ **This machine's slot-number space** — see [`SlotNumberSpace`].
    ///
    /// ⊘⊘ **The cursor belongs to the MACHINE, and that is the whole point (w641).** Slot
    /// numbers are the kernel's, scoped to one VM; every device on one machine must draw from
    /// one cursor or two GPUs collide, and two *different* machines must not share one or a
    /// device that fits is refused a window carved out of someone else's ceiling.
    ///
    /// ⚠ A process-wide static would get the first half right and the second half wrong — and
    /// it did: parking the cursor in a `static` made two integration tests on two independent
    /// mock machines fight over one frontier, which is the same bug as the one being fixed,
    /// one level up. **The plane is the machine; the machine owns the numbers.**
    fn number_space(&self) -> &Arc<SlotNumberSpace>;
}

/// The real one: memslots in a real machine.
#[derive(Debug)]
pub struct KvmSlotPlane {
    vm: Arc<KvmVm>,
    /// This machine's number space, shared by every device that discovered this plane.
    space: Arc<SlotNumberSpace>,
}

impl KvmSlotPlane {
    /// Wrap a machine descriptor — one this process created, adopted, or discovered.
    #[must_use]
    pub fn new(vm: Arc<KvmVm>) -> Self {
        KvmSlotPlane {
            vm,
            space: Arc::new(SlotNumberSpace::new()),
        }
    }

    /// ★ Find the hypervisor's own machine and wrap it — the door
    /// `host_execution_plane.md` §1 actually opens, ported from
    /// `C: src/qemu/virtio_nvgpu.c:1114-1141`.
    ///
    /// # Errors
    /// As [`KvmVm::discover_in_this_process`]: no machine, more than one, or a descriptor
    /// that did not confirm.
    pub fn discover() -> Result<Self, RawError> {
        // ⊘⊘⊘ **MEMOISED, AND THE SECOND DEVICE IS WHY (w641).** `discover_in_this_process`
        // requires **exactly one** `/proc/self/fd` entry linking to `anon_inode:kvm-vm`, and it
        // keeps its dup alive for the device's lifetime. So device 0 realizing leaves a second
        // descriptor behind, device 1's scan finds two, and the refusal reads *"this process
        // holds more than one KVM machine"* — **false**: the process holds one machine and two
        // descriptors to it, the second of which is **ours**. The operator is pointed at their
        // own invocation for a mess we made.
        //
        // ★ There is exactly one machine per QEMU process, so both the descriptor AND the slot
        // number space it owns are process-wide here — the test being **not "is it a static"
        // but "is the thing it names process-wide?"**. Every device shares one `Arc` of each,
        // and the scan runs once, while the count is still one.
        static PLANE: Mutex<Option<(Arc<KvmVm>, Arc<SlotNumberSpace>)>> = Mutex::new(None);
        let mut g = PLANE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((vm, space)) = g.as_ref() {
            return Ok(KvmSlotPlane {
                vm: Arc::clone(vm),
                space: Arc::clone(space),
            });
        }
        let vm = Arc::new(KvmVm::discover_in_this_process()?);
        let space = Arc::new(SlotNumberSpace::new());
        *g = Some((Arc::clone(&vm), Arc::clone(&space)));
        Ok(KvmSlotPlane { vm, space })
    }

    /// [`KvmSlotPlane::discover`], with the refusal already told apart.
    ///
    /// ⊘ **The discrimination the library built was being discarded.** The caller in
    /// `kayfabe-qemu-raw` matched `Err(_)` and printed one sentence, so a log could not say
    /// whether the process had **no** machine or **several** — and those are different
    /// problems for whoever reads it. The classification lives here because this is the crate
    /// that can see [`RawError`]'s variants.
    ///
    /// # Errors
    /// A sentence naming which of the two arms fired.
    pub fn discover_or_reason() -> Result<Self, &'static str> {
        Self::discover().map_err(|e| match e {
            RawError::Unsupported { what, .. } if what == "this process's KVM VM descriptor" => {
                "found no accelerator machine in this process; the memory plane installs its \
                 own slots and has nowhere to install them"
            }
            _ => {
                "could not find exactly one accelerator machine in this process; the memory \
                 plane installs its own slots and will not guess which machine to install \
                 them in"
            }
        })
    }
}

impl SlotPlane for KvmSlotPlane {
    fn ceiling(&self) -> Result<u32, RawError> {
        self.vm.max_memslots()
    }

    fn number_space(&self) -> &Arc<SlotNumberSpace> {
        &self.space
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
        let s = KvmMemslot::install(
            Arc::clone(&self.vm),
            slot,
            gpa,
            Arc::clone(window),
            window_offset,
            len,
            guest_readonly,
        )?;
        Ok(Box::new(s))
    }
}

// =====================================================================================
// The allocator
// =====================================================================================

/// Refusal: the descending allocator reached its floor.
pub const SLOT_BUDGET_EXHAUSTED: &str = "this device's memslot budget, counted DOWN from the kernel's ceiling; going further \
     would reach numbers the hypervisor allocates upward into, and a number collision is \
     not an error — it silently REPLACES the hypervisor's own mapping";

/// Refusal: the kernel's ceiling is too small to carve a budget out of.
pub const CEILING_TOO_SMALL: &str = "the kernel's memslot ceiling is smaller than this device's budget plus the room the \
     hypervisor needs beneath it; there is no disjoint range to allocate from";

/// ★★★ Memslot numbers, allocated **downward from the kernel's ceiling**.
///
/// # Why not the C's hardcoded base
///
/// `C: nvkvm_mmap_host.c:390-435` allocates from a fixed base of 64 upward, on the stated
/// convention that *"below the base is reserved for QEMU's static regions"* — a convention
/// **enforced by nothing**. The hypervisor allocates densely from zero
/// (`qemu: kvm-all.c:250-262`, `slots[i].slot = i`), starting at 16 slots and doubling. So
/// disjointness holds by arithmetic for one device set and breaks under memory hotplug, a
/// virtio memory device, or several pass-through devices with RAM BARs — and the failure is
/// not an error. `KVM_SET_USER_MEMORY_REGION` on a number that is already live is a
/// **replace**: the hypervisor's mapping for that range goes away and ours takes its place,
/// successfully, silently.
///
/// Counting down from `KVM_CAP_NR_MEMSLOTS` inverts that: the hypervisor grows upward from
/// zero and aborts before wrapping, so a collision needs it to have consumed everything
/// below our floor first. [`OUR_SLOT_BUDGET`] is the width of that gap, and reaching the
/// floor is a **refusal**, never a wrap.
///
/// # ★★ Recycling, and the one rule that makes it safe
///
/// The C recycles through a LIFO free stack (`:404-421`) after an earlier
/// never-recycle allocator *"exhausted the pool after a few CUDA processes"* (`:382-389`).
/// So recycling is required, not optional — but a number may return to the pool **only
/// after the kernel has been told the slot is gone**, or the next install re-uses a number
/// the kernel still has a live mapping for and gets a replace instead of an add. That
/// ordering is not expressible in this type: it is enforced by
/// [`SlotAllocator::release`]'s contract and by the caller dropping the [`LiveSlot`] first.
#[derive(Debug)]
pub struct SlotAllocator {
    /// The kernel's ceiling. Slot numbers are `< ceiling`.
    ceiling: u32,
    /// ★ **The top of THIS device's window** (w641). Equals `ceiling` for the first device
    /// and the previous device's floor for each one after — see [`SlotNumberSpace`].
    ///
    /// ⊘ Kept separate from `ceiling` rather than replacing it: `ceiling` is what the KERNEL
    /// said and is what a census reports, `top` is what this device may use. Collapsing them
    /// would make `live()` and `budget()` right for device 0 and quietly wrong for every
    /// device after it — the exact shape of the bug this field exists to close.
    top: u32,
    /// The lowest number this device will ever hand out.
    floor: u32,
    /// The next fresh number, descending. Equals `top` before the first allocation.
    next: u32,
    /// Numbers whose slot has been **cleared in the kernel** and may be re-issued.
    free: Vec<u32>,
    /// Cumulative reuses — the non-vacuity witness for the free list.
    recycled: u64,
    /// ★ The window this allocator claimed from its machine's [`SlotNumberSpace`], and the
    /// space to return it to on `Drop`.
    ///
    /// ⊘ `None` for an allocator built directly (tests, [`SlotAllocator::new`]) — those never
    /// took anything from the process cursor and must not give anything back. **A release that
    /// did not correspond to a claim would hand a second device a window still in use**, which
    /// is the very collision this whole mechanism exists to prevent.
    claim: Option<(Arc<SlotNumberSpace>, u32, u32)>,
}

impl Drop for SlotAllocator {
    fn drop(&mut self) {
        // ⊘⊘ **A DEVICE THAT GOES AWAY RETURNS ITS NUMBERS (w641).** Without this the cursor
        // only ever descends, so a process that realizes devices in sequence — a test binary,
        // a guest that re-plugs a GPU — exhausts a 32 764-slot space after a handful of them
        // and the refusal blames the ceiling. ⚠ Caught by three integration tests that realize
        // several devices in one process and began failing the moment the cursor was added:
        // **the first thing the per-device window broke was the harness that could see it.**
        if let Some((space, top, budget)) = self.claim.take() {
            space.release((top, budget));
        }
    }
}

impl SlotAllocator {
    /// # Errors
    /// [`CEILING_TOO_SMALL`] if the kernel's ceiling cannot hold the budget plus a
    /// hypervisor's worth of slots beneath it.
    pub fn new(ceiling: u32) -> Result<Self, &'static str> {
        Self::with_budget(ceiling, OUR_SLOT_BUDGET)
    }

    /// ★ w393 — the allocator a real machine gets: [`OUR_SLOT_BUDGET`] plus
    /// [`MIRROR_SLOT_BUDGET`] when the ceiling can hold both with a hypervisor's worth
    /// beneath, else [`OUR_SLOT_BUDGET`] alone. The choice is reported through
    /// [`SlotAllocator::budget`], so a boot can state which one it ran with.
    ///
    /// # Errors
    /// As [`SlotAllocator::new`].
    pub fn for_machine(space: &Arc<SlotNumberSpace>, ceiling: u32) -> Result<Self, &'static str> {
        let (top, budget) = space.claim(ceiling)?;
        match Self::for_machine_below(ceiling, top) {
            Ok(mut a) => {
                a.claim = Some((Arc::clone(space), top, budget));
                Ok(a)
            }
            // ⊘ A claim whose allocator then refused must be given straight back, or the
            // refusal permanently narrows the space for everyone after it.
            Err(e) => {
                space.release((top, budget));
                Err(e)
            }
        }
    }

    /// ★★★ **The same choice, against an explicit top** — what [`SlotAllocator::for_machine`]
    /// is once the process-wide cursor has said where this device's window begins.
    ///
    /// Pure, and public for tests: a process-global cursor cannot be exercised twice in one
    /// test binary, so the arithmetic that matters is reachable without it.
    ///
    /// # Errors
    /// [`CEILING_TOO_SMALL`] if `top` cannot hold the budget plus a hypervisor's worth
    /// beneath.
    pub fn for_machine_below(ceiling: u32, top: u32) -> Result<Self, &'static str> {
        let wide = OUR_SLOT_BUDGET.saturating_add(MIRROR_SLOT_BUDGET);
        if top >= wide.saturating_mul(2) {
            Self::with_budget_below(ceiling, top, wide)
        } else {
            Self::with_budget_below(ceiling, top, OUR_SLOT_BUDGET)
        }
    }

    fn with_budget(ceiling: u32, budget: u32) -> Result<Self, &'static str> {
        Self::with_budget_below(ceiling, ceiling, budget)
    }

    fn with_budget_below(ceiling: u32, top: u32, budget: u32) -> Result<Self, &'static str> {
        // The hypervisor starts at 16 slots and doubles (`qemu: kvm-all.c:250-262`), so a
        // top that leaves it fewer than the budget itself beneath us is one where the
        // two ranges are not credibly disjoint. Stated as arithmetic rather than as a
        // magic minimum.
        if top < budget.saturating_mul(2) || top > ceiling {
            return Err(CEILING_TOO_SMALL);
        }
        Ok(SlotAllocator {
            ceiling,
            top,
            floor: top - budget,
            next: top,
            free: Vec::new(),
            recycled: 0,
            claim: None,
        })
    }

    /// How many numbers this allocator may hold at once — `ceiling - floor`.
    #[must_use]
    pub fn budget(&self) -> u32 {
        self.top - self.floor
    }

    /// The top of this device's window — `ceiling` for the first device, the previous
    /// device's floor for each one after.
    #[must_use]
    pub fn top(&self) -> u32 {
        self.top
    }

    /// The kernel's ceiling this allocator was built from.
    #[must_use]
    pub fn ceiling(&self) -> u32 {
        self.ceiling
    }

    /// The lowest number this allocator will hand out.
    #[must_use]
    pub fn floor(&self) -> u32 {
        self.floor
    }

    /// How many numbers have been re-issued from the free list.
    #[must_use]
    pub fn recycled(&self) -> u64 {
        self.recycled
    }

    /// How many numbers are currently handed out.
    #[must_use]
    pub fn live(&self) -> u32 {
        // ⊘ `top`, not `ceiling` (w641): for the second device those differ, and using the
        // kernel's ceiling here would report every number between the two windows as live.
        (self.top - self.next) - u32::try_from(self.free.len()).unwrap_or(u32::MAX)
    }

    /// Take `n` numbers, or refuse **without taking any**.
    ///
    /// All-or-nothing because the caller installs a window's spans as a unit: handing out
    /// two of three and refusing the third would leave the caller to unwind numbers it had
    /// not recorded yet.
    ///
    /// # Errors
    /// [`SLOT_BUDGET_EXHAUSTED`].
    pub fn alloc(&mut self, n: usize) -> Result<Vec<u32>, &'static str> {
        let fresh_available = (self.next - self.floor) as usize;
        if n > self.free.len() + fresh_available {
            return Err(SLOT_BUDGET_EXHAUSTED);
        }
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            if let Some(s) = self.free.pop() {
                self.recycled += 1;
                out.push(s);
            } else {
                self.next -= 1;
                out.push(self.next);
            }
        }
        Ok(out)
    }

    /// Return a number to the pool.
    ///
    /// # ★★★ CONTRACT
    ///
    /// **The kernel must already have been told the slot is gone.** Call this only after
    /// the [`LiveSlot`] holding `slot` has been dropped — its `Drop` is what issues the
    /// clearing ioctl, and it asserts that the ioctl succeeded. Re-issuing a number the
    /// kernel still has a live mapping for turns the next install from an ADD into a
    /// REPLACE, which does not fail.
    ///
    /// # Panics
    /// If `slot` was never handed out by this allocator, or is already free. Both are
    /// bookkeeping bugs that would otherwise show up as a double-installed slot much later.
    pub fn release(&mut self, slot: u32) {
        assert!(
            slot >= self.next && slot < self.ceiling,
            "slot {slot} was never handed out by this allocator (live range \
             {next}..{ceiling})",
            next = self.next,
            ceiling = self.ceiling,
        );
        assert!(
            !self.free.contains(&slot),
            "slot {slot} is already free; releasing it twice would hand the same number to \
             two live windows"
        );
        self.free.push(slot);
    }
}

// =====================================================================================
// The span algebra
// =====================================================================================

/// One contiguous piece of a window, and what the kernel is told about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    /// Byte offset within the window.
    pub offset: u64,
    /// Length in bytes.
    pub len: u64,
    /// Which tier this piece is.
    pub tier: Tier,
}

/// What a piece of a window is, physically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    /// An ordinary read-write memslot. Reads and writes are both native.
    Passthrough,
    /// A memslot with the kernel's read-only flag: reads native, writes exit.
    ReadNative,
    /// **No memslot.** Every access exits.
    Observe,
}

impl Tier {
    /// Whether this tier installs a slot at all, and with which polarity.
    #[must_use]
    pub fn readonly_slot(self) -> Option<bool> {
        match self {
            Tier::Passthrough => Some(false),
            Tier::ReadNative => Some(true),
            Tier::Observe => None,
        }
    }
}

/// Refusal: a tier sub-range that is not inside the window it re-tiers.
pub const TIER_OUTSIDE_ITS_WINDOW: &str =
    "a tier sub-range that is not inside the window it applies to";

/// Refusal: two tier sub-ranges of the same window overlap after page rounding.
pub const TIERS_OVERLAP: &str = "two tier sub-ranges of one window overlap once rounded out to whole host pages; a page \
     cannot be two tiers at once, and rounding is what makes a request that looked disjoint \
     stop being so";

/// ★★ Cut a window of `len` bytes into tiered spans.
///
/// `cuts` are `(offset, len, tier)` triples **already rounded to whole host pages** by the
/// caller — rounding lives with the page size, not here, so this function is pure and
/// testable at every page size. Everything not named by a cut is [`Tier::Passthrough`].
///
/// # Errors
/// [`TIER_OUTSIDE_ITS_WINDOW`], [`TIERS_OVERLAP`].
pub fn spans(len: u64, cuts: &[(u64, u64, Tier)]) -> Result<Vec<Span>, &'static str> {
    let mut sorted: Vec<(u64, u64, Tier)> = Vec::with_capacity(cuts.len());
    for &(off, l, tier) in cuts {
        if l == 0 || off.checked_add(l).is_none_or(|e| e > len) {
            return Err(TIER_OUTSIDE_ITS_WINDOW);
        }
        sorted.push((off, l, tier));
    }
    sorted.sort_unstable_by_key(|c| c.0);
    for w in sorted.windows(2) {
        if w[0].0 + w[0].1 > w[1].0 {
            return Err(TIERS_OVERLAP);
        }
    }
    let mut out = Vec::new();
    let mut at = 0u64;
    for (off, l, tier) in sorted {
        if off > at {
            out.push(Span {
                offset: at,
                len: off - at,
                tier: Tier::Passthrough,
            });
        }
        out.push(Span {
            offset: off,
            len: l,
            tier,
        });
        at = off + l;
    }
    if at < len {
        out.push(Span {
            offset: at,
            len: len - at,
            tier: Tier::Passthrough,
        });
    }
    // A window with no cuts at all is one passthrough span, never an empty list: an empty
    // list would install nothing and the guest would see an unbacked range that every layer
    // above still believes is a window.
    if out.is_empty() {
        out.push(Span {
            offset: 0,
            len,
            tier: Tier::Passthrough,
        });
    }
    Ok(out)
}

/// ★★★★★ **THE KERNEL'S SLOT NUMBERS ARE ONE SPACE, AND EVERY EMULATED GPU DRAWS FROM IT.**
///
/// **Owner, 2026-09-13:** *"there is a doorbell page per gpu, each gpu has its own bar0/1/2 we
/// need to emulate in kayfabe … ensure this remains possible."* This is the seam where that was
/// **not** possible, and the failure was silent.
///
/// # ⊘⊘⊘ What went wrong, precisely
///
/// [`SlotAllocator`] is minted **per device instance** and every instance set `next = ceiling`.
/// Two emulated GPUs in one QEMU process therefore handed out the **same numbers** — 32763,
/// 32762, … — from two allocators that each stayed politely inside their own budget. And
/// `KVM_SET_USER_MEMORY_REGION` on a live number is not an error, it is a **replace**: GPU 1's
/// BAR1-mirror slot would evict GPU 0's mapping at a different GPA, GPU 0's guest would start
/// exiting on an aperture it believes is mapped, and GPU 0's `LiveSlot::drop` would later clear
/// a slot the kernel had reassigned to GPU 1. `SLOT_BUDGET_EXHAUSTED` never fires, because
/// **neither allocator has done anything wrong by its own accounting.**
///
/// ⚠ The module doc above spends twenty lines on exactly this hazard — *"a number collision is
/// not an error — it silently REPLACES the hypervisor's own mapping"* — and argues it against
/// QEMU, against memory hotplug, and against *"several pass-through devices with RAM BARs"*.
/// It never argued it against **a second instance of ourselves**. ★ The analysis was right and
/// its quantifier was one case short.
///
/// # The rule this encodes
///
/// A process-global `static` is the **wrong** home for per-device state — that is the defect
/// class that also produced `MIRROR_FOR_BIRTH` and `SHADOW_SINK`. It is the **right** home for
/// a genuinely process-wide kernel resource, and the KVM slot number space is one: one machine,
/// one slot table, shared by every device in the process. ⇒ The test is not *"is it a static"*
/// but **"is the thing it names process-wide?"** Here it is, so the cursor lives here and each
/// device claims a disjoint window below the previous one.
#[derive(Debug)]
pub struct SlotNumberSpace {
    /// The frontier and the returned windows, for ONE machine.
    state: Mutex<SpaceState>,
}

#[derive(Debug, Default)]
struct SpaceState {
    /// The lowest number claimed so far; the next device's window ends here. `None` means
    /// nothing has been claimed and the first claimant starts at the kernel's ceiling.
    frontier: Option<u32>,
    /// Windows returned by devices that have gone away, as `(top, budget)`. Reused before the
    /// frontier descends, so a machine that realizes devices in sequence does not exhaust a
    /// space it is barely using.
    free: Vec<(u32, u32)>,
}

impl SlotNumberSpace {
    /// A fresh space for one machine.
    #[must_use]
    pub fn new() -> Self {
        SlotNumberSpace {
            state: Mutex::new(SpaceState::default()),
        }
    }

    /// Claim the next device's window as `(top, budget)`, given the kernel's ceiling.
    ///
    /// The first caller gets `ceiling`; each later caller gets the previous claimant's floor,
    /// so the windows are disjoint by construction rather than by each device's good behaviour.
    ///
    /// # Errors
    /// [`CEILING_TOO_SMALL`] when the space left beneath would not hold another device's
    /// minimum budget plus a hypervisor's worth. ⊘ **A refusal, never a wrap** — the whole
    /// point is that the Nth device is told it cannot fit instead of quietly taking the
    /// (N−1)th's numbers.
    pub fn claim(&self, ceiling: u32) -> Result<(u32, u32), &'static str> {
        let mut g = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // ★ A returned window first — see [`SlotAllocator`]'s `Drop`. Reuse keeps a machine
        // that realizes devices in sequence from walking the frontier down for no reason.
        if let Some(w) = g.free.pop() {
            return Ok(w);
        }
        let top = g.frontier.unwrap_or(ceiling);
        // Every window must still leave a hypervisor's worth beneath it, so the floor of the
        // narrowest acceptable window is what the next claimant would need.
        if top < OUR_SLOT_BUDGET.saturating_mul(2) {
            return Err(CEILING_TOO_SMALL);
        }
        let wide = OUR_SLOT_BUDGET.saturating_add(MIRROR_SLOT_BUDGET);
        let budget = if top >= wide.saturating_mul(2) {
            wide
        } else {
            OUR_SLOT_BUDGET
        };
        g.frontier = Some(top - budget);
        Ok((top, budget))
    }

    /// Give a window back. Called from [`SlotAllocator`]'s `Drop`, never by hand.
    fn release(&self, claim: (u32, u32)) {
        let mut g = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        g.free.push(claim);
    }

    /// The next window's top, without claiming it. For census lines and tests.
    #[must_use]
    pub fn peek(&self, ceiling: u32) -> u32 {
        let g = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        g.free
            .last()
            .map_or_else(|| g.frontier.unwrap_or(ceiling), |&(t, _)| t)
    }
}

impl Default for SlotNumberSpace {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ★★★ **Down, not up** — and the numbers are exactly the top of the kernel's range.
    #[test]
    fn numbers_descend_from_the_kernels_ceiling_and_never_reach_the_hypervisors_end() {
        let mut a = SlotAllocator::new(512).expect("512 is a real ceiling");
        assert_eq!(a.floor(), 512 - OUR_SLOT_BUDGET);
        assert_eq!(
            a.alloc(3).expect("three fresh numbers"),
            vec![511, 510, 509],
            "the first numbers handed out must be the HIGHEST the kernel allows; the C's \
             base-64-upward convention is enforced by nothing and the hypervisor allocates \
             densely from zero"
        );
        // Everything this allocator will ever hand out is above the floor, which is the
        // whole disjointness argument.
        let rest = a.alloc((OUR_SLOT_BUDGET - 3) as usize).expect("the rest");
        assert!(
            rest.iter()
                .chain([511, 510, 509].iter())
                .all(|&s| (512 - OUR_SLOT_BUDGET..512).contains(&s)),
            "every number must lie in [ceiling - budget, ceiling)"
        );
    }

    /// ★★ The floor is a **refusal**, not a wrap — and it refuses without consuming.
    #[test]
    fn reaching_the_floor_refuses_by_name_and_takes_nothing() {
        let mut a = SlotAllocator::new(256).expect("ceiling");
        let all = a.alloc(OUR_SLOT_BUDGET as usize).expect("the whole budget");
        assert_eq!(all.len(), OUR_SLOT_BUDGET as usize);
        assert_eq!(a.alloc(1), Err(SLOT_BUDGET_EXHAUSTED));
        // ...and the refusal really took nothing: releasing everything and re-allocating
        // the full budget must still work. A refusal that had decremented `next` would
        // leave the allocator one short forever.
        for s in all {
            a.release(s);
        }
        assert_eq!(
            a.alloc(OUR_SLOT_BUDGET as usize).map(|v| v.len()),
            Ok(OUR_SLOT_BUDGET as usize),
            "an all-or-nothing refusal must not have consumed a number on its way out"
        );
    }

    /// ★★ An all-or-nothing request that cannot be met in full takes **none**.
    #[test]
    fn a_partial_request_is_refused_whole() {
        let mut a = SlotAllocator::new(200).expect("ceiling");
        let held = a.alloc(OUR_SLOT_BUDGET as usize - 1).expect("all but one");
        assert_eq!(
            a.alloc(2),
            Err(SLOT_BUDGET_EXHAUSTED),
            "one number is available and two were asked for"
        );
        assert_eq!(
            a.alloc(1).map(|v| v.len()),
            Ok(1),
            "the one that WAS available must still be there — a refusal that had handed \
             out the first of the two would have lost it"
        );
        drop(held);
    }

    /// ★★ Recycling works, and the free list is really used (the C's own fix, `:382-389`).
    #[test]
    fn a_released_number_is_reissued_rather_than_descending_further() {
        let mut a = SlotAllocator::new(512).expect("ceiling");
        let first = a.alloc(4).expect("four");
        assert_eq!(a.recycled(), 0, "nothing has been recycled yet");
        for s in &first {
            a.release(*s);
        }
        let again = a.alloc(4).expect("four again");
        assert_eq!(
            a.recycled(),
            4,
            "all four must have come from the free list"
        );
        let mut a_sorted = first.clone();
        let mut b_sorted = again.clone();
        a_sorted.sort_unstable();
        b_sorted.sort_unstable();
        assert_eq!(
            a_sorted, b_sorted,
            "the same four numbers must come back; descending further instead would burn \
             the budget at teardown frequency, which is what the C's first allocator did"
        );
        assert_eq!(a.live(), 4);
    }

    /// ★ A ceiling too small to carve a budget from is a refusal at construction.
    #[test]
    fn a_ceiling_that_cannot_hold_the_budget_is_refused_at_construction() {
        for ceiling in [0, 1, 16, OUR_SLOT_BUDGET, OUR_SLOT_BUDGET * 2 - 1] {
            assert_eq!(
                SlotAllocator::new(ceiling).map(|_| ()),
                Err(CEILING_TOO_SMALL),
                "ceiling {ceiling} leaves no disjoint range"
            );
        }
        assert!(
            SlotAllocator::new(OUR_SLOT_BUDGET * 2).is_ok(),
            "and the first ceiling that DOES fit must be accepted, or the bound above is \
             refusing everything"
        );
    }

    /// ★★ Releasing a number twice is a panic, not a silently double-issued slot.
    #[test]
    #[should_panic(expected = "is already free")]
    fn releasing_the_same_number_twice_is_loud() {
        let mut a = SlotAllocator::new(512).expect("ceiling");
        let s = a.alloc(1).expect("one")[0];
        a.release(s);
        a.release(s);
    }

    /// ★ And a number this allocator never handed out.
    #[test]
    #[should_panic(expected = "was never handed out")]
    fn releasing_a_foreign_number_is_loud() {
        let mut a = SlotAllocator::new(512).expect("ceiling");
        a.release(7);
    }

    /// ★★★ The span algebra: an untiered window is **one** passthrough span covering
    /// everything, and a cut list produces exactly the alternation the kernel is told.
    #[test]
    fn a_window_with_no_cuts_is_one_passthrough_span_over_the_whole_thing() {
        assert_eq!(
            spans(0x4000, &[]).expect("no cuts"),
            vec![Span {
                offset: 0,
                len: 0x4000,
                tier: Tier::Passthrough
            }],
            "an empty span list would install NOTHING and leave every layer above believing \
             the window is backed"
        );
    }

    /// ★★ Every cut is surrounded by passthrough, and the pieces tile the window exactly.
    #[test]
    fn tiered_cuts_tile_the_window_with_no_gap_and_no_overlap() {
        /// `(window length, the cuts applied to it)` — named so the sweep below is not a
        /// "very complex type" the reader has to re-derive either.
        type Case = (u64, Vec<(u64, u64, Tier)>);
        let cases: Vec<Case> = vec![
            (0x4000, vec![(0x1000, 0x1000, Tier::ReadNative)]),
            (0x4000, vec![(0, 0x1000, Tier::Observe)]),
            (0x4000, vec![(0x3000, 0x1000, Tier::ReadNative)]),
            (0x4000, vec![(0, 0x4000, Tier::Observe)]),
            (
                0x8000,
                vec![
                    (0x1000, 0x1000, Tier::ReadNative),
                    (0x5000, 0x2000, Tier::Observe),
                ],
            ),
            // deliberately out of order: the algebra sorts, the caller does not have to
            (
                0x8000,
                vec![
                    (0x5000, 0x2000, Tier::Observe),
                    (0x1000, 0x1000, Tier::ReadNative),
                ],
            ),
        ];
        for (len, cuts) in cases {
            let s = spans(len, &cuts).unwrap_or_else(|e| panic!("{cuts:?}: {e}"));
            let mut at = 0;
            for sp in &s {
                assert_eq!(sp.offset, at, "a gap or an overlap in {s:?}");
                assert!(sp.len > 0, "a zero-length span in {s:?}");
                at += sp.len;
            }
            assert_eq!(at, len, "the spans must tile the WHOLE window: {s:?}");
            for (off, l, tier) in &cuts {
                assert!(
                    s.iter()
                        .any(|sp| sp.offset == *off && sp.len == *l && sp.tier == *tier),
                    "the cut {off:#x}+{l:#x} {tier:?} must survive as its own span: {s:?}"
                );
            }
        }
    }

    /// ★★ Overlapping cuts are refused by name — a page cannot be two tiers at once.
    #[test]
    fn overlapping_cuts_are_refused_and_so_is_a_cut_that_leaves_the_window() {
        assert_eq!(
            spans(
                0x4000,
                &[
                    (0x1000, 0x2000, Tier::ReadNative),
                    (0x2000, 0x1000, Tier::Observe)
                ]
            ),
            Err(TIERS_OVERLAP)
        );
        for bad in [
            (0x4000_u64, 0x1000_u64),
            (0x3800, 0x1000),
            (0, 0x4001),
            (0x1000, 0),
            (u64::MAX, 0x1000),
        ] {
            assert_eq!(
                spans(0x4000, &[(bad.0, bad.1, Tier::ReadNative)]),
                Err(TIER_OUTSIDE_ITS_WINDOW),
                "a cut at {bad:x?} is not inside a 0x4000-byte window"
            );
        }
    }

    /// ★ The polarity table, asserted apart. `Observe` installing a slot at all — of either
    /// polarity — would make "every access exits" quietly false.
    #[test]
    fn only_observe_installs_no_slot_and_only_read_native_installs_a_read_only_one() {
        assert_eq!(Tier::Passthrough.readonly_slot(), Some(false));
        assert_eq!(Tier::ReadNative.readonly_slot(), Some(true));
        assert_eq!(Tier::Observe.readonly_slot(), None);
    }
}

#[cfg(test)]
mod two_gpus_draw_from_one_number_space {
    //! ★★★★★ **THE OWNER'S PER-GPU REQUIREMENT, 2026-09-13** — *"each gpu has its own
    //! bar0/1/2 we need to emulate in kayfabe … ensure this remains possible."*
    //!
    //! ⊘ These test the **arithmetic**, through [`SlotAllocator::for_machine_below`] and a
    //! local [`SlotNumberSpace`]. A process-global cursor cannot be exercised twice in one
    //! test binary, and a test that could only run first is a test that stops running.
    use super::*;

    /// A real kernel's ceiling (`KVM_CAP_NR_MEMSLOTS` on a modern host).
    const CEILING: u32 = 32764;

    fn fresh() -> SlotNumberSpace {
        SlotNumberSpace::new()
    }

    #[test]
    fn two_devices_get_windows_that_do_not_overlap() {
        // ⊘⊘ **The defect this exists for:** before w641 both devices set `next = ceiling` and
        // handed out the SAME numbers, and `KVM_SET_USER_MEMORY_REGION` on a live number is a
        // **replace**, not an error — so GPU 1 would silently evict GPU 0's mapping and
        // neither allocator's budget check would fire.
        let space = fresh();
        let a = SlotAllocator::for_machine_below(CEILING, space.claim(CEILING).unwrap().0)
            .expect("device 0 fits");
        let b = SlotAllocator::for_machine_below(CEILING, space.claim(CEILING).unwrap().0)
            .expect("device 1 fits");

        assert!(
            b.top() <= a.floor(),
            "device 1's window {:?} overlaps device 0's {:?}",
            (b.floor(), b.top()),
            (a.floor(), a.top()),
        );
    }

    #[test]
    fn the_numbers_two_devices_actually_hand_out_are_disjoint() {
        // The window arithmetic is one claim; what the allocators DO is another. Take real
        // numbers from both and assert the sets never meet — the property a replace violates.
        let space = fresh();
        let mut a = SlotAllocator::for_machine_below(CEILING, space.claim(CEILING).unwrap().0)
            .expect("device 0");
        let mut b = SlotAllocator::for_machine_below(CEILING, space.claim(CEILING).unwrap().0)
            .expect("device 1");

        let mine: Vec<u32> = (0..64)
            .map(|_| a.alloc(1).expect("device 0 number")[0])
            .collect();
        let theirs: Vec<u32> = (0..64)
            .map(|_| b.alloc(1).expect("device 1 number")[0])
            .collect();

        for n in &theirs {
            assert!(!mine.contains(n), "slot {n} handed to BOTH devices");
        }
    }

    #[test]
    fn a_device_that_does_not_fit_is_refused_rather_than_wrapped() {
        // ⊘ **A refusal, never a wrap.** The Nth device must be told it cannot fit; taking the
        // (N−1)th's numbers would succeed and corrupt, which is the whole failure mode.
        let space = fresh();
        let mut claims = 0u32;
        loop {
            match space.claim(CEILING) {
                Ok(_) => {
                    claims += 1;
                    assert!(claims < 10_000, "the cursor never refuses — it is wrapping");
                }
                Err(e) => {
                    assert_eq!(e, CEILING_TOO_SMALL);
                    break;
                }
            }
        }
        assert!(
            claims >= 2,
            "a real ceiling must fit at least two devices, fit {claims}"
        );
    }

    #[test]
    fn the_first_device_still_starts_at_the_kernels_ceiling() {
        // ⊘ The regression guard for the ORIGINAL property: counting down from
        // `KVM_CAP_NR_MEMSLOTS` is what keeps us clear of the hypervisor, which allocates
        // densely upward from zero. Carving per-device windows must not move device 0 down.
        let space = fresh();
        assert_eq!(space.peek(CEILING), CEILING);
        let a = SlotAllocator::for_machine_below(CEILING, space.claim(CEILING).unwrap().0)
            .expect("device 0");
        assert_eq!(a.top(), CEILING);
    }
}
