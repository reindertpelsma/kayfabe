//! ★★★★★ **The Translated channel's rewriter** — `THE_TRANSLATED_PLANE.md` §24.2.
//!
//! A kernel CE channel (RM's CeUtils scrub, UVM) runs on **our own** host channel, in a host VA
//! space that mirrors the guest's kernel space. Its pushbuffer is copied, not decoded into
//! intents, and exactly two things are rewritten:
//!
//! 1. **A CE `LAUNCH_DMA` with a PHYSICAL side** — the operand becomes a virtual address in the
//!    identity window (local FB) or the guest-RAM window (sysmem), the type bit flips to
//!    VIRTUAL, and the guest's own offsets are restored right after the launch so a later
//!    launch that reuses them still reads the guest's values. This is RM's own `fbAliasVA`
//!    rewrite (`ogkm-610 channel_utils.c:1055-1056`).
//! 2. **`MEM_OP_A..D`** — privileged host methods naming guest PDB addresses. They are DROPPED
//!    from the forwarded stream; a `MEM_OP_D` TLB invalidate becomes a [`Piece::Invalidate`]
//!    split point, where the worker walks the named root and reconciles before the rest runs.
//!
//! Everything else — semaphores, virtual operands, host methods — is forwarded unchanged.
//!
//! ⊘ Pure: no GPU, no isolate, no table. The output is NORMALISED to one method per header,
//! which is what makes insertion before a launch trivial; it is equivalent method-for-method.

use kayfabe_abi::submit::{MethodForm, ce, method_header_decode, method_header_inc};

/// `NVC56F_MEM_OP_A..D` — `ogkm-580 clc56f.h:133-181`.
const MEM_OP_A: u32 = 0x28;
const MEM_OP_B: u32 = 0x2c;
const MEM_OP_C: u32 = 0x30;
const MEM_OP_D: u32 = 0x34;
/// `NVC56F_MEM_OP_D_OPERATION` 31:27 — `MMU_TLB_INVALIDATE` / `_TARGETED`.
const OP_TLB_INVALIDATE: u32 = 9;
const OP_TLB_INVALIDATE_TARGETED: u32 = 0xa;
/// `NVC7B5_LINE_COUNT` — `clc7b5.h`.
const LINE_COUNT: u32 = 0x41c;
/// `NVC7B5_OFFSET_IN_LOWER` / `OUT_LOWER`.
const OFFSET_IN_LOWER: u32 = 0x404;
const OFFSET_OUT_LOWER: u32 = 0x40c;

/// `NVC7B5_SET_*_PHYS_MODE_TARGET` — `clc7b5.h:67-71`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// `LOCAL_FB` (0) — a GPGA offset.
    LocalFb,
    /// `COHERENT_SYSMEM` (1) — a guest-physical address.
    CoherentSysmem,
    /// `NONCOHERENT_SYSMEM` (2).
    NonCoherentSysmem,
    /// `PEERMEM` (3) — meaningless for a single-GPU guest; refused.
    Peer,
}

impl Target {
    const fn from_bits(v: u32) -> Self {
        match v & ce::PHYS_MODE_TARGET_MASK {
            0 => Self::LocalFb,
            1 => Self::CoherentSysmem,
            2 => Self::NonCoherentSysmem,
            _ => Self::Peer,
        }
    }
}

/// Where a physical operand lives in the host VA space. Implemented by the worker from the two
/// window bases and the VMM's guest-RAM layout.
pub trait Window {
    /// The host VA of `[phys, phys+len)` in `target`, or `None` if it is not one contiguous
    /// range of a window (refused by name by the caller).
    fn translate(&self, target: Target, phys: u64, len: u64) -> Option<u64>;
}

/// The CE engine state a channel builds up across doorbells — the registers the rewriter needs
/// to see to translate a launch. One per channel, kept by the caller.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CeState {
    /// Subchannels bound to a CE class, as a bit mask.
    pub ce_subch: u8,
    /// `OFFSET_IN` as the guest wrote it.
    pub off_in: u64,
    /// `OFFSET_OUT` as the guest wrote it.
    pub off_out: u64,
    /// `LINE_LENGTH_IN`.
    pub line_len: u32,
    /// `LINE_COUNT`.
    pub line_count: u32,
    /// `SET_SRC_PHYS_MODE` target bits.
    pub src_mode: u32,
    /// `SET_DST_PHYS_MODE` target bits.
    pub dst_mode: u32,
    /// `MEM_OP_C` as last written — the low PDB bits and `PDB_ALL` of a following `MEM_OP_D`.
    pub mem_op_c: u32,
}

/// One piece of rewritten work, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    /// Method words to submit on our host channel.
    Words(Vec<u32>),
    /// The guest invalidated a VA space here: walk and reconcile before what follows runs.
    /// `pdb == None` is `PDB_ALL`.
    Invalidate {
        /// The named root (guest FB/phys address), or `None` for all spaces.
        pdb: Option<u64>,
    },
}

/// Why a segment could not be forwarded. Refused by name, never guessed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// A header this codec cannot decode.
    BadHeader { at: usize, word: u32 },
    /// A method's arguments run past the segment.
    Truncated { at: usize },
    /// A physical operand in `PEERMEM`.
    PeerOperand,
    /// A physical operand no window covers contiguously.
    Untranslatable { target: Target, phys: u64, len: u64 },
}

/// A CE class id? Supplied by the caller from the chip's class table.
pub type IsCeClass = fn(u32) -> bool;

/// ★ **Rewrite one pushbuffer segment.** `st` carries CE state across calls for this channel.
///
/// # Errors
/// [`Refusal`], by name.
pub fn rewrite(
    words: &[u32],
    is_ce: IsCeClass,
    st: &mut CeState,
    w: &dyn Window,
) -> Result<Vec<Piece>, Refusal> {
    let mut out: Vec<Piece> = Vec::new();
    let mut cur: Vec<u32> = Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        let hw = words[i];
        let Some(h) = method_header_decode(hw) else {
            return Err(Refusal::BadHeader { at: i, word: hw });
        };
        i += 1;
        let (addrs_vals, consumed): (Vec<(u32, u32)>, usize) = match h.form {
            MethodForm::EndPbSegment => break,
            MethodForm::Immediate => (vec![(h.method, h.immd)], 0),
            MethodForm::SubDeviceMask => {
                // No arguments; carries no state we translate. Forwarded as written.
                cur.push(hw);
                continue;
            }
            MethodForm::Incrementing | MethodForm::Legacy => {
                let n = h.arg_words;
                let args = words.get(i..i + n).ok_or(Refusal::Truncated { at: i })?;
                (
                    args.iter()
                        .enumerate()
                        .map(|(k, &v)| (h.method + 4 * k as u32, v))
                        .collect(),
                    n,
                )
            }
            MethodForm::NonIncrementing => {
                let n = h.arg_words;
                let args = words.get(i..i + n).ok_or(Refusal::Truncated { at: i })?;
                (args.iter().map(|&v| (h.method, v)).collect(), n)
            }
            MethodForm::IncrementOnce => {
                let n = h.arg_words;
                let args = words.get(i..i + n).ok_or(Refusal::Truncated { at: i })?;
                (
                    args.iter()
                        .enumerate()
                        .map(|(k, &v)| (if k == 0 { h.method } else { h.method + 4 }, v))
                        .collect(),
                    n,
                )
            }
        };
        i += consumed;
        let sub = h.subchannel;
        for (m, v) in addrs_vals {
            one_write(&mut out, &mut cur, is_ce, st, w, sub, m, v)?;
        }
    }
    if !cur.is_empty() {
        out.push(Piece::Words(cur));
    }
    Ok(out)
}

fn emit(cur: &mut Vec<u32>, sub: u32, m: u32, v: u32) {
    // `method_header_inc` refuses only an out-of-range method or subchannel, and every method
    // here came out of a header that decoded, so both are in range.
    if let Some(h) = method_header_inc(sub, m, 1) {
        cur.push(h);
        cur.push(v);
    }
}

#[allow(clippy::too_many_arguments)]
fn one_write(
    out: &mut Vec<Piece>,
    cur: &mut Vec<u32>,
    is_ce: IsCeClass,
    st: &mut CeState,
    w: &dyn Window,
    sub: u32,
    m: u32,
    v: u32,
) -> Result<(), Refusal> {
    // ── host methods (any subchannel, below 0x100) ─────────────────────────────────────────
    if m == 0 {
        // SET_OBJECT: track which subchannels hold a CE.
        let bit = 1u8 << (sub & 7);
        if is_ce(v & 0xFFFF) {
            st.ce_subch |= bit;
        } else {
            st.ce_subch &= !bit;
        }
        emit(cur, sub, m, v);
        return Ok(());
    }
    if m == MEM_OP_A || m == MEM_OP_B {
        return Ok(()); // dropped: privileged, and consumed by the split point below
    }
    if m == MEM_OP_C {
        st.mem_op_c = v;
        return Ok(());
    }
    if m == MEM_OP_D {
        let op = v >> 27;
        if op == OP_TLB_INVALIDATE || op == OP_TLB_INVALIDATE_TARGETED {
            if !cur.is_empty() {
                out.push(Piece::Words(std::mem::take(cur)));
            }
            let all = st.mem_op_c & 1 != 0;
            let lo = u64::from(st.mem_op_c & 0xFFFF_F000);
            let hi = u64::from(v & 0x07FF_FFFF) << 32;
            out.push(Piece::Invalidate {
                pdb: (!all).then_some(hi | lo),
            });
        }
        return Ok(()); // every MEM_OP_D is privileged; only the invalidate means anything to us
    }
    let is_ce_sub = st.ce_subch & (1u8 << (sub & 7)) != 0;
    if !is_ce_sub {
        emit(cur, sub, m, v);
        return Ok(());
    }
    // ── CE methods ─────────────────────────────────────────────────────────────────────────
    match m {
        ce::OFFSET_IN_UPPER => st.off_in = (st.off_in & 0xFFFF_FFFF) | (u64::from(v & 0x1_FFFF) << 32),
        OFFSET_IN_LOWER => st.off_in = (st.off_in & !0xFFFF_FFFF) | u64::from(v),
        ce::OFFSET_OUT_UPPER => {
            st.off_out = (st.off_out & 0xFFFF_FFFF) | (u64::from(v & 0x1_FFFF) << 32);
        }
        OFFSET_OUT_LOWER => st.off_out = (st.off_out & !0xFFFF_FFFF) | u64::from(v),
        ce::LINE_LENGTH_IN => st.line_len = v,
        LINE_COUNT => st.line_count = v,
        ce::SET_SRC_PHYS_MODE => st.src_mode = v,
        ce::SET_DST_PHYS_MODE => st.dst_mode = v,
        ce::LAUNCH_DMA => {
            let src_phys = v & ce::LAUNCH_SRC_PHYSICAL != 0;
            let dst_phys = v & ce::LAUNCH_DST_PHYSICAL != 0;
            if src_phys || dst_phys {
                return launch_translated(cur, st, w, sub, v, src_phys, dst_phys);
            }
        }
        _ => {}
    }
    emit(cur, sub, m, v);
    Ok(())
}

fn span(st: &CeState, v: u32) -> u64 {
    let lines = if v & ce::LAUNCH_MULTI_LINE_ENABLE != 0 {
        u64::from(st.line_count.max(1))
    } else {
        1
    };
    // ⊘ Pitch-linear multi-line spans are bounded by len × lines only when pitch == len; a
    // wider pitch is bounded by the translate call refusing a range no window covers.
    u64::from(st.line_len).saturating_mul(lines).max(1)
}

fn xlate(w: &dyn Window, mode: u32, phys: u64, len: u64) -> Result<u64, Refusal> {
    let t = Target::from_bits(mode);
    if t == Target::Peer {
        return Err(Refusal::PeerOperand);
    }
    w.translate(t, phys, len)
        .ok_or(Refusal::Untranslatable { target: t, phys, len })
}

fn put_offset(cur: &mut Vec<u32>, sub: u32, upper: u32, va: u64) {
    emit(cur, sub, upper, ((va >> 32) & 0x1_FFFF) as u32);
    emit(cur, sub, upper + 4, (va & 0xFFFF_FFFF) as u32);
}

fn launch_translated(
    cur: &mut Vec<u32>,
    st: &CeState,
    w: &dyn Window,
    sub: u32,
    v: u32,
    src_phys: bool,
    dst_phys: bool,
) -> Result<(), Refusal> {
    let len = span(st, v);
    let transfer = v & ce::LAUNCH_TRANSFER_MASK != 0;
    if src_phys && transfer {
        let va = xlate(w, st.src_mode, st.off_in, len)?;
        put_offset(cur, sub, ce::OFFSET_IN_UPPER, va);
    }
    if dst_phys {
        let va = xlate(w, st.dst_mode, st.off_out, len)?;
        put_offset(cur, sub, ce::OFFSET_OUT_UPPER, va);
    }
    emit(
        cur,
        sub,
        ce::LAUNCH_DMA,
        v & !(ce::LAUNCH_SRC_PHYSICAL | ce::LAUNCH_DST_PHYSICAL),
    );
    // Restore the guest's own offsets: a later VIRTUAL launch that does not rewrite them must
    // still read what the guest wrote, not our window address.
    if src_phys && transfer {
        put_offset(cur, sub, ce::OFFSET_IN_UPPER, st.off_in);
    }
    if dst_phys {
        put_offset(cur, sub, ce::OFFSET_OUT_UPPER, st.off_out);
    }
    Ok(())
}
