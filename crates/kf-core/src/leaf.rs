//! Page-table leaves, and the bound the system-memory leaf must carry — §6.4.
//!
//! ## ★★★ This module exists to make the WRONG thing unexpressible
//!
//! §6.4's rule, and `crates/kayfabe-isolate/src/lib.rs:518` on why it must be a type:
//!
//! > **We originate the numbers. We never validate a number the guest proposed against a range we
//! > derived from that same proposal.**
//! >
//! > *"Checks can be added later; shapes cannot."*
//!
//! ⊘⊘⊘ **Why the obvious implementation is circular.** The tempting form is
//! `resolve(guest_addr, len) -> Option<HostPtr>`: take the guest's leaf, read its address, then
//! ask *"is that inside guest RAM?"* That validates a request **against itself** — and if the
//! layout answering it is a generic guest-physical→host-pointer map, it will happily resolve
//! **our own** memslots: the register read shadow, the doorbell bitmap, the boot pages. ⇒ The
//! guest names one of those, we "validate" it, and we have **pinned our own state and handed it
//! to the GPU as a DMA target** — the engine can then write the bits the drainer owns.
//!
//! ⇒ **There is deliberately no such function in this module.** The only way to obtain a host
//! slice is [`GuestRamBlock::slice`], which starts from a block **we** minted.

/// A registered region of genuine guest RAM.
///
/// ⊘ Minted only at guest-RAM registration (§6.3's memfd, registered **once at startup**), never
/// derived from a guest value. ⚠ The field is private and there is no public constructor taking a
/// guest-supplied base — that absence *is* the security property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestRamBlock {
    id: u32,
    gpa_base: u64,
    len: u64,
}

/// A bounded slice of a registered block. The only thing a host mapping call may be given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostSlice {
    pub block: u32,
    pub offset: u64,
    pub len: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeafRefusal {
    /// The guest named a guest-physical address in no registered block.
    ///
    /// ⚠ §6.4: *"A leaf whose guest-physical address falls in no registered guest-RAM block is a
    /// refused leaf, by name and counted — never a fall-through to a generic map."* ⊘ The most
    /// important instance of this is the guest naming **our own** memslots.
    NotInAnyRegisteredBlock,
    /// It starts inside a block but runs past its end.
    CrossesBlockEnd,
    /// A bit we translate only as an allowlist carried a value we do not mint.
    ForbiddenAttribute,
}

impl GuestRamBlock {
    /// ⊘ Called by the registration path **only**, at startup, before the guest runs. §6.4's first
    /// consequence: *"the set of blocks is closed before the guest runs, so no guest action can
    /// add one."*
    pub fn register(id: u32, gpa_base: u64, len: u64) -> GuestRamBlock {
        GuestRamBlock { id, gpa_base, len }
    }

    /// ★ The ONLY way to a host slice. The guest's leaf **selects a block and an offset within
    /// it**; it can never name a base.
    pub fn slice(&self, offset: u64, len: u64) -> Result<HostSlice, LeafRefusal> {
        // ⊘ Checked arithmetic: an overflowing `offset + len` must not wrap into a legal-looking
        // range. This is the one place a guest value meets a bound, so it is the one place
        // wrapping would be fatal.
        let end = offset.checked_add(len).ok_or(LeafRefusal::CrossesBlockEnd)?;
        if end > self.len {
            return Err(LeafRefusal::CrossesBlockEnd);
        }
        Ok(HostSlice { block: self.id, offset, len })
    }

    #[inline]
    pub fn contains_gpa(&self, gpa: u64) -> bool {
        gpa >= self.gpa_base && gpa < self.gpa_base + self.len
    }
}

/// The closed set of registered blocks.
///
/// ⊘⊘ **Our own memslots are never in here** — §6.4's second consequence. The read shadow, the
/// doorbell bitmap and the boot pages are host memory installed as guest-physical ranges: the
/// guest's **CPU** may reach them (deliberately), and a **DMA leaf** never may. *"One map serving
/// both is the bug."*
#[derive(Debug, Default)]
pub struct GuestRamLayout {
    blocks: Vec<GuestRamBlock>,
    /// §6.4's third consequence: *"The refusal is counted, not silent."*
    refused: std::cell::Cell<u64>,
}

impl GuestRamLayout {
    pub fn new() -> GuestRamLayout {
        GuestRamLayout::default()
    }
    pub fn register(&mut self, b: GuestRamBlock) {
        self.blocks.push(b);
    }

    /// Resolve a guest leaf.
    ///
    /// ★ Note the shape: this does **not** return a host pointer derived from `gpa`. It finds
    /// which block we already hold contains it, and then the caller must go through
    /// [`GuestRamBlock::slice`]. The guest value is used to **select**, never to **construct**.
    pub fn leaf(&self, gpa: u64, len: u64) -> Result<HostSlice, LeafRefusal> {
        let Some(b) = self.blocks.iter().find(|b| b.contains_gpa(gpa)) else {
            self.refused.set(self.refused.get() + 1);
            return Err(LeafRefusal::NotInAnyRegisteredBlock);
        };
        let r = b.slice(gpa - b.gpa_base, len);
        if r.is_err() {
            self.refused.set(self.refused.get() + 1);
        }
        r
    }

    /// ⚠ A zero here is evidence, not silence — see §6.4 and `a_census_zero_needs_a_known_positive`.
    #[inline]
    pub fn refused(&self) -> u64 {
        self.refused.get()
    }
}
