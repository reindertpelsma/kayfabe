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
 * ⊘ BY DEFAULT THIS OBSERVES ONLY. It forwards every call to the real `ioctl`
 * unmodified, copies out of the caller's buffers and never into them. A trace
 * that changed what it measured would be worthless for the thing it is for.
 *
 * ────────────────────────────────────────────────────────────────────────────
 * ★★★ THE OPT-IN SECOND MODE: FAULT INJECTION, AND WHY IT IS IN THE SAME FILE
 *
 * A trace establishes that libcuda *asks* something. It cannot establish that
 * libcuda *needs* it. §14.26 was explicit about this and refused to close the
 * gap by assertion: *"`cuInit` failing 100 beside a refused `0x2081` is a
 * correlation of two facts in one window, not a proof of causation."*
 *
 * The experiment that settles it is to make a REAL GA106, driving a REAL
 * libcuda, refuse exactly one thing and nothing else — then look at what
 * `cuInit` returns. `NVFAULT_CTRL` / `NVFAULT_ALLOC` do that: the real ioctl
 * still runs, and only the `status` word the caller reads back is overwritten,
 * which is precisely the shape our emulated GSP's refusal has on the wire.
 *
 * ⚠ Injection is **off unless an env var names a target**, every injection
 * prints an `INJECT` line into the trace, and a trace containing `INJECT` lines
 * is NOT a capture of what hardware does. Keeping the two modes in one file is
 * deliberate: they must share the decode, or the injector would be aiming at a
 * different field than the tracer reports.
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
 *
 *   NVFAULT_CTRL=0x20810108,0x20800102   force these controls' status
 *   NVFAULT_ALLOC=0x2081                 force these classes' alloc status
 *   NVFAULT_STATUS=0x56                  the forced value (default 0x56,
 *                                        NV_ERR_NOT_SUPPORTED — the status our
 *                                        port's refusal actually puts on the
 *                                        wire, not an arbitrary error)
 *
 * ────────────────────────────────────────────────────────────────────────────
 * ★★★ THE THIRD MODE: `NVSWEEP_GPUINFO=1` — BISECT `GPU_GET_INFO_V2` IN PLACE
 *
 * `execution_plane_increments.md` §14.28 ends on one line of this very trace:
 * libcuda's eleven-index `0x20800102` comes back `status=0x56` with `out ==
 * in`, and **no RPC ever crosses to our port** — so the whole of this project's
 * GSP-boundary instrumentation is blind to it. §14.28 named two candidates and
 * refused to pick: an arm of `getGpuInfos` failing before `default:` (its loop
 * `break`s on the first non-`NV_OK` and returns it for the WHOLE call,
 * `ogkm-580: subdevice_ctrl_gpu_kernel.c:566-569`), or the RM control cache
 * replaying a negative.
 *
 * ★ The measurement that discriminates them is to ask each index ON ITS OWN.
 * An arm-level failure is attributable to exactly one index; a cache hit cannot
 * depend on which index was asked. `rmladder`'s R21 rung already does this —
 * but R21 runs on the HOST, against real firmware, and the question is about
 * the GUEST.
 *
 * ⊘ A guest-side R21 would need its own client/device/subdevice, i.e. a second
 * implementation of the setup whose fidelity nobody has checked. This mode
 * instead **reuses libcuda's own `hClient`/`hObject`**, on the same fd, in the
 * same process, at the same instant — so any difference from R21 is a fact
 * about the machine and not about the harness.
 *
 * It fires ONCE, on the first `0x20800102` carrying more than one index, and it
 * runs AFTER that call has been made and logged, so the subject of the
 * measurement is never perturbed by the measurement. Three sweeps, in order:
 *
 *   SWEEPIDX  each index of the observed request, one call each
 *   SWEEPPFX  prefixes of the observed request, length 1..N — names the
 *             POSITION at which a batch starts failing, which separates "one
 *             bad arm" from "a conjunction"
 *   SWEEPALL  every index 0x00..0x45, one call each — positionally diffable
 *             against `traces/real_ga106/rmladder_r21_gpuinfo_sweep_real_ga106.txt`
 *
 * ⚠ Like `NVFAULT_*`, a run with `SWEEP` lines in it is an EXPERIMENT and not a
 * capture of what libcuda does: it issues ~90 controls libcuda never issued.
 * The banner says so, in the file, so a later reader cannot mistake one for the
 * other.
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
 * The injection targets. `NULL` — the default — means "inject nothing", which
 * is the only state in which this library's output is a capture of hardware.
 */
static const char *fault_ctrl;
static const char *fault_alloc;
static uint32_t fault_status = 0x56; /* NV_ERR_NOT_SUPPORTED */

/*
 * `NVSWEEP_GPUINFO` — the §14.28 bisect. Off (0) unless set. `sweep_done`
 * makes it fire exactly once: libcuda issues `GPU_GET_INFO_V2` more than once
 * over a full `cuInit`, and ninety extra controls per occurrence would bury the
 * trace the sweep is meant to explain.
 */
static int sweep_gpuinfo;
static int sweep_done;

/*
 * Is `id` named in a comma-separated env list of hex numbers?
 *
 * ⚠ Matched NUMERICALLY, never as a substring of the text. `strstr` on the
 * list would make `NVFAULT_CTRL=0x108` silently match `0x20810108`, and an
 * injector that hits a target nobody named produces a result attributed to the
 * wrong control — the whole point of the experiment is the attribution.
 */
static int in_fault_list(const char *list, uint32_t id) {
  const char *p = list;
  while (p != NULL && *p != '\0') {
    char *end = NULL;
    unsigned long v = strtoul(p, &end, 0);
    if (end == p) {
      break;
    }
    if ((uint32_t)v == id) {
      return 1;
    }
    p = (*end == ',') ? end + 1 : end;
    while (*p == ' ') {
      p++;
    }
    if (*p == '\0') {
      break;
    }
  }
  return 0;
}

/*
 * ★ A raw fd and `write(2)`, never stdio. The traced process is libcuda, which
 * runs its own threads and its own atfork handlers; a `FILE*` shared across
 * them can deadlock inside the very call we are trying to observe. `write` on
 * an O_APPEND fd is the one primitive that does not.
 */
static void emit(const char *buf, size_t len);

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

  fault_ctrl = getenv("NVFAULT_CTRL");
  fault_alloc = getenv("NVFAULT_ALLOC");
  {
    const char *s = getenv("NVFAULT_STATUS");
    if (s != NULL && s[0] != '\0') {
      fault_status = (uint32_t)strtoul(s, NULL, 0);
    }
  }
  {
    const char *s = getenv("NVSWEEP_GPUINFO");
    if (s != NULL && s[0] != '\0' && s[0] != '0') {
      char line[256];
      int n;
      sweep_gpuinfo = 1;
      n = snprintf(line, sizeof(line),
                   "SWEEP-CONFIG gpuinfo=on  ⚠ THIS RUN ISSUES CONTROLS LIBCUDA "
                   "DID NOT — IT IS AN EXPERIMENT, NOT A CAPTURE\n");
      if (n > 0) {
        emit(line, (size_t)n);
      }
    }
  }
  if ((fault_ctrl != NULL && fault_ctrl[0] != '\0') ||
      (fault_alloc != NULL && fault_alloc[0] != '\0')) {
    char line[512];
    int n = snprintf(line, sizeof(line),
                     "INJECT-CONFIG ctrl=%s alloc=%s status=0x%08x  ⚠ THIS RUN "
                     "IS NOT A CAPTURE OF HARDWARE\n",
                     fault_ctrl != NULL ? fault_ctrl : "-",
                     fault_alloc != NULL ? fault_alloc : "-", fault_status);
    if (n > 0) {
      emit(line, (size_t)n);
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

/*
 * ★★★ The §14.28 bisect. See the header block for why it reuses the caller's
 * handles instead of allocating its own.
 *
 * `nvos54` is the CALLER's 32-byte NVOS54, copied — `hClient` and `hObject`
 * come from it verbatim and nothing else does. `params`/`paramsSize`/`status`
 * are overwritten with our own buffer, so the caller's buffer is never touched:
 * an instrument that wrote into libcuda's params would change the very call it
 * was launched from.
 */

/* `ogkm-580: ctrl2080gpu.h:122` NV2080_CTRL_GPU_INFO_MAX_LIST_SIZE. */
#define GPUINFO_MAX_LIST 0x46u
/* `4 + 8 * 0x46` — and confirmed on the wire as `size=564`. */
#define GPUINFO_PARAMS (4u + 8u * GPUINFO_MAX_LIST)
#define GPUINFO_CMD 0x20800102u

/*
 * One `GPU_GET_INFO_V2` with `n` (index, 0) pairs. Returns the RM status;
 * `out_idx`/`out_data` receive the first `n` echoed pairs when non-NULL.
 *
 * ⚠ The unused tail is seeded 0xCD, R18's reason: it separates "RM wrote back
 * the entries I declared" from "RM wrote back the whole params". A zeroed tail
 * would make those two indistinguishable, which is the ambiguity
 * `traces/real_ga106/README.md` warns about for in==out.
 */
static uint32_t gpuinfo_call(int fd, unsigned long request,
                             const unsigned char *nvos54, const uint32_t *idx,
                             uint32_t n, uint32_t *out_idx, uint32_t *out_data,
                             int *tail_written) {
  static unsigned char params[GPUINFO_PARAMS];
  unsigned char nv[NVOS54_SIZE];
  uint64_t pp;
  uint32_t psize = GPUINFO_PARAMS;
  uint32_t zero = 0;
  uint32_t i;

  memset(params, 0xCD, sizeof(params));
  memcpy(params, &n, sizeof(n));
  for (i = 0; i < n; i++) {
    memcpy(params + 4 + 8 * i, &idx[i], sizeof(idx[i]));
    memcpy(params + 8 + 8 * i, &zero, sizeof(zero));
  }

  memcpy(nv, nvos54, sizeof(nv));
  pp = (uint64_t)(uintptr_t)params;
  memcpy(nv + 16, &pp, sizeof(pp));
  memcpy(nv + 24, &psize, sizeof(psize));
  memcpy(nv + 28, &zero, sizeof(zero));

  (void)real_ioctl(fd, request, nv);

  if (tail_written != NULL) {
    *tail_written = 0;
    for (i = 4 + 8 * n; i < GPUINFO_PARAMS; i++) {
      if (params[i] != 0xCD) {
        *tail_written = 1;
        break;
      }
    }
  }
  for (i = 0; i < n; i++) {
    if (out_idx != NULL) {
      out_idx[i] = rd32(params, 4 + 8 * i);
    }
    if (out_data != NULL) {
      out_data[i] = rd32(params, 8 + 8 * i);
    }
  }
  return rd32(nv, 28);
}

static void gpuinfo_sweep(int fd, unsigned long request,
                          const unsigned char *nvos54, const uint32_t *req_idx,
                          uint32_t req_n) {
  char line[512];
  uint32_t echoed[GPUINFO_MAX_LIST];
  uint32_t data[GPUINFO_MAX_LIST];
  uint32_t i;
  size_t n;

  n = (size_t)snprintf(line, sizeof(line),
                       "SWEEP-BEGIN hClient=0x%08x hObject=0x%08x observed_n=%u"
                       "  (params=%u, tail seeded 0xCD)\n",
                       rd32(nvos54, 0), rd32(nvos54, 4), req_n, GPUINFO_PARAMS);
  emit(line, n);

  /* (1) each index of the OBSERVED request, alone. An arm-level failure is
   * attributable to exactly one of these; a cache hit cannot be. */
  for (i = 0; i < req_n; i++) {
    int tail = 0;
    uint32_t st = gpuinfo_call(fd, request, nvos54, &req_idx[i], 1, echoed, data,
                               &tail);
    n = (size_t)snprintf(line, sizeof(line),
                         "SWEEPIDX pos=%u idx=0x%08x status=0x%08x "
                         "echo=0x%08x data=0x%08x tail=%s\n",
                         i, req_idx[i], st, echoed[0], data[0],
                         tail ? "WRITTEN" : "untouched");
    emit(line, n);
  }

  /* (2) prefixes. Names the POSITION at which the batch starts failing —
   * which is what separates one bad arm from a conjunction, and is also the
   * only way to see an index that fails ONLY in company. */
  for (i = 1; i <= req_n; i++) {
    uint32_t st = gpuinfo_call(fd, request, nvos54, req_idx, i, echoed, data,
                               NULL);
    n = (size_t)snprintf(line, sizeof(line),
                         "SWEEPPFX len=%u status=0x%08x last_echo=0x%08x "
                         "last_data=0x%08x\n",
                         i, st, echoed[i - 1], data[i - 1]);
    emit(line, n);
  }

  /* (3) the whole index space, one call each — positionally diffable against
   * `rmladder_r21_gpuinfo_sweep_real_ga106.txt`, which asks a real GA106 the
   * identical seventy questions in the identical shape. */
  for (i = 0; i < GPUINFO_MAX_LIST; i++) {
    int tail = 0;
    uint32_t one = i;
    uint32_t st = gpuinfo_call(fd, request, nvos54, &one, 1, echoed, data,
                               &tail);
    n = (size_t)snprintf(line, sizeof(line),
                         "SWEEPALL idx=0x%02x status=0x%08x echo=0x%08x "
                         "data=0x%08x tail=%s\n",
                         i, st, echoed[0], data[0],
                         tail ? "WRITTEN" : "untouched");
    emit(line, n);
  }

  n = (size_t)snprintf(line, sizeof(line), "SWEEP-END\n");
  emit(line, n);
}

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

    /* ⚠ The overwrite happens AFTER the `after` snapshot, so the trace still
     * reports the body hardware actually produced. The caller sees the forced
     * status; the log records both, which is what makes an injected run
     * interpretable instead of merely broken. */
    if (fault_ctrl != NULL && in_fault_list(fault_ctrl, cmd)) {
      char inj[192];
      size_t k = (size_t)snprintf(inj, sizeof(inj),
                                  "INJECT CTRL cmd=0x%08x real_status=0x%08x "
                                  "forced=0x%08x\n",
                                  cmd, status, fault_status);
      memcpy(p + 28, &fault_status, sizeof(fault_status));
      if (k > sizeof(inj)) {
        k = sizeof(inj);
      }
      emit(inj, k);
    }

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

    /* ★★★ §14.28's bisect, fired AFTER the observed call has completed and
     * been logged — the subject is never perturbed by the measurement. The
     * gate is `>= 2` indices because the guest kernel's OWN one-index calls
     * (seq303/seq780 of `rpctrace_ga106_boot1.bin`) succeed and are not the
     * question; libcuda's is the eleven-index one. */
    if (sweep_gpuinfo && !sweep_done && cmd == GPUINFO_CMD &&
        params_size >= GPUINFO_PARAMS && params != 0) {
      const unsigned char *pb = (const unsigned char *)(uintptr_t)params;
      uint32_t req_n = rd32(pb, 0);
      if (req_n >= 2 && req_n <= GPUINFO_MAX_LIST) {
        uint32_t req_idx[GPUINFO_MAX_LIST];
        uint32_t i;
        sweep_done = 1;
        for (i = 0; i < req_n; i++) {
          /* ⚠ Mask off bit 31. The kernel OR-s INDEX_FORWARD_TO_PHYSICAL into
           * the index word of every entry it forwards, so a post-call read of
           * a SUCCEEDING request hands back `0x80000011`, not `0x11`. Replaying
           * that verbatim would ask a question libcuda never asked. */
          req_idx[i] = rd32(pb, 4 + 8 * i) & 0x00FFFFFFu;
        }
        gpuinfo_sweep(fd, request, p, req_idx, req_n);
      }
    }
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

    if (fault_alloc != NULL && in_fault_list(fault_alloc, h_class)) {
      char inj[192];
      size_t k = (size_t)snprintf(inj, sizeof(inj),
                                  "INJECT ALLOC hClass=0x%08x "
                                  "real_status=0x%08x forced=0x%08x\n",
                                  h_class, status, fault_status);
      memcpy(p + (v2 ? 40 : 28), &fault_status, sizeof(fault_status));
      if (k > sizeof(inj)) {
        k = sizeof(inj);
      }
      emit(inj, k);
    }

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
