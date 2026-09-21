//! Element layout as a **descriptor**, not a version branch — §1.2's worked example of the `Dg`
//! (guest-driver) compatibility axis.
//!
//! ## ★★★ Why this file is the pattern every axis-carrying fact should copy
//!
//! §1.2: the per-element header is *"the single best worked example of the `Dg` axis in this
//! tree"*:
//!
//! | | 580.159.04 | 610.43.02 |
//! |---|---|---|
//! | header size | **48 bytes** | **16 bytes** |
//! | fields | `authTagBuffer[16]`, `aadBuffer[16]`, `checkSum@32`, `seqNum@36`, `elemCount@40` | `mctpHeader`, `nvdmHeader`, `checkSum@8`, `seqNum@12` |
//!
//! ⇒ **The size and every field offset changed between two versions we must both support, and
//! `elemCount` DISAPPEARED.** ⚠ A `if version >= 610 { … } else { … }` branch would work once and
//! then multiply: the next version adds a third arm at every site that reads a field. A
//! descriptor adds a row.
//!
//! ⊘ Note `elem_count_off: Option<usize>` — the axis is expressed as a field that **may not
//! exist**, which a branch cannot represent without a sentinel that later reads as a real offset.
//!
//! ## ⊘⊘⊘ AND THE ROW USED TO BE SELECTED BY A GUESS (fable w824, MEDIUM 3)
//!
//! `layout_for(driver_major)` picked `LAYOUT_610` for `driver_major >= 595`. **595 came from
//! nowhere** — and worse, the tree already held the measurement that contradicts it: a nine-tag
//! probe (`docs/archive/mode2_gsp_port_plan.md:2001,2035-2039`) put the break at
//! **`(595.84, 610.43.02]`** — 575, 580, 590 **and 595.84 are all on the 48-byte side**. So the
//! guess would have handed a 595.x guest the 16-byte header. ⊘ *Check whether the question is
//! already answered* — it was, in the archive, and a threshold nobody cited overrode it.
//! ⇒ Replaced by
//! [`detect_layout`], which reads the answer off the element itself: a 610 element's word 0 is an
//! MCTP transport header whose `MCTP_HEADER_VERSION 3:0` is **built as 1** and **validated as 1
//! on receive**; a 580 element's word 0 is `authTagBuffer[0..4]`, zero unless CC encryption is on.

/// One driver version's element framing. ⊘ Data, not code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElementLayout {
    pub header_bytes: usize,
    pub checksum_off: usize,
    pub seq_num_off: usize,
    /// ⊘ `None` where the field does not exist. §1.2: it vanished between 580 and 610.
    pub elem_count_off: Option<usize>,
    pub transport: TransportHdr,
}

/// The transport prefix, which also differs by version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportHdr {
    /// 580: `authTagBuffer[16]` + `aadBuffer[16]`, no MCTP.
    None,
    /// 610: MCTP + NVDM headers.
    Mctp { mctp_off: usize, nvdm_off: usize },
}

/// ⊘ The two versions this tree supports, as rows.
/// §50 level 2: `ogkm-580.159.04/src/nvidia/inc/kernel/gpu/gsp/message_queue_priv.h:43-50`.
pub const LAYOUT_580: ElementLayout = ElementLayout {
    header_bytes: 48,
    checksum_off: 32,
    seq_num_off: 36,
    elem_count_off: Some(40),
    transport: TransportHdr::None,
};

/// §50 level 2: `ogkm/src/nvidia/inc/kernel/gpu/gsp/message_queue_priv.h:52-58`.
pub const LAYOUT_610: ElementLayout = ElementLayout {
    header_bytes: 16,
    checksum_off: 8,
    seq_num_off: 12,
    // ★ The field is gone. Not zero, not sentinel — gone.
    elem_count_off: None,
    transport: TransportHdr::Mctp { mctp_off: 0, nvdm_off: 4 },
};

/// `MCTP_HEADER_VERSION 3:0` — `src/nvidia/arch/nvalloc/common/inc/mctp_format.h:40`. Built as
/// `REF_NUM(MCTP_HEADER_VERSION, 0x1)` (`:79`, *"Header version is hard-coded to 0x1"*) and
/// rejected on receive unless `== 0x1` (`message_queue_cpu.c:739-746`).
pub const MCTP_HEADER_VERSION_MASK: u32 = 0xF;
pub const MCTP_HEADER_VERSION_1: u32 = 0x1;
/// `MCTP_HEADER_SOM 31:31`, `MCTP_HEADER_EOM 30:30` (`mctp_format.h:48,47`); both set to 1 by
/// `gspMsgQueueSendCommand` (`message_queue_cpu.c:505-511`).
pub const MCTP_HEADER_SOM: u32 = 1 << 31;
pub const MCTP_HEADER_EOM: u32 = 1 << 30;
/// Word 1: `MCTP_MSG_HEADER_TYPE 6:0` = `_VENDOR_PCI 0x7e`, `MCTP_MSG_HEADER_VENDOR_ID 23:8` =
/// `_NV 0x10de`, `MCTP_MSG_HEADER_NVDM_TYPE 31:24` = `NVDM_TYPE_RM_RPC 0x25`
/// (`mctp_format.h:51-57`, `nvdm_format.h:61`); vendor id validated at `message_queue_cpu.c:750`.
pub const MCTP_MSG_HEADER_TYPE_VENDOR_PCI: u32 = 0x7e;
pub const MCTP_MSG_HEADER_VENDOR_ID_NV: u32 = 0x10de;
pub const NVDM_TYPE_RM_RPC: u32 = 0x25;

/// The exact two words `gspMsgQueueSendCommand` writes at 610 — what a 610 element starts with.
/// ⊘ Used by tests to build a known-positive; not a value we serve.
pub const MCTP_WORDS_610: [u32; 2] = [
    MCTP_HEADER_VERSION_1 | MCTP_HEADER_SOM | MCTP_HEADER_EOM,
    MCTP_MSG_HEADER_TYPE_VENDOR_PCI | (MCTP_MSG_HEADER_VENDOR_ID_NV << 8) | (NVDM_TYPE_RM_RPC << 24),
];

/// ★ Select the row from the ELEMENT, not from a version number.
///
/// * 610: word 0 carries `MCTP_HEADER_VERSION == 1` **and** word 1 carries the vendor-PCI type
///   with NVIDIA's vendor id — the same three fields ogkm validates or hard-codes on its side.
/// * 580: word 0 is `authTagBuffer[0..4]`. Outside confidential compute it is **zero** — the
///   work area is zeroed at init (`message_queue_cpu.c:141`) and only `ccslEncryptWithRotationChecks`
///   writes the tag (`:499`). ⚠ Under CC the tag is 16 random bytes: a false 610 match needs 27
///   specific bits (P ≈ 2⁻²⁷ per element), and a CC element's payload is ciphertext we could not
///   read anyway.
///
/// ⊘ `None` when there are not even two words to look at: a short buffer is refused, not guessed.
pub fn detect_layout(elem: &[u8]) -> Option<ElementLayout> {
    let w = |o: usize| elem.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let (w0, w1) = (w(0)?, w(4)?);
    let is_mctp = (w0 & MCTP_HEADER_VERSION_MASK) == MCTP_HEADER_VERSION_1
        && (w1 & 0x7f) == MCTP_MSG_HEADER_TYPE_VENDOR_PCI
        && ((w1 >> 8) & 0xffff) == MCTP_MSG_HEADER_VENDOR_ID_NV;
    Some(if is_mctp { LAYOUT_610 } else { LAYOUT_580 })
}

impl ElementLayout {
    /// Read the sequence number. ⊘ Bounds-checked against the buffer, because the element comes
    /// from guest memory and a short buffer must refuse rather than read past it.
    pub fn seq_num(&self, elem: &[u8]) -> Option<u32> {
        let o = self.seq_num_off;
        elem.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn checksum(&self, elem: &[u8]) -> Option<u32> {
        let o = self.checksum_off;
        elem.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// ⊘ Returns `None` **both** when the field does not exist in this version and when the
    /// buffer is short. ★ The caller must treat "this version has no elemCount" as a fact, not as
    /// a read failure — which is why the descriptor carries `Option` rather than a magic value.
    pub fn elem_count(&self, elem: &[u8]) -> Option<u32> {
        let o = self.elem_count_off?;
        elem.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
}

/// ★★★ THE NO-GSP SEAM — a core part of the shape, not a later bolt-on.
///
/// `[owner, 2026-09-21]` *"full compatibility axis compatible (with non gsp for later but not a
/// bolt on, core part)"*.
///
/// ⊘ **Why this must exist now even though the no-GSP plane is not built.** Pre-Turing parts have
/// no GSP at all, and Windows consumer drivers measured on Turing and Ada run with GSP **off**
/// (`THE_WINDOWS_AXIS.md` §1.1). If the control plane is written as *"the GSP RPC path"*, then
/// adding a non-GSP path later means a second copy of every decision. ⇒ The plane a message
/// arrives on is a **parameter of the transport**, and everything above it is written against
/// [`ControlPlane`], not against GSP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlPlane {
    /// Turing and newer with firmware loaded: RPC over the message queues.
    Gsp { layout: ElementLayout },
    /// ⊘ Pre-Turing, and Turing+ with firmware disabled. The driver drives the hardware through
    /// registers directly; there is no message queue and no element header.
    /// ★ nouveau is the standing oracle for this plane (`THE_WINDOWS_AXIS.md` §10).
    NoGsp,
}

impl ControlPlane {
    /// ⊘ The question every caller should ask instead of *"which driver version"*.
    pub fn has_message_queue(&self) -> bool {
        matches!(self, ControlPlane::Gsp { .. })
    }

    /// ⚠ Returns `None` on the no-GSP plane **by construction**, so a caller that assumes an
    /// element layout exists cannot compile against it without handling the absence.
    pub fn element_layout(&self) -> Option<ElementLayout> {
        match self {
            ControlPlane::Gsp { layout } => Some(*layout),
            ControlPlane::NoGsp => None,
        }
    }
}
