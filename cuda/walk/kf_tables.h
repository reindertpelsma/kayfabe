/* kf_tables.h — a HOST-side builder for GA10x VER2 page tables inside a flat
 * buffer standing in for GPGA. Test scaffolding only; nothing here runs on the
 * device.
 *
 * ⚠ The builder is deliberately INDEPENDENT of the kernel's decoder: it writes
 * the raw bits from the dev_mmu.h field definitions and never calls a kernel
 * helper. A builder that shared the decoder would agree with it by construction
 * and prove nothing.
 */
#ifndef KF_TABLES_H
#define KF_TABLES_H

#include <stdint.h>
#include <string.h>
#include <stdio.h>
#include <stdlib.h>
#include <vector>

/* NV_MMU_VER2_PTE_* bits */
#define PTE_VALID          (1ull << 0)
#define PTE_VOL            (1ull << 3)
#define PTE_ENCRYPTED      (1ull << 4)
#define PTE_PRIVILEGE      (1ull << 5)
#define PTE_READ_ONLY      (1ull << 6)
#define PTE_ATOMIC_DISABLE (1ull << 7)

/* aperture nibbles: PTE 0=vid 1=peer 2=syscoh 3=sysnoncoh
 *                   PDE 0=INVALID 1=vid 2=syscoh 3=sysnoncoh              */
#define AP_PTE_VID   0u
#define AP_PTE_PEER  1u
#define AP_PTE_SCOH  2u
#define AP_PTE_SNC   3u
#define AP_PDE_VID   1u
#define AP_PDE_SCOH  2u
#define AP_PDE_SNC   3u

static inline uint64_t kfb_mask(uint32_t bits) { return (1ull << bits) - 1ull; }

/* NV_MMU_VER2_PDE: aperture 2:1, ADDRESS_VID 32:8 (shift 12). */
static inline uint64_t kfb_pde(uint64_t child, uint32_t ap = AP_PDE_VID)
{
    uint32_t bits = (ap == AP_PDE_VID) ? 25u : 46u;
    return ((uint64_t)ap << 1) | (((child >> 12) & kfb_mask(bits)) << 8);
}
/* NV_MMU_VER2_DUAL_PDE big half: aperture 2:1, ADDRESS_BIG_VID 32:4, SHIFT 8. */
static inline uint64_t kfb_big_pde(uint64_t child, uint32_t ap = AP_PDE_VID)
{
    uint32_t bits = (ap == AP_PDE_VID) ? 29u : 50u;
    return ((uint64_t)ap << 1) | (((child >> 8) & kfb_mask(bits)) << 4);
}
/* NV_MMU_VER2_PTE: valid 0, aperture 2:1, ADDRESS_VID 32:8 (shift 12). */
static inline uint64_t kfb_pte(uint64_t phys, uint32_t ap = AP_PTE_VID, uint64_t bits = 0)
{
    uint32_t w = (ap <= AP_PTE_PEER) ? 25u : 46u;
    return PTE_VALID | ((uint64_t)ap << 1) | (((phys >> 12) & kfb_mask(w)) << 8) | bits;
}
/* The SPARSE encoding: valid clear, volatile set (kern_gmmu_gm200.c:46-70). */
static inline uint64_t kfb_sparse_pte(void) { return PTE_VOL; }
static inline uint64_t kfb_sparse_pde(void) { return PTE_VOL; }

static inline uint64_t kfb_pde_child(uint64_t e)
{
    uint32_t ap = (uint32_t)((e >> 1) & 3u);
    uint32_t bits = (ap == 1u) ? 25u : 46u;
    return ((e >> 8) & kfb_mask(bits)) << 12;
}
static inline uint64_t kfb_big_pde_child(uint64_t e)
{
    uint32_t ap = (uint32_t)((e >> 1) & 3u);
    uint32_t bits = (ap == 1u) ? 29u : 50u;
    return ((e >> 4) & kfb_mask(bits)) << 8;
}

struct Gpga {
    std::vector<uint8_t> mem;
    uint64_t bump;

    explicit Gpga(size_t bytes) : mem(bytes, 0), bump(4096) {}

    /* ⊘ Offset 0 is never handed out: the walker treats a zero child pointer as
     * "no sub-table", matching kayfabe-mmu/src/walker.rs (`if e.next != 0`) and
     * the C (`nvkvm_gpu_emul.c:8615`). */
    uint64_t alloc(uint64_t bytes, uint64_t align)
    {
        bump = (bump + align - 1ull) & ~(align - 1ull);
        uint64_t o = bump;
        bump += bytes;
        if (bump > mem.size()) { fprintf(stderr, "Gpga::alloc overflow\n"); abort(); }
        return o;
    }
    uint64_t &u64(uint64_t off)
    {
        if (off + 8 > mem.size()) { fprintf(stderr, "Gpga::u64 oob %llu\n", (unsigned long long)off); abort(); }
        return *(uint64_t *)(mem.data() + off);
    }
    size_t size() const { return mem.size(); }
};

/* Indices of a VA at each GA10x level. */
static inline uint32_t vi3(uint64_t va) { return (uint32_t)((va >> 47) & 3ull); }
static inline uint32_t vi2(uint64_t va) { return (uint32_t)((va >> 38) & 511ull); }
static inline uint32_t vi1(uint64_t va) { return (uint32_t)((va >> 29) & 511ull); }
static inline uint32_t vi0(uint64_t va) { return (uint32_t)((va >> 21) & 255ull); }
static inline uint32_t vib(uint64_t va) { return (uint32_t)((va >> 16) & 31ull); }
static inline uint32_t vis(uint64_t va) { return (uint32_t)((va >> 12) & 511ull); }

struct Tree {
    Gpga *g;
    uint64_t root;

    explicit Tree(Gpga &gg) : g(&gg) { root = g->alloc(4096, 4096); }
    Tree(Gpga &gg, uint64_t r) : g(&gg), root(r) {}

    uint64_t child(uint64_t tbl, uint32_t idx, uint64_t bytes, uint64_t align, bool make)
    {
        uint64_t &e = g->u64(tbl + (uint64_t)idx * 8);
        if (((e >> 1) & 3ull) != 0ull && !(e & PTE_VALID)) return kfb_pde_child(e);
        if (!make) return 0;
        uint64_t c = g->alloc(bytes, align);
        e = kfb_pde(c);
        return c;
    }
    uint64_t pd2(uint64_t va, bool make = true) { return child(root, vi3(va), 4096, 4096, make); }
    uint64_t pd1(uint64_t va, bool make = true)
    { uint64_t t = pd2(va, make); return t ? child(t, vi2(va), 4096, 4096, make) : 0; }
    uint64_t pd0(uint64_t va, bool make = true)
    { uint64_t t = pd1(va, make); return t ? child(t, vi1(va), 4096, 4096, make) : 0; }

    uint64_t pts(uint64_t va, bool make = true)   /* the SMALL half lives in the HIGH word */
    {
        uint64_t t = pd0(va, make);
        if (!t) return 0;
        uint64_t &hi = g->u64(t + (uint64_t)vi0(va) * 16 + 8);
        if (((hi >> 1) & 3ull) != 0ull) return kfb_pde_child(hi);
        if (!make) return 0;
        uint64_t c = g->alloc(4096, 4096);
        hi = kfb_pde(c);
        return c;
    }
    uint64_t ptb(uint64_t va, bool make = true)   /* the BIG half lives in the LOW word */
    {
        uint64_t t = pd0(va, make);
        if (!t) return 0;
        uint64_t &lo = g->u64(t + (uint64_t)vi0(va) * 16);
        if (lo & PTE_VALID) { fprintf(stderr, "ptb: PD0 slot is a 2M leaf\n"); abort(); }
        if (((lo >> 1) & 3ull) != 0ull) return kfb_big_pde_child(lo);
        if (!make) return 0;
        uint64_t c = g->alloc(256, 256);
        lo = kfb_big_pde(c);
        return c;
    }

    void map4k(uint64_t va, uint64_t gpga, uint32_t ap = AP_PTE_VID, uint64_t bits = 0)
    { g->u64(pts(va) + (uint64_t)vis(va) * 8) = kfb_pte(gpga, ap, bits); }
    void map64k(uint64_t va, uint64_t gpga, uint32_t ap = AP_PTE_VID, uint64_t bits = 0)
    { g->u64(ptb(va) + (uint64_t)vib(va) * 8) = kfb_pte(gpga, ap, bits); }
    void map2m(uint64_t va, uint64_t gpga, uint32_t ap = AP_PTE_VID, uint64_t bits = 0)
    { g->u64(pd0(va) + (uint64_t)vi0(va) * 16) = kfb_pte(gpga, ap, bits); }
    void map512m(uint64_t va, uint64_t gpga, uint32_t ap = AP_PTE_VID, uint64_t bits = 0)
    { g->u64(pd1(va) + (uint64_t)vi1(va) * 8) = kfb_pte(gpga, ap, bits); }

    void unmap4k(uint64_t va) { g->u64(pts(va) + (uint64_t)vis(va) * 8) = 0; }

    /* Raw pokes, for the hostile corpus. */
    void poke_pd3(uint32_t i, uint64_t v) { g->u64(root + (uint64_t)i * 8) = v; }
    void poke(uint64_t tbl, uint32_t i, uint64_t v) { g->u64(tbl + (uint64_t)i * 8) = v; }
};

#endif /* KF_TABLES_H */
