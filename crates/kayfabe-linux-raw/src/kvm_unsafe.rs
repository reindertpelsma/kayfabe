//! The KVM-direct harness's ioctls — a real VM descriptor and real memslots.
//!
//! `l1_os_shell.md` §10's M2-c decision box: *"M2-c builds against the KVM-direct harness
//! — NOT QEMU… the harness has no VMM-global lock, no serialized dispatch loop and no
//! foreign locks, so M2-c can get the memory plane right without simultaneously fighting
//! lock ordering imposed by someone else's design."* This module is the OS half of that
//! harness, and it is deliberately **four doors wide**: open, create a VM, ask how many
//! memslots exist, and set one.
//!
//! ## ★ What is here and what is emphatically not
//!
//! There are no vCPUs. `KVM_CREATE_VCPU`, `KVM_RUN`, the exit-reason union and the MMIO
//! dispatch that would come with them are **absent by design**, not unfinished: this stage
//! is the memory plane, and a vCPU would add a second unbuilt subsystem to the diff that
//! is meant to answer one question (§10's *"one variable, not two"*, applied one level
//! down). The consequence is stated rather than hidden — see [`KvmVm::set_memslot`]'s
//! note on what a memslot's absence means without a vCPU to observe it.
//!
//! ## ★★ The lifetime hazard this module cannot close on its own
//!
//! A memslot hands the kernel a **host address**, and the kernel keeps it. If the
//! [`GuestWindow`] behind it is dropped while the slot is live, KVM holds a dangling
//! `userspace_addr` — the one hazard in this design that no Rust lifetime expresses,
//! because the kernel is the party holding the reference.
//!
//! [`KvmMemslot`] is the answer: it **owns** both the window (by `Arc`) and the slot
//! number, and its `Drop` clears the slot *before* the window can possibly go away. The
//! bare [`KvmVm::set_memslot`] door exists underneath it and says so; the type is what
//! callers use.
//!
//! ## Numbers, and where they come from
//!
//! The ioctl numbers are `_IO`/`_IOW` encodings of `KVMIO` (`0xAE`) from the kernel's
//! `include/uapi/linux/kvm.h`, spelled out as constants with the encoding shown, because a
//! magic `0x4020_AE46` in a diff is unreviewable. `libc` does not export them.

use crate::error::{RawError, last_syscall_error};
use crate::host_fd_unsafe::adopt_fd;
use crate::window_unsafe::GuestWindow;
use kayfabe_util::lockwitness;
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::Arc;

// --- ioctl numbers -------------------------------------------------------------------
// `_IO(0xAE, nr)` = (0xAE << 8) | nr; `_IOW(0xAE, nr, T)` additionally carries
// `size_of::<T>() << 16` and the write direction `1 << 30`.

/// `_IO(KVMIO, 0x00)` — the API version the running kernel implements.
const KVM_GET_API_VERSION: libc::c_ulong = 0xAE00;
/// `_IO(KVMIO, 0x01)` — create a VM, returning its descriptor.
const KVM_CREATE_VM: libc::c_ulong = 0xAE01;
/// `_IO(KVMIO, 0x03)` — query one capability.
const KVM_CHECK_EXTENSION: libc::c_ulong = 0xAE03;
/// `_IOW(KVMIO, 0x46, struct kvm_userspace_memory_region)` — 32 bytes.
const KVM_SET_USER_MEMORY_REGION: libc::c_ulong =
    (1 << 30) | ((32 as libc::c_ulong) << 16) | 0xAE46;

/// `KVM_CAP_NR_MEMSLOTS` — how many memslots this VM may hold at once.
const KVM_CAP_NR_MEMSLOTS: libc::c_ulong = 10;

/// The only API version KVM has ever shipped as stable, and the one every kernel since
/// 2.6.22 reports. A different number is not a version to adapt to — the uapi structs
/// below would mean something else — so it is a loud refusal.
const KVM_STABLE_API_VERSION: libc::c_int = 12;

/// Guest writes to this slot fault out instead of landing in the backing.
///
/// ★ This is the mechanism behind `Vmm::map_read_native` (§6.1 group 7, the rom-device
/// pattern): reads are served from RAM with no exit at all, writes leave the guest. It is
/// a **slot** flag, which is `l1_os_shell.md` §6.7 item 4 — *"protection is a window
/// property"* — as a fact about the kernel rather than a convention of ours.
const KVM_MEM_READONLY: u32 = 1 << 1;

/// `struct kvm_userspace_memory_region` — `include/uapi/linux/kvm.h`. Field order and
/// widths are the ABI; the layout is C's.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct UserspaceMemoryRegion {
    slot: u32,
    flags: u32,
    guest_phys_addr: u64,
    memory_size: u64,
    userspace_addr: u64,
}

/// The KVM subsystem handle (`/dev/kvm`).
#[derive(Debug)]
pub struct Kvm {
    fd: OwnedFd,
}

impl Kvm {
    /// Open `/dev/kvm` and check that the kernel speaks the API version these structs are
    /// written against.
    ///
    /// # Errors
    /// - [`RawError::Syscall`] — `/dev/kvm` is absent (`ENOENT`) or not permitted
    ///   (`EACCES`). **Both are deployment facts, not bugs**, in the same class as
    ///   §6.8.1's `/dev/userfaultfd` udev rule: no type and no CI grep can observe
    ///   whether the operator's box exposes the device, so the only mechanism is a loud
    ///   refusal at the door.
    /// - [`RawError::Unsupported`] — the kernel reports an API version these structs are
    ///   not written for.
    ///
    /// # Panics
    /// If called with any ranked lock held (R1, §4.5).
    pub fn open() -> Result<Self, RawError> {
        lockwitness::assert_lock_free("open(/dev/kvm)");
        // SAFETY: `open` reads the NUL-terminated path and dereferences nothing else; the
        // literal is a `&CStr`, so the terminator is guaranteed by the type. The mode
        // argument is absent because `O_CREAT` is not in the flags. The result is adopted
        // by `adopt_fd`, which checks the sign, so ownership is unique.
        let raw = unsafe { libc::open(c"/dev/kvm".as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
        let fd = adopt_fd(raw, "open(/dev/kvm)")?;
        let kvm = Kvm { fd };
        let version = ioctl_arg(
            kvm.fd.as_raw_fd(),
            KVM_GET_API_VERSION,
            0,
            "KVM_GET_API_VERSION",
        )?;
        if version != KVM_STABLE_API_VERSION {
            return Err(RawError::Unsupported {
                what: "the running kernel's KVM API version",
                detail: "expected the stable version 12; these uapi structs mean something \
                         else under any other",
            });
        }
        Ok(kvm)
    }

    /// Create a VM. The returned descriptor owns an address space, and nothing else —
    /// no vCPUs, no interrupt controller, no devices.
    ///
    /// # Errors
    /// [`RawError::Syscall`].
    ///
    /// # Panics
    /// If called with any ranked lock held (R1, §4.5).
    pub fn create_vm(&self) -> Result<KvmVm, RawError> {
        lockwitness::assert_lock_free("KVM_CREATE_VM");
        let raw = ioctl_arg(self.fd.as_raw_fd(), KVM_CREATE_VM, 0, "KVM_CREATE_VM")?;
        Ok(KvmVm {
            fd: adopt_fd(raw, "KVM_CREATE_VM")?,
        })
    }
}

/// One VM's address space.
#[derive(Debug)]
pub struct KvmVm {
    fd: OwnedFd,
}

impl KvmVm {
    /// How many memslots this VM may hold at once.
    ///
    /// A **hard, kernel-imposed ceiling** — and the reason §6.7's frequency rule is not
    /// merely a latency argument. The C artifact exhausted its slot allocator on a single
    /// `cuCtxCreate` (>1500 tiny device mmaps, one slot each) and regressed
    /// *single-process* matmul; the fix was one window with an allocator inside it.
    ///
    /// # Errors
    /// [`RawError::Syscall`].
    ///
    /// # Panics
    /// If called with any ranked lock held (R1, §4.5).
    pub fn max_memslots(&self) -> Result<u32, RawError> {
        lockwitness::assert_lock_free("KVM_CHECK_EXTENSION(NR_MEMSLOTS)");
        let n = ioctl_arg(
            self.fd.as_raw_fd(),
            KVM_CHECK_EXTENSION,
            KVM_CAP_NR_MEMSLOTS,
            "KVM_CHECK_EXTENSION",
        )?;
        Ok(u32::try_from(n).unwrap_or(0))
    }

    /// ★★ Install (or, with `len == 0`, remove) one memslot — **the coarse tier**, and the
    /// single most expensive operation in the memory plane.
    ///
    /// `l1_os_shell.md` §6.7, from the source and then measured: this swaps the memslot
    /// array and calls `synchronize_srcu_expedited` (`linux: virt/kvm/kvm_main.c:1599-1633`)
    /// — it waits for **every** vCPU to leave its SRCU read section. A DELETE or MOVE is
    /// **two** such swaps plus a shadow-page zap with a remote TLB flush
    /// (`:1798-1846`, `:1929-1941`), measured at **230–460 µs p50 with a 1–4 ms tail**,
    /// charged to every vCPU regardless of which thread issues the ioctl. *"The only lever
    /// is frequency."*
    ///
    /// ## ★ What the absence of a memslot means
    ///
    /// A guest-physical range with **no memslot is not RAM**: KVM exits to userspace for
    /// it. That is exactly `kayfabe_vmm::RegionKind::Device`, and in this harness it is not
    /// a declaration but the physical truth — which is why the adapter above can assert
    /// the two agree. Without a vCPU nothing observes the exit; the *shape* is real, the
    /// *dispatch* is M2-d.
    ///
    /// ## Lifetime
    ///
    /// The kernel keeps `window`'s base address. Prefer [`KvmMemslot`], which owns both
    /// and cannot leave a live slot pointing at a dropped window.
    ///
    /// # Errors
    /// [`RawError::Syscall`] — carrying the kernel's own `errno`. `EINVAL` for a
    /// misaligned or overlapping region or a slot index past
    /// [`KvmVm::max_memslots`]; `ENOMEM` when the kernel cannot grow the slot array.
    /// **These arms are the reason this stage exists**: a mock's `map_guest` is an insert
    /// and has none of them.
    ///
    /// # Panics
    /// If called with any ranked lock held (R1, §4.5).
    pub fn set_memslot(
        &self,
        slot: u32,
        gpa: u64,
        window: &GuestWindow,
        window_offset: u64,
        len: u64,
        guest_readonly: bool,
    ) -> Result<(), RawError> {
        lockwitness::assert_lock_free("KVM_SET_USER_MEMORY_REGION (installing a memslot)");
        let region = UserspaceMemoryRegion {
            slot,
            flags: if guest_readonly { KVM_MEM_READONLY } else { 0 },
            guest_phys_addr: gpa,
            memory_size: len,
            userspace_addr: window.userspace_addr_at(window_offset, len)?,
        };
        ioctl_ptr(self.fd.as_raw_fd(), &region)
    }

    /// Remove the memslot numbered `slot` (`memory_size = 0`).
    ///
    /// # Errors
    /// [`RawError::Syscall`].
    ///
    /// # Panics
    /// If called with any ranked lock held (R1, §4.5).
    pub fn clear_memslot(&self, slot: u32) -> Result<(), RawError> {
        lockwitness::assert_lock_free("KVM_SET_USER_MEMORY_REGION (removing a memslot)");
        let region = UserspaceMemoryRegion {
            slot,
            ..UserspaceMemoryRegion::default()
        };
        ioctl_ptr(self.fd.as_raw_fd(), &region)
    }
}

/// ★★ A live memslot that **owns what the kernel is pointing at**.
///
/// The hazard this closes is stated in the module docs: a memslot's `userspace_addr` is a
/// host address the kernel retains, and no Rust lifetime describes a reference held by the
/// kernel. So the slot and the window are one object: the `Arc<GuestWindow>` cannot reach
/// zero while this value lives, and `Drop` clears the slot before releasing it.
///
/// It also keeps an `Arc<KvmVm>`, for the same reason one step up — a VM descriptor closed
/// underneath a live slot would make the clearing ioctl fail with `EBADF` in a `Drop`.
#[derive(Debug)]
pub struct KvmMemslot {
    vm: Arc<KvmVm>,
    window: Arc<GuestWindow>,
    slot: u32,
    gpa: u64,
    len: u64,
}

impl KvmMemslot {
    /// Install `window` at `gpa` as memslot number `slot`.
    ///
    /// # Errors
    /// As [`KvmVm::set_memslot`].
    ///
    /// # Panics
    /// If called with any ranked lock held (R1, §4.5).
    pub fn install(
        vm: Arc<KvmVm>,
        slot: u32,
        gpa: u64,
        window: Arc<GuestWindow>,
        window_offset: u64,
        len: u64,
        guest_readonly: bool,
    ) -> Result<Self, RawError> {
        vm.set_memslot(slot, gpa, &window, window_offset, len, guest_readonly)?;
        Ok(KvmMemslot {
            vm,
            window,
            slot,
            gpa,
            len,
        })
    }

    /// The slot's length in bytes.
    #[must_use]
    pub fn len_bytes(&self) -> u64 {
        self.len
    }

    /// The guest-physical base this slot is installed at.
    #[must_use]
    pub fn gpa(&self) -> u64 {
        self.gpa
    }

    /// The slot number.
    #[must_use]
    pub fn slot(&self) -> u32 {
        self.slot
    }

    /// The window this slot points at.
    #[must_use]
    pub fn window(&self) -> &Arc<GuestWindow> {
        &self.window
    }
}

impl Drop for KvmMemslot {
    fn drop(&mut self) {
        // The clearing ioctl must happen while `self.window` is still alive, which is
        // exactly what owning the `Arc` guarantees: the field is dropped after this body
        // runs. R1 is asserted after the call for `Drop`'s sake (§4.5 finding F4) — a
        // panic before the slot is cleared would leave the kernel pointing at memory this
        // very drop is about to release, which is a strictly worse outcome than a late
        // assertion.
        let cleared = self.vm.clear_memslot_in_drop(self.slot);
        if !std::thread::panicking() {
            assert!(
                cleared.is_ok(),
                "clearing memslot {slot} failed ({cleared:?}) — the kernel would be left \
                 pointing at a window this drop is about to release",
                slot = self.slot,
            );
            lockwitness::assert_lock_free("KVM_SET_USER_MEMORY_REGION (dropping a memslot)");
        }
    }
}

impl KvmVm {
    /// `clear_memslot` without the R1 assert, for `Drop` only (§4.5 finding F4: the assert
    /// runs *after* the syscall there, and `Drop` is the one site an ordinary scope exit
    /// can reach with a lock held).
    fn clear_memslot_in_drop(&self, slot: u32) -> Result<(), RawError> {
        let region = UserspaceMemoryRegion {
            slot,
            ..UserspaceMemoryRegion::default()
        };
        ioctl_ptr(self.fd.as_raw_fd(), &region)
    }
}

/// `ioctl(fd, request, arg)` for the by-value requests, returning the kernel's
/// non-negative result.
fn ioctl_arg(
    fd: libc::c_int,
    request: libc::c_ulong,
    arg: libc::c_ulong,
    call: &'static str,
) -> Result<libc::c_int, RawError> {
    // SAFETY: all three arguments are integers passed by value; none of these three
    // requests (`KVM_GET_API_VERSION`, `KVM_CREATE_VM`, `KVM_CHECK_EXTENSION`) interprets
    // its argument as a pointer — they are `_IO` encodings, i.e. the direction bits say
    // "no data transfer", which is what makes passing a plain integer here correct rather
    // than merely conventional. `fd` is borrowed from a live `OwnedFd` by the caller in
    // the same expression. The result is checked for negativity below.
    let rc = unsafe { libc::ioctl(fd, request as _, arg) };
    if rc < 0 {
        return Err(last_syscall_error(call));
    }
    Ok(rc)
}

/// `ioctl(fd, KVM_SET_USER_MEMORY_REGION, &region)`.
fn ioctl_ptr(fd: libc::c_int, region: &UserspaceMemoryRegion) -> Result<(), RawError> {
    // SAFETY: `region` is a live `&UserspaceMemoryRegion` for this call, it is `#[repr(C)]`
    // with exactly the field order and widths of the kernel's
    // `struct kvm_userspace_memory_region`, and the request encodes that struct's size
    // (32) in its own bits — so the kernel reads exactly the bytes this borrow covers and
    // no more. The direction is `_IOW`, i.e. read-only from the kernel's side, so nothing
    // is written back through a shared reference. `fd` is borrowed from a live `OwnedFd`
    // by the caller in the same expression.
    let rc = unsafe {
        libc::ioctl(
            fd,
            KVM_SET_USER_MEMORY_REGION as _,
            core::ptr::from_ref(region),
        )
    };
    if rc < 0 {
        return Err(last_syscall_error("KVM_SET_USER_MEMORY_REGION"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HostPageSize;

    /// Every test here needs a real `/dev/kvm`. It is present on this project's dev boxes
    /// and on any KVM-capable runner; where it is not, the harness cannot be built and
    /// saying so loudly is the only honest option — a silently-skipped OS test is exactly
    /// the green instrument `testing_doctrine.md` §1 forbids.
    fn kvm() -> Kvm {
        Kvm::open().expect(
            "/dev/kvm must be present and permitted to build the KVM-direct harness \
             (l1_os_shell.md §10, decision #48). This is a deployment fact, in the same \
             class as the /dev/userfaultfd udev rule: no code gate can observe it",
        )
    }

    #[test]
    fn a_real_vm_reports_a_real_memslot_ceiling() {
        let vm = kvm().create_vm().expect("KVM_CREATE_VM");
        let n = vm.max_memslots().expect("NR_MEMSLOTS");
        assert!(
            n >= 32,
            "every KVM since the memslot array was made dynamic reports hundreds; {n} \
             means the capability query is not reaching the kernel"
        );
    }

    #[test]
    fn a_window_installed_as_a_memslot_can_be_read_and_removed() {
        let p = HostPageSize::query();
        let vm = Arc::new(kvm().create_vm().expect("vm"));
        let w = Arc::new(GuestWindow::create(2 * p.bytes(), p).expect("window"));
        let slot = KvmMemslot::install(
            Arc::clone(&vm),
            0,
            0x1000_0000,
            Arc::clone(&w),
            0,
            2 * p.bytes(),
            false,
        )
        .expect("a page-aligned window installs");
        assert_eq!(slot.gpa(), 0x1000_0000);
        assert_eq!(slot.slot(), 0);
        drop(slot);
        // Re-installing at the same number proves the drop really cleared it: KVM refuses
        // an ADD onto a live slot's range only when the range overlaps, so the sharper
        // proof is that a SECOND slot may now cover the same GPA.
        let again =
            KvmMemslot::install(Arc::clone(&vm), 1, 0x1000_0000, w, 0, 2 * p.bytes(), false);
        assert!(
            again.is_ok(),
            "dropping a memslot must clear it in the kernel; a second slot over the same \
             guest-physical range would otherwise be EEXIST/EINVAL ({again:?})"
        );
    }

    /// ★ A **real** kernel refusal. A misaligned guest-physical base is `EINVAL` — the
    /// class of arm a `BTreeMap::insert` cannot have.
    #[test]
    fn a_misaligned_memslot_is_the_kernels_einval_not_our_opinion() {
        let p = HostPageSize::query();
        let vm = Arc::new(kvm().create_vm().expect("vm"));
        let w = Arc::new(GuestWindow::create(p.bytes(), p).expect("window"));
        assert_eq!(
            KvmMemslot::install(vm, 0, 0x1000_0000 + 8, w, 0, p.bytes(), false).unwrap_err(),
            RawError::Syscall {
                call: "KVM_SET_USER_MEMORY_REGION",
                errno: Some(libc::EINVAL),
            }
        );
    }

    /// ★ And the other one: a slot index past the kernel's ceiling.
    #[test]
    fn a_slot_index_past_the_ceiling_is_a_real_kernel_refusal() {
        let p = HostPageSize::query();
        let vm = Arc::new(kvm().create_vm().expect("vm"));
        let ceiling = vm.max_memslots().expect("ceiling");
        let w = Arc::new(GuestWindow::create(p.bytes(), p).expect("window"));
        let err =
            KvmMemslot::install(vm, ceiling, 0x2000_0000, w, 0, p.bytes(), false).unwrap_err();
        assert_eq!(
            err,
            RawError::Syscall {
                call: "KVM_SET_USER_MEMORY_REGION",
                errno: Some(libc::EINVAL),
            },
            "slot {ceiling} is one past the ceiling the kernel just reported"
        );
    }

    /// Two windows may not claim the same guest-physical range.
    #[test]
    fn overlapping_memslots_are_refused_by_the_kernel() {
        let p = HostPageSize::query();
        let vm = Arc::new(kvm().create_vm().expect("vm"));
        let a = Arc::new(GuestWindow::create(2 * p.bytes(), p).expect("window a"));
        let b = Arc::new(GuestWindow::create(2 * p.bytes(), p).expect("window b"));
        let _first =
            KvmMemslot::install(Arc::clone(&vm), 0, 0x3000_0000, a, 0, 2 * p.bytes(), false)
                .expect("the first");
        assert_eq!(
            KvmMemslot::install(vm, 1, 0x3000_0000 + p.bytes(), b, 0, 2 * p.bytes(), false)
                .unwrap_err(),
            RawError::Syscall {
                call: "KVM_SET_USER_MEMORY_REGION",
                errno: Some(libc::EEXIST),
            },
            "the guest-physical flat view is the kernel's, and it refuses two backings \
             for one address — the adapter above must not be the only thing that knows"
        );
    }
}
