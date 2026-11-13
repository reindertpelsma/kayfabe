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

int main(int argc, char **argv)
{
    const bool do_dump = argc > 1 && !strcmp(argv[1], "--dump");
    const unsigned long long a = differential("synthetic", "corpus/corpus.bin", "corpus/rust_leaves.txt", do_dump);
    const unsigned long long b = differential("real-ga106", "corpus/real_ga106.bin", "corpus/real_leaves.txt", do_dump);
    if (do_dump) return g_fail ? 1 : 0;
    /* ⚠ Two decoders that both produced nothing agree perfectly (kf_tests.cu uses the same floors). */
    EXPECT(a > 1200, "the synthetic differential decoded almost nothing (%llu)", a);
    EXPECT(b > 6000 || b == 0, "real-driver differential too small (%llu)", b);
    EXPECT(b > 0, "the real-driver differential agreed on nothing");
    printf("==== kf_cpu_walk: %s ====\n", g_fail ? "FAIL" : "ok");
    return g_fail ? 1 : 0;
}
