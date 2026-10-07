/* SPDX-License-Identifier: MIT */
/*
 * maps_shim.c — LD_PRELOAD wrapper that intercepts open()/openat() of
 * /proc/self/maps and returns a memfd containing rewritten content
 * matching the bare-metal host's layout.
 *
 * Specifically: insert a `---p` (PROT_NONE) guard-page entry between
 * consecutive r--p mappings of libc.so. On bare-metal host, the dynamic
 * loader inserts a small PROT_NONE gap between the read-only sections
 * of libc.so. On guest (different glibc / loader), no such gap exists.
 *
 * Hypothesis: libcuda parses /proc/self/maps when constructing the
 * UVM_INITIALIZE ioctl's input buffer, and fills a pointer field
 * differently depending on whether the gap is present. Reproducing
 * the host's mapping layout should make libcuda produce the same input
 * the host kernel sees.
 *
 * Build:
 *   cc -O2 -shared -fPIC -o maps_shim.so maps_shim.c -ldl
 *
 * Run:
 *   LD_PRELOAD=./maps_shim.so ./cumemalloc_test
 *
 * Env:
 *   NVKVM_MAPS_SHIM_LOG=1   - log every interception to stderr
 *   NVKVM_MAPS_SHIM_DUMP=1  - dump synthesized content to stderr (huge)
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdarg.h>
#include <unistd.h>
#include <fcntl.h>
#include <stdint.h>
#include <sys/syscall.h>
#include <sys/stat.h>
#include <linux/memfd.h>
#include <errno.h>

#ifndef SYS_memfd_create
#define SYS_memfd_create 319
#endif

static int (*real_open)(const char *path, int flags, ...) = NULL;
static int (*real_openat)(int dirfd, const char *path, int flags, ...) = NULL;
static int verbose = 0;
static int dump = 0;

static void __attribute__((constructor)) shim_init(void)
{
    real_open   = dlsym(RTLD_NEXT, "open");
    real_openat = dlsym(RTLD_NEXT, "openat");
    verbose = getenv("NVKVM_MAPS_SHIM_LOG") != NULL;
    dump    = getenv("NVKVM_MAPS_SHIM_DUMP") != NULL;
    fprintf(stderr, "[maps_shim] active (log=%d dump=%d)\n", verbose, dump);
}

/*
 * Decide whether `path` refers to /proc/self/maps or /proc/<pid>/maps for
 * our own pid (which libcuda might use the explicit-pid form for).
 */
static int is_self_maps(const char *path)
{
    if (!path) return 0;
    if (strcmp(path, "/proc/self/maps") == 0) return 1;
    /* /proc/<pid>/maps where pid is our pid */
    if (strncmp(path, "/proc/", 6) == 0) {
        const char *p = path + 6;
        char *end;
        long pid = strtol(p, &end, 10);
        if (end != p && strcmp(end, "/maps") == 0 && pid == getpid())
            return 1;
    }
    return 0;
}

/*
 * Rewrite /proc/self/maps content: when we see two consecutive r--p
 * mappings of the same libc.so file with no gap between them, insert
 * a ---p line between them (mimicking the host's loader-inserted guard).
 *
 * Parses each line minimally — we don't need full structure, just need
 * to spot "the second r--p of libc.so right after another r--p".
 */
struct map_line {
    unsigned long start, end;
    char  perms[5];
    char  rest[256];   /* offset, dev, inode, path */
};

static int parse_line(const char *line, struct map_line *out)
{
    /* Format: START-END PERMS OFFSET DEV INODE  PATH */
    if (sscanf(line, "%lx-%lx %4s %255[^\n]",
               &out->start, &out->end, out->perms, out->rest) < 3)
        return 0;
    return 1;
}

static int is_libc_so(const char *rest)
{
    /* Look for "libc.so.6" or "libc-X.Y.so" anywhere in the trailing text */
    return strstr(rest, "libc.so") != NULL ||
           strstr(rest, "libc-") != NULL;
}

static char *rewrite_maps_content(const char *original, size_t in_len,
                                   size_t *out_len)
{
    /* Allocate generous output buffer (+~10% for inserted guards) */
    size_t out_cap = in_len * 2 + 4096;
    char *out = malloc(out_cap);
    if (!out) { *out_len = 0; return NULL; }
    size_t pos = 0;

    /* Iterate over lines */
    const char *p = original;
    const char *end = original + in_len;
    struct map_line prev = {0};
    int have_prev = 0;
    int inserted = 0;

    while (p < end) {
        const char *nl = memchr(p, '\n', end - p);
        size_t ll = nl ? (size_t)(nl - p) : (size_t)(end - p);

        /* Copy this line raw */
        char linebuf[512];
        if (ll >= sizeof(linebuf)) ll = sizeof(linebuf) - 1;
        memcpy(linebuf, p, ll);
        linebuf[ll] = 0;

        struct map_line cur;
        if (parse_line(linebuf, &cur)) {
            /* Check: previous was r--p of libc.so, current is r--p of
             * libc.so, no gap (cur.start == prev.end), no guard yet
             * between them? Then insert a guard before cur. */
            if (have_prev &&
                strcmp(prev.perms, "r--p") == 0 &&
                strcmp(cur.perms,  "r--p") == 0 &&
                is_libc_so(prev.rest) && is_libc_so(cur.rest) &&
                cur.start == prev.end) {
                /* Synthesize a 1-page ---p guard line. Mimic host
                 * format: keep dev / inode the same as libc; offset
                 * keeps growing from prev.end. */
                size_t guard_start = cur.start;
                size_t guard_end   = cur.start + 0x1000;  /* 1 page */
                /* The current entry's start will be shifted by 1 page.
                 * BUT: we don't actually want to shift the real
                 * mapping — libcuda would notice. Instead, present
                 * the guard as "between prev and cur" by carving 1
                 * page off prev.end. That keeps cur unchanged. */
                /* Carve from prev: shrink prev's end by 1 page in
                 * what we already emitted is too late. Easier: just
                 * place the guard "overlapping" cur's first page — no
                 * that doesn't make sense either. */
                /* Pragmatic approach: emit a guard entry with
                 * zero-byte range AT cur.start. The kernel never
                 * actually produces zero-byte maps lines, but libcuda's
                 * parser may still notice it as a separator. */
                /* Better: emit the guard with the EXACT same range as
                 * cur's first page, then emit cur unchanged. The
                 * parser will see "two entries at the same address"
                 * which is also weird. */
                /* Best: insert guard ABOVE cur, occupying [cur.start-page,
                 * cur.start). Since we can't actually do that in the
                 * real address space, just lie in the synthesized
                 * file. The libcuda parser doesn't VERIFY by accessing
                 * the address; it just reads text. */
                size_t fake_guard_start = cur.start - 0x1000;
                size_t fake_guard_end   = cur.start;
                /* Match the host's typical offset (offset grows
                 * monotonically across libc entries). Just pick one
                 * plausible value. */
                int n = snprintf(out + pos, out_cap - pos,
                    "%012lx-%012lx ---p %08x 00:00 0 \n",
                    fake_guard_start, fake_guard_end, 0);
                if (n > 0) pos += (size_t)n;
                (void)guard_start; (void)guard_end;
                inserted++;
            }
            have_prev = 1;
            prev = cur;
        } else {
            have_prev = 0;
        }

        /* Copy the original line */
        if (pos + ll + 2 >= out_cap) {
            /* shouldn't happen with our 2x sizing */
            break;
        }
        memcpy(out + pos, p, ll);
        pos += ll;
        if (nl) { out[pos++] = '\n'; p = nl + 1; }
        else break;
    }
    if (verbose)
        fprintf(stderr, "[maps_shim] rewrote %zu→%zu bytes (inserted %d guards)\n",
                in_len, pos, inserted);
    *out_len = pos;
    return out;
}

/*
 * Read the real /proc/self/maps, rewrite it, return a memfd containing
 * the rewritten content positioned at offset 0.
 */
static int build_shim_fd(void)
{
    int real_fd = real_open("/proc/self/maps", O_RDONLY);
    if (real_fd < 0) return -1;

    /* Read the entire real content. /proc/self/maps is small (typically
     * under 64 KB even for big programs). */
    char buf[256 * 1024];
    ssize_t total = 0;
    while (total < (ssize_t)sizeof(buf)) {
        ssize_t r = read(real_fd, buf + total, sizeof(buf) - total);
        if (r <= 0) break;
        total += r;
    }
    close(real_fd);
    if (total <= 0) return -1;

    size_t new_len = 0;
    char *new_buf = rewrite_maps_content(buf, total, &new_len);
    if (!new_buf || new_len == 0) {
        free(new_buf);
        return -1;
    }

    int mfd = (int)syscall(SYS_memfd_create, "shim_maps", 0);
    if (mfd < 0) {
        free(new_buf);
        return -1;
    }

    ssize_t w = write(mfd, new_buf, new_len);
    if (dump)
        fprintf(stderr, "[maps_shim] synthesized content (%zu bytes):\n%.*s",
                new_len, (int)new_len, new_buf);
    free(new_buf);
    if (w != (ssize_t)new_len) {
        close(mfd);
        return -1;
    }
    /* Seek to start so the caller's first read sees the beginning. */
    lseek(mfd, 0, SEEK_SET);
    return mfd;
}

int open(const char *path, int flags, ...)
{
    int mode = 0;
    if (flags & O_CREAT) {
        va_list ap;
        va_start(ap, flags);
        mode = va_arg(ap, int);
        va_end(ap);
    }

    if (is_self_maps(path)) {
        int fd = build_shim_fd();
        if (verbose)
            fprintf(stderr, "[maps_shim] open(%s) -> shim_fd=%d\n",
                    path, fd);
        if (fd >= 0) return fd;
        /* fall through to real open on failure */
    }
    return real_open(path, flags, mode);
}

int openat(int dirfd, const char *path, int flags, ...)
{
    int mode = 0;
    if (flags & O_CREAT) {
        va_list ap;
        va_start(ap, flags);
        mode = va_arg(ap, int);
        va_end(ap);
    }
    /* If dirfd is AT_FDCWD and path is absolute or is /proc/self/maps */
    if (is_self_maps(path)) {
        int fd = build_shim_fd();
        if (verbose)
            fprintf(stderr, "[maps_shim] openat(%s) -> shim_fd=%d\n",
                    path, fd);
        if (fd >= 0) return fd;
    }
    return real_openat(dirfd, path, flags, mode);
}
