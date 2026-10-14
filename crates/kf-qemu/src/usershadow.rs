//! Bounded worker-only observations. No native writes, CPU API actions, or receipts.
//! Parser state is a sampled hypothesis, never authority for native execution.
use kf_chan::user_scan::{Refusal, Scan};

pub(super) const BYTE_BUDGET: usize = 4 << 20;
const CLASSES: usize = 12;

#[derive(Clone, Copy, Debug)]
pub(super) enum Class {
    Entry,
    Budget,
    Source,
    Header,
    Legacy,
    Mask,
    Packet,
    Method,
    Trigger,
    UnknownSuffix,
    ObservationBudget,
    InitialMaskHypothesis,
}
impl Class {
    pub fn category(self) -> &'static str {
        match self {
            Self::Entry => "b,c",
            Self::Budget | Self::ObservationBudget => "d",
            Self::Source => "a,c,d",
            Self::Header
            | Self::Legacy
            | Self::Mask
            | Self::Packet
            | Self::Method
            | Self::Trigger
            | Self::UnknownSuffix
            | Self::InitialMaskHypothesis => "b",
        }
    }
}
impl From<Refusal> for Class {
    fn from(v: Refusal) -> Self {
        match v {
            Refusal::Header => Self::Header,
            Refusal::Legacy => Self::Legacy,
            Refusal::SubDeviceMask(_) => Self::Mask,
            Refusal::SoftwarePacket => Self::Packet,
            Refusal::MethodOverflow => Self::Method,
        }
    }
}

/// CPU observation allocation is bounded independently of a guest GP descriptor.
pub(super) fn decode_words(bytes: &[u8], maximum: usize) -> Result<Vec<u32>, &'static str> {
    if bytes.len() > maximum || bytes.len() % 4 != 0 {
        return Err("observation word budget/alignment");
    }
    let mut words = Vec::new();
    words
        .try_reserve_exact(bytes.len() / 4)
        .map_err(|_| "observation allocation")?;
    for b in bytes.chunks_exact(4) {
        words.push(u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    }
    Ok(words)
}

pub(super) struct Shadow {
    scan: Option<Scan>,
    remaining: usize,
    pub counts: [u64; CLASSES],
}
impl Shadow {
    pub fn new(id: u32) -> Self {
        let mut counts = [0; CLASSES];
        if id == 0 {
            counts[Class::InitialMaskHypothesis as usize] = 1;
        }
        Self {
            scan: Scan::for_subdevice(if id == 0 { 1 } else { id }).ok(),
            remaining: BYTE_BUDGET,
            counts,
        }
    }
    /// True only for first sample of a class. Saturation never grants authority.
    pub fn record(&mut self, class: Class) -> bool {
        let c = &mut self.counts[class as usize];
        let first = *c == 0;
        *c = c.saturating_add(1);
        first
    }
    pub fn charge(&mut self, bytes: usize) -> bool {
        if bytes > self.remaining {
            self.lose_history();
            false
        } else {
            self.remaining -= bytes;
            true
        }
    }
    /// A retry keeps the observation quota/counters but never reparses the GP
    /// using metadata advanced by the failed attempt. Coverage is explicitly lost.
    pub fn finish_publication(&mut self, success: bool) {
        if !success {
            self.lose_history();
        }
    }
    pub fn can_observe(&self) -> bool {
        self.scan.is_some()
    }
    pub fn lose_history(&mut self) {
        self.scan = None;
    }
    /// A lost framing boundary is never silently reset at the next GP entry.
    /// The callback receives observations only; no action or modified bytes exist.
    pub fn observe(
        &mut self,
        words: &[u32],
        software: impl Fn(u32) -> bool,
        mut event: impl FnMut(Class, String),
    ) {
        let Some(mut scan) = self.scan.take() else {
            if self.record(Class::UnknownSuffix) {
                event(Class::UnknownSuffix, "history unknown".into());
            }
            return;
        };
        let mut at = 0;
        loop {
            match scan.segment(&words[at..], &software) {
                Ok(Some(t)) => {
                    if self.record(Class::Trigger) {
                        event(
                            Class::Trigger,
                            format!("sampled {:?}; NOT execution; no CPU action", t.call),
                        );
                    }
                    at += t.end;
                }
                Ok(None) => {
                    self.scan = Some(scan);
                    break;
                }
                Err(e) => {
                    let class = Class::from(e);
                    if self.record(class) {
                        event(class, format!("{e:?}; suffix unknown"));
                    }
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn publication_retry_cannot_parse_a_pending_header_twice() {
        let mut s = Shadow::new(1);
        let header = kf_abi::submit::method_header_inc(6, 0, 1).unwrap();
        s.observe(&[header], |_| false, |_, _| {});
        assert!(s.can_observe());
        s.finish_publication(false);
        assert!(!s.can_observe());
        let mut events = Vec::new();
        s.observe(&[header, 0x5080], |_| true, |c, _| events.push(c as usize));
        assert_eq!(events, [Class::UnknownSuffix as usize]);
        assert_eq!(s.counts[Class::Trigger as usize], 0);
    }
    #[test]
    fn real_snapshot_decoder_checks_quota_alignment_and_little_endian() {
        assert_eq!(decode_words(&[1, 2, 3, 4], 4).unwrap(), [0x04030201]);
        assert!(decode_words(&[1, 2, 3, 4], 3).is_err());
        assert!(decode_words(&[1, 2, 3], 4).is_err());
    }
    #[test]
    fn observation_budget_and_framing_loss_never_reset_or_grant_credit() {
        let mut s = Shadow::new(1);
        assert!(s.charge(BYTE_BUDGET));
        assert!(!s.charge(1));
        assert!(s.scan.is_none());
        assert!(s.record(Class::ObservationBudget));
        assert!(!s.record(Class::ObservationBudget));
        let mut events = Vec::new();
        s.observe(&[0], |_| false, |c, _| events.push(c as usize));
        assert_eq!(events, [Class::UnknownSuffix as usize]);
        assert_eq!(s.remaining, 0);
    }
    #[test]
    fn shadow_trigger_observation_preserves_words_and_has_no_action_type() {
        use kf_abi::submit::method_header_inc;
        let words = [
            method_header_inc(6, 0, 1).unwrap(),
            0x5080,
            method_header_inc(6, 0x200, 1).unwrap(),
            27,
        ];
        let original = words;
        let mut s = Shadow::new(1);
        let mut events = Vec::new();
        s.observe(&words, |v| v == 0x5080, |c, _| events.push(c as usize));
        assert_eq!(words, original);
        assert_eq!(events, [Class::Trigger as usize]);
        assert_eq!(s.counts[Class::Trigger as usize], 1);
        s.observe(&[0x00040000], |_| false, |_, _| {});
        assert!(s.scan.is_none());
        s.observe(&words, |v| v == 0x5080, |_, _| {});
        assert_eq!(s.counts[Class::Trigger as usize], 1);
    }
}
