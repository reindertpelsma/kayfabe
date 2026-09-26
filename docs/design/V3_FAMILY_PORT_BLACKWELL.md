# V3 FAMILY PORT — Blackwell (GB203, RTX 5080): the first FSP-booted family on hardware

**STATUS: IN PROGRESS, 2026-09-26 (branch `v3-blackwell`, rebased on master `283a5304`).** Filled
in as measured; §0 is the running verdict. Every result names the revision it was measured at.

## 0. Verdict (running)

- **Bare metal, raw client: 30/30** (`92b9f912`, pre-rebase; the client fixes of §3). 25/30 before them.
- **Thin guest:** 24/30 at `5036450b` (`bws1`); the six reds were the stale guest client (§3) and the
  PCIe link word (§4 row 5). See §6 for the current number.

## 1. Box

vast `52730218`, RTX 5080 16 GB (`0x2C02`, **GB203**, `MC_GET_ARCH_INFO` arch `0x1B0` = consumer
Blackwell), VBIOS `98.03.3B.40.17`, Intel i9-14900. The vast KVM image is itself a VM, so the thin
guest is a **nested** KVM guest. Host driver swapped closed 575.51.03 → **580.159.04 open**
(`provision_host_driver.sh`: `OPEN_MODULE=yes`); the guest runs the same `.run` with
`-m=kernel-open`. The provisioning scripts already pick the open flavour on both sides — nothing to
change there (the purge globs name the 575 packages this template ships; a box with other packages
would need them widened).

## 2. The FSP boot — what ogkm says RM does, and what we serve

Read from ogkm-580 and then watched on hardware (`bw1`): the guest writes exactly this sequence.

1. `kfspWaitForSecureBoot_GB202` polls `NV_THERM_I2CS_SCRATCH` (`0xAD00BC` on GB20x, `0x200BC` on
   GH100/GB10x) for `0xFF`. ⇒ a static boot register (`kf_chip::bar0::boot_regs`). ⊘ The FSP model's
   `on_read` answered it, but boot-sequence-only registers are never published to the shadow.
2. `kfspCheckForClockBoostCapability_GB100` (**called from `kgspInitRm` even on a GSP client**) sends an
   `NVDM_TYPE_CAPS_QUERY` packet; the real FSP on this GB203 answers OK (the host driver never prints
   the LEVEL_ERROR *"doesn't have clock boost capability"*), so RM then sends `NVDM_TYPE_CLOCK_BOOST`.
3. The COT (`NVDM_TYPE_COT`, 868 bytes). Its `gspBootArgsSysmemOffset` names the **GSP-FMC's
   `GSP_FMC_BOOT_PARAMS`**, whose `gspRmParams.bootArgsOffset` (+48) is the LibOS array
   (`kgspSetupGspFmcArgs`, `kernel_gsp_gh100.c:460`). ⊘ The FSP model published the FMC params'
   address as if it were the array → `BootSequence::boot_args_indirection` (default `None`, FSP 48).
4. `kfspWaitForGspTargetMaskReleased` (HWCFG2 ≠ 0), `_kgspLockdownReleasedOrFmcError`, MAILBOX0 = 0,
   RISC-V active, status queue, `kgspWaitForRmInitDone` — all the existing FSM, unchanged.

**Transport — `kf_trap::fspemem`.** Every packet goes through the EMEM data port with
auto-increment, and RM asserts the port moved by exactly N after **both** the write burst
(`_kfspWriteToEmem_GH100`) and the read burst (`kfspReadPacket_GH100`), before anything asynchronous
could have run. The page is already a `Hole` on Blackwell (`memmap::holes_for`); `FspEmem` serves its
six registers on the vCPU, lock-free: cursor + flags, the command's two header words, and FSP's reply —
posted at the queue-HEAD write as one single-packet `NVDM_TYPE_FSP_RESPONSE {taskId 0, commandNvdmType,
FSP_OK}` (5 dwords: RM refuses fewer, `kfspProcessCommandResponse_GH100`). The writes still go to the
plane, where the GSP FSM reads the COT from its own window. ⊘ And only a COT that reaches the field
publishes: the window keeps stale bytes, so a second open's CAPS_QUERY would have re-published the
previous life's pointer (`fsp_cot_sequence.rs`).

`[measured bw1, 92b9f912]` `fsp[replies=3 last_nvdm=0x14]` (CAPS_QUERY, CLOCK_BOOST, COT),
`GSP-PUBLISH gpa=0x19e00000`, GspSetSystemInfo → … → RmInitAdapter's RPCs. ★ The FSP boot worked on
the first try once the four pieces above existed.

## 3. The client had four GA10x-isms (bare metal 25/30 → 30/30)

The standing rule (bare-metal FAIL ⇒ client bug) — all fixed in the client, none in kayfabe:

| arm(s) | GA10x assumption | Blackwell (ogkm / measured) | fix |
|---|---|---|---|
| engines, concurrency, dictated-ring, ce-client | the engine writes USERD `GP_GET` back promptly | **Blackwell's USERD has no `GP_GET`**: `Nvc96fControl`/`Nvca6fControl` are `Ignored00[0x23]` then `GPPut` (`clc96f.h:29-33`, `clca6f.h:27-31`); `[measured]` the word stayed 0 for 22 s after the semaphore released | `ChannelClass::userd_has_gp_get` (class < `0xC96F`); `SubmitOutcome.gp_get_by_hw`; the semaphore is the completion on Blackwell. Where GP_GET exists, the probe now also waits for its async write-back within the same budget |
| uvm-mean (P2) | `COPY(2)` is the first async CE | GB203 presents LCEs `{0,1,4,5}` — `COPY(2)` → `NV_ERR_OBJECT_NOT_FOUND` | `first_async_copy_engine` from the host's `CE_GET_ALL_CAPS` (present & !GRCE); GA106 still picks 2 |
| uvm-mean (P2 runlist check) | runlist = `token >> 16` | GB20x sets `RUNLIST_DOORBELL` (bit 30) in every token | `(token >> 16) & 0x7F` (`RUNLIST_ID 22:16`) |
| ce-client arm 6 | a free VA has no page table (`DMA_GET_PDE_INFO`) | VER3's 512 MiB level is `PDE1`, a directory present for every VA within 256 GiB | ignore blocks above 2 MiB (VER2 has none — unchanged) |

## 4. kayfabe: GA10x/Ada facts that broke on Blackwell

| # | where | assumption | Blackwell (ogkm-580 / measured) | fix |
|---|---|---|---|---|
| 1 | `kf-chip bar0::vbios_profile` | FSP families refused (`RowUnbuilt`) — realize failed | FSP families read no VBIOS (`kgspExtractVbiosFromRom_395e98` = NOT_SUPPORTED) | same image for every family |
| 2 | FSP model / publication | — | §2 items 1–3 | as §2 |
| 3 | `kf-abi deviceinfo` | CE fault ids must be one contiguous run | `[measured bw2]` `INTERNAL_GET_DEVICE_INFO_TABLE` refused `CopyEngineFaultIdsNotContiguous {65, 70, count 3}` ×48 → empty engine list → guest NULL deref in `memmgrCalcReservedFbSpaceHal_GM107` (`pKernelGraphics`). GB203's LCEs are floorswept: ids `CE0+i` = `{65, 69, 70}`, the gap is CE2/CE3's own slots | legal when every id is `CE0 + instance` for one `CE0`; duplicates and foreign ids still refused |
| 4 | `kf-rm inittables` | an encoder refusal was silent (`Err(_) => refuse()`) | cost a boot to see #3 | `W349REFUSE why=encoder {e:?}` on all 31 arms |
| 5 | PCIe link capabilities | served only at BAR0 `NV_XVE` `0x88084` | `[measured bws1]` UVM_REGISTER_GPU `0x40` after *"Unknown PCIe speed"*: GH100/GB20x read them through `gpuReadBusConfigCycle` — the **config space** (`0x6C`, a real config cycle) or, in a VM (`bIsPassthru`), the BAR0 **`NV_EP_PCFGM` mirror** `0x92000+0x6C` | both: `bar0::config_words` (kf3 ABI 7 presets config-space dwords, refusing a collision) and the `0x9206C` boot register |

## 5. Measurements

| run | rev | result |
|---|---|---|
| T0 (bare, `nvdiff_shim` + `nvd_prog ce`) | — | libcuda allocates **two** `0xc661` usermode objects: one without params → BAR0 view `0xc0bb0000`; one **with** params → **BAR1 view** `BAR1+0xA0000` ⇒ libcuda sets `bBar1Mapping` on Blackwell (`traces/v3_blackwell/t0_nvdiff_ce_bare_gb203.jsonl.zst`) |
| bare `bare1` | `74dc3113` | 25/30 (§3) |
| bare `bare3` | `92b9f912` | **30/30** |
| guest `bw1 --timer` | `92b9f912` | CRASH — §4 row 3 |
| guest `bw3 --timer` | `5036450b` | PASS, 5 s |
| guest suite `bws1` | `5036450b` | 24/30 (stale guest client ×4, PCIe ×2) |
| guest `bws2` (6 arms) | `31108a89` | 4/6 (PCIe ×2) |
