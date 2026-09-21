//! ★★★★★ **THE JOIN — and it is where `forwarded=0` lives.**
//!
//! A guest channel's operand names a framebuffer range. If no **host object** stands behind that
//! range, the GPU cannot execute against it and the work falls back to the CPU. That fallback is
//! the thin guest's whole failure signature, measured at w823 across **seven arms**:
//!
//! ```text
//! guest_tokens=1   stranded=1   forwarded=0   refused=0   client rc=0
//! ```
//!
//! ⊘ Note `refused=0`: **nothing was refused by name.** The work silently ran somewhere else and
//! every arm passed its own rows, because once a leaf is joined the guest's window and the host
//! object are one memory and both executors write identical bytes.
//!
//! ## ⊘⊘⊘ THE DEFECT, IN ONE LINE OF THE OLD CODE
//!
//! `kayfabe-isolate-host/src/fbjoin.rs:154` — `token_for(phys, len)` looks up by **equality**:
//!
//! ```ignore
//! t.iter().rev().find(|j| j.phys == phys && j.len == len && !j.alias)
//! ```
//!
//! ⇒ A query for a range **contained in** a joined leaf — a different offset inside it, or a
//! shorter length — matches nothing, returns `None`, and reads as *unbacked*. `CLAUDE.md` already
//! records this class from the other end: *"2 560 bytes our own `resolve` answers `Miss` for
//! inside a page the guest has mapped"*, held open by a `CrossesEnd` refusal, against a C that
//! could not have the hole because it rounded every mapping up to 64 KiB.
//!
//! ★ **So this table looks up by CONTAINMENT**, and returns the offset within the join. An exact
//! match is not a special case; it is the degenerate one.
//!
//! ## What makes this v3 and not a port
//!
//! ⊘ [`JoinTable::resolve`] cannot return a bare `None`. It returns [`JoinRefusal`], which **names
//! itself**, so `refused=0` means *nothing was refused* rather than *we never counted*. There is
//! no arm a caller can read as *"fall back to the CPU"* — that is the property the old code
//! lacked, and the reason six arms passed while doing the wrong thing.

use crate::addr::{page_cover, Fb, HostToken, StoreOffset, PAGE};

/// Why a framebuffer range has no host object behind it. ⊘ **Every arm is reportable by name.**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinRefusal {
    /// No join covers this range at all. ★ The honest answer to *"is this operand backed?"* — and
    /// the one the old code spelled `None` and a caller read as *"run it on the CPU"*.
    NotJoined { fb: Fb, len: u64 },
    /// A join starts inside this range but ends before it does. ⊘ Distinguished from
    /// [`Self::NotJoined`] deliberately: it means the guest is using an extent **larger** than the
    /// one we bound, which is a binding bug on our side, not an unmapped operand on the guest's.
    CrossesJoinEnd { fb: Fb, len: u64, join_end: Fb },
    /// A zero-length operand. Refused rather than silently succeeding on an empty range.
    ZeroLength { fb: Fb },
}

impl JoinRefusal {
    /// A short stable name for counters and logs. ⊘ §"refuse by name means the NAME IS TRUE".
    pub fn name(&self) -> &'static str {
        match self {
            JoinRefusal::NotJoined { .. } => "not_joined",
            JoinRefusal::CrossesJoinEnd { .. } => "crosses_join_end",
            JoinRefusal::ZeroLength { .. } => "zero_length",
        }
    }
}

/// One joined framebuffer leaf: guest FB range ⇄ a slice of the single store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Join {
    /// FB-physical base, **page-aligned** (see [`page_cover`]).
    pub fb: Fb,
    /// Length in bytes, **a whole number of pages**.
    pub len: u64,
    /// Where these bytes live in the single store.
    pub at: StoreOffset,
    /// The host object's name. ⊘ A name, never an address.
    pub token: HostToken,
    /// ⊘ `false` for the join that minted the memory, `true` for a second address over the same
    /// bytes. Two numbers, not one: the count of *addresses* and the count of *memories* differ,
    /// and a boot line printing only the first reported seventeen frames as fifty-one (w380).
    pub alias: bool,
}

impl Join {
    pub fn end(&self) -> Fb {
        Fb(self.fb.0 + self.len)
    }
    pub fn contains(&self, fb: Fb, len: u64) -> bool {
        fb.0 >= self.fb.0 && fb.0 + len <= self.end().0
    }
}

/// Where a resolved operand actually lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backing {
    pub token: HostToken,
    /// Byte offset **within the joined object** — what the old `token_for` could not express, and
    /// therefore had to demand an exact match to avoid needing.
    pub offset: u64,
    pub at: StoreOffset,
}

/// ★★★ Every framebuffer leaf joined for one guest.
#[derive(Debug, Default)]
pub struct JoinTable {
    joins: Vec<Join>,
    refused: Vec<(JoinRefusal, u64)>,
}

impl JoinTable {
    pub fn new() -> JoinTable {
        JoinTable::default()
    }

    /// Record a join. ⊘ The range is **page-covered** on the way in, so a caller that binds at a
    /// declared sub-page length cannot create the hole this module's docs describe.
    pub fn install(&mut self, fb: Fb, len: u64, at: StoreOffset, token: HostToken, alias: bool) -> Join {
        let (base, covered) = page_cover(fb.0, len);
        let j = Join { fb: Fb(base), len: covered, at: StoreOffset(at.0 & !(PAGE - 1)), token, alias };
        self.joins.push(j);
        j
    }

    /// ★★★★★ **THE LOOKUP.** By containment, newest-first, non-alias preferred.
    ///
    /// ⊘ **Newest-first is load-bearing and was measured** (w392j): nothing removes an entry when
    /// the VMM gives a join back, so a frame joined, released and re-joined carries two non-alias
    /// entries. The old `find` answered the **first** — the released object no guest window maps
    /// any more — so an alias of that frame mapped stale pages. Two memories, silently, under the
    /// word that says one.
    pub fn resolve(&mut self, fb: Fb, len: u64) -> Result<Backing, JoinRefusal> {
        if len == 0 {
            return Err(self.refuse(JoinRefusal::ZeroLength { fb }));
        }
        let hit = self
            .joins
            .iter()
            .rev()
            .find(|j| !j.alias && j.contains(fb, len))
            .or_else(|| self.joins.iter().rev().find(|j| j.contains(fb, len)));

        if let Some(j) = hit {
            let offset = fb.0 - j.fb.0;
            return Ok(Backing { token: j.token, offset, at: StoreOffset(j.at.0 + offset) });
        }

        // ⊘ Distinguish "we bound too small" from "never bound". Same `None` in the old code.
        if let Some(j) = self.joins.iter().rev().find(|j| j.contains(fb, 1)) {
            let end = j.end();
            return Err(self.refuse(JoinRefusal::CrossesJoinEnd { fb, len, join_end: end }));
        }
        Err(self.refuse(JoinRefusal::NotJoined { fb, len }))
    }

    fn refuse(&mut self, r: JoinRefusal) -> JoinRefusal {
        match self.refused.iter_mut().find(|(k, _)| k.name() == r.name()) {
            Some((_, n)) => *n += 1,
            None => self.refused.push((r, 1)),
        }
        r
    }

    /// Refusals, by name and count. ★ This is what makes `refused=0` mean something.
    pub fn refusals(&self) -> &[(JoinRefusal, u64)] {
        &self.refused
    }
    pub fn total_refusals(&self) -> u64 {
        self.refused.iter().map(|(_, n)| n).sum()
    }
    /// How many **addresses** are joined.
    pub fn len(&self) -> usize {
        self.joins.len()
    }
    pub fn is_empty(&self) -> bool {
        self.joins.is_empty()
    }
    /// How many **memories** — `len()` minus the aliases. ⊘ Two numbers, never one.
    pub fn memories(&self) -> usize {
        self.joins.iter().filter(|j| !j.alias).count()
    }
}
