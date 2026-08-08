/*
 * cuda_ioctl_trace.c — an LD_PRELOAD interposer on ioctl(2) that records the
 * NVIDIA RM escape traffic of a REAL CUDA PROCESS on a REAL GPU.
 *
 * ────────────────────────────────────────────────────────────────────────────
 * ★★★ WHY THIS EXISTS, AND WHY NOTHING WE ALREADY HOLD COULD REPLACE IT.
 *
 * `execution_plane_increments.md` §14.26 established that all three oracles
 * this project owns — the C artifact's captured control table
 * (`mode2_initctrl_ga106.h`), `cap1_coldboot_hermetic`, and the real-GA106
 * readout in `traces/real_ga106/` — were produced by driving `RmInitAdapter`
 * with **`nvidia-smi`** (`traces/real_ga106/README.md`, "method").
 *
 * ⊘ A world with no CUDA process in it cannot witness a control only libcuda
 * asks for. Three instruments agreeing that `0x20810108` is "never requested"
 * is not corroboration, because they share the defect. That is exactly how
 * §14.22 ruled `NV2081_BINAPI` a *phantom* — a ruling §14.26 refuted by
 * measurement.
 *
 * ⇒ From `cuInit` onward the instrument must be a NEW CAPTURE, and the only
 * capture that can see what libcuda asks is one taken **while libcuda asks**.
 *
 * ────────────────────────────────────────────────────────────────────────────
 * WHAT IT RECORDS
 *
 * Every `ioctl` whose `_IOC_TYPE` is `NV_IOCTL_MAGIC` ('F'), decoded by escape
 * number:
 *
 *   0x29 NV_ESC_RM_FREE     NVOS00 — hRoot, hObjectParent, hObjectOld, status
 *   0x2a NV_ESC_RM_CONTROL  NVOS54 — hClient, hObject, cmd, paramsSize, status,
 *                                    AND the params buffer BEFORE and AFTER
 *   0x2b NV_ESC_RM_ALLOC    NVOS21 (32B) or NVOS64 (48B) — hClass, hParent,
 *                                    hObject, paramsSize, status, params bytes
 *
 * ★ The before/after pair on a control is the whole point: a reply body is the
 * DIFFERENCE the driver wrote, and "RM returned NV_OK having written nothing"
 * is a materially different fact from "RM returned a legitimate zero". Only a
 * before/after pair separates them — the same reasoning `rmladder`'s R18 rung
 * encodes with its `0xCD` seed, here obtained without disturbing the caller's
 * buffer at all.
 *
 * ⚠ `_IOC_TYPE == 'F'` is the gate, NOT the escape number. The C artifact hit
 * this: `NV_ESC_*` numbers collide with UVM's own ioctls, which live on
 * /dev/nvidia-uvm and would otherwise be decoded as RM escapes and printed as
 * nonsense (`ioctl_nr_collision_bug`). UVM's magic is not 'F', so the type
 * check excludes them by construction rather than by a path heuristic.
 *
 * ⊘ THIS OBSERVES ONLY. It forwards every call to the real `ioctl` unmodified,
 * copies out of the caller's buffers and never into them. A trace that changed
 * what it measured would be worthless for the thing it is being built for.
 *
 * ────────────────────────────────────────────────────────────────────────────
 * BUILD / RUN
 *
 *   gcc -shared -fPIC -O2 -o cuda_ioctl_trace.so cuda_ioctl_trace.c -ldl
 *   NVTRACE_OUT=/tmp/trace.txt LD_PRELOAD=./cuda_ioctl_trace.so ./cuinit_probe
 *
 * `NVTRACE_OUT` unset ⇒ stderr. `NVTRACE_MAX` caps the hex dump per buffer
 * (default 256 bytes); a control whose params are longer is marked truncated
 * rather than silently shortened.
 */

#define _GNU_SOURCE
#include <dlfcn.h>
#include <fcntl.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <unistd.h>

/* NVIDIA's ioctl type byte. `nv-ioctl-numbers.h`: #define NV_IOCTL_MAGIC 'F'. */
#define NV_IOCTL_MAGIC 'F'

#define NV_ESC_RM_FREE 0x29
#define NV_ESC_RM_CONTROL 0x2a
#define NV_ESC_RM_ALLOC 0x2b

/* `kayfabe_abi::generated::nvos` — sizes asserted there against rustc. */
#define NVOS21_SIZE 32
#define NVOS64_SIZE 48
#define NVOS54_SIZE 32

typedef int (*ioctl_fn)(int, unsigned long, ...);

static ioctl_fn real_ioctl;
static int trace_fd = -1;
static size_t hex_max = 256;

/*
 * ★ A raw fd and `write(2)`, never stdio. The traced process is libcuda, which
 * runs its own threads and its own atfork handlers; a `FILE*` shared across
 * them can deadlock inside the very call we are trying to observe. `write` on
 * an O_APPEND fd is the one primitive that does not.
 */
static void trace_init(void) {
  const char *path;
  const char *max;

  real_ioctl = (ioctl_fn)dlsym(RTLD_NEXT, "ioctl");
  path = getenv("NVTRACE_OUT");
  if (path != NULL && path[0] != '\0') {
    trace_fd = open(path, O_WRONLY | O_CREAT | O_APPEND | O_CLOEXEC, 0644);
  }
  if (trace_fd < 0) {
    trace_fd = 2;
  }
  max = getenv("NVTRACE_MAX");
  if (max != NULL && max[0] != '\0') {
    long v = strtol(max, NULL, 0);
    if (v > 0 && v <= 65536) {
      hex_max = (size_t)v;
    }
  }
}

static void emit(const char *buf, size_t len) {
  size_t off = 0;
  while (off < len) {
    ssize_t n = write(trace_fd, buf + off, len - off);
    if (n <= 0) {
      return;
    }
    off += (size_t)n;
  }
}

static uint32_t rd32(const unsigned char *p, size_t off) {
  uint32_t v;
  memcpy(&v, p + off, sizeof(v));
  return v;
}

static uint64_t rd64(const unsigned char *p, size_t off) {
  uint64_t v;
  memcpy(&v, p + off, sizeof(v));
  return v;
}

/*
 * Hex-encode up to `hex_max` bytes of a user pointer into `out`.
 *
 * ⚠ `p` is a pointer the traced process supplied. It is read through `memcpy`
 * only after a NULL check and only for the length the ioctl itself declared —
 * we never invent a length. `size == 0` prints `-`, which is distinct from an
 * all-zero body (`00000000`): "no buffer" and "a buffer of zeros" are
 * different facts and the trace must not conflate them.
 */
static void hexdump(char *out, size_t out_cap, const void *p, uint32_t size,
                    int *truncated) {
  const unsigned char *b = (const unsigned char *)p;
  size_t n = size;
  size_t i;
  size_t w = 0;

  *truncated = 0;
  if (b == NULL || size == 0) {
    if (out_cap > 2) {
      out[0] = '-';
      out[1] = '\0';
    } else if (out_cap > 0) {
      out[0] = '\0';
    }
    return;
  }
  if (n > hex_max) {
    n = hex_max;
    *truncated = 1;
  }
  for (i = 0; i < n && w + 3 < out_cap; i++) {
    static const char digits[] = "0123456789abcdef";
    out[w++] = digits[b[i] >> 4];
    out[w++] = digits[b[i] & 0xf];
  }
  out[w] = '\0';
}

/*
 * The params buffer of a control, snapshotted BEFORE the call so the reply can
 * be reported as a difference. Heap-free: a fixed stack buffer sized by
 * `hex_max`'s ceiling, because an allocation inside an interposed ioctl can
 * re-enter an allocator libcuda is already inside.
 */
#define SNAP_MAX 65536
#define HEX_CAP (2 * SNAP_MAX + 8)

int ioctl(int fd, unsigned long request, ...) {
  va_list ap;
  void *arg;
  unsigned char *p;
  unsigned int nr;
  unsigned int len;
  int rc;

  va_start(ap, request);
  arg = va_arg(ap, void *);
  va_end(ap);

  if (real_ioctl == NULL) {
    trace_init();
    if (real_ioctl == NULL) {
      return -1;
    }
  }

  if (_IOC_TYPE(request) != NV_IOCTL_MAGIC || arg == NULL) {
    return real_ioctl(fd, request, arg);
  }

  nr = _IOC_NR(request);
  len = _IOC_SIZE(request);
  p = (unsigned char *)arg;

  if (nr == NV_ESC_RM_CONTROL && len >= NVOS54_SIZE) {
    /* NVOS54: hClient +0, hObject +4, cmd +8, flags +12, params +16,
     * paramsSize +24, status +28. */
    uint32_t h_client = rd32(p, 0);
    uint32_t h_object = rd32(p, 4);
    uint32_t cmd = rd32(p, 8);
    uint64_t params = rd64(p, 16);
    uint32_t params_size = rd32(p, 24);
    uint32_t status;
    static char before[HEX_CAP];
    static char after[HEX_CAP];
    static char line[2 * HEX_CAP + 512];
    int t_before = 0;
    int t_after = 0;
    size_t n;

    hexdump(before, sizeof(before), (const void *)(uintptr_t)params, params_size,
            &t_before);
    rc = real_ioctl(fd, request, arg);
    status = rd32(p, 28);
    hexdump(after, sizeof(after), (const void *)(uintptr_t)params, params_size,
            &t_after);

    n = (size_t)snprintf(
        line, sizeof(line),
        "CTRL cmd=0x%08x hClient=0x%08x hObject=0x%08x size=%u status=0x%08x "
        "rc=%d in=%s%s out=%s%s\n",
        cmd, h_client, h_object, params_size, status, rc, before,
        t_before ? "..TRUNC" : "", after, t_after ? "..TRUNC" : "");
    if (n > sizeof(line)) {
      n = sizeof(line);
    }
    emit(line, n);
    return rc;
  }

  if (nr == NV_ESC_RM_ALLOC && (len == NVOS21_SIZE || len == NVOS64_SIZE)) {
    /* NVOS21: hRoot +0, hObjectParent +4, hObjectNew +8, hClass +12,
     *         pAllocParms +16, paramsSize +24, status +28.
     * NVOS64: same through +16, then pRightsRequested +24, paramsSize +32,
     *         flags +36, status +40. */
    int v2 = (len == NVOS64_SIZE);
    uint32_t h_root = rd32(p, 0);
    uint32_t h_parent = rd32(p, 4);
    uint32_t h_new;
    uint32_t h_class = rd32(p, 12);
    uint64_t alloc_parms = rd64(p, 16);
    uint32_t params_size = rd32(p, v2 ? 32 : 24);
    uint32_t status;
    static char body[HEX_CAP];
    static char line[HEX_CAP + 512];
    int trunc = 0;
    size_t n;

    hexdump(body, sizeof(body), (const void *)(uintptr_t)alloc_parms,
            params_size, &trunc);
    rc = real_ioctl(fd, request, arg);
    h_new = rd32(p, 8);
    status = rd32(p, v2 ? 40 : 28);

    n = (size_t)snprintf(line, sizeof(line),
                         "ALLOC hClass=0x%08x hRoot=0x%08x hParent=0x%08x "
                         "hObject=0x%08x size=%u shape=%s status=0x%08x rc=%d "
                         "params=%s%s\n",
                         h_class, h_root, h_parent, h_new, params_size,
                         v2 ? "NVOS64" : "NVOS21", status, rc, body,
                         trunc ? "..TRUNC" : "");
    if (n > sizeof(line)) {
      n = sizeof(line);
    }
    emit(line, n);
    return rc;
  }

  if (nr == NV_ESC_RM_FREE && len >= 16) {
    /* NVOS00: hRoot +0, hObjectParent +4, hObjectOld +8, status +12. */
    uint32_t h_root = rd32(p, 0);
    uint32_t h_parent = rd32(p, 4);
    uint32_t h_old = rd32(p, 8);
    uint32_t status;
    char line[256];
    size_t n;

    rc = real_ioctl(fd, request, arg);
    status = rd32(p, 12);
    n = (size_t)snprintf(line, sizeof(line),
                         "FREE hRoot=0x%08x hParent=0x%08x hObject=0x%08x "
                         "status=0x%08x rc=%d\n",
                         h_root, h_parent, h_old, status, rc);
    if (n > sizeof(line)) {
      n = sizeof(line);
    }
    emit(line, n);
    return rc;
  }

  /* Every other 'F' escape: counted by number, body not decoded. Printing the
   * number keeps the trace honest about what it did NOT decode — an escape
   * missing from the log would otherwise read as an escape that never
   * happened. */
  rc = real_ioctl(fd, request, arg);
  {
    char line[128];
    size_t n = (size_t)snprintf(line, sizeof(line),
                                "ESC nr=0x%02x len=%u rc=%d\n", nr, len, rc);
    if (n > sizeof(line)) {
      n = sizeof(line);
    }
    emit(line, n);
  }
  return rc;
}
