# Status in detail — what runs, what does not, and where it was measured

> ### STATUS — 2026-09-26 / **LIVE**, written at `master` `74dc3113`
>
> ★ Updated 2026-09-26 on branch `v3-gpcmask` (§1, §5): floor-swept GR — the first boot on a die
> whose GPC mask is not `0..n` ([`design/V3_FLOORSWEPT_GR.md`](design/V3_FLOORSWEPT_GR.md)).
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

Every arm reports `forwarded>0` and `emulated=0` for its passthrough tokens. The suite is graded
by `kayfabe-rm-ladder`, the raw client, which is still built from the frozen pre-v3 crates.

## 2. CUDA and real applications (fat guest)

- **CUDA ladder:** `cup3` returns `CUP3_VAL=43` and `cup8` (2048² matmul) returns
  `BAD=0 MAXERR=0`. Re-checked after the one-host-TSG-per-guest-TSG change, at `486ab7c6` and
  `353ff44a` ([`design/V3_VIDEO_ENGINES.md`](design/V3_VIDEO_ENGINES.md) §5).
- **App matrix, R2 at `670bd310`:** **58/65 apps pass in the guest; the host passes 65/65.**
  Six stream-shape probes pass 6/6. The result is the same with and without guest persistence
  mode. The run used vast 52624429, an RTX 3060 GA106 on an AMD EPYC 7452.
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
  - **C, UVM demand paging (5 apps):** `conjugateGradientUM`, `attach_verify`,
    `UnifiedMemoryPerf`, `torch_ai_bench`, `clpeak`. They fail on managed memory touched first by
    the CPU or GPU, and on kernels that access pageable host memory (HMM). What the guest UVM
    *maps* is published correctly. What fails is demand paging: the guest expects a replayable
    fault, kf3 delivers none, and the host RM tears down the channel group (Xid 31 `FAULT_PDE`).
    The guest is told — CUDA returns 719 — but `conjugateGradientUM` prints `SUCCESS` because it
    checks neither its sync status nor its result.
    - This cannot be closed from host userspace. A replayable host fault needs a VA space that
      UVM owns, and the fault-buffer class is kernel-privileged.
    - The research is `docs/design/V3_UVM_DEMAND_PAGING.md`, on branch `v3-uvm-research`
      (RESEARCH, no code). Its recommendation is a patch to the host's open nvidia-uvm that
      diverts a VA space's faults to kayfabe, which then replays or cancels them.
    - ⚠ The same document reports, from source reading, that the walker **drops the guest PTE's
      READ_ONLY bit**, so a read-only duplicate is mapped read-write on the host. Its predicted
      consequence, a silent stale read under `cudaMemAdviseSetReadMostly`, is **inferred, not
      measured**.
  - **C′, a refused host map (1 app):** `UnifiedMemoryStreams`. kf3 logs `1 run(s) not applied:
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

| family | state | source |
|---|---|---|
| Ampere GA10x (GA106, GA102; ★ floor-swept GA104) | **measured**; GA104 thin guest 30/30 | above; `V3_FLOORSWEPT_GR.md` |
| Ada (AD106; ★ floor-swept AD104 GR facts) | **measured**, thin guest 30/30 on AD106; AD104 (`gpcMask 0x1d`) GR realize path replayed from its own unprivileged answers, **no guest boot** | `V3_FAMILY_PORT_ADA.md`, `V3_FLOORSWEPT_GR.md` |
| Turing | GSP model built, **never booted** | `V3_FAMILY_PORT_ADA.md` (update, branch `v3-families`, merged) |
| Hopper, Blackwell | derived from ogkm-580.159.04 source, including the Hopper+ BAR1 doorbell; unit-tested against source-shaped fixtures; **never run on hardware** | [`design/V3_BAR1_DOORBELL.md`](design/V3_BAR1_DOORBELL.md) (DESIGN+CODE, hardware-unverified); gate 7 covers VER3 page tables |
| GA100 | **refused by name** | `V3_FAMILY_PORT_ADA.md` |

Only guest driver **580.159.04** has been run. The driver-version matrix is a later roadmap step.

## 6. Not started, or not measured

- **Display / scanout:** not started. Xorg with NVIDIA's own display driver needs a display
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

1. Apps: close the matrix (UVM demand paging, the refused map).
2. The headless-graphics test set from nvkvm-pv.
3. Display, and a desktop (Linux Mint).
4. *In parallel:* doorbell-module parity (§4).
5. *In parallel:* Blackwell on hardware.
6. The guest-driver version matrix.
7. Windows.
