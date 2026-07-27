//! vCPUs, `KVM_RUN`, and the MMIO exit — `l1_os_shell.md` §10 stage **M2-d**.
//!
//! M2-c said what it was not building, and why (`kvm_unsafe`'s module docs): *"There are
//! no vCPUs… this stage is the memory plane, and a vCPU would add a second unbuilt
//! subsystem to the diff."* It also recorded the two things the harness therefore could
//! not observe (§14.8 F7): **where** a memslot points, and the **order** of a teardown.
//! This module exists to make both observable, and it is the smallest thing that does:
//!
//! - a vCPU,
//! - a flat 32-bit protected-mode entry set straight through `KVM_SET_SREGS` (no GDT in
//!   guest memory, no real-mode trampoline — the segment caches are ours to program),
//! - `KVM_RUN`, and
//! - the MMIO exit, both directions.
//!
//! ## ★ Why a vCPU is the instrument, not a feature
//!
//! Without one, a memslot is a promise nobody checks. `KVM_SET_USER_MEMORY_REGION`
//! validates alignment and overlap at install time and **nothing else** — so an adapter
//! that computed a sub-range offset wrongly, or that deleted a memslot before it stopped
//! telling callers the range was RAM, passes every test that does not have a guest in it.
//! With a vCPU, the guest reads a value **we** placed at a guest-physical address and
//! writes it back out through a trapped register: the value is the proof that the memslot
//! points where the arithmetic said, and the *exit* is the proof that the range stopped
//! being memory when we said it did.
//!
//! ## Architecture
//!
//! The vCPU **plumbing** (create, map the shared run structure, `KVM_RUN`, read the exit)
//! is architecture-neutral: the exit union's MMIO arm has the same layout everywhere.
//! Entering the guest in a known state is not — segment caches, control registers and the
//! very notion of "protected mode" are x86. So [`KvmVcpu::enter_flat_protected_mode`] is
//! `x86_64`-only *and says so at runtime on any other host*, rather than being `cfg`'d out
//! of existence: an arm64 build compiles (the §9.3 cross-check), and an arm64 host gets a
//! loud [`RawError::Unsupported`] instead of a vCPU in an undefined state.
//!
//! ## Numbers
//!
//! `_IO`/`_IOR`/`_IOW` encodings of `KVMIO` (`0xAE`) from the kernel's
//! `include/uapi/linux/kvm.h`, spelled out with their encodings for the same reason
//! `kvm_unsafe` spells its own out: a magic constant in a diff is unreviewable.

use crate::error::{RawError, last_syscall_error};
use crate::host_fd_unsafe::adopt_fd;
use crate::kvm_unsafe::{Kvm, KvmVm, ioctl_arg};
use kayfabe_util::{leafwitness, lockwitness};
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::Arc;

// --- ioctl numbers -------------------------------------------------------------------

/// `_IO(KVMIO, 0x04)` — the size of the per-vCPU shared run structure.
const KVM_GET_VCPU_MMAP_SIZE: libc::c_ulong = 0xAE04;
/// `_IO(KVMIO, 0x41)` — create a vCPU, returning its descriptor.
const KVM_CREATE_VCPU: libc::c_ulong = 0xAE41;
/// `_IO(KVMIO, 0x47)` — where the hypervisor may put its 3-page task-state segment.
const KVM_SET_TSS_ADDR: libc::c_ulong = 0xAE47;
/// `_IO(KVMIO, 0x80)` — enter the guest.
const KVM_RUN: libc::c_ulong = 0xAE80;
/// `_IOW(KVMIO, 0x82, struct kvm_regs)` — 144 bytes (18 × `__u64`).
const KVM_SET_REGS: libc::c_ulong = (1 << 30) | ((144 as libc::c_ulong) << 16) | 0xAE82;
/// `_IOR(KVMIO, 0x83, struct kvm_sregs)` — 312 bytes.
const KVM_GET_SREGS: libc::c_ulong = (2 << 30) | ((312 as libc::c_ulong) << 16) | 0xAE83;
/// `_IOW(KVMIO, 0x84, struct kvm_sregs)` — 312 bytes.
const KVM_SET_SREGS: libc::c_ulong = (1 << 30) | ((312 as libc::c_ulong) << 16) | 0xAE84;

/// `KVM_CAP_SET_TSS_ADDR` — present on Intel hosts, absent on AMD.
const KVM_CAP_SET_TSS_ADDR: libc::c_ulong = 4;

/// Where the hypervisor's own task-state segment goes: three pages of guest-physical
/// space below 4 GiB that no memslot covers. The value is the one every userspace VMM
/// uses, and the requirement is only that we never map anything there.
const TSS_ADDR: libc::c_ulong = 0xfffb_d000;

/// `KVM_EXIT_MMIO`.
const EXIT_MMIO: u32 = 6;
/// `KVM_EXIT_HLT`.
const EXIT_HLT: u32 = 5;
/// `KVM_EXIT_IO`.
const EXIT_IO: u32 = 2;
/// `KVM_EXIT_SHUTDOWN` — a triple fault, i.e. the guest wedged.
const EXIT_SHUTDOWN: u32 = 8;
/// `KVM_EXIT_FAIL_ENTRY`.
const EXIT_FAIL_ENTRY: u32 = 9;
/// `KVM_EXIT_INTR` — a signal arrived; the run is simply short.
const EXIT_INTR: u32 = 10;
/// `KVM_EXIT_INTERNAL_ERROR` — KVM could not emulate what the guest did, which for us
/// means the guest reached an address it cannot execute from.
const EXIT_INTERNAL_ERROR: u32 = 17;

/// `CR0.PE` — protected mode. Paging stays **off**: a flat 32-bit unpaged guest needs no
/// page tables in guest memory, so guest-physical is guest-linear and every address in a
/// test is the address the memslot was installed at.
const CR0_PE: u64 = 1;

/// The head of `struct kvm_run`, up to (and excluding) the exit union.
///
/// `__u8 request_interrupt_window; __u8 immediate_exit; __u8 padding1[6]; __u32
/// exit_reason; __u8 ready_for_interrupt_injection; __u8 if_flag; __u16 flags; __u64 cr8;
/// __u64 apic_base;` — 32 bytes, and the union that follows is 8-aligned, so
/// [`EXIT_UNION_OFFSET`] is exactly this struct's size.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct KvmRunHead {
    request_interrupt_window: u8,
    immediate_exit: u8,
    padding1: [u8; 6],
    exit_reason: u32,
    ready_for_interrupt_injection: u8,
    if_flag: u8,
    flags: u16,
    cr8: u64,
    apic_base: u64,
}

/// Byte offset of the exit union inside `struct kvm_run`.
const EXIT_UNION_OFFSET: usize = core::mem::size_of::<KvmRunHead>();

/// The `KVM_EXIT_MMIO` arm of the exit union: `__u64 phys_addr; __u8 data[8]; __u32 len;
/// __u8 is_write;`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct KvmMmioExit {
    phys_addr: u64,
    data: [u8; 8],
    len: u32,
    is_write: u8,
}

/// `struct kvm_regs` — the 16 general-purpose registers plus `rip` and `rflags`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct KvmRegs {
    rax: u64,
    rbx: u64,
    rcx: u64,
    rdx: u64,
    rsi: u64,
    rdi: u64,
    rsp: u64,
    rbp: u64,
    r8: u64,
    r9: u64,
    r10: u64,
    r11: u64,
    r12: u64,
    r13: u64,
    r14: u64,
    r15: u64,
    rip: u64,
    rflags: u64,
}

/// `struct kvm_segment` — a segment register *and its cached descriptor*, which is what
/// lets a VMM put a guest straight into protected mode with no GDT anywhere in memory.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct KvmSegment {
    base: u64,
    limit: u32,
    selector: u16,
    kind: u8,
    present: u8,
    dpl: u8,
    db: u8,
    s: u8,
    l: u8,
    g: u8,
    avl: u8,
    unusable: u8,
    padding: u8,
}

/// `struct kvm_dtable`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct KvmDtable {
    base: u64,
    limit: u16,
    padding: [u16; 3],
}

/// `struct kvm_sregs` — 312 bytes, as the ioctl encoding above asserts.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct KvmSregs {
    cs: KvmSegment,
    ds: KvmSegment,
    es: KvmSegment,
    fs: KvmSegment,
    gs: KvmSegment,
    ss: KvmSegment,
    tr: KvmSegment,
    ldt: KvmSegment,
    gdt: KvmDtable,
    idt: KvmDtable,
    cr0: u64,
    cr2: u64,
    cr3: u64,
    cr4: u64,
    cr8: u64,
    efer: u64,
    apic_base: u64,
    interrupt_bitmap: [u64; 4],
}

/// Why a [`KvmVcpu::run`] came back.
///
/// Deliberately **total over what a guest can do to us**, with no `Other(u32)` catch-all:
/// every reason this harness can provoke has a named arm, and a reason it cannot provoke
/// is [`VcpuExit::Unhandled`] carrying the raw number. MISS = FAULT applied to the exit
/// union — a silently-ignored exit reason is a guest that stopped making progress for a
/// reason nobody wrote down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VcpuExit {
    /// The guest touched guest-physical memory that **no memslot covers**. On KVM that is
    /// the entire definition of a device access, which is why `kayfabe_vmm::RegionKind`'s
    /// two variants are physical truths in this backend rather than declarations.
    Mmio {
        /// Guest-physical address of the access.
        gpa: u64,
        /// Access width in bytes (1, 2, 4 or 8).
        len: u8,
        /// True for a store, false for a load.
        is_write: bool,
        /// For a store, the bytes the guest wrote (low `len` bytes are meaningful).
        data: [u8; 8],
    },
    /// A port access. Not used by this harness's guests; named so it cannot be silently
    /// mistaken for an MMIO exit.
    PortIo,
    /// The guest halted.
    Halted,
    /// A signal cut the run short. Nothing happened; go round again.
    Interrupted,
    /// The guest triple-faulted.
    Shutdown,
    /// The hardware refused to enter the guest.
    FailedEntry,
    /// KVM could not emulate what the guest did — for this harness, almost always an
    /// instruction fetch from a guest-physical address with no memslot behind it.
    InternalError,
    /// An exit reason this harness does not provoke, carried rather than swallowed.
    Unhandled {
        /// The raw `exit_reason`.
        reason: u32,
    },
}

/// One vCPU: a descriptor, and the run structure it shares with the kernel.
///
/// **Not `Sync`, and deliberately not `Send`-by-accident either**: a vCPU may be entered
/// from exactly one thread, and the shared run structure is written by the kernel on
/// *that* thread's behalf. It is `Send` (a runner thread owns one) and never `Sync`, which
/// the compiler gives us for free from the raw pointer inside — stated here because the
/// absence of an `unsafe impl` is the mechanism.
#[derive(Debug)]
pub struct KvmVcpu {
    /// Kept so the VM descriptor cannot close underneath a live vCPU.
    _vm: Arc<KvmVm>,
    fd: OwnedFd,
    run: RunMapping,
}

/// The `mmap`ed `struct kvm_run`, owned so it is unmapped exactly once.
#[derive(Debug)]
struct RunMapping {
    base: *mut u8,
    len: usize,
}

impl Drop for RunMapping {
    fn drop(&mut self) {
        // SAFETY: `base`/`len` are the exact return value and length of the single `mmap`
        // in `KvmVcpu::create` below — the only constructor of this type — and this is the
        // only place either is consumed. `RunMapping` is not `Clone`, so no second value
        // can name the same mapping and no double-unmap is expressible.
        unsafe { libc::munmap(self.base.cast::<libc::c_void>(), self.len) };
    }
}

impl KvmVcpu {
    /// Create vCPU number `id` on `vm`.
    ///
    /// `kvm` is needed as well as `vm` because the size of the shared run structure is a
    /// property of the **subsystem**, not of the VM (`KVM_GET_VCPU_MMAP_SIZE` is a
    /// `/dev/kvm` ioctl).
    ///
    /// # Errors
    /// [`RawError::Syscall`] if the kernel refuses the vCPU or its mapping;
    /// [`RawError::Unsupported`] if the run structure is smaller than the fields this
    /// module reads — which would mean the uapi layout is not the one written above.
    ///
    /// # Panics
    /// If called with any ranked lock held (R1, §4.5).
    pub fn create(kvm: &Kvm, vm: Arc<KvmVm>, id: u32) -> Result<Self, RawError> {
        lockwitness::assert_lock_free("KVM_CREATE_VCPU");
        leafwitness::assert_leaf_free("KVM_CREATE_VCPU");
        let run_size = ioctl_arg(
            kvm.as_raw(),
            KVM_GET_VCPU_MMAP_SIZE,
            0,
            "KVM_GET_VCPU_MMAP_SIZE",
        )?;
        let run_size = usize::try_from(run_size).unwrap_or(0);
        if run_size < EXIT_UNION_OFFSET + core::mem::size_of::<KvmMmioExit>() {
            return Err(RawError::Unsupported {
                what: "the kernel's per-vCPU run structure size",
                detail: "smaller than the head plus the MMIO exit arm this module reads; \
                         the uapi layout is not the one these structs are written for",
            });
        }
        let raw = ioctl_arg(
            vm.as_raw(),
            KVM_CREATE_VCPU,
            libc::c_ulong::from(id),
            "KVM_CREATE_VCPU",
        )?;
        let fd = adopt_fd(raw, "KVM_CREATE_VCPU")?;

        // SAFETY: a `PROT_READ|PROT_WRITE`, `MAP_SHARED` mapping of exactly `run_size`
        // bytes at offset 0 of a live vCPU descriptor — the length comes from the kernel
        // itself two statements above, and `run_size` was checked to be at least as large
        // as every field this module reads. The address argument is null, so the kernel
        // chooses the placement and nothing already mapped in this process can be
        // replaced (which is what `MAP_FIXED` would risk — §4.4). The result is checked
        // against `MAP_FAILED` below before it is stored.
        let base = unsafe {
            libc::mmap(
                core::ptr::null_mut(),
                run_size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd.as_raw_fd(),
                0,
            )
        };
        if base == libc::MAP_FAILED {
            return Err(last_syscall_error("mmap(kvm_run)"));
        }
        Ok(KvmVcpu {
            _vm: vm,
            fd,
            run: RunMapping {
                base: base.cast::<u8>(),
                len: run_size,
            },
        })
    }

    /// ★ Put the vCPU into **flat 32-bit protected mode** with `rip = entry` and
    /// `rbx = rbx` — everything else zero, paging off, so guest-linear is guest-physical.
    ///
    /// The segment *caches* are programmed directly, so the guest needs no GDT, no
    /// trampoline and no real-mode stage. `rbx` is a parameter because it is how this
    /// harness gives each vCPU its own trapped register to write to, which is what makes
    /// several vCPUs distinguishable at the exit without several code images.
    ///
    /// # Errors
    /// [`RawError::Unsupported`] on any host architecture but `x86_64`;
    /// [`RawError::Syscall`] if the kernel refuses the register state.
    ///
    /// # Panics
    /// If called with any ranked lock held (R1, §4.5).
    pub fn enter_flat_protected_mode(&self, entry: u64, rbx: u64) -> Result<(), RawError> {
        lockwitness::assert_lock_free("KVM_SET_SREGS/KVM_SET_REGS");
        leafwitness::assert_leaf_free("KVM_SET_SREGS/KVM_SET_REGS");
        if !cfg!(target_arch = "x86_64") {
            return Err(RawError::Unsupported {
                what: "entering a guest in flat protected mode",
                detail: "segment caches, CR0 and 'protected mode' are x86 concepts; this \
                         harness's guest images are x86-32. The vCPU PLUMBING above is \
                         architecture-neutral and compiles everywhere, which is what the \
                         arm64 cross-check gate covers",
            });
        }
        let mut sregs = KvmSregs::default();
        // Read the kernel's post-reset state first and MODIFY it: the reset values of
        // `tr`, `ldt`, `gdt`, `idt` and the reserved bits of `cr0`/`cr4` are the
        // hardware's business, and inventing them is how a guest ends up in a state VMX
        // refuses to enter.
        ioctl_ref_mut(
            self.fd.as_raw_fd(),
            KVM_GET_SREGS,
            &mut sregs,
            "KVM_GET_SREGS",
        )?;

        let mut seg = KvmSegment {
            base: 0,
            limit: 0xFFFF_FFFF,
            selector: 1 << 3,
            kind: 11, // code: execute/read, accessed
            present: 1,
            dpl: 0,
            db: 1, // 32-bit default operand/address size
            s: 1,  // code/data, not system
            l: 0,  // not 64-bit
            g: 1,  // limit in 4 KiB units
            avl: 0,
            unusable: 0,
            padding: 0,
        };
        sregs.cr0 |= CR0_PE;
        sregs.cs = seg;
        seg.kind = 3; // data: read/write, accessed
        seg.selector = 2 << 3;
        sregs.ds = seg;
        sregs.es = seg;
        sregs.fs = seg;
        sregs.gs = seg;
        sregs.ss = seg;
        ioctl_ref(self.fd.as_raw_fd(), KVM_SET_SREGS, &sregs, "KVM_SET_SREGS")?;

        let regs = KvmRegs {
            rbx,
            rip: entry,
            // Bit 1 is reserved and must be set; everything else clear.
            rflags: 0x2,
            ..KvmRegs::default()
        };
        ioctl_ref(self.fd.as_raw_fd(), KVM_SET_REGS, &regs, "KVM_SET_REGS")
    }

    /// Enter the guest and return why it left.
    ///
    /// # Errors
    /// [`RawError::Syscall`] for anything but `EINTR`, which is [`VcpuExit::Interrupted`].
    ///
    /// # Panics
    /// If called with any ranked lock or any adapter leaf lock held (R1, §4.5) — a guest
    /// runs for an unbounded time, so this is the most blocking call in the system after
    /// the readiness wait.
    pub fn run(&mut self) -> Result<VcpuExit, RawError> {
        lockwitness::assert_lock_free("KVM_RUN (entering the guest)");
        leafwitness::assert_leaf_free("KVM_RUN (entering the guest)");
        // SAFETY: `KVM_RUN` is an `_IO` encoding — the direction bits say "no data
        // transfer" — so the third argument is not interpreted as a pointer and a plain
        // integer is correct here rather than merely conventional. The kernel's side
        // channel is the `mmap`ed run structure this vCPU owns, which stays mapped for
        // `self`'s whole life. `self.fd` is borrowed from a live `OwnedFd`.
        let rc = unsafe { libc::ioctl(self.fd.as_raw_fd(), KVM_RUN as _, 0) };
        if rc < 0 {
            return match std::io::Error::last_os_error().raw_os_error() {
                Some(e) if e == libc::EINTR => Ok(VcpuExit::Interrupted),
                _ => Err(last_syscall_error("KVM_RUN")),
            };
        }
        Ok(self.read_exit())
    }

    /// Supply the value a [`VcpuExit::Mmio`] **load** should observe, then resume.
    ///
    /// The bytes are written into the run structure's MMIO arm, which is where the kernel
    /// looks when it resumes the interrupted instruction. Only the low `len` bytes the
    /// exit reported are meaningful; the rest are ignored by the kernel and are written
    /// anyway so no stale byte from a previous exit can survive.
    ///
    /// # Errors
    /// [`RawError::OutOfRange`] if `len` is not a width the union's 8-byte data array can
    /// hold.
    pub fn complete_mmio_read(&mut self, len: u8, value: u64) -> Result<(), RawError> {
        if len == 0 || usize::from(len) > 8 {
            // Read as "a `len`-byte request at offset 0 into the union's 8-byte data
            // array" — which is exactly what it is.
            return Err(RawError::OutOfRange {
                offset: 0,
                len: u64::from(len),
                object_len: 8,
            });
        }
        let bytes = value.to_le_bytes();
        // SAFETY: the destination is `base + EXIT_UNION_OFFSET + offset_of!(data)`, which
        // `KvmVcpu::create` proved is inside the mapping (`run_size` was checked to cover
        // the head plus the whole MMIO arm before the mapping was stored), and exactly 8
        // bytes are written from an 8-byte array in this frame. The kernel is not running
        // this vCPU right now — `run` returned — so nothing else is touching the union.
        unsafe {
            let dst = self
                .run
                .base
                .add(EXIT_UNION_OFFSET + core::mem::offset_of!(KvmMmioExit, data));
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, 8);
        }
        Ok(())
    }

    /// Read the exit reason and, for an MMIO exit, its arm of the union.
    fn read_exit(&self) -> VcpuExit {
        // SAFETY: both reads are `read_unaligned` from inside the mapping — the head is at
        // offset 0 and the MMIO arm at `EXIT_UNION_OFFSET`, and `KvmVcpu::create` refused
        // to build this value unless the mapping covers both. `KvmRunHead` and
        // `KvmMmioExit` are `#[repr(C)]` plain-old-data with no padding invariants and no
        // pointers, so any bit pattern the kernel wrote is a valid value of either type.
        // The kernel is not running this vCPU (`run` has returned), so this is not a
        // concurrent read.
        let (reason, mmio) = unsafe {
            let head = self.run.base.cast::<KvmRunHead>().read_unaligned();
            let mmio = self
                .run
                .base
                .add(EXIT_UNION_OFFSET)
                .cast::<KvmMmioExit>()
                .read_unaligned();
            (head.exit_reason, mmio)
        };
        match reason {
            EXIT_MMIO => VcpuExit::Mmio {
                gpa: mmio.phys_addr,
                len: u8::try_from(mmio.len).unwrap_or(0),
                is_write: mmio.is_write != 0,
                data: mmio.data,
            },
            EXIT_IO => VcpuExit::PortIo,
            EXIT_HLT => VcpuExit::Halted,
            EXIT_INTR => VcpuExit::Interrupted,
            EXIT_SHUTDOWN => VcpuExit::Shutdown,
            EXIT_FAIL_ENTRY => VcpuExit::FailedEntry,
            EXIT_INTERNAL_ERROR => VcpuExit::InternalError,
            other => VcpuExit::Unhandled { reason: other },
        }
    }
}

// A vCPU is owned by one runner thread and entered only from it. `Send` is what lets that
// thread be spawned; `Sync` is deliberately absent (the raw pointer in `RunMapping`
// withholds it), because two threads entering one vCPU is a kernel-level error, not a
// data race we could document our way out of.
// SAFETY: the mapping is created and unmapped by this type alone, it is `MAP_SHARED` with
// the kernel and with nothing else in this process, and every access to it happens between
// a `KVM_RUN` returning and the next one starting — i.e. under the same exclusive `&mut
// self` that `run` requires. Moving that whole bundle to another thread transfers all of
// it at once.
unsafe impl Send for KvmVcpu {}

impl KvmVm {
    /// Tell the hypervisor where it may place its own task-state segment.
    ///
    /// Required before the first `KVM_RUN` on Intel hosts and absent on AMD, so the
    /// capability is **queried** rather than the error being swallowed: "the kernel said
    /// no" and "this host does not have the concept" are different facts and only one of
    /// them is fine.
    ///
    /// # Errors
    /// [`RawError::Syscall`].
    ///
    /// # Panics
    /// If called with any ranked lock held (R1, §4.5).
    pub fn set_tss_addr_if_supported(&self) -> Result<bool, RawError> {
        lockwitness::assert_lock_free("KVM_SET_TSS_ADDR");
        leafwitness::assert_leaf_free("KVM_SET_TSS_ADDR");
        if self.check_extension(KVM_CAP_SET_TSS_ADDR)? == 0 {
            return Ok(false);
        }
        ioctl_arg(
            self.as_raw(),
            KVM_SET_TSS_ADDR,
            TSS_ADDR,
            "KVM_SET_TSS_ADDR",
        )?;
        Ok(true)
    }
}

/// `ioctl(fd, request, &value)` for the `_IOW` (kernel reads) requests.
fn ioctl_ref<T>(
    fd: libc::c_int,
    request: libc::c_ulong,
    value: &T,
    call: &'static str,
) -> Result<(), RawError> {
    // SAFETY: `value` is a live `&T` for this call and `T` is one of the `#[repr(C)]` uapi
    // structs above, whose size is encoded in the request's own bits — so the kernel reads
    // exactly the bytes this borrow covers and no more. The direction is `_IOW`, i.e.
    // read-only from the kernel's side, so nothing is written back through a shared
    // reference. `fd` is borrowed from a live `OwnedFd` by the caller in this same
    // expression.
    let rc = unsafe { libc::ioctl(fd, request as _, core::ptr::from_ref(value)) };
    if rc < 0 {
        return Err(last_syscall_error(call));
    }
    Ok(())
}

/// `ioctl(fd, request, &mut value)` for the `_IOR` (kernel writes) requests.
fn ioctl_ref_mut<T>(
    fd: libc::c_int,
    request: libc::c_ulong,
    value: &mut T,
    call: &'static str,
) -> Result<(), RawError> {
    // SAFETY: `value` is a live `&mut T` for this call and `T` is one of the `#[repr(C)]`
    // uapi structs above, whose size is encoded in the request's own bits — so the kernel
    // writes exactly the bytes this exclusive borrow covers and no more. Exclusivity comes
    // from the `&mut`, so no other reference can observe the partially-written value.
    // `fd` is borrowed from a live `OwnedFd` by the caller in this same expression.
    let rc = unsafe { libc::ioctl(fd, request as _, core::ptr::from_mut(value)) };
    if rc < 0 {
        return Err(last_syscall_error(call));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Backing, GuestWindow, HostOffset, HostPageSize, KvmMemslot};

    /// The guest image: a 32-bit loop that loads from `probe`, stores what it found
    /// through `ebx`, and repeats.
    ///
    /// ```text
    ///   0: A1 <imm32>   mov eax, [probe]     ; served from RAM, or an MMIO read exit
    ///   5: 89 03        mov [ebx], eax       ; always an MMIO write exit
    ///   7: EB F7        jmp  -9              ; back to 0
    /// ```
    ///
    /// The load's address is an immediate, so the image is built per test rather than
    /// hard-coding an address that is only page-aligned on a 4 KiB host — the same reason
    /// no literal `4096` appears anywhere in this crate's alignment path.
    fn probe_loop(probe: u32) -> [u8; 9] {
        let a = probe.to_le_bytes();
        [0xA1, a[0], a[1], a[2], a[3], 0x89, 0x03, 0xEB, 0xF7]
    }

    fn kvm() -> Kvm {
        Kvm::open().expect(
            "/dev/kvm must be present and permitted to build the KVM-direct harness \
             (l1_os_shell.md §10, decision #48)",
        )
    }

    /// One page of guest RAM at `gpa`, as memslot `slot`, optionally pre-filled.
    fn one_page(
        vm: &Arc<KvmVm>,
        slot: u32,
        gpa: u64,
        fill: &[u8],
    ) -> (Arc<GuestWindow>, KvmMemslot) {
        let page = HostPageSize::query();
        let w = Arc::new(GuestWindow::create(page.bytes(), page).expect("window"));
        if !fill.is_empty() {
            w.write_from(HostOffset::ZERO, fill).expect("fill");
        }
        let s = KvmMemslot::install(
            Arc::clone(vm),
            slot,
            gpa,
            Arc::clone(&w),
            0,
            page.bytes(),
            false,
        )
        .expect("memslot");
        (w, s)
    }

    /// ★★ **The instrument, end to end.** A real guest, on a real vCPU, reads a value we
    /// placed in a real memslot and writes it out through a trapped register — and then
    /// that memslot alone is removed and the *same* load exits instead.
    ///
    /// This is `l1_os_shell.md` §14.8 F7(a) closed at the raw layer: *"neutering the
    /// sub-range address arithmetic to ignore its offset broke nothing, because KVM
    /// validates nothing at install time and nothing executes."* Something executes now,
    /// and the value it reports is the memslot's placement.
    #[test]
    fn a_guest_reads_the_memslot_we_installed_and_exits_when_it_is_gone() {
        let page = HostPageSize::query().bytes();
        let (code_gpa, data_gpa, trap_gpa) = (page, 2 * page, 0x8000_0000u64);
        let kvm = kvm();
        let vm = Arc::new(kvm.create_vm().expect("vm"));
        vm.set_tss_addr_if_supported().expect("tss");

        let image = probe_loop(u32::try_from(data_gpa).expect("below 4 GiB"));
        let (_cw, _code) = one_page(&vm, 0, code_gpa, &image);
        let (_dw, data) = one_page(&vm, 1, data_gpa, &0xC0FF_EE42u32.to_le_bytes());

        let mut vcpu = KvmVcpu::create(&kvm, Arc::clone(&vm), 0).expect("vcpu");
        // `ebx` is the trapped register the guest writes to: an address no memslot covers.
        vcpu.enter_flat_protected_mode(code_gpa, trap_gpa)
            .expect("protected mode");

        // ---- With the memslot live, the guest's load is served with NO exit, and the
        // only exit is the store to the trapped address — carrying the value we placed.
        assert_eq!(
            vcpu.run().expect("run"),
            VcpuExit::Mmio {
                gpa: trap_gpa,
                len: 4,
                is_write: true,
                data: [0x42, 0xEE, 0xFF, 0xC0, 0, 0, 0, 0],
            },
            "the guest must have READ the value out of the memslot we installed and \
             written it back through the trapped register — the value IS the proof that \
             the slot points where the arithmetic said"
        );

        // ---- Remove exactly that memslot (the code stays mapped, so the guest can still
        // run). The very same load now has nowhere to land.
        drop(data);
        let exit = vcpu.run().expect("run");
        let VcpuExit::Mmio {
            gpa,
            len,
            is_write,
            data: _,
        } = exit
        else {
            panic!("a guest-physical range with no memslot must exit to userspace: {exit:?}")
        };
        assert_eq!(
            (gpa, len, is_write),
            (data_gpa, 4, false),
            "a guest-physical range with no memslot is not RAM: the load exits to \
             userspace instead of being served (the exit's `data` is undefined for a \
             load, so it is deliberately not asserted)"
        );
    }

    /// The load-completion path: a value we supply at the exit is what the guest goes on
    /// to store. Without this, an MMIO read handler could return anything and no test
    /// would know.
    #[test]
    fn a_completed_mmio_load_is_the_value_the_guest_then_stores() {
        let page = HostPageSize::query().bytes();
        let (code_gpa, data_gpa, trap_gpa) = (page, 2 * page, 0x8000_0000u64);
        let kvm = kvm();
        let vm = Arc::new(kvm.create_vm().expect("vm"));
        vm.set_tss_addr_if_supported().expect("tss");

        // Only the code page is RAM this time, so the load is a device read.
        let image = probe_loop(u32::try_from(data_gpa).expect("below 4 GiB"));
        let (_cw, _code) = one_page(&vm, 0, code_gpa, &image);

        let mut vcpu = KvmVcpu::create(&kvm, Arc::clone(&vm), 0).expect("vcpu");
        vcpu.enter_flat_protected_mode(code_gpa, trap_gpa)
            .expect("protected mode");

        let exit = vcpu.run().expect("run");
        let VcpuExit::Mmio { gpa, is_write, .. } = exit else {
            panic!("expected the load to exit: {exit:?}")
        };
        assert_eq!((gpa, is_write), (data_gpa, false));
        vcpu.complete_mmio_read(4, 0x1234_5678).expect("complete");

        assert_eq!(
            vcpu.run().expect("run"),
            VcpuExit::Mmio {
                gpa: trap_gpa,
                len: 4,
                is_write: true,
                data: [0x78, 0x56, 0x34, 0x12, 0, 0, 0, 0],
            },
            "the guest must store exactly the value the MMIO load was completed with"
        );
    }

    /// A completion wider than the machine can express is refused by name, not truncated.
    #[test]
    fn an_out_of_range_mmio_completion_width_is_refused_by_that_exact_variant() {
        let kvm = kvm();
        let vm = Arc::new(kvm.create_vm().expect("vm"));
        let mut vcpu = KvmVcpu::create(&kvm, vm, 0).expect("vcpu");
        assert_eq!(
            vcpu.complete_mmio_read(9, 0).unwrap_err(),
            RawError::OutOfRange {
                offset: 0,
                len: 9,
                object_len: 8,
            }
        );
        assert_eq!(
            vcpu.complete_mmio_read(0, 0).unwrap_err(),
            RawError::OutOfRange {
                offset: 0,
                len: 0,
                object_len: 8,
            }
        );
    }

    /// The uapi layouts these ioctl encodings assert. A wrong size here is a kernel
    /// reading or writing the wrong number of bytes into our frame, which is exactly the
    /// class the ABI-parity tests exist for one layer up.
    #[test]
    fn the_uapi_struct_sizes_are_the_ones_the_ioctl_encodings_declare() {
        assert_eq!(core::mem::size_of::<KvmRegs>(), 144, "struct kvm_regs");
        assert_eq!(core::mem::size_of::<KvmSregs>(), 312, "struct kvm_sregs");
        assert_eq!(core::mem::size_of::<KvmSegment>(), 24, "struct kvm_segment");
        assert_eq!(core::mem::size_of::<KvmDtable>(), 16, "struct kvm_dtable");
        assert_eq!(
            EXIT_UNION_OFFSET, 32,
            "the exit union follows a 32-byte head in struct kvm_run"
        );
        // And the encodings really do carry those sizes.
        assert_eq!((KVM_SET_REGS >> 16) & 0x3FFF, 144);
        assert_eq!((KVM_GET_SREGS >> 16) & 0x3FFF, 312);
        assert_eq!((KVM_SET_SREGS >> 16) & 0x3FFF, 312);
    }

    /// The head struct is only ever read out of kernel-written memory, so its unused
    /// fields exist to make the layout right. Touch them once so the compiler cannot
    /// decide they are dead and so a field deletion is a test failure rather than a
    /// silent 8-byte shift of everything after it.
    #[test]
    fn the_run_head_carries_every_field_the_layout_needs() {
        let h = KvmRunHead::default();
        assert_eq!(
            (
                h.request_interrupt_window,
                h.immediate_exit,
                h.padding1.len(),
                h.exit_reason,
                h.ready_for_interrupt_injection,
                h.if_flag,
                h.flags,
                h.cr8,
                h.apic_base,
            ),
            (0, 0, 6, 0, 0, 0, 0, 0, 0)
        );
        let m = KvmMmioExit::default();
        assert_eq!((m.phys_addr, m.data.len(), m.len, m.is_write), (0, 8, 0, 0));
    }

    /// Backing a window with a file and executing out of it works the same way — the
    /// guest does not know what is behind the memslot, which is the property the
    /// two-tier memory plane rests on.
    #[test]
    fn a_guest_executes_out_of_a_file_backed_placement() {
        let page = HostPageSize::query();
        let (code_gpa, data_gpa) = (page.bytes(), 2 * page.bytes());
        let kvm = kvm();
        let vm = Arc::new(kvm.create_vm().expect("vm"));
        vm.set_tss_addr_if_supported().expect("tss");
        let ram = crate::SharedRam::create(page.bytes()).expect("backing");
        let win = Arc::new(GuestWindow::create(page.bytes(), page).expect("window"));
        win.place(
            HostOffset::ZERO,
            page.bytes(),
            Backing::SharedFile {
                fd: ram.as_backing_fd(),
                offset: 0,
            },
        )
        .expect("placement");
        let image = probe_loop(u32::try_from(data_gpa).expect("below 4 GiB"));
        win.write_from(HostOffset::ZERO, &image).expect("code");
        let _slot = KvmMemslot::install(
            Arc::clone(&vm),
            0,
            code_gpa,
            Arc::clone(&win),
            0,
            page.bytes(),
            false,
        )
        .expect("slot");

        let mut vcpu = KvmVcpu::create(&kvm, vm, 0).expect("vcpu");
        vcpu.enter_flat_protected_mode(code_gpa, 0x9000_0000)
            .expect("protected mode");
        let exit = vcpu.run().expect("run");
        let VcpuExit::Mmio { gpa, is_write, .. } = exit else {
            panic!("expected a device access: {exit:?}")
        };
        assert_eq!(
            (gpa, is_write),
            (data_gpa, false),
            "the guest ran out of file-backed memory and reached its first device access"
        );
    }
}
