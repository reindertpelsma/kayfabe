//! ★★★★★ **v3 `kf-host` — the host RM verbs, in ONE process.**
//!
//! One RM session (`/dev/nvidiactl` + `/dev/nvidia<N>`, our own root client, device, subdevice)
//! owned by the VMM. No isolates, no worker pool, no IPC plane (THE_ARCHITECTURE_v3.md §1, §8).
//! The verb bodies are copied from the old tree's `RmConnection` (`kayfabe-isolate-host/src/rm.rs`)
//! — the ioctl layer was always v3-fit; the isolate wrapper around it was not.
//!
//! ⊘ Verbs are **authored, never forwarded**: no signature takes a guest flags word.

#![allow(clippy::too_many_arguments)]

pub mod channel;
pub mod event;
pub use event::EventFd;
pub use channel::{Channel, MapBacking, RingSpec, ScatterError, VaSpace};

use kf_abi::bringup::{
    NV_ESC_CHECK_VERSION_STR, NV_ESC_REGISTER_FD, NV_ESC_RM_ALLOC_MEMORY, NV_IOCTL_MAGIC,
    NV01_MEMORY_SYSTEM_OS_DESCRIPTOR, NV20_SUBDEVICE_0,
    NVOS02_FLAGS_COHERENCY_CACHED, NVOS02_FLAGS_LOCATION_PCI, NVOS02_FLAGS_MAPPING_NO_MAP,
    NVOS02_FLAGS_PHYSICALITY_NONCONTIGUOUS, NVOS46_FLAGS_DMA_OFFSET_FIXED_TRUE,
    CardInfo, GpuIdInfoV2, NV_ESC_CARD_INFO, NV0000_CTRL_CMD_GPU_GET_ID_INFO_V2,
    Nv2080AllocParameters, Nvos02ParametersWithFd, RegisterFd,
};
use kf_abi::generated::classes::{
    NV01_DEVICE_0, NV01_ROOT_CLIENT, Nv0080AllocParameters,
};
use kf_abi::generated::nvos::{
    NV_ESC_RM_ALLOC, NV_ESC_RM_CONTROL, NV_ESC_RM_FREE,
    NV_ESC_RM_MAP_MEMORY_DMA, NV_ESC_RM_UNMAP_MEMORY_DMA, Nvos00Parameters, Nvos21Parameters,
    Nvos46Parameters, Nvos47Parameters, Nvos54Parameters,
};
use kf_abi::submit::*;
use kf_arch::ids::GpuId;
use kf_arch::{HostClasses, UsermodeClass};
use kf_linux_raw::{
    Backing, CachePolicy, CharDevice, DevDir, HostOffset, HostPageSize, Indirect, RawError,
    VolatileRegion, ioctl, release_fence,
};
use kf_util::leafwitness;
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;

/// A host RM refusal, by name. ⊘ No isolate-stamped handle variant: there is one session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RmError {
    /// `NV_ERR_INSUFFICIENT_PERMISSIONS`.
    InsufficientPermissions,
    /// `NV_ERR_NO_MEMORY` / `NV_ERR_INSUFFICIENT_RESOURCES`.
    NoMemory,
    /// The syscall was interrupted (`EINTR`) — cancellation, not failure.
    Interrupted,
    /// Any other RM status, or one of this crate's named codes below.
    Other(u32),
    /// A FIXED map landed somewhere other than the VA asked for (the mis-placed mapping is
    /// torn down before this returns).
    PlacementRefused {
        /// The VA the caller required.
        want: u64,
        /// The VA RM produced.
        got: u64,
    },
}

/// The key a CPU view is released by. ⊘ No client or device in it: an escape names THIS
/// session's own client, always, read at release time (the old tree's F11 invariant).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuViewRelease {
    /// The memory object the view was armed on.
    pub h_memory: u32,
    /// The linear address RM returned when it was armed.
    pub p_linear_address: u64,
}

/// An encoder refused a parameter block.
pub const ABI_ENCODE_FAILED: u32 = 0x4B63;
/// An ioctl number could not be built for the block's size.
pub const IOCTL_NUMBER_UNBUILDABLE: u32 = 0x4B64;
/// A reply block did not decode.
pub const ABI_DECODE_FAILED: u32 = 0x4B65;
/// A value did not fit the width the ABI carries.
pub const IMPOSSIBLE_CONVERSION: u32 = 0x4B66;
/// A verb this session cannot perform here.
pub const NOT_ON_THIS_RUNG: u32 = 0x4B46;
/// An offset or length outside the object it names.
pub const NOT_IN_THIS_OBJECT: u32 = 0x4B47;
/// The kernel refused the cache attribute asked for.
pub const MAPPING_ATTRIBUTE_REFUSED: u32 = 0x4B48;
/// A FIXED map at a VA that is already mapped.
pub const VA_ALREADY_MAPPED: u32 = 0x4B69;
/// The store reservation and which form RM granted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reservation {
    /// The `NV01_MEMORY_LOCAL_USER` handle.
    pub handle: u32,
    /// Contiguous and 1 GiB-aligned (`phys ≡ offset` at every page size) — else the fallback.
    pub contiguous_aligned: bool,
}

const FIRST_HANDLE: u32 = 0xCAFE_0001;
const REQUESTED_CLIENT_HANDLE: u32 = 0xCAFE_0000;

/// A bring-up step that failed, by name.
#[derive(Debug)]
pub struct BringUpError {
    /// The step.
    pub rung: &'static str,
    /// What it said.
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
        detail: format!("{e:?}"),
    })
}

fn status_check(status: u32) -> Result<(), RmError> {
    match status {
        0 => Ok(()),
        0x0000_001B => Err(RmError::InsufficientPermissions),
        0x0000_001A | 0x0000_0051 => Err(RmError::NoMemory),
        other => Err(RmError::Other(other)),
    }
}

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
        } if *errno == 4 => RmError::Interrupted,
        RawError::Syscall {
            errno: Some(errno), ..
        } => RmError::Other(0x8000_0000 | (*errno as u32 & 0xFFFF)),
        _ => RmError::Other(NOT_ON_THIS_RUNG),
    }
}

fn host_version_gate(reported: Option<&str>) -> Result<String, String> {
    kf_abi::host_driver::check(reported).map_err(|r| r.to_string())?;
    Ok(reported.unwrap_or_default().to_string())
}

fn read_version(ctl: &CharDevice) -> Option<String> {
    const SIZE: usize = 72;
    let mut arg = [0u8; SIZE];
    arg[0] = b'2'; // NV_RM_API_VERSION_CMD_OVERRIDE: query, never the strict form.
    let req = ioctl::readwrite(NV_IOCTL_MAGIC, NV_ESC_CHECK_VERSION_STR, SIZE).ok()?;
    ctl.ioctl(req, &mut arg, &mut []).ok()?;
    let s = &arg[8..];
    let end = s.iter().position(|&b| b == 0).unwrap_or(s.len());
    Some(String::from_utf8_lossy(&s[..end]).into_owned())
}

/// Our own root client — minted by us, never a guest's.
#[derive(Clone, Copy, PartialEq, Eq)]
struct OwnClient(u32);

impl core::fmt::Debug for OwnClient {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "OwnClient({:#010x})", self.0)
    }
}

impl OwnClient {
    fn allocate_root(ctl: &CharDevice) -> Result<Self, RmError> {
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
        .map_err(|_| RmError::Other(ABI_ENCODE_FAILED))?;
        let req = ioctl::readwrite(NV_IOCTL_MAGIC, NV_ESC_RM_ALLOC as u8, arg.len())
            .map_err(|_| RmError::Other(IOCTL_NUMBER_UNBUILDABLE))?;
        let mut patches: Vec<Indirect<'_>> = Vec::new();
        ctl.ioctl(req, &mut arg, &mut patches)
            .map_err(|e| ioctl_error(&e))?;
        let out = Nvos21Parameters::decode(&arg).map_err(|_| RmError::Other(ABI_DECODE_FAILED))?;
        status_check(out.status)?;
        Ok(Self(out.h_object_new))
    }

    fn raw(self) -> u32 {
        self.0
    }
}

/// Which device node a CPU mapping goes through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapNode {
    /// `/dev/nvidia<N>` — anything inside the GPU's own BARs.
    Gpu,
    /// `/dev/nvidiactl` — system memory.
    Ctl,
}

/// The access a CPU view is armed with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewAccess {
    /// Read and write.
    ReadWrite,
    /// Read only.
    ReadOnly,
}

impl ViewAccess {
    /// The device-node access this view needs.
    #[must_use]
    pub fn dev_access(self) -> kf_linux_raw::DevAccess {
        match self {
            ViewAccess::ReadWrite => kf_linux_raw::DevAccess::ReadWrite,
            ViewAccess::ReadOnly => kf_linux_raw::DevAccess::ReadOnly,
        }
    }

    /// `NVOS33` flags for this access.
    #[must_use]
    pub fn os33_flags(self) -> u32 {
        match self {
            ViewAccess::ReadWrite => 0,
            ViewAccess::ReadOnly => 1,
        }
    }
}

#[derive(Debug)]
struct UsermodeWindow {
    /// The usermode object's handle — freed with the session, read by nothing else.
    #[allow(dead_code)]
    object: u32,
    _node: CharDevice,
    region: VolatileRegion,
}

#[derive(Debug, Default)]
struct Objects {
    next: u32,
    parents: BTreeMap<u32, u32>,
}

/// `NV2080_CTRL_CMD_MC_GET_ARCH_INFO` — NON_PRIVILEGED (`ogkm-580: ctrl2080mc.h:61`).
const NV2080_CTRL_CMD_MC_GET_ARCH_INFO: u32 = 0x2080_1701;

/// The class profile a session holds only until the host has said what it is. Every method
/// refuses loudly: nothing may allocate an arch-varying class before the family is chosen.
#[derive(Debug)]
struct UnchosenClasses;

impl HostClasses for UnchosenClasses {
    fn name(&self) -> &'static str {
        "unchosen (before MC_GET_ARCH_INFO)"
    }
    fn gpfifo_channel(&self) -> kf_arch::ChannelClass {
        unreachable!("an arch-varying class was asked for before the family was chosen")
    }
    fn usermode(&self) -> UsermodeClass {
        unreachable!("an arch-varying class was asked for before the family was chosen")
    }
    fn ce_object(&self) -> kf_arch::CeObjectClass {
        unreachable!("an arch-varying class was asked for before the family was chosen")
    }
    fn compute_object(&self) -> Option<kf_arch::ComputeObjectClass> {
        unreachable!("an arch-varying class was asked for before the family was chosen")
    }
}

/// ★ The one host RM session.
#[derive(Debug)]
pub struct HostRm {
    ctl: CharDevice,
    gpu: CharDevice,
    dev: DevDir,
    gpu_index: u32,
    client: OwnClient,
    device: u32,
    subdevice: u32,
    version: String,
    objects: Mutex<Objects>,
    /// Subdevice notifiers this session has armed REPEAT — arming is per SUBDEVICE and legal only
    /// from `DISABLE` (`subdevice_ctrl_event_kernel.c:123-130`), so it is done once, here.
    armed: Mutex<std::collections::BTreeSet<u32>>,
    cpu_maps: std::sync::atomic::AtomicU64,
    usermode: Result<UsermodeWindow, RmError>,
    classes: Box<dyn HostClasses>,
    /// `MC_GET_ARCH_INFO` as the host answered it: `(architecture, implementation, revision)`.
    arch_info: (u32, u32, u32),
    /// ★ The host GPU this session is bound to, as the frontend's `CARD_INFO` states it for
    /// our minor: its PCI address and RM `gpuId`. The one identity every other per-GPU choice
    /// (the RM device instance, the CUDA device) is resolved from — never an ordinal.
    card: CardInfo,
    /// The RM device instance `GET_ID_INFO_V2` returned for [`HostRm::card`]'s `gpuId`.
    device_instance: u32,
}

impl HostRm {
    /// Open the one host RM session on GPU `gpu`: `nvidiactl` + `nvidia<N>`, version gate,
    /// `REGISTER_FD`, our own root client, device and subdevice, and the usermode window.
    ///
    /// # Errors
    /// The bring-up step that failed, by name.
    pub fn open(
        dev: &DevDir,
        gpu: GpuId,
        classes_for: &dyn Fn(u32, u32, &[u32]) -> Result<Box<dyn HostClasses>, String>,
    ) -> Result<Self, BringUpError> {
        // ⊘ v3: no pinned generation. The session opens with a placeholder class profile only
        // long enough to ask the HOST its architecture (`MC_GET_ARCH_INFO`, NON_PRIVILEGED);
        // `classes_for` (the family row, `kf-chip::Family`) then chooses. No class id is used
        // before the choice: device/subdevice classes are generation-invariant.
        let classes: Box<dyn HostClasses> = Box::new(UnchosenClasses);
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
        // handle it assigned. ★ Ordered this way so there is never an `HostRm`
        // carrying a placeholder client: `OwnClient` has no zero value and no constructor
        // but this one, which is the whole of F11's invariant (see `mod own_client`).
        let client = rung("R4 NV01_ROOT_CLIENT", OwnClient::allocate_root(&ctl))?;

        let conn = HostRm {
            ctl,
            gpu: gpu_node,
            dev: held,
            gpu_index: gpu.0,
            client,
            device: 0,
            subdevice: 0,
            version,
            classes,
            armed: Mutex::new(std::collections::BTreeSet::new()),
            arch_info: (0, 0, 0),
            card: CardInfo::default(),
            device_instance: 0,
            objects: Mutex::new(Objects {
                next: FIRST_HANDLE,
                parents: BTreeMap::new(),
            }),
            cpu_maps: std::sync::atomic::AtomicU64::new(0),
            // Filled in below, once there is a subdevice to parent it to.
            usermode: Err(RmError::Other(NOT_ON_THIS_RUNG)),
        };

        // ★★ R4b/R4c — WHICH RM DEVICE IS OUR MINOR. `deviceId` is RM's *device instance*,
        // assigned at attach, lowest free first; the minor is a Linux chardev number. They
        // diverge whenever a lower-numbered GPU is not attached (V3_MULTI_GPU_AUDIT §2 blocker
        // 2, measured: minor 1 got instance 0, `deviceId=1` → `Other(34)` + RM's
        // *"deviceInstance 0x1 does not exist"*). ⇒ minor → gpuId from the frontend's own
        // table (`CARD_INFO`), gpuId → instance from RM (`GET_ID_INFO_V2`, answered only for
        // an attached GPU — the per-GPU open + `REGISTER_FD` above attached it).
        let mut ci = vec![0u8; CardInfo::SIZE * CardInfo::MAX_ENTRIES];
        let req = rung(
            "R4b CARD_INFO request",
            ioctl::readwrite(NV_IOCTL_MAGIC, NV_ESC_CARD_INFO, ci.len()),
        )?;
        rung("R4b CARD_INFO", conn.ctl.ioctl(req, &mut ci, &mut []))?;
        let cards = rung("R4b CARD_INFO decode", CardInfo::decode_all(&ci))?;
        let card = cards.iter().copied().find(|c| c.minor == gpu.0).ok_or_else(|| BringUpError {
            rung: "R4b CARD_INFO minor",
            detail: format!(
                "no probed GPU has minor {} (the frontend lists minors {:?}) — refused by name",
                gpu.0,
                cards.iter().map(|c| c.minor).collect::<Vec<_>>()
            ),
        })?;
        let mut idinfo = [0u8; GpuIdInfoV2::SIZE];
        rung("R4c GET_ID_INFO_V2 encode", GpuIdInfoV2::encode_request(card.gpu_id, &mut idinfo))?;
        rung(
            "R4c GET_ID_INFO_V2",
            conn.raw_control(conn.client.raw(), NV0000_CTRL_CMD_GPU_GET_ID_INFO_V2, &mut idinfo),
        )?;
        let id = rung("R4c GET_ID_INFO_V2 decode", GpuIdInfoV2::decode(&idinfo))?;
        let conn = HostRm { card, device_instance: id.device_instance, ..conn };

        // R5 — the device. The parameters are NOT optional: without them RM does not
        // associate the device with a physical GPU and every later control answers
        // NOT_SUPPORTED (`C: tests/integration/test_ioctl_fwd.c:657-668`).
        let mut dev_params = [0u8; Nv0080AllocParameters::SIZE];
        rung(
            "R5 NV0080 encode",
            Nv0080AllocParameters {
                device_id: id.device_instance,
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
            Nv2080AllocParameters { sub_device_id: id.sub_device_instance }.encode_into(&mut sub_params),
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
        // `HostRm::usermode` for why it is a stored `Result` rather than a rung.
        let conn = HostRm {
            client,
            device,
            subdevice,
            ..conn
        };
        // ★ Choose the family from what the host reports.
        let mut arch = [0u8; 16];
        rung(
            "R6b MC_GET_ARCH_INFO",
            conn.raw_control(conn.subdevice, NV2080_CTRL_CMD_MC_GET_ARCH_INFO, &mut arch),
        )?;
        let w = |o: usize| u32::from_le_bytes([arch[o], arch[o + 1], arch[o + 2], arch[o + 3]]);
        let (architecture, implementation) = (w(0), w(4));
        let arch_info = (architecture, implementation, w(8));
        // ★ The host's OWN class list (`NV0080_CTRL_CMD_GPU_GET_CLASSLIST_V2`, NON_PRIVILEGED):
        // the chooser intersects it with the family's generated set, so the classes we allocate
        // are ones THIS die supports (GA100 `_A` vs GA10x `_B`; GB100 vs GB202) — derived, never
        // a hand-picked per-family profile.
        let mut list = vec![0u8; 4 + 4 * 200];
        rung(
            "R6c GPU_GET_CLASSLIST_V2",
            conn.raw_control(conn.device, 0x0080_0292, &mut list),
        )?;
        let n = (u32::from_le_bytes([list[0], list[1], list[2], list[3]]) as usize).min(200);
        let host_classes: Vec<u32> = (0..n)
            .map(|i| u32::from_le_bytes([list[4 + 4 * i], list[5 + 4 * i], list[6 + 4 * i], list[7 + 4 * i]]))
            .collect();
        let classes = classes_for(architecture, implementation, &host_classes).map_err(|detail| BringUpError {
            rung: "R6d family row",
            detail: format!(
                "architecture {architecture:#x} implementation {implementation:#x}: {detail} — \
                 refused by name, never a nearest guess"
            ),
        })?;
        let conn = HostRm { classes, arch_info, ..conn };
        let usermode = conn.open_usermode(conn.classes.usermode());
        Ok(HostRm { usermode, ..conn })
    }

    /// ★★★ Allocate the profile's usermode class under the **subdevice** and CPU-map its
    /// 64 KiB
    /// BAR0 window — the mapping whose existence *is* [`HostRm::doorbell`].
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
    /// 3. ★★ **[`CachePolicy::WriteBack`], not write-combining.** This is a BAR0
    ///    *register* range, so `nvidia_mmap_helper` takes the `IS_REG_OFFSET` branch and
    ///    calls `nv_encode_caching(…, NV_MEMORY_UNCACHED, NV_MEMORY_TYPE_REGISTERS)`
    ///    unconditionally (`ogkm-580: kernel-open/nvidia/nv-mmap.c:567-574`); the
    ///    write-combining branch two lines down is the *framebuffer* one. Nothing in this
    ///    process can check that claim — `Backing::DeviceFile`'s attainable policy is
    ///    `None` by design, so `require_attainable` cannot refuse a wrong requirement over
    ///    a device fd — which is precisely why the policy had to become a parameter of
    ///    [`HostRm::map_cpu`] before this call site existed.
    ///
    /// ★★★ **`class` is a parameter, and it is a [`UsermodeClass`] rather than a
    /// `ClassId`** (`#166`). The caller in [`HostRm::open`] must therefore *name
    /// the role* it is asking the profile for, and asking for the wrong one —
    /// `classes.gpfifo_channel()` — is a **type error**, not a silent mis-allocation
    /// that a Hopper host would have served. Before this signature, that exact swap was
    /// bitten and **nothing in the workspace went red**.
    /// ★ The usermode window as an opaque [`kf_linux_raw::HostSpan`] — the pages §53.1 disposition C aliases
    /// read-only into the guest's BAR0 (the live microsecond counter; writes still trap).
    ///
    /// # Errors
    /// The session has no usermode window (the error it was opened with).
    pub fn usermode_view(&self) -> Result<kf_linux_raw::HostSpan, RmError> {
        let w = self.usermode.as_ref().map_err(|e| *e)?;
        Ok(w.region.host_span())
    }

    /// ★ The host GPU this session is bound to (`CARD_INFO` for our minor): PCI address and
    /// RM `gpuId`. Select any other per-GPU resource (the CUDA device) by THIS, never by an
    /// ordinal or the minor.
    #[must_use]
    pub fn card(&self) -> CardInfo {
        self.card
    }

    /// The RM device instance our `NV01_DEVICE_0` was allocated with (resolved, not assumed).
    #[must_use]
    pub fn device_instance(&self) -> u32 {
        self.device_instance
    }

    fn open_usermode(&self, class: UsermodeClass) -> Result<UsermodeWindow, RmError> {
        let want = self.mint();
        let object = self.raw_alloc(self.subdevice, want, class.usermode_id().0, &mut [])?;
        self.remember(object, self.subdevice);
        let (node, region) = self.map_cpu(object, USERMODE_WINDOW_SIZE, CachePolicy::WriteBack)?;
        Ok(UsermodeWindow {
            object,
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
    /// ## ★★★★★ THE WITNESS (owner directive, 2026-08-12) — *"do you have proof this piece
    /// of write instruction is hit for unprivileged guest passthrough channel"*
    ///
    /// ⊘ **We did not.** Every *"the doorbell forwarded"* statement in this campaign rested
    /// on **reading call order in source**, never on an observation, while every doorbell
    /// line in `w268` reads `DOORBELL-REFUSED` and no positive line exists anywhere in the
    /// run. `w268` §1.3 then showed that refusal is **post-hoc** — but that too was a code
    /// reading. This makes the store itself say so.
    ///
    /// ⚠ **The `Err` arm is the load-bearing half.** `self.usermode.as_ref()` can return
    /// early and the store never happens; a silent early return is exactly the shape that
    /// reads as success. *"We did not reach the store"* is a printed line here, never an
    /// absence.
    ///
    /// ⚠ **Volume, bounded by construction.** `cup2` produces a few hundred doorbells (448 at
    /// `w202`, 8–16 of them `GrCompute`), so per-doorbell printing is not a spam risk at this
    /// workload — but this function must never become one if a *spinning* workload reaches
    /// it. So: the first [`DOORBELL_WITNESS_MAX`] stores print in full; after that only a
    /// periodic tally prints, and the tally **says how many it suppressed**. ⊘ Refusals are
    /// **never** suppressed: they are rare by hypothesis, and suppressing the rare event to
    /// save room for the common one inverts the purpose.
    pub fn doorbell(&self, token: u32) -> Result<(), RmError> {
        // ⊘ v3: this runs INLINE in a vCPU trap on the passthrough path — a fenced 32-bit store
        // and nothing else. No print, no lock, no count (the old tree printed up to 512 lines
        // from here: blocking I/O on a vCPU). Counting is kf-trap's census, off this path.
        let window = self.usermode.as_ref().map_err(|e| *e)?;
        release_fence();
        window
            .region
            .store_u32(HostOffset::new(USERMODE_NOTIFY_CHANNEL_PENDING), token)
            .map_err(|e| region_error(&e))
    }


    /// [`ptimer_sample`] over a [`VolatileRegion`], mapping both refusals onto [`RmError`].
    pub fn raw_alloc(
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
        .map_err(|_| RmError::Other(ABI_ENCODE_FAILED))?;
        let req = ioctl::readwrite(NV_IOCTL_MAGIC, NV_ESC_RM_ALLOC as u8, arg.len())
            .map_err(|_| RmError::Other(IOCTL_NUMBER_UNBUILDABLE))?;
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
        let out = Nvos21Parameters::decode(&arg).map_err(|_| RmError::Other(ABI_DECODE_FAILED))?;
        status_check(out.status)?;
        Ok(out.h_object_new)
    }

    /// [`HostRm::raw_alloc`] issued on `node` instead of the control file — an OS event
    /// object must be allocated on the file its events are bound to.
    ///
    /// # Errors
    /// The host's refusal.
    pub fn raw_alloc_via(
        &self,
        node: &CharDevice,
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
        .map_err(|_| RmError::Other(ABI_ENCODE_FAILED))?;
        let req = ioctl::readwrite(NV_IOCTL_MAGIC, NV_ESC_RM_ALLOC as u8, arg.len())
            .map_err(|_| RmError::Other(IOCTL_NUMBER_UNBUILDABLE))?;
        // `pAllocParms` at +16. An empty params block means a null pointer, which is what
        // `NV01_ROOT_CLIENT` wants — so the patch list is empty rather than pointing at a
        // zero-length buffer.
        let mut patches: Vec<Indirect<'_>> = Vec::new();
        if !params.is_empty() {
            patches.push(Indirect::new(16, params));
        }
        node
            .ioctl(req, &mut arg, &mut patches)
            .map_err(|e| ioctl_error(&e))?;
        let out = Nvos21Parameters::decode(&arg).map_err(|_| RmError::Other(ABI_DECODE_FAILED))?;
        status_check(out.status)?;
        Ok(out.h_object_new)
    }

    /// ★★★ [`Self::raw_alloc`] for a class whose parameter block itself carries a
    /// **userspace pointer** — one more level of indirection and nothing else.
    ///
    /// `NV_MEMORY_LIST_ALLOCATION_PARAMS` is the population: it is reached through
    /// `pAllocParms` and carries `NvP64 pageNumberList`, which RM `copy_from_user`s the page
    /// array out of. Both addresses must be live for the same syscall and neither may be
    /// produced outside `kayfabe-linux-raw`, which is exactly what
    /// [`Indirect::nested`](kf_linux_raw::Indirect::nested) is for.
    ///
    /// ⊘ Separate from [`Self::raw_alloc`] rather than a parameter on it: every other
    /// allocation in this file has a flat parameter block, and giving them all an
    /// `Option<(usize, &mut [u8])>` would put a nesting decision at thirty call sites that
    /// have none to make.
    ///
    /// # Errors
    /// Whatever RM refused, or [`RmError::Other`] if the nest does not fit its buffer.
    pub fn raw_alloc_nested(
        &self,
        parent: u32,
        want: u32,
        class: u32,
        params: &mut [u8],
        inner_at: usize,
        inner: &mut [u8],
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
        .map_err(|_| RmError::Other(ABI_ENCODE_FAILED))?;
        let req = ioctl::readwrite(NV_IOCTL_MAGIC, NV_ESC_RM_ALLOC as u8, arg.len())
            .map_err(|_| RmError::Other(IOCTL_NUMBER_UNBUILDABLE))?;
        let mut patches = [Indirect::nested(16, params, inner_at, inner)
            .map_err(|_| RmError::Other(IOCTL_NUMBER_UNBUILDABLE))?];
        self.ctl
            .ioctl(req, &mut arg, &mut patches)
            .map_err(|e| ioctl_error(&e))?;
        let out = Nvos21Parameters::decode(&arg).map_err(|_| RmError::Other(ABI_DECODE_FAILED))?;
        status_check(out.status)?;
        Ok(out.h_object_new)
    }


    /// Mint the next handle value. Taken and released around the ioctl, never held across
    /// one — the leaf-witness assert inside [`CharDevice::ioctl`] would fire if it were.
    pub fn mint(&self) -> u32 {
        let _leaf = leafwitness::Held::enter();
        let mut o = self.objects.lock().expect("objects");
        let h = o.next;
        o.next = o.next.wrapping_add(1);
        h
    }

    /// Record `child`'s parent, so [`HostRm::free`] can name it.
    pub fn remember(&self, child: u32, parent: u32) {
        let _leaf = leafwitness::Held::enter();
        self.objects
            .lock()
            .expect("objects")
            .parents
            .insert(child, parent);
    }

    /// `NV_ESC_RM_CONTROL` on one of our objects.
    ///
    /// # Errors
    /// The host's status, by name.
    pub fn raw_control(&self, object: u32, cmd: u32, payload: &mut [u8]) -> Result<(), RmError> {
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
        .map_err(|_| RmError::Other(ABI_ENCODE_FAILED))?;
        let req = ioctl::readwrite(NV_IOCTL_MAGIC, NV_ESC_RM_CONTROL as u8, arg.len())
            .map_err(|_| RmError::Other(IOCTL_NUMBER_UNBUILDABLE))?;
        let mut patches: Vec<Indirect<'_>> = Vec::new();
        if !payload.is_empty() {
            patches.push(Indirect::new(16, payload));
        }
        self.ctl
            .ioctl(req, &mut arg, &mut patches)
            .map_err(|e| ioctl_error(&e))?;
        let out = Nvos54Parameters::decode(&arg).map_err(|_| RmError::Other(ABI_DECODE_FAILED))?;
        status_check(out.status)
    }

    /// ★★★★★ **The ONE place an `NVOS46` is built** — every GPU map in v3 goes through
    /// [`HostRm::map`](crate::HostRm::map), which delegates here, so constraint 28's placement
    /// assertion below covers every fixed map.
    ///
    /// `NVOS46_PARAMETERS::offset` is the offset **inside `hMemory`**: it is what makes *"one
    /// reserved object, many guest ranges"* expressible — the store maps `[offset, offset+len)`
    /// at the guest's own VA, once per coalesced run.
    ///
    /// `is_shared_slice` (from [`crate::MapBacking`]) is stated by the caller, never inferred from
    /// `offset` — the store's first page is a slice at offset 0.
    ///
    /// # Errors
    /// The host's refusal, or [`RmError::PlacementRefused`] when RM placed the mapping
    /// somewhere other than `at`.
    pub(crate) fn raw_map_dma_slice(
        &self,
        h_dma: u32,
        h_memory: u32,
        offset: u64,
        len: u64,
        at: Option<u64>,
        extra: u32,
        is_shared_slice: bool,
        kind: u32,
    ) -> Result<u64, RmError> {
        let mut arg = [0u8; Nvos46Parameters::SIZE];
        // ★★★★★ **CONSTRAINT 28, HALF ONE — THE PAGE-SIZE FLAG MATCHES THE REQUEST.**
        // `[measured w744]` `DMA_OFFSET_FIXED_TRUE` alone is **not** address identity: RM
        // still picks a page size, and a big page cannot start at a 4 KiB boundary, so RM
        // aligns the request DOWN and answers `NV_OK`. See
        // [`kf_abi::bringup::nvos46_page_size_flag`] for the measurement and for why
        // the predicate is read off `(at, len)`. ⊘ OR-ed rather than assigned: a caller that
        // named the bit itself (`map_local_at_with_flags`) keeps naming it, and the two can
        // only agree.
        let page_size = match at {
            // ★★★★★ w755 — a store slice is a slice of an object whose base we do not know,
            // so its page size is settled by congruence, not by alignment. See
            // [`kf_abi::bringup::nvos46_page_size_flag_for_store_slice`].
            Some(_) if is_shared_slice => {
                // ⊘ The argument is ignored by design (w755e: FIXED is honoured only under the
                // 4 KiB pin, contiguous or not); `false` states the conservative case honestly.
                kf_abi::bringup::nvos46_page_size_flag_for_store_slice(false)
            }
            Some(a) => kf_abi::bringup::nvos46_page_size_flag(a, offset, len),
            None => 0,
        };
        Nvos46Parameters {
            h_client: self.client.raw(),
            h_device: self.device,
            h_dma,
            h_memory,
            offset,
            length: len,
            flags: extra
                | page_size
                | if at.is_some() {
                    NVOS46_FLAGS_DMA_OFFSET_FIXED_TRUE
                } else {
                    0
                },
            flags2: 0,
            // ★ v3-gfx: applied only with `NVOS46_FLAGS_PAGE_KIND_OVERRIDE_YES` in `extra`.
            kind_override: kind,
            dma_offset: at.unwrap_or(0),
            status: 0,
        }
        .encode_into(&mut arg)
        .map_err(|_| RmError::Other(ABI_ENCODE_FAILED))?;
        let req = ioctl::readwrite(NV_IOCTL_MAGIC, NV_ESC_RM_MAP_MEMORY_DMA as u8, arg.len())
            .map_err(|_| RmError::Other(IOCTL_NUMBER_UNBUILDABLE))?;
        self.ctl
            .ioctl(req, &mut arg, &mut [])
            .map_err(|e| ioctl_error(&e))?;
        let out = Nvos46Parameters::decode(&arg).map_err(|_| RmError::Other(ABI_DECODE_FAILED))?;
        // ★★★★★ w755d — see [`VA_ALREADY_MAPPED`]. `0x51` on a FIXED map is ADDRESS
        // OCCUPANCY, not capacity, and `status_check` would report it as `NoMemory`.
        if at.is_some() && out.status == 0x0000_0051 {
            return Err(RmError::Other(VA_ALREADY_MAPPED));
        }
        status_check(out.status)?;
        // ★★★★★ **CONSTRAINT 28, HALF TWO — EVERY FIXED MAP ASSERTS ITS OWN PLACEMENT.**
        //
        // ⊘ *"A `Result<u64, RmError>` returns `Ok` here and tells you nothing"* — the
        // constraint's own words. It is true of the **caller's** reading, not of this
        // function, which has `dmaOffset` beside the status and is the one place every
        // fixed map in the crate passes through. Asserting here makes *"RM relocated it"*
        // impossible to observe as a success at ANY call site, including ones written
        // later, rather than at the three that happen to compare today.
        //
        // ⚠ The mis-placed mapping is TORN DOWN before the refusal returns. Leaving it
        // would be a live mapping at a VA nobody will ever name again — the same leak the
        // relocation itself is, with a refusal on top of it.
        if let Some(want) = at
            && out.dma_offset != want
        {
            let _ = self.raw_unmap_dma(h_dma, out.dma_offset);
            return Err(RmError::PlacementRefused {
                want,
                got: out.dma_offset,
            });
        }
        Ok(out.dma_offset)
    }

    /// One `NV_ESC_RM_UNMAP_MEMORY_DMA`, undoing a [`HostRm::map`](crate::HostRm::map).
    pub fn raw_unmap_dma(&self, h_dma: u32, gpu_va: u64) -> Result<(), RmError> {
        self.raw_unmap_dma_flags(h_dma, gpu_va, 0)
    }

    /// `NV_ESC_RM_UNMAP_MEMORY_DMA` with explicit `NVOS47` flags — the deferred-TLB unmap
    /// the batched reconcile needs (one [`HostRm::invalidate_tlb`] after the batch).
    ///
    /// # Errors
    /// The host's status.
    pub fn raw_unmap_dma_flags(
        &self,
        h_dma: u32,
        gpu_va: u64,
        flags: u32,
    ) -> Result<(), RmError> {
        self.raw_unmap_dma_range(h_dma, gpu_va, 0, flags)
    }

    /// ★★★ `NV_ESC_RM_UNMAP_MEMORY_DMA` over a VA RANGE (`V3_BATCHED_MAP.md` §4): `size != 0`
    /// unmaps **every** mapping in `h_dma` that intersects `[gpu_va, gpu_va+size)`, splitting one
    /// that straddles an edge (`ogkm-580 rs_server.c:2365-2427` `serverInterUnmapInternal`;
    /// `virtual_mem.c:1681-1790`, `virtmemIsPartialUnmapSupported` = `NV_TRUE`). A range that
    /// intersects nothing is `NV_OK` — the post-condition "nothing of ours is mapped there" holds.
    /// `size == 0` is the legacy whole-mapping unmap keyed by the EXACT start.
    ///
    /// # Errors
    /// The host's status.
    pub fn raw_unmap_dma_range(
        &self,
        h_dma: u32,
        gpu_va: u64,
        size: u64,
        flags: u32,
    ) -> Result<(), RmError> {
        let mut arg = [0u8; Nvos47Parameters::SIZE];
        Nvos47Parameters {
            h_client: self.client.raw(),
            h_device: self.device,
            h_dma,
            h_memory: 0,
            flags,
            dma_offset: gpu_va,
            size,
            status: 0,
        }
        .encode_into(&mut arg)
        .map_err(|_| RmError::Other(ABI_ENCODE_FAILED))?;
        let req = ioctl::readwrite(NV_IOCTL_MAGIC, NV_ESC_RM_UNMAP_MEMORY_DMA as u8, arg.len())
            .map_err(|_| RmError::Other(IOCTL_NUMBER_UNBUILDABLE))?;
        self.ctl
            .ioctl(req, &mut arg, &mut [])
            .map_err(|e| ioctl_error(&e))?;
        let out = Nvos47Parameters::decode(&arg).map_err(|_| RmError::Other(ABI_DECODE_FAILED))?;
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
    ///    ⊘⊘ **THIS LINE USED TO READ *"everything mapped here is device-local, so it is
    ///    always the per-GPU node"*, AND THAT SENTENCE WAS THE BUG.** It was true when it
    ///    was written and stopped being true the moment `alloc_notifier_mem` allocated an
    ///    `NV01_MEMORY_SYSTEM` object; nothing in the type system noticed, because the node
    ///    was hardcoded three lines into the body. The kind is a **parameter** now — see
    ///    [`MapNode`] — precisely so a sysmem caller cannot inherit a device-local
    ///    assumption by default.
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
    pub fn map_cpu(
        &self,
        h_memory: u32,
        len: u64,
        cache: CachePolicy,
    ) -> Result<(CharDevice, VolatileRegion), RmError> {
        self.map_cpu_windowed_on(MapNode::Gpu, h_memory, len, len, cache)
    }

    /// [`Self::map_cpu`] for an object whose backing decides the node — see [`MapNode`].
    ///
    /// ⊘ A separate entry point rather than a default argument, because the whole defect
    /// this fixes was a *default* that was correct for every caller that existed and wrong
    /// for the first one that did not.
    pub fn map_cpu_on(
        &self,
        node: MapNode,
        h_memory: u32,
        len: u64,
        cache: CachePolicy,
    ) -> Result<(CharDevice, VolatileRegion), RmError> {
        self.map_cpu_windowed_on(node, h_memory, len, len, cache)
    }

    /// [`Self::map_cpu`] with the **ioctl length and the `mmap` length given separately**.
    ///
    /// ★★★ **They are not always the same number, and assuming they were cost `#128` a
    /// wrong finding.** The escape's length is bounded by the RM resource's own size:
    /// `gpuresMap_IMPL` asks `gpuresGetRegBaseOffsetAndSize` and refuses anything past it
    /// with `NV_ERR_INVALID_LIMIT` (`ogkm-580: src/nvidia/src/kernel/gpu/gpu_resource.c:126-143`).
    /// The `mmap` length, by contrast, must be a whole number of host pages — Linux's
    /// requirement, and independently ours in `Mapping::anywhere`. For an
    /// [`NV01_TIMER`](kf_abi::submit::NV01_TIMER) those two are `0x414` and `0x1000`,
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
    pub fn map_cpu_windowed_on(
        &self,
        which: MapNode,
        h_memory: u32,
        register_len: u64,
        mmap_len: u64,
        cache: CachePolicy,
    ) -> Result<(CharDevice, VolatileRegion), RmError> {
        // ★ Before anything can fail. See `HostRm::cpu_maps`: the measurement is of
        // attempts, so an early `?` must not be able to hide one.
        self.cpu_maps
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // ★ w393 — the registration is its own verb now, because the armed node is a thing
        // a caller may hand on without `mmap`ing it here. Everything below this line is the `mmap`.
        let (node, _cookie) =
            self.arm_cpu_view(which, h_memory, 0, register_len, ViewAccess::ReadWrite)?;

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

    /// ★★★★★ **w393 — the first half of [`Self::map_cpu_windowed_on`], as a verb: open a
    /// FRESH node and register an `mmap` context for `[offset, offset+len)` of `h_memory`
    /// against it, and hand the ARMED NODE back un-`mmap`ed.**
    ///
    /// Split out so a caller can perform the `mmap` itself (e.g. to install a guest memslot
    /// over it — the host-visible half of `DEVICE_LOCAL | HOST_VISIBLE`). Nothing on the driver's framebuffer `mmap` path names
    /// the calling process (`ogkm-580: kernel-open/nvidia/nv-mmap.c:505-641`, a reading),
    /// so the node's `struct file` carries the whole context wherever the descriptor goes.
    ///
    /// ⊘ A FRESH node either way — see fact 3 on [`Self::map_cpu_windowed_on`]. `self.ctl`
    /// is the connection's long-lived control descriptor and already carries RM state;
    /// registering a one-shot mmap context on it would work once and then answer
    /// `NV_ERR_STATE_IN_USE`, so the `Ctl` arm opens its own `nvidiactl` rather than
    /// borrowing that one.
    ///
    /// `offset` is the offset **within the object** (`NVOS33_PARAMETERS::offset`,
    /// `kf_abi::submit::Nvos33ParametersWithFd::offset`); every pre-existing caller
    /// passes `0` through [`Self::map_cpu_windowed_on`] and is unchanged.
    /// ⊘ **Returns the `pLinearAddress` cookie beside the node, and that is not cosmetic.**
    /// `NV_ESC_RM_UNMAP_MEMORY` identifies a mapping by that cookie, so a map path that
    /// status-checks the reply and drops it has made the view **unreleasable** — and
    /// `[measured w722]` an unreleased view keeps its BAR1 aperture for the life of the process,
    /// silently.
    pub fn arm_cpu_view(
        &self,
        which: MapNode,
        h_memory: u32,
        offset: u64,
        len: u64,
        access: ViewAccess,
    ) -> Result<(CharDevice, u64), RmError> {
        let node = self.open_view_node(which, access)?;
        self.arm_cpu_view_on(node, h_memory, offset, len, access)
    }

    /// Open the device node a CPU view's `mmap` context will live on — the half of
    /// [`HostRm::arm_cpu_view`] that can be done AHEAD of time (an `openat`, off any vCPU).
    ///
    /// # Errors
    /// The `openat` refusal.
    pub fn open_view_node(&self, which: MapNode, access: ViewAccess) -> Result<CharDevice, RmError> {
        match which {
            MapNode::Gpu => {
                let name = CString::new(format!("nvidia{}", self.gpu_index))
                    .map_err(|_| RmError::Other(IMPOSSIBLE_CONVERSION))?;
                CharDevice::openat_mode(&self.dev, &name, access.dev_access()).map_err(|e| ioctl_error(&e))
            }
            MapNode::Ctl => {
                CharDevice::openat_mode(&self.dev, c"nvidiactl", access.dev_access()).map_err(|e| ioctl_error(&e))
            }
        }
    }

    /// ★ The ONE ioctl of a CPU view — `NV_ESC_RM_MAP_MEMORY` of `h_memory[offset, offset+len)`
    /// onto a node from [`HostRm::open_view_node`]. The PRAMIN re-point's only RM call on a vCPU
    /// (owner ruling 2026-09-25: one map + one `mmap` per window move, nothing else).
    ///
    /// # Errors
    /// The ioctl or RM's status (the node is dropped with the error).
    pub fn arm_cpu_view_on(
        &self,
        node: CharDevice,
        h_memory: u32,
        offset: u64,
        len: u64,
        access: ViewAccess,
    ) -> Result<(CharDevice, u64), RmError> {
        let mut arg = [0u8; Nvos33ParametersWithFd::SIZE];
        Nvos33ParametersWithFd {
            h_client: self.client.raw(),
            h_device: self.device,
            h_memory,
            offset,
            length: len,
            p_linear_address: 0,
            status: 0,
            flags: access.os33_flags(),
            fd: node.fd_number(),
        }
        .encode_into(&mut arg)
        .map_err(|_| RmError::Other(ABI_ENCODE_FAILED))?;
        let req = ioctl::readwrite(NV_IOCTL_MAGIC, NV_ESC_RM_MAP_MEMORY, arg.len())
            .map_err(|_| RmError::Other(IOCTL_NUMBER_UNBUILDABLE))?;
        self.ctl
            .ioctl(req, &mut arg, &mut [])
            .map_err(|e| ioctl_error(&e))?;
        let out =
            Nvos33ParametersWithFd::decode(&arg).map_err(|_| RmError::Other(ABI_DECODE_FAILED))?;
        status_check(out.status)?;
        Ok((node, out.p_linear_address))
    }

    /// ★★★★★ **GIVE A CPU VIEW'S BAR1 APERTURE BACK** — `NV_ESC_RM_UNMAP_MEMORY` (`0x4F`).
    ///
    /// `[measured w722, GA106]` Over rounds mapping **fresh** offsets each time: with this ioctl,
    /// 224 MiB every round, 5/5. Without it — `munmap` + `close` alone — round 0 gets 224 MiB and
    /// rounds 1–4 get **zero**. ⇒ **Closing the node is not a release.**
    ///
    /// ⊘ On the **control** device, never a per-GPU node: `NV_CTL_DEVICE_ONLY(nv)`
    /// (`ogkm-610: escape.c:631`). And the plain SDK struct, **not** an fd wrapper
    /// (`escape.c:313`) — the map's asymmetry, which would be silent if got wrong.
    ///
    /// # Errors
    /// Whatever RM puts in `status`. ⚠ Which is **the only place a refusal appears**: `ioctl(2)`
    /// returns 0 and leaves `errno` untouched even when the unmap fails.
    pub fn release_cpu_view(&self, r: CpuViewRelease) -> Result<(), RmError> {
        let mut arg = [0u8; Nvos34Parameters::SIZE];
        Nvos34Parameters {
            // ⊘ F11: this session's OWN client and device, read here — never carried in from the
            // release key. See `CpuViewRelease`'s comment for the invariant that forbids it.
            h_client: self.client.raw(),
            h_device: self.device,
            h_memory: r.h_memory,
            p_linear_address: r.p_linear_address,
            status: 0,
            flags: 0,
        }
        .encode_into(&mut arg)
        .map_err(|_| RmError::Other(ABI_ENCODE_FAILED))?;
        let req = ioctl::readwrite(NV_IOCTL_MAGIC, NV_ESC_RM_UNMAP_MEMORY, arg.len())
            .map_err(|_| RmError::Other(IOCTL_NUMBER_UNBUILDABLE))?;
        self.ctl
            .ioctl(req, &mut arg, &mut [])
            .map_err(|e| ioctl_error(&e))?;
        let out = Nvos34Parameters::decode(&arg).map_err(|_| RmError::Other(ABI_DECODE_FAILED))?;
        status_check(out.status)
    }

    /// Allocate `len` bytes of **device-local** memory — the only kind a ring, a USERD
    /// block or a semaphore can be built from.
    ///
    /// Not a `MAPPING_NO_MAP` sysmem object, which makes
    /// the object deliberately un-CPU-mappable. See
    /// `kf_abi::submit::NV01_MEMORY_LOCAL_USER`.
    /// ★★★★★ **RESERVE THE GUEST'S WHOLE VIDEO MEMORY AS ONE OBJECT** —
    /// `docs/design/gpga_is_one_reserved_object.md`.
    ///
    /// Differs from [`Self::alloc_device_local`] in exactly the two ways a multi-gigabyte
    /// request needs, and both were wrong for it:
    ///
    /// 1. **Non-contiguous.** A contiguous multi-gigabyte request is a far stronger demand
    ///    and fails on merely *fragmented* free memory — refusing the boot for a reason that
    ///    is not capacity. Contiguity buys nothing: an object is addressed by OFFSET, so
    ///    slicing GPGA is arithmetic and the physical layout is RM's business.
    /// 2. **Page-aligned, not `len`-aligned.** `alloc_device_local` passes `alignment: len`,
    ///    which for an 8 GiB request demands an 8 GiB-aligned base. Nothing needs that.
    ///
    /// # Errors
    /// Whatever RM refused with. ⊘ A refusal here means **the VM does not start** — that is
    /// the design's central promise, and it is what makes an out-of-memory on the refresh
    /// path (where we cannot recover) impossible rather than unlikely.
    /// ★★★★★ **w755i — IMPORT AN OBJECT FROM AN fd INTO THIS CLIENT.**
    ///
    /// `NV0000_CTRL_CMD_OS_UNIX_IMPORT_OBJECT_FROM_FD` (`0x3d06`) on the client itself.
    ///
    /// ⊘ **The gate is narrow and worth knowing before reading a refusal**
    /// (`ogkm-580.159.04`): `cliresCtrlCmdOsUnixImportObjectFromFd_IMPL` (`os.c:2494`) calls
    /// `nv_get_file_private(fd, NV_TRUE, ..)`, which requires the fd's inode to be
    /// `MAJOR == NV_MAJOR_DEVICE_NUMBER` **and** the control-device minor
    /// (`kernel-open/nvidia/nv.c:4096-4106`) — i.e. **`/dev/nvidiactl`** — and then requires
    /// `nvfp->handles[0] != 0`, which only `NV_ESC_RM_EXPORT_OBJECT_TO_FD` sets.
    /// ⇒ this is **not** a general dma-buf importer, and a refusal usually means the fd was
    /// the wrong KIND rather than that the object was unacceptable.
    ///
    /// # Errors
    /// Whatever RM refused with.
    /// ★★★★★ **w755w — EXPORT AN RM OBJECT TO A CONTROL fd, the direction that works.**
    ///
    /// `[measured w755v]` RM refuses to import CUDA's fd — `nvfp->handles == NULL`
    /// (`os.c:2377`) — because it registers `handles[0]` only in **its own** export
    /// (`os.c:2291`). So the store is exported here and imported by CUDA.
    ///
    /// ⚠ **The caller supplies the fd and it must be a CONTROL fd.** `os.c` requires
    /// `pParams->fd != -1` and resolves it with `nv_get_file_private(fd, NV_TRUE)` — the
    /// `NV_TRUE` is *"require ctl fd"*. A `/dev/nvidia<N>` fd is refused there, and the
    /// refusal reads as a bad parameter rather than as the wrong kind of file.
    ///
    /// ⊘ **The params block is 24 bytes and `object` comes FIRST** — the opposite order from
    /// the import, whose `fd` leads. `[ctrl0000unix.h:150-154]`, read rather than remembered:
    /// w755v's 16-vs-20 transcription error on the sibling struct cost two wrong answers to
    /// the question this verb exists to settle.
    ///
    /// # Errors
    /// Whatever RM refused with.
    pub fn export_object_to_fd(&self, object: u32, fd: i32) -> Result<(), RmError> {
        // `{ NV0000_CTRL_OS_UNIX_EXPORT_OBJECT object; NvS32 fd; NvU32 flags; }` where the
        // object is `{ TYPE type; NvHandle hDevice, hParent, hObject; }`.
        const EXPORT_PARAMS_SIZE: usize = 24;
        // ⊘⊘⊘ **w755x — `_TYPE_RM` IS 1. `0` IS `_TYPE_NONE`.**
        // `[ctrl0000unix.h:103-106]` `{ NONE = 0, RM = 1 }`. This constant was `0` in both
        // the import and the export, so both sent `type = NONE` and both were refused
        // `0x3B NV_ERR_INVALID_PARAMETER` at `os.c`'s first check —
        // `pParams->object.type != ..._TYPE_RM`.
        // ⚠ **That refusal was read as an ANSWER**: w755v concluded from it that *"RM imports
        // only from an fd RM itself exported"*, citing the `handles[0] == 0` path. That path
        // returns the same status, so the citation looked confirmed and the run never reached
        // it. Third transcription error on one probe, third wrong verdict.
        const EXPORT_OBJECT_TYPE_RM: u32 = 1;
        // `EMPTY_FD_FALSE` — we hand RM a real control fd rather than asking it to mint one.
        const FLAGS_EMPTY_FD_FALSE: u32 = 0;
        let mut arg = [0u8; EXPORT_PARAMS_SIZE];
        arg[0..4].copy_from_slice(&EXPORT_OBJECT_TYPE_RM.to_le_bytes());
        arg[4..8].copy_from_slice(&self.device.to_le_bytes());
        arg[8..12].copy_from_slice(&self.device.to_le_bytes());
        arg[12..16].copy_from_slice(&object.to_le_bytes());
        arg[16..20].copy_from_slice(&fd.to_le_bytes());
        arg[20..24].copy_from_slice(&FLAGS_EMPTY_FD_FALSE.to_le_bytes());
        self.raw_control(self.client.raw(), 0x0000_3d05, &mut arg)
    }

    /// Import a memory object exported by [`HostRm::export_object_to_fd`].
    ///
    /// # Errors
    /// The host's refusal.
    pub fn import_object_from_fd(&self, fd: i32) -> Result<u32, RmError> {
        // `NV0000_CTRL_OS_UNIX_IMPORT_OBJECT_FROM_FD_PARAMS { NvS32 fd; NV0000_CTRL_OS_UNIX_EXPORT_OBJECT object; }`
        // and, from `ctrl0000unix.h:108-118` **read rather than remembered**:
        // `NV0000_CTRL_OS_UNIX_EXPORT_OBJECT { TYPE type; union { struct { NvHandle hDevice,
        //  hParent, hObject; } rmObject; } data; }`.
        //
        // ⊘⊘⊘ **w755v — THIS WAS TRANSCRIBED WRONG, AND THE WRONG ANSWER WAS ACTED ON.**
        // The previous version listed only `hParent, hObject` — **`hDevice` was missing** —
        // so the block was **16 bytes where RM expects 20**. `[measured w755v, RTX 3090]`
        // RM answered `0x1F NV_ERR_INVALID_ARGUMENT`, and the probe reported that as
        // *"the fd is an nvidiactl fd and RM still refused the import"* — i.e. as evidence
        // that **CUDA's export cannot be named by RM**, which is the fact the whole
        // store-ownership design turns on.
        //
        // ⚠ The comment that got it wrong also argued for hand-transcription: *"a generated
        // struct would imply a maintained ABI surface this is not."* The argument is fine and
        // the transcription still has to be checked against the header, because a params
        // block that is the wrong SIZE fails as `INVALID_ARGUMENT` — a status that reads like
        // a judgement about the argument's VALUE and is really about its LENGTH.
        const IMPORT_PARAMS_SIZE: usize = 20;
        // ⊘⊘⊘ **w755x — `_TYPE_RM` IS 1. `0` IS `_TYPE_NONE`.**
        // `[ctrl0000unix.h:103-106]` `{ NONE = 0, RM = 1 }`. This constant was `0` in both
        // the import and the export, so both sent `type = NONE` and both were refused
        // `0x3B NV_ERR_INVALID_PARAMETER` at `os.c`'s first check —
        // `pParams->object.type != ..._TYPE_RM`.
        // ⚠ **That refusal was read as an ANSWER**: w755v concluded from it that *"RM imports
        // only from an fd RM itself exported"*, citing the `handles[0] == 0` path. That path
        // returns the same status, so the citation looked confirmed and the run never reached
        // it. Third transcription error on one probe, third wrong verdict.
        const EXPORT_OBJECT_TYPE_RM: u32 = 1;
        let want = self.mint();
        let mut arg = [0u8; IMPORT_PARAMS_SIZE];
        arg[0..4].copy_from_slice(&fd.to_le_bytes());
        arg[4..8].copy_from_slice(&EXPORT_OBJECT_TYPE_RM.to_le_bytes());
        // ★ `hDevice`, `hParent`, `hObject` — the three the importer names. The parent of an
        // imported memory object is the device, and `hObject` is the handle we want it at.
        arg[8..12].copy_from_slice(&self.device.to_le_bytes());
        arg[12..16].copy_from_slice(&self.device.to_le_bytes());
        arg[16..20].copy_from_slice(&want.to_le_bytes());
        self.raw_control(self.client.raw(), 0x0000_3d06, &mut arg)?;
        // ⊘ `hObject` is IN/OUT — read back what RM actually named, never the request.
        let got = u32::from_le_bytes([arg[16], arg[17], arg[18], arg[19]]);
        self.remember(got, self.device);
        Ok(got)
    }

    /// ★ Reserve THE one device-local object that is the guest's whole framebuffer (GPGA =
    /// offset into it). Contiguous and 1 GiB-aligned first, the identity window's precondition;
    /// which one RM granted is RETURNED to the caller, never stored in a process global (review
    /// w826 #5).
    ///
    /// # Errors
    /// The host's refusal.
    pub fn reserve_gpga(&self, len: u64) -> Result<Reservation, RmError> {
        // ★★★★★ **w755c — TRY CONTIGUOUS AND 1 GiB-ALIGNED FIRST, FALL BACK, AND SAY WHICH.**
        //
        // > Owner, 2026-09-16: *"ensure the gpga rm object in the scratchpad va is aligned
        // > with 1GiB, that basically kills all these issues with memory offset harmony."*
        //
        // ★ The intent is exactly right and the mechanism needs one more term. RM honours a
        // FIXED map only when the VA and the PHYSICAL address are **congruent** modulo the
        // page size it picks. The guest already gives `at ≡ gpga (mod its page size)`; what
        // we insert is the store, so harmony reduces to
        // *"is the slice's physical address ≡ its offset?"*
        //
        // ⊘⊘ **On a NONCONTIGUOUS object it is not, and base alignment cannot make it so.**
        // `MapMemoryDma` sub-descriptors walk a page list; aligning the allocation's base
        // constrains page 0 and nothing else. Contiguity is what makes
        // `phys = base + offset` true, and only then does a 1 GiB-aligned base give
        // `phys ≡ offset` for every page size RM could choose.
        //
        // ⚠ And contiguity is exactly what [`ATTR_NONCONTIGUOUS_VIDMEM`]'s own docs refuse,
        // for a good reason: *"a multi-gigabyte contiguous request … can fail on a card whose
        // free memory is merely fragmented — which would refuse the boot for a reason that
        // has nothing to do with capacity."* That argument still stands.
        //
        // ⇒ **Both, in order.** The strong form is strictly better when the card allows it
        // (harmony at EVERY page size, so store slices need no 4 KiB pin and keep their TLB
        // reach); the weak form is what boots on a fragmented card. Trying one and falling
        // back to the other costs one refused allocation and decides by measurement rather
        // than by either of our predictions.
        //
        // ⊘ Which one happened is **recorded**, not inferred: the page-size decision below
        // depends on it, and a caller that guessed would be the second-source-of-truth shape
        // this file keeps paying for.
        const ONE_GIB: u64 = 1 << 30;
        let want = self.mint();
        let mut params = [0u8; NvMemoryAllocationParams::SIZE];
        NvMemoryAllocationParams {
            owner: self.client.raw(),
            kind: 0,
            attr: kf_abi::submit::ATTR_CONTIGUOUS_VIDMEM,
            size: len,
            alignment: ONE_GIB,
        }
        .encode_into(&mut params)
        .map_err(|_| RmError::Other(ABI_ENCODE_FAILED))?;
        if let Ok(h) = self.raw_alloc(self.device, want, NV01_MEMORY_LOCAL_USER, &mut params) {
            self.remember(h, self.device);
            return Ok(Reservation { handle: h, contiguous_aligned: true });
        }
        // ⊘ The fallback is not a degraded mode, it is the documented one. Only the page-size
        // freedom is lost.
        let want = self.mint();
        let mut params = [0u8; NvMemoryAllocationParams::SIZE];
        NvMemoryAllocationParams {
            owner: self.client.raw(),
            kind: 0,
            attr: kf_abi::submit::ATTR_NONCONTIGUOUS_VIDMEM,
            size: len,
            alignment: 4096,
        }
        .encode_into(&mut params)
        .map_err(|_| RmError::Other(ABI_ENCODE_FAILED))?;
        let h = self.raw_alloc(self.device, want, NV01_MEMORY_LOCAL_USER, &mut params)?;
        self.remember(h, self.device);
        Ok(Reservation { handle: h, contiguous_aligned: false })
    }

    /// A device-local memory object of `len` bytes.
    ///
    /// # Errors
    /// The host's refusal.
    pub fn alloc_device_local(&self, len: u64) -> Result<u32, RmError> {
        let mut params = [0u8; NvMemoryAllocationParams::SIZE];
        NvMemoryAllocationParams {
            owner: self.client.raw(),
            kind: 0,
            attr: ATTR_CONTIGUOUS_VIDMEM,
            size: len,
            alignment: len,
        }
        .encode_into(&mut params)
        .map_err(|_| RmError::Other(ABI_ENCODE_FAILED))?;
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
    /// VMM maps the guest's `memfd`, and this call turns a range of it into an RM memory
    /// object that [`HostRm::map`](crate::HostRm::map) can then place in a host VAS. Everything after it is machinery that already exists.
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
    ///    the per-GPU node. The C found the same thing the same way: *"ctl fd -> EINVAL"*
    ///    (`C: nvkvm_gpu_emul.c:7530-7532`).
    /// 3. **`REGISTER_FD` is a prerequisite** — without it RM answers `0x23
    ///    INVALID_CLIENT` (`C: nvkvm_gpu_emul.c:7503-7509`). ⊘ **Already done, and this is
    ///    not a port of it:** [`HostRm::open`]'s R3 binds the GPU node to the control
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
    pub fn alloc_os_descriptor(
        &self,
        region: &kf_linux_raw::MappedRegion,
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
        .map_err(|_| RmError::Other(ABI_ENCODE_FAILED))?;
        let req = ioctl::readwrite(NV_IOCTL_MAGIC, NV_ESC_RM_ALLOC_MEMORY, arg.len())
            .map_err(|_| RmError::Other(IOCTL_NUMBER_UNBUILDABLE))?;
        let mut describe =
            [
                Indirect::describing(Nvos02ParametersWithFd::P_MEMORY_OFFSET, region, offset, len)
                    .map_err(|e| region_error(&e))?,
            ];
        self.gpu
            .ioctl(req, &mut arg, &mut describe)
            .map_err(|e| ioctl_error(&e))?;
        let out =
            Nvos02ParametersWithFd::decode(&arg).map_err(|_| RmError::Other(ABI_DECODE_FAILED))?;
        status_check(out.status)?;
        self.remember(out.h_object_new, self.device);
        Ok(out.h_object_new)
    }


    /// Export `object` to a FRESH control-node fd (owned by the returned device) — how the store
    /// is handed to the CUDA walk context (`WalkKernel::import_store`).
    ///
    /// # Errors
    /// The open, or the host's refusal.
    pub fn export_to_new_fd(&self, object: u32) -> Result<CharDevice, RmError> {
        let ctl = CharDevice::openat(&self.dev, c"nvidiactl").map_err(|e| ioctl_error(&e))?;
        self.export_object_to_fd(object, ctl.fd_number())?;
        Ok(ctl)
    }

    /// This family's CE object class id.
    #[must_use]
    pub fn ce_class_id(&self) -> u32 {
        self.classes.ce_object().ce_object_id().0
    }

    /// The family's compute object class, if it has one.
    #[must_use]
    pub fn compute_class_id(&self) -> Option<u32> {
        self.classes.compute_object().map(|c| c.compute_object_id().0)
    }

    /// `MC_GET_ARCH_INFO` as the host answered it: `(architecture, implementation, revision)` — the
    /// facts the family and the presented `PMC_BOOT_*` are derived from.
    #[must_use]
    pub fn arch_info(&self) -> (u32, u32, u32) {
        self.arch_info
    }

    /// The host driver version string this session gated on (the driver-version axis).
    #[must_use]
    pub fn driver_version(&self) -> &str {
        &self.version
    }

    fn parent_of(&self, child: u32) -> Option<u32> {
        let _leaf = leafwitness::Held::enter();
        self.objects.lock().expect("objects").parents.get(&child).copied()
    }

    /// Drop `object` AND every descendant from the parent map — RM's `NV_ESC_RM_FREE` frees the
    /// subtree, so a child left here would name a handle RM has already released (review w826
    /// #6: CE and event objects outliving `free_channel`).
    fn forget(&self, object: u32) {
        let _leaf = leafwitness::Held::enter();
        let mut o = self.objects.lock().expect("objects");
        let mut doomed = vec![object];
        let mut i = 0;
        while i < doomed.len() {
            let p = doomed[i];
            doomed.extend(o.parents.iter().filter(|&(_, &par)| par == p).map(|(&c, _)| c));
            i += 1;
        }
        for h in doomed {
            o.parents.remove(&h);
        }
    }

    /// `NV_ESC_RM_FREE` of an object this session allocated.
    ///
    /// # Errors
    /// The host's refusal.
    pub fn free(&self, object: u32) -> Result<(), RmError> {
        let parent = self.parent_of(object).ok_or(RmError::Other(NOT_IN_THIS_OBJECT))?;
        let mut arg = [0u8; Nvos00Parameters::SIZE];
        Nvos00Parameters {
            h_root: self.client.raw(),
            h_object_parent: parent,
            h_object_old: object,
            status: 0,
        }
        .encode_into(&mut arg)
        .map_err(|_| RmError::Other(ABI_ENCODE_FAILED))?;
        let req = ioctl::readwrite(NV_IOCTL_MAGIC, NV_ESC_RM_FREE as u8, arg.len())
            .map_err(|_| RmError::Other(IOCTL_NUMBER_UNBUILDABLE))?;
        self.ctl
            .ioctl(req, &mut arg, &mut [])
            .map_err(|e| ioctl_error(&e))?;
        let out = Nvos00Parameters::decode(&arg).map_err(|_| RmError::Other(ABI_DECODE_FAILED))?;
        status_check(out.status)?;
        self.forget(object);
        Ok(())
    }
}
