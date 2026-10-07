/*
 * ★★★ E2 — issue ONE guest MMIO write into the emulated GPU's register aperture, from
 * inside the guest, at a wall-clock instant the caller can record.
 *
 * usage: e2_doorbell_poke <sysfs-resource0> <hex-offset> <hex-value>
 *
 * ## Why this program exists rather than waiting for the driver
 *
 * `docs/design/execution_plane_increments.md` E2 asks for *"a boot in which a guest
 * doorbell write produces a DoorbellOutcome-or-named-FwdFault, counted"*.  At the current
 * wall the guest's own RM **never reaches a doorbell**: `kfifoRingChannelDoorBell_HAL` is
 * called from `channel_utils.c:557`, which runs only after the channel SCHEDULE that fails
 * with 0x56 (`ogkm-580`, and the wall itself is the measured `mem_utils.c:2006` chain in
 * §0 of that document).  A harness that waited for the driver would therefore measure
 * nothing, forever, and would report the transport as untested.
 *
 * So the ring is issued deliberately, by guest userspace, through the SAME physical offset
 * in the SAME base-address register the driver's own `GPU_VREG_WR32` would use — the 64 KiB
 * usermode window `kfifoConstructUsermodeMemdescs_GV100` hands to a client, mapped here
 * through sysfs instead of through RM.  The device cannot tell the two apart, which is the
 * point: there is one classification and both rings arrive at it.
 *
 * ## ⊘ What this program is NOT
 *
 * It is not a proof that the guest DRIVER rings a doorbell — nobody claims that, and the
 * document says where that claim will come from (E6).  It is the guest half of an
 * attribution: this process prints its own start and end timestamps, and the device prints
 * one timestamped line per arrival, so a ring can be bracketed between two instants
 * recorded by a writer that is not the device under test.
 *
 * ★ `volatile uint32_t` and a single store: a `memcpy` may be split, widened or reordered
 * by the compiler, and a doorbell that arrived as two 16-bit stores is a different
 * experiment from the one being run.
 */

#include <fcntl.h>
#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <time.h>
#include <unistd.h>

#define PAGE 4096UL

static void stamp(const char *what)
{
    struct timespec ts;
    struct tm tm;
    char buf[64];

    clock_gettime(CLOCK_REALTIME, &ts);
    gmtime_r(&ts.tv_sec, &tm);
    strftime(buf, sizeof buf, "%Y-%m-%dT%H:%M:%S", &tm);
    printf("E2POKE %s %s.%06ldZ\n", what, buf, ts.tv_nsec / 1000);
    fflush(stdout);
}

int main(int argc, char **argv)
{
    const char *path;
    uint64_t off, page_base, in_page;
    uint32_t val;
    int fd;
    void *map;
    volatile uint32_t *reg;

    if (argc != 4) {
        fprintf(stderr, "usage: %s <resource0> <hex-offset> <hex-value>\n", argv[0]);
        return 2;
    }
    path = argv[1];
    off = strtoull(argv[2], NULL, 16);
    val = (uint32_t)strtoull(argv[3], NULL, 16);

    /* O_SYNC so the mapping is uncached — a write-combined store could sit in a buffer
     * past this program's exit, and the whole instrument is about WHEN the store lands. */
    fd = open(path, O_RDWR | O_SYNC);
    if (fd < 0) {
        perror("E2POKE open");
        return 3;
    }
    page_base = off & ~(PAGE - 1);
    in_page = off - page_base;
    map = mmap(NULL, PAGE, PROT_READ | PROT_WRITE, MAP_SHARED, fd, (off_t)page_base);
    if (map == MAP_FAILED) {
        perror("E2POKE mmap");
        close(fd);
        return 4;
    }
    reg = (volatile uint32_t *)((char *)map + in_page);

    printf("E2POKE target %s +0x%" PRIx64 " value 0x%08" PRIx32 "\n", path, off, val);
    stamp("before");
    *reg = val;              /* ★ THE GUEST MMIO WRITE. One store, 32 bits, volatile. */
    __sync_synchronize();
    stamp("after");

    munmap(map, PAGE);
    close(fd);
    printf("E2POKE ok\n");
    return 0;
}
