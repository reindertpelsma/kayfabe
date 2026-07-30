//! ★ CPU cacheability as a **required** parameter of every mapping — and an honest
//! account of how little of it this layer can enforce.
//!
//! Research and citations: `docs/reference/memory_cacheability.md`. The short version is
//! the part that has to be next to the code, because the failure it prevents is a 2 a.m.
//! failure:
//!
//! ## The four independent deciders
//!
//! A byte of memory has **one** effective CPU cache type, and on the path from our
//! `mmap` to a guest instruction touching it there are **four** parties that each get a
//! say. They do not negotiate; the hardware resolves them, and the resolution rule is not
//! "the last one wins".
//!
//! | # | Decider | Who owns it | Can this crate set it? |
//! |---|---|---|---|
//! | 1 | the **host userspace PTE** — what our own loads/stores see | the kernel: the page allocator for anonymous/page-cache memory, or the *driver's* `mmap` handler for a device fd | **No.** `mmap` has no cacheability flag. See below. |
//! | 2 | the **host EPT/NPT entry** — what a guest access to a memslot gets | KVM, from `kvm_is_mmio_pfn()` and the arch's ignore-PAT rules | No — `kayfabe-vmm`'s memslot install, and even there only indirectly |
//! | 3 | the **guest PTE** — the guest driver's own `vm_page_prot` | the guest kernel (for us: the stock NVIDIA module, plus the guest-side shim) | No. **This is where #111 actually was.** |
//! | 4 | what the **device** requires | NVIDIA RM, fixed by the allocation's `NV_MEMORY_*` attribute at `RM_ALLOC`/`RM_MAP_MEMORY` time | No — that is the forwarding layer's ioctl, not a mapping flag |
//!
//! ## ★ The fact that shapes this whole API: `mmap` has no cacheability argument
//!
//! There is no `MAP_WRITECOMBINE`, no `PROT_UNCACHED`, no `madvise` for it. On Linux the
//! cache type of a userspace mapping is decided **entirely** by what is being mapped:
//!
//! - **anonymous, `memfd`, `tmpfs`, a regular file** → ordinary kernel pages, mapped
//!   **write-back**, always. Nothing a caller passes can change it.
//! - **a device fd** → whatever that driver's `mmap` handler put in `vma->vm_page_prot`.
//!   For NVIDIA that is `nv_encode_caching()`, and *its* input is the `NV_MEMORY_*`
//!   attribute chosen when the allocation was made — an ioctl, hours of code away from
//!   the `mmap` call.
//!
//! So a [`CachePolicy`] argument here cannot *cause* an attribute. What it can do — and
//! what it is for — is exactly three things, in descending order of strength:
//!
//! 1. **Refuse the unattainable, loudly.** Asking for [`CachePolicy::WriteCombining`]
//!    over a `memfd` is [`RawError::CachePolicyUnattainable`], not a silent write-back
//!    mapping. This is the one *mechanism* in this module, and it is the #111 failure
//!    shape — a request quietly downgraded — turned into an error return.
//! 2. **Force every call site to state the requirement**, so the requirement exists in
//!    the source at all. The C reached the same set of attributes by patching them in
//!    afterwards, one region at a time; the parameter is what makes that impossible here.
//! 3. **Name the obligation-holder** for the parts we cannot enforce, per variant, below.
//!
//! **This is deliberately not reassuring.** An API that appeared to set cacheability
//! would be worse than one that says it cannot: the C's days of debugging were spent
//! because a *request* (`remap_pfn_range` with a write-back `vm_page_prot`) was silently
//! not honoured, and a comforting wrapper reproduces exactly that.
//!
//! ## What this layer cannot enforce, and whose job each piece is
//!
//! - **Decider 2 (EPT/NPT).** `kayfabe-vmm`'s `map_guest` / `map_read_native`. On Intel,
//!   KVM sets the EPT type write-back **with `IPAT`** for a struct-page-backed memslot,
//!   which overrides the guest PTE entirely; on **AMD NPT there is no `IPAT`** and the
//!   guest PTE wins. A bug of this class is therefore *invisible on Intel and fatal on
//!   AMD*, which is precisely how #111 survived a passing test run.
//! - **Decider 3 (the guest PTE).** The guest kernel module / the emulated device's BAR
//!   attributes. **Not ours, and no mapping flag reaches it.**
//! - **Decider 4 (the device's requirement).** The `NV_MEMORY_*` attribute on the RM
//!   allocation, issued by the isolate's forwarding path. The [`CachePolicy`] passed here
//!   must *match* what was asked for there; nothing in this crate can check that, and the
//!   check that could is a review of the two call sites together.
//!
//! ## ★★ "Prefer write-back and let something below downgrade it" — half right, and the
//! wrong half is the dangerous one
//!
//! The lesson the C left behind is that write-back is almost always the answer. For
//! **memory we back with host RAM** that is correct and the evidence is overwhelming: the
//! C recovered 130×, 118×, 8.9× and ~100× on four separate paths purely by moving a
//! host-RAM-backed mapping from uncached or write-combining to write-back.
//!
//! But the C's *first* implementation was the mirror-image blanket — write-combining for
//! every device mapping — and every subsequent fix was an after-the-fact correction to it.
//! **Neither blanket is right, and the reason a blanket looks safe is a guest-only
//! accident:** the project measured that mapping *everything* write-back in the guest did
//! not break the doorbell, because inside a VM a real device-BAR pfn gets its EPT type
//! forced uncached by `kvm_is_mmio_pfn()` regardless of what the guest asked for. **The
//! hypervisor was silently correcting the guest.** This crate runs in the **host** process,
//! where there is no such backstop: a wrong attribute here is simply wrong. The safety
//! margin that made "prefer write-back" survivable does not exist at this layer.
//!
//! So the rule this API encodes is not "prefer write-back". It is:
//!
//! > **The attribute belongs to the region, is decided where the region's class is known,
//! > and travels with it.** Never inferred at the mapping site from a proxy for the class.
//!
//! The C's second-generation fix inferred it from the *device node* the mapping came
//! through (`/dev/nvidiactl` ⇒ write-back, `/dev/nvidia0` ⇒ write-combining). NVIDIA's own
//! driver classifies by **address range within the node** and produces *three* different
//! attributes from one fd — uncached for BAR0, uncached for the USERD/doorbell sub-window,
//! write-combining for the rest of the framebuffer. A per-fd proxy cannot express that, and
//! the C's own tracking issue (#95, *"plumb a memtype from the host mmap classification"*)
//! is precisely this parameter, filed and never built.
//!
//! ## ★ The methodological lesson, which is worth more than the table
//!
//! For a while the C believed #111 was in the host EPT, on the strength of an experiment
//! showing that setting the guest PTE write-back changed nothing. The experiment was sound
//! and the conclusion was wrong, because **the PTE never became write-back**:
//! `remap_pfn_range` had silently downgraded it, and only a throwaway kernel module that
//! *decoded the live PTE* revealed it. Requesting an attribute and observing no change is
//! not evidence about the attribute — it is evidence about the request. Anything in this
//! space is only believed once the attribute has been **read back**, and from userspace it
//! cannot be, which is the honest limit stated above.
//!
//! ## No default, deliberately — and one place the type carries it instead
//!
//! [`CachePolicy`] has no `Default` impl (`tests/ui/cache_policy_has_no_default.rs`), the
//! parameter is never `Option`, and the enum is **not** `#[non_exhaustive]` — adding a
//! fourth attribute must be a compile error at every match, i.e. a reviewed change rather
//! than a silently-absorbed one. **A default is a blanket, and a blanket is the bug.**
//!
//! The one region class with exactly one right answer has it encoded in its type rather
//! than trusted to a caller: [`crate::Reservation::new`] takes **no** cache policy at all,
//! because `PROT_NONE` address space is never accessed and so has no attribute to get
//! wrong. Its *placements* take one, like every other mapping. Nothing else in the API is
//! type-determined, and the reason is in [`crate::Backing`]: attainability is a property of
//! **what the pages are**, not of which region type wraps them.

use crate::error::RawError;
use core::fmt;

/// The CPU cache attribute a mapping **requires**.
///
/// Three variants, because three is what the hardware and the NVIDIA driver actually
/// distinguish for the mappings this project makes. There is no `WriteThrough` and no
/// `WriteProtected`: x86 PAT can encode them, `nv_encode_caching` never selects them for
/// a user mapping, and a variant nothing constructs is a variant that gets guessed at.
///
/// ## Reading the per-variant notes
///
/// Each variant answers the two questions that cost the C days: **who else can override
/// this**, and **what does the mismatch look like from the outside**. The second is the
/// important one — every mismatch in this space presents as a *performance or liveness*
/// symptom, never as an error, which is why they are so expensive to find.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CachePolicy {
    /// **Write-back** — normal cached memory. x86 PAT entry `WB`; arm64 `MT_NORMAL`.
    ///
    /// The correct policy for every mapping this crate can make today: guest RAM and its
    /// slices, the RPC queue, pushbuffer pages, and any sysmem buffer the GPU reaches by
    /// **snooped** PCIe DMA (which is coherent on all the hardware in scope — the CPU
    /// cache is not stale, and no explicit flush is owed).
    ///
    /// **Who else can override it:** nobody in our own process. But the *same physical
    /// page* handed to KVM as a memslot acquires deciders 2 and 3, and there the guest
    /// can end up with a different type for the same bytes — **#111 exactly**: the page
    /// was write-back here, write-back-with-`IPAT` in Intel EPT, and `UC-` in the guest's
    /// own PTE, because the guest saw it through a *prefetchable PCI BAR* and x86
    /// `reserve_pfn_range()` silently downgrades a write-back request to `UC-` for any
    /// range `/proc/iomem` does not call System RAM. On Intel the EPT `IPAT` bit hid it;
    /// on AMD NPT, which honours the guest PTE, it did not.
    ///
    /// **Symptom of a mismatch:** a bandwidth cliff of one to two orders of magnitude
    /// with *correct results* — the C measured 0.22 GB/s write / 0.12 GB/s read against
    /// 14 GB/s. Nothing errors, nothing logs, and CUDA still computes the right answer.
    /// If a data path is inexplicably ~50–100× slow and byte-exact, look here first.
    ///
    /// **Coherence:** on x86, PCIe DMA snoops, so a write-back mapping of a page the GPU
    /// also touches is coherent with no explicit flush. **That is an x86 statement.** On
    /// arm64 it holds only on an IO-coherent chipset; NVIDIA's driver carries an explicit
    /// `flush_cache_all()` path for the rest, and it is compiled for aarch64 only.
    WriteBack,

    /// **Write-combining** — uncached, but stores coalesce in a write-combining buffer.
    /// x86 PAT entry `WC`; arm64 `MT_NORMAL_NC` (still *Normal* memory, so unaligned
    /// access is legal — unlike [`CachePolicy::Uncached`] there).
    ///
    /// The policy for the **framebuffer / BAR1 aperture** — and, per ground truth, for
    /// that *only*. NVIDIA's own `nvidia_mmap_helper` picks it for the FB window and
    /// **not** for the doorbell/USERD sub-range, which it maps uncached; see
    /// [`CachePolicy::Uncached`]. Reads from a write-combining mapping are as slow as
    /// uncached, so it is a **write-only** attribute in practice.
    ///
    /// **Who else can override it:** not attainable from `mmap` at all — only a driver's
    /// own `mmap` handler can install it, so in practice **NVIDIA RM does**, from the
    /// `NV_MEMORY_WRITECOMBINED` attribute on the allocation. Requesting it over a
    /// page-cache backing is [`RawError::CachePolicyUnattainable`] here.
    ///
    /// **Symptom of a mismatch, both directions:**
    /// - *asked write-combining, got write-back* — stores land in the cache and the GPU
    ///   reads stale bytes until an eviction that may never come. Presents as a hang or
    ///   as the GPU executing an old pushbuffer, intermittently.
    /// - *asked write-back, got write-combining* — every **read** becomes an uncached
    ///   round trip (the C's decode path lost ~14× this way), and stores become
    ///   **unordered**: the doorbell store can become visible before the pushbuffer bytes
    ///   it announces. That second half is a correctness bug, not a slowdown, and it is
    ///   why a write-combining region owes a **release fence before the doorbell store**
    ///   — an explicit act at that seam, never smeared into every access
    ///   (`crate::VolatileRegion`).
    WriteCombining,

    /// **Uncached** — no caching and no store combining.
    ///
    /// The policy NVIDIA's `mmap` handler picks for **BAR0 registers**, for the
    /// **USERD / doorbell** sub-range of BAR1, and for peer-IO — i.e. for everything
    /// where the access *is* the transaction. It is the variant most likely to be got
    /// wrong by intuition, because "doorbell" sounds like a write-combining case and is
    /// not: RM allocates USERD with a write-combining *object* attribute and then maps it
    /// to userspace **uncached**, and the mapping is the one that governs the CPU.
    ///
    /// **★ Two things "uncached" does not mean, both of which have cost time:**
    /// - On x86 Linux, `pgprot_noncached` is **`UC-`, not strict `UC`** — the weak form,
    ///   which *combines with MTRRs*: `UC-` over a range with a write-combining MTRR is
    ///   effectively write-combining, and only uncached without one. NVIDIA relies on
    ///   this deliberately as its fallback when write-combining is unavailable. So an
    ///   x86 "uncached" mapping's real behaviour depends on state this API cannot see.
    /// - On arm64 the two spellings are **different kinds of memory**: sysmem-uncached is
    ///   `MT_NORMAL_NC` (Normal, non-cacheable — unaligned access and speculation still
    ///   legal), while device-uncached is `Device-nGnRE`, where **unaligned and
    ///   multi-register accesses are architecturally invalid**. A bulk copy that is
    ///   merely slow on x86 can *fault* there. NVIDIA keeps the distinction explicitly
    ///   (`NV_PGPROT_UNCACHED` vs `NV_PGPROT_UNCACHED_DEVICE`) and warns in-source that
    ///   the kernel's `pgprot_noncached` is the device one.
    ///
    /// **Who else can override it:** as write-combining — a driver's `mmap` handler, from
    /// `NV_MEMORY_UNCACHED`. Not attainable over page-cache memory.
    ///
    /// **Symptom of a mismatch:** *asked uncached, got write-back* is the classic
    /// register-polling hang — the first read fills a cache line and every subsequent
    /// poll is served from it, so a status bit the device set long ago is never observed.
    /// The loop spins forever on a value that is correct-as-of-once. It looks exactly
    /// like a missing interrupt, and that is where the time gets spent.
    Uncached,
}

impl CachePolicy {
    /// The attribute's short name, as the kernel and the NVIDIA driver spell it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            CachePolicy::WriteBack => "write-back (WB)",
            CachePolicy::WriteCombining => "write-combining (WC)",
            CachePolicy::Uncached => "uncached (UC)",
        }
    }
}

impl fmt::Display for CachePolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Refuse a mapping whose backing provably cannot have the requested attribute.
///
/// The *entire* enforcement this layer has. Pure, so it is unit-tested without a syscall,
/// and called by the mapping constructors **before** `mmap` — a refusal must not leave a
/// mapping behind.
///
/// # Errors
/// [`RawError::CachePolicyUnattainable`] when `requested != attainable`.
/// Refuse a cache policy the backing provably cannot have.
///
/// ★★ `attainable = None` means **this layer cannot adjudicate** — the driver's own `mmap`
/// handler decided, from an attribute fixed at allocation time, and userspace cannot read
/// it back (`crate::Backing::attainable_cache_policy`, decider 4 in this module's table).
/// It does **not** mean "anything goes": the caller's requirement is still mandatory and
/// still travels with the region; what is absent is a mechanism, and pretending otherwise
/// is precisely the "comforting wrapper" this module's docs refuse to be.
///
/// The distinction is worth the `Option`. Collapsing `None` into "attainable == requested"
/// would make every device mapping *pass*, which reads identically to a checked one in
/// every log and every review.
pub(crate) fn require_attainable(
    requested: CachePolicy,
    attainable: Option<CachePolicy>,
    backing: &'static str,
) -> Result<(), RawError> {
    match attainable {
        None => Ok(()),
        Some(attainable) if requested == attainable => Ok(()),
        Some(attainable) => Err(RawError::CachePolicyUnattainable {
            requested,
            attainable,
            backing,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_attainable_policy_is_accepted_and_every_other_one_is_refused() {
        assert_eq!(
            require_attainable(
                CachePolicy::WriteBack,
                Some(CachePolicy::WriteBack),
                "anonymous memory"
            ),
            Ok(())
        );
        for requested in [CachePolicy::WriteCombining, CachePolicy::Uncached] {
            assert_eq!(
                require_attainable(requested, Some(CachePolicy::WriteBack), "anonymous memory"),
                Err(RawError::CachePolicyUnattainable {
                    requested,
                    attainable: CachePolicy::WriteBack,
                    backing: "anonymous memory"
                }),
                "{requested} over page-cache memory must be a refusal, not a silent \
                 write-back mapping"
            );
        }
    }

    /// The refusal message has to be readable at 2 a.m. by someone who does not yet know
    /// that `mmap` has no cacheability flag, so it says what they will actually get.
    #[test]
    fn the_refusal_says_what_the_caller_would_have_silently_got() {
        let e = require_attainable(
            CachePolicy::WriteCombining,
            Some(CachePolicy::WriteBack),
            "a shared file",
        )
        .unwrap_err();
        assert_eq!(
            e.to_string(),
            "a shared file is always write-back (WB); a mapping of it cannot be \
             write-combining (WC), and asking silently yields write-back (WB)"
        );
    }

    /// ★★★ **The non-vacuity of `None`.** An unadjudicable backing accepts EVERY policy —
    /// that is the point — so this test's job is to make the difference from the checked
    /// case visible, because in a log they look identical.
    ///
    /// The pair below is the whole argument: `WriteCombining` over a page-cache backing is
    /// a REFUSAL, and the same request over a device backing is an ACCEPTANCE, and neither
    /// tells you anything about what the pages actually are. If this test ever passes with
    /// `Some(_)` substituted for `None`, the `Option` has stopped meaning anything.
    #[test]
    fn an_unadjudicable_backing_accepts_every_policy_and_a_known_one_still_refuses() {
        for requested in [
            CachePolicy::WriteBack,
            CachePolicy::WriteCombining,
            CachePolicy::Uncached,
        ] {
            assert_eq!(
                require_attainable(requested, None, "a device file"),
                Ok(()),
                "{requested} over a driver-decided mapping cannot be adjudicated here"
            );
            // …and the SAME request over a backing this layer does know is still judged.
            let known =
                require_attainable(requested, Some(CachePolicy::WriteBack), "a shared file");
            assert_eq!(
                known.is_ok(),
                requested == CachePolicy::WriteBack,
                "the check must still discriminate where it CAN"
            );
        }
    }
}
