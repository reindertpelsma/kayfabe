//! ★★★ **The BAR0 moving window** — `NV_PBUS_BAR0_WINDOW` + `PRAMIN`, and the framebuffer
//! bytes underneath them.
//!
//! # What the guest does with this, exactly (`ogkm-580: kern_bus_gm107.c:4084-4090`)
//!
//! ```text
//! GPU_FLD_WR_DRF_NUM(pGpu, _PBUS, _BAR0_WINDOW, _BASE,   NvU64_LO32(addr >> 16));
//! GPU_FLD_WR_DRF_NUM(pGpu, _PBUS, _BAR0_WINDOW, _TARGET, testAddrSpace);
//! testData = GPU_REG_RD32(pGpu, DRF_BASE(NV_PRAMIN) + NvU64_LO32(addr & 0xffff));
//! GPU_REG_WR32(pGpu, DRF_BASE(NV_PRAMIN) + NvU64_LO32(addr & 0xffff), SAMPLEDATA);
//! if (GPU_REG_RD32(pGpu, DRF_BASE(NV_PRAMIN) + NvU64_LO32(addr & 0xffff)) != SAMPLEDATA)
//! ```
//!
//! No BAR2, no MMU, no page table. A dword written through a 1 MiB aperture in the
//! *register* base-address register must read back, at whatever framebuffer address the
//! window register was pointed at.
//!
//! # ★★★ THREE PLACES A WRITE CAN BE LOST, AND WHY NONE OF THEM EXISTS HERE
//!
//! The boot of 2026-08-01 (`docs/design/boot_measured_2026_08_01.md` §18) recorded the
//! trap this module is written against: **`kbusInitBar2` programs this same window and
//! never reads any of it back**, so a window that silently drops writes lets every earlier
//! step return `NV_OK` and is caught only at `kbusVerifyBar2`, hundreds of operations
//! later, as `NV_ERR_MEMORY_ERROR`. ⇒ *"detected at the verify"* is not good enough; the
//! three ways to lose a write have to be **unrepresentable**.
//!
//! ### 1. The window register is a real LATCH, so a read-modify-write composes
//!
//! `GPU_FLD_WR_DRF_NUM` is a **read-modify-write** — it reads `0x1700`, replaces one
//! field, and writes the whole word back. The guest performs two of them back to back
//! (`_BASE`, then `_TARGET`). If the register read answered a defaulted zero, the second
//! write would put the base back to **zero** and every subsequent access would be
//! *mis-addressed with nothing logged anywhere*.
//!
//! It is worse than that, and the worse half is in RM rather than here:
//! `kbusSetBAR0WindowVidOffset_GM107` keeps a `cachedBar0WindowVidOffset` and
//! **skips the register write entirely** when the cache already holds the offset asked for
//! (`ogkm-580: kern_bus_gm107.c:4728-4760`) — refreshing the cache from
//! `GPU_REG_RD_DRF(_BAR0_WINDOW, _BASE)` when it is still zero. A device that dropped the
//! first write would leave RM believing the window is somewhere it is not, **permanently**,
//! with no further write to correct it.
//!
//! So [`Bar0Window`] is an ordinary 32-bit latch: what the guest wrote is what the guest
//! reads. That is not a convenience — it is what makes the guest's own two-step field
//! update mean what it says.
//!
//! ### 2. ONE address function, called by BOTH sides
//!
//! [`Bar0Window::fb_addr`] is the only arithmetic in this port that turns a window offset
//! into a framebuffer address, and [`crate::plane::RegPlane`]'s read path and write path
//! both call it. Two copies of `(base << 16) + off` could disagree — about the mask width,
//! about `+` versus `|`, about which end the offset is subtracted from — and a
//! read-after-write that resolved to two different addresses is exactly the failure
//! `kbusVerifyBar2` reports. One function cannot disagree with itself.
//!
//! ### 3. The store has NO SILENT-DROP ARM
//!
//! [`FbStore::write`] returns a `Result`. The plane matches it; there is no `let _ = …`,
//! and no arm of [`crate::plane::WriteOutcome`] says *"framebuffer write, dropped"* for a
//! window whose address is resolvable. Either the bytes are in the store or the caller
//! receives an [`FbRefused`] carrying the physical address, the length and the reason —
//! which the hypervisor shell prints on the spot and the audit counts.
//!
//! And within the framebuffer this chip **advertises** there is no refusal to have:
//! [`SparseFb`] allocates a page on first write, so every address below
//! [`crate::ChipProfile::fb_length`] accepts bytes. The refusals are for addresses the
//! guest was never promised ([`OUTSIDE_FRAMEBUFFER`]) and for a store that was never
//! installed ([`NO_FB_PORT`]) — two facts, neither of them a drop.
//!
//! # ★★ An unwritten framebuffer page reads ZERO, and that is NOT the `RefusingRam`
//! argument being abandoned
//!
//! [`crate::plane::RefusingRam`] refuses because *guest* memory is not ours: a zero-filled
//! read of a message queue is a well-formed element with a zero checksum, i.e. a wrong
//! answer the guest acts on, and we have no standing to invent it.
//!
//! The framebuffer is the opposite case. This device **owns** it — it is memory we
//! advertised, no other writer exists, and *"nothing has been written there yet"* has a
//! correct answer that we get to choose. Zero is that answer, it is self-consistent with
//! every later read, and it is what a scrubbed board reports. The distinction to keep is
//! between *inventing an answer for memory somebody else owns* and *stating the initial
//! content of memory we own*, and only the first is a lie.
//!
//! ⊘ It is still **not** the same seam as [`kayfabe_mmu::walker::FbRead`], whose `false`
//! means *"the isolate's fabricated aperture does not reach this address"* and must never
//! be spelled as zeros. See [`FbStore`] for why those two cannot be one type today.

use kayfabe_arch::FbWindow;
use std::collections::HashMap;

/// ★★ **The BAR0 moving window register, decoded** — `NV_PBUS_BAR0_WINDOW`.
///
/// Field layout, `ogkm-580: src/common/inc/swref/published/maxwell/gm107/dev_bus.h:43-50`:
///
/// | field | bits | meaning |
/// |---|---|---|
/// | `BASE` | 23:0 | framebuffer address `>> 16` — the window's origin |
/// | `TARGET` | 25:24 | `0` vidmem, `2` sysmem coherent, `3` sysmem non-coherent |
///
/// ★ The register offset itself is a [`crate::ChipProfile`] row field
/// ([`crate::ChipProfile::bar0_window_reg`]) and not a constant here, for the reason every
/// other geometry is on the row: a plane that hard-codes it is a plane the second
/// generation edits. The **field layout** is here because it has been the same word since
/// Maxwell and the header this port reads it out of is `gm107`'s.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Bar0Window {
    /// The raw 32 bits, exactly as the guest last wrote them.
    ///
    /// ★ Stored raw rather than as decoded fields, so [`Bar0Window::raw`] can hand the
    /// guest back **the same word** it wrote. A struct of decoded fields would have to
    /// re-encode, and a re-encode that dropped an undecoded bit would turn the guest's
    /// read-modify-write into a read-modify-**lose**.
    raw: u32,
}

/// `NV_PBUS_BAR0_WINDOW_BASE`, bits 23:0.
const BASE_MASK: u32 = 0x00FF_FFFF;

/// `NV_PBUS_BAR0_WINDOW_BASE_SHIFT` — the base names a 64 KiB-aligned framebuffer address.
const BASE_SHIFT: u32 = 16;

/// `NV_PBUS_BAR0_WINDOW_TARGET`, bits 25:24.
const TARGET_SHIFT: u32 = 24;

/// Ditto, width.
const TARGET_MASK: u32 = 0x3;

impl Bar0Window {
    /// The window at power-on: base 0, target vidmem — the register's own `_BASE_0`
    /// / `_TARGET_VID_MEM` reset values (`dev_bus.h:45, 47`).
    #[must_use]
    pub fn new() -> Bar0Window {
        Bar0Window { raw: 0 }
    }

    /// The word the guest last wrote, verbatim. See [`Bar0Window::raw`]'s field docs for
    /// why it is verbatim and not re-encoded.
    #[must_use]
    pub fn raw(self) -> u32 {
        self.raw
    }

    /// Latch a guest write.
    pub fn set_raw(&mut self, raw: u32) {
        self.raw = raw;
    }

    /// `BASE`, bits 23:0 — the window origin in units of 64 KiB.
    #[must_use]
    pub fn base(self) -> u32 {
        self.raw & BASE_MASK
    }

    /// `TARGET`, bits 25:24 — which aperture the window looks into.
    #[must_use]
    pub fn target(self) -> u32 {
        (self.raw >> TARGET_SHIFT) & TARGET_MASK
    }

    /// ★★★ **The one address function.** Which framebuffer byte the window offset
    /// `window_off` (0 .. the window's length) currently names.
    ///
    /// `(BASE << 16) + window_off`, in `u64`, over the **full 24-bit** base:
    ///
    /// - **`u64` throughout.** `BASE` is 24 bits and the shift is 16, so the origin alone
    ///   reaches `0xFFFF_FF00_00` — 1 TiB. Computing it in 32 bits would wrap every base
    ///   above `0xFFFF` and land the write inside the first 4 GiB of framebuffer, silently.
    /// - **`+`, not `|`.** The window is 1 MiB and the origin is only 64 KiB-aligned, so a
    ///   window offset above `0xFFFF` overlaps the origin's low bits. `|` would alias two
    ///   different addresses onto one — the C artifact adds
    ///   (`C: nvkvm_gpu_emul.c:905-908`), and so does the hardware the driver was written
    ///   for, whose window is a linear 1 MiB run from the origin.
    /// - **Saturating.** An origin near the top of the 40-bit reach plus a 1 MiB offset
    ///   cannot overflow `u64`, but the arithmetic is written so that no input can make it
    ///   wrap into a *low* address, which is the only wrap that would be dangerous.
    #[must_use]
    pub fn fb_addr(self, window_off: u64) -> u64 {
        (u64::from(self.base()) << BASE_SHIFT).saturating_add(window_off)
    }
}

/// Why a framebuffer access was refused, whole.
///
/// ★ Modelled on [`kayfabe_gsp::RamRefused`] and for the same reason: the tag alone says
/// *which kind*; this says *which address*, *how many bytes* and *why*. The two refusals
/// this port can raise are near neighbours by address and completely different findings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FbRefused {
    /// The framebuffer-physical address the access resolved to.
    pub phys: u64,
    /// How many bytes were asked for.
    pub len: usize,
    /// One sentence. `&'static str` so it crosses the hypervisor seam by value.
    pub why: &'static str,
}

/// ★★★ **The device's framebuffer, as a port.**
///
/// # ⊘ Why this is not `kayfabe_mmu::walker::FbRead`, checked rather than assumed
///
/// `FbRead` is the seam the owner's decision (b) put the framebuffer behind
/// (`eight_blockers_resolved.md` §12.2: *"the content lives … in the isolate's VRAM-backed
/// mapping of the fabricated aperture"*), and its one production implementation is
/// `kayfabe_fwd::IsolateFb` over a `kayfabe_isolate::Worker`. Reusing it here was the first
/// thing tried, and it does not reach, for three independent reasons — each one sufficient:
///
/// 1. **`FbRead` cannot write.** It *deliberately* has no method that hands content in —
///    that absence is what makes *"the core acquired a store of device memory"* (§11.6
///    option 3, rejected) an unrepresentable state. `kbusVerifyBar2` is a **write** then a
///    read.
/// 2. **There is no isolate yet.** `IsolateFb::new` takes a `&mut Worker`, i.e. a worker
///    checked out of an isolate spawned for a guest *process*. `kbusVerifyBar2` runs inside
///    `RmInitAdapter`, before the first client root exists — there is no `Proc`, no isolate
///    and no worker to borrow. The two seams are at different points in the device's life,
///    not at different layers of one point.
/// 3. **The isolate's aperture is the *fabricated* one.** It covers the addresses this port
///    invented for page tables it performs writes into. The BAR0 window is pointed at
///    whatever address RM's own heap handed out — `0x2_EFBA_E000` on the measured boot,
///    which is real advertised framebuffer near the top of the usable region.
///
/// ⇒ **This is a second port and it is not a second description of one fact.** `FbRead`
/// answers *"what is in the isolate's mapping of the fabricated aperture"*; this answers
/// *"what is in the framebuffer this device advertises"*. The day the two ranges overlap —
/// when the data plane exists and a page table lives in advertised framebuffer — the
/// convergence is an implementation of **this** trait that delegates to the isolate, and
/// [`crate::plane::RegPlane::set_fb`] is the seam it is installed through. Nothing above
/// this trait learns that it moved.
///
/// ★ `&mut self`, matching `FbRead` and [`kayfabe_gsp::GuestRam`]: the eventual production
/// implementation is a *connection* to an isolate and every access is a round trip, which
/// is a fact every caller should be able to see in the signature.
pub trait FbStore: Send + core::fmt::Debug {
    /// Fill `buf` from framebuffer-physical address `phys`.
    ///
    /// An address inside the advertised framebuffer that has never been written reads
    /// **zero** and returns `Ok` — see the module docs for why that is not the
    /// `RefusingRam` argument being abandoned.
    ///
    /// # Errors
    /// [`FbRefused`] when this store does not back the range at all.
    fn read(&mut self, phys: u64, buf: &mut [u8]) -> Result<(), FbRefused>;

    /// Write `bytes` at framebuffer-physical address `phys`.
    ///
    /// # Errors
    /// [`FbRefused`] when this store does not back the range at all. ⊘ There is no
    /// success-shaped answer for a write that did not land; that is the whole point of the
    /// `Result`.
    fn write(&mut self, phys: u64, bytes: &[u8]) -> Result<(), FbRefused>;

    /// How many bytes of host memory this store is currently holding on the guest's
    /// behalf.
    ///
    /// ★ On the trait rather than on the implementation because it is what makes the
    /// bound in [`SparseFb`] *observable*: a cap nobody can read is a cap nobody knows was
    /// reached.
    fn resident_bytes(&self) -> u64;

    /// ★★★★ **WHICH bytes this store actually holds** — [`None`] from a store that cannot
    /// answer the question at all.
    ///
    /// # ⊘ The asymmetry this exists to break, and it is MEASURED
    ///
    /// `[measured 2026-08-09, boot `bar1_03a679f`]` the framebuffer page the guest's own
    /// page tables name for its GPFIFO ring dumped as `nz0/4096` — not one non-zero byte.
    /// ⊘ **That single observation has two causes and they need different fixes**: the page
    /// was **never written** (nothing ever addressed it), or it **was written with zeros**
    /// (something addressed it and put nothing there). [`FbStore::read`] returns *zero and
    /// `Ok`* for an unwritten address inside the aperture — deliberately, and documented —
    /// so the byte census **cannot** tell them apart. Residency can: a page nothing ever
    /// wrote is not in the map.
    ///
    /// ★★ **[`None`] is not "nothing is resident".** A store that backs no memory at all
    /// ([`RefusingFb`]) has no residency to report, and answering `0` would be a positive
    /// claim about a device that has no framebuffer port — the same error as decoding an
    /// empty capture to zeros. A counter must carry its own precondition, and this one's
    /// precondition is *"there is a store to ask"*.
    fn residency(&self) -> Option<FbResidency>;

    /// ★★★★ **Write, NAMING THE WRITER** — §16.15's instrument.
    ///
    /// Identical to [`FbStore::write`] in every observable effect on the bytes; the only
    /// difference is that the store records `by` as the page's **first** writer if the page
    /// is created by this call. See [`FbWriter`].
    ///
    /// ⊘ The default implementation ignores `by` and delegates, so a store that does not
    /// track origins is not forced to lie about them.
    ///
    /// # Errors
    /// As [`FbStore::write`].
    fn write_tagged(&mut self, phys: u64, bytes: &[u8], by: FbWriter) -> Result<(), FbRefused> {
        let _ = by;
        self.write(phys, bytes)
    }

    /// ★★★★ **Every resident frame, ascending** — [`None`] when the store cannot enumerate.
    ///
    /// # ⊘ Why this exists when [`FbResidency`] deliberately refuses to carry a list
    ///
    /// [`FbResidency::lo`]'s doc argues, correctly, that a *boot report* is no place for a
    /// frame list and that the frame which matters is asked for **by name** through
    /// [`FbStore::is_resident`]. ★ That argument holds for every question of the form *"is
    /// page X here?"* — and the question that is now open is the **converse**:
    /// `[measured 2026-08-09, boot `res1_fc21926`]` the frame the guest's own page tables
    /// name for its ring is **not** resident, while 90 other frames are. *"Then which frame
    /// holds the ring?"* cannot be asked by name, because the name is exactly what we do
    /// not have.
    ///
    /// ⊘ **This is a search primitive, not a report field.** Its consumer sweeps the
    /// resident set looking for GPFIFO-entry-shaped bytes — a **forward** search that never
    /// consults the walker whose answer is under audit. Two projections of one computation
    /// cannot audit each other; a scan of raw bytes and a page-table descent are genuinely
    /// independent.
    ///
    /// ★ **Ascending, always.** [`SparseFb`] is a [`std::collections::HashMap`], whose
    /// iteration order varies run to run; an unsorted list would make two boots of one
    /// binary produce differently-ordered evidence for the same store.
    fn resident_frames(&self) -> Option<Vec<u64>> {
        None
    }

    /// Who wrote the page containing `phys` FIRST, and when in sequence — [`None`] when
    /// this store cannot say, **or when no page is resident there**.
    ///
    /// ⊘ The two [`None`]s are distinguished by [`FbStore::is_resident`], deliberately: a
    /// caller that needs to tell *"no store"* from *"never written"* already has the method
    /// that answers it, and folding a third state in here would make the common case carry
    /// the rare one.
    fn page_origin(&self, phys: u64) -> Option<FbPageOrigin> {
        let _ = phys;
        None
    }

    /// ★★★★★ **w318 — HOW MANY WRITES THIS STORE HAS TAKEN FROM `by`**, monotone, never
    /// reset. ⊘ [`None`] when the store does not count — *unmeasured*, never `0`, and a
    /// consumer that gates on it must treat `None` as **arm, do not skip**.
    ///
    /// # Why a WRITE count when [`FbStore::page_origin`] already exists
    ///
    /// `page_origin` is deliberately **first**-writer and bumps its sequence only on page
    /// CREATION, so it cannot see the case this exists for: the executor rewriting a page
    /// it already created. `[measured 2026-08-14, w315 boot `full`]` the doorbell handler
    /// re-queues **every** executor-created page for page-table decode on every doorbell —
    /// `resident=171 by-executor=53`, unconditionally — and that alone costs **22.3 ms per
    /// launch** producing `bound=0`. The set of pages is unchanged doorbell to doorbell; what
    /// a gate needs to know is whether their BYTES are, and only a write count says that.
    ///
    /// ⚠ It counts **calls that landed bytes**, not bytes and not pages: a consumer may
    /// conclude *"nothing this writer wrote has changed"* from an unchanged value, and
    /// nothing finer.
    fn writes_by(&self, by: FbWriter) -> Option<u64> {
        let _ = by;
        None
    }

    /// Whether this store holds a page for `phys` — [`None`] when it cannot say.
    ///
    /// ⊘ Deliberately **not** derivable from [`FbStore::read`]: a read of an unwritten
    /// address succeeds and yields zeros, which is exactly the answer this distinguishes
    /// from. See [`FbStore::residency`].
    fn is_resident(&self, phys: u64) -> Option<bool>;

    /// ★★★★★ **Install a JOINED range** — `[phys, phys+region.len())` is served from now on
    /// by memory a second party also maps (`fb_cpu_view.md` §4).
    ///
    /// # ⊘ Why this is a MAPPING and not a connection — the tree contradicted itself here
    ///
    /// [`FbStore`]'s own docs nominate the convergence as *"an implementation of this trait
    /// that delegates to the isolate … a **connection** … and every access is a round trip"*.
    /// **That implementation cannot be installed.** Every call site of this trait holds the
    /// register plane's FSM mutex, which `tests/tests/unranked_locks.rs:56-59` classifies as
    /// *"★★★ THE HAZARD … ⊘ NOTHING may block beneath it, and the R1 witness will not say
    /// so"*. A round trip to another process beneath that lock stalls every vCPU's register
    /// access, and the one instrument that would normally catch it is blind to this lock.
    ///
    /// ⇒ The join replaces the store's **pages**, never its lookup. A `memcpy` into an
    /// `mmap` blocks on nothing, so this is the only shape that is installable at all.
    ///
    /// # ★★★ The establishment copy, and why it belongs INSIDE this call
    ///
    /// Bytes the guest wrote before the backing existed are already in this store. The
    /// implementation must copy them into `region` **before** the range goes live, and it
    /// must read them from its **own** pages rather than through the range it is installing.
    ///
    /// ⊘ That is not tidiness — it is what makes the ordering safe by construction. The
    /// owner's objection was *"mapping after execution seems racy to me"*, and it is correct:
    /// once the engine has written the real object and the guest has written the fabricated
    /// one, there is **no correct merge** — a merge is a choice about which writes to lose.
    /// With the copy here, after this call there is ONE memory and there is never a merge.
    ///
    /// ★ It follows that this call must be **atomic against guest access**, which it is: the
    /// caller holds the plane lock across it, so no framebuffer read or write can land
    /// between the copy and the install.
    ///
    /// # Errors
    /// [`FbRefused`] when this store does not back the range, when the range is already
    /// joined, or when the establishment copy cannot be performed. ⊘ There is deliberately no
    /// success-shaped answer for a join that did not take: a store that reported `Ok` and
    /// kept serving its own pages would be the two-memories defect, re-created by the very
    /// call that exists to end it.
    fn install_join(
        &mut self,
        phys: u64,
        region: Box<dyn FbJoined>,
    ) -> Result<FbJoinInstalled, (FbRefused, Box<dyn FbJoined>)> {
        Err((
            FbRefused {
                phys,
                len: 0,
                why: NO_JOIN_SUPPORT,
            },
            region,
        ))
    }

    /// Every joined range, ascending by address — `(phys, len)`.
    ///
    /// ⊘ Empty is a real answer and means *"this store holds no joined range"*; it is not the
    /// *"cannot say"* [`FbStore::residency`] uses [`None`] for, because a store that cannot
    /// join is a store with no joins, and those are the same fact here.
    fn joined_ranges(&self) -> Vec<(u64, u64)> {
        Vec::new()
    }

    /// ★★★★★ **w329 — GIVE BACK the join installed at `phys`**, so this store serves that
    /// range from its own pages again. [`FbStore::install_join`]'s inverse, and the half whose
    /// absence `w327` measured as an allocation failure.
    ///
    /// Returns the backing that was installed, so the caller — the only party that knows what
    /// the second holder is — can release its half. `None` means **nothing was installed at
    /// exactly `phys`**, which is a refusal and not a success: a caller that read it as *"the
    /// join is gone"* would go on to free a host object this store is still serving bytes out
    /// of, and the next guest read of that range would be a `SIGBUS` in the VMM.
    ///
    /// # ★★★ The base must match EXACTLY, and that is the partial-extent refusal
    ///
    /// A join covers a whole leaf and [`FbStore::install_join`] refuses **any** overlap, so a
    /// release naming an address inside a join is a caller that has confused a leaf with an
    /// address in one. Refused rather than resolved to the containing range: releasing more
    /// than the caller named is exactly the shape that turns a correct release into a
    /// double-free of the neighbour's object.
    ///
    /// # ⊘ THE BYTES ARE NOT CARRIED BACK, and that is a decision with a residual
    ///
    /// [`FbStore::install_join`] copies this store's pages **into** the join, because bytes the
    /// guest wrote before the backing existed are real. The mirror image is deliberately NOT
    /// performed here, for two reasons that point the same way:
    ///
    /// 1. **It would be the wrong answer.** A range is released because the guest's own page
    ///    tables stopped naming it; to this device a framebuffer frame no page table names is
    ///    an unallocated frame, whose truth is *"no page, reads zero"* — the same answer
    ///    [`FbStore::device_reset`] leaves, and the same one a never-written frame gives.
    /// 2. **It would trade one unbounded growth for another.** Copying back materialises one
    ///    resident 4 KiB page per page of every freed allocation, forever — the join leak
    ///    re-created in [`SparseFb::pages`], where the residency ceiling turns it into a
    ///    different hard failure.
    ///
    /// ⚠ **The residual, named:** if the guest still reaches the same frame through a second
    /// alias it did not unmap, that alias's bytes are gone. The caller counts that case rather
    /// than assuming it absent (`kayfabe_mmu::reach::ApplyOutcome::revoked_still_desired`).
    fn release_join(&mut self, phys: u64) -> Option<Box<dyn FbJoined>> {
        let _ = phys;
        None
    }

    /// Power-on: forget every byte.
    ///
    /// ★★ **Not optional, and the reason is not tidiness.** Framebuffer content that
    /// survived a device life would be the *previous* guest's page tables, instance blocks
    /// and semaphores, readable by the next one through this very window — a cross-life
    /// information leak with no other detector. It is also `#130`'s property
    /// (*"unload → reload must yield a device indistinguishable from first boot"*), which
    /// is quantified over **all** of the device's state.
    fn device_reset(&mut self);
}

/// ★★★★★ **Bytes a SECOND party also maps** — the isolate's half of a joined framebuffer
/// leaf, as a port (`fb_cpu_view.md` §4).
///
/// # ⊘ Why a trait and not a mapping type
///
/// This crate is pure: it holds no descriptor, performs no `mmap` and names no OS. The
/// memory behind a join is a shared file the isolate minted and the VMM adopted, and both of
/// those are facts of `kayfabe-linux-raw` and the composition root. What this crate needs is
/// exactly two verbs over a bounded extent, which is what this is.
///
/// ⊘ It is **not** [`FbStore`] with fewer methods. An `FbStore` answers for a whole
/// framebuffer and decides what an unwritten address means; this answers for one range that
/// somebody else is also holding, and has no opinions at all.
///
/// ★ `Send` because the register plane is behind a mutex reached from every vCPU thread. ⊘
/// No `Sync` bound: the store owns it exclusively, which is what makes `&mut self` on
/// [`FbJoined::write`] a true statement rather than a lock in disguise.
pub trait FbJoined: Send + core::fmt::Debug {
    /// How many bytes this join covers. ⊘ Read once at install and never again — a length
    /// that could change under a live mapping is `SIGBUS`, which is why the backing carries
    /// `F_SEAL_SHRINK`.
    fn len(&self) -> u64;

    /// Whether it covers none. (Clippy's companion to [`FbJoined::len`].)
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Fill `buf` from byte `off` of this join.
    ///
    /// # Errors
    /// One sentence, when the access is out of the join's own extent.
    fn read(&self, off: u64, buf: &mut [u8]) -> Result<(), &'static str>;

    /// Write `bytes` at byte `off` of this join.
    ///
    /// # Errors
    /// As [`FbJoined::read`].
    fn write(&mut self, off: u64, bytes: &[u8]) -> Result<(), &'static str>;
}

/// ★★★ What [`FbStore::install_join`] did — **the establishment copy, counted**.
///
/// ⊘ Two numbers and not a `bool`, because *"the join is live"* and *"the bytes the guest had
/// already written came with it"* are different facts and only the second one can be
/// vacuous. A leaf whose pages were never resident copies zero bytes and the join is still
/// correct; a leaf with 90 resident pages that copied zero is a bug. A caller that could not
/// tell those apart would report both as success.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FbJoinInstalled {
    /// How many bytes were copied out of this store's own pages into the join.
    pub copied: u64,
    /// ★ How many of those bytes were **non-zero** — the non-vacuity term. An establishment
    /// copy of an all-zero range is correct and proves nothing, and a report that omitted
    /// this would let it read as evidence.
    pub nonzero: u64,
    /// How many 4 KiB pages of this store the copy read from — i.e. were resident.
    pub pages: u64,
}

/// [`FbStore::install_join`]'s refusal from a store that cannot join at all.
pub const NO_JOIN_SUPPORT: &str = "this framebuffer store cannot hold a joined range; it has no pages of its own to \
     establish from and nothing to install into";

/// [`SparseFb::install_join`]'s refusal for a range that already carries a join.
pub const ALREADY_JOINED: &str = "that framebuffer range is already joined; installing a second backing over it \
     would give one leaf two memories again, which is the defect the join exists to end";

/// [`SparseFb::install_join`]'s refusal for a join whose bytes could not be established.
pub const ESTABLISH_FAILED: &str = "the establishment copy into the joined backing failed, so the join was NOT \
     installed: a live join whose pre-existing bytes never arrived would present the engine \
     a blank pool for a leaf the guest has already written";

/// ★★★★★ **Where one framebuffer page STANDS — and the join is an arm of it, not a footnote.**
///
/// # ⊘⊘ Why this type exists: `is_resident` is JOIN-BLIND, and that blindness was MEASURED
///
/// [`SparseFb::install_join`] **removes the local pages and their `origin` rows** for a joined
/// range — deliberately, so that one leaf is one memory. ⇒ For every address inside a join:
///
/// | asked | answers | true? |
/// |---|---|---|
/// | [`FbStore::read`] / [`FbStore::write`] | the joined backing's **live bytes** | ★ yes |
/// | [`FbStore::is_resident`] | `Some(false)` | ⊘ **reads as "never written"** |
/// | [`FbStore::page_origin`] | `None` | ⊘ **reads as "no first writer"** |
///
/// Both lower rows are *correct about this store's own pages* and **wrong as statements about
/// the guest**, which is the only reading anyone has ever wanted them for.
///
/// `[measured 2026-08-12, boot `w278b_guest`]` — the whole cost, in one line of one artefact:
///
/// ```text
/// fbRING[p0]@0x41000=0000022001400000… nz4/4096 resN-NEVER-WRITTEN by?
/// ```
///
/// **`nz4` and `resN-NEVER-WRITTEN` are in the same line and contradict each other.** The
/// four non-zero bytes are the guest client's own GPFIFO entry (`0x0000400120020000` =
/// `pb @ 0x1_20020000, 16 dwords`), CPU-stored through `NV_ESC_RM_MAP_MEMORY` and served back
/// correctly by the join — while the residency token beside them announced that nothing had
/// ever written the page. On that same boot
/// [`kayfabe_fwd::FwdFault::RingFbNeverWritten`](../../kayfabe_fwd/enum.FwdFault.html) refused
/// the doorbell for the same reason, and the refusal was read as the wall.
///
/// ⇒ **Every caller asking about a page it did not itself allocate must ask THIS**, and
/// `RegPlane::fb_is_resident` was removed so that the join-blind question is no longer
/// reachable from the plane at all. ★ [`FbStore::is_resident`] keeps its meaning — *"does
/// this store hold a page"* — because that is a real question about the store, and
/// `tests/fb_join.rs` asserts its `Some(false)` on a joined address on purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FbPageStanding {
    /// ★★★★★ Inside a joined range: **one memory, held elsewhere.** The bytes are live and
    /// correct; residency and first-writer are questions this store cannot answer, and a
    /// caller must treat them as **unmeasured** — never as *no*. (The `dlen=0` lesson.)
    JoinedOneMemory,
    /// The store holds its own page here; a write landed.
    Resident,
    /// ★ The store holds no page and no join — nothing ever wrote this address. **This is
    /// the only arm that is a positive claim about the guest.**
    NeverWritten,
    /// The store cannot say.
    Unknown,
}

impl FbPageStanding {
    /// The token these appear as in every framebuffer dump row.
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            Self::JoinedOneMemory => "JOINED-one-memory",
            Self::Resident => "resY",
            Self::NeverWritten => "resN-NEVER-WRITTEN",
            Self::Unknown => "res?",
        }
    }

    /// ★★★ **The forwarding plane's reading** — `Some(false)` **only** for
    /// [`Self::NeverWritten`].
    ///
    /// ⊘ [`Self::JoinedOneMemory`] answers [`None`] = *unmeasured*, which is what stops
    /// `kayfabe_fwd::fetch_ring_bytes` refusing a ring whose bytes are live. It is a real
    /// loss of a guard — a joined page nobody wrote reads as zeros and this can no longer
    /// say so — and it is the honest one: the store genuinely does not know.
    #[must_use]
    pub fn written(self) -> Option<bool> {
        match self {
            Self::Resident => Some(true),
            Self::NeverWritten => Some(false),
            Self::JoinedOneMemory | Self::Unknown => None,
        }
    }
}

/// ★★★★ **What a framebuffer store holds, as a census rather than a total.**
///
/// `[measured 2026-08-09, boot `bar1_03a679f`]` the teardown report said `resident 368640
/// bytes` — 90 pages — and that number cannot answer *"is the ring's page one of them?"*,
/// which is the question the boot was run for. A total is a summary of a set; the set is
/// what decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FbResidency {
    /// How many 4 KiB pages the store holds.
    pub pages: u64,
    /// The lowest resident framebuffer address, or [`None`] when nothing is resident.
    ///
    /// ⊘ An extent and not a list: the list is up to a residency cap's worth of frames and
    /// a boot report is not the place for it. The extent plus [`FbResidency::pages`] is
    /// what says whether the resident set is *clustered* or *spread*, which is the shape
    /// question — and the frame that actually matters is asked for by name through
    /// [`FbStore::is_resident`], never found by scanning this.
    pub lo: Option<u64>,
    /// The highest resident framebuffer address, or [`None`] when nothing is resident.
    pub hi: Option<u64>,
    /// ★★★★ **How many resident pages each writer was FIRST to touch.**
    ///
    /// Indexed by [`FbWriter::index`]. See [`FbWriter`] for why first-writer and not
    /// last-writer, and why [`FbWriter::Unattributed`] is a real answer.
    pub by_writer: [u64; FB_WRITER_KINDS],
}

/// How many distinct [`FbWriter`] kinds there are.
pub const FB_WRITER_KINDS: usize = 5;

/// ★★★★ **WHO wrote a framebuffer page first** — `execution_plane_increments.md` §16.15.
///
/// # ⊘ Why this exists, and it is the only discriminator left
///
/// `[measured 2026-08-09, boot `res1_fc21926`]` the page the guest's own page tables name
/// for its GPFIFO ring is **not resident** — nothing ever aimed a write at it — while
/// **624 206** writes landed in **90** distinct pages spread across the whole 11.7 GiB
/// aperture. The guest demonstrably wrote its ring; some page took those bytes. Residency
/// says *which pages exist*; only this says **which window put them there**.
///
/// ★ **FIRST writer, not last.** A page rewritten 6 900 times by a later path would
/// otherwise report that path and erase the one fact worth having — who *created* it. The
/// creation event is what attributes a page to a write path; every subsequent write is
/// evidence about traffic, not about origin.
///
/// ⊘ [`FbWriter::Unattributed`] is **an answer, not a default.** A caller that writes
/// through [`FbStore::write`] genuinely does not say which window it is, and recording a
/// window it did not name would be inventing attribution — the same error class as decoding
/// an empty capture to zeros. It is spelled out so a census that is mostly `Unattributed`
/// reads as *"we did not instrument that path"* rather than as a finding about the guest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FbWriter {
    /// One of the register-aperture windows, named.
    Window(FbWindow),
    /// The shell's own CPU copy executor (`kayfabe_rt::cpu_ce`).
    Executor,
    /// A write whose origin the caller did not state. ⊘ Not "unknown window" — *"nobody
    /// said"*.
    Unattributed,
}

impl FbWriter {
    /// A stable index into [`FbResidency::by_writer`].
    #[must_use]
    pub fn index(self) -> usize {
        match self {
            FbWriter::Window(FbWindow::Pramin) => 0,
            FbWriter::Window(FbWindow::FbAperture) => 1,
            FbWriter::Window(FbWindow::InstanceWindow) => 2,
            FbWriter::Executor => 3,
            FbWriter::Unattributed => 4,
        }
    }

    /// One short word for a diagnostic.
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            FbWriter::Window(FbWindow::Pramin) => "PRAMIN",
            FbWriter::Window(FbWindow::FbAperture) => "BAR1",
            FbWriter::Window(FbWindow::InstanceWindow) => "BAR2",
            FbWriter::Executor => "EXEC",
            FbWriter::Unattributed => "UNATTRIBUTED",
        }
    }

    /// The name for each [`FbWriter::index`], in index order — for a report that prints the
    /// whole census.
    #[must_use]
    pub fn tags() -> [&'static str; FB_WRITER_KINDS] {
        ["PRAMIN", "BAR1", "BAR2", "EXEC", "UNATTRIBUTED"]
    }
}

/// What a store knows about one page's creation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FbPageOrigin {
    /// Who wrote it first.
    pub by: FbWriter,
    /// A monotonic sequence number, so two pages can be ORDERED against each other.
    ///
    /// ⊘ Ordering only — it is not a timestamp and not a write count. It exists because
    /// §16.13's refutation of *"written and then erased"* holds only **up to ordering**, and
    /// neither the byte census nor the residency bit can close that.
    pub seq: u64,
}

/// An [`FbStore`] that refuses every access, by name. The construction default.
///
/// ★ The exact twin of [`crate::plane::RefusingRam`], and for the exact reason: a device
/// whose shell never installed a framebuffer must say *"there is no framebuffer here"*
/// rather than behave like one that is empty. Those are different findings and only one of
/// them is a wiring bug.
#[derive(Debug, Default, Clone, Copy)]
pub struct RefusingFb;

/// [`RefusingFb`]'s one sentence.
pub const NO_FB_PORT: &str = "the register plane has no framebuffer port installed; the shell never called \
     set_fb, so the BAR0 moving window has no device memory to alias";

/// [`SparseFb`]'s refusal for an address outside the framebuffer this chip advertises.
pub const OUTSIDE_FRAMEBUFFER: &str = "that address is outside the framebuffer this chip advertises; the guest was never \
     promised it, and allocating it would let a window base nobody validated grow the \
     host's memory without bound";

/// [`SparseFb`]'s refusal once [`SPARSE_FB_RESIDENT_CAP`] is reached.
pub const RESIDENT_CAP_REACHED: &str = "this device is already holding the whole of its framebuffer-residency budget; a \
     further page would be host memory allocated at a guest's request with no ceiling";

impl FbStore for RefusingFb {
    fn read(&mut self, phys: u64, buf: &mut [u8]) -> Result<(), FbRefused> {
        Err(FbRefused {
            phys,
            len: buf.len(),
            why: NO_FB_PORT,
        })
    }

    fn write(&mut self, phys: u64, bytes: &[u8]) -> Result<(), FbRefused> {
        Err(FbRefused {
            phys,
            len: bytes.len(),
            why: NO_FB_PORT,
        })
    }

    fn resident_bytes(&self) -> u64 {
        0
    }

    /// ⊘ [`None`], never `Some(0)`. This store backs no memory, so *"nothing is resident"*
    /// would be a statement about a framebuffer that does not exist. See the trait method.
    fn residency(&self) -> Option<FbResidency> {
        None
    }

    fn is_resident(&self, _phys: u64) -> Option<bool> {
        None
    }

    fn device_reset(&mut self) {}
}

/// The page this store allocates in, in bytes.
///
/// ★ 4 KiB, matching the C artifact's `g_malloc0(4096)` page store
/// (`C: nvkvm_gpu_emul.c:906-919`) and the smallest page the GMMU has. Nothing depends on
/// the value being this one — [`SparseFb`] keys on `phys / PAGE`, `phys % PAGE` and never
/// on a shift — but a store whose page were larger than the MMU's would allocate memory a
/// guest never asked for.
pub const FB_PAGE: u64 = 4096;

/// ★★ **How much host memory one device life may hold on a guest's behalf.**
///
/// The C artifact had no bound at all: `nvkvm_fb_page(alloc = true)` allocates on any
/// write, so a guest walking its own advertised 12 GiB framebuffer with a dword store per
/// page could make the hypervisor process 12 GiB — on a bench box whose whole guest is
/// 2 GiB. That is boundary-1's own shape (*"guest bytes are hostile"*) applied to an
/// allocation rather than to a pointer.
///
/// 1 GiB is far past anything a boot touches — `[measured]` by replaying the committed C
/// reference traces on 2026-07-31 through this port's own window classifier
/// (`crates/kayfabe-crec/tests/fb_window_census.rs`, over
/// `nvidia-gpu-passthrough/traces/mode2_c_reference/cap1_coldboot_hermetic`): the whole cold
/// boot's `PRAMIN` traffic is **33 978** writes, i.e. at most 33 978 pages even with no
/// reuse at all. And it is far below what a 4-core dev box with **no swap** can survive
/// losing (`local_box_has_no_swap_oom_is_fatal`).
///
/// ⊘ Reaching it is a **named refusal**, counted and printed — never a dropped write. A cap
/// that silently discarded the last page would be the exact defect this module exists to
/// make unrepresentable, wearing a resource-limit costume.
pub const SPARSE_FB_RESIDENT_CAP: u64 = 1 << 30;

/// ★★★ **A sparse, allocate-on-write framebuffer** — the shell's own store of the device
/// memory it advertises.
///
/// This is `eight_blockers_resolved.md` §11.6's **option 1 port shape**: the abstract seam
/// stays ([`FbStore`]), the bytes live outside every pure crate, and the day the isolate
/// backs them the seam does not move. See [`FbStore`] for why the isolate cannot back them
/// *now*.
///
/// # Why sparse and not a `Vec`
///
/// GA106 advertises 12 GiB. A dense image is not allocatable on any box this project runs
/// on, and the boot touches a handful of pages.
///
/// # Why a dropped write is not expressible here
///
/// [`SparseFb::write`] resolves `phys` to `(phys / FB_PAGE, phys % FB_PAGE)` and allocates
/// the page if it is absent. [`SparseFb::read`] resolves it **the same way** and reads
/// zeros where no page exists. So for any address below [`SparseFb::limit`] the pair is
/// total: write-then-read at one address returns what was written, and there is no
/// intermediate state in which the write "succeeded" and the byte is not there.
#[derive(Debug)]
pub struct SparseFb {
    /// One past the highest framebuffer address this store backs — the chip's advertised
    /// `fb_length`.
    limit: u64,
    /// The residency ceiling, in bytes. See [`SPARSE_FB_RESIDENT_CAP`].
    cap: u64,
    /// Page frame → 4 KiB of bytes.
    pages: HashMap<u64, Box<[u8; FB_PAGE as usize]>>,
    /// ★★★★ Page frame → who created it and in what order. Parallel to
    /// [`SparseFb::pages`] and cleared with it; see [`FbWriter`].
    origin: HashMap<u64, FbPageOrigin>,
    /// The monotonic sequence stamped on the next page CREATION. ⊘ Bumped only when a page
    /// is created, not on every write: it orders origins, and a per-write counter would
    /// order traffic instead — a different and much noisier fact.
    seq: u64,
    /// ★★★★★ Framebuffer ranges served from memory a **second party also maps** — the joined
    /// leaves (`fb_cpu_view.md` §4). Ascending by address and non-overlapping, both
    /// maintained by [`SparseFb::install_join`].
    ///
    /// ⊘ **Not a second store.** A joined range's pages are `mmap`ed by the isolate too, so
    /// the guest's write through this window and the engine's read through the GPU MMU are
    /// the same byte. Everything outside these ranges is still [`SparseFb::pages`].
    joined: Vec<(u64, Box<dyn FbJoined>)>,
    /// ★★★★★ **w318 — per-writer WRITE counts**, indexed by [`FbWriter::index`]. See
    /// [`FbStore::writes_by`] for why a *write* count is needed beside
    /// [`SparseFb::origin`]'s *creation* sequence, and what a consumer may conclude.
    ///
    /// ⊘ Bumped **before** the joined-range early return and before the residency ceiling,
    /// so a write into a joined leaf and a write refused for want of room both count. A
    /// counter a gate reads must move whenever the writer *acted*; making it move only on the
    /// paths that happened to land bytes in `pages` would give the gate a blind spot exactly
    /// where the join plane is busiest.
    writes_by: [u64; FB_WRITER_KINDS],
}

impl SparseFb {
    /// A store backing `[0, limit)` with the default residency ceiling.
    #[must_use]
    pub fn new(limit: u64) -> SparseFb {
        SparseFb::with_cap(limit, SPARSE_FB_RESIDENT_CAP)
    }

    /// A store with an explicit residency ceiling, for a test that wants to reach it.
    #[must_use]
    pub fn with_cap(limit: u64, cap: u64) -> SparseFb {
        SparseFb {
            limit,
            cap,
            pages: HashMap::new(),
            origin: HashMap::new(),
            seq: 0,
            joined: Vec::new(),
            writes_by: [0; FB_WRITER_KINDS],
        }
    }

    /// One past the highest address this store backs.
    #[must_use]
    pub fn limit(&self) -> u64 {
        self.limit
    }

    /// How many pages are resident.
    #[must_use]
    pub fn resident_pages(&self) -> usize {
        self.pages.len()
    }

    /// Whether `[phys, phys+len)` lies wholly inside the advertised framebuffer.
    ///
    /// ★ Checked as a **unit**, and with no wrapping arithmetic: an access that starts
    /// inside and ends outside is refused whole rather than truncated, because a truncated
    /// write is a dropped write wearing a partial-success costume.
    /// The joined range wholly containing `[phys, phys+len)`, as `(index, offset)`.
    ///
    /// ⊘ **Wholly, or not at all.** An access that straddles the edge of a join is not split
    /// between the two stores: a read half-served from a joined mapping and half from a local
    /// page is two memories inside one access, which is the defect with a smaller blast
    /// radius rather than a fix. It falls through to the sparse path, where the joined half
    /// reads as this store's own bytes — wrong, and *loudly* wrong at the first comparison,
    /// rather than subtly right. ★ It cannot arise in practice: joins are whole leaves and
    /// the window's accesses are dwords.
    fn joined_at(&self, phys: u64, len: usize) -> Option<(usize, u64)> {
        let len = u64::try_from(len).ok()?;
        let end = phys.checked_add(len)?;
        self.joined
            .iter()
            .position(|(base, r)| {
                let jend = base.saturating_add(r.len());
                phys >= *base && end <= jend
            })
            .map(|i| (i, phys - self.joined[i].0))
    }

    fn covers(&self, phys: u64, len: usize) -> bool {
        let Ok(len) = u64::try_from(len) else {
            return false;
        };
        match phys.checked_add(len) {
            Some(end) => end <= self.limit,
            None => false,
        }
    }

    /// Split `[phys, phys+len)` into `(page frame, offset in page, bytes in this page)`
    /// runs.
    ///
    /// ★ Its own function so the read and the write walk the range **identically**. An
    /// access that straddles a page boundary is the case a hand-rolled loop gets wrong in
    /// one of the two directions, and a read that split differently from the write is a
    /// read-after-write that fails for no visible reason.
    fn runs(phys: u64, len: usize) -> impl Iterator<Item = (u64, usize, usize)> {
        let mut at = phys;
        let mut left = len;
        core::iter::from_fn(move || {
            if left == 0 {
                return None;
            }
            let off = usize::try_from(at % FB_PAGE).unwrap_or(0);
            let take = left.min(FB_PAGE as usize - off);
            let frame = at / FB_PAGE;
            at += take as u64;
            left -= take;
            Some((frame, off, take))
        })
    }
}

impl FbStore for SparseFb {
    fn read(&mut self, phys: u64, buf: &mut [u8]) -> Result<(), FbRefused> {
        if !self.covers(phys, buf.len()) {
            return Err(FbRefused {
                phys,
                len: buf.len(),
                why: OUTSIDE_FRAMEBUFFER,
            });
        }
        // ★★★★★ THE JOIN, checked FIRST. A joined range's bytes are held by memory the
        // isolate also maps; this store's own pages for that range were copied in at install
        // and are dead from that instant. Serving them would be answering with a snapshot.
        if let Some((i, off)) = self.joined_at(phys, buf.len()) {
            return self.joined[i].1.read(off, buf).map_err(|why| FbRefused {
                phys,
                len: buf.len(),
                why,
            });
        }
        let mut done = 0usize;
        for (frame, off, take) in SparseFb::runs(phys, buf.len()) {
            match self.pages.get(&frame) {
                // ★ Memory we advertised and nobody has written. Zero, and `Ok` — the
                // module docs argue why that is a statement rather than an invention.
                None => buf[done..done + take].fill(0),
                Some(p) => buf[done..done + take].copy_from_slice(&p[off..off + take]),
            }
            done += take;
        }
        Ok(())
    }

    fn write(&mut self, phys: u64, bytes: &[u8]) -> Result<(), FbRefused> {
        // ⊘ An untagged write records `Unattributed` — NOT a guessed window. See
        // [`FbWriter::Unattributed`].
        self.write_tagged(phys, bytes, FbWriter::Unattributed)
    }

    fn write_tagged(&mut self, phys: u64, bytes: &[u8], by: FbWriter) -> Result<(), FbRefused> {
        if !self.covers(phys, bytes.len()) {
            return Err(FbRefused {
                phys,
                len: bytes.len(),
                why: OUTSIDE_FRAMEBUFFER,
            });
        }
        // ★★★★★ w318 — the arming edge, bumped HERE: past the "is this even our aperture"
        // refusal (a write to somebody else's address changed nothing of ours) and **before**
        // the joined-range return and the residency ceiling below, so every write this store
        // accepted responsibility for moves it. See [`SparseFb::writes_by`].
        self.writes_by[by.index()] = self.writes_by[by.index()].saturating_add(1);
        // ★★★★★ THE JOIN, checked FIRST and BEFORE the residency ceiling — a joined range
        // costs this store no page at all, so charging it against the budget would refuse a
        // guest write to memory that is already allocated. ⊘ The write does not touch
        // `origin` either: a page that does not exist has no first writer, and inventing one
        // would put a joined leaf in the by-writer census as though it were resident.
        if let Some((i, off)) = self.joined_at(phys, bytes.len()) {
            let _ = by;
            return self.joined[i].1.write(off, bytes).map_err(|why| FbRefused {
                phys,
                len: bytes.len(),
                why,
            });
        }
        // ★★ The ceiling is checked for the WHOLE access before a single byte lands, so a
        // straddling write is never half-applied. A half-applied write is a dropped write
        // that also corrupted something.
        let fresh = SparseFb::runs(phys, bytes.len())
            .filter(|(frame, _, _)| !self.pages.contains_key(frame))
            .count() as u64;
        if self.resident_bytes() + fresh * FB_PAGE > self.cap {
            return Err(FbRefused {
                phys,
                len: bytes.len(),
                why: RESIDENT_CAP_REACHED,
            });
        }
        let mut done = 0usize;
        for (frame, off, take) in SparseFb::runs(phys, bytes.len()) {
            // ★★★★ FIRST writer, recorded once. `or_insert_with` on the origin map is what
            // makes it first-and-not-last: a page rewritten 6 900 times by a later path
            // keeps the attribution of whoever CREATED it, which is the fact that names a
            // write path. The sequence bumps only here, so it orders CREATIONS.
            if !self.pages.contains_key(&frame) {
                self.seq += 1;
                self.origin
                    .insert(frame, FbPageOrigin { by, seq: self.seq });
            }
            let page = self
                .pages
                .entry(frame)
                .or_insert_with(|| Box::new([0u8; FB_PAGE as usize]));
            page[off..off + take].copy_from_slice(&bytes[done..done + take]);
            done += take;
        }
        Ok(())
    }

    fn resident_bytes(&self) -> u64 {
        self.pages.len() as u64 * FB_PAGE
    }

    fn residency(&self) -> Option<FbResidency> {
        let mut by_writer = [0u64; FB_WRITER_KINDS];
        for o in self.origin.values() {
            by_writer[o.by.index()] += 1;
        }
        Some(FbResidency {
            pages: self.pages.len() as u64,
            lo: self.pages.keys().min().map(|f| f * FB_PAGE),
            hi: self.pages.keys().max().map(|f| f * FB_PAGE),
            by_writer,
        })
    }

    fn page_origin(&self, phys: u64) -> Option<FbPageOrigin> {
        self.origin.get(&(phys / FB_PAGE)).copied()
    }

    fn writes_by(&self, by: FbWriter) -> Option<u64> {
        Some(self.writes_by[by.index()])
    }

    fn resident_frames(&self) -> Option<Vec<u64>> {
        // ⊘ Sorted, for the reason on the trait method: this is a `HashMap` and its
        // iteration order is not stable across runs of one binary.
        let mut v: Vec<u64> = self.pages.keys().map(|f| f * FB_PAGE).collect();
        v.sort_unstable();
        Some(v)
    }

    /// ⊘ Asked about the **page**, and answered `Some(false)` for an address inside the
    /// aperture that no write ever touched — which is the arm that separates *"never
    /// written"* from *"written with zeros"*. An address **outside** the aperture is also
    /// `Some(false)`: this store genuinely holds no page for it, and [`FbStore::read`]
    /// refuses it separately, so the two findings stay apart.
    fn is_resident(&self, phys: u64) -> Option<bool> {
        Some(self.pages.contains_key(&(phys / FB_PAGE)))
    }

    fn install_join(
        &mut self,
        phys: u64,
        region: Box<dyn FbJoined>,
    ) -> Result<FbJoinInstalled, (FbRefused, Box<dyn FbJoined>)> {
        let len = region.len();
        let refuse = |why| FbRefused {
            phys,
            len: usize::try_from(len).unwrap_or(usize::MAX),
            why,
        };
        let Ok(len_usize) = usize::try_from(len) else {
            return Err((refuse(OUTSIDE_FRAMEBUFFER), region));
        };
        if !self.covers(phys, len_usize) {
            return Err((refuse(OUTSIDE_FRAMEBUFFER), region));
        }
        // ⊘ Any overlap at all, not just an exact repeat: two joins sharing one byte is two
        // memories for that byte, which is what this whole mechanism removes.
        let end = phys.saturating_add(len);
        if self
            .joined
            .iter()
            .any(|(b, r)| phys < b.saturating_add(r.len()) && *b < end)
        {
            return Err((refuse(ALREADY_JOINED), region));
        }
        // ★★★ THE ESTABLISHMENT COPY. Read from this store's OWN pages — never through
        // `FbStore::read`, which would answer from the join the moment it is installed — and
        // performed BEFORE the range goes live. After this there is one memory and there is
        // never a merge.
        // ★★★★★ **OWNER RULING 2026-08-27 — WHEN THIS BACKING BECOMES VIDMEM, THIS LOOP
        // MUST BECOME A `ce_copy`.** See `docs/design/copy_placement_policy.md` §2.2.
        //
        // A leaf is at least one `FB_LEAF_GRANULE`, so the copy below is **bulk HtoD**. Today
        // `region` is a `memfd` and this is HtoH — a memcpy, correct and fastest. The moment
        // the leaf is backed by vidmem it becomes a bulk CPU write **across the BAR**, which
        // the policy forbids: bulk DtoH, HtoD and DtoD all go to the copy engine
        // (`kayfabe_isolate_host`'s `ce_copy`), and memcpy is for HtoH and small scalars only.
        //
        // ⊘ Do NOT reason *"write-combining is write-optimised, so a bulk write is fine"*.
        // WC writes are **less bad** than WC reads, not good — still single-digit GB/s over
        // PCIe against the engine writing device-local at full framebuffer bandwidth. That
        // exact argument was made and corrected while this comment was being written.
        let mut region = region;
        let mut copied = 0u64;
        let mut nonzero = 0u64;
        let mut pages = 0u64;
        for (frame, off, take) in SparseFb::runs(phys, usize::try_from(len).unwrap_or(usize::MAX)) {
            // ⊘ Only RESIDENT pages are copied. A page this store never held is a page
            // nothing ever wrote, and the fabricated backing is already zero-filled by its
            // own `ftruncate` — so copying zeros over it would be work, and *counting* those
            // zeros as copied bytes would make every establishment report non-vacuous.
            let Some(page) = self.pages.get(&frame) else {
                continue;
            };
            let src = &page[off..off + take];
            let at = frame * FB_PAGE + off as u64 - phys;
            if region.write(at, src).is_err() {
                return Err((refuse(ESTABLISH_FAILED), region));
            }
            pages += 1;
            copied += take as u64;
            nonzero += src.iter().filter(|b| **b != 0).count() as u64;
        }
        // ★ The local pages go, and they go AFTER the copy: they are now a stale second copy
        // of memory that has one authoritative holder, and a store that kept them would have
        // exactly the two memories this call removes, one layer down. ⊘ Their `origin` rows go
        // with them — a first-writer record for a page that no longer exists would attribute
        // a resident-page census entry to memory the census can no longer see.
        for (frame, _, _) in SparseFb::runs(phys, usize::try_from(len).unwrap_or(usize::MAX)) {
            self.pages.remove(&frame);
            self.origin.remove(&frame);
        }
        self.joined.push((phys, region));
        self.joined.sort_unstable_by_key(|(b, _)| *b);
        Ok(FbJoinInstalled {
            copied,
            nonzero,
            pages,
        })
    }

    fn joined_ranges(&self) -> Vec<(u64, u64)> {
        // ⊘ Already ascending — `install_join` sorts — but stated rather than relied on: a
        // future insert that forgot to sort would make two boots of one binary produce
        // differently-ordered evidence.
        let mut v: Vec<(u64, u64)> = self.joined.iter().map(|(b, r)| (*b, r.len())).collect();
        v.sort_unstable();
        v
    }

    fn release_join(&mut self, phys: u64) -> Option<Box<dyn FbJoined>> {
        // ★ EXACT base, per the trait's doc. `install_join` refuses any overlap, so at most
        // one entry can match and a linear scan answers whole.
        let i = self.joined.iter().position(|(b, _)| *b == phys)?;
        let (_, region) = self.joined.remove(i);
        // ⊘ No page is created here. The range now has no join and no local pages, which is
        // exactly `is_resident == Some(false)` / reads-as-zero — the state a framebuffer frame
        // nothing has written is in, and the state this store started in. ⚠ Any bytes the join
        // held are gone; see the trait's residual.
        //
        // ★ The `Vec` stays sorted: `remove` preserves the order of the rest, so
        // `joined_ranges`'s "already ascending" property survives a release as well as an
        // install.
        Some(region)
    }

    fn device_reset(&mut self) {
        // ★★★★★ THE JOINS GO TOO, and this is the arm `fb_cpu_view.md` §4.3 names as a
        // cross-life leak if it is missed. A joined range that survived a device life would
        // be the PREVIOUS guest's framebuffer content, still mapped by an isolate, readable
        // by the next guest through this very window — and unlike a stale local page it is
        // not even this process's memory to have kept. ⊘ Dropping the boxes releases this
        // side's mapping; the isolate's own half dies with the isolate, which is the same
        // lifetime `#130` quantifies over.
        self.joined.clear();
        // ★ `clear()` keeps the map's capacity, which is a host allocation that survived
        // the guest that caused it — but not any of its BYTES. The leak this guards is a
        // content leak; a retained bucket array carries no guest data.
        self.pages.clear();
        // ★★★ The origins go with the bytes, and the sequence RESTARTS. A page's creation
        // record that outlived the guest that created it would attribute one device life's
        // write path to the next one's page — the same cross-life confusion the byte clear
        // exists to prevent, one field over. `#130` quantifies over ALL of the device's
        // state, and this is device state.
        self.origin.clear();
        self.seq = 0;
    }
}

kayfabe_util::assert_send_sync!(Bar0Window, FbRefused, RefusingFb);
