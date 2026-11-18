/* kf_pte_fuzz_walk.cpp — the walk kernel's SERIAL walk (`kf_walk_one`), run on a CPU, as a filter
 * for the GPU-free guest-PTE -> host-PTE differential fuzz (2026-11-18).
 *
 * STATUS: LIVE. Built by `crates/kf-mem/src/sim/pte_fuzz.rs` with g++ (no GPU, no CUDA SDK), the same
 * way `kf_cpu_walk.cpp` is: kf_walk.cu's device half under kf_cpu_shim.h, so every decode decision
 * (valid, aperture, address, flags, KIND, sparse, the big/small veto, the coalescer, the containment
 * check) is made by the functions NVRTC compiles into the shipped kf_walk.ptx. Design and campaign
 * commands: docs/design/V3_PTE_MODEL_FUZZ.md.
 *
 * Protocol (little-endian, stdin, any number of jobs until EOF):
 *   u32 magic 0x4b465054 ('KFPT'), u32 table_version (KF_TBL_VER2 / KF_TBL_VER3),
 *   u8  leaf_ps[KF_DIRS] (the descriptor's dir[k].leaf_ps the caller hands the walk), u8 pad[3],
 *   u64 span (vidmem bytes; also the table window), u64 root, u32 npages,
 *   npages x { u64 offset, 4096 bytes }.
 * Output per job (stdout, text): "J <refuse> <refusals> <sparse> <stop> <nruns>\n" then one
 * "R <va> <gpga> <len> <flags>\n" (hex) per run. A malformed job prints "E <why>\n" and exits 2.
 *
 * ⚠ The PARALLEL walk (production) is not run here (its warp intrinsics are stubs on a CPU); it
 * shares every decode function with the serial walk, and kf_tests.cu grades it on a GPU.
 */
#include "kf_cpu_shim.h"
#define KF_DEVICE_ONLY 1
#include "kf_walk.cu"

#include <stdio.h>
#include <stdlib.h>
#include <vector>

uint64_t shmem[1];

static bool rd(void *p, size_t n) { return fread(p, 1, n, stdin) == n; }

int main(void)
{
    static char obuf[1 << 20];
    setvbuf(stdout, obuf, _IOFBF, sizeof(obuf));
    std::vector<KfMapRun> runs(1u << 20);
    for (;;) {
        uint32_t magic = 0, tv = 0, npages = 0;
        uint8_t leaf[KF_DIRS], pad[3];
        uint64_t span = 0, root = 0;
        if (!rd(&magic, 4)) break;
        if (magic != 0x4b465054u || !rd(&tv, 4) || !rd(leaf, KF_DIRS) || !rd(pad, 3) ||
            !rd(&span, 8) || !rd(&root, 8) || !rd(&npages, 4)) {
            printf("E header\n");
            return 2;
        }
        if (span == 0 || span > (1ull << 36) || (span & 4095ull)) { printf("E span\n"); return 2; }
        /* calloc of the whole window: untouched pages stay unbacked zero pages. */
        uint8_t *mem = (uint8_t *)calloc(1, (size_t)span);
        if (!mem) { printf("E oom\n"); return 2; }
        std::vector<uint64_t> touched;
        for (uint32_t i = 0; i < npages; i++) {
            uint64_t off = 0;
            if (!rd(&off, 8) || (off & 4095ull) || off + 4096ull > span || !rd(mem + off, 4096)) {
                printf("E page\n");
                return 2;
            }
        }
        KfFormat F = (tv == KF_TBL_VER3) ? kf_format_ver3_untested() : kf_format_ver2();
        for (uint32_t k = 0; k < KF_DIRS; k++) F.dir[k].leaf_ps = leaf[k];
        KfCtx c;
        memset(&c, 0, sizeof(c));
        if (!kf_dev_format_ok(F)) {
            printf("J %x 1 0 1 0\n", (unsigned)KFWR_R_BAD_FORMAT);
            free(mem);
            continue;
        }
        c.w.base = mem;
        c.w.len = span;
        c.w.span = span;
        c.fmt = &F;
        c.budget = 64000000ull;
        c.out = runs.data();
        c.cap = (uint32_t)runs.size();
        c.pdb_index = 0;
        c.sdev = NULL;
        kf_walk_one(c, root);
        printf("J %x %x %x %x %x\n", c.refuse, c.refusals, c.sparse, c.stop, c.n);
        for (uint32_t i = 0; i < c.n; i++)
            printf("R %llx %llx %llx %x\n", (unsigned long long)runs[i].va,
                   (unsigned long long)runs[i].gpga, (unsigned long long)runs[i].len, runs[i].flags);
        fflush(stdout);
        free(mem);
    }
    return 0;
}
