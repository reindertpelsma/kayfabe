/* Throwaway validation aid (not committed): does the module's RAM scan (option b) find the GSP queues on REAL hardware?
 * Reads System RAM through /proc/kcore (root, read-only), the module's own kfwc_scan_signature / kfwc_bind_candidate.
 * Never writes anything. usage: scan_probe */
#define _GNU_SOURCE
#include "lib/kfwc_discover.h"
#include <elf.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <time.h>
#include <unistd.h>

#define MAXR 256
static kfwc_u64 rb[MAXR], rl[MAXR];
static kfwc_u32 nr;
static int kfd;
static Elf64_Phdr ph[512];
static int nph;

static int off_of(kfwc_u64 pa, kfwc_u64 len, off_t *off)
{
    for (int i = 0; i < nph; i++)
        if (ph[i].p_type == PT_LOAD && ph[i].p_paddr != (Elf64_Addr)-1 && ph[i].p_paddr <= pa &&
            pa + len <= ph[i].p_paddr + ph[i].p_filesz && ph[i].p_vaddr >= 0xffff800000000000ull) {
            *off = (off_t)(ph[i].p_offset + (pa - ph[i].p_paddr));
            return 0;
        }
    return 1;
}
static int p_read_ram(void *c, kfwc_u64 pa, void *dst, kfwc_u32 len)
{
    off_t off;
    (void)c;
    if (off_of(pa, len, &off))
        return 1;
    return pread(kfd, dst, len, off) != (ssize_t)len;
}
static int p_contains(void *c, kfwc_u64 pa, kfwc_u64 len)
{
    (void)c;
    for (kfwc_u32 i = 0; i < nr; i++)
        if (pa >= rb[i] && pa + len <= rb[i] + rl[i])
            return 1;
    return 0;
}
static kfwc_u32 p_count(void *c) { (void)c; return nr; }
static int p_range(void *c, kfwc_u32 i, kfwc_u64 *b, kfwc_u64 *l)
{
    (void)c;
    if (i >= nr)
        return 1;
    *b = rb[i];
    *l = rl[i];
    return 0;
}
static int p_reg(void *c, kfwc_u32 o, kfwc_u32 *out) { (void)c; (void)o; (void)out; return 1; }
static kfwc_u64 p_now(void *c) { (void)c; return 0; }

int main(void)
{
    FILE *f = fopen("/proc/iomem", "r");
    char line[256];
    Elf64_Ehdr eh;
    kfwc_platform plat;
    static kfwc_binding b;
    static kfwc_u8 buf[1 << 20];
    unsigned long long hits = 0, binds = 0, pages = 0;
    struct timespec t0, t1;
    while (f && fgets(line, sizeof line, f)) {
        unsigned long long a, e;
        if (strstr(line, "System RAM") && line[0] != ' ' && sscanf(line, "%llx-%llx", &a, &e) == 2 && nr < MAXR) {
            rb[nr] = a;
            rl[nr] = e - a + 1;
            nr++;
        }
    }
    kfd = open("/proc/kcore", O_RDONLY);
    if (kfd < 0 || pread(kfd, &eh, sizeof eh, 0) != sizeof eh) {
        perror("kcore");
        return 1;
    }
    nph = eh.e_phnum > 512 ? 512 : eh.e_phnum;
    pread(kfd, ph, nph * sizeof(Elf64_Phdr), eh.e_phoff);
    printf("RAM ranges %u, kcore PT_LOAD %d\n", nr, nph);
    memset(&plat, 0, sizeof plat);
    plat.read_ram = p_read_ram;
    plat.ram_contains = p_contains;
    plat.ram_range_count = p_count;
    plat.ram_range = p_range;
    plat.read_reg32 = p_reg;
    plat.now = p_now;
    clock_gettime(CLOCK_MONOTONIC, &t0);
    for (kfwc_u32 r = 0; r < nr; r++) {
        for (kfwc_u64 pa = rb[r]; pa < rb[r] + rl[r]; pa += sizeof buf) {
            kfwc_u64 n = rb[r] + rl[r] - pa;
            if (n > sizeof buf)
                n = sizeof buf;
            if (p_read_ram(NULL, pa, buf, (kfwc_u32)n))
                continue;
            for (kfwc_u64 o = 0; o + 4096 <= n; o += 4096) {
                pages++;
                if (kfwc_scan_signature(pa + o, buf + o)) {
                    hits++;
                    if (hits <= 20) {
                        int rc = kfwc_bind_candidate(&plat, &b, pa + o);
                        printf("candidate pa=0x%llx -> bind %s\n", (unsigned long long)(pa + o), rc == 0 ? "OK" : "refused");
                        if (rc == 0) {
                            binds++;
                            printf("  table_pa=0x%llx pages=%u cmdq=%u statq=%u (valid %u/%u)\n", (unsigned long long)b.table_pa, b.pages,
                                   b.q[0].size, b.q[1].size, b.q[0].valid, b.q[1].valid);
                        }
                    }
                }
            }
        }
    }
    clock_gettime(CLOCK_MONOTONIC, &t1);
    printf("scanned %llu pages in %.1f s; signature hits %llu; bound %llu\n", pages,
           (t1.tv_sec - t0.tv_sec) + (t1.tv_nsec - t0.tv_nsec) / 1e9, hits, binds);
    return 0;
}
