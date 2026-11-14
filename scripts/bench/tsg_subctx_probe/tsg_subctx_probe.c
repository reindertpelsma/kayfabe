// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
/*
 * tsg_subctx_probe.c -- can ONE TSG hold several REAL context shares (subcontexts) and channels of
 * several engine types, as an UNPRIVILEGED RM client?  Study: docs/design/V3_TSG_SUBCONTEXT_FEASIBILITY_20261018.md.
 *
 * A plain C program over /dev/nvidiactl in the style of scripts/bench/setobj/setobj_probe.c (branch
 * claude/setobj-20261016, the harness whose positive/negative controls were measured on this host): public
 * headers only (open-gpu-kernel-modules 595.x, include path given by the build line in run_probe.sh), no closed
 * binary read, no kayfabe crate touched.
 *
 * One process = one fresh RM client.  The shape is a SCRIPT (one command per line, see below) so that one binary
 * covers every shape and every ordering.  Output: "RM" lines (every RM call and its status), "RES" lines
 * (observations), "PB" lines (pushbuffer words).  The runner brackets each run with a host-journal window and
 * collects Xid lines for the probe's pid.
 *
 * Commands (arguments are key=value):
 *   vas N                          create VA space N (1..3); VA space 0 is created at start
 *   tsg engine=gr|ce0|ce1|none [vas=N]                      NV_CHANNEL_GROUP_ALLOCATION_PARAMETERS (KEPLER_CHANNEL_GROUP_A)
 *   ctx NAME flags=sync|async|spec:ID|prefer [vas=N|none]    FERMI_CONTEXT_SHARE_A under the TSG
 *   chan NAME [ctx=NAME|none] engine=gr|ce0|ce1 [vas=N|none] [ringvas=N] [objs=3d+cmp+ce] [objafter=1]
 *             [thread=0|1|2] (legacy mode: NVOS04_FLAGS_GROUP_CHANNEL_THREAD) [delay=1] [rawflags=0x..]
 *                                  channel alloc under the TSG (AMPERE_CHANNEL_GPFIFO_A); objs are allocated right
 *                                  after the channel unless objafter=1 (then use `obj`)
 *   obj NAME 3d+cmp+ce             allocate engine objects (ADA_A c997, ADA_COMPUTE_A c9c0, AMPERE_DMA_COPY_B c7b5)
 *   bindtsg [engine=gr|ce0]        NVA06C_CTRL_CMD_BIND on the TSG
 *   bindchan NAME [engine=..]      NVA06F_CTRL_CMD_BIND on the channel
 *   token NAME                     NVC36F GET_WORK_SUBMIT_TOKEN
 *   schedtsg [enable=1] [skipsubmit=0]   NVA06C_CTRL_CMD_GPFIFO_SCHEDULE on the TSG
 *   schedchan NAME                 NVA06F_CTRL_CMD_GPFIFO_SCHEDULE on the channel
 *   timeslice us=N                 NVA06C_CTRL_CMD_SET_TIMESLICE on the TSG
 *   work NAME[,NAME...] [objs=3d+cmp+ce] [wait=ms]   engine work (SET_OBJECT + class semaphore release per object) on
 *                                  each named channel, all submitted before any is waited for; host semaphore last
 *   bench NAME[,NAME...] [iters=N]   N rounds of one engine-semaphore release per channel, all in flight together (latency stats)
 *   (any command except `tsg` accepts tsg=K to select TSG #K; `tsg` creates TSG #0,#1,... and selects it)
 *   settle ms                      sleep, then read every notifier
 *   freech NAME | freectx NAME | freetsg
 *   info                           NV2080 FIFO_GET_INFO (max subcontexts per group, max lower subcontext)
 *   # comment
 */
#define _GNU_SOURCE
#include <ctype.h>
#include <errno.h>
#include <fcntl.h>
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
#include "class/cla06c.h"
#include "class/cl9067.h"
#include "class/cl90f1.h"
#include "class/cl5080.h"
#include "class/clc56f.h"
#include "class/clc561.h"
#include "class/clc9c0.h"
#include "class/clc997.h"
#include "class/clc7b5.h"
#include "ctrl/ctrla06c.h"
#include "ctrl/ctrla06f/ctrla06fgpfifo.h"
#include "ctrl/ctrlc36f.h"
#include "ctrl/ctrl0000/ctrl0000gpu.h"
#include "ctrl/ctrl2080/ctrl2080fifo.h"

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
  char b[1536]; va_list ap; va_start(ap, fmt); vsnprintf(b, sizeof b, fmt, ap); va_end(ap);
  struct timespec rt; clock_gettime(CLOCK_REALTIME, &rt);
  fprintf(g_out, "%s t=%.3f e=%ld.%03ld %s\n", kind, now() - t0, (long)rt.tv_sec, rt.tv_nsec / 1000000, b); fflush(g_out);
}

/* ------------------------------------------------------------------ RM escapes */
static uint32_t g_hClient;
static uint32_t g_nexth = 0xb00b0000;
static uint32_t mint(void) { return ++g_nexth; }

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

/* ------------------------------------------------------------------ state */
#define MAXV 4
#define MAXC 8
#define MAXCTX 80
#define MAXH 12
#define RING_BYTES 0x20000ull
#define GP_OFF 0x0000u
#define GP_ENTRIES 512u
#define USERD_OFF 0x1000u
#define SEM_OFF 0x2000u        /* 16 bytes per slot */
#define PB_OFF 0x4000u
#define USERD_GP_PUT 0x8c

enum { O_3D = 0, O_CMP = 1, O_CE = 2, O_N = 3 };
static const char *oname[O_N] = {"3d", "cmp", "ce"};
static const uint32_t oclass[O_N] = {ADA_A, ADA_COMPUTE_A, AMPERE_DMA_COPY_B};
static const unsigned osub[O_N] = {0, 1, 4};   /* NVA06F_SUBCHANNEL_*: graphics 0, compute 1, copy 4 */

#define MAXT 6
static uint32_t hDevice, hSub, hUsermode;
static struct { uint32_t h, vas; int engine; } tsgs[MAXT];
static int ntsg, curtsg = -1;
#define hTSG (tsgs[curtsg].h)
static int g_tsg_engine;
static volatile uint8_t *usermode;
static struct { uint32_t hVAS, hVirt; } vas[MAXV];
static struct { char name[16]; uint32_t h; int flags; uint32_t subctxId; int tsgi; } ctx[MAXCTX];
static int nctx;
static struct {
  char name[16]; uint32_t h, hRing, hNotif, token; volatile uint8_t *ring, *notif; uint64_t ringVA;
  uint32_t gp_put, pb_off; int ctxi, engine, ringvas; int sem_n; int alive; int tsgi;
  uint32_t hObj[O_N]; int has[O_N];
} ch[MAXC];
static int nch;

static uint32_t pb[512]; static int pbn;
static void pb_w(uint32_t w) { if (pbn < 512) pb[pbn++] = w; }
static uint32_t hdr_inc(unsigned sub, unsigned method, unsigned count) {
  return (1u << 29) | (count << 16) | (sub << 13) | ((method >> 2) & 0xfff);
}

static volatile uint8_t *map_cpu(const char *what, uint32_t hMem, uint64_t len, uint32_t *st) {
  int fd = open("/dev/nvidia0", O_RDWR);
  if (fd < 0) { *st = 0xfffe0000; return NULL; }
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
  p.owner = g_hClient; p.type = 0; p.attr = 2u << 27; p.size = size; p.alignment = align;
  return rm_alloc(what, hDevice, NV01_MEMORY_LOCAL_USER, &p, sizeof p, st);
}
static int make_vas(int i) {
  uint32_t st;
  NV_VASPACE_ALLOCATION_PARAMETERS vp; memset(&vp, 0, sizeof vp);
  vas[i].hVAS = rm_alloc("vaspace", hDevice, FERMI_VASPACE_A, &vp, sizeof vp, &st); if (!vas[i].hVAS) return -1;
  NV_MEMORY_VIRTUAL_ALLOCATION_PARAMS mv; memset(&mv, 0, sizeof mv); mv.hVASpace = vas[i].hVAS;
  vas[i].hVirt = rm_alloc("virtual", hDevice, NV01_MEMORY_VIRTUAL, &mv, sizeof mv, &st); if (!vas[i].hVirt) return -1;
  return 0;
}
static int setup(void) {
  uint32_t st;
  int fd = open("/dev/nvidia0", O_RDWR); (void)fd;
  g_ctl = open("/dev/nvidiactl", O_RDWR);
  if (g_ctl < 0) { say("OBS", "SETUP-FAIL open ctl errno=%d", errno); return -1; }
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
  if (make_vas(0)) return -1;
  hUsermode = rm_alloc("usermode", hSub, AMPERE_USERMODE_A, NULL, 0, &st); if (!hUsermode) return -1;
  usermode = map_cpu("usermode(hDevice)", hUsermode, 0x10000, &st);
  if (!usermode) {
    uint32_t saved = hDevice; hDevice = hSub;
    usermode = map_cpu("usermode(hSubDevice)", hUsermode, 0x10000, &st);
    hDevice = saved;
  }
  return usermode ? 0 : -1;
}

/* ------------------------------------------------------------------ parsing helpers */
static int engine_of(const char *s) {
  if (!strcmp(s, "gr")) return NV2080_ENGINE_TYPE_GRAPHICS;
  if (!strcmp(s, "ce0")) return NV2080_ENGINE_TYPE_COPY0;
  if (!strcmp(s, "ce1")) return NV2080_ENGINE_TYPE_COPY1;
  if (!strcmp(s, "none")) return NV2080_ENGINE_TYPE_NULL;
  return -1;
}
static const char *kv(char **tok, int n, const char *key) {
  size_t l = strlen(key);
  for (int i = 0; i < n; i++) if (!strncmp(tok[i], key, l) && tok[i][l] == '=') return tok[i] + l + 1;
  return NULL;
}
static int find_ctx(const char *name) {
  if (!name || !strcmp(name, "none")) return -1;
  for (int i = 0; i < nctx; i++) if (!strcmp(ctx[i].name, name)) return i;
  return -2;
}
static int find_ch(const char *name) {
  for (int i = 0; i < nch; i++) if (!strcmp(ch[i].name, name)) return i;
  return -1;
}
static int parse_objs(const char *s) {
  int m = 0;
  if (!s) return 0;
  if (strstr(s, "3d")) m |= 1 << O_3D;
  if (strstr(s, "cmp")) m |= 1 << O_CMP;
  if (strstr(s, "ce")) m |= 1 << O_CE;
  return m;
}

/* ------------------------------------------------------------------ channel work */
static volatile uint32_t *semp(int c, int slot) { return (volatile uint32_t *)(ch[c].ring + SEM_OFF + 16 * slot); }

/* objects are allocated in the order named: 3d, cmp, ce, sw (NV50_DEFERRED_API_CLASS 0x5080, never used for work) */
static void alloc_objs(int c, const char *list) {
  char buf[64]; snprintf(buf, sizeof buf, "%s", list);
  for (char *t = strtok(buf, "+,"); t; t = strtok(NULL, "+,")) {
    uint32_t st = 0, h = 0; char w[64];
    if (!strcmp(t, "sw")) {
      NV5080_ALLOC_PARAMS ap; memset(&ap, 0, sizeof ap);
      snprintf(w, sizeof w, "%s.sw class", ch[c].name);
      h = rm_alloc(w, ch[c].h, NV50_DEFERRED_API_CLASS, &ap, sizeof ap, &st);
      say("RES", "obj %s.sw class=0x5080 status=0x%x", ch[c].name, st);
      continue;
    }
    int o = !strcmp(t, "3d") ? O_3D : !strcmp(t, "cmp") ? O_CMP : !strcmp(t, "ce") ? O_CE : -1;
    if (o < 0) { say("RES", "unknown object kind %s", t); continue; }
    snprintf(w, sizeof w, "%s.%s class", ch[c].name, oname[o]);
    ch[c].hObj[o] = rm_alloc(w, ch[c].h, oclass[o], NULL, 0, &st);
    ch[c].has[o] = ch[c].hObj[o] != 0;
    say("RES", "obj %s.%s class=0x%x status=0x%x", ch[c].name, oname[o], oclass[o], st);
  }
}

static uint32_t notif_bad(int c, char *buf, size_t n) {
  volatile notif_t *nt = (volatile notif_t *)ch[c].notif;
  buf[0] = 0;
  if (!ch[c].hNotif || !ch[c].notif) { snprintf(buf, n, "(no notifier)"); return 0; }
  snprintf(buf, n, "n0{info32=0x%x info16=0x%x status=0x%x}", nt[0].info32, nt[0].info16, nt[0].status);
  return nt[0].info32 || nt[0].status || nt[0].info16;
}

/* the work-submit token needs a bound channel (RM answers INVALID_STATE before that): ask on first use */
static int ensure_token(int c) {
  if (ch[c].token) return 0;
  NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN_PARAMS tk; memset(&tk, 0, sizeof tk);
  uint32_t st = rm_control("WORK_SUBMIT_TOKEN", ch[c].h, NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN, &tk, sizeof tk);
  say("RES", "token %s status=0x%x token=0x%x", ch[c].name, st, st ? 0 : tk.workSubmitToken);
  if (st) return -1;
  ch[c].token = tk.workSubmitToken; return 0;
}

static void submit(int c) {
  uint32_t start = ch[c].pb_off;
  for (int i = 0; i < pbn; i++) *(volatile uint32_t *)(ch[c].ring + PB_OFF + ch[c].pb_off + 4 * i) = pb[i];
  ch[c].pb_off += 4 * pbn; ch[c].pb_off = (ch[c].pb_off + 0xf) & ~0xfu;
  uint64_t va = ch[c].ringVA + PB_OFF + start;
  uint32_t e0 = (uint32_t)va & ~3u;
  uint32_t e1 = ((uint32_t)(va >> 32) & 0xff) | ((uint32_t)pbn << 10);
  volatile uint32_t *gp = (volatile uint32_t *)(ch[c].ring + GP_OFF + 8 * (ch[c].gp_put % GP_ENTRIES));
  gp[0] = e0; gp[1] = e1;
  ch[c].gp_put = (ch[c].gp_put + 1) % GP_ENTRIES;
  __sync_synchronize();
  *(volatile uint32_t *)(ch[c].ring + USERD_OFF + USERD_GP_PUT) = ch[c].gp_put;
  __sync_synchronize();
  *(volatile uint32_t *)(usermode + 0x90) = ch[c].token;
  __sync_synchronize();
  say("PB", "%s submit words=%d gp_put=%u", ch[c].name, pbn, ch[c].gp_put);
  for (int i = 0; i < pbn; i++) fprintf(g_out, "PBW %s %d 0x%08x\n", ch[c].name, i, pb[i]);
  fflush(g_out);
}

/* class semaphore release (3D / compute report semaphore, one word): methods 0x158..0x168 */
static void pb_cs(int c, unsigned sub, int slot, uint32_t payload) {
  uint64_t a = ch[c].ringVA + SEM_OFF + 16 * slot;
  pb_w(hdr_inc(sub, 0x158, 5));
  pb_w(payload); pb_w(0); pb_w((uint32_t)a); pb_w((uint32_t)(a >> 32) & 0xff);
  pb_w(0x0u | (1u << 3));
}
/* copy-engine semaphore release (LAUNCH_DMA with no transfer, release one word) */
static void pb_ce(int c, unsigned sub, int slot, uint32_t payload) {
  uint64_t a = ch[c].ringVA + SEM_OFF + 16 * slot;
  pb_w(hdr_inc(sub, 0x240, 3));
  pb_w((uint32_t)(a >> 32) & 0x1ffff); pb_w((uint32_t)a); pb_w(payload);
  pb_w(hdr_inc(sub, 0x300, 1));
  pb_w(0u /* DATA_TRANSFER_TYPE_NONE */ | (1u << 2) /* FLUSH_ENABLE */ | (1u << 3) /* RELEASE_ONE_WORD_SEMAPHORE */);
}
/* host semaphore release (host method, subchannel 0) */
static void pb_host_sem(int c, int slot, uint32_t payload) {
  uint64_t a = ch[c].ringVA + SEM_OFF + 16 * slot;
  pb_w(hdr_inc(0, 0x10, 4));
  pb_w((uint32_t)(a >> 32) & 0xff); pb_w((uint32_t)a & ~3u); pb_w(payload);
  pb_w(0x02u | (1u << 24));
}

#define MAXWORK 8
static void do_work(int *cs, int ncs, int mask_override, int wait_ms) {
  struct { int c; int slot[O_N]; uint32_t pay[O_N]; int hslot; uint32_t hpay; double t_sub; } w[MAXWORK];
  if (ncs > MAXWORK) ncs = MAXWORK;
  for (int k = 0; k < ncs; k++) {
    int c = cs[k]; w[k].c = c; pbn = 0;
    if (ensure_token(c)) { w[k].hslot = -1; continue; }
    for (int o = 0; o < O_N; o++) w[k].slot[o] = -1;
    for (int o = 0; o < O_N; o++) {
      if (!ch[c].has[o]) continue;
      if (mask_override >= 0 && !((mask_override >> o) & 1)) continue;
      int slot = ++ch[c].sem_n; w[k].slot[o] = slot; w[k].pay[o] = 0xc0de0000u | ((uint32_t)o << 8) | (uint32_t)slot;
      pb_w(hdr_inc(osub[o], 0x0, 1)); pb_w(oclass[o]);                 /* SET_OBJECT */
      if (o == O_CE) pb_ce(c, osub[o], slot, w[k].pay[o]); else pb_cs(c, osub[o], slot, w[k].pay[o]);
    }
    w[k].hslot = ++ch[c].sem_n; w[k].hpay = 0x5e000000u | (uint32_t)w[k].hslot;
    pb_host_sem(c, w[k].hslot, w[k].hpay);
    w[k].t_sub = now();
    submit(c);
  }
  /* wait for everything */
  for (int k = 0; k < ncs; k++) {
    if (w[k].hslot == -1) { say("RES", "work %s NOT SUBMITTED (no work-submit token)", ch[w[k].c].name); continue; }
    int c = w[k].c; double dl = w[k].t_sub + wait_ms / 1000.0; int hostok = 0; double th = -1;
    while (now() < dl) { if (*semp(c, w[k].hslot) == w[k].hpay) { hostok = 1; th = now() - w[k].t_sub; break; } usleep(200); }
    char line[512]; line[0] = 0;
    for (int o = 0; o < O_N; o++) if (w[k].slot[o] > 0) {
      uint32_t v = *semp(c, w[k].slot[o]);
      snprintf(line + strlen(line), sizeof line - strlen(line), " %s=%s(0x%x)", oname[o], v == w[k].pay[o] ? "OK" : "MISS", v);
    }
    char nb[160]; uint32_t bad = notif_bad(c, nb, sizeof nb);
    say("RES", "work %s host_sem=%s t=%.0fus engine_sems:%s notifier_error=%u %s", ch[c].name, hostok ? "OK" : "MISS",
        th * 1e6, line, bad, nb);
  }
}


/* bench: N iterations of "submit one engine-semaphore release to every named channel, wait for all"; per-iteration
 * wall time from the first submit to the last completion, plus per-channel completion time after the first submit.
 * Uses the first object each channel has (SET_OBJECT is sent once, in iteration 0). */
static int cmp_d(const void *a, const void *b) { double x = *(const double *)a, y = *(const double *)b; return x < y ? -1 : x > y; }
static void stats(const char *what, double *v, int n) {
  qsort(v, (size_t)n, sizeof *v, cmp_d);
  say("RES", "bench %s n=%d min=%.0f p10=%.0f med=%.0f p90=%.0f max=%.0f us", what, n, v[0] * 1e6, v[n / 10] * 1e6, v[n / 2] * 1e6, v[n * 9 / 10] * 1e6, v[n - 1] * 1e6);
}
static int do_bench(int *cs, int ncs, int iters) {
  static double tall[4000], tch[MAXWORK][4000]; int obj[MAXWORK], slot[MAXWORK], alive = 1;
  if (iters > 4000) iters = 4000;
  if (ncs > MAXWORK) ncs = MAXWORK;
  for (int k = 0; k < ncs; k++) {
    int c = cs[k]; obj[k] = -1;
    for (int o = 0; o < O_N; o++) if (ch[c].has[o]) { obj[k] = o; break; }
    if (obj[k] < 0) { say("RES", "bench: channel %s has no object", ch[c].name); return 1; }
    if (ensure_token(c)) return 1;
    slot[k] = ++ch[c].sem_n;
  }
  for (int it = 0; it < iters && alive; it++) {
    uint32_t pay = 0xb0000000u | (uint32_t)(it + 1);
    double t0i = now();
    for (int k = 0; k < ncs; k++) {
      int c = cs[k], o = obj[k]; pbn = 0;
      if (it == 0) { pb_w(hdr_inc(osub[o], 0x0, 1)); pb_w(oclass[o]); }
      if (o == O_CE) pb_ce(c, osub[o], slot[k], pay); else pb_cs(c, osub[o], slot[k], pay);

      /* quiet submit */
      uint32_t start = ch[c].pb_off;
      for (int i = 0; i < pbn; i++) *(volatile uint32_t *)(ch[c].ring + PB_OFF + ch[c].pb_off + 4 * i) = pb[i];
      ch[c].pb_off += 4 * pbn; ch[c].pb_off = (ch[c].pb_off + 0xf) & ~0xfu; if (ch[c].pb_off > RING_BYTES - PB_OFF - 0x1000) ch[c].pb_off = 0;
      uint64_t va = ch[c].ringVA + PB_OFF + start;
      volatile uint32_t *gp = (volatile uint32_t *)(ch[c].ring + GP_OFF + 8 * (ch[c].gp_put % GP_ENTRIES));
      gp[0] = (uint32_t)va & ~3u; gp[1] = ((uint32_t)(va >> 32) & 0xff) | ((uint32_t)pbn << 10);
      ch[c].gp_put = (ch[c].gp_put + 1) % GP_ENTRIES;
      __sync_synchronize();
      *(volatile uint32_t *)(ch[c].ring + USERD_OFF + USERD_GP_PUT) = ch[c].gp_put;
      __sync_synchronize();
      *(volatile uint32_t *)(usermode + 0x90) = ch[c].token;
    }
    int done[MAXWORK] = {0}, nd = 0; double dl = t0i + 2.0, tl = t0i;
    while (nd < ncs && now() < dl) {
      for (int k = 0; k < ncs; k++) if (!done[k] && *semp(cs[k], slot[k]) == pay) { done[k] = 1; nd++; tch[k][it] = now() - t0i; tl = now(); }
    }
    if (nd < ncs) { say("RES", "bench: iteration %d TIMEOUT (%d of %d done); stopping", it, nd, ncs); alive = 0; iters = it; break; }
    tall[it] = tl - t0i;
  }
  if (iters > 2) {
    stats("all-done", tall, iters);
    for (int k = 0; k < ncs; k++) stats(ch[cs[k]].name, tch[k], iters);
  }
  return alive ? 0 : 1;
}

/* ------------------------------------------------------------------ the script */
static int run_line(char *line) {
  char *tok[24]; int n = 0;
  for (char *p = strtok(line, " \t\r\n"); p && n < 24; p = strtok(NULL, " \t\r\n")) tok[n++] = p;
  if (!n || tok[0][0] == '#') return 0;
  say("CMD", "%s", tok[0]);
  { char all[400]; all[0] = 0; for (int i = 0; i < n; i++) { strncat(all, tok[i], sizeof all - strlen(all) - 2); strcat(all, " "); } say("CMDLINE", "%s", all); }
  const char *c = tok[0]; uint32_t st = 0;
  if (strcmp(c, "tsg")) { const char *tk = kv(tok, n, "tsg"); if (tk) { int k = atoi(tk); if (k < 0 || k >= ntsg) { say("RES", "no such tsg %d", k); return 1; } curtsg = k; } }
  if (strcmp(c, "tsg") && strcmp(c, "vas") && curtsg < 0 && strcmp(c, "info") && strcmp(c, "settle") && strcmp(c, "bench")) { say("RES", "no tsg yet"); return 1; }

  if (!strcmp(c, "vas")) { int i = atoi(tok[1]); if (i < 1 || i >= MAXV) return 1; return make_vas(i); }

  if (!strcmp(c, "tsg")) {
    if (ntsg >= MAXT) return 1;
    const char *e = kv(tok, n, "engine"), *v = kv(tok, n, "vas");
    NV_CHANNEL_GROUP_ALLOCATION_PARAMETERS tp; memset(&tp, 0, sizeof tp);
    int eng = e ? engine_of(e) : NV2080_ENGINE_TYPE_GRAPHICS;
    uint32_t tv = v && strcmp(v, "none") ? vas[atoi(v)].hVAS : 0;
    tp.hVASpace = tv; tp.engineType = (uint32_t)eng;
    uint32_t h = rm_alloc("tsg", hDevice, KEPLER_CHANNEL_GROUP_A, &tp, sizeof tp, &st);
    say("RES", "tsg#%d engine=%s vas=%s status=0x%x", ntsg, e ? e : "gr", v ? v : "0", st);
    if (!h) return 1;
    tsgs[ntsg].h = h; tsgs[ntsg].vas = tv; tsgs[ntsg].engine = eng; g_tsg_engine = eng; curtsg = ntsg++;
    return 0;
  }

  if (!strcmp(c, "ctx")) {
    if (nctx >= MAXCTX) return 1;
    const char *fl = kv(tok, n, "flags"), *v = kv(tok, n, "vas");
    NV_CTXSHARE_ALLOCATION_PARAMETERS cp; memset(&cp, 0, sizeof cp);
    cp.hVASpace = v && strcmp(v, "none") ? vas[atoi(v)].hVAS : 0;
    if (!fl || !strcmp(fl, "sync")) cp.flags = NV_CTXSHARE_ALLOCATION_FLAGS_SUBCONTEXT_SYNC;
    else if (!strcmp(fl, "async")) cp.flags = NV_CTXSHARE_ALLOCATION_FLAGS_SUBCONTEXT_ASYNC;
    else if (!strcmp(fl, "prefer")) cp.flags = NV_CTXSHARE_ALLOCATION_FLAGS_SUBCONTEXT_ASYNC_PREFER_LOWER;
    else if (!strncmp(fl, "spec:", 5)) { cp.flags = NV_CTXSHARE_ALLOCATION_FLAGS_SUBCONTEXT_SPECIFIED; cp.subctxId = (uint32_t)atoi(fl + 5); }
    uint32_t h = rm_alloc("ctxshare", hTSG, FERMI_CONTEXT_SHARE_A, &cp, sizeof cp, &st);
    snprintf(ctx[nctx].name, sizeof ctx[nctx].name, "%s", tok[1]);
    ctx[nctx].tsgi = curtsg; ctx[nctx].h = h; ctx[nctx].flags = (int)cp.flags; ctx[nctx].subctxId = cp.subctxId;
    say("RES", "ctx %s flags=%s vas=%s status=0x%x h=0x%x returned_subctxId=0x%x", tok[1], fl ? fl : "sync", v ? v : "none", st, h, cp.subctxId);
    nctx++;
    return h ? 0 : 1;
  }

  if (!strcmp(c, "chan")) {
    if (nch >= MAXC) return 1;
    int i = nch;
    const char *cx = kv(tok, n, "ctx"), *e = kv(tok, n, "engine"), *v = kv(tok, n, "vas"), *rv = kv(tok, n, "ringvas");
    const char *objs = kv(tok, n, "objs"), *oa = kv(tok, n, "objafter");
    int ci = find_ctx(cx); if (ci == -2) { say("RES", "chan: unknown ctx %s", cx); return 1; }
    memset(&ch[i], 0, sizeof ch[i]);
    ch[i].tsgi = curtsg;
    snprintf(ch[i].name, sizeof ch[i].name, "%s", tok[1]);
    ch[i].ctxi = ci; ch[i].engine = e ? engine_of(e) : NV2080_ENGINE_TYPE_GRAPHICS;
    ch[i].ringvas = rv ? atoi(rv) : (v && strcmp(v, "none") ? atoi(v) : 0);
    uint32_t hv = vas[ch[i].ringvas].hVirt;
    ch[i].hRing = alloc_vidmem("ring", RING_BYTES, 0x10000, &st); if (!ch[i].hRing) return 1;
    ch[i].hNotif = alloc_vidmem("notifier", 0x10000, 0x10000, &st); if (!ch[i].hNotif) return 1;
    ch[i].ring = map_cpu("ring", ch[i].hRing, RING_BYTES, &st); if (!ch[i].ring) return 1;
    ch[i].notif = map_cpu("notifier", ch[i].hNotif, 0x10000, &st);
    memset((void *)ch[i].ring, 0, RING_BYTES); if (ch[i].notif) memset((void *)ch[i].notif, 0, 0x10000);
    NVOS46_PARAMETERS dm; memset(&dm, 0, sizeof dm);
    dm.hClient = g_hClient; dm.hDevice = hDevice; dm.hDma = hv; dm.hMemory = ch[i].hRing; dm.length = RING_BYTES;
    int rc = ioctl(g_ctl, _IOWR(NV_IOCTL_MAGIC, NV_ESC_RM_MAP_MEMORY_DMA_, NVOS46_PARAMETERS), &dm);
    say("RM", "map_dma ring(%s) vasidx=%d -> va=0x%llx status=0x%x rc=%d", tok[1], ch[i].ringvas, (unsigned long long)dm.dmaOffset, dm.status, rc);
    if (rc || dm.status) return 1;
    ch[i].ringVA = dm.dmaOffset;
    NV_CHANNEL_ALLOC_PARAMS cp; memset(&cp, 0, sizeof cp);
    cp.hObjectError = ch[i].hNotif; cp.gpFifoOffset = ch[i].ringVA + GP_OFF; cp.gpFifoEntries = GP_ENTRIES;
    cp.hUserdMemory[0] = ch[i].hRing; cp.userdOffset[0] = USERD_OFF; cp.engineType = (uint32_t)ch[i].engine;
    cp.hContextShare = ci >= 0 ? ctx[ci].h : 0;
    cp.hVASpace = v && strcmp(v, "none") ? vas[atoi(v)].hVAS : 0;
    { const char *th = kv(tok, n, "thread"), *dl = kv(tok, n, "delay"), *fl = kv(tok, n, "rawflags");
      if (th) cp.flags |= ((uint32_t)atoi(th) & 3u) << 28;      /* NVOS04_FLAGS_GROUP_CHANNEL_THREAD (legacy ctxshare pick) */
      if (dl && atoi(dl)) cp.flags |= 1u << 6;                  /* NVOS04_FLAGS_DELAY_CHANNEL_SCHEDULING */
      if (fl) cp.flags |= (uint32_t)strtoul(fl, NULL, 0); }
    ch[i].h = rm_alloc("channel", hTSG, AMPERE_CHANNEL_GPFIFO_A, &cp, sizeof cp, &st);
    say("RES", "chan %s ctx=%s engine=0x%x vas=%s status=0x%x h=0x%x", tok[1], cx ? cx : "none(0)", ch[i].engine, v ? v : "none", st, ch[i].h);
    if (!ch[i].h) return 1;
    ch[i].alive = 1; nch++;
    if (objs && !(oa && atoi(oa))) alloc_objs(i, objs);
    return 0;
  }

  if (!strcmp(c, "obj")) { int i = find_ch(tok[1]); if (i < 0) return 1; alloc_objs(i, tok[2]); return 0; }

  if (!strcmp(c, "token")) {
    int i = find_ch(tok[1]); if (i < 0) return 1;
    NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN_PARAMS tk; memset(&tk, 0, sizeof tk);
    st = rm_control("WORK_SUBMIT_TOKEN", ch[i].h, NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN, &tk, sizeof tk);
    if (!st) ch[i].token = tk.workSubmitToken;
    say("RES", "token %s status=0x%x token=0x%x", tok[1], st, ch[i].token);
    return st != 0;
  }

  if (!strcmp(c, "bindtsg")) {
    const char *e = kv(tok, n, "engine");
    NVA06F_CTRL_BIND_PARAMS bp; memset(&bp, 0, sizeof bp); bp.engineType = e ? (uint32_t)engine_of(e) : (uint32_t)g_tsg_engine;
    st = rm_control("BIND(tsg)", hTSG, NVA06C_CTRL_CMD_BIND, &bp, sizeof bp);
    say("RES", "bindtsg engine=0x%x status=0x%x", bp.engineType, st); return st != 0;
  }
  if (!strcmp(c, "bindchan")) {
    int i = find_ch(tok[1]); if (i < 0) return 1;
    const char *e = kv(tok, n, "engine");
    NVA06F_CTRL_BIND_PARAMS bp; memset(&bp, 0, sizeof bp); bp.engineType = e ? (uint32_t)engine_of(e) : (uint32_t)ch[i].engine;
    st = rm_control("BIND(chan)", ch[i].h, NVA06F_CTRL_CMD_BIND, &bp, sizeof bp);
    say("RES", "bindchan %s engine=0x%x status=0x%x", tok[1], bp.engineType, st); return st != 0;
  }
  if (!strcmp(c, "schedtsg")) {
    const char *en = kv(tok, n, "enable"), *ss = kv(tok, n, "skipsubmit");
    NVA06F_CTRL_GPFIFO_SCHEDULE_PARAMS sp; memset(&sp, 0, sizeof sp); sp.bEnable = en ? atoi(en) : 1; sp.bSkipSubmit = ss ? atoi(ss) : 0;
    st = rm_control("SCHEDULE(tsg)", hTSG, NVA06C_CTRL_CMD_GPFIFO_SCHEDULE, &sp, sizeof sp);
    say("RES", "schedtsg enable=%d status=0x%x", sp.bEnable, st); return st != 0;
  }
  if (!strcmp(c, "schedchan")) {
    int i = find_ch(tok[1]); if (i < 0) return 1;
    const char *en = kv(tok, n, "enable");
    NVA06F_CTRL_GPFIFO_SCHEDULE_PARAMS sp; memset(&sp, 0, sizeof sp); sp.bEnable = en ? atoi(en) : 1;
    st = rm_control("SCHEDULE(chan)", ch[i].h, NVA06F_CTRL_CMD_GPFIFO_SCHEDULE, &sp, sizeof sp);
    say("RES", "schedchan %s status=0x%x", tok[1], st); return st != 0;
  }
  if (!strcmp(c, "disable") || !strcmp(c, "enable")) {     /* NV2080_CTRL_CMD_FIFO_DISABLE_CHANNELS on the named channels */
    NV2080_CTRL_FIFO_DISABLE_CHANNELS_PARAMS dp; memset(&dp, 0, sizeof dp);
    const char *os = kv(tok, n, "only_sched");
    dp.bDisable = !strcmp(c, "disable"); dp.bOnlyDisableScheduling = os ? atoi(os) : 0;
    char list[200]; snprintf(list, sizeof list, "%s", tok[1]);
    for (char *p = strtok(list, ","); p && dp.numChannels < 64; p = strtok(NULL, ",")) {
      int i = find_ch(p); if (i < 0) continue;
      dp.hClientList[dp.numChannels] = g_hClient; dp.hChannelList[dp.numChannels] = ch[i].h; dp.numChannels++;
    }
    st = rm_control("DISABLE_CHANNELS", hSub, NV2080_CTRL_CMD_FIFO_DISABLE_CHANNELS, &dp, sizeof dp);
    say("RES", "%s %s only_sched=%d status=0x%x", c, tok[1], dp.bOnlyDisableScheduling, st); return st != 0;
  }
  if (!strcmp(c, "preempt")) {
    const char *w = kv(tok, n, "wait"), *to = kv(tok, n, "timeout_us");
    NVA06C_CTRL_PREEMPT_PARAMS pp; memset(&pp, 0, sizeof pp); pp.bWait = w ? atoi(w) : 1;
    if (to) { pp.bManualTimeout = 1; pp.timeoutUs = (uint32_t)atoi(to); }
    st = rm_control("PREEMPT(tsg)", hTSG, NVA06C_CTRL_CMD_PREEMPT, &pp, sizeof pp);
    say("RES", "preempt wait=%d status=0x%x", pp.bWait, st); return st != 0;
  }
  if (!strcmp(c, "timeslice")) {
    const char *us = kv(tok, n, "us");
    NVA06C_CTRL_TIMESLICE_PARAMS tp; memset(&tp, 0, sizeof tp); tp.timesliceUs = us ? strtoull(us, NULL, 0) : 1000;
    st = rm_control("SET_TIMESLICE(tsg)", hTSG, NVA06C_CTRL_CMD_SET_TIMESLICE, &tp, sizeof tp);
    say("RES", "timeslice us=%llu status=0x%x", (unsigned long long)tp.timesliceUs, st); return st != 0;
  }
  if (!strcmp(c, "work")) {
    int cs[MAXWORK], ncs = 0; char list[200]; snprintf(list, sizeof list, "%s", tok[1]);
    for (char *p = strtok(list, ","); p && ncs < MAXWORK; p = strtok(NULL, ",")) { int i = find_ch(p); if (i >= 0) cs[ncs++] = i; else say("RES", "work: unknown channel %s", p); }
    const char *objs = kv(tok, n, "objs"), *w = kv(tok, n, "wait");
    do_work(cs, ncs, objs ? parse_objs(objs) : -1, w ? atoi(w) : 3000);
    return 0;
  }
  if (!strcmp(c, "bench")) {
    int cs[MAXWORK], ncs = 0; char list[200]; snprintf(list, sizeof list, "%s", tok[1]);
    for (char *p = strtok(list, ","); p && ncs < MAXWORK; p = strtok(NULL, ",")) { int i = find_ch(p); if (i >= 0) cs[ncs++] = i; }
    const char *it = kv(tok, n, "iters");
    return do_bench(cs, ncs, it ? atoi(it) : 500);
  }
  if (!strcmp(c, "settle")) {
    usleep((useconds_t)atoi(tok[1]) * 1000);
    for (int i = 0; i < nch; i++) { char nb[160]; uint32_t bad = notif_bad(i, nb, sizeof nb); say("RES", "settle notifier %s error=%u %s", ch[i].name, bad, nb); }
    return 0;
  }
  if (!strcmp(c, "freech")) {
    int i = find_ch(tok[1]); if (i < 0) return 1;
    st = rm_free("channel", tsgs[ch[i].tsgi].h, ch[i].h); say("RES", "freech %s status=0x%x", tok[1], st); ch[i].alive = 0; return 0;
  }
  if (!strcmp(c, "freectx")) {
    int i = find_ctx(tok[1]); if (i < 0) return 1;
    st = rm_free("ctxshare", tsgs[ctx[i].tsgi].h, ctx[i].h); say("RES", "freectx %s status=0x%x", tok[1], st); return 0;
  }
  if (!strcmp(c, "freetsg")) { st = rm_free("tsg", hDevice, hTSG); say("RES", "freetsg status=0x%x", st); return 0; }
  if (!strcmp(c, "info")) {
    NV2080_CTRL_FIFO_GET_INFO_PARAMS ip; memset(&ip, 0, sizeof ip);
    ip.fifoInfoTblSize = 2; ip.fifoInfoTbl[0].index = NV2080_CTRL_FIFO_INFO_INDEX_MAX_SUBCONTEXT_PER_GROUP;
    ip.fifoInfoTbl[1].index = NV2080_CTRL_FIFO_INFO_INDEX_MAX_LOWER_SUBCONTEXT;
    st = rm_control("FIFO_GET_INFO", hSub, NV2080_CTRL_CMD_FIFO_GET_INFO, &ip, sizeof ip);
    say("RES", "info status=0x%x max_subctx_per_group=%u max_lower_subctx=%u", st, ip.fifoInfoTbl[0].data, ip.fifoInfoTbl[1].data);
    return 0;
  }
  say("RES", "unknown command %s", c);
  return 1;
}

int main(int argc, char **argv) {
  g_out = stdout; t0 = now();
  prctl(PR_SET_NAME, "tsgsub_probe");
  if (argc < 2) { fprintf(stderr, "usage: %s script-file\n", argv[0]); return 2; }
  FILE *f = fopen(argv[1], "r");
  if (!f) { perror("script"); return 2; }
  say("OBS", "START script=%s pid=%d comm=tsgsub_probe uid=%d", argv[1], (int)getpid(), (int)getuid());
  if (setup()) { say("OBS", "SETUP-FAIL (see RM lines above)"); return 3; }
  char line[512]; int lineno = 0, fails = 0;
  while (fgets(line, sizeof line, f)) {
    lineno++;
    char copy[512]; snprintf(copy, sizeof copy, "%s", line);
    int stop_on_fail = strstr(copy, "must") != NULL;
    int r = run_line(line);
    if (r) { fails++; say("RES", "line %d returned %d", lineno, r); if (stop_on_fail) { say("OBS", "ABORT: a `must` line failed"); break; } }
  }
  say("OBS", "END lines=%d fails=%d", lineno, fails);
  return 0;
}
