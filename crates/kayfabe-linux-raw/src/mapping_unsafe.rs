//! Host mappings and the bounded objects over them — `l1_os_shell.md` §4.2, §4.3, §4.4.
//!
//! This is one of the two files in the workspace that may use the dangerous keyword, and
//! it holds **six** blocks: two `mmap`s, one `munmap`, two `copy_nonoverlapping`s and one
//! atomic-view cast. Every one of them establishes its own invariant in its own function,
//! on a line above the block (the crate's house rule) — none has a precondition a caller
//! is trusted to have met.
//!
//! The one invariant that cannot be re-derived per call is [`Mapping`]'s: *its base is
//! valid for its length*. It is documented on that type and established by its **two**
//! constructors, both of which are `mmap` return values in this file, so the audit for it
//! is "read this file", not "audit the crate graph".
//!
//! ## Why the three region types are three types
//!
//! §4.2 distinguishes them by **who else writes the memory**, because that is what decides
//! what access is sound:
//!
//! | type | other writer | access surface |
//! |---|---|---|
//! | [`MappedRegion`] | the **guest**, concurrently | `read_into` / `write_from` — copies only |
//! | [`VolatileRegion`] | **hardware** (semaphores, USERD, fences) | aligned `Relaxed` atomic loads/stores, ≤ 8 bytes |
//! | [`Reservation`] | nobody (`PROT_NONE`) | placement of a `MappedRegion` *inside* it, by offset |

use crate::bounds::{self, HostOffset};
use crate::cache::{self, CachePolicy};
use crate::error::{RawError, last_syscall_error};
use crate::geometry;
use crate::page_size::HostPageSize;
use crate::view::RegionView;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use kayfabe_util::lockwitness;
use std::os::fd::{AsRawFd, BorrowedFd};

// =====================================================================================
// Vocabulary
// =====================================================================================

/// Protection for a **host** mapping.
///
/// Deliberately distinct from `kayfabe_vmm::Prot`, which says what the *guest* may do with
/// a guest-physical mapping. Same two words, different subject; collapsing them is how
/// §11 item 3 ("pages an isolate only reads are mapped read-only **in the isolate**")
/// turns into a mapping that is read-only for the wrong party.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HostProt {
    /// `PROT_READ`.
    ReadOnly,
    /// `PROT_READ | PROT_WRITE`.
    ReadWrite,
}

impl HostProt {
    fn bits(self) -> libc::c_int {
        match self {
            HostProt::ReadOnly => libc::PROT_READ,
            HostProt::ReadWrite => libc::PROT_READ | libc::PROT_WRITE,
        }
    }
}

/// What a mapping is backed by.
///
/// ★ **The backing, not the region type, is what decides cacheability** (see
/// [`crate::cache`]). Both variants below are ordinary kernel pages, so both are
/// **write-back and nothing else**, and [`Backing::attainable_cache_policy`] says so.
/// That is why [`CachePolicy`] is a parameter of the *mapping* rather than a property of
/// `MappedRegion` vs `VolatileRegion`: a semaphore page in guest RAM and a doorbell page
/// in a BAR are both `VolatileRegion`s and need different attributes, while everything
/// backed by a `memfd` needs the same one regardless of which region type wraps it.
#[derive(Debug, Clone, Copy)]
pub enum Backing<'fd> {
    /// Private anonymous zero-fill.
    ///
    /// **Not shareable with another process**, and that is the point of naming it: §4.4.1's
    /// unstated precondition is that guest RAM can only be double-mapped into an isolate if
    /// the VM was *launched* with a shareable memory backing (`--memory shared=on`,
    /// `memory-backend-memfd,share=on`). With a private backing there is nothing to export,
    /// and any handle invented for it produces copy-on-write pages — an isolate writing a
    /// completion the guest never sees. No code gate can observe how the operator started
    /// the VM; the only available mechanism is a loud refusal at the first export, and this
    /// variant is where the distinction is written down.
    ///
    /// Use: scratch and this crate's own tests.
    PrivateAnonymous,
    /// A `MAP_SHARED` file mapping — a `memfd`, or a hypervisor's `memory-backend-file`.
    /// The caller owns the descriptor; the mapping outlives it (Linux keeps the backing
    /// alive), so no lifetime is retained past the call.
    SharedFile {
        /// The descriptor to map. Borrowed: this crate never closes a caller's fd.
        fd: BorrowedFd<'fd>,
        /// Byte offset **into the file**, which must be host-page-aligned. Not a
        /// [`HostOffset`] — that type means "offset within a bounded object", and a file
        /// offset is neither bounded by us nor an offset into anything we mapped.
        offset: u64,
    },
}

impl Backing<'_> {
    /// ★ The **one** cache policy a mapping of this backing can actually have.
    ///
    /// Both of today's variants are ordinary kernel pages — the anonymous page allocator
    /// and the page cache — and Linux maps those write-back with no way to ask for
    /// anything else: `mmap` has no cacheability flag, and `PROT_*`/`MAP_*` do not carry
    /// one. So this is a fact about the pages, reported rather than chosen.
    ///
    /// **The variant that would return something else does not exist yet**, and its
    /// absence is the honest statement: write-combining and uncached are reachable only
    /// through a *driver's* `mmap` handler (for us, `/dev/nvidia*`, where RM's
    /// `nv_encode_caching` acts on the `NV_MEMORY_*` attribute fixed at allocation time).
    /// A `Backing::DeviceFile` arrives with the isolate's RM mappings (M2-d); when it
    /// does, it is this method that stops returning a single answer, and every call site
    /// already names what it requires.
    #[must_use]
    pub const fn attainable_cache_policy(self) -> CachePolicy {
        match self {
            // The anonymous page allocator hands out ordinary write-back RAM.
            Backing::PrivateAnonymous => CachePolicy::WriteBack,
            // A `memfd`/`tmpfs`/regular-file mapping is the page cache: also write-back.
            Backing::SharedFile { .. } => CachePolicy::WriteBack,
        }
    }

    /// How this backing is named in a [`RawError::CachePolicyUnattainable`] message.
    const fn describe(self) -> &'static str {
        match self {
            Backing::PrivateAnonymous => "anonymous memory",
            Backing::SharedFile { .. } => "a shared file",
        }
    }
}

/// A placement inside a [`Reservation`], as returned by [`Reservation::map_fixed_in`].
///
/// An index, not an address (§4.2.1 refusal 3). It is scoped to the reservation that
/// minted it; presenting it to another one is [`RawError::UnknownPlacement`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlacementId(u64);

// =====================================================================================
// The private owner of the pointer
// =====================================================================================

/// Who is responsible for tearing a mapping down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Disposition {
    /// This mapping owns its address range and `munmap`s it on drop.
    OwnedMunmap,
    /// This mapping lives **inside** a [`Reservation`], which unmaps the whole range in
    /// one call. Dropping it must therefore do nothing — a `munmap` here would punch a
    /// hole in a range the reservation still believes it owns, and the next placement
    /// into that hole would succeed against stale accounting.
    InsideReservation,
}

/// The one place a host CPU address exists in this workspace.
///
/// **Type invariant:** `base` is the start of a live mapping of exactly `len` bytes,
/// established by the two `mmap` call sites in this file and maintained by the fact that
/// nothing else constructs a `Mapping` and nothing ever mutates `base` or `len`. Every
/// `// SAFETY:` comment below cites it by name.
///
/// It is private, it has no accessor that yields the pointer, and `Mapping` is neither
/// `Send` nor `Sync` (a `NonNull` field makes it neither by default) — so no region type
/// is either, which is `tests/ui/region_is_not_send.rs`. Adding `Send` later is a reviewed
/// relaxation in this file, visible to the ratchet.
#[derive(Debug)]
struct Mapping {
    base: NonNull<u8>,
    len: usize,
    disposition: Disposition,
}

impl Mapping {
    /// The mapping's length as a `u64`, for the bounds arithmetic.
    fn len_bytes(&self) -> u64 {
        self.len as u64
    }

    /// Take ownership of an address the kernel just returned. Safe — *storing* a pointer
    /// is a safe operation, which is exactly §4.2.1's point — but private, with two call
    /// sites, both `mmap` returns validated in the same function.
    fn adopt(base: NonNull<u8>, len: usize, disposition: Disposition) -> Mapping {
        Mapping {
            base,
            len,
            disposition,
        }
    }

    /// `mmap` at a **kernel-chosen** address.
    ///
    /// Requesting no address is a security property, not a convenience: a mapping that
    /// never names a target cannot displace one that already exists, so the entire
    /// `MAP_FIXED` clobber class is unreachable from here by construction. (It is also why
    /// `MAP_FIXED_NOREPLACE` appears nowhere in this crate — see
    /// [`Reservation::map_fixed_in`].)
    fn anywhere(
        len: u64,
        prot: libc::c_int,
        extra_flags: libc::c_int,
        backing: Backing<'_>,
        page: HostPageSize,
        what: &'static str,
    ) -> Result<Mapping, RawError> {
        lockwitness::assert_lock_free("mmap (creating a host mapping)");

        if len == 0 {
            return Err(RawError::ZeroLength { what });
        }
        geometry::require_aligned(len, page, what)?;
        let len_host =
            usize::try_from(len).map_err(|_| RawError::TooLargeForHost { value: len })?;
        let (fd, file_offset, share_flags) = decode_backing(backing, page)?;

        // SAFETY: `mmap` with a NULL address requests a kernel-chosen placement, so this
        // call cannot displace any existing mapping in this process — the clobber hazard
        // `MAP_FIXED` carries does not exist on this path. It dereferences no caller
        // memory: `len_host` is a checked `usize` conversion of a non-zero, page-aligned
        // length (three lines above), `prot`/`flags` are constants assembled here, and
        // `fd`/`file_offset` come from `decode_backing`, which has already refused a
        // misaligned or unrepresentable file offset. The return value is validated for
        // `MAP_FAILED` immediately below, so the `Mapping` type invariant ("base is a live
        // mapping of exactly `len` bytes") holds for the value that is adopted.
        let ret = unsafe {
            libc::mmap(
                core::ptr::null_mut(),
                len_host,
                prot,
                share_flags | extra_flags,
                fd,
                file_offset,
            )
        };
        if ret == libc::MAP_FAILED {
            return Err(last_syscall_error("mmap"));
        }
        let base = NonNull::new(ret.cast::<u8>()).ok_or(RawError::Syscall {
            call: "mmap",
            errno: None,
        })?;
        Ok(Mapping::adopt(base, len_host, Disposition::OwnedMunmap))
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        if self.disposition == Disposition::InsideReservation {
            return;
        }

        // SAFETY: the `Mapping` type invariant says `base` is a live mapping of exactly
        // `len` bytes, and this is the unique owner of it — `Disposition::OwnedMunmap` is
        // set only by `Mapping::anywhere`, `Mapping` is not `Clone`/`Copy`, and no
        // accessor yields the pointer, so no second `munmap` of this range can exist and
        // no reference into it can outlive this drop (every accessor borrows `&self`).
        // Both arguments are exactly the pair the kernel returned.
        let rc = unsafe { libc::munmap(self.base.as_ptr().cast::<libc::c_void>(), self.len) };

        // ★ R1 is asserted AFTER the call, deliberately (§4.5). Asserting first would
        // panic out of a `Drop` before the resource was released — trading a rule
        // violation for a real leak — and this is the one syscall site that can be reached
        // by an ordinary scope exit under a lock. The syscall has happened either way;
        // what the assert is for is making it loud.
        if !std::thread::panicking() {
            assert!(rc == 0, "munmap failed: {}", last_syscall_error("munmap"));
            lockwitness::assert_lock_free("munmap (dropping a host mapping)");
        }
    }
}

/// Turn a [`Backing`] into the `(fd, offset, flags)` triple `mmap` wants, refusing a
/// misaligned or unrepresentable file offset **before** any call is made.
fn decode_backing(
    backing: Backing<'_>,
    page: HostPageSize,
) -> Result<(libc::c_int, libc::off_t, libc::c_int), RawError> {
    match backing {
        Backing::PrivateAnonymous => Ok((-1, 0, libc::MAP_PRIVATE | libc::MAP_ANONYMOUS)),
        Backing::SharedFile { fd, offset } => {
            geometry::require_aligned(offset, page, "file offset")?;
            let off = libc::off_t::try_from(offset)
                .map_err(|_| RawError::TooLargeForHost { value: offset })?;
            Ok((fd.as_raw_fd(), off, libc::MAP_SHARED))
        }
    }
}

// =====================================================================================
// MappedRegion — the guest-concurrent one, copies only
// =====================================================================================

/// A host mapping the **guest** may be writing while we look at it.
///
/// The access surface is deliberately *copies only*: [`read_into`] and [`write_from`], no
/// `as_slice`, no `as_ptr`, no `Deref`. A borrow into guest-writable memory invites the
/// double-fetch — validate a length field, then re-read it — and a copy makes the parser's
/// input memory the guest cannot touch. See the crate docs.
///
/// [`read_into`]: MappedRegion::read_into
/// [`write_from`]: MappedRegion::write_from
///
/// ## ★ The residual, named rather than papered over
///
/// A bulk copy out of memory another agent is concurrently writing is, in the letter of the
/// Rust abstract machine, a data race — §4.3's own argument (atomics are defined under
/// concurrent modification; volatile and plain accesses are not) applies to this copy as
/// much as to a semaphore load, and it is not a race an ordinary `memcpy` can be rid of.
/// What makes it *tolerable here specifically* is that the copy is performed **once** and
/// the source is never re-read: the miscompilation such a race actually produces is a
/// re-read the optimiser inserts, which is the double-fetch this API exists to prevent, so
/// the discipline and the hazard cancel. It is recorded as a residual rather than claimed
/// as sound, and the alternative (word-wise `Relaxed` atomic copies, which *are* defined
/// and are several times slower) is a measurement's decision, not this stage's.
#[derive(Debug)]
pub struct MappedRegion {
    map: Mapping,
    prot: HostProt,
    cache: CachePolicy,
}

impl MappedRegion {
    /// Map `len` bytes of `backing`, requiring cache policy `cache`.
    ///
    /// `len` must be non-zero and a whole number of **host** pages — stricter than the
    /// kernel, which rounds silently. Silent rounding is how a region ends up longer than
    /// the geometry that produced it believes, which is exactly the "semantically unbounded
    /// bounded object" the crate docs name as the un-mechanisable refusal.
    ///
    /// ★ `cache` is **required and never defaulted**, and it is a *requirement*, not an
    /// instruction: `mmap` cannot set a cache attribute, so all this call can do is refuse
    /// when the backing provably cannot have the one asked for. Read [`crate::cache`] once
    /// before choosing a value — the three things this layer cannot enforce are named
    /// there, with their owners.
    ///
    /// # Errors
    /// [`RawError::CachePolicyUnattainable`] — refused **before** the syscall, so a
    /// refusal never leaves a mapping behind. Otherwise [`RawError::ZeroLength`],
    /// [`RawError::Misaligned`] (length, or a file offset), [`RawError::TooLargeForHost`],
    /// [`RawError::Syscall`].
    ///
    /// # Panics
    /// If called with any ranked lock held (R1, §4.5).
    pub fn map(
        backing: Backing<'_>,
        len: u64,
        prot: HostProt,
        cache: CachePolicy,
        page: HostPageSize,
    ) -> Result<Self, RawError> {
        cache::require_attainable(cache, backing.attainable_cache_policy(), backing.describe())?;
        let map = Mapping::anywhere(len, prot.bits(), 0, backing, page, "mapping length")?;
        Ok(MappedRegion { map, prot, cache })
    }

    /// The cache policy this region was mapped under.
    ///
    /// Safe to return and safe to log — it is an attribute, not an address. It exists so a
    /// caller assembling a device-visible mapping can assert the two halves agree, and so
    /// a bug report can say which attribute was *intended*, which is the fact that is
    /// otherwise unrecoverable after the fact.
    #[must_use]
    pub fn cache_policy(&self) -> CachePolicy {
        self.cache
    }

    /// The region's length in bytes. The bound every accessor checks against, and the only
    /// number about the mapping that ever leaves this crate.
    #[must_use]
    pub fn len_bytes(&self) -> u64 {
        self.map.len_bytes()
    }

    /// Copy `dst.len()` bytes from `offset` into `dst`.
    ///
    /// # Errors
    /// [`RawError::ZeroLength`], [`RawError::LengthOverflow`], [`RawError::OutOfRange`],
    /// [`RawError::TooLargeForHost`] — see [`bounds::checked_span`].
    pub fn read_into(&self, offset: HostOffset, dst: &mut [u8]) -> Result<(), RawError> {
        let (start, n) = bounds::checked_span(
            self.map.len_bytes(),
            offset,
            dst.len() as u64,
            "read length",
        )?;

        // SAFETY: source — the `Mapping` type invariant says `base` is live for `len`
        // bytes, and `checked_span` on the line above proved `start + n <= len` with the
        // overflow checked first, so `base.add(start)` is in bounds of that allocation and
        // readable for `n` bytes. Destination — `dst` is a `&mut [u8]` and `n == dst.len()`
        // by that same call, so it is writable for exactly `n` bytes. Non-overlap — no
        // borrow into this mapping is ever handed out (that is refusal 1, compile-fail
        // tested), so no caller can construct a `dst` that aliases it. Alignment — `u8`.
        // Concurrency — the guest may be writing the source; see the type's "residual".
        unsafe {
            core::ptr::copy_nonoverlapping(self.map.base.as_ptr().add(start), dst.as_mut_ptr(), n);
        }
        Ok(())
    }

    /// Copy `src` into the region at `offset`.
    ///
    /// # Errors
    /// [`RawError::NotWritable`] if the region was mapped [`HostProt::ReadOnly`] — refused
    /// here rather than delivered as a `SIGSEGV`, because a read-only isolate mapping
    /// (§11 item 3) is precisely a place a caller gets this wrong. Otherwise as
    /// [`MappedRegion::read_into`].
    pub fn write_from(&self, offset: HostOffset, src: &[u8]) -> Result<(), RawError> {
        if self.prot == HostProt::ReadOnly {
            return Err(RawError::NotWritable);
        }
        let (start, n) = bounds::checked_span(
            self.map.len_bytes(),
            offset,
            src.len() as u64,
            "write length",
        )?;

        // SAFETY: destination — the `Mapping` type invariant plus `checked_span` on the
        // line above (overflow checked before the bound) make `base.add(start)` in bounds
        // and writable for `n` bytes; the mapping carries `PROT_WRITE` because the
        // `ReadOnly` case returned three lines up. Source — `src` is a `&[u8]` with
        // `n == src.len()`. Non-overlap and alignment as in `read_into`.
        unsafe {
            core::ptr::copy_nonoverlapping(src.as_ptr(), self.map.base.as_ptr().add(start), n);
        }
        Ok(())
    }

    /// ★ **"The thing at offset X, N bytes long"** — the constructive half of the rule.
    ///
    /// Returns another bounded object, borrowed from this one, so composition re-checks
    /// rather than accumulating unchecked arithmetic. This is what a caller reaches for
    /// instead of a base address, and it is why the prohibition is workable: there is
    /// something to say.
    ///
    /// # Errors
    /// As [`bounds::checked_span`].
    pub fn slice(&self, offset: HostOffset, len: u64) -> Result<RegionView<'_>, RawError> {
        RegionView::new(self, offset, len)
    }

    /// Is this region writable? Used by [`RegionView`] so a view cannot smuggle a write
    /// past the read-only check.
    pub(crate) fn is_writable(&self) -> bool {
        self.prot == HostProt::ReadWrite
    }
}

// =====================================================================================
// VolatileRegion — the hardware-written one, aligned atomics only
// =====================================================================================

/// The widths a hardware-shared access may use. Sealed by being private: a caller cannot
/// name it, so [`VolatileRegion`] cannot be talked into a 16-byte or non-atomic access.
trait AtomicWord {}
impl AtomicWord for AtomicU32 {}
impl AtomicWord for AtomicU64 {}

/// A page **real hardware** writes — semaphore payloads, USERD, fences.
///
/// Accessed as `Relaxed` atomics, naturally aligned, ≤ 8 bytes, and *nothing else*
/// (§4.3). A plain or `volatile` read of memory a GPU is concurrently writing is not
/// defined to be non-tearing; a `Relaxed` atomic load is, and lowers to the same
/// instruction. There is no `load_u128` and no bulk accessor, which is the width refusal
/// made structural rather than documented.
///
/// **Ordering against other memory is a separate, explicit act** — a release fence before
/// ringing a doorbell, an acquire fence after observing a semaphore — named at the two
/// seams that need it rather than smeared into every access. Neither seam exists yet, so
/// neither fence is written yet.
#[derive(Debug)]
pub struct VolatileRegion {
    map: Mapping,
    cache: CachePolicy,
}

impl VolatileRegion {
    /// Map `len` bytes of `backing` as a hardware-shared region, requiring cache policy
    /// `cache`. Always read-write: a page whose *other* writer is a GPU is not usefully
    /// read-only to us.
    ///
    /// ★ **This type is exactly why cacheability is a parameter and not a property of the
    /// region type.** The pages a `VolatileRegion` wraps split across all three attributes:
    /// a semaphore or notifier in sysmem is [`CachePolicy::WriteBack`] and coherent by
    /// PCIe snoop on x86; the framebuffer/BAR1 aperture is [`CachePolicy::WriteCombining`];
    /// **USERD, the doorbell page and BAR0 registers are [`CachePolicy::Uncached`]** —
    /// which is the one people guess wrong, because "doorbell" sounds like a
    /// write-combining case and NVIDIA maps it uncached. "The GPU writes it" does not
    /// determine the attribute; *what the page is* does.
    ///
    /// ★★ **The write-combining seam that has no mechanism here:** under
    /// [`CachePolicy::WriteCombining`], stores are **not ordered against each other**. A
    /// doorbell store may become visible to the device before the pushbuffer bytes it
    /// announces, and the GPU then executes whatever was there before. The fix is a
    /// **release fence before the doorbell store** — a named, explicit act at that one
    /// seam (`sfence` on x86, `dsb st` on arm64: exactly NVIDIA's
    /// `os_flush_cpu_write_combine_buffer`). That seam does not exist yet, so the fence is
    /// not written yet; it is recorded here so it is not discovered against real silicon.
    ///
    /// # Errors
    /// As [`MappedRegion::map`].
    ///
    /// # Panics
    /// If called with any ranked lock held (R1, §4.5).
    pub fn map(
        backing: Backing<'_>,
        len: u64,
        cache: CachePolicy,
        page: HostPageSize,
    ) -> Result<Self, RawError> {
        cache::require_attainable(cache, backing.attainable_cache_policy(), backing.describe())?;
        let map = Mapping::anywhere(
            len,
            HostProt::ReadWrite.bits(),
            0,
            backing,
            page,
            "mapping length",
        )?;
        Ok(VolatileRegion { map, cache })
    }

    /// The cache policy this region was mapped under. As [`MappedRegion::cache_policy`].
    #[must_use]
    pub fn cache_policy(&self) -> CachePolicy {
        self.cache
    }

    /// The region's length in bytes.
    #[must_use]
    pub fn len_bytes(&self) -> u64 {
        self.map.len_bytes()
    }

    /// A naturally-aligned atomic view of the word at `offset`.
    ///
    /// The returned reference borrows `&self`, so it cannot outlive the mapping — that is
    /// trybuild row 5 (use-after-`munmap`) held by lifetimes rather than by care.
    fn word_at<T: AtomicWord>(&self, offset: HostOffset) -> Result<&T, RawError> {
        let size = size_of::<T>() as u64;
        let align = align_of::<T>() as u64;
        if !offset.get().is_multiple_of(align) {
            return Err(RawError::Misaligned {
                what: "volatile access offset",
                value: offset.get(),
                required: align,
            });
        }
        let (start, _n) =
            bounds::checked_span(self.map.len_bytes(), offset, size, "volatile access length")?;

        // SAFETY: the `Mapping` type invariant says `base` is live for `len` bytes, and
        // `checked_span` two lines above proved `start + size_of::<T>() <= len` (overflow
        // checked first), so the whole word is in bounds of that allocation. Alignment —
        // `mmap` returns a page-aligned base and `page >= 4096 > align_of::<T>()`, so the
        // absolute address is aligned exactly when `offset` is, which the check above
        // requires. Validity — `T` is `AtomicU32`/`AtomicU64` (the private `AtomicWord`
        // trait has no other impls); both are integers for which every bit pattern is a
        // value, so any bytes the GPU has written are a valid `T`. Aliasing — the returned
        // `&T` is an atomic type, so concurrent modification through it (by us) and outside
        // the abstract machine (by the GPU) is the case `Relaxed` atomics are defined for,
        // and the borrow of `&self` stops it outliving the mapping.
        unsafe { Ok(&*self.map.base.as_ptr().add(start).cast::<T>()) }
    }

    /// Load the 32-bit word at `offset` (`Relaxed`).
    ///
    /// # Errors
    /// [`RawError::Misaligned`] unless `offset % 4 == 0`; otherwise as
    /// [`bounds::checked_span`].
    pub fn load_u32(&self, offset: HostOffset) -> Result<u32, RawError> {
        Ok(self.word_at::<AtomicU32>(offset)?.load(Ordering::Relaxed))
    }

    /// Store `value` at `offset` (`Relaxed`).
    ///
    /// `&self`, not `&mut self`: the memory has another writer regardless of what we hold,
    /// so exclusivity would be a lie the type told.
    ///
    /// # Errors
    /// As [`VolatileRegion::load_u32`].
    pub fn store_u32(&self, offset: HostOffset, value: u32) -> Result<(), RawError> {
        self.word_at::<AtomicU32>(offset)?
            .store(value, Ordering::Relaxed);
        Ok(())
    }

    /// Load the 64-bit word at `offset` (`Relaxed`).
    ///
    /// # Errors
    /// [`RawError::Misaligned`] unless `offset % 8 == 0`; otherwise as
    /// [`bounds::checked_span`].
    pub fn load_u64(&self, offset: HostOffset) -> Result<u64, RawError> {
        Ok(self.word_at::<AtomicU64>(offset)?.load(Ordering::Relaxed))
    }

    /// Store `value` at `offset` (`Relaxed`).
    ///
    /// # Errors
    /// As [`VolatileRegion::load_u64`].
    pub fn store_u64(&self, offset: HostOffset, value: u64) -> Result<(), RawError> {
        self.word_at::<AtomicU64>(offset)?
            .store(value, Ordering::Relaxed);
        Ok(())
    }
}

// =====================================================================================
// Reservation — own the address space before you place anything in it
// =====================================================================================

/// One mapping placed inside a reservation.
#[derive(Debug)]
struct Placement {
    offset: u64,
    len: u64,
    region: MappedRegion,
}

/// ★ A `PROT_NONE` address-space reservation — §4.4's answer to the breakout surface.
///
/// `MAP_FIXED` silently unmaps whatever is already at the target address, and in a process
/// that also contains the VMM, "whatever is already there" can be its heap, our stack or
/// our own text. The double-mmap of guest RAM into an isolate is born from this call, which
/// makes it the single most dangerous call in the codebase.
///
/// The shape is chosen so that violation does not typecheck: **to place a mapping you must
/// first own the address space**, and the placement names an *offset within what you own*,
/// never an address. There is no `map_fixed(addr, …)` free function to reach for
/// (`tests/ui/map_fixed_bare_address.rs`).
///
/// ## ★★ A finding: `MAP_FIXED_NOREPLACE` and reserving first are mutually exclusive
///
/// §4.4 asks for both — *"the underlying call uses `MAP_FIXED_NOREPLACE` and treats a
/// collision as a loud fault, so even a reservation-accounting bug fails loudly rather than
/// clobbering"*. **They cannot both be true.** `MAP_FIXED_NOREPLACE` fails with `EEXIST` if
/// *anything* is mapped in the target range — and in the reserve-first design the
/// reservation itself is always mapped there, so every placement would fail. Punching a
/// hole with `munmap` first and then racing to fill it is strictly worse: it opens a window
/// in which another thread's allocator can take the range, in a process we share with the
/// VMM.
///
/// So the belt-and-braces is *relocated*, not abandoned, and it ends up stronger:
///
/// - the **reservation** is acquired at a kernel-chosen address ([`Mapping::anywhere`],
///   no address requested), which cannot displace an existing mapping at all — a
///   guarantee `MAP_FIXED_NOREPLACE` merely detects;
/// - the **placement** uses `MAP_FIXED` inside a range this process demonstrably owns, and
///   the accounting that `MAP_FIXED_NOREPLACE` was meant to backstop is a refusal here:
///   overlapping an existing placement is [`RawError::OverlappingPlacement`], loudly.
///
/// Consequence worth stating: `MAP_FIXED_NOREPLACE` appears nowhere in this crate, and its
/// absence is deliberate rather than an omission.
#[derive(Debug)]
pub struct Reservation {
    /// Declared **before** `map`, so placements drop first. They are `InsideReservation`
    /// and drop to nothing, but the order is the one that would still be correct if that
    /// ever changed.
    placements: Vec<Placement>,
    map: Mapping,
    page: HostPageSize,
}

impl Reservation {
    /// Reserve `len` bytes of address space, `PROT_NONE`, at a kernel-chosen address.
    ///
    /// `MAP_NORESERVE`: the reservation is address space, not memory — committing swap for
    /// a hole that exists to be overwritten is the opposite of what it is for.
    ///
    /// # Errors
    /// [`RawError::ZeroLength`], [`RawError::Misaligned`] (length not a whole number of
    /// host pages), [`RawError::TooLargeForHost`], [`RawError::Syscall`].
    ///
    /// # Panics
    /// If called with any ranked lock held (R1, §4.5).
    pub fn new(len: u64, page: HostPageSize) -> Result<Self, RawError> {
        let map = Mapping::anywhere(
            len,
            libc::PROT_NONE,
            libc::MAP_NORESERVE,
            Backing::PrivateAnonymous,
            page,
            "reservation length",
        )?;
        Ok(Reservation {
            placements: Vec::new(),
            map,
            page,
        })
    }

    /// The reservation's length in bytes.
    #[must_use]
    pub fn len_bytes(&self) -> u64 {
        self.map.len_bytes()
    }

    /// ★ Place a mapping **at an offset within this reservation**. The address-taking
    /// variant this replaces does not exist.
    ///
    /// # Errors
    /// - [`RawError::ZeroLength`] / [`RawError::Misaligned`] — length or offset is not a
    ///   whole number of host pages.
    /// - [`RawError::OutOfRange`] / [`RawError::LengthOverflow`] — the placement is not
    ///   inside the reservation.
    /// - [`RawError::OverlappingPlacement`] — it collides with one already made. Naming
    ///   the collision is what replaces `MAP_FIXED_NOREPLACE`; see the type docs.
    /// - [`RawError::CachePolicyUnattainable`] — as [`MappedRegion::map`]. A placement is
    ///   a mapping, so it names its requirement like any other; the **reservation** does
    ///   not, because `PROT_NONE` address space is never accessed and so has no attribute
    ///   to get wrong.
    /// - [`RawError::Syscall`] — `mmap` refused.
    ///
    /// # Panics
    /// If called with any ranked lock held (R1, §4.5).
    pub fn map_fixed_in(
        &mut self,
        offset: HostOffset,
        len: u64,
        backing: Backing<'_>,
        prot: HostProt,
        cache: CachePolicy,
    ) -> Result<PlacementId, RawError> {
        lockwitness::assert_lock_free("mmap MAP_FIXED (placing inside a reservation)");

        cache::require_attainable(cache, backing.attainable_cache_policy(), backing.describe())?;
        if len == 0 {
            return Err(RawError::ZeroLength {
                what: "placement length",
            });
        }
        geometry::require_aligned(offset.get(), self.page, "placement offset")?;
        geometry::require_aligned(len, self.page, "placement length")?;
        let (start, len_host) =
            bounds::checked_span(self.map.len_bytes(), offset, len, "placement length")?;
        for p in &self.placements {
            if offset.get() < p.offset + p.len && p.offset < offset.get() + len {
                return Err(RawError::OverlappingPlacement {
                    offset: offset.get(),
                    len,
                    existing_offset: p.offset,
                    existing_len: p.len,
                });
            }
        }
        let (fd, file_offset, share_flags) = decode_backing(backing, self.page)?;

        // SAFETY: this is the `MAP_FIXED` call, and its whole argument is that the target
        // range is one this process already owns and is still holding:
        //  (a) `self.map` was created by `Mapping::anywhere`, i.e. at a KERNEL-CHOSEN
        //      address, so it displaced nothing when it was made, and its type invariant
        //      says it is live for `self.map.len` bytes right now;
        //  (b) `checked_span` above proved `start + len_host <= self.map.len` with the
        //      overflow checked before the bound, so `base.add(start)` is in bounds of that
        //      allocation (the precondition of `add`) and the whole placement lies inside
        //      the reservation — `MAP_FIXED` can therefore replace only our own PROT_NONE
        //      filler, never the VMM's heap, our stack or our text;
        //  (c) the loop above refused any overlap with an existing placement, so it cannot
        //      replace a mapping some live `MappedRegion` still refers to;
        //  (d) `offset` and `len` are host-page-aligned (checked above) and `file_offset`
        //      was validated by `decode_backing`, so `mmap` has no rounding to do.
        // The result is compared against the requested address below, so a kernel that
        // honoured the request differently cannot be adopted, and the mapping is adopted
        // `InsideReservation` so it will not `munmap` a hole in the range on drop.
        let ret = unsafe {
            let target = self.map.base.as_ptr().add(start).cast::<libc::c_void>();
            libc::mmap(
                target,
                len_host,
                prot.bits(),
                share_flags | libc::MAP_FIXED,
                fd,
                file_offset,
            )
        };
        if ret == libc::MAP_FAILED {
            return Err(last_syscall_error("mmap"));
        }
        let base = NonNull::new(ret.cast::<u8>()).ok_or(RawError::Syscall {
            call: "mmap",
            errno: None,
        })?;
        let id = PlacementId(self.placements.len() as u64);
        self.placements.push(Placement {
            offset: offset.get(),
            len,
            region: MappedRegion {
                map: Mapping::adopt(base, len_host, Disposition::InsideReservation),
                prot,
                cache,
            },
        });
        Ok(id)
    }

    /// The region a [`PlacementId`] names. Borrowed from the reservation, so a placed
    /// region cannot outlive the address space it sits in.
    ///
    /// # Errors
    /// [`RawError::UnknownPlacement`] if this reservation did not mint the id.
    pub fn placement(&self, id: PlacementId) -> Result<&MappedRegion, RawError> {
        usize::try_from(id.0)
            .ok()
            .and_then(|i| self.placements.get(i))
            .map(|p| &p.region)
            .ok_or(RawError::UnknownPlacement { id: id.0 })
    }
}

/// ## ★ What the cacheability tests below prove — and what nothing here can
///
/// Stated at the top because the honest scope is small and a reader who assumes otherwise
/// is worse off than one who reads no tests at all.
///
/// **Proved:** that a request for an attribute the backing cannot have is a refusal by an
/// exact error variant, at every mapping door, before the syscall; and that a region
/// reports back the policy it was mapped under.
///
/// **Not proved, and not provable from userspace:** that the pages actually *are*
/// write-back. The x86 PAT index lives in the PTE, which no process can read about itself
/// — the C had to ship a throwaway kernel module (`tools/pteinfo`) to decode it, and a
/// second one (`tools/fixwb`) to change it. The only userspace-visible evidence is a
/// **bandwidth measurement** (the C's `pinned_write_bench`: 0.12 GB/s uncached vs 14 GB/s
/// write-back), which needs a quiet machine and a threshold, and belongs to the L3 bench
/// rather than to a unit test. A test asserting "this mapping is write-back" that could
/// only ever pass would be the green instrument on an unexercised path the testing
/// doctrine forbids, so it is not written.
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write as _;

    fn page() -> HostPageSize {
        HostPageSize::query()
    }

    /// A `MAP_SHARED`-capable backing without a `memfd` (which is M2-c): an ordinary file,
    /// unlinked immediately, so nothing survives the test.
    fn shared_file(len: u64) -> File {
        let path = std::env::temp_dir().join(format!(
            "kayfabe-linux-raw-{}-{:?}.bin",
            std::process::id(),
            std::thread::current().id()
        ));
        let mut f = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .expect("temp file");
        f.write_all(&vec![0_u8; usize::try_from(len).expect("len")])
            .expect("size the file");
        std::fs::remove_file(&path).expect("unlink");
        f
    }

    #[test]
    fn a_mapped_region_round_trips_through_copies_only() {
        let p = page();
        let r = MappedRegion::map(
            Backing::PrivateAnonymous,
            p.bytes(),
            HostProt::ReadWrite,
            CachePolicy::WriteBack,
            p,
        )
        .expect("map");
        assert_eq!(r.len_bytes(), p.bytes());

        r.write_from(HostOffset::new(8), b"kayfabe").expect("write");
        let mut out = [0_u8; 7];
        r.read_into(HostOffset::new(8), &mut out).expect("read");
        assert_eq!(&out, b"kayfabe");

        // Untouched bytes are the zero fill, i.e. the write went exactly where it said.
        let mut before = [0xff_u8; 8];
        r.read_into(HostOffset::ZERO, &mut before).expect("read");
        assert_eq!(before, [0; 8]);
    }

    #[test]
    fn a_read_past_the_end_is_out_of_range_by_that_exact_variant() {
        let p = page();
        let r = MappedRegion::map(
            Backing::PrivateAnonymous,
            p.bytes(),
            HostProt::ReadWrite,
            CachePolicy::WriteBack,
            p,
        )
        .expect("map");
        let mut out = [0_u8; 4];
        assert_eq!(
            r.read_into(HostOffset::new(p.bytes() - 3), &mut out),
            Err(RawError::OutOfRange {
                offset: p.bytes() - 3,
                len: 4,
                object_len: p.bytes()
            })
        );
        assert_eq!(
            r.read_into(HostOffset::new(u64::MAX), &mut out),
            Err(RawError::LengthOverflow {
                offset: u64::MAX,
                len: 4
            })
        );
        assert_eq!(
            r.read_into(HostOffset::ZERO, &mut []),
            Err(RawError::ZeroLength {
                what: "read length"
            })
        );
    }

    /// §11 item 3's counterpart: a write to a read-only mapping is a refusal, not a signal.
    #[test]
    fn writing_a_read_only_region_is_refused_before_the_store() {
        let p = page();
        let r = MappedRegion::map(
            Backing::PrivateAnonymous,
            p.bytes(),
            HostProt::ReadOnly,
            CachePolicy::WriteBack,
            p,
        )
        .expect("map");
        assert_eq!(
            r.write_from(HostOffset::ZERO, b"x"),
            Err(RawError::NotWritable)
        );
        let mut out = [0xff_u8; 1];
        r.read_into(HostOffset::ZERO, &mut out).expect("read");
        assert_eq!(out, [0], "the refused write must not have happened");
    }

    #[test]
    fn a_mapping_length_must_be_whole_host_pages_and_non_zero() {
        let p = page();
        assert_eq!(
            MappedRegion::map(
                Backing::PrivateAnonymous,
                0,
                HostProt::ReadWrite,
                CachePolicy::WriteBack,
                p
            )
            .unwrap_err(),
            RawError::ZeroLength {
                what: "mapping length"
            }
        );
        assert_eq!(
            MappedRegion::map(
                Backing::PrivateAnonymous,
                p.bytes() + 1,
                HostProt::ReadWrite,
                CachePolicy::WriteBack,
                p
            )
            .unwrap_err(),
            RawError::Misaligned {
                what: "mapping length",
                value: p.bytes() + 1,
                required: p.bytes()
            }
        );
    }

    /// ★ The double-mmap's premise, at its smallest: two independent mappings of one
    /// shared backing observe each other's writes. With `PrivateAnonymous` (§4.4.1's
    /// private-backing case) they would not, which is exactly the failure that variant's
    /// docs describe.
    #[test]
    fn two_mappings_of_one_shared_backing_see_each_others_writes() {
        let p = page();
        let f = shared_file(p.bytes());
        let fd = std::os::fd::AsFd::as_fd(&f);
        let a = MappedRegion::map(
            Backing::SharedFile { fd, offset: 0 },
            p.bytes(),
            HostProt::ReadWrite,
            CachePolicy::WriteBack,
            p,
        )
        .expect("map a");
        let b = MappedRegion::map(
            Backing::SharedFile { fd, offset: 0 },
            p.bytes(),
            HostProt::ReadWrite,
            CachePolicy::WriteBack,
            p,
        )
        .expect("map b");

        a.write_from(HostOffset::new(64), b"shared").expect("write");
        let mut out = [0_u8; 6];
        b.read_into(HostOffset::new(64), &mut out).expect("read");
        assert_eq!(&out, b"shared");
    }

    #[test]
    fn a_misaligned_file_offset_is_refused_before_the_syscall() {
        let p = page();
        let f = shared_file(p.bytes());
        let fd = std::os::fd::AsFd::as_fd(&f);
        assert_eq!(
            MappedRegion::map(
                Backing::SharedFile { fd, offset: 1 },
                p.bytes(),
                HostProt::ReadWrite,
                CachePolicy::WriteBack,
                p
            )
            .unwrap_err(),
            RawError::Misaligned {
                what: "file offset",
                value: 1,
                required: p.bytes()
            }
        );
    }

    #[test]
    fn a_view_composes_and_re_checks_at_every_step() {
        let p = page();
        let r = MappedRegion::map(
            Backing::PrivateAnonymous,
            p.bytes(),
            HostProt::ReadWrite,
            CachePolicy::WriteBack,
            p,
        )
        .expect("map");
        let v = r.slice(HostOffset::new(64), 32).expect("slice");
        assert_eq!(v.len_bytes(), 32);

        v.write_from(HostOffset::new(4), b"abcd").expect("write");
        let mut out = [0_u8; 4];
        r.read_into(HostOffset::new(68), &mut out).expect("read");
        assert_eq!(&out, b"abcd", "a view offset is relative to the view");

        // The view's own bound, not the region's: byte 32 of the view is inside the
        // region and outside the view.
        assert_eq!(
            v.read_into(HostOffset::new(32), &mut [0_u8; 1]),
            Err(RawError::OutOfRange {
                offset: 32,
                len: 1,
                object_len: 32
            })
        );
        assert_eq!(
            r.slice(HostOffset::new(p.bytes()), 1).unwrap_err(),
            RawError::OutOfRange {
                offset: p.bytes(),
                len: 1,
                object_len: p.bytes()
            }
        );
    }

    #[test]
    fn volatile_words_load_and_store_at_both_widths() {
        let p = page();
        let v = VolatileRegion::map(
            Backing::PrivateAnonymous,
            p.bytes(),
            CachePolicy::WriteBack,
            p,
        )
        .expect("map");
        assert_eq!(v.len_bytes(), p.bytes());

        v.store_u32(HostOffset::new(16), 0xdead_beef)
            .expect("store");
        assert_eq!(v.load_u32(HostOffset::new(16)), Ok(0xdead_beef));
        v.store_u64(HostOffset::new(24), 0x0123_4567_89ab_cdef)
            .expect("store");
        assert_eq!(v.load_u64(HostOffset::new(24)), Ok(0x0123_4567_89ab_cdef));
        assert_eq!(v.load_u32(HostOffset::ZERO), Ok(0));
    }

    /// The tearing refusals, by exact variant. There is no `load_u128` to test — the width
    /// bound is structural (`tests/ui/no_wide_volatile_load.rs`); the alignment bound is
    /// this.
    #[test]
    fn an_unaligned_or_out_of_range_volatile_access_is_refused() {
        let p = page();
        let v = VolatileRegion::map(
            Backing::PrivateAnonymous,
            p.bytes(),
            CachePolicy::WriteBack,
            p,
        )
        .expect("map");
        assert_eq!(
            v.load_u32(HostOffset::new(2)),
            Err(RawError::Misaligned {
                what: "volatile access offset",
                value: 2,
                required: 4
            })
        );
        assert_eq!(
            v.load_u64(HostOffset::new(4)),
            Err(RawError::Misaligned {
                what: "volatile access offset",
                value: 4,
                required: 8
            })
        );
        assert_eq!(
            v.load_u64(HostOffset::new(p.bytes())),
            Err(RawError::OutOfRange {
                offset: p.bytes(),
                len: 8,
                object_len: p.bytes()
            })
        );
    }

    #[test]
    fn a_reservation_places_disjoint_mappings_by_offset() {
        let p = page();
        let mut res = Reservation::new(p.bytes() * 4, p).expect("reserve");
        assert_eq!(res.len_bytes(), p.bytes() * 4);

        let a = res
            .map_fixed_in(
                HostOffset::ZERO,
                p.bytes(),
                Backing::PrivateAnonymous,
                HostProt::ReadWrite,
                CachePolicy::WriteBack,
            )
            .expect("place a");
        let b = res
            .map_fixed_in(
                HostOffset::new(p.bytes() * 2),
                p.bytes(),
                Backing::PrivateAnonymous,
                HostProt::ReadWrite,
                CachePolicy::WriteBack,
            )
            .expect("place b");
        assert_ne!(a, b);

        res.placement(a)
            .expect("a")
            .write_from(HostOffset::ZERO, b"first")
            .expect("write");
        res.placement(b)
            .expect("b")
            .write_from(HostOffset::ZERO, b"second")
            .expect("write");
        let mut out = [0_u8; 5];
        res.placement(a)
            .expect("a")
            .read_into(HostOffset::ZERO, &mut out)
            .expect("read");
        assert_eq!(&out, b"first", "the two placements are independent memory");
    }

    /// ★ The refusal that replaces `MAP_FIXED_NOREPLACE` — see [`Reservation`]'s docs.
    #[test]
    fn an_overlapping_placement_is_refused_by_that_exact_variant() {
        let p = page();
        let mut res = Reservation::new(p.bytes() * 4, p).expect("reserve");
        res.map_fixed_in(
            HostOffset::new(p.bytes()),
            p.bytes() * 2,
            Backing::PrivateAnonymous,
            HostProt::ReadWrite,
            CachePolicy::WriteBack,
        )
        .expect("place");
        assert_eq!(
            res.map_fixed_in(
                HostOffset::new(p.bytes() * 2),
                p.bytes(),
                Backing::PrivateAnonymous,
                HostProt::ReadWrite,
                CachePolicy::WriteBack
            )
            .unwrap_err(),
            RawError::OverlappingPlacement {
                offset: p.bytes() * 2,
                len: p.bytes(),
                existing_offset: p.bytes(),
                existing_len: p.bytes() * 2
            }
        );
    }

    #[test]
    fn a_placement_outside_the_reservation_is_refused() {
        let p = page();
        let mut res = Reservation::new(p.bytes(), p).expect("reserve");
        assert_eq!(
            res.map_fixed_in(
                HostOffset::new(p.bytes()),
                p.bytes(),
                Backing::PrivateAnonymous,
                HostProt::ReadWrite,
                CachePolicy::WriteBack
            )
            .unwrap_err(),
            RawError::OutOfRange {
                offset: p.bytes(),
                len: p.bytes(),
                object_len: p.bytes()
            }
        );
        assert_eq!(
            res.map_fixed_in(
                HostOffset::new(1),
                p.bytes(),
                Backing::PrivateAnonymous,
                HostProt::ReadWrite,
                CachePolicy::WriteBack
            )
            .unwrap_err(),
            RawError::Misaligned {
                what: "placement offset",
                value: 1,
                required: p.bytes()
            }
        );
        assert_eq!(
            res.map_fixed_in(
                HostOffset::ZERO,
                0,
                Backing::PrivateAnonymous,
                HostProt::ReadWrite,
                CachePolicy::WriteBack
            )
            .unwrap_err(),
            RawError::ZeroLength {
                what: "placement length"
            }
        );
    }

    #[test]
    fn a_placement_id_is_scoped_to_the_reservation_that_minted_it() {
        let p = page();
        let mut a = Reservation::new(p.bytes(), p).expect("reserve a");
        let b = Reservation::new(p.bytes(), p).expect("reserve b");
        let id = a
            .map_fixed_in(
                HostOffset::ZERO,
                p.bytes(),
                Backing::PrivateAnonymous,
                HostProt::ReadWrite,
                CachePolicy::WriteBack,
            )
            .expect("place");
        assert!(a.placement(id).is_ok());
        assert_eq!(
            b.placement(id).map(|_| ()),
            Err(RawError::UnknownPlacement { id: 0 })
        );
    }

    /// ★ The #111 failure shape, as a refusal: a request for an attribute the backing
    /// cannot have is an error at **every** mapping door, not a silent write-back mapping.
    ///
    /// All three doors, both backings, both unattainable policies — because a refusal that
    /// holds at one entry point and not the next is how a rule gets routed around.
    #[test]
    fn an_unattainable_cache_policy_is_refused_at_every_mapping_door() {
        let p = page();
        let f = shared_file(p.bytes());
        let fd = std::os::fd::AsFd::as_fd(&f);
        let backings = [
            (Backing::PrivateAnonymous, "anonymous memory"),
            (Backing::SharedFile { fd, offset: 0 }, "a shared file"),
        ];

        for (backing, name) in backings {
            for requested in [CachePolicy::WriteCombining, CachePolicy::Uncached] {
                let expected = RawError::CachePolicyUnattainable {
                    requested,
                    attainable: CachePolicy::WriteBack,
                    backing: name,
                };
                assert_eq!(
                    MappedRegion::map(backing, p.bytes(), HostProt::ReadWrite, requested, p)
                        .map(|_| ()),
                    Err(expected.clone()),
                    "MappedRegion::map accepted {requested} over {name}"
                );
                assert_eq!(
                    VolatileRegion::map(backing, p.bytes(), requested, p).map(|_| ()),
                    Err(expected.clone()),
                    "VolatileRegion::map accepted {requested} over {name}"
                );
                let mut res = Reservation::new(p.bytes(), p).expect("reserve");
                assert_eq!(
                    res.map_fixed_in(
                        HostOffset::ZERO,
                        p.bytes(),
                        backing,
                        HostProt::ReadWrite,
                        requested
                    )
                    .map(|_| ()),
                    Err(expected),
                    "map_fixed_in accepted {requested} over {name}"
                );
            }
        }
    }

    /// The refusal is checked **before** anything is mapped, so it consumes nothing: the
    /// next legitimate placement is still the reservation's first.
    ///
    /// This is the observable half. That no `mmap` was *performed* is argued by reading
    /// `map_fixed_in` — the check is its first statement after the lock witness — and a
    /// leaked `MAP_FIXED` inside our own `PROT_NONE` filler is not something a process can
    /// see about itself, which is exactly why the ordering is written down rather than
    /// asserted.
    #[test]
    fn a_refused_cache_policy_consumes_no_placement() {
        let p = page();
        let mut res = Reservation::new(p.bytes() * 2, p).expect("reserve");
        assert!(
            res.map_fixed_in(
                HostOffset::ZERO,
                p.bytes(),
                Backing::PrivateAnonymous,
                HostProt::ReadWrite,
                CachePolicy::WriteCombining,
            )
            .is_err()
        );

        let first = res
            .map_fixed_in(
                HostOffset::ZERO,
                p.bytes(),
                Backing::PrivateAnonymous,
                HostProt::ReadWrite,
                CachePolicy::WriteBack,
            )
            .expect("the refused placement must not have occupied the offset");
        let second = res
            .map_fixed_in(
                HostOffset::new(p.bytes()),
                p.bytes(),
                Backing::PrivateAnonymous,
                HostProt::ReadWrite,
                CachePolicy::WriteBack,
            )
            .expect("place second");
        assert!(first < second, "the refusal minted no id of its own");
    }

    /// A region reports the policy it was mapped under — the fact that is otherwise
    /// unrecoverable once the call has returned, and the one a bug report needs.
    #[test]
    fn a_region_reports_the_cache_policy_it_was_mapped_under() {
        let p = page();
        let r = MappedRegion::map(
            Backing::PrivateAnonymous,
            p.bytes(),
            HostProt::ReadWrite,
            CachePolicy::WriteBack,
            p,
        )
        .expect("map");
        assert_eq!(r.cache_policy(), CachePolicy::WriteBack);

        let v = VolatileRegion::map(
            Backing::PrivateAnonymous,
            p.bytes(),
            CachePolicy::WriteBack,
            p,
        )
        .expect("map");
        assert_eq!(v.cache_policy(), CachePolicy::WriteBack);

        let mut res = Reservation::new(p.bytes(), p).expect("reserve");
        let id = res
            .map_fixed_in(
                HostOffset::ZERO,
                p.bytes(),
                Backing::PrivateAnonymous,
                HostProt::ReadWrite,
                CachePolicy::WriteBack,
            )
            .expect("place");
        assert_eq!(
            res.placement(id).expect("placement").cache_policy(),
            CachePolicy::WriteBack,
            "a placement carries its own policy, not the reservation's (it has none)"
        );
    }

    /// ★ Every backing this crate can make is write-back, and that is a *finding* about
    /// Linux rather than a choice: `mmap` has no cacheability flag, so page-cache memory
    /// has exactly one attribute. Pinned as a table so the day a device backing lands, the
    /// row that changes is visible in the diff.
    #[test]
    fn every_backing_today_attains_write_back_and_only_write_back() {
        let p = page();
        let f = shared_file(p.bytes());
        let fd = std::os::fd::AsFd::as_fd(&f);
        for backing in [
            Backing::PrivateAnonymous,
            Backing::SharedFile { fd, offset: 0 },
        ] {
            assert_eq!(backing.attainable_cache_policy(), CachePolicy::WriteBack);
        }
        let _ = p;
    }

    /// ★ §4.5: every syscall-performing entry asserts the R1 witness. The `Drop` path is
    /// the one an ordinary scope exit can reach with a lock held, so it is the one worth
    /// pinning — and the assert deliberately fires *after* the `munmap`, so a violation
    /// is loud without also being a leak.
    #[test]
    fn dropping_a_host_mapping_under_a_held_lock_violates_r1() {
        let p = page();
        let r = MappedRegion::map(
            Backing::PrivateAnonymous,
            p.bytes(),
            HostProt::ReadWrite,
            CachePolicy::WriteBack,
            p,
        )
        .expect("map");

        // No panic-hook suppression: `set_hook` is process-global, and muting it from a
        // test that runs in parallel with others is a race that eats somebody else's
        // diagnostic. The expected backtrace on stderr is the cheaper cost, and it is the
        // precedent `kayfabe_util::lockwitness`'s own test set.
        lockwitness::note_acquired(1);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || drop(r)));
        lockwitness::note_released(1);

        let payload = outcome.expect_err("dropping a mapping under a lock must violate R1");
        let msg = payload
            .downcast_ref::<String>()
            .map_or("", String::as_str)
            .to_owned();
        assert!(
            msg.contains("R1 no-blocking-under-lock violation"),
            "unexpected panic message: {msg}"
        );
    }

    /// The same witness on the creating side.
    #[test]
    fn mapping_under_a_held_lock_violates_r1() {
        let p = page();
        lockwitness::note_acquired(0);
        let outcome = std::panic::catch_unwind(|| {
            MappedRegion::map(
                Backing::PrivateAnonymous,
                p.bytes(),
                HostProt::ReadWrite,
                CachePolicy::WriteBack,
                p,
            )
        });
        lockwitness::note_released(0);

        let payload = outcome.expect_err("mapping under a lock must violate R1");
        let msg = payload
            .downcast_ref::<String>()
            .map_or("", String::as_str)
            .to_owned();
        assert!(
            msg.contains("R1 no-blocking-under-lock violation"),
            "unexpected panic message: {msg}"
        );
    }
}
