//! **S2 — the element codec.** Framing, checksum, multi-element split/join.
//!
//! ## ★ The element header is a different structure between 580 and 610
//!
//! | | 580 (the bench; r535/r570-shaped) | 610 |
//! |---|---|---|
//! | @0 | `authTagBuffer[16]` | `mctpHeader` |
//! | @4 | ↑ | `nvdmHeader` |
//! | @8 | ↑ | `checkSum` |
//! | @12 | ↑ | `seqNum` |
//! | @16 | `aadBuffer[16]` | **payload begins** |
//! | @32 | `checkSum` | — |
//! | @36 | `seqNum` | — |
//! | @40 | **`elemCount`** | `rpc.sequence` (payload + 24) |
//! | @48 | rpc header | — |
//! | header size | **48** | **16** (+ the CC tag where Confidential Compute is on) |
//!
//! [src] read at both endpoints:
//! `ogkm-580: src/nvidia/inc/kernel/gpu/gsp/message_queue_priv.h:43-51` and
//! `ogkm-610: src/nvidia/inc/kernel/gpu/gsp/message_queue_priv.h:52-67`. Independently, the
//! 48-byte form at r535 and r570 — `nv: r535/nvrm/gsp.h:808-816`,
//! `nv: r535/rpc.c:94-102, 119`. So the break is **(595.84, 610.43.02]**, i.e. the
//! predicate is `major >= 610`, and the C — whose 580 implementation matches r535/r570
//! exactly — is right for its era and wrong as a protocol.
//!
//! ★★ The two are not merely differently *shaped*; they differ in **who decides how far
//! the ring moves.** At 580 the receiver reads `elemCount` out of the element and
//! `msgqRxMarkConsumed` advances by it (`ogkm-580: message_queue_cpu.c:652-658, 774`); at
//! 610 there is no such field and the count is derived from `rpc.length`
//! (`ogkm-610: message_queue_cpu.c:698-705`, consumed at `:838`). See
//! [`peek_elem_count`] and [`max_elements`] — the second of which is a **memory-safety
//! bound on what we emit into a guest's kernel**, not a lint.
//!
//! The C hard-codes 48/40/32/36 at ~15 sites. Here the shape is an [`ElementLayout`]
//! **value**, supplied by the ABI layer (`kayfabe_abi::versions::GspElementWire`); this
//! module carries no offset of its own. That is one of the five authorised deviations
//! ("the version key").
//!
//! ## Checksum
//!
//! 64-bit XOR fold, reduced to 32 by `hi ^ lo`
//! (`ogkm-610: message_queue_priv.h:191-209`, `ogkm-580: :106-124`). ★ `_checkSum32` is
//! **byte-identical** at both tags, comment included — the element around it changed
//! shape, the fold did not, so there is no version profile to add for the checksum itself.
//! The routine steps in `NvU64`s `while (p < pEnd)`, i.e. it **reads up to the next
//! 8-byte boundary past `uLen`** — its own comment licenses exactly that — and the sender
//! zero-pads to 8 first (`ogkm-610: message_queue_cpu.c:499-501`, `ogkm-580: :477-479`,
//! likewise identical). Coverage in the plain case is `hdrSize + rpc.length`
//! (`ogkm-610: message_queue_cpu.c:543-546`, `ogkm-580: :517-520`, and the receiver's
//! mirror at `ogkm-610: :723-728`, `ogkm-580: :678-683`); the whole element run when CC is
//! on (`ogkm-610: :540-541`, `ogkm-580: :514-515`). The two tags spell the run differently
//! — 610 `nElements * queueElementSizeMin`, 580 `pCQE->elemCount *
//! GSP_MSG_QUEUE_ELEMENT_SIZE_MIN` — which is the same seam as everywhere else in this
//! file: *who* supplies the element count, not what the checksum covers.
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
    /// (`ogkm-610: message_queue_cpu.c:737-759`: a wrong `MCTP_HEADER_VERSION` or a wrong
    /// NVDM vendor id is `NV_ERR_INVALID_DATA`, *"MCTP protocol violation"*).
    ///
    /// ★★ **This variant exists only at 610.** 580 has no transport header and no such
    /// check anywhere in its receive path — bytes @0–@31 of a 580 element are the CC
    /// `authTagBuffer[16]` and `aadBuffer[16]` (`ogkm-580: message_queue_priv.h:45-46`),
    /// which a CC-off guest never reads. [`TransportHdr::None`] is therefore *correct* for
    /// the bench and must not be "fixed" into a placeholder MCTP pair.
    ///
    /// ⚠ 610 validates **only** those two fields: `REF_VAL(MCTP_HEADER_VERSION, …) == 1`
    /// and `REF_VAL(MCTP_MSG_HEADER_VENDOR_ID, …) == 0x10de`. SOM, EOM, SEID/DEID/SEQ and
    /// the NVDM *type* byte are read by nothing (`ogkm-610: message_queue_cpu.c:739-758`),
    /// so no test may assert that a guest rejects a wrong SOM/EOM/SEQ/NVDM-type — that
    /// would pin a behaviour the driver does not have.
    ///
    /// The words are carried whole rather than as bit fields: the guest's encoder builds
    /// them from fixed arguments — `mctpCreateTransportHeader(SOM=1, EOM=1, 0, 0, 0)` and
    /// `mctpCreateNvdmHeader(NVDM_TYPE_RM_RPC)` (`ogkm-610: message_queue_cpu.c:505-512`;
    /// neither helper nor `mctp_format.h` exists at 580) — so
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
    /// `hdr_size` is the **effective** header size. ★ The two tags reach it differently and
    /// both readings land here as one value:
    ///
    /// - **610** — a runtime `queueElementHdrSize`,
    ///   `NV_OFFSETOF(GSP_MSG_QUEUE_ELEMENT, payload)` **plus**
    ///   `sizeof(GSP_MSG_QUEUE_ENCRYPTION_TAG)` where CC is on
    ///   (`ogkm-610: message_queue_cpu.c:82-86`), and also declared to us in the guest's
    ///   init args.
    /// - **580** — a compile-time `NV_OFFSETOF(GSP_MSG_QUEUE_ELEMENT, rpc)` = 48, with the
    ///   CC `authTagBuffer`/`aadBuffer` *inside* that header rather than appended to it
    ///   (`ogkm-580: message_queue_priv.h:43-51, 93`), so there is nothing to fold in and
    ///   the size does not vary with CC at all.
    ///
    /// It is a computed value at the ABI seam, never a constant here, which is what keeps
    /// CC from needing a second code path on the version that does vary.
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
    /// (`ogkm-610: message_queue_priv.h:52-67`, against 580's own element at
    /// `ogkm-580: :43-51` where `elemCount` really is @40).
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
/// (`ogkm-580: message_queue_priv.h:98-99`, `ogkm-610: message_queue_priv.h:117-121`).
///
/// Returns 0 only for `element_size_min == 0`, which a bound geometry cannot have.
#[must_use]
pub fn bytes_to_elements(bytes: u32, element_size_min: u32) -> u32 {
    if element_size_min == 0 {
        return 0;
    }
    bytes.div_ceil(element_size_min)
}

/// ★★ **GSP-S1 — the receive staging bound.** How many elements one message may occupy
/// before the *guest's* receive path writes past its own allocation.
///
/// The guest copies each element of a run into a staging buffer of exactly
/// `queueElementSizeMax` bytes, advancing by one element per copy, with **no bound on the
/// count**: `_gspMsgQueueInit` allocates
/// `(1 << GSP_MSG_QUEUE_ELEMENT_ALIGN) + GSP_MSG_QUEUE_ELEMENT_SIZE_MAX + msgqGetMetaSize()`
/// from `portMemAllocNonPaged` and carves the live `msgq` metadata **immediately after**
/// the staging area (`ogkm-580: src/nvidia/src/kernel/gpu/gsp/message_queue_cpu.c:132-134,
/// 143-145`); the copy loop is `for (i = 0; i < nElements; i++) { portMemCopy(pTgt, …); pTgt += … }`
/// (`:628, 648-650`) and `nElements` is whatever the element declared. So an over-wide run
/// overwrites the metadata first and then arbitrary kernel heap.
///
/// This is therefore a bound on **what we may emit into a guest's kernel**, not a lint —
/// and it is checked where the extent is decided rather than inferred from the length,
/// because the two are only equal when `element_size` divides `element_size_max`.
///
/// Derived, never declared: on the bench's geometry it evaluates to `65536 / 4096 = 16`,
/// but neither number appears here. `element_size` is the guest's own published `msgSize`.
#[must_use]
pub fn max_elements(element_size: u32, element_size_max: u32) -> u32 {
    if element_size == 0 {
        return 0;
    }
    element_size_max / element_size
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
/// ★ The **lower** bound is the RPC envelope's own size, not zero — and ★★ **the two tags
/// disagree about whether the driver enforces that itself.** Both sanity-check the same
/// quantity in the same two places, send and receive, but against different constants:
///
/// - **610** bounds `msgLen` below by `queueElementHdrSize`
///   (`ogkm-610: message_queue_cpu.c:487-497`, mirror at `:824-833`). So `rpc.length == 0`
///   **passes**, `bytesToElements(hdrSize, 4096) == 1`, and a zero-length message silently
///   consumes an element and then produces garbage upstream. Refusing it here is the
///   authorised "RPC element parsing" deviation.
/// - **580** bounds it by `sizeof(GSP_MSG_QUEUE_ELEMENT)`
///   (`ogkm-580: message_queue_cpu.c:465-475`, mirror at `:760-770`) — the 48-byte header
///   **plus** the 32-byte `rpc_message_header_v` it embeds
///   (`ogkm-580: message_queue_priv.h:43-51`). That is `rpc.length >= 32`, i.e. **the
///   bench's driver already rejects a zero-length message on its own**, and the rule below
///   is not a deviation there but a match.
///
/// Same code either way: the floor is `RpcEnvelope::SIZE`, which is 580's rule exactly and
/// a strict tightening of 610's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MsgLen {
    rpc_length: u32,
    msg_len: u32,
    elements: u32,
    /// ★ Kept rather than recomputed: `elements` is a CEIL of `msg_len`, so the element
    /// size cannot be recovered from the other two — `msg_len / elements` rounds down and
    /// would under-bound the delivered run by up to one element. The one consumer is
    /// `decode_message`'s `delivered` bound, where under-bounding silently truncates an
    /// alloc's params and over-bounding reads past what the guest submitted.
    element_size_min: u32,
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
            element_size_min,
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

    /// `GSP_MSG_QUEUE_ELEMENT_SIZE_MIN` as this message was validated against — the width
    /// one element occupies in the ring.
    #[must_use]
    pub fn element_size_min(self) -> u32 {
        self.element_size_min
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
/// (`ogkm-580: src/nvidia/generated/g_rpc-message-header.h:41-52`, `ogkm-610: :41-52`).
/// ★ That file is **byte-identical at both tags, at the same lines**: the envelope did not
/// move when the element around it did, so nothing keyed on a driver version belongs here.
///
/// ★ **OWED TO `kayfabe-abi`**, like [`encode_envelope`]: that crate owns the layout and
/// exposes a whole-envelope decoder, but a *two-phase* receive needs this one field out of
/// a buffer that is deliberately too short for the whole message. Both drivers do exactly
/// that off element 0 — 610 through `gspMsgQueueGetRpcMessageLength`, to *derive the
/// element count* (`ogkm-610: message_queue_cpu.c:684-702`); 580 by reading
/// `pCmdQueueElement->rpc.length` directly, for the checksum span and the length sanity
/// check (`ogkm-580: :680-682`, `:760`), having taken its element count from the
/// `elemCount` field instead (`ogkm-580: :652-659`). Validating the envelope here instead
/// would refuse every multi-element message, because the declared length exceeds the one
/// element read so far.
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
/// (`ogkm-580: src/nvidia/generated/g_rpc-message-header.h:41-52`, `ogkm-610: :41-52` —
/// identical file, identical lines, eight `u32`s). That is authorised deviation
/// **GSP-D1**.
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
/// The steps are the driver's own send path (`ogkm-610: message_queue_cpu.c:487-546`,
/// `ogkm-580: :465-520`): bound the length, zero-pad, stamp `seqNum`, zero `checkSum`,
/// fold, store. ★ The one step that is version-shaped is the third: 610 fills the
/// **transport headers** there (`ogkm-610: :505-512`) and 580 stamps **`elemCount`**
/// instead (`ogkm-580: :481-483`). The `if let` over [`TransportHdr`] and the `if let` over
/// [`ElementLayout::elem_count_off`] below are precisely those two arms, which is why this
/// one encoder serves both without a version branch.
///
/// # Errors
///
/// [`GspFault::MsgLenOutOfRange`] if the payload does not fit the transport, or
/// [`GspFault::ElementCountOutOfRange`] if the run would overrun the guest's receive
/// staging buffer ([`max_elements`]).
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

    // ★★ GSP-S1. Checked *before* the run is built, and unconditionally — the guest's
    // staging buffer exists on every version, so this is a property of the geometry and
    // not of the element layout, and gating it on `elem_count_off.is_some()` would be a
    // branch on version identity.
    //
    // On the bench's geometry the length bound above already implies this one, and that
    // coincidence is exactly the hazard: the two are equal only when `element_size`
    // divides `element_size_max`, and any future path that sets the count from something
    // other than this `MsgLen` — a continuation record, a CC layout, a replayed trace —
    // breaks the implication silently.
    let max = max_elements(element_size, element_size_max);
    if len.elements() > max {
        return Err(GspFault::ElementCountOutOfRange {
            count: len.elements(),
            max,
        });
    }

    // ★ A zero element size builds a zero-byte run and then stamps fixed fields into it.
    // Found by the `gsp_msgq` fuzz target (2026-08-01): `max_elements(0, _)` is 0 and
    // `bytes_to_elements(_, 0)` is 0, so `elements() > max` is `0 > 0` — false — and the
    // GSP-S1 gate above waves it through into `buf[seqnum_off..+4]` on an empty `Vec`.
    //
    // ⚠ **Not reachable through a bound geometry**: `rx_link_check` refuses
    // `msgSize < MSGQ_MSG_SIZE_MIN` (16) with code `-2`, so `MsgqGeometry` never carries
    // a zero. Refused here anyway for the reason the whole crate refuses by name: the
    // precondition that saves this line lives in a different function, and `encode_message`
    // is `pub`. Spelled as the existing out-of-range fault rather than a new one — a zero
    // element size IS an element count out of range, and inventing a variant for it would
    // split one condition across two names.
    let total = (len.elements() as usize)
        .checked_mul(element_size as usize)
        .filter(|t| *t >= layout.hdr_size())
        .ok_or(GspFault::ElementCountOutOfRange {
            count: len.elements(),
            max,
        })?;
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
    /// The message body after the envelope, as `rpc.length` **declares** it.
    pub payload: Vec<u8>,
    /// ★★★ The message body after the envelope, as the **element run delivers** it — a
    /// superset of [`IncomingRpc::payload`].
    ///
    /// See `crate::RpcCommand::delivered` for the whole argument, which is a protocol fact
    /// rather than a defensive choice: `GSP_RM_ALLOC`'s declared `length` stops where its
    /// flexible `params[]` begins, so an alloc's params live **past** `payload` and inside
    /// the run the guest submitted.
    ///
    /// ⚠ Bounded by the run, never by a length the guest declared: whatever the envelope
    /// says, this stops at `min(run.len(), elements * element_size_min)`.
    pub delivered: Vec<u8>,
}

/// Read the declared length out of a message's **first** element.
///
/// This is step 1 of the driver's own two-phase receive: read element 0, work out how many
/// elements the record occupies, then read the rest. ★★ **How that count is obtained is
/// the version seam**, and this function implements only one half of it: deriving
/// `nElements` from `hdrSize + rpc.length` is **610's** algorithm
/// (`ogkm-610: message_queue_cpu.c:684-702`, consumed at `:838`). At 580 the count is read
/// out of the element's own `elemCount` field (`ogkm-580: :652-659`, consumed at `:774`)
/// and `rpc.length` gates nothing on that path — see [`peek_elem_count`], which is the
/// authority the receive path must prefer where it exists.
///
/// The extent of the second read therefore comes from the first copy and is bounded by
/// `queueElementSizeMax` — which is exactly the shape `gl11_region_arguments.md` §2.1 item
/// 4 permits, and it is why the port can read continuation elements (which the C skips,
/// `C:3341-3350`) without needing a region lock.
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

/// Read the **declared** element count out of a message's first element, on the versions
/// whose layout carries one.
///
/// ★★ This is the authority the receive path must use, and it is not the same number as
/// [`peek_len`]'s derivation. On the command queue the guest is the producer: it writes
/// `pCQE->elemCount = GSP_MSG_QUEUE_BYTES_TO_ELEMENTS(msgLen)` and then advances its own
/// `writePtr` by **that field** (`ogkm-580: message_queue_cpu.c:482, 578`
/// `msgqTxSubmitBuffers(hQueue, pCQE->elemCount)`). So the field, not the length, is how
/// far the producer moved — and a consumer that advances by a derivation desynchronises
/// the ring permanently against a producer that disagrees, with nothing downstream to
/// catch it: the resulting sequence mismatch is `>` , for which the driver's recovery
/// branch does not exist (`ogkm-580: :699-714` handles only `<`).
///
/// `None` where the layout has no such field — at 610 the count is derived from
/// `rpc.length` and offset 40 is `rpc.sequence`
/// (`ogkm-610: message_queue_priv.h:52-67`, `message_queue_cpu.c:698-705`).
///
/// # Errors
///
/// [`GspFault::Truncated`] if the buffer is shorter than the header.
pub fn peek_elem_count(layout: &ElementLayout, first: &[u8]) -> Result<Option<u32>, GspFault> {
    let Some(at) = layout.elem_count_off() else {
        return Ok(None);
    };
    let bytes = first.get(at..at + 4).ok_or(GspFault::Truncated {
        need: at + 4,
        have: first.len(),
    })?;
    let mut b = [0u8; 4];
    b.copy_from_slice(bytes);
    Ok(Some(u32::from_le_bytes(b)))
}

/// Peek the guest's `seqNum` out of a command element without decoding or validating it.
///
/// ★★★★★ **THIS IS THE INSTANCE DISCRIMINATOR** — see `boot.rs`'s PC-D7 block. The guest
/// stamps its CPU-private `txSeqNum` into every command element it submits
/// (`ogkm-580: message_queue_cpu.c:481`, `pCQE->seqNum = pMQI->txSeqNum;`), and that
/// counter is zeroed only at queue construction. So the command stream carries the queue
/// instance's identity **in-band, in the shared region**, and a fresh instance is
/// distinguishable from a continued one without reference to any address.
///
/// ⊘ **Why the obvious discriminator does not exist**: no sequence number crosses the
/// shared region in a header. `msgqTxHeader` is `{version, size, msgSize, msgCount,
/// writePtr, flags, rxHdrOff, entryOff}` and `msgqRxHeader` is `{readPtr}`
/// (`ogkm-580: src/common/shared/msgq/inc/msgq/msgq_priv.h:49-65`) — `rxSeqNum` and
/// `txSeqNum` live in the CPU-private `MESSAGE_QUEUE_INFO` and are unreadable to us. This
/// is why PC-D7 recorded "read the guest's `rxSeqNum` at bind time" as the dissolving
/// discriminator and then could not build it.
///
/// ⚠ **Peek, not decode, and deliberately so.** This runs at bind time, before any
/// service pass, on an element the guest may have written at any point in its life. It
/// must not checksum, must not validate, and must not advance anything — a hostile or
/// simply stale element must be able to produce a *number* without producing a refusal.
/// The number is then used only to choose between "continue" and "reset", and both of
/// those are safe answers.
///
/// # Errors
///
/// [`GspFault::Truncated`] if the buffer is shorter than the header.
pub fn peek_seq_num(layout: &ElementLayout, first: &[u8]) -> Result<u32, GspFault> {
    let at = layout.seqnum_off();
    let bytes = first.get(at..at + 4).ok_or(GspFault::Truncated {
        need: at + 4,
        have: first.len(),
    })?;
    let mut b = [0u8; 4];
    b.copy_from_slice(bytes);
    Ok(u32::from_le_bytes(b))
}

/// Verify and decode a complete element run.
///
/// The checks are the driver's, in the driver's order: checksum folds to zero, then the
/// transport headers, then `seqNum == rxSeqNum` (`ogkm-610: message_queue_cpu.c:710-788`).
/// ★ At 580 the same two surviving checks run in the same order and the middle one is
/// simply absent, because 580 has no transport header to validate
/// (`ogkm-580: :666-719`: checksum at `:666-690`, sequence at `:692-719`). The
/// [`TransportHdr::None`] arm below is that absence, not a skipped check.
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
    // ★★★ The DELIVERED body, which for `GSP_RM_ALLOC` is strictly longer than the
    // declared one — see `crate::RpcCommand::delivered`. Two bounds, and both are
    // necessary: `run.len()` is what the caller actually handed us, and
    // `elements * element_size_min` is what the guest actually submitted. Taking the
    // smaller means a short read can never be extended into memory the caller owns, and a
    // long `run` can never be read past the guest's own element count.
    //
    // ⊘ It is NOT `run.len()` alone. The ring hands over a slice whose length is the
    // caller's business; reading to the end of it would let the size of somebody else's
    // buffer decide how many bytes a guest message contains.
    let run_end = run
        .len()
        .min(len.elements() as usize * len.element_size_min() as usize);
    // `saturating_sub` rather than a bound check: a run shorter than the envelope is
    // already impossible (`MsgLen::new` floors `rpc_length` at `RpcEnvelope::SIZE` and the
    // truncation check above ran), and an arithmetic panic on the hot path would be a
    // worse answer than an empty delivered body.
    let delivered_from = hdr + RpcEnvelope::SIZE;
    let delivered = run
        .get(delivered_from..run_end.max(delivered_from))
        .unwrap_or(&[])
        .to_vec();
    Ok(IncomingRpc {
        seq_num,
        envelope,
        elements: len.elements(),
        payload,
        delivered,
    })
}

kayfabe_util::assert_send_sync!(
    ElementLayout,
    TransportHdr,
    MsgLen,
    OutgoingRpc,
    IncomingRpc
);
