# V3 FAMILY PORT — Blackwell (GB203, RTX 5080): the first FSP-booted family on hardware

**STATUS: ANSWERED, 2026-09-26 (branch `v3-blackwell`, rebased on master `283a5304`).** kf3 boots a
stock 580.159.04 guest over a **GB203** host through the emulated **FSP → GSP-FMC → GSP-RM** chain;
the raw client passes **30/30 on bare metal** and **30/30 in the thin guest**, and the CUDA ladder
(`cup2` → `cup3` → `cup8` 2048² → `cup8bench`) passes in the fat guest with every ledger row
`emulated=0`. GA106 is unchanged (bare 30/30, thin guest 30/30, 1474 crate tests, gates 9/9). §6
has every number with its revision. Hardware-unverified: Hopper and datacenter Blackwell (GB10x) —
the shared FSP path is the same code, the die-group rows differ (§4, §7).

## 0. Verdict

| lane | result | rev |
|---|---|---|
| bare metal raw client, GB203 | **30/30** (25/30 before the four client fixes of §3) | `92b9f912` |
| thin guest, GB203 | **30/30** (24/30 → 28/30 → 30/30 over §4's fixes) | `bbd2a083` (`bws6`); final rev in §6 |
| fat guest CUDA ladder, GB203 | cup2 PASS, cup3 = 43, **cup8 2048² bad=0 maxerr=0**, cup8bench PASS (every timed iteration verified); 2048² matmul 3499 GFLOP/s guest vs 3391 host | `256ec854` |
| BAR1 doorbell (`V3_BAR1_DOORBELL.md` §7) | **T0**: libcuda sets `bBar1Mapping`. **T1**: UVM's view trapped, 10 doorbells through it, **negative control hangs**. **T2**: every CUDA rung rings through the BAR1 view (463–967 doorbells per boot) | §5 |
| GA106 regression (cheap GA10x box) | bare 30/30, thin guest 30/30 (`ab5ccced`), 1474 tests + gates 9/9 (`c2ac8480`); final rev in §6 | — |

## 1. Boxes

- **vb** = vast `52730218`: RTX 5080 16 GB (`0x2C02`, **GB203**, `MC_GET_ARCH_INFO` arch `0x1B0`),
  VBIOS `98.03.3B.40.17`, Intel i9-14900 (the vast KVM image is itself a VM ⇒ the thin guest is a
  **nested** KVM guest), 24 GiB RAM. Arrived with the closed 575.51.03 packages (which cannot drive
  Blackwell); `provision_host_driver.sh` swapped them for **580.159.04 open** (`OPEN_MODULE=yes`),
  and the guest runs the same `.run` with `-m=kernel-open`. ⇒ The provisioning scripts already pick
  the open flavour on both sides. ⚠ The purge globs name the 575 packages this template ships; a box
  with another closed version would need them widened.
- **vga** = vast `52734653`: RTX 3060 (GA106), Xeon E5-2673 v4, same driver — the GA10x regression box.

## 2. The FSP boot — what ogkm says RM does, and what we serve

Read from ogkm-580 first, then watched on hardware (`bw1`): the guest wrote exactly this sequence.

1. `kfspWaitForSecureBoot_GB202` polls `NV_THERM_I2CS_SCRATCH` for `0xFF` (`0xAD00BC` on GB20x,
   `0x200BC` on GH100/GB10x — the host's architecture draws the line). ⇒ a static boot register
   (`kf_chip::bar0::boot_regs`). ⊘ The FSP model's `on_read` answered it, but boot-sequence-only
   registers are never published to the shadow: the guest would have read 0 for 5 s and given up.
2. `kfspCheckForClockBoostCapability_GB100` is called from `kgspInitRm` **even on a GSP client**: an
   `NVDM_TYPE_CAPS_QUERY` packet; the real FSP on this GB203 answers OK (the host never prints the
   LEVEL_ERROR *"doesn't have clock boost capability"*), so RM then sends `NVDM_TYPE_CLOCK_BOOST`.
3. The COT (`NVDM_TYPE_COT`, 868 bytes). Its `gspBootArgsSysmemOffset` names the **GSP-FMC's
   `GSP_FMC_BOOT_PARAMS`** (`kfspGetGspBootArgs`, `kern_fsp_gh100.c:949-970`), whose
   `gspRmParams.bootArgsOffset` (+48) is the LibOS array (`kernel_gsp_gh100.c:460`). ⊘ The model
   published the FMC params' address as if it were the array ⇒ `BootSequence::boot_args_indirection`
   (default `None`; the FSP regime says 48).
4. Target mask (HWCFG2 ≠ 0), lockdown release, MAILBOX0 = 0, RISC-V active, status queue,
   `kgspWaitForRmInitDone` — the existing FSM, unchanged.

**Transport — `kf_trap::fspemem` (vCPU, lock-free).** Every packet goes through the EMEM data port
with auto-increment, and RM asserts the port moved by exactly N after **both** the write burst
(`_kfspWriteToEmem_GH100`) and the read burst (`kfspReadPacket_GH100`) — before anything asynchronous
could run. The page was already a `Hole` on Blackwell; `FspEmem` serves its six registers: cursor +
flags, the command's header words, and FSP's reply posted at the queue-HEAD write as one
single-packet `NVDM_TYPE_FSP_RESPONSE {taskId 0, commandNvdmType, FSP_OK}` (5 dwords; RM refuses
fewer). The writes still reach the plane, where the FSM reads the COT from its own window. ⊘ Only a
COT that reaches the field publishes: the window keeps stale bytes, so a second open's CAPS_QUERY
would otherwise re-publish the previous life's pointer (`fsp_cot_sequence.rs`).

`[measured bw1]` `fsp[replies=3 last_nvdm=0x14]` (CAPS_QUERY, CLOCK_BOOST, COT) and
`GSP-PUBLISH gpa=0x19e00000` — the FSP boot worked on the first boot once these four pieces existed.

## 3. The client had four GA10x-isms (bare metal 25/30 → 30/30)

The standing rule (bare-metal FAIL ⇒ client bug): all four fixed in the client, none in kayfabe.

| arm(s) | GA10x assumption | Blackwell (ogkm / measured) | fix |
|---|---|---|---|
| engines, concurrency, dictated-ring, ce-client | the engine writes USERD `GP_GET` back | **Blackwell's USERD has no `GP_GET`**: `Nvc96fControl`/`Nvca6fControl` are `Ignored00[0x23]` then `GPPut` (`clc96f.h:29-33`, `clca6f.h:27-31`); `[measured]` the word stayed 0 for 22 s after the semaphore released | `ChannelClass::userd_has_gp_get` (class < `0xC96F`); `SubmitOutcome.gp_get_by_hw`; the semaphore is the completion on Blackwell. Where GP_GET exists the probe also waits for its async write-back within the same budget. Kayfabe-side twin: `Family::engine_writes_userd_gp_get` (gates 5/6 measure instead of check on Blackwell) |
| uvm-mean P2 | `COPY(2)` is the first async CE | GB203 presents LCEs `{0,1,4,5}` — `COPY(2)` is `NV_ERR_OBJECT_NOT_FOUND` | `first_async_copy_engine` from the host's `CE_GET_ALL_CAPS` (present & !GRCE); GA106 still gets 2 |
| uvm-mean P2 | runlist = `token >> 16` | GB20x sets `RUNLIST_DOORBELL` (bit 30) in every token | `(token >> 16) & 0x7F` |
| ce-client arm 6 | a free VA has no page table (`DMA_GET_PDE_INFO`) | VER3's 512 MiB level is `PDE1`, present for every VA within 256 GiB | ignore blocks above 2 MiB (VER2 has none) |

## 4. kayfabe: GA10x/Ada facts that broke on Blackwell — each measured, then fixed

| # | where | assumption | Blackwell (ogkm-580 / measured) | fix |
|---|---|---|---|---|
| 1 | `kf-chip bar0::vbios_profile` | FSP families refused (`RowUnbuilt`) — realize failed | FSP families read no VBIOS (`kgspExtractVbiosFromRom_395e98` = NOT_SUPPORTED, `kernel_gsp.c:3990-4015`) | same image for every family |
| 2 | FSP model / publication | — | §2 items 1–3 | §2 |
| 3 | `kf-abi deviceinfo` | CE fault ids must be one contiguous run | `[bw2]` `INTERNAL_GET_DEVICE_INFO_TABLE` refused `CopyEngineFaultIdsNotContiguous {65, 70, count 3}` ×48 → empty engine list → guest NULL deref in `memmgrCalcReservedFbSpaceHal_GM107` (`pKernelGraphics`). GB203's LCEs are floorswept: `CE0 + i` = `{65, 69, 70}`, the gap is CE2/CE3's own slots (`gb202/dev_fault.h:74-79`) | legal when every id is `CE0 + instance` for one `CE0`; duplicates and foreign ids still refused |
| 4 | `kf-rm inittables` | encoder refusals silent (`Err(_) => refuse()`) | cost a boot to see #3 | `W349REFUSE … why=encoder {e:?}` on all 31 arms |
| 5 | PCIe link capabilities | served only at BAR0 `NV_XVE` `0x88084` | `[bws1]` `UVM_REGISTER_GPU` `0x40` after *"Unknown PCIe speed"*: GH100/GB20x read them by `gpuReadBusConfigCycle` — the real **config space** (`NV_EP_PCFG_GPU_LINK_CAPABILITIES` `0x6C`) or, in a VM (`bIsPassthru`, `gpu.c:4745-4769`), the BAR0 **`NV_EP_PCFGM` mirror** `0x92000+0x6C` (`kern_gpu_gh100.c:99-109`) | both: `bar0::config_words` (**kf3 ABI 7**: config-space dwords preset read-only, a collision with a capability refuses realize by name) and the `0x9206C` boot register. GB10x binds `_GB100` (`NV_PF0_LINK_CAPABILITIES` `0x4C`, a real config cycle) — served, unverified |
| 6 | `kf-rm hostquery::query_lce_pce_masks` | ask `CE_GET_CE_PCE_MASK` LCE 0, 1, … until the first refusal | `[bws3]` `UVM_REGISTER_GPU` `0x56` ← `NoMaskForEngine {COPY4, stated 1}`: floorswept LCE2 ended the list | ask over `CE_GET_ALL_CAPS.present`; a hole is `NO_PCE_MASK`, refused at the serve like a missing row; GA106 asks exactly the same 5 |
| 7 | `kf-chan translated` | a physical CE launch can always be made virtual | `[bws3]` **Xid 71 (CE4 error)** on our host ring for every PMA scrub, then `scrubberDestruct` timed out. RM's Hopper+ **fast scrub** (`LAUNCH_DMA` `MEMORY_SCRUB_ENABLE` 23:23, `clc8b5.h:84`; `memmgrMemUtilsCheckMemoryFastScrubEnable_GH100`) *"only works with physical addressing"* (`uvm_hopper_ce.c:185-196`) | a scrub with a physical dst becomes the equivalent **virtual byte zero-fill** (remap `DST_X=CONST_A=0`), guest remap registers restored; bit 23 on an Ampere class (`VPRMODE`) untouched |
| 8 | token index (`kf-core Plane`, `kf-trap`) | the doorbell token's `VECTOR` (chid) is device-unique (`& 0xFFF`) | `[bws4]` `UVM_REGISTER_GPU` `0x1a` ← *"act birth translated REFUSED: token 0x1: OverDeclaredCap {cap: 0}"*: the PMA scrubber and UVM's first channel were both chid 1 on different runlists. **Blackwell allocates chids per runlist**: `bUsePerRunlistChram` defaults TRUE on every GB1xx/GB20x (`g_kernel_fifo_nvoc.c:226-236`); the token is `RUNLIST_ID 22:16 \| VECTOR 11:0 \| bit 30` (`kernel_fifo_gb202.c:58-78`) | `kf_trap::tokenindex::TokenIndex`: `Vector{mask}` (Turing … Hopper: the old `& mask`, byte for byte) or `RunlistVector` (`runlist << 11 \| chid`; 2048 = the per-runlist count we declare; table 2¹⁸); `Family::chids_per_runlist`; a birth takes the runlist the served FIFO table gives its engine; `RC_TRIGGERED` posts the guest chid |
| 9 | `kf-chan translated` CE offsets | `OFFSET_{IN,OUT}_UPPER` is 17 bits | `[bws4]` **Xid 31 FAULT_PDE** at `0x1fffe_0005_0000` on our ring: the host put the identity window at `0x1ff_fffe_0000_0000` (top of a 57-bit VAS) and the rewrite masked it to `NVC7B5`'s `16:0`; `NVC8B5+` is `24:0` (`clc8b5.h:92,96`) | `upper_mask` by the bound CE class |
| 10 | `kf-rm` served GR static info | the guest's client `GR_GET_SM_ISSUE_RATE_MODIFIER` may stay unserved (GA10x libcuda never asks it) | `[nvdiff guest vs bare-metal T0]` fat-guest `cuInit` → `CUDA_ERROR_NO_DEVICE` right after `0x20801230` answered `0x56` (host: OK, `00 03 00 03 03 00 03 00 00`). The guest RM serves it only from the cache `INTERNAL_STATIC_KGR_GET_SM_ISSUE_RATE_MODIFIER` (`0x20800a34`) fills, which we never served | host fact `gr_sm_issue_rate_modifier` (the host's NON_PRIVILEGED client control at realize; refusal = `None`, never a realize failure) served as engine 0's row. Served universe 51 → 52; `cap1b` exception set 32 → 33 (a GR static-info sibling past the capture's closure limit) |

★ Nothing here is a per-die row: #1/#4/#6/#7/#9 are family- or class-level rules from ogkm, #3/#8 are
rules over what the host's own lists say, #5/#10 are host facts asked unprivileged at realize.
⊘ **The walker was not touched** (the coordinator's note: `v3-roperm` is changing it).
⚠ GA106-visible changes, all toward hardware: the guest's `GR_GET_SM_ISSUE_RATE_MODIFIER` now answers
(the host's value) where it answered NOT_SUPPORTED; encoder refusals now log a line.

## 5. The Hopper+/Blackwell BAR1 doorbell (`V3_BAR1_DOORBELL.md` §7) — T0, T1 (+ negative control), T2

- **T0 (bare metal, `nvdiff_shim` + `nvd_prog ce`):** libcuda allocates **two** `HOPPER_USERMODE_A`
  (`0xc661`, also on Blackwell) objects: one without params → mapped at `0xc0bb0000` = BAR0 +
  `0xBB0000` (the BAR0 view); one **with** params → mapped at `BAR1 + 0xA0000` ⇒ **libcuda sets
  `bBar1Mapping`** (`traces/v3_blackwell/t0_nvdiff_ce_bare_gb203.jsonl.zst`).
- **T1 (thin guest, `--uvm-invalidate --uvm-mean`, `c2ac8480`):** `kf3: bar1db view LIVE #1: BAR1
  0x90000+0x10000 (usermode page 0x0)` — UVM's view, placed by the guest RM's BAR1 allocator at
  `0x90000` — then **10 doorbells through it** (UVM's four Translated channels, `fwd` 2–4 each), and
  `REMOVED` at teardown; PASS. **Negative control** (`KF3_NEGCTL_NO_BAR1_DOORBELL=1`, the pre-v3-bar1db
  behaviour: the view's leaf maps guest RAM at `0x30000`): the same arms **hang in
  `UVM_REGISTER_GPU`** until the budget (CRASH) — UVM's doorbells land in guest RAM and its channel
  manager waits forever. ⇒ the detector measures the thing (`traces/v3_blackwell/t1_bar1_doorbell_gb203.txt`).
- **T2 (fat guest CUDA ladder, `256ec854`):** every rung rings through the BAR1 view — 463 (cup2),
  469 (cup3), 522 (cup8), 967 (cup8bench) doorbells per boot; one view per boot at `0x90000`.
- **T5 (signature):** the walked leaf is SYS_COH + kind `0xF` at `0x30000`, `len 0x10000` — classified
  as the user page, exactly as §2 of that doc predicts.
- ⊘ Not run: **T3** (200 sequential / 70 concurrent processes, the 64-view pool) and **T4** (the
  GPU-VA internal-MMIO census; `unmirrored` read 0 in every boot above, but no graph/dynamic-
  parallelism workload was run).

## 6. Measurements (every row names its revision)

| run | rev | result |
|---|---|---|
| bare `bare1` | `74dc3113` | 25/30 (§3) |
| bare `bare3` | `92b9f912` | **30/30** (`traces/v3_blackwell/bare_suite_bare3_92b9f912.out`) |
| guest `bw1 --timer` | `92b9f912` | CRASH — §4 #3 (NULL deref at `RmInitAdapter`) |
| guest `bw3 --timer` | `5036450b` | PASS, 5 s |
| suite `bws1` | `5036450b` | 24/30 (stale guest client ×4, PCIe ×2) |
| suite `bws4` | `bdd80a2d` | 28/30 (uvm ×2: §4 #8, #9) |
| suite `bws6` | `bbd2a083` | **30/30**, 180 s budget, most arms 4–9 s, `ce-client-guest-ram` 30 s, `gpga-reserve-probe` 81 s (`traces/v3_blackwell/thin_guest_suite_bws6_bbd2a083.out`) |
| T1 / negative control | `c2ac8480` | PASS / CRASH (§5) |
| CUDA ladder host + fat guest | `256ec854` | all four PASS both sides (`traces/v3_blackwell/cuda_ladder_gb203_256ec854.out`): guest `cuInit` 572 ms vs host 1158 ms, `cuCtxCreate` 440 vs 74 ms, 2048² matmul 4.91 vs 5.07 ms |
| GA106 bare + thin guest | `ab5ccced` | 30/30 + **30/30** (`traces/v3_blackwell/ga106_thin_guest_suite_ab5ccced.out`) |
| GA106 crate tests + gates | `c2ac8480` | **1474 pass / 0 fail**, gates **9/9** |

The guest's `NVRM` log carries the same pre-existing refusals as GA106 (golden-image channel / kernel
GR = P7 scope, `INTERNAL_INIT_USER_SHARED_DATA`, DECOMP PCE config) plus Blackwell-only, tolerated
ones: `kccuGetBufSize_GB100` (CCU sample info, `0x20800ab2`), `kperfGpuBoostSyncStateInit`
(`0x20800a80`), `NVLINK_GET_NVLINK_DEVICE_INFO` (`0x20800a87` — NOT_SUPPORTED is RM's own *"NVLink is
unavailable"* path).

## 7. What is left

1. **Hopper (GH100) and datacenter Blackwell (GB10x) on hardware.** Everything in §2 is the shared
   FSP path; the die-group rows (`0x200BC` boot gate, `0x4C` config-cycle link caps through
   `_gpuFindPcieRegAddr_GB100`'s capability-list walk — which our bare preset may not satisfy) are
   derived from ogkm and unmeasured. Hopper's PBDMA fault ids are still refused by name (`authored.rs`).
2. **T3 and T4** of the BAR1 doorbell plan (§5).
3. Unserved Blackwell internal controls nothing has needed yet: `SM_ISSUE_RATE_MODIFIER_V2`
   (`0x20800b03`), `SM_ISSUE_THROTTLE_CTRL`, `PPC_MASKS`, `ROP_INFO`, `FECS_TRACE_DEFINES`, CCU sample
   info. Each is tolerated by the guest RM today; the nvdiff method (§4 #10) finds the next one if a
   workload needs it.
4. The fast-scrub conversion (§4 #7) makes PMA scrubs remap memsets on our ring instead of the
   hardware fast scrubber — correct, and slower than it could be.
5. The provisioning purge globs are 575-specific (§1).
