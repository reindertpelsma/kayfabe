//! Passive private-ring observations. This module never decides completion or wakes work.
/// The existing relay's recorded wait, separate from pointer/reference observations.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Wait {
    /// No recorded wait.
    #[default]
    None,
    /// Marker processing, absolute target.
    Marker(u64),
    /// Descriptor consumption, absolute target.
    Popped(u64),
    /// Existing lock/admission contention.
    Contended,
}
impl Wait {
    fn words(self) -> (u64, u64) {
        match self {
            Self::None => (0, 0),
            Self::Marker(n) => (1, n),
            Self::Popped(n) => (2, n),
            Self::Contended => (3, 0),
        }
    }
}
/// The existing cause of a re-ring/request; never inferred from GPU progress.
#[derive(Clone, Copy, Debug)]
#[repr(u16)]
pub enum Wake {
    /// Host event.
    Event = 1,
    /// Bounded existing spin.
    Spin = 2,
    /// Existing safety-net look.
    Backstop = 3,
    /// Durable request, whose underlying cause is not known here.
    Demand = 4,
    /// Existing turn requested another turn.
    Again = 5,
}
/// Already-observed bounded integers. None means no fresh read, never zero.
#[derive(Clone, Copy, Debug)]
pub struct Observation {
    /// Guest token slot.
    pub token: u32,
    /// Private hardware fetch cursor.
    pub private_get: Option<u32>,
    /// Last successfully stored private publication cursor.
    pub private_put: Option<u32>,
    /// Raw private processing reference.
    pub reference: Option<u32>,
    /// Existing parked state.
    pub wait: Wait,
    /// Cumulative existing token doorbell counter.
    pub doorbells: Option<u64>,
    /// A newly observed existing wake/request.
    pub wake: Option<Wake>,
    /// Last real guest write-back.
    pub guest_get: Option<u32>,
    /// Last real guest PUT read.
    pub guest_put: Option<u32>,
    /// Guest descriptor count.
    pub guest_entries: u32,
    /// Absolute native descriptor publications reserved by existing code.
    pub issued: u64,
    /// Absolute native descriptors fetched, not processed.
    pub fetched: u64,
    /// Absolute processing markers issued.
    pub markers: u64,
    /// Live until retirement is actually observed.
    pub active: bool,
    /// Private descriptor capacity.
    pub private_entries: u32,
}
impl Default for Observation {
    fn default() -> Self {
        Self {
            token: 0,
            private_get: None,
            private_put: None,
            reference: None,
            wait: Wait::None,
            doorbells: None,
            wake: None,
            guest_get: None,
            guest_put: None,
            guest_entries: 0,
            issued: 0,
            fetched: 0,
            markers: 0,
            active: true,
            private_entries: 512,
        }
    }
}
/// A backwards clock cannot replace a coherent previous observation.
#[derive(Debug, Eq, PartialEq)]
pub struct BackwardsClock;
/// One producer-owned source record;31 atomic payload words on the wire.
pub struct RelayGauge {
    words: [u64; 31],
}
impl RelayGauge {
    /// Birth-time identity is assigned by the host profiler, not guest handles.
    pub fn new(generation: u64, key: u64, birth: u64) -> Self {
        let mut words = [0; 31];
        words[0] = 1;
        words[1] = generation;
        words[2] = key;
        words[4] = birth;
        words[23] = 1;
        words[28] = birth;
        words[29] = 1;
        Self { words }
    }
    /// Update only from actual observations. No hardware word can clear a Wait.
    pub fn observe(&mut self, now: u64, o: Observation) -> Result<[u64; 31], BackwardsClock> {
        if now < self.words[4] {
            return Err(BackwardsClock);
        }
        let w = &mut self.words;
        w[3] = u64::from(o.token);
        w[4] = now;
        if let Some(n) = o.private_get {
            if w[5] & 1 == 0 || w[6] != u64::from(n) {
                w[15] = now;
            }
            w[6] = u64::from(n);
            w[5] |= 1;
            w[25] = now;
        }
        if let Some(n) = o.private_put {
            w[7] = u64::from(n);
            w[5] |= 2;
            w[26] = now;
        }
        if let Some(n) = o.reference {
            w[8] = u64::from(n);
            w[5] |= 4;
            w[25] = now;
        }
        if let Some(n) = o.guest_get {
            if w[5] & 8 == 0 || w[16] != u64::from(n) {
                w[30] = now;
            }
            w[16] = u64::from(n);
            w[5] |= 8;
            w[19] = now;
        }
        if let Some(n) = o.guest_put {
            w[17] = u64::from(n);
            w[5] |= 16;
            w[19] = now;
        }
        if let Some(n) = o.doorbells {
            if (w[5] & 32 == 0 && n > 0) || (w[5] & 32 != 0 && w[11] != n) {
                w[12] = now;
            }
            w[11] = n;
            w[5] |= 32;
        }
        let (kind, target) = o.wait.words();
        if kind == 0 {
            w[27] = 0;
        } else if w[9] != kind || w[10] != target {
            w[27] = now;
        }
        w[9] = kind;
        w[10] = target;
        if let Some(why) = o.wake {
            w[13] = why as u64;
            w[14] = now;
        }
        w[18] = u64::from(o.guest_entries);
        w[20] = o.issued;
        w[21] = o.fetched;
        w[22] = o.markers;
        w[23] = o.active as u64;
        w[24] = u64::from(o.private_entries);
        Ok(*w)
    }
    /// Mark the producer retired, preserving progress and unanswered Wait identity.
    pub fn retire(&mut self, now: u64) -> Result<[u64; 31], BackwardsClock> {
        if now < self.words[4] {
            return Err(BackwardsClock);
        }
        self.words[4] = now;
        self.words[23] = 0;
        Ok(self.words)
    }
    /// Current trusted model words, for birth-time publication.
    pub fn words(&self) -> &[u64; 31] {
        &self.words
    }
}
