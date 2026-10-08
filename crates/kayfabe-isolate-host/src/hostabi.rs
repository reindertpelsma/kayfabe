//! ★★★ The raw client's **host driver axis**, MEASURED — R2 on every driver of the v3 matrix.
//!
//! **STATUS: LIVE, 2026-10-08.** ⊘ Supersedes the pin `[580.65.06, 581)` that
//! `kayfabe_abi::host_driver` enforced at R2 (`rm.rs`, `host_version_gate`); that module and its
//! tests stay, and record the supersession in their own text.
//!
//! # Why the pin had to go
//!
//! Owner, 2026-10-08: *"the raw client should work on all driver versions kayfabe is also going to
//! support so they all can be tested."* The pin refused every host outside `[580.65.06, 581)` at
//! R2, so on a 595.91.07 host (and in a fast guest built from it, which runs the host's driver)
//! the bare-metal and fast 30-arm suites were 0/30 with one refusal. The reason the pin existed —
//! *"there is exactly one host driver available, so a per-version table would have no red"* — no
//! longer holds: v3 measured every struct per driver (`kf_abi::generated::matrix`, compiled from
//! ogkm at 30 tags 535.309.01 … 615.71.09, `docs/design/V3_DRIVER_MATRIX.md`), and `kf_abi::hostabi`
//! already carries kf-host's structs between the bench layout and the host's.
//!
//! # The rule — the same one kf-host follows, applied to the frozen client
//!
//! The client's encoders keep writing the **bench** layout (`kf_abi::versions::BENCH_DRIVER`,
//! 580.159.04 — the driver `kayfabe_abi::submit` was transcribed from). Every block it sends is
//! listed below with its measured struct, and crosses to the host in one of two ways:
//!
//! - **carried** at one of the client's choke points (`RmConnection::raw_alloc` /
//!   `raw_control` / the `NVOS46`/`NVOS47` sites, `BirthConn`, and the ladder's route-K `Esc`):
//!   identical layout ⇒ the bytes pass through untouched; a different layout ⇒ carried to the
//!   host's layout by field name (`kf_abi::hostabi::HostAbi::carry_out`) and the reply carried
//!   back; a field the host lacks but the request sets ⇒ refused by name;
//! - **verbatim** — the escape wrappers and UVM blocks, written at hand offsets — and then the
//!   host's layout must BE the bench's: [`gate`] refuses by name where it is not.
//! - the UVM blocks (also hand offsets, in the ladder's `uvm_raw`) are carried by name like the
//!   RM ones ([`ClientAbi::uvm_issue`]), with exactly two fields the HOST'S OWN API removed listed
//!   as droppable ([`UVM_API_REMOVED`]); every other drop is refused by name.
//!
//! ⊘ **Exact membership, no nearest neighbour, no override.** A host driver that is not a
//! measured tag is refused by name (`HostAbiError::Unmeasured`); unreadable and unparsable
//! versions are their own refusals (`kf_abi::host_driver::HostDriverRefusal`). There is no
//! environment variable or flag past any of them, for the reason `kayfabe_abi::host_driver` §3
//! gives: what is behind a switched-off refusal is silent memory corruption on the host's GPU.
//!
//! ⊘ **An unlisted block is not a pass-through.** A control or alloc class with a non-empty body
//! that has no row here crosses verbatim ONLY on a host inside the interval the encoders were
//! written for (`[580.65.06, 581)`, where it always did — byte-identical to the pre-2026-10-08
//! client); anywhere else it is refused by name, so a missed row surfaces as one named arm
//! failure instead of a plausible wrong offset.

use kf_abi::DriverVersion;
use kf_abi::generated::matrix as m;
use kf_abi::host_driver::{HostDriverRefusal, HostDriverVersion};
use kf_abi::hostabi::{Carry, HostAbi, HostAbiError};
use kf_abi::matrix::{Resolved, StructRuns, ValueRuns, transcode};
use std::fmt;

// =====================================================================================
// The inventory — every block the raw client sends
// =====================================================================================

/// The escape wrappers the client writes at hand offsets (`kayfabe_abi` encoders, fixed `SIZE`
/// buffers). [`gate`] requires each to have the bench's layout at the host. `[matrix
/// 2026-10-08]` one layout at every measured tag, so this never refuses a measured host today.
pub static VERBATIM_ESCAPES: &[&StructRuns] = &[
    &m::NVOS00_PARAMETERS,
    &m::NVOS02_PARAMETERS,
    &m::NVOS21_PARAMETERS,
    &m::NVOS33_PARAMETERS,
    &m::NVOS34_PARAMETERS,
    &m::NVOS54_PARAMETERS,
    &m::NVOS55_PARAMETERS,
    &m::NV_IOCTL_NVOS02_PARAMETERS_WITH_FD,
    &m::NV_IOCTL_NVOS33_PARAMETERS_WITH_FD,
    &m::NV_IOCTL_REGISTER_FD_T,
    &m::NV_IOCTL_CARD_INFO_T,
    &m::NV_IOCTL_RM_API_VERSION_T,
];

/// The one control issued on a bare `NVOS54` before a connection exists to carry it
/// (`rm::resolve_device_instance`): [`gate`] requires its layout and its id to be the bench's.
pub static VERBATIM_CONTROL: (&str, &StructRuns) = (
    "NV0000_CTRL_CMD_GPU_GET_ID_INFO_V2",
    &m::NV0000_CTRL_GPU_GET_ID_INFO_V2_PARAMS,
);

/// The escape wrappers that DO change across the matrix and are carried at every site that
/// builds one: `NVOS46` (56 bytes ≤575.64.05 — no `flags2`, no `kindOverride`) and `NVOS47`
/// (40 bytes at 535/545).
pub static CARRIED_ESCAPES: &[&StructRuns] = &[&m::NVOS46_PARAMETERS, &m::NVOS47_PARAMETERS];

/// How an alloc class is named in a row of [`ALLOC_PARAMS`].
#[derive(Debug, Clone, Copy)]
pub enum ClassName {
    /// Exactly this `class_ids:` name.
    Exact(&'static str),
    /// Any class whose name contains this (one per generation: `*_CHANNEL_GPFIFO_*`, …).
    Contains(&'static str),
}

impl ClassName {
    fn matches(self, name: &str) -> bool {
        match self {
            ClassName::Exact(n) => name == n,
            ClassName::Contains(n) => name.contains(n),
        }
    }
}

/// Every class the client allocates WITH a parameter block, and that block's matrix struct.
/// Classes allocated with no body (`NV01_ROOT_CLIENT`, `NV01_TIMER`, usermode, `NV2081`) carry
/// nothing and need no row.
pub static ALLOC_PARAMS: &[(ClassName, &StructRuns)] = &[
    (
        ClassName::Exact("NV01_DEVICE_0"),
        &m::NV0080_ALLOC_PARAMETERS,
    ),
    (
        ClassName::Exact("NV20_SUBDEVICE_0"),
        &m::NV2080_ALLOC_PARAMETERS,
    ),
    (
        ClassName::Exact("FERMI_VASPACE_A"),
        &m::NV_VASPACE_ALLOCATION_PARAMETERS,
    ),
    (
        ClassName::Exact("NV01_MEMORY_VIRTUAL"),
        &m::NV_MEMORY_VIRTUAL_ALLOCATION_PARAMS,
    ),
    (
        ClassName::Exact("NV50_MEMORY_VIRTUAL"),
        &m::NV_MEMORY_VIRTUAL_ALLOCATION_PARAMS,
    ),
    (
        ClassName::Exact("NV01_MEMORY_LOCAL_USER"),
        &m::NV_MEMORY_ALLOCATION_PARAMS,
    ),
    (
        ClassName::Exact("NV01_MEMORY_SYSTEM"),
        &m::NV_MEMORY_ALLOCATION_PARAMS,
    ),
    (
        ClassName::Exact("NV01_MEMORY_LIST_OBJECT"),
        &m::NV_MEMORY_LIST_ALLOCATION_PARAMS,
    ),
    (
        ClassName::Exact("NV01_MEMORY_LIST_SYSTEM"),
        &m::NV_MEMORY_LIST_ALLOCATION_PARAMS,
    ),
    (
        ClassName::Exact("NV01_MEMORY_LIST_FBMEM"),
        &m::NV_MEMORY_LIST_ALLOCATION_PARAMS,
    ),
    (
        ClassName::Contains("_CHANNEL_GROUP_"),
        &m::NV_CHANNEL_GROUP_ALLOCATION_PARAMETERS,
    ),
    (
        ClassName::Contains("_CHANNEL_GPFIFO_"),
        &m::NV_CHANNEL_ALLOC_PARAMS,
    ),
    (
        ClassName::Contains("_DMA_COPY_"),
        &m::NVB0B5_ALLOCATION_PARAMETERS,
    ),
    (
        ClassName::Contains("_COMPUTE_"),
        &m::NV_GR_ALLOCATION_PARAMETERS,
    ),
];

/// Every control the client issues, by its SDK name (`ctrl_cmds:<name>` in the matrix — the id is
/// read per tag from the driver matrix, never typed here), and its parameter struct.
pub static CONTROLS: &[(&str, &StructRuns)] = &[
    (
        "NV0000_CTRL_CMD_GPU_GET_ID_INFO_V2",
        &m::NV0000_CTRL_GPU_GET_ID_INFO_V2_PARAMS,
    ),
    (
        "NV0000_CTRL_CMD_CLIENT_SHARE_OBJECT",
        &m::NV0000_CTRL_CLIENT_SHARE_OBJECT_PARAMS,
    ),
    (
        "NV0000_CTRL_CMD_OS_UNIX_EXPORT_OBJECT_TO_FD",
        &m::NV0000_CTRL_OS_UNIX_EXPORT_OBJECT_TO_FD_PARAMS,
    ),
    (
        "NV0000_CTRL_CMD_OS_UNIX_IMPORT_OBJECT_FROM_FD",
        &m::NV0000_CTRL_OS_UNIX_IMPORT_OBJECT_FROM_FD_PARAMS,
    ),
    (
        "NV0041_CTRL_CMD_GET_SURFACE_PHYS_ATTR",
        &m::NV0041_CTRL_GET_SURFACE_PHYS_ATTR_PARAMS,
    ),
    (
        "NV0080_CTRL_CMD_DMA_GET_PTE_INFO",
        &m::NV0080_CTRL_DMA_GET_PTE_INFO_PARAMS,
    ),
    (
        "NV0080_CTRL_CMD_DMA_GET_PDE_INFO",
        &m::NV0080_CTRL_DMA_GET_PDE_INFO_PARAMS,
    ),
    (
        "NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY",
        &m::NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_PARAMS,
    ),
    (
        "NV0080_CTRL_CMD_GPU_GET_CLASSLIST_V2",
        &m::NV0080_CTRL_GPU_GET_CLASSLIST_V2_PARAMS,
    ),
    (
        "NV2080_CTRL_CMD_TIMER_GET_REGISTER_OFFSET",
        &m::NV2080_CTRL_TIMER_GET_REGISTER_OFFSET_PARAMS,
    ),
    (
        "NV2080_CTRL_CMD_GPU_GET_INFO_V2",
        &m::NV2080_CTRL_GPU_GET_INFO_V2_PARAMS,
    ),
    (
        "NV2080_CTRL_CMD_GPU_GET_NAME_STRING",
        &m::NV2080_CTRL_GPU_GET_NAME_STRING_PARAMS,
    ),
    (
        "NV2080_CTRL_CMD_GPU_GET_PIDS",
        &m::NV2080_CTRL_GPU_GET_PIDS_PARAMS,
    ),
    (
        "NV2080_CTRL_CMD_FIFO_GET_INFO",
        &m::NV2080_CTRL_FIFO_GET_INFO_PARAMS,
    ),
    (
        "NV2080_CTRL_CMD_FIFO_GET_ALLOCATED_CHANNELS",
        &m::NV2080_CTRL_FIFO_GET_ALLOCATED_CHANNELS_PARAMS,
    ),
    (
        "NV2080_CTRL_CMD_FIFO_GET_DEVICE_INFO_TABLE",
        &m::NV2080_CTRL_FIFO_GET_DEVICE_INFO_TABLE_PARAMS,
    ),
    (
        "NV2080_CTRL_CMD_MC_GET_ARCH_INFO",
        &m::NV2080_CTRL_MC_GET_ARCH_INFO_PARAMS,
    ),
    (
        "NV2080_CTRL_CMD_BUS_GET_INFO_V2",
        &m::NV2080_CTRL_BUS_GET_INFO_V2_PARAMS,
    ),
    (
        "NV2080_CTRL_CMD_BUS_GET_PCIE_SUPPORTED_GPU_ATOMICS",
        &m::NV2080_CTRL_CMD_BUS_GET_PCIE_SUPPORTED_GPU_ATOMICS_PARAMS,
    ),
    (
        "NV2080_CTRL_CMD_CE_GET_CE_PCE_MASK",
        &m::NV2080_CTRL_CE_GET_CE_PCE_MASK_PARAMS,
    ),
    (
        "NV2080_CTRL_CMD_CE_GET_ALL_CAPS",
        &m::NV2080_CTRL_CE_GET_ALL_CAPS_PARAMS,
    ),
    (
        "NV2080_CTRL_CMD_DMA_INVALIDATE_TLB",
        &m::NV2080_CTRL_DMA_INVALIDATE_TLB_PARAMS,
    ),
    ("NVA06C_CTRL_CMD_BIND", &m::NVA06C_CTRL_BIND_PARAMS),
    (
        "NVA06C_CTRL_CMD_GPFIFO_SCHEDULE",
        &m::NVA06C_CTRL_GPFIFO_SCHEDULE_PARAMS,
    ),
    ("NVA06F_CTRL_CMD_BIND", &m::NVA06F_CTRL_BIND_PARAMS),
    (
        "NVA06F_CTRL_CMD_GPFIFO_SCHEDULE",
        &m::NVA06F_CTRL_GPFIFO_SCHEDULE_PARAMS,
    ),
    (
        "NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN",
        &m::NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN_PARAMS,
    ),
    (
        "NV906F_CTRL_CMD_GET_MMU_FAULT_INFO",
        &m::NV906F_CTRL_GET_MMU_FAULT_INFO_PARAMS,
    ),
];

/// The UVM parameter blocks the ladder's `uvm_raw` writes at hand offsets, with the `uvm_ioctls:`
/// name each is sent under. [`ClientAbi::uvm_issue`] carries each to the host's matrix layout
/// and requires the host's matrix ioctl number to be the bench's.
pub static UVM_BLOCKS: &[(&str, &StructRuns)] = &[
    ("UVM_INITIALIZE", &m::UVM_INITIALIZE_PARAMS),
    ("UVM_MM_INITIALIZE", &m::UVM_MM_INITIALIZE_PARAMS),
    ("UVM_REGISTER_GPU", &m::UVM_REGISTER_GPU_PARAMS),
    ("UVM_UNREGISTER_GPU", &m::UVM_UNREGISTER_GPU_PARAMS),
    (
        "UVM_REGISTER_GPU_VASPACE",
        &m::UVM_REGISTER_GPU_VASPACE_PARAMS,
    ),
    (
        "UVM_CREATE_EXTERNAL_RANGE",
        &m::UVM_CREATE_EXTERNAL_RANGE_PARAMS,
    ),
    (
        "UVM_MAP_EXTERNAL_ALLOCATION",
        &m::UVM_MAP_EXTERNAL_ALLOCATION_PARAMS,
    ),
    ("UVM_FREE", &m::UVM_FREE_PARAMS),
    ("UVM_REGISTER_CHANNEL", &m::UVM_REGISTER_CHANNEL_PARAMS),
    ("UVM_UNREGISTER_CHANNEL", &m::UVM_UNREGISTER_CHANNEL_PARAMS),
];

/// ★ The fields the host driver's OWN UVM API stopped taking, which the bench encoders still
/// write: carrying them to a host that has no such field DROPS them, and only these two may be
/// dropped (every other dropped non-zero field is [`HostAbiError::Unexpressible`]).
///
/// `[matrix 2026-10-08]` both vanish at **590.48.01** (`UVM_FREE_PARAMS` 24 → 16 bytes,
/// `UVM_UNREGISTER_CHANNEL_PARAMS` 28 → 12). `[ogkm-595.84 source]` they are not inputs any more,
/// not moved ones: `uvm_api_free` frees by `params->base` alone (`kernel-open/nvidia-uvm/
/// uvm_va_range.c:744`), and `UVM_UNREGISTER_CHANNEL_PARAMS` names the channel by
/// `{hClient, hChannel}` only (`uvm_ioctl.h:347-352`).
pub static UVM_API_REMOVED: &[(&str, &str)] = &[
    ("UVM_FREE_PARAMS", "length"),
    ("UVM_UNREGISTER_CHANNEL_PARAMS", "gpuUuid"),
];

// =====================================================================================
// Refusals
// =====================================================================================

/// Why a block cannot cross to this host, by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The measured carry refused (unmeasured host, struct absent at the host, a field the host
    /// lacks carrying data, a body of the wrong size for a layout that must be carried, …).
    Abi(HostAbiError),
    /// A control or alloc class with a body and no row in this module, at a host outside the
    /// interval the encoders were written for.
    Unlisted {
        /// `"control"` or `"alloc class"`.
        what: &'static str,
        /// The command or class id.
        id: u32,
        /// The body's length.
        len: usize,
        /// The host driver.
        version: DriverVersion,
    },
    /// A listed control whose matrix id at the host is not the one the client sends.
    ControlIdMoved {
        /// The SDK name.
        name: &'static str,
        /// What the client sends (the bench's id).
        sent: u32,
        /// The host's matrix id, if the control exists there.
        host: Option<u32>,
        /// The host driver.
        version: DriverVersion,
    },
    /// A verbatim block (hand offsets) whose layout or ioctl number differs at this host.
    VerbatimDiffers {
        /// The C type (or the ioctl name).
        strukt: &'static str,
        /// The host driver.
        version: DriverVersion,
    },
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Abi(e) => write!(f, "{e}"),
            Self::Unlisted {
                what,
                id,
                len,
                version,
            } => write!(
                f,
                "raw client {what} {id:#010x} ({len}-byte body) has no row in \
                 kayfabe_isolate_host::hostabi, and host driver {version} is outside the interval \
                 the client's encoders were written for ([580.65.06, 581)); its layout there is \
                 unknown — refusing rather than sending the bench's bytes"
            ),
            Self::ControlIdMoved {
                name,
                sent,
                host,
                version,
            } => match host {
                Some(h) => write!(
                    f,
                    "{name} is {h:#010x} at host driver {version} (measured), not the {sent:#010x} \
                     the client sends; refusing rather than issuing another control"
                ),
                None => write!(
                    f,
                    "{name} does not exist at host driver {version} (measured); refusing"
                ),
            },
            Self::VerbatimDiffers { strukt, version } => write!(
                f,
                "{strukt} differs at host driver {version} from the bench layout the raw client \
                 writes at hand offsets (measured, kf_abi::generated::matrix); refusing rather \
                 than sending the bench's bytes"
            ),
        }
    }
}

impl std::error::Error for Refusal {}

impl From<HostAbiError> for Refusal {
    fn from(e: HostAbiError) -> Self {
        Self::Abi(e)
    }
}

// =====================================================================================
// The gate
// =====================================================================================

/// The host driver's ABI from the driver matrix, as the raw client uses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientAbi {
    abi: HostAbi,
}

/// ★★★ R2's decision: the host driver's version string → its measured ABI, or the named refusal.
///
/// Unreadable, unparsable and unmeasured are three different refusals, none of which defaults to
/// anything; a measured host whose escape wrappers differ from the bench's is refused by struct.
///
/// # Errors
/// The refusal's prose, ready to be R2's `BringUpError::detail`.
pub fn gate(reported: Option<&str>) -> Result<(String, ClientAbi), String> {
    let Some(r) = reported else {
        return Err(HostDriverRefusal::Unreadable.to_string());
    };
    let v = HostDriverVersion::parse(r).ok_or_else(|| {
        HostDriverRefusal::Unparsable {
            reported: r.to_string(),
        }
        .to_string()
    })?;
    let abi = HostAbi::for_host(v).map_err(|e| e.to_string())?;
    let client = ClientAbi { abi };
    for runs in VERBATIM_ESCAPES {
        client.verbatim(runs).map_err(|e| e.to_string())?;
    }
    for runs in CARRIED_ESCAPES {
        abi.carry(runs)
            .map_err(|e| format!("host driver {v}: {e}"))?;
    }
    // `resolve_device_instance` issues GPU_GET_ID_INFO_V2 on a raw `NVOS54` before any
    // connection exists to carry it, so its body and its id must be the bench's.
    let (name, runs) = VERBATIM_CONTROL;
    client.verbatim(runs).map_err(|e| e.to_string())?;
    let key = format!("ctrl_cmds:{name}");
    let host = value(&key, client.version());
    let bench = value(&key, kf_abi::versions::BENCH_DRIVER);
    if bench.is_none() || host != bench {
        return Err(Refusal::ControlIdMoved {
            name,
            sent: bench.unwrap_or(0),
            host,
            version: client.version(),
        }
        .to_string());
    }
    Ok((r.to_string(), client))
}

/// The driver matrix's value of `name` at `version`.
fn value(name: &str, version: DriverVersion) -> Option<u32> {
    m::ALL_VALUES
        .iter()
        .find(|r| r.name == name)
        .and_then(|r: &&ValueRuns| r.at_u32(version).ok().flatten())
}

impl ClientAbi {
    /// The ABI for a host driver, without R2's escape check — for tests and diagnostics.
    ///
    /// # Errors
    /// [`HostAbiError::Unmeasured`].
    pub fn for_host(host: HostDriverVersion) -> Result<Self, HostAbiError> {
        Ok(Self {
            abi: HostAbi::for_host(host)?,
        })
    }

    /// The host driver.
    #[must_use]
    pub fn version(&self) -> DriverVersion {
        self.abi.version()
    }

    /// The underlying matrix ABI.
    #[must_use]
    pub fn host(&self) -> HostAbi {
        self.abi
    }

    /// Inside `[580.65.06, 581)` — where an unlisted block keeps its pre-2026-10-08 contract.
    fn in_encoded_interval(&self) -> bool {
        self.abi.in_encoded_interval()
    }

    /// A block written at hand offsets: the host's layout must be the bench's.
    ///
    /// # Errors
    /// [`Refusal::VerbatimDiffers`] / [`Refusal::Abi`] (struct absent at the host).
    pub fn verbatim(&self, runs: &'static StructRuns) -> Result<(), Refusal> {
        match self.abi.carry(runs)? {
            Carry::Same { .. } => Ok(()),
            Carry::Carried { .. } => Err(Refusal::VerbatimDiffers {
                strukt: runs.name,
                version: self.version(),
            }),
        }
    }

    /// ★★ One UVM ioctl, `ioctl` being a [`UVM_BLOCKS`] name: the bench-layout `body` is carried
    /// to the host's matrix layout (only [`UVM_API_REMOVED`] fields may be dropped), `issue`
    /// runs over the host bytes, and the reply is carried back into `body`. Bytes past the
    /// struct (`UVM_INITIALIZE`'s `_IOC_SIZE` padding) pass through untouched.
    ///
    /// # Errors
    /// [`Refusal::VerbatimDiffers`] naming the ioctl when its matrix NUMBER differs at the host
    /// (or it is unlisted); [`Refusal::Abi`] for an absent struct, a refused drop, a wrong size.
    pub fn uvm_issue<R>(
        &self,
        ioctl: &'static str,
        body: &mut Vec<u8>,
        issue: impl FnOnce(&mut Vec<u8>) -> R,
    ) -> Result<R, Refusal> {
        let v = self.version();
        let Some((name, runs)) = UVM_BLOCKS.iter().find(|(n, _)| *n == ioctl) else {
            return Err(Refusal::VerbatimDiffers {
                strukt: ioctl,
                version: v,
            });
        };
        let key = format!("uvm_ioctls:{name}");
        let bench_no = value(&key, kf_abi::versions::BENCH_DRIVER);
        if bench_no.is_none() || bench_no != value(&key, v) {
            return Err(Refusal::VerbatimDiffers {
                strukt: name,
                version: v,
            });
        }
        let bench =
            Resolved::of(runs, kf_abi::versions::BENCH_DRIVER).map_err(HostAbiError::from)?;
        let host = Resolved::of(runs, v).map_err(HostAbiError::from)?;
        if bench.layout == host.layout {
            return Ok(issue(body));
        }
        if body.len() != bench.size() {
            return Err(HostAbiError::Size {
                strukt: runs.name,
                want: bench.size(),
                got: body.len(),
            }
            .into());
        }
        let (mut wire, dropped) =
            transcode(&bench, &host, body, &[]).map_err(|err| HostAbiError::Transcode {
                strukt: runs.name,
                outgoing: true,
                err,
            })?;
        let refused: Vec<&'static str> = dropped
            .into_iter()
            .filter(|f| !UVM_API_REMOVED.contains(&(runs.name, *f)))
            .collect();
        if !refused.is_empty() {
            return Err(HostAbiError::Unexpressible {
                strukt: runs.name,
                fields: refused,
                version: v,
            }
            .into());
        }
        let r = issue(&mut wire);
        let (back, _) =
            transcode(&host, &bench, &wire, &[]).map_err(|err| HostAbiError::Transcode {
                strukt: runs.name,
                outgoing: false,
                err,
            })?;
        // Fields only the bench layout has (the removed inputs) keep what the client wrote.
        for (path, f) in bench.layout.fields {
            if host.layout.field(path).is_none() {
                continue;
            }
            if let Some(r) = f.range() {
                body[r.clone()].copy_from_slice(&back[r]);
            }
        }
        Ok(r)
    }

    /// How an `NV_ESC_RM_ALLOC` body for `class` crosses.
    ///
    /// # Errors
    /// [`Refusal::Unlisted`] for a class with a body and no row, outside the encoded interval;
    /// [`Refusal::Abi`] when the struct does not exist at the host.
    pub fn alloc_crossing(&self, class: u32, len: usize) -> Result<Crossing, Refusal> {
        if len == 0 {
            return Ok(Crossing::Verbatim);
        }
        let v = self.version();
        let mut found: Option<&'static StructRuns> = None;
        for vr in m::ALL_VALUES {
            let Some(name) = vr.name.strip_prefix("class_ids:") else {
                continue;
            };
            if vr.at_u32(v).ok().flatten() != Some(class) {
                continue;
            }
            if let Some((_, runs)) = ALLOC_PARAMS.iter().find(|(c, _)| c.matches(name)) {
                match found {
                    // By NAME: a `const` referenced twice may be promoted to two addresses.
                    Some(prev) if prev.name != runs.name => {
                        // Two names for one id that map to different structs: not ours to pick.
                        found = None;
                        break;
                    }
                    _ => found = Some(runs),
                }
            }
        }
        match found {
            Some(runs) => self.crossing(runs),
            None if self.in_encoded_interval() => Ok(Crossing::Verbatim),
            None => Err(Refusal::Unlisted {
                what: "alloc class",
                id: class,
                len,
                version: v,
            }),
        }
    }

    /// How an `NV_ESC_RM_CONTROL` body for `cmd` crosses. `cmd` is the id the client sends — the
    /// bench's; the row is found by the bench's matrix id and the host's matrix id must match.
    ///
    /// # Errors
    /// [`Refusal::Unlisted`]; [`Refusal::ControlIdMoved`]; [`Refusal::Abi`].
    pub fn control_crossing(&self, cmd: u32, len: usize) -> Result<Crossing, Refusal> {
        if len == 0 {
            return Ok(Crossing::Verbatim);
        }
        let v = self.version();
        let bench = kf_abi::versions::BENCH_DRIVER;
        let row = CONTROLS
            .iter()
            .find(|(name, _)| value(&format!("ctrl_cmds:{name}"), bench) == Some(cmd));
        match row {
            Some((name, runs)) => {
                let host = value(&format!("ctrl_cmds:{name}"), v);
                if host != Some(cmd) {
                    return Err(Refusal::ControlIdMoved {
                        name,
                        sent: cmd,
                        host,
                        version: v,
                    });
                }
                self.crossing(runs)
            }
            None if self.in_encoded_interval() => Ok(Crossing::Verbatim),
            None => Err(Refusal::Unlisted {
                what: "control",
                id: cmd,
                len,
                version: v,
            }),
        }
    }

    /// How a listed struct crosses.
    ///
    /// # Errors
    /// [`Refusal::Abi`] when the struct does not exist at the host.
    pub fn crossing(&self, runs: &'static StructRuns) -> Result<Crossing, Refusal> {
        Ok(match self.abi.carry(runs)? {
            Carry::Same { .. } => Crossing::Verbatim,
            c @ Carry::Carried { .. } => Crossing::Carried {
                strukt: runs.name,
                carry: c,
            },
        })
    }

    /// The `NVOS46_PARAMETERS` crossing.
    ///
    /// # Errors
    /// As [`Self::crossing`].
    pub fn nvos46(&self) -> Result<Crossing, Refusal> {
        self.crossing(&m::NVOS46_PARAMETERS)
    }

    /// The `NVOS47_PARAMETERS` crossing.
    ///
    /// # Errors
    /// As [`Self::crossing`].
    pub fn nvos47(&self) -> Result<Crossing, Refusal> {
        self.crossing(&m::NVOS47_PARAMETERS)
    }
}

/// How one block crosses to the host.
#[derive(Debug, Clone, Copy)]
pub enum Crossing {
    /// The bytes go as they are.
    Verbatim,
    /// The host's layout differs: carry out, issue, carry back.
    Carried {
        /// The C type, for the refusal.
        strukt: &'static str,
        /// The resolved carry.
        carry: Carry,
    },
}

impl Crossing {
    /// The size the host expects, given the bench body's length.
    #[must_use]
    pub fn host_len(&self, bench_len: usize) -> usize {
        match self {
            Crossing::Verbatim => bench_len,
            Crossing::Carried { carry, .. } => carry.host_size(),
        }
    }

    /// ★ Run `issue` over the body in the HOST's layout, then carry what the host wrote back into
    /// `body` (bench layout). `issue` sees the host-layout bytes and returns whatever the ioctl
    /// returned; that value is returned as-is once the reply has been carried back.
    ///
    /// # Errors
    /// [`Refusal::Abi`] when the body cannot be carried out (a field the host lacks set, a body
    /// that is not the bench layout's size) or the reply cannot be carried back.
    pub fn issue<R>(
        &self,
        abi: &ClientAbi,
        body: &mut [u8],
        issue: impl FnOnce(&mut [u8]) -> R,
    ) -> Result<R, Refusal> {
        match self {
            Crossing::Verbatim => Ok(issue(body)),
            Crossing::Carried { strukt, carry } => {
                let mut host = abi.abi.carry_out(strukt, carry, body)?;
                let r = issue(&mut host);
                let back = abi.abi.carry_in(strukt, carry, &host)?;
                body.copy_from_slice(&back);
                Ok(r)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kf_abi::generated::matrix::MEASURED;
    use kf_abi::matrix::{Layout, Resolved};
    use kf_abi::versions::BENCH_DRIVER;

    fn host(v: &str) -> ClientAbi {
        ClientAbi::for_host(HostDriverVersion::parse(v).expect("parses")).expect("measured")
    }

    fn tag(v: DriverVersion) -> String {
        v.to_string()
    }

    /// Every struct the client sends, by the way it crosses.
    fn carried_rows() -> Vec<&'static StructRuns> {
        let mut v: Vec<&'static StructRuns> = CARRIED_ESCAPES.to_vec();
        v.extend(ALLOC_PARAMS.iter().map(|(_, r)| *r));
        v.extend(CONTROLS.iter().map(|(_, r)| *r));
        v
    }

    fn verbatim_rows() -> Vec<&'static StructRuns> {
        let mut v: Vec<&'static StructRuns> = VERBATIM_ESCAPES.to_vec();
        v.push(VERBATIM_CONTROL.1);
        // `uvm_raw::drive` issues these five on a raw `CharDevice::ioctl`, outside `call`.
        v.extend([
            &m::UVM_INITIALIZE_PARAMS,
            &m::UVM_MM_INITIALIZE_PARAMS,
            &m::UVM_REGISTER_GPU_PARAMS,
            &m::UVM_REGISTER_GPU_VASPACE_PARAMS,
            &m::UVM_UNREGISTER_GPU_PARAMS,
        ]);
        v
    }

    /// ★★★ (a) The owner's two drivers are accepted at R2 — **595.91.07** (the trusted host's)
    /// and every measured 580 tag from **580.65.06** up (the bench line) — and so is every other
    /// measured tag, because nothing R2 checks differs anywhere in the matrix.
    ///
    /// Literals for the named ones, never derived from the matrix under test.
    #[test]
    fn the_595_host_and_the_580_line_pass_r2() {
        for v in [
            "595.91.07",
            "580.65.06",
            "580.159.04",
            "580.178.04",
            "595.84",
            "610.43.02",
        ] {
            let (s, abi) = gate(Some(v)).unwrap_or_else(|e| panic!("{v} must pass R2: {e}"));
            assert_eq!(s, v);
            assert_eq!(
                abi.version().to_string(),
                v,
                "the version R2 resolved is the one read"
            );
        }
        for t in MEASURED {
            let v = tag(*t);
            assert!(gate(Some(&v)).is_ok(), "measured tag {v} must pass R2");
        }
    }

    /// ★★★ (b) A driver the matrix never measured is refused BY NAME at R2 — including one patch
    /// beside a measured tag, where a borrowed neighbour's layout is exactly what goes silently
    /// wrong — and unreadable / unparsable are their own refusals, never a default.
    #[test]
    fn a_driver_absent_from_the_matrix_is_refused_by_name() {
        for v in [
            "595.91.08",
            "595.91.06",
            "580.65.05",
            "610.43.01",
            "590.44.01",
            "999.1.01",
        ] {
            let e = gate(Some(v)).expect_err("unmeasured must refuse");
            assert!(e.contains(v), "names the driver: {e}");
            assert!(e.contains("not a measured tag"), "says why: {e}");
        }
        let unread = gate(None).expect_err("silence refuses");
        assert!(unread.contains("did not answer"), "{unread}");
        for bad in ["", "580", "x595.91.07", "595.91.07.1"] {
            let e = gate(Some(bad)).expect_err("nonsense refuses");
            assert!(e.contains("refusing rather than assuming"), "{bad:?}: {e}");
        }
    }

    /// ★★★ (c, verbatim half) Every block written at HAND offsets has ONE layout at every
    /// matrix tag. If a future sweep moves one, this fails — and the fix is a carry, never a
    /// widened gate.
    #[test]
    fn every_verbatim_block_is_one_layout_across_the_matrix() {
        for runs in verbatim_rows() {
            let layouts: Vec<&Layout> = runs.runs.iter().filter_map(|r| r.value).collect();
            assert!(
                runs.runs.iter().all(|r| r.value.is_some()),
                "{} is absent at some measured tag",
                runs.name
            );
            assert!(
                layouts.windows(2).all(|w| w[0] == w[1]),
                "{} differs across the matrix: carry it instead of writing it verbatim",
                runs.name
            );
        }
        // Every UVM block crosses at every matrix tag (its NUMBER is the bench's, its body
        // carries), and every UVM ioctl number is one value across the matrix.
        for t in MEASURED {
            let abi = ClientAbi::for_host(HostDriverVersion {
                major: t.major,
                minor: t.minor,
                patch: t.patch,
            })
            .expect("measured");
            for (name, runs) in UVM_BLOCKS {
                let size = Resolved::of(runs, BENCH_DRIVER).expect("bench").size();
                let mut body = vec![0u8; size];
                abi.uvm_issue(name, &mut body, |_| ())
                    .unwrap_or_else(|e| panic!("{name} at {}: {e}", tag(*t)));
                let no = format!("uvm_ioctls:{name}");
                assert_eq!(
                    value(&no, *t),
                    value(&no, BENCH_DRIVER),
                    "{no} at {}",
                    tag(*t)
                );
            }
        }
    }

    /// ★★★ The two UVM blocks that DO change in the matrix, carried at the trusted host's
    /// 595.91.07: `[matrix]` `UVM_FREE_PARAMS` 24 → 16 bytes and `UVM_UNREGISTER_CHANNEL_PARAMS`
    /// 28 → 12 bytes at 590.48.01..615.71.09. The inputs the host still takes land at ITS offsets,
    /// the removed ones are dropped (and only those), and `rmStatus` comes back to the bench offset.
    #[test]
    fn the_595_uvm_free_and_unregister_channel_are_carried() {
        let abi = host("595.91.07");
        // UVM_FREE: base @0 (both), length @8 (bench only), rmStatus @16 → @8.
        let mut free = vec![0u8; 24];
        free[0..8].copy_from_slice(&0x7f00_0000_0000u64.to_le_bytes());
        free[8..16].copy_from_slice(&0x20_0000u64.to_le_bytes());
        let mut seen = Vec::new();
        abi.uvm_issue("UVM_FREE", &mut free, |w| {
            seen = w.clone();
            w[8..12].copy_from_slice(&0x1Fu32.to_le_bytes());
        })
        .expect("carried");
        assert_eq!(seen.len(), 16);
        assert_eq!(
            u64::from_le_bytes(seen[0..8].try_into().unwrap()),
            0x7f00_0000_0000
        );
        assert_eq!(
            u32::from_le_bytes(free[16..20].try_into().unwrap()),
            0x1F,
            "status back"
        );
        // UVM_UNREGISTER_CHANNEL: uuid @0 (bench only), hClient @16 → @0, hChannel @20 → @4.
        let mut un = vec![0u8; 28];
        un[0..16].copy_from_slice(&[0xAB; 16]);
        un[16..20].copy_from_slice(&0xC1u32.to_le_bytes());
        un[20..24].copy_from_slice(&0xC2u32.to_le_bytes());
        let mut seen = Vec::new();
        abi.uvm_issue("UVM_UNREGISTER_CHANNEL", &mut un, |w| seen = w.clone())
            .expect("carried");
        assert_eq!(seen.len(), 12);
        assert_eq!(u32::from_le_bytes(seen[0..4].try_into().unwrap()), 0xC1);
        assert_eq!(u32::from_le_bytes(seen[4..8].try_into().unwrap()), 0xC2);
        // At the bench line both are the bench's own layout.
        let mut same = vec![0u8; 24];
        host("580.159.04")
            .uvm_issue("UVM_FREE", &mut same, |w| assert_eq!(w.len(), 24))
            .expect("verbatim");
        // ⊘ A drop that is NOT a host-API removal is refused by name: UVM_REGISTER_CHANNEL has no
        // removed field, so nothing may be dropped from it (asserted via the allow-list itself).
        assert!(
            UVM_API_REMOVED
                .iter()
                .all(|(s, _)| *s == "UVM_FREE_PARAMS" || *s == "UVM_UNREGISTER_CHANNEL_PARAMS"),
            "the allow-list is exactly the two measured API removals"
        );
    }

    /// ★★★ (c, carried half) Every carried struct, at every matrix tag where it differs from
    /// the bench, round-trips through the host's layout: each field present in both layouts
    /// arrives at the HOST's offset with the bench body's bytes, and comes back unchanged.
    /// Prints the census of what differs where (the `[matrix]` rows behind the carry).
    #[test]
    fn every_struct_that_differs_between_tags_is_carried_field_by_field() {
        let mut carried = 0;
        for runs in carried_rows() {
            let bench = Resolved::of(runs, BENCH_DRIVER).expect("present at the bench");
            for t in MEASURED {
                let abi = ClientAbi::for_host(HostDriverVersion {
                    major: t.major,
                    minor: t.minor,
                    patch: t.patch,
                })
                .expect("measured");
                let c = match abi.crossing(runs) {
                    Ok(c) => c,
                    Err(Refusal::Abi(HostAbiError::Layout(
                        kf_abi::matrix::LayoutError::NoStruct { .. },
                    ))) => continue, // absent at this tag: refused by name at use
                    Err(e) => panic!("{} at {}: {e}", runs.name, tag(*t)),
                };
                let Crossing::Carried { .. } = c else {
                    continue;
                };
                let host = Resolved::of(runs, *t).expect("present");
                // Fill every scalar leaf both layouts share with a per-field pattern.
                let mut body = vec![0u8; bench.size()];
                let mut shared = Vec::new();
                for (path, f) in bench.layout.fields {
                    let Some(hf) = host.layout.field(path) else {
                        continue;
                    };
                    let (Some(r), Some(hr)) = (f.range(), hf.range()) else {
                        continue;
                    };
                    let leaf = !bench
                        .layout
                        .fields
                        .iter()
                        .any(|(p, _)| p.len() > path.len() && p.starts_with(path));
                    if !leaf || f.array().is_some() || r.len() != hr.len() || path.contains("[]") {
                        continue;
                    }
                    for (i, b) in body[r.clone()].iter_mut().enumerate() {
                        *b = 0x40 ^ (shared.len() as u8) ^ (i as u8);
                    }
                    shared.push((*path, r, hr));
                }
                let mut wire = Vec::new();
                let r = c.issue(&abi, &mut body.clone(), |h| wire = h.to_vec());
                r.unwrap_or_else(|e| panic!("{} at {}: {e}", runs.name, tag(*t)));
                assert_eq!(wire.len(), host.size(), "{} at {}", runs.name, tag(*t));
                for (path, r, hr) in &shared {
                    assert_eq!(
                        &wire[hr.clone()],
                        &body[r.clone()],
                        "{}.{path} must arrive at the host's offset at {}",
                        runs.name,
                        tag(*t)
                    );
                }
                let mut back = body.clone();
                c.issue(&abi, &mut back, |_| ()).expect("round trip");
                for (path, r, _) in &shared {
                    assert_eq!(
                        &back[r.clone()],
                        &body[r.clone()],
                        "{}.{path} back",
                        runs.name
                    );
                }
                carried += 1;
            }
        }
        assert!(
            carried > 0,
            "the matrix has carried rows (NVOS46 ≤575 at least)"
        );
    }

    /// ★★★ The concrete delta the old R2 refused 610 for — `NV_CHANNEL_ALLOC_PARAMS` gains
    /// `hHandleVASpace` at +32 at 610.43.02 — is now CARRIED: the client's own encoder writes
    /// `engineType` at +128 and the host receives it at **+136**.
    ///
    /// ⊘ CORRECTED 2026-10-08, by measurement: the old R2 refusal text (and `rm.rs`'s module docs)
    /// said `engineType` moves to **+132**. The matrix says +136 (`traces/driver_matrix/ranges.tsv`:
    /// `NV_CHANNEL_ALLOC_PARAMS.engineType` 128+4 at 535.309.01..595.91.07, 136+4 at
    /// 610.43.02..615.71.09) — the +4 insertion also re-pads the 8-aligned fields behind it. A
    /// hand-written +4 shift would have put the engine in padding; the carry goes by name.
    /// `[matrix]` runs: 535.309.01..565.57.01 = 360 B, 570.86.15..595.91.07 = 368 B (the
    /// bench's), 610.43.02..615.71.09 = 376 B.
    #[test]
    fn a_610_channel_is_carried_with_engine_type_at_its_own_offset() {
        use kayfabe_abi::submit::ChannelAllocParams;
        let mut body = [0u8; ChannelAllocParams::SIZE];
        ChannelAllocParams {
            engine_type: 0x0D,
            gp_fifo_entries: 64,
            ..Default::default()
        }
        .encode_into(&mut body)
        .expect("encodes");
        assert_eq!(u32::from_le_bytes(body[128..132].try_into().unwrap()), 0x0D);
        let abi = host("610.43.02");
        let c = abi.crossing(&m::NV_CHANNEL_ALLOC_PARAMS).expect("exists");
        assert!(
            matches!(c, Crossing::Carried { .. }),
            "610 differs from the bench"
        );
        let mut wire = Vec::new();
        c.issue(&abi, &mut body, |h| wire = h.to_vec())
            .expect("carries");
        assert_eq!(wire.len(), 376);
        assert_eq!(u32::from_le_bytes(wire[136..140].try_into().unwrap()), 0x0D);
        assert_eq!(
            u32::from_le_bytes(wire[132..136].try_into().unwrap()),
            0,
            "not +132"
        );
        // And at 595.91.07 the channel is the bench's own layout: untouched.
        assert!(matches!(
            host("595.91.07").crossing(&m::NV_CHANNEL_ALLOC_PARAMS),
            Ok(Crossing::Verbatim)
        ));
    }

    /// ★★ `NVOS46` below 580.65.06 has no `kindOverride`: a map that asks for a kind is REFUSED
    /// by name there (the host cannot be asked it), and one that does not is carried to 56 bytes.
    #[test]
    fn a_kinded_map_on_a_575_host_is_refused_by_name_and_a_plain_one_carried() {
        use kayfabe_abi::generated::nvos::Nvos46Parameters;
        let abi = host("575.64.05");
        let c = abi.nvos46().expect("exists");
        let mut plain = [0u8; Nvos46Parameters::SIZE];
        Nvos46Parameters {
            dma_offset: 0x1234_5000,
            ..Default::default()
        }
        .encode_into(&mut plain)
        .expect("encodes");
        let mut wire = Vec::new();
        c.issue(&abi, &mut plain, |h| wire = h.to_vec())
            .expect("plain map carries");
        assert_eq!(wire.len(), 56);
        let mut kinded = [0u8; Nvos46Parameters::SIZE];
        Nvos46Parameters {
            kind_override: 0xFE,
            ..Default::default()
        }
        .encode_into(&mut kinded)
        .expect("encodes");
        let e = c
            .issue(&abi, &mut kinded, |_| ())
            .expect_err("kind refused");
        assert!(e.to_string().contains("kindOverride"), "{e}");
    }

    /// ★★ Every carried struct the client builds with a typed encoder is the BENCH layout's size —
    /// the carry's precondition (a body of any other size is refused, not guessed at).
    #[test]
    fn the_clients_encoders_are_the_bench_layouts() {
        use kayfabe_abi::generated::nvos::{Nvos46Parameters, Nvos47Parameters};
        use kayfabe_abi::submit as s;
        let size = |r: &'static StructRuns| Resolved::of(r, BENCH_DRIVER).expect("bench").size();
        assert_eq!(Nvos46Parameters::SIZE, size(&m::NVOS46_PARAMETERS));
        assert_eq!(Nvos47Parameters::SIZE, size(&m::NVOS47_PARAMETERS));
        assert_eq!(
            s::ChannelAllocParams::SIZE,
            size(&m::NV_CHANNEL_ALLOC_PARAMS)
        );
        assert_eq!(
            s::NvMemoryAllocationParams::SIZE,
            size(&m::NV_MEMORY_ALLOCATION_PARAMS)
        );
        assert_eq!(
            s::NvMemoryListAllocationParams::SIZE,
            size(&m::NV_MEMORY_LIST_ALLOCATION_PARAMS)
        );
        assert_eq!(
            s::CeAllocParams::SIZE,
            size(&m::NVB0B5_ALLOCATION_PARAMETERS)
        );
        assert_eq!(
            s::GpfifoScheduleParams::SIZE,
            size(&m::NVA06C_CTRL_GPFIFO_SCHEDULE_PARAMS)
        );
        assert_eq!(
            s::DmaGetPteInfoParams::SIZE,
            size(&m::NV0080_CTRL_DMA_GET_PTE_INFO_PARAMS)
        );
        assert_eq!(
            s::DmaGetPdeInfoParams::SIZE,
            size(&m::NV0080_CTRL_DMA_GET_PDE_INFO_PARAMS)
        );
        assert_eq!(
            s::MmuFaultInfoParams::SIZE,
            size(&m::NV906F_CTRL_GET_MMU_FAULT_INFO_PARAMS)
        );
        assert_eq!(s::BIND_PARAMS_SIZE, size(&m::NVA06C_CTRL_BIND_PARAMS));
        assert_eq!(
            s::WORK_SUBMIT_TOKEN_PARAMS_SIZE,
            size(&m::NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN_PARAMS)
        );
        assert_eq!(
            kayfabe_abi::generated::classes::Nv0080AllocParameters::SIZE,
            size(&m::NV0080_ALLOC_PARAMETERS)
        );
        assert_eq!(
            kayfabe_abi::generated::classes::NvChannelGroupAllocationParameters::SIZE,
            size(&m::NV_CHANNEL_GROUP_ALLOCATION_PARAMETERS)
        );
        assert_eq!(
            kayfabe_abi::bringup::NvVaspaceAllocationParameters::SIZE,
            size(&m::NV_VASPACE_ALLOCATION_PARAMETERS)
        );
        assert_eq!(
            kayfabe_abi::bringup::NvMemoryVirtualAllocationParams::SIZE,
            size(&m::NV_MEMORY_VIRTUAL_ALLOCATION_PARAMS)
        );
        assert_eq!(
            kayfabe_abi::bringup::Nv2080AllocParameters::SIZE,
            size(&m::NV2080_ALLOC_PARAMETERS)
        );
        assert_eq!(
            kayfabe_abi::bringup::GpuIdInfoV2::SIZE,
            size(&m::NV0000_CTRL_GPU_GET_ID_INFO_V2_PARAMS)
        );
        assert_eq!(
            kayfabe_chips::host_classes::CLASSLIST_V2_SIZE,
            size(&m::NV0080_CTRL_GPU_GET_CLASSLIST_V2_PARAMS)
        );
    }

    /// ★★★ WHAT 595 ACTUALLY CHANGES for this client, pinned: of every block it sends, exactly
    /// three differ from the bench at 595.91.07 (`GPU_GET_NAME_STRING` 132 → 68 bytes — the
    /// `unicode` member gone; `UVM_FREE` and `UVM_UNREGISTER_CHANNEL` — both since 590.48.01), and
    /// NONE differs at any 580 tag from 580.65.06 up — so on the bench line every block crosses
    /// byte-for-byte as before 2026-10-08.
    #[test]
    fn what_differs_at_595_and_on_the_580_line() {
        let all: Vec<&'static StructRuns> = carried_rows()
            .into_iter()
            .chain(verbatim_rows())
            .chain(UVM_BLOCKS.iter().map(|(_, r)| *r))
            .collect();
        let differs = |v: &str| {
            let host = HostDriverVersion::parse(v).expect("parses");
            let host = DriverVersion {
                major: host.major,
                minor: host.minor,
                patch: host.patch,
            };
            let mut d: Vec<&str> = all
                .iter()
                .filter(|r| r.at(BENCH_DRIVER).ok().flatten() != r.at(host).ok().flatten())
                .map(|r| r.name)
                .collect();
            d.sort_unstable();
            d.dedup();
            d
        };
        assert_eq!(
            differs("595.91.07"),
            vec![
                "NV2080_CTRL_GPU_GET_NAME_STRING_PARAMS",
                "UVM_FREE_PARAMS",
                "UVM_UNREGISTER_CHANNEL_PARAMS"
            ]
        );
        for t in MEASURED.iter().filter(|t| t.major == 580) {
            assert!(
                differs(&tag(*t)).is_empty(),
                "{}: {:?}",
                tag(*t),
                differs(&tag(*t))
            );
        }
    }

    /// ★★ The nested alloc's pointer is written at a BENCH offset, so its body must never move.
    #[test]
    fn the_nested_alloc_body_never_moves() {
        let runs = &m::NV_MEMORY_LIST_ALLOCATION_PARAMS;
        assert_eq!(runs.runs.len(), 1, "one layout at every measured tag");
        let l = runs.runs[0].value.expect("present");
        assert_eq!(
            l.field("pageNumberList").map(|f| f.off()),
            Some(kayfabe_abi::submit::NvMemoryListAllocationParams::PAGE_NUMBER_LIST_OFFSET)
        );
    }

    /// ★★ Every control row's id is in the matrix at the bench (the id the client sends), and each
    /// alloc row names at least one matrix class.
    #[test]
    fn every_row_resolves_in_the_matrix() {
        for (name, _) in CONTROLS {
            assert!(
                value(&format!("ctrl_cmds:{name}"), BENCH_DRIVER).is_some(),
                "{name} has no measured id at the bench"
            );
        }
        for (cn, runs) in ALLOC_PARAMS {
            let any = m::ALL_VALUES.iter().any(|v| {
                v.name.strip_prefix("class_ids:").is_some_and(|n| {
                    cn.matches(n) && v.at_u32(BENCH_DRIVER).ok().flatten().is_some()
                })
            });
            assert!(
                any,
                "{cn:?} ({}) names no measured class at the bench",
                runs.name
            );
        }
    }

    /// ★★ An unlisted control with a body keeps its old contract inside [580.65.06, 581) and is
    /// refused by name outside it; a listed one resolves by its measured id.
    #[test]
    fn an_unlisted_block_is_refused_outside_the_encoded_interval() {
        let probe = 0x2080_0ffe;
        assert!(matches!(
            host("580.159.04").control_crossing(probe, 4),
            Ok(Crossing::Verbatim)
        ));
        let e = host("595.91.07")
            .control_crossing(probe, 4)
            .expect_err("unlisted at 595");
        assert!(
            matches!(
                e,
                Refusal::Unlisted {
                    id: 0x2080_0ffe,
                    ..
                }
            ),
            "{e}"
        );
        assert!(e.to_string().contains("595.91.07"), "{e}");
        // An empty body carries nothing and is never refused.
        assert!(matches!(
            host("595.91.07").control_crossing(probe, 0),
            Ok(Crossing::Verbatim)
        ));
        // The class side: the matrix's channel class carries at 610 and is verbatim at 595.
        // Any GPFIFO channel class in the matrix (the role, never a generation's name).
        let chan = m::ALL_VALUES
            .iter()
            .filter(|v| {
                v.name
                    .strip_prefix("class_ids:")
                    .is_some_and(|n| n.ends_with("_CHANNEL_GPFIFO_A"))
            })
            .find_map(|v| v.at_u32(BENCH_DRIVER).ok().flatten())
            .expect("a channel class in the matrix");
        assert!(matches!(
            host("610.43.02").alloc_crossing(chan, 368),
            Ok(Crossing::Carried {
                strukt: "NV_CHANNEL_ALLOC_PARAMS",
                ..
            })
        ));
        assert!(matches!(
            host("595.91.07").alloc_crossing(chan, 368),
            Ok(Crossing::Verbatim)
        ));
    }
}
