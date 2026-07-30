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
//! | R2 | `NV_ESC_CHECK_VERSION_STR` query | a version string; **not** a gate |
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
//! ## ★★ The verbs that are NOT implemented, and why that is a refusal
//!
//! This section used to list five. `alloc_channel`, `alloc_engine_object` and `schedule`
//! are now real (R13) and proven on hardware. **`ring_doorbell`, `ce_copy` and
//! `export_surface` still return [`RmError::Other`] carrying [`NOT_ON_THIS_RUNG`]**, and
//! each names what it lacks at its own definition: a doorbell *mapping* rather than an
//! ioctl, the mapped ring the doorbell announces, a PRIME export.
//!
//! Returning a plausible success would be the exact failure `mode2_real_forward_not_fake`
//! forbids: *"prove compute via HW sema/util, never green-guest-log"*. A named refusal
//! keeps MISS = FAULT true one layer down.
//!
//! ★ **What R13 does and does not prove.** It proves a channel exists in hardware: RM
//! assigns it a chid out of the GPU's channel RAM and reports a work-submit token we
//! neither compute nor can predict, and two channels get two different ones. It proves
//! nothing whatsoever about *submission* — the ring and USERD are allocated and GPU-mapped
//! but not CPU-mapped, so nothing here can put a byte in front of the engine. That is the
//! next rung, and the separation is deliberate: a token from a channel that has never been
//! written to is a much cleaner fact than a token from one that has.

use kayfabe_abi::bringup::{
    NV_ESC_CHECK_VERSION_STR, NV_ESC_REGISTER_FD, NV_ESC_RM_ALLOC_MEMORY, NV_IOCTL_MAGIC,
    NV01_MEMORY_SYSTEM, NV01_MEMORY_VIRTUAL, NV20_SUBDEVICE_0, NVOS02_FLAGS_LOCATION_PCI,
    NVOS02_FLAGS_MAPPING_NO_MAP, NVOS02_FLAGS_PHYSICALITY_NONCONTIGUOUS,
    NVOS46_FLAGS_DMA_OFFSET_FIXED_TRUE, Nv2080AllocParameters, NvMemoryVirtualAllocationParams,
    NvVaspaceAllocationParameters, Nvos02ParametersWithFd, RegisterFd,
};
use kayfabe_abi::generated::classes::{
    AMPERE_CHANNEL_GPFIFO_A, FERMI_VASPACE_A, KEPLER_CHANNEL_GROUP_A, NV01_DEVICE_0,
    NV01_ROOT_CLIENT, Nv0080AllocParameters, NvChannelGroupAllocationParameters,
};
use kayfabe_abi::generated::nvos::{
    NV_ESC_RM_ALLOC, NV_ESC_RM_CONTROL, NV_ESC_RM_FREE, NV_ESC_RM_MAP_MEMORY_DMA,
    NV_ESC_RM_UNMAP_MEMORY_DMA, Nvos00Parameters, Nvos21Parameters, Nvos46Parameters,
    Nvos47Parameters, Nvos54Parameters,
};
use kayfabe_abi::submit::{
    ATTR_CONTIGUOUS_VIDMEM, BIND_PARAMS_SIZE, ChannelAllocParams, ENGINE_TYPE_GRAPHICS,
    GpfifoScheduleParams, NV01_MEMORY_LOCAL_USER, NVA06C_CTRL_CMD_BIND,
    NVA06C_CTRL_CMD_GPFIFO_SCHEDULE, NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN,
    NvMemoryAllocationParams, WORK_SUBMIT_TOKEN_PARAMS_SIZE, engine_type_copy,
};
use kayfabe_arch::ids::{ClassId, ControlCmd, EngineKind, GpuId, GpuVa};
use kayfabe_isolate::{CeSubCopy, HostHandle, IsolateId, RmBackend, RmError};
use kayfabe_linux_raw::{CharDevice, DevDir, Indirect, RawError, ioctl};
use kayfabe_util::leafwitness;
use kayfabe_vmm::SurfaceHandle;
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::{Arc, Mutex};

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
    client: u32,
    device: u32,
    subdevice: u32,
    /// The driver version string, as the frontend reported it. Diagnostic and ABI-profile
    /// input; never a gate (see the module docs).
    version: String,
    objects: Mutex<Objects>,
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
    pub fn open(dev: &DevDir, gpu: GpuId) -> Result<Self, BringUpError> {
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

        // R2 — the version string. `cmd = '2'` is the query-non-strict form; `cmd = 0` is
        // STRICT and deliberately returns EINVAL after filling the string in, which the
        // open driver enforces (`C: src/qemu/virtio_nvgpu.c:1157-1170`).
        let version = read_version(&ctl).unwrap_or_default();

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

        let conn = RmConnection {
            ctl,
            gpu: gpu_node,
            client: 0,
            device: 0,
            subdevice: 0,
            version,
            objects: Mutex::new(Objects {
                next: FIRST_HANDLE,
                parents: BTreeMap::new(),
                companions: BTreeMap::new(),
                channels: BTreeMap::new(),
            }),
        };

        // R4 — the root client. RM writes back the handle it assigned.
        let client = rung(
            "R4 NV01_ROOT_CLIENT",
            conn.raw_alloc(0, 0, REQUESTED_CLIENT_HANDLE, NV01_ROOT_CLIENT, &mut []),
        )?;

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
            conn.raw_alloc(client, client, FIRST_HANDLE, NV01_DEVICE_0, &mut dev_params),
        )?;

        // R6 — the subdevice.
        let mut sub_params = [0u8; Nv2080AllocParameters::SIZE];
        rung(
            "R6 NV2080 encode",
            Nv2080AllocParameters { sub_device_id: 0 }.encode_into(&mut sub_params),
        )?;
        let subdevice = rung(
            "R6 NV20_SUBDEVICE_0",
            conn.raw_alloc(
                client,
                device,
                FIRST_HANDLE + 1,
                NV20_SUBDEVICE_0,
                &mut sub_params,
            ),
        )?;

        {
            let mut o = conn.objects.lock().expect("objects");
            o.next = FIRST_HANDLE + 2;
            o.parents.insert(device, client);
            o.parents.insert(subdevice, device);
        }
        Ok(RmConnection {
            client,
            device,
            subdevice,
            ..conn
        })
    }

    /// The driver version string the frontend reported, if it answered.
    #[must_use]
    pub fn driver_version(&self) -> &str {
        &self.version
    }

    /// The client handle RM assigned.
    #[must_use]
    pub fn client(&self) -> u32 {
        self.client
    }

    /// The subdevice handle — the parent of most per-GPU controls.
    #[must_use]
    pub fn subdevice(&self) -> u32 {
        self.subdevice
    }

    /// One `NV_ESC_RM_ALLOC`, returning the handle RM ended up assigning.
    fn raw_alloc(
        &self,
        root: u32,
        parent: u32,
        want: u32,
        class: u32,
        params: &mut [u8],
    ) -> Result<u32, RmError> {
        let mut arg = [0u8; Nvos21Parameters::SIZE];
        Nvos21Parameters {
            h_root: root,
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
            h_client: self.client,
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
            h_client: self.client,
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
            h_client: self.client,
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

    /// Allocate `len` bytes of **device-local** memory — the only kind a ring, a USERD
    /// block or a semaphore can be built from.
    ///
    /// Not [`RmBackend::alloc_sysmem`]: that verb asks for `MAPPING_NO_MAP`, which makes
    /// the object deliberately un-CPU-mappable. See
    /// `kayfabe_abi::submit::NV01_MEMORY_LOCAL_USER`.
    fn alloc_device_local(&self, len: u64) -> Result<u32, RmError> {
        let mut params = [0u8; NvMemoryAllocationParams::SIZE];
        NvMemoryAllocationParams {
            owner: self.client,
            kind: 0,
            attr: ATTR_CONTIGUOUS_VIDMEM,
            size: len,
            alignment: len,
        }
        .encode_into(&mut params)
        .map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?;
        let want = self.mint();
        let h = self.raw_alloc(
            self.client,
            self.device,
            want,
            NV01_MEMORY_LOCAL_USER,
            &mut params,
        )?;
        self.remember(h, self.device);
        Ok(h)
    }

    fn forget(&self, child: u32) {
        let _leaf = leafwitness::Held::enter();
        self.objects.lock().expect("objects").parents.remove(&child);
    }
}

/// The runlist an [`EngineKind`] channel belongs on, as an `NV2080_ENGINE_TYPE_*`.
///
/// ★★ **This function is the seam audit's GR-1**, and the reason the port makes `engine`
/// an argument of `alloc_channel` rather than something the adapter guesses. There is
/// exactly ONE channel class per architecture — a graphics channel and a copy channel are
/// both `AMPERE_CHANNEL_GPFIFO_A` — so this value is the *only* thing that decides which
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
}

impl HostRmBackend {
    /// One worker's backend over `conn`.
    #[must_use]
    pub fn new(id: IsolateId, conn: Arc<RmConnection>) -> Self {
        HostRmBackend { id, conn }
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
            self.conn.client
        } else {
            self.narrow(parent)?
        };
        let want = self.conn.mint();
        let mut params = params.to_vec();
        let h = self
            .conn
            .raw_alloc(self.conn.client, parent_raw, want, class.0, &mut params)?;
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
        let space = self.conn.raw_alloc(
            self.conn.client,
            self.conn.device,
            want,
            FERMI_VASPACE_A,
            &mut params,
        )?;
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
        match self.conn.raw_alloc(
            self.conn.client,
            self.conn.device,
            want,
            NV01_MEMORY_VIRTUAL,
            &mut range,
        ) {
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
            h_root: self.conn.client,
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
    ///   chan   = AMPERE_CHANNEL_GPFIFO_A  parent = tsg,     hVASpace = 0 (inherits)
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
        let h = self
            .conn
            .raw_alloc(self.conn.client, parent, want, class.0, &mut params)?;
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
    fn ring_doorbell(&mut self, _host_token: u64) -> Result<(), RmError> {
        // Not an ioctl at all: a doorbell is a store into a mapped BAR page. It cannot be
        // expressed on this rung, and expressing it as a no-op would make every submission
        // test pass while nothing ran.
        Err(RmError::Other(NOT_ON_THIS_RUNG))
    }

    /// ★ NOT ON THIS RUNG, and for two different reasons depending on the arm — which
    /// is why it is one refusal and not a half-built verb.
    ///
    /// [`kayfabe_isolate::CeExecutor::HostCe`] needs a GPFIFO ring, USERD in mapped
    /// memory and a work-submit token — the same machinery `ring_doorbell` is refused
    /// for on this rung. [`kayfabe_isolate::CeExecutor::Ours`] needs the isolate's
    /// mapping of the fabricated aperture, which is the `FbRead` production
    /// implementation deliberately left to the stage after this one
    /// (`eight_blockers_resolved.md` §12.3).
    ///
    /// Returning `Ok(())` for a copy that moved no byte is precisely the
    /// forged-completion failure `mode2_real_forward_not_fake` forbids, and it would be
    /// invisible: the guest's next read is the only thing that would ever notice.
    fn ce_copy(&mut self, _vas: HostHandle, _sub: CeSubCopy) -> Result<(), RmError> {
        Err(RmError::Other(NOT_ON_THIS_RUNG))
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
}

impl HostRmBackend {
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
        let tsg = match self.conn.raw_alloc(
            self.conn.client,
            self.conn.device,
            want,
            KEPLER_CHANNEL_GROUP_A,
            &mut tsg_params,
        ) {
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
            userd_offset_0: 0,
            engine_type,
        }
        .encode_into(&mut chan_params);
        if encoded.is_err() {
            unwind(self, &[ring, userd, tsg]);
            return Err(RmError::Other(NOT_ON_THIS_RUNG));
        }
        let want = self.conn.mint();
        let chan = match self.conn.raw_alloc(
            self.conn.client,
            tsg,
            want,
            AMPERE_CHANNEL_GPFIFO_A,
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
            h_root: self.conn.client,
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

    /// Every isolate mints from the same base — the property that makes two isolates'
    /// handles genuinely collide, which the mock had to be taught to imitate.
    #[test]
    fn the_first_handle_is_the_same_for_every_isolate() {
        assert_eq!(FIRST_HANDLE, 0xCAFE_0001);
        assert_ne!(FIRST_HANDLE, REQUESTED_CLIENT_HANDLE);
    }
}
