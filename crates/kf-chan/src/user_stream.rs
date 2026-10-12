//! Unconnected C56F user-stream snapshot/parser prototype for OWNER_RULINGS §AI.
//!
//! Unlike T-mode, engine operands stay in the channel's OWN restricted user VAS.
//! This module creates no mapping, copies no GP entry to hardware and completes
//! no work. A future adapter must select the supported class at birth, enforce
//! GPU-read-only commands/staging, deny physical operands, pin source lifetimes,
//! serialize against VA-manager changes and provide real ordered completion.
//! Linux Passthrough has no runtime call to this module and no new mapping.
//!
//! Parsing uses an owned bounded snapshot. Software calls and guest references
//! never appear as hardware writes. Object lookup must be channel-owned host
//! metadata; registration reply timing is irrelevant to the parsing mode.
//! Unsupported packet forms are refused, not silently copied. The model
//! carries bounded packet metadata across GP entries. Subroutine/control entries
//! other than the canonical zero GPFIFO NOP remain refused.

use crate::tmode::SwCall;
use crate::ttables::{HostMethod, host_method};
use kf_abi::submit::{
    GpfifoEntry, MethodForm, MethodHeader, fifo, gp_entry_decode_direct_c56f, method_header_decode,
    method_header_inc,
};

/// A validated direct source descriptor, still requiring full pinned resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The canonical zero GPFIFO NOP; no work/completion is claimed here.
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
    if entry == 0 {
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
    /// Guest reference bookkeeping ONLY. It is not emitted as a native reference
    /// and cannot complete work. Publish it to guest USERD only after a real tail.
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

/// Decode one immutable snapshot; output count <= input words, with no I/O/locks.
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
/// is at most this fragment's word count; a header can claim at most the public
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
    out.try_reserve_exact(words.len())
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
