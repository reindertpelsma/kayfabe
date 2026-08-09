/*
 * vasfmt_probe.c — read the GUEST RM's ROOT MMU FORMAT out of a STOCK, UNPATCHED driver.
 *
 * ## The question this exists to answer, and why it is worth its own instrument
 *
 * Boot `s28_933a709_spd` stopped on
 *
 *     NVRM: Assertion failed: vaLimitNew <= pGVAS->vaLimitMax @ gpu_vaspace.c:3094
 *
 * and `UVM_REGISTER_GPU rmStatus = 0x1f` (`NV_ERR_INVALID_ARGUMENT`, the value :3094
 * returns).  Transcribed from `ogkm-580.159.04`, that assert is:
 *
 *   gpu_vaspace.c:3091  vaLimitNew = mmuFmtEntryIndexVirtAddrHi(pGpuState->pFmt->pRoot, 0,
 *                                                               pParams->numEntries - 1);
 *   gpu_vaspace.c:3093  NV_ASSERT_OR_RETURN(vaLimitNew >= pGVAS->vaLimitInternal, …);
 *   gpu_vaspace.c:3094  NV_ASSERT_OR_RETURN(vaLimitNew <= pGVAS->vaLimitMax,      …);
 *
 * with (mmu_fmt.h:178-193, :124-134)
 *
 *   mmuFmtEntryIndexVirtAddrHi(L,0,i) = (i << L->virtAddrBitLo) + (NVBIT64(L->virtAddrBitLo) - 1)
 *
 * and (gpu_vaspace.c:1121)  vaLimitMax = NVBIT64(pFmt->pRoot->virtAddrBitHi + 1) - 1.
 *
 * Substituting i = numEntries-1 the whole thing collapses to
 *
 *   ★ numEntries <= 2^(virtAddrBitHi - virtAddrBitLo + 1)   ==   mmuFmtLevelEntryCount(pRoot)
 *
 * i.e. "the published root may not have more entries than the root level HAS".  Nothing
 * else enters it — not vaStart, not the requested vaLimit (that clamps `vasLimit`, never
 * `vaLimitMax`), not anything this port puts on the wire.  It is a pure function of the
 * guest's own compiled-in root format and the `numEntries` UVM chose.
 *
 * ⊘ So the ONE number that decides this bug had never been measured.  `virtAddrBitLo` and
 * `virtAddrBitHi` appear in no log any boot has produced; "4 entries at shift 47 covers
 * 2^49" was this port's BELIEF about GA106, not a reading of the guest.
 *
 * ## Why this is a userspace probe and NOT a guest driver patch
 *
 * A guest patch is an instrument, and an instrument that changes the subject has to be
 * fenced out again before any claim about stock-driver behaviour.  It also costs a driver
 * rebuild per iteration.  RM already exports both numbers to unprivileged userspace:
 *
 *   NV0080_CTRL_CMD_DMA_ADV_SCHED_GET_VA_CAPS (0x801806)
 *       gpu_vaspace.c:2367   pParams->vaBitCount = pFmt->pRoot->virtAddrBitHi + 1;
 *     — a Device control, served entirely in CPU-RM (dma.c:734 -> vaspaceGetVasInfo), no
 *       RPC to GSP, so it answers even on a GPU whose GSP path is refusing things.
 *
 *   NV90F1_CTRL_CMD_VASPACE_GET_PAGE_LEVEL_INFO (0x90f10102)
 *       gpu_vaspace.c:4003   portMemCopy(&levels[level].levelFmt, …, pLevelFmt, sizeof(MMU_FMT_LEVEL))
 *     — hands back the whole `MMU_FMT_LEVEL` per level, i.e. virtAddrBitLo AND
 *       virtAddrBitHi AND entrySize AND numSubLevels, verbatim.
 *
 * ★ The root level is a per-chip compiled-in constant shared by every VAS on the GPU
 * (`kgmmuFmtGet` returns the same `pFmtFamily->pFmts[b]` object, gpu_vaspace.c:1089), and
 * only levels[4] (the big page table) varies with big page size.  So the root geometry read
 * from THIS client's VAS is the same root geometry :3094 used on UVM's VAS.  That is an
 * inference, and it is labelled as one in the output.
 *
 * ## What the answer means — decided in advance, so the reading cannot be fitted after
 *
 *   vaBitCount == 49  -> GMMU_FMT_VERSION_2, kgmmuFmtInitLevels_GP10X: root PD3 = [48:47],
 *                        4 entries.  numEntries=4 gives vaLimitNew == vaLimitMax EXACTLY
 *                        and :3094 CANNOT fire.  ⇒ the observed assert would then refute
 *                        the transcription above, and only a driver patch can go further.
 *   vaBitCount == 57  -> GMMU_FMT_VERSION_3, kgmmuFmtInitLevels_GH10X: root PD4 = [56:56],
 *                        2 entries.  numEntries=4 > 2 ⇒ :3094 fires, and :3093 passes.
 *                        ⇒ the guest's KernelGmmu resolved a HOPPER-class HAL on a GA106.
 *   vaBitCount == 1   -> the level array is ZEROED: kgmmuFmtInitLevels_GP10X early-returned
 *                        (it asserts version/numLevels/bigPageShift and returns VOID) and
 *                        left the portMemSet(0) levels in place.  vaLimitMax = 1,
 *                        vaLimitNew = 3, so :3093 passes and :3094 fires — also consistent.
 *   anything else     -> unmodelled; report it rather than rounding it to one of the above.
 *
 * ⊘ An empty or absent reply is evidence of NOTHING.  Every step below prints its own
 * status; a failure is NAMED, never silently skipped, because a probe that prints a
 * plausible zero is worse than one that prints nothing.
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <unistd.h>

/* ---- the /dev/nvidiactl ioctl ABI (ogkm-580.159.04: nvos.h, nv-ioctl-numbers.h) ---- */
#define NV_IOCTL_MAGIC 'F'
#define NV_ESC_RM_FREE 0x29
#define NV_ESC_RM_CONTROL 0x2a
#define NV_ESC_RM_ALLOC 0x2b

/* NVOS21_PARAMETERS — nvos.h:461-471 (32 bytes) */
typedef struct {
  uint32_t hRoot;
  uint32_t hObjectParent;
  uint32_t hObjectNew;
  int32_t hClass;
  uint64_t pAllocParms;
  uint32_t paramsSize;
  int32_t status;
} nvos21_t;

/* NVOS54_PARAMETERS — nvos.h:2229-2238 (32 bytes) */
typedef struct {
  uint32_t hClient;
  uint32_t hObject;
  int32_t cmd;
  uint32_t flags;
  uint64_t params;
  uint32_t paramsSize;
  int32_t status;
} nvos54_t;

/* NVOS00_PARAMETERS — nvos.h (16 bytes) */
typedef struct {
  uint32_t hRoot;
  uint32_t hObjectParent;
  uint32_t hObjectOld;
  int32_t status;
} nvos00_t;

/* NV0080_ALLOC_PARAMETERS — cl0080.h:54-64 */
typedef struct {
  uint32_t deviceId;
  uint32_t hClientShare;
  uint32_t hTargetClient;
  uint32_t hTargetDevice;
  int32_t flags;
  uint32_t _pad0;
  uint64_t vaSpaceSize;
  uint64_t vaStartInternal;
  uint64_t vaLimitInternal;
  int32_t vaMode;
  uint32_t _pad1;
} nv0080_alloc_t;

/* NV2080_ALLOC_PARAMETERS — cl2080.h:43-45 */
typedef struct {
  uint32_t subDeviceId;
} nv2080_alloc_t;

/* NV0000_CTRL_GPU_GET_ATTACHED_IDS_PARAMS — ctrl0000gpu.h:68-70 */
typedef struct {
  uint32_t gpuIds[32];
} attached_ids_t;

/* NV0000_CTRL_GPU_GET_ID_INFO_PARAMS — ctrl0000gpu.h:83-93 */
typedef struct {
  uint32_t gpuId;
  uint32_t gpuFlags;
  uint32_t deviceInstance;
  uint32_t subDeviceInstance;
  uint64_t szName;
  uint32_t sliStatus;
  uint32_t boardId;
  uint32_t gpuInstance;
  int32_t numaId;
  uint32_t _pad0;
} id_info_t;

/* NV_VASPACE_ALLOCATION_PARAMETERS — nvos.h:3154-3164 */
typedef struct {
  uint32_t index;
  int32_t flags;
  uint64_t vaSize;
  uint64_t vaStartInternal;
  uint64_t vaLimitInternal;
  uint32_t bigPageSize;
  uint32_t _pad0;
  uint64_t vaBase;
  uint32_t pasid;
  uint32_t _pad1;
} nv90f1_alloc_t;

/* NV0080_CTRL_DMA_ADV_SCHED_GET_VA_CAPS_PARAMS — ctrl0080dma.h:346-360 */
typedef struct {
  uint32_t pageTableSize;
  uint32_t pageTableCoverage;
} pt_fmt_t;
typedef struct {
  uint32_t vaBitCount;
  uint32_t pdeCoverageBitCount;
  uint32_t num4KPageTableFormats;
  uint32_t bigPageSize;
  uint32_t compressionPageSize;
  uint32_t dualPageTableSupported;
  uint32_t idealVRAMPageSize;
  pt_fmt_t pageTableBigFormat;
  pt_fmt_t pageTable4KFormat[16];
  uint32_t hVASpace;
  uint32_t _pad0;
  uint64_t vaRangeLo;
  uint32_t vaSpaceId;
  uint32_t _pad1;
  uint64_t supportedPageSizeMask;
} vacaps_t;

/* MMU_FMT_LEVEL — mmu_fmt_types.h:72-116 (24 bytes: 5 u8, pad, u32 tag, pad, ptr) */
typedef struct {
  uint8_t virtAddrBitLo;
  uint8_t virtAddrBitHi;
  uint8_t entrySize;
  uint8_t bPageTable;
  uint8_t numSubLevels;
  uint8_t _pad0[3];
  uint32_t pageLevelIdTag;
  uint32_t _pad1;
  uint64_t subLevels;
} mmu_fmt_level_t;

/* NV_CTRL_VASPACE_PAGE_LEVEL — ctrl90f1.h:87-120 */
typedef struct {
  uint64_t pFmt;
  mmu_fmt_level_t levelFmt;
  mmu_fmt_level_t sublevelFmt[2];
  uint64_t physAddress;
  uint32_t aperture;
  uint32_t _pad0;
  uint64_t size;
  uint32_t entryIndex;
  uint32_t _pad1;
} vaspace_page_level_t;

/* NV90F1_CTRL_VASPACE_GET_PAGE_LEVEL_INFO_PARAMS — ctrl90f1.h:122-158 */
typedef struct {
  uint32_t hSubDevice;
  uint32_t subDeviceId;
  uint64_t virtAddress;
  uint64_t pageSize;
  uint64_t flags;
  uint32_t numLevels;
  uint32_t _pad0;
  vaspace_page_level_t levels[6];
} pageinfo_t;

static int g_fd = -1;
static int g_fail = 0;

static void hexdump(const char *tag, const void *p, size_t n) {
  const unsigned char *b = p;
  printf("  %s (%zu bytes):", tag, n);
  for (size_t i = 0; i < n; i++) {
    if (i % 32 == 0) printf("\n    +%03zx:", i);
    printf(" %02x", b[i]);
  }
  printf("\n");
}

/* Returns the new handle, or 0 with the failure NAMED. */
static uint32_t rm_alloc(const char *what, uint32_t hRoot, uint32_t hParent,
                         uint32_t hClass, void *params, uint32_t psize) {
  nvos21_t a;
  memset(&a, 0, sizeof a);
  a.hRoot = hRoot;
  a.hObjectParent = hParent;
  a.hObjectNew = 0;
  a.hClass = (int32_t)hClass;
  a.pAllocParms = (uint64_t)(uintptr_t)params;
  a.paramsSize = psize;
  int rc = ioctl(g_fd, _IOWR(NV_IOCTL_MAGIC, NV_ESC_RM_ALLOC, nvos21_t), &a);
  printf("ALLOC %-10s hClass=0x%04x hRoot=0x%08x hParent=0x%08x -> hObject=0x%08x "
         "status=0x%08x rc=%d errno=%d\n",
         what, hClass, hRoot, hParent, a.hObjectNew, a.status, rc, rc ? errno : 0);
  if (rc != 0 || a.status != 0) {
    printf("  ★ FAILED to allocate %s — every later reading in this run is void.\n", what);
    g_fail = 1;
    return 0;
  }
  return a.hObjectNew;
}

static int rm_control(const char *what, uint32_t hClient, uint32_t hObject,
                      uint32_t cmd, void *params, uint32_t psize) {
  nvos54_t c;
  memset(&c, 0, sizeof c);
  c.hClient = hClient;
  c.hObject = hObject;
  c.cmd = (int32_t)cmd;
  c.params = (uint64_t)(uintptr_t)params;
  c.paramsSize = psize;
  int rc = ioctl(g_fd, _IOWR(NV_IOCTL_MAGIC, NV_ESC_RM_CONTROL, nvos54_t), &c);
  printf("CTRL  %-10s cmd=0x%08x hClient=0x%08x hObject=0x%08x size=%u status=0x%08x "
         "rc=%d errno=%d\n",
         what, cmd, hClient, hObject, psize, c.status, rc, rc ? errno : 0);
  return (rc == 0 && c.status == 0) ? 0 : -1;
}

int main(void) {
  printf("=== vasfmt_probe: the guest RM's ROOT MMU FORMAT, from a STOCK driver ===\n");
  printf("=== (see the header of this file for the transcribed :3094 arithmetic) ===\n");

  g_fd = open("/dev/nvidiactl", O_RDWR);
  if (g_fd < 0) {
    printf("★ FAILED: open(/dev/nvidiactl): %s\n", strerror(errno));
    printf("  ⊘ NOT a statement about the format. The observer never ran.\n");
    return 2;
  }

  uint32_t hClient = rm_alloc("client", 0, 0, 0x0 /*NV01_ROOT*/, NULL, 0);
  if (!hClient) return 3;

  /* ⊘ ASK RM which device instance the GPU IS. Do not assume 0.
   *
   * `[measured 2026-08-09, boot s29_933a709_vafmt]` this probe's first version passed
   * `deviceId = 0` and RM answered `NV_ERR_INVALID_STATE` (0x40) from
   * `device.c:366` — `gpumgrGetGpu(gpumgrGetPrimaryForDevice(0))` returned NULL, i.e.
   * device instance 0 is not this GPU.  libcuda never assumes it either: `0x00000201`
   * and `0x00000202` are both in the s28 RM trace, in that order, before its device
   * alloc.  ★ The failure was in the INSTRUMENT, and it was caught only because every
   * step names its own status instead of pressing on with a plausible handle. */
  uint32_t deviceInst = 0, subDeviceInst = 0, gpuId = 0xffffffffu;
  attached_ids_t aids;
  memset(&aids, 0, sizeof aids);
  for (int i = 0; i < 32; i++) aids.gpuIds[i] = 0xffffffffu;
  if (rm_control("ATTACHED_IDS", hClient, hClient, 0x0201, &aids, sizeof aids) == 0) {
    for (int i = 0; i < 32; i++)
      if (aids.gpuIds[i] != 0xffffffffu) {
        gpuId = aids.gpuIds[i];
        break;
      }
    printf("★ ATTACHED_IDS: first gpuId=0x%08x\n", gpuId);
  }
  if (gpuId == 0xffffffffu) {
    printf("★ FAILED: no attached gpuId. ⊘ Nothing below is a reading.\n");
    return 3;
  }
  id_info_t ii;
  memset(&ii, 0, sizeof ii);
  ii.gpuId = gpuId;
  if (rm_control("ID_INFO", hClient, hClient, 0x0202, &ii, sizeof ii) != 0) {
    printf("★ FAILED: GET_ID_INFO. ⊘ Nothing below is a reading.\n");
    return 3;
  }
  deviceInst = ii.deviceInstance;
  subDeviceInst = ii.subDeviceInstance;
  printf("★ ID_INFO: deviceInstance=%u subDeviceInstance=%u gpuInstance=%u flags=0x%x\n",
         deviceInst, subDeviceInst, ii.gpuInstance, ii.gpuFlags);

  nv0080_alloc_t dp;
  memset(&dp, 0, sizeof dp);
  dp.deviceId = deviceInst;
  uint32_t hDevice = rm_alloc("device", hClient, hClient, 0x0080, &dp, sizeof dp);
  if (!hDevice) return 3;

  nv2080_alloc_t sp;
  memset(&sp, 0, sizeof sp);
  sp.subDeviceId = subDeviceInst;
  uint32_t hSubDev = rm_alloc("subdevice", hClient, hDevice, 0x2080, &sp, sizeof sp);
  /* a missing subdevice is survivable: GET_VA_CAPS is a Device control. */

  /* ---- READING 1: vaBitCount == pFmt->pRoot->virtAddrBitHi + 1 ------------------- */
  vacaps_t vc;
  memset(&vc, 0, sizeof vc);
  vc.hVASpace = 0; /* the Device's implicit VAS */
  int ok1 = rm_control("VA_CAPS", hClient, hDevice, 0x801806, &vc, sizeof vc);
  if (ok1 == 0) {
    printf("★ VA_CAPS: vaBitCount=%u  => root->virtAddrBitHi = %d\n", vc.vaBitCount,
           (int)vc.vaBitCount - 1);
    printf("★ VA_CAPS: pdeCoverageBitCount=%u bigPageSize=0x%x compressionPageSize=0x%x\n",
           vc.pdeCoverageBitCount, vc.bigPageSize, vc.compressionPageSize);
    printf("★ VA_CAPS: dualPageTable=%u num4KFmts=%u bigPT{size=%u coverage=0x%x} "
           "vaRangeLo=0x%llx pageSizeMask=0x%llx\n",
           vc.dualPageTableSupported, vc.num4KPageTableFormats,
           vc.pageTableBigFormat.pageTableSize, vc.pageTableBigFormat.pageTableCoverage,
           (unsigned long long)vc.vaRangeLo,
           (unsigned long long)vc.supportedPageSizeMask);
    switch (vc.vaBitCount) {
      case 49:
        printf("★ VERDICT-A: vaBitCount=49 => GMMU_FMT_VERSION_2 (GP10X root [48:47], "
               "4 entries). numEntries=4 makes vaLimitNew == vaLimitMax EXACTLY, so "
               ":3094 CANNOT fire from this format. The transcription is then refuted "
               "or numEntries is not 4 on the failing call.\n");
        break;
      case 57:
        printf("★ VERDICT-B: vaBitCount=57 => GMMU_FMT_VERSION_3 (GH10X root [56:56], "
               "2 entries). numEntries=4 > 2 ⇒ :3094 fires and :3093 passes. The guest "
               "resolved a HOPPER-class KernelGmmu HAL on a GA106.\n");
        break;
      case 1:
        printf("★ VERDICT-C: vaBitCount=1 => the root level is ZEROED; "
               "kgmmuFmtInitLevels early-returned and left portMemSet(0) behind.\n");
        break;
      default:
        printf("★ VERDICT-D: vaBitCount=%u is UNMODELLED. Report it; do not round it to "
               "one of the three above.\n",
               vc.vaBitCount);
    }
  } else {
    printf("  ⊘ VA_CAPS did not answer. That is a fact about the control, NOT about the "
           "format; nothing below may be read as 'the format is unknown because it is "
           "empty'.\n");
    g_fail = 1;
  }

  /* ---- READING 2: the same question asked of a UVM-SHAPED VAS -------------------
   *
   * ⚠ Reading 1 asked the DEVICE's implicit VAS.  The VAS that :3094 asserted on is the
   * one UVM builds, and it is not shaped the same way.  `nvGpuOpsAddressSpaceCreate`
   * (nv_gpu_ops.c:2538-2543) allocates FERMI_VASPACE_A with
   *
   *     index  = NV_VASPACE_ALLOCATION_INDEX_GPU_NEW (0)
   *     vaBase = gpu->parent->rm_va_base   = 0            (uvm_ampere.c:49)
   *     vaSize = gpu->parent->rm_va_size   = 128 TiB      (uvm_ampere.c:50)
   *     flags  = NV_VASPACE_ALLOCATION_FLAGS_SHARED_MANAGEMENT (BIT(2)) when vaSize != 0
   *
   * ⊘ Asking only the default VAS and reporting the answer as "the format" would be a
   * correct capture of the wrong subject.  So both are asked, and any disagreement
   * between them is itself the finding.
   */
  struct {
    const char *name;
    uint64_t vaSize;
    int32_t flags;
  } shapes[2] = {
      {"vas-plain", 0, 0},
      {"vas-uvm", 128ull << 40, 4 /* SHARED_MANAGEMENT */},
  };

  for (int k = 0; k < 2; k++) {
    nv90f1_alloc_t vp;
    memset(&vp, 0, sizeof vp);
    vp.index = 0; /* NV_VASPACE_ALLOCATION_INDEX_GPU_NEW — nvos.h:3184 */
    vp.vaBase = 0;
    vp.vaSize = shapes[k].vaSize;
    vp.flags = shapes[k].flags;
    uint32_t hVas = rm_alloc(shapes[k].name, hClient, hDevice, 0x90f1, &vp, sizeof vp);
    if (!hVas) {
      printf("  ⊘ %s could not be allocated, so nothing below is a statement about it.\n",
             shapes[k].name);
      g_fail = 0; /* a shape we could not build is not a failed reading */
      continue;
    }

    vacaps_t v2;
    memset(&v2, 0, sizeof v2);
    v2.hVASpace = hVas;
    if (rm_control("VA_CAPS/vas", hClient, hDevice, 0x801806, &v2, sizeof v2) == 0)
      printf("★ %s VA_CAPS: vaBitCount=%u (=> root virtAddrBitHi=%d) bigPageSize=0x%x "
             "pdeCoverageBitCount=%u vaRangeLo=0x%llx\n",
             shapes[k].name, v2.vaBitCount, (int)v2.vaBitCount - 1, v2.bigPageSize,
             v2.pdeCoverageBitCount, (unsigned long long)v2.vaRangeLo);

    for (int i = 0; i < 2; i++) {
      uint64_t psz = i == 0 ? 0x1000ull : 0x10000ull;
      pageinfo_t pi;
      memset(&pi, 0, sizeof pi);
      pi.hSubDevice = hSubDev;
      pi.subDeviceId = hSubDev ? 0 : 1;
      pi.virtAddress = 0;
      pi.pageSize = psz;
      char tag[24];
      snprintf(tag, sizeof tag, "PGLVL%d/%llx", k, (unsigned long long)psz);
      if (rm_control(tag, hClient, hVas, 0x90f10102, &pi, sizeof pi) != 0) continue;
      printf("★ %s PAGE_LEVEL_INFO(pageSize=0x%llx): numLevels=%u\n", shapes[k].name,
             (unsigned long long)psz, pi.numLevels);
      for (unsigned l = 0; l < pi.numLevels && l < 6; l++) {
        mmu_fmt_level_t *f = &pi.levels[l].levelFmt;
        unsigned nent = (f->virtAddrBitHi >= f->virtAddrBitLo)
                            ? (1u << (f->virtAddrBitHi - f->virtAddrBitLo + 1))
                            : 0;
        printf("    level[%u] virtAddrBit[%u:%u] entrySize=%u numSubLevels=%u "
               "bPageTable=%u entryCount=%u aperture=%u phys=0x%llx size=0x%llx\n",
               l, f->virtAddrBitHi, f->virtAddrBitLo, f->entrySize, f->numSubLevels,
               f->bPageTable, nent, pi.levels[l].aperture,
               (unsigned long long)pi.levels[l].physAddress,
               (unsigned long long)pi.levels[l].size);
        for (unsigned s = 0; s < f->numSubLevels && s < 2; s++) {
          mmu_fmt_level_t *g = &pi.levels[l].sublevelFmt[s];
          printf("      sub[%u]  virtAddrBit[%u:%u] entrySize=%u\n", s, g->virtAddrBitHi,
                 g->virtAddrBitLo, g->entrySize);
        }
      }
      if (pi.numLevels == 0)
        printf("    ⊘ numLevels=0: the walker had no memdesc for the root, so this reply "
               "carries NO geometry. It is UNMEASURED, not empty.\n");
      else
        hexdump("PAGE_LEVEL_INFO raw head", &pi, 192);
    }
  }

  printf("=== vasfmt_probe done (g_fail=%d) ===\n", g_fail);
  return g_fail ? 1 : 0;
}
