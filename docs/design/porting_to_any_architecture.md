# Porting kayfabe to ANY GPU architecture — and what Blackwell actually costs

> ### STATUS — 2026-09-13 (w566) / **LIVE — analysis only, no code changed, no hardware run.**
>
> **Scope.** The owner's maintainability contract, verbatim in intent: *maintainable = one
> hand-written section per big architecture family (Turing, Ampere, Ada, Hopper, Blackwell);
> NOT maintainable = per-GPU-model tables or per-driver-version tables beyond broad ranges.*
> This note classifies every value the emulated device needs into the four permitted sources
> (A) ogkm-derived, (B) unprivileged runtime, (C) satisfies-ogkm/computed, (D) per-family
> hand-written — and then costs a GB202 port.
>
> ⊘ **Every claim here is a source read** of `/workspace/kf-master` at `89439595`, of
> `research_clones/ogkm-580.159.04` (v580.159.04) and `research_clones/ogkm` (v610.43.02).
> Nothing was booted. Claims that could not be sourced are marked **UNVERIFIED** with the
> measurement that would settle them, and §8 is the list that needs a real Blackwell board.
>
> **Parent doc.** `support_matrix_seam_audit.md` (STATUS LIVE, 2026-08-11) owns the *seam*
> question — which traits exist and which are answered by a mock. This note does not restate
> it; it answers the *source* question the seam audit does not: where each value may legally
> come from. Two of its §2 findings are **independently corroborated** here (the four GMMU
> families; the Blackwell work-submit token adding a bit outside our mask) and are cited, not
> re-derived. One is **extended**: §2.5's *"doorbell reg offset `0x90` fixed Volta→Blackwell"* is
> true, and §3.4 gives a stronger reason than a header diff. One is **scoped**: its *"refused at
> GB"* holds for GB202 and **not** for GB100, where the wrong answer is the quiet one (§5.4).

---

## 1. The rule that makes source (A) mechanical

### 1.1 ★★★ The published headers are SPARSE and INHERITED — a chip directory is not a chip

`ls` of the vendored `src/common/inc/swref/published/<arch>/<chip>/` directories:

| chip dir | header count |
|---|---|
| `turing/tu102` | 36 |
| `ampere/ga102` | 18 |
| `ada/ad102` | **8** |
| `hopper/gh100` | 32 |
| `blackwell/gb100` | 40 |
| `blackwell/gb202` | **18** |
| `blackwell/gb20b` | 13 |

`ampere/ga102/` has no `dev_timer.h`, no `dev_bus.h`, no `dev_mmu.h`, no `dev_fb.h`.
`blackwell/gb202/` has no `dev_gsp.h`, no `dev_falcon_v4.h`, no `dev_gc6_island*.h`, no
`dev_hubmmu_base.h`, and its `dev_boot_zb.h` carries **no `NV_PMC_BOOT_*` at all** (only
`NV_SYSCTRL_SEC_FAULT_*`). **A directory holds only what that chip CHANGED.**

⊘ So *"does the value exist in the vendored gb202 headers?"* is the **wrong question** and
answering it literally produces a false "missing" for almost everything. The right question is
**which header does the code path GB202 dispatches to include?** — and that is answerable
mechanically, because NVIDIA's HALs are ordinary C functions compiled once:

> `kfspSendPacket_GB100` (`ogkm-610: src/nvidia/src/kernel/gpu/fsp/arch/blackwell/kern_fsp_gb100.c:319-337`)
> falls through to `kfspSendPacket_GH100` unless an opt-in registry property is set. That callee
> is compiled in `kern_fsp_gh100.c`, whose includes are `published/hopper/gh100/dev_fsp_pri.h`
> (`ogkm-610: kern_fsp_gh100.c:42`). ⇒ **a default GB202 guest drives the FSP through the
> HOPPER EMEM offsets**, `NV_PFSP_EMEMC = 0x008F2ac0` / `NV_PFSP_EMEMD = 0x008F2ac4`
> (`ogkm-610: src/common/inc/swref/published/hopper/gh100/dev_fsp_pri.h:26,40`) — even though
> `blackwell/gb202/dev_fsp_pri.h` defines neither (it holds only two scratch groups, `:27-35`).

**The derivation rule, stated once:** the register offset a chip uses is the offset in the header
included by the file that *implements* the HAL arm that chip dispatches to — never the offset in
the chip's own directory, which may be empty of it.

### 1.2 ★★★ `g_*_nvoc.c` IS the machine-readable statement of "which chips share an implementation"

NVOC's generated dispatch tables carry, in a comment on every arm, the exact chip list:

```
ogkm-610: src/nvidia/generated/g_kernel_gsp_nvoc.c:731-748
    // kgspBootstrap -- halified (4 hals) body
    ... /* ChipHal: TU102 | TU104 | TU106 | TU116 | TU117 | GA100 | GA102 | GA103 | GA104
                  | GA106 | GA107 | AD102 | AD103 | AD104 | AD106 | AD107 */
            pThis->__kgspBootstrap__ = &kgspBootstrap_TU102;
    else    pThis->__kgspBootstrap__ = &kgspBootstrap_GH100;
```

That is a **derived universe, not a list somebody maintains** — the exact shape
`gates_quantified_over_a_list` asks for. `[measured 2026-09-13, w566]` parsing every
`g_*_nvoc.c` in `ogkm-610/src/nvidia/generated/` (script:
`/tmp/.../scratchpad/haldelta.py`, 1290 halified functions resolved):

| comparison | halified functions whose implementation DIFFERS |
|---|---|
| GA106 vs **AD102** | **21** of 1290 (1.6 %) |
| GA106 vs GH100 | 381 (29.5 %) |
| GA106 vs **GB202** | **425** (33.0 %) |
| **GH100 vs GB202** | **237** (18.4 %) |
| GB100 vs GB202 | 198 (15.3 %) |

★ And the number that actually sizes a port — functions where GB202 differs from **both**
GA106 **and** GH100, i.e. genuinely new work no existing kayfabe arch already covers:
**179 of 1290 (13.9 %)**, concentrated:

| subsystem (`g_*_nvoc.c`) | new-for-Blackwell HALs | does it touch our surface? |
|---|---|---|
| `gpu` | 21 | partly |
| `kernel_gsp` | 17 | **yes** |
| `kernel_fifo` | 16 | **yes** |
| `conf_compute` | 14 | no — CC plane declared off |
| `kernel_bif` | 14 | no — PCIe link management |
| `kern_fsp` | 13 | **yes** |
| `kernel_ce` | 13 | partly |
| `kern_gmmu` | 10 | **yes** |
| `kernel_nvlink` | 8 | no — fabric |
| `mem_mgr` | 8 | partly |
| `kern_mem_sys` | 7 | partly |
| `kern_bus` | 6 | **yes** |
| `kernel_falcon` | 5 | no |
| `kernel_mig_manager` | 5 | no — MIG |
| `intr` | 4 | no — all four are **display** |
| `spdm` | 4 | no — CC plane |
| `kernel_graphics` | 3 | partly |
| `objtmr` | 3 | **yes** |
| `kernel_mc` | 2 | **yes** |
| `kern_hwpm` | 1 | no |

⊘ **This is an upper bound on the driver's divergence, not on OUR work.** Most of those 179
are in subsystems an emulated GSP-client device never reaches: `conf_compute` (14) and `spdm`
(4) are the Confidential-Compute plane this port declares off (`ConfComputeRow`, all-`false`),
`kernel_nvlink` (8) and `kernel_mig_manager` (5) are fabric/MIG, `kernel_bif` (14) is PCIe
link management. The ones that touch our surface are enumerated in §4 and number **fewer than
twenty**.

⚠ **Caveat on the instrument.** The parser resolves each halified block by walking its arms in
order and taking the first whose `ChipHal:` comment names the chip, falling back to the block's
last assignment as the `else`. It does not model the `RmVariantHal` (VF/PF) nesting, so a
function whose only divergence is VF-vs-PF is scored as identical for both chips — which is
correct for our purpose (we emulate a PF) but means the counts are *"differs on the chip
axis"*, not *"differs at all"*. Re-run the script to reproduce; it takes ~4 s and reads only
the vendored tree.
---

## 2. The four sources, and what each one can actually reach

### 2.1 (A) — derived from ogkm source

Reachable, with the §1.1 rule applied. ★ The precedent is built: `crates/kayfabe-abi/gen/`
mechanically derives 6 files / ~6540 lines from ogkm with a hand-rolled scanner, zero
dependencies, and refuses everything it cannot understand rather than guessing
(`crates/kayfabe-abi/gen/src/parse.rs:1-8`). It stamps the tree's own version into the output
(`crates/kayfabe-abi/src/generated/mod.rs:20` = `"610.43.02"`, read from `version.mk` at
`gen/src/main.rs:1515-1523`).

`[measured 2026-09-13]` over `research_clones/ogkm/src/common/inc/swref/published/ampere/ga102/`
— 18 headers, 299 `#define`s:

| `#define` shape | count | handled by the generator today? |
|---|---|---|
| `NAME 0x00110040 /* RW-4R */` (register offset) | 167 | ✅ `scan_defines`, `parse.rs:395` |
| `NAME 31:0 /* RWIVF */` (bit field) | 79 | ✅ `scan_drf_ranges`, `parse.rs:472` |
| `NAME(i) (0x110804+(i)*4)` (indexed) | 28 | ❌ dropped — `parse.rs:405` skips function-like macros |
| `NV_PGSP 0x113fff:0x110000` (aperture) | 2 | ❌ dropped by both scanners |
| guards / aliases / brace-lists | ~23 | ❌ |

⇒ **246/299 (82 %) of a real chip directory parses today with zero new parser code.** The four
genuinely new pieces are enumerated in §7.

⚠ **One trap the register use hits that the struct use does not.** `strip_comments`
(`parse.rs:72`) deletes `/* RW-4R */` before any scanner sees the line — and that trailing tag
is the *only* discriminator between `NAME 0x00000001` (an enum value) and `NAME 0x00110040`
(an offset). A register extractor must keep the tag or re-read the raw line.

⊘ **And a second, larger reach nobody has used.** `ogkm-610:
src/nvidia/generated/g_nv_name_released.h` is a **1927-row** `{devID, subSystemID,
subSystemVendorID, name}` table. `GPU_GET_NAME_STRING` returning 23 zero bytes — the reason
`nvidia-smi` prints `ERR!` in the Name column (`CLAUDE.md`, the nvdiff findings) — is a
generator away, and the table is **NVIDIA's, not ours**: it costs nothing per model and it
expires exactly when ogkm is re-vendored. Same for
`ogkm-610: src/nvidia/generated/g_gpu_class_list.c:2882-2986`, which states GB202's complete
class-descriptor list (`BLACKWELL_CHANNEL_GPFIFO_B`, `BLACKWELL_USERMODE_A`,
`BLACKWELL_DMA_COPY_B`, `BLACKWELL_COMPUTE_B`, `BLACKWELL_INLINE_TO_MEMORY_A`, and the
backward-compatible `AMPERE_CHANNEL_GPFIFO_A`/`AMPERE_USERMODE_A`), per chip, mechanically.

### 2.2 (B) — unprivileged runtime, and it reaches MUCH further than this tree assumes

`[source-derived 2026-09-13 from the Linux tree vendored at
`research_clones/linux` (7.1.0-rc6, `Makefile:1-4`); NOT run against a GPU — `ls /dev/nvidia*`
on this box is empty, though 8 non-NVIDIA PCI devices are present and every mode below comes
from the attribute definition, not the driver]`

**sysfs, world-readable 0444** (`research_clones/linux/drivers/pci/pci-sysfs.c`; `DEVICE_ATTR_RO` = 0444 via
`include/linux/sysfs.h:260-261`): `vendor`, `device`, `subsystem_vendor`, `subsystem_device`,
`revision`, `class` (all `pci-sysfs.c:52-59`), `resource` (`:196` — every BAR's start/end/flags,
so **BAR sizes with no hardware touch**), `irq` (`:78`), `current_link_speed` (`:242`),
`current_link_width` (`:260`), `max_link_speed` (`:206`), `max_link_width` (`:221`),
`numa_node` read (`:397`).

⊘ **The two exclusions this analysis was handed are CONFIRMED from source, not assumed:**
- `resource0` and every BAR mmap — `res_attr->attr.mode = 0600` (`pci-sysfs.c:1247`),
  unconditional.
- the expansion ROM — `BIN_ATTR(rom, 0600, …)` (`pci-sysfs.c:1367`).
- and config space is **64 bytes** to a non-root reader: `unsigned int size = 64;`
  (`pci-sysfs.c:718`) widened to `dev->cfg_size` only under
  `file_ns_capable(filp, &init_user_ns, CAP_SYS_ADMIN)` (`:723`). 64 bytes is exactly the
  Type-0 header — no PCIe capability, no extended space.

**NVIDIA's own procfs, 0444** — `/proc/driver/nvidia/gpus/<bdf>/information` is created
`S_IFREG | S_IRUGO` (`ogkm-580: kernel-open/common/inc/nv-procfs-utils.h:66-75`, created at
`kernel-open/nvidia/nv-procfs.c:1404`) and carries model name, GPU UUID, **VBIOS version** and
**GSP firmware version** (`nv-procfs.c:102-189`).

**RM controls over `/dev/nvidiactl`, default mode 0666** (`ogkm-580:
src/nvidia/arch/nvalloc/unix/include/nv-reg.h:146-157`). The privilege rule, stated once
because it is counter-intuitive:

> `RMCTRL_FLAGS_KERNEL_PRIVILEGED == 0x0` (`ogkm-580:
> src/nvidia/inc/kernel/rmapi/control.h:178`). **Kernel-only is the DEFAULT, inferred from the
> ABSENCE of a bit.** A control carrying none of `NON_PRIVILEGED | PRIVILEGED | INTERNAL` is
> refused to every usermode client **including root** (`ogkm-580:
> src/nvidia/src/kernel/rmapi/control.c:702-709`), and an `INTERNAL` control is unreachable
> from a device node at all (`control.c:799-804` with `bInternal` false for `RMAPI_EXTERNAL`,
> `control.c:385`).

`[measured]` over the NVOC-generated `__nvoc_exported_method_def_*` tables in `ogkm-580`: **1349
exported controls, 758 (56.2 %) carry `NON_PRIVILEGED`.** Of the sixteen this port would want:

| control | cmd | flags (file:line) | verdict |
|---|---|---|---|
| `GPU_GET_INFO_V2` | `0x20800102` | `0x30118` `g_subdevice_nvoc.c:151` | **UNPRIV-OK** |
| `GPU_GET_NAME_STRING` | `0x20800110` | `0x2010a` `:166` | **UNPRIV-OK** |
| `GR_GET_INFO` | `0x20801201` | `0x118` `:5206` | **UNPRIV-OK** |
| `GR_GET_GPC_MASK` | `0x2080122a` | `0x10118` `:5596` | **UNPRIV-OK** |
| `GR_GET_TPC_MASK` | `0x2080122b` | `0x10118` `:5611` | **UNPRIV-OK** |
| `GR_GET_SM_ISSUE_RATE_MODIFIER` | `0x20801230` | `0x10008` `:5656` | **UNPRIV-OK** |
| `CE_GET_CAPS_V2` | `0x20802a03` | `0x10108` `:7606` | **UNPRIV-OK** |
| `CE_GET_CE_PCE_MASK` | `0x20802a02` | `0x30349` `:7591` | **UNPRIV-OK** (routing ≠ privilege) |
| `FB_GET_INFO_V2` | `0x20801303` | `0x10118` `:5851` | **UNPRIV-OK** |
| `BUS_GET_PCI_BAR_INFO` | `0x20801803` | `0x10518` `:6496` | **UNPRIV-OK** |
| `MC_GET_ARCH_INFO` | `0x20801701` | `0x1050b` `:6376` | **UNPRIV-OK** |
| `GPU_GET_ENGINES_V2` | `0x20800170` | `0x10109` `:886` | **UNPRIV-OK** |
| `BUS_GET_INFO_V2` | `0x20801823` | `0x10118` `:6706` | **UNPRIV-OK** |
| `NV0080 GR_GET_INFO` | `0x00801104` | `0x108` `g_device_nvoc.c:438` | **UNPRIV-OK** |
| `FIFO_GET_DEVICE_INFO_TABLE` | `0x20801112` | `0x5c040` `:4996` | ⊘ **kernel-only — refused to root** |
| `CE_GET_FAULT_METHOD_BUFFER_SIZE` | `0x20802a08` | `0x1c040` `:7666` | ⊘ **kernel-only — refused to root** |
| `INTERNAL_GET_DEVICE_INFO_TABLE` | `0x20800a40` | `0x1c4c0` `:2386` | ⊘ **INTERNAL — no device node reaches it** |
| `INTERNAL_GPU_GET_CHIP_INFO` | `0x20800a36` | `0x404c0` `:2251` | ⊘ **INTERNAL** |

★★★ **The headline, and it retires this tree's worst rot.** `NV2080_CTRL_CMD_GR_GET_INFO`
(`0x20801201`) is `NON_PRIVILEGED` and carries the **entire chip floorplan** —
`LITTER_NUM_GPCS`, `NUM_TPC_PER_GPC`, `NUM_SM_PER_TPC`, `NUM_FBPS`, `NUM_FBPAS`, `NUM_LTCS`,
`NUM_LTC_SLICES`, `NUM_PES_PER_GPC`, `NUM_ROP_PER_GPC`, … (`ogkm-580:
src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080gr.h:224-268`). `GR_GET_GPC_MASK` /
`GR_GET_TPC_MASK` give the floorsweep. `MC_GET_ARCH_INFO` gives
`{architecture, implementation, revision, subRevision}` outright
(`ctrl2080mc.h:65-70`). ⇒ **`gr_static` and `gr_info` — the two most model-specific rows in
`ga10x.rs` — are (B)-obtainable and need no per-model table at all.**

⊘ **What (B) provably CANNOT reach**, and the corroboration is that these are exactly the four
the C artifact had to snoop out of GSP RPC: `0x20801112`, `0x20802a08`, `0x20800a40`,
`0x20800a36`. Kayfabe has already **measured** this on real silicon —
`crates/kayfabe-isolate-host/src/bin/rmladder.rs:559-568`: *"`[measured]` 2026-08-01: for
`0x20802a08` on an RTX 3060 that is exactly what happens, so this rung did not supply the number
in `kayfabe_abi::fmbsize` — an instrumented build of the driver did."*

⚠ **A caveat before anyone calls today's isolate a source-(B) client.** `surrender_privilege`
drops **capabilities, not uid** (`crates/kayfabe-isolate-host/src/rm.rs:199-202`;
`crates/kayfabe-linux-raw/src/sandbox_unsafe.rs:471-530`). On a root VMM the host kernel still
sees euid 0. `capable(CAP_SYS_ADMIN)` is false — which is what gates RM — but the claim
*"works as a non-root user"* is **UNVERIFIED**; it would be verified by running `rmladder`
under a genuinely different uid and re-reading the same sixteen rows.

⚠ **And today the tree reads NO PCI sysfs for chip identity.** Three hits workspace-wide, none
load-bearing, and one of them is a prose note naming this exact gap:
`crates/kayfabe-abi/src/businfo.rs:72-79` — *"The truthful upgrade is to read the host device's
`current_link_speed` / `max_link_speed` — **world-readable sysfs, no privilege, no RM
ioctl**."* The chip is selected by a **QEMU device property**, not a probe
(`crates/kayfabe-qemu-raw/src/shim.rs:2537`, `:13759`).

### 2.3 (C) — a value that merely satisfies ogkm, or is computed

The largest class, and the one that makes the maintainability contract achievable. The two
worked precedents already in the tree:

- `NV_PTIMER_TIME_PRIV_LEVEL_MASK` served all-ones so `tmrSetCurrentTime_GV100` does not trip
  its assert (`crates/kayfabe-device/src/ga10x.rs:641-701`, the `0x9430` boot reg).
- The **entire VBIOS is synthetic**. `crates/kayfabe-abi/src/vbios.rs:451-500`: *"the FWSEC
  geometry below is a *generated* geometry that satisfies the driver's inequalities, NOT a
  transcription of any real card's ROM"*, and `:470-480` records that every DMEM offset is
  *"CHOSEN, not read off any card: the driver dictates the inequalities and nothing else."*
  The IFR/ROM-directory prologue is skipped entirely by putting the signature at offset 0
  (`vbios.rs:80-89`) — *"being synthetic is strictly simpler than being a copy."*

⚠ **The line (C) must not cross, and the tree has the scar.** Distinguish values the driver
**validates** from values it **computes with**. `ce_fault_method_buffer_size` is the named
example: a captured `0` against a real `20480` was *"a buffer overrun with a hardware writer"*
(`CLAUDE.md`), and the field's own doc records the refusal that resulted
(`crates/kayfabe-device/src/lib.rs:300-314`, `ChipError::NoFaultMethodBufferSize`).

### 2.4 (D) — per-architecture-family hand-written

The budget. Every use is justified individually in §3/§4. The rule this note applies: **(D) is
legitimate only where the value varies BETWEEN families and is CONSTANT WITHIN one.** A value
that varies within a family (floorsweeping, board memory size, subsystem ids) is not (D) — it
is (B) or (C), and putting it in (D) is how a family row silently becomes a model row.
---

## 3. `ChipProfile`, field by field

`crates/kayfabe-device/src/lib.rs:239-620`; the only instance is `GA106`
(`crates/kayfabe-device/src/ga10x.rs:1686-1764`). 39 fields. Columns: **want** = the source
this note argues it *should* come from; **is** = where it comes from today; **within-family?**
= does it vary between two parts of one architecture family; **GB202** = what a Blackwell row
needs.

| # | field | what it is | **want** | is today | varies within family? | GB202 |
|---|---|---|---|---|---|---|
| 1 | `name` | diagnostics string, never branched on | — | `ga10x.rs:1689` `"GA106"` | n/a | free |
| 2 | `has_c2c` | does the part have an NVIDIA C2C fabric | **(B)** | (D) `:1688` `false`, uncited | **YES** — GB202 has none, GB100/GB110 do | `false`; ⊘ a GB100 row needs the opposite and the field already refuses rather than guesses (`lib.rs:248-251`) |
| 3 | `pci_device_id` | the key into `CHIPS` and `VBIOS_PROFILES` | **(B)** sysfs `device`, `pci-sysfs.c:52,55` | (D) `:1692` `0x2504`, cited to the **C artifact**, not silicon | **YES — it IS the model** | one per SKU; must come from the host, never a table |
| 4 | `pci_revision` | PCI revision / silicon stepping | **(B)** sysfs `revision`, `:52,58` | (D) `:1693` `0xA1` | **YES — per stepping** | from the host |
| 5 | `pci_subsystem_vendor_id` | AIB vendor | **(B)** sysfs `subsystem_vendor`, `:52,56` | (D) `:1696` `0x1462` (MSI), *"the host subsystem id the C's dumped ROM's PCIR block carried"* | **YES — per board** | from the host |
| 6 | `pci_subsystem_id` | AIB board id | **(B)** sysfs `subsystem_device`, `:52,57` | (D) `:1697` `0x397D` | **YES — per board** | from the host |
| 7 | `regs_aperture_len` | BAR0 length | **(B)** sysfs `resource` row 0, `:196` — or **(C)**, it is our aperture | (D) `:725` 16 MiB | no | 16 MiB holds; `NV_PMC0_PRI_BASE 0x0` and `NV_PGRAPH_BASE 0x400000` unchanged (`ogkm-610: blackwell/gb100/hwproject.h:27,37`) |
| 8 | `boot_regs` | 7 silicon-constant registers | **mixed — see §3.1** | `:641-701` | mixed | 4 of 7 move; §3.1 |
| 9 | `ptimer` | where the ns counter is read | **(A)** for TU→AD; **★(C) for GB** | (A) `:710-713` `0xBB0080/84`, cited `ogkm-580: timer_tu102.c:130-155` | no | ★★★ **we CHOOSE it.** `tmrGetTmrBaseAddr_GB100` returns `pEntry->devicePriBase` from the **device-info table** (`ogkm-610: src/nvidia/src/kernel/gpu/timer/arch/blackwell/timer_gb100.c:44-61`) — which on a GSP client we author (§3.2) |
| 10 | `rom_window` | BAR0 VBIOS aperture | (A) | (A) `:719` `0x300000`/1 MiB, `ogkm-580: turing/tu102/dev_ext_devices.h:27` | no | ⊘ **NOT NEEDED.** `kgspExtractVbiosFromRom` is `_395e98` = `return NV_ERR_NOT_SUPPORTED` for GH100 and GB202 (`ogkm-610: g_kernel_gsp_nvoc.h:1997-1999`) |
| 11 | `pramin_window` | BAR0 framebuffer window | (A) | (A) `:198` `0x700000`/1 MiB, `ogkm-580: turing/tu102/dev_ram.h:26` | no | present; the *positioning register* moves — row 12 |
| 12 | `bar0_window_reg` | the register that positions PRAMIN | (A) | (A) `:207` `0x1700` = `NV_PBUS_BAR0_WINDOW`, `ogkm-580: maxwell/gm107/dev_bus.h:43` | no | ★ **moves.** Blackwell reads/writes `NV_XAL_EP_ZB_BAR0_WINDOW_BASE` through the XAL aperture (`ogkm-610: src/nvidia/src/kernel/gpu/bus/arch/blackwell/kern_bus_gb100.c:60-110`); GB202's own `dev_nv_xal_ep_zb.h` is vendored |
| 13 | `vbios_wire` | which VBIOS parse path | (A) | (A) `:1710` `Tu102Bit` | no | ⊘ **unused on GB** — see row 10 |
| 14 | `msix_vectors` | MSI-X vector count we offer | **(C)** — our device's choice, not a chip fact | (D) `:730` `8`, cited to the C artifact's *reasoning* | n/a | free |
| 15 | `ce_fault_method_buffer_size` | CE fault method buffer bytes | **(C) with a stated boundary** — see §3.3 | (D) `abi/fmbsize.rs:111` `20480`, `[measured]` on one GA106 with an instrumented driver | UNVERIFIED | ⊘ **the one row (B) cannot supply** — `0x20802a08` is kernel-privileged (`g_subdevice_nvoc.c:7666`, flags `0x1c040`) |
| 16 | `gsp_model` | ctor for the generation's register map | **(D)** — legitimate | (D) `:1718` | no | new: §5 |
| 17 | `engines` | the engines we advertise | **(C)** — a promise we author | capture of one board, 6 rows / 146 lines (`:779-924`), incl. **acknowledged uninitialised capture noise** | n/a — it is ours | author it; CE count from `GPU_GET_ENGINES_V2` (B) if mirroring a host |
| 18 | `lce_pce_masks` | PCE→LCE backing per logical CE | **(C)** self-consistent with 17, or **(B)** `0x20802a02` | `[measured]` one part; the field's own doc says *"a per-part fact"* (`lib.rs:334-336`) | **YES — floorswept** | author, or read (B) |
| 19 | `intr_table` | `MC_ENGINE_IDX` → vector | **(C)/(D)** — we author vectors | capture, 24 rows / 146 lines (`:946-1091`) | n/a — ours | author; 4 `intr` HALs differ at GB (`intrCacheDispIntrVectors_GB202` …), all **display** |
| 20 | `intr_subtree_map` | `subtreeMap[]` | (A)/(D) | `:1101`, two oracles agree | no | likely unchanged; **UNVERIFIED** for GB |
| 21 | `fb_regions` | the FB regions we promise | **(C)** — computed from 38 | capture of a 12 GiB board | **YES — board memory size** | computed |
| 22 | `pci_bars` | BAR sizes in RM's index order | **(C)** (ours) or **(B)** sysfs `resource` | `:1274-1291`, BAR1 = *"a real RTX 3060's BAR1 with resizable-BAR off"* | **YES — ReBAR setting** | ours |
| 23 | `chip_info` | `chip_sub_rev`, `is_cmp_sku`, named reg bases | **(B)** `MC_GET_ARCH_INFO` `0x20801701` gives `subRevision` (`ctrl2080mc.h:65-70`) | capture `:1401-1405` | **YES — stepping** | (B); the USERMODE base `0xBB0000` holds — §3.4 |
| 24 | `user_register_access_map` | what userspace may `EXEC_REG_OPS` | **(C)** policy | `NOT_PUBLISHED` `:1444`, `ogkm-580: gpu_register_access_map.c:261-267` | n/a | unchanged |
| 25 | `constructed_falcons` | which falcons RM builds | **(C)** policy | `NONE` `:1488` | n/a | unchanged |
| 26 | `memory_system` | L2 geometry, comptag page, RAM type | **(C) constrained by (A)** — §3.3 | capture; the file itself records the capture and the live part **disagree** (real `LTS_COUNT` 18, not 24) | **YES — floorswept** | `kmemsysIsPagePLCable_GA102` branches on `ltsPerLtcCount * ltcCount` ∈ {48, 40, 32, 24} (`ogkm-580: kern_mem_sys_ga102.c:66-120`) ⇒ pick a product the chip's own arm recognises |
| 27 | `device_info` | each engine's PRI base | **(C)** — ★ load-bearing on GB | capture `:1580-1607`, 6 rows | n/a — ours | ★★★ §3.2 |
| 28 | `conf_compute` | two CC booleans | **(D)** `false,false` | derived from a **`dlen=0`** capture row (`0x20800af3`) | no | `false,false`; GB202 adds 14 CC HALs we never reach |
| 29 | `bif_static` | four BIF booleans | **(D)** all `false` | derived from a **`dlen=0`** row (`0x20800aac`); ⚠ its `bPcieGen4Capable=false` **contradicts** field 37 in the same literal | no | all `false` |
| 30 | `fifo_channels` | channels per runlist | **(C)** bounded by (A) | capture, `0x0800` | UNVERIFIED | ours; `kfifoRunlistGetBaseShift_GB202` differs (`g_kernel_fifo_nvoc.c`) |
| 31 | `gmmu_static` | two fault-buffer sizes | **(C)** — multiple of `NVC369_BUF_SIZE` (A) | capture, full body | UNVERIFIED (may scale with GPC count) | ours; `kgmmuSetAndGetDefaultFaultBufferSize` is a **stub** on GB202 (`_13cd8d`) where GA106 has `_TU102` |
| 32 | `gr_static` | GPC/TPC/SM geometry + FECS record | ★★★ **(B)** `GR_GET_INFO` + `GR_GET_GPC_MASK` + `GR_GET_TPC_MASK`, all `NON_PRIVILEGED` | **the worst rot** — `tpc_mask 0x1e/0x1f/0x1f` is one physical die's floorsweep (`abi/grstatic.rs:486-494`) | **YES — per die** | (B) |
| 33 | `gr_info` | GR's 58 `(index,data)` legacy pairs | ★★★ **(B)** same controls | capture; **mixes** the floorswept part with family bounds in one flat array (`abi/grinfo.rs:297-358`) | **YES for 6 of 58** | (B) |
| 34 | `gr_context_buffers` | 26 context-buffer sizes | **(B)** `GR_GET_CTX_BUFFER_SIZE 0x20801218` / `GR_GET_ATTRIBUTE_BUFFER_SIZE 0x2080121e`, both `NON_PRIVILEGED` | capture, full body (`abi/grstatic.rs:790-817`) | **YES — scales with TPC/SM** | (B) |
| 35 | `forwarded_gpu_info` | the `GPU_GET_INFO_V2` indices GSP answers | **(C)** + refusal | 1 row; `0x23`/`0x24` measured to **differ between two physical GA106s** and refused by name | n/a | unchanged |
| 36 | `smc_mode` | MIG mode | **(D)** | `Unsupported`, `[measured]` two parts | no (GeForce) | `Unsupported` for GB202 |
| 37 | `pcie_max_gen` | the DIE's max PCIe generation | **(B)** sysfs `max_link_speed` 0444 (`pci-sysfs.c:206`) — and `businfo.rs:72-79` already says so | (D) `Gen4` | no | Gen5; **(B) removes the need to know** |
| 38 | `fb_length` | framebuffer bytes | **(C)** — it is the size WE present | (D) 12 GiB, *"the only free variable in the WPR2 layout"* | **YES — board** | ours |
| 39 | `bar1_pde_base` | where BAR1's root PD lives, as an FB address | **(C)** computed from 38 + the carve-out | `0x2_F1CA_C000`, a real board's byte, with 3 `const assert!`s pinning containment | **YES — follows 38** | computed |

### 3.1 `boot_regs` — the 7 rows, individually

| off | register | source today | **want** | GB202 |
|---|---|---|---|---|
| `0x0` | `NV_PMC_BOOT_0` = `0x176000A1` | (D), C artifact | **(B)+(A)** — `MC_GET_ARCH_INFO` gives arch/impl/rev; the field layout is `ogkm-610: src/common/inc/swref/published/nv_ref.h:171` | ★ **the register MOVES**: `kmcReadPmcBoot0_GB100` reads `NV_PMC0_PRI_BASE + NV_PMC_ZB_BOOT_0` (`ogkm-610: src/nvidia/src/kernel/gpu/mc/arch/blackwell/kernel_mc_gb100.c:57-64`). `NV_PMC0_PRI_BASE = 0x0` (`blackwell/gb202/hwproject.h:27`), so the *value* of the base is unchanged — the **define** is not |
| `0x4` | `NV_PMC_BOOT_1` = 0 | (C) `VGPU = REAL` | (C) | unchanged |
| `0xA00` | `NV_PMC_BOOT_42` = `0x176A1000` | (D) | **(B)+(A)** — `NV_PMC_BOOT_42_ARCHITECTURE` is `29:24`; `GB100 = 0x1A`, `GB200 = 0x1B` (`ogkm-610: nv_ref.h:171,183-184`) | encode `0x1B` for a GB2xx part |
| `0x9430` | PTIMER PLM = all-ones | **(C)** — the canonical example | (C) | same trick; base still `NV_PTIMER0_PRI_BASE = 0x9000` (`blackwell/gb100/hwproject.h:38`) |
| `0x1183A4` | `NV_USABLE_FB_SIZE_IN_MB` | (D) = board size | **(C)** — derived from field 38 | ⚠ address is a GC6-AON alias (`ogkm-580: ampere/ga102/dev_gc6_island_addendum.h:33`); `blackwell/gb100/dev_gc6_island_addendum.h` defines only `..._MMU_LOCAL_MEMORY_RANGE`. **UNVERIFIED for GB — needs the Blackwell `kmemsysReadUsableFbSize` HAL read** |
| `0x88084` | `NV_XVE_LINK_CAPABILITIES` | (C) derived from field 37 | **(C)** | derive from Gen5 |
| `0xB83110` | access-counter buffer size = 256 | ⊘ **self-declared `[ADVERTISED FICTION]`** — *"no reading of `0xB83110` off a real GA106 exists anywhere in this tree"* | **(C)**, and it already is one, honestly labelled | same |

### 3.2 ★★★★★ The single most load-bearing Blackwell finding: the driver ASKS US where the registers are

`tmrGetTmrBaseAddr_GB100` does not return a constant. It returns
`pEntry->devicePriBase` from a **device-info lookup**
(`ogkm-610: src/nvidia/src/kernel/gpu/timer/arch/blackwell/timer_gb100.c:44-61`). The same
pattern appears in `kern_fsp_gb100.c:428`, `kernel_ce_gb202.c:46`, `kernel_sec2_gb20b.c:74`.
And on a GSP client that table is not read from hardware — it is the reply to
`NV2080_CTRL_CMD_INTERNAL_GET_DEVICE_INFO_TABLE`:

> `gpuConstructDeviceInfoTable_FWCLIENT` (`ogkm-610:
> src/nvidia/src/kernel/gpu/gpu_gspclient.c:224-284`) issues `0x20800a40` and copies
> `faultId, instanceId, typeEnum, resetId, devicePriBase, isEngine, rlEngId, runlistPriBase,
> groupId, ginTargetId, deviceBroadcastPriBase, groupLocalInstanceId` out of the reply.

⇒ **On Blackwell, `ChipProfile::device_info` stops being a transcription and becomes the
authoritative declaration of the register map.** That is source **(C)** in its strongest form:
we say where the timer is and the driver believes us. kayfabe already owns this row, already
validates that a `devicePriBase` lands inside the declared aperture and is `PRI_REGISTER_ALIGN`
aligned, and already refuses a stated base for an engine the FIFO table does not carry
(`crates/kayfabe-abi/src/deviceinfo.rs:555-583`).

⚠ **Two new fields.** The 610 `DEVICE_INFO_ENTRY` carries `groupId`, `ginTargetId` and
`deviceBroadcastPriBase` (`gpu_gspclient.c:272-277`) that the 580-era row does not model.
**UNVERIFIED** whether any Blackwell path reads them; verified by grepping `ginTargetId`
consumers in `ogkm-610` before a GB202 row is written.

### 3.3 The (C) boundary, stated where it actually bites

Two fields sit exactly on the line between *"the driver validates it"* and *"the driver computes
with it"*:

- **`ce_fault_method_buffer_size`.** It sizes a real allocation
  (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/arch/volta/kernel_channel_group_gv100.c:77-95`)
  and is folded into reserved-memory sizing
  (`mem_mgr_gm107.c:1925-1928`). On a fully emulated device nothing external writes into that
  buffer, so **(C)** is defensible; on any path where a *real host CE* writes it, it is not.
  ⊘ **UNVERIFIED which applies to kayfabe's current dataplane.** Until that is settled, treat
  the field as it is treated today — a refusal when unstated — and note that (B) cannot supply
  it for a new arch because `0x20802a08` is kernel-privileged.
- **`memory_system.ltcCount × ltsPerLtcCount`.** `kmemsysIsPagePLCable_GA102` branches on the
  **product** (`ogkm-580: kern_mem_sys_ga102.c:66-120`) to decide post-L2-compression
  page eligibility, and `kern_mem_sys_ga100.c:332-345` computes address boundaries from it.
  ⇒ this is *computed with*, not validated. **(C) is legal only by choosing a product the
  chip's own HAL arm recognises**, which is readable from that arm.

### 3.4 What does NOT move on Blackwell — and why the reason is stronger than a header diff

| thing | offset | why it holds on GB202 |
|---|---|---|
| the VF/usermode region base | `0xB80000` | `gpuGetVirtRegPhysOffset` is `_TU102` for **GA106, GH100 and GB202 alike** `[measured, g_gpu_nvoc.c]`; it returns `DRF_BASE(NV_VIRTUAL_FUNCTION_FULL_PHYS_OFFSET)` = `0x00B80000` (`ogkm-580: turing/tu102/dev_vm.h:29`) |
| the doorbell | `+0x30090` ⇒ **`0xBB0090`** | `NV_VIRTUAL_FUNCTION_DOORBELL 0x30090` is **byte-identical** in `turing/tu102/dev_vm.h:229` and `blackwell/gb202/dev_vm.h:27` |
| GSP falcon mailbox0/1, engine-reset | `0x110040`, `0x110044`, `0x1103c0` | identical in `ampere/ga102/dev_gsp.h:27,29,31`, `hopper/gh100/dev_gsp.h:26,29,32`, `blackwell/gb100/dev_gsp.h:27,30,33` |
| `NV_PGRAPH_BASE`, `NV_PTIMER0_PRI_BASE`, `NV_PMC0_PRI_BASE` | `0x400000`, `0x9000`, `0x0` | `ogkm-610: blackwell/gb100/hwproject.h:27,37,38` |
| the FSP EMEM transport | `0x8F2AC0`/`0x8F2AC4` | §1.1 — GB202's default path *calls the GH100 function*, compiled against `hopper/gh100/dev_fsp_pri.h:26,40` |

⊘ **What DOES move:** the WPR2 check. `kgspIsWpr2Up_GB100` reads
`NV_HUBMMU0_PRI_BASE + NV_HUBMMU_PRI_MMU_WPR2_ADDR_HI` = `0x880000 + 0xA828` = **`0x88A828`**
(`ogkm-610: src/nvidia/src/kernel/gpu/gsp/arch/blackwell/kernel_gsp_gb100.c:332-350`,
`blackwell/gb100/dev_hubmmu_base.h:91-96`, `hwproject.h:36`), where GA106 and GH100 both read
`NV_PFB_PRI_MMU_WPR2_ADDR_HI` = `0x1FA828` (kayfabe's constant,
`crates/kayfabe-chips/src/gh100.rs:129-130`, `crates/kayfabe-device/src/ga10x.rs:116-117`).
**One constant.**
---

## 4. The trait methods — which actually vary per family

Traits: `Arch` (`crates/kayfabe-arch/src/lib.rs:1509`), `GmmuFmt` (`:507`), `UserdModel`
(`:546`), `PushbufferAbi` (`:1426`), `HostClasses` (`:1784`); `GspModel`
(`crates/kayfabe-arch/src/gsp.rs:745`), `BootSequence` (`gsp.rs:646`).

### 4.1 `Arch` — 11 methods

| method | lib.rs | Ampere | Ada | Hopper | verdict | source class |
|---|---|---|---|---|---|---|
| `name` | `:1512` | `ga10x.rs` | `ad10x.rs:348` | `gh100.rs:772` | diagnostics | — |
| `classify` | `:1517` | real | `ad10x.rs:351` → **`MockArch`** | `gh100.rs:775` → **`MockArch`** | ⚠ **mock** | **(A)** — `g_gpu_class_list.c` states every chip's list |
| `vchid_from_userd_flags` | `:1534` | real | `ad10x.rs:371` — **not** the mock (`#174`) | `gh100.rs:795` — not the mock | IDENTICAL, by RM's own HAL | (A) |
| `decode_doorbell` | `:1539` | `ga10x.rs:470-483` | `ad10x.rs:382` → `ga10x::decode_work_submit_token` | `gh100.rs:806` → same | IDENTICAL TU→GH; **DIFFERS at GB** | (A) |
| `mmu` | `:1542` | `Ga10xGmmu`, `ga10x.rs:765` | `ad10x.rs:385` → **`MockArch`** | `gh100.rs:809` → **`MockArch`** | ⚠ **mock**; Hopper's real answer is VER3 | (A) |
| `userd` | `:1545` | real | `ad10x.rs:388` → **`MockArch`** | `gh100.rs:812` → **`MockArch`** | IDENTICAL in truth (`NV_RAMUSERD` unredefined Maxwell→Blackwell) — **but answered by a mock** | (A) |
| `engine_of_object` | `:1553` | provided, derived from `classify` | — | — | follows `classify` | — |
| `is_case2_control` | `:1565` | real | → `MockArch` | → `MockArch` | ⚠ **mock** | (C) policy |
| `pushbuffer` | `:1568` | real | `ad10x.rs:394` → **`MockArch`** | `gh100.rs:818` → **`MockArch`** | ⚠ **mock**; `MockPushbuffer` answers `gpfifo_entry_stride` = **16** where the truth is 8 | (A) |
| `gsp` | `:1579` | `Some` | `ad10x.rs:397` `Some(Ad10xGspModel)` | `gh100.rs:821` `Some(Gh100GspModel)` | **DIFFERS** — the seam's real user | (A)+(D) |
| `host_classes` | `:1593` | `Some(Ga10xHostClasses)` | `ad10x.rs:405` | `gh100.rs:828` — *"★★ NOT delegated to `self.inner`"* | **DIFFERS** | (A) |

★ **The `MockArch` delegation is the seam audit's §2.1 finding and it still stands at
`89439595`** — `ad10x.rs` and `gh100.rs` both hold `inner: MockArch` and answer `classify`,
`mmu`, `userd`, `is_case2_control` and `pushbuffer` from it, inside the shipped archive's
dependency graph. ⊘ Two methods were already rescued (`decode_doorbell`, `vchid_from_userd_flags`)
and **that is the proof the shape is fixable**: both now call a file-scope GA10x function with a
citation showing RM binds the same HAL to all three generations
(`crates/kayfabe-chips/src/ga10x.rs:440-469`).

### 4.2 `GmmuFmt` — 6 methods, and the only trait where a family is a real codec

| method | lib.rs | varies? |
|---|---|---|
| `version` | `:509` | **YES** — `Ver2` (Pascal…Ada) vs `Ver3` (Hopper+). `Ver3` is declared at `:228-233` and **constructed nowhere** |
| `page_sizes` | `:511` | 4K/64K/2M/512M on both VER2-GA10x and VER3-GB202 ⊘ (the 256 GiB level exists in `_GB10X` but `bPageSize256gbSupported` is FALSE for GB202 — `g_kern_gmmu_nvoc.c:322-323`) |
| `entry_size` | `:513` | 8 / 8 / 16 (PTE / PDE / dual-PDE) on **both** (`pascal/gp100/dev_mmu.h:157,97,112` vs `hopper/gh100/dev_mmu.h:188,79,128`) |
| `levels` | `:515` | **YES** — 5 vs **6**; max VA `1<<49` vs `1<<57` (`ogkm-610: kern_gmmu_gm107.c:324,327`) |
| `level_shift` | `:530` | **YES** — the level table |
| `decode_entry` | `:535` | **YES — the codec.** `KIND` `63:56`→**`11:8`**; `ADDRESS_SYS 53:8`/`ADDRESS_VID`→**unified `ADDRESS 51:12`**; discrete `VOL 3:3`/`PRIVILEGE 5:5`/`READ_ONLY 6:6`/`ATOMIC_DISABLE 7:7`→**`PCF 7:3`, 32 encodings**; comptagline **removed**; PDE `VOL 3:3`→**`PDE_PCF 5:3`**; new `VER3_PDE_IS_PTE 0:0` |

⇒ **Four of six differ, and one of them is the whole port's biggest item.** The
four-families count (`_GP10X` Turing, `_GA10X` Ampere+Ada, `_GH10X` Hopper, `_GB10X` Blackwell)
is `support_matrix_seam_audit.md` §2.2's finding, independently reproduced here from
`ogkm-610: g_kern_gmmu_nvoc.c`. What this note adds: **`_GB10X` is `_GH10X` plus one line, and
that line is unreachable on GB202** — so building VER3 lands Hopper, Blackwell and Rubin
together, and Turing is `Ga10xGmmu` minus one match arm (`ga10x.rs:817`).

### 4.3 `UserdModel` — 3 methods, **zero variance Maxwell→Blackwell**

`userd_size` (`:548`), `gp_get_offset` (`:550`), `gp_put_offset` (`:552`). `NV_RAMUSERD_GP_GET`
= dword 34, `GP_PUT` = dword 35, size 512 (`ogkm-610: maxwell/gm107/dev_ram.h:47,48,50`;
`ampere/ga100/dev_ram.h:37,38`), and **no Blackwell header redefines `RAMUSERD`**. ⇒ **(A), one
implementation for the whole band.** ⚠ Yet Ada and Hopper answer it from `MockArch` today.

### 4.4 `PushbufferAbi` — 5 methods

`method_len` (`:1432`), `decode_method` (`:1436`), `gpfifo_entries` (`:1440`),
`gpfifo_entry_stride` (`:1459`), `decode_run` (provided, `:1484`). The `GP_ENTRY` layout —
8 bytes, `GET 31:2`, `GET_HI 7:0`, `LENGTH 30:10` — is **identical in `clc56f.h` (Ampere),
`clc86f.h` (Hopper), `clc96f.h`/`clca6f.h` (Blackwell)** (`support_matrix_seam_audit.md` §2.5,
cross-checked here). ⇒ **(A), one implementation.** ⚠ `MockPushbuffer` answers **16** for the
stride, and it is what Ada and Hopper return — an invented value in the shipped graph.
`decode_method`'s *class method* vocabulary does vary (`SET_REPORT_SEMAPHORE_A` is a compute-class
method address), and that variance currently lives as a literal in `kayfabe-rt` (§6.1).

### 4.5 `HostClasses` — 4 methods, and two refusals that (A) can now retire

| role | Ampere (`host_classes.rs:93-105`) | Ada (`:132-143`) | Hopper (`:186-197`) | **GB202** |
|---|---|---|---|---|
| `gpfifo_channel` | `AMPERE_CHANNEL_GPFIFO_A` `0xC56F` | **same** `0xC56F` | `HOPPER_CHANNEL_GPFIFO_A` `0xC86F` | `BLACKWELL_CHANNEL_GPFIFO_B` **`0xCA6F`** |
| `usermode` | `AMPERE_USERMODE_A` `0xC561` | **same** | `HOPPER_USERMODE_A` | `BLACKWELL_USERMODE_A` **`0xC761`** |
| `ce_object` | `AMPERE_DMA_COPY_B` `0xC7B5` | **same** | `HOPPER_DMA_COPY_A` | `BLACKWELL_DMA_COPY_B` **`0xCAB5`** |
| `compute_object` | `Some(AMPERE_COMPUTE_B 0xC7C0)` | ⊘ **`None`** | ⊘ **`None`** | — |

★★ **Ada's and Hopper's `None`s are now resolvable from (A), and their comments are out of
date.** `host_classes.rs:137-141` says `ADA_COMPUTE_A` is *"a NAME in this tree's capability
tables and `kayfabe-abi` carries no value for it"*; `:162-164` says the same of
`HOPPER_COMPUTE_A`. Both have values in the vendored tree —
**`ADA_COMPUTE_A = 0xC9C0`** (`ogkm-610: src/common/sdk/nvidia/inc/class/clc9c0.h:27`) and
**`HOPPER_COMPUTE_A = 0xCBC0`** (`clcbc0.h:26`) — and both are confirmed *exposed by that
chip* in `g_gpu_class_list.c` (AD102 at `:1382`, GH100 at `:2048`). The refusal was right when
written (`kayfabe-abi` carries no value) and is now a **generator request away**, not a
measurement away. ⊘ It still must go through `gen/`'s manifest, not a hand-typed literal — that
is what `#156` established for the other three roles.

⚠ Blackwell needs care here: GB202 exposes **`BLACKWELL_COMPUTE_B 0xCEC0`** and **not**
`BLACKWELL_COMPUTE_A 0xCDC0`; GB100 is the reverse. Same for `DMA_COPY_B 0xCAB5` (GB202) vs
`_A 0xC9B5` (GB100). Read the chip's own row in `g_gpu_class_list.c`, never the family's name.

### 4.6 `GspModel` — 8 methods, and `BootSequence` — 4

`GspModel` (`gsp.rs:745`): `decode_reg` (`:754`), `is_startcpu` (`:757`), `is_booter_unload`
(`:765`), `is_swgen0_clear` (`:774`), `encode` (`:781`), `libos_region_layout` (`:784`),
`boot_sequence` (`:794`). Its register vocabulary `GspReg` (`gsp.rs:63`) has 20 variants.

For a GB202 model, from §3.4 and §5.1:

| `GspReg` | GB202 |
|---|---|
| `GspFalconMailbox0/1`, `GspFalconEngine`(reset), `GspFalconIrq*`, `GspRiscv*`, `GspQueueHead(i)` | **unchanged offsets** — `kgspConfigureFalcon` resolves to `_GA102` |
| `Wpr2AddrLo/Hi` | **moves** to `0x88A824`/`0x88A828` |
| `GfwBootProgress`, `GfwBootPlm` | **replaced** — Blackwell polls `NV_THERM_I2CS_SCRATCH` at `0xAD00BC == 0xFF`; `NV_PGC6_AON_SECURE_SCRATCH_GROUP_*` does not exist under `blackwell/` |
| `Sec2FalconCpuctl`, `Sec2FalconMailbox0`, `Sec2FalconDmatrfcmd` | ⊘ **not used** — SEC2 boots the GSP-FMC only on GB10B/GB20B/GB20C (`ogkm-610: g_kernel_sec2_nvoc.c:466-474`), not GB202 |

`BootSequence` (`gsp.rs:646`): `stages` (`:650`), `on_write` (`:662`), `may_read` (`:694`,
default `false`), `on_read` (`:699`, default `None`). Two impls exist, and the stage lists are
the clearest statement of how different two boots can be:

| `FalconSecureBooterBoot` — 5 stages (`crates/kayfabe-gsp/src/seq.rs:83-105`) | `Gh100FspBoot` — 3 stages (`crates/kayfabe-chips/src/gh100.rs:383-397`) |
|---|---|
| *"GSP falcon STARTCPU runs FWSEC and raises the protected region"* | *"the FSP command-queue HEAD write starts the GSP-FMC and raises FRTS"* |
| *"SEC2 Booter Load starts the RM firmware"* | *"the same command loads GSP-RM — there is no separate Booter Load"* |
| *"LibOS boot-args address, low half, into the GSP falcon MAILBOX0"* | *"the boot-args pointer comes from the command payload, not a mailbox pair"* |
| *"LibOS boot-args address, high half, into MAILBOX1"* | — |
| *"the completed pair publishes the message queues"* | — |

⊘ `NoBootSequence` (`gsp.rs:711-726`) is the third: it declares **no** stages and answers **no**
step, so a generation whose registers are mapped but whose boot is unwritten stays in
`BootPhase::Cold` and the gap is visible in a test rather than hidden behind a borrowed boot.

★★★ **A Blackwell port needs NO third `BootSequence`.** `kgspBootstrap` and
`kgspPrepareForBootstrap` bind GB202 to the **Hopper** functions (`g_kernel_gsp_nvoc.c:721-748`),
and every FSP protocol HAL is `_GH100` for GB202 (§5.1). What differs is a handful of *register
values* the sequence reads — the WPR2 address and the boot-ready gate — which are
`GspModel`/`may_read` concerns, not ordering concerns. ⇒ **the `#121` seam pays off exactly
where it was built to.**
---

## 5. The Blackwell job — the smallest set of NEW hand-written things

Ordered by what blocks what. **Every item is (C) or (D) unless marked.**

### 5.1 What a GB202 port gets FOR FREE, and this is the surprise

| already built | why it serves GB202 |
|---|---|
| **The entire FSP boot sequence** — `Gh100FspBoot` (`crates/kayfabe-chips/src/gh100.rs`) | `kgspBootstrap` binds GB202 to **`kgspBootstrap_GH100`**, not to a Blackwell function (`ogkm-610: g_kernel_gsp_nvoc.c:742-748`); so does `kgspPrepareForBootstrap` (`:721-727`). The FSP *protocol* layer is shared too — `kfspPrepareBootCommands`, `kfspSendBootCommands`, `kfspProcessCommandResponse`, `kfspProcessNvdmMessage`, `kfspValidateMctpPayloadHeader`, `kfspNvdmToSeid`, `kfspGetPacketInfo`, `kfspWaitForGspTargetMaskReleased` are all `_GH100` for GB202 `[measured]` |
| **The FSP EMEM transport offsets** | §1.1: GB202's default path *calls the GH100 function*, compiled against `hopper/gh100/dev_fsp_pri.h`. The MNOC alternative is regkey-gated OFF (`ogkm-610: src/nvidia/src/kernel/gpu/fsp/kern_fsp.c:170-176`) |
| **The whole FWSEC / Booter / VBIOS machinery is NOT NEEDED** — and deleting a requirement is better than porting it | `kgspExecuteFwsec`, `kgspPrepareForFwsecFrts`, `kgspExecuteBooterLoad/Unload`, `kgspExecuteHsFalcon`, `kgspExtractVbiosFromRom` are all **stubs** returning `NV_ERR_NOT_SUPPORTED` for GB202 (`g_kernel_gsp_nvoc.c:1396-1546`; the stub bodies at `g_kernel_gsp_nvoc.h:1997-2031`). FRTS is created by FSP via the COT command (`ogkm-610: kern_fsp_gh100.c:1446-1447`), not by a falcon we emulate |
| **The GSP falcon register aperture** | `kgspConfigureFalcon` for GB202 resolves to **`_GA102`** (`g_kernel_gsp_nvoc.c:604-605`): `registerBase 0x110000`, `riscvRegisterBase 0x111000`, `fbifBase 0x110600` — kayfabe's existing constants |
| **`NV_PGSP_QUEUE_HEAD 0x110c00 + i*8`** | `kgspSetCmdQueueHead_TU102` for GB202 (`g_kernel_gsp_nvoc.c:705-706`) against `turing/tu102/dev_gsp.h:38` |
| **The usermode page, the doorbell offset, the VF time registers, the TLB-invalidate register and its every field, the fault-buffer registers, the USERD layout, the GPFIFO entry layout** | §3.4, plus: `NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE 0x30B0` and its 13 fields are **bit-identical** `ampere/ga100/dev_vm.h:63-101` vs `blackwell/gb100/dev_vm.h:338-376`; `NV_RAMUSERD_GP_GET/PUT` at dwords 34/35 are unredefined by any Blackwell header |
| **PRAMIN** | `0x007FFFFF:0x00700000`, byte-identical `turing/tu102/dev_ram.h:26` vs `blackwell/gb202/dev_ram.h:27` |
| **The GSP RPC / message-queue ABI** | keyed on DRIVER VERSION, not arch (`crates/kayfabe-abi/src/versions.rs`; `support_matrix_seam_audit.md` §0.4 measured every function and event number unchanged 580→610) |

### 5.2 The NEW hand-written items — eleven, and one of them is most of the work

| # | item | class | size | citation |
|---|---|---|---|---|
| **1** | ★★★★★ **A `GmmuFmt` for VER3** — 6 levels (PD4…PT), 57-bit VA, unified `ADDRESS 51:12`, the 5-bit `PCF 7:3` replacing discrete `VOL`/`PRIVILEGE`/`READ_ONLY`/`ATOMIC_DISABLE`, `KIND` narrowed to `11:8`, comptagline gone, `VER3_DUAL_PDE`, `PDE_PCF 5:3` | (A) — every bit is in `hopper/gh100/dev_mmu.h:53-188` | **the single biggest item**, and `GmmuVersion::Ver3` is already declared and constructed nowhere (`crates/kayfabe-arch/src/lib.rs:228-233`) | `kgmmuFmtIsVersionSupported` is `_GH10X` for GH100 **and every GB and GR chip** (`g_kern_gmmu_nvoc.c:700-703`); `kgmmuFmtInitLevels_GB10X` = `_GH10X` **plus one line**, `pLevels[2].bPageTable = NV_TRUE` (`ogkm-610: src/nvidia/src/kernel/gpu/mmu/arch/blackwell/kern_gmmu_fmt_gb10x.c:51-61`) |
| | ★★★ **and it is shared work.** Building it lands Hopper AND Blackwell AND Rubin at once — `kgmmuFmtIsVersionSupported` names GH100, every GB, and GR100/GR102 in one arm. | | | |
| | ⊘ And the 256 GiB leaf the `_GB10X` line enables is **not** reachable on GB202: `bPageSize256gbSupported` is TRUE only for GB100/GB102/GB110/GB112/GR100/GR102 (`g_kern_gmmu_nvoc.c:322-323`). ⇒ **GB202's `GmmuFmt` is Hopper's VER3, unchanged.** | | **zero extra** | |
| **2** | **A doorbell decoder** — `VECTOR 11:0`, `RUNLIST_ID 22:16`, **plus `RUNLIST_DOORBELL` bit 30 with `_ENABLE = 0x1`** | (A) | ~10 lines | `blackwell/gb202/dev_vm.h:27-32`; encoder `ogkm-610: src/nvidia/src/kernel/gpu/fifo/arch/blackwell/kernel_fifo_gb202.c:65-67`; gating `g_kernel_fifo_nvoc.c` GB202-and-later arm |
| **3** | **The WPR2 register** | (A), one constant | 1 line | `0x880000 + 0xA828` = `0x88A828` (§3.4) |
| **4** | **The GFW-boot-ready gate** — Blackwell does **not** use `NV_PGC6_AON_SECURE_SCRATCH_GROUP_05`; it polls `NV_THERM_I2CS_SCRATCH` at BAR0 **`0x00AD00BC`** for `== 0xFF` | (A) | 1 constant + 1 predicate | `ogkm-610: src/common/inc/swref/published/blackwell/gb202/dev_therm.h:27` (`0x00ad00bc`) + `dev_therm_addendum.h:27-30` (`_FSP_BOOT_COMPLETE_STATUS 31:0`, `_SUCCESS 0x000000FF`), consumed by `_kfspWaitBootCond_GB202` at `ogkm-610: src/nvidia/src/kernel/gpu/fsp/arch/blackwell/kern_fsp_gb202.c:50-62` — *"In GB202, Bootfsm triggers FSP execution out of chip reset. FSP writes 0xFF value in NV_THERM_I2CS_SCRATCH register after completion of boot"*. ⊘ `NV_PGC6_AON_SECURE_SCRATCH_GROUP_03/_05/_42` are **NOT FOUND anywhere under `blackwell/` or `hopper/`** |
| **5** | **The BAR0 PRAMIN window register** — moves from `NV_PBUS_BAR0_WINDOW 0x1700` to `NV_XAL_EP_ZB_BAR0_WINDOW` at **BAR0 `0x0010FD40`**, `BASE` narrowed to **22:0**, `BASE_SHIFT 0x10` unchanged, and the `TARGET` field **removed** (always VIDMEM) | (A) | 3 constants + a field-width change | ★ **a worked example of §1.1 — the register's two halves come from two DIFFERENT chip directories.** GB202 binds `kbusGetXalAperture` to **`_GH100`** (`ogkm-610: g_kern_bus_nvoc.c`, arm `/* ChipHal: GH100 \| GB100 \| … \| GB202 \| … */`), which is compiled in `kern_bus_gh100.c:3077` against **`hopper/gh100/hwproject.h:32`** ⇒ aperture base `NV_XAL_BASE_ADDRESS = 1110016 = 0x10F000`. It binds `kbusWriteBAR0WindowBase` to **`_GB100`**, compiled in `kern_bus_gb100.c` (includes at `:27`) against **`blackwell/gb100/dev_nv_xal_ep_zb.h`** ⇒ offset `0xd40` and `BASE 22:0` (`:40-43`). ⊘ Reading `hopper/gh100/dev_nv_xal_ep_zb.h:28` instead gives `BASE 21:0` — **one bit short**, and silently |
| **6** | **`intr_subtree_map` / interrupt-leaf geometry** — `CPU_INTR_LEAF__SIZE_1` goes **8 → 16** | (A) | a constant that is currently implicit | `ampere/ga100/dev_vm.h:50` vs `blackwell/gb100/dev_vm.h:103`. ⚠ `crates/kayfabe-device/src/cpuintr.rs:103-124` hard-codes the whole block and does **not** read it from `ChipProfile` — see §6 |
| **7** | **`PMC_BOOT_0` / `PMC_BOOT_42` values** — arch `0x1B` (`_GB200`), impl `0x2` | (A)+(B) | 2 values | `NV_PMC_BOOT_42_ARCHITECTURE 29:24` and `_GB200 = 0x1B` at `ogkm-610: src/common/inc/swref/published/nv_ref.h:171,184`; the binding row is `ogkm-610: src/nvidia/generated/g_hal_archimpl.h:91` — `{ NV_PMC_BOOT_42_ARCHITECTURE_GB200, NV_PMC_BOOT_42_IMPLEMENTATION_2, 0x0 }, // GB202` (GA102 is `:73`, `{GA100, IMPL_2}`). ★ `_halmgrIsChipSupported` compares **only** arch(29:24) and impl(23:20) and ignores every revision field (`ogkm-610: src/nvidia/src/kernel/core/hal_mgr.c:139-146`) ⇒ bits 29:20 must be **`0x1B2`**. ⊘ A zero `pPmcBoot42` fails safely (`hal_mgr.c:147-151`), and `0xFFFFFFFF` reads as a dead GPU |
| **8** | **Class ids** | (A), mechanical from `g_gpu_class_list.c` | a `HostClasses` impl | `BLACKWELL_CHANNEL_GPFIFO_B 0xCA6F`, `BLACKWELL_USERMODE_A 0xC761`, `BLACKWELL_DMA_COPY_B 0xCAB5`, `BLACKWELL_COMPUTE_B 0xCEC0`, `BLACKWELL_B 0xCE97`, `BLACKWELL_INLINE_TO_MEMORY_A 0xCD40` — GB202's list at `ogkm-610: src/nvidia/generated/g_gpu_class_list.c:2882-2986`. ⊘ GB202 does **not** expose `BLACKWELL_COMPUTE_A 0xCDC0` or `BLACKWELL_DMA_COPY_A 0xC9B5` — those are GB100's |
| **9** | **MMU fault engine ids** — wholly renumbered | (A) | a table swap | `GRAPHICS` 64→**384**, `BAR1` 128→**256**, `BAR2` 192→**320**, `CE0` 15→**65**, `PHYSICAL` 31→**56**; new `HOST0..44 = 85..129`, `GSPLITE0..7 = 20..27` (`blackwell/gb202/dev_fault.h:27-274` vs `turing/tu102/dev_fault.h:31-53`). ⊘ Packet format unchanged: `NVC369_BUF_SIZE = 32` |
| **10** | **`NV_RAMRL_ENTRY_BASE_SHIFT`** — GA100 `12`, GA102 `10`, **GB202 `8`** | (A) | 1 constant | `ampere/ga102/dev_ram.h:26` vs `blackwell/gb202/dev_ram.h:29`; and `kfifoRunlistGetBaseShift` is `_GB202` where GA106 and GH100 are both `_GA102` `[measured]` |
| **11** | **A `ChipProfile` row + a `BootSequence` selection** | (D) | one row | the row itself; §5.3 |

### 5.3 ⊘ And the item that is NOT on that list because it is not per-arch work: **there is no selector**

`[read 2026-09-13 at 89439595]` the three pins the seam audit named in 2026-08 are **all still
there**:

1. `crates/kayfabe-qemu-raw/src/shim.rs:16976-16978` —
   `Gpu::new(Arc::new(kayfabe_chips::Ga10xArch::new()), …)`, unconditional. Plus three more
   hard-wired `Ga10xGmmu` constructions at `shim.rs:13822`, `:10141`, `:10328`.
2. `crates/kayfabe-chips/src/host_classes.rs:220-222` — `pinned_host_classes()` returns
   `&Ga10xHostClasses` always, though all three profiles exist.
3. `crates/kayfabe-device/src/inittables.rs:2379` — `GspFeatures::GA106` hardcoded inside the
   **chip-generic** encode arm.

Plus `crates/kayfabe-device/src/lib.rs:673` — `CHIPS` has one row, and the chip is chosen by a
**QEMU device property**, not a probe (`shim.rs:2537`, `:13759`).

⇒ **`Ad10xArch` and `Gh100Arch` are reachable only from `tests/`.** The traits are honestly
built and refuse by name, but the shipped binary can only be GA106. A Blackwell `impl` that
nothing can select is a fixture, exactly as Ada and Hopper are today — so **the selector is a
prerequisite of the port, not a follow-up.**

⚠ Constraint on any fix, already encoded: `crates/kayfabe-qemu-raw/tests/e2_doorbell.rs:477-482`
scans `shim.rs`'s source and asserts `Gpu::new(` appears **exactly once**, so the selector must
be an *expression* feeding the one call, not a second call.

### 5.4 ★★ The two traps a Blackwell port walks into, stated before it does

1. **`gb100/dev_vm.h` and `gb202/dev_vm.h` give CONTRADICTORY definitions of the SAME
   register.** `NV_VIRTUAL_FUNCTION_DOORBELL` is at `0x30090` in both. On **GB202**,
   `RUNLIST_DOORBELL` is bit **30** with `_ENABLE = 0x1` (`blackwell/gb202/dev_vm.h:30-32`).
   On **GB100**, it is bit **22** with `_ENABLE = 0x0` and `_DISABLE = 0x1`, plus a
   `GSP_DOORBELL` at bit 31 (`blackwell/gb100/dev_vm.h:625-632`). Same name, same offset,
   different bit, **opposite polarity**. `same_flag_opposite_polarity`, in a vendored header.
   ⇒ A GB202 port that reaches for `gb100/dev_vm.h` — the natural move, since it is the
   directory with 40 files — decodes the wrong bit.
   ★ **And the two fail DIFFERENTLY against today's decoder.**
   `crates/kayfabe-chips/src/ga10x.rs:470-483` refuses any token with a bit outside
   `0x007F_0FFF`. A **GB202** token sets bit 30 ⇒ **refused loudly** — which is what
   `support_matrix_seam_audit.md` §2.5 predicted and is correct. A **GB100** token sets no
   bit outside the mask (its encoder writes only `RUNLIST_ID` and `VECTOR`, and ENABLE is
   `0` — `ogkm-610: kernel_fifo_gb100.c:93`) ⇒ **silently accepted**. So the seam audit's
   *"refused at GB"* holds for GB202 and **does not generalise to GB100**; there the wrong
   answer is the quiet one.
2. **Blackwell discovers register bases from the PTOP/device-info table rather than from
   fixed BAR0 offsets** — TMR, HUBMMU and LCE all (`blackwell/gb202/dev_top_zb.h:27` carries
   exactly one define, `NV_PTOP_ZB_DEVICE_INFO_DEV_TYPE_ENUM_LCE 0x13`). On a GSP client that
   table is **our reply** (§3.2). This is an opportunity, not a hazard — but a port that
   treats `device_info` as a transcription to be captured rather than a map to be authored
   will look for a capture that cannot be taken, because `0x20800a40` is `INTERNAL`.
---

## 6. What in `ga10x.rs` is model-specific rot, ranked

`[read]` `crates/kayfabe-device/src/ga10x.rs` is 1921 lines, of which roughly **355** are
capture-derived table bodies and the rest is doc-comment provenance. The rot is not evenly
spread. Ranked by *"what breaks when this file is copied for a second part of the same
family"*:

| rank | what | why it is rot | the fix |
|---|---|---|---|
| **1** | **`gr_static`** — 3 GPCs, `tpc_mask 0x1e/0x1f/0x1f`, 14 TPCs, 28 SMs (`crates/kayfabe-abi/src/grstatic.rs:486-494`) | `0x1e` **is** one disabled TPC on one physical die. A GA102 is 7 GPC / 42 TPC. Nothing in the tree claims a second GA106 has this mask | **(B)** — `GR_GET_INFO` + `GR_GET_GPC_MASK` + `GR_GET_TPC_MASK`, all `NON_PRIVILEGED` (§2.2) |
| **2** | **`gr_info`** — 58 flat entries (`crates/kayfabe-abi/src/grinfo.rs:297-358`) | ★ the *shape* is the defect: six entries are the floorswept part (`SHADER_PIPE_COUNT 3`, `SHADER_PIPE_SUB_COUNT 14`, `GPU_CORE_COUNT 3584`, `RT_CORE_COUNT 28`, `TENSOR_CORE_COUNT 112`) and the `LITTER_*` entries are **family maxima** (`LITTER_NUM_GPCS 7`, annotated *"family max, a BOUND"*). **One array, two kinds of fact, distinguished only by a trailing comment** | **(B)**; and split the array so the two kinds cannot be confused |
| **3** | **`AMPERE_CORES_PER_SM = 128` / `AMPERE_TENSOR_CORES_PER_SM = 4` inside a chip-generic validator** (`crates/kayfabe-abi/src/grinfo.rs:157,159`, consumed `:253,255`) | an **arch** constant inside `validate_against`, which runs over *any* `GrStaticProfile`. The seam audit already flagged it (§2.4) and honestly refused to state Turing's value | **(D)** — a per-family field on the chip row, so a non-Ampere row is not refused by an Ampere constant |
| **4** | **`lce_pce_masks`** `[0x20,0x10,0x10,0x20]` | the field's own doc: *"a PCE→LCE map is a per-*part* fact … A GA102 answers different words"* (`crates/kayfabe-device/src/lib.rs:334-336`) | **(C)** author it consistently with `engines`, or **(B)** via `0x20802a02` |
| **5** | **`memory_system.ltc_count = 6 / lts_per_ltc = 4 / l2_cache_size = 0x24_0000`** | the file itself records that the **real** part reports `LTS_COUNT = 18`, not 24 — the capture and the live silicon already disagree about the same die | **(C)** bounded by the chip's own HAL arm (§3.3) |
| **6** | **`gr_context_buffers`** — 26 sizes (`crates/kayfabe-abi/src/grstatic.rs:790-817`) | attribute/beta/spill CB sizes scale with TPC and SM count | **(B)** — `GR_GET_CTX_BUFFER_SIZE 0x20801218`, `GR_GET_ATTRIBUTE_BUFFER_SIZE 0x2080121e`, both `NON_PRIVILEGED` |
| **7** | **`fb_length` 12 GiB, and its four dependents** — `fb_regions`, `bar1_pde_base`, `NV_USABLE_FB_SIZE_IN_MB` | one board's memory size propagated four ways. ★ Mitigated well: three `const assert!`s make a wrong FB size a **build error**, not a silent one | **(C)** — it is the size WE present; make it the free variable it already is documented to be |
| **8** | **`pci_device_id 0x2504`, `pci_revision 0xA1`, `subsystem 0x1462/0x397D`, `chip_sub_rev`** | an MSI-brand card's ids from one dumped ROM, and a stepping | **(B)** — all four are 0444 sysfs (§2.2) |
| **9** | **`engines` (146 lines) + `intr_table` (146 lines)** | 292 lines of one board's capture, and the doc **admits reproducing uninitialised capture noise verbatim** (`engineData[7]`, the `SOFTWARE` tail) | **(C)** — these are promises we author, not silicon we transcribe |
| **10** | **`conf_compute` and `bif_static`** | both decoded from **`dlen = 0`** capture rows (`0x20800af3`, `0x20800aac`) — the exact class `CLAUDE.md` records as *"every `dlen=0` row checked against real hardware is CONTRADICTED"*. ⚠ And `bif_static.bPcieGen4Capable = false` **already contradicts `pcie_max_gen = Gen4`** inside the same struct literal; the comment notices and carries it anyway | **(D)** — six booleans, hand-written per family, and delete the citation to an empty row |

### 6.1 ★★ Arch leakage OUTSIDE the chip crates — ~20 production sites

`[read]` A second arch is not confined to `kayfabe-chips` today. The full list, ranked:

| severity | site | what leaks |
|---|---|---|
| ❌ **the only arch constant gating production CONTROL FLOW** | `crates/kayfabe-rt/src/completion_watch.rs:63,153` | `pub const AMPERE_COMPUTE_B: u32 = 0xc7c0;` then `if bound[subch] != Some(AMPERE_COMPUTE_B) { continue; }`. A **local re-declaration** of `kayfabe_abi::generated::classes::AMPERE_COMPUTE_B` in a logic crate, and a completion decode that skips every non-Ampere compute object. Also `:57` `SET_REPORT_SEMAPHORE_A = 0x1b00` — an Ampere-compute **method address**, which is `PushbufferAbi::decode_method`'s job |
| ❌ | `crates/kayfabe-device/src/cpuintr.rs:103-124` | the whole Turing+ interrupt block hard-coded and **not read from `ChipProfile`**: `VF_PRIV_BASE 0x00B8_0000`, `LEAF0 +0x1000`, `LEAF_EN_SET0 +0x1200`, `LEAF_EN_CLEAR0 +0x1400`, `TOP0 +0x1600`, `TOP_EN_SET0 +0x1608`, `TOP_EN_CLEAR0 +0x1610`, `LEAF_TRIGGER +0x1640`. ⇒ item 5.2#6 (leaf count 8 → 16) has **no seam to land in** |
| ❌ | `crates/kayfabe-device/src/inittables.rs:2379` | `GspFeatures::GA106` unconditional in the chip-generic encode path |
| ❌ | `crates/kayfabe-qemu-raw/src/shim.rs:9744` | `const GP_FIFO_ENTRY_BYTES: u64 = 8;` whose own doc says *"⊘ Not a tunable: it is the width of the hardware structure"* — the claim `crates/kayfabe-arch/src/lib.rs:1451` explicitly refutes, and `MockPushbuffer` already answers **16**. Used at `:8828`, `:8881`; `:6696` `PROBE_RING_BYTES` |
| ❌ | `crates/kayfabe-qemu-raw/src/shim.rs:19565` | `base + kayfabe_abi::submit::USERD_GP_GET` — the GA10x `0x88` |
| ❌ | `crates/kayfabe-qemu-raw/src/shim.rs:18446-18450` | a `v <= 0x1000` / `v < 0x20_0000` page-size classifier — 4 K / 2 M baked in |
| ❌ | `crates/kayfabe-util/src/trapwitness.rs:698` | `off == 0x0017_00 \|\| (0x0070_0000..0x0080_0000).contains(&off)` — `NV_PBUS_BAR0_WINDOW` and PRAMIN hard-coded in the crate at the **bottom** of the graph, while both are already chip rows (`ga10x.rs:193,198,207`) |
| ❌ | `crates/kayfabe-isolate-host/src/rm.rs:1718` | `map_cpu(object, USERMODE_WINDOW_SIZE, …)` with `USERMODE_WINDOW_SIZE = 65536` = `NVC361_NV_USERMODE__SIZE` — an **Ampere-class** constant applied to whatever `HostClasses::usermode()` returned |
| ⚠ diagnostics only | `crates/kayfabe-isolate-host/src/rm.rs:1774,1786`; `src/bin/rmladder.rs:207,12882,14845,15114` | `token >> 16` / `token & 0xffff` as doorbell field extraction — **inside `eprintln!` strings**, so not control flow, but **wrong against `decode_work_submit_token`**: `VECTOR` is 11:0 (`0x0FFF`, not `0xffff`) and `RUNLIST_ID` is 22:16 (`>>16 & 0x7F`) |
| ✅ mitigated | `crates/kayfabe-device/src/mmuinval.rs:123,126,129`; `src/doorbell.rs:243` | the deltas are literals but the **base** is read from `chip_info.reg_bases[USERMODE]` and `invalidate_regs()` returns `None` rather than defaulting (`:183-192`) |
| ✅ clean | `kayfabe-core`, `kayfabe-mmu` (`walker.rs:236` is fully generic — **zero** 12/21/29/38/47 literals), `kayfabe-completion`, `kayfabe-gsp`, `kayfabe-rmrpc`, `kayfabe-vmm*`, `-shell`, `-trace`, `-isolate`, `-linux-raw` | |
| ⚠ by design | `crates/kayfabe-abi/` — ~40 `GA106_*`/`AMPERE_*` constants | legitimate *where they live* (the Axis-A quarantine); **the leak is where they are consumed**, i.e. the rows above |

⊘ **Dismissed as false positives, checked:** `0x11_0000_0000` in `tests/tests/{reactor,reactor_os,multi_gpu}.rs` is a **GPA range end**, not `NV_PGSP`. `>> 16 & 0x3FFF` in `kayfabe-linux-raw` is **ioctl-number decoding**. `0x1000` in `abi/generated/rpc.rs:147` is an RPC message-id base. ~34 hits in `kayfabe-device/src/sweep.rs` are inside `why:` rationale **string literals**.
---

## 7. The generator work that would make (A) real

`crates/kayfabe-abi/gen/` already derives 6 files / ~6540 lines from ogkm. Extending it to the
register plane is **four** pieces, sized from §2.1's measurement:

1. **A bulk request kind.** Every constant today is requested individually by name with a
   hand-written doc string (`ConstReq`, `gen/src/main.rs:45-52`). A register table wants *"emit
   every define in file F"*. ★ `MacroListReq` already has the right posture to copy — a `keep`
   allowlist plus a **hard refusal when the scan comes back empty**: *"a silently-empty scan
   must never pass for success"* (`main.rs:1310-1317`).
2. **Keep the classifier comment.** `strip_comments` (`gen/src/parse.rs:72`) deletes
   `/* RW-4R */` before any scanner sees it — and that tag is the only thing distinguishing
   `NAME 0x00000001` (a value) from `NAME 0x00110040` (an offset).
3. **Indexed registers** — `NAME(i) (base+(i)*stride)` plus the paired `NAME__SIZE_1`. ~10 % of
   lines, and they are exactly the ones this port needs: `NV_PGSP_QUEUE_HEAD(i)`,
   `NV_VIRTUAL_FUNCTION_PRIV_CPU_INTR_LEAF(i)`, `NV_PFSP_EMEMC(i)`.
4. **Per-file expected counts.** `scan_defines`' contract is *"anything else is ignored"*
   (`parse.rs:393-394`) — safe when each constant is individually requested and a miss is a hard
   error, dangerous for a bulk sweep where a dropped line is invisible.

**And a fifth that is not parser work at all — a chip axis.** `MODULES` is a single flat list of
hardcoded paths (`main.rs:107-208`), the output is one directory, `OGKM_VERSION` is one scalar,
and the VBIOS module reads `arch/turing/kernel_gsp_vbios_tu102.c` by name with no `<arch>/<chip>`
substitution. The one axis that exists is a **class-ID** axis, not an arch axis
(`main.rs:130-142`, `#156`'s Hopper host classes) — and the arch dispatch for it happens
downstream, hand-written, in `kayfabe-chips`.

★ **The payoff already has a named victim.** `crates/kayfabe-chips/src/gh100.rs:95-104`:
*"★ **The BASE is ASSUMED, not read.** `NV_FALCON2_GSP_BASE` … is **not** defined under
`hopper/gh100/` in the vendored tree. This model reuses the Ampere base."* §1.1 says that
assumption is *right* — `kgspConfigureFalcon` for GH100 and GB202 both resolve to `_GA102`
(`ogkm-610: g_kernel_gsp_nvoc.c:604-605`) — but it was right by luck, and a generator that
followed the include of the implementing HAL would have derived it rather than assumed it.

⊘ **Two gates that do not exist and should, both already written down as gaps.** There is **no
CI check that the committed generated output still matches the generator** — the deletion
recipe is at `docs/reference/nvidia_abi_oracles.md:320-322`. And `gen/` is deliberately outside
the workspace (`gen/Cargo.toml:30`, an empty `[workspace]` table) so ogkm can never become a
build dependency — correct, and the reason its 24 unit tests had never run anywhere until a
dedicated phase was added (`scripts/run_full_suite.sh:250-274`).

★★★ **And two ogkm tables that are free wins, unrelated to register offsets:**
- `ogkm-610: src/nvidia/generated/g_nv_name_released.h` — **1927** `{devID, subSysID,
  subSysVendorID, name}` rows. This retires `GPU_GET_NAME_STRING` returning 23 zero bytes and
  `nvidia-smi` printing `ERR!`, **without a single per-model row of ours**.
- `ogkm-610: src/nvidia/generated/g_gpu_class_list.c` — the per-chip class-descriptor list,
  which is how `HostClasses` should be populated for any new arch (GB202's is at `:2882-2986`).

Both are **NVIDIA's** tables: they cost nothing per model, and they expire exactly when the
vendored tree is re-pinned — which is the failure mode
`a_capture_derived_table_expires_as_a_vendor_regression` describes, but with a version stamp
already on it (`OGKM_VERSION`).
---

## 8. ⊘ What CANNOT be answered from source — the real-Blackwell-hardware list

Everything below is **UNVERIFIED** and the note says so rather than guessing. Each row names
the measurement that would settle it.

| # | question | why source cannot answer it | what would |
|---|---|---|---|
| 1 | **`ce_fault_method_buffer_size` for GB202** | produced inside GSP firmware, in no open source, and `0x20802a08` is **kernel-privileged** — refused to root (`g_subdevice_nvoc.c:7666`, flags `0x1c040`; `[measured]` on an RTX 3060 at `crates/kayfabe-isolate-host/src/bin/rmladder.rs:559-568`) | the same instrument that got GA106's `20480`: an **instrumented build of the guest driver** on a Blackwell part. ⊘ Do **not** carry GA106's number across — the field exists precisely to stop that (`crates/kayfabe-device/src/lib.rs:300-314`) |
| 2 | **`NV_USABLE_FB_SIZE_IN_MB`'s Blackwell address** | its GA102 address is a GC6-AON alias (`ampere/ga102/dev_gc6_island_addendum.h:33`) and Blackwell's `dev_gc6_island*.h` carries only `NV_PGC6_BSI_SECURE_SCRATCH_12` | read the Blackwell `kmemsysReadUsableFbSize` HAL's include chain, then confirm on a board |
| 3 | **Whether the 610 `DEVICE_INFO_ENTRY` fields `groupId` / `ginTargetId` / `deviceBroadcastPriBase` are read on Blackwell** (`gpu_gspclient.c:272-277`) | greppable in principle; **not grepped here** | grep `ginTargetId` / `deviceBroadcastPriBase` consumers in `ogkm-610` — this one is *source*-answerable and simply was not done |
| 4 | **`intr_subtree_map` for GB202** | the map is a GSP reply body, not a header constant; kayfabe's GA106 row rests on two oracles agreeing | a boot, or a GSP-RPC capture on a Blackwell part |
| 5 | **`fifo_channels.channels_per_runlist`, `gmmu_static` fault-buffer sizes** | both are GSP reply bodies. `NV_CHRAM_CHANNEL__SIZE_1 = 2048` on GA100 and GB202 alike (`blackwell/gb202/dev_runlist.h:27`) makes 2048 *plausible*, not measured | a boot |
| 6 | **Whether the GB202 guest driver actually accepts an authored `devicePriBase` for TMR** | §3.2 is a source reading of `gpuConstructDeviceInfoTable_FWCLIENT` + `tmrGetTmrBaseAddr_GB100`. It has never been exercised | boot a GB202 guest against a device that declares a TMR entry, and see whether `tmrGetTimerBar0MapInfo_GB100` maps where we said |
| 7 | **Whether a non-root uid can actually issue the 16 `NON_PRIVILEGED` controls** | today's isolate drops **capabilities, not uid** (`crates/kayfabe-isolate-host/src/rm.rs:199-202`) — on a root VMM the host kernel still sees euid 0 | run `rmladder` under a genuinely different uid and re-read the same rows. ⚠ **This gates the whole of source (B)**, and it is cheap |
| 8 | **Every value in §5.2 in a running boot** | the entire note is a source read | the standing answer in this tree: *"Only LIVE BOOTS are proof"* |
| 9 | **GR geometry of the specific GB202 part being mirrored** | floorsweeping is per-die by construction | (B) at runtime — which is the *point* of moving `gr_static` to (B): the value stops needing to be known in advance |
| 10 | **The BAR firewall** — `NV_EP_PCFG_GPU_VSEC_DEBUG_SEC_2_BAR_FIREWALL_ENGAGE` bit 24 at PCI cfg `0x2B8`, polled by `gpuWaitForBarFirewallHal_GB100` (`blackwell/gb202/dev_xtl_ep_pcfg_gpu.h:29-31`; `ogkm-610: src/nvidia/src/kernel/gpu/arch/blackwell/kern_gpu_gb100.c:1825`) | ⚠ it is at config offset `0x2B8` — **beyond the 64 bytes** a non-root reader gets, and it is a *new gate the guest polls* that no Ampere path has | source says what to answer; only a boot says whether answering it is enough |
| 11 | **GSPLITE** — a new engine class on GB20x with its own MMU engine ids and HUB fault clients (`ogkm-610: src/nvidia/src/kernel/gpu/gsplite/kernel_gsplite.c`; `blackwell/gb202/dev_fault.h:48-55`) | whether the guest requires it present is a boot question | a boot |

⊘ **And one thing that is NOT on this list, deliberately.** The GSP *firmware* is not needed:
kayfabe fakes the GSP, and Blackwell's boot is FSP-driven with every FWSEC/Booter/VBIOS path
stubbed out (§5.1). A Blackwell port needs **no NVIDIA firmware blob**, and that is a smaller
requirement than Ampere's, not a larger one.
---

## 9. The verdict, and the size of the Blackwell job

### 9.1 The contract is satisfiable, and the reason is specific

The owner's *"one hand-written section per family"* holds **only because most of a chip row is
not a fact about silicon at all** — it is a promise the emulated device makes and the guest
believes. `engines`, `intr_table`, `device_info`, `fb_regions`, `fb_length`, `pci_bars`,
`bar1_pde_base`, `msix_vectors`, `lce_pce_masks`, `fifo_channels`, `gmmu_static` and the whole
VBIOS are **(C)**. The tree already proves the posture works: the VBIOS is entirely synthetic
and satisfies the driver's inequalities by construction (`crates/kayfabe-abi/src/vbios.rs:80-89`).

What is genuinely a fact about silicon splits cleanly:
- **the register map and the wire formats** → **(A)**, and the derivation is mechanical (§1.1,
  §2.1);
- **the part's identity and its floorplan** → **(B)**, and 16 of the 20 controls that matter are
  `NON_PRIVILEGED` (§2.2);
- what is left for **(D)** is genuinely small: a `GspModel` constructor, a `BootSequence`
  selection, six booleans (`conf_compute` + `bif_static`), `smc_mode`, `has_c2c`, and a
  per-family cores-per-SM pair. **That is the budget, and it fits.**

### 9.2 Size of a GB202 port

| bucket | size |
|---|---|
| **Already built and reused as-is** | the FSP boot ordering, the FSP transport offsets, the GSP falcon aperture, the queue head, PRAMIN, the usermode page, the doorbell offset, the VF timers, the TLB-invalidate register and all 13 fields, the fault-buffer registers, USERD, the GPFIFO entry, the whole driver-version-keyed RPC ABI |
| **Deleted requirement** | FWSEC, the Booter, the VBIOS ROM window, the synthetic VBIOS image, the GC6-AON scratch gate — **all stubbed out by the driver itself on Blackwell** |
| **New, and it is one real codec** | the VER3 `GmmuFmt` — 6 levels, 57-bit VA, `PCF`, unified `ADDRESS 51:12`, 4-bit `KIND`, `VER3_DUAL_PDE`. ★ **shared with Hopper and Rubin** |
| **New, and each is a handful of lines** | the doorbell decoder (bit 30, ENABLE=1); `WPR2 = 0x88A828`; the boot-ready gate `0xAD00BC == 0xFF`; the BAR0 window at `0x10FD40` with `BASE 22:0` and no `TARGET`; leaf count 8→16; `PMC_BOOT_42` arch/impl `0x1B2`; `RAMRL_ENTRY_BASE_SHIFT` 8; six class ids; the renumbered fault engine ids |
| **Prerequisite, and NOT arch work** | **a selector.** `shim.rs:16976`, `host_classes.rs:220`, `inittables.rs:2379`, `CHIPS` at `lib.rs:673`, and ~20 leakage sites (§6.1) — chief among them `completion_watch.rs:153`, the one arch constant gating production control flow |
| **Needs a real Blackwell board** | the eleven rows of §8 — headed by `ce_fault_method_buffer_size`, which (B) provably cannot supply |

⇒ ★★★ **The Blackwell-specific work is smaller than the Ampere-generic work still outstanding.**
The port is gated on the selector and on VER3, not on Blackwell's novelty — and VER3 is Hopper's
bill, already owed.

⚠ **And the honest counterweight, stated last so it is not lost.** This whole note is a
**source read**. `only_live_boots_are_proof` applies to every row of it. The single cheapest
thing that would turn a large part of it from argument into measurement is §8 row 7 — run the
sixteen `NON_PRIVILEGED` controls under a genuinely non-root uid — because **all of source (B)
rests on it**, and it needs no Blackwell hardware at all.
