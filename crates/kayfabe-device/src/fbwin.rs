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
        if !self.covers(phys, bytes.len()) {
            return Err(FbRefused {
                phys,
                len: bytes.len(),
                why: OUTSIDE_FRAMEBUFFER,
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

    fn device_reset(&mut self) {
        // ★ `clear()` keeps the map's capacity, which is a host allocation that survived
        // the guest that caused it — but not any of its BYTES. The leak this guards is a
        // content leak; a retained bucket array carries no guest data.
        self.pages.clear();
    }
}

kayfabe_util::assert_send_sync!(Bar0Window, FbRefused, RefusingFb);
