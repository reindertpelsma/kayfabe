# Arch-variance audit — what a second GPU architecture actually costs, from source

**Status:** read-only audit, 2026-08-10. No bench time, no boot. Task #240.
**Scope:** `kayfabe` (`/workspace/nvkvm-rs`) at `99672fe`; ground truth from
`ogkm-580.159.04`. Target for arch #2 is **TU10x**, deliberately — Ampere→Ampere shares a HAL and
proves nearly nothing.

> ### ✔ Verified independently before committing
> 1. **Our VBIOS oracle already compiles TURING source** — `rs:tests/build.rs:39` =
>    `…/gsp/arch/turing/kernel_gsp_vbios_tu102.c`, `:52` = `…/turing/kernel_gsp_frts_tu102.c`. ✔
> 2. **The ucode-load mechanism genuinely splits** — `kernel_gsp_tu102.c:73`
>    `falconConfig.bBootFromHs = NV_FALSE` against `kernel_gsp_ga102.c:60` `= NV_TRUE`. ✔
> 3. **We model no IMEM/DMEM PIO window** — `IMEMC` across `kayfabe-{device,gsp,chips,arch}/src`
>    returns **1 hit** (a mention, not a model). ✔ close enough to the reported zero to carry the
>    conclusion; the discrepancy is one file outside the reported search set.

---

## 0. The ceiling, stated first so it cannot drift

**Arch #2's bar is `cup2`, not parity.** The deliverable is: *a TU10x chip row realizes, the guest's
GSP boots, and one `cup2`-shaped trace crosses the seam.* ⊘ **Explicitly out of scope and out of
budget:** GR (`gr_static`, `gr_info`, `gr_context_buffers`), the application matrix, perf,
multi-process, NVLink/C2C. ★ **If a plan for arch #2 grows a GR section, that plan has stopped being
this rung.**

---

## 1. REFUTED FIRST

- ⊘ **R0 — task #240 was never read.** `TaskGet` does not exist in the auditing session; the work ran
  from the relayed brief only. **Treat any conflict between this page and #240 as #240 winning.**
- ⊘ **R1 — *"kayfabe-chips has one real profile"* is false three ways.** `kayfabe-chips` holds
  **three** `impl Arch` (`lib.rs:62,65,68`) and **three** `HostClasses` (`:69`). What has one row is
  `kayfabe_device::CHIPS` = `&[&ga10x::GA106]` (`kayfabe-device/src/lib.rs:663`). `VBIOS_PROFILES` has
  **two** (`kayfabe-abi/src/vbios.rs:456,506`). **The premise conflated three tables.**
- ⊘⊘ **R2 — *"N=1 evidence"* for *all Turing+* is too pessimistic, and the source says so.** The
  entire GSP **boot skeleton** binds the *same `_TU102` symbol* on TU102 and GA106 — `kgspBootstrap`,
  `kgspExtractVbiosFromRom`, `kgspPrepareForFwsecFrts`, `kgspExecuteFwsec`, `kgspExecuteBooterLoad`,
  `kgspPopulateWprMeta`, `kgspIsWpr2Up`, `kgspWaitForGfwBootOk`, `kgspSetCmdQueueHead`,
  `kgspProgramLibosBootArgsAddr`, `kgspWaitForProcessorSuspend`, `kgspTeardown`, `kgspGetFrtsSize`
  (`ogkm: generated/g_kernel_gsp_nvoc.c:684-1502`). ★★★ **GA10x runs Turing's GSP boot code, and our
  VBIOS/FWSEC oracle already compiles Turing's own files.** ⇒ **The Turing evidence is not zero; it
  is unlabelled.**
- ⊘ **R3 — *"NVIDIA's HAL suffixes ARE the arch-variance list"* is false as stated.** It is a sound
  **superset screen** with a cheap follow-up — but it both over- and under-lists.
  **Over:** `kflcnMaskImemAddr_TU102` / `_GA100` have **byte-identical bodies**; so do
  `intrReadRegTopEnSet_CPU_TU102` / `_CPU_GA102`; so, for us, do
  `kfifoGenerateWorkSubmitTokenHal_TU102` / `_GA100`.
  **Under:** `NV_PRISCV_RISCV_BCR_CTRL` exists at `ampere/ga102/dev_riscv_pri.h:55` and **not at all**
  in the Turing header — a register appearing from nowhere shows up only as a *stub* HAL.
  ⇒ **Right instrument; "enumerate the suffixes" is not "enumerate the variance."**
- ⊘ **R4** — the brief's *"2 of 3 wrong Hopper picks are SERVED"* is **3 of 4** (`gh100.rs:252-287`).
- ⊘ **R5** — the `vh` tree is **not dirty**; nothing was worked around.

---

## 2. Question 0 — what actually exists

| Thing | Where | Verdict |
|---|---|---|
| `Ga10xArch` (GA106) | `chips/src/ga10x.rs:108`; constructed in production at `qemu-raw/src/shim.rs:5794` | **BUILT** |
| `Ad10xArch` (AD106) | `chips/src/ad10x.rs:325` | **BUILT-BUT-UNREACHED** (test-only callers) |
| `Gh100Arch` | `chips/src/gh100.rs` | **BUILT-BUT-UNREACHED** |
| `VBIOS_PROFILES` AD106 row | `abi/src/vbios.rs:506` | **BUILT-BUT-UNREACHED** — no `ChipProfile` keys `0x2803`, so `chip_for_device_id` refuses first |
| `ChipProfile` for anything but GA106 | `device/src/lib.rs:663` | **NEITHER** |
| Any TU10x artefact | — | **NEITHER** |

⇒ **One implementor that ships, two that compile.** Better than N=1, worse than N=3 — the honest
number.

---

## 3. The instrument and the count

Resolved every `// <hal> -- halified` block in `ogkm generated/g_*_nvoc.c` per chip (skipping
`RmVariantHal: VF`) for `TU102 / GA106 / AD106 / GH100`. **164 HALs bind differently on TU102 vs
GA106.** Of those, **17 land on a path this port models**; with adjacent facts the audited set is 24.

**Result: 14 PORTABLE-AS-WRITTEN · 6 NEEDS-A-PROFILE-ROW · 1 NEEDS-A-LOGIC-CRATE-EDIT ·
3 UNKNOWN-FROM-SOURCE**, plus **5 SOURCED-ONLY-TO-A-MOCK** counted separately (§5).

### The rows that are NOT free

| # | HAL | The delta | Verdict |
|---|---|---|---|
| 7 | `kflcnIsRiscvActive` | TU reads `RISCV_CORE_SWITCH_RISCV_STATUS` (`0x240`, bit **0**); GA reads `RISCV_CPUCTL` (`0x388`, bit **7**) | **PROFILE-ROW** (two constants). ⚠ the variant *name* `GspRiscvCpuctl` becomes a lie on TU10x |
| 8 | `kflcnRiscvReadIntrStatus` | `IRQMASK/IRQDEST` `0x2b4/0x2b8` vs `0x528/0x52c` | **PROFILE-ROW** — already anticipated at `ad10x.rs:79-83` |
| 17 | `kgmmuFmtInitLevels` `_GP10X` vs `_GA10X` | **7 of the 8 oracle-driven entry points are the SAME symbol** on TU102 and GA106; the one delta is `_GA10X` = `_GP10X` **plus one statement** (`pLevels[2].bPageTable = NV_TRUE`) | **PROFILE-ROW, small** — drop one `PageSize` and one match arm |
| 22 | `gpuGetEngClassDescriptorList` | TU106 gives `TURING_CHANNEL_GPFIFO_A` `0xc46f`, `TURING_DMA_COPY_A` `0xc5b5`, `TURING_USERMODE_A` `0xc461` | **PROFILE-ROW** — the `HostClasses` trait already exists, so add-alongside |
| 24 | `kmemsysReadUsableFbSize`, `kbifInitXveRegMap` | values, not code | **PROFILE-ROW** |
| 10 | `kgspIsDebugModeEnabled` | `_TU102` vs `_GA100` | **UNKNOWN-FROM-SOURCE** — we neither serve nor refuse the fuse |
| **12** | ★★★ **`kgspConfigureFalcon` / `kgspExecuteHsFalcon` — the ucode-load MECHANISM changes** | TU sets `bBootFromHs = NV_FALSE` and loads FWSEC + Booter by **PIO through `IMEMC/IMEMD/DMEMC/DMEMD`** (`kernel_gsp_falcon_tu102.c:309-362`); GA sets `NV_TRUE` and uses **falcon DMA** (`FBIF_TRANSCFG` + `DMATRFCMD`) | ★ **NEEDS-A-LOGIC-CRATE-EDIT — the one real finding** |

### The rows that ARE free (the null result, and it is the bulk)

Nine falcon register offsets, `NV_PGSP` base + queue heads, `NV_PSEC` base, the GFW-boot pair,
`WPR2_ADDR_LO/HI`, `NV_FALCON2_GSP_BASE` — **all byte-identical between
`turing/tu102/*` and `ampere/ga102/*`**. Plus: **the doorbell encoder** (row 16 — the split HAL is a
false alarm; both bodies are the same two `FLD_SET_DRF_NUM` lines and the field definitions are
identical), **USERD size/align** and **chid decode** (both `_GM107` on TU102 — the oracle flip is a
**no-op, and that is the measurement**), the **usermode window** (`_GV100` on both; Ampere runs
Turing's arm), the **interrupt tree** registers, **LibOS 2 vs 3** (`LibosMemoryRegionInitArgument` is
**not halified**; we scan for `RMARGS` only, so extra/missing log regions are entries we skip), WPR
heap sizing and the ucode blobs (guest-side).

---

## 4. ★★★ The single largest finding — and nobody had named it

**Row 12 is the only item that forces a logic-crate edit, and it is a boot-path mechanism change, not
a value.** TU10x loads FWSEC and the SEC2 Booter through an **IMEM/DMEM PIO window** with an
auto-incrementing address cursor. `GspReg` (`kayfabe-arch/src/gsp.rs:67-198`, 18 variants) names none
of those registers, and we model no such window — we answer `DMATRFCMD ⇒ IDLE` and nothing else.

⊘ **Whether it is *needed* is NOT decidable from source.** The writes might be absorbable the way the
DMA path is absorbed today. ⇒ **This is the single largest unknown in the arch-#2 estimate, and the
one item that would need a bring-up attempt to settle.**

---

## 5. ⚠ The third verdict — SOURCED-ONLY-TO-A-MOCK

`Ad10xArch` and `Gh100Arch` delegate **five of ten** `Arch` methods to a composed `MockArch`
(`ad10x.rs:385-395`, `gh100.rs:733-777`): `mmu()`, `userd()`, `pushbuffer()`, `classify()`,
`is_case2_control()`. `MockGmmuFmt`'s own docs are explicit — *"the geometry is not fake … the **bit
layout** of an entry is invented"* (`kayfabe-mocks/src/lib.rs:700-706`).

★ Four methods were **deliberately un-delegated** for exactly this reason (`vchid_from_userd_flags`,
`decode_doorbell`, `gsp`, `host_classes`) — **that is the pattern to repeat.**
⇒ **Any TU10x arch that composes `MockArch` inherits all five and must be counted as five
SOURCED-ONLY-TO-A-MOCK rows, not five green ones.** (This is live finding #197.)

---

## 6. ⊘ The `VbiosProfile` claim — UNTESTED-BY-CONSTRUCTION

*"One `VbiosProfile` row and zero logic-crate edits"* **cannot be tested by reading**, so no verdict is
given. ★ Nothing in the source forces a logic edit until someone attempts a generation the current
fields cannot express — **a source audit will therefore always find the claim holds, which is a
guaranteed false green.** Delivered instead: §4 plus the list of what *would* force an edit —
(1) the PIO ucode path; (2) renaming `GspReg::GspRiscvCpuctl` (cosmetic but this tree refuses lies);
(3) a `GmmuFmt` with a strict-subset leaf set, **only if** `kayfabe_mmu::walker::leaf_disposition`
assumes the 512 MiB level exists — **unverified, check before costing**; (4) a `ChipProfile` field
TU10x needs that GA106 lacks — the failure mode the claim was written to catch, and not rule-out-able
by reading.

---

## 7. The cost of arch #2 (TU106)

**Rows, one each:** `VbiosProfile` (7 + 16 FWSEC fields) · `ChipProfile` + `CHIPS` entry
(**39 fields**) · `Tu10xGspModel` (≈ `ad10x.rs` with **two constants changed**; it reuses
`FalconSecureBooterBoot` unchanged, because `kgspBootstrap` is `_TU102` on both) · `Tu10xGmmu`
(`Ga10xGmmu` minus one `PageSize`, one match arm) · `Tu10xHostClasses` (3 class ids) · `Tu10xArch`
(~10 methods, five mock-sourced unless written out).

⚠ **The rows source CANNOT fill.** `ce_fault_method_buffer_size` is produced inside GSP firmware,
appears in **no** open source, and is refused to every usermode client; a row reaching realize with
zero is **refused** (`ChipError::NoFaultMethodBufferSize`). `bar1_pde_base`, `fb_regions`, `fb_length`
are board facts. ⇒ **A TU10x `ChipProfile` cannot be completed from `ogkm` alone. That is a hard,
sourced dependency on a physical card, and it belongs at the TOP of the estimate, not the bottom.**

---

## 8. Oracle reach after arch #2

| Oracle | Pin | Judges TU10x? |
|---|---|---|
| `vbios_real_parser_oracle` | none — compiles Turing's own files | ★ **already does** |
| `gmmu_fmt_oracle` | `"GA106"` (`build.rs:215`) | **yes, one string** — `kern_gmmu_fmt_gp10x.c` already whitelisted |
| `worksubmit_token_oracle` | `"GA106"` (`:295`) | **yes, one string** — all four slice anchors present in the `_TU102` body |
| `userd_chid_oracle` | `"GA106"` (`:423`) | **yes — and the flip is a NO-OP**, because both HALs are `_GM107`. That *is* the measurement |
| `pushbuffer_abi_oracle` | `"GA106"` (`:334`) | **partially** — also needs class headers `clc56f.h` → `clc46f.h` |

⊘ **Not covered by any oracle we had WHEN THIS WAS WRITTEN: rows 10 (debug fuse) and 12 (the PIO
ucode load)** — *the two items that actually diverge on the boot path.*

★★★ **That asymmetry is the honest summary of this audit: the oracles cover what does not vary, and
do not cover what does.**

---

### ⊘⊘ CORRECTION, same day (2026-08-10) — "no oracle exists or could" for row 12 is REFUTED, by a tree we already had

The owner asked whether **NVIDIA's Rust driver, `nova`**, might serve as an independent oracle.
**It is already vendored** — `research_clones/linux/drivers/gpu/nova-core/` — and it answers row 12
directly:

- `nova-core/falcon/hal.rs:15-16` — `mod ga102;` / `mod tu102;`: **the exact Turing-vs-Ampere split**.
- `nova-core/falcon/hal.rs:63` — *"The only chipsets supporting PIO are those **< GA102**, and PIO is
  the **preferred** method for…"* ⇒ **an independent statement of row 12's rule**, and it is
  *sharper* than ogkm's HAL split, which only showed that the symbol differed.
- `nova-core/regs.rs:381,392,402,411` — `NV_PFALCON_FALCON_IMEMC / IMEMD / DMEMC / DMEMD` modelled in
  full, with strides (`[4, stride = 16]`, `[8, stride = 8]`) and field definitions.
- `nova-core/regs.rs:494` — *"RISC-V status register for debug (**Turing and GA100 only**)"*,
  independently confirming **row 7**.

⇒ ★★★ **Rows 10 and 12 are NOT beyond oracle reach. They were beyond the reach of the oracles this
audit thought to look at** — and the distinction matters, because the first phrasing closes an
investigation and the second opens one.

★★ **And the deeper correction: our oracles were never independent.** The standing finding is that
**every oracle we own was made by `nvidia-smi`**, and all three share one cause. `nova` is a
**from-scratch reimplementation by different people reading the same hardware** — the first source we
have that can disagree with `ogkm` for a reason other than our own misreading. ⇒ It is an
**anti-overfitting** instrument, which is a different and scarcer thing than a second confirmation.

⚠ **Two limits, stated so this is not oversold:** `nova` covers the **boot/GSP** plane and does not
do compute, so it says **nothing** about our current wall (`cuCtxCreate` GR completion) — it is an
oracle for ground we have already crossed. And it is **GPL-2.0**, where `ogkm` is dual MIT/GPL;
given the standing commitment to public release at Mode-1 parity, **the usage posture must be
decided before anything is derived from it** — read it for *hardware facts*, which are not
copyrightable, not for code.

---

## 9. Verdict

**The `all Turing+` claim is closer to earned than "N=1" suggested** — GA10x runs Turing's boot code
and our VBIOS oracle already compiles Turing's parser. **What it is not earned on is the ucode-load
mechanism (row 12), which nobody had named until now**, and which no oracle can judge.
