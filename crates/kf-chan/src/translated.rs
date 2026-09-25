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
//! 2. **`MEM_OP_A..D`** — a `MEM_OP_D` TLB invalidate is a privileged host method naming a guest
//!    PDB address: it is DROPPED from the forwarded stream and becomes a [`Piece::Invalidate`]
//!    split point, where the worker walks the named root and reconciles before the rest runs.
//!    ★ P6b ruling (d): a `MEM_OP_D` **MEMBAR** is FORWARDED (A, B, C, D as the guest wrote them):
//!    it is an ordering method, not a privileged one — RM names the privileged host methods as
//!    exactly `TLB_INVALIDATE` and `ACCESS_COUNTER_CLR` (`ogkm-580 alloc_channel.h:207-214`,
//!    `NVOS04_FLAGS_CHANNEL_DENY_AUTH_LEVEL_PRIV`), so our unprivileged host channel may execute
//!    it, and dropping it would let the guest's later work (a semaphore release UVM orders behind
//!    it, `uvm_pascal_host.c:32-45` / `uvm_hal.c:938`) overtake writes it ordered. An invalidate
//!    that asked for a `SYSMEMBAR` (`MEM_OP_A` 11:11) keeps it the same way: a forwarded MEMBAR
//!    (`SYS_MEMBAR`) ahead of the split. Other `MEM_OP_D` operations (`L2_*`,
//!    `ACCESS_COUNTER_CLR`, Hopper's `MMU_OPERATION`) are still consumed here, unforwarded.
//!
//! Everything else — semaphores, virtual operands, host methods — is forwarded unchanged.
//!
//! ⊘ Pure: no GPU, no isolate, no table. The output is NORMALISED to one method per header,
//! which is what makes insertion before a launch trivial; it is equivalent method-for-method.

use kf_abi::submit::{MethodForm, ce, method_header_decode, method_header_inc};

/// `NVC56F_MEM_OP_A..D` — `ogkm-580 clc56f.h:133-181`.
const MEM_OP_A: u32 = 0x28;
const MEM_OP_B: u32 = 0x2c;
const MEM_OP_C: u32 = 0x30;
const MEM_OP_D: u32 = 0x34;
/// `NVC56F_MEM_OP_D_OPERATION` 31:27 — `MEMBAR` (`clc56f.h:184`; the same value in every
/// family's channel class: `clc46f.h:179`, `clc86f.h:118`).
const OP_MEMBAR: u32 = 5;
/// `NVC56F_MEM_OP_A_TLB_INVALIDATE_SYSMEMBAR` 11:11 (`clc56f.h`).
const MEM_OP_A_SYSMEMBAR_EN: u32 = 1 << 11;
/// `NVC56F_MEM_OP_C_MEMBAR_TYPE_SYS_MEMBAR` (2:0 = 0).
const MEMBAR_TYPE_SYS: u32 = 0;
/// `NVC56F_MEM_OP_D_OPERATION` 31:27 — `MMU_TLB_INVALIDATE` / `_TARGETED`.
const OP_TLB_INVALIDATE: u32 = 9;
const OP_TLB_INVALIDATE_TARGETED: u32 = 0xa;
/// `NVC7B5_LINE_COUNT` — `clc7b5.h`.
const LINE_COUNT: u32 = 0x41c;
/// `NVC7B5_OFFSET_IN_LOWER` / `OUT_LOWER`.
const OFFSET_IN_LOWER: u32 = 0x404;
const OFFSET_OUT_LOWER: u32 = 0x40c;
/// `NVC7B5_PITCH_IN` / `PITCH_OUT` — `clc7b5.h:169-172`.
const PITCH_IN: u32 = 0x410;
const PITCH_OUT: u32 = 0x414;
/// `NVC7B5_SET_REMAP_COMPONENTS_NUM_SRC_COMPONENTS` 21:20, size-minus-one (`clc7b5.h:219-223`).
const REMAP_NUM_SRC_SHIFT: u32 = 20;

/// `GP100_UVM_SW` — the software class UVM binds on a subchannel of its kernel channels
/// (`ogkm-580: clc076.h:33`, `uvm_pascal_host.c:317`). No engine executes it: on bare metal its
/// methods trap to RM. ⇒ It never reaches our host channel.
pub const GP100_UVM_SW: u32 = 0xc076;
/// `NVC076_NO_OPERATION` (`clc076.h:36`).
const SW_NO_OPERATION: u32 = 0x100;

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
    /// Subchannels a `SET_OBJECT` bound to a CE class, as a bit mask. ⊘ Recorded, NOT the routing
    /// rule: every hardware subchannel of a CE channel reaches the CE (see `one_write`).
    pub ce_subch: u8,
    /// Subchannels bound to `GP100_UVM_SW`, as a bit mask — consumed here, never forwarded.
    pub sw_subch: u8,
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
    /// `PITCH_IN` — the source line stride in bytes when multi-line.
    pub pitch_in: u32,
    /// `PITCH_OUT` — the destination line stride in bytes when multi-line.
    pub pitch_out: u32,
    /// `SET_REMAP_COMPONENTS` — with `REMAP_ENABLE`, `LINE_LENGTH_IN` counts ELEMENTS of
    /// `component_size × num_components` bytes, not bytes.
    pub remap: u32,
    /// `MEM_OP_C` as last written — the low PDB bits and `PDB_ALL` of a following `MEM_OP_D`.
    pub mem_op_c: u32,
    /// ★ P6b: `MEM_OP_A` as last written (a forwarded MEMBAR re-emits it; a SYSMEMBAR request).
    pub mem_op_a: u32,
    /// ★ P6b: `MEM_OP_B` as last written.
    pub mem_op_b: u32,
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
    /// A physical operand in BLOCK-LINEAR layout: its footprint is a function of the block
    /// geometry, which a kernel scrub never uses — refused rather than bounded by a guess.
    BlockLinearPhysical,
    /// A launch whose byte extent overflows 64 bits.
    ExtentOverflow,
    /// `SET_OBJECT` of a class that is neither a copy engine nor `GP100_UVM_SW`: our host channel
    /// has no such object, and forwarding it would fault OUR channel.
    ForeignClass {
        /// The subchannel.
        subch: u32,
        /// The class named.
        class: u32,
    },
    /// ★ P6b: a method at or above `0x100` on a software subchannel (5-7) no `SET_OBJECT` bound —
    /// on bare metal a software-method trap to RM; nothing on our host channel may run it.
    UnboundSubchannel {
        /// The subchannel.
        subch: u32,
        /// The method.
        method: u32,
    },
    /// A `GP100_UVM_SW` method with work behind it (`FAULT_CANCEL_*`, `CLEAR_FAULTED_*`): the fault
    /// plane that would serve it does not exist yet — refused rather than silently dropped.
    SwMethod {
        /// The method.
        method: u32,
    },
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
    let bit = 1u8 << (sub & 7);
    if m == 0 {
        // SET_OBJECT: which subchannels hold a CE, which hold the SW class; nothing else is ours.
        let class = v & 0xFFFF;
        st.ce_subch &= !bit;
        st.sw_subch &= !bit;
        if is_ce(class) {
            st.ce_subch |= bit;
            emit(cur, sub, m, v);
        } else if class == GP100_UVM_SW {
            st.sw_subch |= bit; // consumed: no host object stands behind it
        } else {
            return Err(Refusal::ForeignClass { subch: sub, class });
        }
        return Ok(());
    }
    if st.sw_subch & bit != 0 && m >= 0x100 {
        // A SW subchannel's own methods (host methods below 0x100 still apply to the channel).
        return if m == SW_NO_OPERATION { Ok(()) } else { Err(Refusal::SwMethod { method: m }) };
    }
    // `MEM_OP_A..C` are operands of the `MEM_OP_D` that follows ("MEM_OP_D MUST be preceded by
    // MEM_OPs A-C", `clc56f.h`): held here, emitted with the D when its operation is forwarded.
    if m == MEM_OP_A {
        st.mem_op_a = v;
        return Ok(());
    }
    if m == MEM_OP_B {
        st.mem_op_b = v;
        return Ok(());
    }
    if m == MEM_OP_C {
        st.mem_op_c = v;
        return Ok(());
    }
    if m == MEM_OP_D {
        let op = v >> 27;
        if op == OP_MEMBAR {
            // ★ P6b (d): an ordering method, not a privileged one — forwarded as written.
            emit(cur, sub, MEM_OP_A, st.mem_op_a);
            emit(cur, sub, MEM_OP_B, st.mem_op_b);
            emit(cur, sub, MEM_OP_C, st.mem_op_c);
            emit(cur, sub, MEM_OP_D, v);
            return Ok(());
        }
        if op == OP_TLB_INVALIDATE || op == OP_TLB_INVALIDATE_TARGETED {
            if st.mem_op_a & MEM_OP_A_SYSMEMBAR_EN != 0 {
                // ★ P6b (d): the invalidate's SYSMEMBAR is kept as a forwarded MEMBAR ahead of it.
                emit(cur, sub, MEM_OP_A, 0);
                emit(cur, sub, MEM_OP_B, 0);
                emit(cur, sub, MEM_OP_C, MEMBAR_TYPE_SYS);
                emit(cur, sub, MEM_OP_D, OP_MEMBAR << 27);
            }
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
        return Ok(()); // the other MEM_OP_D operations are consumed here (see the module doc)
    }
    // Host methods (below 0x100) apply to the channel whatever the subchannel: forwarded.
    if m < 0x100 {
        emit(cur, sub, m, v);
        return Ok(());
    }
    // ★★★★★ P6b: on a COPY-ENGINE channel every HARDWARE subchannel (0-4) reaches the CE — "HW
    // uses a fixed subchannel for CE" (`NVA06F_SUBCHANNEL_COPY_ENGINE` = 4, `cla06fsubch.h`;
    // `uvm_push_macros.h:84-85`) — whichever subchannel the SET_OBJECT named. nvidia-uvm binds the
    // CE on subchannel 0 (`uvm_maxwell_ce.c:31-36`, to verify the engine type) and pushes every CE
    // method on subchannel 4. `[measured p6b8]` keyed on the SET_OBJECT's subchannel, this
    // rewriter saw NONE of UVM's launches as CE launches and forwarded them VERBATIM — PHYSICAL
    // operands (UVM's page-table writes, `OFFSET_OUT` `0x201000`…) straight onto our host ring,
    // i.e. aimed at HOST physical memory — and the guest's own root stayed all zeros in the store.
    // ⇒ Every Translated channel is a CE channel (the plane births no other kind), so subchannels
    // 0-4 are the CE here, always; 5-7 are software subchannels and must be bound to be used.
    if sub > 4 {
        return Err(Refusal::UnboundSubchannel { subch: sub, method: m });
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
        PITCH_IN => st.pitch_in = v,
        PITCH_OUT => st.pitch_out = v,
        ce::SET_REMAP_COMPONENTS => st.remap = v,
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

/// The bytes a launch touches on each side: `(source, destination)`, the source `None` when the
/// engine does not read it (no data transfer, or a remap whose every written component is a
/// constant — the scrub's fill). ★ Exact, never an under-estimate: the window check vets THIS
/// range, and a short one would pass a launch the engine then runs past the window's end.
fn extents(st: &CeState, v: u32) -> Result<(Option<u64>, u64), Refusal> {
    let remap = v & ce::LAUNCH_REMAP_ENABLE != 0;
    let comp = u64::from(ce::remap_component_bytes(st.remap));
    let n_dst = ce::remap_num_dst_components(st.remap);
    let n_src = u64::from(((st.remap >> REMAP_NUM_SRC_SHIFT) & 0x3) + 1);
    let (src_elem, dst_elem) = if remap { (comp * n_src, comp * u64::from(n_dst)) } else { (1, 1) };
    let reads_src = v & ce::LAUNCH_TRANSFER_MASK != ce::LAUNCH_TRANSFER_NONE
        && (!remap || (0..n_dst).any(|c| ce::remap_dst_sel(st.remap, c) <= ce::REMAP_DST_SEL_SRC_MAX));
    let lines = if v & ce::LAUNCH_MULTI_LINE_ENABLE != 0 { u64::from(st.line_count) } else { 1 };
    let ext = |elem: u64, pitch: u32| -> Result<u64, Refusal> {
        let line = u64::from(st.line_len).checked_mul(elem).ok_or(Refusal::ExtentOverflow)?;
        let span = match lines {
            0 | 1 => line,
            n => u64::from(pitch)
                .checked_mul(n - 1)
                .and_then(|x| x.checked_add(line))
                .ok_or(Refusal::ExtentOverflow)?,
        };
        Ok(span.max(1))
    };
    let src = if reads_src { Some(ext(src_elem, st.pitch_in)?) } else { None };
    Ok((src, ext(dst_elem, st.pitch_out)?))
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
    let (src_len, dst_len) = extents(st, v)?;
    let src_rewritten = src_phys && src_len.is_some();
    if src_rewritten && v & ce::LAUNCH_SRC_PITCH == 0 || dst_phys && v & ce::LAUNCH_DST_PITCH == 0 {
        return Err(Refusal::BlockLinearPhysical);
    }
    if let (true, Some(len)) = (src_phys, src_len) {
        let va = xlate(w, st.src_mode, st.off_in, len)?;
        put_offset(cur, sub, ce::OFFSET_IN_UPPER, va);
    }
    if dst_phys {
        let va = xlate(w, st.dst_mode, st.off_out, dst_len)?;
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
    if src_rewritten {
        put_offset(cur, sub, ce::OFFSET_IN_UPPER, st.off_in);
    }
    if dst_phys {
        put_offset(cur, sub, ce::OFFSET_OUT_UPPER, st.off_out);
    }
    Ok(())
}
