/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 *
 * Linux twin of the module's option (a): does the GSP boot-args mailbox pair still lead to the
 * message queues while the Linux NVIDIA driver is running? Same library code, different
 * platform: BAR0 through the sysfs resource0 file mapped PROT_READ, RAM through /dev/mem opened
 * O_RDONLY. It reads exactly the three whitelisted registers and never writes anything.
 *
 *   mailbox_probe /sys/bus/pci/devices/0000:01:00.0     (run as root; device chosen by the caller)
 *
 * /dev/mem needs CONFIG_STRICT_DEVMEM off or iomem=relaxed; without it the probe still prints
 * the mailbox pair and says RAM is unreadable. It prints only: the mailbox status, the shared
 * region address, queue sizes and the number of entries between read and write pointers.
 * This is a validation aid (step 0 of the plan), not part of the capture module.
 */
#define _GNU_SOURCE
#include "../lib/kfwc_discover.h"
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>

typedef struct probe {
    const volatile unsigned char *bar0;
    int memfd;
} probe;

static int p_read_ram(void *ctx, kfwc_u64 pa, void *dst, kfwc_u32 len)
{
    probe *p = (probe *)ctx;
    return p->memfd < 0 || pread(p->memfd, dst, len, (off_t)pa) != (ssize_t)len;
}
static int p_ram_contains(void *ctx, kfwc_u64 pa, kfwc_u64 len)
{
    (void)ctx;
    return pa < (1ull << 48) && len < (1ull << 32); /* /dev/mem refuses non-RAM itself under STRICT_DEVMEM */
}
static kfwc_u32 p_range_count(void *ctx) { (void)ctx; return 0; }
static int p_range(void *ctx, kfwc_u32 i, kfwc_u64 *b, kfwc_u64 *l) { (void)ctx; (void)i; (void)b; (void)l; return 1; }
static int p_reg(void *ctx, kfwc_u32 off, kfwc_u32 *out)
{
    probe *p = (probe *)ctx;
    if (!kfwc_reg_allowed(off))
        return 1;
    *out = *(const volatile kfwc_u32 *)(p->bar0 + off);
    return 0;
}
static kfwc_u64 p_now(void *ctx) { (void)ctx; return 0; }

int main(int argc, char **argv)
{
    char path[512];
    int fd;
    probe p;
    kfwc_platform plat;
    static kfwc_binding b;
    kfwc_mb_diag d;
    kfwc_u32 lo = 0, hi = 0, id = 0;
    int rc;
    if (argc != 2) {
        fprintf(stderr, "usage: %s /sys/bus/pci/devices/DDDD:BB:DD.F\n", argv[0]);
        return 2;
    }
    snprintf(path, sizeof(path), "%s/resource0", argv[1]);
    fd = open(path, O_RDONLY);
    if (fd < 0) {
        perror(path);
        return 1;
    }
    p.bar0 = mmap(NULL, KFWC_BAR0_MAP_BYTES, PROT_READ, MAP_SHARED, fd, 0);
    if (p.bar0 == MAP_FAILED) {
        perror("mmap resource0 (PROT_READ)");
        return 1;
    }
    p.memfd = open("/dev/mem", O_RDONLY);
    plat.ctx = &p;
    plat.read_ram = p_read_ram;
    plat.ram_contains = p_ram_contains;
    plat.ram_range_count = p_range_count;
    plat.ram_range = p_range;
    plat.read_reg32 = p_reg;
    plat.now = p_now;
    kfwc_reg_read(&plat, KFWC_REG_PMC_BOOT_0, &id);
    kfwc_reg_read(&plat, KFWC_REG_GSP_MAILBOX0, &lo);
    kfwc_reg_read(&plat, KFWC_REG_GSP_MAILBOX1, &hi);
    printf("mailbox0=0x%08x mailbox1=0x%08x (boot0 readable: %s)\n", lo, hi, id ? "yes" : "no");
    if (p.memfd < 0)
        printf("/dev/mem not readable: only the register half of option (a) was tested\n");
    rc = kfwc_discover_mailbox(&plat, &b, &d);
    printf("mailbox chain: status %u (%s)\n", d.status, rc == 0 ? "OK" : "not reached");
    if (rc == 0)
        printf("shared region 0x%llx, %u pages, command queue %u bytes, status queue %u bytes (valid %u/%u)\n",
               (unsigned long long)b.table_pa, b.pages, b.q[0].size, b.q[1].size, b.q[0].valid, b.q[1].valid);
    return rc == 0 ? 0 : 3;
}
