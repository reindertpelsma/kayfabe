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
pub const LAYOUT_580: ElementLayout = ElementLayout {
    header_bytes: 48,
    checksum_off: 32,
    seq_num_off: 36,
    elem_count_off: Some(40),
    transport: TransportHdr::None,
};

pub const LAYOUT_610: ElementLayout = ElementLayout {
    header_bytes: 16,
    checksum_off: 8,
    seq_num_off: 12,
    // ★ The field is gone. Not zero, not sentinel — gone.
    elem_count_off: None,
    transport: TransportHdr::Mctp { mctp_off: 0, nvdm_off: 4 },
};

/// Select by guest driver major version. ⊘ One place, so adding a version is one row plus one arm
/// **here**, never a branch at every read site.
pub fn layout_for(driver_major: u32) -> ElementLayout {
    if driver_major >= 595 { LAYOUT_610 } else { LAYOUT_580 }
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
