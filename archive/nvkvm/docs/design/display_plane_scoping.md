# The graphics / display plane — scoping it as a v1.0 schedule risk

> ### STATUS — 2026-08-12 / **LIVE — ANSWERED, as a SIZING, not as a design.**
> Read-only audit. No code changed, nothing built, no bench session consumed.
> **Question asked:** the owner's v1.0 scope names *"whole f\* graphics and display plane mode 1 C
> had"* as one line item. It was carried as one of only **two genuine unknowns** (the other being
> the `cuCtxCreate` wall, owned elsewhere). ⇒ **It is not an unknown of the same kind.** It is a
> *scoping decision with a known cost curve*, and the deciding fact is a **guest-driver build
> flag**, not a piece of hardware we have to reverse-engineer.
>
> ⚠ **This doc supersedes the framing of `virtual_modeset.md` (2026-05-31), not its content.**
> That doc's engineering is sound and still describes what shipped; its **TL;DR is contradicted by
> its own source tree** (§2.1). It has no STATUS block and reads as current design; it is
> **Mode-1 history + a Mode-2 prediction that was never tested**.
>
> ⊘ **Not a build plan.** §8 names one experiment. The owner schedules.

---

## 0. ★★★ THE ANSWER, WITH ERROR BARS

Display is **not one number**, because it is **not one problem**. Its cost is dominated by a
posture choice that has never been made explicitly. Three postures, measured floors:

| posture | what the guest gets | cost | error bar | confidence |
|---|---|---|---|---|
| **A — displayless** (status quo) | render + compute only; no `card0` modeset | **1–3 days** | ±2 d | **high** — already shipped in the C as *two lies* (§3.1) |
| **B — virtual display** (`NVA083`) | real `card0`, virtual head, default EDID, any Wayland desktop | **1.5–4 weeks** | **±100 %** | **low** — gated on a **build flag**, §5 |
| **C — emulate the GA10x display engine** | EVO core/window/cursor channels, real scanout | **3–6 months** | ±100 % | n/a — ⊘ **nothing recommends this**, §4.3 |

★★★ **The estimate that matters is that A and B are separated by a decision, not by a discovery.**
Posture C — the one that looks like the scary unknown, and the one the brief's framing implies — is
**dominated**: NVIDIA already shipped the virtualised-display path we would be re-deriving, it is
**open source in a tree we already vendor**, and it allocates **zero display channels** (§4).

> ### ★ THE SINGLE MEASUREMENT THAT MOST REDUCES THESE BARS
> **Does Vulkan/EGL device creation survive `NVKMS_IOCTL_ALLOC_DEVICE` returning
> `NVKMS_ALLOC_DEVICE_STATUS_NO_HARDWARE_AVAILABLE`?**
>
> Two strong pieces of evidence **disagree**, and exactly one bench session settles it (§8):
> - **Says NO:** the Mode-1 C added NVKMS forwarding *specifically* to unblock `vkCreateDevice`
>   (commit `a895f95`, 2026-05-30) — MEASURED.
> - **Says YES:** NVIDIA datacenter parts run Vulkan with no display, and the driver's own comment
>   calls a missing display class *"expected in some configurations"*
>   (`nvkms-rm.c:1855-1858`) — MEASURED (source), INFERRED (that it covers our case).
>
> **If YES** → graphics is fully separable from display; **posture A ships v1.0 graphics**, and the
> display line item collapses to a *streaming* feature with no VMM display code at all. Total
> display cost ≈ **days**.
> **If NO** → *every* graphics app, including headless offscreen Vulkan, is gated behind a display
> object. Posture B becomes **mandatory**, its build-flag problem (§5) becomes a **v1.0 blocker**,
> and this line item is a **multi-week** risk that must be scheduled beside the `cuCtxCreate` wall.
>
> ⚠ This single bit is worth more than everything else in this document.

---

## 1. ★★★ WHAT CONTRADICTS THE BRIEF I WAS GIVEN

Leading with these, per standing practice. Four of the brief's premises are refuted by measurement.

**1.1 ⊘ "NVKMS is closed, so there is no source ground truth."** — **REFUTED, MEASURED.**
The claim lives in memory `display_arch_nvkms_gap.md` (2026-06): *"NVKMS is effectively CLOSED even
in open-gpu-kernel-modules: the shim links a precompiled blob (`nv-modeset-kernel.o_binary`) … So no
source ground-truth like RM/nvproxy gave us."*
Measured 2026-08-12 in both vendored trees:

```
research_clones/ogkm/src/nvidia-modeset/src/          45 .c files   4.2 MB
research_clones/ogkm-580.159.04/src/nvidia-modeset/src/  46 .c files   5.0 MB
find research_clones -name '*.o_binary'            → (no results)
NVKMS core, ogkm 610.43.02                         → 80 545 lines of C
```
`research_clones/ogkm` = **610.43.02**; `ogkm-580.159.04` = **580.159.04** (`version.mk`).
⇒ **NVKMS is fully open in both.** The stated reason display looked unownable is dead.

★ **But be precise about what this changes** — the brief asked exactly this, and the answer is
*reference, not reduction*: 80 kLOC of readable modesetting logic tells us **what the guest driver
will demand of us and why**. It does **not** shrink v1.0 by one line, and it is **versioned, not the
spec** (memory `ogkm_is_versioned.md`) — 580 and 610 are two data points, and the closed driver must
also work. Its value is that it converts *"we would have to guess display semantics"* into
*"we can read them"* — which is precisely how §4's finding was obtained, and that finding is worth
months.

**1.2 ⊘ "Nobody has touched display in Mode 2."** — **REFUTED, MEASURED.**
Commit `d49bd89` (2026-06-03) is a Mode-2 display posture, and it is still live on `master`
(`src/qemu/nvkvm_gpu_emul.c:1536`). §3.1.

**1.3 ⊘ "Under the two-axis split, graphics is passthrough, so display may be far smaller."**
— **The conclusion is right; the reason is wrong, and the wrong reason is dangerous.**
Display channels *are* pushbuffer channels of exactly the same shape as GR/CE — `NVC57D_CORE_
CHANNEL_DMA`, `NVC57E_WINDOW_CHANNEL_DMA`, `NVC57A_CURSOR` — so the passthrough *mechanism* applies
formally. It fails on the **resource**, not the mechanism: the display engine is **host-global,
singular hardware** that drives the host's real monitors, with no per-context VMMU/IOMMU containment.
Reason **F** of `is_passthrough_the_only_correct_route.md` (*"delineation by privilege — a page guest
userspace can write to cannot carry privileged content"*) is exactly what **does not hold** for
display: NVKMS gates modeset behind ownership/`CAP_SYS_ADMIN` precisely because those pages *are*
privileged. ⇒ **Display is the one plane where passthrough is ruled out on the security axis rather
than the decodability axis.** Getting this right matters: *"graphics is passthrough"* would otherwise
license forwarding a display channel, which is the single worst thing in this design space.
★ And the answer is still *"smaller than it looks"* — for the reason in §4, not this one.

**1.4 ⊘ "The differential oracle explicitly does not cover this plane" — true, but it UNDERSTATES
the problem by one whole failure mode.** §6.2: the oracle is not merely *blind* on display, it is
**positively wrong** there, and the tree's own guard **does not catch it**. This is new.

---

## 2. WHAT THE MODE-1 C ACTUALLY HAD — IN SOURCE

### 2.1 ★★ The doc and the source disagree, and the disagreement is the finding

`docs/design/virtual_modeset.md` TL;DR (2026-05-31), verbatim:

> *"We do **not** forward `/dev/nvidia-modeset` (NVKMS) to the host."*
> *"3. **NVKMS forwarding** … **Rejected**."*

The source on `master`, today: NVKMS **is** forwarded, behind
`src/qemu/nvkvm_nvkms_allowlist.h` (45 lines), which admits **six** inner `cmdType`s:

| cmdType | name | display programming? |
|---|---|---|
| 0 | `NVKMS_IOCTL_ALLOC_DEVICE` | no |
| 1 | `NVKMS_IOCTL_FREE_DEVICE` | no |
| 17 | `NVKMS_IOCTL_REGISTER_SURFACE` | no |
| 18 | `NVKMS_IOCTL_UNREGISTER_SURFACE` | no |
| 61, 62 | query-class (captured, *not yet identified against `nvkms-api.h`* — the file says so itself) | no |

⊘ **Both statements are true, and that is why the doc misleads.** The doc means *"we never let the
guest program the host display engine"* — which **holds**. Its wording says *"we never forward
NVKMS"* — which **does not**. The distinction is load-bearing: what is forwarded is *device
enumeration and surface naming for Vulkan/EGL*, and it is the reason `vkCreateDevice` works at all
(`a895f95`).
⇒ ★ **Fold this correction into `virtual_modeset.md` above its TL;DR, not beside it** (doc-hygiene
rule 2). Not done here: this audit writes exactly one file.

### 2.2 ★★★ MEASURED: the C's cross-boundary display surface is EMPTY

Not small — **empty**. `src/qemu/nvkvm_drm_allowlist.h` annotates every forwarded ioctl, in the
source, with why it is not display:

> *"DISPLAY/MODESET/permissions surfaces (NVKMS import/alloc, CRTC CRC, grant/revoke permissions,
> connector/dpy id) are deliberately excluded — a render node should never drive them, and they are
> privileged on the host."*
> on `GEM_ALLOC_NVKMS_MEMORY`: *"Despite the 'NVKMS' name it is `DRM_RENDER_ALLOW` and does NO
> display programming … SCANOUT is only a memory-layout capability, not a CRTC attachment."*

⇒ **Every pixel of Mode-1 "display" was produced without one host display call.** That is the
architectural fact worth carrying forward, and it is **date-durable**: it rests on the host boundary
(shared display hardware, multi-tenant), which the C→Rust rewrite does not change. ★ The 2026-05-31
ruling survives its own date check.

### 2.3 MEASURED: the size and the calendar

| component | file | LOC | what it is |
|---|---|---|---|
| virtual KMS head | `src/guest/nvkvm_kms.c` | **313** | one connector/CRTC/plane, fake EDID, sw vblank, ADDFB2, PAGE_FLIP |
| DRM/GEM proxy | `src/guest/nvkvm_drm.c` | 887 | shared with the render path, not display-specific |
| host present + EGL import | `src/qemu/nvkvm_present_egl.{c,h}` | **569** | dma-buf → host EGL → QEMU window |
| allowlists | `nvkvm_drm_allowlist.h`, `nvkvm_nvkms_allowlist.h` | 137 | the boundary |

**≈ 1 900 LOC display-specific.** Calendar span of the whole graphics+display effort:
`680b18b` (2026-05-30, first graphics forward) → `d49bd89` (2026-06-03, Mode-2 display-off) =
**5 calendar days**, ~55 commits — on top of an already-working Mode-1 forwarding stack.

★ **The 313-line virtual head is the entire "display plane" people mean.** It is *small* because it
does nothing: it is a sink that accepts a flip and hands the buffer on.

### 2.4 MEASURED: what actually worked, and what never did

**Worked** (`realapp_matrix.md` 2026-06-01, memory `headless_compositor_unlock.md` 2026-06-02):
headless sway + wlroots, **1920×1080 zero-copy capture at ~60 fps** (output-refresh-bound; glmark2
rendered at **795 fps** through nvkvm — ~13× headroom); host-side EGL import pixel-exact
(`acb8e68`); cross-isolate dma-buf brokering; NVENC H.264/HEVC at 720p **parity** (`0.96×`).

**Never worked** — and it is **intrinsic, not a bug**: every DRM-backend compositor
(weston/mutter/sway) **hangs forever** in `libnvidia-egl-gbm`'s `gbm_surface`→scanout path, because
NVIDIA's closed userspace EGL couples presentation to *real NVKMS flip completion*. Diagnosed by gdb
backtrace on the live guest. `gbmflip` (no `gbm_surface`, no EGL present) flips fine.
⇒ ★★ **"The display plane Mode 1 C had" never included a DRM-backend compositor on NVIDIA.** It
included a **headless GPU compositor plus a capture/present bridge**. Anyone scoping "what Mode 1
had" from the docs will over-scope this by a lot.

⚠ **A minor stale-doc instance, flagged not chased:** `virtual_modeset.md:23` cites headless EGL at
*"RTX 3060 @ 632 FPS, 2048², 800-iter shader"*; `realapp_matrix.md` reports EGL offscreen at
**1.8 Mtri/s** and **632** appears there as *ResNet-50 inference img/s*. Reads like a transcription
collision. Do not cite the 632 figure for graphics.

---

## 3. WHAT MODE 2 HAS TODAY

### 3.1 ★★ MEASURED: the Mode-2 display plane is TWO LIES, and the C's own comment overstates one

`src/qemu/nvkvm_gpu_emul.c:1533-1536` (commit `d49bd89`, 2026-06-03):

```c
/* Report display fused-off => compute-only displayless GPU.  The driver's
 * gpuFuseSupportsDisplay_HAL gives NV_ERR_NOT_SUPPORTED in display
 * StatePreInit, skipping all display engine init (inst-mem/heads/channels). */
case NV_FUSE_STATUS_OPT_DISPLAY: return NVKVM_FUSE_OPT_DISPLAY_DISABLED;
```

⊘ **"skipping all display engine init" is contradicted by the same commit, 1 866 lines later**
(`:3400-3413`): the driver still reaches `kdispStateInitLocked`, still issues
`NV2080_CTRL_CMD_INTERNAL_DISPLAY_GET_STATIC_INFO` (`0x20800a01`), and still needs
`numDispChannels` — the capture answered **0**, producing
`portMemAllocNonPaged(0) = NULL` → *"Could not allocate clientChannelTable"*. The fix synthesises
**128**, annotated `(generate-to-satisfy)`.
⇒ ★ **Display-off is not display-absent.** The guest driver walks part of `kdisp` regardless. Both
lies landed in one commit, i.e. the author discovered this within the same session — the comment was
simply never corrected. **INFERRED**, from the commit's own contents.

**MEASURED, from the real-hardware control census** (`../nvkvm-rs/docs/reference/
gsp_control_classification.tsv`, 106 rows): **12 of 106 (11.3 %)** of the controls a real GA106
bring-up demands are display-family — 7 × `NV0073_*`, 5 × `NV2080_*_DISPLAY_*`/EDID/BRIGHTC. They are
**bring-up**, not operation. So display is ~1/9th of a control surface we are already paying for.

### 3.2 MEASURED: none of it is ported

- **Rust**: `kayfabe-vmm` has `trait Present` + `struct Vblank`, whose doc comment already says
  **"NEVER NVKMS forwarding"** (the 2026-05-31 ruling, carried across the rewrite); `kayfabe-fwd::
  present_scanout` exists. **Zero production implementations.** The abstract seam landed at
  `c5489a1` and has had no consumer since.
- **C**: `nvkvm_present_egl.c` is wired **only** into the Mode-1 device (`virtio_nvgpu.c:1292`).
  `nvkvm_gpu_emul.c` — the Mode-2 device — has **no present path at all**.
  ⇒ ⊘ `virtual_modeset.md`'s *"Mode-agnostic — reused verbatim by Mode 2"* is a **2026-05-31
  prediction that was never tested**, and it is **half wrong by construction** (§4.4).

---

## 4. ★★★★★ THE FINDING — NVIDIA ALREADY SHIPPED OUR DISPLAY PLANE

This is the reason posture C is dominated, and it is the highest-value item in this audit.
**All MEASURED, from `research_clones/ogkm` (610.43.02) source.**

### 4.1 `NVA083_GRID_DISPLAYLESS` — a first-class virtual-display HAL

NVKMS carries a **complete alternate display HAL for hardware with no display engine**, built for
NVIDIA's own vGPU/GRID product:

- **`src/nvidia-modeset/src/nvkms-displayless.c` — 885 lines**, the whole thing.
- Selected in `src/nvidia-modeset/src/nvkms-hal.c:160-173` by class `NVA083_GRID_DISPLAYLESS`
  (`0x0000a083`, `src/common/sdk/nvidia/inc/class/cla083.h`), sitting in the same HAL table as
  Ampere/Ada/Turing — and its entry reads:

  ```c
  { .class = NVA083_GRID_DISPLAYLESS,
    .pEvoHal = &nvDisplayless,
    .coreChannelDma = { },          /* ← EMPTY */
    .evoCaps = { .maxWidthInPixels = 0, .maxHeight = 0, ... } },
  ```

★★★ **`.coreChannelDma = { }` is the whole argument.** The displayless HAL allocates **no EVO
channels** — no core (`NVC57D`), no window (`NVC57E`), no cursor (`NVC57A`), no pushbuffer, no
scanout, no pixel clock, no EDID probe, no output resources. `nvkms-rm.c` branches around every
physical path on `pDevEvo->displaylessHw` (`:437`, `:650`, and throughout).

### 4.2 Its entire interface is SIX controls, all pure data

`src/common/sdk/nvidia/inc/ctrl/ctrla083.h` — the complete class:

| control | params |
|---|---|
| `0xa0830101` `GET_NUM_HEADS` | `numHeads`, `maxNumHeads` |
| `0xa0830102` `GET_MAX_RESOLUTION` | `headIndex` [in]; `maxHResolution`, `maxVResolution` |
| `0xa0830103` `GET_DEFAULT_EDID` | `pEdidBuffer`, `edidSize`, `connectorType` |
| `0xa0830104` `IS_ACTIVE` | `isDisplayActive` |
| `0xa0830105` `IS_CONNECTED` | `isDisplayConnected` |
| `0xa0830106` `GET_MAX_PIXELS` | (max pixels) |

NVKMS 610 uses **three** of them (`GET_NUM_HEADS`, `GET_MAX_RESOLUTION`, `GET_DEFAULT_EDID`).
Every one is **scalars plus an EDID blob**: no addresses, no memdescs, no device acts — the safest
possible category under `mode2_forwarding_model.md`.

And the values are served **entirely inside the guest's own CPU-RM**
(`src/nvidia/src/kernel/griddisplayless/griddisplaylessctrl.c`) out of an `OBJGRIDDISPLAYLESS`
populated by a **GSP RPC event** — `NV_VGPU_MSG_EVENT_UPDATE_GRID_DISPLAYLESS_PARAMS`
(`src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:1571`).
★★ **We are the GSP.** The parameters arrive over a transport we already own.

### 4.3 ⇒ Why posture C is dominated

To give a Mode-2 guest a real `card0` with a head and an EDID, the work is **not** "emulate the
GA10x display engine". It is: satisfy a gate, and answer three data controls whose semantics are
readable in a tree we already vendor. **Emulating EVO would be re-deriving, at 100× the cost, a
path NVIDIA wrote for exactly our situation.**

### 4.4 ⊘ And it explains why the C's present path does NOT port

`virtual_modeset.md` predicted the present path is reused verbatim by Mode 2. Precisely:
- The **consumer** half — QEMU-side EGL import → window, `nvkvm_present_egl.c`, **569 LOC** — is
  genuinely mode-agnostic and does port.
- The **producer** half — the 313-line virtual KMS head — **has no Mode-2 analogue and cannot be
  ported**, because in Mode 2 there is **no guest kernel module of ours to put it in**. The stock
  `nvidia-drm` owns `card0`.
⇒ Under posture B the producer is replaced by **stock NVKMS on the displayless HAL**. Under posture
A there is no producer, and frames leave the guest some other way (§7).

---

## 5. ★★★ THE GATE — and it is a BUILD FLAG, not a device property

The honest limit, and it is what keeps posture B's error bar at ±100 %.
`src/nvidia/src/kernel/gpu/gpu.c:1958-1984`:

```c
NvBool gpuIsGridDisplaylessClassSupported_IMPL(OBJGPU *pGpu) {
    NvBool bDisplayPresent = (GPU_GET_KERNEL_DISPLAY(pGpu) != NULL);
    if (bDisplayPresent) return NV_FALSE;   /* mutually exclusive with DISPLAY_COMMON */
    if (osIsGridSupported(pGpu)) return NV_TRUE;
    return NV_FALSE;
}
```
and `:2280-2287` deletes the class from the class DB when that returns false.

**Condition 1** — no `KernelDisplay` — is **already satisfied** by the C's existing fuse lie (§3.1).
Free.

**Condition 2** — `osIsGridSupported()` — is **not device-visible**
(`arch/nvalloc/unix/src/os-hypervisor.c:918`):
```c
NvBool osIsGridSupported(OBJGPU *pGpu) {
    return (os_is_grid_supported() || os_get_grid_csp_support() || hypervisorIsVgxHyper());
}
```
and `kernel-open/nvidia/os-interface.c:1445-1458` resolves the first two to
`#if defined(NV_GRID_BUILD)` / `NV_GRID_BUILD_CSP` — **compile-time flags of the guest driver
build**. The third is the *host-side* vGPU hypervisor role, not a guest property.

⇒ ★★★ **On a STOCK guest driver, `NVA083` is unreachable no matter what the device says.** There is
no register, no fuse, no GSP field, no VBIOS byte that turns it on. NVIDIA ships a **separate vGPU
guest driver package** with `NV_GRID_BUILD` defined.

★ **This reframes display from an engineering unknown into a DRIVER-MATRIX choice** — which is a
scheduling question the owner already owns (*"port to all architectures and driver versions"*), and
a far better kind of problem than a reverse-engineering unknown. It also lands squarely on
`support_matrix_asymmetry.md`: adding the vGPU guest driver to the supported set is an **additive**
axis move, not a core patch.
⊘ **What it must not become:** patching the guest driver to define `NV_GRID_BUILD`. That would
forfeit *"a STOCK, unpatched guest"* — the property that makes this artifact the oracle it is.

### 5.1 And this is exactly why §0's measurement is the deciding one

`src/nvidia-modeset/src/nvkms-rm.c:1819-1859` is a **three-way** branch inside `AllocDevice`:

1. `NV04_DISPLAY_COMMON` in the class list → normal display path;
2. else `NVA083_GRID_DISPLAYLESS` → displayless path, `pDevEvo->displaylessHw = NV_TRUE`;
3. **else → `NVKMS_ALLOC_DEVICE_STATUS_NO_HARDWARE_AVAILABLE`, `goto fail`** — with the comment
   *"Not supporting `NV04_DISPLAY_COMMON` is expected in some configurations: e.g., GF117 (an
   Optimus-only or 'coproc' GPU), emulation netlists. Fail with 'no hardware'."*

A stock guest on our display-fused-off emulated GA106 takes **branch 3**. That is a *named,
expected, supported* outcome — **not** a crash. So the entire question is whether anything above
NVKMS needs `AllocDevice` to have succeeded. §0. §8.

---

## 6. THE ORACLE QUESTION — and the oracle is WORSE than blind here

### 6.1 Why no differential oracle exists for display today — the reason, not the absence

- ★★ **`nvdiff` would record display traffic and capture NONE OF ITS CONTENT — which is worse than
  not seeing it.** ⊘ *Corrected in this audit*: an earlier draft of this section said the shim
  "cannot see" NVKMS. **MEASURED, `nvdiff_shim.c`:**
  - `:115` matches on `strncmp(path, "/dev/nvidia", 11)`. `/dev/nvidia-modeset` **passes that
    prefix** — the shim *does* record it, under the device name `nvidia-modeset`.
  - `:196` (`arg_len`) then takes the `"ioc"` branch and returns `iocsize`. NVKMS's wrapper is
    `_IOWR('m',0,{u32 cmdType; u32 size; u64 address;})` ⇒ `iocsize = 16`. **Sixteen bytes are
    recorded: the wrapper. The payload at `address` is never followed — zero bytes, on both sides
    of the call.**
  - `/dev/dri/card*` and `/dev/dri/renderD*` fail the prefix entirely ⇒ genuinely invisible.

  ⇒ ★★★ A display workload would produce a **well-aligned, plausible, green** nvdiff over records
  that contain an opaque `cmdType` and nothing else. This is the tree's own objection to `strace`
  (*"without a before/after pair you cannot tell 'RM wrote nothing' from 'we didn't capture the
  reply'"*) — reproduced **inside our own instrument**, where it looks like coverage.
  ★ **But this also makes the fix small and nameable**, which "no oracle" would have hidden: follow
  the `address` pointer and size the inner payload by `cmdType` — those sizes are in `nvkms-api.h`,
  which §1.1 establishes is **open** — and widen the prefix to `/dev/dri/`. That is an
  oracle *extension*, not an oracle *invention*. ⊘ Scoped, not proposed: §8.4 does not need it.
- **No capture contains display traffic, and it is UNMEASURED rather than empty.** Every committed
  capture was produced by a **compute** workload — `cup2`/`cup8`, `nvd_prog`, `nvidia-smi`
  (memory: *every oracle we own was made by nvidia-smi*). A compute workload cannot emit display
  traffic. ⊘ **Their silence on display is evidence of nothing.**
- **The bench cannot produce a host-side display reference at all**: rented vast.ai boxes, no
  physical connector. A host `nvdiff` reference for *scanout* is **physically impossible there**,
  independent of any tooling. ⇒ For anything past modeset *bring-up*, a host-vs-guest differential
  is not available at any price on the current bench.

★ **But §4 makes this matter much less than it looks.** The reason an oracle was load-bearing for
compute is that RM semantics had to be *guessed*. For display the semantics are **readable**: 80 kLOC
of NVKMS, plus a 885-line displayless HAL, plus a 6-control class header with documented fields. That
is a **stronger** instrument than a differential capture for *bring-up* questions, and it is the
instrument that produced §4 and §5 in this audit, at zero bench cost.
⊘ It is **not** stronger for *behavioural* questions (does a real flip complete, in what order, with
what timing) — there the oracle gap is real and unclosable on this bench.

### 6.2 ★★★ NEW, MEASURED: the oracle is POSITIVELY WRONG on display, and the tree's guard MISSES IT

Two findings, both from `src/qemu/mode2_initctrl_ga106.h` (2026-08-12).

**(a) The single most consequential empty row is a DISPLAY row.**
`{0x20800a4bu, status 0, psize 4, dlen 0}` (`:6257`) is `NV2080_CTRL_CMD_INTERNAL_DISPLAY_GET_
IP_VERSION`. Per the control census, *"the reply **SELECTS THE WHOLE DISPLAY HAL** —
`gpuInitDispIpHal(pGpu, ctrlParams.ipVersion)`."* Real GA106 answers `00 00 01 04`
(`../nvkvm-rs/crates/kayfabe-abi/src/oracle.rs`, measured 2026-08-01); the C's empty row decodes to
**0**. It is already on the **contradicted** list. ⇒ Display sits **exactly on top of** the oracle's
known-wrong region — the same class as `CE_GET_FAULT_METHOD_BUFFER_SIZE` decoding to 0 against a
real 20480.

**(b) ★★★ The documented limit is "11 of 56 rows are `dlen = 0`". There are SIXTEEN MORE that are
TRUNCATED — `0 < dlen < psize` — and they are counted among the "45 rows with bodies".**
Measured by parsing all 56 rows:

```
dlen == 0          (documented)                 11   (19.6 %)
0 < dlen < psize   (TRUNCATED — NOT documented) 16   (28.6 %)
dlen >= psize      (complete)                   29   (51.8 %)
```
Worst offenders: `0x20800a22` missing **18 216 of 34 592** bytes (53 %); `0x20800a40`
`GET_DEVICE_INFO_TABLE` missing 8 196 of 24 580; `0x20800b03` missing 8 160 of 16 352. Eleven others
lose a 4–32-byte **tail** — two distinct causes (a ~8/16 KiB recorder cap, and a dropped struct tail).

⇒ ⚠ **`CLAUDE.md`'s ★★★ FIFTH LIMIT says *"every row carrying a body matches BYTE FOR BYTE"*. That
comparison could only ever have covered the captured prefix.** Only **29 of 56 (51.8 %)** rows are
complete, not 45. A truncated tail **decodes to zeros** — the exact defect the rule exists to
prevent, one level down.
★ **And it has ALREADY BITTEN, ON DISPLAY**: `0x20800a01` (`INTERNAL_DISPLAY_GET_STATIC_INFO`) is
`psize 36, dlen 32`. `numDispChannels` lives in the missing 4 bytes, read as 0, and produced the
NULL `clientChannelTable` of §3.1.
★ ⊘ **The Rust guard does not catch this class as a refusal.** `captured_row_evidence`
(`kayfabe-abi/src/oracle.rs:744`) *does* return `BodyTruncated` — but the doc-level rule, the
`EMPTY_CAPTURE_ROWS` remediation list, and the 11-row headline are all built around `dlen == 0`.
**16 rows are in a documented-as-safe majority they do not belong to.** ⇒ Worth its own correction,
folded into `CLAUDE.md`'s FIFTH LIMIT. **Out of scope here; recorded so it is not re-derived.**

---

## 7. WHAT DISPLAY NEEDS THAT THE COMPUTE PATH DOES NOT — preconditions vs nice

| need | verdict | evidence |
|---|---|---|
| **Host display hardware / a physical output** | ⊘ **NOT a precondition** — and must never become one. The C produced a pixel-exact desktop on headless rented boxes | `acb8e68`; bench is vast.ai |
| **A second BAR** | ⊘ **NOT a precondition.** Mode-1 present crossed as a **dma-buf fd over SCM_RIGHTS**, not a memory window | `e297cd5`, `d9d58c3` |
| **Interrupt delivery / vblank** | ⊘ **NOT a precondition.** `#108` was closed as *not-a-bug*: software vblank pacing sufficed; host-paced vblank *"only matters with a live window consumer"* | memory `desktop_on_host_session.md` (2026-06-02) |
| **A compositor in the guest** | ★ **PRECONDITION for posture A/B** — and it is **guest userspace**, not our code. Measured working: headless sway/weston + wlroots capture | `headless_compositor_unlock.md` |
| **Vulkan/EGL device creation surviving NVKMS** | ★★★ **THE precondition, and it is UNMEASURED** | §0, §5.1, §8 |
| **NVENC** | ★ precondition for *streaming out*, and **already working** (720p parity 0.96×) | `ee53c90`, `PRE_PUBLIC_CHECKLIST.md` |
| **The kernel GR channel / RC watchdog** | ⚠ **a real coupling, easy to miss**: shipping graphics/display **enables** the emulated-axis kernel GR channel (`FERMI_TWOD_A`, 5 × `NV902D`) that compute-only never arms | `docs/design/kernel_gr_channels_and_the_mme_exposure.md` |

★★ **Nothing on the hardware axis is a precondition.** That is the good news, it is MEASURED, and it
is the opposite of what "display plane" intuitively implies.

---

## 8. IS IT REQUIRED FOR v1.0? — and the deciding experiment

### 8.1 The apps: display is required by ZERO of them

`tests/perf/realapp_matrix.md` (2026-06-01) — the real matrix, and it is **22 workloads, not 30**
(the Rust `PRODUCT_POSITIONING.md`, owner 2026-08-08, also says *"22 real GPU apps"*; the owner's
2026-08-11 *"30"* is not backed by any list found in either tree — **worth confirming with the
owner, cheap, and it changes the parity gate**):

| bucket | count | needs display? |
|---|---|---|
| CUDA compute | 10 | no |
| PyTorch AI | 7 | no |
| LLM (llama.cpp 7B) | 2 | no |
| Graphics/media — **"Headless throughout"** | 3 + NVENC | **no** |

The matrix's own graphics section states it is **headless throughout** (EGL device platform, Vulkan
compute, ffmpeg) over the render node. ⇒ **MEASURED: not one of the 22 requires a display plane.**

### 8.2 The LLM north star needs none of it

Compute + NVENC at most. Display is **separable** and can ship after v1.0 without touching the
release commitment as written on 2026-08-08 (*"CUDA at ~host parity, multi-process, graphics/Vulkan,
NVENC, 22 real GPU apps, 7B LLM, PyTorch"* — **"display plane" does not appear**).
★ **A ruling's date is part of its citation**: the 2026-08-11 verbatim scope **adds** display over
the 2026-08-08 commitment. Both are the owner's. **This doc does not adjudicate that** — it observes
that the two differ by exactly this line item, and that §8.1 says nothing else in the list depends
on it.

### 8.3 ★ And the posture was arguably already chosen — 2026-06-13

Memory `mode2_graphics_product_angle.md`, agreed with the owner:

> *"GRAPHICS SPINE = HEADLESS render → in-guest VIRTUAL DISPLAY/framebuffer → stream out
> (VNC/RDP/Moonlight-style). This is BOTH the more useful framing (remote workstation / cloud-render)
> AND the LOWER-RISK engineering path than present-to-live-host-window."*

⇒ Under that spine, **frames leave the guest over the network and the VMM has no display code at
all.** ★ **CHECK WHETHER THE QUESTION IS ALREADY ANSWERED** — this is close to a sixth instance. But
apply the date rule honestly: it was decided **before** the owner's 2026-08-11 *"display plane mode 1
C had"*, and it is a **product** decision, not an architecture ruling. It should be **re-confirmed,
not assumed** — which is cheap, and is the second-highest-value question here after §8.4.

### 8.4 ★★★ THE EXPERIMENT — one bench session, no context required

> **Name:** *does headless Vulkan survive a displayless GPU?*
> **Cost:** one Mode-2 boot. No build. No new code. No host-side reference.
> **Converts:** the single largest unknown (§0) — whether graphics is separable from display.

**Steps, runnable by someone with no context:**

1. Boot a Mode-2 guest the normal way — `scripts/mode2_diag/bench_boot.sh`, then
   `bench_wait.sh`. ⚠ Allow **20–25 s** to a login prompt; `-serial file:` **lags**; `nvktap0` does
   **not survive a host reboot** and QEMU needs it to pre-exist. A slow boot is not a crash.
2. In the guest, over ssh: `sudo modprobe nvidia-modeset` — then **immediately** persist the driver's
   output: `dmesg > run_<tag>_dmesg.log`. ⚠⚠ **The serial log is NOT where this output goes** — a
   post-boot `modprobe` writes to whoever ran it and **nowhere else**. Assert the file is
   **non-empty and contains `NVRM`** before believing anything.
3. Record whether `/dev/nvidia-modeset` appears, and grep the log for
   `NVKMS_ALLOC_DEVICE_STATUS_NO_HARDWARE_AVAILABLE` / *"Failed to initialize the display subsystem"*
   — the branch-3 signature of `nvkms-rm.c:1857`.
4. Run `vulkaninfo --summary` in the guest. **The whole experiment is one bit: does the RTX 3060
   enumerate, or does the ICD fall back / fail?**
5. Belt and braces: repeat step 4 with `/dev/nvidia-modeset` removed, to confirm the result is
   attributable to NVKMS and not to something else on the boot.

**Pre-registered predictions — write these down before running** (`a_falsifier_that_cant_tell_THE_
blocker_from_the_ONLY_blocker`: use three outcomes, not two):

| outcome | reading | consequence |
|---|---|---|
| **Vulkan enumerates, NVKMS returned NO_HARDWARE** | graphics is **independent** of display | posture A ships v1.0 graphics; display → a **streaming** feature; **days**, not weeks |
| **Vulkan fails, and the log shows the NVKMS failure on its path** | graphics is **gated** on a display object | posture B becomes **mandatory** and the §5 build-flag problem is a **v1.0 blocker** — schedule beside `cuCtxCreate` |
| **Vulkan fails for an unrelated reason** (ICD staging, `cuInit`, the `cuCtxCreate` wall) | **the experiment did not run** | ⊘ **do NOT read this as either answer.** Mode-1 measured guest Vulkan ICD staging as its own independent failure (`graphics_milestone_start.md`) — expect this outcome and be ready to distinguish it |

⚠ **Traps that have each cost real cycles here** — encoded so this is runnable cold:
`pgrep -x qemu-system-x86_64` can **never** match (`comm` truncates at 15) and `pgrep -f <literal>`
**always** matches the asker; use `pgrep -x qemu-system-x86` plus `ss -tln | grep 2223`, and the
bracket trick for `-f`. Have the job write a **start marker and an exit-status line**: a zero-byte
output file is a *state needing its own check*, not "not yet" — and `143` (the job was killed) and
`124` (the *launcher* timed out while the job ran on fine) mean **opposite** things.
⚠ **Any result must carry the SOURCE REVISION it was measured at.**

---

## 9. SUMMARY FOR SCHEDULING

1. **Display is not a second `cuCtxCreate`.** That wall is an unknown *mechanism*; this is a known
   mechanism behind a **scoping decision** (§0) and a **driver-package decision** (§5).
2. **The C's cross-boundary display surface was EMPTY** — every pixel produced with zero host
   display calls (§2.2). Date-durable; the ruling rests on the host boundary, which the rewrite
   does not change.
3. **NVIDIA already wrote our display plane** — `NVA083_GRID_DISPLAYLESS`, 885 lines, **zero display
   channels**, six pure-data controls, fed by a GSP event **we already own** (§4).
4. **It is gated behind a guest-driver BUILD FLAG**, not a device property (§5). ⇒ a support-matrix
   move, and never a guest patch.
5. **Zero of the 22 measured apps and none of the LLM north star require display** (§8.1–8.2). It is
   separable **unless** §8.4 comes back "gated".
6. ⚠ **The oracle is positively wrong on display**, and **16 truncated rows** hide in a
   documented-as-safe majority (§6.2). Fold into `CLAUDE.md`. And our own recorder would go
   **green on empty display records** (§6.1) — a small, named fix, not a missing instrument.
7. **Run §8.4 before scheduling anything here.** One boot. One bit. It moves the estimate between
   *days* and *weeks*.
