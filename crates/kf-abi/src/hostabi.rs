//! ★★★ THE HOST DRIVER AXIS, MEASURED (`docs/design/V3_DRIVER_MATRIX.md` §2.2, §8.3).
//!
//! # The problem
//!
//! Every host-side RM struct kayfabe builds (`kf-host`, `kf-rm::hostquery`) was written against
//! ONE layout — the bench driver's, 580.159.04 — and [`crate::host_driver`] pinned the host to the
//! interval where that layout is true (`[580.65.06, 581)`). The matrix shows the same structs at
//! every other measured tag: `NVOS46` is 56 bytes below 580.65.06 (no `flags2`, no
//! `kindOverride`), `NVOS47` 40 bytes at 535/545, `GPFIFO_SCHEDULE` loses `bSkipEnable` at ≤570,
//! `NV_CHANNEL_ALLOC_PARAMS` is 360 bytes at ≤565 and gains `hHandleVASpace` at 610,
//! `GET_CLASSLIST_V2` holds 100 classes at 555–575 …
//!
//! # The rule: one encoder, carried at the boundary
//!
//! ⊘ Not a second set of encoders per version — that is the hand-row shape the matrix exists to
//! retire. The host side keeps speaking the BENCH layout internally; the struct is CARRIED to the
//! host's own measured layout on the way out and back on the way in, by field name, through the
//! same transcoder the guest axis uses ([`crate::matrix::transcode`]). What that buys, per struct:
//!
//! - **same layout** at the host's version (the common case inside a branch) ⇒ the bytes pass
//!   through untouched ([`HostAbi::carry`] returns [`Carry::Same`]);
//! - **moved fields** ⇒ carried by name;
//! - **a field the host's version does not have**, carrying a value ⇒ [`HostAbiError::Unexpressible`]:
//!   the host cannot be asked that (e.g. a kind override on a 575 host), and saying so by name is
//!   the only honest answer — the silent alternative is the host reading our field as its
//!   neighbour's;
//! - **a reply field only the host has** ⇒ dropped (reported), since the bench encoder cannot
//!   read it anyway.
//!
//! ⊘ **Exact membership, no nearest neighbour** — the matrix's own rule: a host driver that was
//! never measured is refused ([`HostAbiError::Unmeasured`]), because a release between two
//! measured tags is exactly where a borrowed layout is silently wrong.

use crate::DriverVersion;
use crate::host_driver::HostDriverVersion;
use crate::matrix::{LayoutError, Resolved, StructRuns, TranscodeError, is_measured, transcode};
use std::fmt;

/// The host driver's measured ABI: which layout each host-side struct has at that version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostAbi {
    version: DriverVersion,
}

/// How a struct crosses the boundary at this host version.
#[derive(Debug, Clone, Copy)]
pub enum Carry {
    /// The host's layout IS the bench's: the bytes pass through.
    Same {
        /// `sizeof` at both.
        size: usize,
    },
    /// The layouts differ: carry by name ([`HostAbi::to_host`] / [`HostAbi::from_host`]).
    Carried {
        /// The bench layout (what the encoders write).
        bench: Resolved,
        /// The host's layout (what the ioctl must carry).
        host: Resolved,
    },
}

impl Carry {
    /// The size the host's frontend expects for this struct.
    #[must_use]
    pub fn host_size(&self) -> usize {
        match self {
            Carry::Same { size } => *size,
            Carry::Carried { host, .. } => host.size(),
        }
    }
}

/// Why a struct cannot cross to this host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostAbiError {
    /// The host driver's version is not one of the measured tags.
    Unmeasured {
        /// The host driver.
        version: DriverVersion,
    },
    /// The struct does not exist at the bench or at the host version.
    Layout(LayoutError),
    /// The bench body sets a field the host's layout does not have — the host cannot be asked it.
    Unexpressible {
        /// The C type.
        strukt: &'static str,
        /// The fields carrying data that the host's version lacks.
        fields: Vec<&'static str>,
        /// The host driver.
        version: DriverVersion,
    },
    /// The transcoder refused (an array the target cannot hold, a narrowing that loses data, …).
    Transcode {
        /// The C type.
        strukt: &'static str,
        /// Which direction: `true` = to the host.
        outgoing: bool,
        /// The transcoder's refusal.
        err: TranscodeError,
    },
    /// A control kayfabe has no row for, at a host outside the interval its encoders were written for.
    UnlistedControl {
        /// The command.
        cmd: u32,
        /// The host driver.
        version: DriverVersion,
    },
    /// A GSS-legacy control (no public header) at a host whose layout for it was never measured.
    NoHeader {
        /// The command.
        cmd: u32,
        /// Where the known layout came from.
        source: &'static str,
        /// The host driver.
        version: DriverVersion,
    },
    /// The body handed in is not the size its layout says.
    Size {
        /// The C type.
        strukt: &'static str,
        /// Bytes expected.
        want: usize,
        /// Bytes given.
        got: usize,
    },
}

impl fmt::Display for HostAbiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unmeasured { version } => write!(
                f,
                "host driver {version} is not a measured tag of the driver matrix; kayfabe carries \
                 host structs only between measured layouts (tools/drivermatrix/regen.sh {version})"
            ),
            Self::Layout(e) => write!(f, "{e}"),
            Self::Unexpressible { strukt, fields, version } => write!(
                f,
                "{strukt}: host driver {version} has no {fields:?}, and the request sets them — the host \
                 cannot be asked this; refusing rather than letting it read the bytes as another field"
            ),
            Self::Transcode { strukt, outgoing, err } => write!(
                f,
                "{strukt} could not be carried {} the host driver's layout: {err}",
                if *outgoing { "to" } else { "back from" }
            ),
            Self::Size { strukt, want, got } => write!(f, "{strukt}: body is {got} bytes, its layout is {want}"),
            Self::UnlistedControl { cmd, version } => write!(
                f,
                "host control {cmd:#010x} has no row in kf_abi::hostabi::HOST_CONTROLS, and host driver \
                 {version} is outside the interval the host encoders were written for — its layout is unknown"
            ),
            Self::NoHeader { cmd, source, version } => write!(
                f,
                "host control {cmd:#010x} has no public header ({source}); its layout at host driver \
                 {version} was never measured"
            ),
        }
    }
}

impl std::error::Error for HostAbiError {}

impl From<LayoutError> for HostAbiError {
    fn from(e: LayoutError) -> Self {
        Self::Layout(e)
    }
}

impl HostAbi {
    /// The measured ABI of this host driver.
    ///
    /// # Errors
    /// [`HostAbiError::Unmeasured`] for a version outside the committed sweep.
    pub fn for_host(host: HostDriverVersion) -> Result<Self, HostAbiError> {
        let version = DriverVersion { major: host.major, minor: host.minor, patch: host.patch };
        if !is_measured(version) {
            return Err(HostAbiError::Unmeasured { version });
        }
        Ok(Self { version })
    }

    /// The host driver version this ABI was resolved for.
    #[must_use]
    pub fn version(&self) -> DriverVersion {
        self.version
    }

    /// Is the host the bench driver itself (where the encoders' layouts were written)?
    #[must_use]
    pub fn is_bench(&self) -> bool {
        self.version == crate::versions::BENCH_DRIVER
    }

    /// How `runs` crosses to this host.
    ///
    /// # Errors
    /// [`HostAbiError::Layout`] when the struct is absent at the bench or at the host.
    pub fn carry(&self, runs: &'static StructRuns) -> Result<Carry, HostAbiError> {
        let bench = Resolved::of(runs, crate::versions::BENCH_DRIVER)?;
        let host = Resolved::of(runs, self.version)?;
        // By value: two runs of one layout may be distinct promoted constants.
        if bench.layout == host.layout {
            Ok(Carry::Same { size: bench.size() })
        } else {
            Ok(Carry::Carried { bench, host })
        }
    }

    /// [`Self::carry`] for a struct the SDK renamed: the bench layout under `before`, the host's
    /// under `before` where it still exists, else under `after`.
    ///
    /// # Errors
    /// [`HostAbiError::Layout`] when neither name exists at the host.
    pub fn carry_renamed(&self, before: &'static StructRuns, after: &'static StructRuns) -> Result<Carry, HostAbiError> {
        let bench = Resolved::of(before, crate::versions::BENCH_DRIVER)?;
        let host = Resolved::of(before, self.version).or_else(|_| Resolved::of(after, self.version))?;
        if bench.layout == host.layout {
            Ok(Carry::Same { size: bench.size() })
        } else {
            Ok(Carry::Carried { bench, host })
        }
    }

    /// The size the host's frontend expects for `runs`.
    ///
    /// # Errors
    /// As [`Self::carry`].
    pub fn host_size(&self, runs: &'static StructRuns) -> Result<usize, HostAbiError> {
        Ok(self.carry(runs)?.host_size())
    }

    /// [`Self::to_host`] through an already-resolved [`Carry`] (a renamed struct, a control row).
    ///
    /// # Errors
    /// As [`Self::to_host`].
    pub fn carry_out(&self, strukt: &'static str, c: &Carry, bench_body: &[u8]) -> Result<Vec<u8>, HostAbiError> {
        match c {
            Carry::Same { size } => {
                if bench_body.len() != *size {
                    return Err(HostAbiError::Size { strukt, want: *size, got: bench_body.len() });
                }
                Ok(bench_body.to_vec())
            }
            Carry::Carried { bench, host } => {
                if bench_body.len() != bench.size() {
                    return Err(HostAbiError::Size { strukt, want: bench.size(), got: bench_body.len() });
                }
                let (out, dropped) =
                    transcode(bench, host, bench_body, &[]).map_err(|err| HostAbiError::Transcode { strukt, outgoing: true, err })?;
                if !dropped.is_empty() {
                    return Err(HostAbiError::Unexpressible { strukt, fields: dropped, version: self.version });
                }
                Ok(out)
            }
        }
    }

    /// [`Self::from_host`] through an already-resolved [`Carry`].
    ///
    /// # Errors
    /// As [`Self::from_host`].
    pub fn carry_in(&self, strukt: &'static str, c: &Carry, host_body: &[u8]) -> Result<Vec<u8>, HostAbiError> {
        match c {
            Carry::Same { size } => {
                if host_body.len() != *size {
                    return Err(HostAbiError::Size { strukt, want: *size, got: host_body.len() });
                }
                Ok(host_body.to_vec())
            }
            Carry::Carried { bench, host } => {
                if host_body.len() != host.size() {
                    return Err(HostAbiError::Size { strukt, want: host.size(), got: host_body.len() });
                }
                transcode(host, bench, host_body, &[])
                    .map(|(b, _)| b)
                    .map_err(|err| HostAbiError::Transcode { strukt, outgoing: false, err })
            }
        }
    }

    /// A bench-layout body, carried to the host's layout (outgoing).
    ///
    /// # Errors
    /// [`HostAbiError::Unexpressible`] when a field the host lacks carries data;
    /// [`HostAbiError::Transcode`] / [`HostAbiError::Size`] / [`HostAbiError::Layout`].
    pub fn to_host(&self, runs: &'static StructRuns, bench_body: &[u8]) -> Result<Vec<u8>, HostAbiError> {
        match self.carry(runs)? {
            Carry::Same { size } => {
                if bench_body.len() != size {
                    return Err(HostAbiError::Size { strukt: runs.name, want: size, got: bench_body.len() });
                }
                Ok(bench_body.to_vec())
            }
            Carry::Carried { bench, host } => {
                if bench_body.len() != bench.size() {
                    return Err(HostAbiError::Size { strukt: runs.name, want: bench.size(), got: bench_body.len() });
                }
                let (out, dropped) = transcode(&bench, &host, bench_body, &[])
                    .map_err(|err| HostAbiError::Transcode { strukt: runs.name, outgoing: true, err })?;
                if !dropped.is_empty() {
                    return Err(HostAbiError::Unexpressible { strukt: runs.name, fields: dropped, version: self.version });
                }
                Ok(out)
            }
        }
    }

    /// A host-layout body (the host's reply), carried back to the bench layout (incoming). Fields
    /// only the host's version has are dropped — the bench encoders cannot read them.
    ///
    /// # Errors
    /// [`HostAbiError::Transcode`] (e.g. the host returned more array entries than the bench
    /// struct holds) / [`HostAbiError::Size`] / [`HostAbiError::Layout`].
    pub fn from_host(&self, runs: &'static StructRuns, host_body: &[u8]) -> Result<Vec<u8>, HostAbiError> {
        match self.carry(runs)? {
            Carry::Same { size } => {
                if host_body.len() != size {
                    return Err(HostAbiError::Size { strukt: runs.name, want: size, got: host_body.len() });
                }
                Ok(host_body.to_vec())
            }
            Carry::Carried { bench, host } => {
                if host_body.len() != host.size() {
                    return Err(HostAbiError::Size { strukt: runs.name, want: host.size(), got: host_body.len() });
                }
                transcode(&host, &bench, host_body, &[])
                    .map(|(b, _dropped)| b)
                    .map_err(|err| HostAbiError::Transcode { strukt: runs.name, outgoing: false, err })
            }
        }
    }
}

// =====================================================================================
// ★★ Every control kayfabe issues to its HOST, and the struct it carries
// =====================================================================================

/// What a host control's parameter block is, for the carry.
#[derive(Debug, Clone, Copy)]
pub enum HostParams {
    /// A measured struct: carried between the bench layout and the host's.
    Measured(&'static StructRuns),
    /// A measured struct that the SDK RENAMED at some version (the old name then survives only as a
    /// `#define` alias, which DWARF cannot see): the layout is the old name's where it exists, the
    /// new name's elsewhere. `[matrix]` MSENC/BSP caps became NVENC/NVDEC at 610.43.02.
    Renamed {
        /// The name the encoders were written against (the bench's).
        before: &'static StructRuns,
        /// The name the struct carries after the rename.
        after: &'static StructRuns,
    },
    /// No public header (GSS-legacy ids — bit 15 of the command, routed straight to the GSP): the
    /// layout is only known from measurements against 580.x hosts, so it is passed through on the
    /// interval the host encoders were written for and refused elsewhere, by name.
    NoHeader {
        /// Where the layout came from.
        source: &'static str,
    },
}

/// One control issued to the host.
#[derive(Debug, Clone, Copy)]
pub struct HostControl {
    /// The SDK's name for the command (`ctrl_cmds:<name>` in the matrix) — `""` for GSS-legacy.
    pub name: &'static str,
    /// The command id kayfabe sends.
    pub cmd: u32,
    /// Its parameter block.
    pub params: HostParams,
}

use crate::generated::matrix as hm;

macro_rules! hc {
    ($name:literal, $cmd:literal, $s:ident) => {
        HostControl { name: $name, cmd: $cmd, params: HostParams::Measured(&hm::$s) }
    };
}

/// ★★ The complete set of controls kf-host and kf-rm's host queries send (the host-side
/// inventory of 2026-09-26, `V3_DRIVER_MATRIX.md` §2.2). ⊘ A control NOT here is refused at any
/// host whose layouts differ from the bench's — an unlisted row must never mean "pass it through".
/// `every_host_control_id_is_the_measured_one` pins each id to the matrix at every tag.
pub static HOST_CONTROLS: &[HostControl] = &[
    hc!("NV0000_CTRL_CMD_GPU_GET_ID_INFO_V2", 0x0000_0205, NV0000_CTRL_GPU_GET_ID_INFO_V2_PARAMS),
    hc!("NV0000_CTRL_CMD_OS_UNIX_EXPORT_OBJECT_TO_FD", 0x0000_3d05, NV0000_CTRL_OS_UNIX_EXPORT_OBJECT_TO_FD_PARAMS),
    hc!("NV0000_CTRL_CMD_OS_UNIX_IMPORT_OBJECT_FROM_FD", 0x0000_3d06, NV0000_CTRL_OS_UNIX_IMPORT_OBJECT_FROM_FD_PARAMS),
    hc!("NV0080_CTRL_CMD_GPU_GET_CLASSLIST_V2", 0x0080_0292, NV0080_CTRL_GPU_GET_CLASSLIST_V2_PARAMS),
    hc!("NV0080_CTRL_CMD_PERF_CUDA_LIMIT_SET_CONTROL", 0x0080_1909, NV0080_CTRL_PERF_CUDA_LIMIT_CONTROL_PARAMS),
    HostControl {
        name: "NV0080_CTRL_CMD_MSENC_GET_CAPS_V2",
        cmd: 0x0080_1b02,
        params: HostParams::Renamed { before: &hm::NV0080_CTRL_MSENC_GET_CAPS_V2_PARAMS, after: &hm::NV0080_CTRL_NVENC_GET_CAPS_V2_PARAMS },
    },
    HostControl {
        name: "NV0080_CTRL_CMD_BSP_GET_CAPS_V2",
        cmd: 0x0080_1c02,
        params: HostParams::Renamed { before: &hm::NV0080_CTRL_BSP_GET_CAPS_PARAMS_V2, after: &hm::NV0080_CTRL_NVDEC_GET_CAPS_PARAMS_V2 },
    },
    hc!("NV2080_CTRL_CMD_GPU_GET_INFO_V2", 0x2080_0102, NV2080_CTRL_GPU_GET_INFO_V2_PARAMS),
    hc!("NV2080_CTRL_CMD_GPU_GET_NAME_STRING", 0x2080_0110, NV2080_CTRL_GPU_GET_NAME_STRING_PARAMS),
    hc!("NV2080_CTRL_CMD_GPU_GET_SHORT_NAME_STRING", 0x2080_0111, NV2080_CTRL_GPU_GET_SHORT_NAME_STRING_PARAMS),
    hc!("NV2080_CTRL_CMD_GPU_GET_PES_INFO", 0x2080_0168, NV2080_CTRL_GPU_GET_PES_INFO_PARAMS),
    hc!("NV2080_CTRL_CMD_GPU_GET_ENGINES_V2", 0x2080_0170, NV2080_CTRL_GPU_GET_ENGINES_V2_PARAMS),
    hc!("NV2080_CTRL_CMD_GPU_GET_CONSTRUCTED_FALCON_INFO", 0x2080_01b0, NV2080_CTRL_GPU_GET_CONSTRUCTED_FALCON_INFO_PARAMS),
    hc!("NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION", 0x2080_0301, NV2080_CTRL_EVENT_SET_NOTIFICATION_PARAMS),
    hc!("NV2080_CTRL_CMD_BIOS_GET_INFO_V2", 0x2080_0810, NV2080_CTRL_BIOS_GET_INFO_V2_PARAMS),
    hc!("NV2080_CTRL_CMD_FIFO_DISABLE_CHANNELS", 0x2080_110b, NV2080_CTRL_FIFO_DISABLE_CHANNELS_PARAMS),
    hc!("NV2080_CTRL_CMD_GR_GET_ZCULL_INFO", 0x2080_1206, NV2080_CTRL_GR_GET_ZCULL_INFO_PARAMS),
    hc!("NV2080_CTRL_CMD_GR_CTXSW_ZCULL_BIND", 0x2080_1208, NV2080_CTRL_GR_CTXSW_ZCULL_BIND_PARAMS),
    hc!("NV2080_CTRL_CMD_GR_SET_CTXSW_PREEMPTION_MODE", 0x2080_1210, NV2080_CTRL_GR_SET_CTXSW_PREEMPTION_MODE_PARAMS),
    hc!("NV2080_CTRL_CMD_GR_GET_GLOBAL_SM_ORDER", 0x2080_121b, NV2080_CTRL_GR_GET_GLOBAL_SM_ORDER_PARAMS),
    hc!("NV2080_CTRL_CMD_GR_GET_CAPS_V2", 0x2080_1227, NV2080_CTRL_GR_GET_CAPS_V2_PARAMS),
    hc!("NV2080_CTRL_CMD_GR_GET_INFO_V2", 0x2080_1228, NV2080_CTRL_GR_GET_INFO_V2_PARAMS),
    hc!("NV2080_CTRL_CMD_GR_GET_GPC_MASK", 0x2080_122a, NV2080_CTRL_GR_GET_GPC_MASK_PARAMS),
    hc!("NV2080_CTRL_CMD_GR_GET_TPC_MASK", 0x2080_122b, NV2080_CTRL_GR_GET_TPC_MASK_PARAMS),
    hc!("NV2080_CTRL_CMD_GR_GET_ENGINE_CONTEXT_PROPERTIES", 0x2080_122d, NV2080_CTRL_GR_GET_ENGINE_CONTEXT_PROPERTIES_PARAMS),
    hc!("NV2080_CTRL_CMD_GR_GET_SM_ISSUE_RATE_MODIFIER", 0x2080_1230, NV2080_CTRL_GR_GET_SM_ISSUE_RATE_MODIFIER_PARAMS),
    hc!("NV2080_CTRL_CMD_GR_GET_NUM_TPCS_FOR_GPC", 0x2080_1234, NV2080_CTRL_GR_GET_NUM_TPCS_FOR_GPC_PARAMS),
    hc!("NV2080_CTRL_CMD_GR_GET_ZCULL_MASK", 0x2080_1237, NV2080_CTRL_GR_GET_ZCULL_MASK_PARAMS),
    hc!("NV2080_CTRL_CMD_GR_GET_GFX_GPC_AND_TPC_INFO", 0x2080_1239, NV2080_CTRL_GR_GET_GFX_GPC_AND_TPC_INFO_PARAMS),
    hc!("NV2080_CTRL_CMD_FB_GET_INFO_V2", 0x2080_1303, NV2080_CTRL_FB_GET_INFO_V2_PARAMS),
    hc!("NV2080_CTRL_CMD_FB_FLUSH_GPU_CACHE", 0x2080_130e, NV2080_CTRL_FB_FLUSH_GPU_CACHE_PARAMS),
    hc!("NV2080_CTRL_CMD_FB_GET_GPU_CACHE_INFO", 0x2080_1315, NV2080_CTRL_FB_GET_GPU_CACHE_INFO_PARAMS),
    hc!("NV2080_CTRL_CMD_MC_GET_ARCH_INFO", 0x2080_1701, NV2080_CTRL_MC_GET_ARCH_INFO_PARAMS),
    hc!("NV2080_CTRL_CMD_MC_GET_STATIC_INTR_TABLE", 0x2080_170e, NV2080_CTRL_MC_GET_STATIC_INTR_TABLE_PARAMS),
    hc!("NV2080_CTRL_CMD_MC_GET_INTR_CATEGORY_SUBTREE_MAP", 0x2080_170f, NV2080_CTRL_MC_GET_INTR_CATEGORY_SUBTREE_MAP_PARAMS),
    hc!("NV2080_CTRL_CMD_BUS_GET_INFO_V2", 0x2080_1823, NV2080_CTRL_BUS_GET_INFO_V2_PARAMS),
    hc!("NV2080_CTRL_CMD_BUS_GET_C2C_INFO", 0x2080_182b, NV2080_CTRL_CMD_BUS_GET_C2C_INFO_PARAMS),
    hc!("NV2080_CTRL_CMD_PERF_GET_LEVEL_INFO_V2", 0x2080_200b, NV2080_CTRL_PERF_GET_LEVEL_INFO_V2_PARAMS),
    hc!("NV2080_CTRL_CMD_DMA_INVALIDATE_TLB", 0x2080_2502, NV2080_CTRL_DMA_INVALIDATE_TLB_PARAMS),
    hc!("NV2080_CTRL_CMD_CE_GET_CE_PCE_MASK", 0x2080_2a02, NV2080_CTRL_CE_GET_CE_PCE_MASK_PARAMS),
    hc!("NV2080_CTRL_CMD_CE_GET_CAPS_V2", 0x2080_2a03, NV2080_CTRL_CE_GET_CAPS_V2_PARAMS),
    hc!("NV2080_CTRL_CMD_CE_GET_ALL_CAPS", 0x2080_2a0a, NV2080_CTRL_CE_GET_ALL_CAPS_PARAMS),
    hc!("NV2080_CTRL_CMD_GSP_GET_FEATURES", 0x2080_3601, NV2080_CTRL_GSP_GET_FEATURES_PARAMS),
    hc!("NV2080_CTRL_CMD_GRMGR_GET_GR_FS_INFO", 0x2080_3801, NV2080_CTRL_GRMGR_GET_GR_FS_INFO_PARAMS),
    hc!("NV83DE_CTRL_CMD_DEBUG_SET_EXCEPTION_MASK", 0x83de_0309, NV83DE_CTRL_DEBUG_SET_EXCEPTION_MASK_PARAMS),
    hc!("NV9096_CTRL_CMD_GET_ZBC_CLEAR_TABLE_SIZE", 0x9096_0106, NV9096_CTRL_GET_ZBC_CLEAR_TABLE_SIZE_PARAMS),
    hc!("NVA06C_CTRL_CMD_GPFIFO_SCHEDULE", 0xa06c_0101, NVA06C_CTRL_GPFIFO_SCHEDULE_PARAMS),
    hc!("NVA06C_CTRL_CMD_BIND", 0xa06c_0102, NVA06C_CTRL_BIND_PARAMS),
    hc!("NVA06C_CTRL_CMD_SET_TIMESLICE", 0xa06c_0103, NVA06C_CTRL_TIMESLICE_PARAMS),
    hc!("NVA06C_CTRL_CMD_PREEMPT", 0xa06c_0105, NVA06C_CTRL_PREEMPT_PARAMS),
    hc!("NVA06F_CTRL_CMD_BIND", 0xa06f_0104, NVA06F_CTRL_BIND_PARAMS),
    hc!("NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN", 0xc36f_0108, NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN_PARAMS),
    // GSS-legacy (bit 15; no open header) — `crate::gssreplay`.
    HostControl { name: "", cmd: 0x2080_9064, params: HostParams::NoHeader { source: "kf_abi::gssreplay ROWS (measured on 580.159.04 hosts)" } },
    HostControl { name: "", cmd: 0x2080_a028, params: HostParams::NoHeader { source: "kf_abi::gssreplay ROWS (measured on 580.159.04 hosts)" } },
    HostControl { name: "", cmd: 0x2080_a084, params: HostParams::NoHeader { source: "kf_abi::gssreplay ROWS (measured on 580.159.04 hosts)" } },
    HostControl { name: "", cmd: 0x2080_a026, params: HostParams::NoHeader { source: "kf_abi::gssreplay ROWS (measured on 580.159.04 hosts)" } },
    HostControl { name: "", cmd: 0x2080_8163, params: HostParams::NoHeader { source: "kf_abi::gssreplay ENC session (measured on 580.159.04 hosts)" } },
    HostControl { name: "", cmd: 0x2080_8164, params: HostParams::NoHeader { source: "kf_abi::gssreplay ENC session (measured on 580.159.04 hosts)" } },
];

/// The row for `cmd`, if kayfabe issues it to a host.
#[must_use]
pub fn host_control(cmd: u32) -> Option<&'static HostControl> {
    HOST_CONTROLS.iter().find(|c| c.cmd == cmd)
}

impl HostAbi {
    /// Is the host inside the interval the host encoders were written for (`[580.65.06, 581)`,
    /// `crate::host_driver`)? Where it is, a control with no public header keeps its old contract.
    #[must_use]
    pub fn in_encoded_interval(&self) -> bool {
        HostDriverVersion { major: self.version.major, minor: self.version.minor, patch: self.version.patch }
            .is_encoded_for()
    }

    /// How control `cmd` crosses to this host. `Ok(None)` = pass the bytes through unchanged.
    ///
    /// # Errors
    /// [`HostAbiError::UnlistedControl`] — a control outside [`HOST_CONTROLS`] at a host whose
    /// layouts are not the ones kayfabe's encoders were written for; [`HostAbiError::NoHeader`] —
    /// a GSS-legacy control outside that interval; the [`Self::carry`] refusals.
    pub fn control_carry(&self, cmd: u32) -> Result<Option<Carry>, HostAbiError> {
        match host_control(cmd).map(|c| c.params) {
            Some(HostParams::Measured(runs)) => self.carry(runs).map(Some),
            Some(HostParams::Renamed { before, after }) => self.carry_renamed(before, after).map(Some),
            Some(HostParams::NoHeader { source }) => {
                if self.in_encoded_interval() {
                    Ok(None)
                } else {
                    Err(HostAbiError::NoHeader { cmd, source, version: self.version })
                }
            }
            None if self.is_bench() || self.in_encoded_interval() => Ok(None),
            None => Err(HostAbiError::UnlistedControl { cmd, version: self.version }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generated::matrix as m;

    fn host(v: &str) -> HostAbi {
        HostAbi::for_host(HostDriverVersion::parse(v).expect("version")).expect("measured")
    }

    /// ★ Every listed id is the SDK's own for its name, at every measured tag where it exists —
    /// and where the id is absent, its struct is absent too (the carry then refuses by name).
    #[test]
    fn every_host_control_id_is_the_measured_one() {
        for c in HOST_CONTROLS {
            let (runs, after) = match c.params {
                HostParams::Measured(r) => (r, None),
                HostParams::Renamed { before, after } => (before, Some(after)),
                HostParams::NoHeader { .. } => continue,
            };
            let row = hm::ALL_VALUES
                .iter()
                .find(|r| r.name == format!("ctrl_cmds:{}", c.name) || r.name == format!("host_zbc_cmds:{}", c.name))
                .unwrap_or_else(|| panic!("{} is not in the matrix (consumed.txt)", c.name));
            let mut seen = false;
            for &v in hm::MEASURED {
                let id = row.at(v).expect("measured");
                let lay = runs.at(v).expect("measured").or_else(|| after.and_then(|a| a.at(v).expect("measured")));
                match id {
                    Some(id) => {
                        assert_eq!(id, u64::from(c.cmd), "{} at {v}", c.name);
                        assert!(lay.is_some(), "{} exists at {v} but {} does not", c.name, runs.name);
                        seen = true;
                    }
                    None => {}
                }
            }
            assert!(seen, "{} is absent at every tag", c.name);
        }
    }

    /// Ids are unique, so a lookup is unambiguous.
    #[test]
    fn host_control_ids_are_unique() {
        let mut ids: Vec<u32> = HOST_CONTROLS.iter().map(|c| c.cmd).collect();
        ids.sort_unstable();
        let n = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), n);
    }

    /// An unlisted control passes through only where the old contract holds; elsewhere it is refused.
    #[test]
    fn an_unlisted_control_is_refused_outside_the_encoded_interval() {
        assert!(host("580.159.04").control_carry(0x2080_dead).expect("bench").is_none());
        assert!(host("580.65.06").control_carry(0x2080_dead).expect("interval").is_none());
        assert!(matches!(host("575.57.08").control_carry(0x2080_dead), Err(HostAbiError::UnlistedControl { .. })));
        assert!(matches!(host("575.57.08").control_carry(0x2080_a026), Err(HostAbiError::NoHeader { .. })));
        assert!(host("580.126.09").control_carry(0x2080_a026).expect("interval").is_none());
    }

    /// ★ The MSENC caps query was malformed on hosts the old gate accepted: 8 bytes with
    /// `instanceId` at +4 on 580.65.06 / 580.82.07, where the encoder writes 12 bytes, +8.
    #[test]
    fn msenc_caps_is_carried_on_early_580_hosts() {
        let h = host("580.65.06");
        let c = h.control_carry(0x0080_1b02).expect("listed").expect("carried");
        assert_eq!(c.host_size(), 8);
        let mut body = vec![0u8; 12];
        body[8] = 1; // instanceId = 1 at the bench offset
        let out = h.carry_out("NV0080_CTRL_MSENC_GET_CAPS_V2_PARAMS", &c, &body).expect("carried");
        assert_eq!(out, vec![0, 0, 0, 0, 1, 0, 0, 0]);
        // …and at 610 the struct is NV0080_CTRL_NVENC_GET_CAPS_V2_PARAMS, same bytes as the bench's.
        let c610 = host("610.57.04").control_carry(0x0080_1b02).expect("listed").expect("carried");
        assert_eq!(c610.host_size(), 12);
    }

    #[test]
    fn an_unmeasured_host_is_refused_by_name() {
        let r = HostAbi::for_host(HostDriverVersion::parse("580.142").expect("version"));
        assert!(matches!(r, Err(HostAbiError::Unmeasured { .. })), "{r:?}");
    }

    #[test]
    fn the_bench_host_passes_every_struct_through() {
        let h = host("580.159.04");
        assert!(h.is_bench());
        for runs in m::ALL_STRUCTS {
            if let Ok(c) = h.carry(runs) {
                assert!(matches!(c, Carry::Same { .. }), "{} is carried at the bench", runs.name);
            }
        }
    }

    /// `NVOS46` at 575: 56 bytes, `dmaOffset` at +40 where the bench has +48. A plain map carries;
    /// a kind override (a field 575 lacks) is refused by name, never sent as `dmaOffset`'s bytes.
    #[test]
    fn nvos46_carries_to_a_575_host_and_refuses_a_kind_override() {
        let h = host("575.57.08");
        let runs = &m::NVOS46_PARAMETERS;
        let b = Resolved::of(runs, crate::versions::BENCH_DRIVER).expect("bench");
        let mut body = vec![0u8; b.size()];
        let put = |body: &mut Vec<u8>, p: &'static str, v: u64, w: usize| {
            let o = b.need(p).expect(p).off();
            body[o..o + w].copy_from_slice(&v.to_le_bytes()[..w]);
        };
        put(&mut body, "hClient", 0xc1e0_0001, 4);
        put(&mut body, "hMemory", 0xcafe_0004, 4);
        put(&mut body, "offset", 0x20_0000, 8);
        put(&mut body, "length", 0x1000, 8);
        put(&mut body, "flags", 0x10, 4);
        put(&mut body, "dmaOffset", 0x7f00_0000, 8);
        assert_eq!(h.host_size(runs).expect("size"), 56);
        let out = h.to_host(runs, &body).expect("carried");
        assert_eq!(out.len(), 56);
        assert_eq!(u64::from_le_bytes(out[40..48].try_into().expect("8")), 0x7f00_0000, "dmaOffset at +40 on 575");
        let back = h.from_host(runs, &out).expect("back");
        assert_eq!(back, body, "a plain map round-trips");
        put(&mut body, "kindOverride", 0xfe, 4);
        match h.to_host(runs, &body) {
            Err(HostAbiError::Unexpressible { fields, .. }) => assert_eq!(fields, vec!["kindOverride"]),
            other => panic!("expected Unexpressible, got {other:?}"),
        }
    }

    /// `GPFIFO_SCHEDULE` at ≤570 has no `bSkipEnable`: asking for it is refused, a plain enable carries.
    #[test]
    fn gpfifo_schedule_skip_enable_is_unexpressible_below_575() {
        let h = host("570.148.08");
        let runs = &m::NVA06C_CTRL_GPFIFO_SCHEDULE_PARAMS;
        assert_eq!(h.host_size(runs).expect("size"), 2);
        assert_eq!(h.to_host(runs, &[1, 0, 0]).expect("enable"), vec![1, 0]);
        assert!(matches!(h.to_host(runs, &[1, 0, 1]), Err(HostAbiError::Unexpressible { .. })));
    }

    /// `GET_CLASSLIST_V2` holds 100 classes at 555–575 and 200 at the bench: a host reply carries
    /// back into the larger bench array.
    #[test]
    fn a_smaller_host_classlist_reply_carries_back() {
        let h = host("575.57.08");
        let runs = &m::NV0080_CTRL_GPU_GET_CLASSLIST_V2_PARAMS;
        let hs = h.host_size(runs).expect("size");
        assert_eq!(hs, 404);
        let mut reply = vec![0u8; hs];
        reply[0..4].copy_from_slice(&3u32.to_le_bytes());
        for (i, c) in [0xc56fu32, 0xc6b5, 0xc7c0].iter().enumerate() {
            reply[4 + i * 4..8 + i * 4].copy_from_slice(&c.to_le_bytes());
        }
        let back = h.from_host(runs, &reply).expect("back");
        assert_eq!(back.len(), 804);
        assert_eq!(&back[..16], &reply[..16]);
    }

    /// `NV_CHANNEL_ALLOC_PARAMS` at 610 inserts `hHandleVASpace`: every later field moves by 4 and
    /// the carry lands them where 610 reads them.
    #[test]
    fn channel_alloc_params_carry_to_610() {
        let h = host("610.57.04");
        let runs = &m::NV_CHANNEL_ALLOC_PARAMS;
        let b = Resolved::of(runs, crate::versions::BENCH_DRIVER).expect("bench");
        let t = Resolved::of(runs, h.version()).expect("610");
        let mut body = vec![0u8; b.size()];
        let e = b.need("engineType").expect("engineType").off();
        body[e..e + 4].copy_from_slice(&0x13u32.to_le_bytes());
        let out = h.to_host(runs, &body).expect("carried");
        let te = t.need("engineType").expect("engineType").off();
        assert_ne!(e, te, "engineType moved at 610");
        assert_eq!(u32::from_le_bytes(out[te..te + 4].try_into().expect("4")), 0x13);
    }
}
