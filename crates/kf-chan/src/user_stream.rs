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
//! Unsupported packet forms are refused, not silently copied. The model does
//! not yet handle packets split across GP entries or subroutine/control entries.

use crate::tmode::SwCall;
use crate::ttables::{HostMethod, host_method};
use kf_abi::submit::{MethodForm, fifo, method_header_decode, method_header_inc};

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
    mut lookup: impl FnMut(u32) -> Option<Binding>,
) -> Result<Vec<Op>, Refusal> {
    let words = &commands.0;
    let mut next = state.clone();
    let mut out = Vec::new();
    out.try_reserve_exact(words.len())
        .map_err(|_| Refusal::Allocation)?;
    let mut at = 0usize;
    while at < words.len() {
        let header_at = at;
        let raw = words[at];
        // Canonical legacy DMA_NOP only. No unmeasured legacy/control flow.
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
        {
            return Err(Refusal::Header { at });
        }
        at += 1;
        let count = if h.form == MethodForm::Immediate {
            1
        } else {
            h.arg_words
        };
        if count == 0 {
            return Err(Refusal::Header { at: header_at });
        }
        if h.form != MethodForm::Immediate && count > words.len() - at {
            return Err(Refusal::Truncated { at });
        }
        for k in 0..count {
            let m = match h.form {
                MethodForm::Immediate | MethodForm::NonIncrementing => h.method,
                MethodForm::IncrementOnce if k > 0 => h.method + 4,
                _ => h.method + 4 * k as u32,
            };
            // Bound every expanded method; never wrap an overflowing increment
            // into SET_OBJECT/SET_REFERENCE or a software method.
            if method_header_inc(h.subchannel, m, 1).is_none() {
                return Err(Refusal::Method { method: m });
            }
            let data = if h.form == MethodForm::Immediate {
                h.immd
            } else {
                words[at + k]
            };
            if m == 0 {
                let binding = lookup(data).ok_or(Refusal::Object { value: data })?;
                next.bindings[h.subchannel as usize] = Some(binding);
                if let Binding::Hardware { class } = binding {
                    out.push(Op::Bind {
                        sub: h.subchannel,
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
                        sub: h.subchannel,
                        method: m,
                        data,
                    }),
                }
            } else {
                match next.bindings[h.subchannel as usize] {
                    Some(Binding::Hardware { .. }) => out.push(Op::Hardware {
                        sub: h.subchannel,
                        method: m,
                        data,
                    }),
                    Some(Binding::Software { value }) => out.push(Op::Software(SwCall {
                        sub: h.subchannel,
                        value,
                        method: m,
                        data,
                    })),
                    None => return Err(Refusal::Unbound { sub: h.subchannel }),
                }
            }
        }
        if h.form != MethodForm::Immediate {
            at += count;
        }
    }
    *state = next;
    Ok(out)
}
