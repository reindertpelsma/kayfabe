/* kf_corpus.cpp — builds the DIFFERENTIAL CORPUS: a set of GA10x VER2 table
 * images that two independent decoders are then run over.
 *
 * Host-only (g++, no CUDA, no GPU). Run it LOCALLY; the corpus it writes is
 * committed, the Rust walker's decode of it is committed beside it, and the box
 * only ever compares. Nothing executable travels back.
 *
 *   g++ -O2 -std=c++14 -o kf_corpus kf_corpus.cpp && ./kf_corpus corpus/corpus.bin
 *
 * Images are stored SPARSELY — only the 4 KiB pages that are non-zero — because
 * a page-table hierarchy is a handful of pages inside a multi-megabyte space.
 */
#include "kf_tables.h"
#include <string>
#include <vector>

struct Image {
    std::string name;
    uint8_t benign;          /* 1 = the two decoders must agree EXACTLY */
    uint64_t gpga_len;
    uint64_t root;
    std::vector<uint8_t> data;
};
static std::vector<Image> g_imgs;

static void emit(const char *name, uint8_t benign, Gpga &g, uint64_t root)
{
    Image im;
    im.name = name; im.benign = benign;
    im.gpga_len = g.size(); im.root = root; im.data = g.mem;
    g_imgs.push_back(im);
}

static const uint64_t VB = ((uint64_t)1 << 47) | ((uint64_t)3 << 38) |
                           ((uint64_t)7 << 29) | ((uint64_t)5 << 21);
#define IMG(bytes) Gpga g(bytes); Tree t(g)

int main(int argc, char **argv)
{
    const char *out = (argc > 1) ? argv[1] : "corpus/corpus.bin";

    { IMG(512u << 10); t.map4k(VB, 0x30000ull); emit("single_4k", 1, g, t.root); }

    { IMG(1u << 20);
      for (uint32_t i = 0; i < 1024; i++) t.map4k(VB + (uint64_t)i * 4096ull, 0x40000ull + (uint64_t)i * 4096ull);
      emit("long_run_1024", 1, g, t.root); }

    { IMG(1u << 20);
      for (uint32_t i = 0; i < 96; i++) {
          uint64_t gp = 0x40000ull + (uint64_t)i * 4096ull;
          if (i >= 32) gp += 0x10000ull;
          t.map4k(VB + (uint64_t)i * 4096ull, gp, AP_PTE_VID, (i >= 64) ? PTE_READ_ONLY : 0);
      }
      emit("split_runs", 1, g, t.root); }

    { IMG(1u << 20);
      t.map64k(VB, 0x40000ull);
      t.map2m(VB + (2ull << 21), 0x200000ull);
      t.map512m(VB + (1ull << 29), 0x20000000ull);
      emit("all_page_sizes", 1, g, t.root); }

    { IMG(1u << 20);
      for (uint32_t i = 0; i < 8; i++)  t.map64k(VB + (uint64_t)i * 65536ull, 0x100000ull + (uint64_t)i * 65536ull);
      for (uint32_t i = 0; i < 32; i++) t.map4k(VB + (uint64_t)i * 4096ull, 0x300000ull + (uint64_t)i * 4096ull);
      emit("dual_pde_both_halves", 1, g, t.root); }

    { IMG(1u << 20);
      t.map4k(VB, 0x30000ull);
      uint64_t pts = t.pts(VB);
      g.u64(pts + 1 * 8) = kfb_sparse_pte();
      g.u64(pts + 2 * 8) = 0ull;
      g.u64(pts + 5 * 8) = kfb_pte(0x31000ull);
      g.u64(pts + 9 * 8) = kfb_sparse_pte();
      g.u64(pts + 10 * 8) = kfb_pte(0x32000ull, AP_PTE_VID, PTE_READ_ONLY);
      emit("sparse_holes", 1, g, t.root); }

    { IMG(1u << 20);
      t.map4k(VB, 0x30000ull);
      t.map4k(VB + 0x1000ull, 0x31000ull);
      uint64_t pts = t.pts(VB);
      uint64_t va2 = VB + (4ull << 21);
      g.u64(t.pd0(va2) + (uint64_t)vi0(va2) * 16 + 8) = kfb_pde(pts);
      emit("shared_page_table", 1, g, t.root); }

    { IMG(1u << 20);
      t.map4k(VB + 0 * 4096ull, 0x30000ull, AP_PTE_VID);
      t.map4k(VB + 1 * 4096ull, 0x31000ull, AP_PTE_SCOH);
      t.map4k(VB + 2 * 4096ull, 0x32000ull, AP_PTE_SNC);
      t.map4k(VB + 3 * 4096ull, 0x33000ull, AP_PTE_PEER);
      emit("apertures", 1, g, t.root); }

    { IMG(2u << 20);
      t.map4k(VB, 0x30000ull);
      t.map4k(VB + (1ull << 38), 0x40000ull);                 /* another PD2 slot */
      t.map4k(VB + (1ull << 47), 0x50000ull);                 /* another PD3 slot */
      t.map4k(VB + (1ull << 29), 0x60000ull);                 /* another PD1 slot */
      t.map4k(VB + (1ull << 21), 0x70000ull);                 /* another PD0 slot */
      emit("spread_across_levels", 1, g, t.root); }

    { IMG(1u << 20);
      for (uint32_t i = 0; i < 32; i++) t.map64k(VB + (uint64_t)i * 65536ull, 0x100000ull + (uint64_t)i * 65536ull);
      emit("full_big_table", 1, g, t.root); }

    /* ── hostile: the two decoders need not AGREE, but the kernel's answer must
     * be a SUBSET of the Rust walker's. ─────────────────────────────────────── */
    { IMG(1u << 20);
      t.map4k(VB, 0x30000ull);
      uint64_t pd2 = t.pd2(VB);
      g.u64(pd2 + (uint64_t)vi2(VB) * 8) = kfb_pde(pd2);
      emit("hostile_self_cycle", 0, g, t.root); }

    { IMG(1u << 20);
      t.map4k(VB, 0x30000ull);
      g.u64(t.root + (uint64_t)vi3(VB) * 8) = kfb_pde((uint64_t)g.size() + (4u << 20));
      emit("hostile_ptr_past_end", 0, g, t.root); }

    { IMG(1u << 20);
      t.map4k(VB, 0x30000ull);
      g.u64(t.root + 0) = ~0ull;
      g.u64(t.pd2(VB) + 8) = ~0ull;
      g.u64(t.pd1(VB) + 8) = ~0ull;
      g.u64(t.pd0(VB) + 16) = ~0ull;
      g.u64(t.pts(VB) + 8) = ~0ull;
      emit("hostile_all_ones", 0, g, t.root); }

    { IMG(1u << 20);
      t.map4k(VB, 0x30000ull);
      for (uint32_t i = 0; i < 64; i++)
          g.u64(t.pts(VB) + i * 8) = kfb_pte(0x40000ull + i * 4096ull, AP_PTE_SCOH);
      g.u64(t.pd2(VB) + (uint64_t)vi2(VB) * 8) = kfb_pde(t.pts(VB));
      emit("hostile_type_confusion", 0, g, t.root); }

    { IMG(1u << 20);
      t.map4k(VB, 0x30000ull);
      uint64_t pd1 = t.pd1(VB), pd2 = t.pd2(VB);
      g.u64(pd1 + (uint64_t)vi1(VB) * 8) = kfb_pde(pd2);     /* PD1 -> PD2's page */
      emit("hostile_two_cycle", 0, g, t.root); }

    FILE *f = fopen(out, "wb");
    if (!f) { perror(out); return 1; }
    fwrite("KFCORPUS", 1, 8, f);
    uint32_t n = (uint32_t)g_imgs.size();
    fwrite(&n, 4, 1, f);
    size_t pages_total = 0;
    for (size_t i = 0; i < g_imgs.size(); i++) {
        const Image &im = g_imgs[i];
        uint32_t nl = (uint32_t)im.name.size();
        fwrite(&nl, 4, 1, f);
        fwrite(im.name.data(), 1, nl, f);
        fwrite(&im.benign, 1, 1, f);
        fwrite(&im.gpga_len, 8, 1, f);
        fwrite(&im.root, 8, 1, f);
        std::vector<uint64_t> offs;
        for (uint64_t o = 0; o + 4096 <= im.data.size(); o += 4096) {
            bool nz = false;
            for (uint64_t k = 0; k < 4096 && !nz; k++) if (im.data[o + k]) nz = true;
            if (nz) offs.push_back(o);
        }
        uint32_t np = (uint32_t)offs.size();
        fwrite(&np, 4, 1, f);
        for (size_t k = 0; k < offs.size(); k++) {
            fwrite(&offs[k], 8, 1, f);
            fwrite(im.data.data() + offs[k], 1, 4096, f);
        }
        pages_total += offs.size();
        printf("%-28s benign=%u len=%llu root=0x%llx pages=%u\n", im.name.c_str(), im.benign,
               (unsigned long long)im.gpga_len, (unsigned long long)im.root, np);
    }
    fclose(f);
    printf("wrote %s: %u images, %zu pages\n", out, n, pages_total);
    return 0;
}
