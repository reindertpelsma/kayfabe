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
//! reflected — the rule task #127 imposed, and `kf_gsp::GspFsm::answer` carries the
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

/// Byte offset of `guestClNum` — the sixth `NvU32` (`ogkm-580: g_rpc-structures.h:36-47`; the
/// guest fills it with `NV_BUILD_CHANGELIST_NUM`, `src/nvidia/src/kernel/vgpu/rpc.c:8732` at 580.65.06).
pub const GUEST_CL_NUM_OFF: usize = 5 * 4;

/// Byte offset of `guestVersion` (`NV_BUILD_BRANCH_VERSION`, e.g. `rel/gpu_drv/r580/r580_78-179`
/// on Linux and `r580_78-7` in nvBldVer.h's Windows block at 580.65.06).
pub const GUEST_VERSION_OFF: usize = GUEST_DRIVER_VERSION_OFF + GUEST_STRING_LEN;

/// Byte offset of `guestTitle` (`NV_DISPLAY_DRIVER_TITLE`).
pub const GUEST_TITLE_OFF: usize = GUEST_VERSION_OFF + GUEST_STRING_LEN;

/// ★ What a guest says about itself at fn 1, decoded for the LOG only (2026-10-04, branch
/// `v3-windows`, runbook C3): the three `[IN]` strings, the changelist and the vGPU pair.
///
/// ⊘ Nothing decides on this. The strings are the guest's, so each is taken up to its NUL (or the
/// whole array), decoded lossily and escaped by [`GuestIdentity`]'s `Display`; the only decision
/// fn 1 makes stays [`decode_guest_driver_version`]'s strict parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestIdentity {
    /// `vgxVersionMajorNum` / `vgxVersionMinorNum`.
    pub vgx: VgxVersion,
    /// `guestClNum`.
    pub cl_num: u32,
    /// `guestDriverVersion` (`NV_VERSION_STRING`).
    pub driver_version: String,
    /// `guestVersion` (`NV_BUILD_BRANCH_VERSION`).
    pub version: String,
    /// `guestTitle` (`NV_DISPLAY_DRIVER_TITLE`).
    pub title: String,
}

impl GuestIdentity {
    /// Decode fn 1's identity fields.
    ///
    /// # Errors
    /// [`GuestSystemInfoError::Truncated`].
    pub fn decode(payload: &[u8]) -> Result<GuestIdentity, GuestSystemInfoError> {
        let vgx = decode_declared_vgx(payload)?;
        let text = |at: usize| {
            let arr = &payload[at..at + GUEST_STRING_LEN];
            let end = arr.iter().position(|&b| b == 0).unwrap_or(arr.len());
            String::from_utf8_lossy(&arr[..end]).into_owned()
        };
        let cl = &payload[GUEST_CL_NUM_OFF..GUEST_CL_NUM_OFF + 4];
        Ok(GuestIdentity {
            vgx,
            cl_num: u32::from_le_bytes([cl[0], cl[1], cl[2], cl[3]]),
            driver_version: text(GUEST_DRIVER_VERSION_OFF),
            version: text(GUEST_VERSION_OFF),
            title: text(GUEST_TITLE_OFF),
        })
    }
}

impl core::fmt::Display for GuestIdentity {
    /// One log line's worth; every string escaped (`escape_debug`), so a guest cannot write a line
    /// break or a terminal escape into the host log.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "guestDriverVersion=\"{}\" guestVersion=\"{}\" guestTitle=\"{}\" guestClNum={} vgx={:#x}.{:#x}",
            self.driver_version.escape_debug(),
            self.version.escape_debug(),
            self.title.escape_debug(),
            self.cl_num,
            self.vgx.major,
            self.vgx.minor
        )
    }
}

/// ★ What fn 1 says the guest IS, keyed the way this port keys tables (2026-10-04, branch
/// `v3-windows`, runbook C2; `docs/design/V3_WINDOWS_DISCOVERY.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportedDriver {
    /// `guestDriverVersion`, verbatim (strictly decoded: NUL-terminated UTF-8).
    pub said: String,
    /// The version the port keys on: the guest's own when it is a driver-matrix tag; else the Linux
    /// tag whose Windows build the guest's identity names ([`crate::windows_twin::linux_twin`]); else
    /// the guest's own as parsed (and refused later by the caller, by name).
    pub version: Option<crate::DriverVersion>,
    /// Set when [`ReportedDriver::version`] came from a Windows twin.
    pub twin: Option<&'static crate::generated::windows_twins::WindowsTwin>,
    /// Why a guest string that is not a tag was not accepted as a Windows twin (`None` when the
    /// string is a tag, or names no Windows build at all).
    pub twin_refusal: Option<crate::windows_twin::TwinRefusal>,
}

impl ReportedDriver {
    /// Decode fn 1's driver identity.
    ///
    /// # Errors
    /// [`decode_guest_driver_version`]'s.
    pub fn decode(payload: &[u8]) -> Result<ReportedDriver, GuestSystemInfoError> {
        let said = decode_guest_driver_version(payload)?.to_owned();
        let id = GuestIdentity::decode(payload)?;
        let parsed = crate::DriverVersion::parse(&said);
        if let Some(v) = parsed
            && crate::versions::table_for(v).is_ok()
        {
            return Ok(ReportedDriver {
                said,
                version: Some(v),
                twin: None,
                twin_refusal: None,
            });
        }
        match crate::windows_twin::linux_twin(&said, &id.version, id.cl_num) {
            Ok((v, t)) => Ok(ReportedDriver {
                said,
                version: Some(v),
                twin: Some(t),
                twin_refusal: None,
            }),
            Err(crate::windows_twin::TwinRefusal::NoTwin) => Ok(ReportedDriver {
                said,
                version: parsed,
                twin: None,
                twin_refusal: None,
            }),
            Err(r) => Ok(ReportedDriver {
                said,
                version: parsed,
                twin: None,
                twin_refusal: Some(r),
            }),
        }
    }
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

#[cfg(test)]
mod identity_tests {
    use super::*;

    /// ★ 2026-10-04 (v3-windows, runbook C3): fn 1's identity, decoded for the log. The values are
    /// nvBldVer.h's Windows block at ogkm 580.65.06 (`NV_BUILD_NAME "580.88"`, `r580_78-7`, CL
    /// 36308443); a guest string cannot break the log line.
    #[test]
    fn the_identity_decodes_every_field_and_escapes_the_strings() {
        let mut p = vec![0u8; SET_GUEST_SYSTEM_INFO_SIZE];
        p[VGX_MAJOR_OFF..VGX_MAJOR_OFF + 4].copy_from_slice(&0x2Bu32.to_le_bytes());
        p[VGX_MINOR_OFF..VGX_MINOR_OFF + 4].copy_from_slice(&0x13u32.to_le_bytes());
        p[GUEST_CL_NUM_OFF..GUEST_CL_NUM_OFF + 4].copy_from_slice(&36_308_443u32.to_le_bytes());
        p[GUEST_DRIVER_VERSION_OFF..GUEST_DRIVER_VERSION_OFF + 6].copy_from_slice(b"580.88");
        p[GUEST_VERSION_OFF..GUEST_VERSION_OFF + 9].copy_from_slice(b"r580_78-7");
        p[GUEST_TITLE_OFF..GUEST_TITLE_OFF + 5].copy_from_slice(b"a\nb\x1b[");
        let id = GuestIdentity::decode(&p).unwrap();
        assert_eq!(id.cl_num, 36_308_443);
        assert_eq!(id.driver_version, "580.88");
        assert_eq!(id.version, "r580_78-7");
        assert_eq!(
            id.vgx,
            VgxVersion {
                major: 0x2B,
                minor: 0x13
            }
        );
        let line = id.to_string();
        assert!(
            line.contains("guestDriverVersion=\"580.88\"") && line.contains("guestClNum=36308443"),
            "{line}"
        );
        assert!(!line.contains('\n') && !line.contains('\x1b'), "{line}");
        assert!(matches!(
            GuestIdentity::decode(&p[..100]),
            Err(GuestSystemInfoError::Truncated { .. })
        ));
    }

    fn fn1(version: &str, branch: &str, cl: u32) -> Vec<u8> {
        let mut p = vec![0u8; SET_GUEST_SYSTEM_INFO_SIZE];
        p[VGX_MAJOR_OFF..VGX_MAJOR_OFF + 4].copy_from_slice(&0x2Bu32.to_le_bytes());
        p[VGX_MINOR_OFF..VGX_MINOR_OFF + 4].copy_from_slice(&0x13u32.to_le_bytes());
        p[GUEST_CL_NUM_OFF..GUEST_CL_NUM_OFF + 4].copy_from_slice(&cl.to_le_bytes());
        p[GUEST_DRIVER_VERSION_OFF..GUEST_DRIVER_VERSION_OFF + version.len()]
            .copy_from_slice(version.as_bytes());
        p[GUEST_VERSION_OFF..GUEST_VERSION_OFF + branch.len()].copy_from_slice(branch.as_bytes());
        p
    }

    /// ★ The Windows 580.88 guest's fn 1 on `vwin` (2026-10-04, kf3 `b98bdbec`) is keyed as Linux
    /// 580.65.06; a Linux tag is keyed as itself; an unknown Windows branch is refused by name.
    #[test]
    fn a_windows_twin_is_keyed_as_its_linux_tag_and_a_tag_as_itself() {
        let w = ReportedDriver::decode(&fn1("580.88", "r580_78-7", 0)).unwrap();
        assert_eq!(w.said, "580.88");
        assert_eq!(w.version, crate::DriverVersion::parse("580.65.06"));
        assert_eq!(w.twin.map(|t| t.linux_tag), Some("580.65.06"));
        let l =
            ReportedDriver::decode(&fn1("580.65.06", "rel/gpu_drv/r580/r580_78-179", 0)).unwrap();
        assert_eq!(l.version, crate::DriverVersion::parse("580.65.06"));
        assert!(l.twin.is_none() && l.twin_refusal.is_none());
        let bad = ReportedDriver::decode(&fn1("580.88", "r580_78-9", 0)).unwrap();
        assert_eq!(bad.version, crate::DriverVersion::parse("580.88"));
        assert!(matches!(
            bad.twin_refusal,
            Some(crate::windows_twin::TwinRefusal::GuestBranch { .. })
        ));
    }
}
