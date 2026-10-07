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
    LaunchPiece, PerimeterRefusal, PieceRegs, PieceSema, TWindows, WindowAddr,
    put_host_sem_execute, put_host_semaphore, put_launch_piece,
};
use crate::ttables::{
    CeMethod, HostMethod, Launch, REMAP_NAMED, SemExecute, SemaphoreD, Tier, ce_method,
    decode_launch, host_method,
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

    /// ★ Review fix 2026-10-04 (§3.5, §7.13): [`Rows::resolve`] together with the rows' change
    /// EPOCH, read under the same guard — the epoch the walk bumps with every commit it makes to
    /// these rows. The default (no epoch kept) answers 0.
    ///
    /// # Errors
    /// As [`Rows::resolve`].
    fn resolve_epoch(&self, va: u64, len: u64) -> (Result<Vec<Span>, u64>, u64) {
        (self.resolve(va, len), 0)
    }

    /// ★ The earliest commit after `epoch` that changed rows overlapping `[va, va+len)` — when it
    /// landed — so the stale-bind counter can tell a change the engine may have run behind from
    /// one made after the guest saw the work complete. The default keeps no log: `Unknown`.
    fn changed_since(&self, epoch: u64, va: u64, len: u64) -> Changed {
        let _ = (epoch, va, len);
        Changed::Unknown
    }

    /// ★ A walk the guest asked for has not landed yet in this space's rows (an invalidate, a
    /// split or a statement the VA thread has not applied). A virtual operand no row covers is
    /// then WAITED on — re-bound after the walk — rather than refused (§3.5). Default: none.
    fn walk_pending(&self) -> bool {
        false
    }
}

/// What [`Rows::changed_since`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Changed {
    /// No commit since the epoch touched the range.
    No,
    /// The first commit since the epoch that touched it landed at this instant.
    At(std::time::Instant),
    /// The log no longer reaches back to the epoch (or none is kept).
    Unknown,
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
    /// ★ GR tier (batch 2): an ADDRESS register pair on a graphics subchannel — the guest VA the
    /// engine may write `bytes` at, resolved at bind and emitted by the perimeter.
    GrAddress {
        /// The subchannel.
        sub: u32,
        /// The upper register's method.
        upper: u32,
        /// The guest VA.
        va: u64,
        /// The bytes the engine may write there.
        bytes: u64,
    },
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
    /// ★ The kernel-GR tier and the software-subchannel rule (owner rulings 2026-10-07): which
    /// extensions this channel's decoder applies. Both off by default ([`GrConfig::default`]).
    pub gr: GrConfig,
    /// ★ GR tier: the graphics class each hardware subchannel 0-3 is bound to (0 = none; such a
    /// subchannel reaches the CE as before).
    pub gr_subch: [u32; 4],
    /// ★ Software subchannels (5-7) bound to a value that is not an engine class — accepted
    /// silently; any later method of theirs at or above `0x100` is refused by name.
    pub inert_subch: u8,
    /// The value each inert subchannel was bound to (for the refusal's name).
    pub inert_value: [u32; 8],
    /// Inert binds accepted over the channel's life (for the log).
    pub inert_binds: u64,
    /// GR-tier methods re-authored over the channel's life (for the log).
    pub gr_methods: u64,
    /// ★ GR tier (batch 2): each GR subchannel's last `ADDRESS_UPPER` (held until its lower word).
    pub gr_addr_hi: [u32; 4],
}

/// ★ Owner rulings 2026-10-07 (`OWNER_RULINGS.md` §S, items 1-4): what a Translated channel's
/// T-mode decoder may do beyond copy-engine work. Default: nothing.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct GrConfig {
    /// The ring is on the GRAPHICS runlist and its host channel holds a host object of each class
    /// the tier admits ([`crate::grtables::GrClass`]): `SET_OBJECT` of such a class on subchannel 0-3 binds
    /// it, and that subchannel's methods are re-authored from [`crate::grtables`].
    pub tier: bool,
    /// A `SET_OBJECT` on a software subchannel (5-7) of a value no family lists as a class is
    /// accepted silently (inferred hardware behaviour: stored, later methods trap to RM).
    pub inert_sw_subch: bool,
}

#[cfg(test)]
#[path = "tmode_gr_tests.rs"]
mod gr_tier_tests;

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

/// Writes one [`Ir::Words`] item holds at most: each binds to two words, so an item never exceeds
/// a quarter of [`CHUNK_BYTES`] and the chunker (which cuts only BETWEEN items) keeps every piece
/// under the cap however long a run of address-free methods a segment carries.
pub const WORDS_PER_ITEM: usize = CHUNK_BYTES / 32;

fn words_ir(out: &mut Vec<Ir>, w: &[(u32, u32, u32)]) {
    if let Some(Ir::Words(v)) = out.last_mut()
        && v.len() + w.len() <= WORDS_PER_ITEM
    {
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
    if st.inert_subch & bit != 0 {
        // ★ Ruling 4: a software method on a subchannel bound to a non-class value. On bare metal
        // it would trap to the guest's RM; nothing on our host channel may run it.
        return Err(Refusal::InertSubchannelMethod {
            subch: sub,
            method: m,
            value: st.inert_value[(sub & 7) as usize],
        });
    }
    if let Some(&class) = st.gr_subch.get(sub as usize)
        && class != 0
    {
        return gr_write(out, st, sub, class, m, v);
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
        CeMethod::RemapComponents => {
            // ★ Every field named (`clc7b5.h:181-228`: DST_X..W, COMPONENT_SIZE, NUM_SRC/DST);
            // an unnamed bit is refused, never re-emitted (review fix 2026-10-04).
            if v & !REMAP_NAMED != 0 {
                return Err(Refusal::FieldValue {
                    method: m,
                    word: v,
                    what: "SET_REMAP_COMPONENTS beyond its named fields",
                });
            }
            st.regs.remap = v;
        }
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

/// ★ GR tier: one method on a subchannel bound to graphics `class` — admitted only by a row of
/// [`crate::grtables`], its argument re-authored from the row's field; anything else is refused by
/// name (ruling 2). No row carries an address, so nothing here reaches memory.
fn gr_write(
    out: &mut Vec<Ir>,
    st: &mut TState,
    sub: u32,
    class: u32,
    m: u32,
    v: u32,
) -> Result<(), Refusal> {
    use crate::grtables::{Disposition, GrClass, gr_method, reauthor};
    let gc = GrClass::of_class(class).ok_or(Refusal::ForeignClass { subch: sub, class })?;
    match gr_method(gc, m) {
        Disposition::Refused(name) => Err(Refusal::GrMethod {
            class,
            subch: sub,
            method: m,
            name,
        }),
        Disposition::Allowed(row) => {
            let word = reauthor(&row, v).map_err(|what| Refusal::GrField {
                class,
                method: m,
                word: v,
                name: row.name,
                what,
            })?;
            st.gr_methods += 1;
            match row.field {
                crate::grtables::Field::AddressUpper8 => {
                    // Held: emitted (as a window address) with its lower word.
                    if let Some(h) = st.gr_addr_hi.get_mut(sub as usize) {
                        *h = word;
                    }
                }
                crate::grtables::Field::AddressLower32 { upper, bytes } => {
                    let hi = st.gr_addr_hi.get(sub as usize).copied().unwrap_or(0);
                    out.push(Ir::GrAddress {
                        sub,
                        upper,
                        va: (u64::from(hi) << 32) | u64::from(word),
                        bytes,
                    });
                }
                _ => words_ir(out, &[(sub, row.method, word)]),
            }
            Ok(())
        }
    }
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
            st.inert_subch &= !bit;
            if let Some(c) = st.gr_subch.get_mut(sub as usize) {
                *c = 0;
            }
            if v & !NVCLASS_MASK != 0 {
                return Err(Refusal::FieldValue {
                    method: m,
                    word: v,
                    what: "SET_OBJECT beyond NVCLASS (15:0)",
                });
            }
            if st.gr.tier && crate::grtables::GrClass::of_class(class).is_some() {
                // ★ GR tier: a graphics object on a GR hardware subchannel. Subchannel 4 is the
                // GR runlist's fixed copy-engine subchannel (`NVA06F_SUBCHANNEL_COPY_ENGINE`,
                // `cla06fsubch.h`), 5-7 are software: neither may hold one.
                let slot = st
                    .gr_subch
                    .get_mut(sub as usize)
                    .ok_or(Refusal::GrSubchannel { subch: sub, class })?;
                // ★ Authored: the class alone. The host ring must hold a host object of it,
                // allocated at birth from the allowlist (never from this word); its router refuses
                // a class it holds none of.
                *slot = class;
                if let Some(h) = st.gr_addr_hi.get_mut(sub as usize) {
                    *h = 0;
                }
                words_ir(out, &[(sub, 0, class)]);
                return Ok(());
            }
            if st.gr.inert_sw_subch
                && sub > 4
                && class != GP100_UVM_SW
                && !is_ce(class)
                && !crate::grtables::is_known_class(class)
            {
                // ★ Ruling 4 (inferred, untested): stored, nothing emitted; its methods refuse.
                st.inert_subch |= bit;
                st.inert_value[(sub & 7) as usize] = class;
                st.inert_binds += 1;
                return Ok(());
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
    // ★★ A Hopper+ fast scrub on a physical destination becomes a VIRTUAL remap fill of the same
    // bytes with the SAME pattern (review fix 2026-10-04). The scrub writes the pattern the remap
    // registers hold over `LINE_LENGTH_IN` BYTES (remap disabled: one-byte elements): UVM's
    // `memset_8` sends an arbitrary 64-bit value through the scrubber as `CONST_A`/`CONST_B` with
    // `DST_X = CONST_A`, `DST_Y = CONST_B`, four-byte components ×2
    // (`ogkm-580: kernel-open/nvidia-uvm/uvm_hopper_ce.c:298-310`, `:232-258`; `memset_1/4` widen
    // their value to it, `:313-320`), and CeUtils sets its pattern the same way
    // (`src/nvidia/src/kernel/gpu/mem_mgr/channel_utils.c:617-628`). ⊘ Before this fix T-mode
    // authored a ZERO fill (`CONST_A = 0`, one-byte `DST_X`), dropping the guest's pattern —
    // UVM's non-zero PDE `clear_bits` included (`uvm_mmu.c:475-498`). So the fill keeps the
    // guest's `CONST_A`/`CONST_B`/`SET_REMAP_COMPONENTS` and counts `LINE_LENGTH_IN` in pattern
    // elements. A remap that reads a source, or a byte count that is not whole elements, is
    // refused by name. UNVERIFIED on hardware: the scrubber's own semantics are closed firmware;
    // the source above is what fixes them here.
    let mut l = l;
    let mut regs = st.regs;
    if l.scrub {
        let what = if !l.dst_phys || !tier.has_fast_scrub() {
            Some("MEMORY_SCRUB on a virtual destination")
        } else {
            None
        };
        let (_, d, reads) = remap_elems(st.regs.remap);
        let what = what.or_else(|| {
            if reads {
                Some("MEMORY_SCRUB with a remap that reads a source (no pattern)")
            } else if !u64::from(st.regs.line_len).is_multiple_of(d) {
                Some("MEMORY_SCRUB over bytes that are not whole pattern elements")
            } else {
                None
            }
        });
        if let Some(what) = what {
            return Err(Refusal::FieldValue {
                method: ce::LAUNCH_DMA,
                word: crate::ttables::encode_launch(&l),
                what,
            });
        }
        (src_elem, dst_elem, reads_src) = (1, d, false);
        l.scrub = false;
        l.remap = true;
        regs.line_len = u32::try_from(u64::from(st.regs.line_len) / d).unwrap_or(u32::MAX);
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

/// A perimeter refusal, as the rewriter names it.
const fn perimeter(r: PerimeterRefusal) -> Refusal {
    match r {
        PerimeterRefusal::Sem40 { va } => Refusal::Sem40 { va },
        PerimeterRefusal::Footprint { side, need, have } => Refusal::Footprint { side, need, have },
        PerimeterRefusal::Operands { what } | PerimeterRefusal::Field { what } => {
            Refusal::Perimeter { what }
        }
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
        Ir::GrAddress {
            sub,
            upper,
            va,
            bytes,
        } => {
            // ★ Validated as a guest VA of this channel's space (one writable row), inside the
            // VM's windows, then emitted only by the perimeter.
            let a = sema_addr(*va, *bytes, true, false, rows, w)?;
            crate::tspace_unsafe::put_gr_address(out, *sub, *upper, a, *bytes)
                .map_err(perimeter)?;
            Ok(0)
        }
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
            put_host_semaphore(&mut words, h.sub, a, payload, &op).map_err(perimeter)?;
        }
        HostSemForm::Execute {
            payload_lo,
            payload_hi,
            op,
            wide,
        } => {
            let a = sema_addr(h.va, op.bytes(), op.writes(), op.op == 6, rows, w)?;
            put_host_sem_execute(&mut words, h.sub, a, (payload_lo, payload_hi), &op, wide)
                .map_err(perimeter)?;
        }
    }
    out.extend(words);
    Ok(())
}

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
        Some(s) => Some((s, sema_addr(s.va, s.bytes, true, s.reduction, rows, w)?)),
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
        // Each side's validated bytes for THIS piece; the perimeter recomputes the engine's
        // footprint from the registers it emits and refuses the piece unless it fits them.
        let side_addr =
            |side: &Option<(Side, u64)>, elem: u64| -> Result<Option<WindowAddr>, Refusal> {
                let Some((s, n)) = side else {
                    return Ok(None);
                };
                let (pos, len) = if n_pieces > 1 {
                    (e0 * elem, (e1 - e0) * elem)
                } else {
                    (0, *n)
                };
                piece_addr(s, pos, len, w)
                    .map(Some)
                    .ok_or(Refusal::ExtentOverflow)
            };
        let piece = LaunchPiece {
            sub: l.sub,
            tier: l.tier,
            launch: f,
            regs: PieceRegs {
                line_len: if n_pieces > 1 {
                    line_len
                } else {
                    l.regs.line_len
                },
                line_count: l.regs.line_count,
                pitch_in: l.regs.pitch_in,
                pitch_out: l.regs.pitch_out,
                remap: l.regs.remap,
                const_a: l.regs.const_a,
                const_b: l.regs.const_b,
                req_attr: l.req_attr,
            },
            src: side_addr(&src, se)?,
            dst: side_addr(&dst, de)?,
            sema: sema.filter(|_| f.sema != 0).map(|(s, at)| PieceSema {
                at,
                payload: s.payload,
                upper: s.payload_upper.filter(|_| f.payload_two_word),
            }),
        };
        put_launch_piece(&mut words, &piece).map_err(perimeter)?;
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
    /// ★ Review fix 2026-10-04: items that followed an `Ir::Invalidate` in their segment — held
    /// UNBOUND and bound at the NEXT observed segment, after the default path ran that segment's
    /// splits (their walks): the rows T-mode would bind them against (§3.5).
    deferred: Vec<Ir>,
    /// Of the items bound, those bound late that way.
    pub bound_after_split: u64,
    /// `KF3_NEGCTL_SHADOW` — the positive control ([`Shadow::negctl_probe`]).
    pub negctl: bool,
}

/// `NVC56F_WFI` (`ogkm-580: src/common/sdk/nvidia/inc/class/clc56f.h`).
const HOST_WFI: u32 = 0x78;

/// No placement row at all — the positive control's resolver.
struct NoRows;
impl Rows for NoRows {
    fn resolve(&self, va: u64, _: u64) -> Result<Vec<Span>, u64> {
        Err(va)
    }
    fn dma_to_file_range(&self, _: u64, _: u64) -> Option<u64> {
        None
    }
}

impl Shadow {
    /// A shadow, with its positive control on or off ([`Shadow::negctl_probe`]).
    #[must_use]
    pub fn with_negctl(negctl: bool) -> Shadow {
        Shadow {
            negctl,
            ..Shadow::default()
        }
    }

    /// Observe one segment as T-mode would decode and bind it. Stops at the first refusal of the
    /// segment (T-mode would have stopped the channel there).
    ///
    /// ★ Review fix 2026-10-04: T-mode binds the items after a segment's invalidate only AFTER the
    /// split's walk (§3.5), so the shadow binds the items before the segment's first
    /// `Ir::Invalidate` now and holds the rest UNBOUND until the next segment is observed — by
    /// then the default path has run every split of this one (the ring hands out no new segment
    /// while a split is pending). Binding them at fetch would count correct UVM streams (write the
    /// PTEs, invalidate, use the new mapping in one push) as resolution misses.
    pub fn observe(&mut self, words: &[u32], is_ce: IsCeClass, rows: &dyn Rows, w: &TWindows) {
        let late = std::mem::take(&mut self.deferred);
        if !late.is_empty() {
            let n = late
                .iter()
                .filter(|i| !matches!(i, Ir::Invalidate { .. }))
                .count();
            if self.bind_items(&late, rows, w) {
                self.bound_after_split += n as u64;
            }
        }
        self.segments += 1;
        if self.negctl {
            self.negctl_probe(is_ce, w);
        }
        let mut items = match decode(words, is_ce, &mut self.st, None) {
            Ok(v) => v,
            Err(r) => return self.refused(&r),
        };
        if let Some(k) = items
            .iter()
            .position(|i| matches!(i, Ir::Invalidate { .. }))
        {
            self.deferred = items.split_off(k + 1);
        }
        if !self.bind_items(&items, rows, w) {
            self.deferred.clear();
        }
    }

    /// Bind `items` in order, counting; `false` at the first refusal (the rest are not bound).
    fn bind_items(&mut self, items: &[Ir], rows: &dyn Rows, w: &TWindows) -> bool {
        for it in items {
            let mut sink = Vec::new();
            match bind(it, rows, w, &mut sink) {
                Ok(p) => {
                    self.items += 1;
                    if matches!(it, Ir::Launch(_)) {
                        self.launches += 1;
                        self.max_pieces = self.max_pieces.max(p);
                    }
                }
                Err(r) => {
                    self.refused(&r);
                    return false;
                }
            }
        }
        true
    }

    /// ★ `KF3_NEGCTL_SHADOW=1` — the shadow counters' POSITIVE CONTROL (review fix 2026-10-04): per
    /// observed segment, three synthetic writes go through the REAL decode and bind on a copy of
    /// the channel's decoder state — an unclassified host method, a `WFI` with a bit beyond its
    /// field, and a host release no placement row covers — so `unclassified`, `unknown_field`,
    /// `resolve_miss` and `would_refuse` MUST move on any run that observes a segment. Their output
    /// is discarded like everything the shadow binds; the channel's own state is untouched.
    pub fn negctl_probe(&mut self, is_ce: IsCeClass, w: &TWindows) {
        let probes: [&[u32]; 3] = [
            // An unclassified host method (`0x7C`).
            &[method_header_inc(0, 0x7C, 1).unwrap_or(0), 0],
            // WFI with a bit beyond SCOPE (0:0).
            &[method_header_inc(0, HOST_WFI, 1).unwrap_or(0), 2],
            // SEM_ADDR_LO..SEM_EXECUTE: a 32-bit RELEASE at a VA no row covers.
            &[
                method_header_inc(0, fifo::SEM_ADDR_LO, 5).unwrap_or(0),
                0x1000,
                0,
                1,
                0,
                1,
            ],
        ];
        for words in probes {
            let mut st = self.st;
            match decode(words, is_ce, &mut st, None) {
                Err(r) => self.refused(&r),
                Ok(items) => {
                    for it in &items {
                        if let Err(r) = bind(it, &NoRows, w, &mut Vec::new()) {
                            self.refused(&r);
                        }
                    }
                }
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
            "segments={} items={} launches={} max_pieces={} would_refuse=[{}] resolve_miss={} unknown_field={} unclassified={} bound_after_split={} unbound_at_free={}{}",
            self.segments,
            self.items,
            self.launches,
            self.max_pieces,
            wr.join(" "),
            self.resolve_miss,
            self.unknown_field,
            self.unclassified,
            self.bound_after_split,
            self.deferred.len(),
            if self.negctl { " NEGCTL" } else { "" }
        )
    }
}

/// ★ P1+P2 inc D (§3.5, §7.13): one virtual resolution a bound piece used — what the stale-bind
/// counter re-resolves when the piece's fence retires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bound {
    /// The operand's VA.
    pub va: u64,
    /// Its bytes.
    pub len: u64,
    /// The spans it resolved to.
    pub spans: Vec<Span>,
    /// ★ The rows' epoch it was resolved at ([`Rows::resolve_epoch`]).
    pub epoch: u64,
}

/// A [`Rows`] that records every resolution it answers.
struct Recording<'a> {
    inner: &'a dyn Rows,
    rec: std::cell::RefCell<Vec<Bound>>,
}

impl Rows for Recording<'_> {
    fn resolve(&self, va: u64, len: u64) -> Result<Vec<Span>, u64> {
        let (r, epoch) = self.inner.resolve_epoch(va, len);
        let r = r?;
        self.rec.borrow_mut().push(Bound {
            va,
            len,
            spans: r.clone(),
            epoch,
        });
        Ok(r)
    }
    fn dma_to_file_range(&self, dma: u64, len: u64) -> Option<u64> {
        self.inner.dma_to_file_range(dma, len)
    }
}

/// [`bind`], recording the virtual resolutions the item used into `rec`.
///
/// # Errors
/// As [`bind`]; nothing is recorded for a refused item.
pub fn bind_rec(
    ir: &Ir,
    rows: &dyn Rows,
    w: &TWindows,
    out: &mut Vec<u32>,
    rec: &mut Vec<Bound>,
) -> Result<usize, Refusal> {
    let r = Recording {
        inner: rows,
        rec: std::cell::RefCell::new(Vec::new()),
    };
    let n = bind(ir, &r, w, out)?;
    rec.extend(r.rec.into_inner());
    Ok(n)
}

/// What [`push_bound`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pushed {
    /// Every item was bound and pushed, in `pieces` pushes.
    All {
        /// Pushes made.
        pieces: usize,
    },
    /// The host ring was full: the items from `rest` on were NOT pushed and stay UNBOUND — they
    /// are bound again, against the rows as they are then, when the push is retried (§3.5).
    Busy {
        /// The unpushed items, unbound.
        rest: Vec<Ir>,
        /// Pushes made before the ring filled.
        pieces: usize,
    },
    /// ★ Review fix 2026-10-04 (§3.5): an item's virtual operand had NO placement row at bind —
    /// `rest` (that item on) was NOT pushed and stays UNBOUND; everything before it was. The
    /// runner WAITS (a walk pending on the space, or a host acquire ahead of it whose release
    /// another channel's walk may precede) and binds `rest` again, or refuses `why` by name
    /// (unmapped).
    Unresolved {
        /// The unpushed items, unbound.
        rest: Vec<Ir>,
        /// Pushes made before it.
        pieces: usize,
        /// The refusal, should the runner not wait.
        why: Refusal,
    },
}

/// ★ A host semaphore ACQUIRE (`SEMAPHORED` ACQUIRE/ACQ_GEQ/ACQ_AND, `SEM_EXECUTE` ACQUIRE/
/// ACQ_STRICT_GEQ/ACQ_CIRC_GEQ/ACQ_AND/ACQ_NOR): the work behind it waits on another channel's
/// release — possibly made after that channel's own invalidate and walk.
#[must_use]
pub const fn is_acquire(ir: &Ir) -> bool {
    match ir {
        Ir::HostSem(HostSem {
            form: HostSemForm::Legacy { op, .. },
            ..
        }) => matches!(op.op, 1 | 4 | 8),
        Ir::HostSem(HostSem {
            form: HostSemForm::Execute { op, .. },
            ..
        }) => matches!(op.op, 0 | 2 | 3 | 4 | 5),
        _ => false,
    }
}

/// Why a [`push_bound`] stopped the channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushError {
    /// An item was refused at bind, by name.
    Refused(Refusal),
    /// The host push failed.
    Host(String),
}

/// ★★ **Bind at submit** (§3.5): bind `items` one at a time against `rows` AS THEY ARE NOW, cut the
/// output into pieces of at most [`CHUNK_BYTES`], and hand each piece to `push` (`Ok(true)` = taken,
/// `Ok(false)` = the ring is full). A piece the ring refuses is NOT kept bound: its items are
/// returned unbound in [`Pushed::Busy`]. `rec` gathers the resolutions of the pieces that WERE
/// pushed (for the stale-bind counter).
///
/// # Errors
/// [`PushError`]: a bind refusal (the channel is dead) or a host failure.
pub fn push_bound(
    items: &[Ir],
    rows: &dyn Rows,
    w: &TWindows,
    rec: &mut Vec<Bound>,
    mut push: impl FnMut(&[u32]) -> Result<bool, String>,
) -> Result<Pushed, PushError> {
    let mut pieces = 0usize;
    let mut cur: Vec<u32> = Vec::new();
    let mut cur_rec: Vec<Bound> = Vec::new();
    let mut cur_first = 0usize;
    let mut i = 0usize;
    while i < items.len() {
        let mut words = Vec::new();
        let mut r = Vec::new();
        match bind_rec(&items[i], rows, w, &mut words, &mut r) {
            Ok(_) => {}
            Err(why @ Refusal::VirtualUnresolved { .. }) => {
                // ★ Push what is bound before it; hand the rest back UNBOUND (§3.5).
                if !cur.is_empty() {
                    if !push(&cur).map_err(PushError::Host)? {
                        return Ok(Pushed::Busy {
                            rest: items[cur_first..].to_vec(),
                            pieces,
                        });
                    }
                    pieces += 1;
                    rec.append(&mut cur_rec);
                }
                return Ok(Pushed::Unresolved {
                    rest: items[i..].to_vec(),
                    pieces,
                    why,
                });
            }
            Err(why) => return Err(PushError::Refused(why)),
        }
        if !cur.is_empty() && 4 * (cur.len() + words.len()) > CHUNK_BYTES {
            if !push(&cur).map_err(PushError::Host)? {
                return Ok(Pushed::Busy {
                    rest: items[cur_first..].to_vec(),
                    pieces,
                });
            }
            pieces += 1;
            rec.append(&mut cur_rec);
            cur.clear();
            cur_first = i;
        }
        cur.extend(words);
        cur_rec.extend(r);
        i += 1;
    }
    if !cur.is_empty() {
        if !push(&cur).map_err(PushError::Host)? {
            return Ok(Pushed::Busy {
                rest: items[cur_first..].to_vec(),
                pieces,
            });
        }
        pieces += 1;
        rec.append(&mut cur_rec);
    }
    Ok(Pushed::All { pieces })
}

/// ★ P1+P2 inc D (§3.5, §7.13) — **the stale-bind counter**, review fix 2026-10-04.
/// Bind-at-submit is translation at PUSH time, not at execution time across channels: a piece
/// pushed behind a host acquire was bound before another channel's walk could complete. Stock
/// producers appear not to depend on it (UNVERIFIED), so every bound piece's resolutions are
/// checked when its fence retires against the rows' COMMIT LOG ([`Rows::changed_since`]): a commit
/// to an operand's rows after the bind's epoch is classified by WHEN it landed —
///
/// - `stale`: before the last time the runner read the piece's fence and found it INCOMPLETE —
///   the walk committed while the engine had not finished the piece, so (within the few
///   microseconds a `RELEASE_WFI` fence trails the guest's own release) BEFORE the guest saw the
///   piece's work complete: the piece may have run on a translation the guest had already
///   replaced. This is the gated count (§11 decision 6: non-zero ⇒ a host acquire becomes a bind
///   barrier).
/// - `late`: after it — the benign shape (the guest saw the release, then unmapped and remapped;
///   UVM does this routinely) as well as an indeterminate tail; never gated.
/// - `indeterminate`: the log no longer reaches back to the bind's epoch.
///
/// ⊘ Before this fix the counter re-resolved at retire and counted ANY difference: correct runs
/// that remap after the guest-visible release counted as stale, and the gate required 0.
#[derive(Debug, Default)]
pub struct StaleBind {
    pending: std::collections::VecDeque<(u32, Vec<Bound>, std::time::Instant)>,
    /// Resolutions checked.
    pub checked: u64,
    /// Of those, changed while the piece's fence was still incomplete (gated).
    pub stale: u64,
    /// Of those, changed after the last incomplete reading (benign or indeterminate; not gated).
    pub late: u64,
    /// Of those, beyond the commit log's reach.
    pub indeterminate: u64,
    /// `KF3_NEGCTL_STALE_BIND` — the positive control: every check reads as a change made while the
    /// fence was incomplete, so `stale` MUST move on the first retire that checks a resolution.
    pub negctl: bool,
}

impl StaleBind {
    /// The resolutions `recs` were bound under, covered by fence `seq`, pushed at `now`.
    pub fn record(&mut self, seq: u32, recs: Vec<Bound>, now: std::time::Instant) {
        if recs.is_empty() {
            return;
        }
        self.pending.push_back((seq, recs, now));
    }

    /// The fence read `done` at `now`: every record not yet reached was INCOMPLETE now; every one
    /// reached is checked against the rows' commit log.
    pub fn observe(&mut self, done: u32, rows: &dyn Rows, now: std::time::Instant) {
        while self
            .pending
            .front()
            .is_some_and(|&(s, _, _)| crate::host::reached(done, s))
        {
            let Some((_, recs, incomplete_at)) = self.pending.pop_front() else {
                break;
            };
            for b in recs {
                self.checked += 1;
                let c = if self.negctl {
                    Changed::At(incomplete_at)
                } else {
                    rows.changed_since(b.epoch, b.va, b.len)
                };
                match c {
                    Changed::No => {}
                    Changed::At(t) if t <= incomplete_at => self.stale += 1,
                    Changed::At(_) => self.late += 1,
                    Changed::Unknown => self.indeterminate += 1,
                }
            }
        }
        for p in &mut self.pending {
            p.2 = now;
        }
    }

    /// `stale_binds=stale/checked late=… indeterminate=…`.
    #[must_use]
    pub fn line(&self) -> String {
        format!(
            "stale_binds={}/{} late={} indeterminate={}",
            self.stale, self.checked, self.late, self.indeterminate
        )
    }
}
