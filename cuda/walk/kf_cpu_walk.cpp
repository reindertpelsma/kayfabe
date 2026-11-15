/* kf_cpu_walk.cpp — the walk kernel's SOURCE, run on a CPU, against the committed corpus (2026-11-13).
 *
 * STATUS: LIVE. `make check-cpu` (g++ only, no GPU, no CUDA SDK). Built by including kf_walk.cu's
 * device half (`KF_DEVICE_ONLY`) under kf_cpu_shim.h, so every decision below is made by the same
 * functions NVRTC compiles into the shipped kf_walk.ptx.
 *
 *   ./kf_cpu_walk            the differential: the serial walk of every image of corpus/corpus.bin
 *                            and corpus/real_ga106.bin against corpus/rust_leaves.txt / real_leaves.txt
 *                            (the CPU reference decoder's output, crates/kf-cuda/tests/gmmu_ver2), then
 *                            the named cases below. Exit 0 only if all agree.
 *   ./kf_cpu_walk --dump     every image's runs, bit for bit (va gpga len flags op) plus its refusal
 *                            mask and sparse count: the input to an old-vs-new identity diff of a
 *                            kernel change (`diff <(old --dump) <(new --dump)` must be empty).
 *
 * ⚠ The PARALLEL walk (production) is not run here; kf_tests.cu grades it on a GPU, against the same
 * corpus. The two share every decode function, which is what this file pins.
 */
#include "kf_cpu_shim.h"
#define KF_DEVICE_ONLY 1
#include "kf_walk.cu"
#include "kf_tables.h"   /* the host VER2 table builder (independent of the decoder) */

#include <stdio.h>
#include <stdlib.h>
#include <algorithm>
#include <string>
#include <vector>

/* The one `extern __shared__` array the (never-called) warp kernels name. */
uint64_t shmem[1];

static int g_fail = 0;
#define EXPECT(c, ...) do { if (!(c)) { g_fail++; printf("  FAIL %s:%d %s: ", __FILE__, __LINE__, #c); \
    printf(__VA_ARGS__); printf("\n"); } } while (0)

struct Leaf {
    uint64_t va, gpga, size;
    uint32_t ap, ro;
    bool operator<(const Leaf &o) const
    {
        if (va != o.va) return va < o.va;
        if (size != o.size) return size < o.size;
        if (gpga != o.gpga) return gpga < o.gpga;
        if (ap != o.ap) return ap < o.ap;
        return ro < o.ro;
    }
    bool operator==(const Leaf &o) const
    { return va == o.va && gpga == o.gpga && size == o.size && ap == o.ap && ro == o.ro; }
};

struct Img {
    std::string name;
    bool benign;
    uint64_t gpga_len, root;
    std::vector<uint8_t> mem;
    std::vector<Leaf> ref;
};

/* The corpus format of kf_corpus.cpp / kf_real_tables.py (the same reader as kf_tests.cu's d_load). */
static bool load(std::vector<Img> &out, const char *corpus, const char *leaves)
{
    FILE *f = fopen(corpus, "rb");
    if (!f) { printf("%s missing\n", corpus); return false; }
    char magic[8];
    uint32_t n = 0;
    if (fread(magic, 1, 8, f) != 8 || memcmp(magic, "KFCORPUS", 8) || fread(&n, 4, 1, f) != 1) { fclose(f); return false; }
    for (uint32_t i = 0; i < n; i++) {
        Img im;
        uint32_t nl = 0, np = 0;
        uint8_t b = 0;
        if (fread(&nl, 4, 1, f) != 1 || nl > 4096) { fclose(f); return false; }
        im.name.resize(nl);
        if (fread(&im.name[0], 1, nl, f) != nl || fread(&b, 1, 1, f) != 1) { fclose(f); return false; }
        im.benign = b != 0;
        if (fread(&im.gpga_len, 8, 1, f) != 1 || fread(&im.root, 8, 1, f) != 1 || fread(&np, 4, 1, f) != 1) { fclose(f); return false; }
        if (im.gpga_len > (1ull << 32)) { fclose(f); return false; }
        im.mem.assign((size_t)im.gpga_len, 0);
        for (uint32_t k = 0; k < np; k++) {
            uint64_t off = 0;
            if (fread(&off, 8, 1, f) != 1 || off + 4096 > im.gpga_len) { fclose(f); return false; }
            if (fread(im.mem.data() + off, 1, 4096, f) != 4096) { fclose(f); return false; }
        }
        out.push_back(im);
    }
    fclose(f);
    FILE *e = fopen(leaves, "r");
    if (!e) { printf("%s missing\n", leaves); return false; }
    char line[256];
    int cur = -1;
    while (fgets(line, sizeof(line), e)) {
        if (line[0] == '#') continue;
        if (!strncmp(line, "image ", 6)) {
            char nm[128];
            int bg = 0, cnt = 0;
            if (sscanf(line + 6, "%127s %d %d", nm, &bg, &cnt) != 3) { fclose(e); return false; }
            cur = -1;
            for (size_t i = 0; i < out.size(); i++) if (out[i].name == nm) cur = (int)i;
            if (cur < 0) { fclose(e); return false; }
        } else if (!strncmp(line, "leaf ", 5) && cur >= 0) {
            unsigned long long a, b2, c2;
            unsigned d2, e2;
            if (sscanf(line + 5, "%llx %llx %llx %u %u", &a, &b2, &c2, &d2, &e2) != 5) { fclose(e); return false; }
            Leaf l = {a, b2, c2, d2, e2};
            out[(size_t)cur].ref.push_back(l);
        }
    }
    fclose(e);
    for (size_t i = 0; i < out.size(); i++) std::sort(out[i].ref.begin(), out[i].ref.end());
    return true;
}

struct Walked {
    std::vector<KfMapRun> runs;
    uint32_t refuse, refusals, sparse, stop;
    uint64_t visited;
};

/* One serial walk of `root` over `mem`, exactly as kf_walk_kernel sets it up (no samples). */
static Walked walk(const KfFormat &F, const std::vector<uint8_t> &mem, uint64_t root,
                   uint64_t span = KF_GPGA_SPAN_UNBOUNDED, uint64_t budget = 4000000ull)
{
    Walked w;
    w.runs.assign(1u << 16, KfMapRun());
    KfCtx c;
    memset(&c, 0, sizeof(c));
    c.w.base = mem.data();
    c.w.len = mem.size();
    c.w.span = span;
    c.fmt = &F;
    c.budget = budget;
    c.out = w.runs.data();
    c.cap = (uint32_t)w.runs.size();
    c.pdb_index = 0;
    c.sdev = NULL;
    kf_walk_one(c, root);
    w.runs.resize(c.n);
    w.refuse = c.refuse; w.refusals = c.refusals; w.sparse = c.sparse; w.stop = c.stop;
    w.visited = c.visited;
    return w;
}

static std::vector<Leaf> leaves_of(const KfFormat &F, const std::vector<KfMapRun> &runs)
{
    std::vector<Leaf> v;
    for (size_t k = 0; k < runs.size(); k++) {
        const KfMapRun &r = runs[k];
        const uint64_t ps = 1ull << F.ps_log2[(r.flags >> KFWR_RF_PS_SHIFT) & 3u];
        for (uint64_t o = 0; o < r.len; o += ps) {
            Leaf l = {r.va + o, r.gpga + o, ps, r.flags & KFWR_RF_AP_MASK, (r.flags & KFWR_RF_READ_ONLY) ? 1u : 0u};
            v.push_back(l);
        }
    }
    std::sort(v.begin(), v.end());
    return v;
}

static void dump(const char *set, const Img &im, const Walked &w)
{
    printf("image %s %s refuse=%#x refusals=%u sparse=%u stop=%u visited=%llu runs=%zu\n", set, im.name.c_str(),
           w.refuse, w.refusals, w.sparse, w.stop, (unsigned long long)w.visited, w.runs.size());
    for (size_t k = 0; k < w.runs.size(); k++) {
        const KfMapRun &r = w.runs[k];
        printf("run %llx %llx %llx %x %u\n", (unsigned long long)r.va, (unsigned long long)r.gpga,
               (unsigned long long)r.len, r.flags, (unsigned)r.op);
    }
}

/* The differential over one corpus file; returns the number of benign leaves that agreed exactly. */
static unsigned long long differential(const char *set, const char *corpus, const char *leaves, bool do_dump)
{
    std::vector<Img> imgs;
    if (!load(imgs, corpus, leaves)) { g_fail++; printf("  FAIL cannot load %s / %s\n", corpus, leaves); return 0; }
    const KfFormat F = kf_format_ver2();
    unsigned long long agreed = 0, subset = 0;
    for (size_t i = 0; i < imgs.size(); i++) {
        Img &im = imgs[i];
        Walked w = walk(F, im.mem, im.root);
        if (do_dump) { dump(set, im, w); continue; }
        EXPECT(!w.stop, "%s: the corpus must fit (stop=%u)", im.name.c_str(), w.stop);
        std::vector<Leaf> mine = leaves_of(F, w.runs);
        if (im.benign) {
            /* The kernel refuses PEER leaves (w825 A1); the reference decoder reports them. */
            im.ref.erase(std::remove_if(im.ref.begin(), im.ref.end(),
                                        [](const Leaf &l) { return l.ap == KFWR_AP_PEER; }), im.ref.end());
            EXPECT(mine == im.ref, "%s: %zu kernel leaves vs %zu reference leaves (or a leaf differs)",
                   im.name.c_str(), mine.size(), im.ref.size());
            if (mine == im.ref) agreed += mine.size();
        } else {
            for (size_t k = 0; k < mine.size(); k++) {
                const bool in = std::binary_search(im.ref.begin(), im.ref.end(), mine[k]);
                EXPECT(in, "%s: kernel leaf va=%llx outside the reference set", im.name.c_str(),
                       (unsigned long long)mine[k].va);
                if (!in) break;
            }
            subset += mine.size();
        }
    }
    if (!do_dump)
        printf("  [%s] %zu images, %llu benign leaves agreed exactly, %llu hostile leaves inside the reference\n",
               set, imgs.size(), agreed, subset);
    return agreed;
}


/* ══ VER3 (Hopper, Blackwell): the big PTE and the big half of a dual PDE veto the 4 KiB table ══
 * Security review 3 F-1 (S1-60, S1-67), 2026-11-13. The tables are written from the field definitions
 * of ogkm-580.159.04 `hopper/gh100/dev_mmu.h` (crates/kf-chip/data/hwref-580.159.04.tsv), never through
 * a kernel helper, so the builder cannot agree with the decoder by construction:
 *   PDE        APERTURE 2:1 (1 = vidmem), PCF 5:3, ADDRESS 51:12 (shift 12)
 *   DUAL PDE   low word = BIG half: APERTURE 2:1, PCF_BIG 5:3, ADDRESS_BIG 51:8 (shift 8);
 *              high word = SMALL half: APERTURE 66:65, PCF_SMALL 69:67, ADDRESS_SMALL 115:76 (shift 12)
 *   PTE        VALID 0, APERTURE 2:1 (0 = vidmem), PCF 7:3, KIND 11:8, ADDRESS 51:12
 *   PTE PCF    INVALID 0, SPARSE 1, MAPPING_NOWHERE 2, NO_VALID_4KB_PAGE 3
 *   PDE PCF    INVALID_ATS_ALLOWED 0, SPARSE_ATS_ALLOWED 1, INVALID_ATS_NOT_ALLOWED 2, SPARSE_ATS_NOT_ALLOWED 3 */
static const uint64_t V3_ADDR = 0x000FFFFFFFFFF000ull;
static uint64_t v3_pde(uint64_t child) { return (1ull << 1) | (child & V3_ADDR); }
static uint64_t v3_pte(uint64_t phys, uint32_t pcf = 0u) { return 1ull | ((uint64_t)pcf << 3) | (phys & V3_ADDR); }
static uint64_t v3_invalid_pte(uint32_t pcf) { return (uint64_t)pcf << 3; }

struct V3Tree {
    std::vector<uint8_t> mem;
    uint64_t root, pd0, big, small;
    static const uint64_t VA = 3ull << 21;      /* PD4/PD3/PD2/PD1 index 0, PD0 index 3 */
    V3Tree() : mem(1u << 20, 0)
    {
        root = 0x1000; const uint64_t pd3 = 0x2000, pd2 = 0x3000, pd1 = 0x4000;
        pd0 = 0x5000; big = 0x6000; small = 0x7000;
        u64(root) = v3_pde(pd3); u64(pd3) = v3_pde(pd2); u64(pd2) = v3_pde(pd1); u64(pd1) = v3_pde(pd0);
    }
    uint64_t &u64(uint64_t off) { return *(uint64_t *)(mem.data() + off); }
    /* The dual PDE at PD0 index 3: `lo` = big half, `hi` = small half, raw. */
    void dual(uint64_t lo, uint64_t hi) { u64(pd0 + 3u * 16u) = lo; u64(pd0 + 3u * 16u + 8u) = hi; }
    uint64_t big_half(uint32_t pcf = 0u) const { return (1ull << 1) | ((uint64_t)pcf << 3) | (big & 0x000FFFFFFFFFFF00ull); }
    uint64_t small_half(uint32_t pcf = 0u) const { return (1ull << 1) | ((uint64_t)pcf << 3) | (small & V3_ADDR); }
};

static size_t v3_leaves(const V3Tree &t, std::vector<Leaf> *out = NULL)
{
    const KfFormat F = kf_format_ver3_untested();
    Walked w = walk(F, t.mem, t.root);
    std::vector<Leaf> l = leaves_of(F, w.runs);
    if (out) *out = l;
    return l.size();
}

static void ver3_cases(void)
{
    {   /* F-1a: an UNMAPPED big PTE (VALID=0, PCF=NO_VALID_4KB_PAGE) hides the 16 stale 4 KiB PTEs of its slot. */
        V3Tree t;
        t.dual(t.big_half(), t.small_half());
        t.u64(t.big + 0u) = v3_invalid_pte(3u);         /* slot 0: unmapped -- its 4 KiB PTEs are stale */
        t.u64(t.big + 8u) = 0ull;                       /* slot 1: INVALID -- consult the 4 KiB table   */
        for (uint32_t i = 0; i < 32u; i++) t.u64(t.small + 8u * i) = v3_pte((i < 16u ? 0x100000ull : 0x200000ull) + 4096ull * i);
        std::vector<Leaf> l;
        EXPECT(v3_leaves(t, &l) == 16u, "ver3 unmapped big PTE: %zu leaves, want the 16 of slot 1 only", l.size());
        for (size_t k = 0; k < l.size(); k++)
            EXPECT(l[k].va >= V3Tree::VA + 0x10000ull, "ver3 unmapped big PTE: stale leaf va=%llx published", (unsigned long long)l[k].va);
    }
    {   /* F-1a, mixed: MAPPING_NOWHERE in slot 0 keeps its 4 KiB PTEs, NO_VALID_4KB_PAGE in slot 1 hides its own. */
        V3Tree t;
        t.dual(t.big_half(), t.small_half());
        t.u64(t.big + 0u) = v3_invalid_pte(2u);
        t.u64(t.big + 8u) = v3_invalid_pte(3u);
        for (uint32_t i = 0; i < 32u; i++) t.u64(t.small + 8u * i) = v3_pte(0x100000ull + 4096ull * i);
        std::vector<Leaf> l;
        v3_leaves(t, &l);
        EXPECT(l.size() == 16u && l.back().va == V3Tree::VA + 15u * 4096ull,
               "ver3 mixed slots: %zu leaves, want slot 0's 16 only", l.size());
    }
    {   /* F-1a, mixed: a SPARSE slot 0 keeps its 4 KiB PTEs, NO_VALID_4KB_PAGE in slot 1 hides its own. */
        V3Tree t;
        t.dual(t.big_half(), t.small_half());
        t.u64(t.big + 0u) = v3_invalid_pte(1u);
        t.u64(t.big + 8u) = v3_invalid_pte(3u);
        for (uint32_t i = 0; i < 32u; i++) t.u64(t.small + 8u * i) = v3_pte(0x100000ull + 4096ull * i);
        std::vector<Leaf> l;
        v3_leaves(t, &l);
        EXPECT(l.size() == 16u && l.back().va == V3Tree::VA + 15u * 4096ull,
               "ver3 sparse+unmapped slots: %zu leaves, want slot 0's 16 only", l.size());
    }
    {   /* control: a SPARSE (1) big PTE is not "no 4 KiB page below". */
        V3Tree t;
        t.dual(t.big_half(), t.small_half());
        t.u64(t.big) = v3_invalid_pte(1u);
        for (uint32_t i = 0; i < 16u; i++) t.u64(t.small + 8u * i) = v3_pte(0x100000ull + 4096ull * i);
        EXPECT(v3_leaves(t) == 16u, "ver3 sparse big PTE must not veto its 4 KiB PTEs");
    }
    {   /* control: a MAPPING_NOWHERE (2) big PTE is not "no 4 KiB page below". */
        V3Tree t;
        t.dual(t.big_half(), t.small_half());
        t.u64(t.big) = v3_invalid_pte(2u);
        for (uint32_t i = 0; i < 16u; i++) t.u64(t.small + 8u * i) = v3_pte(0x100000ull + 4096ull * i);
        EXPECT(v3_leaves(t) == 16u, "ver3 mapping-nowhere big PTE must not veto its 4 KiB PTEs");
    }
    {   /* control: a VALID big PTE owns its slot (already true before F-1). */
        V3Tree t;
        t.dual(t.big_half(), t.small_half());
        t.u64(t.big) = v3_pte(0x400000ull);
        for (uint32_t i = 0; i < 16u; i++) t.u64(t.small + 8u * i) = v3_pte(0x100000ull + 4096ull * i);
        std::vector<Leaf> l;
        EXPECT(v3_leaves(t, &l) == 1u && l[0].size == 0x10000ull, "ver3 valid big PTE: %zu leaves, want one 64 KiB", l.size());
    }
    {   /* F-1b: an INVALID big half whose PCF says SPARSE vetoes the whole small table, for BOTH ATS settings,
         * and is counted as a sparse slot. */
        V3Tree t;
        t.dual(1ull << 3, t.small_half());          /* aperture INVALID, PCF_BIG = SPARSE_ATS_ALLOWED */
        for (uint32_t i = 0; i < 4u; i++) t.u64(t.small + 8u * i) = v3_pte(0x100000ull + 4096ull * i);
        EXPECT(v3_leaves(t) == 0u, "ver3 sparse big half (PCF_BIG=1) must hide the small table");
        t.dual(3ull << 3, t.small_half());          /* PCF_BIG = SPARSE_ATS_NOT_ALLOWED */
        EXPECT(v3_leaves(t) == 0u, "ver3 sparse big half (PCF_BIG=3) must hide the small table");
        const Walked w = walk(kf_format_ver3_untested(), t.mem, t.root);
        EXPECT(w.sparse == 1u, "ver3 sparse big half (PCF_BIG=3) is one sparse slot, got %u", w.sparse);
    }
    {   /* control: INVALID_ATS_NOT_ALLOWED (2) is not sparse: the small table is walked. */
        V3Tree t;
        t.dual(2ull << 3, t.small_half());
        for (uint32_t i = 0; i < 4u; i++) t.u64(t.small + 8u * i) = v3_pte(0x100000ull + 4096ull * i);
        EXPECT(v3_leaves(t) == 4u, "ver3 invalid (not sparse) big half must not hide the small table");
    }
}

/* ══ VER2: the vetoes F-1 must leave exactly as they were ══ (kf_tables.h builds the tables) */
static size_t v2_leaves(Gpga &g, uint64_t root)
{
    const KfFormat F = kf_format_ver2();
    return leaves_of(F, walk(F, g.mem, root).runs).size();
}

static void ver2_cases(void)
{
    const uint64_t VA = (1ull << 47) | (3ull << 38) | (7ull << 29) | (5ull << 21);
    {   /* an UNMAPPED big PTE (VALID=0 VOL=0 PRIV=1, 0x20) hides the 16 stale 4 KiB PTEs of slot 0 only */
        Gpga g(8u << 20);
        Tree t(g);
        for (uint32_t i = 0; i < 17u; i++) t.map4k(VA + 4096ull * i, 0x300000ull + 4096ull * i);
        g.u64(t.ptb(VA)) = 0x20ull;
        EXPECT(v2_leaves(g, t.root) == 1u, "ver2 unmapped big PTE must hide slot 0's 16 4 KiB PTEs");
    }
    {   /* a SPARSE big PTE (VALID=0 VOL=1 PRIV=1, 0x28) is not unmapped: the 4 KiB PTEs stand */
        Gpga g(8u << 20);
        Tree t(g);
        for (uint32_t i = 0; i < 16u; i++) t.map4k(VA + 4096ull * i, 0x300000ull + 4096ull * i);
        g.u64(t.ptb(VA)) = 0x28ull;
        EXPECT(v2_leaves(g, t.root) == 16u, "ver2 sparse big PTE must not hide its 4 KiB PTEs");
    }
    {   /* a sparse big HALF (dual PDE low word VALID=0 VOL=1, aperture INVALID) hides the small table */
        Gpga g(8u << 20);
        Tree t(g);
        for (uint32_t i = 0; i < 4u; i++) t.map4k(VA + 4096ull * i, 0x300000ull + 4096ull * i);
        g.u64(t.pd0(VA) + (uint64_t)vi0(VA) * 16u) = PTE_VOL;
        EXPECT(v2_leaves(g, t.root) == 0u, "ver2 sparse big half must hide the small table");
        EXPECT(walk(kf_format_ver2(), g.mem, t.root).sparse == 1u, "ver2 sparse big half is one sparse slot");
    }
    {   /* mixed: a SPARSE slot 0 keeps its 4 KiB PTEs, an UNMAPPED slot 1 hides its own */
        Gpga g(8u << 20);
        Tree t(g);
        for (uint32_t i = 0; i < 32u; i++) t.map4k(VA + 4096ull * i, 0x300000ull + 4096ull * i);
        g.u64(t.ptb(VA)) = 0x28ull;
        g.u64(t.ptb(VA) + 8u) = 0x20ull;
        EXPECT(v2_leaves(g, t.root) == 16u, "ver2 sparse+unmapped slots: want slot 0's 16 leaves only");
    }
}

/* ══ THE HOSTILE-DESCRIPTOR CORPUS (review 3 F-10): every break of kf_format_break is refused by the
 * device-side check, on both shipped descriptors; the shipped descriptors themselves pass. ══ */
static void format_cases(void)
{
    EXPECT(kf_dev_format_ok(kf_format_ver2()), "the shipped VER2 descriptor must pass the device check");
    EXPECT(kf_dev_format_ok(kf_format_ver3_untested()), "the shipped VER3 descriptor must pass the device check");
    int n = 0;
    for (int which = 0; which < 1000; which++) {
        KfFormat F = kf_format_ver2();
        if (!kf_format_break(&F, which)) break;
        n++;
        EXPECT(!kf_dev_format_ok(F), "VER2 descriptor break %d accepted by the device check", which);
    }
    for (int which = 0; which < n; which++) {
        KfFormat F = kf_format_ver3_untested();
        kf_format_break(&F, which);
        EXPECT(!kf_dev_format_ok(F), "VER3 descriptor break %d accepted by the device check", which);
    }
    EXPECT(n >= 40, "the hostile-descriptor corpus shrank to %d cases", n);
    printf("  [descriptor] %d hostile descriptors refused on VER2 and VER3\n", n);
}


/* ══ THE REPORT'S OWN CAPACITY (M4, 2026-11-15): kf_diff_emit on staged data ═══════════════════════
 * kf_diff_emit has no barrier and no shared memory, so one CPU thread per block runs the real
 * source. The staged diff of an entry is eight sorted lists [UNMAP c0][MAP c0] ... [UNMAP c3][MAP c3]
 * (kf_diff_slots' output, which only the GPU suite runs); here it is built by hand. */
struct SRun { uint64_t va, len; unsigned cls; bool map; };

struct EmitRig {
    static const uint32_t NP = 3;
    std::vector<KfMapRun> scratch, rrun;
    KfDev dev;
    KfLayout lay;
    KfPdbEntry rpdb[KF_MAX_PDB];
    KfReportHeader hdr;
    uint64_t pdbs[KF_MAX_PDB];
    uint32_t slots[KF_MAX_PDB];
    uint32_t cap_each;
    std::vector<std::vector<KfMapRun>> staged;   /* per entry, in staged order */
    uint32_t np;
    EmitRig(uint32_t cap, uint32_t run_capacity) : cap_each(cap), np(0)
    {
        memset(&dev, 0, sizeof(dev));
        memset(&lay, 0, sizeof(lay));
        memset(rpdb, 0, sizeof(rpdb));
        memset(&hdr, 0, sizeof(hdr));
        dev.run_capacity = run_capacity;
        dev.pdb_capacity = KF_MAX_PDB;
        for (uint32_t t = 0; t < NP; t++) { lay.walk_off[t] = t * cap; lay.walk_cap[t] = cap; pdbs[t] = 0x1000u * (t + 1); slots[t] = t; }
        scratch.assign((size_t)NP * cap * 4u + 8u, KfMapRun());
        rrun.assign((size_t)run_capacity + 8u, KfMapRun());
    }
    /* Stage entry t: lists in staged order, each sorted by VA. */
    void stage(uint32_t t, std::vector<SRun> rs)
    {
        std::stable_sort(rs.begin(), rs.end(), [](const SRun &a, const SRun &b) {
            const unsigned ka = a.cls * 2u + (a.map ? 1u : 0u), kb = b.cls * 2u + (b.map ? 1u : 0u);
            return ka != kb ? ka < kb : a.va < b.va;
        });
        std::vector<KfMapRun> v;
        for (const SRun &r : rs) {
            KfMapRun m;
            memset(&m, 0, sizeof(m));
            m.va = r.va; m.len = r.len; m.gpga = r.map ? 0x100000ull + r.va : 0ull;
            m.flags = (uint32_t)r.cls << KFWR_RF_PS_SHIFT;
            m.op = r.map ? KFWR_OP_MAP : KFWR_OP_UNMAP;
            m.pdb_index = (uint16_t)t;
            v.push_back(m);
        }
        if (staged.size() <= t) staged.resize(t + 1);
        staged[t] = v;
        KfMapRun *stg = scratch.data() + (size_t)lay.walk_off[t] * 4u + lay.walk_cap[t];
        for (size_t i = 0; i < v.size(); i++) stg[i] = v[i];
        dev.diff_count[t] = (uint32_t)v.size();
        if (t + 1 > np) np = t + 1;
    }
    void emit()
    {
        KfArgs a;
        memset(&a, 0, sizeof(a));
        a.dev = &dev; a.npdb = np; a.lay = &lay; a.scratch = scratch.data();
        a.hdr = &hdr; a.rpdb = rpdb; a.rrun = rrun.data(); a.pdbs = pdbs; a.slots = slots;
        for (uint32_t t = 0; t < KF_MAX_PDB; t++) {
            blockIdx.x = t; blockDim.x = 1; threadIdx.x = 0;
            kf_diff_emit(a);
        }
    }
    uint32_t total() const { uint32_t s = 0; for (auto &v : staged) s += (uint32_t)v.size(); return s; }
};

/* ★ M4 CHARACTERIZATION (2026-11-15, written BEFORE the fix; the fix replaces it). `[measured f4a
 * b20fd84b]` one walk diff larger than the report's run capacity (262 144) made kf_diff_emit write
 * flags 0x85 (TRUNCATED | REFUSED | DIFF), refuse_mask RUN_CAP, run_count 0 and no runs — the host's
 * "walk report TRUNCATED (flags=0x85, refuse_mask=0x10, runs 0 of 0 ...)" — and the host does not retry
 * a report-level truncation, so the split was refused and the guest-kernel copy channel marked dead.
 * 1 boot in 23; near misses at 260 446 and 253 996 runs. The same source, on a CPU, at a small capacity. */
static void emit_m4_legacy(void)
{
    {   /* one entry, a diff of twice the capacity: the refusal */
        EmitRig r(1024, 100);
        std::vector<SRun> v;
        for (uint32_t i = 0; i < 200; i++) v.push_back({0x1000000ull + 0x2000ull * i, 0x1000ull, 0, true});
        r.stage(0, v);
        r.emit();
        EXPECT(r.hdr.flags == (KFWR_HF_TRUNCATED | KFWR_HF_REFUSED | KFWR_HF_DIFF), "M4: flags %#x, want 0x85", r.hdr.flags);
        EXPECT(r.hdr.refuse_mask == KFWR_R_RUN_CAP && r.hdr.refusals == 1, "M4: refuse_mask %#x refusals %u", r.hdr.refuse_mask, r.hdr.refusals);
        EXPECT(r.hdr.run_count == 0 && r.rpdb[0].run_count == 0, "M4: runs 0 of 0 (the diff is handed out as NOTHING)");
        EXPECT(r.rpdb[0].reserved2 == 0, "M4: no per-entry refusal bits (\"refusing entries: []\")");
    }
    {   /* the near miss: a diff that exactly fills the report is reported whole */
        EmitRig r(1024, 100);
        std::vector<SRun> v;
        for (uint32_t i = 0; i < 100; i++) v.push_back({0x1000000ull + 0x2000ull * i, 0x1000ull, 0, true});
        r.stage(0, v);
        r.emit();
        EXPECT(r.hdr.flags == KFWR_HF_DIFF && r.hdr.run_count == 100, "near miss: exactly at capacity is whole (flags %#x)", r.hdr.flags);
    }
    {   /* ... and one run more is the refusal */
        EmitRig r(1024, 100);
        std::vector<SRun> v;
        for (uint32_t i = 0; i < 101; i++) v.push_back({0x1000000ull + 0x2000ull * i, 0x1000ull, 0, true});
        r.stage(0, v);
        r.emit();
        EXPECT((r.hdr.flags & KFWR_HF_TRUNCATED) && r.hdr.run_count == 0, "one run over capacity is refused (flags %#x)", r.hdr.flags);
    }
}

int main(int argc, char **argv)
{
    const bool do_dump = argc > 1 && !strcmp(argv[1], "--dump");
    const unsigned long long a = differential("synthetic", "corpus/corpus.bin", "corpus/rust_leaves.txt", do_dump);
    const unsigned long long b = differential("real-ga106", "corpus/real_ga106.bin", "corpus/real_leaves.txt", do_dump);
    if (do_dump) return g_fail ? 1 : 0;
    ver3_cases();
    ver2_cases();
    format_cases();
    emit_m4_legacy();
    /* ⚠ Two decoders that both produced nothing agree perfectly (kf_tests.cu uses the same floors). */
    EXPECT(a > 1200, "the synthetic differential decoded almost nothing (%llu)", a);
    EXPECT(b > 6000 || b == 0, "real-driver differential too small (%llu)", b);
    EXPECT(b > 0, "the real-driver differential agreed on nothing");
    printf("==== kf_cpu_walk: %s ====\n", g_fail ? "FAIL" : "ok");
    return g_fail ? 1 : 0;
}
