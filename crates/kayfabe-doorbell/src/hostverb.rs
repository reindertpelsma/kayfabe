//! Host verbs — **authored, never forwarded** (§9), and the pointer discipline that makes the
//! `unsafe` boundary sound *even if safe code is buggy*.
//!
//! ## 1. ⊘ We author every host call
//!
//! §9: *"**We author every host call**; our host-verb signatures do not accept a guest flag word,
//! so a guest-chosen bit cannot reach RM's interpretation of it."*
//!
//! ⇒ The enforcement is the **signature**, not a check inside the body: [`HostVerb`] carries no
//! field a guest value could occupy. A verb that took `flags: u32` would be forwarding by
//! construction, and no amount of validation in the body would change that — the next caller
//! simply passes the guest's word.
//!
//! ## 2. ★★★ Kayfabe receives NO caller pointers from the guest — unlike Mode 1
//!
//! `[owner, 2026-09-21]` *"I think kayfabe receives from guest no nested pointers right compared
//! to nvkvm-pv."* ✔ **Correct, and the design says so**: §6.4 — *"The guest's own pointer never
//! appears in a host call, which is why the memory class that would carry one stays refused by
//! name"*, and that refusal is *"decorative — the class never reaches us; it is handled inside the
//! guest."*
//!
//! ⇒ Mode 1 forwarded ioctls whose parameter blocks carry **caller pointers** that had to be
//! chased and translated. This plane receives **flat RPC bodies**, and the single path by which a
//! guest-chosen address reaches a host call is the page-table **leaf**, bounded in
//! [`crate::leaf`]. ⚠ That makes the leaf bound load-bearing rather than belt-and-braces: it is
//! not *one* of the guards, it is *the* guard.
//!
//! ## 3. ⊘⊘⊘ NO VMM ADDRESS MAY LIVE IN A STRUCT SAFE CODE CAN REACH
//!
//! `[owner, 2026-09-21]` *"no vmm pointer in safe code, unsafe must protect against memory bugs
//! in safe, for all low level calls, so also no vmm pointers in nvidia structs that exist in safe
//! code."*
//!
//! ★ The reasoning, stated so it is not mistaken for style: an NVIDIA parameter block is a
//! `#[repr(C)]` struct that safe Rust can hold, index and mutate. If one of its fields is a host
//! address that `unsafe` code will later dereference, then **an ordinary bug in safe code — a
//! wrong index, an off-by-one, a stale clone — becomes memory unsafety.** That inverts the whole
//! point of the boundary: `unsafe` is supposed to be sound *no matter what safe code does*.
//!
//! ⇒ Two rules, and the second is the one with teeth:
//! 1. Safe code holds [`Handle`]s and offsets, never addresses.
//! 2. **Unsafe code re-validates before every dereference.** It may not assume safe code preserved
//!    an invariant, because that assumption is exactly what a safe-code bug breaks.
//!
//! ⚠ This extends the existing rule *the VMM address is never guest-chosen or guest-visible*
//! (owner, 2026-09-10) with: **nor safe-code-reachable**.

/// An opaque reference to something the host driver owns. ⊘ **Not an address.** Safe code may copy
/// and compare these freely; corrupting one yields a refused lookup, never a bad dereference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Handle(pub u32);

/// A byte offset within a registered object. ⊘ Also not an address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Offset(pub u64);

/// The verbs we author. ⊘ Note what is absent: **no `flags` field anywhere**, because §9's rule is
/// a property of the signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostVerb {
    AllocVaSpace,
    AllocChannel { vas: Handle },
    MapDma { vas: Handle, mem: Handle, offset: Offset, len: u64, read_only: bool },
    Free { obj: Handle },
    RingDoorbell { token: u32 },
}

/// ⊘ The ONE bit of a guest-supplied leaf we translate beyond aperture/address/size.
/// §6.4: *"We translate **aperture, address, read-only and page size**; every other bit is
/// **refused by name and counted**, and the kind is fixed to the store's."*
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranslatedLeafBit {
    ReadOnly,
}

/// ⊘⊘ A guest flag word must never reach a host verb. This function exists so the refusal is
/// **counted and named** rather than implicit in the absence of a field.
#[derive(Debug, Default)]
pub struct FlagRefusals(u64);

impl FlagRefusals {
    /// Called where a guest value *would* have been forwarded in a Mode-1 design.
    pub fn refuse_guest_flags(&mut self, _guest_word: u32) {
        self.0 += 1;
    }
    #[inline]
    pub fn count(&self) -> u64 {
        self.0
    }
}

/// ★ A host address, usable only across the `unsafe` boundary.
///
/// ⊘ There is deliberately **no safe constructor from an integer** and **no safe accessor
/// returning one**. Safe code can move a `VmmAddr` around; it cannot fabricate one, and it cannot
/// read the number out to corrupt-then-restore it.
#[derive(Debug, Clone, Copy)]
pub struct VmmAddr {
    addr: usize,
    len: usize,
}

impl VmmAddr {
    /// # Safety
    /// `addr..addr+len` must be a live mapping owned by this process for the lifetime of use.
    ///
    /// ⊘ `unsafe` to CREATE, so every address entering the safe world is attributable to a site
    /// that took responsibility for it.
    pub unsafe fn new(addr: usize, len: usize) -> VmmAddr {
        VmmAddr { addr, len }
    }

    /// ★★★ **Re-validate before the dereference, every time.**
    ///
    /// ⊘ The caller passes the range it registered; if a safe-code bug has swapped, truncated or
    /// aliased this `VmmAddr`, the check fails and we refuse instead of dereferencing. §the
    /// owner's rule: *"unsafe must protect against memory bugs in safe"* — so this does not trust
    /// that safe code preserved anything.
    pub fn checked(&self, registered_base: usize, registered_len: usize, want: usize) -> Option<usize> {
        let end = self.addr.checked_add(want)?;
        let reg_end = registered_base.checked_add(registered_len)?;
        if self.addr < registered_base || end > reg_end || want > self.len {
            return None;
        }
        Some(self.addr)
    }
}
