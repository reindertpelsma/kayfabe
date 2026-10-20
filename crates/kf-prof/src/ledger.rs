//! A producer-owned fixed-slot obligation ledger with O(1) open and close.

/// Opaque close authority; identity includes owner, kind, slot, generation and id.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Token {
    owner: u64,
    kind: usize,
    slot: usize,
    generation: u64,
    id: u64,
}
impl Token {
    /// Kind-local slot index, for wire publication only.
    pub fn slot(self) -> usize {
        self.slot
    }
    /// Slot generation; never reused after overflow.
    pub fn generation(self) -> u64 {
        self.generation
    }
    /// Static obligation kind.
    pub fn kind(self) -> usize {
        self.kind
    }
}
/// An outstanding obligation, never a production decision input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Entry {
    /// Static obligation kind.
    pub kind: usize,
    /// Bounded integer correlation ID.
    pub id: u64,
    /// Host monotonic birth timestamp.
    pub birth_ns: u64,
    /// Identity of this slot's current life.
    pub generation: u64,
}
impl Entry {
    /// Conservative age; a backwards observation yields zero, never integer wrap.
    pub fn age_ns(self, now: u64) -> u64 {
        now.saturating_sub(self.birth_ns)
    }
    /// Strict advisory comparison; this cannot mutate work or complete an entry.
    pub fn overdue(self, now: u64, bound: u64) -> bool {
        self.age_ns(now) > bound
    }
}
struct Kind<const S: usize> {
    entries: [Option<Entry>; S],
    free: [usize; S],
    free_len: usize,
    generations: [u64; S],
}
impl<const S: usize> Kind<S> {
    fn new() -> Self {
        Self {
            entries: [None; S],
            free: core::array::from_fn(|i| S - 1 - i),
            free_len: S,
            generations: [0; S],
        }
    }
}
/// Bounded ledger local to one producer. Slots are not shared between threads.
///
/// Owner IDs must be assigned uniquely by host initialization. Rejected/stale
/// tokens are harmless even when safe callers use arbitrary ledger dimensions.
pub struct Ledger<const K: usize, const S: usize> {
    owner: u64,
    kinds: [Kind<S>; K],
    dropped: u64,
}
impl<const K: usize, const S: usize> Ledger<K, S> {
    /// Initialize kind-local free lists; call before entering a hot path.
    pub fn new(owner: u64) -> Self {
        Self {
            owner,
            kinds: core::array::from_fn(|_| Kind::new()),
            dropped: 0,
        }
    }
    /// Open an obligation in O(1); full or invalid kinds count a rejection.
    pub fn open(&mut self, kind: usize, id: u64, birth_ns: u64) -> Option<Token> {
        let Some(k) = self.kinds.get_mut(kind) else {
            self.dropped = self.dropped.saturating_add(1);
            return None;
        };
        if k.free_len == 0 {
            self.dropped = self.dropped.saturating_add(1);
            return None;
        }
        k.free_len -= 1;
        let slot = k.free[k.free_len];
        // Slots with exhausted generations are retired on close, not rescanned here.
        let generation = k.generations[slot] + 1;
        k.generations[slot] = generation;
        k.entries[slot] = Some(Entry {
            kind,
            id,
            birth_ns,
            generation,
        });
        Some(Token {
            owner: self.owner,
            kind,
            slot,
            generation,
            id,
        })
    }
    /// Close only this exact life; stale, double or wrong-owner closes do nothing.
    /// `close_ns` is accepted for future wire publication, never a work decision.
    pub fn close(&mut self, token: Token, _close_ns: u64) -> bool {
        if token.owner != self.owner {
            return false;
        }
        let Some(k) = self.kinds.get_mut(token.kind) else {
            return false;
        };
        let Some(entry) = k.entries.get_mut(token.slot) else {
            return false;
        };
        let Some(current) = *entry else {
            return false;
        };
        if current.generation != token.generation || current.id != token.id {
            return false;
        }
        *entry = None;
        if current.generation != u64::MAX {
            k.free[k.free_len] = token.slot;
            k.free_len += 1;
        }
        true
    }
    /// Iterate a compile-time bounded inventory. Aggregator-side use only.
    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.kinds.iter().flat_map(|k| k.entries.iter().flatten())
    }
    /// Number of rejected opens (saturating).
    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}
