/* Throwaway real-hardware validation aid (not committed): the module's OWN engine (discovery + walker + loss accounting)
 * driven against real memory on Linux: RAM via /proc/kcore (read-only), BAR0 mailbox/boot0 via sysfs resource0 (PROT_READ).
 * Writes a KFGSPCAP v1 file. Never writes to the device or to RAM.
 * usage: capture_probe /sys/bus/pci/devices/0000:00:08.0 out.kfgspcap SECONDS [mailbox-trace.txt]
 */
#define _GNU_SOURCE
#include "lib/kfwc_engine.h"
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
static int kfd, outfd;
static Elf64_Phdr ph[512];
static int nph;
static const volatile kfwc_u8 *bar0;
static unsigned long long nrec, nbytes;

static kfwc_u64 now_ns(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return (kfwc_u64)t.tv_sec * 1000000000ull + (kfwc_u64)t.tv_nsec;
}
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
static int p_reg(void *c, kfwc_u32 o, kfwc_u32 *out)
{
    (void)c;
    if (!kfwc_reg_allowed(o))
        return 1;
    *out = *(const volatile kfwc_u32 *)(bar0 + o);
    return 0;
}
static kfwc_u64 p_now(void *c) { (void)c; return now_ns(); }
static int sink_put(void *c, const kfwc_u8 *rec, kfwc_u32 len)
{
    (void)c;
    if (write(outfd, rec, len) != (ssize_t)len)
        return 1;
    nrec++;
    nbytes += len;
    return 0;
}

int main(int argc, char **argv)
{
    FILE *f;
    char line[256], path[512];
    Elf64_Ehdr eh;
    kfwc_platform plat;
    kfwc_sink sink;
    kfwc_config cfg;
    static kfwc_engine eng;
    kfwc_cap_file fh;
    kfwc_u8 hdr[KFWC_CAP_FILE_HEADER_BYTES];
    int rfd, secs;
    FILE *mt = NULL;
    kfwc_u64 t_end, last_mb = 0;
    kfwc_u32 lastlo = 0xdeadbeef, lasthi = 0xdeadbeef;
    if (argc < 4) {
        fprintf(stderr, "usage: %s sysfs-dev out SECONDS [mbtrace]\n", argv[0]);
        return 2;
    }
    secs = atoi(argv[3]);
    if (argc > 4)
        mt = fopen(argv[4], "w");
    f = fopen("/proc/iomem", "r");
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
    if (pread(kfd, ph, nph * sizeof(Elf64_Phdr), eh.e_phoff) < 0)
        return 1;
    snprintf(path, sizeof path, "%s/resource0", argv[1]);
    rfd = open(path, O_RDONLY);
    bar0 = mmap(NULL, KFWC_BAR0_MAP_BYTES, PROT_READ, MAP_SHARED, rfd, 0);
    if (bar0 == MAP_FAILED) {
        perror("mmap");
        return 1;
    }
    outfd = open(argv[2], O_WRONLY | O_CREAT | O_TRUNC, 0600);
    memset(&fh, 0, sizeof fh);
    fh.os_id = KFWC_CAP_OS_LINUX;
    fh.module_version = 1;
    fh.pci_vendor_id = 0x10de;
    fh.pci_device_id = 0x2705;
    fh.ts_hz = 1000000000ull;
    fh.ts_start = now_ns();
    kfwc_cap_set_string(fh.driver_version, "575.51.03");
    kfwc_cap_set_string(fh.decode_tag, "580.82.07");
    kfwc_cap_file_encode(&fh, hdr);
    if (write(outfd, hdr, sizeof hdr) != (ssize_t)sizeof hdr)
        return 1;
    memset(&plat, 0, sizeof plat);
    plat.read_ram = p_read_ram;
    plat.ram_contains = p_contains;
    plat.ram_range_count = p_count;
    plat.ram_range = p_range;
    plat.read_reg32 = p_reg;
    plat.now = p_now;
    sink.ctx = NULL;
    sink.put = sink_put;
    memset(&cfg, 0, sizeof cfg);
    cfg.layout = 0;
    cfg.use_mailbox = 1;
    cfg.use_scan = 1;
    cfg.scan_delay_ticks = 2ull * 1000000000ull;
    cfg.mailbox_period_ticks = 100ull * 1000000ull;
    cfg.scan_bytes_per_step = KFWC_SCAN_CHUNK;
    cfg.max_msgs_per_step = 64;
    cfg.mapping_check_steps = 16;
    cfg.scan_backoff_initial_ticks = 5ull * 1000000000ull;
    cfg.scan_backoff_max_ticks = 60ull * 1000000000ull;
    kfwc_engine_init(&eng, &plat, &sink, &cfg);
    kfwc_engine_note_start(&eng, 0x2705, 1);
    t_end = now_ns() + (kfwc_u64)secs * 1000000000ull;
    while (now_ns() < t_end) {
        kfwc_u32 n = kfwc_engine_step(&eng);
        if (mt && now_ns() - last_mb > 50000000ull) {
            kfwc_u32 lo = 0, hi = 0;
            last_mb = now_ns();
            p_reg(NULL, KFWC_REG_GSP_MAILBOX0, &lo);
            p_reg(NULL, KFWC_REG_GSP_MAILBOX1, &hi);
            if (lo != lastlo || hi != lasthi) {
                fprintf(mt, "%.3f mailbox0=0x%08x mailbox1=0x%08x\n", (now_ns() - fh.ts_start) / 1e9, lo, hi);
                fflush(mt);
                lastlo = lo;
                lasthi = hi;
            }
        }
        if (n == 0)
            usleep(1000);
    }
    kfwc_engine_stop(&eng);
    printf("records=%llu bytes=%llu state=%llu attaches=%llu detaches=%llu msgs cmd=%llu stat=%llu loss_records=%llu seq_gap=%llu resyncs=%llu torn=%llu scan_passes=%llu scan_bytes=%llu method=%llu mailbox_status=%llu\n",
           nrec, nbytes, (unsigned long long)eng.stats.state, (unsigned long long)eng.stats.attaches,
           (unsigned long long)eng.stats.detaches, (unsigned long long)eng.stats.messages[0],
           (unsigned long long)eng.stats.messages[1], (unsigned long long)eng.stats.loss_records,
           (unsigned long long)eng.stats.seq_gap_messages, (unsigned long long)eng.stats.resyncs,
           (unsigned long long)eng.stats.torn_reads, (unsigned long long)eng.stats.scan_passes,
           (unsigned long long)eng.stats.scan_bytes, (unsigned long long)eng.stats.method,
           (unsigned long long)eng.stats.mailbox_status);
    close(outfd);
    return 0;
}
