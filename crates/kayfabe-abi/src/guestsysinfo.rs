//! `SET_GUEST_SYSTEM_INFO` (fn 1) — the **version handshake**, and the first thing the
//! guest's RM does once its GSP is up.
//!
//! ## ★★★ Why an echo used to be enough here, and why that is exactly the problem
//!
//! The guest fills `vgxVersionMajorNum`/`vgxVersionMinorNum` with its **own** constants and
//! then reads the same two fields back **out of the reply** — the reply's, not the
//! request's — and feeds them to `rpcSetIpVersion`, which selects the RPC function table
//! every later message is encoded against (`ogkm-580:
//! src/nvidia/src/kernel/vgpu/rpc.c:8760-8828`). An echoing GSP therefore *always* agrees,
//! whatever the guest said, and the handshake becomes a mirror: a device that speaks no
//! version at all passes it, and the disagreement surfaces hundreds of messages later as a
//! struct read at the wrong offsets.
//!
//! So this is a control where "advertise only what we can serve" has teeth. The port
//! answers with **its own** version out of [`crate::versions::DriverAbiTable`], and a guest
//! that declared a different one is refused by name rather than agreed with.
//!
//! ## ★★ The two version constants are an Axis-A seam, and they really do move
//!
//! | tag | `VGX_MAJOR` | `VGX_MINOR` |
//! |---|---|---|
//! | `ogkm-580: src/nvidia/inc/kernel/vgpu/vgpu_version.h:33-34` | `0x2B` | `0x13` |
//! | `ogkm-610: src/nvidia/inc/kernel/vgpu/vgpu_version.h:33-34` | `0x2E` | `0x0D` |
//!
//! ★ Cross-checked, not read once: 610's own table names `VGX_*_VERSION_NUMBER_VGPU_19_0 =
//! 0x2B / 0x13` (`ogkm-610: vgpu_version.h:41-42`), i.e. 610 agrees that 580's pair is the
//! vGPU-19.0 pair. Two independent statements of the same fact in two trees.
//!
//! ⊘ A driver version this port has no citation for gets **`None`**, and the policy refuses.
//! Inventing a pair would be the mirror again, with extra steps.
//!
//! ## ★ What the reply body carries, and what it deliberately does not
//!
//! Only the two version words, in an otherwise **zeroed** body. `RmRpcSetGuestSystemInfo`
//! reads `rpc_result_private` off the envelope and those two fields off the body, and then
//! nothing else — `guestDriverVersion`, `guestVersion`, `guestTitle` and `guestClNum` are
//! `[IN]`, three 256-byte strings the guest sent us (`ogkm-580:
//! src/nvidia/generated/g_rpc-structures.h:36-47`). The reply is **authored**, not
//! reflected — the rule task #127 imposed, and `kayfabe_gsp::GspFsm::answer` carries the
//! run behind it (`t126b` at `f2acb89`, a guest-kernel page fault, twice).
//!
//! ## ⊘ The down-negotiation branch is NOT built
//!
//! The protocol has one: a GSP that speaks a different version answers `rpc_result_private
//! != NV_OK` **with its own pair in the body**, and the guest either retries at that pair
//! or reports *"the host version is too old"* (`ogkm-580: rpc.c:8765-8801`). That is
//! strictly more useful than a refusal, and it is not built because nothing has been
//! observed to need it — this port answers exactly one guest driver version. If a
//! mismatched guest ever appears, the refusal names it, and *that* is when the branch gets
//! written against a real observation instead of a reading.

/// `sizeof(rpc_set_guest_system_info_v03_00)` — six `NvU32` then three `char[0x100]`
/// (`ogkm-580:`/`ogkm-610: src/nvidia/generated/g_rpc-structures.h:36-47`, identical at
/// both tags). No alignment hole: every member is 4-byte aligned or a byte array.
pub const SET_GUEST_SYSTEM_INFO_SIZE: usize = 6 * 4 + 3 * 0x100;

/// Byte offset of `vgxVersionMajorNum` — the first member.
pub const VGX_MAJOR_OFF: usize = 0;

/// Byte offset of `vgxVersionMinorNum`.
pub const VGX_MINOR_OFF: usize = 4;

/// Byte offset of `guestDriverVersion` — after the six `NvU32`
/// (`ogkm-580:`/`ogkm-610: g_rpc-structures.h:36-47`).
pub const GUEST_DRIVER_VERSION_OFF: usize = 6 * 4;

/// `sizeof(guestDriverVersion)` — `char[0x100]` (same citation).
pub const GUEST_STRING_LEN: usize = 0x100;

/// ★★★ The guest driver's own `NV_VERSION_STRING`, which it sends **unprompted** in this
/// message and which is the only measured source for
/// [`crate::gspfeatures`]'s `firmwareVersion`.
///
/// `RmRpcSetGuestSystemInfo` copies `NV_VERSION_STRING` in verbatim
/// (`ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:8724-8727`), and that macro is
/// `"580.159.04"` in the 580.159.04 tree (`ogkm-580: src/common/inc/nvUnixVersion.h:7`) —
/// byte-identical to the `firmwareVersion` a real GA106 returns from
/// `NV2080_CTRL_CMD_GSP_GET_FEATURES` (`traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:73`).
/// GSP firmware ships inside the driver package, so the two are the same string by
/// construction rather than by coincidence.
///
/// ⊘ The bytes are the **guest's**, so nothing here trusts them: the value is returned as a
/// borrowed `&str` and the only consumer validates it into a bounded type
/// ([`crate::gspfeatures::FirmwareVersion::parse`]). This function's own contract is
/// narrow — find a NUL and decode UTF-8 — and it refuses rather than repairing either.
///
/// # Errors
///
/// [`GuestSystemInfoError::Truncated`] if the payload cannot hold the struct;
/// [`GuestSystemInfoError::DriverVersionUnterminated`] if the array carries no NUL;
/// [`GuestSystemInfoError::DriverVersionNotUtf8`] otherwise.
pub fn decode_guest_driver_version(payload: &[u8]) -> Result<&str, GuestSystemInfoError> {
    if payload.len() < SET_GUEST_SYSTEM_INFO_SIZE {
        return Err(GuestSystemInfoError::Truncated {
            need: SET_GUEST_SYSTEM_INFO_SIZE,
            got: payload.len(),
        });
    }
    let arr = &payload[GUEST_DRIVER_VERSION_OFF..GUEST_DRIVER_VERSION_OFF + GUEST_STRING_LEN];
    let end = arr
        .iter()
        .position(|&b| b == 0)
        .ok_or(GuestSystemInfoError::DriverVersionUnterminated)?;
    core::str::from_utf8(&arr[..end]).map_err(|_| GuestSystemInfoError::DriverVersionNotUtf8)
}

/// The vGPU RPC version a driver speaks.
///
/// Two `NvU32` on the wire even though both values fit in a byte, because
/// `RPC_VERSION_FROM_VGX_VERSION` packs them into bit fields 31:24 and 23:16 only *after*
/// they cross (`ogkm-580: vgpu_version.h:28-32`) — the wire form is two whole words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VgxVersion {
    /// `VGX_MAJOR_VERSION_NUMBER`.
    pub major: u32,
    /// `VGX_MINOR_VERSION_NUMBER`.
    pub minor: u32,
}

/// Why a `SET_GUEST_SYSTEM_INFO` could not be answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestSystemInfoError {
    /// The payload is shorter than the struct, so the two version words the guest claims
    /// to have sent are not there. Not a message this port models.
    Truncated {
        /// What the struct needs.
        need: usize,
        /// What arrived.
        got: usize,
    },
    /// This port has no `VGX_*_VERSION_NUMBER` citation for the guest's driver version, so
    /// it cannot state one. See this module's docs: agreeing anyway is the mirror.
    NoVersionForDriver,
    /// The guest declared a version this port does not speak. ⚠ The protocol's own answer
    /// is a down-negotiation, which is not built — see this module's docs.
    VersionMismatch {
        /// What the guest said.
        guest: VgxVersion,
        /// What this port speaks.
        ours: VgxVersion,
    },
    /// `guestDriverVersion` carries no NUL, so the array declares no end.
    ///
    /// ⊘ Refused rather than taking all 0x100 bytes: a version string with trailing
    /// garbage would be reported back to the guest's own users as its firmware version.
    DriverVersionUnterminated,
    /// `guestDriverVersion` is not UTF-8 up to its NUL.
    DriverVersionNotUtf8,
}

impl core::fmt::Display for GuestSystemInfoError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Truncated { need, got } => write!(
                f,
                "SET_GUEST_SYSTEM_INFO needs {need} bytes of payload and {got} arrived"
            ),
            Self::NoVersionForDriver => write!(
                f,
                "this port has no VGX version for the guest's driver, so it cannot state \
                 one; echoing the guest's would make the handshake a mirror"
            ),
            Self::VersionMismatch { guest, ours } => write!(
                f,
                "the guest speaks vGPU RPC {:#x}.{:#x} and this port speaks {:#x}.{:#x}; \
                 the down-negotiation branch is not built",
                guest.major, guest.minor, ours.major, ours.minor
            ),
            Self::DriverVersionUnterminated => write!(
                f,
                "the guest's guestDriverVersion[0x100] carries no NUL, so it declares no \
                 end; taking all of it would report a version with trailing garbage"
            ),
            Self::DriverVersionNotUtf8 => write!(
                f,
                "the guest's guestDriverVersion is not UTF-8 up to its NUL"
            ),
        }
    }
}

impl core::error::Error for GuestSystemInfoError {}

/// The version the guest declared in its request.
///
/// # Errors
///
/// [`GuestSystemInfoError::Truncated`].
pub fn decode_declared_vgx(payload: &[u8]) -> Result<VgxVersion, GuestSystemInfoError> {
    if payload.len() < SET_GUEST_SYSTEM_INFO_SIZE {
        return Err(GuestSystemInfoError::Truncated {
            need: SET_GUEST_SYSTEM_INFO_SIZE,
            got: payload.len(),
        });
    }
    let w = |at: usize| {
        u32::from_le_bytes([
            payload[at],
            payload[at + 1],
            payload[at + 2],
            payload[at + 3],
        ])
    };
    Ok(VgxVersion {
        major: w(VGX_MAJOR_OFF),
        minor: w(VGX_MINOR_OFF),
    })
}

/// Encode the reply body: a zeroed struct carrying only the version this port speaks.
///
/// The whole [`SET_GUEST_SYSTEM_INFO_SIZE`] is returned so the guest's own `[IN]` strings
/// are **overwritten with zeros** rather than handed back — the reply says what we answer
/// and nothing about what was asked.
#[must_use]
pub fn encode_set_guest_system_info_reply(ours: VgxVersion) -> Vec<u8> {
    let mut body = vec![0u8; SET_GUEST_SYSTEM_INFO_SIZE];
    body[VGX_MAJOR_OFF..VGX_MAJOR_OFF + 4].copy_from_slice(&ours.major.to_le_bytes());
    body[VGX_MINOR_OFF..VGX_MINOR_OFF + 4].copy_from_slice(&ours.minor.to_le_bytes());
    body
}
