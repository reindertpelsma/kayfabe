//! **S2 — the element codec.** Framing, checksum, multi-element split/join.
//!
//! ## ★ The element header is a different structure between 580 and 610
//!
//! | | 580 (bench; r535/r570-shaped) | 610 (the vendored ogkm) |
//! |---|---|---|
//! | @0 | `authTagBuffer[16]` | `mctpHeader` |
//! | @4 | ↑ | `nvdmHeader` |
//! | @8 | ↑ | `checkSum` |
//! | @12 | ↑ | `seqNum` |
//! | @16 | `aadBuffer[16]` | **payload begins** |
//! | @32 | `checkSum` | — |
//! | @36 | `seqNum` | — |
//! | @40 | **`elemCount`** | — |
//! | @48 | rpc header | — |
//! | header size | **48** | **16** (+40 with Confidential Compute) |
//!
//! [src] 610: `ogkm: src/nvidia/inc/kernel/gpu/gsp/message_queue_priv.h:52-67`,
//! `ogkm: src/nvidia/src/kernel/gpu/gsp/message_queue_cpu.c:80-86`.
//! [src] r535 **and** r570, independently: the 48-byte form, byte-identical to the C
//! artifact's — `nv: r535/nvrm/gsp.h:808-816`, `nv: r535/rpc.c:94-102, 119`.
//! So the break is bracketed to **(570, 610]**, and the C — whose 580 implementation
//! matches r535/r570 exactly — is right for its era and wrong as a protocol.
//!
//! The C hard-codes 48/40/32/36 at ~15 sites. Here the shape is an [`ElementLayout`]
//! **value**, supplied by the ABI layer; this module carries no offset of its own. That
//! is one of the five authorised deviations ("the version key").
//!
//! ## Checksum
//!
//! 64-bit XOR fold, reduced to 32 by `hi ^ lo` (`ogkm: message_queue_priv.h:191-209`).
//! The routine steps in `NvU64`s `while (p < pEnd)`, i.e. it **reads up to the next
//! 8-byte boundary past `uLen`** — its own comment licenses exactly that — and the sender
//! zero-pads to 8 first (`ogkm: message_queue_cpu.c:499-503`). Coverage in the plain case
//! is `hdrSize + rpc.length` (`ogkm: message_queue_cpu.c:543-546`, and the receiver's
//! mirror at `:724-734`); the whole element run when CC is on (`:540-541`).
//!
//! ★ nouveau folds over the **page-rounded whole element** instead
//! (`nv: r535/rpc.c:364-375`). Both agree only if the element's tail is zero — so
//! [`encode_message`] zeroes the entire run, which is also what the C does
//! (`C: src/qemu/nvkvm_gpu_emul.c:1583`, `memset(el, 0, …)`). One encoder, valid under
//! both conventions.

use crate::fault::{GspFault, LayoutError};
use kayfabe_abi::view::RpcEnvelope;

/// The transport headers an element carries in front of the RPC envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportHdr {
    /// None — the r535/r570/580 form. The first 32 bytes are the (unused, zero) CC
    /// authentication/AAD buffers.
    None,
    /// MCTP over NVDM — the 610 form, which the guest **validates** on receive
    /// (`ogkm: message_queue_cpu.c:737-759`: a wrong `MCTP_HEADER_VERSION` or a wrong
    /// NVDM vendor id is `NV_ERR_INVALID_DATA`, *"MCTP protocol violation"*).
    ///
    /// The words are carried whole rather than as bit fields: the guest's encoder builds
    /// them from fixed arguments — `mctpCreateTransportHeader(SOM=1, EOM=1, 0, 0, 0)` and
    /// `mctpCreateNvdmHeader(NVDM_TYPE_RM_RPC)` (`ogkm: message_queue_cpu.c:505-512`) — so
    /// every conforming element carries the same two constants, and the ABI layer that
    /// knows the bit positions supplies them already assembled. This crate never encodes
    /// a bit field it has not seen.
    Mctp {
        /// Byte offset of the MCTP transport word.
        header_off: usize,
        /// The word a conforming sender writes there.
        header_word: u32,
        /// Byte offset of the NVDM word.
        nvdm_off: usize,
        /// The word a conforming sender writes there.
        nvdm_word: u32,
    },
}

/// Where an element's fixed fields live, for one driver version.
///
/// Validated at construction: every field must fit inside the header and none may
/// overlap. An `ElementLayout` that cannot describe a real element is unconstructible,
/// so no decode path has to re-check it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElementLayout {
    hdr_size: usize,
    checksum_off: usize,
    seqnum_off: usize,
    elem_count_off: Option<usize>,
    transport: TransportHdr,
}

impl ElementLayout {
    /// Describe an element header.
    ///
    /// `hdr_size` is the **effective** header size — `queueElementHdrSize`, i.e. with the
    /// Confidential-Compute tag already folded in where CC is on
    /// (`ogkm: message_queue_cpu.c:80-86`). It is a computed value at the ABI seam, never
    /// a constant here, which is what keeps CC from needing a second code path.
    ///
    /// # Errors
    ///
    /// [`LayoutError`] if a field escapes the header or two fields overlap.
    pub fn new(
        hdr_size: usize,
        checksum_off: usize,
        seqnum_off: usize,
        elem_count_off: Option<usize>,
        transport: TransportHdr,
    ) -> Result<ElementLayout, LayoutError> {
        if hdr_size < 8 {
            return Err(LayoutError::HeaderTooSmall { hdr_size });
        }
        let mut fields = vec![checksum_off, seqnum_off];
        if let Some(o) = elem_count_off {
            fields.push(o);
        }
        if let TransportHdr::Mctp {
            header_off,
            nvdm_off,
            ..
        } = transport
        {
            fields.push(header_off);
            fields.push(nvdm_off);
        }
        for &o in &fields {
            if o + 4 > hdr_size {
                return Err(LayoutError::FieldOutsideHeader {
                    offset: o,
                    hdr_size,
                });
            }
        }
        for (i, &a) in fields.iter().enumerate() {
            for &b in &fields[i + 1..] {
                if a == b {
                    return Err(LayoutError::FieldsOverlap { a, b });
                }
            }
        }
        Ok(ElementLayout {
            hdr_size,
            checksum_off,
            seqnum_off,
            elem_count_off,
            transport,
        })
    }

    /// `queueElementHdrSize` — where the RPC envelope starts inside an element.
    #[must_use]
    pub fn hdr_size(&self) -> usize {
        self.hdr_size
    }

    /// Byte offset of `checkSum`.
    #[must_use]
    pub fn checksum_off(&self) -> usize {
        self.checksum_off
    }

    /// Byte offset of `seqNum`.
    #[must_use]
    pub fn seqnum_off(&self) -> usize {
        self.seqnum_off
    }

    /// Byte offset of `elemCount`, on the versions that have one.
    ///
    /// ★ `None` on 610 — and getting this wrong is not a cosmetic difference: at that
    /// offset 610 has `rpc.sequence` (payload@16 + 24), so an encoder that wrote the
    /// element count there would corrupt the transaction id
    /// (`ogkm: message_queue_priv.h:52-67`).
    #[must_use]
    pub fn elem_count_off(&self) -> Option<usize> {
        self.elem_count_off
    }

    /// The transport headers this version carries.
    #[must_use]
    pub fn transport(&self) -> TransportHdr {
        self.transport
    }
}

/// `gspMsgQueueBytesToElements` — `ceil(bytes / elementSizeMin)`
/// (`ogkm: message_queue_priv.h:117-121`).
///
/// Returns 0 only for `element_size_min == 0`, which a bound geometry cannot have.
#[must_use]
pub fn bytes_to_elements(bytes: u32, element_size_min: u32) -> u32 {
    if element_size_min == 0 {
        return 0;
    }
    bytes.div_ceil(element_size_min)
}

/// `_checkSum32` — the 64-bit XOR fold, reduced to 32 by `hi ^ lo`.
///
/// `len` is the *declared* coverage; the fold runs to the next 8-byte boundary past it,
/// which is what the driver does and why the sender zero-pads. `bytes` must be at least
/// that long — an element buffer always is, since it is a whole number of page-sized
/// elements.
#[must_use]
pub fn checksum32(bytes: &[u8], len: usize) -> u32 {
    let end = len.next_multiple_of(8).min(bytes.len());
    let mut acc: u64 = 0;
    for chunk in bytes[..end].chunks_exact(8) {
        let mut b = [0u8; 8];
        b.copy_from_slice(chunk);
        acc ^= u64::from_le_bytes(b);
    }
    ((acc >> 32) as u32) ^ (acc as u32)
}

/// A validated message length: the one bound every extent in the transport derives from.
///
/// ★ The **lower** bound is the RPC envelope's own size, not zero. `rpc.length == 0`
/// passes the driver's own sanity check (`msgLen == hdrSize`,
/// `ogkm: message_queue_cpu.c:487-497` and the mirror at `:824-833`) and
/// `bytesToElements(hdrSize, 4096) == 1`, so a zero-length message silently consumes an
/// element and then produces garbage upstream. Refusing it is part of the authorised
/// "RPC element parsing" deviation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MsgLen {
    rpc_length: u32,
    msg_len: u32,
    elements: u32,
}

impl MsgLen {
    /// Validate a declared `rpc.length` against the transport's bounds.
    ///
    /// # Errors
    ///
    /// [`GspFault::MsgLenOutOfRange`].
    pub fn new(
        rpc_length: u32,
        layout: &ElementLayout,
        element_size_min: u32,
        element_size_max: u32,
    ) -> Result<MsgLen, GspFault> {
        let hdr = u32::try_from(layout.hdr_size()).unwrap_or(u32::MAX);
        let envelope = u32::try_from(RpcEnvelope::SIZE).unwrap_or(u32::MAX);
        let max = element_size_max.saturating_sub(hdr);
        if rpc_length < envelope || rpc_length > max {
            return Err(GspFault::MsgLenOutOfRange {
                declared: rpc_length,
                min: envelope,
                max,
            });
        }
        let msg_len = hdr.saturating_add(rpc_length);
        Ok(MsgLen {
            rpc_length,
            msg_len,
            elements: bytes_to_elements(msg_len, element_size_min),
        })
    }

    /// The declared `rpc.length`.
    #[must_use]
    pub fn rpc_length(self) -> u32 {
        self.rpc_length
    }

    /// `queueElementHdrSize + rpc.length` — the checksum's coverage.
    #[must_use]
    pub fn msg_len(self) -> u32 {
        self.msg_len
    }

    /// How many elements the message occupies.
    #[must_use]
    pub fn elements(self) -> u32 {
        self.elements
    }
}

/// The RPC envelope, as we emit it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingRpc {
    /// `function` — an `NV_VGPU_MSG_FUNCTION_*` / `NV_VGPU_MSG_EVENT_*` id.
    pub function: u32,
    /// `sequence` — the transaction id. A reply echoes the request's.
    pub sequence: u32,
    /// `rpc_result`.
    pub rpc_result: u32,
    /// `rpc_result_private` — the driver reads this one too, so it is set explicitly
    /// rather than left zero (`C: src/qemu/nvkvm_gpu_emul.c:1584-1586` sets both).
    pub rpc_result_private: u32,
    /// The message body after the 32-byte envelope.
    pub payload: Vec<u8>,
}

/// Word index of `length` within the RPC envelope — the third of its eight `u32`s
/// (`ogkm: src/nvidia/generated/g_rpc-message-header.h:41-52`).
///
/// ★ **OWED TO `kayfabe-abi`**, like [`encode_envelope`]: that crate owns the layout and
/// exposes a whole-envelope decoder, but a *two-phase* receive needs this one field out of
/// a buffer that is deliberately too short for the whole message — the driver's own first
/// step is exactly that (`gspMsgQueueGetRpcMessageLength` on element 0,
/// `ogkm: message_queue_cpu.c:684-702`). Validating the envelope here instead would refuse
/// every multi-element message, because the declared length exceeds the one element read
/// so far.
const ENVELOPE_LENGTH_WORD: usize = 2;

/// Encode the RPC envelope into `out[..32]`.
///
/// ★ **OWED TO `kayfabe-abi`.** That crate owns `rpc_message_header_v03_00` and decodes
/// it (`DriverAbiTable::decode_rpc_envelope`); it has no *encoder*, and this crate may
/// not add one to it. So the write is here, expressed as eight little-endian words in the
/// generated struct's field order rather than as transcribed offsets, and it is pinned by
/// a round-trip through that decoder — the generated layout is the oracle, not a comment.
/// The proper home is `kayfabe-abi`.
///
/// `length` is `32 + payload.len()`, i.e. **32** for a bare header. The C writes 36
/// (`C:1586`) with a comment claiming it is the header's size; the same file uses 32 in
/// two other places (`C:1637`, `C:1657`) and `sizeof(rpc_message_header_v03_00)` is 32
/// (`ogkm: src/nvidia/generated/g_rpc-message-header.h:41-52`). That is authorised
/// deviation **GSP-D1**.
fn encode_envelope(out: &mut [u8], header_version: u32, rpc: &OutgoingRpc, length: u32) {
    let words = [
        header_version,
        RpcEnvelope::SIGNATURE_VALID,
        length,
        rpc.function,
        rpc.rpc_result,
        rpc.rpc_result_private,
        rpc.sequence,
        0, // the `u` union (`spare`/`cpuRmGfid`)
    ];
    for (i, v) in words.into_iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
}

/// Build the full element run for one message: `elements * element_size` bytes, zeroed,
/// with every fixed field and a checksum that folds the whole thing to zero.
///
/// The steps are the driver's own send path (`ogkm: message_queue_cpu.c:487-546`):
/// bound the length, zero-pad, fill the transport headers, stamp `seqNum`, zero
/// `checkSum`, fold, store.
///
/// # Errors
///
/// [`GspFault::MsgLenOutOfRange`] if the payload does not fit the transport.
pub fn encode_message(
    layout: &ElementLayout,
    header_version: u32,
    element_size: u32,
    element_size_max: u32,
    seq_num: u32,
    rpc: &OutgoingRpc,
) -> Result<Vec<u8>, GspFault> {
    let rpc_length = u32::try_from(RpcEnvelope::SIZE + rpc.payload.len()).map_err(|_| {
        GspFault::MsgLenOutOfRange {
            declared: u32::MAX,
            min: u32::try_from(RpcEnvelope::SIZE).unwrap_or(u32::MAX),
            max: element_size_max,
        }
    })?;
    let len = MsgLen::new(rpc_length, layout, element_size, element_size_max)?;

    let total = (len.elements() as usize) * (element_size as usize);
    let mut buf = vec![0u8; total];

    if let TransportHdr::Mctp {
        header_off,
        header_word,
        nvdm_off,
        nvdm_word,
    } = layout.transport()
    {
        buf[header_off..header_off + 4].copy_from_slice(&header_word.to_le_bytes());
        buf[nvdm_off..nvdm_off + 4].copy_from_slice(&nvdm_word.to_le_bytes());
    }
    let so = layout.seqnum_off();
    buf[so..so + 4].copy_from_slice(&seq_num.to_le_bytes());
    if let Some(eo) = layout.elem_count_off() {
        buf[eo..eo + 4].copy_from_slice(&len.elements().to_le_bytes());
    }

    let hdr = layout.hdr_size();
    encode_envelope(&mut buf[hdr..], header_version, rpc, rpc_length);
    buf[hdr + RpcEnvelope::SIZE..hdr + RpcEnvelope::SIZE + rpc.payload.len()]
        .copy_from_slice(&rpc.payload);

    // checkSum is included in its own coverage, so it is zeroed first (it already is)
    // and then set to the fold — after which the whole element folds to 0.
    let sum = checksum32(&buf, len.msg_len() as usize);
    let co = layout.checksum_off();
    buf[co..co + 4].copy_from_slice(&sum.to_le_bytes());
    Ok(buf)
}

/// A command decoded out of the guest's command queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingRpc {
    /// The element sequence number it carried.
    pub seq_num: u32,
    /// The validated envelope.
    pub envelope: RpcEnvelope,
    /// How many elements it occupied.
    pub elements: u32,
    /// The message body after the envelope.
    pub payload: Vec<u8>,
}

/// Read the declared length out of a message's **first** element.
///
/// This is step 1 of the driver's own two-phase receive: read element 0, derive
/// `nElements` from `hdrSize + rpc.length`, then read the rest
/// (`ogkm: message_queue_cpu.c:684-702`). The extent of the second read therefore comes
/// from the first copy and is bounded by `queueElementSizeMax` — which is exactly the
/// shape `gl11_region_arguments.md` §2.1 item 4 permits, and it is why the port can read
/// continuation elements (which the C skips, `C:3341-3350`) without needing a region lock.
///
/// # Errors
///
/// [`GspFault::Truncated`] or [`GspFault::MsgLenOutOfRange`]. The envelope itself is
/// validated later, by [`decode_message`], once the whole run has been read.
pub fn peek_len(
    layout: &ElementLayout,
    first: &[u8],
    element_size: u32,
    element_size_max: u32,
) -> Result<MsgLen, GspFault> {
    let hdr = layout.hdr_size();
    let at = hdr + ENVELOPE_LENGTH_WORD * 4;
    let bytes = first.get(at..at + 4).ok_or(GspFault::Truncated {
        need: at + 4,
        have: first.len(),
    })?;
    let mut b = [0u8; 4];
    b.copy_from_slice(bytes);
    MsgLen::new(
        u32::from_le_bytes(b),
        layout,
        element_size,
        element_size_max,
    )
}

/// Verify and decode a complete element run.
///
/// The checks are the driver's, in the driver's order
/// (`ogkm: message_queue_cpu.c:710-788`): checksum folds to zero, then the transport
/// headers, then `seqNum == rxSeqNum`.
///
/// # Errors
///
/// [`GspFault::ChecksumMismatch`], [`GspFault::TransportHeaderInvalid`],
/// [`GspFault::SeqNumGap`], [`GspFault::Truncated`], [`GspFault::Envelope`].
pub fn decode_message(
    layout: &ElementLayout,
    run: &[u8],
    len: MsgLen,
    expect_seq: u32,
    abi: &kayfabe_abi::versions::DriverAbiTable,
) -> Result<IncomingRpc, GspFault> {
    if run.len() < len.msg_len() as usize {
        return Err(GspFault::Truncated {
            need: len.msg_len() as usize,
            have: run.len(),
        });
    }
    let folded = checksum32(run, len.msg_len() as usize);
    if folded != 0 {
        return Err(GspFault::ChecksumMismatch { folded });
    }
    if let TransportHdr::Mctp {
        header_off,
        header_word,
        nvdm_off,
        nvdm_word,
    } = layout.transport()
    {
        for (offset, expected) in [(header_off, header_word), (nvdm_off, nvdm_word)] {
            let mut b = [0u8; 4];
            b.copy_from_slice(&run[offset..offset + 4]);
            let found = u32::from_le_bytes(b);
            if found != expected {
                return Err(GspFault::TransportHeaderInvalid {
                    offset,
                    found,
                    expected,
                });
            }
        }
    }
    let mut b = [0u8; 4];
    b.copy_from_slice(&run[layout.seqnum_off()..layout.seqnum_off() + 4]);
    let seq_num = u32::from_le_bytes(b);
    if seq_num != expect_seq {
        return Err(GspFault::SeqNumGap {
            expected: expect_seq,
            got: seq_num,
        });
    }

    let hdr = layout.hdr_size();
    let body = &run[hdr..len.msg_len() as usize];
    let envelope = abi.decode_rpc_envelope(body)?;
    let payload = body[RpcEnvelope::SIZE..].to_vec();
    Ok(IncomingRpc {
        seq_num,
        envelope,
        elements: len.elements(),
        payload,
    })
}

kayfabe_util::assert_send_sync!(
    ElementLayout,
    TransportHdr,
    MsgLen,
    OutgoingRpc,
    IncomingRpc
);
