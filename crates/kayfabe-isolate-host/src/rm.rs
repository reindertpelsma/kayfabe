//! ★★★ The real thing: an [`RmBackend`] whose implementation is **NVIDIA RM ioctls**.
//!
//! This file is what `host_execution_plane.md` §0 says did not exist. Everything above it
//! — the port, the pool, the plan/execute split, the whole L1 lock discipline — was
//! designed against a double that returns promptly by construction. Here the verbs land on
//! a driver that serialises them on a per-client write lock and waits uninterruptibly.
//!
//! ## The bring-up ladder (§4's *"the run ladder exists from day one"*)
//!
//! Each rung names what is attempted and what "working" looks like, so a failure localises
//! to a layer instead of arriving as one undifferentiated `cuCtxCreate 999`.
//!
//! | rung | attempt | working looks like |
//! |------|---------|--------------------|
//! | R0 | `openat` the control node from the granted `/dev` directory | a descriptor |
//! | R1 | `openat` `nvidia<gpu>` | a descriptor |
//! | R2 | `NV_ESC_CHECK_VERSION_STR` query | a version string inside the interval these encoders were transcribed for; ★ **a gate since 2026-07-31** |
//! | R3 | `NV_ESC_REGISTER_FD` binding the GPU node to the control session | rc 0 |
//! | R4 | `NV01_ROOT_CLIENT` | RM writes back an `hClient` |
//! | R5 | `NV01_DEVICE_0` with `deviceId = gpu` | status 0 |
//! | R6 | `NV20_SUBDEVICE_0` | status 0 |
//! | R7 | `FERMI_VASPACE_A` | a per-`Vas` host address space (#14's fix) |
//! | R8 | `NV_ESC_RM_ALLOC_MEMORY` sysmem | a memory handle |
//! | R9 | `NV_ESC_RM_MAP_MEMORY_DMA` | a host GPU VA |
//! | R13 | a channel group, a ring, USERD, a channel, BIND, SCHEDULE | a **work-submit token** |
//!
//! ## ★ What the minimum handshake actually is — measured, not assumed
//!
//! **Nothing.** A freshly opened control node accepts `NV_ESC_RM_ALLOC` of a client
//! immediately: two independent paths in the C artifact do exactly that with no version
//! check, no `SYS_PARAMS` and no `REGISTER_FD`
//! (`C: src/qemu/nvkvm_isolate_handlers.c:690-696`, `C: src/qemu/nvkvm_gpu_emul.c:6411`).
//! R2 is kept because the version string is what selects an ABI profile, and R3 because a
//! *device node* used without it answers `0x23 INVALID_CLIENT`
//! (`C: src/qemu/nvkvm_gpu_emul.c:7217-7231`).
//!
//! ## ★★★ R2 IS A GATE, and what it is a gate on is THIS FILE
//!
//! Every parameter block below is encoded by a **const-size, version-free** encoder —
//! `…::SIZE` buffers and `encode_into`, used unconditionally — and those encoders were
//! transcribed from one driver (`kayfabe_abi::submit` §"Provenance": `ogkm-580:
//! 580.159.04`). So this file is silently pinned to a host driver interval it never
//! states. Run it against a host outside that interval and nothing errors: the ioctls
//! succeed and the fields land in the wrong places — at `ogkm-610` `NV_CHANNEL_ALLOC_PARAMS`
//! gains a field at +32 and `engineType` moves from +128 to +132, which is the C's proven
//! `engineType = 0` bug class arrived at from a different road.
//!
//! ⊘ **The fix is not a host version axis.** There is one host driver available to this
//! project, so a per-version table set here would be a mechanism with no red available to
//! it. The fix is to make the pin **say its own name**: [`host_version_gate`] refuses at
//! R2, quoting the host's version and the layout delta, and
//! `kayfabe_abi::host_driver` holds the interval because a version fact is data.
//! `docs/design/host_driver_version_pin.md` is the note; §5 there names the host-side
//! table as the follow-on that is deliberately not built.
//!
//! ## ★★ The verbs that are NOT implemented, and why that is a refusal
//!
//! This section used to list five, then three. `alloc_channel`, `alloc_engine_object` and
//! `schedule` are real (R13); `ring_doorbell` is real (R15) and `ce_copy`'s **`HostCe`
//! arm** is real (R17), both proven on hardware. What still returns [`RmError::Other`]
//! carrying [`NOT_ON_THIS_RUNG`], each naming what it lacks at its own definition:
//!
//! - **`fb_read`** and **`ce_copy`'s [`CeExecutor::Ours`] arm** — the isolate's own
//!   mapping of the *fabricated* aperture, whose extent is not written down anywhere in
//!   this tree.
//! - **`ce_copy` with a [`CeSource::Constant`]** — a fill needs `REMAP_ENABLE` and the
//!   `SET_REMAP_*` method block, which the ABI module does not transcribe.
//! - **`export_surface`** — a PRIME export.
//!
//! Returning a plausible success would be the exact failure `mode2_real_forward_not_fake`
//! forbids: *"prove compute via HW sema/util, never green-guest-log"*. A named refusal
//! keeps MISS = FAULT true one layer down.
//!
//! ## ★★★ What each rung proves, and what it deliberately does not
//!
//! - **R13 — a channel exists in hardware.** RM assigns it a chid out of the GPU's channel
//!   RAM and reports a work-submit token we neither compute nor can predict, and two
//!   channels get two different ones. It proves nothing about *submission*.
//! - **R14 — the ring is the GPU's memory.** Written through one mapping, read back
//!   through a second, independent one. It proves nothing about *execution*.
//! - **R15 — hardware executed our methods.** The semaphore goes `0 -> payload` **and**
//!   `GP_GET` advances to meet `GP_PUT`. `GP_GET` is the only word in this crate hardware
//!   writes and we do not. Measured RTX 3090 / 580.159.04.
//! - **R16 — the mapping survives the SANDBOX.** The capability-less isolate CPU-maps the
//!   ring, USERD and the usermode BAR0 window and rings. ⊘ It produces **no submission
//!   evidence**: the port's verb surface cannot build a pushbuffer through the child, so
//!   R16 shows the mapping and the store, not that anything ran.
//! - **R17 — a real copy engine moved device memory.** Destination read before and after,
//!   the "after" through an independent mapping, plus the engine's own release semaphore.

use crate::export::ChildExports;
use kayfabe_abi::bringup::{
    NV_ESC_CHECK_VERSION_STR, NV_ESC_REGISTER_FD, NV_ESC_RM_ALLOC_MEMORY, NV_IOCTL_MAGIC,
    NV01_MEMORY_SYSTEM, NV01_MEMORY_VIRTUAL, NV20_SUBDEVICE_0, NVOS02_FLAGS_LOCATION_PCI,
    NVOS02_FLAGS_MAPPING_NO_MAP, NVOS02_FLAGS_PHYSICALITY_NONCONTIGUOUS,
    NVOS46_FLAGS_DMA_OFFSET_FIXED_TRUE, Nv2080AllocParameters, NvMemoryVirtualAllocationParams,
    NvVaspaceAllocationParameters, Nvos02ParametersWithFd, RegisterFd,
};
// ★★ #156 — the three ARCH-VARYING class ids that used to be imported here
// (`AMPERE_CHANNEL_GPFIFO_A`, `AMPERE_USERMODE_A`, `AMPERE_DMA_COPY_B`) are gone. They
// now arrive through [`RmConnection::classes`], a `kayfabe_arch::HostClasses` profile.
// The three that remain are NOT arch-varying: `FERMI_VASPACE_A`,
// `KEPLER_CHANNEL_GROUP_A` and `NV01_*` are NVIDIA's permanent identifiers for classes
// that are current on every part from Fermi/Kepler to Blackwell — the generation word in
// the name is not a generation claim. Sourced, not assumed: all three appear verbatim in
// GA106's, AD106's and GH100's own class lists (`ogkm-580:
// src/nvidia/generated/g_gpu_class_list.c` — `FERMI_VASPACE_A` at `:1124`/`:1748`/`:2001`,
// `KEPLER_CHANNEL_GROUP_A` at `:1134`/`:1758`/`:2031`).
use kayfabe_abi::generated::classes::{
    NV01_DEVICE_0, NV01_ROOT_CLIENT, Nv0080AllocParameters, NvChannelGroupAllocationParameters,
};
// ★★ The two host classes that do NOT vary, by ROLE. `kayfabe_abi::invariant_classes`
// carries the ids and, more importantly, the per-chip citations that make "does not vary"
// a checked statement rather than an assumption. Naming them by role is what the
// Generation-name gate's own failure text prescribes ("a name that says what it MEANS,
// not which chip has it") and is why this crate can be SCOPED by that gate rather than
// excused from it — a name scan cannot distinguish `KEPLER_CHANNEL_GROUP_A`, whose
// generation word is vestigial, from `AMPERE_DMA_COPY_B`, whose is not.
use kayfabe_abi::generated::nvos::{
    NV_ESC_RM_ALLOC, NV_ESC_RM_CONTROL, NV_ESC_RM_FREE, NV_ESC_RM_MAP_MEMORY_DMA,
    NV_ESC_RM_UNMAP_MEMORY_DMA, Nvos00Parameters, Nvos21Parameters, Nvos46Parameters,
    Nvos47Parameters, Nvos54Parameters,
};
use kayfabe_abi::invariant_classes::{CHANNEL_GROUP, VA_SPACE};
use kayfabe_abi::submit::{
    ATTR_CONTIGUOUS_VIDMEM, BIND_PARAMS_SIZE, CeAllocParams, ChannelAllocParams, ENGINE_TYPE_COPY0,
    ENGINE_TYPE_GRAPHICS, GP_ENTRY_SIZE, GpfifoScheduleParams, NV_ESC_RM_MAP_MEMORY,
    NV01_MEMORY_LOCAL_USER, NVA06C_CTRL_CMD_BIND, NVA06C_CTRL_CMD_GPFIFO_SCHEDULE,
    NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN, NvMemoryAllocationParams, Nvos33ParametersWithFd,
    SET_OBJECT, USERD_GP_GET, USERD_GP_PUT, USERMODE_NOTIFY_CHANNEL_PENDING, USERMODE_WINDOW_SIZE,
    WORK_SUBMIT_TOKEN_PARAMS_SIZE, ce, engine_type_copy, fifo, gp_entry, method_header_inc,
};
use kayfabe_arch::ids::{ClassId, ControlCmd, EngineKind, GpuId, GpuVa};
use kayfabe_arch::{CeObjectClass, ChannelClass, HostClasses, UsermodeClass};
use kayfabe_isolate::{
    CeExecutor, CeSource, CeSubCopy, ExportRequest, ExportSource, ExportedBacking, HostHandle,
    IsolateId, RmBackend, RmError,
};
use kayfabe_linux_raw::{
    Backing, CachePolicy, CharDevice, DevDir, HostOffset, HostPageSize, Indirect, RawError,
    VolatileRegion, ioctl, release_fence,
};
use kayfabe_util::leafwitness;
use kayfabe_vmm::SurfaceHandle;
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The opaque status a verb this rung does not implement reports.
///
/// A distinct, greppable value rather than `0` or an RM status: it must never be mistaken
/// for something the driver said. `0x4B46` is `"KF"`.
pub const NOT_ON_THIS_RUNG: u32 = 0x4B46;

/// The first handle this isolate mints for itself.
///
/// ★ **Every isolate starts here**, which is the point. RM mints from one
/// `RS_CLIENT_HANDLE_BASE` for every client (`ogkm-610:
/// src/nvidia/generated/g_resserv_nvoc.h:173`, `ogkm-580: :188`), so two isolates' *n*-th
/// objects genuinely collide in value and are unrelated live objects. The mock had to be
/// taught to imitate that (`host_execution_plane.md` §2.1); here it is what happens.
const FIRST_HANDLE: u32 = 0xCAFE_0001;

/// The handle we *ask* RM for when allocating our root client. RM writes back the one it
/// actually assigned, which is what we keep — asking is not choosing.
const REQUESTED_CLIENT_HANDLE: u32 = 0xCAFE_0000;

/// ★★★ **`hClient` is never guest-derived — as a TYPE, not as a habit.**
///
/// This module exists so that *"the client handle in every RM escape we issue is one this
/// isolate minted"* is a fact the compiler enforces, rather than a property held by the
/// eight call sites that happen to write `self.client` today.
///
/// ## Why this is worth a module (`guest_blast_radius.md` §4 F11)
///
/// [`crate::sandbox`]'s `surrender_privilege` drops **capabilities, not uid**: the user
/// namespace map is the single line `0 <outer_uid> 1`
/// (`crates/kayfabe-linux-raw/src/sandbox_unsafe.rs:596-617`), so on a VMM running as root
/// the isolate's euid **as the host kernel sees it** is 0. RM keys a real check on exactly
/// that value, and the check is an **OR**:
///
/// ```c
/// if ((pClientTokenUser->euid != pCurrentTokenUser->euid) &&
///     (pClientTokenUser->pid  != pCurrentTokenUser->pid))
///     return NV_ERR_INVALID_CLIENT;
/// ```
///
/// (`ogkm-580: src/nvidia/arch/nvalloc/unix/src/os.c:3844-3868`, driven from
/// `_rmclientUserClientSecurityCheck`, `ogkm-580: src/nvidia/src/kernel/rmapi/client.c:447-512`;
/// on by default — the property initialises true independent of any registry key,
/// `ogkm-580: src/nvidia/generated/g_system_nvoc.c:103`). A matching euid **alone** passes.
///
/// ⇒ A local unprivileged process (euid 1000) fails that check against a root-owned RM
/// client. **We pass it.** So RM's cross-user client-handle protection — a real boundary
/// between an unprivileged process and every root GPU client on the host
/// (`nvidia-persistenced`, a display server, another root CUDA process) — does not stand
/// between this isolate and those clients.
///
/// The reason that is a *latent* widening and not a live one is the whole of this type:
/// **there is no way for us to name a client we did not mint.** Before this module that
/// reason was one line — [`RmConnection::raw_alloc`] took a `root: u32` parameter and every
/// caller passed `self.client` — and a single future call site passing anything else would
/// have turned a latent widening into a live one with nothing red anywhere.
///
/// ## The construction that makes it structural
///
/// [`OwnClient`] wraps a `u32` in a **private field inside a private module**, and the only
/// constructor is [`OwnClient::allocate_root`], which *performs* the `NV01_ROOT_CLIENT`
/// allocation and wraps the handle RM wrote back. So the two statements
///
/// * *"an `OwnClient` value exists"*, and
/// * *"this process allocated that client against this control node"*
///
/// are **one statement**. There is no `From<u32>`, no `new`, no `Default`, and the field is
/// unreachable from `rm.rs` itself. A call site cannot name a foreign client because it
/// cannot *build* the only thing the escape-issuing code will accept.
///
/// ⚠ **What this does NOT close, stated plainly.** The ABI parameter blocks
/// ([`Nvos54Parameters::h_client`] and friends) are `u32`, and typing them is a
/// `kayfabe-abi`-wide change this module deliberately does not make. So a *new* struct
/// literal in `rm.rs` could still write `h_client: <some other u32>` and compile. That
/// residue is covered by a **checked** gate rather than a structural one —
/// `tests/own_client_invariant.rs::every_rm_escape_in_rm_rs_stamps_the_isolates_own_client`,
/// which derives its universe by scanning the file rather than from a pinned list. The
/// honest split is: the *parameter* hole is structural, the *literal* hole is tested.
///
/// ⊘ **No run stands behind the security reasoning above.** The euid mechanism is read out
/// of `ogkm-580` source; whether F11's widening is exploitable at all is `[unknown]` and
/// needs a root-owned RM client on the box whose handle value we could guess. Nothing here
/// has been in front of a real driver.
mod own_client {
    use super::{
        CharDevice, Indirect, NOT_ON_THIS_RUNG, NV_ESC_RM_ALLOC, NV_IOCTL_MAGIC, NV01_ROOT_CLIENT,
        Nvos21Parameters, REQUESTED_CLIENT_HANDLE, RmError, ioctl, ioctl_error, status_check,
    };

    /// **The client handle this isolate minted for itself, and the only kind of client
    /// handle any RM escape in this crate will accept.**
    ///
    /// Construct it with [`OwnClient::allocate_root`] — which is the `NV01_ROOT_CLIENT`
    /// allocation, not a wrapper around it. See the module docs for why that identity is
    /// the point.
    ///
    /// `Copy`, because it is a 32-bit handle and passing it around must not be a reason to
    /// reach for the raw value. **No** `From<u32>`, `new`, `Default`, `FromStr` or
    /// `Deserialize` — every one of those would re-open exactly what this exists to close,
    /// so the absence is deliberate and adding one needs the module docs re-read first.
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub(super) struct OwnClient(u32);

    impl core::fmt::Debug for OwnClient {
        /// Named rather than bare-hex: a handle in a log is only ever interesting relative
        /// to *whose* namespace it came from, and this type's whole content is the answer.
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            write!(f, "OwnClient({:#010x})", self.0)
        }
    }

    impl OwnClient {
        /// **R4 — allocate this isolate's root RM client, and BE the only way to obtain an
        /// [`OwnClient`].**
        ///
        /// `hRoot` is `0` here and nowhere else in the crate: a root-client allocation is
        /// the one escape with no owning client, which is precisely why it is the one
        /// constructor. RM writes the handle it actually assigned back into
        /// `hObjectNew`, and *that* — not [`REQUESTED_CLIENT_HANDLE`] — is what we keep.
        ///
        /// ★ Taking `&CharDevice` rather than `&RmConnection` is load-bearing: it lets the
        /// client be minted **before** the connection struct is built, so there is no
        /// window in which an [`RmConnection`](super::RmConnection) exists carrying a
        /// placeholder client. That placeholder (`client: 0`) is what the previous shape
        /// needed and it was a second way to be wrong.
        pub(super) fn allocate_root(ctl: &CharDevice) -> Result<Self, RmError> {
            let mut arg = [0u8; Nvos21Parameters::SIZE];
            Nvos21Parameters {
                h_root: 0,
                h_object_parent: 0,
                h_object_new: REQUESTED_CLIENT_HANDLE,
                h_class: NV01_ROOT_CLIENT,
                p_alloc_parms: 0,
                params_size: 0,
                status: 0,
            }
            .encode_into(&mut arg)
            .map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?;
            let req = ioctl::readwrite(NV_IOCTL_MAGIC, NV_ESC_RM_ALLOC as u8, arg.len())
                .map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?;
            // No `pAllocParms`: `NV01_ROOT_CLIENT` takes none, so the patch list is empty
            // rather than pointing at a zero-length buffer.
            let mut patches: Vec<Indirect<'_>> = Vec::new();
            ctl.ioctl(req, &mut arg, &mut patches)
                .map_err(|e| ioctl_error(&e))?;
            let out =
                Nvos21Parameters::decode(&arg).map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?;
            status_check(out.status)?;
            Ok(Self(out.h_object_new))
        }

        /// The raw handle, for the one thing it is for: filling an ABI parameter block.
        ///
        /// ⚠ This is an *exit*, not a hole. It hands out the client we minted; it cannot
        /// manufacture one we did not. The direction that would matter — `u32 ->
        /// OwnClient` — does not exist.
        pub(super) fn raw(self) -> u32 {
            self.0
        }
    }
}

use own_client::OwnClient;

/// The shared RM connection: **one per isolate, shared by its whole worker pool**.
///
/// That sharing is the fact `host_execution_plane.md` §0 is about. RM serialises every
/// ioctl on this client's write lock and waits uninterruptibly, so N pool workers issuing
/// concurrently do **not** get N verbs on the wire. The pool buys latency isolation, which
/// is what `DEFAULT_POOL_WORKERS`' own docs already say; wire concurrency comes from having
/// **more clients**, i.e. more isolates.
#[derive(Debug)]
pub struct RmConnection {
    /// The **control** node. `NV_CTL_DEVICE_ONLY` escapes go here — see
    /// [`RmConnection::open`]'s docs for the routing rule and where it is enforced.
    ctl: CharDevice,
    /// The per-GPU node. `NV_ACTUAL_DEVICE_ONLY` escapes go here, and it is held for its
    /// whole life because `REGISTER_FD` binds the *session*.
    gpu: CharDevice,
    /// ★ The `/dev` grant, **held** rather than borrowed. A CPU mapping needs a
    /// *freshly opened* per-GPU node for every single mapping (the driver's mmap context
    /// is one-shot per descriptor — see `kayfabe_abi::submit::NV_ESC_RM_MAP_MEMORY`), and
    /// after `pivot_root` there is no path to re-derive one from. So the connection keeps
    /// the capability it was opened with instead of taking a borrow it cannot outlive.
    dev: DevDir,
    /// Which GPU node to open for those mappings. Kept as the index rather than as a name
    /// so the naming rule lives in exactly one place.
    gpu_index: u32,
    /// ★★★ **F11's invariant, as a type.** Not a `u32`: see [`mod@own_client`]. The only
    /// value that can be here is one [`OwnClient::allocate_root`] produced, so every
    /// escape this connection issues names a client this isolate minted.
    client: OwnClient,
    device: u32,
    subdevice: u32,
    /// The **host** driver's version string, as its frontend reported it.
    ///
    /// ★ It got here by passing [`host_version_gate`], so its presence means the interval
    /// check succeeded — but nothing downstream reads it to *select* anything, and that is
    /// the honest state of the host axis rather than an omission. See the module docs.
    version: String,
    objects: Mutex<Objects>,
    /// ★ The CPU mappings, in their **own** mutex rather than inside [`Objects`].
    ///
    /// Two reasons, and the second is the real one. (a) [`Objects`] is copied out from
    /// under its lock by every accessor, and a mapping is not copyable. (b) A ring access
    /// is a *store into a page hardware reads*; it must not be serialised behind the handle
    /// table, and the handle table must not be held across it. Two locks, each held for one
    /// kind of thing, is the R3 lock-rank discipline rather than a convenience.
    rings: Mutex<BTreeMap<u32, ChannelRings>>,
    /// ★★★ The **doorbell window** — a [`HostClasses::usermode`] object and its CPU mapping,
    /// established once at [`RmConnection::open`] and immutable afterwards.
    ///
    /// A `Result`, deliberately, and it is the honest shape rather than a convenience:
    ///
    /// - It is **not** `Mutex<Option<…>>`. Building it lazily would mean issuing an
    ///   `NV_ESC_RM_ALLOC`, an `openat`, an `NV_ESC_RM_MAP_MEMORY` and an `mmap` **while
    ///   holding a ranked lock**, which R1 forbids with no exception (the same rule that
    ///   makes `forget_rings` drop its mappings outside the lock). Immutable-after-open
    ///   needs no lock at all.
    /// - It is **not** a hard failure of `open`. An isolate that never submits work is
    ///   perfectly usable without a doorbell, and making bring-up depend on a BAR mapping
    ///   would turn "this GPU refused one mapping" into "this isolate is stillborn", which
    ///   loses the diagnosis.
    /// - So the error is **kept** and re-reported by [`RmBackend::ring_doorbell`]. The
    ///   refusal names what actually happened at open time instead of a generic
    ///   "unimplemented", which is the difference between *"the sandbox blocked the BAR
    ///   mapping"* and *"nobody wrote this yet"*.
    usermode: Result<UsermodeWindow, RmError>,
    /// ★★★ **The host GPU's class profile** (`#156`) — the three class ids whose correct
    /// value depends on which generation the *host* board is, supplied once at
    /// [`RmConnection::open`] and immutable afterwards.
    ///
    /// Before this field, three `AMPERE_*` constants were spelled at eighteen sites in
    /// this file. That was not merely untidy: two of the three have a **different** id on
    /// a Hopper host and are still *allocatable* there under the Ampere name, so the
    /// wrong one would have been served rather than refused. See
    /// [`kayfabe_arch::HostClasses`] for the table and the sourcing.
    ///
    /// ⊘ It is a **pin, not a probe.** Nothing here asks the device what it is; the
    /// caller passes a profile and today every caller passes the same one
    /// (`kayfabe_chips::pinned_host_classes`, GA10x — the only part any of this has been
    /// measured on). The seam's value is that the decision is now ONE call site instead
    /// of eighteen literals, and that a second generation costs an `impl` and no edit
    /// here. Turning it into a probe is a separate, hardware-requiring increment and is
    /// named in `kayfabe_chips::host_classes`' module docs.
    ///
    /// ★★★ **Which ROLE each site asks this for is now a type, not a convention**
    /// (`#166`). The three methods return [`ChannelClass`], [`UsermodeClass`] and
    /// [`CeObjectClass`], and the four consumers in this file —
    /// [`RmConnection::open_usermode`], [`RmConnection::alloc_gpfifo_channel`],
    /// [`CePush::class_id`] and [`HostRmBackend::alloc_ce_engine_object`] — each name the
    /// role in a parameter or field type, so asking for the wrong one does not compile.
    ///
    /// That was measured to be worth doing rather than assumed: at `36f746a` the bite
    /// harness reported `WIRING: 0/3 caught` — every role swap in this file compiled and
    /// left the whole suite green, and a Hopper host **serves** two of the three wrong
    /// picks with no error to notice.
    classes: &'static dyn HostClasses,
}

/// The [`HostClasses::usermode`] object, the node its mmap context is registered against, and
/// the mapping itself. All three must live exactly as long as each other.
#[derive(Debug)]
struct UsermodeWindow {
    /// The RM object handle. Held so a teardown could free it; nothing frees it today
    /// because the connection outlives every channel by construction.
    _object: u32,
    /// The freshly opened per-GPU node the mmap context is registered against.
    _node: CharDevice,
    /// The 64 KiB BAR0 window. [`kayfabe_abi::submit::USERMODE_NOTIFY_CHANNEL_PENDING`]
    /// is the only offset in it this code ever touches.
    region: VolatileRegion,
}

#[derive(Debug, Default)]
struct Objects {
    next: u32,
    /// `child -> parent`, because `NV_ESC_RM_FREE` needs the parent and the port's `free`
    /// verb does not carry one.
    ///
    /// ★ A gap in the port, found by implementing it. `RmBackend::free(obj)` is
    /// parent-free, which is right for the *core* (a handle names one object) and
    /// insufficient for RM (`NVOS00_PARAMETERS` has `hObjectParent`). The backend therefore
    /// has to remember. It is a small table, but it is state a "stateless forwarder" was
    /// not supposed to need, and it is the reason a handle freed twice is refused here
    /// rather than by the driver.
    parents: BTreeMap<u32, u32>,
    /// ★ `object -> a second object that must die with it`.
    ///
    /// Exactly one producer: [`RmBackend::alloc_vaspace`] allocates **two** RM objects — a
    /// `FERMI_VASPACE_A` and the `NV01_MEMORY_VIRTUAL` range over it that
    /// `NV_ESC_RM_MAP_MEMORY_DMA` will actually name — and the port has one handle to
    /// return. Without this table the address space would leak every time a `Vas` was
    /// freed, and it would leak *silently*, because RM frees the range happily and says
    /// nothing about the space it referenced.
    companions: BTreeMap<u32, u32>,
    /// ★ `channel handle -> the four objects and one address that make it work`.
    ///
    /// Same shape of problem as [`Objects::companions`] and a different answer, because a
    /// channel is not *one* extra object: [`RmBackend::alloc_channel`] returns a single
    /// handle for a group, a channel, two memory objects and a GPU mapping. A second
    /// `companions` entry cannot express that, and chaining them would make the free
    /// ORDER implicit — which for a channel is not a detail (the TSG must outlive the
    /// channel in it).
    ///
    /// It is also what the verbs *after* `alloc_channel` read: `schedule` needs the TSG
    /// (the control is on the group, not the channel), and the ring verbs need the ring's
    /// GPU VA.
    channels: BTreeMap<u32, ChannelParts>,
}

/// One channel's **CPU mappings** — the ring and USERD, as this process sees them.
///
/// Separate from [`ChannelParts`] and not `Copy`, which is the whole reason: a
/// [`VolatileRegion`] owns an `mmap` and a [`CharDevice`] owns a descriptor, and both must
/// be dropped exactly once, in this process, when the channel goes. A handle table can be
/// copied out from under a lock; a mapping cannot.
#[derive(Debug)]
struct ChannelRings {
    /// The node the ring's mmap context was registered against. Held rather than used:
    /// Linux keeps the mapping alive without it, but `NV_ESC_RM_UNMAP_MEMORY` names it, and
    /// a descriptor closed early makes teardown unexpressible.
    _ring_node: CharDevice,
    /// The pushbuffer / GPFIFO / semaphore object.
    ring: VolatileRegion,
    /// The node USERD's mmap context was registered against.
    _userd_node: CharDevice,
    /// USERD — where `GP_GET` (hardware writes) and `GP_PUT` (we write) live.
    userd: VolatileRegion,
}

/// Everything [`RmBackend::alloc_channel`] built, kept because the port hands back one
/// handle and the later verbs need the rest.
#[derive(Debug, Clone, Copy)]
struct ChannelParts {
    /// The `KEPLER_CHANNEL_GROUP_A` this channel lives in. ★ The schedule and bind
    /// controls are issued **here**, not on the channel — see
    /// `kayfabe_abi::submit::NVA06C_CTRL_CMD_GPFIFO_SCHEDULE`.
    tsg: u32,
    /// The device-local object holding the pushbuffer, the GPFIFO ring and the
    /// semaphore.
    ring: u32,
    /// The device-local object holding USERD.
    userd: u32,
    /// The `NV01_MEMORY_VIRTUAL` range [`ChannelParts::ring`] is mapped through — the
    /// handle `NV_ESC_RM_UNMAP_MEMORY_DMA` needs, which is NOT the address space.
    range: u32,
    /// Where RM put the ring in the channel's address space.
    ring_va: u64,
}

/// Size of the device-local object holding a channel's pushbuffer, GPFIFO ring and
/// semaphore. 64 KiB because that is the granularity RM's device-local allocator works
/// in, and because it is what the C's proven host channel uses
/// (`C: src/qemu/nvkvm_gpu_emul.c:9491-9495`).
const RING_OBJECT_BYTES: u64 = 0x1_0000;

/// Offset of the GPFIFO ring within the ring object. A whole page after the pushbuffer:
/// hardware reads both, and keeping them in different pages means a diagnostic dump of
/// one cannot be confused for the other.
const GPFIFO_OFFSET: u64 = 0x1000;

/// GPFIFO entries. A power of two, as RM requires, and small: this ring exists to carry
/// the isolate's own submissions, not the guest's.
const GPFIFO_ENTRIES: u32 = 64;

/// Offset of the pushbuffer within the ring object — the methods hardware fetches.
///
/// The layout below is the C's proven one, offset for offset
/// (`C: src/qemu/nvkvm_gpu_emul.c:9460-9463`: pushbuffer at base, GPFIFO at `+0x1000`,
/// semaphore at `+0x2000`). Keeping the numbers identical is deliberate: when a
/// submission fails, "is our layout the same as the one that worked?" must not be a
/// question anyone has to re-answer.
const PUSHBUFFER_OFFSET: u64 = 0;

/// One pushbuffer slot per GPFIFO entry, so a submission never overwrites methods a
/// previous one may still be being fetched. 64 slots × 64 bytes fits in the page before
/// [`GPFIFO_OFFSET`].
const PUSHBUFFER_SLOT_BYTES: u64 = 64;

/// Offset of the semaphore word **hardware writes** within the ring object.
///
/// ★ A whole page away from both the pushbuffer and the GPFIFO. It has to be somewhere,
/// and putting it adjacent to either would mean a length mistake in one corrupts the
/// other — with the failure appearing as "the semaphore never landed", which is also what
/// a broken doorbell looks like.
const SEMAPHORE_OFFSET: u64 = 0x2000;

/// Everything that can go wrong bringing an RM connection up, with the rung it failed on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BringUpError {
    /// Which ladder rung (module docs).
    pub rung: &'static str,
    /// What happened.
    pub detail: String,
}

impl std::fmt::Display for BringUpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RM bring-up failed at {}: {}", self.rung, self.detail)
    }
}

impl std::error::Error for BringUpError {}

fn rung<T, E: std::fmt::Debug>(r: &'static str, x: Result<T, E>) -> Result<T, BringUpError> {
    x.map_err(|e| BringUpError {
        rung: r,
        // `Debug`, not `Display`: `RmError` deliberately has no `Display` impl (a host
        // status is not prose), and a bring-up diagnosis wants the exact variant anyway.
        detail: format!("{e:?}"),
    })
}

/// An RM status word that is not zero. Carried through as [`RmError::Other`] so a caller
/// sees the driver's own number, never a re-classification.
/// ★ Every number here is read off `ogkm-580:
/// src/common/sdk/nvidia/inc/nvstatuscodes.h`, line cited per arm. The first draft of this
/// function had `0x55` as `INSUFFICIENT_PERMISSIONS` from memory; `0x55` is `NOT_READY` and
/// permissions is `0x1B`. Nothing in the suite could have caught it, because a mock never
/// produces an RM status at all — the only reason it is right now is that hardware returned
/// a status this function had to name.
fn status_check(status: u32) -> Result<(), RmError> {
    match status {
        0 => Ok(()),
        // `NV_ERR_INSUFFICIENT_PERMISSIONS` (`:56`) — lesson L2 says this means "wrong
        // layer", never "gain privilege": a Case-2 GSP-internal control replayed on the
        // host gets exactly this, and the caller must treat it as a design error in the
        // forwarding decision rather than retry.
        0x0000_001B => Err(RmError::InsufficientPermissions),
        // `NV_ERR_INSUFFICIENT_RESOURCES` (`:55`) / `NV_ERR_NO_MEMORY` (`:110`).
        0x0000_001A | 0x0000_0051 => Err(RmError::NoMemory),
        other => Err(RmError::Other(other)),
    }
}

/// The opaque status a **bounds** refusal made by this crate reports.
///
/// ★ Distinct from [`NOT_ON_THIS_RUNG`], and the distinction is not cosmetic. An access
/// that leaves a mapped object is a caller error with an exact answer; *"this rung does not
/// implement that"* is a statement about the port's completeness. Collapsing them — which
/// the first draft of this file did, because every non-syscall `RawError` fell into one
/// catch-all arm — makes a real out-of-range read indistinguishable from an unimplemented
/// verb in every log and every assertion. `0x4B47` is `"KG"`, one past `"KF"`.
pub const NOT_IN_THIS_OBJECT: u32 = 0x4B47;

/// The opaque status a **cache-attribute** refusal reports.
///
/// ★ A third local status, and it exists because a bite produced the second one for
/// something that is not a bound. `RawError::CachePolicyUnattainable` means *"the backing
/// provably cannot have the attribute this call site requires"* — a configuration fault in
/// the pairing of an RM allocation with its mapping, which is neither an out-of-range
/// access nor an unimplemented verb. Reporting it as either is the symptom-not-truth
/// failure §7.3 forbids. `0x4B48` is `"KH"`.
///
/// Unreachable in normal operation: every policy this file passes is a constant. That is
/// the point — if it ever appears, someone changed a mapping's attribute without changing
/// the allocation's, and the status says so.
pub const MAPPING_ATTRIBUTE_REFUSED: u32 = 0x4B48;

/// How long [`RmBackend::ce_copy`] waits for the copy engine's own release semaphore
/// before calling the copy failed.
///
/// ★ Generous on purpose. The failure this bounds is a **wedge**, not a slow copy: a
/// copy that has genuinely started retires in microseconds, so anything that reaches this
/// deadline did not start. Two seconds is long enough that a scheduling hiccup or a busy
/// GPU cannot manufacture a false failure, and short enough that a wedged engine does not
/// look like a hang. The C polls its equivalent self-test for five (`C:
/// src/qemu/nvkvm_gpu_emul.c:9622`).
pub const CE_COPY_TIMEOUT: Duration = Duration::from_secs(2);

/// The opaque status a copy that **never released its semaphore** reports.
///
/// ★★ The single most important refusal in this file. The copy engine writes this word
/// after the copy retires; if it never appears, the bytes did not move — and the *only*
/// alternative to reporting that is returning `Ok(())`, which is the forged completion
/// `mode2_real_forward_not_fake` exists to forbid. It is deliberately **not**
/// [`RmError::Interrupted`]: nothing cancelled it. `0x4B4B` is `"KK"`.
pub const CE_NEVER_RETIRED: u32 = 0x4B4B;

/// The opaque status an **unencodable pushbuffer or GPFIFO entry** reports.
///
/// ★ Distinct from [`NOT_ON_THIS_RUNG`] because it is the opposite kind of statement: the
/// verb exists and the *arguments* cannot be expressed on the wire — a semaphore VA above
/// the GPFIFO's 2^40 ceiling, a method count past the header's 13 bits, a pushbuffer whose
/// length is not a whole number of dwords. Every one of those is a value
/// `kayfabe_abi::submit`'s encoders answer `None` for, and every one of them, if forced
/// through, produces an entry that **runs** — pointing the engine at a truncated address
/// or a wrong method. `0x4B4A` is `"KJ"`.
pub const BAD_ENCODE: u32 = 0x4B4A;

/// The opaque status a **work-submit token that does not fit in 32 bits** reports.
///
/// ★ The port carries the token as a `u64` because a port must not encode NVIDIA's field
/// widths, and the doorbell register is 32 bits wide. Truncating instead of refusing would
/// not error — it would ring **a different channel**, chosen by whichever low bits
/// survived, and the only symptom would be work executing on hardware nobody asked. That
/// is the whole reason this is a named status rather than an `as u32`. `0x4B49` is `"KI"`.
pub const NOT_A_WORK_TOKEN: u32 = 0x4B49;

/// Classify a failure from a mapped region: a bounds refusal, or a syscall.
///
/// Deliberately a different function from [`ioctl_error`]. They share the syscall arm and
/// nothing else, and the reason they are not one function with a flag is that the *default*
/// differs: an unrecognised failure from an ioctl is a rung gap, and an unrecognised
/// failure from a region access is a bound.
fn region_error(e: &RawError) -> RmError {
    match e {
        RawError::Syscall { .. } => ioctl_error(e),
        RawError::CachePolicyUnattainable { .. } => RmError::Other(MAPPING_ATTRIBUTE_REFUSED),
        _ => RmError::Other(NOT_IN_THIS_OBJECT),
    }
}

/// Classify an ioctl-level failure. `EINTR` is **the cancellation signal**, not an error.
fn ioctl_error(e: &RawError) -> RmError {
    match e {
        RawError::Syscall {
            errno: Some(errno), ..
        } if *errno == libc_eintr() => RmError::Interrupted,
        RawError::Syscall {
            errno: Some(errno), ..
        } => RmError::Other(0x8000_0000 | (*errno as u32 & 0xFFFF)),
        _ => RmError::Other(NOT_ON_THIS_RUNG),
    }
}

/// `EINTR`. Named through a function so this file states the constant once — the whole
/// cancellation design turns on recognising it.
const fn libc_eintr() -> i32 {
    4
}

impl RmConnection {
    /// Walk the bring-up ladder against the real driver.
    ///
    /// # Errors
    /// [`BringUpError`], naming the rung.
    ///
    /// `classes` is the **host** board's class profile (`#156`). It is a parameter rather
    /// than a constant because the three ids it carries differ on a Hopper host, and a
    /// caller that has no opinion should pass `kayfabe_chips::pinned_host_classes()`
    /// rather than have this function invent one.
    pub fn open(
        dev: &DevDir,
        gpu: GpuId,
        classes: &'static dyn HostClasses,
    ) -> Result<Self, BringUpError> {
        // R0/R1 — the two nodes, by name, relative to the granted directory. The naming is
        // the C's `dev_id_to_path`: the control node is the literal `nvidiactl`, NOT
        // `nvidia` with an index (`C: src/stub/nvkvm_stub.c:1544-1563`).
        let ctl = rung(
            "R0 openat(nvidiactl)",
            CharDevice::openat(dev, c"nvidiactl"),
        )?;
        let name = rung(
            "R1 device node name",
            CString::new(format!("nvidia{}", gpu.0)).map_err(|e| e.to_string()),
        )?;
        let gpu_node = rung("R1 openat(nvidia<gpu>)", CharDevice::openat(dev, &name))?;

        // ★★★ R2 — the version string, and IT IS NOW A GATE. `cmd = '2'` is the
        // query-non-strict form; `cmd = 0` is STRICT and deliberately returns EINVAL after
        // filling the string in, which the open driver enforces
        // (`C: src/qemu/virtio_nvgpu.c:1157-1170`). See [`host_version_gate`] for why the
        // rung changed and why the answer is a refusal rather than a table.
        let version =
            host_version_gate(read_version(&ctl).as_deref()).map_err(|detail| BringUpError {
                rung: "R2 host driver version",
                detail,
            })?;

        // R3 — bind the device node to the control session. Required, and the failure
        // without it is `0x23 INVALID_CLIENT` rather than anything that names a binding.
        let mut reg = [0u8; 4];
        rung(
            "R3 REGISTER_FD encode",
            RegisterFd {
                ctl_fd: ctl.fd_number(),
            }
            .encode_into(&mut reg),
        )?;
        let req = rung(
            "R3 REGISTER_FD request",
            ioctl::readwrite(NV_IOCTL_MAGIC, NV_ESC_REGISTER_FD, reg.len()),
        )?;
        rung("R3 REGISTER_FD", gpu_node.ioctl(req, &mut reg, &mut []))?;

        let held = rung("R1 hold the /dev grant", dev.try_clone())?;

        // R4 — the root client, minted **before** the connection exists. RM writes back the
        // handle it assigned. ★ Ordered this way so there is never an `RmConnection`
        // carrying a placeholder client: `OwnClient` has no zero value and no constructor
        // but this one, which is the whole of F11's invariant (see `mod own_client`).
        let client = rung("R4 NV01_ROOT_CLIENT", OwnClient::allocate_root(&ctl))?;

        let conn = RmConnection {
            ctl,
            gpu: gpu_node,
            dev: held,
            gpu_index: gpu.0,
            client,
            device: 0,
            subdevice: 0,
            version,
            classes,
            objects: Mutex::new(Objects {
                next: FIRST_HANDLE,
                parents: BTreeMap::new(),
                companions: BTreeMap::new(),
                channels: BTreeMap::new(),
            }),
            rings: Mutex::new(BTreeMap::new()),
            // Filled in below, once there is a subdevice to parent it to.
            usermode: Err(RmError::Other(NOT_ON_THIS_RUNG)),
        };

        // R5 — the device. The parameters are NOT optional: without them RM does not
        // associate the device with a physical GPU and every later control answers
        // NOT_SUPPORTED (`C: tests/integration/test_ioctl_fwd.c:657-668`).
        let mut dev_params = [0u8; Nv0080AllocParameters::SIZE];
        rung(
            "R5 NV0080 encode",
            Nv0080AllocParameters {
                device_id: gpu.0,
                ..Default::default()
            }
            .encode_into(&mut dev_params),
        )?;
        let device = rung(
            "R5 NV01_DEVICE_0",
            conn.raw_alloc(client.raw(), FIRST_HANDLE, NV01_DEVICE_0, &mut dev_params),
        )?;

        // R6 — the subdevice.
        let mut sub_params = [0u8; Nv2080AllocParameters::SIZE];
        rung(
            "R6 NV2080 encode",
            Nv2080AllocParameters { sub_device_id: 0 }.encode_into(&mut sub_params),
        )?;
        let subdevice = rung(
            "R6 NV20_SUBDEVICE_0",
            conn.raw_alloc(device, FIRST_HANDLE + 1, NV20_SUBDEVICE_0, &mut sub_params),
        )?;

        {
            let mut o = conn.objects.lock().expect("objects");
            o.next = FIRST_HANDLE + 2;
            o.parents.insert(device, client.raw());
            o.parents.insert(subdevice, device);
        }
        // ★★ R6b — the doorbell window, attempted here and NOT fatal. See
        // `RmConnection::usermode` for why it is a stored `Result` rather than a rung.
        let conn = RmConnection {
            client,
            device,
            subdevice,
            ..conn
        };
        let usermode = conn.open_usermode(conn.classes.usermode());
        Ok(RmConnection { usermode, ..conn })
    }

    /// ★★★ Allocate the profile's usermode class under the **subdevice** and CPU-map its
    /// 64 KiB
    /// BAR0 window — the mapping whose existence *is* [`RmBackend::ring_doorbell`].
    ///
    /// Three things here are not obvious and each was read out of the driver or the C:
    ///
    /// 1. **The parent is the subdevice, the mapper is the device.** The object is
    ///    allocated under `hSubdevice`, but the `NV_ESC_RM_MAP_MEMORY` that maps it names
    ///    `hDevice` — exactly as the C's proven self-test does
    ///    (`C: src/qemu/nvkvm_gpu_emul.c:9532-9546`, alloc under `SUB`, `mm.h_device =
    ///    DEV`). Passing the subdevice as the mapper is the plausible-looking variant.
    /// 2. **No alloc parameters at all**, not a zeroed struct: `clc561.h` defines the
    ///    class id and nothing else. ★ Still correct on a Hopper host, where the class
    ///    DOES accept optional params: omitting them leaves `bBar1Mapping = NV_FALSE`,
    ///    which selects the same BAR0 register window every earlier usermode class gives
    ///    unconditionally (`ogkm-580:
    ///    src/nvidia/src/kernel/gpu/fifo/usermode_api.c:61-98`).
    /// 3. ★★ **[`CachePolicy::Uncached`], not write-combining.** This is a BAR0
    ///    *register* range, so `nvidia_mmap_helper` takes the `IS_REG_OFFSET` branch and
    ///    calls `nv_encode_caching(…, NV_MEMORY_UNCACHED, NV_MEMORY_TYPE_REGISTERS)`
    ///    unconditionally (`ogkm-580: kernel-open/nvidia/nv-mmap.c:567-574`); the
    ///    write-combining branch two lines down is the *framebuffer* one. Nothing in this
    ///    process can check that claim — `Backing::DeviceFile`'s attainable policy is
    ///    `None` by design, so `require_attainable` cannot refuse a wrong requirement over
    ///    a device fd — which is precisely why the policy had to become a parameter of
    ///    [`RmConnection::map_cpu`] before this call site existed.
    ///
    /// ★★★ **`class` is a parameter, and it is a [`UsermodeClass`] rather than a
    /// `ClassId`** (`#166`). The caller in [`RmConnection::open`] must therefore *name
    /// the role* it is asking the profile for, and asking for the wrong one —
    /// `classes.gpfifo_channel()` — is a **type error**, not a silent mis-allocation
    /// that a Hopper host would have served. Before this signature, that exact swap was
    /// bitten and **nothing in the workspace went red**.
    fn open_usermode(&self, class: UsermodeClass) -> Result<UsermodeWindow, RmError> {
        let want = self.mint();
        let object = self.raw_alloc(self.subdevice, want, class.usermode_id().0, &mut [])?;
        self.remember(object, self.subdevice);
        let (node, region) = self.map_cpu(object, USERMODE_WINDOW_SIZE, CachePolicy::Uncached)?;
        Ok(UsermodeWindow {
            _object: object,
            _node: node,
            region,
        })
    }

    /// ★★★ **The doorbell store**: tell the GPU's host unit that the channel named by
    /// `token` has work.
    ///
    /// Two acts, in this order and no other:
    ///
    /// 1. [`release_fence`] — the ring's stores are into a **write-combining** mapping and
    ///    are therefore *not* ordered against this one. Without the fence the doorbell can
    ///    reach the device before the pushbuffer bytes it announces and the engine runs
    ///    whatever was in the ring before, with no error anywhere.
    /// 2. A single 32-bit store of the token to
    ///    [`USERMODE_NOTIFY_CHANNEL_PENDING`] in the uncached window.
    ///
    /// There is no completion to check and no status to read: the store either happened or
    /// the process took a fault. Everything that can be *known* about a submission is
    /// downstream of it — the semaphore and `GP_GET`.
    fn doorbell(&self, token: u32) -> Result<(), RmError> {
        let window = self.usermode.as_ref().map_err(|e| *e)?;
        release_fence();
        window
            .region
            .store_u32(HostOffset::new(USERMODE_NOTIFY_CHANNEL_PENDING), token)
            .map_err(|e| region_error(&e))
    }

    /// The driver version string the frontend reported, if it answered.
    #[must_use]
    pub fn driver_version(&self) -> &str {
        &self.version
    }

    /// The client handle RM assigned.
    #[must_use]
    pub fn client(&self) -> u32 {
        self.client.raw()
    }

    /// The subdevice handle — the parent of most per-GPU controls.
    #[must_use]
    pub fn subdevice(&self) -> u32 {
        self.subdevice
    }

    /// One `NV_ESC_RM_ALLOC` under **this isolate's own client**, returning the handle RM
    /// ended up assigning.
    ///
    /// ★★★ **There is deliberately no `root` parameter** (`guest_blast_radius.md` §4 F11).
    /// This function used to take `root: u32` and every caller passed `self.client`, which
    /// made *"we never allocate under a client we did not mint"* a property of eight call
    /// sites rather than of the code. Stamping it here means a caller cannot express the
    /// wrong thing: the only client this escape can carry is [`OwnClient`], and the only
    /// way to obtain one is to have allocated it ([`OwnClient::allocate_root`]).
    ///
    /// The one allocation with no owning client — `NV01_ROOT_CLIENT` itself, `hRoot = 0` —
    /// does not come through here at all; it *is* [`OwnClient::allocate_root`].
    fn raw_alloc(
        &self,
        parent: u32,
        want: u32,
        class: u32,
        params: &mut [u8],
    ) -> Result<u32, RmError> {
        let mut arg = [0u8; Nvos21Parameters::SIZE];
        Nvos21Parameters {
            h_root: self.client.raw(),
            h_object_parent: parent,
            h_object_new: want,
            h_class: class,
            p_alloc_parms: 0,
            params_size: params.len() as u32,
            status: 0,
        }
        .encode_into(&mut arg)
        .map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?;
        let req = ioctl::readwrite(NV_IOCTL_MAGIC, NV_ESC_RM_ALLOC as u8, arg.len())
            .map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?;
        // `pAllocParms` at +16. An empty params block means a null pointer, which is what
        // `NV01_ROOT_CLIENT` wants — so the patch list is empty rather than pointing at a
        // zero-length buffer.
        let mut patches: Vec<Indirect<'_>> = Vec::new();
        if !params.is_empty() {
            patches.push(Indirect::new(16, params));
        }
        self.ctl
            .ioctl(req, &mut arg, &mut patches)
            .map_err(|e| ioctl_error(&e))?;
        let out = Nvos21Parameters::decode(&arg).map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?;
        status_check(out.status)?;
        Ok(out.h_object_new)
    }

    /// [`Self::raw_alloc`] for a **GPFIFO channel**, and the only reason it exists is the
    /// type of `class` (`#166`).
    ///
    /// The generic [`Self::raw_alloc`] takes a bare `u32` because it allocates everything
    /// — VA spaces, TSGs, memory, engine objects — so it cannot name a role. That left
    /// the channel's class id a `u32` at its call site, where swapping
    /// `classes.gpfifo_channel()` for `classes.ce_object()` compiled and turned nothing
    /// red (measured: `scripts/bite_host_classes.py`, WIRING 0/3 at `36f746a`). Naming
    /// the role in the *parameter* makes that swap a type error, and the wrapper is one
    /// line — the cheapest place to put a compile-time refusal on this path.
    ///
    /// ⊘ It is deliberately not "alloc anything, but typed": there is one channel class
    /// per generation ([`HostClasses::gpfifo_channel`]), and a GR channel and a CE
    /// channel differ only by `engineType`, so exactly one role can ever reach here.
    fn alloc_gpfifo_channel(
        &self,
        tsg: u32,
        want: u32,
        class: ChannelClass,
        params: &mut [u8],
    ) -> Result<u32, RmError> {
        self.raw_alloc(tsg, want, class.channel_id().0, params)
    }

    /// Mint the next handle value. Taken and released around the ioctl, never held across
    /// one — the leaf-witness assert inside [`CharDevice::ioctl`] would fire if it were.
    fn mint(&self) -> u32 {
        let _leaf = leafwitness::Held::enter();
        let mut o = self.objects.lock().expect("objects");
        let h = o.next;
        o.next = o.next.wrapping_add(1);
        h
    }

    fn remember(&self, child: u32, parent: u32) {
        let _leaf = leafwitness::Held::enter();
        self.objects
            .lock()
            .expect("objects")
            .parents
            .insert(child, parent);
    }

    fn parent_of(&self, child: u32) -> Option<u32> {
        let _leaf = leafwitness::Held::enter();
        self.objects
            .lock()
            .expect("objects")
            .parents
            .get(&child)
            .copied()
    }

    fn pair(&self, object: u32, companion: u32) {
        let _leaf = leafwitness::Held::enter();
        self.objects
            .lock()
            .expect("objects")
            .companions
            .insert(object, companion);
    }

    fn companion_of(&self, object: u32) -> Option<u32> {
        let _leaf = leafwitness::Held::enter();
        self.objects
            .lock()
            .expect("objects")
            .companions
            .remove(&object)
    }

    /// ★ **Peek** the companion, leaving it in place — [`RmConnection::companion_of`]
    /// removes, because its one caller is `free`.
    ///
    /// The distinction is load-bearing rather than stylistic. `alloc_vaspace` returns the
    /// `NV01_MEMORY_VIRTUAL` *range* handle, and a channel group needs the
    /// `FERMI_VASPACE_A` **space** handle it was built over. Reading it with the removing
    /// accessor would make allocating a channel silently un-free the address space: the
    /// range's later `free` would find no companion and leak a live VAS with no handle
    /// anyone can name.
    fn space_of(&self, range: u32) -> Option<u32> {
        let _leaf = leafwitness::Held::enter();
        self.objects
            .lock()
            .expect("objects")
            .companions
            .get(&range)
            .copied()
    }

    fn remember_channel(&self, chan: u32, parts: ChannelParts) {
        let _leaf = leafwitness::Held::enter();
        self.objects
            .lock()
            .expect("objects")
            .channels
            .insert(chan, parts);
    }

    fn channel_parts(&self, chan: u32) -> Option<ChannelParts> {
        let _leaf = leafwitness::Held::enter();
        self.objects
            .lock()
            .expect("objects")
            .channels
            .get(&chan)
            .copied()
    }

    /// Run `f` against one channel's mappings.
    ///
    /// ★ A closure rather than a getter, because a [`VolatileRegion`] cannot leave the
    /// mutex — and that is the correct shape rather than a limitation: the mapping is
    /// shared by every worker on this connection, so the only safe borrow is a scoped one.
    ///
    /// ★★ R1: `f` must not block. Everything passed to it here is a `Relaxed` atomic load
    /// or store into a mapped page — nanoseconds, no syscall — and an ioctl under this lock
    /// would be the violation. The lock is deliberately *not* the handle table's, so a ring
    /// access and an object operation cannot contend.
    fn with_rings<T>(&self, chan: u32, f: impl FnOnce(&ChannelRings) -> T) -> Option<T> {
        let _leaf = leafwitness::Held::enter();
        let rings = self.rings.lock().expect("rings");
        rings.get(&chan).map(f)
    }

    fn remember_rings(&self, chan: u32, rings: ChannelRings) {
        let _leaf = leafwitness::Held::enter();
        self.rings.lock().expect("rings").insert(chan, rings);
    }

    /// Drop one channel's mappings, returning whether there were any. Taken out of the map
    /// and dropped **outside** the lock: `munmap` and `close` are syscalls, and R1 does not
    /// have an exception for teardown.
    fn forget_rings(&self, chan: u32) -> bool {
        let taken = {
            let _leaf = leafwitness::Held::enter();
            self.rings.lock().expect("rings").remove(&chan)
        };
        taken.is_some()
    }

    fn forget_channel(&self, chan: u32) -> Option<ChannelParts> {
        let _leaf = leafwitness::Held::enter();
        self.objects.lock().expect("objects").channels.remove(&chan)
    }

    /// One `NV_ESC_RM_CONTROL` on a raw object handle.
    ///
    /// Split out of [`RmBackend::control`] because the channel verbs issue controls on
    /// objects the *port* never sees — a channel group is an implementation detail of
    /// `alloc_channel`, and there is no [`HostHandle`] for it to narrow.
    fn raw_control(&self, object: u32, cmd: u32, payload: &mut [u8]) -> Result<(), RmError> {
        let mut arg = [0u8; Nvos54Parameters::SIZE];
        Nvos54Parameters {
            h_client: self.client.raw(),
            h_object: object,
            cmd,
            flags: 0,
            params: 0,
            params_size: payload.len() as u32,
            status: 0,
        }
        .encode_into(&mut arg)
        .map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?;
        let req = ioctl::readwrite(NV_IOCTL_MAGIC, NV_ESC_RM_CONTROL as u8, arg.len())
            .map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?;
        let mut patches: Vec<Indirect<'_>> = Vec::new();
        if !payload.is_empty() {
            patches.push(Indirect::new(16, payload));
        }
        self.ctl
            .ioctl(req, &mut arg, &mut patches)
            .map_err(|e| ioctl_error(&e))?;
        let out = Nvos54Parameters::decode(&arg).map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?;
        status_check(out.status)
    }

    /// One `NV_ESC_RM_MAP_MEMORY_DMA`. `at = Some(va)` sets
    /// `NVOS46_FLAGS_DMA_OFFSET_FIXED_TRUE` and demands that address; `at = None` lets RM
    /// choose and reports back where it put the mapping.
    ///
    /// ★★ `None` is **not** a weakening of `#102`. Address identity exists so a
    /// *forwarded* pushbuffer's guest VAs resolve; it says nothing about memory the
    /// isolate allocated for itself, which no guest ever names. A channel's own ring is
    /// exactly that, and demanding a fixed address for it would mean inventing a
    /// host-private VA window — a policy this rung has no way to enforce and every way to
    /// get wrong.
    ///
    /// ★ The residual, named: RM's own VA allocator and our fixed publishes share one
    /// address space, so RM *could* place a ring where a guest later demands a fixed
    /// mapping. That collision surfaces as a refused fixed map with an RM status, which
    /// is loud; it is not silent corruption. A host-private reservation is the real fix
    /// and it belongs with the address plane, not here.
    fn raw_map_dma(
        &self,
        h_dma: u32,
        h_memory: u32,
        len: u64,
        at: Option<u64>,
    ) -> Result<u64, RmError> {
        let mut arg = [0u8; Nvos46Parameters::SIZE];
        Nvos46Parameters {
            h_client: self.client.raw(),
            h_device: self.device,
            h_dma,
            h_memory,
            offset: 0,
            length: len,
            flags: if at.is_some() {
                NVOS46_FLAGS_DMA_OFFSET_FIXED_TRUE
            } else {
                0
            },
            flags2: 0,
            kind_override: 0,
            dma_offset: at.unwrap_or(0),
            status: 0,
        }
        .encode_into(&mut arg)
        .map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?;
        let req = ioctl::readwrite(NV_IOCTL_MAGIC, NV_ESC_RM_MAP_MEMORY_DMA as u8, arg.len())
            .map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?;
        self.ctl
            .ioctl(req, &mut arg, &mut [])
            .map_err(|e| ioctl_error(&e))?;
        let out = Nvos46Parameters::decode(&arg).map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?;
        status_check(out.status)?;
        Ok(out.dma_offset)
    }

    /// One `NV_ESC_RM_UNMAP_MEMORY_DMA`, undoing a [`RmConnection::raw_map_dma`].
    fn raw_unmap_dma(&self, h_dma: u32, gpu_va: u64) -> Result<(), RmError> {
        let mut arg = [0u8; Nvos47Parameters::SIZE];
        Nvos47Parameters {
            h_client: self.client.raw(),
            h_device: self.device,
            h_dma,
            h_memory: 0,
            flags: 0,
            dma_offset: gpu_va,
            size: 0,
            status: 0,
        }
        .encode_into(&mut arg)
        .map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?;
        let req = ioctl::readwrite(NV_IOCTL_MAGIC, NV_ESC_RM_UNMAP_MEMORY_DMA as u8, arg.len())
            .map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?;
        self.ctl
            .ioctl(req, &mut arg, &mut [])
            .map_err(|e| ioctl_error(&e))?;
        let out = Nvos47Parameters::decode(&arg).map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?;
        status_check(out.status)
    }

    /// ★★★ R14 — **CPU-map an RM memory object.** Two syscalls, in an order neither of
    /// them documents, plus a descriptor whose *kind* and whose *freshness* both matter.
    ///
    /// ```text
    ///   node = openat(dev, "nvidia<N>")          a FRESH per-GPU node, per mapping
    ///   NV_ESC_RM_MAP_MEMORY on the CONTROL node, naming node's descriptor NUMBER
    ///   mmap(node, len, offset = 0)
    /// ```
    ///
    /// Four facts, each of which is a different failure if got wrong:
    ///
    /// 1. **The escape goes on the control node** — it is `NV_CTL_DEVICE_ONLY`
    ///    (`ogkm-580: src/nvidia/arch/nvalloc/unix/src/escape.c:521`) — while the `mmap`
    ///    goes on the *device* node. The two halves of one mapping use two different files.
    /// 2. **The descriptor's kind must match what is being mapped.** RM chooses the device
    ///    node's state for an address inside a BAR and the control node's for system memory
    ///    (`ogkm-580: .../osapi.c:2270-2279`); `nv_get_file_private` then refuses a
    ///    descriptor of the other kind (`ogkm-580: kernel-open/nvidia/nv-usermap.c:45-47`).
    ///    Everything mapped here is device-local, so it is always the per-GPU node.
    /// 3. **A fresh node per mapping.** The context is one-shot: a second registration on a
    ///    descriptor that already has one is `NV_ERR_STATE_IN_USE`
    ///    (`ogkm-580: kernel-open/nvidia/nv-usermap.c:53-57`). Reusing `self.gpu` would work
    ///    exactly once and then start failing on the second channel, which is the kind of
    ///    bug that looks like a resource leak.
    /// 4. **The `mmap` offset is zero and the length is exact** — the driver refuses any
    ///    other offset with `EINVAL` and any other length with `ENXIO`
    ///    (`ogkm-580: kernel-open/nvidia/nv-mmap.c:533-536`, `:562-565`).
    ///
    /// The node is returned alongside the region and must be kept: the mapping outlives the
    /// descriptor on Linux, but `NV_ESC_RM_UNMAP_MEMORY` needs it, and dropping it early
    /// makes the teardown unexpressible.
    fn map_cpu(
        &self,
        h_memory: u32,
        len: u64,
        cache: CachePolicy,
    ) -> Result<(CharDevice, VolatileRegion), RmError> {
        let name = CString::new(format!("nvidia{}", self.gpu_index))
            .map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?;
        let node = CharDevice::openat(&self.dev, &name).map_err(|e| ioctl_error(&e))?;

        let mut arg = [0u8; Nvos33ParametersWithFd::SIZE];
        Nvos33ParametersWithFd {
            h_client: self.client.raw(),
            h_device: self.device,
            h_memory,
            offset: 0,
            length: len,
            p_linear_address: 0,
            status: 0,
            flags: 0,
            fd: node.fd_number(),
        }
        .encode_into(&mut arg)
        .map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?;
        let req = ioctl::readwrite(NV_IOCTL_MAGIC, NV_ESC_RM_MAP_MEMORY, arg.len())
            .map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?;
        self.ctl
            .ioctl(req, &mut arg, &mut [])
            .map_err(|e| ioctl_error(&e))?;
        let out =
            Nvos33ParametersWithFd::decode(&arg).map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?;
        status_check(out.status)?;

        // ★ `VolatileRegion`, not `MappedRegion`, and the choice is the type system doing
        // the work: this is memory **hardware writes**, so every access must be a naturally
        // aligned atomic of at most eight bytes. A bulk `read_into` of USERD while the GPU
        // is advancing `GP_GET` is exactly the tearing `VolatileRegion` exists to forbid.
        //
        // ★★ The cache policy is a REQUIREMENT that this layer cannot check, and says so:
        // `Backing::DeviceFile`'s attainable policy is `None` because one NVIDIA descriptor
        // yields three different attributes depending on the range, so
        // `require_attainable` CANNOT refuse a wrong requirement over a device fd. That is
        // exactly why it is a parameter and not a constant here: a hardcoded
        // write-combining is right for a framebuffer object and **wrong for the doorbell**,
        // which is a BAR0 register range NVIDIA maps uncached unconditionally
        // (`ogkm-580: kernel-open/nvidia/nv-mmap.c:567-574` vs `:575-597`), and no test in
        // this workspace could have failed on the difference. The obligation therefore sits
        // with each call site, which is the least dishonest place available.
        let region = VolatileRegion::map(
            Backing::DeviceFile { fd: node.as_fd() },
            len,
            cache,
            HostPageSize::query(),
        )
        .map_err(|e| region_error(&e))?;
        Ok((node, region))
    }

    /// Allocate `len` bytes of **device-local** memory — the only kind a ring, a USERD
    /// block or a semaphore can be built from.
    ///
    /// Not [`RmBackend::alloc_sysmem`]: that verb asks for `MAPPING_NO_MAP`, which makes
    /// the object deliberately un-CPU-mappable. See
    /// `kayfabe_abi::submit::NV01_MEMORY_LOCAL_USER`.
    fn alloc_device_local(&self, len: u64) -> Result<u32, RmError> {
        let mut params = [0u8; NvMemoryAllocationParams::SIZE];
        NvMemoryAllocationParams {
            owner: self.client.raw(),
            kind: 0,
            attr: ATTR_CONTIGUOUS_VIDMEM,
            size: len,
            alignment: len,
        }
        .encode_into(&mut params)
        .map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?;
        let want = self.mint();
        let h = self.raw_alloc(self.device, want, NV01_MEMORY_LOCAL_USER, &mut params)?;
        self.remember(h, self.device);
        Ok(h)
    }

    fn forget(&self, child: u32) {
        let _leaf = leafwitness::Held::enter();
        self.objects.lock().expect("objects").parents.remove(&child);
    }
}

/// ★★★ The subchannel a copy engine's methods go on — **4, and it is load-bearing**
/// (`ogkm-580: kernel-open/nvidia-uvm/cla06fsubch.h:30`,
/// `NVA06F_SUBCHANNEL_COPY_ENGINE`).
///
/// It looks arbitrary and is not. UVM's own comment
/// (`ogkm-580: kernel-open/nvidia-uvm/uvm_maxwell_ce.c:31-36`) says subchannel 4 is
/// *"required to match CE usage on GRCE"* — and GRCE is exactly what this port gets:
/// `NV2080_ENGINE_TYPE_COPY0` was measured (rung 1, `--engines`) to land on **runlist 0**,
/// the graphics runlist, because on this architecture the first two logical copy engines
/// *are* the graphics copy engines.
///
/// ## ★★ MEASURED, RTX 3090 / 580.159.04, 2026-07-30 — and it corrected a wrong diagnosis
///
/// Rung 4's first failure was attributed to `SET_OBJECT` carrying an object handle instead
/// of a class id. That reading was **wrong**, and the bite that was supposed to confirm it
/// disconfirmed it instead. Isolated, one variable at a time:
///
/// | subchannel | `SET_OBJECT` data | result |
/// |---|---|---|
/// | 0 | the class id, correct | `GP_GET` advanced to `GP_PUT`, **semaphore never released, destination unchanged** |
/// | 4 | the class id, correct | 4096 bytes copied, semaphore released |
/// | 4 | a garbage handle (`0xCAFE_000E`) | **4096 bytes copied anyway** |
///
/// So on this part, with the channel group bound to `COPY0`, the *subchannel* routes and
/// `SET_OBJECT`'s data does not appear to. The failure shape is the dangerous one: the
/// entry **is fetched** — `GP_GET` moves — and the methods simply evaporate. No fault, no
/// Xid, no RM status; only the destination not changing says anything happened wrongly.
const CE_SUBCHANNEL: u32 = 4;

/// One copy engine request, as the arguments [`ce_pushbuffer`] needs.
///
/// A struct because six positional arguments of which four are addresses is exactly how
/// a source and a destination get swapped.
#[derive(Debug, Clone, Copy)]
struct CePush {
    /// The **class id** for `SET_OBJECT` — the host profile's
    /// [`HostClasses::ce_object`] (`0xc7b5` on the pinned GA10x profile, `0xc8b5` on a
    /// Hopper host), and **not** the engine object's handle.
    ///
    /// `NVC56F_SET_OBJECT_NVCLASS` is bits `15:0` of the data word
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc56f.h:68-71`), i.e. a *class
    /// number*, and it is what UVM sends
    /// (`ogkm-580: kernel-open/nvidia-uvm/uvm_maxwell_ce.c:36`, `rm_info.ceClass`).
    ///
    /// ★ **Honest limit: this field was NOT observed to matter.** A deliberately wrong
    /// value here still produced a correct 4096-byte copy on RTX 3090 / 580.159.04 — see
    /// [`CE_SUBCHANNEL`] for the table. The class is sent because that is what the
    /// encoding and the driver's own client say it is, not because a measurement here
    /// distinguishes it, and claiming otherwise would be attributing a green run to the
    /// wrong cause.
    ///
    /// ★★ **Typed [`CeObjectClass`], not `u32`** (`#166`). This field is the pushbuffer
    /// half of the role wiring, and the bite that put `classes.gpfifo_channel()` here
    /// used to compile and stay green. It cannot now: the only value that fits is one a
    /// [`HostClasses`] handed back **from the `ce_object` role**.
    class_id: CeObjectClass,
    /// Source GPU VA.
    src: u64,
    /// Destination GPU VA.
    dst: u64,
    /// Bytes.
    len: u32,
    /// Where the engine releases its completion payload.
    sem_va: u64,
    /// The payload it releases.
    payload: u32,
}

/// ★★ Build the pushbuffer for one copy-engine copy — **pure**, so it is testable with no
/// GPU and no RM connection, which is the only part of rung 4 that can be.
///
/// Five method runs, in submission order:
///
/// 1. `SET_OBJECT` — binds the engine object to subchannel 0. Without it the subchannel
///    holds whatever it last held, and the address methods below go to that class.
/// 2. `OFFSET_IN_UPPER … OFFSET_OUT_LOWER` — four consecutive dwords, so one header.
/// 3. `LINE_LENGTH_IN`, `LINE_COUNT` — a pair. With `MULTI_LINE_ENABLE_FALSE`,
///    `LINE_LENGTH_IN` is a **byte** count and `LINE_COUNT` is 1.
/// 4. `SET_SEMAPHORE_A/B/PAYLOAD` — a run of three. `_A` is address bits 48:32, `_B` is
///    31:0 (`ogkm-580: clc7b5.h:47-52`), which is the reverse of the host-FIFO
///    semaphore's LO/HI order and is a real trap.
/// 5. `LAUNCH_DMA` — the flags, last, because it is what starts the copy.
///
/// ★ The address `_UPPER` fields are **17 bits** (`clc7b5.h:162`), not eight like the
/// GPFIFO entry's. The check is still here because a truncated destination is a copy into
/// somebody else's page, and it succeeds.
fn ce_pushbuffer(p: CePush) -> Result<Vec<u32>, RmError> {
    let bad = || RmError::Other(BAD_ENCODE);
    for va in [p.src, p.dst, p.sem_va] {
        if va >> 49 != 0 {
            return Err(bad());
        }
    }
    if !p.sem_va.is_multiple_of(4) {
        return Err(bad());
    }
    let flags = ce::LAUNCH_TRANSFER_NON_PIPELINED
        | ce::LAUNCH_FLUSH_ENABLE
        | ce::LAUNCH_SEMAPHORE_RELEASE_ONE_WORD
        | ce::LAUNCH_SRC_PITCH
        | ce::LAUNCH_DST_PITCH
        | ce::LAUNCH_MULTI_LINE_DISABLE
        | ce::LAUNCH_SRC_VIRTUAL
        | ce::LAUNCH_DST_VIRTUAL;
    let sub = CE_SUBCHANNEL;
    Ok(vec![
        method_header_inc(sub, SET_OBJECT, 1).ok_or_else(bad)?,
        p.class_id.ce_object_id().0,
        method_header_inc(sub, ce::OFFSET_IN_UPPER, 4).ok_or_else(bad)?,
        (p.src >> 32) as u32,
        (p.src & 0xFFFF_FFFF) as u32,
        (p.dst >> 32) as u32,
        (p.dst & 0xFFFF_FFFF) as u32,
        method_header_inc(sub, ce::LINE_LENGTH_IN, 2).ok_or_else(bad)?,
        p.len,
        1,
        method_header_inc(sub, ce::SET_SEMAPHORE_A, 3).ok_or_else(bad)?,
        (p.sem_va >> 32) as u32,
        (p.sem_va & 0xFFFF_FFFF) as u32,
        p.payload,
        method_header_inc(sub, ce::LAUNCH_DMA, 1).ok_or_else(bad)?,
        flags,
    ])
}

/// The runlist an [`EngineKind`] channel belongs on, as an `NV2080_ENGINE_TYPE_*`.
///
/// ★★ **This function is the seam audit's GR-1**, and the reason the port makes `engine`
/// an argument of `alloc_channel` rather than something the adapter guesses. There is
/// exactly ONE channel class per architecture — a graphics channel and a copy channel are
/// both [`HostClasses::gpfifo_channel`] — so this value is the *only* thing that decides which
/// runlist the channel lands on. The C's proven failure is `engineType = 0`: the channel
/// binds to runlist 0, the schedule answers `NV_ERR_NOT_READY`, and the visible symptom is
/// `cuCtxCreate` returning 401 several layers away (`dma_copy_class_alloc_params`).
///
/// [`EngineKind::Other`] gets **no answer**, deliberately: an engine the core routes but
/// does not interpret has no runlist this table can name, and picking one would be picking
/// wrongly and silently. Its caller refuses.
///
/// ## ★★★ MEASURED on RTX 3060 / 580.159.04, because the first run looked like the bug
///
/// A CE channel and a GR channel both came back with **runlist 0**, which is precisely
/// what `engineType = 0` looks like — so the sweep below was run before believing either
/// reading (`--engines`, `R13b`). The engine type in the alloc params was varied and the
/// runlist read out of the work-submit token:
///
/// | `NV2080_ENGINE_TYPE_COPY(i)` | runlist |
/// |---|---|
/// | 0, 1 | **0** — the same runlist as GR |
/// | 2 | 1 |
/// | 3 | 2 |
/// | 4 | 8 |
/// | 5 and up | refused, RM status `0x57` |
///
/// So `engineType` **does** route — five distinct outcomes and a refusal past the end —
/// and *"CE0 is on runlist 0"* is a fact about this part, not a symptom: on this
/// architecture the first two logical copy engines are the graphics copy engines and share
/// the graphics runlist. The C's proven host channel measured the same thing and did not
/// remark on it (its token was `0xc` = runlist 0, chid 12,
/// `C: docs/design/mode2_dataplane_architecture.md:148-167`).
///
/// ★ The consequence worth stating: an isolate's CE traffic and its GR traffic currently
/// contend for one runlist. Choosing CE2+ would separate them, and *that* is a scheduling
/// decision with a cost (those engines are further from the GR context's memory) which
/// nothing at this rung is in a position to make. Recorded rather than guessed at.
fn engine_type_for(engine: EngineKind) -> Option<u32> {
    match engine {
        // GR runs both compute and graphics contexts; the distinction is the context, not
        // the runlist, so both map to the same engine type.
        EngineKind::GrCompute | EngineKind::GrGraphics => Some(ENGINE_TYPE_GRAPHICS),
        // ★ Index 0, which is what the C's proven host channel uses
        // (`C: src/qemu/nvkvm_gpu_emul.c:9509`) — see the table above for what that
        // costs and what it does not.
        EngineKind::Ce => engine_type_copy(0),
        // Named rather than folded into `Other`: these are engines with real
        // `NV2080_ENGINE_TYPE_*` values that this port has never allocated a channel on,
        // so the honest answer is "not on this rung", not a number read off a header and
        // never sent.
        EngineKind::NvEnc | EngineKind::NvDec | EngineKind::Other => None,
    }
}

/// ★★★ R2's **decision**, separated from R2's ioctl so the whole gate is testable.
///
/// The argument is `Option<&str>` — *"what the frontend said, if it said anything"* — and
/// the `None` arm is inside [`kayfabe_abi::host_driver::check`] rather than here, because
/// this is the one function that could turn a failed query into a default, so it never
/// gets the chance. `unwrap_or_default()` is exactly what used to be here, and an empty
/// string is what it produced.
///
/// Returns the reported string on success. The **parsed** version is deliberately dropped:
/// nothing on this side selects on it, and that is the finding
/// (`host_driver_version_pin.md` §2) rather than an oversight — the value of the check is
/// that a host outside the pinned interval stops here instead of being encoded for.
///
/// # Errors
/// The refusal's own prose, ready to be the [`BringUpError::detail`] of rung R2.
fn host_version_gate(reported: Option<&str>) -> Result<String, String> {
    kayfabe_abi::host_driver::check(reported).map_err(|r| r.to_string())?;
    Ok(reported.unwrap_or_default().to_string())
}

/// R2: `NV_ESC_CHECK_VERSION_STR`, query form.
fn read_version(ctl: &CharDevice) -> Option<String> {
    // `nv_ioctl_rm_api_version_t { NvU32 cmd; NvU32 reply; char versionString[64]; }`
    // — `ogkm-580: kernel-open/common/inc/nv-ioctl.h:98-103`.
    const SIZE: usize = 72;
    let mut arg = [0u8; SIZE];
    arg[0] = b'2'; // NV_RM_API_VERSION_CMD_OVERRIDE: query, never the strict form.
    let req = ioctl::readwrite(NV_IOCTL_MAGIC, NV_ESC_CHECK_VERSION_STR, SIZE).ok()?;
    ctl.ioctl(req, &mut arg, &mut []).ok()?;
    let s = &arg[8..];
    let end = s.iter().position(|&b| b == 0).unwrap_or(s.len());
    Some(String::from_utf8_lossy(&s[..end]).into_owned())
}

/// One pool worker's view of the shared connection.
///
/// `&mut self` on every verb, as the port requires, but the *connection* behind it is
/// shared: that is the whole point (see [`RmConnection`]).
#[derive(Debug)]
pub struct HostRmBackend {
    id: IsolateId,
    conn: Arc<RmConnection>,
    /// `channel -> how many entries this worker has published`, so the next submission
    /// takes the next GPFIFO slot instead of overwriting the live one. Per **worker**
    /// rather than per connection: see [`HostRmBackend::next_slot`].
    slots: BTreeMap<u32, u64>,
    /// `host VAS range -> the copy-engine channel this worker built over it`, for
    /// [`RmBackend::ce_copy`]. Built on first use and reused, because a channel is six RM
    /// objects and a copy is one pushbuffer.
    ce_channels: BTreeMap<u32, CeChannel>,
    /// ★ The isolate's table of backings minted for the VMM (`crate::export`). Shared with
    /// every sibling worker: a backing belongs to the isolate, not to the pool slot that
    /// happened to mint it.
    exports: Arc<ChildExports>,
}

/// A copy-engine channel and the engine object bound into its subchannel 0.
#[derive(Debug, Clone, Copy)]
struct CeChannel {
    /// The channel, in this backend's namespace.
    chan: HostHandle,
    /// Its work-submit token.
    token: u64,
    // ★ There is deliberately NO engine-object handle here. The object must be
    // ALLOCATED — it is what gives the channel a copy-engine context — but its handle is
    // never named again: `SET_OBJECT`'s data field is `NVCLASS`, a class number
    // (`ogkm-580: clc56f.h:68-71`). Keeping the handle would invite exactly the mistake
    // that was made and measured here on 2026-07-30. It dies with the channel, as a
    // child of it.
    /// The payload the next copy will release, so two copies on one channel cannot be
    /// confused for each other by a stale word.
    next_payload: u32,
}

/// What one submission produced — the whole evidence bar for rung 3, as data.
///
/// ★ A struct rather than a `bool` because the *interesting* case is the one where every
/// field is legal and the submission did nothing: `semaphore = 0`, `gp_get = 0`,
/// `gp_put = 1` is the `userdOffset` failure, and it reports no error anywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubmitOutcome {
    /// The semaphore word after the wait — the payload if the engine released it.
    pub semaphore: u32,
    /// USERD `GP_GET`: **hardware's** consume cursor. Advancing to meet `gp_put` is the
    /// one fact in this struct that no store of ours can produce.
    pub gp_get: u32,
    /// USERD `GP_PUT`: our produce cursor, read back so the pair is a comparison and not
    /// an assumption.
    pub gp_put: u32,
}

/// What [`HostRmBackend::prove_ce_copy`] observed in **device memory**, before and after.
///
/// ★ The expectations travel with the observations rather than being re-derived by the
/// caller: a diagnostic that computes its own expected value from the same variable it
/// printed is how a copy of the wrong length reads as a pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CeEvidence {
    /// The destination's first word **before** the copy — the sentinel, i.e. `!pattern`.
    pub before: u32,
    /// The destination's first word after, read through an independent second mapping.
    pub after: u32,
    /// The destination's **last** word after. A truncated copy matches `after` and not
    /// this.
    pub after_last: u32,
    /// What `after` must be.
    pub expect_after: u32,
    /// What `after_last` must be.
    pub expect_after_last: u32,
    /// How many bytes were asked for.
    pub bytes: u64,
    /// What the submission itself did — the cursors and the engine's release semaphore.
    /// Carried so a failed copy can be triaged without a second run: see
    /// [`HostRmBackend::ce_copy_outcome`].
    pub submit: SubmitOutcome,
    /// The payload the engine was told to release.
    pub payload: u32,
}

impl CeEvidence {
    /// Did the destination change **from the sentinel to the source's bytes**, first word
    /// and last, *and* did the engine say it had retired?
    ///
    /// ★ The semaphore is part of the conjunction rather than a separate check. Bytes that
    /// match without a release would mean something moved them that we did not ask, and
    /// that is not a pass — it is a different question.
    #[must_use]
    pub fn copied(&self) -> bool {
        self.before != self.expect_after
            && self.after == self.expect_after
            && self.after_last == self.expect_after_last
            && self.submit.semaphore == self.payload
    }
}

impl SubmitOutcome {
    /// Did hardware both **consume** the entry and **release** the semaphore?
    ///
    /// Both, never either: a `GP_GET` that moved with no semaphore means the methods did
    /// not execute, and a semaphore without a `GP_GET` means the word was not written by
    /// the submission this call made.
    #[must_use]
    pub fn landed(&self, payload: u32) -> bool {
        self.semaphore == payload && self.gp_get == self.gp_put
    }
}

impl HostRmBackend {
    /// One worker's backend over `conn`.
    #[must_use]
    pub fn new(id: IsolateId, conn: Arc<RmConnection>, exports: Arc<ChildExports>) -> Self {
        HostRmBackend {
            id,
            conn,
            slots: BTreeMap::new(),
            ce_channels: BTreeMap::new(),
            exports,
        }
    }

    fn stamp(&self, raw: u32) -> HostHandle {
        HostHandle::new(self.id, u64::from(raw))
    }

    /// Narrow a handle back to RM's 32 bits. A value that does not fit was never minted by
    /// this connection, so it is a `BadHandle` **here** rather than an ioctl that would name
    /// a truncated, possibly live, object.
    fn narrow(&self, h: HostHandle) -> Result<u32, RmError> {
        u32::try_from(h.raw()).map_err(|_| RmError::BadHandle(h))
    }
}

impl RmBackend for HostRmBackend {
    fn alloc(
        &mut self,
        parent: HostHandle,
        class: ClassId,
        params: &[u8],
    ) -> Result<HostHandle, RmError> {
        let parent_raw = if parent == HostHandle::NULL {
            self.conn.client.raw()
        } else {
            self.narrow(parent)?
        };
        let want = self.conn.mint();
        let mut params = params.to_vec();
        let h = self
            .conn
            .raw_alloc(parent_raw, want, class.0, &mut params)?;
        self.conn.remember(h, parent_raw);
        Ok(self.stamp(h))
    }

    fn alloc_vaspace(&mut self) -> Result<HostHandle, RmError> {
        // R7. All-zero parameters: index 0, no flags, `vaSize = 0` meaning the default
        // range. Per-`Vas` separation is the property that matters, not the geometry.
        let mut params = [0u8; NvVaspaceAllocationParameters::SIZE];
        NvVaspaceAllocationParameters::default()
            .encode_into(&mut params)
            .map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?;
        let want = self.conn.mint();
        let space = self
            .conn
            .raw_alloc(self.conn.device, want, VA_SPACE, &mut params)?;
        self.conn.remember(space, self.conn.device);

        // ★★ R7b, and it was missing until hardware said so. `NV_ESC_RM_MAP_MEMORY_DMA`'s
        // `hDma` does NOT name an address space — it names an `NV01_MEMORY_VIRTUAL` RANGE
        // within one. Handing it the `FERMI_VASPACE_A` handle is refused with
        // `NV_ERR_INVALID_OBJECT_HANDLE` (0x33), which is what the first end-to-end run of
        // this ladder returned. The C already knew (`mode2_mapdma_primitive`); the port did
        // not, and no mock could have said so because a mock's `map_gpu_va` takes whatever
        // handle it is given.
        //
        // So one `Vas` is TWO host objects, and this verb returns the one the map verb
        // needs. The space rides along as its companion so freeing the range frees both.
        let mut range = [0u8; NvMemoryVirtualAllocationParams::SIZE];
        NvMemoryVirtualAllocationParams {
            offset: 0,
            limit: 0,
            h_va_space: space,
        }
        .encode_into(&mut range)
        .map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?;
        let want = self.conn.mint();
        match self
            .conn
            .raw_alloc(self.conn.device, want, NV01_MEMORY_VIRTUAL, &mut range)
        {
            Ok(h) => {
                self.conn.remember(h, self.conn.device);
                self.conn.pair(h, space);
                Ok(self.stamp(h))
            }
            Err(e) => {
                // The address space exists and the caller will never learn its handle, so
                // it is disposed of HERE rather than becoming an orphan nobody can name.
                let space_handle = self.stamp(space);
                let _ = self.free(space_handle);
                Err(e)
            }
        }
    }

    fn alloc_sysmem(&mut self, len: u64) -> Result<HostHandle, RmError> {
        // R8. A different escape and a different struct — see `Nvos02ParametersWithFd`'s
        // docs for why "allocate memory" cannot ride `NV_ESC_RM_ALLOC`.
        if len == 0 {
            return Err(RmError::NoMemory);
        }
        let want = self.conn.mint();
        let mut arg = [0u8; Nvos02ParametersWithFd::SIZE];
        Nvos02ParametersWithFd {
            h_root: self.conn.client.raw(),
            h_object_parent: self.conn.device,
            h_object_new: want,
            h_class: NV01_MEMORY_SYSTEM,
            flags: NVOS02_FLAGS_LOCATION_PCI
                | NVOS02_FLAGS_PHYSICALITY_NONCONTIGUOUS
                | NVOS02_FLAGS_MAPPING_NO_MAP,
            p_memory: 0,
            pad1: 0,
            // ★ `limit`, not `length`. Off by one BY ABI: RM wants the highest valid
            // offset. Passing `len` here over-allocates by a byte and rounds up a page,
            // which is invisible until a size assertion somewhere else disagrees.
            limit: len - 1,
            status: 0,
            fd: -1,
        }
        .encode_into(&mut arg)
        .map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?;
        let req = ioctl::readwrite(NV_IOCTL_MAGIC, NV_ESC_RM_ALLOC_MEMORY, arg.len())
            .map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?;
        // ★★ THE ROUTING RULE, measured against the driver: `NV_ESC_RM_ALLOC_MEMORY` is
        // `NV_ACTUAL_DEVICE_ONLY` and MUST be issued on the per-GPU node
        // (`ogkm-580: src/nvidia/arch/nvalloc/unix/src/escape.c:328`, macro at `:66` —
        // it refuses `NV_FLAG_CONTROL` outright). Every other escape this file issues is
        // `NV_CTL_DEVICE_ONLY` and must be on the CONTROL node (`:442`, `:634`, `:650`,
        // `:730`, and the two `RM_ALLOC` arms at `:400`/`:415`). Two disjoint sets, and
        // getting one wrong costs `EINVAL` from the frontend before RM ever sees it —
        // which is exactly how this was found, on the first real-hardware run.
        self.conn
            .gpu
            .ioctl(req, &mut arg, &mut [])
            .map_err(|e| ioctl_error(&e))?;
        let out =
            Nvos02ParametersWithFd::decode(&arg).map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?;
        status_check(out.status)?;
        self.conn.remember(out.h_object_new, self.conn.device);
        Ok(self.stamp(out.h_object_new))
    }

    /// ★★★ R13 — a real host channel. Six RM objects, one GPU mapping and two controls,
    /// in an order where every step's failure has a different status.
    ///
    /// ```text
    ///   ring   = NV01_MEMORY_LOCAL_USER (64 KiB)   pushbuffer | GPFIFO | semaphore
    ///   userd  = NV01_MEMORY_LOCAL_USER (64 KiB)   GP_GET / GP_PUT
    ///   map     ring into the Vas (RM chooses the VA)
    ///   tsg    = KEPLER_CHANNEL_GROUP_A   parent = device,  hVASpace = the SPACE
    ///   chan   = <profile>.gpfifo_channel parent = tsg,     hVASpace = 0 (inherits)
    ///   BIND(engineType)                  on the TSG   -- must precede the token
    ///   GET_WORK_SUBMIT_TOKEN             on the CHANNEL
    /// ```
    ///
    /// ## What is deliberately absent
    ///
    /// **No `FERMI_CONTEXT_SHARE_A`.** A context share is how two channels come to share
    /// a subcontext, and an isolate's channel shares with none; the C's proven host
    /// channel leaves `hContextShare` zero too (`C: src/qemu/nvkvm_gpu_emul.c:9517-9522`).
    /// The module docs listed a context share among the missing machinery, and building
    /// it would have been an object nothing reads.
    ///
    /// **No CPU mapping of the ring or USERD**, so nothing here can submit anything yet.
    /// That is the next rung, and separating them is what makes the token below mean
    /// something: it is produced by a channel that exists in hardware, before any of our
    /// own bytes are involved.
    ///
    /// ## The evidence
    ///
    /// The returned token is `(runlistId << 16) | chid`, assigned by RM from the GPU's
    /// channel RAM (`C: docs/design/mode2_doorbell_chid.md:337-345`). We do not compute
    /// it, cannot predict it, and a channel that was never bound to a runlist cannot have
    /// one — the control answers `NV_ERR_INVALID_STATE` (0x40) instead
    /// (`C: src/qemu/nvkvm_gpu_emul.c:9568-9572`).
    ///
    /// ## Unwind
    ///
    /// Every failure after the first allocation frees what it built, newest first. A
    /// channel that half-exists is worse than one that does not: the objects are live in
    /// RM, and the caller has no handle for any of them.
    fn alloc_channel(
        &mut self,
        vas: HostHandle,
        engine: EngineKind,
    ) -> Result<(HostHandle, u64), RmError> {
        // ★ Refused HERE rather than sent as a zero. See `engine_type_for`: a channel with
        // no engine type is not a channel with a default one, it is a channel on runlist 0.
        let engine_type = engine_type_for(engine).ok_or(RmError::Other(NOT_ON_THIS_RUNG))?;
        self.alloc_channel_on(vas, engine_type)
    }

    /// The generic alloc with `parent = chan`, exactly as the port's docs say — the host
    /// verb surface does not grow to add an engine.
    ///
    /// ★ `params` is **not** optional in practice and this rung does not enforce that,
    /// deliberately: which classes need which blob is Axis-A knowledge and belongs to the
    /// lowering, not here. The failure it guards against is nonetheless worth naming,
    /// because it is the C's and it is silent — a copy-engine object whose eight-byte
    /// `NVB0B5_ALLOCATION_PARAMETERS` is not forwarded reads as `engineType = 0`, binds
    /// to runlist 0, and the *schedule* then fails with `NV_ERR_NOT_READY`, several steps
    /// away from the cause (`C: src/abi/nvgpu.h:87-95`, `dma_copy_class_alloc_params`).
    fn alloc_engine_object(
        &mut self,
        chan: HostHandle,
        class: ClassId,
        params: &[u8],
    ) -> Result<HostHandle, RmError> {
        let parent = self.narrow(chan)?;
        // Stricter than RM on purpose: a handle this connection never minted as a channel
        // would still be a legal parent for many classes, and an engine object under a
        // non-channel is a class of bug that surfaces at submission time.
        if self.conn.channel_parts(parent).is_none() {
            return Err(RmError::BadHandle(chan));
        }
        let want = self.conn.mint();
        let mut params = params.to_vec();
        let h = self.conn.raw_alloc(parent, want, class.0, &mut params)?;
        self.conn.remember(h, parent);
        Ok(self.stamp(h))
    }

    /// `NVA06C_CTRL_CMD_GPFIFO_SCHEDULE` with `bEnable = 1`, **on the channel's group**.
    ///
    /// ★ Per-channel, never a one-shot: #12's second context rang off-runlist because
    /// scheduling was a sticky global in the C. Here the group is looked up from the
    /// channel every time, so there is no state to be stale.
    fn schedule(&mut self, chan: HostHandle) -> Result<(), RmError> {
        let raw = self.narrow(chan)?;
        let parts = self
            .conn
            .channel_parts(raw)
            .ok_or(RmError::BadHandle(chan))?;
        let mut params = [0u8; GpfifoScheduleParams::SIZE];
        GpfifoScheduleParams {
            b_enable: 1,
            b_skip_submit: 0,
            b_skip_enable: 0,
        }
        .encode_into(&mut params)
        .map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?;
        self.conn
            .raw_control(parts.tsg, NVA06C_CTRL_CMD_GPFIFO_SCHEDULE, &mut params)
    }

    fn free(&mut self, obj: HostHandle) -> Result<(), RmError> {
        let raw = self.narrow(obj)?;
        // ★★ A `Vas` freed while this worker holds a copy-engine channel over it must take
        // that channel with it. Otherwise the channel outlives its address space (RM would
        // refuse the free of the space, or worse, accept it) and — the sharper failure —
        // the handle value gets recycled, so a later `ce_copy` on a *different* `Vas` with
        // the same raw handle would submit into the dead one's ring. Recycling is exactly
        // the #80 regression class, and it is cheap to close here.
        if let Some(ce) = self.ce_channels.remove(&raw) {
            let _ = self.free(ce.chan);
        }
        // The slot counter is per-channel state with nothing to free; dropping it keeps a
        // recycled handle from inheriting a stale cursor.
        self.slots.remove(&raw);
        // ★★ A channel is six objects and a mapping (see `ChannelParts`), and the ORDER
        // is the reason this is not another `companions` chain. The channel goes first
        // because the group must outlive it; the mapping is torn down before the memory
        // it names; the two memory objects go last.
        //
        // ★ The first error is remembered and the rest of the teardown still runs. A
        // channel whose group refused to free must not also leak 128 KiB of device-local
        // memory and a GPU mapping — and the caller must still hear that something did
        // not free, because the alternative is a silent leak that only shows up as the
        // *next* allocation failing.
        if let Some(parts) = self.conn.forget_channel(raw) {
            // ★ The CPU mappings go FIRST, before any RM object is freed. `munmap` of a
            // device mapping whose backing object RM has already destroyed is the classic
            // use-after-free of this layer, and the driver's own revocation path
            // (`ogkm-580: kernel-open/nvidia/nv-mmap.c:786-800`) exists because it happens.
            self.conn.forget_rings(raw);
            let mut first: Result<(), RmError> = Ok(());
            let mut keep = |r: Result<(), RmError>| {
                if first.is_ok() {
                    first = r;
                }
            };
            keep(self.free_one(raw));
            keep(self.conn.raw_unmap_dma(parts.range, parts.ring_va));
            keep(self.free_one(parts.tsg));
            keep(self.free_one(parts.ring));
            keep(self.free_one(parts.userd));
            return first;
        }
        self.free_one(raw)
    }

    fn control(
        &mut self,
        obj: HostHandle,
        cmd: ControlCmd,
        payload: &mut [u8],
    ) -> Result<(), RmError> {
        let raw = self.narrow(obj)?;
        self.conn.raw_control(raw, cmd.0, payload)
    }

    fn map_gpu_va(
        &mut self,
        vas: HostHandle,
        memory: HostHandle,
        len: u64,
        at: GpuVa,
    ) -> Result<u64, RmError> {
        // R9. `at` is not a hint: `raw_map_dma` with `Some` sets
        // `DMA_OFFSET_FIXED_TRUE` so `dmaOffset` is an **[IN]** parameter and RM places
        // the mapping at the address we name instead of choosing one
        // (`C: nvkvm_gpu_emul.c:7663-7692`, *"the irreducible primitive the whole data
        // plane rests on"*). A forwarded pushbuffer carries guest VAs, and the host MMU
        // walks the host VAS for exactly those numbers.
        //
        // ★ The 64-byte `NVOS46` is the 580.65.06-and-later shape, which is the bench's
        // driver. A host older than that speaks the 56-byte one
        // (`kayfabe_abi::transcribed::Nvos46ParametersPre580`), and selecting between
        // them from the R2 version string is the follow-up this rung does not do.
        let h_dma = self.narrow(vas)?;
        let h_memory = self.narrow(memory)?;
        self.conn.raw_map_dma(h_dma, h_memory, len, Some(at.0))
    }

    fn unmap_gpu_va(&mut self, vas: HostHandle, gpu_va: u64) -> Result<(), RmError> {
        let h_dma = self.narrow(vas)?;
        self.conn.raw_unmap_dma(h_dma, gpu_va)
    }
    /// ★★★ **Rung 3.** Not an ioctl at all: a store into the mapped usermode BAR window
    /// (see `RmConnection::doorbell`).
    ///
    /// ★ The token is **32 bits** — `(runlistId << 16) | chid`, as
    /// `NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN` reports it — while the port carries
    /// it as a `u64` because a port must not encode a vendor's field widths. A value that
    /// does not fit was never a token this connection handed out, so it is refused here
    /// rather than truncated into a store that would ring **some other channel**.
    fn ring_doorbell(&mut self, host_token: u64) -> Result<(), RmError> {
        let token = u32::try_from(host_token).map_err(|_| RmError::Other(NOT_A_WORK_TOKEN))?;
        self.conn.doorbell(token)
    }

    /// ★★★ **Rung 4 — a real copy engine moves the bytes**, for the
    /// [`CeExecutor::HostCe`] arm only.
    ///
    /// The owner's ruling that frames the split: *only a CE whose operands are genuinely
    /// GPGA (physical) must be emulated; everything VA-addressed can be forwarded, because
    /// we control the mapping.* This is the forwarding half, and it is forwarded **as
    /// virtual addresses** — `LAUNCH_DMA` goes out with `SRC_TYPE_VIRTUAL` and
    /// `DST_TYPE_VIRTUAL`, so the engine walks the isolate's own host VAS (`#14`'s
    /// per-`Vas` boundary) and cannot be pointed at physical memory even by a wrong
    /// address. `kayfabe_abi::submit::ce` deliberately does not define the `_PHYSICAL`
    /// constants.
    ///
    /// ## What it does
    ///
    /// A copy-engine channel is built over `vas` on first use and kept
    /// (`CeChannel`): six RM objects, a [`HostClasses::ce_object`] engine object, and a
    /// schedule. Each copy is then one pushbuffer — `SET_OBJECT`, the four address
    /// methods, length and line count, a one-word release semaphore, `LAUNCH_DMA` — one
    /// GPFIFO entry, one doorbell, and a **wait for the engine's own semaphore**.
    ///
    /// ★★ The wait is not optional and is not a convenience. The port's verb is
    /// synchronous, so returning before the engine retires would mean returning `Ok(())`
    /// for bytes that have not moved — the forged completion `mode2_real_forward_not_fake`
    /// forbids, and the guest's next read would be the only thing that ever noticed. A
    /// copy that does not retire inside [`CE_COPY_TIMEOUT`] is [`RmError::Other`] carrying
    /// [`CE_NEVER_RETIRED`], never a success.
    ///
    /// ## Two named refusals, both deliberate
    ///
    /// - [`CeExecutor::Ours`] — needs the isolate's mapping of the *fabricated* aperture,
    ///   which does not exist (the `FbRead` production implementation,
    ///   `eight_blockers_resolved.md` §12.3). Unchanged from the previous rung.
    /// - [`CeSource::Constant`] — a fill is `LAUNCH_DMA` with `REMAP_ENABLE` plus the
    ///   `SET_REMAP_*` method block, which `kayfabe_abi::submit::ce` does not transcribe.
    ///   Emitting a copy from address zero instead would be a plausible success that
    ///   scrubs the destination with whatever is at VA 0.
    fn ce_copy(&mut self, vas: HostHandle, sub: CeSubCopy) -> Result<(), RmError> {
        // ★ The verb answers `Ok`/`Err`; the OUTCOME is what a diagnostic needs to tell
        // "the engine never fetched the entry" from "it fetched it and released nothing",
        // and those two have completely different causes. So the body is one level down
        // and this is the port's projection of it.
        let (outcome, payload) = self.ce_copy_outcome(vas, sub)?;
        if outcome.semaphore == payload {
            Ok(())
        } else {
            Err(RmError::Other(CE_NEVER_RETIRED))
        }
    }
    /// ★★★ NOT ON THIS RUNG — and this refusal is the honest half of `#102` stage C3.
    ///
    /// The seam, the decoder and the production `FbRead`
    /// (`kayfabe_fwd::IsolateFb`) are built and exercised. What is **not** built is this:
    /// the isolate's own VRAM-backed mapping of the fabricated aperture, which needs an
    /// RM allocation of host video memory plus a CPU mapping of it, held for the life of
    /// the isolate. Neither exists on this rung, and neither could be written honestly
    /// without a GPU to run it against.
    ///
    /// ★ **What is owed, precisely.** On a host with a real device: allocate the
    /// fabricated aperture's backing object, CPU-map it inside the isolate, write a known
    /// pattern through [`kayfabe_isolate::CeExecutor::Ours`], and read it back here — the
    /// bytes must be identical, and an address outside the mapped extent must answer
    /// `Ok(false)` rather than zeros. The extent itself (where the aperture begins, how
    /// large it is) is **not written down anywhere in this tree**, which is the second
    /// reason this is a refusal and not a guess.
    ///
    /// Serving zeros instead would be worse than refusing by a wide margin: a page of
    /// zeros decodes as a page-table page that legitimately maps nothing, so a whole
    /// address space would read as empty and every mapping in it would silently vanish —
    /// the same class as the forged completion `mode2_real_forward_not_fake` forbids,
    /// with a longer fuse.
    fn fb_read(&mut self, _phys: u64, _buf: &mut [u8]) -> Result<bool, RmError> {
        Err(RmError::Other(NOT_ON_THIS_RUNG))
    }

    fn export_surface(&mut self, _memory: HostHandle) -> Result<SurfaceHandle, RmError> {
        Err(RmError::Other(NOT_ON_THIS_RUNG))
    }

    /// ★★★ Decision (b): perform the mapping here, hand back memory —
    /// **and refuse the device class BY NAME** (`isolate_vmm_fd_crossing.md` §12).
    ///
    /// ## The arm that succeeds
    ///
    /// [`ExportSource::Fabricated`] mints a sealed `memfd`. That is the whole of *"the
    /// isolate performs the mapping"* for memory we invented: the pages exist, both
    /// processes can map them, and the descriptor that crosses has no `ioctl` handler for
    /// anything. ★ Note it needs **no GPU at all**, which is why this arm is real on this
    /// rung while [`RmBackend::fb_read`] is not: minting a backing and knowing what the
    /// emulated device puts in it are different questions, and only the second one is
    /// blocked.
    ///
    /// ## ⊘ The arm that refuses, and why it is a RESULT rather than a gap
    ///
    /// [`ExportSource::HostDeviceMemory`] is always
    /// [`RmError::NotExportableAsMemory`]. A host GPU page is reachable through exactly
    /// two kinds of object and **neither** can be handed to the VMM as memory:
    ///
    /// 1. `/dev/nvidia<N>` with a registered mapping context — what
    ///    [`RmConnection::map_cpu`] uses, and a **character device**. Crossing it would put
    ///    an RM escape surface in the VMM, where `secInfo.privLevel` is recomputed from the
    ///    **caller** on every escape
    ///    (`ogkm-580: src/nvidia/arch/nvalloc/unix/src/escape.c:304`), i.e. exactly what
    ///    decision (b) exists to stop.
    /// 2. An NVIDIA **dma-buf**, which is *not* an RM surface and would therefore have been
    ///    the escape hatch — except that its CPU mapping is gated on
    ///    `*pbCanMmap = pGpu->getProperty(pGpu, PDB_PROP_GPU_ZERO_FB)`
    ///    (`ogkm-580: src/nvidia/arch/nvalloc/unix/src/osapi.c:5609`), and
    ///    `nv_dma_buf_mmap` refuses outright when that is false
    ///    (`ogkm-580: kernel-open/nvidia/nv-dmabuf.c:1246-1250`). `PDB_PROP_GPU_ZERO_FB` is
    ///    an **integrated**-part property; on every discrete card this project targets a
    ///    dma-buf of device memory cannot be `mmap`ped by the CPU at all.
    ///
    /// ★ And the memory plane refuses the result independently anyway:
    /// `kayfabe_linux_raw::GuestWindow::place` rejects `Backing::DeviceFile` with
    /// `RawError::DeviceBackingNotPlaceable`. Three shut doors, none of them ours to open.
    ///
    /// ⊘ Do not "fix" this by copying the device pages into a `memfd`. A copy is not a
    /// mapping: the guest would read a snapshot of a live aperture, which is the forged-
    /// completion class with a longer fuse.
    fn export_backing(&mut self, want: ExportRequest) -> Result<ExportedBacking, RmError> {
        let ExportSource::Fabricated = want.source else {
            let ExportSource::HostDeviceMemory { memory } = want.source else {
                unreachable!("ExportSource has exactly two variants")
            };
            return Err(RmError::NotExportableAsMemory { memory });
        };
        mint_fabricated(&self.exports, want)
    }
}

/// ★ The fabricated arm, shared by the real backend and the loopback fixture.
///
/// One implementation rather than two, because the arm has **nothing to do with RM**: it
/// is `memfd_create` plus a table insert, and a second copy in the fixture would be a
/// second place for the seal set or the length handling to drift. `host_execution_plane.md`
/// §5's warning is about a fixture that *models* the driver; this is the fixture and the
/// real backend agreeing on a fact neither of them models.
pub(crate) fn mint_fabricated(
    exports: &ChildExports,
    want: ExportRequest,
) -> Result<ExportedBacking, RmError> {
    let token = exports.mint(want.len).map_err(|_| RmError::NoMemory)?;
    Ok(ExportedBacking {
        token,
        offset: 0,
        len: want.len,
        // ★ Echoed rather than narrowed, and that is a *statement*: this backing carries
        // no seal that would make it read-only, so claiming read-only would be a claim the
        // descriptor does not support. When a read-only export is built it will be
        // `F_SEAL_WRITE` on the memfd and this line is where it becomes visible.
        prot: want.prot,
    })
}

impl HostRmBackend {
    /// [`RmBackend::alloc_engine_object`] for the **copy engine**, and — like
    /// [`RmConnection::alloc_gpfifo_channel`] — it exists for the type of `class`
    /// (`#166`).
    ///
    /// The trait verb takes a bare [`ClassId`] and must: it is the generic
    /// engine-object forward, and the guest's own compute/graphics/NVENC classes come
    /// through it from [`crate::child`] as numbers off the wire. That genericity is
    /// correct there and wrong *here*, where the class is not the guest's at all but the
    /// host profile's `ce_object` role. This wrapper is the one call site in the tree
    /// that allocates an engine object from a [`HostClasses`] rather than from guest
    /// intent, so it is the one that can afford to name the role.
    fn alloc_ce_engine_object(
        &mut self,
        chan: HostHandle,
        class: CeObjectClass,
        params: &[u8],
    ) -> Result<HostHandle, RmError> {
        self.alloc_engine_object(chan, class.ce_object_id(), params)
    }

    /// ★★ [`RmBackend::alloc_channel`]'s body, taking the **raw** `NV2080_ENGINE_TYPE_*`
    /// instead of an [`EngineKind`].
    ///
    /// The port's verb takes an abstract engine and that is right: the core must not name
    /// NVIDIA engine numbers. But `engine_type_for`'s table is a claim about hardware, and
    /// the only way to *check* a claim about which runlist an engine type lands on is to
    /// vary the engine type — which the abstract verb cannot express, because two of its
    /// variants map to one number and one maps to none.
    ///
    /// So this is the adapter's own lower entry point, used by the `kayfabe-rm-ladder`
    /// diagnostic's `--engines` sweep. It is `pub` for that reason and no other; nothing
    /// in the core can reach it, because nothing in the core has an engine number to pass.
    ///
    /// # Errors
    /// Whatever RM refuses with, after unwinding whatever it had already built.
    pub fn alloc_channel_on(
        &mut self,
        vas: HostHandle,
        engine_type: u32,
    ) -> Result<(HostHandle, u64), RmError> {
        let range = self.narrow(vas)?;
        // ★ The channel group names the ADDRESS SPACE, and `alloc_vaspace` returned the
        // mappable RANGE over it. A handle we never paired is not a `Vas` at all.
        let space = self.conn.space_of(range).ok_or(RmError::BadHandle(vas))?;

        let ring = self.conn.alloc_device_local(RING_OBJECT_BYTES)?;
        let unwind = |me: &mut Self, objs: &[u32]| {
            for h in objs.iter().rev() {
                let _ = me.free(me.stamp(*h));
            }
        };

        let userd = match self.conn.alloc_device_local(RING_OBJECT_BYTES) {
            Ok(h) => h,
            Err(e) => {
                unwind(self, &[ring]);
                return Err(e);
            }
        };

        // The ring must be resolvable by hardware before a channel may name it. RM picks
        // the address — see `raw_map_dma` for why `None` here does not weaken `#102`.
        let ring_va = match self.conn.raw_map_dma(range, ring, RING_OBJECT_BYTES, None) {
            Ok(va) => va,
            Err(e) => {
                unwind(self, &[ring, userd]);
                return Err(e);
            }
        };

        let mut tsg_params = [0u8; NvChannelGroupAllocationParameters::SIZE];
        let encoded = NvChannelGroupAllocationParameters {
            h_object_error: 0,
            h_object_ecc_error: 0,
            // ★★ Explicit, never zero, and it is the **VASpace object** — measured. A
            // group that leaves this zero asks for the device's default address space,
            // which a forwarding host device does not have; the C measured
            // `NV_ERR_INVALID_OBJECT_HANDLE` (0x33) for exactly that
            // (`C: src/qemu/nvkvm_gpu_emul.c:6828-6836`), and substituting the
            // `NV01_MEMORY_VIRTUAL` range handle here was bitten on this hardware and
            // produced the same 0x33. It is also #14's fix: per-`Vas` separation is a
            // property of this field.
            h_va_space: space,
            // ★★★ **THIS is the field that routes**, measured rather than assumed. Zeroing
            // it makes every allocation fail `NV_ERR_INVALID_ARGUMENT` (0x1F); zeroing the
            // *channel's* `engineType` changes nothing at all, because a channel in a
            // group inherits the group's engine exactly as it inherits its address space.
            // See `kayfabe_abi::submit::ChannelAllocParams::engine_type`.
            engine_type,
            b_is_calling_context_vgpu_plugin: 0,
        }
        .encode_into(&mut tsg_params);
        if encoded.is_err() {
            unwind(self, &[ring, userd]);
            return Err(RmError::Other(NOT_ON_THIS_RUNG));
        }
        let want = self.conn.mint();
        let tsg = match self
            .conn
            .raw_alloc(self.conn.device, want, CHANNEL_GROUP, &mut tsg_params)
        {
            Ok(h) => {
                self.conn.remember(h, self.conn.device);
                h
            }
            Err(e) => {
                unwind(self, &[ring, userd]);
                return Err(e);
            }
        };

        let mut chan_params = [0u8; ChannelAllocParams::SIZE];
        let encoded = ChannelAllocParams {
            h_object_error: 0,
            gp_fifo_offset: ring_va + GPFIFO_OFFSET,
            gp_fifo_entries: GPFIFO_ENTRIES,
            flags: 0,
            // Both zero: a channel in a group inherits the group's subcontext and address
            // space, and naming either again is refused. See `ChannelAllocParams`.
            h_context_share: 0,
            h_va_space: 0,
            h_userd_memory_0: userd,
            // ★★★ ZERO, and it is now a MEASURED requirement rather than a plausible
            // default. Hardware reads USERD at `hUserdMemory[0] + userdOffset[0]`, so a
            // non-zero offset makes it look for `GP_PUT` somewhere our store never lands:
            // it sees `GP_PUT == GP_GET` forever, fetches nothing, and **reports no error
            // at all** — the C's M5.47 root cause
            // (`C: src/qemu/nvkvm_gpu_emul.c:9291-9299`). Bitten on RTX 3090 / 580.159.04
            // on 2026-07-30 by setting it to `0x2000`: every ioctl still returned 0, the
            // channel scheduled, the doorbell rang, and R15 reported `sem 0x00000000
            // GP_GET 0 GP_PUT 1` with R17's destination byte-for-byte unchanged.
            userd_offset_0: 0,
            engine_type,
        }
        .encode_into(&mut chan_params);
        if encoded.is_err() {
            unwind(self, &[ring, userd, tsg]);
            return Err(RmError::Other(NOT_ON_THIS_RUNG));
        }
        let want = self.conn.mint();
        let chan = match self.conn.alloc_gpfifo_channel(
            tsg,
            want,
            self.conn.classes.gpfifo_channel(),
            &mut chan_params,
        ) {
            Ok(h) => {
                self.conn.remember(h, tsg);
                h
            }
            Err(e) => {
                unwind(self, &[ring, userd, tsg]);
                return Err(e);
            }
        };

        // ★★ BIND, on the GROUP, and it must come before the token control.
        let mut bind = [0u8; BIND_PARAMS_SIZE];
        bind.copy_from_slice(&engine_type.to_le_bytes());
        if let Err(e) = self.conn.raw_control(tsg, NVA06C_CTRL_CMD_BIND, &mut bind) {
            unwind(self, &[ring, userd, tsg, chan]);
            return Err(e);
        }

        let mut token = [0u8; WORK_SUBMIT_TOKEN_PARAMS_SIZE];
        if let Err(e) = self.conn.raw_control(
            chan,
            NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN,
            &mut token,
        ) {
            unwind(self, &[ring, userd, tsg, chan]);
            return Err(e);
        }
        let token = u32::from_le_bytes(token);

        // ★★★ R14 — the CPU mappings. Deliberately AFTER the token: everything above is a
        // fact about hardware that no byte of ours has touched, and everything below is
        // this process getting its hands on the channel. Keeping the order means a failure
        // here cannot be confused for a channel that never existed.
        let rings = match (
            // ★ Both write-combining, and that is a claim about what these OBJECTS are,
            // not about what they are used for: they are `NV01_MEMORY_LOCAL_USER`
            // allocations in the framebuffer, so RM's mmap handler takes the
            // write-combining branch for both (`ogkm-580: nv-mmap.c:575-597`). The
            // *uncached* sub-case two lines below it is RM's own USERD window for a
            // channel whose USERD the driver allocated — not this one, which is our own
            // vidmem object handed to the channel via `hUserdMemory[0]`. Claiming
            // uncached here because the word "USERD" appears would be a comfortable
            // guess, and the fence discipline is what makes write-combining survivable.
            self.conn
                .map_cpu(ring, RING_OBJECT_BYTES, CachePolicy::WriteCombining),
            self.conn
                .map_cpu(userd, RING_OBJECT_BYTES, CachePolicy::WriteCombining),
        ) {
            (Ok((ring_node, ring_map)), Ok((userd_node, userd_map))) => ChannelRings {
                _ring_node: ring_node,
                ring: ring_map,
                _userd_node: userd_node,
                userd: userd_map,
            },
            (a, b) => {
                // Either half failing means the channel cannot be submitted to, so it is
                // torn down here rather than handed back as a channel that silently is not
                // one. The first error is the one reported.
                let e = a
                    .err()
                    .or(b.err())
                    .unwrap_or(RmError::Other(NOT_ON_THIS_RUNG));
                unwind(self, &[ring, userd, tsg, chan]);
                return Err(e);
            }
        };
        self.conn.remember_rings(chan, rings);

        self.conn.remember_channel(
            chan,
            ChannelParts {
                tsg,
                ring,
                userd,
                range,
                ring_va,
            },
        );
        Ok((self.stamp(chan), u64::from(token)))
    }

    /// Read USERD's two ring cursors: `(GP_GET, GP_PUT)`.
    ///
    /// ★★ **`GP_GET` is the only word in this whole crate that hardware writes and we do
    /// not.** It is the GPU host unit's consume cursor. Every claim this port can make
    /// about submission bottoms out in watching it move, so it is surfaced as its own
    /// accessor rather than as an offset a caller has to know.
    ///
    /// # Errors
    /// [`RmError::BadHandle`] if `chan` is not a channel of this connection, and whatever
    /// the bounds check refuses with if USERD is somehow shorter than its own cursors.
    pub fn userd_cursors(&self, chan: HostHandle) -> Result<(u32, u32), RmError> {
        let raw = self.narrow(chan)?;
        self.conn
            .with_rings(raw, |r| {
                let get = r
                    .userd
                    .load_u32(HostOffset::new(USERD_GP_GET))
                    .map_err(|e| region_error(&e))?;
                let put = r
                    .userd
                    .load_u32(HostOffset::new(USERD_GP_PUT))
                    .map_err(|e| region_error(&e))?;
                Ok((get, put))
            })
            .unwrap_or(Err(RmError::BadHandle(chan)))
    }

    /// Store one 32-bit word into the channel's ring object at `offset`.
    ///
    /// The ring object holds this channel's pushbuffer, its GPFIFO and its semaphore, all
    /// of which are built a dword at a time. It is `pub` because the ladder builds them and
    /// the next rung's `ring_doorbell` will; nothing in the core can reach it, and nothing
    /// should — the ring is the adapter's own object, not an address the guest names.
    ///
    /// # Errors
    /// [`RmError::BadHandle`], or the bounds refusal if `offset` leaves the object.
    pub fn ring_store_u32(&self, chan: HostHandle, offset: u64, value: u32) -> Result<(), RmError> {
        let raw = self.narrow(chan)?;
        self.conn
            .with_rings(raw, |r| {
                r.ring
                    .store_u32(HostOffset::new(offset), value)
                    .map_err(|e| region_error(&e))
            })
            .unwrap_or(Err(RmError::BadHandle(chan)))
    }

    /// Load one 32-bit word from the channel's ring object at `offset`.
    ///
    /// # Errors
    /// As [`HostRmBackend::ring_store_u32`].
    pub fn ring_load_u32(&self, chan: HostHandle, offset: u64) -> Result<u32, RmError> {
        let raw = self.narrow(chan)?;
        self.conn
            .with_rings(raw, |r| {
                r.ring
                    .load_u32(HostOffset::new(offset))
                    .map_err(|e| region_error(&e))
            })
            .unwrap_or(Err(RmError::BadHandle(chan)))
    }

    /// ★★★ R15 — **submit one host-FIFO semaphore release and watch hardware answer.**
    ///
    /// The smallest thing a GPU can be asked to do that leaves evidence *we cannot
    /// forge*: five consecutive `NVC56F_SEM_*` methods executed by the channel's own front
    /// end. No engine object, no golden context, no compute — so a failure localises to
    /// the submission machinery and to nothing else. It is the C's own host-channel
    /// self-test, method for method (`C: src/qemu/nvkvm_gpu_emul.c:9597-9640`).
    ///
    /// ## ★★ The evidence bar, and why it is two facts and not one
    ///
    /// Returns [`SubmitOutcome`], and a pass requires **both**:
    ///
    /// - `semaphore == payload` — a word in device memory changed to a value only the
    ///   engine could have written there;
    /// - `gp_get` advanced to meet `gp_put` — the GPU's host unit *consumed* the ring
    ///   entry. `GP_GET` is the one word in this crate hardware writes and we do not.
    ///
    /// Either alone is weaker than it looks. A semaphore could in principle be stale from
    /// an earlier submission (hence the sentinel below); `GP_GET` advancing without the
    /// semaphore landing would mean the entry was fetched and the methods did nothing.
    ///
    /// ★ `payload` is chosen by the caller and must be **neither zero** (the sentinel this
    /// writes first) **nor the token** (which we store into the doorbell window and could
    /// alias). A false pass should be unavailable, not merely unlikely.
    ///
    /// ## ★★★ What this is the FIRST live consumer of
    ///
    /// `userdOffset[0]`. Rungs 1 and 2 allocated USERD and mapped it; nothing *read* it.
    /// Here hardware does, at `hUserdMemory[0] + userdOffset[0]`, and a wrong offset makes
    /// the GPU look for `GP_PUT` somewhere our store never lands: it sees
    /// `GP_PUT == GP_GET` forever, fetches nothing, and **reports no error at all**
    /// (the C's M5.47 root cause, `C: src/qemu/nvkvm_gpu_emul.c:9291-9299`). Zero
    /// utilisation and no Xid is the worst failure shape available, which is why this
    /// function returns the cursors rather than a `bool`.
    ///
    /// # Errors
    /// [`RmError::BadHandle`] if `chan` is not this connection's channel, whatever the
    /// doorbell refuses with (including the stored open-time failure of the usermode
    /// mapping), or a bounds refusal if the ring object cannot hold the layout.
    pub fn submit_semaphore_probe(
        &mut self,
        chan: HostHandle,
        token: u64,
        payload: u32,
        timeout: Duration,
    ) -> Result<SubmitOutcome, RmError> {
        let raw = self.narrow(chan)?;
        let parts = self
            .conn
            .channel_parts(raw)
            .ok_or(RmError::BadHandle(chan))?;
        let slot = self.next_slot(raw);
        let sem_va = parts.ring_va + SEMAPHORE_OFFSET;
        let pb_off = PUSHBUFFER_OFFSET + slot * PUSHBUFFER_SLOT_BYTES;
        let pb_va = parts.ring_va + pb_off;

        // The sentinel FIRST, so "the payload is there" cannot be satisfied by whatever
        // the previous submission left behind.
        self.ring_store_u32(chan, SEMAPHORE_OFFSET, 0)?;

        // One incrementing run, SEM_ADDR_LO..SEM_EXECUTE. `SEM_ADDR_HI` is eight bits of
        // address: the 2^40 ceiling `gp_entry` enforces applies here too, and a VA above
        // it would be silently truncated into someone else's page.
        let header =
            method_header_inc(0, fifo::SEM_ADDR_LO, 5).ok_or(RmError::Other(BAD_ENCODE))?;
        if sem_va >= 1 << 40 || !sem_va.is_multiple_of(4) {
            return Err(RmError::Other(BAD_ENCODE));
        }
        let words = [
            header,
            (sem_va & 0xFFFF_FFFC) as u32,
            ((sem_va >> 32) & 0xFF) as u32,
            payload,
            0,
            fifo::SEM_EXECUTE_RELEASE_32BIT,
        ];
        for (i, w) in words.iter().enumerate() {
            self.ring_store_u32(chan, pb_off + 4 * i as u64, *w)?;
        }

        self.submit_entry(chan, pb_va, 4 * words.len() as u64, slot, token)?;
        self.await_semaphore(chan, SEMAPHORE_OFFSET, payload, timeout)
    }

    /// Publish one GPFIFO entry and ring for it: entry → fence → `GP_PUT` → fence →
    /// doorbell.
    ///
    /// ★★ **The two fences are the whole point of the ordering.** The ring is a
    /// write-combining mapping, so its stores are not ordered against each other or
    /// against the doorbell store; without the first fence the GPU can see a `GP_PUT` that
    /// announces methods that have not landed, and without the second it can see a
    /// doorbell for a `GP_PUT` that has not landed. Neither produces an error — the engine
    /// simply executes whatever bytes were there.
    fn submit_entry(
        &mut self,
        chan: HostHandle,
        pb_va: u64,
        pb_len: u64,
        slot: u64,
        token: u64,
    ) -> Result<(), RmError> {
        let entry = gp_entry(pb_va, pb_len).ok_or(RmError::Other(BAD_ENCODE))?;
        let at = GPFIFO_OFFSET + slot * GP_ENTRY_SIZE;
        self.ring_store_u32(chan, at, entry as u32)?;
        self.ring_store_u32(chan, at + 4, (entry >> 32) as u32)?;

        release_fence();
        // ★ `GP_PUT` is an INDEX INTO THE RING, so it wraps with the ring: after the last
        // entry it is 0, not `GPFIFO_ENTRIES`. Writing 64 into a 64-entry ring names an
        // entry that does not exist. Latent rather than live at this rung — nothing here
        // submits 64 times — which is exactly the kind of arithmetic that is wrong for a
        // year and then wrong at scale.
        let put = u32::try_from((slot + 1) % u64::from(GPFIFO_ENTRIES))
            .map_err(|_| RmError::Other(BAD_ENCODE))?;
        self.userd_store_u32(chan, USERD_GP_PUT, put)?;
        release_fence();
        self.ring_doorbell(token)
    }

    /// Poll a semaphore word in the channel's ring object until it holds `payload` or
    /// `timeout` expires, then report it together with both USERD cursors.
    ///
    /// ★ Polling, not waiting on an interrupt: this rung deliberately has no event
    /// delivery, and a poll cannot mistake "we were never woken" for "it never landed".
    /// The cursors are read **after** the loop ends either way, so a timeout returns the
    /// same three facts a success does — which is what makes the `userdOffset` failure
    /// (`sem = 0`, `gp_get = 0`, `gp_put = 1`, no error) legible instead of invisible.
    fn await_semaphore(
        &mut self,
        chan: HostHandle,
        sem_offset: u64,
        payload: u32,
        timeout: Duration,
    ) -> Result<SubmitOutcome, RmError> {
        let deadline = Instant::now() + timeout;
        let mut semaphore = self.ring_load_u32(chan, sem_offset)?;
        while semaphore != payload && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
            semaphore = self.ring_load_u32(chan, sem_offset)?;
        }
        let (gp_get, gp_put) = self.userd_cursors(chan)?;
        Ok(SubmitOutcome {
            semaphore,
            gp_get,
            gp_put,
        })
    }

    /// [`RmBackend::ce_copy`]'s body, returning **what hardware did** rather than a
    /// verdict: the [`SubmitOutcome`] and the payload that was asked for.
    ///
    /// ★ The distinction it preserves is the one that costs a day to re-derive by hand.
    /// `gp_get == gp_put` with no semaphore means the entry was *fetched* and the methods
    /// did nothing — a wrong class in `SET_OBJECT`, a wrong subchannel, a bad operand.
    /// `gp_get == 0` with `gp_put == 1` means it was never fetched at all — USERD, the
    /// token, or the schedule. One error status cannot carry that, and a verb whose only
    /// answer is `Err(CE_NEVER_RETIRED)` cannot be debugged.
    ///
    /// # Errors
    /// As [`RmBackend::ce_copy`], minus the never-retired verdict which is the caller's.
    pub fn ce_copy_outcome(
        &mut self,
        vas: HostHandle,
        sub: CeSubCopy,
    ) -> Result<(SubmitOutcome, u32), RmError> {
        if sub.by != CeExecutor::HostCe {
            return Err(RmError::Other(NOT_ON_THIS_RUNG));
        }
        let CeSource::Address(src) = sub.src else {
            return Err(RmError::Other(NOT_ON_THIS_RUNG));
        };
        // A zero-length sub-copy is a partition bug upstream, and `LINE_LENGTH_IN = 0` is
        // not a no-op on every part — so it is refused rather than issued.
        let len = u32::try_from(sub.len).map_err(|_| RmError::Other(BAD_ENCODE))?;
        if len == 0 {
            return Err(RmError::Other(BAD_ENCODE));
        }

        let ce_chan = self.ce_channel(vas)?;
        let payload = ce_chan.next_payload;
        let chan = ce_chan.chan;
        let raw = self.narrow(chan)?;
        let parts = self
            .conn
            .channel_parts(raw)
            .ok_or(RmError::BadHandle(chan))?;
        let sem_va = parts.ring_va + SEMAPHORE_OFFSET;
        let slot = self.next_slot(raw);
        let pb_off = PUSHBUFFER_OFFSET + slot * PUSHBUFFER_SLOT_BYTES;
        let pb_va = parts.ring_va + pb_off;

        let words = ce_pushbuffer(CePush {
            class_id: self.conn.classes.ce_object(),
            src,
            dst: sub.dst,
            len,
            sem_va,
            payload,
        })?;
        if 4 * words.len() as u64 > PUSHBUFFER_SLOT_BYTES {
            return Err(RmError::Other(BAD_ENCODE));
        }
        self.ring_store_u32(chan, SEMAPHORE_OFFSET, 0)?;
        for (i, w) in words.iter().enumerate() {
            self.ring_store_u32(chan, pb_off + 4 * i as u64, *w)?;
        }
        self.submit_entry(chan, pb_va, 4 * words.len() as u64, slot, ce_chan.token)?;

        let outcome = self.await_semaphore(chan, SEMAPHORE_OFFSET, payload, CE_COPY_TIMEOUT)?;
        let key = self.narrow(vas)?;
        if let Some(c) = self.ce_channels.get_mut(&key) {
            c.next_payload = c.next_payload.wrapping_add(1);
        }
        Ok((outcome, payload))
    }

    /// The copy-engine channel over `vas`, built on first use.
    ///
    /// ★★ Three RM acts in a fixed order, and the order is the C's proven one: the
    /// channel (which carries `engineType = COPY0` into the group, GR-1), then the
    /// [`HostClasses::ce_object`] object **under the channel**, then the schedule. Allocating the
    /// engine object after scheduling is the variant that looks equivalent and is not.
    ///
    /// ★ The engine object's eight alloc bytes are [`CeAllocParams`] with the **same**
    /// ordinal the channel used. Omitting them is the C's `engineType = 0` bug, whose
    /// symptom is `NV_ERR_NOT_READY` from the schedule — two steps from the cause.
    ///
    /// ★★ **Honest limit, measured on RTX 3090 / 580.159.04:** skipping the engine-object
    /// allocation entirely **still produced a correct 4096-byte copy**. With the group
    /// bound to `COPY0`, subchannel 4's methods reach the copy engine without it. It is
    /// allocated anyway — the C allocates it, UVM allocates it, and a channel with no
    /// engine context is not a thing this port wants to depend on being fine — but the
    /// dependency is *asserted from those sources*, not from a bite that fired here. A
    /// step kept for a reason that has not been demonstrated must say so.
    fn ce_channel(&mut self, vas: HostHandle) -> Result<CeChannel, RmError> {
        let key = self.narrow(vas)?;
        if let Some(c) = self.ce_channels.get(&key) {
            return Ok(*c);
        }
        let (chan, token) = self.alloc_channel_on(vas, ENGINE_TYPE_COPY0)?;
        let mut params = [0u8; CeAllocParams::SIZE];
        CeAllocParams {
            version: CeAllocParams::VERSION_1,
            engine_type: ENGINE_TYPE_COPY0,
        }
        .encode_into(&mut params)
        .map_err(|_| RmError::Other(BAD_ENCODE))?;
        if let Err(e) = self.alloc_ce_engine_object(chan, self.conn.classes.ce_object(), &params) {
            let _ = self.free(chan);
            return Err(e);
        }
        if let Err(e) = self.schedule(chan) {
            let _ = self.free(chan);
            return Err(e);
        }
        let c = CeChannel {
            chan,
            token,
            // ★ Starts at 1, never 0: zero is the sentinel written before every
            // submission, so a payload of zero would be satisfied by the sentinel itself.
            next_payload: 1,
        };
        self.ce_channels.insert(key, c);
        Ok(c)
    }

    /// The next GPFIFO slot for `chan`, wrapping at [`GPFIFO_ENTRIES`].
    ///
    /// ★ Kept per **backend**, not per connection: `submit_entry` is the only writer and
    /// it runs under `&mut self`, so the counter needs no lock. A second worker submitting
    /// to the same channel would need one — and would need much more than a counter, which
    /// is why nothing here pretends to support it.
    fn next_slot(&mut self, chan: u32) -> u64 {
        let n = self.slots.entry(chan).or_insert(0);
        let slot = *n % u64::from(GPFIFO_ENTRIES);
        *n += 1;
        slot
    }

    /// Store one 32-bit word into the channel's USERD.
    fn userd_store_u32(&self, chan: HostHandle, offset: u64, value: u32) -> Result<(), RmError> {
        let raw = self.narrow(chan)?;
        self.conn
            .with_rings(raw, |r| {
                r.userd
                    .store_u32(HostOffset::new(offset), value)
                    .map_err(|e| region_error(&e))
            })
            .unwrap_or(Err(RmError::BadHandle(chan)))
    }

    /// ★★★ R14b — **prove the mapped bytes are in the GPU's memory and not in ours.**
    ///
    /// A mapping that succeeds proves nothing. `mmap` of an anonymous page succeeds too,
    /// and a store into it reads back exactly as well — so "I wrote `0xDEADBEEF` and read
    /// `0xDEADBEEF`" is a statement about our own process, which is precisely the class of
    /// evidence `mode2_real_forward_not_fake` rejects.
    ///
    /// What this does instead: write a pattern through the channel's live mapping, then
    /// build a **completely independent second mapping** of the same RM object — a fresh
    /// device node, a fresh mmap context, a kernel-chosen address that has nothing to do
    /// with the first — and read the pattern back through *that*. Two mappings of one
    /// anonymous allocation cannot exist; two mappings of one device object can, and they
    /// alias because the bytes are in the object.
    ///
    /// A control word is read at a second offset in the same pass, so a mapping that
    /// returned a constant would fail even though it "matched".
    ///
    /// Returns `(observed_at_offset, observed_at_control_offset)` read through the second
    /// mapping. The second mapping is dropped before returning: it exists only to be a
    /// different mapping.
    ///
    /// # Errors
    /// [`RmError::BadHandle`], or whatever the driver refuses the second mapping with.
    pub fn prove_ring_is_device_memory(
        &mut self,
        chan: HostHandle,
        offset: u64,
        pattern: u32,
    ) -> Result<(u32, u32), RmError> {
        const CONTROL_OFFSET: u64 = 0x40;
        let raw = self.narrow(chan)?;
        let parts = self
            .conn
            .channel_parts(raw)
            .ok_or(RmError::BadHandle(chan))?;

        self.ring_store_u32(chan, offset, pattern)?;
        // A second, deliberately different value, so "the mapping returns a constant" and
        // "the mapping aliases the object" are distinguishable outcomes.
        self.ring_store_u32(chan, CONTROL_OFFSET, !pattern)?;

        // The independent mapping. `map_cpu` opens its own node, so this shares nothing
        // with the first — not the descriptor, not the mmap context, not the address.
        let (node, second) =
            self.conn
                .map_cpu(parts.ring, RING_OBJECT_BYTES, CachePolicy::WriteCombining)?;
        let a = second
            .load_u32(HostOffset::new(offset))
            .map_err(|e| region_error(&e))?;
        let b = second
            .load_u32(HostOffset::new(CONTROL_OFFSET))
            .map_err(|e| region_error(&e))?;
        drop(second);
        drop(node);
        Ok((a, b))
    }

    /// ★★★ R17 — **prove a copy engine moved bytes of device memory**, by reading the
    /// destination before and after through mappings that are not the ones written.
    ///
    /// Two device-local buffers are allocated, GPU-mapped into `vas` and CPU-mapped. The
    /// source is filled with a per-word pattern; the destination is filled with a
    /// **different** sentinel so that "the copy happened" and "the destination already
    /// looked like that" are distinguishable outcomes. Then one [`RmBackend::ce_copy`]
    /// runs, and the destination is read back through a **freshly opened, independent**
    /// mapping — a different device node, a different mmap context, a kernel-chosen
    /// address — so the answer cannot come from our own page cache.
    ///
    /// ★ The last word is returned as well as the first. A copy engine that wrote only a
    /// header, or a length that got truncated to one dword, would match on word 0 alone.
    ///
    /// Everything it allocates is freed before it returns, including on the error paths
    /// that matter (the copy itself failing still tears down).
    ///
    /// # Errors
    /// Whatever the allocation, the mapping or the copy refuses with.
    pub fn prove_ce_copy(&mut self, vas: HostHandle, pattern: u32) -> Result<CeEvidence, RmError> {
        const BYTES: u64 = 4096;
        const WORDS: u64 = BYTES / 4;
        let range = self.narrow(vas)?;
        let sentinel = !pattern;

        let src = self.conn.alloc_device_local(BYTES)?;
        let dst = match self.conn.alloc_device_local(BYTES) {
            Ok(h) => h,
            Err(e) => {
                let _ = self.free(self.stamp(src));
                return Err(e);
            }
        };
        let mut cleanup: Vec<(u32, Option<u64>)> = vec![(src, None), (dst, None)];
        let mut go = || -> Result<CeEvidence, RmError> {
            let src_va = self.conn.raw_map_dma(range, src, BYTES, None)?;
            cleanup[0].1 = Some(src_va);
            let dst_va = self.conn.raw_map_dma(range, dst, BYTES, None)?;
            cleanup[1].1 = Some(dst_va);

            let (src_node, src_map) = self.conn.map_cpu(src, BYTES, CachePolicy::WriteCombining)?;
            let (dst_node, dst_map) = self.conn.map_cpu(dst, BYTES, CachePolicy::WriteCombining)?;
            for i in 0..WORDS {
                src_map
                    .store_u32(HostOffset::new(i * 4), pattern.wrapping_add(i as u32))
                    .map_err(|e| region_error(&e))?;
                dst_map
                    .store_u32(HostOffset::new(i * 4), sentinel)
                    .map_err(|e| region_error(&e))?;
            }
            let before = dst_map
                .load_u32(HostOffset::new(0))
                .map_err(|e| region_error(&e))?;
            // The stores above are into a write-combining mapping; the engine must not be
            // launched while they are still in a write-combining buffer.
            release_fence();
            drop(dst_map);
            drop(dst_node);
            drop(src_map);
            drop(src_node);

            let (submit, payload) = self.ce_copy_outcome(
                vas,
                CeSubCopy {
                    dst: dst_va,
                    src: CeSource::Address(src_va),
                    len: BYTES,
                    by: CeExecutor::HostCe,
                },
            )?;

            // ★ The read-back mapping is opened AFTER the copy and is not the one the
            // sentinel was written through.
            let (node, second) = self.conn.map_cpu(dst, BYTES, CachePolicy::WriteCombining)?;
            let after = second
                .load_u32(HostOffset::new(0))
                .map_err(|e| region_error(&e))?;
            let after_last = second
                .load_u32(HostOffset::new((WORDS - 1) * 4))
                .map_err(|e| region_error(&e))?;
            drop(second);
            drop(node);
            Ok(CeEvidence {
                before,
                after,
                after_last,
                expect_after: pattern,
                expect_after_last: pattern.wrapping_add(WORDS as u32 - 1),
                bytes: BYTES,
                submit,
                payload,
            })
        };
        let out = go();
        for (h, va) in cleanup.into_iter().rev() {
            if let Some(va) = va {
                let _ = self.conn.raw_unmap_dma(range, va);
            }
            let _ = self.free(self.stamp(h));
        }
        out
    }

    /// Free exactly one RM object — the body [`RmBackend::free`] had before a channel
    /// became six of them.
    fn free_one(&mut self, raw: u32) -> Result<(), RmError> {
        // ★ The port's `free` carries no parent and RM needs one — see `Objects::parents`.
        // A handle we never minted is refused HERE, which is stricter than the host: RM
        // would have destroyed whatever that value names in this client.
        let parent = self
            .conn
            .parent_of(raw)
            .ok_or_else(|| RmError::BadHandle(self.stamp(raw)))?;
        let mut arg = [0u8; Nvos00Parameters::SIZE];
        Nvos00Parameters {
            h_root: self.conn.client.raw(),
            h_object_parent: parent,
            h_object_old: raw,
            status: 0,
        }
        .encode_into(&mut arg)
        .map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?;
        let req = ioctl::readwrite(NV_IOCTL_MAGIC, NV_ESC_RM_FREE as u8, arg.len())
            .map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?;
        self.conn
            .ctl
            .ioctl(req, &mut arg, &mut [])
            .map_err(|e| ioctl_error(&e))?;
        let out = Nvos00Parameters::decode(&arg).map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?;
        status_check(out.status)?;
        self.conn.forget(raw);
        // ★ The companion (see `Objects::companions`): freeing a `Vas`'s mappable range
        // must also free the address space it referenced. Its own failure does not mask
        // this free's success — the object the caller named IS gone — but it is not
        // swallowed either: it comes back as the result of the second free.
        if let Some(companion) = self.conn.companion_of(raw) {
            return self.free(self.stamp(companion));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ★★★ R2's gate, over the arm that used to be silent.
    ///
    /// The path this replaces was `read_version(&ctl).unwrap_or_default()` — a frontend
    /// that did not answer produced `""` and bring-up walked on to encode 580-era offsets
    /// against an unknown driver. The two things this asserts are that the bench's own
    /// driver still gets through, and that **neither** silence nor a nonsense reply is a
    /// way through. The strings are literals rather than anything derived from
    /// `kayfabe_abi::host_driver`'s constants.
    #[test]
    fn r2_admits_the_benchs_host_driver_and_refuses_silence() {
        assert_eq!(
            host_version_gate(Some("580.159.04")).as_deref(),
            Ok("580.159.04"),
            "the driver this crate's encoders were transcribed from must pass R2"
        );
        for absent in [None, Some(""), Some("580")] {
            let refusal = host_version_gate(absent).expect_err("R2 must refuse");
            assert!(
                refusal.contains("refusing rather than assuming 580"),
                "{absent:?} must refuse rather than default: {refusal}"
            );
        }
    }

    /// ★★ The refusal that reaches a human is the one R2 builds, so assert **that** value
    /// — a `BringUpError` naming the rung, carrying the prose whole.
    ///
    /// ⚠ `rung()` cannot be used for it: it formats with `Debug`, which would deliver the
    /// message quoted and backslash-escaped. This asserts the shape a log actually shows.
    #[test]
    fn a_refused_host_driver_arrives_as_a_named_r2_failure() {
        let detail = host_version_gate(Some("610.43.02")).expect_err("610 must refuse");
        let e = BringUpError {
            rung: "R2 host driver version",
            detail,
        };
        let shown = e.to_string();
        assert!(
            shown.starts_with("RM bring-up failed at R2 host driver version: "),
            "names the rung: {shown}"
        );
        assert!(shown.contains("host driver is 610.43.02"), "{shown}");
        assert!(shown.contains("NV_CHANNEL_ALLOC_PARAMS"), "{shown}");
        assert!(!shown.contains('\\'), "not Debug-escaped: {shown}");
    }

    /// The status map, asserted by variant — never `is_err()`.
    #[test]
    fn rm_statuses_map_to_the_named_variants_and_zero_is_success() {
        assert_eq!(status_check(0), Ok(()));
        assert_eq!(status_check(0x1B), Err(RmError::InsufficientPermissions));
        assert_eq!(status_check(0x1A), Err(RmError::NoMemory));
        assert_eq!(status_check(0x51), Err(RmError::NoMemory));
        // ★ The three that must NOT be re-classified, each measured on hardware or read
        // off the header: NOT_READY, INVALID_FLAGS (what a bad NVOS02 flag word returns),
        // INVALID_CLIENT (what an unregistered device node returns).
        assert_eq!(status_check(0x55), Err(RmError::Other(0x55)));
        assert_eq!(status_check(0x29), Err(RmError::Other(0x29)));
        assert_eq!(status_check(0x23), Err(RmError::Other(0x23)));
    }

    /// ★★ `EINTR` is the cancellation signal, and it must not be classified as a host
    /// failure. If this ever returns `Other`, cancellation reports "the host refused" and
    /// §7.3's *"a fault must name the truth, not the symptom"* is broken.
    #[test]
    fn eintr_is_interrupted_and_every_other_errno_is_not() {
        assert_eq!(
            ioctl_error(&RawError::Syscall {
                call: "ioctl",
                errno: Some(4),
            }),
            RmError::Interrupted
        );
        for errno in [1, 5, 12, 22, 25] {
            assert_ne!(
                ioctl_error(&RawError::Syscall {
                    call: "ioctl",
                    errno: Some(errno),
                }),
                RmError::Interrupted,
                "errno {errno} must not read as a cancellation"
            );
        }
    }

    /// A distinct errno produces a distinct opaque status, so a diagnostic can tell
    /// `ENOTTY` from `EINVAL` without this file having to enumerate them.
    #[test]
    fn distinct_errnos_produce_distinct_statuses() {
        let a = ioctl_error(&RawError::Syscall {
            call: "ioctl",
            errno: Some(25),
        });
        let b = ioctl_error(&RawError::Syscall {
            call: "ioctl",
            errno: Some(22),
        });
        assert_ne!(a, b);
        assert_eq!(a, RmError::Other(0x8000_0000 | 25));
    }

    /// The not-implemented status is never zero and never collides with the errno lane —
    /// otherwise "this rung does not do that" would be indistinguishable from a driver
    /// answer.
    #[test]
    fn the_not_on_this_rung_status_cannot_be_mistaken_for_a_driver_answer() {
        assert_ne!(NOT_ON_THIS_RUNG, 0);
        assert_eq!(NOT_ON_THIS_RUNG & 0x8000_0000, 0);
        assert_eq!(
            status_check(NOT_ON_THIS_RUNG),
            Err(RmError::Other(NOT_ON_THIS_RUNG))
        );
    }

    /// ★★ The engine table, by value and by variant. Getting a row wrong here is the C's
    /// wrong-runlist bug and it does NOT fail at the alloc — it fails at the schedule, or
    /// later, or not at all.
    #[test]
    fn the_engine_table_maps_exactly_the_engines_this_rung_can_place() {
        assert_eq!(engine_type_for(EngineKind::GrCompute), Some(1));
        assert_eq!(engine_type_for(EngineKind::GrGraphics), Some(1));
        assert_eq!(engine_type_for(EngineKind::Ce), Some(9));
        // ★ `None`, not a number. An engine this rung cannot place on a runlist must be a
        // refusal: `Some(0)` would be `engineType = 0`, which is the exact bug.
        assert_eq!(engine_type_for(EngineKind::NvEnc), None);
        assert_eq!(engine_type_for(EngineKind::NvDec), None);
        assert_eq!(engine_type_for(EngineKind::Other), None);
    }

    /// ★ A copy channel and a graphics channel must not ask for the same **engine type**,
    /// even though — measured on this part — they land on the same runlist. The two facts
    /// are independent, and conflating them is how "they end up on runlist 0 anyway"
    /// becomes a licence to send one number for both.
    #[test]
    fn copy_and_graphics_request_different_engine_types() {
        assert_ne!(
            engine_type_for(EngineKind::Ce),
            engine_type_for(EngineKind::GrCompute)
        );
        assert_ne!(engine_type_for(EngineKind::Ce), Some(ENGINE_TYPE_GRAPHICS));
    }

    /// The ring geometry: the GPFIFO must fit between its own offset and the semaphore,
    /// and every piece must be inside the object. An arithmetic slip here puts the
    /// semaphore inside the ring, which corrupts entries with payloads.
    #[test]
    fn the_ring_geometry_does_not_overlap_itself_or_leave_the_object() {
        let fifo_bytes = u64::from(GPFIFO_ENTRIES) * 8;
        assert!(GPFIFO_OFFSET + fifo_bytes <= RING_OBJECT_BYTES);
        assert!(
            GPFIFO_ENTRIES.is_power_of_two(),
            "RM requires a power of two"
        );
    }

    /// ★★★ A class id that is **deliberately not a real one** (`#156`).
    ///
    /// [`ce_pushbuffer`]'s contract is that `SET_OBJECT` carries *whatever class the host
    /// profile named*. Feeding it the profile's own answer cannot tell "carried it" from
    /// "hardcoded it" — the encoder would be acting as its own observer, and a mutation
    /// that replaced the parameter with a constant would survive. A value no NVIDIA part
    /// defines can only appear in `w[1]` by having been passed in.
    const NOT_A_REAL_CLASS: u32 = 0x0000_C0DE;

    /// The same value, wearing the **role** [`CePush::class_id`] now demands (`#166`).
    ///
    /// ★ Note what has to be written to get here: `CeObjectClass::new`. There is no way
    /// to hand [`ce_pushbuffer`] a channel or usermode class any more — the field's type
    /// refuses it — so the test can go on checking the *value* is carried while rustc
    /// checks the *role* is right at every production call site.
    fn probe_ce_class() -> CeObjectClass {
        CeObjectClass::new(ClassId(NOT_A_REAL_CLASS))
    }

    /// ★★ The copy-engine pushbuffer, word for word. This is the only part of rung 4 that
    /// can be checked without a GPU, and the two things it pins are the two that failed
    /// silently on hardware: **which subchannel** every header names, and that the source
    /// and destination do not swap.
    #[test]
    fn the_ce_pushbuffer_addresses_the_copy_engine_subchannel_and_does_not_swap_operands() {
        let w = ce_pushbuffer(CePush {
            class_id: probe_ce_class(),
            src: 0x1234_5678_9ABC,
            dst: 0x0000_DEAD_0000,
            len: 4096,
            sem_va: 0x7_0000_2000,
            payload: 7,
        })
        .expect("encodable");

        // Every header names subchannel 4 — bits 15:13. A header on subchannel 0 is the
        // measured silent failure (the entry is fetched and nothing happens).
        for (i, word) in w.iter().enumerate() {
            if *word >> 29 == 1 {
                assert_eq!(
                    (word >> 13) & 0x7,
                    CE_SUBCHANNEL,
                    "header at {i} is on the wrong subchannel"
                );
            }
        }
        // SET_OBJECT carries the CLASS, and the addresses go out in-then-out, hi-then-lo.
        assert_eq!(
            w[1], NOT_A_REAL_CLASS,
            "SET_OBJECT must carry the class the PROFILE named, not one this encoder knows"
        );
        assert_eq!(w[3], 0x1234);
        assert_eq!(w[4], 0x5678_9ABC, "source low");
        assert_eq!(w[5], 0x0000);
        assert_eq!(w[6], 0xDEAD_0000, "destination low");
        assert_eq!(w[8], 4096, "LINE_LENGTH_IN is a BYTE count");
        assert_eq!(w[9], 1, "LINE_COUNT");
        // ★ The CE semaphore is A = HIGH, B = LOW — the REVERSE of the host-FIFO
        // semaphore's LO/HI order, and swapping them writes the payload into a page 4 GiB
        // away that we happen to own.
        assert_eq!(w[11], 0x7, "SET_SEMAPHORE_A is the HIGH bits");
        assert_eq!(w[12], 0x0000_2000, "SET_SEMAPHORE_B is the LOW bits");
        assert_eq!(w[13], 7);
        // The launch flags: virtual on both sides, and a one-word release.
        let flags = w[15];
        assert_eq!(flags & 0b11, ce::LAUNCH_TRANSFER_NON_PIPELINED);
        assert_ne!(flags & ce::LAUNCH_SEMAPHORE_RELEASE_ONE_WORD, 0);
        assert_eq!(flags & (1 << 12), 0, "SRC_TYPE must stay VIRTUAL");
        assert_eq!(flags & (1 << 13), 0, "DST_TYPE must stay VIRTUAL");
        assert_eq!(flags & (1 << 9), 0, "MULTI_LINE must stay disabled");
    }

    /// An address the methods cannot express is refused, never truncated. A truncated
    /// destination is a copy into somebody else's page and it succeeds.
    #[test]
    fn an_inexpressible_copy_operand_is_refused() {
        let base = CePush {
            class_id: probe_ce_class(),
            src: 0,
            dst: 0,
            len: 4,
            sem_va: 0,
            payload: 1,
        };
        for bad in [
            CePush {
                dst: 1 << 49,
                ..base
            },
            CePush {
                src: 1 << 49,
                ..base
            },
            CePush {
                sem_va: 1 << 49,
                ..base
            },
            // A semaphore address that is not dword-aligned: the low bits are not part of
            // the field, so the release would land somewhere else entirely.
            CePush {
                sem_va: 0x1002,
                ..base
            },
        ] {
            assert_eq!(
                ce_pushbuffer(bad).err(),
                Some(RmError::Other(BAD_ENCODE)),
                "{bad:?} must be refused"
            );
        }
        // …and the whole pushbuffer fits in one slot, which the submit path asserts too.
        let w = ce_pushbuffer(base).expect("encodable");
        assert!(4 * w.len() as u64 <= PUSHBUFFER_SLOT_BYTES);
    }

    /// ★★★ The evidence bar, as a predicate. `landed` must require BOTH facts: a
    /// semaphore alone could be stale and a `GP_GET` alone means the methods did nothing.
    #[test]
    fn a_submission_has_landed_only_when_both_facts_hold() {
        let ok = SubmitOutcome {
            semaphore: 0xBEEF,
            gp_get: 1,
            gp_put: 1,
        };
        assert!(ok.landed(0xBEEF));
        assert!(
            !ok.landed(0xBEE0),
            "a different payload is not this submission"
        );
        // The `userdOffset` failure shape, measured on hardware: everything legal, nothing
        // happened, no error anywhere.
        let userd_bug = SubmitOutcome {
            semaphore: 0,
            gp_get: 0,
            gp_put: 1,
        };
        assert!(!userd_bug.landed(0xBEEF));
        // Fetched, but the methods evaporated — the wrong-subchannel shape.
        let fetched_only = SubmitOutcome {
            semaphore: 0,
            gp_get: 1,
            gp_put: 1,
        };
        assert!(!fetched_only.landed(0xBEEF));
    }

    /// ★★ A copy is only proven when the destination CHANGED — the `before` reading is
    /// what makes it non-vacuous, and a destination that already held the answer must not
    /// pass.
    #[test]
    fn a_copy_into_a_destination_that_already_matched_is_not_evidence() {
        let landed = SubmitOutcome {
            semaphore: 3,
            gp_get: 1,
            gp_put: 1,
        };
        let good = CeEvidence {
            before: 0xFFFF_FFFF,
            after: 0xC0FF_EE00,
            after_last: 0xC0FF_F1FF,
            expect_after: 0xC0FF_EE00,
            expect_after_last: 0xC0FF_F1FF,
            bytes: 4096,
            submit: landed,
            payload: 3,
        };
        assert!(good.copied());
        assert!(
            !CeEvidence {
                before: 0xC0FF_EE00,
                ..good
            }
            .copied(),
            "the destination already held the answer"
        );
        assert!(
            !CeEvidence {
                after_last: 0,
                ..good
            }
            .copied(),
            "a copy that moved only the first word is not a copy"
        );
        assert!(
            !CeEvidence {
                submit: SubmitOutcome {
                    semaphore: 0,
                    ..landed
                },
                ..good
            }
            .copied(),
            "bytes without a release is a different question, not a pass"
        );
    }

    /// ★★ The two local refusal statuses must be distinguishable from each other AND
    /// from anything the driver can say. A bounds error reported as `NOT_ON_THIS_RUNG`
    /// reads as "unimplemented", which is how a real out-of-range access gets triaged as
    /// a missing feature.
    #[test]
    fn a_bounds_refusal_is_not_the_unimplemented_status() {
        assert_ne!(NOT_IN_THIS_OBJECT, NOT_ON_THIS_RUNG);
        assert_ne!(NOT_IN_THIS_OBJECT, 0);
        assert_eq!(NOT_IN_THIS_OBJECT & 0x8000_0000, 0);
        assert_eq!(
            region_error(&RawError::OutOfRange {
                offset: 0x1_0000,
                len: 4,
                object_len: 0x1_0000,
            }),
            RmError::Other(NOT_IN_THIS_OBJECT)
        );
        // …and a syscall failure through the SAME function still classifies as one, so the
        // split did not swallow the errno lane.
        assert_eq!(
            region_error(&RawError::Syscall {
                call: "mmap",
                errno: Some(4),
            }),
            RmError::Interrupted
        );
        // ★ …and the THIRD one, which a bite produced: an attribute the backing cannot
        // have is not a bound. All three local statuses are pairwise distinct and none
        // collides with the errno lane, so a triage can tell them apart without reading
        // this file.
        assert_eq!(
            region_error(&RawError::CachePolicyUnattainable {
                requested: kayfabe_linux_raw::CachePolicy::WriteCombining,
                attainable: kayfabe_linux_raw::CachePolicy::WriteBack,
                backing: "a device file",
            }),
            RmError::Other(MAPPING_ATTRIBUTE_REFUSED)
        );
        // ★ Quantified over the LIST, and the list is every local status this file
        // defines: shortening it would weaken the gate with no red test. Adding a status
        // without adding it here is the mistake, and it is a mistake in one place.
        let all = [
            NOT_ON_THIS_RUNG,
            NOT_IN_THIS_OBJECT,
            MAPPING_ATTRIBUTE_REFUSED,
            BAD_ENCODE,
            NOT_A_WORK_TOKEN,
            CE_NEVER_RETIRED,
        ];
        for (i, a) in all.iter().enumerate() {
            assert_ne!(*a, 0, "no local status may be success");
            assert_eq!(
                a & 0x8000_0000,
                0,
                "no local status may enter the errno lane"
            );
            for b in all.iter().skip(i + 1) {
                assert_ne!(a, b, "two local statuses collide");
            }
        }
    }

    /// Every isolate mints from the same base — the property that makes two isolates'
    /// handles genuinely collide, which the mock had to be taught to imitate.
    #[test]
    fn the_first_handle_is_the_same_for_every_isolate() {
        assert_eq!(FIRST_HANDLE, 0xCAFE_0001);
        assert_ne!(FIRST_HANDLE, REQUESTED_CLIENT_HANDLE);
    }
}
