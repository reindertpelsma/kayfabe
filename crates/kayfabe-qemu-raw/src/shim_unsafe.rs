//! ★★★ The **entire** foreign-function surface of the hypervisor adapter.
//!
//! `l2_qemu_adapter.md` §2.2 / decision Q2. Everything in this file is one of three things
//! and nothing else:
//!
//! 1. the wire types the C shim and this crate both spell (`kayfabe_shim.h` is the other
//!    half, and the two are kept honest by an ABI number **and** a struct size, checked in
//!    both directions);
//! 2. [`ForeignHost`] — [`QemuHost`] implemented over a table of C function pointers;
//! 3. the entry points the C shim calls, each of which decodes its arguments and
//!    immediately hands off to [`crate::shim`], which is safe and is where every decision
//!    lives.
//!
//! # ★★★ THE ONE THING TO READ BEFORE CHANGING THIS FILE
//!
//! **Not one address in this file comes from the guest.** The pointers here are (a) the
//! device pointer the hypervisor handed us, (b) pointers to C's own static tables, and (c)
//! pointers to buffers *we* own. There is no `MAP_FIXED`, no mapping call of any kind, and
//! no arithmetic on a guest-supplied value — a guest-supplied address reaching a placement
//! primitive in the hypervisor's own address space is an arbitrary-write against ourselves,
//! and the adapter's placements happen one crate away, inside `kayfabe_linux_raw`, against a
//! reservation that crate created. If a future edit needs an address from the guest here,
//! that is the signal the seam is in the wrong place.
//!
//! # ★★ Why the function pointers carry the keyword, when Rust would let them not
//!
//! Rust would accept these as ordinary `extern "C" fn` pointers, and then **every call in
//! this file would be a plain expression with no `// SAFETY:` and no ratchet entry**. That
//! is exactly backwards for a crate whose acceptance criterion is *"an auditor enumerates
//! the unsound surface"*: the calls carry real preconditions (the device pointer is live,
//! the C honours the buffer length we pass, the callee does not take the hypervisor's global
//! lock), and a precondition nobody wrote down is a precondition nobody checks. Declaring
//! them the other way costs one block and one sentence per call and makes the obligation
//! greppable. **Do not "simplify" it away.**
//!
//! # ★★ Where the surface was FOLDED, and where it deliberately was not
//!
//! Three private helpers — [`MsgOut::set`], `borrow` and `ForeignHost::adopt` — are ordinary
//! safe functions with one documented relaxation inside each, rather than `unsafe fn`s that
//! every caller must re-acknowledge. That folded ~20 blocks into 3. It is honest because
//! each has **exactly one kind of caller**: an entry point that is already `unsafe`, whose
//! `# Safety` clause states the very precondition the helper needs. Twenty scattered
//! acknowledgements of one obligation is worse for an auditor than one place that performs
//! it.
//!
//! **What was NOT folded, and would have been easy to:** the thirteen calls into the C's
//! primitive table. A macro emitting the call would collapse all thirteen to a single
//! textual relaxation, because the ratchet counts source tokens and a macro body is one
//! token. That is gaming the instrument, not reducing the surface: there really are thirteen
//! foreign calls with thirteen different obligations, and each one's `// SAFETY:` says a
//! different thing. **The count is thirteen because the surface is thirteen.**
//!
//! # What is NOT here, and is named rather than stubbed
//!
//! [`ForeignHost`]'s vector-raising method refuses. Message-signalled interrupts and their
//! descriptor plumbing are `l2_qemu_adapter.md` §7, and this stage builds §3/§5/§8 — the
//! device, the region table and the lifecycle. A refusal that names itself is a smaller lie
//! than a function that returns success without raising anything.

use core::ffi::{c_char, c_void};
use std::sync::Arc;

use kayfabe_vmm::BarId;
use kayfabe_vmm_qemu::host::{BlockerId, HostError, MrHandle, QemuHost};
use kayfabe_vmm_qemu::slots::KvmSlotPlane;

use crate::shim::{
    ABI_VERSION, BarDesc, KayfabeChipIdentity, KayfabeRegAudit, Regs, SectionWire, Shim,
    ShimConfig, Status, bar_index,
};

/// What one register WRITE did, on the wire.
///
/// ★ This structure lives in the **unsafe** half and its two siblings
/// ([`KayfabeChipIdentity`], [`KayfabeRegAudit`]) do not, and the split is not arbitrary:
/// this one holds a raw pointer. The host-pointer gate refuses a host CPU address in any
/// file not named `*_unsafe.rs`, and it refused the first draft of this — the same rule that
/// already keeps [`KayfabeRealizeCfg`]'s register array on this side.
///
/// `fault` addresses this archive's read-only data and has `'static` lifetime, so the C may
/// hold it indefinitely. It is **not** NUL-terminated.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct KayfabeRegWrite {
    /// The fault's tag, or null.
    pub fault: *const u8,
    /// Its length in bytes.
    pub fault_len: u64,
    /// ★★ Stage Q5: **why** a guest-RAM access was refused, in the port's own words, or
    /// null. Non-null exactly when the fault was a guest-RAM refusal, which is the only
    /// fault carrying an address — so it is also the validity flag for the two fields
    /// below, which have no reserved value of their own (a guest-physical address of zero
    /// is a perfectly ordinary address).
    pub ram_why: *const u8,
    /// `ram_why`'s length in bytes, or zero.
    pub ram_why_len: u64,
    /// The guest-physical address refused. Meaningful only when `ram_why` is non-null.
    pub ram_gpa: u64,
    /// The refused access's length in bytes. Meaningful only when `ram_why` is non-null.
    pub ram_len: u64,
    /// ★★★ `#146`: **why** a framebuffer write through the BAR0 moving window was refused,
    /// or null. Exactly the shape `ram_why` has, one aperture over, and non-null is the
    /// validity flag for the two fields below for exactly the same reason: framebuffer
    /// address zero is an ordinary address and a zero-length access is legal.
    ///
    /// ⊘ Non-null means **the bytes did not land**. There is no third answer — see
    /// `kayfabe_device::plane::RegPlane::fb_write`.
    pub fb_why: *const u8,
    /// `fb_why`'s length in bytes, or zero.
    pub fb_why_len: u64,
    /// The framebuffer-physical address refused. Meaningful only when `fb_why` is non-null.
    pub fb_phys: u64,
    /// The refused access's length in bytes. Meaningful only when `fb_why` is non-null.
    pub fb_refused_len: u64,
    /// ★★ `#146`: the framebuffer-physical address a write **landed** at, and non-zero
    /// `fb_landed_valid` beside it.
    ///
    /// ⚠ Two fields for one fact, and the second is not redundant: address **zero** is
    /// where a window at its reset base points, which is exactly where a boot that never
    /// programmed the window would write. A single field could not tell "landed at 0" from
    /// "did not land", and those are the two answers this whole rung is about.
    pub fb_landed: u64,
    /// Non-zero when `fb_landed` is meaningful.
    pub fb_landed_valid: i32,
    /// How many boot transitions fired.
    pub transitions: u32,
    /// How many commands were decoded off the command queue.
    pub commands: u32,
    /// Non-zero if the register model owns this offset.
    pub claimed: i32,
    /// Non-zero if the status-queue interrupt wants announcing.
    pub raise_status_irq: i32,
    /// ★★★ `#151`: non-zero if a vector is pending in the CPU interrupt tree and a
    /// message-signalled interrupt should be delivered. See
    /// [`kayfabe_device::plane::WriteOutcome::raise_cpu_intr`] for why this is a second
    /// field and not the one above.
    pub raise_cpu_intr: i32,
    /// ★★★ **E2** — this write was a **usermode doorbell**:
    /// [`DOORBELL_NONE`] (it was not one — the *control*), [`DOORBELL_SERVED`] (the core
    /// answered with a `DoorbellOutcome`), [`DOORBELL_SERVED_LOCAL`] (**E10e** — the shell's
    /// own CPU copy-engine executor did the work) or [`DOORBELL_REFUSED`] (refused by name,
    /// and `doorbell_kind` is that name).
    ///
    /// ⊘ Four values and no fifth: *"counted as a doorbell, went nowhere, looked
    /// healthy"* is unrepresentable, which is the whole shape of
    /// [`kayfabe_device::DoorbellReport`]. The two servings are separate values because they
    /// are separate events — one rang a host channel and one moved bytes with the CPU — and
    /// a boot that could not tell them apart could not tell forwarding from emulation.
    pub doorbell: i32,
    /// The work-submit token the guest stored. Meaningful only when `doorbell` is not
    /// [`DOORBELL_NONE`].
    pub doorbell_token: u64,
    /// The refusal's stable **kind**, or null — a `FaultTag`, i.e. a `&'static str` from a
    /// fixed finite set, so no host allocation crosses here.
    ///
    /// ★ Non-null exactly when `doorbell == DOORBELL_REFUSED`; the *sentence* (the fault
    /// variant's payload, which is formatted and therefore owned) does **not** cross per
    /// write — it reaches the shell once, through `KayfabeRegAudit::doorbell_refusal`.
    pub doorbell_kind: *const u8,
    /// `doorbell_kind`'s length in bytes, or zero.
    pub doorbell_kind_len: u64,
}

/// [`KayfabeRegWrite::doorbell`] — the write was not a doorbell.
pub const DOORBELL_NONE: i32 = 0;
/// [`KayfabeRegWrite::doorbell`] — the core served it.
pub const DOORBELL_SERVED: i32 = 1;
/// [`KayfabeRegWrite::doorbell`] — the core refused it, by name.
pub const DOORBELL_REFUSED: i32 = 2;
/// ★★★ **E10e** — [`KayfabeRegWrite::doorbell`]: the **shell** served it, on the CPU, with
/// no host ring involved. `doorbell_kind` carries the constant name below; what the executor
/// did reaches the shell once, through `KayfabeRegAudit::doorbell_local_serving`, for the
/// reason the refusal's sentence does — a `format!`ed string has no `'static` address.
pub const DOORBELL_SERVED_LOCAL: i32 = 3;

/// The `doorbell_kind` a [`DOORBELL_SERVED_LOCAL`] write carries. A constant, because there
/// is exactly one way to be served locally; the variation is in the audit's sentence.
const SERVED_LOCAL_KIND: &str = "CpuCe::ServedLocally";

impl KayfabeRegWrite {
    /// The wire form of the port's outcome. The one place the sentence becomes an address.
    fn from_outcome(o: &kayfabe_device::WriteOutcome) -> KayfabeRegWrite {
        let (fault, fault_len) = match o.fault {
            None => (core::ptr::null(), 0),
            Some(m) => (m.as_ptr(), m.len() as u64),
        };
        let (ram_why, ram_why_len, ram_gpa, ram_len) = match o.ram_refusal {
            None => (core::ptr::null(), 0, 0, 0),
            Some(r) => (r.why.as_ptr(), r.why.len() as u64, r.gpa, r.len as u64),
        };
        let (fb_why, fb_why_len, fb_phys, fb_refused_len) = match o.fb_refusal {
            None => (core::ptr::null(), 0, 0, 0),
            Some(r) => (r.why.as_ptr(), r.why.len() as u64, r.phys, r.len as u64),
        };
        // ★★★ E2. ⊘ Only the KIND crosses per write, and only because it is a
        // `&'static str` in this archive's read-only data — the same lifetime claim
        // `fault` above already makes. The sentence is `format!`ed and therefore owned by
        // a temporary; handing out an address into it would be a dangling pointer the
        // instant this function returns. It reaches the shell through the audit instead.
        let (doorbell, doorbell_token, doorbell_kind, doorbell_kind_len) = match &o.doorbell {
            None => (DOORBELL_NONE, 0, core::ptr::null(), 0),
            Some(kayfabe_device::DoorbellReport::Served { token, .. }) => {
                (DOORBELL_SERVED, *token, core::ptr::null(), 0)
            }
            Some(kayfabe_device::DoorbellReport::ServedLocally { token, .. }) => (
                DOORBELL_SERVED_LOCAL,
                *token,
                SERVED_LOCAL_KIND.as_ptr(),
                SERVED_LOCAL_KIND.len() as u64,
            ),
            Some(kayfabe_device::DoorbellReport::Refused { token, refusal }) => (
                DOORBELL_REFUSED,
                *token,
                refusal.kind.0.as_ptr(),
                refusal.kind.0.len() as u64,
            ),
        };
        KayfabeRegWrite {
            fault,
            fault_len,
            ram_why,
            ram_why_len,
            ram_gpa,
            ram_len,
            fb_why,
            fb_why_len,
            fb_phys,
            fb_refused_len,
            fb_landed: o.fb_landed.unwrap_or(0),
            fb_landed_valid: i32::from(o.fb_landed.is_some()),
            transitions: o.transitions as u32,
            commands: o.commands as u32,
            claimed: i32::from(o.claimed),
            raise_status_irq: i32::from(o.raise_status_irq),
            raise_cpu_intr: i32::from(o.raise_cpu_intr),
            doorbell,
            doorbell_token,
            doorbell_kind,
            doorbell_kind_len,
        }
    }
}

// =====================================================================================
// The wire vocabulary — mirrored, field for field, in `qemu/hw/misc/nvkvm/kayfabe_shim.h`
// =====================================================================================

/// The call did what it said.
const WIRE_OK: i32 = 0;
/// Refused, with no number to report.
const WIRE_REFUSED: i32 = 1;
/// A conflicting *requirer* is present.
const WIRE_BUSY: i32 = 2;
/// The host cannot express the request at all.
const WIRE_UNSUPPORTED: i32 = 3;

/// The single sentence a [`HostError::Busy`] carries.
///
/// ★ It is a Rust constant rather than a string the C hands up, and that is deliberate.
/// The hypervisor's discard-disable call reports `-EBUSY` and **does not name the requirer**
/// — there is no name to marshal — so a C string crossing this seam would buy nothing and
/// would cost a `&'static str` lifetime claim over C-owned storage. Keeping the sentence on
/// this side means **no string ever crosses in that direction**, which is one whole class of
/// FFI hazard that simply does not exist in this crate.
const BUSY_REQUIRER: &str = "another device in this machine requires guest-driven RAM discard (a memory balloon with \
     free-page reporting, or virtio-mem). It and this device cannot coexist: this device \
     exports guest RAM to helper processes, and a discard punches the shared backing out from \
     under every mapping of it. Remove one of the two from the command line.";

/// The C's table of primitives, as this crate reads it.
///
/// Every entry is an `Option`, so a C shim that leaves a slot empty produces a named
/// [`Status::Malformed`] at realize rather than a jump to address zero on first use. The
/// `struct_size` field makes a *shorter* table (an older C shim against a newer archive) a
/// refusal too — the case the ABI number alone misses when somebody bumps the table but
/// forgets the number.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct KayfabeHostOps {
    /// Must equal [`ABI_VERSION`].
    pub abi_version: u32,
    /// Must equal `size_of::<KayfabeHostOps>()`.
    pub struct_size: u32,
    /// The hypervisor binary's major version.
    pub version_major: Option<unsafe extern "C" fn(*mut c_void) -> u32>,
    /// The hypervisor binary's minor version.
    pub version_minor: Option<unsafe extern "C" fn(*mut c_void) -> u32>,
    /// Non-zero if this machine runs on the hardware accelerator.
    pub kvm_enabled: Option<unsafe extern "C" fn(*mut c_void) -> i32>,
    /// Block migration, naming our reason. `reason` is `(ptr, len)`, not NUL-terminated.
    pub migrate_add_blocker:
        Option<unsafe extern "C" fn(*mut c_void, *const u8, u64, *mut u64) -> i32>,
    /// Withdraw a blocker.
    pub migrate_del_blocker: Option<unsafe extern "C" fn(*mut c_void, u64)>,
    /// Refuse guest-driven discard machine-wide. Returns [`WIRE_BUSY`] for `-EBUSY`.
    pub ram_block_discard_disable: Option<unsafe extern "C" fn(*mut c_void, i32) -> i32>,
    /// Arrange for the topology callbacks to start.
    pub register_listener: Option<unsafe extern "C" fn(*mut c_void) -> i32>,
    /// Non-zero if the register is a pure-MMIO reservation the hypervisor does not back.
    pub bar_is_unbacked_reservation: Option<unsafe extern "C" fn(*mut c_void, u32) -> i32>,
    /// Where the register is currently programmed. [`WIRE_OK`] and the base, or
    /// [`WIRE_UNSUPPORTED`] while it is unmapped.
    pub bar_base: Option<unsafe extern "C" fn(*mut c_void, u32, *mut u64) -> i32>,
    /// Take a counted reference to a reported region.
    pub ref_region: Option<unsafe extern "C" fn(*mut c_void, u64) -> i32>,
    /// Release a counted reference.
    pub unref_region: Option<unsafe extern "C" fn(*mut c_void, u64)>,
    /// Bounded copy out of the named region's own backing.
    pub read_region: Option<unsafe extern "C" fn(*mut c_void, u64, u64, *mut u8, u64) -> i32>,
    /// Bounded copy into the named region's own backing.
    pub write_region: Option<unsafe extern "C" fn(*mut c_void, u64, u64, *const u8, u64) -> i32>,
    /// Raise an interrupt vector.
    pub signal_msix: Option<unsafe extern "C" fn(*mut c_void, u16) -> i32>,
}

impl KayfabeHostOps {
    /// Whether every primitive is present. See the type's own docs for why absence is
    /// representable at all.
    fn complete(&self) -> bool {
        self.version_major.is_some()
            && self.version_minor.is_some()
            && self.kvm_enabled.is_some()
            && self.migrate_add_blocker.is_some()
            && self.migrate_del_blocker.is_some()
            && self.ram_block_discard_disable.is_some()
            && self.register_listener.is_some()
            && self.bar_is_unbacked_reservation.is_some()
            && self.bar_base.is_some()
            && self.ref_region.is_some()
            && self.unref_region.is_some()
            && self.read_region.is_some()
            && self.write_region.is_some()
            && self.signal_msix.is_some()
    }
}

/// One base-address register, on the wire.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct KayfabeBarCfg {
    /// PCI base-address-register index.
    pub index: u32,
    /// Padding, so the layout is the same on every ABI that cares.
    pub reserved: u32,
    /// Guest-physical base the register is programmed at.
    pub base: u64,
    /// Length in bytes.
    pub len: u64,
}

/// What realize is asked for, on the wire.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct KayfabeRealizeCfg {
    /// Must equal [`ABI_VERSION`].
    pub abi_version: u32,
    /// Must equal `size_of::<KayfabeRealizeCfg>()`.
    pub struct_size: u32,
    /// Non-zero if the machine's RAM has a shareable backing.
    pub shareable_ram: i32,
    /// How many entries `bars` has.
    pub n_bars: u32,
    /// The register table, in table order.
    pub bars: *const KayfabeBarCfg,
}

/// One topology section, on the wire. Five facts, unclassified — the rule that turns them
/// into a verdict lives in one place and it is not here.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct KayfabeSection {
    /// Opaque region identity.
    pub mr: u64,
    /// Guest-physical base.
    pub gpa: u64,
    /// Length in bytes.
    pub len: u64,
    /// Byte offset within the region's backing.
    pub offset_within_region: u64,
    /// The region reports itself as memory.
    pub is_ram: i32,
    /// The region is a device memory region.
    pub is_ram_device: i32,
    /// Reads direct, writes to callbacks.
    pub is_rom_device: i32,
    /// The section is read-only.
    pub readonly: i32,
    /// The section is non-volatile.
    pub nonvolatile: i32,
    /// ★★ The region has a backing file whose identity the hypervisor could take. When
    /// zero, the three fields below are not read.
    pub fd_backed: i32,
    /// `st_dev` of the backing file.
    pub backing_dev: u64,
    /// `st_ino` of the backing file.
    pub backing_ino: u64,
    /// Byte offset into the backing file at which the region begins.
    pub file_offset_of_region: u64,
}

// =====================================================================================
// The two out-parameters every fallible entry point carries
// =====================================================================================

/// The caller's diagnostic out-parameters, as one object with one place that writes them.
///
/// ★ This exists to fold what was twenty scattered relaxations into one. It is safe to
/// construct — holding an address is not the hazard, writing through it is — and
/// [`MsgOut::set`] is the single place that writes.
struct MsgOut {
    msg: *mut *const u8,
    len: *mut u64,
}

impl MsgOut {
    /// Publish a diagnostic, if the caller asked for one.
    ///
    /// The pointer published is into this archive's read-only data and has `'static`
    /// lifetime, so the C may hold it indefinitely. It is **not** NUL-terminated; the header
    /// says to print it with a length.
    fn set(&self, m: &'static str) {
        if self.msg.is_null() || self.len.is_null() {
            return;
        }
        // SAFETY: both pointers were just proven non-null. That they are writable and
        // correctly aligned is the documented precondition of every entry point that
        // constructs a `MsgOut`, and the C shim discharges it by passing the addresses of
        // two of its own stack locals. The two writes are of a `'static` pointer and its
        // length, so nothing dangles after this returns.
        unsafe {
            self.msg.write(m.as_ptr());
            self.len.write(m.len() as u64);
        }
    }
}

// =====================================================================================
// ForeignHost — `QemuHost` over the C's table
// =====================================================================================

/// The hypervisor's device pointer, wrapped so its thread-safety claim is written down
/// once instead of implied everywhere.
#[derive(Clone, Copy, Debug)]
struct DevPtr(*mut c_void);

// SAFETY: the pointer is an opaque token minted by the C shim and handed back to it
// unchanged; this crate never dereferences it. Sending it between threads is therefore
// sound on this side, and the obligation it places on the C side is stated in
// `kayfabe_shim.h`: the object it names must outlive the handle `kayfabe_shim_realize`
// returned, which the shim guarantees by destroying the handle in its unrealize before the
// device object is finalized.
unsafe impl Send for DevPtr {}
// SAFETY: as above, plus the obligation `kayfabe_shim.h` states on the primitives
// themselves — each is either a field read of the device's own PCI bookkeeping, a bounded
// copy against one region's own backing, or a hypervisor call documented as internally
// synchronised. None takes a lock of ours and none mutates state this crate can observe, so
// concurrent calls from our worker threads are sound.
unsafe impl Sync for DevPtr {}

/// [`QemuHost`] implemented over a validated copy of the C's primitive table.
///
/// The table is copied **by value** at construction rather than held by reference: the C
/// shim's table is a `static const`, but depending on that is a promise nobody wrote down,
/// and a by-value copy costs one memcpy at realize and removes the question.
#[derive(Debug)]
pub struct ForeignHost {
    dev: DevPtr,
    ops: KayfabeHostOps,
}

/// Fetch a primitive, or refuse by name. See [`KayfabeHostOps`] for why this arm exists.
macro_rules! op {
    ($self:ident, $field:ident) => {
        match $self.ops.$field {
            Some(f) => f,
            None => {
                return Err(HostError::Unsupported(concat!(
                    "the shim's primitive table has no `",
                    stringify!($field),
                    "`"
                )));
            }
        }
    };
}

/// The same, for the two primitives that return nothing.
macro_rules! op_void {
    ($self:ident, $field:ident) => {
        match $self.ops.$field {
            Some(f) => f,
            None => return,
        }
    };
}

/// Turn a wire status into the port's error type.
fn wire(rc: i32, what: &'static str) -> Result<(), HostError> {
    match rc {
        WIRE_OK => Ok(()),
        WIRE_REFUSED => Err(HostError::Refused { what, errno: None }),
        WIRE_BUSY => Err(HostError::Busy {
            what: BUSY_REQUIRER,
        }),
        WIRE_UNSUPPORTED => Err(HostError::Unsupported(what)),
        n if n < 0 => Err(HostError::Refused {
            what,
            errno: Some(-n),
        }),
        _ => Err(HostError::Refused { what, errno: None }),
    }
}

impl ForeignHost {
    /// Adopt a primitive table, or say why it cannot be adopted.
    ///
    /// Safe to call: its one precondition — that `ops` is readable for the length of this
    /// call — is stated on [`kayfabe_shim_realize`], which is the only caller and is itself
    /// `unsafe`.
    fn adopt(ops: *const KayfabeHostOps, dev: *mut c_void) -> Result<Self, &'static str> {
        if ops.is_null() {
            return Err("the shim passed an empty primitive table");
        }
        // SAFETY: non-null was just established. Readability, alignment and lifetime are the
        // documented precondition of `kayfabe_shim_realize`, discharged by the C shim, which
        // passes the address of its own `static const` table — valid for the whole process.
        // The read is a plain copy of a `Copy` structure with no ownership in it.
        let ops = unsafe { core::ptr::read(ops) };
        if ops.abi_version != ABI_VERSION {
            return Err(
                "the shim and this archive disagree about the wire ABI; they are from \
                 different builds",
            );
        }
        if ops.struct_size as usize != size_of::<KayfabeHostOps>() {
            return Err(
                "the shim and this archive disagree about the size of the primitive table; \
                 the table grew on one side only",
            );
        }
        if !ops.complete() {
            return Err("the shim's primitive table has a hole in it");
        }
        Ok(ForeignHost {
            dev: DevPtr(dev),
            ops,
        })
    }
}

impl QemuHost for ForeignHost {
    fn version(&self) -> (u32, u32) {
        let (Some(maj), Some(min)) = (self.ops.version_major, self.ops.version_minor) else {
            // A table with a hole never reaches here — `adopt` refuses it — and (0, 0) is
            // below every floor, so this arm is a refusal either way.
            return (0, 0);
        };
        // SAFETY: both primitives were present in the table `adopt` validated, and `dev` is
        // the token that table came with. Each is one read of the hypervisor's own version
        // constants: no lock, no allocation, no re-entry into us.
        unsafe { (maj(self.dev.0), min(self.dev.0)) }
    }

    fn kvm_enabled(&self) -> bool {
        let Some(f) = self.ops.kvm_enabled else {
            return false;
        };
        // SAFETY: a validated primitive and its own device token. `kayfabe_shim.h` documents
        // it as one call to the accelerator's presence predicate, which takes no lock.
        unsafe { f(self.dev.0) != 0 }
    }

    fn migrate_add_blocker(&self, reason: &'static str) -> Result<BlockerId, HostError> {
        let f = op!(self, migrate_add_blocker);
        let mut id: u64 = 0;
        // SAFETY: a validated primitive. `reason` is a `&'static str`, so the (pointer,
        // length) pair is readable for exactly `len` bytes and outlives the call, which is
        // what `kayfabe_shim.h` requires — the C copies it and never retains the pointer.
        // `&raw mut id` is a live local this frame owns.
        let rc = unsafe {
            f(
                self.dev.0,
                reason.as_ptr(),
                reason.len() as u64,
                &raw mut id,
            )
        };
        wire(rc, "adding a migration blocker")?;
        Ok(BlockerId(id))
    }

    fn migrate_del_blocker(&self, id: BlockerId) {
        let f = op_void!(self, migrate_del_blocker);
        // SAFETY: a validated primitive and its own device token; `id` is a value this crate
        // received from `migrate_add_blocker` on the same host and passes back unchanged.
        unsafe { f(self.dev.0, id.0) }
    }

    fn ram_block_discard_disable(&self, disable: bool) -> Result<(), HostError> {
        let f = op!(self, ram_block_discard_disable);
        // SAFETY: a validated primitive and its own device token. The `-EBUSY` arm arrives
        // as `WIRE_BUSY` and becomes `HostError::Busy`, which realize reports separately
        // from every other refusal.
        let rc = unsafe { f(self.dev.0, i32::from(disable)) };
        wire(rc, "disabling guest-driven RAM discard")
    }

    fn register_listener(&self) -> Result<(), HostError> {
        let f = op!(self, register_listener);
        // SAFETY: a validated primitive and its own device token. It is called from realize,
        // where the hypervisor's global lock is already held by the caller above us, and
        // `kayfabe_shim.h` forbids the callbacks from running before it returns — this crate
        // has no handle to give them yet.
        let rc = unsafe { f(self.dev.0) };
        wire(rc, "registering the topology listener")
    }

    fn bar_is_unbacked_reservation(&self, bar: BarId) -> bool {
        let Some(f) = self.ops.bar_is_unbacked_reservation else {
            return false;
        };
        // SAFETY: a validated primitive and its own device token. ★ The default on a missing
        // primitive is `false`, i.e. "the hypervisor backs it", because the consequence of
        // guessing wrong in the other direction is two memslots over one guest-physical
        // range with only one winner.
        unsafe { f(self.dev.0, bar_index(bar)) != 0 }
    }

    fn bar_base(&self, bar: BarId) -> Option<u64> {
        let f = self.ops.bar_base?;
        let mut base: u64 = 0;
        // SAFETY: a validated primitive; `&raw mut base` is a live local this frame owns.
        // `kayfabe_shim.h` requires this to be one read of the device's own PCI bookkeeping
        // — it is on the hot path and must never take the global lock.
        let rc = unsafe { f(self.dev.0, bar_index(bar), &raw mut base) };
        (rc == WIRE_OK).then_some(base)
    }

    fn ref_region(&self, mr: MrHandle) -> Result<(), HostError> {
        let f = op!(self, ref_region);
        // SAFETY: a validated primitive; `mr` is an opaque value this crate received from a
        // topology callback of the same host and passes back unchanged. This crate never
        // does arithmetic on it and never dereferences it.
        let rc = unsafe { f(self.dev.0, mr.0) };
        wire(rc, "taking a reference to a reported region")
    }

    fn unref_region(&self, mr: MrHandle) {
        let f = op_void!(self, unref_region);
        // SAFETY: as `ref_region`. Called from the topology callback itself, which arrives
        // holding the hypervisor's global lock — the one context where a finalizer running
        // underneath is legal.
        unsafe { f(self.dev.0, mr.0) }
    }

    fn read_region(&self, mr: MrHandle, off: u64, dst: &mut [u8]) -> Result<(), HostError> {
        let f = op!(self, read_region);
        // SAFETY: a validated primitive. `dst.as_mut_ptr()` is writable for exactly
        // `dst.len()` bytes, which is the length passed, and the C side's obligation stated
        // in `kayfabe_shim.h` is to write no more than that and to refuse rather than
        // truncate. The buffer outlives the call: it is a borrow held across one synchronous
        // call and nothing retains it.
        let rc = unsafe { f(self.dev.0, mr.0, off, dst.as_mut_ptr(), dst.len() as u64) };
        wire(rc, "copying out of a reported region")
    }

    fn write_region(&self, mr: MrHandle, off: u64, src: &[u8]) -> Result<(), HostError> {
        let f = op!(self, write_region);
        // SAFETY: as `read_region`, with the sharper direction: `src.as_ptr()` is readable
        // for exactly `src.len()` bytes and the C reads no more. A stray write into a
        // register window is a side effect on hardware, not merely a bad byte, which is why
        // the bound is passed rather than inferred.
        let rc = unsafe { f(self.dev.0, mr.0, off, src.as_ptr(), src.len() as u64) };
        wire(rc, "copying into a reported region")
    }

    /// ★★★ `#151`: the vector is raised, through the host's own op.
    ///
    /// ⚠ It used to refuse **without calling `signal_msix` at all**, which is a shape worth
    /// naming: the op existed in the vtable and was presence-checked by
    /// [`KayfabeHostOps::complete`], so every gate that asked *"is the host op wired"*
    /// answered yes while this side answered `Unsupported` regardless of what the host
    /// could do. A stub on the near side of a validated seam is invisible to the seam's own
    /// checks.
    ///
    /// ⊘ The host is allowed to refuse — it does, by name, when the guest has not enabled
    /// its own vector table — and that refusal is carried through as a refusal rather than
    /// swallowed. A completion notifier that is told a vector went out when none did is the
    /// exact lie this method was written to avoid telling.
    fn signal_msix(&self, vector: u16) -> Result<(), HostError> {
        let f = op!(self, signal_msix);
        // SAFETY: a validated primitive with no pointer arguments at all — the device
        // handle is the one this `QemuHost` was built around, and the vector is a scalar
        // the host bounds-checks against its own table size before using it as an index.
        let rc = unsafe { f(self.dev.0, vector) };
        wire(rc, "raising a message-signalled interrupt")
    }
}

// =====================================================================================
// The entry points
// =====================================================================================

/// Borrow the shim behind a handle.
///
/// Safe to call: the precondition — that `handle` is one this crate minted and has not yet
/// reclaimed — is stated on every entry point, all of which are `unsafe`.
fn borrow<'a>(handle: *mut c_void) -> Option<&'a Shim> {
    if handle.is_null() {
        return None;
    }
    // SAFETY: non-null was just established. That the address is a live `Box<Shim>` this
    // crate created is the documented precondition of every entry point; the C shim
    // discharges it by storing the handle in the device's own state and clearing it in the
    // same call that reclaims it, so no entry point can see a stale one. The returned borrow
    // is immutable and the shim is `Sync`, so concurrent borrows are sound.
    Some(unsafe { &*handle.cast::<Shim>() })
}

/// The wire ABI this archive speaks. The C shim refuses to realize if it disagrees.
///
/// This is the one entry point that takes no address, which is also what makes it usable as
/// the very first call, before anything has been validated.
#[unsafe(no_mangle)]
pub extern "C" fn kayfabe_shim_abi_version() -> u32 {
    ABI_VERSION
}

/// ★★★★ **§16.65 — the LABEL for bucket `idx` of `KayfabeRegAudit::doorbells_by_engine`.**
///
/// # ⊘ Why the names cross the seam instead of being written down in C
///
/// The census is an array of counts whose meaning is entirely positional, so a C-side name
/// table would be a **second declaration of one ordering** — and this campaign's own record
/// on two projections of one fact is that they eventually disagree and the weaker one is
/// what a reader sees (§16.64). Here the disagreement would be silent and total: every
/// bucket would still print, with a plausible number under the wrong name, and no gate
/// anywhere could tell. So the order is `kayfabe_rt::EngineKind::ALL`'s, once, and C asks.
///
/// ⊘ An out-of-range index returns `"?"` rather than a null: this is a *printer's* label,
/// and a `NULL` handed to `%s` is undefined behaviour in the caller — the refusal must not
/// be more dangerous than the thing it refuses. It also cannot be reached, because the C
/// loop is bounded by `KAYFABE_ENGINE_KINDS`, which a test pins to the Rust count.
///
/// The returned pointer is `'static` and NUL-terminated; the caller must not free it.
#[unsafe(no_mangle)]
pub extern "C" fn kayfabe_shim_engine_kind_name(idx: u32) -> *const c_char {
    engine_kind_c_name(idx).as_ptr()
}

/// [`kayfabe_shim_engine_kind_name`]'s whole decision, as a **safe** function.
///
/// ⊘ The entry point above is then a pointer conversion and nothing else, which is this
/// file's own stated shape: *"each of which decodes its arguments and immediately hands off
/// to [`crate::shim`], which is safe and is where every decision lives."* ★ It also means the
/// label table is testable **without a raw pointer** — the drift test in
/// `tests/shim_logic.rs` calls this, so checking the FFI surface does not require the test
/// tree to name the relaxation keyword and breach the `*_unsafe.rs` naming audit.
#[must_use]
pub fn engine_kind_c_name(idx: u32) -> &'static core::ffi::CStr {
    ENGINE_KIND_C_NAMES
        .get(idx as usize)
        .copied()
        .unwrap_or(c"?")
}

/// The bucket labels as NUL-terminated C strings, in `kayfabe_rt::EngineKind::ALL` order.
///
/// ⊘ `tests/shim_logic.rs` asserts, element by element, that this equals
/// `kayfabe_rt::engine_kind_names()` — so the literals below cannot drift from the enum
/// they label, and a variant added to `EngineKind` fails that test rather than shifting
/// every name in a boot log by one.
const ENGINE_KIND_C_NAMES: [&core::ffi::CStr; crate::shim::ENGINE_KINDS] = [
    c"GrCompute",
    c"GrGraphics",
    c"Ce",
    c"NvEnc",
    c"NvDec",
    c"Other",
];

/// Realize the memory plane. On success `*out_handle` is a handle to be passed to every
/// other entry point and finally to [`kayfabe_shim_unrealize`].
///
/// Returns a [`Status`] code. On any non-zero return `*out_handle` is untouched and
/// `*out_msg` / `*out_msg_len` describe the refusal.
///
/// # Safety
/// `ops` and `cfg` must address readable, correctly aligned structures of the matching
/// types; `cfg->bars` must be readable for `cfg->n_bars` entries; `out_handle`, `out_msg`
/// and `out_msg_len` must be writable or empty.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kayfabe_shim_realize(
    ops: *const KayfabeHostOps,
    dev: *mut c_void,
    cfg: *const KayfabeRealizeCfg,
    out_handle: *mut *mut c_void,
    out_msg: *mut *const u8,
    out_msg_len: *mut u64,
) -> i32 {
    let out = MsgOut {
        msg: out_msg,
        len: out_msg_len,
    };
    let fail = |s: Status, m: &'static str| -> i32 {
        out.set(m);
        s.code()
    };
    if out_handle.is_null() || cfg.is_null() {
        return fail(
            Status::Malformed,
            "the shim called realize with no configuration or nowhere to put the handle",
        );
    }
    // SAFETY: non-null established; readability and alignment are this function's documented
    // precondition, discharged by the C shim, which passes the address of one of its own
    // stack locals it has just filled. The structure is `Copy` and owns nothing.
    let cfg = unsafe { core::ptr::read(cfg) };
    if cfg.abi_version != ABI_VERSION || cfg.struct_size as usize != size_of::<KayfabeRealizeCfg>()
    {
        return fail(
            Status::Malformed,
            "the shim and this archive disagree about the realize configuration's ABI or \
             size; they are from different builds",
        );
    }
    if cfg.n_bars == 0 || cfg.bars.is_null() {
        return fail(
            Status::Malformed,
            "the shim called realize with an empty register table; the table is meant to be \
             the complete enumeration of this device's regions",
        );
    }
    // SAFETY: non-null established, and readability for exactly `n_bars` entries is this
    // function's documented precondition — the C shim passes its own realize-time array,
    // whose length is the same `ARRAY_SIZE` it filled `n_bars` from. The slice is consumed
    // into owned values before this frame returns, so it never outlives the array.
    let raw = unsafe { core::slice::from_raw_parts(cfg.bars, cfg.n_bars as usize) };
    let shim_cfg = ShimConfig {
        shareable_ram: cfg.shareable_ram != 0,
        bars: raw
            .iter()
            .map(|b| BarDesc {
                index: b.index,
                base: b.base,
                len: b.len,
            })
            .collect(),
    };

    let host = match ForeignHost::adopt(ops, dev) {
        Ok(h) => Arc::new(h),
        Err(m) => return fail(Status::Malformed, m),
    };
    let slots = match KvmSlotPlane::discover() {
        Ok(s) => Arc::new(s),
        Err(_) => {
            return fail(
                Status::Refused,
                "could not find exactly one accelerator machine in this process; the memory \
                 plane installs its own slots and will not guess which machine to install \
                 them in",
            );
        }
    };
    match Shim::realize(&shim_cfg, host, slots) {
        Ok(shim) => {
            let handle = Box::into_raw(Box::new(shim)).cast::<c_void>();
            // SAFETY: `out_handle` was proven non-null above, and that it is writable and
            // aligned is this function's documented precondition. The value written is a
            // freshly leaked allocation this crate owns until the shim returns it.
            unsafe { out_handle.write(handle) };
            out.set("");
            Status::Ok.code()
        }
        Err((s, m)) => fail(s, m),
    }
}

/// Tear the memory plane down and destroy the handle. An empty handle is accepted; passing
/// the same live handle twice is a use-after-free and the shim must not do it.
///
/// # Safety
/// `handle` must be empty, or one [`kayfabe_shim_realize`] returned and not yet passed here.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kayfabe_shim_unrealize(handle: *mut c_void) {
    if handle.is_null() {
        return;
    }
    // SAFETY: non-null established; that this addresses a live `Box<Shim>` this crate
    // created and has not yet reclaimed is the documented precondition. Reclaiming it here
    // is the only place the allocation is freed.
    let shim = unsafe { Box::from_raw(handle.cast::<Shim>()) };
    shim.unrealize();
}

/// The listener's add callback.
///
/// # Safety
/// `handle` as [`kayfabe_shim_realize`] returned it; `section` must address a readable,
/// correctly aligned section; the two message out-parameters must be writable or empty.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kayfabe_shim_region_add(
    handle: *mut c_void,
    section: *const KayfabeSection,
    out_msg: *mut *const u8,
    out_msg_len: *mut u64,
) -> i32 {
    let out = MsgOut {
        msg: out_msg,
        len: out_msg_len,
    };
    let Some(shim) = borrow(handle) else {
        out.set("a topology callback with no memory plane behind it");
        return Status::Malformed.code();
    };
    if section.is_null() {
        out.set("a topology callback with no section");
        return Status::Malformed.code();
    }
    // SAFETY: non-null established; readability and alignment are this function's documented
    // precondition, discharged by the C shim passing the address of a stack local it just
    // filled. The structure is `Copy` and owns nothing.
    let s = unsafe { core::ptr::read(section) };
    let w = SectionWire {
        mr: s.mr,
        gpa: s.gpa,
        len: s.len,
        offset_within_region: s.offset_within_region,
        is_ram: s.is_ram != 0,
        is_ram_device: s.is_ram_device != 0,
        is_rom_device: s.is_rom_device != 0,
        readonly: s.readonly != 0,
        nonvolatile: s.nonvolatile != 0,
        fd_backed: s.fd_backed != 0,
        backing_dev: s.backing_dev,
        backing_ino: s.backing_ino,
        file_offset_of_region: s.file_offset_of_region,
    };
    match shim.region_add(w) {
        Ok(()) => Status::Ok.code(),
        Err((st, m)) => {
            out.set(m);
            st.code()
        }
    }
}

/// The listener's delete callback.
///
/// # Safety
/// `handle` as [`kayfabe_shim_realize`] returned it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kayfabe_shim_region_del(handle: *mut c_void, gpa: u64, len: u64) {
    if let Some(shim) = borrow(handle) {
        shim.region_del(gpa, len);
    }
}

/// The preventer: refuse a base-address-register move once a reservation lives in it.
///
/// # Safety
/// `handle` as [`kayfabe_shim_realize`] returned it; the message out-parameters must be
/// writable or empty.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kayfabe_shim_bar_move_requested(
    handle: *mut c_void,
    bar: u32,
    out_msg: *mut *const u8,
    out_msg_len: *mut u64,
) -> i32 {
    let out = MsgOut {
        msg: out_msg,
        len: out_msg_len,
    };
    let Some(shim) = borrow(handle) else {
        // Nothing of ours lives in that register yet, so nothing of ours objects.
        return Status::Ok.code();
    };
    match shim.bar_move_requested(bar) {
        Ok(()) => Status::Ok.code(),
        Err((st, m)) => {
            out.set(m);
            st.code()
        }
    }
}

/// The detector: record where a base-address register is now.
///
/// # Safety
/// `handle` as [`kayfabe_shim_realize`] returned it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kayfabe_shim_note_bar_mapping(
    handle: *mut c_void,
    bar: u32,
    mapped: i32,
    base: u64,
) -> i32 {
    let Some(shim) = borrow(handle) else {
        return Status::Malformed.code();
    };
    match shim.note_bar_mapping(bar, (mapped != 0).then_some(base)) {
        Ok(()) => Status::Ok.code(),
        Err((st, _)) => st.code(),
    }
}

/// Install one reservation, once a base-address register has a base.
///
/// # Safety
/// `handle` as [`kayfabe_shim_realize`] returned it; the message out-parameters must be
/// writable or empty.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kayfabe_shim_install_window(
    handle: *mut c_void,
    gpa: u64,
    len: u64,
    out_msg: *mut *const u8,
    out_msg_len: *mut u64,
) -> i32 {
    let out = MsgOut {
        msg: out_msg,
        len: out_msg_len,
    };
    let Some(shim) = borrow(handle) else {
        out.set("a reservation with no memory plane behind it");
        return Status::Malformed.code();
    };
    match shim.install_window(gpa, len) {
        Ok(_) => Status::Ok.code(),
        Err((st, m)) => {
            out.set(m);
            st.code()
        }
    }
}

/// Copy the counters out, so an acceptance test outside this process can assert on
/// something other than an exit code.
///
/// # Safety
/// `handle` as [`kayfabe_shim_realize`] returned it; `out` must be writable and correctly
/// aligned.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kayfabe_shim_audit(
    handle: *mut c_void,
    out: *mut crate::shim::KayfabeAudit,
) -> i32 {
    if out.is_null() {
        return Status::Malformed.code();
    }
    let Some(shim) = borrow(handle) else {
        return Status::Malformed.code();
    };
    // SAFETY: non-null established; writability and alignment are this function's documented
    // precondition. `KayfabeAudit` is `Copy` and owns nothing, so the write cannot leak or drop
    // anything the C would then have to reclaim.
    unsafe { out.write(shim.audit()) };
    Status::Ok.code()
}

// =====================================================================================
// The register plane (stage Q4) — a SECOND handle with its own lifetime
// =====================================================================================

/// Borrow the register plane behind a handle.
///
/// Safe to call for the same reason [`borrow`] is: the precondition is stated on every
/// entry point, all of which are `unsafe`.
fn borrow_regs<'a>(handle: *mut c_void) -> Option<&'a Regs> {
    if handle.is_null() {
        return None;
    }
    // SAFETY: non-null was just established. That the address is a live `Box<Regs>` this
    // crate created is the documented precondition of every entry point below; the C shim
    // discharges it by storing the handle in the device's own state and clearing it in the
    // same call that reclaims it. The borrow is immutable and `Regs` is `Sync`, so the
    // concurrent borrows a multi-vCPU machine produces are sound.
    Some(unsafe { &*handle.cast::<Regs>() })
}

/// The identity a chip's device claims. `device_id` of 0 selects the table's default row.
///
/// # Safety
/// `out` must address a writable, correctly aligned [`KayfabeChipIdentity`]; the message
/// out-parameters must be writable or empty.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kayfabe_shim_chip_identity(
    device_id: u16,
    out: *mut KayfabeChipIdentity,
    out_msg: *mut *const u8,
    out_msg_len: *mut u64,
) -> i32 {
    let msg = MsgOut {
        msg: out_msg,
        len: out_msg_len,
    };
    if out.is_null() {
        msg.set("the shim asked for a chip identity with nowhere to put it");
        return Status::Malformed.code();
    }
    match crate::shim::chip_identity(device_id) {
        Ok(id) => {
            // SAFETY: non-null established; writability and alignment are this function's
            // documented precondition, discharged by the C shim passing the address of one
            // of its own stack locals. The structure is `Copy` and owns nothing, so the
            // write cannot leak or drop anything the C would have to reclaim.
            unsafe { out.write(id) };
            Status::Ok.code()
        }
        Err((s, m)) => {
            msg.set(m);
            s.code()
        }
    }
}

/// Create the register plane. `device_id` of 0 selects the table's default row.
///
/// `probe_arm` / `probe_arm_len` carry the `probe-arm-notifier` device property — a
/// comma-separated decimal list of notifier indices the boot will probe-arm. Pass
/// `(NULL, 0)` or an empty string for the shipping configuration. ⊘ Junk in the string
/// refuses the device by name rather than booting probe-off.
///
/// # Safety
/// `out_handle` must be writable; the message out-parameters must be writable or empty;
/// `probe_arm` must point to `probe_arm_len` readable bytes, or be NULL with
/// `probe_arm_len == 0`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kayfabe_shim_regs_create(
    device_id: u16,
    probe_arm: *const u8,
    probe_arm_len: u64,
    out_handle: *mut *mut c_void,
    out_msg: *mut *const u8,
    out_msg_len: *mut u64,
) -> i32 {
    let msg = MsgOut {
        msg: out_msg,
        len: out_msg_len,
    };
    if out_handle.is_null() {
        msg.set("the shim asked for a register plane with nowhere to put the handle");
        return Status::Malformed.code();
    }
    let probe_str = if probe_arm.is_null() {
        if probe_arm_len != 0 {
            msg.set("probe-arm-notifier arrived as NULL with a non-zero length");
            return Status::Malformed.code();
        }
        ""
    } else {
        // SAFETY: non-null with `probe_arm_len` readable bytes is this function's
        // documented precondition, checked as far as a pointer can be.
        let bytes = unsafe { core::slice::from_raw_parts(probe_arm, probe_arm_len as usize) };
        match core::str::from_utf8(bytes) {
            Ok(s) => s,
            Err(_) => {
                msg.set(
                    "probe-arm-notifier is not UTF-8; a probe string that cannot be read \
                     must refuse the device rather than boot probe-off",
                );
                return Status::Malformed.code();
            }
        }
    };
    match Regs::create_probed(device_id, probe_str) {
        Ok(regs) => {
            let handle = Box::into_raw(Box::new(regs)).cast::<c_void>();
            // SAFETY: `out_handle` was proven non-null above, and that it is writable and
            // aligned is this function's documented precondition. The value written is a
            // freshly leaked allocation this crate owns until the shim returns it to
            // `kayfabe_shim_regs_destroy`.
            unsafe { out_handle.write(handle) };
            msg.set("");
            Status::Ok.code()
        }
        Err((s, m)) => {
            msg.set(m);
            s.code()
        }
    }
}

/// ★★★ **Stage Q5.** Join the two planes: give the register plane the realized memory
/// plane's guest RAM.
///
/// Called once the memory plane has realized — which is *later* than the register plane's
/// creation, because a memory plane needs a programmed base-address register and a
/// register plane does not. Until it is called, and again after
/// [`kayfabe_shim_regs_detach_ram`], every guest-memory access the emulated GSP attempts is
/// refused by name.
///
/// Returns [`Status::Ok`], or [`Status::Malformed`] if either handle is empty — which is
/// the C shim calling in the wrong order, and is worth a code rather than a silent no-op:
/// the symptom of the missing call is a device that boots normally and then refuses one
/// specific write thousands of records in.
///
/// # Safety
/// `regs` must be empty or one [`kayfabe_shim_regs_create`] returned and not yet destroyed;
/// `shim` must be empty or one [`kayfabe_shim_realize`] returned and not yet unrealized.
/// The memory plane must outlive the register plane's use of it, which the C shim
/// discharges by calling [`kayfabe_shim_regs_detach_ram`] before `kayfabe_shim_unrealize`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kayfabe_shim_regs_attach_ram(regs: *mut c_void, shim: *mut c_void) -> i32 {
    let (Some(regs), Some(shim)) = (borrow_regs(regs), borrow(shim)) else {
        return Status::Malformed.code();
    };
    regs.attach_ram(shim);
    Status::Ok.code()
}

/// ★★★ §5.7 — report the stated guest-RAM layout **again, at the end of the run**.
///
/// # ⊘ Why this exists at all: the attach-time report answered the wrong question
///
/// `[measured 2026-08-10, boots w225a/w225b, rev fbc8cd7/99672fe]` the report taken inside
/// [`kayfabe_shim_regs_attach_ram`] said **`0 reported -> 0 RAM -> 0 backed`** on a machine
/// with 2 GiB of guest RAM open at a descriptor this device had already adopted. The layout
/// was not wrong; the *instant* was. The topology listener is registered on the device's
/// bus-master address space, whose flat view is empty until the guest turns bus mastering
/// on, and attach-time is long before that.
///
/// ★ That is `a_correct_capture_can_answer_the_wrong_question` exactly: a working instrument,
/// a correct number, and a question about a **lifetime** answered by sampling one instant.
/// So the layout is reported twice and both reports say which instant they are, because the
/// difference between them is itself the finding.
///
/// # Safety
/// `regs` must be empty or one [`kayfabe_shim_regs_create`] returned and not yet destroyed;
/// `shim` must be empty or one [`kayfabe_shim_realize`] returned and not yet unrealized.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kayfabe_shim_regs_report_ram_layout(regs: *mut c_void, shim: *mut c_void) {
    if let (Some(regs), Some(shim)) = (borrow_regs(regs), borrow(shim)) {
        regs.report_stated_guest_ram_at(shim, "END OF RUN");
    }
}

/// Put the register plane back to refusing every guest-memory access, by name.
///
/// ★ The C shim calls this **before** `kayfabe_shim_unrealize`, and the ordering is the
/// point: the register surface keeps answering across a memory-plane teardown by design,
/// so the port that reaches into the memory plane has to be withdrawn explicitly rather
/// than implied by a lifetime the C cannot see.
///
/// # Safety
/// `regs` must be empty, or one [`kayfabe_shim_regs_create`] returned and not yet
/// destroyed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kayfabe_shim_regs_detach_ram(regs: *mut c_void) {
    if let Some(regs) = borrow_regs(regs) {
        regs.detach_ram();
    }
}

/// Destroy the register plane. An empty handle is accepted; passing the same live handle
/// twice is a use-after-free and the shim must not do it.
///
/// # Safety
/// `handle` must be empty, or one [`kayfabe_shim_regs_create`] returned and not yet passed
/// here.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kayfabe_shim_regs_destroy(handle: *mut c_void) {
    if handle.is_null() {
        return;
    }
    // SAFETY: non-null established; that this addresses a live `Box<Regs>` this crate
    // created and has not yet reclaimed is the documented precondition. This is the only
    // place the allocation is freed.
    drop(unsafe { Box::from_raw(handle.cast::<Regs>()) });
}

/// ★★★ **The hot path.** One trapped register read, one value.
///
/// An empty handle reads zero rather than trapping: a device whose plane failed to build
/// has already said so loudly at realize, and crashing a guest afterwards adds nothing.
///
/// # Safety
/// `handle` must be empty, or one [`kayfabe_shim_regs_create`] returned and not yet
/// destroyed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kayfabe_shim_regs_read(
    handle: *mut c_void,
    bar: u32,
    off: u64,
    size: u32,
) -> u64 {
    match borrow_regs(handle) {
        None => 0,
        // ★★★ w323 — the read path is a guest trap too, and it is marked for the SAME
        // reason the write path is: it holds the BQL. ⊘ It is expected to be cheap
        // (`[w320, measured]` MMIO reads total **20.5 ms per BOOT**, and this tree's
        // standing directive is READ-NATIVE, WRITE-TRAP), so marking it should cost
        // nothing — and if a host verb ever appears beneath a read, this is what makes it
        // say so instead of being invisible. **A gate that only watches the path you
        // already suspect cannot refute you.**
        Some(regs) => {
            let _trap = kayfabe_util::trapwitness::TrapGuard::enter();
            regs.read(bar, off, size)
        }
    }
}

/// One trapped register write. `out` may be empty if the caller does not care.
///
/// # Safety
/// `handle` as [`kayfabe_shim_regs_read`]; `out` must be writable and correctly aligned, or
/// empty.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kayfabe_shim_regs_write(
    handle: *mut c_void,
    bar: u32,
    off: u64,
    size: u32,
    val: u64,
    out: *mut KayfabeRegWrite,
) {
    let Some(regs) = borrow_regs(handle) else {
        return;
    };
    // ★★★★★ **w323 — THE TRAP MARK, AT THE ONE PLACE THE GUEST CROSSES INTO US.**
    //
    // Every guest MMIO write arrives here with the QEMU BQL held, so everything beneath
    // this line runs with **every vCPU and QEMU's main loop** blocked
    // (`blocking_and_completion_model.md` §0). The guard is what makes that expressible: a
    // host RM verb reached from below it can no longer obtain an honest
    // `kayfabe_util::trapwitness::OffTrap` and is counted as an **enumerated inline
    // exception** instead of passing unremarked.
    //
    // ⊘ Installed here rather than in `Regs::write` deliberately: this is the OUTERMOST
    // boundary — the C's `nvkvm_trap_write` calls exactly this symbol — and a guard one
    // frame in would leave the drains `Regs::write` runs *after* the plane call
    // (materialize, err-grants, the budgeted drain, the reap) outside the mark, which is
    // precisely where `w317` measured a 3.70 s hold.
    //
    // ⚠ It measures a duration too, published on `Drop` as
    // `kayfabe_util::trapwitness::worst_trap_us()`. A census that only counted violations
    // could not answer clause (b) — *"is the residue BOUNDED"* — and that is the half a
    // predicate about placement cannot reach on its own.
    let _trap = kayfabe_util::trapwitness::TrapGuard::enter();
    let o = KayfabeRegWrite::from_outcome(&regs.write(bar, off, size, val));
    if out.is_null() {
        return;
    }
    // SAFETY: non-null established; writability and alignment are this function's
    // documented precondition. The structure is `Copy`; its one pointer addresses this
    // archive's read-only data and has `'static` lifetime, so the C may hold it
    // indefinitely and nothing dangles when this returns.
    unsafe { out.write(o) };
}

/// Power-on reset: rebuild the emulated GSP's state machine.
///
/// # Safety
/// `handle` as [`kayfabe_shim_regs_read`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kayfabe_shim_regs_reset(handle: *mut c_void) {
    if let Some(regs) = borrow_regs(handle) {
        regs.reset();
    }
}

/// Copy the register plane's counters out.
///
/// # Safety
/// `handle` as [`kayfabe_shim_regs_read`]; `out` must be writable and correctly aligned.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kayfabe_shim_regs_audit(
    handle: *mut c_void,
    out: *mut KayfabeRegAudit,
) -> i32 {
    if out.is_null() {
        return Status::Malformed.code();
    }
    let Some(regs) = borrow_regs(handle) else {
        return Status::Malformed.code();
    };
    // SAFETY: non-null established; writability and alignment are this function's
    // documented precondition. `KayfabeRegAudit` is `Copy` and owns nothing.
    unsafe { out.write(regs.audit()) };
    Status::Ok.code()
}
