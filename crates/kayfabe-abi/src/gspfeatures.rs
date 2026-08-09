//! `NV2080_CTRL_CMD_GSP_GET_FEATURES` (`0x20803601`) — §14.35's wall, and **the first
//! control this port serves whose reply is a fact about the GUEST rather than about the
//! silicon**.
//!
//! # ★★★ Routing: it is ours, and the generated dispatch says so without an argument
//!
//! Flags are `0x40549` (`ogkm-580: src/nvidia/generated/g_subdevice_nvoc.c:9466`) =
//! `NO_GPUS_LOCK(0x1) | NON_PRIVILEGED(0x8) | ROUTE_TO_PHYSICAL(0x40) |
//! API_LOCK_READONLY(0x100) | CACHEABLE(0x400) | PHYSICAL_IMPLEMENTED_ON_VGPU_GUEST(0x40000)`.
//!
//! ⊘⊘ **§14.35 reached the right answer by a route this module does not need, and the
//! route is worth correcting because it is the one a reader would re-derive.** It argued
//! that `PHYSICAL_IMPLEMENTED_ON_VGPU_GUEST` makes `NVOC_EXPORTED_METHOD_DISABLED_BY_FLAG`
//! false, so *"a CPU-RM body **is** compiled in"*, and that the prologue's RPC then wins
//! over that body. The first half conflates **compiled into the image** with **installed
//! on this variant's vtable**, and the generated dispatch settles it outright
//! (`g_subdevice_nvoc.c:10711-10719`):
//!
//! ```text
//! if (RmVariantHal: VF)  __subdeviceCtrlCmdGspGetFeatures__ = subdeviceCtrlCmdGspGetFeatures_KERNEL;
//! else                   __subdeviceCtrlCmdGspGetFeatures__ = subdeviceCtrlCmdGspGetFeatures_92bfc3;
//! ```
//!
//! and `_92bfc3` is `{ NV_ASSERT_PRECOMP(0); return NV_ERR_NOT_SUPPORTED; }`
//! (`g_subdevice_nvoc.h:8017-8020`). A bare-metal GSP client is **not** the `VF` variant,
//! so the `bValid = NV_FALSE` body §14.35 quotes
//! (`subdevice_ctrl_gpu_kernel.c:3569-3578`) is not on our vtable at all — the guest's only
//! local body here is a refusal stub. ⇒ There is no race to adjudicate: `NV_OK` for this
//! command can only ever have come off the `ROUTE_TO_PHYSICAL` RPC, which makes it
//! unambiguously ours to serve. `[measured, boot `gt1434_373c145`]`
//! `unserviced fn 76 cmd 0x20803601` confirms it arrives here.
//!
//! ★ The correction *strengthens* the conclusion rather than weakening it, which is why it
//! is written down: the old argument depended on reading `resource.c`'s prologue ordering
//! correctly, and this one depends on nothing but the vtable assignment.
//!
//! # ★★★ `firmwareVersion` — where it comes from, and the two candidates that are WRONG
//!
//! A real GA106 answers `"580.159.04"`
//! (`traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:73`, the ASCII run at offset 6).
//! Three sources could produce that string on this bench, and **two of them are wrong**:
//!
//! | candidate | value | verdict |
//! |---|---|---|
//! | the **host** driver's version ([`crate::host_driver`]) | `580.159.04` on `vh` | ⊘ a different machine's fact; the product property is that the two need not agree |
//! | [`crate::versions::DriverAbiTable`]'s own `version()` | **`580.65.06`** | ⊘⊘ **contradicted** — see below |
//! | the guest's own `NV_VERSION_STRING`, off the wire | `580.159.04` | ★ this one |
//!
//! ⊘⊘ **The second is the trap, and it is the reading §14.35's own sentence invites.** That
//! section says to *"serve it from the guest `DriverVersion` the device already detects to
//! select its ABI table"* — and the value the device retains after that selection is the
//! **table row's** version, not the guest's. `[measured 2026-08-09 at `1998769`]`
//! `table_for(BENCH_DRIVER).version()` is `DriverVersion { major: 580, minor: 65, patch: 6 }`
//! while `BENCH_DRIVER` is `580.159.04`, because `table_for` picks *the newest entry `<=`
//! requested* and the newest 580 row is `580.65.06`. A port that served `table.version()`
//! here would have told a `580.159.04` guest that its GSP firmware is `580.65.06` — a
//! fidelity defect of exactly the kind §14.35 warned survives, planted by the sentence that
//! warned about it.
//!
//! ★★★ **The third candidate is not a choice at all — it is a measurement, and the guest
//! hands it to us unprompted.** `RmRpcSetGuestSystemInfo` copies `NV_VERSION_STRING` into
//! `rpc_set_guest_system_info_v.guestDriverVersion` (`ogkm-580: rpc.c:8724-8727`), and
//! `NV_VERSION_STRING` is `"580.159.04"` in the 580.159.04 tree
//! (`ogkm-580: src/common/inc/nvUnixVersion.h:7`) — byte-identical to what the real GA106
//! put in `firmwareVersion`. That is the physical truth this field reports: **GSP firmware
//! ships inside the driver package**, so a guest running version *V* loads version *V*'s
//! firmware, and this port *is* that firmware.
//!
//! ⇒ [`crate::guestsysinfo::decode_guest_driver_version`] reads it off fn 1, which the
//! guest sends during `kgspInitRm` long before any control, and this module echoes it back.
//! Nothing is formatted, projected or tabulated: no `{:02}` patch-padding rule has to be
//! invented (it would have had exactly two supporting points, `580.159.04` and
//! `610.43.02`), and no constant has to be kept in step with the guest.
//!
//! ★ It also **decides a question §14.35 recorded as undecidable on this bench**. Host and
//! guest are both `580.159.04` on `vh`, so no boot can separate those two — true, and
//! stated correctly there. But the *third* candidate is separated by `580.65.06 ≠
//! 580.159.04` without booting anything, and the source that removes the ambiguity
//! altogether is the wire.
//!
//! # ⚠ What a hostile guest gets out of this, stated rather than assumed
//!
//! The string is guest-controlled: it is a buffer the guest wrote. Three things bound it.
//!
//! 1. **It reaches nothing.** A repo-wide grep for `firmwareVersion` in `ogkm-580`'s
//!    `src/nvidia/src/` finds one hit and it is an unrelated HWBC struct
//!    (`client_resource.c:3426`). The field is report-only — `nvidia-smi` and libcuda
//!    display it. A guest that lies here lies to itself.
//! 2. **It is validated, not copied.** [`FirmwareVersion::parse`] takes printable ASCII
//!    only, non-empty, and at most [`FIRMWARE_VERSION_MAX`] bytes so the NUL fits. Anything
//!    else is a named refusal — ⊘ never a truncation, because a truncated version string is
//!    an *invented* version string and this port would be the one that invented it.
//! 3. **It cannot grow the reply.** The destination is a fixed `[u8; 64]` inside a
//!    72-byte struct; length is a type invariant rather than a check at the copy.
//!
//! # The other three fields
//!
//! - `gspFeatures = 1` — `NV2080_CTRL_GSP_GET_FEATURES_UVM_ENABLED_TRUE`
//!   (`ogkm-580: ctrl2080gsp.h:78-80`). Bit 1 (`VGPU_GSP_MIG_REFACTORING_ENABLED`, `:81-83`)
//!   is clear on the real part and this is not a MIG device.
//! - `bValid = 1` — and it is **truthful rather than copied**: the header defines it as
//!   *"RM is a GSP client with GPU support offloaded to GSP firmware"* (`:44-49`), which is
//!   precisely what this port arranges.
//! - `bDefaultGspRmGpu = 1` — GA106 is an Ampere GeForce part, on which GSP-RM is enabled
//!   by default.
//!
//! # ★★ This is the first served row [`crate::sticky`]'s branch (a) actually covers
//!
//! `CACHEABLE (0x400)` is set, so the guest may cache our answer permanently and later asks
//! never reach the wire. Every id this port served before §14.35 was outside that mask,
//! which is why the guard at the serve site had been unreachable. The decision that branch
//! forces — *"is this answer constant for the life of the device?"* — is **yes**, and for a
//! reason rather than by luck: three of the four fields are build-time constants, and the
//! fourth is latched from fn 1, which the guest sends exactly once per driver load and
//! before any control. A guest that reloads its driver re-sends fn 1 and gets a fresh
//! device-side latch.

/// `NV2080_CTRL_CMD_GSP_GET_FEATURES`
/// (`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080gsp.h:64`).
pub const NV2080_CTRL_CMD_GSP_GET_FEATURES: u32 = 0x2080_3601;

/// `NV2080_GSP_MAX_BUILD_VERSION_LENGTH` — the `firmwareVersion` array (`ctrl2080gsp.h:66`).
pub const NV2080_GSP_MAX_BUILD_VERSION_LENGTH: usize = 0x40;

/// The longest version string that fits, i.e. the array less its NUL terminator.
///
/// ⊘ Stated as *"the array minus one"* rather than as `63`, so that a driver that widens
/// the array moves this with it instead of leaving a literal behind.
pub const FIRMWARE_VERSION_MAX: usize = NV2080_GSP_MAX_BUILD_VERSION_LENGTH - 1;

/// `sizeof(NV2080_CTRL_GSP_GET_FEATURES_PARAMS)` — `NvU32 + NvBool + NvBool + NvU8[64]`
/// = 70 bytes of members, padded to the struct's 4-byte alignment
/// (`ogkm-580: ctrl2080gsp.h:70-75`).
///
/// `[measured 2026-08-09, real GA106 on 580.159.04]` the wire agrees:
/// `traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:73` declares `size=72` and carries
/// 72 bytes in each direction. The two are checked against each other, not both against a
/// literal, by `gsp_get_features.rs::the_id_and_the_size_are_one_statement`.
pub const GSP_GET_FEATURES_PARAMS_SIZE: usize = 72;

/// Byte offset of `gspFeatures`.
pub const GSP_FEATURES_OFF: usize = 0;

/// Byte offset of `bValid`.
pub const B_VALID_OFF: usize = 4;

/// Byte offset of `bDefaultGspRmGpu`.
pub const B_DEFAULT_GSP_RM_GPU_OFF: usize = 5;

/// Byte offset of `firmwareVersion`.
///
/// ⚠ Six, not eight: two `NvBool` are two `NvU8`, so the array starts on an odd-ish offset
/// with no padding before it. `[measured]` the real GA106's reply writes its ASCII run at
/// exactly 6 (`cuinit_ioctl_trace_real_ga106.txt:73`).
pub const FIRMWARE_VERSION_OFF: usize = 6;

/// `NV2080_CTRL_GSP_GET_FEATURES_UVM_ENABLED` — bit `0:0` (`ogkm-580: ctrl2080gsp.h:78-80`).
pub const UVM_ENABLED_BIT: u32 = 1 << 0;

/// `NV2080_CTRL_GSP_GET_FEATURES_VGPU_GSP_MIG_REFACTORING_ENABLED` — bit `1:1`
/// (`ogkm-580: ctrl2080gsp.h:81-83`).
pub const VGPU_GSP_MIG_REFACTORING_ENABLED_BIT: u32 = 1 << 1;

/// The `gspFeatures` bit mask, as named bits rather than a bare `u32`.
///
/// ★ A struct and not an integer for [`crate::smcmode`]'s reason: the header declares
/// exactly two code points, and a mask this port cannot name is a mask it did not measure.
/// [`GspFeatures::from_u32`] refuses unknown bits rather than carrying them, so a value
/// invented by a future driver surfaces as a refusal instead of as a silently-copied word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GspFeatures {
    /// `UVM_ENABLED`. `[measured]` `TRUE` on a real GA106.
    pub uvm_enabled: bool,
    /// `VGPU_GSP_MIG_REFACTORING_ENABLED`. `[measured]` `FALSE` on a real GA106, which is
    /// not a MIG part.
    pub vgpu_gsp_mig_refactoring_enabled: bool,
}

impl GspFeatures {
    /// Every bit this port models, so a gate can quantify over the encoding.
    pub const KNOWN_BITS: u32 = UVM_ENABLED_BIT | VGPU_GSP_MIG_REFACTORING_ENABLED_BIT;

    /// The mask a real GA106 reports — `[measured]`
    /// `cuinit_ioctl_trace_real_ga106.txt:73`, word at offset 0 is `0x00000001`.
    pub const GA106: Self = Self {
        uvm_enabled: true,
        vgpu_gsp_mig_refactoring_enabled: false,
    };

    /// The wire word.
    #[must_use]
    pub const fn as_u32(self) -> u32 {
        let mut w = 0;
        if self.uvm_enabled {
            w |= UVM_ENABLED_BIT;
        }
        if self.vgpu_gsp_mig_refactoring_enabled {
            w |= VGPU_GSP_MIG_REFACTORING_ENABLED_BIT;
        }
        w
    }

    /// Read a mask back, or `None` if it sets a bit this port cannot name.
    ///
    /// ⊘ Unknown bits are refused, not masked off. Dropping them would turn *"a driver
    /// declared a feature we have never heard of"* into *"the feature is absent"*, which is
    /// the shape that made the C oracle's empty rows wrong.
    #[must_use]
    pub const fn from_u32(word: u32) -> Option<Self> {
        if word & !Self::KNOWN_BITS != 0 {
            return None;
        }
        Some(Self {
            uvm_enabled: word & UVM_ENABLED_BIT != 0,
            vgpu_gsp_mig_refactoring_enabled: word & VGPU_GSP_MIG_REFACTORING_ENABLED_BIT != 0,
        })
    }
}

/// A validated GSP firmware build-version string, sized to the field it goes in.
///
/// ★ `Copy` and fixed-width on purpose: the only producer is a guest buffer, the only
/// consumer is a policy that must stay `Copy`, and making the length a **type invariant**
/// means the encoder has no bound to check and no way to truncate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FirmwareVersion {
    /// NUL-padded; `bytes[..len]` is the string.
    bytes: [u8; NV2080_GSP_MAX_BUILD_VERSION_LENGTH],
    len: usize,
}

/// Why a firmware-version string could not be accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirmwareVersionError {
    /// Zero-length. RM always writes something here; an empty string would be reported to
    /// the user as a GPU with no firmware version at all.
    Empty,
    /// Longer than [`FIRMWARE_VERSION_MAX`], so it cannot be stored NUL-terminated.
    ///
    /// ⊘ Refused rather than truncated: a truncated version string is a version string this
    /// port invented, and it would be indistinguishable from one a driver really reported.
    TooLong {
        /// How many bytes arrived.
        got: usize,
    },
    /// A byte outside printable ASCII (`0x20..=0x7e`).
    ///
    /// ⚠ Named with its position. The value is displayed to a user by `nvidia-smi`, so
    /// control bytes and non-UTF-8 are refused at the boundary rather than passed through.
    NotPrintableAscii {
        /// Index of the first offending byte.
        at: usize,
        /// The byte.
        byte: u8,
    },
}

impl core::fmt::Display for FirmwareVersionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Empty => write!(
                f,
                "an empty GSP firmware version would report a GPU with no firmware at all"
            ),
            Self::TooLong { got } => write!(
                f,
                "a GSP firmware version of {got} bytes does not fit in \
                 {FIRMWARE_VERSION_MAX} plus a NUL; truncating would invent a version"
            ),
            Self::NotPrintableAscii { at, byte } => write!(
                f,
                "byte {at} of the GSP firmware version is {byte:#04x}, which is not \
                 printable ASCII, and this string is displayed to a user"
            ),
        }
    }
}

impl core::error::Error for FirmwareVersionError {}

impl FirmwareVersion {
    /// Validate a string into the field's width.
    ///
    /// # Errors
    ///
    /// [`FirmwareVersionError::Empty`], [`FirmwareVersionError::TooLong`] or
    /// [`FirmwareVersionError::NotPrintableAscii`] — see each variant for why the answer is
    /// a refusal rather than a repair.
    pub const fn parse(s: &str) -> Result<Self, FirmwareVersionError> {
        let src = s.as_bytes();
        if src.is_empty() {
            return Err(FirmwareVersionError::Empty);
        }
        if src.len() > FIRMWARE_VERSION_MAX {
            return Err(FirmwareVersionError::TooLong { got: src.len() });
        }
        let mut bytes = [0u8; NV2080_GSP_MAX_BUILD_VERSION_LENGTH];
        let mut i = 0;
        while i < src.len() {
            let b = src[i];
            if b < 0x20 || b > 0x7e {
                return Err(FirmwareVersionError::NotPrintableAscii { at: i, byte: b });
            }
            bytes[i] = b;
            i += 1;
        }
        Ok(Self {
            bytes,
            len: src.len(),
        })
    }

    /// The string, without its NUL padding.
    #[must_use]
    pub fn as_str(&self) -> &str {
        // Every byte was checked printable ASCII by `parse`, the only constructor.
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }

    /// The whole `firmwareVersion` array as it goes on the wire — NUL-padded to
    /// [`NV2080_GSP_MAX_BUILD_VERSION_LENGTH`].
    #[must_use]
    pub const fn wire(&self) -> &[u8; NV2080_GSP_MAX_BUILD_VERSION_LENGTH] {
        &self.bytes
    }
}

impl core::fmt::Display for FirmwareVersion {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Encode the `[OUT]` reply.
///
/// ## The transport question — does the caller read its own params back?
///
/// Yes, and this is the one control where the answer is *only* the body: the whole point of
/// the command is the four fields. libcuda issues it on its subdevice and reads the struct
/// `rpcRmApiControl_GSP`'s copyout wrote (`ogkm-580: rpc.c:11085-11090`), the same shape as
/// [`crate::smcmode`] and unlike [`crate::l2evict`].
///
/// ⊘ The request is **not** read and not echoed. `[measured 2026-08-09, real GA106 on
/// 580.159.04]` `cuinit_ioctl_trace_real_ga106.txt:73` shows all 72 request bytes zero —
/// asserted, not assumed, by
/// `gsp_get_features.rs::libcudas_own_request_gets_the_real_ga106s_own_reply` — and every member
/// of the struct is `[out]` per the header's own comment block (`ctrl2080gsp.h:38-57`), so
/// there is nothing to preserve — the reply is constructed, like
/// [`crate::cecaps::encode_ce_get_all_physical_caps`].
#[must_use]
pub fn encode_gsp_get_features(
    features: GspFeatures,
    valid: bool,
    default_gsp_rm_gpu: bool,
    firmware: &FirmwareVersion,
) -> Vec<u8> {
    let mut body = vec![0u8; GSP_GET_FEATURES_PARAMS_SIZE];
    body[GSP_FEATURES_OFF..GSP_FEATURES_OFF + 4].copy_from_slice(&features.as_u32().to_le_bytes());
    body[B_VALID_OFF] = u8::from(valid);
    body[B_DEFAULT_GSP_RM_GPU_OFF] = u8::from(default_gsp_rm_gpu);
    body[FIRMWARE_VERSION_OFF..FIRMWARE_VERSION_OFF + NV2080_GSP_MAX_BUILD_VERSION_LENGTH]
        .copy_from_slice(firmware.wire());
    body
}

/// Everything a `GSP_GET_FEATURES` reply says, decoded — for tests and the trace
/// differential.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GspGetFeaturesReply {
    /// The feature mask.
    pub features: GspFeatures,
    /// `bValid` — whether the mask means anything.
    pub valid: bool,
    /// `bDefaultGspRmGpu`.
    pub default_gsp_rm_gpu: bool,
    /// The firmware build version.
    pub firmware: FirmwareVersion,
}

/// Why a reply body could not be decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GspGetFeaturesError {
    /// Fewer than [`GSP_GET_FEATURES_PARAMS_SIZE`] bytes.
    ShortBody {
        /// What arrived.
        got: usize,
    },
    /// The mask sets a bit this port cannot name — see [`GspFeatures::from_u32`].
    UnknownFeatureBits {
        /// The whole word, so the caller can see which.
        word: u32,
    },
    /// An `NvBool` that is neither 0 nor 1. ⚠ Refused rather than coerced: RM writes
    /// `NV_TRUE`/`NV_FALSE` and nothing else, so a third value means this is not the struct
    /// we think it is.
    NotABool {
        /// Which offset.
        at: usize,
        /// The byte.
        byte: u8,
    },
    /// The version array is not a NUL-terminated printable-ASCII string.
    BadFirmwareVersion(FirmwareVersionError),
    /// The version array has no NUL at all, so it declares no end.
    UnterminatedFirmwareVersion,
}

/// Read a reply body back — the inverse of [`encode_gsp_get_features`].
///
/// # Errors
///
/// See [`GspGetFeaturesError`]; every arm is a refusal rather than a repair, for the reason
/// [`crate::smcmode::decode_smc_mode`] states.
pub fn decode_gsp_get_features(params: &[u8]) -> Result<GspGetFeaturesReply, GspGetFeaturesError> {
    if params.len() < GSP_GET_FEATURES_PARAMS_SIZE {
        return Err(GspGetFeaturesError::ShortBody { got: params.len() });
    }
    let word = u32::from_le_bytes([
        params[GSP_FEATURES_OFF],
        params[GSP_FEATURES_OFF + 1],
        params[GSP_FEATURES_OFF + 2],
        params[GSP_FEATURES_OFF + 3],
    ]);
    let features =
        GspFeatures::from_u32(word).ok_or(GspGetFeaturesError::UnknownFeatureBits { word })?;
    let boolean = |at: usize| match params[at] {
        0 => Ok(false),
        1 => Ok(true),
        byte => Err(GspGetFeaturesError::NotABool { at, byte }),
    };
    let valid = boolean(B_VALID_OFF)?;
    let default_gsp_rm_gpu = boolean(B_DEFAULT_GSP_RM_GPU_OFF)?;
    let arr =
        &params[FIRMWARE_VERSION_OFF..FIRMWARE_VERSION_OFF + NV2080_GSP_MAX_BUILD_VERSION_LENGTH];
    let end = arr
        .iter()
        .position(|&b| b == 0)
        .ok_or(GspGetFeaturesError::UnterminatedFirmwareVersion)?;
    let text = core::str::from_utf8(&arr[..end]).map_err(|_| {
        GspGetFeaturesError::BadFirmwareVersion(FirmwareVersionError::NotPrintableAscii {
            at: 0,
            byte: arr[0],
        })
    })?;
    let firmware = FirmwareVersion::parse(text).map_err(GspGetFeaturesError::BadFirmwareVersion)?;
    Ok(GspGetFeaturesReply {
        features,
        valid,
        default_gsp_rm_gpu,
        firmware,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real GA106's whole 72-byte reply, transcribed from the raw `out=` hex of
    /// `traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:73` rather than from a
    /// deserialised reading of it.
    const REAL_GA106_REPLY: [u8; GSP_GET_FEATURES_PARAMS_SIZE] = {
        let mut b = [0u8; GSP_GET_FEATURES_PARAMS_SIZE];
        b[0] = 0x01;
        b[4] = 0x01;
        b[5] = 0x01;
        // "580.159.04"
        b[6] = b'5';
        b[7] = b'8';
        b[8] = b'0';
        b[9] = b'.';
        b[10] = b'1';
        b[11] = b'5';
        b[12] = b'9';
        b[13] = b'.';
        b[14] = b'0';
        b[15] = b'4';
        b
    };

    #[test]
    fn the_encoder_reproduces_the_real_ga106_reply_byte_for_byte() {
        let fw = FirmwareVersion::parse("580.159.04").expect("a real version parses");
        let body = encode_gsp_get_features(GspFeatures::GA106, true, true, &fw);
        assert_eq!(body.len(), GSP_GET_FEATURES_PARAMS_SIZE);
        assert_eq!(
            body.as_slice(),
            REAL_GA106_REPLY.as_slice(),
            "the reply must match what a real GA106 put on the wire"
        );
    }

    #[test]
    fn the_round_trip_holds_and_names_every_field() {
        let fw = FirmwareVersion::parse("580.159.04").expect("parses");
        let body = encode_gsp_get_features(GspFeatures::GA106, true, true, &fw);
        let back = decode_gsp_get_features(&body).expect("our own encoding decodes");
        assert_eq!(back.features, GspFeatures::GA106);
        assert!(back.valid);
        assert!(back.default_gsp_rm_gpu);
        assert_eq!(back.firmware.as_str(), "580.159.04");
        // ★ And it decodes the HARDWARE bytes, not merely our own — the falsifier for the
        // claim that our encoding is the observed one.
        let real = decode_gsp_get_features(&REAL_GA106_REPLY).expect("hardware decodes");
        assert_eq!(real, back);
    }

    #[test]
    fn the_offsets_are_the_ones_hardware_wrote_and_not_a_padded_guess() {
        // ⚠ The obvious wrong layout is `firmwareVersion` at 8, from assuming the two
        // `NvBool` pad out to a word. Hardware wrote ASCII at 6, so this pins it.
        assert_eq!(FIRMWARE_VERSION_OFF, 6);
        assert_eq!(REAL_GA106_REPLY[FIRMWARE_VERSION_OFF], b'5');
        assert_ne!(REAL_GA106_REPLY[8], b'5', "8 would be the padded reading");
        // 4 + 1 + 1 + 64 = 70, and the struct's alignment rounds it to 72.
        assert_eq!(
            FIRMWARE_VERSION_OFF + NV2080_GSP_MAX_BUILD_VERSION_LENGTH,
            70
        );
        assert_eq!(GSP_GET_FEATURES_PARAMS_SIZE, 72);
    }

    #[test]
    fn an_over_long_version_is_refused_rather_than_truncated() {
        // ⊘ The whole point: 64 bytes of "5" truncated to 63 would be a version string this
        // port invented, and nothing downstream could tell it from a real one.
        let long = "5".repeat(NV2080_GSP_MAX_BUILD_VERSION_LENGTH);
        assert_eq!(
            FirmwareVersion::parse(&long),
            Err(FirmwareVersionError::TooLong {
                got: NV2080_GSP_MAX_BUILD_VERSION_LENGTH
            })
        );
        // ...and exactly the maximum is accepted, so the boundary is off-by-one-proof.
        let max = "5".repeat(FIRMWARE_VERSION_MAX);
        let v = FirmwareVersion::parse(&max).expect("the maximum fits");
        assert_eq!(v.as_str().len(), FIRMWARE_VERSION_MAX);
        // The NUL is still there, which is what makes the length safe.
        assert_eq!(v.wire()[FIRMWARE_VERSION_MAX], 0);
    }

    #[test]
    fn a_hostile_guest_string_is_refused_at_the_boundary() {
        // The producer is a guest buffer, so these are the real inputs.
        assert_eq!(FirmwareVersion::parse(""), Err(FirmwareVersionError::Empty));
        assert_eq!(
            FirmwareVersion::parse("580\n159"),
            Err(FirmwareVersionError::NotPrintableAscii { at: 3, byte: 0x0a })
        );
        assert_eq!(
            FirmwareVersion::parse("580\u{7f}"),
            Err(FirmwareVersionError::NotPrintableAscii { at: 3, byte: 0x7f })
        );
        // ★ A NUL mid-string is caught by the same rule rather than by a special case —
        // it would otherwise truncate the wire form silently.
        assert_eq!(
            FirmwareVersion::parse("580\u{0}159"),
            Err(FirmwareVersionError::NotPrintableAscii { at: 3, byte: 0x00 })
        );
    }

    #[test]
    fn an_unnameable_feature_bit_is_refused_rather_than_dropped() {
        assert_eq!(
            GspFeatures::from_u32(0),
            Some(GspFeatures {
                uvm_enabled: false,
                vgpu_gsp_mig_refactoring_enabled: false
            })
        );
        assert_eq!(GspFeatures::from_u32(1), Some(GspFeatures::GA106));
        assert_eq!(
            GspFeatures::from_u32(3),
            Some(GspFeatures {
                uvm_enabled: true,
                vgpu_gsp_mig_refactoring_enabled: true
            })
        );
        // ⊘ Bit 2 is not a code point the header declares.
        assert_eq!(GspFeatures::from_u32(4), None);
        assert_eq!(GspFeatures::from_u32(0xCDCD_CDCD), None);
        // ...and the decoder surfaces that as a named error rather than as a mask.
        let mut body = REAL_GA106_REPLY;
        body[0] = 0x04;
        assert_eq!(
            decode_gsp_get_features(&body),
            Err(GspGetFeaturesError::UnknownFeatureBits { word: 4 })
        );
    }

    #[test]
    fn a_third_boolean_value_is_refused() {
        // RM writes NV_TRUE/NV_FALSE; anything else means this is not the struct we think.
        for at in [B_VALID_OFF, B_DEFAULT_GSP_RM_GPU_OFF] {
            let mut body = REAL_GA106_REPLY;
            body[at] = 0xCD;
            assert_eq!(
                decode_gsp_get_features(&body),
                Err(GspGetFeaturesError::NotABool { at, byte: 0xCD })
            );
        }
    }

    #[test]
    fn a_short_body_is_refused_rather_than_zero_extended() {
        for n in [0, 1, 8, GSP_GET_FEATURES_PARAMS_SIZE - 1] {
            assert_eq!(
                decode_gsp_get_features(&REAL_GA106_REPLY[..n]),
                Err(GspGetFeaturesError::ShortBody { got: n })
            );
        }
    }

    #[test]
    fn the_abi_tables_version_is_not_the_guests_version_and_serving_it_would_be_wrong() {
        // ★★★ The falsifier for this module's central sourcing claim, and the measurement
        // that refuted §14.35's own instruction to *"serve it from the guest `DriverVersion`
        // the device already detects to select its ABI table"*. What the device retains
        // after that selection is the TABLE ROW, and `table_for` picks the newest entry
        // `<=` requested — so the row's version is not the guest's.
        use crate::DriverAbi;
        let table = crate::versions::table_for(crate::versions::BENCH_DRIVER)
            .expect("the bench driver is supported");
        assert_ne!(
            table.version(),
            crate::versions::BENCH_DRIVER,
            "if these ever coincide, this test stops protecting anything and the reason \
             the wire is the source has to be re-argued rather than assumed"
        );
        // And concretely, so the failure message names the defect rather than a mismatch.
        let served_if_wrong = format!(
            "{}.{}.{:02}",
            table.version().major,
            table.version().minor,
            table.version().patch
        );
        assert_eq!(served_if_wrong, "580.65.06");
        assert_ne!(
            served_if_wrong, "580.159.04",
            "a real GA106 answers 580.159.04; serving the table row would have told a \
             580.159.04 guest its GSP firmware is 580.65.06"
        );
    }

    #[test]
    fn the_guests_own_fn1_string_is_what_hardware_reports() {
        // ★ The positive half: `NV_VERSION_STRING` (ogkm-580 nvUnixVersion.h:7) is what RM
        // copies into `guestDriverVersion` (rpc.c:8724-8727), and it is byte-identical to
        // the `firmwareVersion` a real GA106 returned. This encodes the ONE relation this
        // rung rests on, so a future reader can see it is an identity and not a fit.
        let from_wire = "580.159.04";
        let fw = FirmwareVersion::parse(from_wire).expect("parses");
        let body = encode_gsp_get_features(GspFeatures::GA106, true, true, &fw);
        let real = decode_gsp_get_features(&REAL_GA106_REPLY).expect("hardware decodes");
        assert_eq!(real.firmware.as_str(), from_wire);
        assert_eq!(body, REAL_GA106_REPLY.to_vec());
    }

    #[test]
    fn an_unterminated_version_array_is_refused() {
        // ⚠ Without the NUL the array declares no end, and a decoder that took all 64 bytes
        // would report a version with trailing garbage as if the driver had sent it.
        let fw = FirmwareVersion::parse("x").expect("parses");
        let mut body = encode_gsp_get_features(GspFeatures::GA106, true, true, &fw);
        for b in &mut body[FIRMWARE_VERSION_OFF..] {
            *b = b'5';
        }
        assert_eq!(
            decode_gsp_get_features(&body),
            Err(GspGetFeaturesError::UnterminatedFirmwareVersion)
        );
    }
}
