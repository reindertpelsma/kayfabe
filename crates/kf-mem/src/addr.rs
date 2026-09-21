//! Address spaces, as **distinct types**.
//!
//! ⊘⊘⊘ **THIS MODULE EXISTS BECAUSE OF A BUG I WROTE TODAY.** `memmap::install` passed a
//! BAR-relative offset where a guest-physical address was wanted; both were `u64`, nothing
//! complained, and BAR1 (256 MiB from 0) silently "covered" a BAR0 hole at `0x8F2000`. It was
//! caught only by a test that checked a concrete address.
//!
//! ⇒ **Two address spaces that are both plain `u64` WILL be confused.** This crate handles four
//! of them at once, so each gets a type and the compiler does the checking.

/// **Guest-physical.** What a guest page-table leaf names, and what the VMM speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Gpa(pub u64);

/// **Framebuffer-physical**, in the EMULATED device's frame. ⊘ The VMM's number — nothing here
/// derives it, and it is the only key by which a range can be named across processes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Fb(pub u64);

/// A byte offset **into the single store**. ⊘ Never an address: the store's base lives behind the
/// host adapter's `unsafe`, so safe code can hold this and still not be able to dereference it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StoreOffset(pub u64);

/// **GPU virtual**, inside one VA space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Gva(pub u64);

/// An opaque **name** for a host memory object. ⊘ §"no VMM pointer in safe code": a token is a
/// name we can pass around and compare; turning one into a mapping is the host adapter's job and
/// happens behind its own `unsafe`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HostToken(pub u64);

pub const PAGE: u64 = 0x1000;

/// ★ Round a range out to page boundaries.
///
/// ⊘⊘⊘ **NOT COSMETIC — this is the sub-page-hole defect.** `CLAUDE.md` records it: the C artifact
/// rounded every promote-derived mapping **up to 64 KiB** (`nvkvm_gpu_emul.c:7920`), while the
/// Rust port bound at the **declared length** and produced *"2 560 bytes our own `resolve` answers
/// `Miss` for inside a page the guest has mapped"*. ⇒ A join must cover whole pages, because the
/// guest's own mapping does, and a join narrower than a page leaves a hole the guest can reach.
#[must_use]
pub fn page_cover(base: u64, len: u64) -> (u64, u64) {
    let start = base & !(PAGE - 1);
    let end = (base + len).div_ceil(PAGE) * PAGE;
    (start, end - start)
}
