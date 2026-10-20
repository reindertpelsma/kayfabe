//! Single-owner state transitions with bounded nesting and exact residence totals.
use crate::{
    histogram::Histogram,
    tables::{Site, State},
};

const STATES: usize = 32;
const DEPTH: usize = 8;
/// Published current state, including the original logical span entry time.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StateSnapshot {
    /// Current statically enumerated thread state.
    pub state: State,
    /// Current static instrumentation site.
    pub site: Site,
    /// Bounded integer argument.
    pub arg: u64,
    /// Logical span start, monotonic nanoseconds.
    pub enter_ns: u64,
}
/// Unique logical span identity, copied only to support rejected-close diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpanToken {
    depth: usize,
    generation: u64,
}
/// A transition rejected before changing the current span.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpanError {
    /// Fixed nesting depth exhausted.
    Full,
    /// Clock precedes the last accepted transition.
    Backwards,
    /// Span identity exhausted; new spans cannot reuse an ABA identity.
    Exhausted,
}
#[derive(Clone, Copy)]
struct Frame {
    parent: StateSnapshot,
    token: SpanToken,
}
/// Cache-line-aligned producer-local state; no allocator, lock or atomic RMW.
#[repr(align(64))]
pub struct ThreadState {
    current: StateSnapshot,
    segment_ns: u64,
    depth: usize,
    generation: u64,
    stack: [Option<Frame>; DEPTH],
    histograms: [Histogram; STATES],
    exclusive: [u64; STATES],
    dropped: u64,
}
impl ThreadState {
    /// Begin in Idle at an already calibrated timestamp.
    pub fn new(now: u64) -> Self {
        Self {
            current: StateSnapshot {
                state: State::Idle,
                site: Site::Unknown,
                arg: 0,
                enter_ns: now,
            },
            segment_ns: now,
            depth: 0,
            generation: 0,
            stack: [None; DEPTH],
            histograms: [Histogram::new(); STATES],
            exclusive: [0; STATES],
            dropped: 0,
        }
    }
    fn segment(&mut self, now: u64) {
        let index = self.current.state as usize;
        self.exclusive[index] = self.exclusive[index].saturating_add(now - self.segment_ns);
        self.segment_ns = now;
    }
    /// Enter a nested state, preserving the parent's original logical entry time.
    pub fn enter(
        &mut self,
        state: State,
        site: Site,
        arg: u64,
        now: u64,
    ) -> Result<SpanToken, SpanError> {
        if now < self.segment_ns {
            return Err(SpanError::Backwards);
        }
        if self.depth == DEPTH {
            self.dropped = self.dropped.saturating_add(1);
            return Err(SpanError::Full);
        }
        let generation = self.generation.checked_add(1).ok_or(SpanError::Exhausted)?;
        self.segment(now);
        let token = SpanToken {
            depth: self.depth,
            generation,
        };
        self.stack[self.depth] = Some(Frame {
            parent: self.current,
            token,
        });
        self.depth += 1;
        self.generation = generation;
        self.current = StateSnapshot {
            state,
            site,
            arg,
            enter_ns: now,
        };
        Ok(token)
    }
    /// Leave only the current logical span. Invalid/out-of-order closes do nothing.
    pub fn leave(&mut self, token: SpanToken, now: u64) -> bool {
        if now < self.segment_ns || self.depth == 0 {
            return false;
        }
        let Some(frame) = self.stack[self.depth - 1] else {
            return false;
        };
        if frame.token != token {
            return false;
        }
        self.segment(now);
        self.histograms[self.current.state as usize]
            .record(now - self.current.enter_ns, self.current.arg);
        self.current = frame.parent;
        self.depth -= 1;
        self.stack[self.depth] = None;
        true
    }
    /// Current logical state. Live residence includes any unfinished current segment.
    pub fn snapshot(&self) -> StateSnapshot {
        self.current
    }
    /// Exact residence integrated through the last transition, excluding child states.
    pub fn exclusive_ns(&self, state: State) -> u64 {
        self.exclusive[state as usize]
    }
    /// Completed inclusive spans, with count, total, maximum argument and histogram.
    pub fn histogram(&self, state: State) -> &Histogram {
        &self.histograms[state as usize]
    }
    /// Rejected span entries due to fixed stack exhaustion.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}
