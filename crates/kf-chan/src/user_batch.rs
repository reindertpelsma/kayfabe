//! Bounded authored user-stream publications, unconnected to a VM birth yet.
//!
//! Every hardware method is a complete public C56F packet. Software methods
//! split the plan and never become native writes. Guest SET_REFERENCE is an
//! actual native method, never a CPU completion. HOST-consumption receipts
//! govern command storage; they do not retire engine operands.

use crate::tmode::SwCall;
use crate::user_storage::CELL_BYTES;
use crate::user_stream::Op;
use kf_abi::submit::{fifo, method_header_inc};

/// Trusted per-worker immutable snapshot policy, not a GPU/die constant.
pub const MAX_OPS: usize = 16 * 1024;
/// One private staging cell, with complete header/data pairs only.
pub const WORDS: usize = (CELL_BYTES / 4) as usize;

/// Worker actions must execute in order; a software action holds later ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Complete authored packets for one command/barrier/witness publication.
    Commands(Vec<u32>),
    /// Requires prior HOST consumption and the channel-owned software registry.
    /// This is an action description, never an executed or completed call.
    Software(SwCall),
    /// CPU worker waits for the VA manager's own-space result; never GPU words.
    OwnInvalidate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Trusted snapshot/action budget exceeded.
    Budget,
    /// Bounded bookkeeping allocation refused.
    Allocation,
    /// Method metadata cannot be expressed as a complete public packet.
    Method,
}

/// Build an owned immutable plan from the parser's channel-validated operations.
/// No output has reached hardware on failure. Neither parsing nor this plan
/// advances a guest cursor or reference. The runtime must bind every publication
/// and receipt to its live native allocation and guest source generation.
/// IMPORTANT: a multi-cell plan must NOT label every cell with the final source
/// GP_GET. Intermediate receipts retire only their command cell; source progress
/// remains at the old cursor until ALL later cells/software actions have run.
pub fn plan(ops: &[Op]) -> Result<Vec<Action>, Refusal> {
    if ops.len() > MAX_OPS {
        return Err(Refusal::Budget);
    }
    let mut actions = Vec::new();
    // Worst case: every operation is software, or one command group per op.
    actions
        .try_reserve(ops.len())
        .map_err(|_| Refusal::Allocation)?;
    let mut words = Vec::new();
    for op in ops {
        let (sub, method, value) = match *op {
            Op::Bind { sub, class } => (sub, 0, class),
            Op::Hardware { sub, method, data } => (sub, method, data),
            Op::GuestReference(value) => (0, fifo::SET_REFERENCE_C56F, value),
            Op::OwnInvalidate => {
                flush(&mut actions, &mut words);
                actions.push(Action::OwnInvalidate);
                continue;
            }
            Op::Software(call) => {
                flush(&mut actions, &mut words);
                actions.push(Action::Software(call));
                continue;
            }
        };
        if words.len() == WORDS {
            flush(&mut actions, &mut words);
        }
        let header = method_header_inc(sub, method, 1).ok_or(Refusal::Method)?;
        // Allocate for actual methods; alternating hardware/software must not
        // retain a whole 1KiB allocation for each two-word command group.
        if words.capacity() - words.len() < 2 {
            let capacity = words.capacity().saturating_mul(2).max(4).min(WORDS);
            words
                .try_reserve_exact(capacity - words.len())
                .map_err(|_| Refusal::Allocation)?;
        }
        words.push(header);
        words.push(value);
    }
    if !words.is_empty() {
        actions.push(Action::Commands(words));
    }
    Ok(actions)
}

fn flush(actions: &mut Vec<Action>, words: &mut Vec<u32>) {
    if !words.is_empty() {
        actions.push(Action::Commands(std::mem::take(words)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kf_abi::submit::method_header_decode;
    #[test]
    fn software_splits_are_ordered_and_never_reach_native_words() {
        let sw = SwCall {
            sub: 6,
            value: 0x5080,
            method: 0x200,
            data: 27,
        };
        let actions = plan(&[
            Op::Bind {
                sub: 0,
                class: 0xc997,
            },
            Op::Hardware {
                sub: 0,
                method: 0x100,
                data: 1,
            },
            Op::Software(sw),
            Op::GuestReference(9),
        ])
        .unwrap();
        assert_eq!(actions.len(), 3);
        assert!(matches!(&actions[0], Action::Commands(w) if w.len() == 4));
        assert_eq!(actions[1], Action::Software(sw));
        let Action::Commands(words) = &actions[2] else {
            panic!()
        };
        assert_eq!(words.len(), 2);
        let header = method_header_decode(words[0]).unwrap();
        assert_eq!(header.method, fifo::SET_REFERENCE_C56F);
        assert_eq!(words[1], 9);
    }
    #[test]
    fn every_cell_contains_complete_packets_at_the_bound_and_across_it() {
        for n in [0, 1, WORDS / 2, WORDS / 2 + 1, MAX_OPS] {
            let input = vec![
                Op::Hardware {
                    sub: 7,
                    method: 0x100,
                    data: 4
                };
                n
            ];
            let actions = plan(&input).unwrap();
            let mut methods = 0;
            for action in actions {
                let Action::Commands(words) = action else {
                    panic!()
                };
                assert!(!words.is_empty() && words.len() <= WORDS && words.len() % 2 == 0);
                for packet in words.chunks_exact(2) {
                    assert!(method_header_decode(packet[0]).is_some());
                    assert_eq!(packet[1], 4);
                    methods += 1;
                }
            }
            assert_eq!(methods, n);
        }
    }
    #[test]
    fn alternating_software_keeps_memory_proportional_to_actual_words() {
        let sw = SwCall {
            sub: 6,
            value: 0x5080,
            method: 0x200,
            data: 27,
        };
        let input: Vec<_> = (0..MAX_OPS / 2)
            .flat_map(|_| [Op::GuestReference(1), Op::Software(sw)])
            .collect();
        let actions = plan(&input).unwrap();
        assert_eq!(actions.len(), input.len());
        for action in actions {
            if let Action::Commands(words) = action {
                assert_eq!(words.len(), 2);
                assert!(words.capacity() <= 4);
            }
        }
    }
    #[test]
    fn a_bad_method_or_over_budget_snapshot_yields_no_publication() {
        assert_eq!(
            plan(&[Op::Hardware {
                sub: 8,
                method: 0x100,
                data: 0
            }]),
            Err(Refusal::Method)
        );
        assert_eq!(
            plan(&vec![Op::GuestReference(0); MAX_OPS + 1]),
            Err(Refusal::Budget)
        );
    }
}
