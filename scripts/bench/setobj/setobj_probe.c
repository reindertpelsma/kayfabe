// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
/*
 * setobj_probe.c — does a SECOND SET_OBJECT on a software subchannel get accepted or rejected?
 *
 * Experiment of traces/setobj_20261016/PREREG.md (predictions and falsifiers are written there, committed
 * BEFORE the first run). A plain C program over /dev/nvidiactl in the style of scripts/bench/vasfmt_probe.c:
 * public headers only (open-gpu-kernel-modules 595.91.07, -I via the build line in run_setobj.sh), no closed
 * binary read, no kayfabe crate touched. It makes ONE fresh RM client, TSG and GR channel per process, submits
 * hand-built pushbuffers (public methods only) and reports what it could observe:
 *
 *   D1 a host-method semaphore release after every step (the channel kept running),
 *   D2 the channel error notifier, D4 every RM status; D3 (the host kernel journal) is read by the runner,
 *   D5 the handler-reached oracle: a deferred API registered on an object (DMA_INVALIDATE_TLB, implicit delete)
 *      is "executed" iff the object's handler took a 0x200 method that named it; REMOVE_API tells which.
 *
 * Usage: setobj_probe <case> [rep]       one case per process (a fresh channel each).
 * Output: lines "OBS ..." (observations) and "RES ..." (per-step results); the runner collects them.
 *
 * Build (runner does it): cc -O1 -Wall -I$H -I$H/class -I$H/ctrl -I$H/alloc setobj_probe.c -o setobj_probe
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <sys/prctl.h>
#include <time.h>
#include <unistd.h>

#include "nvtypes.h"
#include "nvos.h"
#include "alloc/alloc_channel.h"
#include "class/cl0080.h"
#include "class/cl2080.h"
#include "class/cl0040.h"
#include "class/cl003e.h"
#include "class/cl0070.h"
#include "class/cl90f1.h"
#include "class/cla06c.h"
#include "class/clc56f.h"
#include "class/clc561.h"
#include "class/clc9c0.h"
#include "class/clc997.h"
#include "class/cl5080.h"
#include "ctrl/ctrl5080.h"
#include "ctrl/ctrl906f.h"
#include "ctrl/ctrla06c.h"
#include "ctrl/ctrla06f/ctrla06fgpfifo.h"
#include "ctrl/ctrlc36f.h"
#include "ctrl/ctrl0000/ctrl0000gpu.h"
#include "ctrl/ctrl2080/ctrl2080dma.h"

#define NV_IOCTL_MAGIC 'F'
#define NV_ESC_REGISTER_FD 201
#define NV_ESC_RM_FREE_ 0x29
#define NV_ESC_RM_CONTROL_ 0x2a
#define NV_ESC_RM_ALLOC_ 0x2b
#define NV_ESC_RM_MAP_MEMORY_ 0x4e
#define NV_ESC_RM_MAP_MEMORY_DMA_ 0x57

typedef struct { NVOS33_PARAMETERS p; int fd; } os33fd_t;
typedef struct { uint64_t ts; uint32_t info32; uint16_t info16; uint16_t status; } notif_t;

static int g_ctl = -1;
static double t0;
static FILE *g_out;

static double now(void) {
  struct timespec ts; clock_gettime(CLOCK_MONOTONIC, &ts);
  return ts.tv_sec + ts.tv_nsec * 1e-9;
}
static void say(const char *kind, const char *fmt, ...) {
  char b[1024]; va_list ap; va_start(ap, fmt); vsnprintf(b, sizeof b, fmt, ap); va_end(ap);
  fprintf(g_out, "%s t=%.3f %s\n", kind, now() - t0, b); fflush(g_out);
}

/* ------------------------------------------------------------------ RM escapes */
static uint32_t g_hClient;
static uint32_t g_nexth = 0xb00b0000;
static uint32_t mint(void) { return ++g_nexth; }

/* status is returned through *st; the handle is the new object (0 on failure). */
static uint32_t rm_alloc(const char *what, uint32_t parent, uint32_t cls, void *params, uint32_t psize, uint32_t *st) {
  NVOS21_PARAMETERS a; memset(&a, 0, sizeof a);
  a.hRoot = g_hClient; a.hObjectParent = parent; a.hObjectNew = (cls == 0 && parent == 0) ? 0 : mint();
  a.hClass = cls; a.pAllocParms = (NvP64)(uintptr_t)params; a.paramsSize = psize;
  int rc = ioctl(g_ctl, _IOWR(NV_IOCTL_MAGIC, NV_ESC_RM_ALLOC_, NVOS21_PARAMETERS), &a);
  uint32_t s = rc ? 0xffff0000u | (uint32_t)errno : a.status;
  say("RM", "alloc %s class=0x%x parent=0x%x -> h=0x%x status=0x%x", what, cls, parent, a.hObjectNew, s);
  if (st) *st = s;
  return s ? 0 : a.hObjectNew;
}
static uint32_t rm_control(const char *what, uint32_t obj, uint32_t cmd, void *params, uint32_t psize) {
  NVOS54_PARAMETERS c; memset(&c, 0, sizeof c);
  c.hClient = g_hClient; c.hObject = obj; c.cmd = cmd; c.params = (NvP64)(uintptr_t)params; c.paramsSize = psize;
  int rc = ioctl(g_ctl, _IOWR(NV_IOCTL_MAGIC, NV_ESC_RM_CONTROL_, NVOS54_PARAMETERS), &c);
  uint32_t s = rc ? 0xffff0000u | (uint32_t)errno : c.status;
  say("RM", "control %s obj=0x%x cmd=0x%08x -> status=0x%x", what, obj, cmd, s);
  return s;
}
static uint32_t rm_free(const char *what, uint32_t parent, uint32_t obj) {
  NVOS00_PARAMETERS f; memset(&f, 0, sizeof f);
  f.hRoot = g_hClient; f.hObjectParent = parent; f.hObjectOld = obj;
  int rc = ioctl(g_ctl, _IOWR(NV_IOCTL_MAGIC, NV_ESC_RM_FREE_, NVOS00_PARAMETERS), &f);
  uint32_t s = rc ? 0xffff0000u | (uint32_t)errno : f.status;
  say("RM", "free %s obj=0x%x -> status=0x%x", what, obj, s);
  return s;
}

/* ------------------------------------------------------------------ the channel context */
static uint32_t hDevice, hSub, hVAS, hVirt, hRing, hNotif, hTSG, hChan, hUsermode, hCompute, h3d;
static uint64_t ringVA;
static volatile uint8_t *ring, *notifp, *usermode;
static uint32_t token;
static uint32_t gp_put;      /* next GP index */
static uint32_t pb_off;      /* next pushbuffer byte offset in the ring object */
static int g_dev_fd = -1;
static int g_pid;

#define RING_BYTES 0x20000ull
#define GP_OFF 0x0000u        /* 512 entries x 8 */
#define GP_ENTRIES 512u
#define USERD_OFF 0x1000u
#define SEM_OFF 0x2000u       /* 16 bytes per slot */
#define PB_OFF 0x4000u
#define USERD_GP_PUT 0x8c
#define USERD_GP_GET 0x88

static volatile uint32_t *semp(int slot) { return (volatile uint32_t *)(ring + SEM_OFF + 16 * slot); }

static volatile uint8_t *map_cpu(const char *what, uint32_t hMem, uint64_t len, uint32_t *st) {
  int fd = open("/dev/nvidia0", O_RDWR);
  if (fd < 0) { *st = 0xfffe0000; return NULL; }
  int ctlfd = g_ctl; (void)ctlfd;
  struct { int ctl_fd; } reg = { g_ctl };
  ioctl(fd, _IOWR(NV_IOCTL_MAGIC, NV_ESC_REGISTER_FD, reg), &reg);
  os33fd_t m; memset(&m, 0, sizeof m);
  m.p.hClient = g_hClient; m.p.hDevice = hDevice; m.p.hMemory = hMem; m.p.offset = 0; m.p.length = len;
  m.fd = fd;
  int rc = ioctl(g_ctl, _IOWR(NV_IOCTL_MAGIC, NV_ESC_RM_MAP_MEMORY_, os33fd_t), &m);
  *st = rc ? 0xffff0000u | (uint32_t)errno : m.p.status;
  if (*st) { say("RM", "map_cpu %s -> status=0x%x", what, *st); return NULL; }
  void *p = mmap(NULL, len, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
  if (p == MAP_FAILED) { say("RM", "map_cpu %s mmap errno=%d", what, errno); *st = 0xfffd0000u | (uint32_t)errno; return NULL; }
  say("RM", "map_cpu %s ok len=0x%llx", what, (unsigned long long)len);
  return (volatile uint8_t *)p;
}

static uint32_t alloc_vidmem(const char *what, uint64_t size, uint64_t align, uint32_t *st) {
  NV_MEMORY_ALLOCATION_PARAMS p; memset(&p, 0, sizeof p);
  p.owner = g_hClient; p.type = 0; p.attr = 2u << 27 /* CONTIGUOUS, VIDMEM */; p.size = size; p.alignment = align;
  return rm_alloc(what, hDevice, NV01_MEMORY_LOCAL_USER, &p, sizeof p, st);
}

static int setup(void) {
  uint32_t st;
  g_dev_fd = open("/dev/nvidia0", O_RDWR);
  g_ctl = open("/dev/nvidiactl", O_RDWR);
  if (g_ctl < 0 || g_dev_fd < 0) { say("OBS", "SETUP-FAIL open ctl=%d dev=%d errno=%d", g_ctl, g_dev_fd, errno); return -1; }
  g_hClient = rm_alloc("client", 0, 0, NULL, 0, &st);
  if (!g_hClient) return -1;
  NV0000_CTRL_GPU_GET_ATTACHED_IDS_PARAMS aid; memset(&aid, 0xff, sizeof aid);
  if (rm_control("ATTACHED_IDS", g_hClient, NV0000_CTRL_CMD_GPU_GET_ATTACHED_IDS, &aid, sizeof aid)) return -1;
  NV0000_CTRL_GPU_GET_ID_INFO_PARAMS ii; memset(&ii, 0, sizeof ii); ii.gpuId = aid.gpuIds[0];
  if (rm_control("ID_INFO", g_hClient, NV0000_CTRL_CMD_GPU_GET_ID_INFO, &ii, sizeof ii)) return -1;
  NV0080_ALLOC_PARAMETERS dp; memset(&dp, 0, sizeof dp); dp.deviceId = ii.deviceInstance;
  hDevice = rm_alloc("device", g_hClient, NV01_DEVICE_0, &dp, sizeof dp, &st); if (!hDevice) return -1;
  NV2080_ALLOC_PARAMETERS sp; memset(&sp, 0, sizeof sp); sp.subDeviceId = ii.subDeviceInstance;
  hSub = rm_alloc("subdevice", hDevice, NV20_SUBDEVICE_0, &sp, sizeof sp, &st); if (!hSub) return -1;
  NV_VASPACE_ALLOCATION_PARAMETERS vp; memset(&vp, 0, sizeof vp);
  hVAS = rm_alloc("vaspace", hDevice, FERMI_VASPACE_A, &vp, sizeof vp, &st); if (!hVAS) return -1;
  NV_MEMORY_VIRTUAL_ALLOCATION_PARAMS mv; memset(&mv, 0, sizeof mv); mv.hVASpace = hVAS;
  hVirt = rm_alloc("virtual", hDevice, NV01_MEMORY_VIRTUAL, &mv, sizeof mv, &st); if (!hVirt) return -1;
  hRing = alloc_vidmem("ring", RING_BYTES, 0x10000, &st); if (!hRing) return -1;
  /* error notifier: vidmem by default (same path as the ring); SETOBJ_NOTIF=sys selects system memory */
  if (getenv("SETOBJ_NOTIF") && !strcmp(getenv("SETOBJ_NOTIF"), "sys")) {
    NV_MEMORY_ALLOCATION_PARAMS p; memset(&p, 0, sizeof p);
    p.owner = g_hClient; p.attr = 1u << 25 /* LOCATION_PCI */; p.size = 0x1000; p.alignment = 0x1000;
    hNotif = rm_alloc("notifier(sys)", hDevice, NV01_MEMORY_SYSTEM, &p, sizeof p, &st);
  } else {
    hNotif = alloc_vidmem("notifier(vid)", 0x10000, 0x10000, &st);
  }
  if (!hNotif) return -1;
  ring = map_cpu("ring", hRing, RING_BYTES, &st); if (!ring) return -1;
  notifp = map_cpu("notifier", hNotif, 0x10000, &st);
  if (!notifp) say("OBS", "notifier not CPU-mappable (status 0x%x): D2 will read as unavailable", st);
  memset((void *)ring, 0, RING_BYTES); if (notifp) memset((void *)notifp, 0, 0x10000);
  NVOS46_PARAMETERS dm; memset(&dm, 0, sizeof dm);
  dm.hClient = g_hClient; dm.hDevice = hDevice; dm.hDma = hVirt; dm.hMemory = hRing; dm.length = RING_BYTES;
  int rc = ioctl(g_ctl, _IOWR(NV_IOCTL_MAGIC, NV_ESC_RM_MAP_MEMORY_DMA_, NVOS46_PARAMETERS), &dm);
  say("RM", "map_dma ring -> va=0x%llx status=0x%x rc=%d", (unsigned long long)dm.dmaOffset, dm.status, rc);
  if (rc || dm.status) return -1;
  ringVA = dm.dmaOffset;
  /* TSG on the GR engine */
  NV_CHANNEL_GROUP_ALLOCATION_PARAMETERS tp; memset(&tp, 0, sizeof tp);
  tp.hVASpace = hVAS; tp.engineType = NV2080_ENGINE_TYPE_GRAPHICS;
  hTSG = rm_alloc("tsg", hDevice, KEPLER_CHANNEL_GROUP_A, &tp, sizeof tp, &st); if (!hTSG) return -1;
  NV_CHANNEL_ALLOC_PARAMS cp; memset(&cp, 0, sizeof cp);
  cp.hObjectError = hNotif; cp.gpFifoOffset = ringVA + GP_OFF; cp.gpFifoEntries = GP_ENTRIES;
  cp.hUserdMemory[0] = hRing; cp.userdOffset[0] = USERD_OFF; cp.engineType = NV2080_ENGINE_TYPE_GRAPHICS;
  hChan = rm_alloc("channel", hTSG, AMPERE_CHANNEL_GPFIFO_A, &cp, sizeof cp, &st);
  if (!hChan) {
    say("OBS", "channel alloc with error notifier FAILED (0x%x): retrying WITHOUT a notifier (D2 unavailable)", st);
    cp.hObjectError = 0; hNotif = 0;
    hChan = rm_alloc("channel(no notifier)", hTSG, AMPERE_CHANNEL_GPFIFO_A, &cp, sizeof cp, &st);
    if (!hChan) return -1;
  }
  NVA06F_CTRL_BIND_PARAMS bp; memset(&bp, 0, sizeof bp); bp.engineType = NV2080_ENGINE_TYPE_GRAPHICS;
  if (rm_control("BIND", hTSG, NVA06C_CTRL_CMD_BIND, &bp, sizeof bp)) return -1;
  NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN_PARAMS tk; memset(&tk, 0, sizeof tk);
  if (rm_control("WORK_SUBMIT_TOKEN", hChan, NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN, &tk, sizeof tk)) return -1;
  token = tk.workSubmitToken;
  hUsermode = rm_alloc("usermode", hSub, AMPERE_USERMODE_A, NULL, 0, &st); if (!hUsermode) return -1;
  usermode = map_cpu("usermode(hDevice)", hUsermode, 0x10000, &st);
  if (!usermode) {
    uint32_t saved = hDevice; hDevice = hSub;
    usermode = map_cpu("usermode(hSubDevice)", hUsermode, 0x10000, &st);
    hDevice = saved;
  }
  if (!usermode) return -1;
  return 0;
}

static int sched(void) {
  NVA06F_CTRL_GPFIFO_SCHEDULE_PARAMS sp; memset(&sp, 0, sizeof sp); sp.bEnable = 1;
  return rm_control("SCHEDULE", hTSG, NVA06C_CTRL_CMD_GPFIFO_SCHEDULE, &sp, sizeof sp) ? -1 : 0;
}

/* ------------------------------------------------------------------ pushbuffer builder */
static uint32_t pb[512]; static int pbn;
static void pb_reset(void) { pbn = 0; }
static void pb_w(uint32_t w) { if (pbn < 512) pb[pbn++] = w; }
/* type-1 incrementing: SEC_OP=1 (31:29), COUNT 28:16, SUBCHANNEL 15:13, ADDRESS (method>>2) 11:0 */
static uint32_t hdr_inc(unsigned sub, unsigned method, unsigned count) {
  return (1u << 29) | (count << 16) | (sub << 13) | ((method >> 2) & 0xfff);
}
static void pb_set_object(unsigned sub, uint32_t v) { pb_w(hdr_inc(sub, 0x0, 1)); pb_w(v); }
static void pb_sw(unsigned sub, unsigned method, uint32_t data) { pb_w(hdr_inc(sub, method, 1)); pb_w(data); }
/* host semaphore release: SEMAPHOREA/B/C/D at 0x10..0x1c, on subchannel 0 (a host method). */
static void pb_sem(int slot, uint32_t payload) {
  uint64_t a = ringVA + SEM_OFF + 16 * slot;
  pb_w(hdr_inc(0, 0x10, 4));
  pb_w((uint32_t)(a >> 32) & 0xff); pb_w((uint32_t)a & ~3u); pb_w(payload);
  pb_w(0x02u /* OPERATION_RELEASE */ | (1u << 24) /* 4-byte */);
}
/* compute-class report semaphore release (engine work): methods 0x158..0x168. */
static void pb_cs(unsigned sub, int slot, uint32_t payload) {
  uint64_t a = ringVA + SEM_OFF + 16 * slot;
  pb_w(hdr_inc(sub, 0x158, 5));
  pb_w(payload); pb_w(0); pb_w((uint32_t)a); pb_w((uint32_t)(a >> 32) & 0xff);
  pb_w(0x0u /* RELEASE */ | (1u << 3) /* ONE_WORD */);
}

static void submit(void) {
  /* copy the words into the pushbuffer area, add one GP entry, publish GP_PUT, ring the doorbell */
  uint32_t start = pb_off;
  for (int i = 0; i < pbn; i++) *(volatile uint32_t *)(ring + PB_OFF + pb_off + 4 * i) = pb[i];
  pb_off += 4 * pbn; pb_off = (pb_off + 0xf) & ~0xfu;
  uint64_t va = ringVA + PB_OFF + start;
  uint32_t e0 = (uint32_t)va & ~3u;
  uint32_t e1 = ((uint32_t)(va >> 32) & 0xff) | ((uint32_t)pbn << 10);
  volatile uint32_t *gp = (volatile uint32_t *)(ring + GP_OFF + 8 * (gp_put % GP_ENTRIES));
  gp[0] = e0; gp[1] = e1;
  gp_put = (gp_put + 1) % GP_ENTRIES;
  __sync_synchronize();
  *(volatile uint32_t *)(ring + USERD_OFF + USERD_GP_PUT) = gp_put;
  __sync_synchronize();
  *(volatile uint32_t *)(usermode + 0x90) = token;
  __sync_synchronize();
  say("PB", "submit words=%d gp_put=%u pb@0x%x:", pbn, gp_put, start);
  for (int i = 0; i < pbn; i++) fprintf(g_out, "PBW %d 0x%08x\n", i, pb[i]);
  fflush(g_out);
}

static int wait_sem(int slot, uint32_t payload, int ms) {
  double dl = now() + ms / 1000.0;
  while (now() < dl) {
    if (*semp(slot) == payload) return 1;
    usleep(500);
  }
  return *semp(slot) == payload;
}
static uint32_t notif_status(char *buf, size_t n) {
  volatile notif_t *nt = (volatile notif_t *)notifp;
  if (!hNotif || !notifp) { snprintf(buf, n, "(no notifier)"); return 0; }
  uint32_t bad = 0;
  buf[0] = 0;
  for (int i = 0; i < 2; i++) {
    snprintf(buf + strlen(buf), n - strlen(buf), "n%d{info32=0x%x info16=0x%x status=0x%x} ", i, nt[i].info32, nt[i].info16, nt[i].status);
    if (i == 0 && (nt[i].info32 || nt[i].status || nt[i].info16)) bad = 1;
  }
  return bad;
}

/* One step = the words already in pb[] + a semaphore release; returns 1 iff the release arrived. */
static int g_sem_n;
static int step(const char *label) {
  int slot = ++g_sem_n; uint32_t pay = 0x5e000000u | (uint32_t)slot;
  pb_sem(slot, pay);
  submit();
  int ok = wait_sem(slot, pay, 3000);
  char nb[200]; uint32_t bad = notif_status(nb, sizeof nb);
  say("RES", "step=%s sem_arrived=%d notifier_error=%u %s", label, ok, bad, nb);
  pb_reset();
  return ok;
}

/* ------------------------------------------------------------------ objects */
static uint32_t alloc_sw(const char *what, uint32_t *classid) {
  uint32_t st; NV5080_ALLOC_PARAMS ap; memset(&ap, 0, sizeof ap);
  uint32_t h = rm_alloc(what, hChan, NV50_DEFERRED_API_CLASS, &ap, sizeof ap, &st);
  *classid = 0;
  if (!h) { say("RES", "alloc_sw %s FAILED status=0x%x", what, st); return 0; }
  NV906F_CTRL_GET_CLASS_ENGINEID_PARAMS g; memset(&g, 0, sizeof g); g.hObject = h;
  uint32_t s2 = rm_control("GET_CLASS_ENGINEID", hChan, NV906F_CTRL_GET_CLASS_ENGINEID, &g, sizeof g);
  say("RES", "alloc_sw %s ok h=0x%x classEngineID=0x%x classID=0x%x engineID=0x%x ctlstatus=0x%x", what, h, g.classEngineID, g.classID, g.engineID, s2);
  *classid = s2 ? 0 : g.classEngineID;
  return h;
}

/* registration oracle ---------------------------------------------------------------- */
static uint32_t register_api(const char *what, uint32_t hObj, uint32_t hApi) {
  NV5080_CTRL_DEFERRED_API_PARAMS p; memset(&p, 0, sizeof p);
  p.hApiHandle = hApi; p.cmd = NV2080_CTRL_CMD_DMA_INVALIDATE_TLB;
  p.flags = NV5080_CTRL_CMD_DEFERRED_API_FLAGS_DELETE_IMPLICIT;
  p.hClientVA = g_hClient; p.hDeviceVA = hSub;
  p.api_bundle.InvalidateTlb.hVASpace = hVAS;
  return rm_control(what, hObj, NV5080_CTRL_CMD_DEFERRED_API, &p, sizeof p);
}
static uint32_t remove_api(const char *what, uint32_t hObj, uint32_t hApi) {
  NV5080_CTRL_REMOVE_API_PARAMS p; memset(&p, 0, sizeof p); p.hApiHandle = hApi;
  return rm_control(what, hObj, NV5080_CTRL_CMD_REMOVE_API, &p, sizeof p);
}
/* Register H on every object in objs[], send SW(sub, 0x200, H), wait, REMOVE_API(H) on each.
 * REMOVE_API failing = the entry is gone = that object's handler executed it. */
static int reach(const char *label, unsigned sub, uint32_t *objs, const char **names, int n, uint32_t H) {
  int reg_ok = 1;
  for (int i = 0; i < n; i++) {
    uint32_t s = register_api(names[i], objs[i], H);
    if (s) reg_ok = 0;
    say("RES", "reach %s register on %s status=0x%x", label, names[i], s);
  }
  if (!reg_ok) { say("RES", "reach %s UNOBSERVED (registration refused)", label); return -1; }
  pb_reset(); pb_sw(sub, 0x200, H);
  int ok = step(label);
  int executed = -1;
  for (int i = 0; i < n; i++) {
    uint32_t s = remove_api(names[i], objs[i], H);
    say("RES", "reach %s remove_api(%s) status=0x%x => %s", label, names[i], s,
        s == 0 ? "NOT executed (entry still present)" : "executed (entry gone) or object gone");
    if (s) executed = i;
  }
  say("RES", "reach %s sem_arrived=%d executed_by=%s", label, ok, executed < 0 ? "none" : names[executed]);
  return ok ? executed : -2;
}

/* ------------------------------------------------------------------ cases */
static void do_compute(void) {
  uint32_t st;
  if (hCompute) return;
  hCompute = rm_alloc("compute ADA_COMPUTE_A", hChan, ADA_COMPUTE_A, NULL, 0, &st);
  if (!hCompute) say("RES", "compute alloc FAILED status=0x%x", st);
}
static void do_3d(void) {
  uint32_t st;
  if (h3d) return;
  h3d = rm_alloc("3d ADA_A", hChan, ADA_A, NULL, 0, &st);
  if (!h3d) say("RES", "3d alloc FAILED status=0x%x", st);
}
#define SWENG(v) (v)

static int case_run(const char *c) {
  uint32_t A = 0, B = 0, cidA = 0, cidB = 0;

  /* ---- S4 controls ---- */
  if (!strcmp(c, "c_ok")) {
    do_compute();
    pb_reset(); pb_set_object(1, ADA_COMPUTE_A); step("SO(1,compute)");
    pb_reset(); pb_cs(1, 20, 0xc0de0001); step("compute report-semaphore on sub1");
    say("RES", "engine_sem slot20=0x%x (expect c0de0001: engine work on a hardware subchannel is detectable)", *semp(20));
    return 0;
  }
  if (!strcmp(c, "c_badclass")) {
    do_compute();
    pb_reset(); pb_set_object(1, ADA_COMPUTE_A); step("SO(1,compute) [good]");
    pb_reset(); pb_set_object(2, 0xdead); step("SO(2,0xdead) [INVALID class: fault expected]");
    pb_reset(); step("tail (channel alive?)");
    return 0;
  }
  if (!strcmp(c, "c_badsw")) {
    do_compute();
    A = alloc_sw("A", &cidA); if (!A) return 1;
    pb_reset(); pb_set_object(6, cidA); pb_sw(6, 0x100, 0); step("SO(6,A) + NOP [good]");
    pb_reset(); pb_sw(6, 0x300, 0); step("SW(6,0x300) [method outside the 5080 table: fault expected]");
    pb_reset(); step("tail (channel alive?)");
    return 0;
  }
  if (!strcmp(c, "c_hw_rebind")) {          /* S4d */
    do_compute();
    pb_reset(); pb_set_object(1, ADA_COMPUTE_A); step("SO(1,compute) #1");
    pb_reset(); pb_set_object(1, ADA_COMPUTE_A); step("SO(1,compute) #2 (same object, same hw subchannel)");
    pb_reset(); pb_cs(1, 20, 0xc0de0002); step("compute report-semaphore on sub1 after rebind");
    say("RES", "engine_sem slot20=0x%x (expect c0de0002)", *semp(20));
    return 0;
  }
  if (!strcmp(c, "c_hw_two")) {             /* S4e */
    do_compute();
    pb_reset(); pb_set_object(1, ADA_COMPUTE_A); pb_set_object(2, ADA_COMPUTE_A); step("SO(1,compute) + SO(2,compute)");
    pb_reset(); pb_cs(2, 20, 0xc0de0003); step("compute report-semaphore on sub2");
    say("RES", "engine_sem slot20=0x%x (expect c0de0003)", *semp(20));
    return 0;
  }
  if (!strcmp(c, "r0")) {                   /* oracle self-test */
    do_compute();
    A = alloc_sw("A", &cidA); if (!A) return 1;
    pb_reset(); pb_set_object(6, cidA); pb_sw(6, 0x100, 0); step("SO(6,A) + NOP");
    uint32_t s1 = register_api("register H1", A, 0xde000001), s2 = register_api("register H2", A, 0xde000002);
    say("RES", "r0 register H1=0x%x H2=0x%x", s1, s2);
    pb_reset(); pb_sw(6, 0x200, 0xde000001); step("SW(6,0x200,H1)");
    uint32_t r1 = remove_api("remove H1 (triggered: expect FAIL=executed)", A, 0xde000001);
    uint32_t r2 = remove_api("remove H2 (never triggered: expect OK=not executed)", A, 0xde000002);
    say("RES", "r0 ORACLE %s (remove H1 triggered=0x%x, remove H2 untriggered=0x%x)",
        (r1 != 0 && r2 == 0) ? "VALID" : "INVALID", r1, r2);
    return 0;
  }
  /* ---- S1 ---- */
  if (!strcmp(c, "s1") || !strcmp(c, "s1r")) {
    int withr = !strcmp(c, "s1r");
    do_compute();
    A = alloc_sw("A", &cidA); if (!A) return 1;
    pb_reset(); pb_set_object(6, cidA); pb_sw(6, 0x100, 0); step("SO(6,A) #1 + NOP");
    pb_reset(); pb_set_object(6, cidA); pb_sw(6, 0x100, 0); pb_sw(6, 0x101, 0); step("SO(6,A) #2 (identical) + NOP + NOP(0x101)");
    pb_reset(); pb_sw(6, 0x100, 0); step("NOP after second bind");
    if (withr) { const char *nm[] = {"A"}; uint32_t o[] = {A}; reach("S1 reach on sub6 after re-bind", 6, o, nm, 1, 0xde100001); }
    return 0;
  }
  /* ---- S2 family ---- */
  if (!strncmp(c, "s2", 2) && strcmp(c, "s2e5") && strcmp(c, "s2e7") && strncmp(c, "s2e_hw", 6)) {
    int withr = c[strlen(c) - 1] == 'r';
    char base[16]; strncpy(base, c, sizeof base - 1); base[sizeof base - 1] = 0;
    if (withr) base[strlen(base) - 1] = 0;
    int bsub = 6;
    do_compute();
    if (!strcmp(base, "s2d")) {            /* B allocated BEFORE the first submission */
      A = alloc_sw("A", &cidA); if (!A) return 1;
      B = alloc_sw("B (before first submission)", &cidB);
      pb_reset(); pb_set_object(6, cidA); pb_sw(6, 0x100, 0); step("SO(6,A) + NOP (first work)");
      if (!B) return 0;
      pb_reset(); pb_set_object(6, cidB); pb_sw(6, 0x100, 0); step("SO(6,B) [second bind of sub6, different object]");
    } else {
      A = alloc_sw("A", &cidA); if (!A) return 1;
      pb_reset(); pb_set_object(6, cidA); pb_sw(6, 0x100, 0); step("SO(6,A) + NOP (first work)");
      if (!strcmp(base, "s2b")) { rm_free("A (bound on sub6)", hChan, A); A = 0; }
      B = alloc_sw("B (after the channel ran work)", &cidB);
      if (!B) { say("RES", "S2 verdict-part: alloc of B REJECTED"); return 0; }
      if (!strcmp(base, "s2c5") || !strcmp(base, "s2c7")) bsub = base[3] == '5' ? 5 : 7;
      pb_reset(); pb_set_object(bsub, cidB); pb_sw(bsub, 0x100, 0); pb_sw(6, 0x100, 0);
      step(bsub == 6 ? "SO(6,B) [rebind sub6 to a later object]" : "SO(B) on a DIFFERENT software subchannel; A stays on 6");
    }
    if (withr && B) {
      const char *nm[2] = {"A", "B"}; uint32_t o[2] = {A, B}; int n = 2;
      if (!A) { nm[0] = "B"; o[0] = B; n = 1; }
      reach("S2 reach: 0x200 on the (re)bound subchannel", bsub, o, nm, n, 0xde200001);
      if (bsub != 6 && A) { const char *na[1] = {"A"}; uint32_t oa[1] = {A}; reach("S2c reach: 0x200 on sub6 (A still bound)", 6, oa, na, 1, 0xde200002); }
    }
    return 0;
  }
  /* ---- S2e ---- */
  if (!strcmp(c, "s2e5") || !strcmp(c, "s2e7") || !strcmp(c, "s2e5r") || !strcmp(c, "s2e7r")) {
    int sub2 = c[3] == '5' ? 5 : 7, withr = c[4] == 'r';
    do_compute();
    A = alloc_sw("A", &cidA); if (!A) return 1;
    pb_reset(); pb_set_object(6, cidA); pb_sw(6, 0x100, 0); step("SO(6,A) + NOP (first work)");
    pb_reset(); pb_set_object(sub2, cidA); step("SO(second sub,A) same object, second software subchannel");
    pb_reset(); pb_sw(sub2, 0x100, 0); pb_sw(6, 0x100, 0); step("NOP on both subchannels");
    if (withr) {
      const char *nm[] = {"A"}; uint32_t o[] = {A};
      reach("S2e reach: 0x200 on the SECOND subchannel", sub2, o, nm, 1, 0xde300001);
      reach("S2e reach: 0x200 on sub6", 6, o, nm, 1, 0xde300002);
    }
    return 0;
  }
  if (!strncmp(c, "s2e_hw", 6)) {           /* SO(A) on a hardware subchannel k (digit after s2e_hw), enc: 'n' plain, 'e' with ENGINE_SW */
    int k = c[6] - '0'; int enc = c[7] == 'e';
    do_compute();
    A = alloc_sw("A", &cidA); if (!A) return 1;
    pb_reset(); pb_set_object(6, cidA); pb_sw(6, 0x100, 0); step("SO(6,A) + NOP (first work)");
    uint32_t v = enc ? (cidA | (0x1fu << 16)) : cidA;
    pb_reset(); pb_set_object(k, v); step(enc ? "SO(hw k,A with ENGINE_SW bits)" : "SO(hw k,A plain classID)");
    pb_reset(); step("tail");
    return 0;
  }
  /* ---- encodings on 6 ---- */
  if (!strcmp(c, "enc")) {
    do_compute();
    A = alloc_sw("A", &cidA); if (!A) return 1;
    pb_reset(); pb_set_object(6, cidA); pb_sw(6, 0x100, 0); step("SO(6,A plain)");
    pb_reset(); pb_set_object(6, cidA | (0x1fu << 16)); pb_sw(6, 0x100, 0); step("SO(6,A | ENGINE_SW<<16)");
    return 0;
  }
  /* ---- S3: engine class on a software subchannel ---- */
  if (!strncmp(c, "s3_", 3)) {              /* s3_<sub>[_3d] */
    int sub = c[3] - '0'; int is3d = strstr(c, "_3d") != NULL;
    do_compute(); if (is3d) do_3d();
    uint32_t cls = is3d ? ADA_A : ADA_COMPUTE_A;
    pb_reset(); pb_set_object(sub, cls); step(is3d ? "SO(sw sub,3D)" : "SO(sw sub,compute)");
    if (!is3d) {
      pb_reset(); pb_cs(sub, 20, 0xc0de0010 + sub); step("compute report-semaphore on that subchannel");
      say("RES", "engine_sem slot20=0x%x (0xc0de001%d would mean engine work executed on a software subchannel)", *semp(20), sub);
    } else {
      /* 3D class has no cheap observable method without more state; the NOP below only shows no fault */
      pb_reset(); pb_sw(sub, 0x100, 0); step("method 0x100 (3D NO_OPERATION) after bind");
    }
    return 0;
  }
  say("OBS", "unknown case %s", c);
  return 2;
}

int main(int argc, char **argv) {
  g_out = stdout; t0 = now(); g_pid = getpid();
  prctl(PR_SET_NAME, "setobj_probe");
  if (argc < 2) { fprintf(stderr, "usage: %s <case> [rep]\n", argv[0]); return 2; }
  say("OBS", "START case=%s rep=%s pid=%d comm=setobj_probe", argv[1], argc > 2 ? argv[2] : "0", g_pid);
  if (setup()) { say("OBS", "SETUP-FAIL (see RM lines above)"); return 3; }
  do_compute();
  if (strstr(argv[1], "_3d")) do_3d();
  if (sched()) { say("OBS", "SCHEDULE-FAIL"); return 3; }
  say("OBS", "channel scheduled: hClient=0x%x hChan=0x%x token=0x%x ringVA=0x%llx", g_hClient, hChan, token, (unsigned long long)ringVA);
  int r = case_run(argv[1]);
  say("OBS", "END case=%s rc=%d", argv[1], r);
  return r;
}
