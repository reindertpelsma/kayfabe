//! ★★ **An RM control larger than one queue message** — owner ruling 2
//! (`docs/design/V3_DRIVER_MATRIX.md` §8.2), implemented at the transport, below every policy.
//!
//! # The protocol, from the guest's side
//!
//! `rpcRmApiControl_GSP` sends a control whose message exceeds `pRpc->maxRpcSize` through
//! `_issueRpcAndWaitLarge(…, bBidirectional = NV_TRUE)` (`ogkm-580: src/nvidia/src/kernel/vgpu/
//! rpc.c:2038-2223`, `ogkm-610: :2058-2244` — the same function). For a GSP client `maxRpcSize` is
//! `GSP_MSG_QUEUE_RPC_SIZE_MAX` = the element maximum minus the element header
//! (`ogkm-580: kernel_gsp.c:2587`, `message_queue_priv.h:95-96`):
//!
//! - **sending**: the HEAD is the first `maxRpcSize` bytes of the whole message (the 32-byte RPC
//!   header included) with the real function; every further `maxRpcSize - 32` bytes go out as a
//!   `CONTINUATION_RECORD` carrying `length = chunk + 32` and the next sequence number. No reply
//!   is awaited per fragment.
//! - **receiving**: the guest waits at `(function, firstSequence)` and copies the reply's first
//!   `length` bytes — header included — over the start of its buffer, then, for each fragment it
//!   sent, waits for a `CONTINUATION_RECORD` at the next sequence and copies `length - 32` bytes of
//!   its payload after it. It reads `rpc_result` from the LAST message it received.
//!
//! ⇒ The reply is the response message split at exactly the boundaries of the request: a head
//! reply as long as the head, one continuation reply per continuation, each at its own sequence.
//! [`split_reply`] is that split; [`Assembler`] joins the request before any policy sees it, so
//! every policy answers one WHOLE command whatever its size.
//!
//! `[matrix]` At 580 no served control needs it — the largest, `KGR_GET_GLOBAL_SM_ORDER`, is
//! 34 592 bytes and fits one 64 KiB message. At 610 it is 73 760 bytes.
//!
//! ⊘ Bounded: a head whose declared total exceeds [`MAX_LARGE_PAYLOAD`], or a run of more than
//! [`MAX_FRAGMENTS`] continuations, is refused by name — a guest cannot make the device hold an
//! unbounded message.

use crate::element::OutgoingRpc;
use crate::rpc::{RpcCommand, RpcFunction};

/// The largest joined payload this transport holds. ★ A policy bound: the largest control any
/// measured version carries is 73 760 bytes (610's SM order); 1 MiB caps a hostile guest.
pub const MAX_LARGE_PAYLOAD: usize = 1 << 20;

/// The largest number of continuation records one head may absorb.
pub const MAX_FRAGMENTS: usize = 64;

/// The 32-byte RPC envelope (`rpc_message_header_v`) every fragment repeats.
pub const RPC_HEADER: usize = 32;

/// One fragment of a joined message, as the guest sent it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fragment {
    /// Its `rpc.sequence`.
    pub sequence: u32,
    /// Its wire function id (the real function for the head, `CONTINUATION_RECORD` after).
    pub code: u32,
    /// Its payload length (after the 32-byte envelope).
    pub len: usize,
}

/// Why a large message was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LargeRefusal {
    /// The head declares more than [`MAX_LARGE_PAYLOAD`].
    TooLarge {
        /// The declared payload length.
        declared: usize,
    },
    /// More than [`MAX_FRAGMENTS`] continuations.
    TooManyFragments,
    /// A continuation carried more than the head declared.
    Overrun {
        /// The declared payload length.
        declared: usize,
        /// What arrived.
        got: usize,
    },
    /// A command other than a continuation arrived while a large message was open.
    Interrupted {
        /// The head's sequence.
        head_sequence: u32,
        /// The interrupting function.
        by: u32,
    },
}

impl core::fmt::Display for LargeRefusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooLarge { declared } => write!(f, "a large RPC declares {declared} bytes (bound {MAX_LARGE_PAYLOAD})"),
            Self::TooManyFragments => write!(f, "a large RPC ran past {MAX_FRAGMENTS} continuation records"),
            Self::Overrun { declared, got } => write!(f, "continuations carried {got} bytes, the head declared {declared}"),
            Self::Interrupted { head_sequence, by } => {
                write!(f, "function {by} arrived while the large RPC at sequence {head_sequence} was open")
            }
        }
    }
}

/// What one command does to the assembler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Not part of a large message: answer it as always.
    NotLarge,
    /// A head or a middle fragment: held, no reply yet.
    Held,
    /// The last fragment: the whole command, and every fragment in order (head first).
    Complete {
        /// The joined command (head's function and sequence, the whole payload).
        whole: RpcCommand,
        /// The fragments, head first — what [`split_reply`] splits the answer by.
        fragments: Vec<Fragment>,
    },
    /// Refused by name; the open message (if any) is dropped.
    Refused(LargeRefusal),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Open {
    whole: RpcCommand,
    declared: usize,
    fragments: Vec<Fragment>,
}

/// Joins a large RPC's fragments. ⊘ Transactional: [`Assembler::step`] computes the next state
/// without committing it; the caller [`Assembler::commit`]s only once the command is consumed
/// (answered or held), so a retried pass re-reading the same element rebuilds the same step.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Assembler {
    open: Option<Open>,
    staged: Option<Option<Open>>,
}

impl Assembler {
    /// Is a large message open?
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Classify `cmd`. `declared` is the whole message's payload length when `cmd` is a
    /// `GSP_RM_CONTROL` head (`params_off + paramsSize` from its control header), else `None`.
    pub fn step(&mut self, cmd: &RpcCommand, declared: Option<usize>) -> Step {
        match (&self.open, cmd.function) {
            (Some(open), RpcFunction::ContinuationRecord) => {
                if open.fragments.len() > MAX_FRAGMENTS {
                    self.staged = Some(None);
                    return Step::Refused(LargeRefusal::TooManyFragments);
                }
                let mut next = open.clone();
                next.whole.payload.extend_from_slice(&cmd.payload);
                next.fragments.push(Fragment { sequence: cmd.sequence, code: cmd.code, len: cmd.payload.len() });
                let got = next.whole.payload.len();
                if got > next.declared {
                    self.staged = Some(None);
                    return Step::Refused(LargeRefusal::Overrun { declared: next.declared, got });
                }
                if got == next.declared {
                    self.staged = Some(None);
                    return Step::Complete { whole: next.whole, fragments: next.fragments };
                }
                self.staged = Some(Some(next));
                Step::Held
            }
            (Some(open), _) => {
                let head_sequence = open.whole.sequence;
                self.staged = Some(None);
                Step::Refused(LargeRefusal::Interrupted { head_sequence, by: cmd.code })
            }
            (None, RpcFunction::RmControl) => match declared {
                Some(d) if d > cmd.payload.len() => {
                    if d > MAX_LARGE_PAYLOAD {
                        self.staged = Some(None);
                        return Step::Refused(LargeRefusal::TooLarge { declared: d });
                    }
                    let open = Open {
                        whole: RpcCommand { delivered: Vec::new(), ..cmd.clone() },
                        declared: d,
                        fragments: vec![Fragment { sequence: cmd.sequence, code: cmd.code, len: cmd.payload.len() }],
                    };
                    self.staged = Some(Some(open));
                    Step::Held
                }
                _ => {
                    self.staged = None;
                    Step::NotLarge
                }
            },
            (None, _) => {
                self.staged = None;
                Step::NotLarge
            }
        }
    }

    /// Commit the last [`Self::step`] (the command it classified was consumed).
    pub fn commit(&mut self) {
        if let Some(next) = self.staged.take() {
            self.open = next;
        }
    }

    /// Drop an open message (a device reset).
    pub fn reset(&mut self) {
        self.open = None;
        self.staged = None;
    }
}

/// ★★ Split the answer to a joined command at the request's own boundaries: the head reply is as
/// long as the head was (the real function, the head's sequence), then one `CONTINUATION_RECORD`
/// reply per continuation at ITS sequence, each carrying the next slice of the answer's payload.
/// Every reply carries the answer's `rpc_result` (the guest reads it from the last one).
#[must_use]
pub fn split_reply(full: &OutgoingRpc, fragments: &[Fragment], continuation_code: u32) -> Vec<OutgoingRpc> {
    let mut out = Vec::with_capacity(fragments.len());
    let mut at = 0usize;
    for (i, f) in fragments.iter().enumerate() {
        let end = (at + f.len).min(full.payload.len());
        let mut payload = full.payload.get(at..end).map(<[u8]>::to_vec).unwrap_or_default();
        payload.resize(f.len, 0);
        out.push(OutgoingRpc {
            function: if i == 0 { full.function } else { continuation_code },
            sequence: f.sequence,
            rpc_result: full.rpc_result,
            rpc_result_private: full.rpc_result_private,
            payload,
        });
        at += f.len;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONT: u32 = 71;

    fn cmd(function: RpcFunction, code: u32, sequence: u32, payload: Vec<u8>) -> RpcCommand {
        RpcCommand { function, code, sequence, payload, elements: 1, delivered: Vec::new() }
    }

    /// Fragment a message the way `_issueRpcLarge` does: the head is `max_rpc - 32` payload
    /// bytes, every continuation `max_rpc - 32` more, the last one the remainder.
    fn fragment(payload: &[u8], max_rpc: usize, first_seq: u32) -> Vec<RpcCommand> {
        let chunk = max_rpc - RPC_HEADER;
        let mut v = vec![cmd(RpcFunction::RmControl, 76, first_seq, payload[..chunk.min(payload.len())].to_vec())];
        let mut at = chunk;
        let mut seq = first_seq + 1;
        while at < payload.len() {
            let end = (at + chunk).min(payload.len());
            v.push(cmd(RpcFunction::ContinuationRecord, CONT, seq, payload[at..end].to_vec()));
            at = end;
            seq += 1;
        }
        v
    }

    /// The guest's receive loop (`_issueRpcLarge`, receive half): the head's payload, then each
    /// continuation's, until `buf` is full; `rpc_result` from the last message read.
    fn guest_receives(replies: &[OutgoingRpc], total: usize, first_seq: u32) -> (Vec<u8>, u32) {
        let mut buf = Vec::new();
        let mut result = 0;
        for (i, r) in replies.iter().enumerate() {
            assert_eq!(r.sequence, first_seq + i as u32, "reply {i} at the wrong sequence");
            assert_eq!(r.function, if i == 0 { 76 } else { CONT }, "reply {i} under the wrong function");
            buf.extend_from_slice(&r.payload);
            result = r.rpc_result;
            if buf.len() >= total {
                break;
            }
        }
        buf.truncate(total);
        (buf, result)
    }

    /// ★★ The round trip the ruling asks for: a 610-sized SM-order control (73 760 bytes of
    /// params behind a 40-byte control header), fragmented at a 64 KiB element maximum, joined,
    /// answered, split — and the guest's own receive loop reads back exactly the answer.
    #[test]
    fn split_and_reassembly_round_trip_a_610_sm_order_control() {
        let max_rpc = 65536 - 48;
        let request: Vec<u8> = (0..40 + 73_760).map(|i| (i % 251) as u8).collect();
        let frags = fragment(&request, max_rpc, 100);
        assert_eq!(frags.len(), 2, "73 800 bytes is a head and one continuation");
        let mut a = Assembler::default();
        let mut complete = None;
        for (i, f) in frags.iter().enumerate() {
            let declared = (i == 0).then_some(request.len());
            match a.step(f, declared) {
                Step::Held => assert!(i + 1 < frags.len()),
                Step::Complete { whole, fragments } => complete = Some((whole, fragments)),
                other => panic!("fragment {i}: {other:?}"),
            }
            a.commit();
        }
        let (whole, fragments) = complete.expect("joined");
        assert!(!a.is_open());
        assert_eq!(whole.payload, request, "the policy sees the whole message");
        assert_eq!((whole.function, whole.sequence), (RpcFunction::RmControl, 100));
        // The answer: the request's bytes transformed, as a served control would rewrite them.
        let answer: Vec<u8> = request.iter().map(|b| b.wrapping_mul(3).wrapping_add(1)).collect();
        let full = OutgoingRpc { function: 76, sequence: 100, rpc_result: 0, rpc_result_private: 0, payload: answer.clone() };
        let replies = split_reply(&full, &fragments, CONT);
        assert_eq!(replies.len(), frags.len(), "one reply per fragment");
        for (r, f) in replies.iter().zip(&frags) {
            assert_eq!(r.payload.len(), f.payload.len(), "each reply as long as its fragment");
            assert!(r.payload.len() + RPC_HEADER <= max_rpc, "each reply fits maxRpcSize");
        }
        let (got, result) = guest_receives(&replies, answer.len(), 100);
        assert_eq!(got, answer);
        assert_eq!(result, 0);
    }

    /// The refusal status reaches the guest from the LAST reply it reads.
    #[test]
    fn a_refused_answer_is_refused_in_every_fragment() {
        let full = OutgoingRpc { function: 76, sequence: 5, rpc_result: 0x56, rpc_result_private: 0x56, payload: vec![0; 10] };
        let frags = [Fragment { sequence: 5, code: 76, len: 6 }, Fragment { sequence: 6, code: CONT, len: 4 }];
        let r = split_reply(&full, &frags, CONT);
        assert!(r.iter().all(|m| m.rpc_result == 0x56));
    }

    /// ⊘ Transactional: a step that is not committed (a failed post) leaves the assembler as it
    /// was, and re-reading the same element rebuilds the same step.
    #[test]
    fn an_uncommitted_step_is_replayable() {
        let request = vec![7u8; 100];
        let frags = fragment(&request, 64 + RPC_HEADER, 1);
        let mut a = Assembler::default();
        for (i, f) in frags.iter().enumerate().take(frags.len() - 1) {
            let _ = a.step(f, (i == 0).then_some(100));
            a.commit();
        }
        let last = frags.last().expect("last");
        let s1 = a.step(last, None);
        // no commit — a failed post
        let s2 = a.step(last, None);
        assert_eq!(s1, s2);
        assert!(matches!(s2, Step::Complete { .. }));
    }

    /// Bounds: a hostile head, an overrun, an interruption — each refused by name.
    #[test]
    fn hostile_large_messages_are_refused_by_name() {
        let mut a = Assembler::default();
        let head = cmd(RpcFunction::RmControl, 76, 1, vec![0; 16]);
        assert_eq!(a.step(&head, Some(MAX_LARGE_PAYLOAD + 1)), Step::Refused(LargeRefusal::TooLarge { declared: MAX_LARGE_PAYLOAD + 1 }));
        a.commit();
        assert!(!a.is_open());
        assert_eq!(a.step(&head, Some(20)), Step::Held);
        a.commit();
        let over = cmd(RpcFunction::ContinuationRecord, CONT, 2, vec![0; 8]);
        assert_eq!(a.step(&over, None), Step::Refused(LargeRefusal::Overrun { declared: 20, got: 24 }));
        a.commit();
        assert!(!a.is_open());
        assert_eq!(a.step(&head, Some(20)), Step::Held);
        a.commit();
        let other = cmd(RpcFunction::RmAlloc, 103, 2, vec![0; 8]);
        assert!(matches!(a.step(&other, None), Step::Refused(LargeRefusal::Interrupted { head_sequence: 1, by: 103 })));
    }

    /// A control that fits is untouched, and so is a continuation outside any large message
    /// (SET_REGISTRY's async large path keeps its own handling).
    #[test]
    fn small_controls_and_stray_continuations_are_not_large() {
        let mut a = Assembler::default();
        assert_eq!(a.step(&cmd(RpcFunction::RmControl, 76, 1, vec![0; 16]), Some(16)), Step::NotLarge);
        assert_eq!(a.step(&cmd(RpcFunction::RmControl, 76, 1, vec![0; 16]), None), Step::NotLarge);
        assert_eq!(a.step(&cmd(RpcFunction::ContinuationRecord, CONT, 2, vec![0; 16]), None), Step::NotLarge);
    }
}
