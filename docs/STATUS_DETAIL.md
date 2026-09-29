# Status in detail — what runs, what does not, and where it was measured

> ### STATUS — 2026-09-26 / **LIVE**, written at `master` `74dc3113`
>
> ★ **Updated 2026-09-27 at master `db038f5f`** (v3-mc20; checked against the v3-mc21 merge candidate
> `4c48ca0c`): a truth pass. Claims that said "on branch X, not on `master`" are corrected where they
> stand — each correction is a dated ⊘ note placed at or just before the text it corrects, and that
> text is kept. The latest merge bar is in §1. Branches in flight and the next steps:
> [`STATUS_AND_HANDOFF.md`](STATUS_AND_HANDOFF.md).
>
> ★ Updated 2026-09-26 on branch `v3-gpcmask` (§1, §5): floor-swept GR — the first boot on a die
> whose GPC mask is not `0..n` ([`design/V3_FLOORSWEPT_GR.md`](design/V3_FLOORSWEPT_GR.md)).
>
> ★ Updated 2026-09-26 on branch `v3-gfxset` (§3, §7): nvkvm-pv's headless-graphics test set
> ([`design/V3_GFX_TESTSET.md`](design/V3_GFX_TESTSET.md)), roadmap item 2.
>
> The long form of the README's status. Each line names the revision, the box and the document
> that holds the evidence. ⚠ Where a dated measurement and a summary disagree, believe the
> measurement. ⊘ The pre-v3 version of this file (the `kayfabe-*` tree, the old test-suite
> counts, and the corrections behind them) is archived verbatim at
> [`archive/STATUS_DETAIL_pre_v3.md`](archive/STATUS_DETAIL_pre_v3.md).

All hardware results come from rented vast.ai boxes. Those boxes are **themselves KVM guests**,
so every kayfabe guest is nested; that inflates the cost of a VM exit by roughly 10–40× (see §4).
The host driver is NVIDIA **580.159.04 (open)**, and the guest runs the stock driver from the
same `.run`. "Bare metal" means the same program on the same box's host, with no kayfabe.

## 1. Boot and the thin-guest suite

| result | revision | box / GPU | evidence |
|---|---|---|---|
| v3 harness gates **9/9** (`kf-gate1`…`9`, real GPU, no QEMU) | `d536595d`; `f703cdaf` | vast, RTX 3060 GA106 | `traces/v3_int_ga106/v3_gates_d536595d.txt`, `traces/v3_appfix/v3_gates_f703cdaf.txt` |
| thin guest **30/30** (`KF_DEVICE=kf3 fast_suite.sh`, one boot per arm, 180 s each) | `d536595d`; `f703cdaf` | GA106 | `traces/v3_int_ga106/fast_suite_d536595d.txt`, `traces/v3_appfix/fast_suite_f703cdaf.out` |
| thin guest **30/30** on **Ada**; bare metal 30/30 after one client fix | `1d6bb323`, `6ccb4585` | vast 52660152, RTX 4060 Ti AD106 | [`design/V3_FAMILY_PORT_ADA.md`](design/V3_FAMILY_PORT_ADA.md) |
| the only Ada boot blocker (a SEC2 scrubber handoff register), fixed as a per-family row and A/B-checked on hardware | `09a3944b` | AD106 | same, §2 |
| ★ **floor-swept GA104** (`GR_GET_GPC_MASK = 0x3e`): realize refused at `283a5304` (*"a non-contiguous GPC mask"*); thin guest **30/30** after the fix, v3 gates **9/9**; bare metal 30/30; fat-guest CUDA `cup3`/`cup8` pass; the guest's GR floorsweeping controls answer byte-identically to the host's | `25edb757` (30/30, `fb-mb=6144`); `948b38e2` (gates, suite at the lanes' new card-aware default); `2e32b7b1` (CUDA); `0d8426a3` (probe diff); `524b3d17` (head: gates 9/9, suite 30/30) | vast 52739422, RTX 3060 Ti 8 GB | [`design/V3_FLOORSWEPT_GR.md`](design/V3_FLOORSWEPT_GR.md), `traces/v3_gpcmask/` |
| ★ **Blackwell GB203**: thin guest **30/30** and gates **9/9** (first FSP-booted family); bare metal 30/30; fat-guest CUDA ladder passes. On `master` since `6fce7f54` | `256ec854` (thin, gates, ladder); `54158daa` (bare; code = `256ec854`) | vast 52730218, RTX 5080 GB203 | [`design/V3_FAMILY_PORT_BLACKWELL.md`](design/V3_FAMILY_PORT_BLACKWELL.md), `traces/v3_blackwell/` |
| ★ **merge bar v3-mc20** (2026-09-27): every `kf-*` crate test **1639 / 0**, gates **9/9**, `KF3_RC=0`, thin guest **30/30**. `master` `db038f5f` = this revision + its evidence commit | `c0ef7b75` | vast 53004208, RTX 3060 GA106 | `traces/v3_mc20/` |

Every arm reports `forwarded>0` and `emulated=0` for its passthrough tokens. The suite is graded
by `kayfabe-rm-ladder`, the raw client, which is still built from the frozen pre-v3 crates.

## 2. CUDA and real applications (fat guest)

- **CUDA ladder:** `cup3` returns `CUP3_VAL=43` and `cup8` (2048² matmul) returns
  `BAD=0 MAXERR=0`. Re-checked after the one-host-TSG-per-guest-TSG change, at `486ab7c6` and
  `353ff44a` ([`design/V3_VIDEO_ENGINES.md`](design/V3_VIDEO_ENGINES.md) §5).
- **App matrix, R2 at `670bd310`:** **58/65 apps pass in the guest; the host passes 65/65.**
  Six stream-shape probes pass 6/6. The result is the same with and without guest persistence
  mode. The run used vast 52624429, an RTX 3060 GA106 on an AMD EPYC 7452.
  ⊘ *2026-09-27: `v3-apps2` is on `master` since `f108f47a` (v3-mc9), the document and its evidence
  both — the next sentence's "not on `master`" is stale.*
  `docs/design/V3_APP_MATRIX.md` §R2 has this, and is **on branch `v3-apps2`, not on `master`**;
  its evidence is under `traces/v3_app_matrix/` on that branch. Passing apps include PyTorch
  (`torch_correct`: CNN training-step digest equal to the host's), Hugging Face `generate`
  (digest equal to the host's), llama.cpp (generated text equal to the host's) and
  `llama-bench`, CuPy, hashcat, Blender Cycles CUDA+OptiX (mean equal to the host's),
  Geekbench 6 (OpenCL), clinfo, and the CUDA samples, including every CUDA-graph and
  multi-stream sample.
- **Fixed on `master` since R2** (branch `v3-appfix`, merged at `74dc3113`; box vast 52689820,
  GA106), per [`design/V3_BUILD.md`](design/V3_BUILD.md), *App-matrix fixes*:
  - `gpu_burn` crashed because the guest refused `nvidia-smi -l` event arming. Fixed at
    `b0ceaf09`.
  - Host BAR1 leaked about 4.5 MiB per process through Translated rings that were never
    released, so a boot stopped at about 60 processes (cause J). Fixed at `f372f63f`. After
    the fix, **100/100 processes ran in one persistence-mode boot**, with host BAR1 flat.
  - A re-run of the single-stream and UVM rows scored guest 38/43, host 43/43. The **full
    65-app matrix has not been re-run since.**
- **Still failing — six apps:**
  - ⊘ *2026-09-27: `clpeak` is misfiled in class C below. Its host Xid 31 came from kf3 refusing
    `MC_SERVICE_INTERRUPTS`: the blocked waiter read the refusal as the end of its wait (a forged
    completion). kf3 serves it since `56032c46` (`v3-mapfix`, on `master` since `01b8cb5a`), and
    `clpeak` passes in the guest at `393012fd` ([`design/V3_REFUSAL_AUDIT.md`](design/V3_REFUSAL_AUDIT.md)
    §1, §6.2). That was the refusal audit's workload set; the app matrix has not been re-run, so its
    recorded count is unchanged.*
  - **C, UVM demand paging (5 apps):** `conjugateGradientUM`, `attach_verify`,
    `UnifiedMemoryPerf`, `torch_ai_bench`, `clpeak`. They fail on managed memory touched first by
    the CPU or GPU, and on kernels that access pageable host memory (HMM). What the guest UVM
    *maps* is published correctly. What fails is demand paging: the guest expects a replayable
    fault, kf3 delivers none, and the host RM tears down the channel group (Xid 31 `FAULT_PDE`).
    The guest is told — CUDA returns 719 — but `conjugateGradientUM` prints `SUCCESS` because it
    checks neither its sync status nor its result.
    - This cannot be closed from host userspace. A replayable host fault needs a VA space that
      UVM owns, and the fault-buffer class is kernel-privileged.
    - ⊘ *2026-09-27: that document is on `master` since `f740da10` (v3-mc9). Its continuation — §11
      (no NVIDIA patch: a separate module, N4), §12 (N4 in depth; experiments E5, E6, E6′), §13 (the
      guest-side fault plane) — is on branch `v3-uvm-n4`, and E6″ phase 0 (§12.6, a capture of the
      real libcuda's mapping calls; no fault numbers yet) on `v3-uvm-e6pp` `c6765f5c`; neither is on
      `master`. The owner prefers a separate host module to a patch, and the route is undecided
      ([`OWNER_RULINGS.md`](OWNER_RULINGS.md) §E).*
      The research is `docs/design/V3_UVM_DEMAND_PAGING.md`, on branch `v3-uvm-research`
      (RESEARCH, no code). Its recommendation is a patch to the host's open nvidia-uvm that
      diverts a VA space's faults to kayfabe, which then replays or cancels them.
    - ⊘ **MEASURED AND FIXED 2026-09-26 (branch `v3-roperm`; ⊘ *on `master` since `59cc98a9`,
      v3-mc14 — 2026-09-27*) — this bullet used to say
      "inferred, not measured".** The same document reported, from source reading, that the
      walker **drops the guest PTE's READ_ONLY bit**, so a read-only duplicate was mapped
      read-write on the host. Measured on master `283a5304` (vast 52732498, GA106):
      `readmostly_probe gpuwrite` and `downgrade` returned **`bad=1048576` — every value wrong —
      with no CUDA error and no Xid**. On `v3-roperm` one host policy (`kf_mem::apply::PermPolicy`)
      keys and maps READ_ONLY and VOLATILE (`NVOS46` `ACCESS_READ_ONLY` / `GPU_CACHEABLE_NO`):
      both modes now fail **loudly** (719 + host Xid 31 `FAULT_RO_VIOLATION`), never with a wrong
      value. They pass on bare metal only because the write fault is replayable there, which is
      fault delivery (class C above). ATOMIC_DISABLE is carried only with
      `KF3_CARRY_ATOMIC_DISABLE=1`: carrying it made `atomicAdd_system` on a CPU-resident managed
      page a 719, where leaving it off gives the bare-metal values. PRIVILEGED leaves are withheld
      from user twins: none in CUDA spaces; vkpeak's GR context buffers were withheld and it ran at
      host speed. Verified at `d6959acb` (gates 9/9, fast suite 30/30, ladder, 9/9 app samples).
      Evidence: `traces/v3_roperm/`.
  - **C′, a refused host map (1 app):** ⊘ *2026-09-27: the refused map is fixed, on `master`; the
    app still fails. The rest of this bullet is the pre-fix signature at `670bd310`, and its
    "working on it … not on `master`" is stale. Cause of the refused map: the walker reported the
    stale 4 KiB PTEs under a valid 64 KiB PTE as live leaves, so two host maps covered one VA and
    host RM refused the second (`Other(31)` = `NV_ERR_INVALID_ARGUMENT`). A valid big PTE now owns
    its slot, and a refused map no longer poisons the guest's shared UVM kernel channel
    (`62a50c44`, `v3-mapfix`; on `master` since `283a5304`, the rest of the branch since
    `01b8cb5a`). `UnifiedMemoryStreams` was re-run in the guest after the fix (the merge commit
    `283a5304` says so: "fails contained (719)"), on vh (vast 52624429, RTX 3060 GA106): kf3
    `80264e63` in runs `mapfix_g1`, `mapfix_g2` and `mapfix_contain`, and `56032c46` in
    `mapfix_cfix20` (all in `traces/vh_archive/vh_apps_results.tgz`). None of those runs logs
    `not applied`; the pre-fix
    runs `nb1`, `iso1` (`670bd310`) and `mapfix_base` (`74dc3113`) each logged one. The no-PM boot
    no longer wedges: in `mapfix_contain` the apps after it in the same boot pass. **The app still
    fails in every run:** CUDA 719 at `cudaStreamAttachMemAsync` / `cudaStreamSynchronize`, or
    CUBLAS 13 at `cublasDgemv`. kf3 logs `RC host twin … except_type=0x1f (Xid 31)` on eight of its
    channels, and the one host-dmesg window that caught it (`mapfix_cfix20`) shows Xid 31
    `FAULT_PTE VIRT_READ`. The pre-fix run with persistence mode (`pm2`, `670bd310`) had already
    failed the same way (719 at `cudaStreamAttachMemAsync`), with no refused map in the app's kf3
    log. So the refused map was not needed for the failure, and what faults is still open. The
    65-app matrix has not been re-run.* `UnifiedMemoryStreams`. kf3 logs `1 run(s) not applied:
    map … Other(31)`, and the host then reports Xid 31 `FAULT_PTE` inside that range. Without
    persistence mode the boot stays wedged afterwards. Branch `v3-mapfix` is working on it; it
    is not on `master`.

## 3. Graphics, video, multi-process, multi-GPU

- **Headless graphics** ([`design/V3_HEADLESS_GRAPHICS.md`](design/V3_HEADLESS_GRAPHICS.md)
  §6; kf3 `68fb3768`; vast 52661950, RTX 3080 Ti GA102). Five steps pass, and every render
  **matches bare metal bit for bit** on the same box:
  1. `nvidia-drm modeset=1` registers `card0` and `renderD128` in displayless mode.
  2. `vulkaninfo`, plus a Vulkan compute job with every element checked.
  3. A Vulkan two-pass render with a depth test.
  4. An EGL desktop-GL 4.6 render (FBO, depth, textured pass).
  5. Xvfb + VirtualGL running a GLX window.

  Evidence is in `traces/v3_gfx/`. It was re-run on the merged tree at `d536595d`
  (`traces/v3_int_ga106/gfx_guest_d536595d.txt`).
- **nvkvm-pv's headless-graphics test set** ([`design/V3_GFX_TESTSET.md`](design/V3_GFX_TESTSET.md);
  `v3-gfxset` `d06833f0` = master `59cc98a9` + the branch — ⊘ *2026-09-27: on `master` since
  `f89f66bb` (v3-mc17; its bar, 1625 / 0, gates 9/9, 30/30, is recorded in the handoff at `e9c37b1c`,
  and no log of that run is in the repo)*; vast 52775275, RTX 3070 GA104): nvkvm-pv's 25
  headless rows in 23 items (Vulkan, EGL/GLES, GBM, dma-buf sharing, headless weston and sway with
  capture, glmark2, NVENC/NVDEC, Geekbench Vulkan, Blender Open Data) and 15 more (ffmpeg
  CUDA/Vulkan/OpenCL/libplacebo filters, Blender Cycles CUDA/OptiX, EEVEE on GL and Vulkan, VirtualGL,
  optical flow). **38/38**: 31 items byte-identical to bare metal, the Cycles renders inside bare metal's
  measured spread, 5 by nvkvm-pv's own criterion, no Xid. Master `dd3aed08` was 33/37: the optical-flow
  engine was not advertised, and ctxsw preemption was refused on copy channels (ffmpeg's Vulkan device);
  both fixed on the branch, with the SW engine row order the first fix exposed. Display is next (§7).
- **NVENC/NVDEC** ([`design/V3_VIDEO_ENGINES.md`](design/V3_VIDEO_ENGINES.md); vast 52661900,
  RTX 3060 GA106). Every NVENC and NVDEC output of the graded lane is **byte-identical to bare
  metal**. The engine inventory comes from host queries and ogkm, not from a per-die row. Turing,
  Ada and Blackwell video are derived from source and **not measured**. Hopper video is refused
  by name.
- **Several CUDA processes per guest:** 100 sequential processes in one persistence-mode boot
  (above). Without persistence mode, R2 measured 39 before cause J was fixed. The no-PM budget
  has not been re-measured after the fix.
- **Multi-GPU** ([`design/V3_MULTI_GPU_AUDIT.md`](design/V3_MULTI_GPU_AUDIT.md) §7; `74fced87`;
  vast 52664396, 8× RTX 3060, with 2 of the GPUs used). There is one kf3 device per host GPU,
  keyed by host identity rather than minor number.
  - Gates pass 9/9 on GPU 0 and on GPU 1.
  - The thin guest with two distinct host GPUs passes, run one at a time and concurrently. That
    includes host minors that differ from their RM instance numbers.
  - Asking for the same card twice is **refused at realize, by name**, when the BAR1 budget does
    not fit.
  - The single-device suite still passes 30/30.
  - **Not measured:** CUDA inside a two-GPU guest, UVM across two GPUs, and peer access.

## 4. Performance

LLM decode, `Qwen/Qwen2-0.5B-Instruct`, Hugging Face eager, from `V3_BUILD.md`
(*LLM lane measured on kf3*, w828/w828b; raw data in `traces/llm_parity/`). The guest's text is
**identical** to the host's for 16, 512 and 2048 tokens (sha256).

| box | guest / same box's host, steady decode |
|---|---|
| `vh3`: nested Intel | 0.30–0.31× |
| `vh`: nested AMD EPYC 7452, guest persistence mode | 0.29× |

- **Doorbells are the main cost.** There are about 1,080 doorbells per decoded token (eager
  decode is about 1,000 kernel launches), and each one is a trapped MMIO write, which means one
  VM exit. Doorbells are 99.7 % of trapped exits and, by the doorbell-module design's estimate,
  55–80 % of the guest-vs-host gap.
- On the nested AMD box each doorbell costs about **51 µs** of guest time; on the nested Intel
  box about **106 µs**. Reaching 0.8× would need ≤ ~5–8 µs per doorbell, and no trapped exit
  reaches that on these boxes.
- **The planned fix is designed, not built:** an optional guest doorbell module
  ([`design/V3_GUEST_DOORBELL_MODULE.md`](design/V3_GUEST_DOORBELL_MODULE.md), DESIGN-ONLY). It
  would let the guest write the real host doorbell page for passthrough channels through a
  read-only routing table. A doorbell would then cost a guest page fault instead of a VM exit.
  Stock guests keep the trapped path.
- Host maps: VA-contiguous guest-RAM runs are one host OS descriptor, mapped once
  ([`design/V3_BATCHED_MAP.md`](design/V3_BATCHED_MAP.md)). A 12,288-run space takes 3 map
  verbs and 1 unmap verb (it was 24,576).

## 5. GPU families and driver versions

⊘ *2026-09-27: the Hopper/Blackwell row below is stale for Blackwell. GB203 (RTX 5080) has run on
hardware since 2026-09-26 — thin guest and bare metal 30/30, gates 9/9, the fat-guest CUDA ladder, at
`256ec854` (§1; [`design/V3_FAMILY_PORT_BLACKWELL.md`](design/V3_FAMILY_PORT_BLACKWELL.md),
`traces/v3_blackwell/`; on `master` since `6fce7f54`). Hopper and datacenter Blackwell (GB10x) are
still source-derived only.*

| family | state | source |
|---|---|---|
| Ampere GA10x (GA106, GA102; ★ floor-swept GA104) | **measured**; GA104 thin guest 30/30 | above; `V3_FLOORSWEPT_GR.md` |
| Ada (AD106; ★ floor-swept AD104 GR facts) | **measured**, thin guest 30/30 on AD106; AD104 (`gpcMask 0x1d`) GR realize path replayed from its own unprivileged answers, **no guest boot** | `V3_FAMILY_PORT_ADA.md`, `V3_FLOORSWEPT_GR.md` |
| Turing | GSP model built, **never booted** | `V3_FAMILY_PORT_ADA.md` (update, branch `v3-families`, merged) |
| Hopper, Blackwell (⊘ *GB203 since measured: the note above*) | derived from ogkm-580.159.04 source, including the Hopper+ BAR1 doorbell; unit-tested against source-shaped fixtures; **never run on hardware** | [`design/V3_BAR1_DOORBELL.md`](design/V3_BAR1_DOORBELL.md) (DESIGN+CODE, hardware-unverified); gate 7 covers VER3 page tables |
| GA100 | **refused by name** | `V3_FAMILY_PORT_ADA.md` |

⊘ *2026-09-27: the next sentence is stale. The driver-version matrix (both axes) is on `master`
since `5018bb57` (v3-mc20). With host 580.159.04, the CUDA ladder passes 4/4 for guests 580.159.04,
580.105.08, 590.48.01, 595.84, 575.57.08 and 610.57.04; with guest 580.159.04, hosts 575.57.08,
580.95.05 and 580.65.06 pass gates 9/9, thin 30/30 and ladder 4/4. These were measured on `v3-drivers`
heads before the merge (GA102 boxes; each row names its revision); the grid is
[`design/V3_DRIVER_MATRIX.md`](design/V3_DRIVER_MATRIX.md) §6.0, derived from
`traces/driver_matrix/walk/`. Not yet: the 570 / 565 guest ladders (the re-init wall is fixed on
`master`; 570 then meets its UVM first-channel wall), the 550 fat guest, and 535 / 545, whose
capability rows (`a50265f8`) are held for owner review.*
Only guest driver **580.159.04** has been run. The driver-version matrix is a later roadmap step.

## 6. Not started, or not measured

- ⊘ *2026-09-27: display is started, not on `master`. `v3-display` M0 (`5dbf670b`, device property
  `display=on`, default off): with it the guest's KernelDisplay comes up on a virtual NVDisplay, and
  NVKMS stops at its first physical-RM query (`NV0073_CTRL_CMD_SYSTEM_GET_CAPS_V2`), so nvidia-drm
  stays displayless — run `m0a`, vast 52837869, RTX 3090 GA102, host and guest 580.159.04
  ([`design/V3_DISPLAY.md`](design/V3_DISPLAY.md) stop note, `traces/v3_display/m0a/`). The branch
  is in the v3-mc21 merge candidate `4c48ca0c`, whose bar is in progress.*
  **Display / scanout:** not started. Xorg with NVIDIA's own display driver needs a display
  object (`V3_HEADLESS_GRAPHICS.md` §5). Headless rendering works (§3).
- **Windows guests:** research only (`design/THE_WINDOWS_AXIS.md`,
  `design/V3_WINDOWS_DOORBELL_RESEARCH.md`). It is the last roadmap step.
- **Two VMs sharing one GPU:** not measured. The multi-GPU audit records this case as
  *assumed, not specified*.
- **Rootless end-to-end boot:** not recorded. By design, the host side uses only RM controls
  that the host driver marks `NON_PRIVILEGED`, and it needs no host kernel module
  ([`PRODUCT_POSITIONING.md`](PRODUCT_POSITIONING.md) §2.1).
- **GitHub CI** is red on `master`. It still carries the pre-v3 tree's jobs. The verdict of
  record is the hardware sequence in §1.

## 7. Roadmap (owner, 2026-09-26)

⊘ *2026-09-27, progress since this list was written (the items themselves are the owner's and stand):
item 1 — the refused map is fixed on `master`, but `UnifiedMemoryStreams` still fails (§2 C′); item 3 —
display M0 started, not on `master` (§6); item 5 — Blackwell GB203 passes 30/30 on hardware (§1, §5);
item 6 — the driver matrix is on `master`, and its walk is in progress (§5).*

1. Apps: close the matrix (UVM demand paging, the refused map).
2. The headless-graphics test set from nvkvm-pv. — ⊘ *2026-09-27: on `master` since `f89f66bb`; the
   "held there" below is stale.* **38/38 on `v3-gfxset` `d06833f0`**, RTX 3070
   ([`design/V3_GFX_TESTSET.md`](design/V3_GFX_TESTSET.md); merge-ready bar held there).
3. Display, and a desktop (Linux Mint).
4. *In parallel:* doorbell-module parity (§4).
5. *In parallel:* Blackwell on hardware.
6. The guest-driver version matrix.
7. Windows.
