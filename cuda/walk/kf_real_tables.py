#!/usr/bin/env python3
"""kf_real_tables.py — ★★★★★ EXTRACT PAGE TABLES A REAL NVIDIA DRIVER WROTE.

⊘⊘⊘ THE GAP THIS CLOSES, w725
============================
All 58 cases in `kf_tests.cu` build their tables with `kf_tables.h` — **our own builder,
encoding our own understanding of VER2**. The differential against the Rust walker does not
help: both decoders share that understanding. A real guest's tables can carry encodings we
never produce, and they do (see THE FINDING below).

WHERE THE REAL TABLES CAME FROM — no new capture was needed
===========================================================
`traces/cap1b_coldboot_hermetic_d6.rec` is a Mode-2 replay capture of a **stock, unpatched
NVIDIA open 580.159.04 guest driver** on a real GA106 (`traces/README.md`). It records every
trapped MMIO access. The driver builds its page tables in two ways, and **both are in the
stream**:

  1. through the **PRAMIN window** — BAR0 `0x700000..0x7FFFFF`, whose framebuffer base is
     latched by a write to `NV_PBUS_BAR0_WINDOW` at BAR0 `0x1700`. This is how the directory
     spine is bootstrapped.
  2. through the **BAR2 self-map** — recorder `bar == 3` (RM's logical BAR2 is PCI BAR3).
     These are *virtual* addresses, and translating them requires walking the very tables
     built in (1).

So the tables are recoverable by replaying the write stream: latch the window, apply PRAMIN
writes to framebuffer addresses, and translate every BAR2 write through the live tables.

★★★ THE SELF-CONSISTENCY PROOF, and it is what makes this trustworthy:
**all 177 856 BAR2 writes translate, with ZERO misses.** If the geometry, the field
positions, or the dual-PDE half order were wrong, translation would fail. It is the tables
validating themselves against 177 856 independent accesses.

★ Independently corroborated: `traces/rpctrace_ga106_boot1.bin` was recorded **inside CPU-RM**
by a different instrument on real hardware, and its `UPDATE_BAR_PDE` RPC (function 70,
`barType=1`, `levelShift=47`) carries `entryValue=0x000000002efbc302` — **bit-identical** to
the root PDE reconstructed here from the MMIO stream.

⚠ SCOPE — what these tables are and are NOT
===========================================
- They are the guest driver's **BAR2 / GSP-bootstrap** address spaces: six of them, 6 998
  valid leaves, 4 KiB + 64 KiB + 2 MiB pages, vidmem and sysmem-coherent apertures.
- ⊘ They are NOT the ~1872-page user/compute working set of `w422`. That was a live count on
  a rented box; the pages themselves were never persisted anywhere in this tree.
- ⊘ Only MMIO *writes* are recorded. A table page written by DMA/CE, or written before the
  recorder armed, is invisible. The full zero→build→teardown lifecycle of *these* tables is
  present, which is why a peak snapshot is well defined.
- ⊘ `cap1b` is a hermetic emulator capture, so the *physical addresses* are the emulator's FB
  layout. The **entry encodings are the real driver's** and that is what is under test.

RELOCATION — what is rewritten and what is byte-identical
=========================================================
The tables live across ~12 GiB of framebuffer, which no test buffer can hold. So table
**pages** are relocated into a compact arena and the **address field of directory entries**
(`IS_PTE == 0`) is rewritten to point at the new home.

★ **Every leaf PTE is copied byte for byte, untouched** — including the KIND and COMPTAGLINE
fields that are the whole point. A directory entry's non-address bits (aperture, VOL) are
preserved too; only its address bits move. A leaf's target address is *reported*, never
dereferenced, so it needs no relocation at all.

★★★★★ THE FINDING — real tables carry two fields our builder cannot produce
==========================================================================
Of **6 998** valid leaf PTEs the real driver wrote:

    KIND (bits 63:56)        = 9  on 6 017     = 6 on   959     = 0 on only 22
    COMPTAGLINE (bits 55:36) nonzero on 6 017

`kf_tables.h`'s `kfb_pte()` can only ever set bits 0..7 and the address field. ⇒ **99.7 % of
real leaf entries are encodings no test case in the suite has ever contained.**

⊘ The decode is unaffected — VER2's vidmem address field is 32:8, so KIND and COMPTAGLINE sit
above it and both decoders mask them off correctly. What is affected is **coalescing**: the
kernel's run identity is (contiguous VA, contiguous GPGA, equal decoded flags), and the
decoded flags carry no KIND. Measured on the real 12 GiB VAS at root `0x2efa6c000`:

    runs emitted (KIND-blind, what the kernel does):   1
    runs if KIND/COMPTAGLINE were part of the identity: 3

with two boundaries at which the guest changed memory kind across contiguous VA *and*
contiguous physical addresses under identical permissions. **The report cannot express that
the guest asked for two different memory kinds, and merges across the boundary silently.**
That is very likely the right call for kayfabe — KIND is how the *GPU* interprets bytes, not
where they live, and the host publishes addresses — but nothing in the suite or the format doc
decides it. It was a blind spot, not a decision.

Everything else matched: the sparse encoding is exactly `VOL` with `VALID` clear (the builder's
`kfb_sparse_pte()`, 8 358 occurrences), the dual PDE's big half is the LOW word and the small
half the HIGH word, and every permission-bit combination the driver used is one the builder
can already spell.

USAGE
=====
    python3 kf_real_tables.py [--rec traces/cap1b_coldboot_hermetic_d6.rec]
                              [--out corpus/real_ga106.bin]

Writes a `KFCORPUS`-format file (the same format `kf_corpus.cpp` writes), so both the CUDA
kernel and `crates/kayfabe-mmu/tests/walk_kernel_differential.rs` consume it unchanged.
"""

import argparse
import collections
import os
import struct
import sys

# ── the .rec format (crates/kayfabe-crec/src/format.rs) ────────────────────────────────────
REC_ENTRY = 32
KIND_MMIO_WRITE = 2

# ── PRAMIN (ogkm `dev_bus.h`: NV_PBUS_BAR0_WINDOW) ─────────────────────────────────────────
BAR0_WINDOW_REG = 0x1700
PRAMIN_LO, PRAMIN_HI = 0x700000, 0x800000

# ── GA10x VER2 geometry. ⊘ Duplicated from kf_tables.h ON PURPOSE: this script is an
# independent instrument, and one that shared the builder's helpers would agree with it by
# construction. Any disagreement shows up as an unresolvable BAR2 write. ──────────────────
def pde_child(e):
    ap = (e >> 1) & 3
    return 0 if ap == 0 else ((e >> 8) & ((1 << (25 if ap == 1 else 46)) - 1)) << 12


def big_child(e):
    ap = (e >> 1) & 3
    return 0 if ap == 0 else ((e >> 4) & ((1 << (29 if ap == 1 else 50)) - 1)) << 8


def pte_addr(e):
    ap = (e >> 1) & 3
    return ((e >> 8) & ((1 << (25 if ap <= 1 else 46)) - 1)) << 12


def set_pde_child(e, child):
    """Rewrite ONLY the address bits of a directory entry, preserving every other bit."""
    ap = (e >> 1) & 3
    bits = 25 if ap == 1 else 46
    mask = ((1 << bits) - 1) << 8
    return (e & ~mask) | (((child >> 12) & ((1 << bits) - 1)) << 8)


def set_big_child(e, child):
    ap = (e >> 1) & 3
    bits = 29 if ap == 1 else 50
    mask = ((1 << bits) - 1) << 4
    return (e & ~mask) | (((child >> 8) & ((1 << bits) - 1)) << 4)


class Fb:
    """The reconstructed framebuffer, as 32-bit words. Sparse: only what was written."""

    def __init__(self):
        self.w = {}

    def rd64(self, p):
        return (self.w.get(p + 4, 0) << 32) | self.w.get(p, 0)

    def write(self, p, v, width):
        if width == 8:
            self.w[p] = v & 0xFFFFFFFF
            self.w[p + 4] = (v >> 32) & 0xFFFFFFFF
        elif width == 4:
            self.w[p] = v & 0xFFFFFFFF
        elif width in (1, 2):
            n = 8 * width
            sh = (p & 3) * 8
            cur = self.w.get(p & ~3, 0)
            self.w[p & ~3] = (cur & ~(((1 << n) - 1) << sh) | ((v & ((1 << n) - 1)) << sh)) & 0xFFFFFFFF


def replay(path, bar2_root):
    """Replay the MMIO write stream. Returns (fb at peak, stats)."""
    blob = open(path, "rb").read()
    (hdr_len,) = struct.unpack_from("<I", blob, 12)
    fb, win, off, n = Fb(), None, hdr_len, 0
    pramin = resolved = unresolved = 0
    best, best_live = None, -1

    def translate(va):
        """BAR2 VA -> fb address through the live tables. None on a miss."""
        e = fb.rd64(bar2_root + ((va >> 47) & 3) * 8)
        if (e & 1) or not ((e >> 1) & 3):
            return None
        e = fb.rd64(pde_child(e) + ((va >> 38) & 511) * 8)
        if (e & 1) or not ((e >> 1) & 3):
            return None
        e = fb.rd64(pde_child(e) + ((va >> 29) & 511) * 8)
        if e & 1:
            return pte_addr(e) + (va & ((1 << 29) - 1))
        if not ((e >> 1) & 3):
            return None
        base = pde_child(e) + ((va >> 21) & 255) * 16
        lo, hi = fb.rd64(base), fb.rd64(base + 8)
        if lo & 1:
            return pte_addr(lo) + (va & ((1 << 21) - 1))
        if hi and ((hi >> 1) & 3):
            e = fb.rd64(pde_child(hi) + ((va >> 12) & 511) * 8)
            if e & 1:
                return pte_addr(e) + (va & 0xFFF)
        if lo and ((lo >> 1) & 3):
            e = fb.rd64(big_child(lo) + ((va >> 16) & 31) * 8)
            if e & 1:
                return pte_addr(e) + (va & 0xFFFF)
        return None

    while off + REC_ENTRY <= len(blob):
        kind, width, bar = blob[off + 8], blob[off + 9], blob[off + 10]
        (ln,) = struct.unpack_from("<I", blob, off + 12)
        a, b = struct.unpack_from("<QQ", blob, off + 16)
        if kind == KIND_MMIO_WRITE:
            if bar == 0:
                if a == BAR0_WINDOW_REG:
                    win = b
                elif PRAMIN_LO <= a < PRAMIN_HI and win is not None:
                    fb.write(((win & 0xFFFFFF) << 16) | (a - PRAMIN_LO), b, width)
                    pramin += 1
            elif bar == 3:
                p = translate(a)
                if p is None:
                    unresolved += 1
                else:
                    fb.write(p, b, width)
                    resolved += 1
        n += 1
        # ⚠ The driver ZEROES the whole hierarchy at teardown, so the FINAL state is empty.
        # Snapshot at peak population instead, sampled cheaply.
        if n % 10000 == 0:
            live = sum(1 for v in fb.w.values() if v)
            if live > best_live:
                best_live, best = live, dict(fb.w)
        off += REC_ENTRY + ln + ((8 - (ln & 7)) & 7)

    out = Fb()
    out.w = best if best is not None else fb.w
    return out, dict(records=n, pramin=pramin, resolved=resolved, unresolved=unresolved)


def find_roots(fb):
    """Table pages nothing points at. ⊘ Derived from the data, not from a list someone typed."""
    byp = collections.defaultdict(dict)
    for a, v in fb.w.items():
        if v:
            byp[a & ~0xFFF][a & 0xFFF] = v

    def entries(base):
        pg = byp.get(base, {})
        return [
            (o, (pg.get(o + 4, 0) << 32) | pg.get(o, 0))
            for o in range(0, 4096, 8)
            if pg.get(o, 0) or pg.get(o + 4, 0)
        ]

    tables = {b for b in byp if any((e & 1) or ((e >> 1) & 3) for _, e in entries(b))}
    children = set()
    for b in tables:
        for _, e in entries(b):
            if not (e & 1) and ((e >> 1) & 3):
                for c in (pde_child(e), big_child(e) & ~0xFFF):
                    if c in byp:
                        children.add(c)
    return sorted(tables - children)


class Arena:
    """The compact relocation target. Offset 0 is never handed out — a zero child pointer
    means 'no sub-table' to both walkers."""

    def __init__(self):
        self.mem = bytearray(4096)
        self.map = {}

    def place(self, src, size, align):
        if src in self.map:
            return self.map[src]
        off = (len(self.mem) + align - 1) & ~(align - 1)
        self.mem.extend(b"\0" * (off + size - len(self.mem)))
        self.map[src] = off
        return off

    def put64(self, off, v):
        struct.pack_into("<Q", self.mem, off, v)

    def get64(self, off):
        return struct.unpack_from("<Q", self.mem, off)[0]


def relocate(fb, root):
    """Copy one address space's tables into an arena, rewriting only directory addresses.

    ★ Leaf PTEs are copied byte for byte — KIND, COMPTAGLINE and all.
    """
    a = Arena()
    stats = collections.Counter()

    def copy_page(src, size):
        dst = a.place(src, size, 4096 if size >= 4096 else 256)
        for o in range(0, size, 8):
            a.put64(dst + o, fb.rd64(src + o))
        return dst

    def fix(tbl_dst, idx, stride, half, child_src, child_size, big):
        """Point entry `idx` (word `half`) of the relocated table at the relocated child."""
        at = tbl_dst + idx * stride + half
        e = a.get64(at)
        dst = copy_page(child_src, child_size)
        a.put64(at, set_big_child(e, dst) if big else set_pde_child(e, dst))
        return dst

    root_dst = copy_page(root, 4096)
    for i3 in range(4):
        e3 = fb.rd64(root + i3 * 8)
        if not e3 or (e3 & 1) or not ((e3 >> 1) & 3):
            continue
        d2 = fix(root_dst, i3, 8, 0, pde_child(e3), 4096, False)
        src2 = pde_child(e3)
        for i2 in range(512):
            e2 = fb.rd64(src2 + i2 * 8)
            if not e2 or (e2 & 1) or not ((e2 >> 1) & 3):
                continue
            src1 = pde_child(e2)
            d1 = fix(d2, i2, 8, 0, src1, 4096, False)
            for i1 in range(512):
                e1 = fb.rd64(src1 + i1 * 8)
                if not e1:
                    continue
                if e1 & 1:
                    stats["leaf512M"] += 1
                    continue          # a 512 MiB leaf: untouched
                if not ((e1 >> 1) & 3):
                    continue
                src0 = pde_child(e1)
                d0 = fix(d1, i1, 8, 0, src0, 4096, False)
                for i0 in range(256):
                    lo, hi = fb.rd64(src0 + i0 * 16), fb.rd64(src0 + i0 * 16 + 8)
                    if lo & 1:
                        stats["leaf2M"] += 1   # a 2 MiB leaf in the LOW half: untouched
                    elif lo and ((lo >> 1) & 3):
                        fix(d0, i0, 16, 0, big_child(lo), 256, True)
                        stats["bigPT"] += 1
                    if hi and ((hi >> 1) & 3):
                        fix(d0, i0, 16, 8, pde_child(hi), 4096, False)
                        stats["smallPT"] += 1
    return a, root_dst, stats


def count_leaves(a, root):
    """Walk the relocated image the way both decoders will, purely as a census."""
    c = collections.Counter()
    kinds = collections.Counter()

    def rd(p):
        return a.get64(p)

    for i3 in range(4):
        e3 = rd(root + i3 * 8)
        if not e3 or (e3 & 1) or not ((e3 >> 1) & 3):
            continue
        t2 = pde_child(e3)
        for i2 in range(512):
            e2 = rd(t2 + i2 * 8)
            if not e2 or (e2 & 1) or not ((e2 >> 1) & 3):
                continue
            t1 = pde_child(e2)
            for i1 in range(512):
                e1 = rd(t1 + i1 * 8)
                if not e1:
                    continue
                if e1 & 1:
                    c["512M"] += 1
                    kinds[(e1 >> 56) & 0xFF] += 1
                    continue
                if not ((e1 >> 1) & 3):
                    continue
                t0 = pde_child(e1)
                for i0 in range(256):
                    lo, hi = rd(t0 + i0 * 16), rd(t0 + i0 * 16 + 8)
                    if lo & 1:
                        c["2M"] += 1
                        kinds[(lo >> 56) & 0xFF] += 1
                    elif lo and ((lo >> 1) & 3):
                        pb = big_child(lo)
                        for ib in range(32):
                            e = rd(pb + ib * 8)
                            if e & 1:
                                c["64K"] += 1
                                kinds[(e >> 56) & 0xFF] += 1
                    if hi and ((hi >> 1) & 3):
                        ps = pde_child(hi)
                        for i in range(512):
                            e = rd(ps + i * 8)
                            if e & 1:
                                c["4K"] += 1
                                kinds[(e >> 56) & 0xFF] += 1
    return c, kinds


def write_corpus(path, images):
    """The `KFCORPUS` format of kf_corpus.cpp, byte for byte."""
    with open(path, "wb") as f:
        f.write(b"KFCORPUS")
        f.write(struct.pack("<I", len(images)))
        pages = 0
        for name, benign, mem, root in images:
            f.write(struct.pack("<I", len(name)))
            f.write(name.encode())
            f.write(bytes([benign]))
            f.write(struct.pack("<QQ", len(mem), root))
            offs = [o for o in range(0, len(mem) - 4095, 4096) if any(mem[o : o + 4096])]
            f.write(struct.pack("<I", len(offs)))
            for o in offs:
                f.write(struct.pack("<Q", o))
                f.write(bytes(mem[o : o + 4096]))
            pages += len(offs)
            print(f"  {name:<28} benign={benign} len={len(mem)} root=0x{root:x} pages={len(offs)}")
        print(f"wrote {path}: {len(images)} images, {pages} pages")


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    repo = os.path.abspath(os.path.join(here, "..", ".."))
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--rec", default=os.path.join(repo, "traces", "cap1b_coldboot_hermetic_d6.rec"))
    ap.add_argument("--out", default=os.path.join(here, "corpus", "real_ga106.bin"))
    ap.add_argument("--bar2-root", type=lambda s: int(s, 0), default=0x2EFBC2000,
                    help="the BAR2 PDB, corroborated by UPDATE_BAR_PDE in rpctrace_ga106_boot1.bin")
    args = ap.parse_args()

    if not os.path.exists(args.rec):
        sys.exit(f"{args.rec}: not found")
    print(f"replaying {args.rec}")
    fb, st = replay(args.rec, args.bar2_root)
    print("  records={records} pramin_writes={pramin} bar2_resolved={resolved} "
          "bar2_UNRESOLVED={unresolved}".format(**st))
    # ★★★ The self-consistency proof. A single miss means the geometry is wrong somewhere.
    if st["unresolved"] != 0:
        sys.exit(f"⊘ {st['unresolved']} BAR2 writes did not translate -- the reconstruction is "
                 f"NOT self-consistent and nothing below can be trusted")
    if st["resolved"] == 0:
        sys.exit("⊘ zero BAR2 writes translated -- that is a broken replay, not an empty trace")

    roots = find_roots(fb)
    print(f"  {len(roots)} address spaces: " + " ".join(f"0x{r:x}" for r in roots))

    images, total, allkinds = [], collections.Counter(), collections.Counter()
    for r in roots:
        a, root_dst, _ = relocate(fb, r)
        c, kinds = count_leaves(a, root_dst)
        if not c:
            print(f"  skipping 0x{r:x}: no valid leaves")
            continue
        total += c
        allkinds += kinds
        images.append((f"real_ga106_vas_{r:x}", 1, a.mem, root_dst))
        print(f"  0x{r:x}: {sum(c.values())} leaves {dict(c)} arena={len(a.mem)}B")

    if not images:
        sys.exit("⊘ no address space produced any leaf -- refusing to write an empty corpus")
    print(f"\ntotal leaves: {sum(total.values())} {dict(total)}")
    print(f"KIND census (bits 63:56): {dict(allkinds)}")
    nonzero = sum(v for k, v in allkinds.items() if k)
    print(f"★ {nonzero}/{sum(allkinds.values())} leaves carry a NON-ZERO KIND -- an encoding "
          f"kf_tables.h cannot produce")

    os.makedirs(os.path.dirname(args.out), exist_ok=True)
    write_corpus(args.out, images)


if __name__ == "__main__":
    main()
