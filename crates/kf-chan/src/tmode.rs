// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★★★★ P1+P2 inc C (`docs/design/V3_P1P2_TSPACE.md` §3) — **the T-mode rewriter: an address
//! author, not a forwarder.**
//!
//! Under T-mode a Translated channel runs in the per-VM T-space (`kf_qemu::tspace`), where no guest
//! row exists. So nothing the guest wrote may reach the engine as written:
//!
//! - every word kayfabe emits is AUTHORED from decoded fields that passed the tier's table
//!   ([`crate::ttables`]); no guest word is copied;
//! - every address the engine dereferences is COMPUTED by kayfabe — a physical operand by window
//!   arithmetic, a virtual operand or semaphore by resolving it through the placement rows of the
//!   guest VA space the channel was born in ([`Rows`]) — and lies inside one of the two windows
//!   ([`crate::tspace_unsafe::TWindows`]); the address words themselves are emitted only by the
//!   perimeter file;
//! - every emitted `(subchannel, method)` is in the tier's table or is an authored address
//!   register.
//!
//! The pipeline: [`decode`] turns a pushbuffer segment into an UNBOUND IR ([`Ir`]); [`bind`] turns
//! one IR item into words against the rows AS THEY ARE when it runs — the runner binds each piece
//! when it pushes it (§3.5), so an operand bound after the guest's last invalidate on its own
//! channel sees the post-invalidate rows. [`Shadow`] runs both on today's path behind
//! `KF3_TSHADOW=1` and counts what T-mode would do, changing nothing.
//!
//! ⊘ Pure: no GPU, no host call, no lock beyond what a [`Rows`] implementation takes per operand.

use crate::census::{Census, SubKind};
use crate::translated::{GP100_UVM_SW, IsCeClass, Refusal, Target, form_tag};
use crate::tspace_unsafe::{
    CeSide, TWindows, WindowAddr, put_ce_offset, put_ce_semaphore, put_host_sem_addr,
    put_host_semaphore, put_launch,
};
use crate::ttables::{
    CeMethod, HostMethod, Launch, SemExecute, SemaphoreD, Tier, ce_method, decode_launch,
    host_method,
};
use kf_abi::submit::{MethodForm, ce, fifo, method_header_decode, method_header_inc};
use kf_host::MapPerm;

/// At most this many pieces per launch (§3.4); a launch that would need more is refused by name.
pub const MAX_PIECES: usize = 64;
/// The output cap per pushed piece (§3.5): far below the host ring's half-pushbuffer limit.
pub const CHUNK_BYTES: usize = 64 << 10;

/// One resolved span of a guest VA range: its backing (`ram` = the guest memfd, else the store),
/// the backing offset, the bytes, and the guest leaf's permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    /// Guest RAM (memfd offset) rather than guest VRAM (store offset).
    pub ram: bool,
    /// The backing offset of the span's first byte.
    pub off: u64,
    /// Its length.
    pub len: u64,
    /// The guest leaf's permission (`kf_mem::ledger::Desired::perm`).
    pub perm: MapPerm,
}

/// ★ The placement rows of the guest VA space a Translated channel was born in — kayfabe's own
/// map calls, never a copy of the guest's tables (§56 rule 2, `V3_P1P2_TSPACE.md` §11.4).
pub trait Rows {
    /// Resolve `[va, va+len)` under ONE read guard: its spans in VA order, merged where contiguous
    /// in both VA and backing with one permission. `Err(at)` names the first byte no row covers.
    ///
    /// # Errors
    /// The first uncovered byte.
    fn resolve(&self, va: u64, len: u64) -> Result<Vec<Span>, u64>;

    /// ★ The vIOMMU seam (§2.6): the memfd offset of guest DMA range `[dma, dma+len)` when it is one
    /// contiguous run — today `RamMap::file_range`; under a vIOMMU, IOVA→GPA first.
    fn dma_to_file_range(&self, dma: u64, len: u64) -> Option<u64>;
}

/// ★ The pure core of [`Rows::resolve`]: walk `prefix(va) = (ram, off, bytes left in the row, perm)`
/// piece by piece, merging adjacent pieces contiguous in backing with one permission.
///
/// # Errors
/// The first uncovered byte.
pub fn resolve_spans(
    va: u64,
    len: u64,
    prefix: impl Fn(u64) -> Option<(bool, u64, u64, MapPerm)>,
) -> Result<Vec<Span>, u64> {
    let end = va.checked_add(len).ok_or(va)?;
    let mut out: Vec<Span> = Vec::new();
    let mut at = va;
    while at < end {
        let (ram, off, avail, perm) = prefix(at).filter(|p| p.2 > 0).ok_or(at)?;
        let n = avail.min(end - at);
        match out.last_mut() {
            Some(s) if s.ram == ram && s.perm == perm && s.off.checked_add(s.len) == Some(off) => {
                s.len += n;
            }
            _ => out.push(Span {
                ram,
                off,
                len: n,
                perm,
            }),
        }
        at += n;
    }
    Ok(out)
}

/// A data operand as the guest named it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operand {
    /// PHYSICAL: guest VRAM or guest RAM by its own address.
    Physical(Target, u64),
    /// VIRTUAL: a guest VA in the channel's VA space.
    Virtual(u64),
}

/// The footprint registers a launch was decoded with — re-emitted at every bound piece, so the
/// engine runs with exactly the values the bound was computed from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Footprint {
    /// `LINE_LENGTH_IN` (elements with remap, else bytes).
    pub line_len: u32,
    /// `LINE_COUNT` (multi-line only).
    pub line_count: u32,
    /// `PITCH_IN`.
    pub pitch_in: u32,
    /// `PITCH_OUT`.
    pub pitch_out: u32,
    /// `SET_REMAP_COMPONENTS`.
    pub remap: u32,
    /// `SET_REMAP_CONST_A`.
    pub const_a: u32,
    /// `SET_REMAP_CONST_B`.
    pub const_b: u32,
}

/// A CE semaphore release a launch asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CeSema {
    /// Its guest VA.
    pub va: u64,
    /// `SET_SEMAPHORE_PAYLOAD`.
    pub payload: u32,
    /// `SET_SEMAPHORE_PAYLOAD_UPPER` (two-word payload only).
    pub payload_upper: Option<u32>,
    /// Bytes written: 4, 8 (two-word payload) or 16 (four-word / with timestamp).
    pub bytes: u64,
    /// A reduction (an atomic): refused through an `atomic_disable` row.
    pub reduction: bool,
}

/// ★ One CE `LAUNCH_DMA`, unbound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CeLaunch {
    /// The subchannel.
    pub sub: u32,
    /// The tier the launch was decoded against.
    pub tier: Tier,
    /// The decoded word (its `SRC/DST_TYPE` are the guest's; never emitted).
    pub launch: Launch,
    /// The source and its bytes, when the engine reads one.
    pub src: Option<(Operand, u64)>,
    /// The destination and its bytes, when data moves.
    pub dst: Option<(Operand, u64)>,
    /// The footprint registers.
    pub regs: Footprint,
    /// Element bytes on each side (1 without remap).
    pub elem: (u64, u64),
    /// The release, if any.
    pub sema: Option<CeSema>,
    /// `REQ_ATTR` (`CAB5`), when the guest set one.
    pub req_attr: Option<u32>,
}

/// Which host semaphore form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostSemForm {
    /// `SEMAPHOREA-D` (40-bit on every tier).
    Legacy {
        /// `SEMAPHOREC`.
        payload: u32,
        /// `SEMAPHORED`, decoded.
        op: SemaphoreD,
    },
    /// `SEM_ADDR_LO/HI` + `SEM_PAYLOAD_LO/HI` + `SEM_EXECUTE`.
    Execute {
        /// `SEM_PAYLOAD_LO`.
        payload_lo: u32,
        /// `SEM_PAYLOAD_HI`.
        payload_hi: u32,
        /// `SEM_EXECUTE`, decoded.
        op: SemExecute,
        /// 57-bit `SEM_ADDR_HI` (Hopper+).
        wide: bool,
    },
}

/// ★ One host semaphore operation, unbound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostSem {
    /// The subchannel.
    pub sub: u32,
    /// Its guest VA.
    pub va: u64,
    /// The form and its fields.
    pub form: HostSemForm,
}

/// ★ The unbound IR (§3.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ir {
    /// Address-free method writes, authored: `(subchannel, method, value)`.
    Words(Vec<(u32, u32, u32)>),
    /// A CE launch.
    Launch(Box<CeLaunch>),
    /// A host semaphore operation.
    HostSem(HostSem),
    /// The guest invalidated a VA space here (a split; `None` = `PDB_ALL`).
    Invalidate {
        /// The named root.
        pdb: Option<u64>,
    },
}

/// ★ The channel state T-mode decodes against — one per channel, carried across segments.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TState {
    /// The bound CE class's tier.
    pub tier: Option<Tier>,
    /// Subchannels bound to `GP100_UVM_SW`.
    pub sw_subch: u8,
    off_in: u64,
    off_out: u64,
    src_mode: u32,
    dst_mode: u32,
    regs: Footprint,
    sem_a: u32,
    sem_b: u32,
    sem_payload: u32,
    sem_payload_upper: u32,
    req_attr: Option<u32>,
    host_a: u32,
    host_b: u32,
    host_c: u32,
    sem_lo: u32,
    sem_hi: u32,
    sem_plo: u32,
    sem_phi: u32,
    mem_op_a: u32,
    mem_op_b: u32,
    mem_op_c: u32,
}

/// `MEM_OP_D_OPERATION` values (`ogkm-580: src/common/sdk/nvidia/inc/class/clc56f.h:182-193`).
const OP_MEMBAR: u32 = 5;
const OP_TLB_INVALIDATE: u32 = 9;
const OP_TLB_INVALIDATE_TARGETED: u32 = 0xA;
const OPS_L2: [u32; 6] = [0xD, 0xE, 0xF, 0x10, 0x11, 0x15];
const OP_ACCESS_COUNTER_CLR: u32 = 0x16;
/// `NVC56F_MEM_OP_A_TLB_INVALIDATE_SYSMEMBAR` (11).
const MEM_OP_A_SYSMEMBAR: u32 = 1 << 11;
/// `NVC56F_SET_OBJECT_NVCLASS` (15:0).
const NVCLASS_MASK: u32 = 0xFFFF;

/// ★ **Decode one segment into unbound IR.** `st` carries the channel's state across segments.
///
/// # Errors
/// [`Refusal`], by name: an undecodable header, a `SubDeviceMask` header, an unclassified or
/// refused method, a field the table does not name, an operation the class does not define.
pub fn decode(
    words: &[u32],
    is_ce: IsCeClass,
    st: &mut TState,
    mut census: Option<&mut Census>,
) -> Result<Vec<Ir>, Refusal> {
    let mut out: Vec<Ir> = Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        let hw = words[i];
        let h = method_header_decode(hw).ok_or(Refusal::BadHeader { at: i, word: hw })?;
        if let Some(c) = census.as_deref_mut() {
            c.form(form_tag(h.form));
        }
        i += 1;
        let (writes, consumed): (Vec<(u32, u32)>, usize) = match h.form {
            MethodForm::EndPbSegment => break,
            MethodForm::SubDeviceMask => return Err(Refusal::SubDeviceMask { at: i - 1 }),
            MethodForm::Immediate => (vec![(h.method, h.immd)], 0),
            MethodForm::Incrementing
            | MethodForm::Legacy
            | MethodForm::NonIncrementing
            | MethodForm::IncrementOnce => {
                let n = h.arg_words;
                let args = words.get(i..i + n).ok_or(Refusal::Truncated { at: i })?;
                let at = |k: usize| match h.form {
                    MethodForm::NonIncrementing => h.method,
                    MethodForm::IncrementOnce if k > 0 => h.method + 4,
                    MethodForm::IncrementOnce => h.method,
                    _ => h.method + 4 * k as u32,
                };
                (
                    args.iter().enumerate().map(|(k, &v)| (at(k), v)).collect(),
                    n,
                )
            }
        };
        i += consumed;
        for (m, v) in writes {
            one(
                &mut out,
                is_ce,
                st,
                h.subchannel,
                m,
                v,
                census.as_deref_mut(),
            )?;
        }
    }
    Ok(out)
}

fn words_ir(out: &mut Vec<Ir>, w: &[(u32, u32, u32)]) {
    if let Some(Ir::Words(v)) = out.last_mut() {
        v.extend_from_slice(w);
    } else {
        out.push(Ir::Words(w.to_vec()));
    }
}

#[allow(clippy::too_many_lines)]
fn one(
    out: &mut Vec<Ir>,
    is_ce: IsCeClass,
    st: &mut TState,
    sub: u32,
    m: u32,
    v: u32,
    census: Option<&mut Census>,
) -> Result<(), Refusal> {
    let bit = 1u8 << (sub & 7);
    if let Some(c) = census {
        let kind = if m < 0x100 {
            SubKind::Host
        } else if st.sw_subch & bit != 0 {
            SubKind::Sw
        } else if sub > 4 {
            SubKind::Unbound
        } else {
            SubKind::Ce
        };
        c.method(st.tier.map_or(0, tier_class), kind, m);
    }
    if m < 0x100 {
        let Some(hm) = host_method(m) else {
            return Err(Refusal::Unclassified {
                subch: sub,
                method: m,
            });
        };
        return host(out, is_ce, st, sub, m, v, bit, hm);
    }
    if st.sw_subch & bit != 0 {
        // `GP100_UVM_SW`'s own methods: `NO_OPERATION` is consumed; anything else needs the fault
        // plane (refused, as today).
        return if m == 0x100 {
            Ok(())
        } else {
            Err(Refusal::SwMethod { method: m })
        };
    }
    if sub > 4 {
        return Err(Refusal::UnboundSubchannel {
            subch: sub,
            method: m,
        });
    }
    let tier = st.tier.ok_or(Refusal::NoCeObject {
        subch: sub,
        method: m,
    })?;
    let Some(cm) = ce_method(tier, m) else {
        return Err(Refusal::Unclassified {
            subch: sub,
            method: m,
        });
    };
    let upper = tier.upper_mask();
    match cm {
        CeMethod::Refused(name) => {
            return Err(Refusal::RefusedMethod {
                subch: sub,
                method: m,
                name,
            });
        }
        CeMethod::Nop | CeMethod::BlockLinearState | CeMethod::ScrubParameters => {}
        CeMethod::SemA => st.sem_a = v,
        CeMethod::SemB => st.sem_b = v,
        CeMethod::SemPayload => st.sem_payload = v,
        CeMethod::SemPayloadUpper => st.sem_payload_upper = v,
        CeMethod::SrcPhysMode => st.src_mode = v,
        CeMethod::DstPhysMode => st.dst_mode = v,
        CeMethod::OffsetInUpper => {
            st.off_in = (st.off_in & 0xFFFF_FFFF) | (u64::from(v & upper) << 32);
        }
        CeMethod::OffsetInLower => st.off_in = (st.off_in & !0xFFFF_FFFF) | u64::from(v),
        CeMethod::OffsetOutUpper => {
            st.off_out = (st.off_out & 0xFFFF_FFFF) | (u64::from(v & upper) << 32);
        }
        CeMethod::OffsetOutLower => st.off_out = (st.off_out & !0xFFFF_FFFF) | u64::from(v),
        CeMethod::PitchIn => st.regs.pitch_in = v,
        CeMethod::PitchOut => st.regs.pitch_out = v,
        CeMethod::LineLength => st.regs.line_len = v,
        CeMethod::LineCount => st.regs.line_count = v,
        CeMethod::RemapConstA => st.regs.const_a = v,
        CeMethod::RemapConstB => st.regs.const_b = v,
        CeMethod::RemapComponents => st.regs.remap = v,
        CeMethod::ReqAttr => {
            if v & !0x3 != 0 {
                return Err(Refusal::FieldValue {
                    method: m,
                    word: v,
                    what: "REQ_ATTR beyond PREFETCH_L2_CLASS (1:0)",
                });
            }
            st.req_attr = Some(v);
        }
        CeMethod::Launch => {
            let l = decode_launch(tier, v)?;
            out.push(Ir::Launch(Box::new(ce_launch(st, tier, sub, l)?)));
        }
    }
    Ok(())
}

const fn tier_class(t: Tier) -> u32 {
    match t {
        Tier::C5b5 => 0xC5B5,
        Tier::C6b5 => 0xC6B5,
        Tier::C7b5 => 0xC7B5,
        Tier::C8b5 => 0xC8B5,
        Tier::C9b5 => 0xC9B5,
        Tier::Cab5 => 0xCAB5,
    }
}

#[allow(clippy::too_many_arguments)]
fn host(
    out: &mut Vec<Ir>,
    is_ce: IsCeClass,
    st: &mut TState,
    sub: u32,
    m: u32,
    v: u32,
    bit: u8,
    hm: HostMethod,
) -> Result<(), Refusal> {
    match hm {
        HostMethod::Refused(name) => {
            return Err(Refusal::RefusedMethod {
                subch: sub,
                method: m,
                name,
            });
        }
        HostMethod::SetObject => {
            let class = v & NVCLASS_MASK;
            st.sw_subch &= !bit;
            if v & !NVCLASS_MASK != 0 {
                return Err(Refusal::FieldValue {
                    method: m,
                    word: v,
                    what: "SET_OBJECT beyond NVCLASS (15:0)",
                });
            }
            if is_ce(class) {
                let tier =
                    Tier::of_ce_class(class).ok_or(Refusal::ForeignClass { subch: sub, class })?;
                st.tier = Some(tier);
                // ★ Authored: the class alone, `ENGINE` zero.
                words_ir(out, &[(sub, 0, class)]);
            } else if class == GP100_UVM_SW {
                st.sw_subch |= bit;
            } else {
                return Err(Refusal::ForeignClass { subch: sub, class });
            }
        }
        // Data the engine never executes: UVM's inline payloads are read by the CE from the
        // guest's own pushbuffer through the window, never from ours — consumed.
        HostMethod::Nop => {}
        HostMethod::SemaphoreA => st.host_a = v,
        HostMethod::SemaphoreB => st.host_b = v,
        HostMethod::SemaphoreC => st.host_c = v,
        HostMethod::SemaphoreD => {
            let op = SemaphoreD::decode(v)?;
            if st.host_a & !0xFF != 0 || st.host_b & 3 != 0 {
                return Err(Refusal::FieldValue {
                    method: m,
                    word: st.host_a,
                    what: "SEMAPHOREA beyond 7:0 or SEMAPHOREB below bit 2",
                });
            }
            out.push(Ir::HostSem(HostSem {
                sub,
                va: (u64::from(st.host_a) << 32) | u64::from(st.host_b),
                form: HostSemForm::Legacy {
                    payload: st.host_c,
                    op,
                },
            }));
        }
        HostMethod::NonStallInterrupt => words_ir(out, &[(sub, m, v)]),
        HostMethod::FbFlush | HostMethod::SetReference => words_ir(out, &[(sub, m, v)]),
        HostMethod::Wfi => {
            if v & !1 != 0 {
                return Err(Refusal::FieldValue {
                    method: m,
                    word: v,
                    what: "WFI beyond SCOPE (0:0)",
                });
            }
            words_ir(out, &[(sub, m, v)]);
        }
        HostMethod::Yield => {
            if v & !3 != 0 {
                return Err(Refusal::FieldValue {
                    method: m,
                    word: v,
                    what: "YIELD beyond OP (1:0)",
                });
            }
            words_ir(out, &[(sub, m, v)]);
        }
        HostMethod::MemOpA => st.mem_op_a = v,
        HostMethod::MemOpB => st.mem_op_b = v,
        HostMethod::MemOpC => st.mem_op_c = v,
        HostMethod::MemOpD => mem_op(out, st, sub, v)?,
        HostMethod::SemAddrLo => st.sem_lo = v,
        HostMethod::SemAddrHi => st.sem_hi = v,
        HostMethod::SemPayloadLo => st.sem_plo = v,
        HostMethod::SemPayloadHi => st.sem_phi = v,
        HostMethod::SemExecute => {
            let op = SemExecute::decode(v)?;
            let wide = st.tier.is_some_and(Tier::wide_sem_addr);
            let hi_mask: u32 = if wide { 0x1FF_FFFF } else { 0xFF };
            if st.sem_hi & !hi_mask != 0 || st.sem_lo & 3 != 0 {
                return Err(Refusal::FieldValue {
                    method: m,
                    word: st.sem_hi,
                    what: "SEM_ADDR_HI beyond the tier's field or SEM_ADDR_LO below bit 2",
                });
            }
            out.push(Ir::HostSem(HostSem {
                sub,
                va: (u64::from(st.sem_hi) << 32) | u64::from(st.sem_lo),
                form: HostSemForm::Execute {
                    payload_lo: st.sem_plo,
                    payload_hi: st.sem_phi,
                    op,
                    wide,
                },
            }));
        }
    }
    Ok(())
}

fn mem_op(out: &mut Vec<Ir>, st: &TState, sub: u32, d: u32) -> Result<(), Refusal> {
    let op = d >> 27;
    let (a, b, c) = (st.mem_op_a, st.mem_op_b, st.mem_op_c);
    let fields = |what| Refusal::FieldValue {
        method: fifo::MEM_OP_A,
        word: d,
        what,
    };
    if op == OP_MEMBAR {
        // ★ Authored from MEMBAR_TYPE (C 2:0) alone — UVM's own shape (`uvm_pascal_host.c:32-45`).
        if a != 0 || b != 0 || c & !0x7 != 0 || c > 1 || d & 0x07FF_FFFF != 0 {
            return Err(fields("MEMBAR with fields beyond MEMBAR_TYPE"));
        }
        words_ir(out, &membar(sub, c));
        return Ok(());
    }
    if op == OP_TLB_INVALIDATE || op == OP_TLB_INVALIDATE_TARGETED {
        if a & MEM_OP_A_SYSMEMBAR != 0 {
            words_ir(out, &membar(sub, 0));
        }
        let all = c & 1 != 0;
        let lo = u64::from(c & 0xFFFF_F000);
        let hi = u64::from(d & 0x07FF_FFFF) << 32;
        out.push(Ir::Invalidate {
            pdb: (!all).then_some(hi | lo),
        });
        return Ok(());
    }
    if OPS_L2.contains(&op) {
        // ★ Authored: A-C zero, D the operation alone (`uvm_ampere_host.c:465-490`).
        if a != 0 || b != 0 || c != 0 || d & 0x07FF_FFFF != 0 {
            return Err(fields("an L2 operation with operands"));
        }
        words_ir(
            out,
            &[
                (sub, fifo::MEM_OP_A, 0),
                (sub, fifo::MEM_OP_A + 4, 0),
                (sub, fifo::MEM_OP_A + 8, 0),
                (sub, fifo::MEM_OP_A + 12, op << 27),
            ],
        );
        return Ok(());
    }
    if op == OP_ACCESS_COUNTER_CLR {
        return Ok(()); // served: the device we present has no access counters
    }
    Err(Refusal::MemOp { op })
}

fn membar(sub: u32, membar_type: u32) -> [(u32, u32, u32); 4] {
    [
        (sub, fifo::MEM_OP_A, 0),
        (sub, fifo::MEM_OP_A + 4, 0),
        (sub, fifo::MEM_OP_A + 8, membar_type),
        (sub, fifo::MEM_OP_A + 12, OP_MEMBAR << 27),
    ]
}

/// `SET_REMAP_COMPONENTS` fields (`clc7b5.h:219-223`): `COMPONENT_SIZE` 17:16, `NUM_SRC` 21:20,
/// `NUM_DST` 25:24, `DST_{X,Y,Z,W}` 2:0 / 6:4 / 10:8 / 14:12 (value ≤ 3 reads the source).
fn remap_elems(remap: u32) -> (u64, u64, bool) {
    let comp = u64::from(ce::remap_component_bytes(remap));
    let n_dst = ce::remap_num_dst_components(remap);
    let n_src = u64::from(((remap >> 20) & 0x3) + 1);
    let reads = (0..n_dst).any(|c| ce::remap_dst_sel(remap, c) <= ce::REMAP_DST_SEL_SRC_MAX);
    (comp * n_src, comp * u64::from(n_dst), reads)
}

fn ce_launch(st: &TState, tier: Tier, sub: u32, l: Launch) -> Result<CeLaunch, Refusal> {
    let moves = l.transfer != 0;
    let (mut src_elem, mut dst_elem, mut reads_src) = (1, 1, moves);
    if l.remap {
        let (s, d, r) = remap_elems(st.regs.remap);
        (src_elem, dst_elem, reads_src) = (s, d, moves && r);
    }
    // A Hopper+ fast scrub on a physical destination is a zero-fill of the same bytes (§3.2, as
    // `translated::launch_scrub_translated` today): remap ON, `DST_X = CONST_A = 0`, one-byte
    // components; with remap disabled a scrub's element is one byte.
    let mut l = l;
    let mut regs = st.regs;
    if l.scrub {
        if !l.dst_phys || !tier.has_fast_scrub() {
            return Err(Refusal::FieldValue {
                method: ce::LAUNCH_DMA,
                word: crate::ttables::encode_launch(&l),
                what: "MEMORY_SCRUB on a virtual destination",
            });
        }
        (src_elem, dst_elem, reads_src) = (1, 1, false);
        l.scrub = false;
        l.remap = true;
        regs.remap = ce::REMAP_DST_SEL_CONST_A;
        regs.const_a = 0;
    }
    if (reads_src && !l.src_pitch) || (moves && !l.dst_pitch) {
        return Err(if l.src_phys || l.dst_phys {
            Refusal::BlockLinearPhysical
        } else {
            Refusal::BlockLinearVirtual
        });
    }
    let lines = if l.multi_line {
        u64::from(regs.line_count)
    } else {
        1
    };
    let ext = |elem: u64, pitch: u32| -> Result<u64, Refusal> {
        let line = u64::from(regs.line_len)
            .checked_mul(elem)
            .ok_or(Refusal::ExtentOverflow)?;
        let span = match lines {
            0 | 1 => line,
            n => u64::from(pitch)
                .checked_mul(n - 1)
                .and_then(|x| x.checked_add(line))
                .ok_or(Refusal::ExtentOverflow)?,
        };
        Ok(span.max(1))
    };
    let side = |phys: bool, mode: u32, at: u64| {
        if phys {
            Operand::Physical(target_of(mode), at)
        } else {
            Operand::Virtual(at)
        }
    };
    let src = if reads_src {
        Some((
            side(l.src_phys, st.src_mode, st.off_in),
            ext(src_elem, regs.pitch_in)?,
        ))
    } else {
        None
    };
    let dst = if moves {
        Some((
            side(l.dst_phys, st.dst_mode, st.off_out),
            ext(dst_elem, regs.pitch_out)?,
        ))
    } else {
        None
    };
    let sema = match l.sema {
        0 => None,
        t => {
            let two_word = l.payload_two_word;
            Some(CeSema {
                va: (u64::from(st.sem_a & tier.upper_mask()) << 32) | u64::from(st.sem_b),
                payload: st.sem_payload,
                payload_upper: two_word.then_some(st.sem_payload_upper),
                bytes: if t == 2 {
                    16
                } else if two_word {
                    8
                } else {
                    4
                },
                reduction: l.reduction_enable,
            })
        }
    };
    Ok(CeLaunch {
        sub,
        tier,
        launch: l,
        src,
        dst,
        regs,
        elem: (src_elem, dst_elem),
        sema,
        req_attr: st.req_attr,
    })
}

const fn target_of(mode: u32) -> Target {
    match mode & ce::PHYS_MODE_TARGET_MASK {
        0 => Target::LocalFb,
        1 => Target::CoherentSysmem,
        2 => Target::NonCoherentSysmem,
        _ => Target::Peer,
    }
}

/// One resolved span of a launch side: `(in guest RAM, backing offset, bytes, guest permission)`
/// — a physical operand is one span with no permission (kernel physical access).
type Side = Vec<(bool, u64, u64, Option<MapPerm>)>;

/// Resolve one operand into backing spans, each checked to lie wholly inside its window.
fn resolve_side(op: Operand, len: u64, rows: &dyn Rows, w: &TWindows) -> Result<Side, Refusal> {
    let untranslatable = |target, phys| Refusal::Untranslatable { target, phys, len };
    match op {
        Operand::Physical(Target::Peer, _) => Err(Refusal::PeerOperand),
        Operand::Physical(Target::LocalFb, at) => {
            // ★ Bounded by the carve-out, not the store (§3.4).
            w.fb(at, len)
                .ok_or_else(|| untranslatable(Target::LocalFb, at))?;
            Ok(vec![(false, at, len, None)])
        }
        Operand::Physical(t, at) => {
            let off = rows
                .dma_to_file_range(at, len)
                .ok_or_else(|| untranslatable(t, at))?;
            w.ram(off, len).ok_or_else(|| untranslatable(t, at))?;
            Ok(vec![(true, off, len, None)])
        }
        Operand::Virtual(va) => {
            let spans = rows
                .resolve(va, len)
                .map_err(|at| Refusal::VirtualUnresolved { va, at })?;
            let mut out = Vec::with_capacity(spans.len());
            let mut cur = va;
            for s in spans {
                let inside = if s.ram {
                    w.ram(s.off, s.len)
                } else {
                    w.fb(s.off, s.len)
                };
                if inside.is_none() {
                    return Err(Refusal::OutsideWindow {
                        va: cur,
                        ram: s.ram,
                        off: s.off,
                    });
                }
                out.push((s.ram, s.off, s.len, Some(s.perm)));
                cur += s.len;
            }
            Ok(out)
        }
    }
}

/// A side's span boundaries, as byte offsets from its start (excluding 0 and its length).
fn cuts(side: &Side) -> Vec<u64> {
    let mut at = 0;
    let mut v = Vec::new();
    for &(_, _, n, _) in &side[..side.len().saturating_sub(1)] {
        at += n;
        v.push(at);
    }
    v
}

/// The window address of `[pos, pos+len)` of `side` — through the perimeter, so re-bounded: the
/// range must lie inside ONE span (piece cuts include every span boundary).
fn piece_addr(side: &Side, pos: u64, len: u64, w: &TWindows) -> Option<WindowAddr> {
    let mut base = 0;
    for &(ram, off, n, _) in side {
        if pos < base + n {
            if pos + len > base + n {
                return None;
            }
            let at = off + (pos - base);
            return if ram { w.ram(at, len) } else { w.fb(at, len) };
        }
        base += n;
    }
    None
}

fn emit(out: &mut Vec<u32>, sub: u32, m: u32, v: u32) {
    if let Some(h) = method_header_inc(sub & 7, m, 1) {
        out.push(h);
        out.push(v);
    }
}

/// ★ **Bind one IR item** against `rows` as they are NOW, emitting only authored words and window
/// addresses. A launch is split at the union of its sides' row boundaries (§3.4); the returned
/// count is the number of launches it became (the shadow's maximum).
///
/// # Errors
/// [`Refusal`], by name — nothing is emitted for a refused item.
pub fn bind(ir: &Ir, rows: &dyn Rows, w: &TWindows, out: &mut Vec<u32>) -> Result<usize, Refusal> {
    match ir {
        Ir::Words(v) => {
            for &(sub, m, val) in v {
                emit(out, sub, m, val);
            }
            Ok(0)
        }
        Ir::Invalidate { .. } => Ok(0),
        Ir::HostSem(h) => {
            bind_host_sem(h, rows, w, out)?;
            Ok(0)
        }
        Ir::Launch(l) => bind_launch(l, rows, w, out),
    }
}

fn sema_addr(
    va: u64,
    bytes: u64,
    writes: bool,
    reduction: bool,
    rows: &dyn Rows,
    w: &TWindows,
) -> Result<WindowAddr, Refusal> {
    let spans = rows
        .resolve(va, bytes)
        .map_err(|at| Refusal::VirtualUnresolved { va, at })?;
    let [s] = spans.as_slice() else {
        return Err(Refusal::SemaphoreSpansRows { va, bytes });
    };
    if writes && s.perm.read_only {
        return Err(Refusal::ReadOnlyRow { va });
    }
    if reduction && s.perm.atomic_disable {
        return Err(Refusal::AtomicDisabledRow { va });
    }
    let a = if s.ram {
        w.ram(s.off, bytes)
    } else {
        w.fb(s.off, bytes)
    };
    a.ok_or(Refusal::OutsideWindow {
        va,
        ram: s.ram,
        off: s.off,
    })
}

fn bind_host_sem(
    h: &HostSem,
    rows: &dyn Rows,
    w: &TWindows,
    out: &mut Vec<u32>,
) -> Result<(), Refusal> {
    let mut words = Vec::new();
    match h.form {
        HostSemForm::Legacy { payload, op } => {
            let a = sema_addr(h.va, op.bytes(), op.writes(), op.op == 0x10, rows, w)?;
            put_host_semaphore(&mut words, h.sub, a).map_err(|va| Refusal::Sem40 { va })?;
            emit(&mut words, h.sub, HOST_SEMAPHORE_C, payload);
            emit(&mut words, h.sub, HOST_SEMAPHORE_D, op.encode());
        }
        HostSemForm::Execute {
            payload_lo,
            payload_hi,
            op,
            wide,
        } => {
            let a = sema_addr(h.va, op.bytes(), op.writes(), op.op == 6, rows, w)?;
            put_host_sem_addr(&mut words, h.sub, a, wide).map_err(|va| Refusal::Sem40 { va })?;
            emit(&mut words, h.sub, fifo::SEM_PAYLOAD_LO, payload_lo);
            emit(&mut words, h.sub, fifo::SEM_PAYLOAD_HI, payload_hi);
            emit(&mut words, h.sub, fifo::SEM_EXECUTE, op.encode());
        }
    }
    out.extend(words);
    Ok(())
}

/// `NV906F_SEMAPHOREC` / `_D`.
const HOST_SEMAPHORE_C: u32 = 0x18;
const HOST_SEMAPHORE_D: u32 = 0x1C;
/// `NVC7B5_PITCH_IN` / `_OUT`, `NVC7B5_SET_REMAP_CONST_B`, `NVCAB5_REQ_ATTR`.
const PITCH_IN: u32 = 0x410;
const PITCH_OUT: u32 = 0x414;
const SET_REMAP_CONST_B: u32 = 0x704;
const REQ_ATTR: u32 = 0x754;

#[allow(clippy::too_many_lines)]
fn bind_launch(
    l: &CeLaunch,
    rows: &dyn Rows,
    w: &TWindows,
    out: &mut Vec<u32>,
) -> Result<usize, Refusal> {
    let src = match l.src {
        Some((op, n)) => Some((resolve_side(op, n, rows, w)?, n)),
        None => None,
    };
    let dst = match l.dst {
        Some((op, n)) => Some((resolve_side(op, n, rows, w)?, n)),
        None => None,
    };
    if let (Some((d, _)), Some((op, _))) = (&dst, l.dst)
        && d.iter().any(|&(_, _, _, p)| p.is_some_and(|p| p.read_only))
    {
        let va = match op {
            Operand::Virtual(va) => va,
            Operand::Physical(_, at) => at,
        };
        return Err(Refusal::ReadOnlyRow { va });
    }
    let sema = match l.sema {
        Some(s) => Some(sema_addr(s.va, s.bytes, true, s.reduction, rows, w)?),
        None => None,
    };
    let multi = l.launch.multi_line;
    if multi
        && (src.as_ref().is_some_and(|(s, _)| s.len() > 1)
            || dst.as_ref().is_some_and(|(d, _)| d.len() > 1))
    {
        return Err(Refusal::MultiLineDiscontiguous);
    }
    // ★ The pieces: element-index cut points from both sides' row boundaries (§3.4).
    let (se, de) = l.elem;
    let total = u64::from(l.regs.line_len);
    let mut idx: Vec<u64> = Vec::new();
    for (side, elem) in [(&src, se), (&dst, de)] {
        if let Some((s, _)) = side {
            for c in cuts(s) {
                if c % elem != 0 {
                    return Err(Refusal::SplitInsideElement);
                }
                idx.push(c / elem);
            }
        }
    }
    idx.sort_unstable();
    idx.dedup();
    idx.retain(|&e| e > 0 && e < total);
    let mut bounds = vec![0];
    bounds.extend(idx);
    bounds.push(total.max(1));
    let n_pieces = bounds.len() - 1;
    if n_pieces > MAX_PIECES {
        return Err(Refusal::TooManyPieces { pieces: n_pieces });
    }
    let mut words = Vec::new();
    for (k, win) in bounds.windows(2).enumerate() {
        let (e0, e1) = (win[0], win[1]);
        let last = k + 1 == n_pieces;
        let mut f = l.launch;
        if n_pieces > 1 {
            if k > 0 {
                f.transfer = 1; // PIPELINED: an independent range of the same copy
            }
            if last {
                f.transfer = 2; // NON_PIPELINED: ordered behind every earlier piece
            } else {
                // Only the LAST piece carries the release, the interrupt and the flush.
                f.sema = 0;
                f.intr = 0;
                f.flush = false;
                f.flush_gl = false;
                f.reduction_enable = false;
                f.reduction = 0;
                f.reduction_unsigned = false;
                f.payload_two_word = false;
            }
        }
        let line_len = u32::try_from(e1 - e0).map_err(|_| Refusal::ExtentOverflow)?;
        // Footprint registers, re-emitted at every piece: the engine runs with exactly the values
        // the bound was computed from.
        emit(
            &mut words,
            l.sub,
            ce::LINE_LENGTH_IN,
            if n_pieces > 1 {
                line_len
            } else {
                l.regs.line_len
            },
        );
        if multi {
            emit(&mut words, l.sub, ce::LINE_COUNT, l.regs.line_count);
            emit(&mut words, l.sub, PITCH_IN, l.regs.pitch_in);
            emit(&mut words, l.sub, PITCH_OUT, l.regs.pitch_out);
        }
        if f.remap {
            emit(&mut words, l.sub, ce::SET_REMAP_CONST_A, l.regs.const_a);
            emit(&mut words, l.sub, SET_REMAP_CONST_B, l.regs.const_b);
            emit(&mut words, l.sub, ce::SET_REMAP_COMPONENTS, l.regs.remap);
        }
        if let Some(r) = l.req_attr {
            emit(&mut words, l.sub, REQ_ATTR, r);
        }
        if let (Some(s), Some(a)) = (&l.sema, sema)
            && f.sema != 0
        {
            put_ce_semaphore(&mut words, l.sub, l.tier, a);
            emit(&mut words, l.sub, ce::SET_SEMAPHORE_PAYLOAD, s.payload);
            if let Some(u) = s.payload_upper {
                emit(&mut words, l.sub, ce::SET_SEMAPHORE_PAYLOAD_UPPER, u);
            }
        }
        for (side, elem, which) in [(&src, se, CeSide::In), (&dst, de, CeSide::Out)] {
            let Some((s, n)) = side else { continue };
            let (pos, len) = if n_pieces > 1 {
                (e0 * elem, (e1 - e0) * elem)
            } else {
                (0, *n)
            };
            let a = piece_addr(s, pos, len, w).ok_or(Refusal::ExtentOverflow)?;
            put_ce_offset(&mut words, l.sub, l.tier, which, a);
        }
        put_launch(&mut words, l.sub, &f);
    }
    out.extend(words);
    Ok(n_pieces)
}

/// ★ Cut IR into pushable pieces: consecutive items whose bound words stay within
/// [`CHUNK_BYTES`]; a split ([`Ir::Invalidate`]) always ends a piece. `bind_one` binds an item.
///
/// # Errors
/// The first item's refusal.
pub fn chunk(
    items: &[Ir],
    mut bind_one: impl FnMut(&Ir, &mut Vec<u32>) -> Result<(), Refusal>,
) -> Result<Vec<Vec<u32>>, Refusal> {
    let mut pieces: Vec<Vec<u32>> = Vec::new();
    let mut cur: Vec<u32> = Vec::new();
    for it in items {
        if matches!(it, Ir::Invalidate { .. }) {
            if !cur.is_empty() {
                pieces.push(std::mem::take(&mut cur));
            }
            continue;
        }
        let mut w = Vec::new();
        bind_one(it, &mut w)?;
        if !cur.is_empty() && 4 * (cur.len() + w.len()) > CHUNK_BYTES {
            pieces.push(std::mem::take(&mut cur));
        }
        cur.extend(w);
    }
    if !cur.is_empty() {
        pieces.push(cur);
    }
    Ok(pieces)
}

/// ★ P1+P2 inc C (§3.6) — **the shadow**: T-mode decode + bind on today's path, its output
/// discarded, its verdicts counted. `KF3_TSHADOW=1`; one `TSHADOW` line per channel at free.
#[derive(Debug, Default, Clone)]
pub struct Shadow {
    st: TState,
    /// Segments observed.
    pub segments: u64,
    /// IR items bound.
    pub items: u64,
    /// Launches bound.
    pub launches: u64,
    /// The most pieces one launch became.
    pub max_pieces: usize,
    /// Refusals T-mode would have made, by reason.
    pub would_refuse: std::collections::BTreeMap<&'static str, u64>,
    /// Of those, operands no placement row covered.
    pub resolve_miss: u64,
    /// Of those, a field or operation the tier's table does not name.
    pub unknown_field: u64,
    /// Of those, a method no table row classifies.
    pub unclassified: u64,
}

impl Shadow {
    /// Observe one segment as T-mode would decode and bind it. Stops at the first refusal of the
    /// segment (T-mode would have stopped the channel there).
    pub fn observe(&mut self, words: &[u32], is_ce: IsCeClass, rows: &dyn Rows, w: &TWindows) {
        self.segments += 1;
        let items = match decode(words, is_ce, &mut self.st, None) {
            Ok(v) => v,
            Err(r) => return self.refused(&r),
        };
        for it in &items {
            let mut sink = Vec::new();
            match bind(it, rows, w, &mut sink) {
                Ok(p) => {
                    self.items += 1;
                    if matches!(it, Ir::Launch(_)) {
                        self.launches += 1;
                        self.max_pieces = self.max_pieces.max(p);
                    }
                }
                Err(r) => return self.refused(&r),
            }
        }
    }

    fn refused(&mut self, r: &Refusal) {
        *self.would_refuse.entry(r.reason()).or_default() += 1;
        match r {
            Refusal::VirtualUnresolved { .. } => self.resolve_miss += 1,
            Refusal::LaunchField { .. }
            | Refusal::HostSemOp { .. }
            | Refusal::FieldValue { .. } => {
                self.unknown_field += 1;
            }
            Refusal::Unclassified { .. } => self.unclassified += 1,
            _ => {}
        }
    }

    /// The one line dumped at free.
    #[must_use]
    pub fn line(&self) -> String {
        let wr: Vec<String> = self
            .would_refuse
            .iter()
            .map(|(k, n)| format!("{k}:{n}"))
            .collect();
        format!(
            "segments={} items={} launches={} max_pieces={} would_refuse=[{}] resolve_miss={} unknown_field={} unclassified={}",
            self.segments,
            self.items,
            self.launches,
            self.max_pieces,
            wr.join(" "),
            self.resolve_miss,
            self.unknown_field,
            self.unclassified
        )
    }
}
