//! Bounded classification of one intact USER descriptor; never a GPU submission.
//! One part is copied per worker turn. Only publication commits the resulting Scan.
#![cfg_attr(
    not(test),
    deny(
        clippy::arithmetic_side_effects,
        clippy::indexing_slicing,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable
    )
)]

use crate::user_scan::{PartCursor, PartOutcome, Scan};
use crate::user_stream::{Source, inspect_format};
use kf_abi::submit::GpfifoEntry;

/// ceil((2^21 - 1) * 4 / 917504): the public format requires at most ten parts.
pub const MAX_PARTS: u8 = 10;

/// Result of one bounded classification step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fed {
    /// Another part is owed, without another guest notification.
    More,
    /// END or the descriptor end was reached without a software trigger.
    Done,
}

/// A refusal grants no publication, software action or consumption credit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Invalid descriptor, cut bound, part count or supplied word count.
    Bounds,
    /// Existing parser refusal (the owner applies its existing bounded grace).
    Scan(crate::user_scan::Refusal),
    /// A trigger requires a whole patched copy larger than staging.
    StagedSpanBudget {
        /// Absolute trigger-header position within the original descriptor.
        at_word: usize,
    },
}

/// One pending descriptor, with no guest pointers, source bytes, I/O or clock.
#[derive(Clone, Debug)]
pub struct Stream {
    entry: u64,
    va: u64,
    total: u64,
    maximum: u64,
    next: u64,
    part: u8,
    scan: Scan,
    cursor: PartCursor,
    ended: bool,
}

impl Stream {
    /// Start only after a Scan plan and its synchronization gate.
    ///
    /// # Errors
    /// Inconsistent format/descriptor, a zero/unaligned/oversized window, a small
    /// descriptor, or more than the format-derived ten bounded parts.
    pub fn begin(
        desc: GpfifoEntry,
        entry: u64,
        scan: &Scan,
        maximum: u64,
    ) -> Result<Self, Refusal> {
        if inspect_format(entry) != Ok(Source::Segment(desc))
            || maximum == 0
            || maximum & 3 != 0
            || maximum > crate::user_storage::MAX_SOURCE_BYTES
            || desc.len_bytes <= maximum
        {
            return Err(Refusal::Bounds);
        }
        let round = maximum.checked_sub(1).ok_or(Refusal::Bounds)?;
        let parts = desc
            .len_bytes
            .checked_add(round)
            .and_then(|n| n.checked_div(maximum))
            .ok_or(Refusal::Bounds)?;
        if parts > u64::from(MAX_PARTS) {
            return Err(Refusal::Bounds);
        }
        Ok(Self {
            entry,
            va: desc.gpu_va,
            total: desc.len_bytes,
            maximum,
            next: 0,
            part: 0,
            scan: scan.clone(),
            cursor: PartCursor::default(),
            ended: false,
        })
    }

    /// The next fixed cut; None after END or exhaustion.
    #[must_use]
    pub fn next_read(&self) -> Option<(u64, u64)> {
        if self.ended || self.part >= MAX_PARTS {
            return None;
        }
        let left = self.total.checked_sub(self.next)?;
        if left == 0 {
            return None;
        }
        Some((self.va.checked_add(self.next)?, left.min(self.maximum)))
    }

    /// Parts already consumed; also the ordinal of the next part.
    #[must_use]
    pub fn parts(&self) -> u8 {
        self.part
    }

    /// True only after a complete classification without a trigger.
    #[must_use]
    pub fn done(&self) -> bool {
        self.ended || self.next == self.total
    }

    /// The original intact word; output never contains split descriptors.
    #[must_use]
    pub fn entry(&self) -> u64 {
        self.entry
    }

    /// Consume exactly the planned bounded words. A trigger is never executed.
    ///
    /// # Errors
    /// Word-count/part bound, the existing parser's refusal, or a trigger whose
    /// original descriptor exceeds the staging span.
    pub fn feed(&mut self, words: &[u32], software: impl Fn(u32) -> bool) -> Result<Fed, Refusal> {
        let (_, bytes) = self.next_read().ok_or(Refusal::Bounds)?;
        if u64::try_from(words.len())
            .ok()
            .and_then(|n| n.checked_mul(4))
            != Some(bytes)
        {
            return Err(Refusal::Bounds);
        }
        let outcome = self
            .scan
            .part(words, &mut self.cursor, software)
            .map_err(Refusal::Scan)?;
        match outcome {
            PartOutcome::Trigger(t) => return Err(Refusal::StagedSpanBudget { at_word: t.start }),
            PartOutcome::Ended { .. } => self.ended = true,
            PartOutcome::Exhausted => {}
        }
        self.next = self.next.checked_add(bytes).ok_or(Refusal::Bounds)?;
        self.part = self
            .part
            .checked_add(1)
            .filter(|n| *n <= MAX_PARTS)
            .ok_or(Refusal::Bounds)?;
        Ok(if self.done() { Fed::Done } else { Fed::More })
    }

    /// Return the owned metadata. The owner commits it only after publication.
    #[must_use]
    pub fn into_scan(self) -> Scan {
        self.scan
    }
}
