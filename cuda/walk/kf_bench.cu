/* kf_bench.cu — ★★★★★ HOW LONG DOES A FULL WALK ACTUALLY TAKE?
 *
 * ⊘⊘⊘ WHY THIS EXISTS, w725
 * =========================
 * `dirty_tracking_without_uffd.md` and `gpga_is_one_reserved_object.md` size increment 6's
 * refresh budget on **"a full walk is ~67 µs"**, and `cuda/walk/README.md` says plainly that
 * this is *"not a claim this implementation makes"* — one thread per address space, a
 * single-threaded diff. Nobody had measured it. A design that depends on a number nobody has
 * measured is a design resting on an assumption, and the number is cheap to get.
 *
 * ⊘ **THE NUMBER IS THE DELIVERABLE, NOT A PASS/FAIL.** Nothing here asserts a threshold.
 * If the answer is milliseconds rather than microseconds, that is the result — the refresh
 * budget then does not close and the plan changes. This program is therefore a *measurement*,
 * and it deliberately does no tuning: it times the walk exactly as `kf_tests` exercises it.
 *
 * WHAT IS MEASURED
 * ================
 * The design's two stated sizes, from `gpga_is_one_reserved_object.md:77`:
 *
 *   - **the measured working set** — 1872 resident page-table pages, ~7.3 MiB;
 *   - **the worst case** — 12 GiB mapped at 4 KiB, i.e. 6144 page tables, ~24 MiB.
 *
 * plus the real GA106 driver tables (`corpus/real_ga106.bin`), and two sweeps that separate
 * the two things a walk's cost can be made of:
 *
 *   - **ENTRIES VISITED** — the descent itself, a chain of dependent loads;
 *   - **RUNS PRODUCED** — the coalescer and the single-threaded per-class diff.
 *
 * ★ Fragmentation is the knob that moves runs without moving entries: the same page tables,
 *   with every k-th PTE's target displaced so a run cannot continue through it. Comparing a
 *   dense and a fragmented table of identical entry count is what makes "what dominates" a
 *   measurement rather than a guess.
 *
 * ★ And the parallel axis: the kernel is **one thread per address space**, so N address
 *   spaces cost about what one does until the SMs fill. That is measured too, because it is
 *   the difference between "a walk costs X" and "a refresh costs X".
 *
 * ⚠ TIMING DISCIPLINE — the instrument, first
 * ===========================================
 *  - every figure is the MEDIAN of `REPS` timed refreshes after `WARMUP` untimed ones, so a
 *    first-call JIT (this binary is PTX-only, so the driver compiles on first launch) cannot
 *    be reported as the walk;
 *  - `cudaDeviceSynchronize()` before and after each timed region, because a launch is
 *    asynchronous and timing one without synchronising measures the enqueue;
 *  - `kf_refresh` is timed WHOLE — the host-to-device copies, the launch and the readback —
 *    because that is what a refresh costs the host. The kernel-only time is reported beside
 *    it, from the same run, so the fixed overhead is visible rather than folded in;
 *  - `entries_visited` comes from the report, so µs/entry is derived from what the walk
 *    actually touched and not from what the builder thinks it built;
 *  - a START marker and an `EXIT=<n>` terminator, so *"the file exists but has no
 *    terminator"* is a state of its own.
 *
 *   make kf_bench && ./kf_bench            # everything
 *   ./kf_bench working                     # one case, by name prefix
 */
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <cmath>
#include <string>
#include <vector>
#include <algorithm>

#include <cuda_runtime.h>

#include "kf_walk.h"
#include "kf_tables.h"

#define WARMUP 3
#define REPS   11

static const char *g_filter = NULL;

#define CU(x) do { cudaError_t e_ = (x); if (e_ != cudaSuccess) { \
    fprintf(stderr, "CUDA %s at %s:%d: %s\n", #x, __FILE__, __LINE__, cudaGetErrorString(e_)); \
    printf("EXIT=2\n"); exit(2); } } while (0)

static double now_us(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return (double)t.tv_sec * 1e6 + (double)t.tv_nsec / 1e3;
}

static double median(std::vector<double> &v)
{
    std::sort(v.begin(), v.end());
    return v[v.size() / 2];
}

/* ── the builders ───────────────────────────────────────────────────────────────────────
 * ⊘ Deliberately built with kf_tables.h, the SAME scaffolding the suite uses. A benchmark
 * that built its tables some other way would be timing a different program.
 */

/* `pt_pages` small page tables, each holding `per_pt` valid 4 KiB PTEs, laid out so that a
 * dense table coalesces into as few runs as possible. `frag` > 0 displaces every frag-th
 * PTE's target, which breaks the run there WITHOUT changing the entry count. */
static uint64_t build_4k(Gpga &g, Tree &t, uint32_t pt_pages, uint32_t per_pt, uint32_t frag,
                         uint64_t va_base)
{
    uint64_t n = 0;
    for (uint32_t p = 0; p < pt_pages; p++) {
        uint64_t va0 = va_base + (uint64_t)p * (2ull << 20);
        for (uint32_t i = 0; i < per_pt; i++) {
            uint64_t va = va0 + (uint64_t)i * 4096ull;
            uint64_t gp = 0x1000000ull + n * 4096ull;
            if (frag && (n % frag) == (uint64_t)(frag - 1)) gp += 0x40000000ull;
            t.map4k(va, gp);
            n++;
        }
    }
    return n;
}

/* ── one timed case ─────────────────────────────────────────────────────────────────── */
struct Result {
    std::string name;
    uint64_t entries;      /* from the report: what the walk actually touched */
    uint32_t runs;
    uint32_t pdbs;
    double refresh_us;     /* whole kf_refresh, median */
    double kernel_us;      /* the launch alone, median, by cudaEvent */
    size_t  bytes;         /* live table bytes */
};

static std::vector<Result> g_results;

static void run_case(const char *name, Gpga &g, const std::vector<uint64_t> &pdbs,
                     size_t live_bytes, const char *note)
{
    if (g_filter && strncmp(name, g_filter, strlen(g_filter))) return;

    void *dev = NULL;
    CU(cudaMalloc(&dev, g.size()));
    CU(cudaMemcpy(dev, g.mem.data(), g.size(), cudaMemcpyHostToDevice));

    KfWalkCfg c;
    memset(&c, 0, sizeof c);
    /* ⊘ 2026-09-25: the diff's scratch bounds a slot to 16 384 runs (4 × runs_per_pdb per
     * entry in the 4 Mi-run stage); a case with more runs per space truncates and is SKIPPED. */
    c.runs_per_pdb = 16384u;
    c.run_capacity = 1u << 20;
    c.max_slots = 64;
    c.pdb_capacity = 64;
    c.entry_budget = 64u * 1024u * 1024u;   /* generous: a budget stop would not be a walk */
    c.table_version = KF_TBL_VER2;
    c.gpga_span = KF_GPGA_SPAN_UNBOUNDED;   /* §39(c): a bench image is tables, not a framebuffer */
    c.max_pdbs = 64;

    KfWalk *w = kf_create(&c);
    if (!w) { fprintf(stderr, "%s: kf_create failed\n", name); printf("EXIT=3\n"); exit(3); }
    std::vector<uint32_t> slots(pdbs.size());
    for (size_t i = 0; i < slots.size(); i++) slots[i] = (uint32_t)i;

    KfReportHeader h;
    std::vector<KfPdbEntry> pe(c.pdb_capacity + 4);
    std::vector<KfMapRun> rn(c.run_capacity + 4);

    /* ⚠ WARMUP is not politeness: this binary embeds PTX and NO cubin, so the FIRST launch
     * pays a JIT compile. Reporting that as the walk time would be wrong by orders of
     * magnitude, in the flattering direction for nobody. */
    for (int i = 0; i < WARMUP; i++) {
        int rc = kf_refresh(w, dev, g.size(), pdbs.data(), slots.data(), (uint32_t)pdbs.size(),
                            &h, pe.data(), rn.data());
        if (rc != 0) { fprintf(stderr, "%s: kf_refresh rc=%d\n", name, rc); printf("EXIT=4\n"); exit(4); }
    }
    /* ⚠ A truncated walk is NOT a measurement of a full walk, so it is reported by name and
     * skipped rather than timed. ⊘ It must not abort the sweep either: one oversized case
     * silently taking the rest of the run with it is how a measurement session produces
     * nothing and reads as a crash. */
    if (h.flags & (KFWR_HF_TRUNCATED | KFWR_HF_PDB_TRUNCATED | KFWR_HF_BUDGET)) {
        printf("  %-34s SKIPPED: TRUNCATED/BUDGET (flags=0x%x refuse=0x%x, %u runs) -- not a full walk\n",
               name, h.flags, h.refuse_mask, h.run_count);
        kf_destroy(w);
        CU(cudaFree(dev));
        return;
    }

    std::vector<double> whole, kern;
    cudaEvent_t e0, e1;
    CU(cudaEventCreate(&e0));
    CU(cudaEventCreate(&e1));
    for (int i = 0; i < REPS; i++) {
        /* ★ No verdict is ever given, so every slot stays empty and each rep diffs the full
         * mapping set against nothing: the report IS the whole walk. */
        CU(cudaDeviceSynchronize());
        double t0 = now_us();
        int rc = kf_refresh(w, dev, g.size(), pdbs.data(), slots.data(), (uint32_t)pdbs.size(),
                            &h, pe.data(), rn.data());
        CU(cudaDeviceSynchronize());
        double t1 = now_us();
        if (rc != 0) { fprintf(stderr, "%s: kf_refresh rc=%d\n", name, rc); printf("EXIT=4\n"); exit(4); }
        whole.push_back(t1 - t0);

        /* The launch alone, measured by device events around a second, identical refresh.
         * ⊘ It is NOT a second measurement of the same thing: the difference between the two
         * is the host-side fixed cost (three cudaMemcpys and the launch overhead), which is
         * the part a bigger table does not grow. */
        CU(cudaDeviceSynchronize());
        CU(cudaEventRecord(e0));
        kf_refresh(w, dev, g.size(), pdbs.data(), slots.data(), (uint32_t)pdbs.size(),
                   &h, pe.data(), rn.data());
        CU(cudaEventRecord(e1));
        CU(cudaEventSynchronize(e1));
        float ms = 0.f;
        CU(cudaEventElapsedTime(&ms, e0, e1));
        kern.push_back((double)ms * 1000.0);
    }
    CU(cudaEventDestroy(e0));
    CU(cudaEventDestroy(e1));

    Result r;
    r.name = name;
    r.entries = h.entries_visited;
    r.runs = h.run_count;
    r.pdbs = h.pdb_count;
    r.refresh_us = median(whole);
    r.kernel_us = median(kern);
    r.bytes = live_bytes;
    g_results.push_back(r);

    printf("  %-34s %10llu entries %8u runs %3u vas  %9.1f us refresh  %9.1f us device  "
           "%6.1f ns/entry  %s\n",
           name, (unsigned long long)r.entries, r.runs, r.pdbs, r.refresh_us, r.kernel_us,
           r.entries ? r.refresh_us * 1000.0 / (double)r.entries : 0.0, note);
    fflush(stdout);

    kf_destroy(w);
    CU(cudaFree(dev));
}

/* Like run_case, but ACKNOWLEDGES every run, so each timed refresh diffs an UNCHANGED space
 * against its committed placements (commit-on-ack): the steady state of a guest whose
 * invalidate changed nothing. */
static void ack_all(KfWalk *w, const KfReportHeader &h)
{
    std::vector<uint8_t> c(h.run_count ? h.run_count : 1, KFWR_ACK_APPLIED);
    kf_ack(w, h.generation, c.data(), h.run_count, NULL, 0);
}
static void run_case_acked(const char *name, Gpga &g, const std::vector<uint64_t> &pdbs,
                           size_t live_bytes, const char *note)
{
    if (g_filter && strncmp(name, g_filter, strlen(g_filter))) return;
    void *dev = NULL;
    CU(cudaMalloc(&dev, g.size()));
    CU(cudaMemcpy(dev, g.mem.data(), g.size(), cudaMemcpyHostToDevice));
    KfWalkCfg c;
    memset(&c, 0, sizeof c);
    /* ⊘ 2026-09-25: the diff's scratch bounds a slot to 16 384 runs (4 × runs_per_pdb per
     * entry in the 4 Mi-run stage); a case with more runs per space truncates and is SKIPPED. */
    c.runs_per_pdb = 16384u;
    c.run_capacity = 1u << 20;
    c.max_slots = 64;
    c.pdb_capacity = 64;
    c.entry_budget = 64u * 1024u * 1024u;
    c.table_version = KF_TBL_VER2;
    c.gpga_span = KF_GPGA_SPAN_UNBOUNDED;   /* §39(c): a bench image is tables, not a framebuffer */
    c.max_pdbs = 64;
    KfWalk *w = kf_create(&c);
    if (!w) { fprintf(stderr, "%s: kf_create failed\n", name); printf("EXIT=3\n"); exit(3); }
    std::vector<uint32_t> slots(pdbs.size());
    for (size_t i = 0; i < slots.size(); i++) slots[i] = (uint32_t)i;
    KfReportHeader h;
    std::vector<KfPdbEntry> pe(c.pdb_capacity + 4);
    std::vector<KfMapRun> rn(c.run_capacity + 4);
    for (int i = 0; i < WARMUP; i++) {
        kf_refresh(w, dev, g.size(), pdbs.data(), slots.data(), (uint32_t)pdbs.size(), &h, pe.data(), rn.data());
        ack_all(w, h);
    }
    std::vector<double> whole;
    for (int i = 0; i < REPS; i++) {
        CU(cudaDeviceSynchronize());
        double t0 = now_us();
        kf_refresh(w, dev, g.size(), pdbs.data(), slots.data(), (uint32_t)pdbs.size(), &h, pe.data(), rn.data());
        CU(cudaDeviceSynchronize());
        whole.push_back(now_us() - t0);
        ack_all(w, h);
    }
    double med = median(whole);
    printf("  %-34s %10llu entries %8u runs %3u vas  %9.1f us refresh  %9s  %6.1f ns/entry  %s\n",
           name, (unsigned long long)h.entries_visited, h.run_count, h.pdb_count, med, "-",
           h.entries_visited ? med * 1000.0 / (double)h.entries_visited : 0.0, note);
    fflush(stdout);
    kf_destroy(w);
    CU(cudaFree(dev));
}

/* ── the real driver tables, from the committed corpus ───────────────────────────────── */
static bool load_real(std::vector<std::pair<std::string, std::pair<std::vector<uint8_t>, uint64_t> > > &out)
{
    FILE *f = fopen("corpus/real_ga106.bin", "rb");
    if (!f) return false;
    char magic[8];
    if (fread(magic, 1, 8, f) != 8 || memcmp(magic, "KFCORPUS", 8)) { fclose(f); return false; }
    uint32_t n = 0;
    if (fread(&n, 4, 1, f) != 1) { fclose(f); return false; }
    for (uint32_t i = 0; i < n; i++) {
        uint32_t nl = 0;
        if (fread(&nl, 4, 1, f) != 1) { fclose(f); return false; }
        std::string nm(nl, '\0');
        if (fread(&nm[0], 1, nl, f) != nl) { fclose(f); return false; }
        uint8_t b = 0;
        uint64_t glen = 0, root = 0;
        uint32_t np = 0;
        if (fread(&b, 1, 1, f) != 1 || fread(&glen, 8, 1, f) != 1 ||
            fread(&root, 8, 1, f) != 1 || fread(&np, 4, 1, f) != 1) { fclose(f); return false; }
        std::vector<uint8_t> mem((size_t)glen, 0);
        for (uint32_t k = 0; k < np; k++) {
            uint64_t off = 0;
            if (fread(&off, 8, 1, f) != 1 || off + 4096 > glen) { fclose(f); return false; }
            if (fread(mem.data() + off, 1, 4096, f) != 4096) { fclose(f); return false; }
        }
        out.push_back(std::make_pair(nm, std::make_pair(mem, root)));
    }
    fclose(f);
    return true;
}

int main(int argc, char **argv)
{
    if (argc > 1) g_filter = argv[1];

    cudaDeviceProp prop;
    CU(cudaGetDeviceProperties(&prop, 0));
    int drv = 0, rt = 0;
    cudaDriverGetVersion(&drv);
    cudaRuntimeGetVersion(&rt);
    printf("START kf_bench  gpu=\"%s\" sm_%d%d  SMs=%d  clock=%.2fGHz  driver=%d runtime=%d\n",
           prop.name, prop.major, prop.minor, prop.multiProcessorCount,
           prop.clockRate / 1e6, drv, rt);
    printf("  warmup=%d reps=%d (median reported)\n\n", WARMUP, REPS);

    const uint64_t VB = ((uint64_t)1 << 47) | ((uint64_t)3 << 38);

    /* ══ 1. THE TWO SIZES THE DESIGN NAMES ═══════════════════════════════════════════════ */
    printf("-- the design's two sizes (gpga_is_one_reserved_object.md:77) --\n");
    {   /* 1872 resident page-table pages, ~7.3 MiB, densely mapped. */
        Gpga g(64u << 20); Tree t(g);
        uint64_t n = build_4k(g, t, 1872, 512, 0, VB);
        std::vector<uint64_t> p(1, t.root);
        char note[128];
        snprintf(note, sizeof note, "1872 PTs, %llu 4K mappings = %.2f GiB mapped",
                 (unsigned long long)n, (double)n * 4096.0 / (1024.0 * 1024.0 * 1024.0));
        run_case("working_set_1872pt_dense", g, p, 1872u * 4096u, note);
    }
    {   /* The worst case: 12 GiB at 4 KiB = 3 145 728 mappings = 6144 page tables = 24 MiB. */
        Gpga g(160u << 20); Tree t(g);
        uint64_t n = build_4k(g, t, 6144, 512, 0, VB);
        std::vector<uint64_t> p(1, t.root);
        char note[128];
        snprintf(note, sizeof note, "6144 PTs, %llu 4K mappings = %.0f GiB mapped",
                 (unsigned long long)n, (double)n * 4096.0 / (1024.0 * 1024.0 * 1024.0));
        run_case("worst_case_12GiB_at_4K", g, p, 6144u * 4096u, note);
    }

    /* ══ 2. WHAT DOMINATES — entries, or runs? ══════════════════════════════════════════
     * Same entry count, different run count. If the cost tracks entries the descent
     * dominates; if it tracks runs the coalescer and the diff do. */
    printf("\n-- what dominates: same ENTRIES (1872 PTs), different RUNS --\n");
    {
        const uint32_t frags[] = { 0, 64, 16, 4, 2 };
        for (size_t k = 0; k < sizeof frags / sizeof frags[0]; k++) {
            uint32_t frag = frags[k];
            Gpga g(64u << 20); Tree t(g);
            build_4k(g, t, 1872, 512, frag, VB);
            std::vector<uint64_t> p(1, t.root);
            char nm[64], note[96];
            snprintf(nm, sizeof nm, "frag_1872pt_every%u", frag);
            snprintf(note, sizeof note, frag ? "every %uth PTE breaks the run => ~%u runs"
                                             : "dense: ONE run for the whole chain",
                     frag, frag ? 958464u / frag : 1u);
            run_case(nm, g, p, 1872u * 4096u, note);
        }
    }

    /* ══ 2b. THE DELTA — what a refresh costs when NOTHING changed ══════════════════════
     * ★ Increment 6 refreshes on a timer, and most refreshes find no change. If the cost is
     * in the descent then an unchanged refresh costs the same as a full one, and the budget
     * cannot be rescued by "usually nothing moved". That is worth measuring rather than
     * assuming in either direction. */
    printf("\n-- an ACKED refresh over unchanged tables (the common case at steady state) --\n");
    {
        Gpga g(64u << 20); Tree t(g);
        build_4k(g, t, 1872, 512, 0, VB);
        std::vector<uint64_t> p(1, t.root);
        run_case_acked("working_set_1872pt_acked_delta", g, p, 1872u * 4096u,
                       "same tables, ack honoured => empty delta");
    }

    /* ══ 3. THE PARALLEL AXIS — one thread per address space ════════════════════════════
     * ★ The README's scope limit is "one thread per address space", so N address spaces run
     * CONCURRENTLY and a refresh of many costs far less than N times one. That is the
     * difference between "a walk costs X" and "a refresh costs X", and increment 6 refreshes
     * every address space at once. */
    printf("\n-- the parallel axis: N address spaces in ONE refresh --\n");
    for (uint32_t nvas = 1; nvas <= 32; nvas *= 4) {
        Gpga g(192u << 20);
        std::vector<uint64_t> roots;
        for (uint32_t v = 0; v < nvas; v++) {
            Tree t(g);
            build_4k(g, t, 128, 512, 0, VB + ((uint64_t)v << 38));
            roots.push_back(t.root);
        }
        std::sort(roots.begin(), roots.end());     /* kf_refresh requires ascending pdbs */
        char nm[64], note[96];
        snprintf(nm, sizeof nm, "parallel_%02uvas_128pt_each", nvas);
        snprintf(note, sizeof note, "%u x 128 PTs = %u PT pages total", nvas, nvas * 128);
        run_case(nm, g, roots, (size_t)nvas * 128u * 4096u, note);
    }

    /* ══ 4. PAGE SIZE — the same VA coverage at 2 MiB instead of 4 KiB ══════════════════ */
    printf("\n-- page size: the same VA coverage, 512x fewer entries --\n");
    {
        Gpga g(64u << 20); Tree t(g);
        uint64_t n = 0;
        for (uint32_t i = 0; i < 6144; i++) { t.map2m(VB + (uint64_t)i * (2ull << 21), 0x1000000ull + (uint64_t)i * (2ull << 21)); n++; }
        std::vector<uint64_t> p(1, t.root);
        char note[96];
        snprintf(note, sizeof note, "%llu 2M mappings = %.0f GiB mapped, NO leaf page tables",
                 (unsigned long long)n, (double)n * 2.0 / 1024.0);
        run_case("same_12GiB_at_2M", g, p, 0, note);
    }

    /* ══ 5. TABLES A REAL DRIVER WROTE ══════════════════════════════════════════════════ */
    printf("\n-- real GA106 driver tables (corpus/real_ga106.bin) --\n");
    {
        std::vector<std::pair<std::string, std::pair<std::vector<uint8_t>, uint64_t> > > imgs;
        if (!load_real(imgs)) {
            printf("  corpus/real_ga106.bin missing -- SKIPPED (run from cuda/walk/)\n");
        } else {
            for (size_t i = 0; i < imgs.size(); i++) {
                Gpga g(imgs[i].second.first.size());
                memcpy(g.mem.data(), imgs[i].second.first.data(), g.mem.size());
                std::vector<uint64_t> p(1, imgs[i].second.second);
                run_case(imgs[i].first.c_str(), g, p, g.mem.size(), "stock NVIDIA 580.159.04");
            }
        }
    }

    /* ══ THE VERDICT, stated in the design's own terms ══════════════════════════════════ */
    printf("\n== the number ==\n");
    double ws = -1, wc = -1;
    for (size_t i = 0; i < g_results.size(); i++) {
        if (g_results[i].name == "working_set_1872pt_dense") ws = g_results[i].refresh_us;
        if (g_results[i].name == "worst_case_12GiB_at_4K") wc = g_results[i].refresh_us;
    }
    if (ws > 0) {
        printf("  measured working set (1872 PT pages, 7.3 MiB): %.1f us   = %.1fx the "
               "design's 67 us assumption\n", ws, ws / 67.0);
        printf("  1178 refreshes a boot would cost %.2f s of GPU time\n", ws * 1178.0 / 1e6);
    }
    if (wc > 0) {
        printf("  worst case (12 GiB at 4 KiB, 24 MiB of tables): %.1f us   = %.1fx 67 us\n",
               wc, wc / 67.0);
    }
    printf("EXIT=0\n");
    return 0;
}
