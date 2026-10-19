// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
// MEASUREMENT ONLY (trap experiments 2026-10-19). Linux-guest probe: what does one access to the kf3 GPU's BAR0 cost
// the guest, per mapping type? Runs as /bin/kftool in the fast guest (KF_TOOL=1), prints "TP ..." lines on the serial.
//   WB   : an ordinary malloc'd page (write-back)
//   UC-  : sysfs resource0 (pgprot_noncached = UC-) read of PMC_BOOT_0 (0x0, a shadow ROMD page: read = no exit)
//   WC   : sysfs resource0_wc if the BAR is prefetchable, same offset
//   IRQR : read of CPU_INTR_LEAF_EN_SET(7) (0xB8121C): ROMD shadow in production, RAM in the spinner build
//   IRQW : write 0 to the same word (a no-op for the model): a TRAPPED write in production (one VM exit to QEMU),
//          a plain RAM store in the spinner build
// Only the interrupt page's EN_SET(7) word is written, and only the value 0 (EN_SET ORs it in: no effect).
#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <string.h>
#include <fcntl.h>
#include <unistd.h>
#include <time.h>
#include <dirent.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <x86intrin.h>

static double tsc_per_ns;
static uint64_t now_ns(void) { struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t); return (uint64_t)t.tv_sec * 1000000000ull + t.tv_nsec; }
static int cmpd(const void *a, const void *b) { double x = *(const double *)a, y = *(const double *)b; return x < y ? -1 : x > y; }

static void report(const char *name, volatile uint32_t *p, int write, int n, int rounds) {
    double *r = malloc(sizeof(double) * rounds);
    uint32_t sink = 0;
    for (int k = 0; k < rounds; k++) {
        unsigned aux;
        uint64_t t0 = __rdtscp(&aux);
        if (write) for (int i = 0; i < n; i++) { *p = 0; _mm_mfence(); }
        else       for (int i = 0; i < n; i++) { sink += *p; _mm_lfence(); }
        uint64_t t1 = __rdtscp(&aux);
        r[k] = (double)(t1 - t0) / tsc_per_ns / n;
    }
    qsort(r, rounds, sizeof(double), cmpd);
    printf("TP %-5s %-5s n=%d rounds=%d ns/op median=%.1f min=%.1f max=%.1f (sink %u)\n", name, write ? "write" : "read", n, rounds, r[rounds / 2], r[0], r[rounds - 1], sink);
    free(r);
}

static char *find_gpu(char *out) {
    DIR *d = opendir("/sys/bus/pci/devices"); struct dirent *e;
    while (d && (e = readdir(d))) {
        char p[300], v[16] = {0};
        snprintf(p, sizeof p, "/sys/bus/pci/devices/%s/vendor", e->d_name);
        FILE *f = fopen(p, "r"); if (!f) continue; if (!fgets(v, sizeof v, f)) v[0] = 0; fclose(f);
        if (strncmp(v, "0x10de", 6) == 0) { snprintf(out, 300, "/sys/bus/pci/devices/%s", e->d_name); closedir(d); return out; }
    }
    if (d) closedir(d);
    return NULL;
}

int main(void) {
    uint64_t a = now_ns(); unsigned aux; uint64_t c0 = __rdtscp(&aux);
    while (now_ns() - a < 200000000ull) {}
    uint64_t c1 = __rdtscp(&aux); tsc_per_ns = (double)(c1 - c0) / (double)(now_ns() - a);
    printf("TP tsc_ghz=%.4f\n", tsc_per_ns);
    uint32_t *wb = aligned_alloc(4096, 4096); memset((void *)wb, 0, 4096);
    report("WB", wb, 0, 200000, 21); report("WB", wb, 1, 200000, 21);
    char dev[300];
    if (!find_gpu(dev)) { printf("TP no NVIDIA device in sysfs\n"); return 1; }
    char p[400]; snprintf(p, sizeof p, "%s/resource", dev);
    FILE *f = fopen(p, "r"); char line[200];
    if (f && fgets(line, sizeof line, f)) printf("TP resource0: %s", line);   // start end flags; flags bit 3 (0x8) = prefetchable
    if (f) fclose(f);
    for (int wc = 0; wc < 2; wc++) {
        snprintf(p, sizeof p, "%s/resource0%s", dev, wc ? "_wc" : "");
        int fd = open(p, O_RDWR | O_SYNC);
        if (fd < 0) { printf("TP %s: open failed (no such mapping)\n", wc ? "resource0_wc" : "resource0"); continue; }
        struct stat st; fstat(fd, &st);
        size_t len = 0xC00000;
        uint8_t *m = mmap(NULL, len, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
        if (m == MAP_FAILED) { printf("TP %s: mmap failed\n", p); continue; }
        const char *tag = wc ? "WC" : "UC-";
        report(tag, (volatile uint32_t *)(m + 0x0), 0, 20000, 21);               // PMC_BOOT_0
        report(tag, (volatile uint32_t *)(m + 0xB8121C), 0, 20000, 21);          // LEAF_EN_SET(7) read
        report(tag, (volatile uint32_t *)(m + 0xB8121C), 1, 2000, 21);           // LEAF_EN_SET(7) write 0
        munmap(m, len); close(fd);
    }
    return 0;
}
