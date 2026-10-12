//! Unconnected C56F user-stream snapshot/parser prototype for OWNER_RULINGS §AI.
//!
//! Unlike T-mode, engine operands stay in the channel's OWN restricted user VAS.
//! This module creates no mapping, copies no GP entry to hardware and completes
//! no work. A future adapter must select the supported class at birth, enforce
//! GPU-read-only commands/staging, deny physical operands, pin source lifetimes,
//! serialize against VA-manager changes and provide real ordered completion.
//! Linux Passthrough has no runtime call to this module and no new mapping.
//!
//! Parsing uses an owned bounded snapshot. Software calls never appear as
//! hardware writes. Guest references are typed operations: user_batch authors
//! real native SET_REFERENCE packets; parsing itself completes nothing.
//! Object lookup must be channel-owned host
//! metadata; registration reply timing is irrelevant to the parsing mode.
//! Unsupported packet forms are refused, not silently copied. The model
//! carries bounded packet metadata across GP entries. Control entries other
//! than public GPFIFO NOP remain refused. LEVEL1 suppresses top-level
//! progress reporting; it is not nested execution.

use crate::tmode::SwCall;
use crate::ttables::{HostMethod, host_method};
use kf_abi::submit::{
    GpfifoEntry, MethodForm, MethodHeader, fifo, gp_entry_decode_direct_c56f, method_header_decode,
    method_header_inc,
};

/// A validated direct source descriptor, still requiring full pinned resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Public CONTROL/NOP, including SYNC_WAIT/LEVEL; no PB data is fetched.
    /// Original descriptor must reach native hardware; no CPU credit is granted.
    Nop,
    /// Source span and explicit SYNC_WAIT bit. Never a host GP entry to copy raw.
    Segment(GpfifoEntry),
}

/// Check one captured C56F GP word before copying any pushbuffer. No indirect,
/// conditional, privileged/reserved or extended-base entry can bypass inspection.
/// The TRUSTED worker's byte budget bounds the subsequent snapshot, not guest PUT.
///
/// # Errors
/// Unsupported entry, over-budget or out-of-format span. No mapping is resolved.
pub fn inspect_entry(entry: u64, maximum_bytes: u64) -> Result<Source, Refusal> {
    // Public clc56f.h: LENGTH0/OPCODE_NOP; dev_pbdma.ref.txt:107-115:
    // no operation or PB fetch, but SYNC is respected. OPERAND is unused.
    // Reject all other upper bits (including reserved bit8); preserve the raw
    // descriptor in the caller so SYNC_WAIT is never converted to a CPU noop.
    if (entry >> 32) & !((1_u64 << 31) | (1_u64 << 9)) == 0 {
        return Ok(Source::Nop);
    }
    let segment = gp_entry_decode_direct_c56f(entry).ok_or(Refusal::Entry)?;
    if segment.len_bytes > maximum_bytes {
        return Err(Refusal::Budget);
    }
    // Public NVIDIA dev_pbdma.ref: PB PUT names the first dword beyond the
    // segment, so even a span ENDING exactly at2^40 cannot be represented.
    if segment
        .gpu_va
        .checked_add(segment.len_bytes)
        .is_none_or(|end| end >= 1 << 40)
    {
        return Err(Refusal::Entry);
    }
    Ok(Source::Segment(segment))
}

/// A method stream copied into private host memory; no borrowed guest storage.
#[derive(Debug)]
pub struct Commands(Vec<u32>);

impl Commands {
    /// Copy within a TRUSTED worker/per-VM word budget. A later hostile mutation
    /// of the source cannot change what is decoded/emitted from this snapshot.
    ///
    /// # Errors
    /// Budget or bounded allocation failure. Mapping/copy lifetime is external.
    pub fn copy_from(words: &[u32], maximum: usize) -> Result<Self, Refusal> {
        if words.len() > maximum {
            return Err(Refusal::Budget);
        }
        let mut copy = Vec::new();
        copy.try_reserve_exact(words.len())
            .map_err(|_| Refusal::Allocation)?;
        copy.extend_from_slice(words);
        Ok(Self(copy))
    }
}

/// A binding from the channel's own validated object registry, never guest flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binding {
    /// A hardware class supported and allocated under this channel by its owner.
    Hardware {
        /// Native class binding authored by the future adapter.
        class: u32,
    },
    /// A software object numbered within this channel (e.g. its5080 object).
    Software {
        /// Its channel-scoped software classID, used by SwCall lookup.
        value: u32,
    },
}

/// Stream metadata, cloned transactionally while a snapshot is checked.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct State {
    bindings: [Option<Binding>; 8],
    pending: Option<Packet>,
    mem_op: [u32; 3],
    mem_op_seen: u8,
}

/// Only packet metadata survives a source snapshot. No borrowed guest bytes,
/// unbounded buffered payload or native header is held for a continuation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Packet {
    header: MethodHeader,
    consumed: usize,
}

impl State {
    /// A packet still needs data from a later immutable GP-entry snapshot.
    /// A future adapter must keep source receipts/pins for it: accepting the
    /// prefix does not retire its source or synthesize a completion.
    #[must_use]
    pub const fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Check an explicitly terminated stream, never an idle doorbell or timeout.
    /// Partial packets may legally wait indefinitely for another source entry.
    ///
    /// # Errors
    /// A packet has declared arguments not supplied by the stream.
    pub fn finish(&self) -> Result<(), Refusal> {
        if let Some(p) = self.pending {
            return Err(Refusal::Truncated { at: p.consumed });
        }
        Ok(())
    }
}

/// Bounded unbound operations, not GPU words or executed actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    /// A hardware SET_OBJECT, re-authored from validated native class metadata.
    Bind {
        /// Subchannel.
        sub: u32,
        /// Native class.
        class: u32,
    },
    /// A user hardware method. Its addresses remain in the restricted user VAS;
    /// this is NEVER a T-space IR or permission to use kernel physical operands.
    Hardware {
        /// Subchannel.
        sub: u32,
        /// Byte method address.
        method: u32,
        /// Method value.
        data: u32,
    },
    /// An ordered software split, classified later against the channel registry.
    Software(SwCall),
    /// VA-manager reconcile/invalidate of this channel's OWN restricted space.
    /// The guest PDB operand is never forwarded or used to select another space.
    OwnInvalidate,
    /// A requested native SET_REFERENCE, distinct from CPU bookkeeping. The
    /// submission adapter must author this method on hardware. A later HOST
    /// receipt may attribute its real processing; parsing completes nothing.
    GuestReference(u32),
}

/// Named refusals; state commits only when the whole snapshot passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Unsupported C56F GP entry or span beyond its40-bit address format.
    Entry,
    /// Snapshot exceeds the trusted word budget.
    Budget,
    /// Bounded allocation failed.
    Allocation,
    /// Unknown/reserved header, reserved bit or unsupported packet form.
    Header {
        /// Word index.
        at: usize,
    },
    /// A packet's declared arguments exceed the snapshot.
    Truncated {
        /// Argument start.
        at: usize,
    },
    /// SET_OBJECT does not name this channel's validated registry entry.
    Object {
        /// Guest binding value.
        value: u32,
    },
    /// An engine method has no validated binding.
    Unbound {
        /// Subchannel.
        sub: u32,
    },
    /// Unclassified/refused FIFO method or address beyond the C56F packet field.
    Method {
        /// Byte method address.
        method: u32,
    },
}

/// Decode one immutable snapshot with no I/O/locks. An authored MEMBAR/L2
/// operation can expand one datum into four methods; output is bounded by five
/// times the trusted snapshot budget. Planning applies its own operation cap.
/// The adapter must execute Software splits serially, not publish later operations
/// until their VA/action gate lands, and never convert bindings into a raw bypass.
///
/// # Errors
/// Refusal leaves the input state unchanged and exposes no partial output.
pub fn decode(
    commands: &Commands,
    state: &mut State,
    lookup: impl FnMut(u32) -> Option<Binding>,
) -> Result<Vec<Op>, Refusal> {
    decode_fragment(commands, state, lookup, false)
}

/// Decode one source fragment, retaining a bounded unfinished packet in `state`.
/// Every fragment is an owned snapshot with a trusted word budget. Output count
/// is at most five times this fragment's word count; a header can claim at most the public
/// C56F field's count, and no allocation depends on unsupplied arguments.
///
/// Returned operations are inspection IR, not work or source retirement. A
/// future adapter must preserve their order across fragments/software gates,
/// retain contributing source lifetimes until a real completion, and honor GP
/// SYNC_WAIT independently. No partial native packet is forwarded.
///
/// # Errors
/// Any refusal leaves state unchanged and exposes no output from this fragment.
/// Previously accepted fragments cannot be undone; a refused live stream must
/// be stopped/quiesced, never bypassed or released on a timer.
pub fn feed(
    commands: &Commands,
    state: &mut State,
    lookup: impl FnMut(u32) -> Option<Binding>,
) -> Result<Vec<Op>, Refusal> {
    decode_fragment(commands, state, lookup, true)
}

fn method_at(h: MethodHeader, index: usize) -> Result<u32, Refusal> {
    let method = match h.form {
        MethodForm::Immediate | MethodForm::NonIncrementing => h.method,
        MethodForm::IncrementOnce if index > 0 => h.method + 4,
        _ => h.method + 4 * index as u32,
    };
    // Expanded method arithmetic is bounded by the public 13-bit count. Check
    // the last method at header admission as well as every consumed argument.
    if method_header_inc(h.subchannel, method, 1).is_none() {
        return Err(Refusal::Method { method });
    }
    Ok(method)
}

fn decode_fragment(
    commands: &Commands,
    state: &mut State,
    mut lookup: impl FnMut(u32) -> Option<Binding>,
    fragmented: bool,
) -> Result<Vec<Op>, Refusal> {
    let words = &commands.0;
    let mut next = state.clone();
    let mut out = Vec::new();
    let maximum = words.len().checked_mul(5).ok_or(Refusal::Budget)?;
    out.try_reserve_exact(maximum)
        .map_err(|_| Refusal::Allocation)?;
    let mut at = 0usize;
    while at < words.len() {
        if let Some(mut packet) = next.pending {
            let m = method_at(packet.header, packet.consumed)?;
            apply(
                &mut next,
                &mut out,
                packet.header.subchannel,
                m,
                words[at],
                &mut lookup,
            )?;
            packet.consumed += 1;
            next.pending = (packet.consumed < packet.header.arg_words).then_some(packet);
            at += 1;
            continue;
        }
        let raw = words[at];
        // A zero is NOP only at a header position, never as a pending datum.
        if raw == 0 {
            at += 1;
            continue;
        }
        let h = method_header_decode(raw).ok_or(Refusal::Header { at })?;
        if raw & (1 << 12) != 0
            || matches!(
                h.form,
                MethodForm::Legacy | MethodForm::SubDeviceMask | MethodForm::EndPbSegment
            )
            || (h.form != MethodForm::Immediate && h.arg_words == 0)
        {
            return Err(Refusal::Header { at });
        }
        let count = if h.form == MethodForm::Immediate {
            1
        } else {
            h.arg_words
        };
        method_at(h, count - 1)?;
        at += 1;
        if h.form == MethodForm::Immediate {
            apply(
                &mut next,
                &mut out,
                h.subchannel,
                h.method,
                h.immd,
                &mut lookup,
            )?;
        } else {
            if !fragmented && count > words.len() - at {
                return Err(Refusal::Truncated { at });
            }
            next.pending = Some(Packet {
                header: h,
                consumed: 0,
            });
        }
    }
    if !fragmented {
        next.finish()?;
    }
    *state = next;
    Ok(out)
}

fn apply(
    state: &mut State,
    out: &mut Vec<Op>,
    subchannel: u32,
    m: u32,
    data: u32,
    lookup: &mut impl FnMut(u32) -> Option<Binding>,
) -> Result<(), Refusal> {
    if m == 0 {
        let binding = lookup(data).ok_or(Refusal::Object { value: data })?;
        state.bindings[subchannel as usize] = Some(binding);
        if let Binding::Hardware { class } = binding {
            out.push(Op::Bind {
                sub: subchannel,
                class,
            });
        }
    } else if m == fifo::SET_REFERENCE_C56F {
        out.push(Op::GuestReference(data));
    } else if m < 0x100 {
        match host_method(m) {
            Some(HostMethod::MemOpA) => {
                state.mem_op[0] = data;
                state.mem_op_seen |= 1;
            }
            Some(HostMethod::MemOpB) => {
                state.mem_op[1] = data;
                state.mem_op_seen |= 2;
            }
            Some(HostMethod::MemOpC) => {
                state.mem_op[2] = data;
                state.mem_op_seen |= 4;
            }
            Some(HostMethod::MemOpD) => {
                if state.mem_op_seen != 7 {
                    return Err(Refusal::Method { method: m });
                }
                mem_op(out, state, subchannel, data)?;
                state.mem_op_seen = 0;
            }
            Some(HostMethod::Refused(_)) | None => {
                return Err(Refusal::Method { method: m });
            }
            _ => out.push(Op::Hardware {
                sub: subchannel,
                method: m,
                data,
            }),
        }
    } else {
        match state.bindings[subchannel as usize] {
            Some(Binding::Hardware { .. }) => out.push(Op::Hardware {
                sub: subchannel,
                method: m,
                data,
            }),
            Some(Binding::Software { value }) => out.push(Op::Software(SwCall {
                sub: subchannel,
                value,
                method: m,
                data,
            })),
            None => return Err(Refusal::Unbound { sub: subchannel }),
        }
    }
    Ok(())
}

/// Same public FIFO contract as T-mode, but invalidation has no guest-selected
/// PDB target and cannot be emitted as native GPU words in a user-space plan.
fn mem_op(out: &mut Vec<Op>, state: &State, sub: u32, d: u32) -> Result<(), Refusal> {
    use crate::tmode::{
        MEM_OP_A_SYSMEMBAR, OP_ACCESS_COUNTER_CLR, OP_MEMBAR, OP_TLB_INVALIDATE,
        OP_TLB_INVALIDATE_TARGETED, OPS_L2,
    };
    let [a, b, c] = state.mem_op;
    let op = d >> 27;
    let refuse = || Refusal::Method {
        method: fifo::MEM_OP_A + 12,
    };
    // Every operation adds at most four authored methods plus one typed split.
    out.try_reserve(5).map_err(|_| Refusal::Allocation)?;
    let mut emit = |words: &[(u32, u32, u32)]| {
        out.extend(
            words
                .iter()
                .map(|&(sub, method, data)| Op::Hardware { sub, method, data }),
        );
    };
    if op == OP_MEMBAR {
        if a != 0 || b != 0 || c > 1 || d & 0x07ff_ffff != 0 {
            return Err(refuse());
        }
        emit(&crate::tmode::membar(sub, c));
    } else if op == OP_TLB_INVALIDATE || op == OP_TLB_INVALIDATE_TARGETED {
        // Public clc56f.h145-178 and dev_pbdma.ref4178-4263: this clean
        // subset serves GPC_ENABLE, REPLAY_NONE, GLOBAL ACK, all page levels
        // and ALL_TLBS scope only. Replay/cancel is a different operation.
        let all = c & 1 != 0;
        let targeted = op == OP_TLB_INVALIDATE_TARGETED;
        if c & 0x3fe != 0x20
            || c & 0xc00 == 0x400
            || (all && (targeted || c & 0xffff_fc00 != 0 || d & 0x07ff_ffff != 0))
            || a & 0x7ff != 0
            || (!targeted && (a & 0xffff_f000 != 0 || b != 0))
        {
            return Err(refuse());
        }
        out.push(Op::OwnInvalidate);
        // dev_pbdma.ref4161-4166,4278-4291: SYSMEMBAR follows successful
        // invalidate + GLOBAL ACK, never precedes it. The runtime must prove
        // that backend contract before marking the typed action done.
        if a & MEM_OP_A_SYSMEMBAR != 0 {
            out.extend(
                crate::tmode::membar(sub, 0)
                    .iter()
                    .map(|&(sub, method, data)| Op::Hardware { sub, method, data }),
            );
        }
    } else if OPS_L2.contains(&op) {
        if a != 0 || b != 0 || c != 0 || d & 0x07ff_ffff != 0 {
            return Err(refuse());
        }
        emit(&[
            (sub, fifo::MEM_OP_A, 0),
            (sub, fifo::MEM_OP_A + 4, 0),
            (sub, fifo::MEM_OP_A + 8, 0),
            (sub, fifo::MEM_OP_A + 12, op << 27),
        ]);
    } else if op != OP_ACCESS_COUNTER_CLR {
        return Err(refuse());
    }
    Ok(())
}
