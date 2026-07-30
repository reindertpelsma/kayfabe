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
//! ## ★★ The five verbs that are NOT implemented, and why that is a refusal
//!
//! `alloc_channel`, `alloc_engine_object`, `schedule`, `ring_doorbell` and
//! `export_surface` return [`RmError::Other`] carrying [`NOT_ON_THIS_RUNG`]. Each needs
//! machinery this rung does not have — a GPFIFO ring and USERD in mapped memory, a channel
//! group and a context share, a work-submit token read back by a control, a doorbell
//! *mapping* rather than an ioctl, a PRIME export. Returning a plausible success would be
//! the exact failure `mode2_real_forward_not_fake` forbids: *"prove compute via HW
//! sema/util, never green-guest-log"*. A named refusal keeps MISS = FAULT true one layer
//! down.

use kayfabe_abi::bringup::{
    NV_ESC_CHECK_VERSION_STR, NV_ESC_REGISTER_FD, NV_ESC_RM_ALLOC_MEMORY, NV_IOCTL_MAGIC,
    NV01_MEMORY_SYSTEM, NV01_MEMORY_VIRTUAL, NV20_SUBDEVICE_0, NVOS02_FLAGS_LOCATION_PCI,
    NVOS02_FLAGS_MAPPING_NO_MAP, NVOS02_FLAGS_PHYSICALITY_NONCONTIGUOUS,
    NVOS46_FLAGS_DMA_OFFSET_FIXED_TRUE, Nv2080AllocParameters, NvMemoryVirtualAllocationParams,
    NvVaspaceAllocationParameters, Nvos02ParametersWithFd, RegisterFd,
};
use kayfabe_abi::generated::classes::Nv0080AllocParameters;
use kayfabe_abi::generated::classes::{FERMI_VASPACE_A, NV01_DEVICE_0, NV01_ROOT_CLIENT};
use kayfabe_abi::generated::nvos::{
    NV_ESC_RM_ALLOC, NV_ESC_RM_CONTROL, NV_ESC_RM_FREE, NV_ESC_RM_MAP_MEMORY_DMA,
    NV_ESC_RM_UNMAP_MEMORY_DMA, Nvos00Parameters, Nvos21Parameters, Nvos46Parameters,
    Nvos47Parameters, Nvos54Parameters,
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

    fn forget(&self, child: u32) {
        let _leaf = leafwitness::Held::enter();
        self.objects.lock().expect("objects").parents.remove(&child);
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

    fn alloc_channel(
        &mut self,
        _vas: HostHandle,
        _engine: EngineKind,
    ) -> Result<(HostHandle, u64), RmError> {
        // Needs a GPFIFO ring and USERD in mapped memory, a channel group, a context share,
        // and a control to read the work-submit token back. See the module docs: a
        // plausible success here is the failure `mode2_real_forward_not_fake` forbids.
        Err(RmError::Other(NOT_ON_THIS_RUNG))
    }

    fn alloc_engine_object(
        &mut self,
        _chan: HostHandle,
        _class: ClassId,
        _params: &[u8],
    ) -> Result<HostHandle, RmError> {
        Err(RmError::Other(NOT_ON_THIS_RUNG))
    }

    fn schedule(&mut self, _chan: HostHandle) -> Result<(), RmError> {
        Err(RmError::Other(NOT_ON_THIS_RUNG))
    }

    fn free(&mut self, obj: HostHandle) -> Result<(), RmError> {
        let raw = self.narrow(obj)?;
        // ★ The port's `free` carries no parent and RM needs one — see `Objects::parents`.
        // A handle we never minted is refused HERE, which is stricter than the host: RM
        // would have destroyed whatever that value names in this client.
        let parent = self.conn.parent_of(raw).ok_or(RmError::BadHandle(obj))?;
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

    fn control(
        &mut self,
        obj: HostHandle,
        cmd: ControlCmd,
        payload: &mut [u8],
    ) -> Result<(), RmError> {
        let raw = self.narrow(obj)?;
        let mut arg = [0u8; Nvos54Parameters::SIZE];
        Nvos54Parameters {
            h_client: self.conn.client,
            h_object: raw,
            cmd: cmd.0,
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
        self.conn
            .ctl
            .ioctl(req, &mut arg, &mut patches)
            .map_err(|e| ioctl_error(&e))?;
        let out = Nvos54Parameters::decode(&arg).map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?;
        status_check(out.status)
    }

    fn map_gpu_va(
        &mut self,
        vas: HostHandle,
        memory: HostHandle,
        len: u64,
        at: GpuVa,
    ) -> Result<u64, RmError> {
        // R9. The 64-byte `NVOS46` — the 580.65.06-and-later shape, which is the bench's
        // driver. A host older than that speaks the 56-byte one
        // (`kayfabe_abi::transcribed::Nvos46ParametersPre580`), and selecting between them
        // from the R2 version string is the follow-up this rung does not do.
        let h_dma = self.narrow(vas)?;
        let h_memory = self.narrow(memory)?;
        let mut arg = [0u8; Nvos46Parameters::SIZE];
        Nvos46Parameters {
            h_client: self.conn.client,
            h_device: self.conn.device,
            h_dma,
            h_memory,
            offset: 0,
            length: len,
            // ★★★ #102 — `DMA_OFFSET_FIXED_TRUE`. With this bit set `dmaOffset` is an
            // **[IN]** parameter: RM places the mapping at the address we name instead of
            // choosing one and reporting it back. Without it, `flags: 0, dma_offset: 0`
            // asked the driver to pick — and a forwarded pushbuffer naming guest VAs then
            // resolves against nothing (`C: nvkvm_gpu_emul.c:7663-7692`, *"the irreducible
            // primitive the whole data plane rests on"*).
            flags: NVOS46_FLAGS_DMA_OFFSET_FIXED_TRUE,
            flags2: 0,
            kind_override: 0,
            dma_offset: at.0,
            status: 0,
        }
        .encode_into(&mut arg)
        .map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?;
        let req = ioctl::readwrite(NV_IOCTL_MAGIC, NV_ESC_RM_MAP_MEMORY_DMA as u8, arg.len())
            .map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?;
        self.conn
            .ctl
            .ioctl(req, &mut arg, &mut [])
            .map_err(|e| ioctl_error(&e))?;
        let out = Nvos46Parameters::decode(&arg).map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?;
        status_check(out.status)?;
        Ok(out.dma_offset)
    }

    fn unmap_gpu_va(&mut self, vas: HostHandle, gpu_va: u64) -> Result<(), RmError> {
        let h_dma = self.narrow(vas)?;
        let mut arg = [0u8; Nvos47Parameters::SIZE];
        Nvos47Parameters {
            h_client: self.conn.client,
            h_device: self.conn.device,
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
        self.conn
            .ctl
            .ioctl(req, &mut arg, &mut [])
            .map_err(|e| ioctl_error(&e))?;
        let out = Nvos47Parameters::decode(&arg).map_err(|_| RmError::Other(NOT_ON_THIS_RUNG))?;
        status_check(out.status)
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

    fn export_surface(&mut self, _memory: HostHandle) -> Result<SurfaceHandle, RmError> {
        Err(RmError::Other(NOT_ON_THIS_RUNG))
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

    /// Every isolate mints from the same base — the property that makes two isolates'
    /// handles genuinely collide, which the mock had to be taught to imitate.
    #[test]
    fn the_first_handle_is_the_same_for_every_isolate() {
        assert_eq!(FIRST_HANDLE, 0xCAFE_0001);
        assert_ne!(FIRST_HANDLE, REQUESTED_CLIENT_HANDLE);
    }
}
