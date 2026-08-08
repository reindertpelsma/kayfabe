//! `NV2080_CTRL_CMD_BUS_GET_PCIE_SUPPORTED_GPU_ATOMICS` (`0x2080182a`) — and ★★★ the
//! measurement that had to come **before** the value, because the instrument was the wall.
//!
//! ## ⊘⊘ The refutation this module exists to record
//!
//! `execution_plane_increments.md` §14.30 handed its successor a two-clause finding:
//!
//! > *"`rmladder --probe-ctrl 0x2080182a:112`, twice, on the real GA106: `refused
//! > Other(86)`, both times. So on the same physical part, in the same hour, **libcuda gets
//! > `NV_OK` and a bare Subdevice gets `0x56`.** The handler is a `_DISPATCH`, so the answer
//! > depends on caller state that `rmladder` does not reproduce."*
//!
//! ★ **The two callers never issued the same call.** `capType` is an **`[IN]`** field
//! (`ogkm-580: ctrl2080bus.h:1256-1258` and the struct at `:1311-1315`), and `probe_ctrl`
//! seeds *every* byte of the params with `0xCD` — so R18 asked for `capType = 0xCDCDCDCD`,
//! which is none of `_CAPTYPE_SYSMEM(0)` / `_GPU(1)` / `_P2P(2)` (`:1226-1228`). libcuda
//! hands RM a **zeroed** buffer, so it asks for `_CAPTYPE_SYSMEM`.
//!
//! `[measured 2026-08-08, real GA106 `GPU-d0913685`, driver 580.159.04 Open, `rmladder
//! --atomics-probe` (R23), rev `1d5704dd9`,
//! `traces/real_ga106/rmladder_r23_atomics_real_ga106.txt`]` — eight arms on the **same bare
//! Subdevice R18 used**, nothing else allocated:
//!
//! | arm | `capType` | tail seed | result |
//! |---|---|---|---|
//! | R18 replay | `0xCDCDCDCD` | `0xCD` | refused `0x56` — §14.30 reproduced |
//! | capType poisoned only | `0xCDCDCDCD` | `0x00` | refused `0x56` |
//! | ★ **SYSMEM, tail poisoned** | `0` | `0xCD` | **`NV_OK`, body WRITTEN** |
//! | libcuda replay | `0` | `0x00` | `NV_OK` (body indistinguishable — see below) |
//! | GPU | `1` | `0xCD` | refused `0x56` |
//! | P2P | `2` | `0xCD` | refused `0x56` |
//! | undeclared | `3` | `0xCD` | refused `0x56` |
//! | SYSMEM, `dbdf` poisoned | `0` | `0xCD` | `NV_OK`, `dbdf` echoed back `0xCDCDCDCD` |
//!
//! ⇒ **The refusal was the instrument's own sentinel sitting in an input field**, and no
//! caller state was involved: the bare Subdevice answers `NV_OK` the moment it asks the
//! question libcuda asks. The `0xCD` seed that lets R18 tell *written* from *unwritten* is
//! only sound on a **pure-`[OUT]`** struct; on a struct with an `[IN]` field it is an input
//! **mutation**, and the instrument perturbs the thing it measures.
//!
//! ⚠ The `_DISPATCH` was never the decider either. This control's flags are `0x40048` =
//! `NON_PRIVILEGED | ROUTE_TO_PHYSICAL | PHYSICAL_IMPLEMENTED_ON_VGPU_GUEST`
//! (`ogkm-580: g_subdevice_nvoc.c:6806-6819`, `rmapi/control.h:202-308`), so on a bare-metal
//! GSP client the kernel RM never runs the local arm at all — it RPCs the whole struct to
//! GSP-RM, which is why the boot ledger carries `unserviced fn 76 cmd 0x2080182a` exactly
//! once. The `_92bfc3` arm NVOC installs for every non-VF variant is a bare `return
//! NV_ERR_NOT_SUPPORTED` (`g_subdevice_nvoc.h:6999-7002`) that exists precisely **because it
//! should never run**. Reading a HAL suffix and inferring caller-dependence skipped the two
//! flags that say the HAL is bypassed.
//!
//! ## ★★★ The value, and why this zero is NOT the `dlen = 0` zero
//!
//! `[measured]` For `_CAPTYPE_SYSMEM` a real GA106 writes all thirteen entries as
//! `bSupported = 0x00, attributes = 0x00000000` — **into a buffer seeded `0xCD`**, which is
//! what makes it a positive reading rather than the committed trace's ambiguity. The
//! all-zero libcuda replay in `traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:48`
//! agrees, and could never have established it: `traces/real_ga106/README.md` says why
//! (*"libcuda hands RM zeroed buffers, so an all-zero pair is ambiguous"*). The seeded arm
//! is the one that decides.
//!
//! ★ Corroborated from a second, independent source in the same second: RM's **own** vGPU
//! guest implementation writes exactly this — `subdeviceCtrlCmdBusGetPcieSupportedGpuAtomics_VF`
//! loops all thirteen ops setting `bSupported = NV_FALSE, attributes = 0x0` under the comment
//! *"Atomics not supported in VF. See bug 3497203."* (`ogkm-580: kern_bus_ctrl.c:693-707`).
//! NVIDIA's answer for a **virtualized** GPU is this port's answer, arrived at independently.
//!
//! ⊘ And the shape that separates it from `c_oracle_empty_rows_are_wrong`: the `0x20802a08`
//! disaster was a zero **decoded out of an unmeasured row** that became a buffer size with a
//! hardware DMA writer downstream. This zero is measured, corroborated, and its failure
//! direction is conservative: `bSupported = FALSE` denies a capability, so the driver takes
//! its fallback path. A wrong `TRUE` would be the dangerous direction, and nothing here can
//! produce one.
//!
//! ## ⊘ What is refused, and why refusing is the measured behaviour
//!
//! `_CAPTYPE_GPU` and `_CAPTYPE_P2P` are **declared in the header and refused by the
//! hardware** — `0x56`, measured above. So [`answer_bus_get_pcie_supported_gpu_atomics`]
//! refuses every captype but `SYSMEM` **by name**, and that is not this port confessing
//! ignorance: it is this port reproducing a real GA106. ⊘ Answering `GPU`/`P2P` with
//! thirteen `FALSE`s would be a *stronger* claim than hardware makes, and the difference is
//! observable — `NV_OK` where a real part returns `NV_ERR_NOT_SUPPORTED`.
//!
//! ## Why there is no chip row here
//!
//! Whether a GPU atomic completes to coherent sysmem is a property of the **root complex's**
//! PCIe AtomicOp completer support as much as of the die, so it is `PCIE_GEN_INFO`'s species
//! and not `GPU_GEN`'s: no chip-family row may state it. What this port serves is a
//! statement about **the link this port presents** — `derive_what_you_cannot_query_then_oracle_it`
//! — identical on every host by construction, exactly as [`crate::businfo::PcieGenInfo::fully_trained`]
//! is. [`GpuAtomicOp::none_supported`] is that derivation, and it takes no chip argument.

extern crate alloc;
use alloc::vec::Vec;

/// `NV2080_CTRL_CMD_BUS_GET_PCIE_SUPPORTED_GPU_ATOMICS` (`ogkm-580: ctrl2080bus.h:1273`).
pub const NV2080_CTRL_CMD_BUS_GET_PCIE_SUPPORTED_GPU_ATOMICS: u32 = 0x2080_182a;

/// `NV2080_CTRL_PCIE_SUPPORTED_GPU_ATOMICS_OP_TYPE_COUNT` (`ogkm-580: ctrl2080bus.h:1289`).
pub const PCIE_SUPPORTED_GPU_ATOMICS_OP_TYPE_COUNT: usize = 13;

/// `sizeof(NV2080_CTRL_CMD_BUS_GET_PCIE_SUPPORTED_GPU_ATOMICS_PARAMS)` — `4 + 4 + 13 * 8`.
///
/// The per-op struct is `{NvBool bSupported; NvU32 attributes;}`, so the `NvU32` forces
/// 4-byte alignment and each entry is **8** bytes with three padding bytes after
/// `bSupported`. `[measured]` on the wire as `size=112` in both the real-GA106 and the guest
/// `cuInit` traces, and `[measured]` that real RM leaves those three padding bytes exactly
/// as they arrived — R23's seeded arm reads them back as `0xCD`.
pub const PCIE_SUPPORTED_GPU_ATOMICS_PARAMS_SIZE: usize =
    4 + 4 + 8 * PCIE_SUPPORTED_GPU_ATOMICS_OP_TYPE_COUNT;

/// `NV2080_CTRL_CMD_BUS_PCIE_ATOMICS_CAPTYPE_SYSMEM` (`ogkm-580: ctrl2080bus.h:1226`) — the
/// only captype a real GA106 answers, `[measured]`.
pub const CAPTYPE_SYSMEM: u32 = 0x0;
/// `..._CAPTYPE_GPU` (`:1227`). `[measured]` refused `0x56` on a real GA106.
pub const CAPTYPE_GPU: u32 = 0x1;
/// `..._CAPTYPE_P2P` (`:1228`). `[measured]` refused `0x56` on a real GA106.
pub const CAPTYPE_P2P: u32 = 0x2;

/// One `NV2080_CTRL_BUS_PCIE_GPU_ATOMIC_OP_INFO` (`ogkm-580: ctrl2080bus.h:1303-1306`).
///
/// ★ `supported` is a `bool` and not a `u32`: `bSupported` is an `NvBool`, RM writes exactly
/// one byte for it, and the three bytes after it are **padding RM does not own**. A `u32`
/// here would invite a port to write four.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuAtomicOp {
    /// `bSupported` — is the GPU atomic natively supported by the PCIe path.
    pub supported: bool,
    /// `attributes` — the `NV2080_CTRL_PCIE_SUPPORTED_GPU_ATOMICS_ATTRIB_*` mask
    /// (`ogkm-580: ctrl2080bus.h:1316-1338`). Meaningful only when `supported`.
    pub attributes: u32,
}

impl GpuAtomicOp {
    /// The link this port presents: **no PCIe GPU atomics**.
    ///
    /// `[measured 2026-08-08, real GA106, R23]` a real part answers `_CAPTYPE_SYSMEM` with
    /// exactly this, into a `0xCD`-seeded buffer. `[src]` RM's own vGPU guest arm writes
    /// exactly this too (`ogkm-580: kern_bus_ctrl.c:693-707`, *"Atomics not supported in
    /// VF"*).
    ///
    /// ⊘ Takes no chip argument **on purpose**: PCIe atomics to coherent sysmem need the
    /// root complex to be an AtomicOp completer, so this is a fact about a machine and a
    /// link, never about a die. See the module docs.
    #[must_use]
    pub fn none_supported() -> [GpuAtomicOp; PCIE_SUPPORTED_GPU_ATOMICS_OP_TYPE_COUNT] {
        [GpuAtomicOp {
            supported: false,
            attributes: 0,
        }; PCIE_SUPPORTED_GPU_ATOMICS_OP_TYPE_COUNT]
    }
}

/// Why a `BUS_GET_PCIE_SUPPORTED_GPU_ATOMICS` request could not be answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuAtomicsError {
    /// The params buffer is shorter than [`PCIE_SUPPORTED_GPU_ATOMICS_PARAMS_SIZE`].
    ShortParams {
        /// What arrived.
        len: usize,
        /// What the struct is.
        need: usize,
    },
    /// ★★★ A `capType` this port does not answer.
    ///
    /// ⊘ This is **not** an admission of ignorance, it is a reproduction: a real GA106
    /// refuses `_CAPTYPE_GPU`, `_CAPTYPE_P2P` and every undeclared value with `0x56`,
    /// `[measured]`. Answering them "all unsupported" would claim more than hardware does.
    UnansweredCapType {
        /// The `capType` the guest asked for.
        cap_type: u32,
    },
}

impl core::fmt::Display for GpuAtomicsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ShortParams { len, need } => {
                write!(f, "{len}-byte params, {need} needed for the struct")
            }
            Self::UnansweredCapType { cap_type } => write!(
                f,
                "PCIe atomics capType {cap_type:#x} is refused, as a real GA106 refuses every \
                 captype but SYSMEM(0) with NV_ERR_NOT_SUPPORTED; answering it \"all \
                 unsupported\" would be a stronger claim than the hardware makes"
            ),
        }
    }
}

impl core::error::Error for GpuAtomicsError {}

/// Answer a `BUS_GET_PCIE_SUPPORTED_GPU_ATOMICS` RPC: **the request, edited**.
///
/// `capType` and `dbdf` are echoed back untouched and the three padding bytes of every entry
/// are left exactly as they arrived — all three `[measured 2026-08-08, real GA106, R23]`:
/// the `dbdf`-poisoned arm reads `0xCDCDCDCD` back out, and every entry's padding survives
/// as `0xCD` while `bSupported` and `attributes` are written.
///
/// # Errors
///
/// [`GpuAtomicsError::ShortParams`], and [`GpuAtomicsError::UnansweredCapType`] for every
/// `capType` but [`CAPTYPE_SYSMEM`].
pub fn answer_bus_get_pcie_supported_gpu_atomics(
    request: &[u8],
    sysmem: &[GpuAtomicOp; PCIE_SUPPORTED_GPU_ATOMICS_OP_TYPE_COUNT],
) -> Result<Vec<u8>, GpuAtomicsError> {
    let Some(body) = request.get(..PCIE_SUPPORTED_GPU_ATOMICS_PARAMS_SIZE) else {
        return Err(GpuAtomicsError::ShortParams {
            len: request.len(),
            need: PCIE_SUPPORTED_GPU_ATOMICS_PARAMS_SIZE,
        });
    };
    let cap_type = u32::from_le_bytes([body[0], body[1], body[2], body[3]]);
    if cap_type != CAPTYPE_SYSMEM {
        return Err(GpuAtomicsError::UnansweredCapType { cap_type });
    }
    let mut out = body.to_vec();
    for (i, op) in sysmem.iter().enumerate() {
        // In range by construction: `i < 13` and the buffer is `8 + 8 * 13` long.
        let at = 8 + 8 * i;
        out[at] = u8::from(op.supported);
        out[at + 4..at + 8].copy_from_slice(&op.attributes.to_le_bytes());
    }
    Ok(out)
}

/// Read the thirteen ops back out of a params buffer — for tests and the trace differential.
///
/// # Errors
///
/// [`GpuAtomicsError::ShortParams`].
pub fn decode_gpu_atomics(
    params: &[u8],
) -> Result<[GpuAtomicOp; PCIE_SUPPORTED_GPU_ATOMICS_OP_TYPE_COUNT], GpuAtomicsError> {
    let Some(body) = params.get(..PCIE_SUPPORTED_GPU_ATOMICS_PARAMS_SIZE) else {
        return Err(GpuAtomicsError::ShortParams {
            len: params.len(),
            need: PCIE_SUPPORTED_GPU_ATOMICS_PARAMS_SIZE,
        });
    };
    let mut out = [GpuAtomicOp {
        supported: false,
        attributes: 0,
    }; PCIE_SUPPORTED_GPU_ATOMICS_OP_TYPE_COUNT];
    for (i, op) in out.iter_mut().enumerate() {
        let at = 8 + 8 * i;
        op.supported = body[at] != 0;
        op.attributes =
            u32::from_le_bytes([body[at + 4], body[at + 5], body[at + 6], body[at + 7]]);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate alloc;
    use alloc::vec;

    /// The struct size is `4 + 4 + 13 * 8`, and it is the size on the wire in two traces.
    #[test]
    fn params_size_is_the_wire_size() {
        assert_eq!(PCIE_SUPPORTED_GPU_ATOMICS_PARAMS_SIZE, 112);
    }

    /// ★★★ The R23 measurement, as an assertion: `capType = SYSMEM` in a `0xCD`-seeded
    /// buffer comes back with thirteen zeroed ops and **`0xCD` padding**. If a port ever
    /// starts writing the padding, this goes red — and the padding is the only evidence
    /// left that RM writes 5 bytes per entry and not 8.
    #[test]
    fn sysmem_is_answered_and_padding_survives() {
        let mut req = vec![0xCDu8; PCIE_SUPPORTED_GPU_ATOMICS_PARAMS_SIZE];
        req[0..4].copy_from_slice(&CAPTYPE_SYSMEM.to_le_bytes());
        let out = answer_bus_get_pcie_supported_gpu_atomics(&req, &GpuAtomicOp::none_supported())
            .expect("SYSMEM is answered");
        assert_eq!(&out[0..4], &CAPTYPE_SYSMEM.to_le_bytes());
        // `dbdf` echoed untouched, exactly as the R23 poisoned-`dbdf` arm measured.
        assert_eq!(&out[4..8], &[0xCD, 0xCD, 0xCD, 0xCD]);
        for i in 0..PCIE_SUPPORTED_GPU_ATOMICS_OP_TYPE_COUNT {
            let at = 8 + 8 * i;
            assert_eq!(out[at], 0, "op {i} bSupported");
            assert_eq!(&out[at + 1..at + 4], &[0xCD, 0xCD, 0xCD], "op {i} padding");
            assert_eq!(&out[at + 4..at + 8], &[0, 0, 0, 0], "op {i} attributes");
        }
    }

    /// ★★ The libcuda request replayed byte for byte: 112 zero bytes in,
    /// `traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:48` says 112 zero bytes out.
    #[test]
    fn libcuda_all_zero_request_reproduces_the_committed_reply() {
        let req = vec![0u8; PCIE_SUPPORTED_GPU_ATOMICS_PARAMS_SIZE];
        let out = answer_bus_get_pcie_supported_gpu_atomics(&req, &GpuAtomicOp::none_supported())
            .expect("SYSMEM is answered");
        assert_eq!(out, vec![0u8; PCIE_SUPPORTED_GPU_ATOMICS_PARAMS_SIZE]);
    }

    /// ⊘ The three captypes hardware refuses are refused here, by name — including the
    /// two the **header declares**. A port that answered `GPU`/`P2P` with thirteen `FALSE`s
    /// would return `NV_OK` where a real GA106 returns `0x56`.
    #[test]
    fn every_captype_but_sysmem_is_refused() {
        for cap in [CAPTYPE_GPU, CAPTYPE_P2P, 3, 0xCDCD_CDCD, u32::MAX] {
            let mut req = vec![0xCDu8; PCIE_SUPPORTED_GPU_ATOMICS_PARAMS_SIZE];
            req[0..4].copy_from_slice(&cap.to_le_bytes());
            assert_eq!(
                answer_bus_get_pcie_supported_gpu_atomics(&req, &GpuAtomicOp::none_supported()),
                Err(GpuAtomicsError::UnansweredCapType { cap_type: cap }),
                "capType {cap:#x}"
            );
        }
    }

    /// ★ The R18 seed, verbatim: `0xCD` in every byte. This is the arm that refused on real
    /// hardware, and it must refuse here for the **same reason** — the captype, not the
    /// length. A port that ignored `capType` would answer it, and a diff against hardware
    /// would go red at exactly this request.
    #[test]
    fn the_r18_seed_refuses_on_the_captype_not_the_length() {
        let req = vec![0xCDu8; PCIE_SUPPORTED_GPU_ATOMICS_PARAMS_SIZE];
        assert_eq!(
            answer_bus_get_pcie_supported_gpu_atomics(&req, &GpuAtomicOp::none_supported()),
            Err(GpuAtomicsError::UnansweredCapType {
                cap_type: 0xCDCD_CDCD
            })
        );
    }

    /// A short buffer is refused before `capType` is read out of it.
    #[test]
    fn short_params_are_refused() {
        for len in [0usize, 3, 111] {
            let req = vec![0u8; len];
            assert_eq!(
                answer_bus_get_pcie_supported_gpu_atomics(&req, &GpuAtomicOp::none_supported()),
                Err(GpuAtomicsError::ShortParams {
                    len,
                    need: PCIE_SUPPORTED_GPU_ATOMICS_PARAMS_SIZE
                })
            );
        }
    }

    /// A longer buffer is answered over its first 112 bytes and truncated to them, which is
    /// what the RPC reply carries.
    #[test]
    fn a_longer_buffer_is_answered_over_the_struct_only() {
        let mut req = vec![0u8; PCIE_SUPPORTED_GPU_ATOMICS_PARAMS_SIZE + 64];
        req[0..4].copy_from_slice(&CAPTYPE_SYSMEM.to_le_bytes());
        let out = answer_bus_get_pcie_supported_gpu_atomics(&req, &GpuAtomicOp::none_supported())
            .expect("SYSMEM is answered");
        assert_eq!(out.len(), PCIE_SUPPORTED_GPU_ATOMICS_PARAMS_SIZE);
    }

    /// ★ The round trip, over a table that is **not** all-`false` — because a decoder that
    /// only ever sees zeros is a decoder no test has exercised. `bSupported` must land in
    /// byte 0 of the entry and `attributes` in bytes 4..8; a decoder reading a `u32`
    /// `bSupported` would swallow the padding and fail here.
    #[test]
    fn round_trip_over_a_non_zero_table() {
        let mut ops = GpuAtomicOp::none_supported();
        ops[0] = GpuAtomicOp {
            supported: true,
            attributes: 0b1001_1001,
        };
        ops[12] = GpuAtomicOp {
            supported: true,
            attributes: 0xFFFF_FFFF,
        };
        let mut req = vec![0xCDu8; PCIE_SUPPORTED_GPU_ATOMICS_PARAMS_SIZE];
        req[0..4].copy_from_slice(&CAPTYPE_SYSMEM.to_le_bytes());
        let out =
            answer_bus_get_pcie_supported_gpu_atomics(&req, &ops).expect("SYSMEM is answered");
        assert_eq!(out[8], 1);
        assert_eq!(
            &out[9..12],
            &[0xCD, 0xCD, 0xCD],
            "padding is still not ours"
        );
        assert_eq!(decode_gpu_atomics(&out).expect("decodes"), ops);
    }

    /// `none_supported` is thirteen entries and every one of them is a denial. ⊘ The count
    /// is asserted because a shorter array is a smaller true statement.
    #[test]
    fn none_supported_denies_all_thirteen() {
        let ops = GpuAtomicOp::none_supported();
        assert_eq!(ops.len(), PCIE_SUPPORTED_GPU_ATOMICS_OP_TYPE_COUNT);
        assert!(ops.iter().all(|o| !o.supported && o.attributes == 0));
    }
}
