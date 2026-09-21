//! The **join**: a guest framebuffer range ⇄ a host object, looked up by containment.
//!
//! ## ⊘⊘⊘ THIS FILE'S OPENING CLAIM WAS WRONG, AND IT IS KEPT AS THE CORRECTION
//!
//! The first version of this docstring was headed *"THE JOIN — and it is where `forwarded=0`
//! lives"*, and its regression test was named as proving *"THIS WAS THE DEFECT"*. **It was not.**
//! `[fable w824]`, confirming the owner's doubt:
//!
//! - the only real caller of the old `token_for` already **refuses by name** (`FB_ALIAS_NO_JOIN`),
//!   so nothing silently fell back there;
//! - its strictness has a stated rationale — a partial or shifted alias *"would place a host
//!   mapping whose bytes are only partly the ones the guest reaches, and would do so
//!   **successfully**"*;
//! - and the real cause of `forwarded=0` is **`kayfabe-rt/src/device.rs:8978`**:
//!   ```ignore
//!   EngineKind::Ce => DoorbellRoute::CpuCe,   // unconditional
//!   ```
//!   CE doorbells are routed to the CPU executor **by design**. The seven failing arms are the old
//!   architecture working as specified, not a lookup bug.
//!
//! ⇒ **The rule this cost, now standing:** *"THE DEFECT"* is a label only a commit **whose gate
//! line moved** may use. A reading of old code is a **hypothesis**; it belongs in a commit body
//! said as one, never in a module's opening line, where it becomes the thing the next reader
//! believes. ⚠ This is the tree's own lesson — `worst_trap` names the site, not the cause — and I
//! committed the mistake into the code rather than merely into a message.
//!
//! ## What this module is actually for, stated without the story
//!
//! A guest channel operand names a framebuffer range; something must say which host object stands
//! behind it. [`JoinTable::resolve`] answers **by containment** rather than by equality, so a
//! range *inside* a joined leaf resolves with an offset instead of reading as absent. The old
//! `fbjoin.rs:154` matched `phys == phys && len == len`, and the old tree has tests pinning that
//! (`assert_eq!(t.token_for(0x1_0000, 0x8000), None, "half the frame")`).
//!
//! ⊘ **Measured improvement, no causal claim:** containment resolves ranges equality refuses; the
//! regression below demonstrates that difference and nothing more. Whether it moves any gate line
//! is unknown until one moves.
//!
//! ★ And [`JoinTable::resolve`] cannot return a bare `None`: [`JoinRefusal`] **names itself**, so
//! `refused=0` means nothing was refused rather than nobody counted.

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
