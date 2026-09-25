# Headless graphics in kayfabe v3: what it takes, and whether channels are sufficient

**STATUS: research only, 2026-09-26.** No code changed, no box used. Read against `kf-master`
`42ca9d71`, ogkm `580.159.04` (`research_clones/ogkm-580.159.04`) and `610.43.02`
(`research_clones/ogkm`), and the C tree at `8319499`.

**Legend.** **[E]** means established: read in source at the cited `file:line`, or measured in a
cited record. **[I]** means inferred: a reasoned conclusion that nothing has measured yet.
**[U]** means unknown: a boot is needed to settle it.

---

## 0. Short answer

**Rendering: yes, channels are enough.** A 3D channel is an ordinary user GR channel. v3 already
carries the 3D class onto the host twin under the guest's own class id, exactly as it does for
compute, and never parses the pushbuffer. What is missing is a **small control-plane layer** (§2):
about six channel-scoped GR controls, ZBC, one refused static-info control, and possibly the PTE
**kind** on host mappings.

**Display: no emulation is needed for headless.** DRM and NVKMS run inside the guest against the
guest's own RM. In 580, NVKMS KAPI has an explicit **"no display hardware → displayless mode"**
fallback, so `nvidia-drm` registers its render node even with `modeset=1` (§3).

**Video: NVENC/NVDEC are NOT free.** v3 deliberately advertises neither engine and names no
falcons. The guest therefore refuses every video class with `NV_ERR_INVALID_CLASS` before kayfabe
ever sees it. Enabling them is a new-engine job across four layers (§4).

**Xorg** with NVIDIA's own DDX (the Xorg display driver) and a virtual screen is the only item
that needs a real display object. That object is either `NVA083` (a guest-driver-matrix choice)
or an emulated display engine. The Xvfb or headless-Wayland plus EGL route gives the same user
outcome with no kayfabe code (§5).

---

## 1. What the graphics userspace needs, and which parts reach kayfabe at all

The key structural fact of Mode 2: **the guest kernel owns `/dev/dri/*` and
`/dev/nvidia-modeset`.** Mode 1 had to forward the DRM render node and NVKMS across the VM
boundary, and its hardest wall was an fd crossing inside `NVKMS_IOCTL_REGISTER_SURFACE`
(memory `vulkan_progress_modeset.md`). In Mode 2 that whole surface is **guest-internal**. What
reaches kayfabe is only what the guest's CPU-RM sends to "GSP", plus doorbells.

### 1.1 Objects the graphics UMDs allocate, and whether the alloc reaches us

Mode 1 found this list empirically, by host-vs-guest ioctl diffs. The flags below come from
`ogkm-580 src/nvidia/src/kernel/rmapi/resource_list.h`.

| class | what it is | alloc RPC to GSP (= us)? | v3 today |
|---|---|---|---|
| `AMPERE_B 0xc797` / `ADA_A 0xc997` / … (3D) | 3D engine object on a GR channel | **yes** (channel descendant, same path as compute) `[E] :2009-2012` | ✔ carried to twin with guest class `[E] kf-rm/src/chanlink.rs:490-507`, `kf-chan/src/passthrough.rs:131`; per-family sets generated `[E] kf-chip/src/classes.rs:115-151` |
| `GF100_DISP_SW 0x9072` | "display SW" object on a channel (flip/semaphore helper; Mode 1 needed it for `vkCreateDevice`) | **yes**, `RPC_TO_ALL` `[E] :1506-1510` | allowlisted opaque `[E] kf-abi/src/capability.rs:1096`; answered as a graph node `[I]` |
| `GF100_ZBC_CLEAR 0x9096` | zero-bandwidth-clear table handle | **yes**, `RPC_TO_ALL` `[E] :824-828` | allowlisted opaque `[E] capability.rs:1101` |
| `NV_SEMAPHORE_SURFACE 0xda` | Vulkan timeline / sync_fd semaphores | no, CPU-RM-local `[E] :148-151` | n/a |
| `NV_MEMORY_MAPPER 0xfe` | Vulkan sparse binding | no; CPU-RM writes the guest PTEs `[E] mem_mapper.c` | n/a (lands as a walker diff) |
| `NV01_CONTEXT_DMA 0x2`, `NV01_EVENT 0x5` | ctx-DMA (NVENC), events | no `[E] :2170-2173` | n/a |
| `NVC7B7_VIDEO_ENCODER` (NVENC) | encoder engine object | **yes** (channel descendant) `[E] :1935-1939` | ⊘ **refused in the guest before it reaches us**, §4 |
| `NVENC_SW_SESSION 0xa0bc` | encoder telemetry | no `[E] :719-723` | n/a |
| `NV04_DISPLAY_COMMON` | display root | would be `RPC_TO_ALL`, **but the class is removed** | ✔ display engine amputated `[E] kf-rm/src/sweep.rs:692-702` |

### 1.2 RM controls: the graphics-specific ones that are `ROUTE_TO_PHYSICAL`

These were enumerated from the NVOC export tables (`ogkm-580 src/nvidia/generated/g_subdevice_nvoc.c`,
`g_zbc_api_nvoc.c`, flag bit `0x40`). **[E]** that they reach us if issued. **[U]** which of them
the UMDs actually issue; that needs an nvdiff run.

| cmd | name | what it needs from us | v3 today |
|---|---|---|---|
| `0x20801210` | `GR_SET_CTXSW_PREEMPTION_MODE` | apply to the twin (the GFXP flag is new for graphics) | ✔ decoded `[E] kf-rm/src/chanlink.rs:226-238` |
| `0x20801211` | `GR_CTXSW_PREEMPTION_BIND` | **UMD-supplied GFXP buffer VAs** → bind on the twin (VA-identical) | ⊘ absent (no match in `kf-*`) |
| `0x20801205` / `0x20801208` | `GR_CTXSW_ZCULL_MODE` / `_ZCULL_BIND` | zcull buffer VA → bind on the twin | ⊘ absent (`0x20801208` allowlisted only) |
| `0x2080123a` | `GR_CTXSW_SETUP_BIND` | ctx buffer bind → twin | ⊘ absent |
| `0x2080120a` / `0x20801236` | `GR_SET/GET_GPC_TILE_MAP` | tile map (probably kernel-only) | ⊘ absent |
| `0x9096010x` | ZBC `SET_*_CLEAR`, `GET_*_TABLE*` | a **host-global** table, §2.4 | allowlisted only |
| `0x20800a2c` | `INTERNAL_STATIC_KGR_GET_ZCULL_INFO` | the guest caches it at boot | ⊘ **deliberately refused** `[E] kf-rm/src/sweep.rs:607-617` |

★ **The ZCULL refusal has a graphics-visible consequence.** The client control
`GR_GET_ZCULL_INFO 0x20801206` is served **locally** by the guest's CPU-RM (flags `0x10109`, no
route bit). It asserts that the cached `pZcullInfo != NULL` and otherwise returns
`NV_ERR_NOT_SUPPORTED` `[E] ogkm-580 kernel_graphics.c:3862-3863`. The refusal was correct for
compute boot, but **a GL/Vulkan UMD that queries zcull geometry now gets 0x56** `[E]`. What the UMD
does with that is `[U]`. The fix is cheap: serve it from the host's non-privileged
`GR_GET_ZCULL_INFO`. `hostquery.rs:418-464` already does the same for `ZCULL_MASK`.

`GR_GET_INFO`'s `GFX_CAPABILITIES` is already served from real GA106 data (`15`)
`[E] kf-abi/src/grinfo.rs:351`, and `physGfxGpcMask` is served (`grfsinfo.rs:157`). So KernelGraphics
comes up graphics-capable.

---

## 2. Is a 3D channel just another user GR channel?

**Yes for the transport. [E]** `AMPERE_B` is a sibling of compute on `ENG_GR(0)`
(`kayfabe-chips/src/ga10x.rs:167`). It rides the same GPFIFO/USERD/doorbell passthrough twin.
`engine_object()` accepts `Compute | ThreeD` on a GR twin, and host RM builds the graphics golden
context itself (`kf-chan/src/passthrough.rs:100-133`). The owner's ruling that `GPU_PROMOTE_CTX`
is a stub is what makes this sound: *"the host twin already holds the real context"*
(`owner_rulings_2026_09_25.md` #3). ⇒ **golden context, ctxsw, bundles/pagepool/attribute/RTV
circular buffers, and FECS/GPCCS are all the host RM's problem, not ours.** The Mode-1 C proved
the same thing from the other side: the host RM executed real graphics at 1.00x (vkpeak,
EGL offscreen; `tests/perf/realapp_matrix.md:62-69`).

What differs from compute, in order of risk:

### 2.1 PTE kind and compression. The one data-plane item. [E] facts, [U] consequence

- The guest's real page tables are full of kinds and comptaglines. In a real GA106 cold-boot
  corpus, **6 986 of 7 008 leaf PTEs set KIND and 6 017 set COMPTAGLINE**
  `[E] cuda/walk/kf_tests.cu:2139-2140`.
- The GPU walker **reads** kind and makes it part of run identity
  `[E] cuda/walk/kf_walk.cu:155-157, 254-256`. The host side then **drops** it:
  - `host_key()` places runs by aperture only; *"kind, read-only, volatile, page size is not
    part of what the host places"* `[E] crates/kf-cuda/src/diffmodel.rs:48-58`.
  - The one NVOS46 builder sends `kind_override: 0`, with no `PAGE_KIND_OVERRIDE` flag
    `[E] crates/kf-host/src/lib.rs:771-816`.
  - The store is allocated `kind: 0` (PITCH) `[E] kf-host/src/lib.rs:1324-1349`.
- ⇒ **every host-twin PTE is PITCH, uncompressed, whatever the guest asked for.** This is
  self-consistent, because every engine and every BAR1 view sees the same host PTE. So
  **compression is silently off (bandwidth, not correctness) [I]**, and ZBC can never be
  referenced by hardware [I].
- ⚠ The risk is **[U] depth/stencil and block-linear surfaces on Turing+**. The kind list is
  PITCH `0x00`, GENERIC `0x06`, Z16 `0x01`, S8 `0x02`, S8Z24 `0x03`, ZF32_X24S8 `0x04`, Z24S8
  `0x05`, plus compressible variants `[E] ogkm swref/published/turing/tu102/dev_mmu.h:97-112`.
  `nvidia-drm` hard-codes *generic kind `0x06` ⇒ page-kind generation 2* for Turing+ and exports it
  to userspace (`nvidia-drm-drv.c:754-757`). If the ZROP or texture units interpret Z/generic
  kinds differently from PITCH, depth-tested rendering will be wrong while compute stays green.
- **The fix is small if needed [E, mechanism].** RM supports a per-map
  `NVOS46_FLAGS_PAGE_KIND_OVERRIDE_YES` + `kindOverride`, validated by `FB_IS_KIND_SUPPORTED`
  (`ogkm-580 src/nvidia/src/kernel/mem_mgr/virtual_mem.c:1311-1328`). The change is to carry the
  run's kind into `raw_map_dma_slice` (uncompressed variant only), which splits runs on kind. The
  walker already splits.
- Real compression (comptag backing / CBC for the 11.9 GiB store) is a **separate, later**
  project: allocating the store with compression attributes and honouring `FB_CBC_OP`, which
  is privileged and route-to-physical (`0x20801337`, flags `0x44`). ⊘ Not needed for
  correctness.

### 2.2 ZBC table: author it, never forward it [I]

The ZBC controls are `NON_PRIVILEGED | ROUTE_TO_PHYSICAL | ROUTE_TO_VGPU_HOST` (`0x10248`)
`[E] g_zbc_api_nvoc.c:180-182`. So they reach us. The table in hardware is **GPU-global**:
forwarding it puts guest entries in a table shared with every host process, which is a
cross-tenant resource with finite slots. Because §2.1 makes all host mappings uncompressed,
**hardware never consults ZBC for guest surfaces**. ⇒ Serve a **per-VM local table**: accept
`SET_*`, answer `GET_*` consistently, and never touch the host. This is the same "author, never
forward" posture as `author_host_flags_never_forward_them.md`. ⚠ If real compression lands later,
this decision re-opens.

### 2.3 Channel-scoped binds need twin translation (§1.2) [E] absent, [U] exercised

`ZCULL_BIND`, `PREEMPTION_BIND` and `CTXSW_SETUP_BIND` name a **guest channel handle plus buffer
VAs** in that channel's VAS. The twin shares VA identity, so this is a handle rewrite plus a
host call, the same shape as the w827 `CtxswPreemption` / `Debugger` twin verbs
(`kf-qemu/src/chan.rs:1386-1420`). There are about three new `ChanStatement`s. Refusing them
probably leaves zcull and GFXP off. Whether that is merely slower or fatal to the UMD is **[U]**.

### 2.4 TSG and subcontext fidelity [E] shape, [U] impact

The host twin is **one TSG per guest channel**. `birth_channel` allocates
TSG → channel → BIND (`kf-host/src/channel.rs:~183-275`). `FERMI_CONTEXT_SHARE_A` is decoded
for its VAS only (`kf-rm/src/chanlink.rs:485-489`), and no host subcontext/VEID is replicated.
CUDA passes with this shape. A graphics device that puts a 3D channel and async-compute
channels in **one** TSG on different subcontexts would get **separate host GR contexts**. The
likely result is timeslicing instead of concurrency, and duplicated context memory [I].
Correctness should hold because each channel's pushbuffer sets its own state and cross-queue
sync is by semaphore [I]. The risk is UMD assumptions about a shared TSG context [U].

### 2.5 Bigger context buffers and VA collisions [I]

Graphics contexts add the GFX global buffers and, with GFXP, preemption buffers of roughly tens
of MB per context. Host RM places them at RM-chosen VAs in the twin's VAS. v3 already names a
collision `0x51` at reconcile rather than failing silently (`passthrough.rs:103-104`). The
GSP split reserves a 512 MiB server-RM window per VAS (`ogkm-580 gpu_vaspace.c:422-431`), which
likely keeps these clear of guest mappings [I]. Graphics raises the volume, not the class.

### 2.6 Completion and interrupts [E] design, same as compute

GL/Vulkan fences use semaphore surfaces and OS events woken by the **per-engine non-stall
interrupt** (`THE_ARCHITECTURE_v3.md` §5, `:1083-1116`). A 3D channel is on the GR engine, so the
GR non-stall vector that compute already depends on covers it. Nothing new is needed unless
§4's video engines come in: each of those has its own vector
(`kf-abi/src/eventnotify.rs:184-186`: NVENC `0x01`, NVDEC `0x03`, OFA `0x09`).

### 2.7 BAR1 budget [I]

On a non-ReBAR GA106, BAR1 is 256 MiB, shared with the host
(`vidmem_cpu_reads_are_48_mibs_and_bar1_bounds_views.md`). Vulkan's DEVICE_LOCAL|HOST_VISIBLE
heap and GL persistent maps draw from it. This is a capacity knob, not a blocker.

### 2.8 Pushbuffer contents are never parsed ✔

This is the owner rule *"parsing, not placement, is forbidden"*. The 3D method stream, including
the MME macros that `kernel_gr_channels_and_the_mme_exposure.md` worried about, is passthrough.
It is decoded by nobody. ⇒ **no graphics method knowledge enters kayfabe.** The kernel-side 2D
channel (the RC watchdog's `FERMI_TWOD_A`) is gated only on MIG and 2D-class support, not on
display (`ogkm-580 kernel_rc_watchdog.c:~470-474`). It is already refused by name.

---

## 3. The display plane: headless needs no display engine

### 3.1 What the stock 580 guest does with a displayless GPU. [E] throughout

1. The kernel display engine returns `NOT_SUPPORTED` in `StatePreInit`, and RM **removes the
   display classes from the class DB**: *"On Displayless GPU's, Display Engine is not
   present"* (`ogkm-580 src/nvidia/src/kernel/gpu/gpu.c:2177-2181`). v3 does this on purpose
   (`kf-rm/src/sweep.rs:692-702`, `ENG_KERNEL_DISPLAY` amputation).
2. NVKMS `AllocDevice` sees no `NV04_DISPLAY_COMMON` and returns
   `NVKMS_ALLOC_DEVICE_STATUS_NO_HARDWARE_AVAILABLE`. The comment calls this *"expected in some
   configurations"* (`ogkm-580 src/nvidia-modeset/src/nvkms-rm.c:1697-1721`).
3. ★★★ **New in this audit: NVKMS KAPI, the `nvidia-drm` side, TREATS THAT AS SUCCESS.**
   *"Display hardware is not available; falling back to displayless mode"*, `ret = TRUE`
   (`ogkm-580 src/nvidia-modeset/kapi/src/nvkms-kapi.c:462-471`). The device exists with
   `hKmsDevice == 0`. Every modeset entry point checks for that
   (`:979, 995, 1023, …, 4051`). `GetDeviceResourcesInfo` still fills `genericPageKind`,
   `hasVideoMemory` and the semaphore-surface layout, and returns before any head query
   (`:1136-1176`).
4. So `nvidia-drm modeset=1`: `nv_drm_dev_load` succeeds, and the DRM device (`card0` +
   `renderD128`) registers with **zero connectors**, `supports_alloc=true`, and semsurf/sync_fd
   support (`kernel-open/nvidia-drm/nvidia-drm-drv.c:669-760, 1039-1061, 2019-2029`).
5. `nvidia-drm modeset=0` (the **source default**, `nvidia-drm-os-interface.c:41`) skips NVKMS
   entirely. The render node still registers, but `GET_DEV_INFO` reports `supports_alloc=false`
   and **no semsurf/sync_fd** (`nvidia-drm-drv.c:1043-1061`), and `DMABUF_SUPPORTED` fails
   (`:1074-1082`). ⇒ **`modeset=1` is the better headless config**, because it gives
   Vulkan sync_fd / external-memory interop. Ubuntu's packaging sets it by default [I].

⚠ **This corrects the 2026-08-12 scoping doc.** `display_plane_scoping.md` §5.1 describes a
three-way NVKMS branch that includes `NVA083`. That is **610** (`research_clones/ogkm`). In
**580** there is no `NVA083` branch in NVKMS at all: grep for `NVA083|displayless` in
`ogkm-580.159.04/src/nvidia-modeset` hits only the KAPI fallback. The KAPI fallback is the
piece that matters for a render node, and the scoping doc did not see it.

### 3.2 The remaining display unknown, and it is one bit [U]

Does the **userspace** path survive `NO_HARDWARE`? That is the Vulkan ICD / EGL doing its own
`NVKMS_IOCTL_ALLOC_DEVICE` on `/dev/nvidia-modeset` (the Mode-1 C needed NVKMS forwarding
*specifically* for `vkCreateDevice`, commit `a895f95`, **on a host that had a display**).
The evidence for YES:
- the KAPI fallback above, which is NVIDIA's own statement that render-without-display is a
  supported configuration;
- datacenter parts with no display engine (A100/H100/T4 class) run headless Vulkan/EGL
  [I, external, not measured in either tree].

This is `display_plane_scoping.md` §8.4. It is still the single cheapest high-value boot:
`vulkaninfo --summary` on a v3 guest with `nvidia-drm modeset=1`.

### 3.3 When a display object *is* needed

Only for a **real KMS head in the guest**: NVIDIA's Xorg DDX driving a virtual screen, or a
DRM-backend Wayland compositor. The options are unchanged from `display_plane_scoping.md`:
- **`NVA083_GRID_DISPLAYLESS`**: gated on `osIsGridSupported()` → `NV_GRID_BUILD`, a guest-driver
  **build flag** (`gpu.c:1958-1984`, `os-interface.c:1445-1458`, per that doc). In 580 NVKMS
  has no `NVA083` HAL at all (above). ⇒ it means **610+ plus the vGPU/GRID guest package**, a
  support-matrix decision.
- **Emulating EVO/NVDisplay** core/window channels: 3–6 months, dominated.
- ⚠ The Mode-1 C measured that **DRM-backend compositors hang forever** in
  `libnvidia-egl-gbm`'s scanout path even when a KMS head exists. Headless-backend compositors
  work (`headless_compositor_unlock.md`). So a KMS head buys less than it appears to.

---

## 4. Video engines: same passthrough plane, but not reachable today

**The transport is the same. [I]** An NVENC/NVDEC channel is a GPFIFO channel on its own runlist,
with an engine object and a falcon context that **host RM/GSP** owns, including its firmware. That
is the same twin pattern, with no method parsing.

**But four v3 layers exclude it on purpose. [E]**

1. **Engine inventory.** `GA106_ENGINES` serves only `GR0, CE0..CE3` and omits
   *"NVDEC0, NVENC0, OFA, SEC2-class … an engine we advertise is an engine RM goes on to use"*
   (`kf-abi/src/deviceinfo.rs:92-99`).
2. **Falcon inventory.** `GET_CONSTRUCTED_FALCON_INFO` names **zero** falcons. It explicitly
   predicts that *"a later guest allocating a SEC2/NVDEC/NVENC/NVJPG/OFA engine-class channel
   object gets `NV_ERR_INVALID_CLASS`"* (`kf-abi/src/falconinfo.rs:84-93`). Confirmed in the guest's
   source: non-GR, non-CE classes need `kflcnGetKernelFalconForEngine` to be non-NULL, else
   *"engine is missing for class"* (`ogkm-580 src/nvidia/src/kernel/gpu/fifo/channel_descendant.c:216-233`).
   ⚠ Naming a falcon makes it an `IntrService` whose registers are read straight from BAR0
   (`falconinfo.rs:36-56`, e.g. MSENC at `0x1c8000`). So each video falcon needs a **quiescent
   BAR0 register model** (`IRQSTAT/IRQMASK/HWCFG…`).
3. **Twin birth.** `birth_twin` refuses anything but a copy engine or GR0 by name
   (`kf-chan/src/passthrough.rs:83-85`). `engine_object` has no video kind, and `kf-chip`'s
   generated `Kind` has no NVENC/NVDEC/NVJPG/OFA set (`kf-chip/src/classes.rs:19-31`).
4. **Interrupts.** Per-engine non-stall vectors exist in the ABI tables, but only GR/CE are armed [I].

The rest is guest-local and needs nothing. `NVENC_SW_SESSION`, `NV01_CONTEXT_DMA` and
`GPU_GET_ENCODER_CAPACITY` (flags `0x8`) are all CPU-RM-served [E]. NVENC's falcon context
buffer goes through `GPU_PROMOTE_CTX` from `kernel_falcon.c:273`, which is already a status-only
stub [E] (`THE_SURFACE_v3.md` §2.4).

⚠ **This is an all-families item** (`all_families_are_first_class.md`). Engine counts and classes
vary per die: GA106 has 1×NVENC and 1×NVDEC, and Ada adds AV1 and more instances. So it should
be generated like `classes.rs`, not hand-rowed.

---

## 5. Effort and order

The estimates assume the current bench cadence (one agent lane, vast boxes) and that §3.2
comes back YES.

| step | outcome | kayfabe changes | estimate | confidence |
|---|---|---|---|---|
| **0** | the one-bit boot (§3.2) plus an nvdiff host-vs-guest of `vulkaninfo` / `vkcube --headless` / an EGL offscreen program | none. Stage the gfx libs in the guest; extend nvdiff to follow the NVKMS `address` payload and `/dev/dri` (`display_plane_scoping.md` §6.1) | **1–2 days** | high |
| **(a)** | `vulkaninfo`, Vulkan compute (vkpeak), offscreen Vulkan render bit-compared to bare metal | serve `INTERNAL_KGR_GET_ZCULL_INFO` from the host; twin verbs for `ZCULL_BIND/MODE`, `PREEMPTION_BIND` (GFXP), `CTXSW_SETUP_BIND`; local ZBC table; confirm `GF100_DISP_SW` / `ZBC_CLEAR` graph-node answers; **carry the PTE kind** if the depth test fails; add graphics arms to the grader | **1–2 weeks** | medium. Vulkan *compute* is probably days, since it is the CUDA path; *render* carries the kind/zcull unknowns |
| **(b)** | EGL-device headless GL (glmark2 offscreen, egl_offscreen) | none beyond (a) [I]. Same RM surface, same 3D class | **+2–4 days** | medium-high |
| **(d1)** | "headless Xorg" as users mean it: Xvfb/Xorg-dummy + VirtualGL (EGL back end), or headless weston/sway with GL | **none** | **+1–3 days** after (b) | high. Mode 1 ran exactly this |
| **(c)** | NVENC H.264/HEVC | engine rows (device-info, engine lists) + falcon inventory + quiescent falcon BAR0 model + twin birth on a video runlist + video engine-object kind + non-stall vector, all generated per family | **2–3 weeks**, NVDEC **+~1 week** | low-medium. Falcon-register modelling is the unknown |
| **(d2)** | NVIDIA Xorg DDX / KMS head | `NVA083`: support the 610+ vGPU guest driver and emit the GSP event plus 3–6 data controls | **1.5–4 weeks ±100 %** plus a licensing/product decision | low. Emulating EVO instead is 3–6 months |

⇒ **Owner's "display ~1 week" (2026-09-26 roadmap) fits (0)+(a-compute)+(b)+(d1).** It does
not fit render-with-depth if the kind issue bites, NVENC, or an NVIDIA-DDX Xorg.

**Order:** 0 → (a) Vulkan compute → (a) render → (b) → (d1) → (c) → (d2) only if a product need
appears. Step 0 comes first because it can collapse (a)'s control list to only the calls the UMD
actually makes.

### 5.1 What must change in v3, one list

- `kf-rm`: serve `0x20800a2c` ZCULL_INFO from the host, which moves it out of `AmputationIntended`.
  Add a ZBC local table (`0x90960101-07`). Add `ChanStatement`s for the three GR binds.
- `kf-host`/`kf-cuda`: kind into `host_key`/NVOS46 `kindOverride` (uncompressed variant only),
  **only if** the depth-compare fails.
- `kf-chip`/`kf-chan`/`kf-abi` (video only): a `Kind::{Nvenc,Nvdec,Nvjpg,Ofa}` generated set;
  device-info and falcon rows; twin birth on video runlists; per-engine non-stall.
- Grader: graphics arms. There are none today; the 30-arm suite is CUDA-only [E, grep of
  `tests/`, `crates/kf-harness`].
- **Not needed:** any display emulation, any DRM/NVKMS forwarding, any pushbuffer decoding, any
  golden-context work.

### 5.2 What the Mode-1 C already proved, and what does not transfer

- **Proved [E]**: NVIDIA's closed graphics userspace runs to host parity when its RM calls reach a
  real host RM. vkpeak 1.00x, EGL offscreen 1.00x, NVENC 720p 0.96x, headless weston GL plus
  zero-copy capture (`realapp_matrix.md:62-69`, `nvenc_101_root_cause_wc_input.md`,
  `headless_compositor_unlock.md`). It also produced the object list in §1.1.
- **Does not transfer**: all of its marshalling fixes (class param sizes, NVOS32 truncation,
  NVKMS/DRM forwarding, REGISTER_SURFACE fd). In Mode 2 the guest kernel issues those itself. And
  Mode 1 never faced §2.1 (kinds), §2.2 (ZBC authorship) or §4 (engine inventory), because the
  host RM saw the guest's real allocations.
- The Mode-2 C never attempted graphics [E]. Its only display work is the fuse lie plus
  `numDispChannels=128` (`display_plane_scoping.md` §3.1).

### 5.3 Unknowns, ranked by how much they move the estimate

1. **[U] Userspace survives NVKMS `NO_HARDWARE`** (§3.2). If NO, then `NVA083` becomes mandatory
   for all graphics, which makes it weeks plus a driver-matrix decision. One boot settles it.
2. **[U] Is PITCH-kind-everywhere correct for depth/stencil and block-linear on Turing+?** (§2.1).
   A depth-tested render compare settles it. The fix is small.
3. **[U] Which ROUTE_TO_PHYSICAL GR controls the UMDs issue**, and whether refusing zcull or GFXP
   is fatal or merely slower (§1.2, §2.3). nvdiff settles it.
4. **[U] TSG/subcontext split** under a real Vulkan device (§2.4).
5. **[U] Video falcon BAR0 model**: which registers the guest reads once a falcon is named (§4).
