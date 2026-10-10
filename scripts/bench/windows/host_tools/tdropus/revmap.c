// revmap.c — every kernel VA (PML4 256..511) that maps guest-physical page PA, walking the guest's 4-level page
// tables in QEMU's guest-RAM memfd (mmap'd read-only; no VM stop). Host-only diagnosis tool.
// usage: revmap MEMFD_PATH CR3 PA [LOWMEM]   (pc machine: GPA<LOWMEM at offset GPA, GPA>=4G at GPA-4G+LOWMEM)
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <sys/stat.h>

static const uint8_t *ram;
static uint64_t ramsz, lowmem;
#define M 0x000FFFFFFFFFF000ULL

static const uint64_t *tab(uint64_t g) {
    uint64_t o;
    if (g < lowmem) o = g;
    else if (g >= (1ULL << 32)) o = g - (1ULL << 32) + lowmem;
    else return NULL;
    if (o + 4096 > ramsz) return NULL;
    return (const uint64_t *)(ram + o);
}
static uint64_t canon(uint64_t v) { return (v & (1ULL << 47)) ? (v | 0xFFFF000000000000ULL) : v; }

int main(int argc, char **argv) {
    if (argc < 4) return 2;
    int fd = open(argv[1], O_RDONLY);
    if (fd < 0) { perror("open"); return 1; }
    struct stat st; fstat(fd, &st); ramsz = st.st_size;
    ram = mmap(NULL, ramsz, PROT_READ, MAP_SHARED, fd, 0);
    if (ram == MAP_FAILED) { perror("mmap"); return 1; }
    uint64_t cr3 = strtoull(argv[2], 0, 0) & M, pa = strtoull(argv[3], 0, 0) & ~0xFFFULL;
    lowmem = argc > 4 ? strtoull(argv[4], 0, 0) : 0xC0000000ULL;
    const uint64_t *l4 = tab(cr3);
    if (!l4) return 1;
    unsigned long tables = 0, hits = 0;
    for (int i4 = 256; i4 < 512; i4++) {
        uint64_t e4 = l4[i4];
        if (!(e4 & 1) || (e4 & M) == cr3) continue;
        const uint64_t *l3 = tab(e4 & M); tables++;
        if (!l3) continue;
        for (int i3 = 0; i3 < 512; i3++) {
            uint64_t e3 = l3[i3];
            if (!(e3 & 1)) continue;
            uint64_t va3 = ((uint64_t)i4 << 39) | ((uint64_t)i3 << 30);
            if (e3 & 0x80) {
                uint64_t b = e3 & 0x000FFFFFC0000000ULL;
                if (pa >= b && pa < b + (1ULL << 30)) { printf("%#llx\n", (unsigned long long)canon(va3 + pa - b)); hits++; }
                continue;
            }
            const uint64_t *l2 = tab(e3 & M); tables++;
            if (!l2) continue;
            for (int i2 = 0; i2 < 512; i2++) {
                uint64_t e2 = l2[i2];
                if (!(e2 & 1)) continue;
                uint64_t va2 = va3 | ((uint64_t)i2 << 21);
                if (e2 & 0x80) {
                    uint64_t b = e2 & 0x000FFFFFFFE00000ULL;
                    if (pa >= b && pa < b + (1ULL << 21)) { printf("%#llx\n", (unsigned long long)canon(va2 + pa - b)); hits++; }
                    continue;
                }
                const uint64_t *l1 = tab(e2 & M); tables++;
                if (!l1) continue;
                for (int i1 = 0; i1 < 512; i1++) {
                    uint64_t e1 = l1[i1];
                    if ((e1 & 1) && (e1 & M) == pa) { printf("%#llx\n", (unsigned long long)canon(va2 | ((uint64_t)i1 << 12))); hits++; }
                }
            }
        }
    }
    fprintf(stderr, "tables=%lu hits=%lu\n", tables, hits);
    return 0;
}
