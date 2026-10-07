/*
 * dbfast_exitbench — time guest stores to one register of a PCI BAR, from inside the guest.
 * (docs/design/V3_DOORBELL_IOEVENTFD.md §7 — the vCPU cost of a doorbell store, fast path ON vs OFF.)
 *
 *   usage: dbfast_exitbench <sysfs-resourceN> <hex-offset> <hex-value> [stores=200000] [batch=100]
 *   prints: DBX off=… value=… stores=… batch=… p50_ns=… p90_ns=… p99_ns=… mean_ns=… max_ns=…
 *
 * Every store is ONE `volatile uint32_t` store (never a memcpy that could be split or widened). The
 * time is taken per BATCH of stores (two clock reads per batch, amortised) and reported as ns per
 * store; the distribution is over batches. On kf3 the doorbell page is a read-only memslot, so every
 * store leaves the guest: matched by a KVM ioeventfd (handled in the host kernel) or not (an exit to
 * QEMU and the device's trap). ⊘ Root only (sysfs resource mapping). Writes nothing else.
 */
#include <fcntl.h>
#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <time.h>
#include <unistd.h>

static uint64_t now_ns(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000000ull + (uint64_t)ts.tv_nsec;
}

static int cmp(const void *a, const void *b)
{
    uint64_t x = *(const uint64_t *)a, y = *(const uint64_t *)b;
    return x < y ? -1 : x > y;
}

int main(int argc, char **argv)
{
    if (argc < 4) {
        fprintf(stderr, "usage: %s <sysfs-resourceN> <hex-offset> <hex-value> [stores] [batch]\n", argv[0]);
        return 2;
    }
    uint64_t off = strtoull(argv[2], NULL, 16);
    uint32_t val = (uint32_t)strtoul(argv[3], NULL, 16);
    long stores = argc > 4 ? atol(argv[4]) : 200000;
    long batch = argc > 5 ? atol(argv[5]) : 100;
    if (batch < 1 || stores < batch) {
        fprintf(stderr, "bad stores/batch\n");
        return 2;
    }
    int fd = open(argv[1], O_RDWR | O_SYNC);
    if (fd < 0) {
        perror("open");
        return 1;
    }
    uint64_t page = off & ~0xfffull;
    volatile uint8_t *m = mmap(NULL, 0x1000, PROT_READ | PROT_WRITE, MAP_SHARED, fd, (off_t)page);
    if (m == MAP_FAILED) {
        perror("mmap");
        return 1;
    }
    volatile uint32_t *reg = (volatile uint32_t *)(m + (off - page));
    for (int i = 0; i < 2000; i++) {
        *reg = val; /* warm-up */
    }
    long nb = stores / batch;
    uint64_t *per = malloc(sizeof(*per) * (size_t)nb);
    if (!per) {
        return 1;
    }
    uint64_t total = 0;
    for (long b = 0; b < nb; b++) {
        uint64_t t0 = now_ns();
        for (long i = 0; i < batch; i++) {
            *reg = val;
        }
        uint64_t dt = now_ns() - t0;
        per[b] = dt / (uint64_t)batch;
        total += dt;
    }
    qsort(per, (size_t)nb, sizeof(*per), cmp);
    printf("DBX off=0x%" PRIx64 " value=0x%08x stores=%ld batch=%ld p50_ns=%" PRIu64 " p90_ns=%" PRIu64
           " p99_ns=%" PRIu64 " mean_ns=%" PRIu64 " max_ns=%" PRIu64 "\n",
           off, val, nb * batch, batch, per[nb / 2], per[(nb * 9) / 10], per[(nb * 99) / 100],
           total / (uint64_t)(nb * batch), per[nb - 1]);
    return 0;
}
