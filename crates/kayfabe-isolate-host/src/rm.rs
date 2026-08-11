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
    NV01_MEMORY_SYSTEM, NV01_MEMORY_SYSTEM_OS_DESCRIPTOR, NV01_MEMORY_VIRTUAL, NV20_SUBDEVICE_0,
    NVOS02_FLAGS_COHERENCY_CACHED, NVOS02_FLAGS_LOCATION_PCI, NVOS02_FLAGS_MAPPING_NO_MAP,
    NVOS02_FLAGS_PHYSICALITY_NONCONTIGUOUS, NVOS46_FLAGS_DMA_OFFSET_FIXED_TRUE,
    Nv2080AllocParameters, NvMemoryVirtualAllocationParams, NvVaspaceAllocationParameters,
    Nvos02ParametersWithFd, RegisterFd,
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
    PTIMER_PAGE_TIME_0, PTIMER_PAGE_TIME_1, PtimerSampleError, SET_OBJECT, USERD_GP_GET,
    USERD_GP_PUT, USERMODE_NOTIFY_CHANNEL_PENDING, USERMODE_TIME_0, USERMODE_TIME_1,
    USERMODE_WINDOW_SIZE, WORK_SUBMIT_TOKEN_PARAMS_SIZE, ce, engine_type_copy, fifo, gp_entry,
    method_header_inc, ptimer_sample,
};
use kayfabe_arch::ids::{ClassId, ControlCmd, EngineKind, GpuId, GpuVa};
use kayfabe_arch::{CeObjectClass, ChannelClass, HostClasses, UsermodeClass};
use kayfabe_isolate::{
    CeExecutor, CeSource, CeSubCopy, ExportRequest, ExportSource, ExportedBacking, GuestRamGrant,
    GuestRamMapped, HostHandle, HostedObject, IsolateId, RmBackend, RmError,
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
    /// ★★★ How many times [`RmConnection::map_cpu_windowed`] has been entered — the
    /// instrument for [`HostRmBackend::cpu_map_calls`].
    ///
    /// ⊘ Counted at the **entry** of the one function that issues `NV_ESC_RM_MAP_MEMORY`,
    /// not at its successful exit, and the difference is the whole point: the claim being
    /// measured is *"no CPU map was ATTEMPTED"*, and a counter that only recorded successes
    /// would read zero for a mapping that was tried and refused.
    ///
    /// An `AtomicU64` rather than a `Cell` because a connection is shared by every worker
    /// of the isolate; `Relaxed` because nothing is ordered against it.
    cpu_maps: std::sync::atomic::AtomicU64,
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
    /// ★★★★★ **W229** — `guest range -> the isolate's own address space over it`
    /// ([`ExecutorVas`]).
    ///
    /// ⊘⊘ **On the CONNECTION, never on a worker, and that was measured rather than
    /// designed.** It lived in `HostRmBackend` for one revision and `tests/e6_hw_join.rs`
    /// caught it on a real GA106: an isolate is a **bounded POOL** of workers, a publish
    /// and the copy that reads it are two requests that need not land on the same slot, and
    /// a per-worker table gave the second worker a **fresh, empty** shadow. The operands
    /// were mapped in worker A's shadow and the engine walked worker B's — arm 1 retired
    /// and arm 2 reported `NEVER-RETIRED` with `sem = 0`.
    ///
    /// ★ The rule it teaches: this table is keyed by an object that belongs to the
    /// **isolate** (a `Vas`), so it belongs where the isolate's other object state is. A
    /// pool slot may own a *channel*; it may not own an *address space* that other slots'
    /// mappings are placed into.
    exec_vases: BTreeMap<u32, u32>,
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
    ///
    /// ★★★ `None` for a channel over a [`GuestRing`], together with
    /// [`ChannelRings::ring`], and that is the whole of G4: the guest's ring is **never
    /// CPU-mapped by this process**. See [`RING_NOT_OURS`].
    _ring_node: Option<CharDevice>,
    /// The pushbuffer / GPFIFO / semaphore object — `None` when the ring is the guest's.
    ring: Option<VolatileRegion>,
    /// The node USERD's mmap context was registered against.
    _userd_node: CharDevice,
    /// USERD — where `GP_GET` (hardware writes) and `GP_PUT` (we write) live.
    ///
    /// ⊘ Always present, on both kinds of channel, and the asymmetry with the ring is the
    /// design rather than an oversight: USERD is **ours** on every channel we allocate (we
    /// hand RM `hUserdMemory[0]`), and `GP_PUT` is the one 32-bit cursor a shadow channel
    /// exists to advance.
    userd: VolatileRegion,
}

/// Who allocated the object a channel's GPFIFO lives in, and therefore who must free it.
///
/// ★★★ It is recorded per channel rather than inferred, because the two arms differ in
/// **three** places that are nowhere near each other: the alloc (we allocate, or we do
/// not), the CPU map (we map, or we must not), and the teardown (we unmap-and-free, or we
/// must not touch it). A `bool` at any one of those sites would be a fact re-derived at
/// the other two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RingOwner {
    /// The isolate's own 64 KiB device-local ring object, allocated by
    /// [`HostRmBackend::alloc_channel_in`] and mapped through [`ChannelParts::range`].
    Ours,
    /// A handle **handed in** — on the shadow path, an
    /// `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` over the guest's own pages, already placed in
    /// the channel's address space by whoever pinned it.
    ///
    /// ⚠ Freeing the channel must not free it and must not unmap it. The pin has its own
    /// lifetime, held by the party that made the grant, and a channel teardown that
    /// unmapped the guest's ring would leave a *live* guest channel pointing at nothing.
    HandedIn,
}

/// The three numbers a channel's GPFIFO is described by, kept **per channel** because on a
/// shadow channel they are the **guest's** and not this file's constants.
///
/// ★★★ `entries` is the load-bearing one. `GP_PUT` is an **index**, so the ring's entry
/// count is the modulus of the wrap arithmetic in [`HostRmBackend::submit_entry`]; a
/// channel created with the guest's ring and our [`GPFIFO_ENTRIES`] would have two parties
/// disagreeing about which entry a number names, and they would wrap in different places.
/// [measured, `run_w229b_b66bd44_execvas_real_qemu.log`] this guest declares **4096**,
/// **1024** and **32**-entry rings — never 64, and never the fixture's 512.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RingLayout {
    /// `gpFifoOffset` as declared to RM: an **absolute VA** in the channel's address
    /// space. For an [`RingOwner::Ours`] channel it is `ring_va + GPFIFO_OFFSET`; for a
    /// [`GuestRing`] it is the guest's own `gpFifoOffset`, passed through untouched.
    gp_fifo_va: u64,
    /// `gpFifoEntries` as declared to RM.
    entries: u32,
}

/// Everything [`RmBackend::alloc_channel`] built, kept because the port hands back one
/// handle and the later verbs need the rest.
#[derive(Debug, Clone, Copy)]
struct ChannelParts {
    /// The `KEPLER_CHANNEL_GROUP_A` this channel lives in. ★ The schedule and bind
    /// controls are issued **here**, not on the channel — see
    /// `kayfabe_abi::submit::NVA06C_CTRL_CMD_GPFIFO_SCHEDULE`.
    tsg: u32,
    /// The object holding the GPFIFO — and, for an [`RingOwner::Ours`] channel, the
    /// pushbuffer and the semaphore too.
    ring: u32,
    /// Whether [`ChannelParts::ring`] is this connection's to unmap and free.
    owner: RingOwner,
    /// The device-local object holding USERD.
    userd: u32,
    /// The `NV01_MEMORY_VIRTUAL` range [`ChannelParts::ring`] is mapped through — the
    /// handle `NV_ESC_RM_UNMAP_MEMORY_DMA` needs, which is NOT the address space.
    range: u32,
    /// Where the ring object is in the channel's address space. RM's **[OUT]** `dmaOffset`
    /// for an [`RingOwner::Ours`] ring; the address the caller states for a handed-in one.
    ring_va: u64,
    /// The GPFIFO as **declared to RM**, never re-derived from a constant afterwards.
    layout: RingLayout,
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

/// ★★★★★ **The guest's own ring, as the guest declared it** — the argument that turns
/// [`HostRmBackend::alloc_channel_at`] from *"a channel with a ring of ours"* into *"a
/// channel over the ring the guest is already pushing into"*.
///
/// # The blocker this exists to remove, stated exactly
///
/// A host channel allocated the old way has **its own** command queue, which stays empty,
/// so the engine consumes nothing forever while the guest pushes into a queue our channel
/// does not read. The owner's ruling is not to copy the methods across: it is to **map the
/// guest's queue into the GPU's view at identical addresses and let hardware read them
/// directly**. Under that shape the only verb left is advancing one 32-bit cursor — and
/// this struct is what makes the channel name the guest's bytes in the first place.
///
/// # ⊘ Every field is HANDED IN. Nothing here is derived, and that is the invariant
///
/// | field | whose number it is | ⊘ what it must never be |
/// |---|---|---|
/// | [`Self::memory`] | the pinning party's — an `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` over the guest's pages | an object this file allocated (`alloc_device_local`), which is the old behaviour |
/// | [`Self::gp_fifo_va`] | the **guest's** `gpFifoOffset`, as its own channel alloc declared it | `ring_va + `[`GPFIFO_OFFSET`] — our layout, applied to memory that is not laid out that way |
/// | [`Self::gp_fifo_entries`] | the **guest's** `gpFifoEntries` | [`GPFIFO_ENTRIES`] — see [`RingLayout::entries`] for why a wrong modulus is not cosmetic |
///
/// # ⚠ What this type does NOT do
///
/// It does not map anything. [`Self::memory`] must **already be placed** at an address
/// covering `[gp_fifo_va, gp_fifo_va + 8 * gp_fifo_entries)` in the same address space the
/// channel is created in — on the production path by the guest-RAM pin
/// (`kayfabe_rt::device::SharedDevice::pin_guest_ram`), which is committed at the doorbell.
/// That ordering is the whole reason the host channel's birth has to move; see
/// `docs/design/guest_ring_adoption.md` §3.
///
/// ⊘ And it does not make the channel *runnable*. Nothing in this rung writes the guest's
/// `GP_PUT` into our USERD, so the engine still has nothing to fetch. Adopting the ring and
/// advancing the cursor are two rungs, and this is the first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestRing {
    /// The memory object carrying the guest's GPFIFO. Neither allocated nor freed here.
    pub memory: HostHandle,
    /// Where the object is placed in the channel's address space — the base the caller
    /// asked for and RM honoured, kept so a diagnostic can say which mapping the
    /// `gpFifoOffset` below lives inside.
    pub ring_va: u64,
    /// The guest's `gpFifoOffset`: an **absolute VA**, not an offset into anything.
    ///
    /// ⚠ `0` is a value and not a blank — the driver deliberately declares
    /// `gpFifoOffset = 0` for its golden-context channel
    /// (`kayfabe_core::rmgraph::GpFifoRing`). A caller with no ring to name must not
    /// synthesise one; it has no [`GuestRing`] to pass.
    pub gp_fifo_va: u64,
    /// The guest's `gpFifoEntries`.
    pub gp_fifo_entries: u32,
}

/// Where a channel's GPFIFO comes from — the one degree of freedom
/// [`HostRmBackend::alloc_channel_in`] gained on this rung.
///
/// ⊘ Deliberately not `Option<GuestRing>`. The `Ours` arm carries its own parameter
/// (R26's dictated placement), and collapsing the two into an `Option` would make
/// *"no guest ring"* and *"no dictated address"* the same word.
#[derive(Debug, Clone, Copy)]
enum RingSource {
    /// Allocate the isolate's own [`RING_OBJECT_BYTES`] device-local ring, optionally at
    /// an address we dictate (R26).
    Ours(Option<GpuVa>),
    /// Adopt the guest's, already placed. See [`GuestRing`].
    Guest(GuestRing),
}

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

/// The opaque status **a ring access on a channel whose ring this process does not hold a
/// CPU mapping of** reports — i.e. every channel built over a [`GuestRing`].
///
/// ★★★ It is the *positive* form of "we do not CPU-map the guest's ring". Omitting the
/// mapping would leave `ring_store_u32` reading a `None` and answering
/// [`RmError::BadHandle`], which says *"that is not a channel"* about a channel that
/// certainly exists — the symptom-not-truth failure. This status says the true thing: the
/// channel is real, the ring is real, and **the bytes are not ours to write from the CPU**.
///
/// ⊘ It is also the shape of the next rung's boundary. A guest-backed ring is advanced by
/// copying the guest's own cursor, not by this process composing methods into it; a caller
/// that reaches for `ring_store_u32` here is reaching for the wrong verb, and gets told so
/// by name rather than by a bounds error somewhere inside a mapping that was never opened.
/// `0x4B4C` is `"KL"`.
pub const RING_NOT_OURS: u32 = 0x4B4C;

/// The opaque status a **GPFIFO entry count that cannot be an index modulus** reports.
///
/// ★★ Only zero is refused here, and the narrowness is the point. RM requires a power of
/// two and refuses anything else itself — pre-empting it would be this file re-deriving a
/// rule the driver already enforces, and would turn *"the host refused the guest's ring"*
/// into *"we refused it first"*, which is a different fact about a boot. Zero is different
/// in kind: it is the divisor of the wrap arithmetic (`slot % entries`), so it is a
/// **panic** rather than a refusal, and a guest that declares it is not hypothetical —
/// `kayfabe_core::rmgraph::GpFifoRing`'s own docs record the driver declaring
/// `gpFifoOffset = 0` for its golden-context channel. `0x4B4D` is `"KM"`.
pub const RING_ENTRIES_REFUSED: u32 = 0x4B4D;

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
                exec_vases: BTreeMap::new(),
            }),
            rings: Mutex::new(BTreeMap::new()),
            cpu_maps: std::sync::atomic::AtomicU64::new(0),
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

    /// ★★★ `#128` T3 — the host GPU's PTIMER, read through the **usermode window this
    /// connection already holds**.
    ///
    /// No new RM object and no new mapping: [`USERMODE_TIME_0`]/[`USERMODE_TIME_1`] are
    /// sixteen bytes below the doorbell in the same 64 KiB window
    /// [`Self::open_usermode`] maps at bring-up. That is the point of this accessor — it
    /// proves a **capability-less** process can read a real host nanosecond counter
    /// *without acquiring anything it did not already need to submit work*.
    ///
    /// ⊘ It is **not** the read-native path and must never be mistaken for it. Every call
    /// here is a function call inside one process; the guest reaching this would be an
    /// exit plus an IPC, which is the jitter `#128` exists to remove. This is the
    /// **oracle** — the value a passthrough mapping must agree with — and the fallback
    /// for a host that refuses the dedicated page.
    ///
    /// # Errors
    /// The refusal [`Self::open_usermode`] kept, if the window never mapped; or
    /// [`RmError::Other`] if the counter never settled across
    /// [`kayfabe_abi::submit::PTIMER_SAMPLE_ROUNDS`] rounds — ⊘ never a zero.
    pub fn host_ptimer_via_usermode(&self) -> Result<u64, RmError> {
        let window = self.usermode.as_ref().map_err(|e| *e)?;
        Self::sample(&window.region, USERMODE_TIME_1, USERMODE_TIME_0)
    }

    /// [`ptimer_sample`] over a [`VolatileRegion`], mapping both refusals onto [`RmError`].
    fn sample(region: &VolatileRegion, hi: u64, lo: u64) -> Result<u64, RmError> {
        ptimer_sample(hi, lo, |off| region.load_u32(HostOffset::new(off))).map_err(|e| match e {
            PtimerSampleError::Read(r) => region_error(&r),
            // ⊘ The standing rule, at the one place it could be broken: an incoherent
            // counter is a REFUSAL. Returning the last pair, or zero, would be plausible.
            PtimerSampleError::Incoherent => RmError::Other(NOT_ON_THIS_RUNG),
        })
    }

    /// ★★★ `#128` T4 — allocate an [`NV01_TIMER`](kayfabe_abi::submit::NV01_TIMER) object,
    /// the handle behind **a dedicated mapping of BAR0 `0x9000`, the PTIMER page**.
    ///
    /// This is the mapping the read-native design first reached for, and it differs from
    /// [`Self::host_ptimer_via_usermode`] in the one way that looked decisive: the range
    /// contains **only timer registers**, no doorbell, so the whole page could be exposed
    /// to a guest without exposing a work-submit path.
    /// `tmrapiGetRegBaseOffsetAndSize_IMPL` reports `DRF_BASE(NV_PTIMER)` and
    /// `sizeof(Nv01TimerMap)` for it
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/timer/timer.c:1712-1734`).
    ///
    /// ⊘ **It cannot actually back the guest's timer page, and that is not a permission
    /// problem.** The guest reads its counter at page offset `0x080`; this page carries it
    /// at `0x400`; a memslot cannot re-base within a page. The route is kept because it is
    /// the *control* for the usermode mirror — two independent mappings agreeing is what
    /// licenses treating either as the host's counter — and because it is the only route
    /// that demonstrates a **doorbell-free** BAR0 range is mappable at all. See
    /// `docs/design/read_native_timer_measured.md` §2.
    ///
    /// ★★★ **The mapping RM grants here is READ-ONLY, and that is hardware policy rather
    /// than ours.** `subdeviceCtrlCmdValidateMemMapRequest_IMPL` walks BAR0 range by range
    /// for any caller that is not `osIsAdministrator()`, and the PTIMER row — the *first*
    /// row it tries — returns `NV_PROTECT_READABLE`
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/subdevice/subdevice_ctrl_gpu_kernel.c:2905-2917`,
    /// reached from `RmValidateMmapRequest`, `.../unix/src/osapi.c:2023-2054`). So an
    /// unprivileged holder **cannot** write `NV_PTIMER_TIME_0`, and
    /// `tmrSetCurrentTime_GV100`'s register is out of reach by construction rather than by
    /// a check we could forget. ⚠ A *root* caller takes the `osIsAdministrator()` fast path
    /// and gets `NV_PROTECT_READ_WRITE` instead — which is why the ladder rung that measures
    /// this means nothing unless it is run as an unprivileged uid.
    ///
    /// ⊘ **There is deliberately no `map_ptimer_page` that does the alloc and the map in one
    /// call.** There was, and it was dead code by the end of the task that added it: the
    /// rung has to report *which of the two acts* failed, because the first version of this
    /// rung (2026-08-02, GA106, revision 6213a24) collapsed them and printed our own length
    /// arithmetic as a driver refusal.
    /// A convenience that re-collapses them is the one shape this seam must not offer.
    ///
    /// # Errors
    /// Whatever RM refuses the alloc with.
    pub fn alloc_timer_object(&self) -> Result<u32, RmError> {
        let want = self.mint();
        let object = self.raw_alloc(
            self.subdevice,
            want,
            kayfabe_abi::submit::NV01_TIMER,
            &mut [],
        )?;
        self.remember(object, self.subdevice);
        Ok(object)
    }

    /// CPU-map an already-allocated object, uncached — the policy every BAR0 register
    /// range gets (`ogkm-580: kernel-open/nvidia/nv-mmap.c:567-574`). A *cached* mapping of
    /// a free-running counter would read one value forever, which is this task's failure
    /// arriving through the cache instead of through a trap.
    ///
    /// See [`Self::map_cpu_windowed`] for why the two lengths are separate.
    ///
    /// # Errors
    /// As [`Self::map_cpu_windowed`].
    pub fn map_object_uncached(
        &self,
        object: u32,
        register_len: u64,
        mmap_len: u64,
    ) -> Result<(CharDevice, VolatileRegion), RmError> {
        self.map_cpu_windowed(object, register_len, mmap_len, CachePolicy::Uncached)
    }

    /// Read the PTIMER pair out of a region produced by [`Self::alloc_timer_object`] +
    /// [`Self::map_object_uncached`].
    ///
    /// # Errors
    /// As [`Self::host_ptimer_via_usermode`].
    pub fn ptimer_page_read(region: &VolatileRegion) -> Result<u64, RmError> {
        Self::sample(region, PTIMER_PAGE_TIME_1, PTIMER_PAGE_TIME_0)
    }

    /// `NV2080_CTRL_CMD_TIMER_GET_REGISTER_OFFSET` on this connection's subdevice — the
    /// control that exists expressly *"so that clients may map them directly"*
    /// (`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080tmr.h:107-110`).
    ///
    /// `payload` should be four bytes and is left **as the caller seeded it** if RM answers
    /// without writing — the `#128` rung seeds `0xCD` for exactly the reason R18 does, so
    /// *"answered `NV_OK` and wrote nothing"* is distinguishable from *"answered zero"*.
    ///
    /// # Errors
    /// Whatever RM refuses the control with.
    pub fn timer_register_offset(&self, payload: &mut [u8]) -> Result<(), RmError> {
        self.raw_control(
            self.subdevice,
            kayfabe_abi::submit::NV2080_CTRL_CMD_TIMER_GET_REGISTER_OFFSET,
            payload,
        )
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

    /// The isolate's own address space over `guest_range`, if one has been built.
    fn exec_vas_of(&self, guest_range: u32) -> Option<u32> {
        let _leaf = leafwitness::Held::enter();
        self.objects
            .lock()
            .expect("objects")
            .exec_vases
            .get(&guest_range)
            .copied()
    }

    /// Publish `exec` as the executor space for `guest_range`, and report **the winner**.
    ///
    /// ★ It returns whatever is in the table afterwards rather than `()`, because two pool
    /// workers can mint concurrently: the loser must be told so it can free the space it
    /// just allocated instead of leaking one that nothing will ever name. The check and the
    /// insert are one critical section — doing them as two calls is the race with extra
    /// steps.
    fn remember_exec_vas(&self, guest_range: u32, exec: u32) -> u32 {
        let _leaf = leafwitness::Held::enter();
        *self
            .objects
            .lock()
            .expect("objects")
            .exec_vases
            .entry(guest_range)
            .or_insert(exec)
    }

    /// Take the executor space out of the table — `free`'s accessor, which is why it
    /// removes.
    fn forget_exec_vas(&self, guest_range: u32) -> Option<u32> {
        let _leaf = leafwitness::Held::enter();
        self.objects
            .lock()
            .expect("objects")
            .exec_vases
            .remove(&guest_range)
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
    /// isolate allocated for itself. A channel's own ring is exactly that.
    ///
    /// # ⊘⊘⊘ THE SENTENCE THAT USED TO FOLLOW WAS FALSE, AND IT WAS THE INVARIANT
    ///
    /// This paragraph read *"…memory the isolate allocated for itself, **which no guest
    /// ever names**"*, and the owner's invariant — *"VMM state must never be placed where a
    /// guest VA can name it"* — rested on it and on nothing else. It was **untrue as
    /// placement** for as long as the copy-engine path existed. `plan_ce` →
    /// `ce_channel(vas)` → `alloc_channel_on(vas, COPY0)` put the isolate's ring, USERD and
    /// completion semaphore in **the one address space a guest channel is bound to**, at an
    /// RM-chosen address — which makes it *unpredictable, not unnameable*, and
    /// unpredictability is not a boundary
    /// (`C: docs/design/s1_what_does_it_protect.md` §3).
    ///
    /// ⚠ **[measured 2026-08-10, `vh`, at `cc5d55c`]** a copy engine bound to that space
    /// retired a read of the semaphore's VA and moved `0x00000001` — **the exact payload
    /// the isolate's own last copy had released**, a number that channel has no other way
    /// to obtain (`kayfabe-rm-ladder --executor-vas-alias`, arm C).
    ///
    /// ⇒ Closed by **separation**, not by a reservation: see [`ExecutorVas`]. ⊘ A reserved
    /// window inside this space would have stopped RM's *allocator* from colliding with our
    /// objects and done nothing about a guest **naming** them, because the mapping would
    /// still be in the page tables the guest's engine walks. The two fixes are easy to
    /// confuse and only one of them is this one. At `2ce8bd0` the same probe faults:
    /// `Xid 31 … ENGINE CE0 … FAULT_PDE ACCESS_TYPE_VIRT_READ @ 0x1_20022000`.
    ///
    /// ★ The residual, still named and now *only* a collision: RM's own VA allocator and
    /// our fixed publishes share the guest-facing space, so RM could place something where
    /// a guest later demands a fixed mapping. That surfaces as a refused fixed map with an
    /// RM status, which is loud; it is not silent corruption, and it is a different problem
    /// from the one above.
    ///
    /// ⚠ **Amended by R26.** The first paragraph once said *"demanding a fixed address for
    /// a ring would mean inventing a host-private VA window"*, which no longer describes
    /// the tree: [`HostRmBackend::alloc_channel_at`] takes `Some` here and a **caller**
    /// supplies the address, which is what a shadow-forwarded channel needs. What it got
    /// right is that the *policy* is not this function's — it is still the caller's.
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
        self.map_cpu_windowed(h_memory, len, len, cache)
    }

    /// [`Self::map_cpu`] with the **ioctl length and the `mmap` length given separately**.
    ///
    /// ★★★ **They are not always the same number, and assuming they were cost `#128` a
    /// wrong finding.** The escape's length is bounded by the RM resource's own size:
    /// `gpuresMap_IMPL` asks `gpuresGetRegBaseOffsetAndSize` and refuses anything past it
    /// with `NV_ERR_INVALID_LIMIT` (`ogkm-580: src/nvidia/src/kernel/gpu/gpu_resource.c:126-143`).
    /// The `mmap` length, by contrast, must be a whole number of host pages — Linux's
    /// requirement, and independently ours in `Mapping::anywhere`. For an
    /// [`NV01_TIMER`](kayfabe_abi::submit::NV01_TIMER) those two are `0x414` and `0x1000`,
    /// so **no single value can satisfy both**: `0x414` never reaches the driver and
    /// `0x1000` is refused by it.
    ///
    /// The two are reconciled inside RM rather than by the caller:
    /// `nv_align_mmap_offset_length` rounds the range it registers up to a page
    /// (`ogkm-580: src/nvidia/arch/nvalloc/unix/src/osapi.c:1976-1986`), and
    /// `nvidia_mmap_helper` then compares the `mmap` length against that **rounded** size
    /// (`ogkm-580: kernel-open/nvidia/nv-mmap.c:560-565`). So the correct call passes the
    /// object's true size to the ioctl and the page-rounded size to `mmap`.
    ///
    /// ⚠ Every pre-existing caller passes the same value twice and is unchanged by this:
    /// their objects are already page multiples. This exists for the one object whose size
    /// is not.
    fn map_cpu_windowed(
        &self,
        h_memory: u32,
        register_len: u64,
        mmap_len: u64,
        cache: CachePolicy,
    ) -> Result<(CharDevice, VolatileRegion), RmError> {
        // ★ Before anything can fail. See `RmConnection::cpu_maps`: the measurement is of
        // attempts, so an early `?` must not be able to hide one.
        self.cpu_maps
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let len = register_len;
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
            mmap_len,
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

    /// ★★★ **R25 — describe memory this process already owns to RM, so the host GPU can
    /// reach it.** `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` over `[offset, offset+len)` of
    /// `region`.
    ///
    /// This is the one primitive that makes *guest* RAM addressable by the host GPU: the
    /// VMM maps a slice of the guest's `memfd`, the isolate maps the same pages, and this
    /// call turns that range into an RM memory object that
    /// [`RmBackend::map_gpu_va`](kayfabe_isolate::RmBackend::map_gpu_va) can then place in
    /// a host VAS. Everything after it is machinery that already exists.
    ///
    /// ## The four things that are easy to get wrong
    ///
    /// **INFERRED** unless a row says otherwise — three are readings of the C artifact and
    /// of `ogkm`, and the fourth is a property of this crate's own type. What has been
    /// **MEASURED** is that the assembled call works:
    /// `traces/real_ga106/rmladder_r25_osdescriptor_real_ga106.txt` (RTX 3060 GA106,
    /// 580.159.04, `REV_UNDER_TEST=40d44db84`). ⊘ That run does not isolate any individual
    /// row below — it says the four together are sufficient, never that each is necessary.
    ///
    /// 1. **The address never crosses a crate boundary.** `pMemory` is filled in by
    ///    [`Indirect::describing`] inside `kayfabe-linux-raw` and scrubbed back to zero
    ///    before this function can observe it — §4.2.1's rule, and the reason
    ///    [`Nvos02ParametersWithFd::p_memory`]'s own docs forbid this crate from writing it.
    /// 2. **The node.** `NV_ESC_RM_ALLOC_MEMORY` is `NV_ACTUAL_DEVICE_ONLY`, so it goes on
    ///    the per-GPU node exactly as [`RmBackend::alloc_sysmem`](kayfabe_isolate::RmBackend::alloc_sysmem)
    ///    does. The C found the same thing the same way: *"ctl fd -> EINVAL"*
    ///    (`C: nvkvm_gpu_emul.c:7530-7532`).
    /// 3. **`REGISTER_FD` is a prerequisite** — without it RM answers `0x23
    ///    INVALID_CLIENT` (`C: nvkvm_gpu_emul.c:7503-7509`). ⊘ **Already done, and this is
    ///    not a port of it:** [`RmConnection::open`]'s R3 binds the GPU node to the control
    ///    session for the connection's whole life, so by the time any caller reaches here
    ///    the prerequisite is a structural property of the type rather than a step. Porting
    ///    the C's lazy `m2_gpu_registered` flag would add a second, weaker copy of an
    ///    invariant we already hold.
    /// 4. ★ **`MAPPING_NO_MAP` is required, not an optimisation.** Without it the driver
    ///    tries to build an `mmap` context around a describe-only allocation and returns
    ///    `EINVAL` (`C: nvkvm_gpu_emul.c:7519-7524`). The flag word is `0x40001010` and is
    ///    reassembled here from four named constants, pinned by
    ///    `nvos02_flags_encode_a_value_into_their_field`.
    ///
    /// ⚠ **The pages stay pinned until the object is freed.** Dropping `region` unmaps this
    /// process's view; it does not release RM's reference. Free the returned handle.
    fn alloc_os_descriptor(
        &self,
        region: &kayfabe_linux_raw::MappedRegion,
        offset: HostOffset,
        len: u64,
    ) -> Result<u32, RmError> {
        if len == 0 {
            return Err(RmError::NoMemory);
        }
        let want = self.mint();
        let mut arg = [0u8; Nvos02ParametersWithFd::SIZE];
        Nvos02ParametersWithFd {
            h_root: self.client.raw(),
            h_object_parent: self.device,
            h_object_new: want,
            h_class: NV01_MEMORY_SYSTEM_OS_DESCRIPTOR,
            flags: NVOS02_FLAGS_LOCATION_PCI
                | NVOS02_FLAGS_PHYSICALITY_NONCONTIGUOUS
                | NVOS02_FLAGS_COHERENCY_CACHED
                | NVOS02_FLAGS_MAPPING_NO_MAP,
            // ★ Left ZERO on purpose. `Indirect` writes the address and scrubs it; a value
            // here would be overwritten before the syscall and zeroed after it, so the only
            // effect of setting it would be to make a reader think this crate mints
            // addresses.
            p_memory: 0,
            pad1: 0,
            // `limit`, not `length` — the ABI's off-by-one, same as `alloc_sysmem`.
            limit: len - 1,
            status: 0,
            fd: -1,
        }
        .encode_into(&mut arg)
        .map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?;
        let req = ioctl::readwrite(NV_IOCTL_MAGIC, NV_ESC_RM_ALLOC_MEMORY, arg.len())
            .map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?;
        let mut describe =
            [
                Indirect::describing(Nvos02ParametersWithFd::P_MEMORY_OFFSET, region, offset, len)
                    .map_err(|e| region_error(&e))?,
            ];
        self.gpu
            .ioctl(req, &mut arg, &mut describe)
            .map_err(|e| ioctl_error(&e))?;
        let out =
            Nvos02ParametersWithFd::decode(&arg).map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?;
        status_check(out.status)?;
        self.remember(out.h_object_new, self.device);
        Ok(out.h_object_new)
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

/// ★★★★★ **§16.106 — WHICH copy engine the channel must be built on, taken from the
/// object the guest is putting on it.**
///
/// [`engine_type_for`] answers *which kind of engine*; it cannot answer *which instance*,
/// because [`EngineKind::Ce`] does not carry one. So it picked index 0, and its own
/// closing paragraph said choosing CE2+ *"is a scheduling decision with a cost which
/// nothing at this rung is in a position to make."* ⊘ That is still true — **and we do not
/// have to make it. The guest already did**, in the eight bytes it hands us.
///
/// # ★★★ The 14 refusals this exists to remove, and both ends of the number
///
/// `[measured 2026-08-11, boots w250 / w251 / w254, real GA106, host driver open
/// 580.159.04]` every one of this port's engine-object refusals is this mismatch:
///
/// ```text
/// NVRM: kfifoRunlistSetId_GM107: Channel has already been assigned a runlist
///       incompatible with this engine (requested: 0x1 current: 0x0).
/// NVRM: kfifoRunlistSetIdByEngine_GM107: Unable to program runlist for CE2
/// NVRM: chandesConstruct_IMPL: Invalid object allocation request on channel 0x00000004
/// ```
///
/// - **`current: 0x0`** is OURS: the TSG was allocated with `ENGINE_TYPE_COPY0`, and
///   `engine_type_for`'s own measured sweep records `COPY(0)`/`COPY(1)` → **runlist 0**.
/// - **`requested: 0x1` for `CE2` / `0x2` for `CE3`** is the GUEST'S, read out of the
///   object's `NVB0B5_ALLOCATION_PARAMETERS` by RM's `kceGetEngineDescFromAllocParams`
///   (`ogkm-580: src/nvidia/src/kernel/gpu/ce/kernel_ce_context.c:60-175`) — and the same
///   sweep records `COPY(2)` → runlist **1**, `COPY(3)` → runlist **2**. Both ends agree
///   with the table; nothing here is inferred from the symptom.
///
/// The refusal is `kfifoRunlistSetId_GM107`'s first branch (`NV_ERR_INVALID_STATE`, `0x40`),
/// reached from `chandesConstruct_IMPL` (`ogkm-580: channel_descendant.c:243-250`), whose
/// status returns out to our `alloc_engine_object`.
///
/// # ⊘ Why the CHANNEL moves and not the OBJECT
///
/// The other repair — rewrite the guest's `engineType` to `COPY0` — is refused on three
/// counts. **(1)** The declaration is not ours to edit: the same ordinal goes out again in
/// the guest's own `NVA06F_CTRL_CMD_BIND` (`engineType = 11` = `COPY2`, measured on real
/// hardware, `traces/real_ga106/rpc_transcript_real_ga106.txt:63`), so the guest would
/// believe `COPY2` while the host ran `COPY0` — a disagreement invisible until a copy runs
/// on an engine nobody asked for. **(2)** It is a *wrong answer* rather than a missing one:
/// `COPY0`/`COPY1` are the GRCE pair and share the graphics runlist, so forcing them
/// serialises copies against GR work the guest expects to overlap. **(3)** It re-creates
/// the C's `dma_copy_class_alloc_params` defect deliberately, having just measured it.
///
/// # ⊘ Scope, stated narrowly
///
/// - **Only [`EngineKind::Ce`].** A GR channel that later takes a CE object binds it as
///   GRCE and needs no move — that is the 8 forwards that already succeed, and keying on
///   the class alone would break them by building a CE channel for a GR context.
/// - **Only a declaration RM itself would accept.** `None` from
///   [`CeAllocParams::declared_copy_engine_type`] falls through to `engine_type_for`, so
///   absent/short/unknown-version params leave behaviour **byte-identical to before**.
///   ⊘ `None` is never read as "copy engine 0"; the fall-through arrives at `COPY0`
///   through the unchanged path, which is a different sentence.
fn declared_channel_engine_type(
    engine: EngineKind,
    hosting: Option<HostedObject<'_>>,
) -> Option<u32> {
    if engine != EngineKind::Ce {
        return None;
    }
    let hosting = hosting?;
    CeAllocParams::decode(hosting.params)
        .ok()?
        .declared_copy_engine_type()
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
    /// ★★★ This isolate's guest-RAM plane, or `None` when the VM was launched without a
    /// shared memory backing. Shared with every sibling worker — a guest-RAM mapping
    /// belongs to the isolate, not to the pool slot that happened to order it.
    guest_ram: Option<Arc<crate::guestram::GuestRamPlane>>,
    /// ★★★ **E6 — the recorder-only CE witness**, `None` unless a diagnostic asked for
    /// one ([`HostRmBackend::with_ce_witness`]). See [`CeWitness`].
    ce_witness: Option<Arc<CeWitness>>,
}

/// ★★★ **E6 — what the LAST [`RmBackend::ce_copy`] this backend performed actually
/// observed**, so a caller that drove the copy *through the core* can build the very
/// [`CeEvidence`] rung R17 built, instead of a re-derivation of it.
///
/// # Why this exists at all, stated so it is not mistaken for a convenience
///
/// [`CeEvidence::copied()`] is a **conjunction of four facts**, and the fourth —
/// *"the engine said it had retired"* — is not observable from the destination's bytes.
/// The port's verb answers `Ok(())`/`Err(..)`, which is the right shape for a port and
/// erases the number. So a caller driving the join from `kayfabe_fwd` can see three of the
/// four and would have to **assume** the fourth, or invent a weaker predicate.
///
/// ⊘ Inventing a weaker predicate is exactly what `execution_plane_increments.md` §1
/// forbids: *"Nothing below invents a new acceptance instrument, and E6's acceptance is
/// literally R17's, re-driven."* This is what makes that literal.
///
/// ⊘ **Recorder-only, and off by default.** [`HostRmBackend::new`] installs none, so the
/// shipped isolate child records nothing and pays nothing. It is the same posture — and
/// the same warning — as the C artifact's `m2rec`: an instrument that is on by default
/// stops being an instrument.
///
/// ⚠ **It cannot cross the sandbox.** A real isolate is a separate process, so a witness
/// held by a parent is not the one the child's backend writes. A diagnostic that needs it
/// must drive an **in-process** [`HostRmBackend`], exactly as `kayfabe-rm-ladder`'s R14-R17
/// rungs already do, and say so.
#[derive(Debug, Default)]
pub struct CeWitness {
    last: Mutex<Option<(SubmitOutcome, u32)>>,
}

impl CeWitness {
    /// A fresh witness that has observed nothing.
    #[must_use]
    pub fn new() -> CeWitness {
        CeWitness::default()
    }

    /// The most recent `(outcome, payload)` — `None` if no copy has run on the backend
    /// this witness is installed in.
    ///
    /// ★ `None` is a real answer and must not be read as a zeroed outcome: *an empty
    /// capture is evidence of nothing*.
    #[must_use]
    pub fn latest(&self) -> Option<(SubmitOutcome, u32)> {
        *self.last.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn record(&self, outcome: SubmitOutcome, payload: u32) {
        *self.last.lock().unwrap_or_else(|e| e.into_inner()) = Some((outcome, payload));
    }
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

/// ★★★★★ **W229 — a host address space NO GUEST CHANNEL IS EVER BOUND TO.**
///
/// This is the type half of the owner's invariant, *"VMM state must never be placed where a
/// guest VA can name it"*. Before it, that invariant was a sentence in
/// [`RmConnection::raw_map_dma`]'s doc comment — *"memory the isolate allocated for itself,
/// which no guest ever names"* — and the isolate's copy-engine ring, USERD and completion
/// semaphore were mapped into the very space `kayfabe_fwd::plan_doorbell` materializes a
/// guest's channel in. Measured at `124b69b`: a copy engine bound to that space read the
/// isolate's semaphore and moved its payload (`R30` arm C).
///
/// # ★★ SEPARATION, not a reservation — and the distinction is the whole point
///
/// `raw_map_dma`'s own docs propose *"a host-private reservation"*, and a reserved window
/// inside the shared space is **not this and is not sufficient**. A reservation stops RM's
/// allocator from *colliding* with our objects; it does nothing about a guest **naming**
/// the address, because the address is still mapped in the space the guest's engine walks.
/// ⇒ What this type carries is a **different `FERMI_VASPACE_A`**, so the isolate's control
/// structures are not in the guest's page tables at all and the address does not resolve
/// there.
///
/// # ⊘ It is NOT a weakening of `#102`, and the guest's addresses do not move
///
/// Address identity exists so a *forwarded pushbuffer's* guest VAs resolve. Every isolate
/// publish — fabricated backings, guest-RAM pins, `w228`'s FB leaves — is still placed
/// **FIXED at the guest's own VA**, and is now placed at that same VA in **both** spaces
/// ([`HostRmBackend::map_dma_both`]), because the isolate's own engine has to resolve the
/// operands it is asked to copy. Nothing the guest names moves. Only our ring does.
///
/// # ★ The teeth
///
/// The field is **private**, and the only expression that builds one is
/// [`HostRmBackend::executor_vas`]. There is no `From<HostHandle>`, no public constructor
/// and no `pub` field, so `ce_channel`'s signature is not a convention a later refactor can
/// quietly reinterpret: a caller holding a guest `Vas`'s [`HostHandle`] has **no way to
/// spell** the argument. `tests/ui/name_an_executor_vas.rs` pins it, and
/// `tests/executor_vas_census.rs` pins the mint site count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutorVas {
    /// The `NV01_MEMORY_VIRTUAL` range over the isolate's own `FERMI_VASPACE_A`.
    ///
    /// ⊘ Deliberately **not** a [`HostHandle`]: a port handle is a thing the core can be
    /// handed and can pass back into any verb, and this space must never be reachable that
    /// way. It is a raw handle behind a private field precisely so it cannot leave.
    range: u32,
}

/// ★★★ **W229 — where the isolate's own copy-engine control structures were PLACED**,
/// reported as raw range handles so the comparison is the caller's.
///
/// The two space fields being **equal** is the co-location defect
/// (`C: docs/design/s1_what_does_it_protect.md` §3): our ring, USERD and completion
/// semaphore sitting in the one address space a guest channel is bound to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CeControlPlacement {
    /// The `NV01_MEMORY_VIRTUAL` range a guest channel over this `Vas` is bound to, and
    /// the one every fixed publish lands in.
    pub guest_space: u32,
    /// The range the isolate's own CE ring is mapped through.
    pub control_space: u32,
    /// Where the ring object landed.
    pub ring_va: u64,
    /// The completion semaphore word hardware writes.
    pub sem_va: u64,
    /// The ring object's size. ★★ It is here because it is the **granularity the question
    /// has to be asked at**: RM maps device-local memory with 64 KiB big pages, so a VA
    /// 8 KiB above a mapped object still resolves and a 4 KiB probe cannot buy finer
    /// resolution. [measured 2026-08-10, `vh`] — a 4 KiB fixed ask at `ring_va + 0x2000`
    /// was placed at `ring_va`. ⇒ *"is our semaphore nameable"* is answered by asking about
    /// the object that contains it.
    pub ring_bytes: u64,
    /// The payload the isolate's last copy over this `Vas` released — the value an engine
    /// that reads [`CeControlPlacement::sem_va`] would find, and one nothing else has.
    pub last_payload: u32,
}

/// What [`HostRmBackend::probe_va`] found at one VA in one address space.
#[derive(Debug)]
pub enum VaProbe {
    /// Nothing was there: a fresh object took the address exactly as asked.
    Free,
    /// RM refused the fixed placement — something already occupies it.
    Occupied(RmError),
    /// ⊘ RM placed it elsewhere. Occupied, *and* the ask was treated as a hint — a
    /// distinct finding from a clean refusal and never folded into it.
    Relocated(u64),
}

/// What an engine bound to the **guest's** address space did with the isolate's semaphore
/// VA — [`HostRmBackend::probe_guest_reachability`]'s verdict.
#[derive(Debug)]
pub enum GuestReach {
    /// ⊘ The positive control did not land, so the probe was never issued and this run
    /// says nothing about reachability.
    ControlFailed,
    /// ★★★ **THE DEFECT, MEASURED**: the copy retired and moved a word out of the
    /// isolate's semaphore address.
    Read {
        /// What landed in the destination.
        word: u32,
        /// The submission's own cursors and release.
        outcome: SubmitOutcome,
    },
    /// The engine did not retire the copy: the address does not resolve in this space.
    /// ⚠ Expect a host `Xid 31 FAULT_PDE` for this channel.
    NotResolved(SubmitOutcome),
    /// Bytes moved and the engine did not report the release. Neither arm, so neither is
    /// claimed.
    Ambiguous {
        /// What landed in the destination.
        word: u32,
        /// The submission's own cursors and release.
        outcome: SubmitOutcome,
    },
}

/// [`HostRmBackend::probe_guest_reachability`]'s full report — the control and the probe,
/// never the probe alone.
#[derive(Debug)]
pub struct GuestReachProbe {
    /// The positive control's submission.
    pub control: SubmitOutcome,
    /// What the control actually moved.
    pub control_read: u32,
    /// What the control was supposed to move.
    pub control_want: u32,
    /// The probe's verdict.
    pub reach: GuestReach,
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

/// The first word of a whole-buffer compare that did not match.
///
/// ★ An *index*, not a count. "17 words differed" and "the first difference is at word 17"
/// are different facts, and only the second tells you whether a page boundary, a cache
/// line or the whole buffer is the story.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WordMismatch {
    /// Index of the first mismatching 32-bit word.
    pub word: u64,
    /// What was read there.
    pub got: u32,
    /// What was written there before the descriptor was ever allocated.
    pub want: u32,
}

/// ★★★ What [`HostRmBackend::prove_guest_ram_pin`] observed, as separable facts.
///
/// ⊘ Separable on purpose, and it is [`OsDescEvidence`]'s own lesson: `placed_as_asked` and
/// *"the isolate is looking at the window the grant named"* are different questions, and a
/// single boolean over both would score a plane that mapped the wrong window as a placement
/// failure — sending the next reader to re-check `DMA_OFFSET_FIXED`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestRamPinEvidence {
    /// The host GPU VA the caller dictated.
    pub asked_va: u64,
    /// The host GPU VA RM wrote back — its **[OUT]** `dmaOffset`, not our argument echoed.
    pub got_va: u64,
    /// How many bytes the grant named.
    pub bytes: u64,
    /// The grant's offset into the block. Non-zero by construction; see the prover.
    pub offset: u64,
    /// The first word the ISOLATE's mapping reads.
    pub first_word: u32,
    /// The word the VMM wrote at that offset.
    pub expected_word: u32,
}

impl GuestRamPinEvidence {
    /// The **fixed** map landed where it was asked to.
    #[must_use]
    pub fn placed_as_asked(&self) -> bool {
        self.got_va == self.asked_va
    }

    /// The isolate is looking at the **window the grant named**, not at the block's start.
    #[must_use]
    pub fn window_is_the_granted_one(&self) -> bool {
        self.first_word == self.expected_word
    }
}

/// ★★★ What [`HostRmBackend::prove_os_descriptor`] observed — **shaped so the four
/// falsifier arms cannot be collapsed into one boolean.**
///
/// Each field answers a different question, and the reason they are separate is that a
/// "did the ioctl succeed?" test scores three of the four failures as a pass:
///
/// | field | the question it answers alone |
/// |---|---|
/// | (an `Err` from the call) | may a process of this privilege pin its own pages for RM? |
/// | [`Self::got_va`] vs [`Self::asked_va`] | does address identity extend to described memory? |
/// | [`Self::submit`] | did the engine actually fetch and retire? |
/// | [`Self::mismatch`] | are the pages the GPU saw the pages we wrote? |
#[derive(Debug, Clone, Copy)]
pub struct OsDescEvidence {
    /// The GPU VA we asked `DMA_OFFSET_FIXED_TRUE` to place the mapping at.
    pub asked_va: u64,
    /// The GPU VA RM reported. Equal to [`Self::asked_va`] or the rung has failed, however
    /// green everything downstream looks.
    pub got_va: u64,
    /// How many bytes were described, mapped and copied.
    pub bytes: u64,
    /// The destination's first word **before** the copy — the sentinel. Non-vacuity: it
    /// says the destination did not already hold the answer.
    pub before: u32,
    /// The destination's first word after, through an independent mapping.
    pub after: u32,
    /// What `before` was set to.
    pub sentinel: u32,
    /// The first word of the destination that is not what we wrote into the memfd, if any.
    /// `None` means **every** word of [`Self::bytes`] matched.
    pub mismatch: Option<WordMismatch>,
    /// ★★ How many bytes were actually **compared**, counted by the comparison loop.
    ///
    /// ⊘ Not a copy of [`Self::bytes`], and the distinction is a defect this rung already
    /// shipped once: the first version printed `"{} of {} bytes match"` from `bytes`
    /// **twice**, so the reassuring number was a tautology that would have read `65536 of
    /// 65536` over a loop that compared nothing. A reported count must come from the thing
    /// that did the counting — `measure_at_the_boundary_not_inside`, in miniature.
    pub bytes_compared: u64,
    /// ⊘ **Was the pattern written into the memfd at all?** `false` is the deliberate
    /// negative control ([`OsDescSeed::Never`]), where a mismatch is the PASS.
    pub seeded: bool,
    /// The submission's own cursors and release semaphore.
    pub submit: SubmitOutcome,
    /// The payload the engine was told to release.
    pub payload: u32,
}

impl OsDescEvidence {
    /// Was the mapping placed **exactly** where it was asked for?
    #[must_use]
    pub fn placed_as_asked(&self) -> bool {
        self.got_va == self.asked_va
    }

    /// ★ Did the comparison loop actually look at every byte that was copied?
    ///
    /// The guard on [`Self::bytes_compared`]: a loop that compared nothing reports zero
    /// here and `None` for [`Self::mismatch`], which without this check is
    /// indistinguishable from a perfect match.
    #[must_use]
    pub fn compared_everything(&self) -> bool {
        self.bytes_compared == self.bytes
    }

    /// Did the whole chain hold: placed as asked, engine retired, **every** byte compared,
    /// and every byte the GPU delivered a byte we wrote?
    ///
    /// ★ `before != after` is in the conjunction for R17's reason — bytes that match
    /// without the destination ever changing would mean the sentinel write, not the copy,
    /// is what we are reading. [`Self::compared_everything`] is in it for the same reason
    /// one step further out: a `None` mismatch over an empty loop is not agreement.
    ///
    /// ⊘ **Meaningless for [`OsDescSeed::Never`]**, where the correct answer is `false` and
    /// the caller must invert its own verdict rather than ask this.
    #[must_use]
    pub fn reached(&self) -> bool {
        self.placed_as_asked()
            && self.submit.semaphore == self.payload
            && self.before == self.sentinel
            && self.after != self.sentinel
            && self.compared_everything()
            && self.mismatch.is_none()
    }
}

/// ★★★ What [`HostRmBackend::prove_fb_memfd_join`] measured — **R32**.
///
/// ⊘ Every field here is a separate question, and a "did the ioctl succeed?" test scores
/// **six** of the seven failures as a pass:
///
/// | field | the question it answers alone |
/// |---|---|
/// | (an `Err` from the call) | may this process describe the *second* mapping's pages? |
/// | [`Self::join_after`] | are the two mappings one memory **at all**, before RM is involved? |
/// | [`Self::got_va`] vs [`Self::asked_va`] | did address identity survive? |
/// | [`Self::fwd_submit`] / [`Self::rev_submit`] | did each engine actually fetch and retire? |
/// | [`Self::fwd_mismatch`] | **J1** — did the GPU read what the OTHER mapping wrote? |
/// | [`Self::rev_before`] | non-vacuity: did the memfd hold the OLD pattern first? |
/// | [`Self::rev_mismatch`] | **J2** — did the OTHER mapping read what the GPU wrote? |
#[derive(Debug, Clone, Copy)]
pub struct FbJoinEvidence {
    /// The GPU VA `DMA_OFFSET_FIXED_TRUE` was asked for.
    pub asked_va: u64,
    /// The GPU VA RM reported.
    pub got_va: u64,
    /// How many bytes were described, mapped and copied — in each direction.
    pub bytes: u64,
    /// The join probe word read through `S` **before** `I` wrote it. Zero on a fresh
    /// memfd; anything else means the file was not pristine and the join is not a
    /// measurement.
    pub join_before: u32,
    /// The join probe word read through `S` **after** `I` wrote it.
    pub join_after: u32,
    /// What `I` wrote there.
    pub join_want: u32,
    /// The vidmem destination's word 0 before the forward copy — the sentinel.
    pub fwd_before: u32,
    /// Its word 0 after.
    pub fwd_after: u32,
    /// What [`Self::fwd_before`] was set to.
    pub fwd_sentinel: u32,
    /// **J1**: the first vidmem word that is not what `S` wrote into the memfd. `None`
    /// means every word matched.
    pub fwd_mismatch: Option<WordMismatch>,
    /// Counted by the forward comparison loop, never re-derived from [`Self::bytes`].
    pub fwd_bytes_compared: u64,
    /// The forward submission's cursors and release semaphore.
    pub fwd_submit: SubmitOutcome,
    /// The payload the forward copy was told to release.
    pub fwd_payload: u32,
    /// ★★ The memfd's word 0 **through `S`**, immediately before the reverse copy. In the
    /// seeded arm this must be [`Self::rev_first`] — never the reverse pattern, and never
    /// zero. It is what separates *"the engine wrote"* from *"we are reading our own
    /// earlier write"*.
    pub rev_before: u32,
    /// What [`Self::rev_before`] must be in the seeded arm: the forward pattern's first
    /// word.
    pub rev_first: u32,
    /// **J2**: the first memfd word, read **through `S`**, that is not what the engine was
    /// given. `None` means every word matched.
    pub rev_mismatch: Option<WordMismatch>,
    /// Counted by the reverse comparison loop.
    pub rev_bytes_compared: u64,
    /// The reverse submission's cursors and release semaphore.
    pub rev_submit: SubmitOutcome,
    /// The payload the reverse copy was told to release.
    pub rev_payload: u32,
    /// ⊘ Was the forward pattern written through `S` at all? `false` is the negative
    /// control, where a forward mismatch at word 0 is the PASS.
    pub seeded: bool,
}

impl FbJoinEvidence {
    /// Was the mapping placed exactly where it was asked for?
    #[must_use]
    pub fn placed_as_asked(&self) -> bool {
        self.got_va == self.asked_va
    }

    /// Are the two mappings one memory, measured with no GPU in the path?
    ///
    /// ★ Both halves: the probe word must have been **absent** before and **present**
    /// after. A store that answered [`Self::join_want`] unconditionally would pass the
    /// second half alone.
    #[must_use]
    pub fn joined(&self) -> bool {
        self.join_before == 0 && self.join_after == self.join_want
    }

    /// Did the forward comparison look at every byte that was copied?
    #[must_use]
    pub fn fwd_compared_everything(&self) -> bool {
        self.fwd_bytes_compared == self.bytes
    }

    /// Did the reverse comparison look at every byte that was copied?
    #[must_use]
    pub fn rev_compared_everything(&self) -> bool {
        self.rev_bytes_compared == self.bytes
    }

    /// **J1** — the GPU read, through a descriptor over mapping `I`, exactly what mapping
    /// `S` wrote.
    ///
    /// ⊘ Meaningless for [`OsDescSeed::Never`], where the correct answer is `false`.
    #[must_use]
    pub fn forward_reached(&self) -> bool {
        self.joined()
            && self.placed_as_asked()
            && self.fwd_submit.semaphore == self.fwd_payload
            && self.fwd_before == self.fwd_sentinel
            && self.fwd_after != self.fwd_sentinel
            && self.fwd_compared_everything()
            && self.fwd_mismatch.is_none()
    }

    /// **J2** — mapping `S` read exactly what the GPU wrote through the descriptor over
    /// mapping `I`.
    ///
    /// ★ [`Self::rev_before`] is in the conjunction for the reason the whole rung exists:
    /// bytes that match without the memfd ever having held something *else* first would
    /// mean we are reading the seed, not the engine's write. In the unseeded arm the memfd
    /// starts at zero, so the check is against that instead.
    #[must_use]
    pub fn reverse_reached(&self) -> bool {
        let expected_before = if self.seeded { self.rev_first } else { 0 };
        self.joined()
            && self.rev_submit.semaphore == self.rev_payload
            && self.rev_before == expected_before
            && self.rev_compared_everything()
            && self.rev_mismatch.is_none()
    }
}

/// ⊘ **Whether [`HostRmBackend::prove_os_descriptor`] writes the pattern at all** — the
/// negative control, as a parameter rather than a comment.
///
/// ★★★ This exists because a rung that has only ever been seen to pass is an instrument
/// with no demonstrated failure mode. `Never` runs the identical chain over a memfd nobody
/// wrote, so the copy engine reads the kernel's zero pages: a mismatch at word 0 is the
/// **expected** result, and a *match* would mean the comparison is not looking at what it
/// claims to.
///
/// **MEASURED** — `traces/real_ga106/rmladder_r25_osdescriptor_real_ga106.txt`, arms A and
/// C, RTX 3060 GA106 / 580.159.04, binary stamped `REV_UNDER_TEST=40d44db84`: the engine
/// delivered `0x00000000` at word 0 where the pattern would have been `0x5eed0001`, at both
/// euid 0 and euid 65534. ⇒ this arm is not hypothetical; the comparison has been watched
/// to fail on the same hardware that produced the green.
///
/// ⚠ It is not a "disable the check" flag. Both arms compare every word; they differ only
/// in what the correct answer is, and [`HostRmBackend::prove_os_descriptor`]'s caller must
/// invert its verdict accordingly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsDescSeed {
    /// Write the per-word pattern into the memfd **before** describing it to RM — the arm
    /// whose agreement is the rung's result.
    BeforeDescribe,
    /// Write nothing. The memfd is a fresh `memfd_create` + `ftruncate`, so its pages read
    /// as zero — and the pattern's first word is deliberately non-zero, so word 0 must
    /// mismatch.
    Never,
}

/// ★★★★★ **R31's evidence** — what happened when a host channel was created over memory
/// shaped exactly like the guest's ring, with the guest's own numbers.
///
/// Five separable facts, and they are separate fields for [`OsDescEvidence`]'s stated
/// reason: a single verdict over all of them would score *"RM refused the ring"* and
/// *"RM accepted a ring it should have refused"* as the same colour, and those send the
/// next reader to opposite places.
#[derive(Debug)]
pub struct GuestRingEvidence {
    /// **Arm A** — the fixed placement of the `OS_DESCRIPTOR` the channel's GPFIFO lives
    /// in. Asked, and RM's **[OUT]** `dmaOffset`.
    pub ring_asked_va: u64,
    /// What RM wrote back.
    pub ring_got_va: u64,
    /// The `gpFifoOffset` handed to the channel alloc — an absolute VA inside the mapping
    /// above, and deliberately **not** `ring_va + `[`GPFIFO_OFFSET`].
    pub gp_fifo_va: u64,
    /// The `gpFifoEntries` handed to the channel alloc — the count this bench's guest
    /// actually declares, and 64× ours.
    pub gp_fifo_entries: u32,
    /// What the channel alloc answered: the work-submit token, or RM's refusal.
    pub channel: Result<u64, RmError>,
    /// The layout the connection recorded for the channel it built, if it built one.
    pub declared: Option<(u64, u32)>,
    /// [`RmConnection::map_cpu_windowed`] entries before and after the channel alloc.
    /// ★ The measurement behind *"no CPU map is attempted on a guest-backed ring"*: the
    /// difference must be exactly **1** — USERD, which is ours.
    pub cpu_maps: (u64, u64),
    /// What [`HostRmBackend::ring_store_u32`] answered on the resulting channel. Must be
    /// [`RING_NOT_OURS`]; an `Ok` would mean a CPU view of the guest's ring exists.
    pub ring_store: Result<(), RmError>,
    /// **Arm B, the negative control on the mapping**: `NV_ESC_RM_MAP_MEMORY` issued
    /// deliberately against the guest-backed ring object. Expected to be refused; an `Ok`
    /// is a finding, not a pass, and the mapping is dropped immediately either way.
    pub cpu_map_of_guest_ring: Result<(), RmError>,
    /// **Arm C, the negative control on the binding**: the same channel alloc with a
    /// `gpFifoOffset` at an address **nothing was ever mapped at**. Expected to be refused
    /// — that refusal is what makes arm A's success a statement about the *binding* rather
    /// than about the ioctl being well-formed.
    pub unbound: Result<u64, RmError>,
    /// The address arm C named.
    pub unbound_va: u64,
}

impl GuestRingEvidence {
    /// The fixed map landed where it was asked to.
    #[must_use]
    pub fn placed_as_asked(&self) -> bool {
        self.ring_got_va == self.ring_asked_va
    }

    /// RM was told the caller's two numbers, unchanged.
    #[must_use]
    pub fn adopted_the_guests_numbers(&self) -> bool {
        self.declared == Some((self.gp_fifo_va, self.gp_fifo_entries))
    }

    /// Exactly one CPU mapping was asked for while the channel was built — USERD's.
    #[must_use]
    pub fn mapped_only_userd(&self) -> bool {
        self.cpu_maps.1 == self.cpu_maps.0 + 1
    }
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
            guest_ram: None,
            ce_witness: None,
        }
    }

    /// Install this isolate's guest-RAM plane (or, with `None`, state that it has none).
    #[must_use]
    pub fn with_guest_ram(mut self, plane: Option<Arc<crate::guestram::GuestRamPlane>>) -> Self {
        self.guest_ram = plane;
        self
    }

    /// ★★★ **E6** — install a recorder-only [`CeWitness`]. See that type for why it
    /// exists, why it is off by default, and why it cannot cross the sandbox.
    #[must_use]
    pub fn with_ce_witness(mut self, witness: Arc<CeWitness>) -> Self {
        self.ce_witness = Some(witness);
        self
    }

    /// ★★★ **E6 instrument** — allocate a **CPU-mappable** device-local buffer, the class
    /// [`HostRmBackend::prove_ce_copy`] already builds its two buffers from.
    ///
    /// # ⊘ Why a diagnostic needs its own allocator at all — this is a MEASURED fact
    ///
    /// `[measured]` 2026-08-03 on the RTX 3060 bench: [`RmBackend::alloc_sysmem`] — the
    /// verb every *published guest backing* is minted by — passes
    /// [`NVOS02_FLAGS_MAPPING_NO_MAP`], and `NV_ESC_RM_MAP_MEMORY` on the result is refused
    /// `NV_ERR_INVALID_ARGUMENT` (`0x1F`). That flag is **deliberate and documented**:
    /// *"right for a data buffer the GPU alone touches"*, and it stops the frontend
    /// building an `mmap` context around the descriptor at all
    /// (`ogkm-580: src/nvidia/arch/nvalloc/unix/src/escape.c:342-345`).
    ///
    /// ⇒ **A published backing is opaque to the CPU in both directions, by design.** So the
    /// R17 evidence shape — write a sentinel, copy, read back through an independent
    /// mapping — is *structurally unavailable* on one, and relaxing `NO_MAP` to make a
    /// diagnostic work would be changing the product to fit its instrument.
    ///
    /// ⊘ **This is not a second data path.** Nothing in the forwarding plane calls it; it
    /// exists so a hardware diagnostic can build an operand it can *see*, exactly as
    /// `prove_ce_copy` does one method over.
    ///
    /// # Errors
    /// Whatever RM refuses the allocation with.
    pub fn alloc_probe_local(&mut self, len: u64) -> Result<HostHandle, RmError> {
        let raw = self.conn.alloc_device_local(len)?;
        Ok(self.stamp(raw))
    }

    /// ★★ **E6 instrument** — fill `memory` with `len` bytes of the ramp
    /// `first, first+step, first+2*step, …`, one word at a time, through a CPU mapping
    /// this call opens and drops.
    ///
    /// `step == 0` writes a constant, which is how a **sentinel** is written; a non-zero
    /// step is how a source is filled so that a copy which moved only its first word is
    /// distinguishable from one that moved all of them.
    ///
    /// ★ The mapping is released before returning, and a release fence runs first: the
    /// stores go into a write-combining mapping and an engine must not be launched while
    /// they are still in a write-combining buffer. That ordering is not an optimisation —
    /// getting it wrong makes a *correct* copy read as a failed one.
    ///
    /// # Errors
    /// Whatever the mapping or the stores refuse with; [`RmError::BadHandle`] for a
    /// `memory` this connection never minted.
    pub fn fill_words(
        &self,
        memory: HostHandle,
        len: u64,
        first: u32,
        step: u32,
    ) -> Result<(), RmError> {
        let raw = self.narrow(memory)?;
        let (node, map) = self.conn.map_cpu(raw, len, CachePolicy::WriteCombining)?;
        for i in 0..(len / 4) {
            map.store_u32(
                HostOffset::new(i * 4),
                first.wrapping_add(step.wrapping_mul(i as u32)),
            )
            .map_err(|e| region_error(&e))?;
        }
        release_fence();
        drop(map);
        drop(node);
        Ok(())
    }

    /// ★★ **E6 instrument** — read the words at `offsets` out of `memory` through a
    /// **freshly opened, independent** mapping: a different device node, a different mmap
    /// context, a kernel-chosen address.
    ///
    /// ⊘ Independence is the whole content of the call. Reading back through the mapping
    /// the sentinel was written through proves a page is writable and nothing else, which
    /// is the failure `prove_ring_is_device_memory`'s docs already name one object over.
    ///
    /// # Errors
    /// Whatever the mapping or the loads refuse with; [`RmError::BadHandle`] for a
    /// `memory` this connection never minted.
    pub fn read_words_independently(
        &self,
        memory: HostHandle,
        len: u64,
        offsets: &[u64],
    ) -> Result<Vec<u32>, RmError> {
        let raw = self.narrow(memory)?;
        let (node, map) = self.conn.map_cpu(raw, len, CachePolicy::WriteCombining)?;
        let mut out = Vec::with_capacity(offsets.len());
        for &off in offsets {
            out.push(
                map.load_u32(HostOffset::new(off))
                    .map_err(|e| region_error(&e))?,
            );
        }
        drop(map);
        drop(node);
        Ok(out)
    }

    /// [`RmBackend::alloc_vaspace`]'s body, returning the **raw** `NV01_MEMORY_VIRTUAL`
    /// range handle instead of a port handle.
    ///
    /// ★★ It exists because there are now TWO kinds of address space in this backend and
    /// only one of them is the port's. [`HostRmBackend::executor_vas`] mints a space that
    /// is deliberately **not** reachable through a [`HostHandle`] — handing one out is
    /// exactly how the isolate's own memory ends up somewhere a guest can name — so it
    /// cannot go through the trait verb, and duplicating R7b's two-object dance is how the
    /// pairing gets forgotten.
    fn alloc_vaspace_raw(&mut self) -> Result<u32, RmError> {
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
                Ok(h)
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

    /// ★★★★★ **THE ONE MINT SITE for [`ExecutorVas`]** — the isolate's own address space
    /// over the guest `Vas` named by the raw range handle `guest_range`, built on first use.
    ///
    /// ⊘ **Nothing else in this crate may write `ExecutorVas { … }`.** The type's guarantee
    /// is *"no guest channel is bound to this space"*, and that is a claim about how the
    /// handle was **obtained**, which only the constructor can make. A second construction
    /// site is a second claim, made by whoever wrote it.
    /// `tests/executor_vas_census.rs` counts them.
    ///
    /// ★ Lazy rather than allocated alongside every `Vas`: a `Vas` that never carries an
    /// isolate copy costs nothing, and the cost of being wrong about that is one extra
    /// `FERMI_VASPACE_A`, not a wrong address.
    ///
    /// # Errors
    /// Whatever RM refused the address space or its range with.
    fn executor_vas(&mut self, guest_range: u32) -> Result<ExecutorVas, RmError> {
        if let Some(range) = self.conn.exec_vas_of(guest_range) {
            return Ok(ExecutorVas { range });
        }
        // ⊘ The mint is OUTSIDE the lock — it is three ioctls — so two pool workers can
        // reach here for the same `Vas`. `remember_exec_vas` reports the winner and the
        // loser disposes of what it built: an address space nothing can name is exactly the
        // orphan `alloc_vaspace`'s error arm exists to avoid.
        let mine = self.alloc_vaspace_raw()?;
        let winner = self.conn.remember_exec_vas(guest_range, mine);
        if winner != mine {
            let _ = self.free(self.stamp(mine));
        }
        Ok(ExecutorVas { range: winner })
    }

    /// ★★★ **Map `memory` into BOTH the guest-facing space and the isolate's own — at the
    /// SAME address.**
    ///
    /// This is what makes [`ExecutorVas`] affordable. The isolate's copy engine now lives
    /// in a space no guest channel is bound to, and the operands it is asked to copy are
    /// **guest VAs**; if those VAs resolved only in the guest's space, every forwarded copy
    /// would walk the host MMU into nothing (`Xid 31 FAULT_PDE`). So each publish is placed
    /// twice, at one address.
    ///
    /// ⊘ **The guest's address is unchanged and is still chosen first.** `at = Some(va)`
    /// demands it in the guest's space exactly as before; the shadow is then made to match
    /// **the address RM reported back**, never the one we asked for. With `at = None` RM
    /// picks, and the shadow follows. Either way the guest-facing placement is the
    /// authority and the isolate's copy is derived from it — the reverse would let a
    /// shadow failure silently relocate a guest VA.
    ///
    /// ★ It is all-or-nothing. A shadow that refuses tears the guest-side mapping down and
    /// returns the refusal, because a range mapped in one space and not the other is
    /// exactly the state that makes a later copy fault somewhere unrelated.
    ///
    /// # Errors
    /// Whatever either mapping refused, or [`RmError::PlacementRefused`] if the shadow
    /// could not take the guest-side address.
    fn map_dma_both(
        &mut self,
        guest_range: u32,
        memory: u32,
        len: u64,
        at: Option<u64>,
    ) -> Result<u64, RmError> {
        let va = self.conn.raw_map_dma(guest_range, memory, len, at)?;
        let exec = match self.executor_vas(guest_range) {
            Ok(e) => e,
            Err(e) => {
                let _ = self.conn.raw_unmap_dma(guest_range, va);
                return Err(e);
            }
        };
        match self.conn.raw_map_dma(exec.range, memory, len, Some(va)) {
            Ok(got) if got == va => Ok(va),
            Ok(got) => {
                let _ = self.conn.raw_unmap_dma(exec.range, got);
                let _ = self.conn.raw_unmap_dma(guest_range, va);
                Err(RmError::PlacementRefused { want: va, got })
            }
            Err(e) => {
                let _ = self.conn.raw_unmap_dma(guest_range, va);
                Err(e)
            }
        }
    }

    /// Undo one [`HostRmBackend::map_dma_both`]. The shadow first, so a failure to unmap it
    /// cannot leave the guest side free for reuse while the isolate's engine still
    /// resolves the address.
    fn unmap_dma_both(&mut self, guest_range: u32, va: u64) -> Result<(), RmError> {
        if let Some(exec) = self.conn.exec_vas_of(guest_range) {
            let _ = self.conn.raw_unmap_dma(exec, va);
        }
        self.conn.raw_unmap_dma(guest_range, va)
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
        self.alloc_vaspace_raw().map(|r| self.stamp(r))
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

    /// ★★★ **THE SECOND CROSSING'S OBJECT** — a blank host vidmem allocation.
    ///
    /// ⊘ **Not a port of a new primitive.** [`RmConnection::alloc_device_local`] has
    /// existed since the channel work (it is what a ring, a USERD block and a semaphore
    /// are built from) and issues exactly what the C issues here:
    /// `NV01_MEMORY_LOCAL_USER` (class `0x0040`) with `CONTIGUOUS | LOCATION_VIDMEM`
    /// (`C: nvkvm_gpu_emul.c:7286-7294`). This method only gives that allocation an
    /// **intent name** so the plan layer can ask for it without knowing the class — the
    /// same anti-bolt-on rule [`kayfabe_isolate::RmBackend::alloc_engine_object`] states.
    ///
    /// ⚠ The C aligns to `0x10000` and we align to `len`; `len` is the guest leaf's own
    /// size, so for every leaf this port will meet it is the stricter of the two. The
    /// difference is only ever more alignment, never less.
    ///
    /// ⊘ **The object is BLANK and this method does not pretend otherwise.** It is the C's
    /// `nvkvm_m2_host_alloc_vidmem_gpu_only` shape (`C: :7354-7368`): allocate, do not
    /// build a CPU view. The C chose that arm for a measured reason — the CPU mapping is
    /// what consumes the host's 256 MiB BAR1, its *"proven D2 wall"* (`C: :7340-7344`) —
    /// and this port has no CPU view for a different one: the isolate holds the mapping
    /// and the shell holds the framebuffer, and the descriptor that would join them
    /// ([`crate::proto::Request::ExportBacking`]) is not wired to this path.
    fn alloc_vidmem(&mut self, len: u64) -> Result<HostHandle, RmError> {
        if len == 0 {
            return Err(RmError::NoMemory);
        }
        let h = self.conn.alloc_device_local(len)?;
        Ok(self.stamp(h))
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
        hosting: Option<HostedObject<'_>>,
    ) -> Result<(HostHandle, u64), RmError> {
        // ★★★★★ §16.106 — THE GUEST'S OWN DECLARATION FIRST. See `declared_channel_engine_type`.
        // ★ Refused HERE rather than sent as a zero. See `engine_type_for`: a channel with
        // no engine type is not a channel with a default one, it is a channel on runlist 0.
        let engine_type = declared_channel_engine_type(engine, hosting)
            .or_else(|| engine_type_for(engine))
            .ok_or(RmError::Other(NOT_ON_THIS_RUNG))?;
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
        // ★★★ W229 — and the isolate's OWN address space over that `Vas` goes with it,
        // AFTER the channel whose ring is mapped in it. The order is not cosmetic: `free`
        // of the channel unmaps `ChannelParts::ring_va` through `ChannelParts::range`,
        // which for an isolate channel IS this space, and unmapping through a freed range
        // handle names an object RM has destroyed.
        if let Some(exec) = self.conn.forget_exec_vas(raw) {
            let _ = self.free_one(exec);
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
            // ★★★★★ **W230 — a channel over a [`GuestRing`] takes NOTHING of the ring with
            // it.** The mapping was made by the guest-RAM pin, at the guest's own VA, and
            // the `OS_DESCRIPTOR` is the pin's object; both outlive this channel by
            // construction, because the guest is still pushing into those pages. Unmapping
            // here would leave a live guest channel whose ring resolves to nothing, and
            // freeing here would un-pin pages RM is still DMAing into — with the second
            // symptom appearing anywhere but at this call.
            //
            // ⊘ It is also not a leak: `ChannelParts::owner` says whose it is, and the
            // owner frees it. What would be a leak is the opposite default.
            match parts.owner {
                RingOwner::Ours => {
                    keep(self.conn.raw_unmap_dma(parts.range, parts.ring_va));
                    keep(self.free_one(parts.tsg));
                    keep(self.free_one(parts.ring));
                }
                RingOwner::HandedIn => {
                    keep(self.free_one(parts.tsg));
                }
            }
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
        //
        // ★★★ **W229 — placed TWICE, at ONE address.** The guest-facing placement below is
        // unchanged, bit for bit; what is new is that the same object is also mapped at the
        // same VA in the isolate's own [`ExecutorVas`], because the isolate's copy engine
        // no longer lives in the guest's space and still has to resolve these operands. See
        // [`HostRmBackend::map_dma_both`].
        let h_dma = self.narrow(vas)?;
        let h_memory = self.narrow(memory)?;
        self.map_dma_both(h_dma, h_memory, len, Some(at.0))
    }

    fn unmap_gpu_va(&mut self, vas: HostHandle, gpu_va: u64) -> Result<(), RmError> {
        let h_dma = self.narrow(vas)?;
        self.unmap_dma_both(h_dma, gpu_va)
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
        //
        // ★★★★ **§16.70 — R26's TWO-FACT BAR, one plane over, PRINTED.** R26 settled that a
        // submission is believed on two facts — the placement RM reports back, and `GP_GET`
        // moving — and that `Ok(())` from the call under test is not one of them. The same
        // bar applies here and had no instrument: `[measured 2026-08-10, boot
        // p2_29e7c25_planereal]` three guest doorbells reported `forwarded (host channel
        // rung)` and the guest's scrubber died waiting for a completion, with **neither**
        // `GP_GET` nor the release semaphore read back anywhere a boot log could hold them.
        // [`SubmitOutcome`] has carried both since it existed; [`HostRmBackend::ce_witness`]
        // is the in-process recorder for them and has **zero production callers** (only
        // `tests/tests/e6_hw_join.rs`), so on a boot the two facts were computed and thrown
        // away.
        //
        // ⊘ This process is the isolate child; its stderr is QEMU's stderr, which
        // `scripts/bench/boot_nvkvm.sh` redirects to `run_<tag>_qemu.log`. So the line lands
        // in the boot's own on-disk evidence rather than in a session transcript — the trap
        // `CLAUDE.md` records for the guest's `dmesg`, avoided by construction.
        //
        // ★ Printed on the REFUSAL path too, and before the verdict, for
        // [`HostRmBackend::ce_witness`]'s own stated reason: a diagnostic that only speaks
        // when the submission got as far as hardware is silent on exactly the outcomes it is
        // run to see — `CeExecutor::Ours` and `CeSource::Constant` are both refused by
        // `ce_copy_outcome` *before* any ring store, and a scrubber's fill is a
        // `CeSource::Constant`.
        let result = self.ce_copy_outcome(vas, sub);
        let (outcome, payload) = match result {
            Ok(pair) => pair,
            Err(e) => {
                eprintln!(
                    "kayfabe-isolate: CE-SUBMIT dst={:#x} len={} by={:?} src={:?} → REFUSED \
                     BEFORE SUBMISSION {e:?} (no ring store, no doorbell, no semaphore)",
                    sub.dst, sub.len, sub.by, sub.src,
                );
                return Err(e);
            }
        };
        // ★★★ E6 — recorded BEFORE the verdict, and unconditionally: the interesting case
        // is the one where the copy did **not** retire, and a witness that only recorded
        // successes would be blind to exactly the outcome a diagnostic is run to see.
        if let Some(w) = &self.ce_witness {
            w.record(outcome, payload);
        }
        // ⊘ `gp_get` and `gp_put` are printed as the PAIR they are: `gp_get == gp_put` means
        // the engine fetched everything we published, `gp_get == 0` with `gp_put == 1` means
        // it fetched nothing at all, and one of those numbers alone cannot say either.
        eprintln!(
            "kayfabe-isolate: CE-SUBMIT dst={:#x} len={} by={:?} gp_get={} gp_put={} \
             sem={:#010x} want={:#010x} → {}",
            sub.dst,
            sub.len,
            sub.by,
            outcome.gp_get,
            outcome.gp_put,
            outcome.semaphore,
            payload,
            if outcome.semaphore == payload {
                "RETIRED"
            } else {
                "NEVER-RETIRED"
            },
        );
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

    /// ★★★★★ The real backend's guest-RAM door. It **decides nothing**: the grant's numbers
    /// are the VMM's, and everything this body adds is the refusal for an isolate that was
    /// never given a descriptor.
    fn map_guest_ram(&mut self, grant: GuestRamGrant) -> Result<GuestRamMapped, RmError> {
        let plane = self
            .guest_ram
            .as_ref()
            .ok_or(RmError::GuestRamUnavailable)?;
        let raw = plane.honour(grant)?;
        Ok(GuestRamMapped {
            // ★ Stamped through `HostHandle::new` directly rather than through `stamp`,
            // which narrows to RM's 32 bits: a guest-RAM name is deliberately WIDER than an
            // RM handle (`guestram::GUEST_RAM_NAME_TAG`) so that presenting one where an RM
            // object is expected is refused by `narrow` — a gate that already exists.
            region: HostHandle::new(self.id, raw),
            len: grant.len(),
        })
    }

    fn unmap_guest_ram(&mut self, mapped: GuestRamMapped) -> Result<(), RmError> {
        let plane = self
            .guest_ram
            .as_ref()
            .ok_or(RmError::GuestRamUnavailable)?;
        plane.release(mapped.region.raw())
    }

    /// ★★★★★ **`OS_DESCRIPTOR` OVER GUEST RAM** — the one call that makes the host GPU
    /// able to reach the guest's own pages, and it is `alloc_os_descriptor` applied to a
    /// mapping this isolate did not choose.
    ///
    /// ⊘ **`HostOffset::ZERO` and `mapped.len`, and neither is a decision.**
    /// [`crate::guestram::GuestRamPlane::honour`] mapped exactly the grant's slice, so
    /// offset zero of that mapping *is* the grant's first byte. Passing anything else here
    /// would be this process re-deriving a range the VMM already stated — the circularity
    /// `mode2_isolate_memory_boundary.md` §3 forbids, arriving through a parameter instead
    /// of through a request.
    ///
    /// ⚠ **The pages are now pinned by RM and stay pinned until the returned handle is
    /// freed.** Releasing the guest-RAM mapping does *not* release them; that asymmetry is
    /// `alloc_os_descriptor`'s own warning and is why the port carries the two names
    /// separately in [`kayfabe_isolate::VerbReply::GuestRamPinned`].
    fn describe_guest_ram(&mut self, mapped: GuestRamMapped) -> Result<HostHandle, RmError> {
        let plane = self
            .guest_ram
            .as_ref()
            .ok_or(RmError::GuestRamUnavailable)?;
        // ★ The closure keeps the `MappedRegion` inside the plane — `with_region`'s whole
        // shape — so the address never becomes a value this file can hold. `Indirect`
        // writes it into the ioctl argument and scrubs it back out, one crate down.
        let raw = plane.with_region(mapped.region.raw(), |region| {
            self.conn
                .alloc_os_descriptor(region, HostOffset::ZERO, mapped.len)
        })??;
        Ok(self.stamp(raw))
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
        self.alloc_channel_at(vas, engine_type, None)
    }

    /// ★★★ **R26 — a host channel whose ring lives at an address WE dictate.**
    ///
    /// [`Self::alloc_channel_on`]'s body, with one degree of freedom added: `ring_at`.
    /// `None` reproduces the previous behaviour exactly — RM chooses where the ring goes.
    /// `Some(va)` demands that address via `DMA_OFFSET_FIXED_TRUE` and **refuses** with
    /// [`RmError::PlacementRefused`] if RM reports a different one.
    ///
    /// # ★★ `ring_at` names the RING OBJECT'S BASE, not `gpFifoOffset`
    ///
    /// The two differ by [`GPFIFO_OFFSET`], and picking the wrong one of them is a silent
    /// off-by-a-page: the channel would be told its GPFIFO is where the **pushbuffer**
    /// lives, hardware would fetch 64 bytes of methods as GPFIFO entries, and the failure
    /// would surface as a wild `gpEntry` — nowhere near this call. So the parameter names
    /// the object, and `gpFifoOffset` is derived from it here, exactly as it was before.
    ///
    /// ⊘ **This is deliberately not the guest's `gpFifoOffset` yet.** A shadow-forwarded
    /// channel's ring is the *guest's* memory, and its whole 64 KiB layout —
    /// pushbuffer, GPFIFO, semaphore — is the guest's rather than
    /// [`PUSHBUFFER_OFFSET`]/[`GPFIFO_OFFSET`]/[`SEMAPHORE_OFFSET`]. What this verb
    /// establishes is the *one fact* that stood between here and there: that host RM will
    /// let a channel name a ring at an address its caller chose. Deriving `gpFifoOffset`
    /// from a guest-declared layout is the next increment and belongs with the memory that
    /// carries it.
    ///
    /// # ⊘ Why this is not (yet) a port verb
    ///
    /// [`RmBackend`] deliberately does not grow a method here. Nothing in the core has a
    /// dictated ring VA to pass — the shadow-forward that will is unbuilt — and a trait
    /// verb with no caller is the bolt-on `alloc_engine_object`'s docs already warn about
    /// one method up. It is `pub` for the same single reason [`Self::alloc_channel_on`]
    /// is: the `kayfabe-rm-ladder` diagnostic, which is the only thing that can ask
    /// hardware this question.
    ///
    /// # ★ The residual `raw_map_dma` named, now reachable
    ///
    /// `raw_map_dma`'s docs record that RM's own VA allocator and our fixed publishes
    /// share one address space, so RM *could* place something where a later fixed map is
    /// demanded. With `Some` that collision is now reachable **from this call** — and it
    /// surfaces as `PlacementRefused` or an RM status, both loud. It is still not silent
    /// corruption, and a host-private reservation is still the real fix.
    ///
    /// # Errors
    /// Whatever RM refuses with, or [`RmError::PlacementRefused`] if `ring_at` was named
    /// and not honoured — after unwinding everything already built, in both cases.
    pub fn alloc_channel_at(
        &mut self,
        vas: HostHandle,
        engine_type: u32,
        ring_at: Option<GpuVa>,
    ) -> Result<(HostHandle, u64), RmError> {
        let range = self.narrow(vas)?;
        self.alloc_channel_in(range, engine_type, RingSource::Ours(ring_at))
    }

    /// ★★★★★ **W230 — a host channel over the GUEST'S ring**: the same body, with the
    /// object hardware fetches from handed in instead of allocated.
    ///
    /// This is the verb the blocker asks for. See [`GuestRing`] for what each number is and
    /// whose it is; everything this method adds on top of that type is the refusal for a
    /// count that cannot be an index modulus, and the promise that **no CPU mapping of the
    /// guest's ring is attempted** ([`RING_NOT_OURS`], [`HostRmBackend::cpu_map_calls`]).
    ///
    /// # ⊘ What comes back is a channel that is NOT runnable, and saying so is the point
    ///
    /// RM will have accepted it, [`RmBackend::schedule`] will make it eligible, and it will
    /// still execute **nothing**, because the engine reads `GP_PUT` out of the USERD *we*
    /// gave it and nothing on this rung writes the guest's cursor into that word. A green
    /// return here is *"the host driver accepted the guest's ring"* and is not
    /// *"the guest's work runs"*.
    ///
    /// # Errors
    /// [`RmError::Other`] carrying [`RING_ENTRIES_REFUSED`] for a zero entry count,
    /// [`RmError::BadHandle`] for a `vas` or a ring handle this connection did not mint,
    /// and otherwise whatever RM refused — after unwinding everything already built, and
    /// **without** touching the handed-in ring.
    pub fn alloc_channel_over_guest_ring(
        &mut self,
        vas: HostHandle,
        engine_type: u32,
        ring: GuestRing,
    ) -> Result<(HostHandle, u64), RmError> {
        let range = self.narrow(vas)?;
        self.alloc_channel_in(range, engine_type, RingSource::Guest(ring))
    }

    /// ★★★ **W229 — the isolate's OWN channel, in the isolate's OWN address space.**
    ///
    /// [`Self::alloc_channel_at`]'s body over an [`ExecutorVas`] instead of a guest `Vas`.
    /// The two verbs differ in exactly one thing and it is the one that matters: which
    /// address space the channel's ring, USERD and completion semaphore land in.
    ///
    /// ⊘ There is no `HostHandle` overload of this. A caller who has a guest `Vas` has no
    /// way to spell an [`ExecutorVas`], which is the whole mechanism — see that type.
    ///
    /// # Errors
    /// As [`Self::alloc_channel_at`].
    fn alloc_channel_for_isolate(
        &mut self,
        vas: ExecutorVas,
        engine_type: u32,
    ) -> Result<(HostHandle, u64), RmError> {
        self.alloc_channel_in(vas.range, engine_type, RingSource::Ours(None))
    }

    /// The body all of the above share, over a raw `NV01_MEMORY_VIRTUAL` range.
    fn alloc_channel_in(
        &mut self,
        range: u32,
        engine_type: u32,
        ring: RingSource,
    ) -> Result<(HostHandle, u64), RmError> {
        // ★ The channel group names the ADDRESS SPACE, and `alloc_vaspace` returned the
        // mappable RANGE over it. A handle we never paired is not a `Vas` at all.
        let space = self
            .conn
            .space_of(range)
            .ok_or_else(|| RmError::BadHandle(self.stamp(range)))?;

        let unwind = |me: &mut Self, objs: &[u32]| {
            for h in objs.iter().rev() {
                let _ = me.free(me.stamp(*h));
            }
        };

        // ★★★★★ **G1 — WHERE THE RING COMES FROM, and it is now a question rather than a
        // line.** `Ours` allocates 64 KiB of device-local memory exactly as before.
        // `Guest` allocates **nothing**: the object is a handle handed in, over the guest's
        // own pages, and this connection neither made it nor may unmake it.
        let (ring_obj, owner) = match ring {
            RingSource::Ours(_) => (
                self.conn.alloc_device_local(RING_OBJECT_BYTES)?,
                RingOwner::Ours,
            ),
            RingSource::Guest(g) => {
                // ⊘ Refused HERE, before any host object exists, because it is the ONE
                // number in the guest's declaration this file cannot pass through: it is
                // the modulus of `submit_entry`'s wrap. See `RING_ENTRIES_REFUSED` for why
                // nothing else in the declaration is second-guessed.
                if g.gp_fifo_entries == 0 {
                    return Err(RmError::Other(RING_ENTRIES_REFUSED));
                }
                (self.narrow(g.memory)?, RingOwner::HandedIn)
            }
        };
        // What a later failure must give back. ⊘ The guest's ring is not in it on any arm,
        // and that is the invariant, not an optimisation: unwinding a channel must never
        // free memory the guest is still pushing into.
        let owned_ring = [ring_obj];
        let ours: &[u32] = match owner {
            RingOwner::Ours => &owned_ring,
            RingOwner::HandedIn => &[],
        };

        let userd = match self.conn.alloc_device_local(RING_OBJECT_BYTES) {
            Ok(h) => h,
            Err(e) => {
                unwind(self, ours);
                return Err(e);
            }
        };

        // The ring must be resolvable by hardware before a channel may name it.
        //
        // - `Ours` maps it here. With `None` RM picks the address — see `raw_map_dma` for
        //   why that does not weaken `#102`. With `Some`, `DMA_OFFSET_FIXED_TRUE` makes
        //   `dmaOffset` an [IN] parameter and the address is ours.
        // - ★★★ `Guest` maps **nothing**, and that is not a shortcut: the binding is the
        //   guest-RAM pin's, made at the guest's own VA and committed on the doorbell path,
        //   and re-mapping an already-placed object here would either fail or double-bind
        //   the guest's pages. ⇒ The channel alloc below is therefore the FIRST thing to
        //   test whether that binding exists, which is exactly why the host channel's birth
        //   has to move to the doorbell (`docs/design/guest_ring_adoption.md` §3).
        let (ring_va, layout) = match ring {
            RingSource::Ours(ring_at) => {
                let va = match self.conn.raw_map_dma(
                    range,
                    ring_obj,
                    RING_OBJECT_BYTES,
                    ring_at.map(|a| a.0),
                ) {
                    Ok(va) => va,
                    Err(e) => {
                        unwind(self, &[ours, &[userd]].concat());
                        return Err(e);
                    }
                };
                // ★★★ The placement check, and it reads RM's **[OUT]** `dmaOffset` rather
                // than the value we asked for. `raw_map_dma` returns what RM wrote back, so
                // this is a comparison between two different parties' numbers and not our
                // argument echoed.
                //
                // ⊘ A downgraded placement must never be adopted. A channel whose ring RM
                // quietly relocated is created, schedulable, and rings a doorbell — and the
                // *guest's* pushbuffer, which names the address we asked for, then walks
                // the host MMU into nothing. `RmError::PlacementRefused`'s own docs name
                // that end state: `Xid 31 FAULT_PDE`, a host-side fault with no
                // guest-visible cause.
                if let Some(want) = ring_at
                    && va != want.0
                {
                    let _ = self.conn.raw_unmap_dma(range, va);
                    unwind(self, &[ours, &[userd]].concat());
                    return Err(RmError::PlacementRefused {
                        want: want.0,
                        got: va,
                    });
                }
                (
                    va,
                    RingLayout {
                        gp_fifo_va: va + GPFIFO_OFFSET,
                        entries: GPFIFO_ENTRIES,
                    },
                )
            }
            // ★★ G2 + G3: the two numbers RM is about to be told are the GUEST'S, passed
            // through untouched. Neither is derived from `ring_va`, and neither is one of
            // this file's constants.
            RingSource::Guest(g) => (
                g.ring_va,
                RingLayout {
                    gp_fifo_va: g.gp_fifo_va,
                    entries: g.gp_fifo_entries,
                },
            ),
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
            unwind(self, &[ours, &[userd]].concat());
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
                unwind(self, &[ours, &[userd]].concat());
                return Err(e);
            }
        };

        let mut chan_params = [0u8; ChannelAllocParams::SIZE];
        let encoded = ChannelAllocParams {
            h_object_error: 0,
            // ★★★ **G2 + G3 — the two numbers that used to be constants.** For an
            // `Ours` ring `layout` still computes exactly `ring_va + GPFIFO_OFFSET` and
            // `GPFIFO_ENTRIES`, bit for bit; for a `Guest` ring they are the guest's own
            // declaration. ⊘ Neither is spelled here any more, because a constant at this
            // site is invisible to the one caller that must not use it.
            gp_fifo_offset: layout.gp_fifo_va,
            gp_fifo_entries: layout.entries,
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
            unwind(self, &[ours, &[userd, tsg]].concat());
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
                unwind(self, &[ours, &[userd, tsg]].concat());
                return Err(e);
            }
        };

        // ★★ BIND, on the GROUP, and it must come before the token control.
        let mut bind = [0u8; BIND_PARAMS_SIZE];
        bind.copy_from_slice(&engine_type.to_le_bytes());
        if let Err(e) = self.conn.raw_control(tsg, NVA06C_CTRL_CMD_BIND, &mut bind) {
            unwind(self, &[ours, &[userd, tsg, chan]].concat());
            return Err(e);
        }

        let mut token = [0u8; WORK_SUBMIT_TOKEN_PARAMS_SIZE];
        if let Err(e) = self.conn.raw_control(
            chan,
            NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN,
            &mut token,
        ) {
            unwind(self, &[ours, &[userd, tsg, chan]].concat());
            return Err(e);
        }
        let token = u32::from_le_bytes(token);

        // ★★★ R14 — the CPU mappings. Deliberately AFTER the token: everything above is a
        // fact about hardware that no byte of ours has touched, and everything below is
        // this process getting its hands on the channel. Keeping the order means a failure
        // here cannot be confused for a channel that never existed.
        // ★★★★★ **G4 — THE CPU MAP OF THE RING IS CONDITIONAL, AND THE CONDITION IS
        // PROVENANCE.**
        //
        // ⊘ On a guest-backed ring this is not an omission we can get away with; it is a
        // call that **cannot succeed**. `map_cpu` issues `NV_ESC_RM_MAP_MEMORY` against the
        // memory object, and the object here is an `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` over
        // pages RM pinned out of another process's address space. R31 arm B attempts it
        // deliberately and prints what the driver answered, so the claim in this comment is
        // a measurement rather than a plausible sentence.
        //
        // ★ And we do not need it. The isolate already holds those pages mapped — the same
        // `GuestRamPlane` grant that the `OS_DESCRIPTOR` was built from — so a second view
        // through RM would be a second name for memory this process can already read.
        // Every access this file would have made through the ring view is refused by name
        // instead (`RING_NOT_OURS`).
        let ring_view = match owner {
            // ★ Write-combining, and that is a claim about what this OBJECT is, not about
            // what it is used for: it is an `NV01_MEMORY_LOCAL_USER` allocation in the
            // framebuffer, so RM's mmap handler takes the write-combining branch
            // (`ogkm-580: nv-mmap.c:575-597`). The *uncached* sub-case two lines below it
            // is RM's own USERD window for a channel whose USERD the driver allocated —
            // not this one, which is our own vidmem object handed to the channel via
            // `hUserdMemory[0]`. Claiming uncached here because the word "USERD" appears
            // would be a comfortable guess, and the fence discipline is what makes
            // write-combining survivable.
            RingOwner::Ours => Some(self.conn.map_cpu(
                ring_obj,
                RING_OBJECT_BYTES,
                CachePolicy::WriteCombining,
            )),
            RingOwner::HandedIn => None,
        };
        let rings = match (
            ring_view,
            self.conn
                .map_cpu(userd, RING_OBJECT_BYTES, CachePolicy::WriteCombining),
        ) {
            (Some(Ok((ring_node, ring_map))), Ok((userd_node, userd_map))) => ChannelRings {
                _ring_node: Some(ring_node),
                ring: Some(ring_map),
                _userd_node: userd_node,
                userd: userd_map,
            },
            (None, Ok((userd_node, userd_map))) => ChannelRings {
                _ring_node: None,
                ring: None,
                _userd_node: userd_node,
                userd: userd_map,
            },
            (a, b) => {
                let a = a.and_then(Result::err);
                // Either half failing means the channel cannot be submitted to, so it is
                // torn down here rather than handed back as a channel that silently is not
                // one. The first error is the one reported.
                let e = a.or(b.err()).unwrap_or(RmError::Other(NOT_ON_THIS_RUNG));
                unwind(self, &[ours, &[userd, tsg, chan]].concat());
                return Err(e);
            }
        };
        self.conn.remember_rings(chan, rings);

        self.conn.remember_channel(
            chan,
            ChannelParts {
                tsg,
                ring: ring_obj,
                owner,
                userd,
                range,
                ring_va,
                layout,
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
    /// ★★★ **W230** — and on a channel over a [`GuestRing`] it is
    /// [`RmError::Other`]`(`[`RING_NOT_OURS`]`)`, always. That refusal *is* G4's assertion:
    /// the absence of the mapping is expressed as a named answer to the call that would
    /// have used it, not as a comment saying we did not make one.
    ///
    /// # Errors
    /// [`RmError::BadHandle`], [`RING_NOT_OURS`] if the ring is the guest's, or the bounds
    /// refusal if `offset` leaves the object.
    pub fn ring_store_u32(&self, chan: HostHandle, offset: u64, value: u32) -> Result<(), RmError> {
        let raw = self.narrow(chan)?;
        self.conn
            .with_rings(raw, |r| {
                r.ring
                    .as_ref()
                    .ok_or(RmError::Other(RING_NOT_OURS))?
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
                    .as_ref()
                    .ok_or(RmError::Other(RING_NOT_OURS))?
                    .load_u32(HostOffset::new(offset))
                    .map_err(|e| region_error(&e))
            })
            .unwrap_or(Err(RmError::BadHandle(chan)))
    }

    /// ★★★ **How many CPU mappings this connection has asked RM for, since it opened.**
    ///
    /// ⊘ It exists for one reason and it is not curiosity: *"we do not CPU-map the guest's
    /// ring"* is a claim about a call that **did not happen**, and the only way to measure
    /// an absence is to count the occurrences. A diagnostic that merely observed
    /// `ring_load_u32` refusing would be reading this file's own bookkeeping; this counts
    /// at the door every `NV_ESC_RM_MAP_MEMORY` in this process goes through.
    ///
    /// ★ [`HostRmBackend::alloc_channel_over_guest_ring`] must move it by exactly **one**
    /// — USERD, which is ours on every channel — where [`Self::alloc_channel_at`] moves it
    /// by two.
    #[must_use]
    pub fn cpu_map_calls(&self) -> u64 {
        self.conn
            .cpu_maps
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The GPFIFO layout `chan` was **created with**: `(gp_fifo_va, entries)`.
    ///
    /// ⊘ Read from [`ChannelParts`], which was written from the values handed to the
    /// channel alloc — so a caller comparing this against what it asked for is checking
    /// that the numbers reached RM, and **not** that RM agreed with them. Nothing but a
    /// submission hardware fetches says that.
    #[must_use]
    pub fn channel_ring_layout(&self, chan: HostHandle) -> Option<(u64, u32)> {
        let raw = self.narrow(chan).ok()?;
        self.conn
            .channel_parts(raw)
            .map(|p| (p.layout.gp_fifo_va, p.layout.entries))
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
        let slot = self.next_slot(raw)?;
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
        let raw = self.narrow(chan)?;
        let parts = self
            .conn
            .channel_parts(raw)
            .ok_or(RmError::BadHandle(chan))?;
        // ★★★ Refused HERE, at the top, on a channel whose ring is the guest's — not three
        // stores later. `ring_store_u32` would refuse anyway (`RING_NOT_OURS`), but the
        // failure would then be *"a store was refused"* on a function whose subject is a
        // SUBMISSION, and the offsets below are ours: `GPFIFO_OFFSET` is where OUR GPFIFO
        // sits inside OUR ring object, and the guest's ring has its own layout. ⇒ Composing
        // methods into a ring we do not own is the wrong verb, and it says so by name.
        if parts.owner == RingOwner::HandedIn {
            return Err(RmError::Other(RING_NOT_OURS));
        }
        let layout = parts.layout;
        let entry = gp_entry(pb_va, pb_len).ok_or(RmError::Other(BAD_ENCODE))?;
        let at = GPFIFO_OFFSET + slot * GP_ENTRY_SIZE;
        self.ring_store_u32(chan, at, entry as u32)?;
        self.ring_store_u32(chan, at + 4, (entry >> 32) as u32)?;

        release_fence();
        // ★ `GP_PUT` is an INDEX INTO THE RING, so it wraps with the ring: after the last
        // entry it is 0, not the entry count. Writing 64 into a 64-entry ring names an
        // entry that does not exist. Latent rather than live at this rung — nothing here
        // submits 64 times — which is exactly the kind of arithmetic that is wrong for a
        // year and then wrong at scale.
        //
        // ★★★ **W230 — the modulus is the CHANNEL'S**, for [`Self::next_slot`]'s reason:
        // the two are the same number for every ring this file allocates and differ by 64×
        // the moment the ring is the guest's.
        if layout.entries == 0 {
            return Err(RmError::Other(RING_ENTRIES_REFUSED));
        }
        let put = u32::try_from((slot + 1) % u64::from(layout.entries))
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

        // ★★★ W229 — the CE channel is built in the isolate's OWN address space, never in
        // `vas`. `vas` still names the space the OPERANDS live in, and `map_dma_both` has
        // placed them at the same addresses in both, which is why `src`/`dst` below need no
        // translation.
        let key = self.narrow(vas)?;
        let exec = self.executor_vas(key)?;
        let ce_chan = self.ce_channel(key, exec)?;
        let payload = ce_chan.next_payload;
        let chan = ce_chan.chan;
        let raw = self.narrow(chan)?;
        let parts = self
            .conn
            .channel_parts(raw)
            .ok_or(RmError::BadHandle(chan))?;
        let sem_va = parts.ring_va + SEMAPHORE_OFFSET;
        let slot = self.next_slot(raw)?;
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
    fn ce_channel(&mut self, key: u32, vas: ExecutorVas) -> Result<CeChannel, RmError> {
        if let Some(c) = self.ce_channels.get(&key) {
            return Ok(*c);
        }
        let (chan, token) = self.alloc_channel_for_isolate(vas, ENGINE_TYPE_COPY0)?;
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

    /// The next GPFIFO slot for `chan`, wrapping at **that channel's own entry count**.
    ///
    /// ★ Kept per **backend**, not per connection: `submit_entry` is the only writer and
    /// it runs under `&mut self`, so the counter needs no lock. A second worker submitting
    /// to the same channel would need one — and would need much more than a counter, which
    /// is why nothing here pretends to support it.
    ///
    /// ★★★ **W230 — the modulus is READ FROM THE CHANNEL, not from [`GPFIFO_ENTRIES`].**
    /// For every channel this file allocates its own ring for the two are the same number,
    /// so nothing about the isolate's submissions changes. They stop being the same the
    /// moment a channel is created over a [`GuestRing`], and the failure a constant would
    /// produce there is silent: a slot index taken modulo 64 in a 4096-entry ring is a
    /// legal entry, just not the one either party meant.
    ///
    /// # Errors
    /// [`RmError::BadHandle`] if `chan` is not a channel of this connection —
    /// deliberately, rather than falling back to the constant, because the fallback would
    /// be a guess about a ring whose geometry we did not find.
    fn next_slot(&mut self, chan: u32) -> Result<u64, RmError> {
        let entries = self
            .conn
            .channel_parts(chan)
            .ok_or_else(|| RmError::BadHandle(self.stamp(chan)))?
            .layout
            .entries;
        if entries == 0 {
            return Err(RmError::Other(RING_ENTRIES_REFUSED));
        }
        let n = self.slots.entry(chan).or_insert(0);
        let slot = *n % u64::from(entries);
        *n += 1;
        Ok(slot)
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

    /// ★★ Where this connection's records say `chan`'s ring object was placed, or `None`
    /// if `chan` is not a channel of ours.
    ///
    /// ★ It exists so a caller can check a placement **without asking the call that made
    /// it**. `alloc_channel_at` returning `Ok` is the thing under test; verifying it by
    /// reading its own return value is the R25 tautology one plane over. This reads
    /// `ChannelParts::ring_va`, which was written from RM's `[OUT]` `dmaOffset` — so a
    /// diagnostic comparing it against the address it asked for is comparing two parties.
    ///
    /// ⊘ It is still not hardware's word. Nothing but a submission that the engine
    /// **fetches** says the GPU agrees the ring is there; `SubmitOutcome::gp_get` is that
    /// word, and R26 requires both.
    #[must_use]
    pub fn channel_ring_va(&self, chan: HostHandle) -> Option<u64> {
        let raw = self.narrow(chan).ok()?;
        self.conn.channel_parts(raw).map(|p| p.ring_va)
    }

    /// ★★★ **W229 — which address space the isolate's OWN copy-engine control structures
    /// are in, reported next to the one a GUEST channel over the same `Vas` is bound to.**
    ///
    /// The owner's invariant is *"VMM state must never be placed where a guest VA can name
    /// it"*, and until this existed the only thing upholding it was a doc comment
    /// ([`RmConnection::raw_map_dma`]: *"memory the isolate allocated for itself, which no
    /// guest ever names"*). A sentence is not a measurement. This returns the **two range
    /// handles as separate fields precisely so a caller can compare them** — equality is
    /// the defect, and it is the whole reason the accessor reports both rather than
    /// answering a `bool` we computed ourselves.
    ///
    /// ⊘ Two equal handles are not *proof* of reachability and two different ones are not
    /// proof of unreachability; they are handles. What settles it is
    /// [`HostRmBackend::probe_va`] (is anything mapped at that VA in that space?) and
    /// [`HostRmBackend::probe_guest_reachability`] (does an engine bound to the guest's
    /// space read our word?). This accessor exists to tell those two probes *where to
    /// look*.
    ///
    /// `None` if `vas` is not a handle of this backend's, or if no copy-engine channel has
    /// been built over it yet — the placement does not exist before the channel does.
    #[must_use]
    pub fn ce_control_placement(&self, vas: HostHandle) -> Option<CeControlPlacement> {
        let guest_space = self.narrow(vas).ok()?;
        let ce = *self.ce_channels.get(&guest_space)?;
        let chan_raw = self.narrow(ce.chan).ok()?;
        let parts = self.conn.channel_parts(chan_raw)?;
        Some(CeControlPlacement {
            guest_space,
            control_space: parts.range,
            ring_va: parts.ring_va,
            sem_va: parts.ring_va + SEMAPHORE_OFFSET,
            ring_bytes: RING_OBJECT_BYTES,
            last_payload: ce.next_payload.wrapping_sub(1),
        })
    }

    /// ★★★ **Is anything mapped at `va` in the address space named by the raw range handle
    /// `space`?** — asked the only way this layer can ask it: by trying to put something
    /// there.
    ///
    /// A fresh device-local object is allocated and fixed-mapped at `va`. RM's **[OUT]**
    /// `dmaOffset` decides the answer, so this is two parties and not our own argument
    /// echoed — the shape `dictated_ring_negative` established on this hardware, reused
    /// because it is already calibrated.
    ///
    /// ⊘ **The limit, stated because the pass arm is the weak one.** [`VaProbe::Free`] says
    /// the VA was **unclaimed at this instant**, which is what *"a guest VA cannot name our
    /// semaphore"* reduces to in a GPU address space — an unmapped VA resolves to nothing
    /// and faults. It does **not** say a later mapping could not put something there, and
    /// it is not a statement about any other address. That is why the rung that uses it
    /// runs the **same call** against the space our ring *is* in, where the answer must be
    /// [`VaProbe::Occupied`]: a probe whose refusing arm is unreachable proves nothing.
    ///
    /// Everything allocated is freed before returning, on every arm.
    ///
    /// # Errors
    /// Only if the *allocation* failed — a refused placement is [`VaProbe::Occupied`],
    /// which is an answer rather than an error.
    pub fn probe_va(&mut self, space: u32, va: u64, len: u64) -> Result<VaProbe, RmError> {
        // ★★ THE LENGTH IS THE INSTRUMENT, and it is the caller's because only the caller
        // knows what it is asking about. `alloc_device_local` passes `alignment = len`, and
        // RM maps device-local memory with 64 KiB big pages regardless, so:
        //
        //   [measured 2026-08-10, `vh`] a 64 KiB probe object at `ring_va + 0x2000` was
        //   placed at `ring_va`  — the probe's own alignment, read as `Relocated`;
        //   [measured 2026-08-10, `vh`] a 4 KiB probe object at the same address was ALSO
        //   placed at `ring_va`  — so a smaller probe buys no resolution at all.
        //
        // ⊘ An instrument whose own geometry produces the answer it is looking for is not
        // an instrument, and a finer one that cannot be finer is worse: it looks like it
        // resolved something. ⇒ Ask about the OBJECT, at its own base and its own size.
        let obj = self.conn.alloc_device_local(len)?;
        let out = match self.conn.raw_map_dma(space, obj, len, Some(va)) {
            Ok(got) if got == va => {
                let _ = self.conn.raw_unmap_dma(space, got);
                VaProbe::Free
            }
            Ok(got) => {
                let _ = self.conn.raw_unmap_dma(space, got);
                VaProbe::Relocated(got)
            }
            Err(e) => VaProbe::Occupied(e),
        };
        let _ = self.free(self.stamp(obj));
        Ok(out)
    }

    /// ★★★★★ **W229's real falsifier — point a copy engine BOUND TO THE GUEST'S ADDRESS
    /// SPACE at the isolate's own semaphore and see whether it reads it.**
    ///
    /// [`HostRmBackend::probe_va`] asks RM's allocator a question about a VA.
    /// This asks **hardware** the question the invariant is actually about: a channel is
    /// created over `vas` — the same address space `kayfabe_fwd::plan_doorbell`
    /// materializes a *guest's* channel in — and made to `LAUNCH_DMA` four bytes out of
    /// `sem_va` into a scratch buffer we then read.
    ///
    /// ## The two submissions, and why the order is not negotiable
    ///
    /// 1. **The positive control runs FIRST**: the same probe channel copies from a scratch
    ///    source holding a known word. Without it, *"the probe copy never retired"* is
    ///    indistinguishable from *"this channel never worked"* — and the second is what a
    ///    typo produces. ⊘ A run whose control did not land reports nothing about the
    ///    probe.
    /// 2. **The probe** then copies from `sem_va`. If the isolate's ring is in this space,
    ///    the engine resolves the address and the word lands: [`GuestReach::Read`] carrying
    ///    a value the caller can compare against the payload **our** last copy released —
    ///    a number the guest-bound channel has no other way to obtain.
    ///
    /// ⚠ **A `NotResolved` verdict means the engine faulted**, which is the correct end
    /// state and is *not* free: the host `dmesg` will carry an `Xid 31 FAULT_PDE` for this
    /// channel, and the channel is dead afterwards. That is why this is a stand-alone
    /// diagnostic that tears its own channel down, and why the control precedes it.
    ///
    /// # Errors
    /// Whatever the allocations, mappings, channel or schedule refused — each before any
    /// submission, so an error here is never a fault.
    pub fn probe_guest_reachability(
        &mut self,
        vas: HostHandle,
        sem_va: u64,
    ) -> Result<GuestReachProbe, RmError> {
        const BYTES: u64 = 0x1_0000;
        /// Neither zero nor a plausible semaphore payload: the control's word must not be
        /// confusable with what the probe is looking for.
        const CONTROL_WORD: u32 = 0x5EA1_C071;
        /// What the destination holds before either copy. A word that survives is a copy
        /// that did not happen.
        const SENTINEL: u32 = 0xDEAD_0000;
        /// ★★★★★ **EVERY address this probe owns is DICTATED, and far away.**
        ///
        /// [measured 2026-08-10, `vh`, and it inverted the verdict] letting RM choose put
        /// the probe's OWN channel ring at `0x1_2002_0000` — the address the isolate's ring
        /// had just been freed from — so `sem_va = ring_va + 0x2000` landed **inside the
        /// probe's own ring**. The copy retired, moved `0x00000000`, and the rung read it as
        /// *"the address still resolves in the guest's space"*. It did: in the instrument's
        /// memory, not the isolate's.
        ///
        /// ⊘ A probe that allocates from the same allocator, in the same space, at the same
        /// moment, is not an independent observer. R26 established that a channel ring can
        /// be placed where its caller says, so there is no reason to let RM choose here.
        /// 64 KiB-aligned and three objects apart, well clear of RM's own base.
        const PROBE_RING_AT: u64 = 0x0000_0007_0000_0000;
        const CTRL_SRC_AT: GpuVa = GpuVa(0x0000_0007_0010_0000);
        const DST_AT: GpuVa = GpuVa(0x0000_0007_0020_0000);

        let range = self.narrow(vas)?;
        let ctrl_src = self.conn.alloc_device_local(BYTES)?;
        let dst = match self.conn.alloc_device_local(BYTES) {
            Ok(h) => h,
            Err(e) => {
                let _ = self.free(self.stamp(ctrl_src));
                return Err(e);
            }
        };

        let mut mapped: Vec<u64> = Vec::new();
        let mut chan: Option<HostHandle> = None;
        let mut go = || -> Result<GuestReachProbe, RmError> {
            // ⊘ `raw_map_dma`, NOT `map_dma_both`, and deliberately: these buffers stand
            // in for the GUEST's memory, and the channel below is bound to the guest's
            // space. Publishing them into the isolate's space too would be the rung
            // arranging for its own operands to resolve where the thing under test lives.
            //
            // ★ FIXED, and refused rather than adopted if RM disagrees — see
            // `PROBE_RING_AT`. An operand RM placed next to `sem_va` is an operand the
            // probe would then read instead of the thing it is asking about.
            let ctrl_src_va = self
                .conn
                .raw_map_dma(range, ctrl_src, BYTES, Some(CTRL_SRC_AT.0))?;
            mapped.push(ctrl_src_va);
            if ctrl_src_va != CTRL_SRC_AT.0 {
                return Err(RmError::PlacementRefused {
                    want: CTRL_SRC_AT.0,
                    got: ctrl_src_va,
                });
            }
            let dst_va = self.conn.raw_map_dma(range, dst, BYTES, Some(DST_AT.0))?;
            mapped.push(dst_va);
            if dst_va != DST_AT.0 {
                return Err(RmError::PlacementRefused {
                    want: DST_AT.0,
                    got: dst_va,
                });
            }

            // Seed both buffers through CPU mappings that are dropped before any engine
            // runs — the read-back below opens its own.
            {
                let (n, m) = self
                    .conn
                    .map_cpu(ctrl_src, BYTES, CachePolicy::WriteCombining)?;
                m.store_u32(HostOffset::new(0), CONTROL_WORD)
                    .map_err(|e| region_error(&e))?;
                drop(m);
                drop(n);
                let (n, m) = self.conn.map_cpu(dst, BYTES, CachePolicy::WriteCombining)?;
                m.store_u32(HostOffset::new(0), SENTINEL)
                    .map_err(|e| region_error(&e))?;
                m.store_u32(HostOffset::new(4), SENTINEL)
                    .map_err(|e| region_error(&e))?;
                drop(m);
                drop(n);
                release_fence();
            }

            // ★ THE STAND-IN FOR A GUEST CHANNEL. `alloc_channel_on` over the `Vas`'s own
            // range is exactly what `plan_doorbell` reaches for a guest submission, engine
            // and all — the point of the rung is that this channel is ORDINARY.
            let (c, token) =
                self.alloc_channel_at(vas, ENGINE_TYPE_COPY0, Some(GpuVa(PROBE_RING_AT)))?;
            chan = Some(c);
            let mut params = [0u8; CeAllocParams::SIZE];
            CeAllocParams {
                version: CeAllocParams::VERSION_1,
                engine_type: ENGINE_TYPE_COPY0,
            }
            .encode_into(&mut params)
            .map_err(|_| RmError::Other(BAD_ENCODE))?;
            self.alloc_ce_engine_object(c, self.conn.classes.ce_object(), &params)?;
            self.schedule(c)?;

            let control = self.probe_copy(c, token, ctrl_src_va, dst_va, 1)?;
            let (node, view) = self.conn.map_cpu(dst, BYTES, CachePolicy::WriteCombining)?;
            let control_read = view
                .load_u32(HostOffset::new(0))
                .map_err(|e| region_error(&e))?;
            drop(view);
            drop(node);

            // ⊘ The probe is not issued at all if the control did not land: a fault
            // provoked on a channel that was never shown to work is a measurement of
            // nothing, and it costs a real `Xid`.
            if !(control.landed(1) && control_read == CONTROL_WORD) {
                return Ok(GuestReachProbe {
                    control,
                    control_read,
                    control_want: CONTROL_WORD,
                    reach: GuestReach::ControlFailed,
                });
            }

            let probe = self.probe_copy(c, token, sem_va, dst_va + 4, 2)?;
            let (node, view) = self.conn.map_cpu(dst, BYTES, CachePolicy::WriteCombining)?;
            let probe_read = view
                .load_u32(HostOffset::new(4))
                .map_err(|e| region_error(&e))?;
            drop(view);
            drop(node);

            let reach = if probe.landed(2) {
                GuestReach::Read {
                    word: probe_read,
                    outcome: probe,
                }
            } else if probe_read != SENTINEL {
                // ⊘ Neither arm: bytes moved and the engine did not say so. Reported
                // rather than folded into one of the two, because a partial answer that
                // looks like a clean one is how a green gets believed.
                GuestReach::Ambiguous {
                    word: probe_read,
                    outcome: probe,
                }
            } else {
                GuestReach::NotResolved(probe)
            };
            Ok(GuestReachProbe {
                control,
                control_read,
                control_want: CONTROL_WORD,
                reach,
            })
        };
        let out = go();
        if let Some(c) = chan {
            let _ = self.free(c);
        }
        for va in mapped.into_iter().rev() {
            let _ = self.conn.raw_unmap_dma(range, va);
        }
        let _ = self.free(self.stamp(dst));
        let _ = self.free(self.stamp(ctrl_src));
        out
    }

    /// One four-byte `LAUNCH_DMA` on `chan`, releasing `payload`. The submission half of
    /// [`HostRmBackend::probe_guest_reachability`], factored out only because that rung
    /// issues it twice and the two must be identical in everything but their operands.
    fn probe_copy(
        &mut self,
        chan: HostHandle,
        token: u64,
        src: u64,
        dst: u64,
        payload: u32,
    ) -> Result<SubmitOutcome, RmError> {
        let raw = self.narrow(chan)?;
        let parts = self
            .conn
            .channel_parts(raw)
            .ok_or(RmError::BadHandle(chan))?;
        let sem_va = parts.ring_va + SEMAPHORE_OFFSET;
        let slot = self.next_slot(raw)?;
        let pb_off = PUSHBUFFER_OFFSET + slot * PUSHBUFFER_SLOT_BYTES;
        let pb_va = parts.ring_va + pb_off;
        let words = ce_pushbuffer(CePush {
            class_id: self.conn.classes.ce_object(),
            src,
            dst,
            len: 4,
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
        self.submit_entry(chan, pb_va, 4 * words.len() as u64, slot, token)?;
        self.await_semaphore(chan, SEMAPHORE_OFFSET, payload, CE_COPY_TIMEOUT)
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
            // ★ W229 — through `map_dma_both`, exactly as a production publish is: the
            // isolate's copy engine is in its own space now, and an operand mapped only in
            // the guest's would fault. RM still chooses the address, in the guest's space,
            // and the shadow follows it.
            let src_va = self.map_dma_both(range, src, BYTES, None)?;
            cleanup[0].1 = Some(src_va);
            let dst_va = self.map_dma_both(range, dst, BYTES, None)?;
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
                let _ = self.unmap_dma_both(range, va);
            }
            let _ = self.free(self.stamp(h));
        }
        out
    }

    /// ★★★★★ **R29 — the SAME proof as [`Self::prove_os_descriptor`], but through the
    /// PRODUCTION verbs**: the guest-RAM plane, the port's `describe_guest_ram`, and a
    /// fixed `map_dma` at an address the caller dictates.
    ///
    /// # ⊘ Why R25 does not already answer this, stated because it nearly does
    ///
    /// R25 settled the *ioctl*: a sealed `memfd`, described to RM, placed as asked, read by
    /// a real engine, byte-identical —
    /// `traces/real_ga106/rmladder_r25_osdescriptor_real_ga106.txt`. ★ That is a real result
    /// and this rung does not re-open it. What R25 exercises is a **parallel
    /// implementation**: it maps the block itself, into its own `Reservation`, and hands the
    /// region straight to `alloc_os_descriptor`. Not one line of the code the VMM path runs
    /// is on that route.
    ///
    /// ⇒ This rung runs the route that ships: [`crate::guestram::GuestRamPlane::honour`]
    /// mints the mapping from a **grant**, [`kayfabe_isolate::RmBackend::describe_guest_ram`]
    /// borrows it through `with_region` and describes it, and
    /// [`kayfabe_isolate::RmBackend::map_gpu_va`] places it and refuses a placement it did
    /// not get. A defect in any of the three would leave R25 green.
    ///
    /// ## ★★ The grant's offset is NON-ZERO, deliberately
    ///
    /// `offset` selects a window *inside* the block. At zero, a plane that ignored the
    /// grant's offset entirely would map the same pages and every assertion would still
    /// hold. Here the window is the **second** half and the first half carries a decoy word,
    /// so a plane that mapped from zero shows up as a value mismatch rather than as nothing.
    ///
    /// ## ⊘ What a pass does NOT establish
    ///
    /// - **Not that any guest byte was pinned.** The block is a `memfd` this process made;
    ///   it is *shaped* like guest RAM and it is not the guest's.
    /// - **Not the cap-dropped case**, for R25's reason exactly — `euid` is printed.
    /// - **Nothing about a guest VA.** The address is one we chose.
    ///
    /// # Errors
    /// Whatever the plane, the descriptor or the placement refused — each by its own name,
    /// so a failure attributes to one of the three.
    pub fn prove_guest_ram_pin(
        &mut self,
        vas: HostHandle,
        at: GpuVa,
        pattern: u32,
    ) -> Result<GuestRamPinEvidence, RmError> {
        use kayfabe_isolate::{GuestRamGrant, RmBackend};
        const HALF: u64 = 0x1_0000;
        const BYTES: u64 = 2 * HALF;
        let page = HostPageSize::query();

        // 1 — the block, and the plane over it. ★ The SAME types the isolate is spawned
        // with: `SharedRam` is what a VMM's shareable backing is, and `GuestRamPlane` is the
        // one door guest memory comes through in the child.
        let ram = kayfabe_linux_raw::SharedRam::create(BYTES).map_err(|e| region_error(&e))?;
        // The VMM's own view, for writing the pattern. Two mappings of one block is exactly
        // the shape the crossing is: the VMM writes, the isolate reads the same pages.
        let ours = kayfabe_linux_raw::MappedRegion::map(
            Backing::SharedFile {
                fd: ram.as_backing_fd(),
                offset: 0,
            },
            BYTES,
            kayfabe_linux_raw::HostProt::ReadWrite,
            CachePolicy::WriteBack,
            page,
        )
        .map_err(|e| region_error(&e))?;
        let mut image = vec![0u8; BYTES as usize];
        for i in 0..(BYTES as usize) / 4 {
            // ⊘ The two halves carry DIFFERENT words. A plane that ignored the grant's
            // offset would describe the first half, and the read below would find the decoy
            // rather than find nothing.
            let w = if (i as u64) * 4 < HALF {
                !pattern
            } else {
                pattern.wrapping_add(i as u32)
            };
            image[4 * i..4 * i + 4].copy_from_slice(&w.to_le_bytes());
        }
        ours.write_from(HostOffset::new(0), &image)
            .map_err(|e| region_error(&e))?;

        let fd = ram.dup_for_export().map_err(|e| region_error(&e))?;
        let plane = Arc::new(crate::guestram::GuestRamPlane::new(fd, BYTES, page));
        let restore = self.guest_ram.replace(Arc::clone(&plane));

        // 2 — the PRODUCTION chain, verb for verb, in the order `VerbPlan::PinGuestRam`
        // runs it.
        let mut go = || -> Result<GuestRamPinEvidence, RmError> {
            let mapped = self.map_guest_ram(GuestRamGrant::originated_by_the_vmm(
                HALF,
                HALF,
                kayfabe_vmm::Prot::ReadWrite,
            ))?;
            let memory = match self.describe_guest_ram(mapped) {
                Ok(m) => m,
                Err(e) => {
                    let _ = self.unmap_guest_ram(mapped);
                    return Err(e);
                }
            };
            let got_va = match self.map_gpu_va(vas, memory, HALF, at) {
                Ok(v) => v,
                Err(e) => {
                    let _ = self.free(memory);
                    let _ = self.unmap_guest_ram(mapped);
                    return Err(e);
                }
            };
            // 3 — ★ read the described pages back through the ISOLATE's own mapping, which
            // is the only thing that can say the plane mapped the window the grant named.
            // ⊘ Not the GPU's view, and it does not claim to be: R25 already proved a real
            // engine reads these pages, and a second CE round trip here would give a rung
            // whose subject is the ROUTE a second subject.
            let first = plane.with_region(mapped.region.raw(), |r| {
                let mut buf = [0u8; 4];
                r.read_into(HostOffset::new(0), &mut buf).map(|()| buf)
            })?;
            let first = first.map_err(|e| region_error(&e))?;
            let evidence = GuestRamPinEvidence {
                asked_va: at.0,
                got_va,
                bytes: HALF,
                offset: HALF,
                first_word: u32::from_le_bytes(first),
                expected_word: pattern.wrapping_add((HALF as u32) / 4),
            };
            // Undo everything. A ladder rung that leaked would poison the next one.
            let _ = self.unmap_gpu_va(vas, got_va);
            let _ = self.free(memory);
            let _ = self.unmap_guest_ram(mapped);
            Ok(evidence)
        };
        let out = go();
        self.guest_ram = restore;
        out
    }

    /// ★★★ **R25 — does memory shaped like guest RAM reach the host GPU's MMU?**
    ///
    /// The whole chain, once, on real hardware, with every step's failure distinguishable
    /// from every other step's:
    ///
    /// ```text
    ///   SharedRam::create            a sealed memfd — what a VMM backs guest RAM with
    ///   Reservation + map_fixed_in   MAP_SHARED into a range we own (the GuestWindow shape)
    ///   write a per-word pattern     ordinary CPU stores, write-back
    ///   alloc_os_descriptor          RM pins those pages                        <- arm B
    ///   raw_map_dma(FIXED, at)       into a host VAS at an address WE choose     <- arm C
    ///   ce_copy(src = that VA)       a real engine reads it, real semaphore      <- arm ⊘
    ///   read the destination back    through a mapping opened after the copy
    /// ```
    ///
    /// ## ★★ Why the destination is device-local and not a second descriptor
    ///
    /// If both ends were the same kind of memory, a copy that moved nothing but happened to
    /// find matching bytes would be indistinguishable from one that worked. The destination
    /// is vidmem, pre-filled with a **sentinel** through its own mapping and read back
    /// through a second, independent one — R17's discipline, reused because it is the part
    /// that makes the answer non-vacuous. The bytes arriving in the destination therefore
    /// travelled: `our CPU store -> memfd page -> RM's pin -> host GPU VAS -> engine ->
    /// vidmem`, and only the first and last are ours.
    ///
    /// ## ⊘ What this CANNOT see, stated here rather than discovered later
    ///
    /// The VA is one **we** choose, so nothing here says whether a host GPU walking a host
    /// VAS built from *guest* VAs would miss — and with fault delivery unbuilt, such a miss
    /// is a hang inside UVM's replayable-fault loop rather than an error. That is a limit of
    /// this rung, not a gap in it.
    ///
    /// ★★★★★ **R31 — will host RM build a channel whose command queue is memory we did
    /// not allocate, at the guest's own numbers?**
    ///
    /// The blocker this rung exists for, asked of hardware with no guest in the picture: we
    /// allocate a host channel with **its own** queue, which stays empty, while the guest
    /// pushes into **its** queue, which our channel does not read. The fix is not a copier —
    /// it is to name the guest's queue in the channel alloc and let the engine fetch from
    /// it. Everything before that is unmeasurable without this answer.
    ///
    /// ## The three arms, and the second and third are the ones that make the first mean
    /// something
    ///
    /// - **A** — a sealed `memfd` (what a VMM backs guest RAM with) → `OS_DESCRIPTOR` →
    ///   **fixed** map at an address we dictate → a channel whose `gpFifoOffset` is an
    ///   absolute VA *inside that mapping* and whose `gpFifoEntries` is **4096**, the count
    ///   this bench's guest actually declares (`run_w229b_…_qemu.log`), not our 64 and not
    ///   the fixture's 512. The channel is never scheduled and never rung.
    /// - **B, the mapping control** — `NV_ESC_RM_MAP_MEMORY` issued *deliberately* against
    ///   that same descriptor. G4 claims a CPU view of a guest-backed ring cannot be had;
    ///   this is the line that tests it instead of asserting it. ⊘ It is expected to be
    ///   **refused**, and an `Ok` is reported as a finding rather than quietly dropped.
    /// - **C, the binding control** — the same channel alloc with `gpFifoOffset` at
    ///   [`Self::prove_guest_ring_channel`]'s `UNBOUND_AT`, an address **nothing has ever
    ///   been mapped at** in this freshly allocated address space. If RM refuses it, arm A's
    ///   acceptance is a statement about the *binding*; if RM accepts it, arm A proved only
    ///   that the ioctl was well-formed — and this rung says so out loud.
    ///
    /// ## ★★ Every address this prover owns is DICTATED, and it does not share an allocator
    /// with what it observes
    ///
    /// W229's probe was handed the address its own ring had just been freed from and scored
    /// a correct boundary as a violation. So: the descriptor's VA, the `gpFifoOffset` inside
    /// it and arm C's unbound VA are three constants, 4 GiB apart, in an address space this
    /// call allocates and frees itself; and the `gpFifoOffset` is deliberately **not**
    /// `ring_va + `[`GPFIFO_OFFSET`], so a regression that fell back to our constant would
    /// change the number RM was told rather than reproduce it.
    ///
    /// ## ⊘ What a green arm A does NOT establish
    ///
    /// Not that the guest's work runs — nothing writes `GP_PUT`, so the engine has nothing
    /// to fetch, and this rung does not schedule or ring the channel at all. Not that the
    /// pages are coherent (that is R25). Not that a *guest*-declared VA resolves in a host
    /// VAS built from guest page tables. It establishes exactly one thing: **host RM will
    /// build a channel over a queue it did not allocate, at an address and an entry count
    /// its caller states.**
    ///
    /// # Errors
    /// Only the setup can fail this way — the memfd, the reservation, the descriptor and
    /// its fixed map. Every arm's own outcome is carried inside [`GuestRingEvidence`],
    /// because a refusal from RM is this rung's *result* and not its failure.
    pub fn prove_guest_ring_channel(
        &mut self,
        vas: HostHandle,
    ) -> Result<GuestRingEvidence, RmError> {
        /// 64 KiB, the R25 shape: large enough to hold a 4096-entry GPFIFO (32 KiB) at a
        /// non-zero offset inside it.
        const BYTES: u64 = 0x1_0000;
        /// Where the descriptor is fixed-mapped. ⊘ Used by no other rung: R25 uses
        /// `0x3_0040_0000`, `probe_guest_reachability` uses `0x7_…`, R30 its own.
        const RING_AT: GpuVa = GpuVa(0x0000_0009_0000_0000);
        /// The guest's `gpFifoOffset` **inside** that mapping. ★ `0x3000`, deliberately not
        /// [`GPFIFO_OFFSET`]: the whole claim is that the layout is the caller's.
        const GP_FIFO_IN_RING: u64 = 0x3000;
        /// [measured 2026-08-10, `run_w229b_b66bd44_execvas_real_qemu.log`] the entry counts
        /// this guest declares are **32**, **1024** and **4096** — the ring that carries the
        /// doorbells we forward is the 4096 one. Never 64 (ours) and never 512 (the ABI
        /// fixture's).
        const GUEST_ENTRIES: u32 = 4096;
        /// Arm C's `gpFifoOffset`: 4 GiB above the descriptor's mapping, in an address space
        /// this call allocated, and never mapped by anything.
        const UNBOUND_AT: u64 = 0x0000_000B_0000_0000;

        let range = self.narrow(vas)?;
        let page = HostPageSize::query();

        // 1 — the backing a VMM gives guest RAM, exactly as R25 builds it.
        let ram = kayfabe_linux_raw::SharedRam::create(BYTES).map_err(|e| region_error(&e))?;
        let mut reservation =
            kayfabe_linux_raw::Reservation::new(BYTES, page).map_err(|e| region_error(&e))?;
        let placed = reservation
            .map_fixed_in(
                HostOffset::new(0),
                BYTES,
                Backing::SharedFile {
                    fd: ram.as_backing_fd(),
                    offset: 0,
                },
                kayfabe_linux_raw::HostProt::ReadWrite,
                CachePolicy::WriteBack,
            )
            .map_err(|e| region_error(&e))?;
        let region = reservation
            .placement(placed)
            .map_err(|e| region_error(&e))?;

        // 2 — RM's own object over those pages. This is the handle the channel will be
        // handed; nothing below allocates a ring.
        let desc = self
            .conn
            .alloc_os_descriptor(region, HostOffset::new(0), BYTES)?;

        // 3 — the binding. ⊘ `raw_map_dma`, not `map_dma_both`: this rung's channel lives in
        // the guest-facing space only and submits nothing, so publishing a shadow copy would
        // add a second mapping the measurement would then have to account for.
        let ring_got_va = match self.conn.raw_map_dma(range, desc, BYTES, Some(RING_AT.0)) {
            Ok(va) => va,
            Err(e) => {
                let _ = self.free(self.stamp(desc));
                return Err(e);
            }
        };
        let gp_fifo_va = ring_got_va + GP_FIFO_IN_RING;

        // 4 — arm A. The counter is read on both sides of the ALLOC and of nothing else.
        let cpu_before = self.cpu_map_calls();
        let built = self.alloc_channel_over_guest_ring(
            vas,
            ENGINE_TYPE_COPY0,
            GuestRing {
                memory: self.stamp(desc),
                ring_va: ring_got_va,
                gp_fifo_va,
                gp_fifo_entries: GUEST_ENTRIES,
            },
        );
        let cpu_after = self.cpu_map_calls();

        let (channel, declared, ring_store) = match built {
            Ok((chan, token)) => {
                let declared = self.channel_ring_layout(chan);
                // ⊘ A store of a value that could not be mistaken for a GPFIFO entry, at
                // offset 0, and it must be REFUSED. This is G4 asserted rather than omitted.
                let store = self.ring_store_u32(chan, 0, 0xBAD0_BAD0);
                let _ = self.free(chan);
                (Ok(token), declared, store)
            }
            Err(e) => (Err(e), None, Err(RmError::Other(NOT_ON_THIS_RUNG))),
        };

        // 5 — arm B, the mapping control. ★ Which line this is expected to execute:
        // `RmConnection::map_cpu_windowed`'s `status_check(out.status)` — i.e. the driver
        // answering the escape, not a bounds check of ours. An `Ok` here is dropped
        // immediately and reported as a finding.
        let cpu_map_of_guest_ring =
            match self.conn.map_cpu(desc, BYTES, CachePolicy::WriteCombining) {
                Ok((node, map)) => {
                    drop(map);
                    drop(node);
                    Ok(())
                }
                Err(e) => Err(e),
            };

        // 6 — arm C, the binding control. Same call, same descriptor, same entry count; the
        // ONLY thing that changes is that `gpFifoOffset` names an address nothing was mapped
        // at. ★ Which line this is expected to execute: `RmConnection::alloc_gpfifo_channel`
        // returning a non-zero RM status, i.e. the `Err(e)` arm of the channel alloc inside
        // `alloc_channel_in` — after the group was built and before any CPU mapping.
        let unbound = self
            .alloc_channel_over_guest_ring(
                vas,
                ENGINE_TYPE_COPY0,
                GuestRing {
                    memory: self.stamp(desc),
                    ring_va: UNBOUND_AT,
                    gp_fifo_va: UNBOUND_AT,
                    gp_fifo_entries: GUEST_ENTRIES,
                },
            )
            .map(|(chan, token)| {
                let _ = self.free(chan);
                token
            });

        let _ = self.conn.raw_unmap_dma(range, ring_got_va);
        let _ = self.free(self.stamp(desc));
        drop(reservation);
        drop(ram);

        Ok(GuestRingEvidence {
            ring_asked_va: RING_AT.0,
            ring_got_va,
            gp_fifo_va,
            gp_fifo_entries: GUEST_ENTRIES,
            channel,
            declared,
            cpu_maps: (cpu_before, cpu_after),
            ring_store,
            cpu_map_of_guest_ring,
            unbound,
            unbound_va: UNBOUND_AT,
        })
    }

    /// # Errors
    /// Whatever the memfd, the mapping, the descriptor alloc, the DMA map or the copy
    /// refuses with. An `Err` from the descriptor alloc is the falsifier's **arm B** and is
    /// the one a caller must report by its RM status rather than as "R25 failed".
    pub fn prove_os_descriptor(
        &mut self,
        vas: HostHandle,
        at: GpuVa,
        pattern: u32,
        seed: OsDescSeed,
    ) -> Result<OsDescEvidence, RmError> {
        const BYTES: u64 = 0x1_0000;
        const WORDS: u64 = BYTES / 4;
        let range = self.narrow(vas)?;
        let page = HostPageSize::query();
        let sentinel = !pattern;

        // 1 — the backing a VMM gives guest RAM: a sealed, shareable memfd.
        let ram = kayfabe_linux_raw::SharedRam::create(BYTES).map_err(|e| region_error(&e))?;
        // 2 — placed inside a reservation, which is the `GuestWindow` shape rather than a
        // bare `mmap`: the address space is acquired at a kernel-chosen address first and
        // the backing is `MAP_FIXED` into a hole we demonstrably own.
        let mut reservation =
            kayfabe_linux_raw::Reservation::new(BYTES, page).map_err(|e| region_error(&e))?;
        let placed = reservation
            .map_fixed_in(
                HostOffset::new(0),
                BYTES,
                Backing::SharedFile {
                    fd: ram.as_backing_fd(),
                    offset: 0,
                },
                kayfabe_linux_raw::HostProt::ReadWrite,
                CachePolicy::WriteBack,
            )
            .map_err(|e| region_error(&e))?;
        let region = reservation
            .placement(placed)
            .map_err(|e| region_error(&e))?;

        // 3 — the pattern, by ordinary CPU stores through a write-back mapping. Per-word so
        // a copy that moved only a header, or a length truncated to one dword, is visible.
        //
        // ⊘ `OsDescSeed::Never` skips exactly this and nothing else: the memfd's pages stay
        // as `ftruncate` left them, which is zero. Everything downstream — the descriptor,
        // the mapping, the engine, the whole-buffer compare — runs identically, so a run
        // that still reports a match is reporting on something other than these bytes.
        let seeded = seed == OsDescSeed::BeforeDescribe;
        if seeded {
            let mut image = vec![0u8; BYTES as usize];
            for i in 0..WORDS as usize {
                image[4 * i..4 * i + 4]
                    .copy_from_slice(&pattern.wrapping_add(i as u32).to_le_bytes());
            }
            region
                .write_from(HostOffset::new(0), &image)
                .map_err(|e| region_error(&e))?;
        }

        // 4 — arm B. Reported by its own `Err` so a refusal here is never read as a copy
        // that did not land.
        let desc = self
            .conn
            .alloc_os_descriptor(region, HostOffset::new(0), BYTES)?;

        let dst = match self.conn.alloc_device_local(BYTES) {
            Ok(h) => h,
            Err(e) => {
                let _ = self.free(self.stamp(desc));
                return Err(e);
            }
        };
        let mut cleanup: Vec<(u32, Option<u64>)> = vec![(desc, None), (dst, None)];
        let mut go = || -> Result<OsDescEvidence, RmError> {
            // 5 — arm C. `Some(at)` sets `DMA_OFFSET_FIXED_TRUE`; the returned VA is
            // compared against `at` by the caller, because `Ok` is not placement.
            // ★ W229 — `map_dma_both`, because the copy below runs on the isolate's own
            // channel in its own address space. The FIXED ask is unchanged and is still
            // made against the guest-facing space; the shadow follows the address RM
            // reported, so a relocation is still this rung's finding and not hidden by it.
            let got_va = self.map_dma_both(range, desc, BYTES, Some(at.0))?;
            cleanup[0].1 = Some(got_va);
            let dst_va = self.map_dma_both(range, dst, BYTES, None)?;
            cleanup[1].1 = Some(dst_va);

            let (dst_node, dst_map) = self.conn.map_cpu(dst, BYTES, CachePolicy::WriteCombining)?;
            for i in 0..WORDS {
                dst_map
                    .store_u32(HostOffset::new(i * 4), sentinel)
                    .map_err(|e| region_error(&e))?;
            }
            let before = dst_map
                .load_u32(HostOffset::new(0))
                .map_err(|e| region_error(&e))?;
            release_fence();
            drop(dst_map);
            drop(dst_node);

            // 6 — a real engine, waiting on its own release semaphore. No forged completion.
            let (submit, payload) = self.ce_copy_outcome(
                vas,
                CeSubCopy {
                    dst: dst_va,
                    src: CeSource::Address(got_va),
                    len: BYTES,
                    by: CeExecutor::HostCe,
                },
            )?;

            // 7 — read back through a mapping opened AFTER the copy, and compare EVERY
            // word. ★ The whole-buffer compare is the point of arm ⊘: a coherency or
            // cache-policy failure is not "the copy did not happen", it is *some* of the
            // bytes being the ones we wrote. A first-and-last check would score a partial
            // page as a pass.
            let (node, second) = self.conn.map_cpu(dst, BYTES, CachePolicy::WriteCombining)?;
            let mut mismatch = None;
            // ★ Counted by the loop that does the comparing, never re-derived from `BYTES`.
            // The first version of this rung printed `BYTES` twice and called it a result.
            let mut bytes_compared = 0u64;
            for i in 0..WORDS {
                let got = second
                    .load_u32(HostOffset::new(i * 4))
                    .map_err(|e| region_error(&e))?;
                let want = pattern.wrapping_add(i as u32);
                bytes_compared += 4;
                if got != want {
                    mismatch = Some(WordMismatch { word: i, got, want });
                    break;
                }
            }
            let after = second
                .load_u32(HostOffset::new(0))
                .map_err(|e| region_error(&e))?;
            drop(second);
            drop(node);
            Ok(OsDescEvidence {
                asked_va: at.0,
                got_va,
                bytes: BYTES,
                before,
                after,
                sentinel,
                mismatch,
                bytes_compared,
                seeded,
                submit,
                payload,
            })
        };
        let out = go();
        for (h, va) in cleanup.into_iter().rev() {
            if let Some(va) = va {
                let _ = self.unmap_dma_both(range, va);
            }
            // ⚠ Freeing the descriptor object is what un-pins the guest-RAM pages. Dropping
            // `reservation` below only unmaps our own view of them.
            let _ = self.free(self.stamp(h));
        }
        drop(reservation);
        drop(ram);
        out
    }

    /// ★★★ **R32 — the framebuffer memfd JOIN: is ONE memfd, mapped TWICE, ONE memory
    /// on BOTH sides of the GPU?**
    ///
    /// R25 ([`Self::prove_os_descriptor`]) measured a sealed memfd described to RM, placed
    /// at a dictated VA and read correctly by a real copy engine. ⊘ It measured that
    /// through **one** mapping and in **one** direction, and the framebuffer-memfd design
    /// rests on neither of those being the limit:
    ///
    /// | | property | R25 | why the FB port needs it |
    /// |---|---|---|---|
    /// | **J1** | write through mapping **S**, describe mapping **I**, the GPU reads **S**'s bytes | ⊘ **no** — R25 writes and describes through the same [`kayfabe_linux_raw::MappedRegion`] | the shell holds the BAR view and the isolate holds the described view. They are different mappings; a design proved only through the described one has not been proved |
    /// | **J2** | the GPU **writes** and a CPU mapping **reads** it back | ⊘ **no** — R25 is CPU-write → GPU-read only | ★ this is the direction `cuCtxCreate` is stuck on. The guest's completion semaphore is a word the **engine writes** and the **guest reads**; every byte of OS_DESCRIPTOR evidence this tree owns runs the other way |
    ///
    /// # The chain
    ///
    /// ```text
    ///   memfd  = SharedRam::create(BYTES)              ONE sealed memfd
    ///   S      = Reservation A + map_fixed_in(memfd)   "the shell's BAR view"
    ///   I      = Reservation B + map_fixed_in(memfd)   "the isolate's describe view"
    ///
    ///   0. CPU join   : write a probe word through S, read it through I
    ///   1. seed       : write P1 through S             (skipped by `OsDescSeed::Never`)
    ///   2. describe I : alloc_os_descriptor(I, 0, BYTES)
    ///   3. map        : FIXED at `at`
    ///   4. FORWARD    : CE copies memfd -> vidmem; compare vidmem against P1   ⇒ J1
    ///   5. reload     : write P2 into vidmem through its own CPU map
    ///   6. REVERSE    : CE copies vidmem -> memfd; compare **through S** against P2 ⇒ J2
    /// ```
    ///
    /// ★ **Step 6 reads through `S`, never through `I`.** Reading back through the mapping
    /// that was described would leave *"did the other mapping see it"* unasked, which is
    /// the whole of J1 and J2.
    ///
    /// ★ **Three patterns, all distinguishable.** Zero, `P1` and `P2` are different, so the
    /// reverse arm's three failure modes print apart: `0` = the copy never landed; `P1` =
    /// we are reading step 1's own write and the engine did nothing; `P2` = the engine
    /// wrote and `S` saw it. A control that only asked *"is it P2?"* would collapse the
    /// middle case into the first and lose the one reading that names it.
    ///
    /// ⊘ **The CPU join probe sits at the LAST word**, not the first, so that
    /// [`OsDescSeed::Never`]'s forward compare still meets a pristine zero at word 0 and
    /// breaks there. Placing it at word 0 would have made the negative control read its
    /// own probe.
    ///
    /// ⊘ **Two mappings, one process.** The cross-process case adds `SCM_RIGHTS` and
    /// nothing else about the memory; that step is *reasoned*, not measured here, and is
    /// labelled as such wherever it is used.
    ///
    /// # Errors
    /// Whatever the memfd, either mapping, the descriptor alloc, either DMA map or either
    /// copy refuses with. An `Err` from the descriptor alloc is R25's **arm B** and must be
    /// reported by its RM status rather than as "R32 failed".
    pub fn prove_fb_memfd_join(
        &mut self,
        vas: HostHandle,
        at: GpuVa,
        seed: OsDescSeed,
    ) -> Result<FbJoinEvidence, RmError> {
        const BYTES: u64 = 0x1_0000;
        const WORDS: u64 = BYTES / 4;
        /// The forward pattern — what mapping `S` writes and the GPU must read.
        const P1: u32 = 0x5EED_0001;
        /// The reverse pattern — what the GPU writes and mapping `S` must read.
        /// ⊘ Deliberately unrelated to [`P1`] and to zero.
        const P2: u32 = 0xB0B0_0001;
        /// The CPU-level join probe, at the last word so the negative control's forward
        /// compare still meets zero at word 0.
        const JOIN: u32 = 0x1010_FACE;
        let range = self.narrow(vas)?;
        let page = HostPageSize::query();
        let sentinel = !P1;
        let join_off = HostOffset::new(BYTES - 4);

        // 1 — ONE memfd. Everything below is two views of these pages.
        let ram = kayfabe_linux_raw::SharedRam::create(BYTES).map_err(|e| region_error(&e))?;

        // 2 — TWO independent reservations, each `MAP_FIXED` over the same descriptor. Two
        // `mmap` calls at two kernel-chosen addresses: distinct mappings by construction.
        // ⊘ Their addresses are NOT reported, and cannot be: `MappedRegion::addr_at` is
        // `pub(crate)` by a deliberate refusal — no representation of a host address
        // crosses that crate boundary. Distinctness is therefore structural, and *sharing*
        // is what this rung measures (step 3).
        let mut res_s =
            kayfabe_linux_raw::Reservation::new(BYTES, page).map_err(|e| region_error(&e))?;
        let placed_s = res_s
            .map_fixed_in(
                HostOffset::new(0),
                BYTES,
                Backing::SharedFile {
                    fd: ram.as_backing_fd(),
                    offset: 0,
                },
                kayfabe_linux_raw::HostProt::ReadWrite,
                CachePolicy::WriteBack,
            )
            .map_err(|e| region_error(&e))?;
        let shell = res_s.placement(placed_s).map_err(|e| region_error(&e))?;

        let mut res_i =
            kayfabe_linux_raw::Reservation::new(BYTES, page).map_err(|e| region_error(&e))?;
        let placed_i = res_i
            .map_fixed_in(
                HostOffset::new(0),
                BYTES,
                Backing::SharedFile {
                    fd: ram.as_backing_fd(),
                    offset: 0,
                },
                kayfabe_linux_raw::HostProt::ReadWrite,
                CachePolicy::WriteBack,
            )
            .map_err(|e| region_error(&e))?;
        let described = res_i.placement(placed_i).map_err(|e| region_error(&e))?;

        // 3 — the CPU join, before RM exists in this story at all. If these two mappings
        // were not one memory, nothing downstream could be.
        let mut w = [0u8; 4];
        shell.read_into(join_off, &mut w).map_err(|e| region_error(&e))?;
        let join_before = u32::from_le_bytes(w);
        described
            .write_from(join_off, &JOIN.to_le_bytes())
            .map_err(|e| region_error(&e))?;
        shell.read_into(join_off, &mut w).map_err(|e| region_error(&e))?;
        let join_after = u32::from_le_bytes(w);

        // 4 — the forward seed, through **S**, per word.
        //
        // ⊘ `OsDescSeed::Never` skips exactly this and nothing else. Everything downstream
        // runs identically, so a run that still reports a forward match is reporting on
        // something other than these bytes.
        let seeded = seed == OsDescSeed::BeforeDescribe;
        if seeded {
            let mut image = vec![0u8; BYTES as usize];
            for i in 0..WORDS as usize {
                image[4 * i..4 * i + 4].copy_from_slice(&P1.wrapping_add(i as u32).to_le_bytes());
            }
            shell
                .write_from(HostOffset::new(0), &image)
                .map_err(|e| region_error(&e))?;
        }

        // 5 — describe the OTHER mapping. This is the line the whole rung is about: RM
        // pins `I`'s pages, and every byte compared afterwards was written or read through
        // `S`.
        let desc = self
            .conn
            .alloc_os_descriptor(described, HostOffset::new(0), BYTES)?;
        let dst = match self.conn.alloc_device_local(BYTES) {
            Ok(h) => h,
            Err(e) => {
                let _ = self.free(self.stamp(desc));
                return Err(e);
            }
        };
        let mut cleanup: Vec<(u32, Option<u64>)> = vec![(desc, None), (dst, None)];
        let mut go = || -> Result<FbJoinEvidence, RmError> {
            let got_va = self.map_dma_both(range, desc, BYTES, Some(at.0))?;
            cleanup[0].1 = Some(got_va);
            let dst_va = self.map_dma_both(range, dst, BYTES, None)?;
            cleanup[1].1 = Some(dst_va);

            // 6 — sentinel the vidmem destination, so a forward match cannot be the
            // destination having already held the answer.
            let (node, map) = self.conn.map_cpu(dst, BYTES, CachePolicy::WriteCombining)?;
            for i in 0..WORDS {
                map.store_u32(HostOffset::new(i * 4), sentinel)
                    .map_err(|e| region_error(&e))?;
            }
            let fwd_before = map
                .load_u32(HostOffset::new(0))
                .map_err(|e| region_error(&e))?;
            release_fence();
            drop(map);
            drop(node);

            // 7 — FORWARD. A real engine, waiting on its own release semaphore.
            let (fwd_submit, fwd_payload) = self.ce_copy_outcome(
                vas,
                CeSubCopy {
                    dst: dst_va,
                    src: CeSource::Address(got_va),
                    len: BYTES,
                    by: CeExecutor::HostCe,
                },
            )?;

            // 8 — compare the vidmem against what **S** wrote, through a mapping opened
            // after the copy; then reload it with P2 for the reverse arm. One mapping does
            // both because `NV_ESC_RM_MAP_MEMORY` is one-shot per descriptor and a third
            // node is a third failure mode for no gain.
            let (node, map) = self.conn.map_cpu(dst, BYTES, CachePolicy::WriteCombining)?;
            let mut fwd_mismatch = None;
            let mut fwd_compared = 0u64;
            for i in 0..WORDS {
                let got = map
                    .load_u32(HostOffset::new(i * 4))
                    .map_err(|e| region_error(&e))?;
                let want = P1.wrapping_add(i as u32);
                fwd_compared += 4;
                if got != want {
                    fwd_mismatch = Some(WordMismatch { word: i, got, want });
                    break;
                }
            }
            let fwd_after = map
                .load_u32(HostOffset::new(0))
                .map_err(|e| region_error(&e))?;
            for i in 0..WORDS {
                map.store_u32(HostOffset::new(i * 4), P2.wrapping_add(i as u32))
                    .map_err(|e| region_error(&e))?;
            }
            release_fence();
            drop(map);
            drop(node);

            // 9 — the memfd's word 0 through **S**, immediately before the reverse copy.
            // ★ Non-vacuity for J2, and it is the reading that separates the three failure
            // modes: it must be P1 in the seeded arm (step 4's own write), never P2.
            let mut w0 = [0u8; 4];
            shell
                .read_into(HostOffset::new(0), &mut w0)
                .map_err(|e| region_error(&e))?;
            let rev_before = u32::from_le_bytes(w0);

            // 10 — REVERSE. The engine writes into the described memfd.
            let (rev_submit, rev_payload) = self.ce_copy_outcome(
                vas,
                CeSubCopy {
                    dst: got_va,
                    src: CeSource::Address(dst_va),
                    len: BYTES,
                    by: CeExecutor::HostCe,
                },
            )?;

            // 11 — ★★★ J2. Read every word back **through S** — the mapping RM was never
            // told about — and compare against what the engine was given.
            let mut image = vec![0u8; BYTES as usize];
            shell
                .read_into(HostOffset::new(0), &mut image)
                .map_err(|e| region_error(&e))?;
            let mut rev_mismatch = None;
            let mut rev_compared = 0u64;
            for i in 0..WORDS as usize {
                let got = u32::from_le_bytes(
                    image[4 * i..4 * i + 4]
                        .try_into()
                        .map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?,
                );
                let want = P2.wrapping_add(i as u32);
                rev_compared += 4;
                if got != want {
                    rev_mismatch = Some(WordMismatch {
                        word: i as u64,
                        got,
                        want,
                    });
                    break;
                }
            }
            Ok(FbJoinEvidence {
                asked_va: at.0,
                got_va,
                bytes: BYTES,
                join_before,
                join_after,
                join_want: JOIN,
                fwd_before,
                fwd_after,
                fwd_sentinel: sentinel,
                fwd_mismatch,
                fwd_bytes_compared: fwd_compared,
                fwd_submit,
                fwd_payload,
                rev_before,
                rev_first: P1,
                rev_mismatch,
                rev_bytes_compared: rev_compared,
                rev_submit,
                rev_payload,
                seeded,
            })
        };
        let out = go();
        for (h, va) in cleanup.into_iter().rev() {
            if let Some(va) = va {
                let _ = self.unmap_dma_both(range, va);
            }
            // ⚠ Freeing the descriptor object is what un-pins the pages. Dropping the
            // reservations below only unmaps this process's two views of them.
            let _ = self.free(self.stamp(h));
        }
        drop(res_i);
        drop(res_s);
        drop(ram);
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

    /// The eight bytes a CE object declares, as the guest sends them.
    fn ce_params(version: u32, engine_type: u32) -> Vec<u8> {
        let mut b = vec![0u8; CeAllocParams::SIZE];
        CeAllocParams {
            version,
            engine_type,
        }
        .encode_into(&mut b)
        .expect("encode");
        b
    }

    /// ★★★★★ **§16.106 — the channel follows the OBJECT'S declared copy engine.**
    ///
    /// The 14 refusals of `w250`/`w251`/`w254` are `COPY0` (ours, runlist 0) against
    /// `COPY2`/`COPY3` (the guest's, runlists 1 and 2). This is the decision that removes
    /// them, tested where it is made.
    #[test]
    fn a_ce_channel_takes_the_engine_the_guest_declared() {
        let copy2 = engine_type_copy(2).expect("COPY2");
        let copy3 = engine_type_copy(3).expect("COPY3");
        for (declared, want) in [(copy2, copy2), (copy3, copy3)] {
            let params = ce_params(CeAllocParams::VERSION_1, declared);
            let hosting = HostedObject {
                class: ClassId(0xc7b5),
                params: &params,
            };
            assert_eq!(
                declared_channel_engine_type(EngineKind::Ce, Some(hosting)),
                Some(want),
                "the declared ordinal reaches the channel unchanged"
            );
            // ⊘ …and the number it replaces is the one the host driver called `current`.
            assert_eq!(engine_type_for(EngineKind::Ce), Some(ENGINE_TYPE_COPY0));
            assert_ne!(want, ENGINE_TYPE_COPY0);
        }
    }

    /// ★★ VERSION_0 numbers the same field as a bare **instance index**, so the two
    /// versions must not be read through one lens
    /// (`ogkm-580: kernel_ce_context.c:115-125`).
    #[test]
    fn version_zero_is_an_index_and_version_one_is_an_ordinal() {
        let by_index = ce_params(CeAllocParams::VERSION_0, 2);
        let by_ordinal = ce_params(
            CeAllocParams::VERSION_1,
            engine_type_copy(2).expect("COPY2"),
        );
        let of = |p: &[u8]| {
            declared_channel_engine_type(
                EngineKind::Ce,
                Some(HostedObject {
                    class: ClassId(0xc7b5),
                    params: p,
                }),
            )
        };
        assert_eq!(of(&by_index), of(&by_ordinal), "both name COPY2");
        // ⊘ And the same bytes under the other version name a DIFFERENT engine — which is
        // why the version is decoded rather than assumed.
        assert_eq!(of(&ce_params(CeAllocParams::VERSION_1, 2)), None);
    }

    /// ★★★ **The fall-through is byte-identical to the old behaviour**, and it must be:
    /// every arm that cannot name an engine from the guest's own declaration returns
    /// `None` so `alloc_channel` reaches `engine_type_for` exactly as before.
    ///
    /// ⊘ `None` here is *"nothing was declared"*, never *"copy engine 0"* — the two
    /// arrive at the same ordinal by different routes and only one of them is a claim.
    #[test]
    fn nothing_declarable_falls_through_untouched() {
        let good = ce_params(
            CeAllocParams::VERSION_1,
            engine_type_copy(2).expect("COPY2"),
        );
        let host = |engine, params: &[u8]| {
            declared_channel_engine_type(
                engine,
                Some(HostedObject {
                    class: ClassId(0xc7b5),
                    params,
                }),
            )
        };
        // A GR channel that later takes a CE object binds it as GRCE and must NOT move —
        // these are the 8 forwards that already succeed.
        assert_eq!(host(EngineKind::GrCompute, &good), None);
        assert_eq!(host(EngineKind::GrGraphics, &good), None);
        // No object at all (the doorbell materialization).
        assert_eq!(declared_channel_engine_type(EngineKind::Ce, None), None);
        // Short, absent, unknown-version, and not-a-copy-ordinal params.
        assert_eq!(host(EngineKind::Ce, &[]), None);
        assert_eq!(host(EngineKind::Ce, &[1, 0, 0]), None);
        assert_eq!(host(EngineKind::Ce, &ce_params(7, 11)), None);
        assert_eq!(
            host(EngineKind::Ce, &ce_params(CeAllocParams::VERSION_1, 1)),
            None,
            "ENGINE_TYPE_GRAPHICS is not a copy engine and is never read as one"
        );
    }
}
