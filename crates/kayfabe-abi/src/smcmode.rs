//! `NV2080_CTRL_CMD_INTERNAL_GPU_GET_SMC_MODE` (`0x20800a4c`) — ★★★ **the control that
//! decided `cuInit`**, and the one this port had classified as *"by definition not part of
//! init"* for as long as it has had a demand list.
//!
//! # `[measured 2026-08-08, boot `gis1_e6ed6bc`, real GA106 on `vh`]` What it cost
//!
//! `execution_plane_increments.md` §14.28 ended on one line of a guest-side `cuInit` trace:
//! libcuda's eleven-index `NV2080_CTRL_CMD_GPU_GET_INFO_V2` came back `status=0x56` with
//! `out == in`, and **no RPC crossed to this port at all**. §14.29 bisected it, in the guest,
//! on libcuda's own subdevice handle:
//!
//! ```text
//! SWEEPIDX pos=3 idx=0x0000002a status=0x00000056   ← GPU_SMC_MODE, alone
//! SWEEPIDX (the other ten)      status=0x00000000   ← all ten NV_OK, all ten matching
//!                                                     a real GA106's values exactly
//! SWEEPPFX len=1,2,3            status=0x00000000
//! SWEEPPFX len=4..11            status=0x00000056   ← the break is AT position 3
//! ```
//!
//! ⇒ Exactly **one** of the eleven indices poisons the call. `getGpuInfos`'s arm for
//! `NV2080_CTRL_GPU_INFO_INDEX_GPU_SMC_MODE` (`0x2a`) is not answered from kernel state at
//! all on a bare-metal GSP client: it issues *this* control on the **physical** RMAPI and
//! assigns its status straight to the loop's `status`
//! (`ogkm-580: src/nvidia/src/kernel/gpu/subdevice/subdevice_ctrl_gpu_kernel.c:232-266`).
//! The loop then `break`s and **returns that status for the whole call** (`:566-569`), so a
//! single refused internal control fails ten indices that were already computed correctly.
//!
//! And the corroboration at the other boundary, same boot:
//! `nvkvm: unserviced fn 76 cmd 0x20800a4c` — in that boot's QEMU log and in **seven earlier
//! committed bench boots**.
//!
//! ## ⊘⊘ This also refutes the reason the row was dismissed
//!
//! `docs/reference/remaining_boot_surface.md` §1 computed `rows − transcript = {0x20800a4c}`
//! over two committed artefacts and concluded *"the transcript covers the init set exactly,
//! missing precisely the one command that is by definition not part of init."* The
//! **set-difference was right**; the inference drawn from it was that the leftover was
//! therefore uninteresting. It was the wall. ★ *"Not reached during `RmInitAdapter`"* and
//! *"not needed"* are different statements, and every oracle this project owned was
//! `nvidia-smi`-driven — so none of them could ever have distinguished them
//! (`traces/real_ga106/README.md`, "the method row is also this directory's blind spot").
//!
//! # ★★★ Where the value comes from, and where it must NOT come from
//!
//! ⊘ **Not from the C oracle.** `C: src/qemu/mode2_initctrl_ga106.h:6243` is `0x20800a4c`'s row
//! with `psize = 4, dlen = 0` — it is one of the eleven **empty** rows, nine of which real
//! hardware contradicts outright (`crate::oracle`). An empty capture is evidence of nothing,
//! and `traces/real_ga106/README.md` marks this row *"⚠ coincides"* precisely because
//! **nothing about the row itself** distinguishes it from the nine that are wrong.
//!
//! ★ The value is taken from two positive measurements on **two different physical parts**,
//! by two different instruments:
//!
//! | source | part | what it says |
//! |---|---|---|
//! | `traces/real_ga106/rpc_bodies_real_ga106.txt:617-628` (4 calls) | `GPU-e28d7776-…` | reply body `00 00 00 00`, `psize=4`, `gspst=0x0` |
//! | `traces/real_ga106/rmladder_r21_gpuinfo_sweep_real_ga106.txt` (R21 `0x2a`) | `GPU-d0913685-…` | `NV_OK data=0x00000000` |
//!
//! The second reads the same number through the *whole* `getGpuInfos` arm — an unprivileged
//! `GPU_GET_INFO_V2[0x2a]` on the host — so it is also the standing way to obtain this field
//! on **any** part without a table (`derive_what_you_cannot_query_then_oracle_it`).
//!
//! ⚠ Zero here is a **named meaning**, not an absence: `NV2080_CTRL_GPU_INFO_GPU_SMC_MODE_
//! UNSUPPORTED` (`ogkm-580: ctrl2080gpu.h:162`). GA106 is a GeForce part and MIG is not a
//! GeForce feature. That is why this module encodes an **enum**, never a bare `u32`: the
//! distinction between *"UNSUPPORTED, measured"* and *"we captured nothing and it defaulted
//! to zero"* is exactly the one the fifth-limit rows lost, and a type is the only place it
//! survives a refactor.

/// `NV2080_CTRL_CMD_INTERNAL_GPU_GET_SMC_MODE`.
///
/// `ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080internal.h:935`.
pub const NV2080_CTRL_CMD_INTERNAL_GPU_GET_SMC_MODE: u32 = 0x2080_0a4c;

/// `sizeof(NV2080_CTRL_INTERNAL_GPU_GET_SMC_MODE_PARAMS)` — a single `NvU32 smcMode`.
///
/// `ogkm-580: ctrl2080internal.h:939-941`; confirmed on the wire as `psize=4` in
/// `traces/real_ga106/rpc_bodies_real_ga106.txt:617`.
pub const INTERNAL_GPU_GET_SMC_MODE_PARAMS_SIZE: usize = 4;

/// The SMC (MIG) mode of a GPU, as `NV2080_CTRL_GPU_INFO_GPU_SMC_MODE_*`.
///
/// `ogkm-580: ctrl2080gpu.h:162-166`. ★ An enum rather than a `u32` so that *"zero because
/// SMC is unsupported"* cannot be confused with *"zero because nothing was measured"* — the
/// confusion that made nine of the C oracle's eleven empty rows wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum SmcMode {
    /// The part has no SMC/MIG capability at all. `[measured]` the answer on GA106.
    Unsupported = 0,
    /// MIG is on.
    Enabled = 1,
    /// The part is MIG-capable and MIG is off.
    Disabled = 2,
    /// A MIG enable is staged and takes effect after the next GPU reset.
    EnablePending = 3,
    /// A MIG disable is staged and takes effect after the next GPU reset.
    DisablePending = 4,
}

impl SmcMode {
    /// Every variant, so a gate can quantify over the encoding rather than over a sample.
    pub const ALL: [Self; 5] = [
        Self::Unsupported,
        Self::Enabled,
        Self::Disabled,
        Self::EnablePending,
        Self::DisablePending,
    ];

    /// The wire word.
    #[must_use]
    pub const fn as_u32(self) -> u32 {
        self as u32
    }
}

/// ★★★ The mode a **real GA106** reports, `[measured]` on two physical parts — see the
/// module docs' provenance table.
///
/// ⊘ Exported so a chip row and a test can be checked against one literal, **not** so a
/// second chip can borrow it: a MIG-capable part answers something else, and this is a
/// statement about GeForce silicon rather than about Ampere.
pub const GA106_SMC_MODE: SmcMode = SmcMode::Unsupported;

/// Why an SMC mode could not be decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmcModeError {
    /// The body was shorter than the four bytes the struct declares.
    ///
    /// ⊘ Refused rather than zero-extended. A truncated reply that decoded to `Unsupported`
    /// would be indistinguishable from a hardware-measured `Unsupported`, which is the
    /// laundering this module exists to prevent.
    ShortBody,
    /// The word is not one of the five `NV2080_CTRL_GPU_INFO_GPU_SMC_MODE_*` code points.
    ///
    /// ⚠ Named, not clamped. RM hands `params.smcMode` straight through to a client as the
    /// value of `GPU_INFO_INDEX_GPU_SMC_MODE`, so an out-of-range word would become a
    /// client's view of the machine.
    UnknownMode(u32),
}

/// Encode the `[OUT]` reply — the mode word, little-endian.
///
/// ## ★★ The transport question, which is the only one that decides whether the body matters
///
/// *"Does the caller read its own params after the call returns?"* — **yes, immediately**:
/// `getGpuInfos` does `data = params.smcMode;` on the line after the control
/// (`ogkm-580: subdevice_ctrl_gpu_kernel.c:265`), reading the struct
/// `rpcRmApiControl_GSP`'s copyout wrote over (`rpc.c:11085-11090`). So the body is
/// load-bearing, exactly as it is for [`crate::fmbsize`] and unlike [`crate::l2evict`].
#[must_use]
pub fn encode_smc_mode(mode: SmcMode) -> Vec<u8> {
    mode.as_u32().to_le_bytes().to_vec()
}

/// Read a mode back out of a reply body — the inverse of [`encode_smc_mode`], for tests and
/// for the trace differential.
///
/// # Errors
///
/// [`SmcModeError::ShortBody`] if fewer than [`INTERNAL_GPU_GET_SMC_MODE_PARAMS_SIZE`] bytes
/// are present; [`SmcModeError::UnknownMode`] if the word names no known mode.
pub fn decode_smc_mode(params: &[u8]) -> Result<SmcMode, SmcModeError> {
    let Some(w) = params.get(..INTERNAL_GPU_GET_SMC_MODE_PARAMS_SIZE) else {
        return Err(SmcModeError::ShortBody);
    };
    let word = u32::from_le_bytes([w[0], w[1], w[2], w[3]]);
    SmcMode::ALL
        .into_iter()
        .find(|m| m.as_u32() == word)
        .ok_or(SmcModeError::UnknownMode(word))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reply_is_the_four_bytes_the_real_ga106_put_on_the_wire() {
        // ★★★ The literal is transcribed from the raw RPC reply
        // (`traces/real_ga106/rpc_bodies_real_ga106.txt:618`,
        // `KAYFABE-BODY: cmd=0x20800a4c +00000 00 00 00 00`), not from a deserialised number,
        // so this fails if the encoder ever disagrees with what was observed.
        let body = encode_smc_mode(GA106_SMC_MODE);
        assert_eq!(body, vec![0x00, 0x00, 0x00, 0x00]);
        assert_eq!(body.len(), INTERNAL_GPU_GET_SMC_MODE_PARAMS_SIZE);
        // ★ And the second part, read through the whole `getGpuInfos` arm rather than off
        // the RPC: `rmladder_r21_gpuinfo_sweep_real_ga106.txt`, `R21 0x2a NV_OK data=0x0`.
        assert_eq!(GA106_SMC_MODE.as_u32(), 0);
    }

    #[test]
    fn zero_is_a_named_meaning_and_the_type_is_what_says_so() {
        // ⊘ The falsifier for the module's central claim. The C oracle's row for this id is
        // `psize 4, dlen 0` — an EMPTY body that decodes to the same four zero bytes as the
        // hardware measurement. The bytes cannot tell them apart; only the fact that this
        // port names `Unsupported` and takes it from a POSITIVE capture can.
        assert_eq!(decode_smc_mode(&[0, 0, 0, 0]), Ok(SmcMode::Unsupported));
        assert_ne!(SmcMode::Unsupported.as_u32(), SmcMode::Disabled.as_u32());
        // ⚠ ...and the two are not synonyms: a MIG-capable part with MIG off answers 2, and
        // a client that saw 0 would conclude the part cannot do MIG at all.
        assert_eq!(decode_smc_mode(&[2, 0, 0, 0]), Ok(SmcMode::Disabled));
    }

    #[test]
    fn a_short_body_is_refused_rather_than_zero_extended() {
        // A truncated reply must not become `Unsupported`, which is the one value that would
        // look exactly like a correct answer on this part.
        for n in 0..INTERNAL_GPU_GET_SMC_MODE_PARAMS_SIZE {
            assert_eq!(
                decode_smc_mode(&[0u8; 4][..n]),
                Err(SmcModeError::ShortBody),
                "{n} bytes must not decode"
            );
        }
    }

    #[test]
    fn an_unknown_code_point_is_named_not_clamped() {
        // RM hands `params.smcMode` straight to a client as GPU_INFO_INDEX_GPU_SMC_MODE, so
        // clamping would put a value on a client's view of the machine that no RM ever
        // produced.
        assert_eq!(
            decode_smc_mode(&[5, 0, 0, 0]),
            Err(SmcModeError::UnknownMode(5))
        );
        assert_eq!(
            decode_smc_mode(&0xCDCD_CDCDu32.to_le_bytes()),
            Err(SmcModeError::UnknownMode(0xCDCD_CDCD)),
            "the sweep's 0xCD seed must not decode as a mode"
        );
    }

    #[test]
    fn the_round_trip_holds_for_every_declared_mode() {
        for mode in SmcMode::ALL {
            assert_eq!(decode_smc_mode(&encode_smc_mode(mode)), Ok(mode));
        }
        // ★ And the encoding is dense over 0..=4 with no gaps, which is what makes the
        // `find` above a total function of the declared set rather than of a sample.
        let words: Vec<u32> = SmcMode::ALL.iter().map(|m| m.as_u32()).collect();
        assert_eq!(words, vec![0, 1, 2, 3, 4]);
    }
}
