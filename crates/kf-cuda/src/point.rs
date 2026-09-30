//! ★★★ **The walk kernel's translation of ONE guest VA, on the CPU** — for the moment the GPU
//! walker cannot run (`docs/design/V3_UVM_GUEST_FAULT_PLANE.md` §3.8a).
//!
//! `[measured uvmg4, 2026-09-30, d178a737]` a GR context stalled on a PARKED replayable fault cannot
//! be context-switched out: the walk kernel submitted for the guest's servicing invalidate waited
//! 4.3 s for the GR engine, the host's ctxsw watchdog then killed the faulted context (Xid 109), and
//! only after that did the walk run and the mapping land — the replay it was needed for came too
//! late. (The b3 host-only proof shows the same property from the other side: `coexist` lost ~3 s of
//! GR time during a 3 s park, `traces/v3_uvm_b3/run_full.log` §D2.) So while a guest fault is
//! parked, the pages it names must be resolved without the GPU walker: this module reads the SAME
//! tables with the SAME rules the kernel uses (`cuda/walk/kf_walk.cu` `kf_dir_step`,
//! `kf_walk_one`), for one VA, through a caller-supplied loader.
//!
//! ⊘ It is the kernel's decode transcribed, field by field, off the same [`KfFormat`] descriptor —
//! never a second format table. What it does NOT do: coalesce, diff, or commit. It answers "what does
//! the guest's table say about this one page", and the caller decides what to do with that.

use crate::abi::{
    AP_VID, KF_DIRS, KF_PS_NONE, KF_TBL_VER2, KFWR_RF_AP_MASK, KFWR_RF_ATOMIC_DISABLE,
    KFWR_RF_KIND_MASK, KFWR_RF_KIND_SHIFT, KFWR_RF_PRIVILEGE, KFWR_RF_PS_MASK, KFWR_RF_PS_SHIFT,
    KFWR_RF_READ_ONLY, KFWR_RF_VOLATILE, KfField, KfFormat,
};

/// One translation, as the walk kernel would emit it for the page containing the VA.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Leaf {
    /// The leaf's first VA (aligned to its page size).
    pub va: u64,
    /// Its backing: a GPGA offset (vidmem) or a guest-physical address (sysmem).
    pub addr: u64,
    /// Its size in bytes.
    pub bytes: u64,
    /// The run flags the kernel would report (`KFWR_RF_*`: aperture, permissions, KIND, page size).
    pub flags: u32,
}

/// What the tables say about a VA.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Point {
    /// A valid translation.
    Leaf(Leaf),
    /// No translation (an empty, sparse or invalid slot anywhere on the path).
    Unmapped,
    /// The walk could not be completed as the kernel would: a directory in a foreign aperture, or a
    /// table the loader could not read. Named, never guessed.
    Refused(&'static str),
}

fn field(raw: u64, f: KfField) -> u64 {
    let mask = if f.bits >= 64 {
        u64::MAX
    } else {
        (1u64 << f.bits) - 1
    };
    ((raw >> f.lo) & mask) << f.shift
}

fn ap_raw(f: &KfFormat, raw: u64) -> usize {
    ((raw >> f.ap_lo) & ((1u64 << f.ap_bits) - 1)) as usize
}

fn valid(f: &KfFormat, raw: u64) -> bool {
    (raw >> f.valid_bit) & 1 != 0
}

fn addr(f: &KfFormat, raw: u64, apc: usize) -> u64 {
    field(
        raw,
        if f.addr_sel[apc & 3] != 0 {
            f.addr_sys
        } else {
            f.addr_local
        },
    )
}

fn big_addr(f: &KfFormat, raw: u64, apc: usize) -> u64 {
    field(
        raw,
        if f.addr_sel[apc & 3] != 0 {
            f.big_addr_sys
        } else {
            f.big_addr_local
        },
    )
}

/// `kf_dir_present`: the aperture IS the validity (VER2 and VER3 alike).
fn dir_present(f: &KfFormat, apc: usize) -> bool {
    apc != usize::from(f.pde_ap_invalid)
}

/// `kf_slot_sparse`.
fn slot_sparse(f: &KfFormat, raw: u64) -> bool {
    if f.table_version == KF_TBL_VER2 {
        (raw >> f.bit_volatile) & 1 != 0
    } else {
        field(raw, KfField { shift: 0, ..f.pcf }) == u64::from(f.pcf_sparse)
    }
}

/// `kf_big_pte_unmapped` (VER2 only): VALID=0, not sparse, PRIVILEGE set.
fn big_pte_unmapped(f: &KfFormat, e: u64) -> bool {
    f.table_version == KF_TBL_VER2
        && !valid(f, e)
        && !slot_sparse(f, e)
        && (e >> f.bit_privilege) & 1 != 0
}

/// `kf_leaf_flags`.
fn leaf_flags(f: &KfFormat, raw: u64, ps: u8) -> u32 {
    let mut fl = u32::from(f.pte_ap_map[ap_raw(f, raw) & 3]) & KFWR_RF_AP_MASK;
    if (raw >> f.bit_volatile) & 1 != 0 {
        fl |= KFWR_RF_VOLATILE;
    }
    if (raw >> f.bit_privilege) & 1 != 0 {
        fl |= KFWR_RF_PRIVILEGE;
    }
    if (raw >> f.bit_read_only) & 1 != 0 {
        fl |= KFWR_RF_READ_ONLY;
    }
    if (raw >> f.bit_atomic_disable) & 1 != 0 {
        fl |= KFWR_RF_ATOMIC_DISABLE;
    }
    fl |= ((field(raw, f.kind) as u32) & KFWR_RF_KIND_MASK) << KFWR_RF_KIND_SHIFT;
    fl | ((u32::from(ps) & KFWR_RF_PS_MASK) << KFWR_RF_PS_SHIFT)
}

fn leaf(f: &KfFormat, va: u64, raw: u64, ps: u8) -> Point {
    let bytes = 1u64 << f.ps_log2[usize::from(ps) & 3];
    Point::Leaf(Leaf {
        va: va & !(bytes - 1),
        addr: addr(f, raw, ap_raw(f, raw)),
        bytes,
        flags: leaf_flags(f, raw, ps),
    })
}

fn index(va: u64, va_lo: u8, entries: u16) -> u64 {
    (va >> va_lo) & (u64::from(entries).max(1) - 1)
}

/// ★ Translate `va` through the tables rooted at `pdb` (a vidmem offset). `load(off)` reads the
/// little-endian 64-bit word at vidmem offset `off`, or `None` when it cannot (outside the store,
/// unreadable) — which refuses the lookup by name, as the kernel's `kf_table_ok` would skip it.
pub fn lookup(f: &KfFormat, pdb: u64, va: u64, load: &mut dyn FnMut(u64) -> Option<u64>) -> Point {
    let mut tbl = pdb;
    for k in 0..KF_DIRS - 1 {
        let l = f.dir[k];
        if l.active == 0 {
            continue;
        }
        let idx = index(va, l.va_lo, l.entries);
        let Some(raw) = load(tbl + idx * u64::from(l.entry_bytes)) else {
            return Point::Refused("a directory entry is unreadable");
        };
        if l.leaf_ps != KF_PS_NONE && valid(f, raw) {
            return leaf(f, va, raw, l.leaf_ps);
        }
        let apc = ap_raw(f, raw);
        if !dir_present(f, apc) {
            return Point::Unmapped;
        }
        if f.pde_ap_map[apc & 3] != AP_VID {
            return Point::Refused("a directory in a non-vidmem aperture");
        }
        let next = addr(f, raw, apc);
        if next == 0 {
            return Point::Unmapped;
        }
        tbl = next;
    }
    // The dual level: one 16-byte entry, a big (64 KiB) and a small (4 KiB) sub-table.
    let d = f.dir[KF_DIRS - 1];
    let at = tbl + index(va, d.va_lo, d.entries) * u64::from(d.entry_bytes);
    let (Some(lo), Some(hi)) = (load(at), load(at + 8)) else {
        return Point::Refused("a dual directory entry is unreadable");
    };
    if d.leaf_ps != KF_PS_NONE && valid(f, lo) {
        return leaf(f, va, lo, d.leaf_ps);
    }
    let (aps, apb) = (ap_raw(f, hi), ap_raw(f, lo));
    let mut small = None;
    let mut big = None;
    if dir_present(f, aps) {
        if f.pde_ap_map[aps & 3] != AP_VID {
            return Point::Refused("a small page table in a non-vidmem aperture");
        }
        small = Some(addr(f, hi, aps)).filter(|&a| a != 0);
    }
    if dir_present(f, apb) {
        if f.pde_ap_map[apb & 3] != AP_VID {
            return Point::Refused("a big page table in a non-vidmem aperture");
        }
        big = Some(big_addr(f, lo, apb)).filter(|&a| a != 0);
    } else if slot_sparse(f, lo) {
        // An invalid big half with VOL set is SPARSE for the whole 2 MiB (ogkm
        // `_gmmuIsInvalidPdeOk`): the small table is never read.
        small = None;
    }
    if let Some(ptb) = big {
        let b = index(va, f.big_va_lo, f.big_entries);
        let Some(e) = load(ptb + b * u64::from(f.big_entry_bytes)) else {
            return Point::Refused("a big page table entry is unreadable");
        };
        if valid(f, e) {
            return leaf(f, va, e, f.big_ps);
        }
        // ★ A VALID or UNMAPPED big PTE owns its 64 KiB slot: the 4 KiB entries under it are not
        // translations (`kf_big_pte_owns_slot`).
        if big_pte_unmapped(f, e) {
            return Point::Unmapped;
        }
    }
    if let Some(pts) = small {
        let s = index(va, f.small_va_lo, f.small_entries);
        let Some(e) = load(pts + s * u64::from(f.small_entry_bytes)) else {
            return Point::Refused("a small page table entry is unreadable");
        };
        if valid(f, e) {
            return leaf(f, va, e, f.small_ps);
        }
    }
    Point::Unmapped
}

/// The leaf's aperture code (`KFWR_RF_AP_*`: 0 vidmem, 1 peer, 2/3 sysmem).
#[must_use]
pub const fn aperture(l: &Leaf) -> u8 {
    (l.flags & KFWR_RF_AP_MASK) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::{PS_4K, PS_64K, kf_format_ver2, kf_format_ver3};
    use crate::synth::{self, Image};

    fn loader(img: &Image) -> impl FnMut(u64) -> Option<u64> + '_ {
        move |off| {
            let o = usize::try_from(off.checked_sub(img.origin)?).ok()?;
            let b = img.mem.get(o..o + 8)?;
            Some(u64::from_le_bytes(b.try_into().ok()?))
        }
    }

    /// ★ The kernel's own fixture: every mapped 4 KiB page is found at its GPGA, the page after
    /// them (declared sparse) and one in another 2 MiB are unmapped.
    #[test]
    fn small_pages_translate_as_the_kernel_reports_them() {
        let va = 0x7f00_1234_0000;
        let (img, pdb, exp) = synth::contiguous_small_pages(va, 8, 0x40_0000);
        let f = kf_format_ver2();
        for i in 0..8 {
            let got = lookup(&f, pdb, va + i * 4096 + 0x123, &mut loader(&img));
            let Point::Leaf(l) = got else {
                panic!("page {i}: {got:?}")
            };
            assert_eq!(l.va, va + i * 4096);
            assert_eq!(l.addr, exp.gpga + i * 4096);
            assert_eq!(l.bytes, 4096);
            assert_eq!(aperture(&l), AP_VID);
            assert_eq!(
                (l.flags >> KFWR_RF_PS_SHIFT) & KFWR_RF_PS_MASK,
                u32::from(PS_4K)
            );
        }
        assert_eq!(
            lookup(&f, pdb, va + 8 * 4096, &mut loader(&img)),
            Point::Unmapped
        );
        assert_eq!(
            lookup(&f, pdb, va + (4 << 20), &mut loader(&img)),
            Point::Unmapped
        );
    }

    /// A valid BIG PTE owns its 64 KiB slot — the stale 4 KiB entries under it are not reported
    /// (the w826 / v3-mapfix rule) — and an UNMAPPED big PTE hides them too.
    #[test]
    fn a_big_page_owns_its_slot() {
        let va = 0x7f00_0000_0000u64;
        let mut img = Image::new(0x20_0000);
        let pd3 = img.alloc(4 * 8, 4096);
        let pd2 = img.alloc(512 * 8, 4096);
        let pd1 = img.alloc(512 * 8, 4096);
        let pd0 = img.alloc(256 * 16, 4096);
        let small = img.alloc(512 * 8, 4096);
        let big = img.alloc(32 * 8, 256);
        img.put64(pd3 + 8 * synth::vi3(va) as u64, synth::pde(pd2));
        img.put64(pd2 + 8 * synth::vi2(va) as u64, synth::pde(pd1));
        img.put64(pd1 + 8 * synth::vi1(va) as u64, synth::pde(pd0));
        img.put64(pd0 + 16 * synth::vi0(va) as u64, synth::big_pde(big));
        img.put64(pd0 + 16 * synth::vi0(va) as u64 + 8, synth::pde(small));
        // slot 0: a valid 64 KiB page over stale valid 4 KiB entries
        img.put64(big, synth::pte(0x100_0000));
        img.put64(small, synth::pte(0x200_0000));
        // slot 1: an UNMAPPED big PTE (VALID 0, VOL 0, PRIV 1) over a stale 4 KiB entry
        img.put64(big + 8, 1 << 5);
        img.put64(small + 8 * 16, synth::pte(0x300_0000));
        // slot 2: no big entry: the 4 KiB entry is the translation
        img.put64(small + 8 * 32, synth::pte(0x400_0000));
        let f = kf_format_ver2();
        let p = lookup(&f, pd3, va + 0x2345, &mut loader(&img));
        let Point::Leaf(l) = p else { panic!("{p:?}") };
        assert_eq!((l.va, l.addr, l.bytes), (va, 0x100_0000, 0x10000));
        assert_eq!(
            (l.flags >> KFWR_RF_PS_SHIFT) & KFWR_RF_PS_MASK,
            u32::from(PS_64K)
        );
        assert_eq!(
            lookup(&f, pd3, va + 0x10000, &mut loader(&img)),
            Point::Unmapped
        );
        let p = lookup(&f, pd3, va + 0x20000, &mut loader(&img));
        let Point::Leaf(l) = p else { panic!("{p:?}") };
        assert_eq!((l.va, l.addr, l.bytes), (va + 0x20000, 0x400_0000, 0x1000));
    }

    /// Sysmem leaves carry their aperture and guest-physical address; unreadable tables refuse.
    #[test]
    fn sysmem_leaves_and_unreadable_tables() {
        let va = 0x1_2340_0000u64;
        let (mut img, pdb, _) = synth::contiguous_small_pages(va, 1, 0x40_0000);
        // Re-point the one PTE at guest RAM.
        let f = kf_format_ver2();
        let Point::Leaf(before) = lookup(&f, pdb, va, &mut loader(&img)) else {
            panic!()
        };
        assert_eq!(before.addr, 0x40_0000);
        // find the PTE by scanning for its value
        let want = synth::pte(0x40_0000).to_le_bytes();
        let at = img
            .mem
            .windows(8)
            .position(|w| w == want)
            .expect("the fixture's PTE");
        img.put64(img.origin + at as u64, synth::pte_sys(0x8765_4000));
        let Point::Leaf(l) = lookup(&f, pdb, va, &mut loader(&img)) else {
            panic!()
        };
        assert_eq!((aperture(&l), l.addr), (2, 0x8765_4000));
        assert_eq!(
            lookup(&f, pdb, va, &mut |_| None),
            Point::Refused("a directory entry is unreadable")
        );
    }

    /// VER3 (Hopper/Blackwell) through its own builders: a 4 KiB vidmem page.
    #[test]
    fn ver3_small_page() {
        let va = 0x7f00_0010_3000u64;
        let mut img = Image::new(0x10_0000);
        let pd4 = img.alloc(2 * 8, 4096);
        let pd3 = img.alloc(512 * 8, 4096);
        let pd2 = img.alloc(512 * 8, 4096);
        let pd1 = img.alloc(512 * 8, 4096);
        let pd0 = img.alloc(256 * 16, 4096);
        let small = img.alloc(512 * 8, 4096);
        let i = synth::ver3::idx(va);
        img.put64(pd4 + 8 * i[0] as u64, synth::ver3::pde(pd3));
        img.put64(pd3 + 8 * i[1] as u64, synth::ver3::pde(pd2));
        img.put64(pd2 + 8 * i[2] as u64, synth::ver3::pde(pd1));
        img.put64(pd1 + 8 * i[3] as u64, synth::ver3::pde(pd0));
        let [lo, hi] = synth::ver3::dual_small(small);
        img.put64(pd0 + 16 * i[4] as u64, lo);
        img.put64(pd0 + 16 * i[4] as u64 + 8, hi);
        img.put64(small + 8 * i[5] as u64, synth::ver3::pte(0x55_5000));
        let f = kf_format_ver3();
        let p = lookup(&f, pd4, va + 7, &mut loader(&img));
        let Point::Leaf(l) = p else { panic!("{p:?}") };
        assert_eq!((l.va, l.addr, l.bytes), (va, 0x55_5000, 0x1000));
    }
}
