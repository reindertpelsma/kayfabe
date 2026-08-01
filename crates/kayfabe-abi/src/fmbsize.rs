//! `NV2080_CTRL_CMD_CE_GET_FAULT_METHOD_BUFFER_SIZE` (`0x20802a08`) — one `NvU32`, and the
//! first number in this port **measured on real silicon** because nothing else could say it.
//!
//! ## Why this control has its own module
//!
//! It is four bytes. The reason it is not a literal somewhere is that the four bytes are a
//! **buffer length RM then allocates and hardware then writes into**, and every other source
//! of the number was either absent or wrong:
//!
//! - **The open driver does not contain it.** `subdeviceCtrlCmdCeGetFaultMethodBufferSize_IMPL`
//!   is declared at `ogkm-580: src/nvidia/generated/g_subdevice_nvoc.h:5094` and referenced
//!   from the export table at `g_subdevice_nvoc.c:7664` — and **defined nowhere in the
//!   tree**. The body is compiled into GSP firmware. Reading source cannot answer it.
//! - **A usermode probe cannot ask for it.** Its export row carries `flags = 0x1c040`
//!   (`g_subdevice_nvoc.c:7666`), which contains `ROUTE_TO_PHYSICAL` (`0x40`) but none of
//!   `PRIVILEGED` (`0x4`), `NON_PRIVILEGED` (`0x8`) or `INTERNAL` (`0x80`) — RM's default,
//!   `KERNEL_PRIVILEGED`, refused to every usermode client including root at
//!   `ogkm-580: src/nvidia/src/kernel/rmapi/control.c:702-709`.
//! - ★★★ **The C oracle's captured GA106 table is WRONG here.** Its row is
//!   `{0x20802a08u, 0x0u, 4u, 0u, ctl_20802a08}` with an **empty** body
//!   (`C: src/qemu/mode2_initctrl_ga106.h:6233`), which decodes to `size = 0` — the very
//!   value that produces the wall this module removes. See
//!   [the measurement](#the-measurement) for how it was falsified and for the five other
//!   rows that fall with it.
//!
//! ## Why a wrong number is not a survivable guess
//!
//! `bufSizeInBytes` is handed straight to `memdescCreate` as the allocation length
//! (`ogkm-580: kernel_channel_group_gv100.c:109-110`), once per runqueue per channel group,
//! and the buffer is where the **host FIFO saves a faulting engine's methods**. Under-size it
//! and the failure is a hardware writer running off the end of a real allocation, surfacing
//! much later as an MMU fault attributed to something else. That is why the previous rung
//! refused to guess it, and the refusal was right.
//!
//! ⊘ Over-size is not free either, but it is loud: the same descriptor is mapped into BAR2's
//! invisible aperture (`kernel_channel_group_gv100.c:256-261`), so an absurd value exhausts
//! the aperture at a named statement.
//!
//! # The measurement
//!
//! ★★★ `[measured]` **on a real GA106** — vast instance `46494693`, NVIDIA GeForce RTX 3060,
//! `GPU-e28d7776-e4f9-704b-d392-d46f187343f8`, host driver **NVIDIA open 580.159.04**, the
//! same version the guest runs. The method: the matching open-driver source
//! (`research_clones/ogkm-580.159.04`, tag `b81d58e`) rebuilt with a print at the two points
//! that see the value, loaded in place of the stock module, and `RmInitAdapter` driven by
//! opening `/dev/nvidia0`.
//!
//! Two **independent** instrumentation points agree, which is the reason there are two:
//!
//! ```text
//! kceGetFaultMethodBufferSize_IMPL: 0x20802a08 -> status=0x0 params.size=20480 (0x5000)
//! rpcRmApiControl_GSP:              cmd=0x20802a08 psize=4 gspst=0x0 head=00 50 00 00
//! ```
//!
//! The first reads the deserialised `NvU32` after the RPC returns; the second reads the raw
//! reply bytes on the wire before anything interprets them. `00 50 00 00` little-endian is
//! `0x5000` is `20480`. Transcripts are committed under `traces/real_ga106/`.
//!
//! ★★ The same run also measured the two multipliers, so the reserved-FB total this feeds
//! (`kfifoCalcTotalSizeOfFaultMethodBuffers_GV100`, `kernel_fifo_gv100.c:293-325`) is
//! derivable rather than assumed: `maxChannelGroups = 4096`, `runQueues = 2`.
//!
//! ## ⊘ What the measurement does NOT establish
//!
//! - ⊘ **It is a fact about GA106 and about nothing else.** It is therefore a
//!   [`crate::fmbsize`] *encoding* plus a **chip-row field**, not a constant: `AD10x` and
//!   `GH100` were never asked, and a port that answered them 20480 would be stating an
//!   unmeasured number in the flattering direction. A chip that declares zero is refused at
//!   realize rather than served.
//! - ⊘ **It does not establish that the guest's own boot reaches this control**, only what
//!   the answer must be when it does.
//! - ⊘ **It says nothing about what hardware writes into the buffer**, only how much room it
//!   is promised.
//!
//! # ★★ The C oracle's `dlen = 0` rows are a capture defect
//!
//! Falsifying this control's row falsifies a class. Against the same real-GA106 transcript,
//! every row of `mode2_initctrl_ga106.h` that carries a **non-empty** body agrees with
//! hardware byte for byte (`0x20800a61` → `00 00 00 00 00 08 00 00`; `0x20800a80` → sixteen
//! zeros), while **every** `dlen = 0` row checked is contradicted:
//!
//! | control | C's captured row | real GA106 |
//! |---|---|---|
//! | `0x20802a08` | `psize 4, dlen 0` | `00 50 00 00` |
//! | `0x20802a06` | `psize 4, dlen 0` | `10 00 00 00` |
//! | `0x2080017e` | `psize 8, dlen 0` | `00 00 00 02 00 00 00 00` |
//! | `0x20800af3` | `psize 2, dlen 0` | `01 01` |
//! | `0x20800a4b` | `psize 4, dlen 0` | `00 00 01 04` |
//! | `0x20800aac` | `psize 4, dlen 0` | `00 00 01 00` |
//!
//! ⚠ So *"the oracle answered it with an empty body"* is **not** evidence that a real GSP
//! answers zero, and any triage row resting on that reading is resting on a capture artefact.
//! This is a new entry in `docs/design/c_rust_trace_differential.md`'s list of measured
//! limits: the C is an oracle for the *reply plane's structure*, and for row bodies it
//! actually captured — not for rows it recorded empty.

/// `NV2080_CTRL_CMD_CE_GET_FAULT_METHOD_BUFFER_SIZE`.
///
/// `ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080ce.h:295`.
pub const NV2080_CTRL_CMD_CE_GET_FAULT_METHOD_BUFFER_SIZE: u32 = 0x2080_2a08;

/// `sizeof(NV2080_CTRL_CE_GET_FAULT_METHOD_BUFFER_SIZE_PARAMS)` — a single `NvU32`.
///
/// `ogkm-580: ctrl2080ce.h:291-293`, and confirmed on the wire as `psize=4`.
pub const CE_FAULT_METHOD_BUFFER_SIZE_PARAMS_SIZE: usize = 4;

/// ★★★ The value a **real GA106** returns, `[measured]` — see the module docs.
///
/// ⊘ Exported so a chip row and a test can be checked against the same literal, **not** so
/// that a second chip can borrow it. It is 20 KiB.
pub const GA106_CE_FAULT_METHOD_BUFFER_SIZE: u32 = 20480;

/// Why a fault-method-buffer size could not be encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultMethodBufferSizeError {
    /// ★★★ The size is zero, which is the one value that is certainly wrong.
    ///
    /// It is not merely unhelpful: `memdescCreate` rejects a zero length outright
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/mem_desc.c:239-241`,
    /// `if (allocSize == 0) return NV_ERR_INVALID_ARGUMENT;`), and that rejection *is* the
    /// `0x25:0x1f:1249` this module exists to remove. Serving zero would be indistinguishable
    /// from not serving the control at all — so it is refused here, loudly, rather than
    /// encoded into a reply that reproduces the wall while looking like an answer.
    Zero,
}

/// Encode the `[OUT]` reply — the size, little-endian.
///
/// ## ★★ The transport question, and it is the only one that matters for this control
///
/// *"Does the caller read its own params after the call returns?"* — **yes, immediately**:
/// `kceGetFaultMethodBufferSize_IMPL` does `*size = params.size;` on the line after the
/// control (`ogkm-580: src/nvidia/src/kernel/gpu/ce/kernel_ce.c:846`), reading the struct
/// that `rpcRmApiControl_GSP`'s copyout wrote over (`rpc.c:11085-11090`). So unlike
/// [`crate::l2evict`], whose caller reads nothing back, the **body is load-bearing** and a
/// reply that carried the right length and the wrong bytes would be exactly as fatal as no
/// reply at all.
///
/// ⊘ Refuses zero rather than encoding it — see [`FaultMethodBufferSizeError::Zero`]. This is
/// what stops a chip row that forgot to state a size from being served as *"the buffer is
/// zero bytes"*.
///
/// # Errors
///
/// [`FaultMethodBufferSizeError::Zero`] if `size` is zero.
pub fn encode_fault_method_buffer_size(size: u32) -> Result<Vec<u8>, FaultMethodBufferSizeError> {
    if size == 0 {
        return Err(FaultMethodBufferSizeError::Zero);
    }
    Ok(size.to_le_bytes().to_vec())
}

/// Read a size back out of a reply body — the inverse of
/// [`encode_fault_method_buffer_size`], for tests and for the trace differential.
///
/// # Errors
///
/// [`FaultMethodBufferSizeError::Zero`] if the body decodes to zero, so that a round trip
/// cannot launder a zero that the encoder would have refused.
pub fn decode_fault_method_buffer_size(params: &[u8]) -> Result<u32, FaultMethodBufferSizeError> {
    let Some(w) = params.get(..CE_FAULT_METHOD_BUFFER_SIZE_PARAMS_SIZE) else {
        return Err(FaultMethodBufferSizeError::Zero);
    };
    let size = u32::from_le_bytes([w[0], w[1], w[2], w[3]]);
    if size == 0 {
        return Err(FaultMethodBufferSizeError::Zero);
    }
    Ok(size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reply_is_the_four_bytes_the_real_ga106_put_on_the_wire() {
        // ★★★ The literal is the hardware's, transcribed from the raw RPC reply rather than
        // from the deserialised number, so this test fails if the encoder ever disagrees with
        // what was actually observed: `head=00 50 00 00`.
        let body = encode_fault_method_buffer_size(GA106_CE_FAULT_METHOD_BUFFER_SIZE)
            .expect("20480 is not zero");
        assert_eq!(body, vec![0x00, 0x50, 0x00, 0x00]);
        assert_eq!(body.len(), CE_FAULT_METHOD_BUFFER_SIZE_PARAMS_SIZE);
        // And the number, stated the other way round so a mistyped constant is caught here
        // rather than by a boot.
        assert_eq!(GA106_CE_FAULT_METHOD_BUFFER_SIZE, 0x5000);
        assert_eq!(GA106_CE_FAULT_METHOD_BUFFER_SIZE, 20 * 1024);
    }

    #[test]
    fn zero_is_refused_on_both_sides_because_zero_is_the_bug() {
        // ⊘ The falsifier for the whole module: encoding zero would rebuild the exact wall
        // (`mem_desc.c:239-241`) while presenting as a served control.
        assert_eq!(
            encode_fault_method_buffer_size(0),
            Err(FaultMethodBufferSizeError::Zero)
        );
        assert_eq!(
            decode_fault_method_buffer_size(&[0, 0, 0, 0]),
            Err(FaultMethodBufferSizeError::Zero)
        );
        // ★ And the C oracle's own captured row decodes to exactly that, which is the
        // measured reason this port does not take its answer from the capture — the run
        // that measured it: RTX 3060, open 580.159.04, 2026-08-01, `traces/real_ga106/`.
        assert_eq!(
            decode_fault_method_buffer_size(&[0u8; 4]),
            Err(FaultMethodBufferSizeError::Zero),
            "the oracle's empty ctl_20802a08[] body is the value hardware falsified"
        );
    }

    #[test]
    fn a_short_body_is_not_read_as_a_smaller_number() {
        // A truncated reply must not decode as a partial little-endian value — that is how a
        // 20480 becomes an 80 and the overrun becomes silent.
        for n in 0..CE_FAULT_METHOD_BUFFER_SIZE_PARAMS_SIZE {
            assert_eq!(
                decode_fault_method_buffer_size(&[0x00, 0x50, 0x00, 0x00][..n]),
                Err(FaultMethodBufferSizeError::Zero),
                "{n} bytes must not decode"
            );
        }
    }

    #[test]
    fn the_round_trip_holds_for_every_plausible_size() {
        for size in [1u32, 4096, GA106_CE_FAULT_METHOD_BUFFER_SIZE, u32::MAX] {
            let body = encode_fault_method_buffer_size(size).expect("non-zero");
            assert_eq!(decode_fault_method_buffer_size(&body), Ok(size));
        }
    }
}
