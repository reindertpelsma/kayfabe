//! ★★★ **A SYNTHETIC GA10x VER2 PAGE-TABLE IMAGE, BUILT HERE** — the thing the walk kernel
//! is pointed at when the isolate proves it can run the kernel at all.
//!
//! # ⊘ Why the isolate builds its own image rather than reading the guest's
//!
//! `SINGLE_STORE_PLAN.md` increment 4 proves the kernel **can run in-process and return a
//! correct answer**. Pointing it at the guest's real tables is increment 6, and it would make
//! this increment's result depend on a guest being in a particular state — so a red would not
//! distinguish *"CUDA is broken in the isolate"* from *"the guest had not built its tables
//! yet"*. An image we construct has an answer we know **before** the kernel runs.
//!
//! # ⊘ It is written INDEPENDENTLY of `cuda/walk/kf_tables.h`
//!
//! That file is the CUDA suite's own builder and shares no code with either decoder — which
//! is what makes the suite's 1212-leaf differential mean something (`THE_CONSTRAINTS.md`
//! §21: *"keep an independently-written table builder in the TESTS"*). This is a second
//! independent builder for the same format, written from the descriptor. ⚠ Do **not**
//! "de-duplicate" it against the kernel's `KfFormat`: a builder that shared the decoder's
//! constants could not disagree with it, and disagreeing is its entire job.

/// The GA10x VER2 encodings this builder needs. Bit positions written down **once**, here,
/// from `NV_MMU_VER2_*` (`pascal/gp100/dev_mmu.h`) — the same source the kernel's descriptor
/// is read from, and deliberately not the descriptor itself.
mod ver2 {
    /// `_PTE_VALID`, bit 0.
    pub const PTE_VALID: u64 = 1 << 0;
    /// `_PTE_VOL`, bit 3 — and, with VALID clear, the SPARSE encoding.
    pub const PTE_VOL: u64 = 1 << 3;
    /// Aperture, bits 2:1.
    pub const AP_SHIFT: u32 = 1;
    /// A PTE's `VIDEO_MEMORY` aperture value.
    pub const AP_PTE_VID: u64 = 0;
    /// A PTE's `SYSTEM_COHERENT_MEMORY` aperture value (`NV_MMU_PTE_APERTURE_*`, gp100
    /// `dev_mmu.h:82`).
    pub const AP_PTE_SYS_COH: u64 = 2;
    /// `_ADDRESS_SYS` is 53:8, i.e. 46 bits at offset 8, holding a 4 KiB page number (`:139`).
    pub const ADDR_SYS_BITS: u32 = 46;
    /// A PDE's `VIDEO_MEMORY` aperture value. ⊘ Not the same number as a PTE's: `0` on a PDE
    /// means *"no sub-level"*, which is why `pde_ap_invalid` exists in the descriptor.
    pub const AP_PDE_VID: u64 = 1;
    /// `_ADDRESS_VID` is 32:8, i.e. 25 bits at offset 8, holding a 4 KiB page number.
    pub const ADDR_LO: u32 = 8;
    /// Its width.
    pub const ADDR_BITS: u32 = 25;
    /// The dual PDE's big half: `_ADDRESS_BIG_VID` is 32:4, 29 bits, holding a 256 B unit.
    pub const BIG_ADDR_LO: u32 = 4;
    /// Its width.
    pub const BIG_ADDR_BITS: u32 = 29;
}

fn mask(bits: u32) -> u64 {
    (1u64 << bits) - 1
}

/// A page-directory entry pointing at `child` (a GPGA byte offset).
#[must_use]
pub fn pde(child: u64) -> u64 {
    (ver2::AP_PDE_VID << ver2::AP_SHIFT)
        | (((child >> 12) & mask(ver2::ADDR_BITS)) << ver2::ADDR_LO)
}

/// The **big** half of a dual PDE, pointing at a 64 KiB-leaf table at `child`.
#[must_use]
pub fn big_pde(child: u64) -> u64 {
    (ver2::AP_PDE_VID << ver2::AP_SHIFT)
        | (((child >> 8) & mask(ver2::BIG_ADDR_BITS)) << ver2::BIG_ADDR_LO)
}

/// A valid page-table entry mapping `phys` (a GPGA byte offset) in video memory.
#[must_use]
pub fn pte(phys: u64) -> u64 {
    ver2::PTE_VALID
        | (ver2::AP_PTE_VID << ver2::AP_SHIFT)
        | (((phys >> 12) & mask(ver2::ADDR_BITS)) << ver2::ADDR_LO)
}

/// A valid page-table entry mapping guest-physical `gpa` in coherent SYSTEM memory — the leaf a
/// guest kernel writes for a pushbuffer, GPFIFO or USERD it placed in its own RAM.
#[must_use]
pub fn pte_sys(gpa: u64) -> u64 {
    ver2::PTE_VALID
        | (ver2::AP_PTE_SYS_COH << ver2::AP_SHIFT)
        | (((gpa >> 12) & mask(ver2::ADDR_SYS_BITS)) << ver2::ADDR_LO)
}

/// The SPARSE encoding: VALID clear, VOLATILE set.
#[must_use]
pub fn sparse_pte() -> u64 {
    ver2::PTE_VOL
}

/// VA index at PD3 (`[48:47]`).
#[must_use]
pub fn vi3(va: u64) -> usize {
    ((va >> 47) & 3) as usize
}
/// VA index at PD2 (`[46:38]`).
#[must_use]
pub fn vi2(va: u64) -> usize {
    ((va >> 38) & 511) as usize
}
/// VA index at PD1 (`[37:29]`).
#[must_use]
pub fn vi1(va: u64) -> usize {
    ((va >> 29) & 511) as usize
}
/// VA index at PD0, the dual level (`[28:21]`).
#[must_use]
pub fn vi0(va: u64) -> usize {
    ((va >> 21) & 255) as usize
}
/// VA index in the small (4 KiB) leaf table (`[20:12]`).
#[must_use]
pub fn vis(va: u64) -> usize {
    ((va >> 12) & 511) as usize
}

/// A bump-allocated buffer standing in for GPGA.
///
/// ⊘ **Offset 0 is never handed out.** Both decoders treat a zero child pointer as *"no
/// sub-table"* (`kayfabe-mmu/src/walker.rs`'s `if e.next != 0`, and the C at
/// `nvkvm_gpu_emul.c:8615`), so a table placed there would be invisible rather than wrong —
/// the worst kind of test fixture.
pub struct Image {
    /// The bytes. `mem[0]` sits at GPGA offset [`Image::origin`].
    pub mem: Vec<u8>,
    bump: u64,
    /// ★ `[w825]` The GPGA offset of `mem[0]`. Every offset this image hands out and every
    /// entry it writes is ABSOLUTE — so an image built at `origin = 11.5 GiB` names the
    /// addresses a real guest's tables name, and is copied there verbatim. ⊘ No entry is ever
    /// rewritten on the way: rewriting is the relocation the identity window exists to remove.
    pub origin: u64,
}

impl Image {
    /// A zeroed image of `bytes`.
    #[must_use]
    pub fn new(bytes: usize) -> Image {
        Image::at(0, bytes)
    }

    /// An image whose first byte is GPGA offset `origin`. Offset 0 is still never handed out.
    #[must_use]
    pub fn at(origin: u64, bytes: usize) -> Image {
        Image { mem: vec![0u8; bytes], bump: origin.max(4096), origin }
    }

    /// Carve `bytes` at `align`. Panics if the image is too small — a fixture that silently
    /// truncated would produce a walk whose answer is a property of the fixture.
    ///
    /// # Panics
    /// If the allocation would run past the end of the image.
    pub fn alloc(&mut self, bytes: u64, align: u64) -> u64 {
        self.bump = (self.bump + align - 1) & !(align - 1);
        let o = self.bump;
        self.bump += bytes;
        assert!(
            self.bump - self.origin <= self.mem.len() as u64,
            "the synthetic GPGA image is too small: wanted {} bytes, have {}",
            self.bump,
            self.mem.len()
        );
        o
    }

    /// Write a little-endian `u64` at `off`.
    ///
    /// # Panics
    /// If `off` is out of range.
    pub fn put64(&mut self, off: u64, v: u64) {
        let o = usize::try_from(off - self.origin).expect("a GPGA offset fits usize");
        self.mem[o..o + 8].copy_from_slice(&v.to_le_bytes());
    }

    /// Its length.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.mem.len() as u64
    }

    /// Whether it is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.mem.is_empty()
    }
}

/// One mapping the builder was asked for, and therefore one the walk must find.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Expect {
    /// Guest virtual address.
    pub va: u64,
    /// GPGA offset it maps to.
    pub gpga: u64,
    /// Length in bytes.
    pub len: u64,
}

/// ★★★ **The fixture.** A root page directory plus `pages` contiguous 4 KiB mappings starting
/// at `va_base`, and **one sparse slot** after them so the report's `sparse_slots` counter has
/// something to count.
///
/// Returns `(image, root_pdb, expected)` where `root_pdb` is the GPGA offset of the root
/// table — the number the kernel is handed as the page-directory base.
///
/// ⊘ The expectation is **one coalesced run**, not `pages` runs: the kernel coalesces, and a
/// fixture whose expectation did not would be testing the fixture's arithmetic rather than the
/// kernel's.
///
/// # Panics
/// If `pages` is zero, or the image is too small for the tables it implies.
#[must_use]
pub fn contiguous_small_pages(va_base: u64, pages: u64, phys_base: u64) -> (Image, u64, Expect) {
    contiguous_small_pages_at(0, va_base, pages, phys_base)
}

/// [`contiguous_small_pages`], with the TABLES placed at GPGA offset `origin` and upward.
///
/// ⊘ `[w825]` The live guest's VAS roots sit near **11.5 GiB** (`pdb=0x2cea9c000`). A fixture
/// laid out from offset 4096 never exercises the question that matters for an identity window:
/// whether the kernel can reach tables *that high*, in place.
#[must_use]
pub fn contiguous_small_pages_at(
    origin: u64,
    va_base: u64,
    pages: u64,
    phys_base: u64,
) -> (Image, u64, Expect) {
    assert!(pages > 0, "a fixture with no mappings proves nothing");
    // ⊘ Sized from the fixture rather than a round number, so growing `pages` cannot silently
    // outgrow the image: five tables plus the pages' own backing, plus slack for alignment.
    let bytes = 0x40_0000 + usize::try_from(pages * 4096).expect("pages fit usize");
    let mut img = Image::at(origin, bytes);

    let pd3 = img.alloc(4 * 8, 4096);
    let pd2 = img.alloc(512 * 8, 4096);
    let pd1 = img.alloc(512 * 8, 4096);
    let pd0 = img.alloc(256 * 16, 4096);
    let small = img.alloc(512 * 8, 4096);

    img.put64(pd3 + 8 * vi3(va_base) as u64, pde(pd2));
    img.put64(pd2 + 8 * vi2(va_base) as u64, pde(pd1));
    img.put64(pd1 + 8 * vi1(va_base) as u64, pde(pd0));
    // The dual entry is 16 bytes: [big half, small half]. Only the small half is used here.
    img.put64(pd0 + 16 * vi0(va_base) as u64, big_pde(0));
    img.put64(pd0 + 16 * vi0(va_base) as u64 + 8, pde(small));

    for i in 0..pages {
        let va = va_base + i * 4096;
        assert_eq!(
            vi0(va),
            vi0(va_base),
            "the fixture's pages must stay inside ONE small leaf table; {pages} pages from \
             {va_base:#x} crosses a 2 MiB boundary and the expectation below would be wrong"
        );
        img.put64(small + 8 * vis(va) as u64, pte(phys_base + i * 4096));
    }
    // ★ One declared-empty slot, so `sparse_slots` in the report is a number this fixture
    // can assert rather than a field nobody exercises.
    let sparse_idx = vis(va_base) + usize::try_from(pages).expect("pages fit usize");
    if sparse_idx < 512 {
        img.put64(small + 8 * sparse_idx as u64, sparse_pte());
    }

    (
        img,
        pd3,
        Expect {
            va: va_base,
            gpga: phys_base,
            len: pages * 4096,
        },
    )
}
