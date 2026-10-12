//! Read-only USER stream scan. Hardware USER permissions, not this scan,
//! enforce isolation. A scan is not proof of GPU execution or completion.
//! Host software actions require independent channel-owned registry validation.

use crate::tmode::SwCall;
use kf_abi::submit::{MethodForm, MethodHeader, method_header_decode};

/// Packet/binding metadata tracked from the first USER submission.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scan {
    bindings: [u32; 8],
    pending: Option<(MethodHeader, usize)>,
}

/// One complete software-method packet at its exact position in this segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trigger {
    /// Entire one-datum software packet, including its header.
    pub start: usize,
    /// First word after the trigger packet.
    pub end: usize,
    /// Still needs independent channel-owned object/registration validation.
    pub call: SwCall,
}

/// Named parser refusal; failed calls leave all metadata unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Public ABI cannot size the header.
    Header,
    /// Shared ABI sizes legacy packets but does not retain their inc/non-inc form.
    Legacy,
    /// Sub-device filtering changes which methods hardware executes.
    SubDeviceMask,
    /// Software packet is split across segments or has unsupported arity.
    SoftwarePacket,
    /// Incremented method exceeds the public method address range.
    MethodOverflow,
}

impl Scan {
    /// Scan at most the supplied bounded segment. State commits transactionally;
    /// return at the first trigger, with state AFTER that packet only. Ordinary
    /// methods remain unchanged and gain no CPU-authored privilege.
    pub fn segment(
        &mut self,
        words: &[u32],
        software: impl Fn(u32) -> bool,
    ) -> Result<Option<Trigger>, Refusal> {
        let mut next = self.clone();
        let mut i = 0;
        let mut packet_start = None;
        while i < words.len() {
            if next.pending.is_none() {
                let raw = words[i];
                let h = method_header_decode(raw).ok_or(Refusal::Header)?;
                if raw & (1 << 12) != 0
                    && matches!(
                        h.form,
                        MethodForm::Incrementing
                            | MethodForm::NonIncrementing
                            | MethodForm::IncrementOnce
                            | MethodForm::Immediate
                    )
                {
                    return Err(Refusal::Header);
                }
                packet_start = Some(i);
                i += 1;
                if h.form == MethodForm::EndPbSegment {
                    break;
                }
                // The shared decoder intentionally collapses old INC/NON_INC
                // into sized-only Legacy. Never invent method/binding semantics.
                // Canonical zero DMA_NOP still has no arguments and is accepted.
                if h.form == MethodForm::Legacy && h.arg_words != 0 {
                    return Err(Refusal::Legacy);
                }
                if h.form == MethodForm::SubDeviceMask {
                    return Err(Refusal::SubDeviceMask);
                }
                if h.form == MethodForm::Immediate {
                    if let Some(call) = next.datum(h, 0, h.immd, &software)? {
                        let t = Trigger {
                            start: i - 1,
                            end: i,
                            call,
                        };
                        *self = next;
                        return Ok(Some(t));
                    }
                    continue;
                }
                if h.arg_words == 0 {
                    continue;
                }
                next.pending = Some((h, 0));
                if i == words.len() {
                    break;
                }
            }
            let (h, n) = next.pending.ok_or(Refusal::Header)?;
            let call = next.datum(h, n, words[i], &software)?;
            i += 1;
            next.pending = (n + 1 < h.arg_words).then_some((h, n + 1));
            if let Some(call) = call {
                // Public FIFO software methods cannot straddle GP segments.
                if h.arg_words != 1 || n != 0 {
                    return Err(Refusal::SoftwarePacket);
                }
                let start = packet_start.ok_or(Refusal::SoftwarePacket)?;
                let t = Trigger {
                    start,
                    end: i,
                    call,
                };
                *self = next;
                return Ok(Some(t));
            }
        }
        *self = next;
        Ok(None)
    }

    fn datum(
        &mut self,
        h: MethodHeader,
        n: usize,
        data: u32,
        software: &impl Fn(u32) -> bool,
    ) -> Result<Option<SwCall>, Refusal> {
        let add = match h.form {
            MethodForm::NonIncrementing => 0,
            MethodForm::IncrementOnce => usize::from(n != 0),
            _ => n,
        };
        let method = u32::try_from(add)
            .ok()
            .and_then(|n| n.checked_mul(4))
            .and_then(|n| h.method.checked_add(n))
            .filter(|m| *m <= 0x3ffc)
            .ok_or(Refusal::MethodOverflow)?;
        let sub = h.subchannel as usize;
        if method == 0 {
            self.bindings[sub] = data;
        }
        // Public dev_ram.ref/dev_pbdma.ref: fixed C56F runlist mapping sends
        // subchannels0..4 to hardware engines and5..7 to SW DEVICE dispatch.
        if sub >= 5 && (0x200..=0x203).contains(&method) && software(self.bindings[sub]) {
            return Ok(Some(SwCall {
                sub: h.subchannel,
                value: self.bindings[sub],
                method,
                data,
            }));
        }
        Ok(None)
    }
}

/// Paired private-GP fetch receipts. These retire descriptors/source GP slots,
/// never pushbuffer bytes, engine operands, or GPU work.
#[derive(Debug)]
pub struct Fetch {
    entries: u32,
    issued: u64,
    fetched: u64,
    pending: std::collections::VecDeque<Option<(u64, u32)>>,
}
impl Fetch {
    /// New private ring, with zero hardware cursors and one empty slot.
    pub fn new(entries: u32) -> Result<Self, &'static str> {
        if entries < 4 || !entries.is_power_of_two() || entries > 512 {
            return Err("USER_FETCH_CAPACITY");
        }
        let mut pending = std::collections::VecDeque::new();
        pending
            .try_reserve_exact((entries - 1) as usize)
            .map_err(|_| "USER_FETCH_ALLOC")?;
        Ok(Self {
            entries,
            issued: 0,
            fetched: 0,
            pending,
        })
    }
    /// Available descriptor slots; these grant no pushbuffer-byte reuse.
    pub fn available(&self) -> u32 {
        self.entries - 1 - self.pending.len() as u32
    }
    /// Absolute descriptor publication sequence, refusing exhaustion.
    pub fn issued(&self) -> u64 {
        self.issued
    }
    /// Absolute descriptor fetch sequence, never GPU engine completion.
    pub fn fetched(&self) -> u64 {
        self.fetched
    }
    /// Reserve before I/O. No rollback after possibly visible publication.
    pub fn publish(&mut self, source: Option<(u64, u32)>) -> Result<(u32, u32), &'static str> {
        if self.available() == 0 {
            return Err("USER_FETCH_FULL");
        }
        let next = self.issued.checked_add(1).ok_or("USER_FETCH_SEQUENCE")?;
        let slot = (self.issued & u64::from(self.entries - 1)) as u32;
        self.pending.push_back(source);
        self.issued = next;
        Ok((slot, (next & u64::from(self.entries - 1)) as u32))
    }
    /// Fresh ordered hardware GET from this exact private native USERD only.
    pub fn observe(&mut self, get: u32) -> Result<Vec<(u64, u32)>, &'static str> {
        if get >= self.entries {
            return Err("USER_FETCH_GET");
        }
        let old = (self.fetched & u64::from(self.entries - 1)) as u32;
        let delta = get.wrapping_sub(old) & (self.entries - 1);
        if delta as usize > self.pending.len() {
            return Err("USER_FETCH_UNISSUED_GET");
        }
        let mut receipts = Vec::new();
        receipts
            .try_reserve_exact(delta as usize)
            .map_err(|_| "USER_FETCH_ALLOC")?;
        for _ in 0..delta {
            if let Some(source) = self.pending.pop_front().flatten() {
                receipts.push(source);
            }
        }
        self.fetched += u64::from(delta);
        Ok(receipts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kf_abi::submit::method_header_inc as h;
    #[test]
    fn header_only_segment_retains_hardware_continuation_without_reading_past_end() {
        let mut s = Scan::default();
        assert_eq!(s.segment(&[h(2, 0x300, 2).unwrap()], |_| false), Ok(None));
        assert_eq!(s.segment(&[], |_| false), Ok(None));
        assert_eq!(s.segment(&[17], |_| false), Ok(None));
        assert_eq!(s.segment(&[19], |_| false), Ok(None));
        assert_eq!(s.pending, None);
    }
    #[test]
    fn software_packet_split_at_header_is_refused_without_committing_state() {
        let mut s = Scan::default();
        s.segment(&[h(6, 0, 1).unwrap(), 1], |v| v == 1).unwrap();
        s.segment(&[h(6, 0x200, 1).unwrap()], |v| v == 1).unwrap();
        let before = s.clone();
        assert_eq!(s.segment(&[27], |v| v == 1), Err(Refusal::SoftwarePacket));
        assert_eq!(s, before);
    }
    #[test]
    fn hardware_packet_continuation_is_not_mistaken_for_a_trigger() {
        let mut s = Scan::default();
        s.segment(&[h(6, 0, 1).unwrap(), 1], |v| v == 1).unwrap();
        s.segment(&[h(2, 0x300, 2).unwrap(), 99], |v| v == 1)
            .unwrap();
        assert_eq!(s.segment(&[h(6, 0x200, 1).unwrap()], |v| v == 1), Ok(None));
        let t = s
            .segment(&[h(6, 0x200, 1).unwrap(), 27], |v| v == 1)
            .unwrap()
            .unwrap();
        assert_eq!((t.start, t.end, t.call.data), (0, 2, 27));
    }
    #[test]
    fn triggers_require_this_channels_software_binding() {
        let mut s = Scan::default();
        assert_eq!(
            s.segment(&[h(6, 0x200, 1).unwrap(), 27], |_| false),
            Ok(None)
        );
        s.segment(&[h(6, 0, 1).unwrap(), 1], |v| v == 1).unwrap();
        let t = s
            .segment(&[0, h(6, 0x200, 1).unwrap(), 27, 0], |v| v == 1)
            .unwrap()
            .unwrap();
        assert_eq!((t.start, t.end), (1, 3));
        assert_eq!(t.call.value, 1);
    }
    #[test]
    fn software_object_value_on_hardware_subchannel_does_not_create_cpu_action() {
        for sub in 0..8 {
            let mut s = Scan::default();
            s.segment(&[h(sub, 0, 1).unwrap(), 1], |v| v == 1).unwrap();
            let trigger = s
                .segment(&[h(sub, 0x200, 1).unwrap(), 27], |v| v == 1)
                .unwrap();
            assert_eq!(trigger.is_some(), sub >= 5, "subchannel={sub}");
        }
    }
    #[test]
    fn failed_scan_does_not_change_binding_state() {
        let mut s = Scan::default();
        let before = s.clone();
        assert_eq!(
            s.segment(&[h(6, 0, 1).unwrap(), 1, 6 << 29], |v| v == 1),
            Err(Refusal::Header)
        );
        assert_eq!(s, before);
    }
    #[test]
    fn sized_only_old_nonincrementing_packets_never_invent_bindings_or_triggers() {
        let mut s = Scan::default();
        s.segment(&[h(6, 0, 1).unwrap(), 1], |v| v == 1).unwrap();
        let before = s.clone();
        // Public C56F old GRP2_NON_INC: method0x1fc remains0x1fc for BOTH
        // arguments. A guessed increment would turn its second datum into0x200.
        let old = (2 << 29) | (2 << 18) | (6 << 13) | 0x1fc;
        assert_eq!(s.segment(&[old, 17, 27], |v| v == 1), Err(Refusal::Legacy));
        assert_eq!(s, before);
        // Old NON_INC SET_OBJECT would bind twice, ending at9, whereas guessing
        // INC would incorrectly leave1. Until ABI preserves the form, refuse it.
        let bind = (2 << 29) | (2 << 18) | (6 << 13);
        assert_eq!(s.segment(&[bind, 1, 9], |v| v == 1), Err(Refusal::Legacy));
        assert_eq!(s, before);
        assert_eq!(s.segment(&[0, 0], |v| v == 1), Ok(None));
    }
    #[test]
    fn unsupported_subdevice_filters_never_produce_cpu_software_actions() {
        for tert_op in 1..=3 {
            let mut s = Scan::default();
            let before = s.clone();
            let words = [
                h(6, 0, 1).unwrap(),
                1,
                tert_op << 16,
                h(6, 0x200, 1).unwrap(),
                27,
            ];
            assert_eq!(s.segment(&words, |v| v == 1), Err(Refusal::SubDeviceMask));
            assert_eq!(s, before, "masked/unexecuted binding must not be committed");
        }
    }
    #[test]
    fn unsupported_modern_bit12_never_becomes_a_software_trigger() {
        let immediate = (4 << 29) | (27 << 16) | (6 << 13) | (0x200 / 4);
        for packet in [
            vec![h(6, 0x200, 1).unwrap() | (1 << 12), 27],
            vec![immediate | (1 << 12)],
        ] {
            let mut s = Scan::default();
            s.segment(&[h(6, 0, 1).unwrap(), 1], |v| v == 1).unwrap();
            let before = s.clone();
            assert_eq!(s.segment(&packet, |v| v == 1), Err(Refusal::Header));
            assert_eq!(s, before);
        }
    }
    #[test]
    fn hardware_packets_parse_identically_at_every_fragment_boundary() {
        // Headers used as ARGUMENTS must not become software calls. Exercise
        // every possible GP split, including directly after each header.
        let words = [
            h(6, 0, 1).unwrap(),
            1,
            h(2, 0x300, 3).unwrap(),
            h(6, 0x200, 1).unwrap(),
            27,
            0,
            h(6, 0, 1).unwrap(),
            9,
        ];
        let mut whole = Scan::default();
        assert_eq!(whole.segment(&words, |v| v == 1), Ok(None));
        for split in 0..=words.len() {
            let mut fragmented = Scan::default();
            assert_eq!(fragmented.segment(&words[..split], |v| v == 1), Ok(None));
            assert_eq!(fragmented.segment(&words[split..], |v| v == 1), Ok(None));
            assert_eq!(fragmented, whole, "split={split}");
        }
    }
    #[test]
    fn modern_nonincrementing_and_one_increment_binding_state_survives_fragmentation() {
        for (opcode, expected_binding) in [(3, 9), (5, 1)] {
            let header = (opcode << 29) | (3 << 16) | (6 << 13);
            let words = [header, 1, 7, 9];
            for split in 0..=words.len() {
                let mut s = Scan::default();
                assert_eq!(s.segment(&words[..split], |_| false), Ok(None));
                assert_eq!(s.segment(&words[split..], |_| false), Ok(None));
                assert_eq!(
                    s.bindings[6], expected_binding,
                    "opcode={opcode} split={split}"
                );
                assert_eq!(s.pending, None);
            }
        }
    }
    #[test]
    fn end_pb_segment_does_not_interpret_ignored_suffix() {
        let mut s = Scan::default();
        s.segment(&[h(6, 0, 1).unwrap(), 1], |v| v == 1).unwrap();
        assert_eq!(
            s.segment(&[7 << 29, h(6, 0x200, 1).unwrap(), 27], |v| v == 1),
            Ok(None)
        );
        assert_eq!(s.pending, None);
        assert_eq!(s.bindings[6], 1);
    }
    #[test]
    fn stop_at_first_trigger_and_resume_suffix_without_scanning_ahead() {
        let mut s = Scan::default();
        s.segment(&[h(6, 0, 1).unwrap(), 1], |v| v == 1).unwrap();
        let words = [
            0,
            h(6, 0x200, 1).unwrap(),
            27,
            h(6, 0, 1).unwrap(),
            9,
            h(6, 0x200, 1).unwrap(),
            28,
        ];
        let trigger = s.segment(&words, |v| v == 1).unwrap().unwrap();
        assert_eq!((trigger.start, trigger.end, trigger.call.data), (1, 3, 27));
        assert_eq!(
            s.bindings[6], 1,
            "suffix binding must not affect the current trigger"
        );
        assert_eq!(s.segment(&words[trigger.end..], |v| v == 1), Ok(None));
        assert_eq!(s.bindings[6], 9);
    }
    #[test]
    fn immediate_trigger_has_exact_single_word_span_and_multi_argument_trigger_refuses() {
        let mut s = Scan::default();
        s.segment(&[h(6, 0, 1).unwrap(), 1], |v| v == 1).unwrap();
        // Public C56F IMMD_DATA_METHOD opcode4; data=27, subchannel6.
        let immediate = (4 << 29) | (27 << 16) | (6 << 13) | (0x200 / 4);
        let trigger = s.segment(&[0, immediate, 0], |v| v == 1).unwrap().unwrap();
        assert_eq!((trigger.start, trigger.end, trigger.call.data), (1, 2, 27));
        let before = s.clone();
        assert_eq!(
            s.segment(&[h(6, 0x200, 2).unwrap(), 27, 28], |v| v == 1),
            Err(Refusal::SoftwarePacket)
        );
        assert_eq!(s, before);
    }
    #[test]
    fn method_overflow_and_unknown_header_refuse_transactionally_without_panic() {
        let mut s = Scan::default();
        let before = s.clone();
        assert_eq!(
            s.segment(&[h(2, 0x3ffc, 2).unwrap(), 1, 2], |_| false),
            Err(Refusal::MethodOverflow)
        );
        assert_eq!(s, before);
        // Bounded deterministic hostile inputs include header-only fragments.
        let mut random = 0x68c0_1234u32;
        for count in 0..128 {
            let words: Vec<_> = (0..count % 17)
                .map(|_| {
                    random ^= random << 13;
                    random ^= random >> 17;
                    random ^= random << 5;
                    random
                })
                .collect();
            let mut scan = Scan::default();
            let before = scan.clone();
            if scan.segment(&words, |_| true).is_err() {
                assert_eq!(scan, before);
            }
        }
    }
    #[test]
    fn receipts_pair_wrap_and_source_generations_without_completion_claims() {
        let mut f = Fetch::new(4).unwrap();
        for g in 1..10 {
            let (_, p) = f.publish(Some((g, 7))).unwrap();
            f.publish(None).unwrap();
            let (_, end) = f.publish(None).unwrap();
            assert_eq!(f.available(), 0);
            assert_eq!(f.publish(None), Err("USER_FETCH_FULL"));
            assert_eq!(f.observe(p).unwrap(), vec![(g, 7)]);
            assert_eq!(f.observe(end).unwrap(), vec![]);
            assert_eq!(f.available(), 3);
        }
        assert_eq!(f.observe(4), Err("USER_FETCH_GET"));
        assert_eq!(
            f.observe((f.fetched() as u32 + 1) & 3),
            Err("USER_FETCH_UNISSUED_GET")
        );
    }
    #[test]
    fn invalid_receipt_is_transactional_and_does_not_release_ring_capacity() {
        let mut f = Fetch::new(8).unwrap();
        let (_, put) = f.publish(Some((5, 7))).unwrap();
        assert_eq!(f.observe(2), Err("USER_FETCH_UNISSUED_GET"));
        assert_eq!((f.issued(), f.fetched(), f.available()), (1, 0, 6));
        assert_eq!(f.observe(put).unwrap(), vec![(5, 7)]);
        assert_eq!(
            f.observe(put).unwrap(),
            vec![],
            "GET replay is not a second receipt"
        );
    }
    #[test]
    fn exhausted_absolute_fetch_sequence_never_wraps_or_resets_live_state() {
        let mut f = Fetch::new(8).unwrap();
        f.issued = u64::MAX;
        f.fetched = u64::MAX;
        assert_eq!(f.publish(Some((5, 7))), Err("USER_FETCH_SEQUENCE"));
        assert_eq!(f.available(), 7);
        assert_eq!((f.issued(), f.fetched()), (u64::MAX, u64::MAX));
    }
    #[test]
    fn final_possible_receipt_advances_to_max_without_overflow_or_replay() {
        let mut f = Fetch::new(8).unwrap();
        f.issued = u64::MAX - 2;
        f.fetched = u64::MAX - 2;
        f.publish(Some((91, 7))).unwrap();
        let (_, put) = f.publish(Some((92, 0))).unwrap();
        assert_eq!(f.observe(put).unwrap(), vec![(91, 7), (92, 0)]);
        assert_eq!(f.fetched(), u64::MAX);
        assert_eq!(f.observe(put).unwrap(), vec![]);
        assert_eq!(f.observe(0), Err("USER_FETCH_UNISSUED_GET"));
        assert_eq!(f.publish(None), Err("USER_FETCH_SEQUENCE"));
    }
}
