# V3 — THE HARDWARE-BOUNDARY INVENTORY: every hardware-defined interface kayfabe touches, per die group

**STATUS: LIVE, 2026-09-26 (branch `v3-hwinv`, off master `e05ff74d`). Source-only — no box was used.**
The hardware-side counterpart of `nvkvm-pv`'s ioctl ABI table. It answers the owner's question
*"We know all driver boundaries for ioctl structs from nvkvm-pv. Do we also know all hardware boundary
structs?"*, lists every register, field and in-memory format kayfabe reads, writes, emulates or decodes,
per die group, with where its value comes from and whether hardware ever confirmed it. It also documents
the generator (`tools/derive_hwref.sh` → `kf_chip::hwref`) that now checks those values against ogkm.
Companion documents: `THE_BAR0_DISPOSITION_MAP.md` (how each page is *trapped*; this document covers what
the values *are*), `V3_FAMILY_PORT_ADA.md`, `V3_FAMILY_PORT_BLACKWELL.md`, `V3_FLOORSWEPT_GR.md`.

## 0. The answer

**Before this branch: no.** The hardware boundary existed only as hand-written literals scattered over
a dozen crates and the C device. Each literal cited a header line in a comment, and nothing checked any of
them against ogkm. `kf-chip`'s own crate doc listed *"register offsets — generated from ogkm `dev_*.h` per
family (to come)"*. The walk kernel's page-table fields were pinned only to their own second copy (the
Rust descriptor matched the `.cu` byte for byte, but neither was compared with `dev_mmu.h`).

**Now:**

- **An inventory.** §4 has 100 rows (plus 15 host-fact rows and the firmware-protocol list) across 7 die
  groups: TU10x, GA100, GA10x, AD10x, GH100, GB10x, GB20x.
- **A generated table.** `tools/derive_hwref.sh` produces 9 448 `(chip directory, macro)` rows from
  ogkm-580. They are **evaluated by the compiler, never parsed**. §2 gives the lineage resolution per die
  group.
- **43 tests** hold the hand constants of `kf-chip`, `kf-trap`, `kf-mem` (the walk descriptors),
  `kf-chan`, `kf-host` and `kf-rm` to the header of every die group or class that uses them. They cover
  ≥ 270 distinct ogkm names. See §3 for the per-family counts.

⚠ **The table CHECKS the literals; it does not yet SUPPLY them.** Only 1–3 rows per die group are
class `a`, derived from source. The ranked list of what should move from `c` to `a` is §7.

**The inventory found, and this branch fixed**, four defects that source alone proves. Each is its own
commit with a test (§5):

| commit | fix | die groups |
|---|---|---|
| `3cb0175a` | **Security.** The usermode window a guest *user* client maps is 64 KiB, but only its first 4 KiB was classified as userspace-mappable. Writes to 0xBB1000–0xBBFFFF from unprivileged guest userspace entered the **privileged ring**, which a guest process could fill to poison the device. | all |
| `1e37217d` | Hopper's MMU fault ids *are* in the tree (UVM's `hwref/hopper/gh100/dev_fault.h:83`, `HOST0 = 64`). A false *"no HOST0 anywhere"* premise had refused **every GH100 realize**. | GH100 |
| `8a669b81` | `NV2080_NOTIFIERS_CE10` is 166, not 184 (184 is `GSP_PERF_TRACE`). Copy engines 10–19 armed the wrong notifiers. | GB100, GB110 |
| `e47b30e5` | GB10B (arch GB100, impl 0xB) is an integrated part and is now refused. It had been accepted as a discrete GB10x. | GB10B |

It also lists 28 discrepancies that are **not** fixed here. Some would change GA10x or another
family's behaviour; others need design work. They are ranked in §5.2, and two of them matter most:

1. **A GSP boot retry can never succeed on Turing, Ampere or Ada after one failed attempt.** WPR2 is
   derived without RM's WPR-end margin, and that margin becomes non-zero after a failure.
2. **GB100 and GB102 cannot read their PCIe link capabilities.** RM walks the PCIe capability list, and
   kf3's configuration space has no PCIe capability. Expect the same *"Unknown PCIe speed"* UVM failure
   GB203 had before bws1.

**Hopper and GB10x have never run.** Every row they touch is source-only. §6 lists what each will meet.

## 1. Scope and method

**In scope** is what the GPU architecture defines, so it varies by chip rather than by driver version:

- BAR0 registers and their fields;
- in-memory formats that the GPU or the guest RM defines: page tables, USERD, GPFIFO entries, method
  headers, semaphores, fault-buffer entries, CE method layouts;
- doorbell tokens and channel-id spaces;
- the per-die facts kayfabe reads from host controls.

**Out of scope** is RM control and RPC parameter layouts and the GSP message queue. These are the driver
boundary, which `nvkvm-pv`'s table and `kf-abi` cover. Where a driver-ABI enumeration is indexed per
engine instance (the `cl2080` notifiers), it is listed and marked as driver ABI.

**Measured:** 0 of the 7 811 `(directory, name)` rows that both ogkm **580.159.04** and **610.43.02**
publish changed value. The vocabulary is version-stable; only the published subset grows. That confirms
the premise above.

**Die groups, not families.** A hardware fact varies by die group: GA100 is not GA10x, and GB10x is not
GB20x. `kf_chip::Family` has five rows, so `kf_chip::hwref::DieGroup` adds the seven columns used here.

**Method.**

1. **Five read-only sweeps.** Each covered one area: the trap plane; page tables; channels and CE; GSP and
   falcon boot; per-die facts and topology. Each listed every hardware literal with its `file:line`, and
   checked it against the ogkm-580 header that the family's **bound HAL** compiles against. That
   binding comes from `src/nvidia/generated/g_*_nvoc.c` and the `#include` of the HAL's source file.
2. **Spot-checks against the source.** Before any row was trusted, the findings that change a verdict
   were re-read in the source: the usermode span, CE10, HOST0, GB10B, the WPR margin, the GB100
   capability walk, the swallowed extended-base entry, GB20x's kind rule and GB10x's 256 GiB leaf.
3. **The generator.** It made the header side mechanical. Every hand constant a test now checks is
   marked `T` in §4.

Line numbers are at `e05ff74d`.

## 2. The generator and the checked table

| piece | what it does |
|---|---|
| `tools/hwref_spec.txt` | The vocabulary: macro-**name** prefixes (`NV_MMU_VER3_`, `NV_PFSP_`, …), exact names (`NV_PGSP$`), whole class headers (`class clc86f.h`) and struct layouts (`struct clc86f.h Nvc86fControl GPGet GPPut …`). Names only — never C. |
| `tools/derive_hwref.sh` | For every discrete chip directory under `published/` (Kepler … Blackwell), the root `nv_ref.h`, UVM's own `hwref/` copies, and the SDK class headers: the **preprocessor** lists each header's macros (`gcc -E -dM`, which also sidesteps headers that are not self-contained C), and the **compiler** evaluates each body — pass 1 as a value, pass 2 through ogkm's own `DRF_EXTENT`/`DRF_BASE` (`(1?x)`/`(0?x)`, `nvmisc.h:233-234`) for `hi:lo` fields, apertures and structure-bit ranges, pass 3 through `DRF_PICK_MW` for the `MW(...)` fault-buffer fields; `offsetof`/`sizeof` for structs. One-parameter macros are evaluated at `(0)` and `(1)` (base and stride). `--check FILE` diffs a committed table against a fresh derivation. |
| `crates/kf-chip/data/hwref-580.159.04.tsv` | The committed output: **9 448 rows, 28 directories, 5 448 names** (7 566 values, 1 803 ranges, 38 multi-word fields, 41 struct rows). ~620 KB. |
| `kf_chip::hwref` | **Resolution**: which directory a die group's driver compiles against. Each die group has a **lineage** — tiers of directories, its own first, then the ancestors its HALs reuse (`NV_RAMUSERD_GP_GET` exists only in `gm107`/`ga100`; Hopper binds GA100 *and* GA102 bodies, so both sit in one tier). A name resolves in the nearest tier that defines it; if that tier's directories disagree, it is **`Ambiguous`** and must be settled by a **pin** naming the bound HAL. Four pins exist (the RISC-V IRQ pair on GH100/GB10x → `kflcnRiscvReadIntrStatus_GA102`). `kf_chip::hwref::expect` gives resolve-or-panic helpers whose panic says which directory disagreed. |
| `cargo run -p kf-chip --example hwref -- NAME…` | Prints a name's value per die group (`·` inherited, `📌` pinned, `⊘amb`, `—` absent); `--differs` lists every name whose value differs between die groups (1 274 today). |
| `crates/kf-chip/tests/hwref_is_current.rs` | Re-derives the table when ogkm-580 is present (`$KF_OGKM_580`, `third_party/ogkm-580`, or the research clone) and fails if the committed file is stale; skips loudly otherwise. |

**The tests that consume it** (43 `#[test]`s; every one GPU-free):

| where | holds to the headers |
|---|---|
| `kf-chip/src/{bar0,falcon_gsp,fsp_gsp,usermode,ptekind}.rs` (`hwref_check`) | every boot register at its die group's offset (with a named `UNREAD` list for served-but-unread words), PMC_BOOT fields, config-space words; every falcon/RISC-V/queue/GFW/WPR2/PRAMIN offset and encoding for the TU102 and GA102 layouts; every FSP-regime offset, EMEMC field and encoding; the usermode-MMIO row; every PTE kind by name. **Ratchet:** `FspRow::BLACKWELL` carries GB20x's boot gate for GB10x. |
| `kf-chip/src/hwref.rs` (unit) | planted positives for `Own` (RISC-V IRQ moved), `Inherited` (RAMUSERD, and the nearest ancestor winning over a farther differing one), `Ambiguous` (GB20x's `NV_XAL_EP_BAR0_WINDOW_BASE`: GB100 22:0 vs GH100 21:0), pin validity, every emitted directory placed in a lineage. |
| `kf-trap/tests/hw_boundary_vs_ogkm.rs`, `fspemem.rs`/`tokenindex.rs` (in-module) | PRAMIN window and base register per die group (Blackwell's 24:0 asserted as a superset), MMU invalidate registers and fields, both cache-op protocols, the CPU interrupt tree (`LEAF__SIZE_1` 8 vs 16), timer refusals, hole pages, FSP channel, EMEMC fields, token fields (and the GB10x/GB20x `RUNLIST_DOORBELL` split), `CHID_BITS`, and the usermode window span (§5 D1). |
| `kf-mem/tests/walk_format_vs_ogkm.rs` | the walk kernel's VER2/VER3 descriptors field by field against `dev_mmu.h` per die group. **Ratchet:** VER3 PDE PCF. |
| `kf-chan/tests/channel_formats_vs_ogkm.rs`, `notifiers_vs_ogkm.rs`, `translated.rs` (in-module) | USERD cursors per channel class (GPGet absent on C96F/CA6F), GPFIFO entry, method header, host semaphore/MEM_OP, every CE method/field per CE class, the OFFSET_UPPER width split, the rewriter's own codes; `cl2080` notifier indices. |
| `kf-rm/src/authored.rs` (in-module) | every family's MMU fault ids (GRAPHICS, CE0 + i, HOST0, NVENC/NVDEC) against `dev_fault.h`. |

**What the table does not cover, by construction:** page-table **level geometry** and leaf sets (they are
C code in `kern_gmmu_fmt_*.c`, not headers); **which HAL a die group binds** (the lineage approximates it
and refuses where it cannot — the rows of §4 cite the binding); firmware protocol and RM ABI (§1); and any
register the spec does not name — **the tests are the enumeration**, so a hand constant with no test is
still unchecked (the `c` cells without `T` in §4).

## 3. Counts per status class per die group

Counted from the 100 rows of §4.1 to §4.6. Each cell of a row counts once for its die group. The 15
host-fact rows of §4.7 are `b` on every die group and are not included here.

| class | TU | GA100 | GA10x | AD | GH | GB10x | GB20x |
|---|---|---|---|---|---|---|---|
| `a` | 3 | 3 | 3 | 3 | 2 | 1 | 1 |
| `b` | 10 | 10 | 10 | 10 | 9 | 8 | 9 |
| `c✓` | 38 | 41 | 42 | 39 | 43 | 40 | 43 |
| `c✗` | 3 | 2 | 1 | 1 | 3 | 7 | 7 |
| `c?` | 12 | 10 | 10 | 14 | 14 | 10 | 11 |
| `d` | 9 | 11 | 10 | 10 | 13 | 17 | 14 |
| `s` | 2 | 2 | 2 | 2 | 2 | 2 | 2 |
| `·` | 23 | 21 | 22 | 21 | 14 | 15 | 13 |
| of which held by a test (`T`) | 31 | 31 | 31 | 32 | 35 | 32 | 34 |
| **touched** (rows not `·`) | 77 | 79 | 78 | 79 | 86 | 85 | 87 |

- **`c✓` is the bulk.** About 40 per die group are hand-written constants that match the header. About
  four fifths of those (the `T` row) are now held there by a test. Before this branch, none was checked
  against a header; the existing tests compared literals with literals.
- **`c✗` and `d` concentrate on the families that never ran.** GB10x has 7 `c✗` and 17 `d`, GB20x 7 and
  14, GH100 3 and 13. GA10x has 1 and 10, and its `d` rows are design gaps shared by every family, such
  as fault delivery and 128 KiB pages.
- **`c?` is mostly the authored topology** (§4.6): PBDMA ids, runlists, reset bits, vectors, buffer
  sizes. No header states these, and they are anchored to one GA106 capture.
- **`a` is tiny on purpose.** The generator checks the literals rather than supplying them (§7).

A row counts as **MEASURED** for a die group only when its evidence names a die of that group.
Otherwise it is **SOURCE-ONLY**, **UNIT** (unit tests only) or **REFUSED**.

| verification | TU | GA100 | GA10x | AD | GH | GB10x | GB20x |
|---|---|---|---|---|---|---|---|
| MEASURED | 0 | 0 | 55 | 28 | 0 | 0 | 47 |
| SOURCE-ONLY | 75 | 77 | 21 | 49 | 83 | 82 | 37 |
| UNIT | 1 | 1 | 1 | 1 | 2 | 2 | 2 |
| REFUSED | 1 | 1 | 1 | 1 | 1 | 1 | 1 |

⇒ **TU10x, GA100, GH100 and GB10x have no measured row.** No die of those groups has run kayfabe. The
AD10x count is lower than GA10x only because most register-level evidence cites GA106 captures. AD106
booted 30/30 through the same code (`V3_FAMILY_PORT_ADA.md`).

## 4. The inventory

Legend for the seven die-group cells:

| cell | meaning |
|---|---|
| `a` | derived from source: generated from ogkm, or computed from derived facts |
| `b` | read from the host at VM start (an unprivileged RM control, or sysfs) |
| `c✓` | hand-written constant that equals the header this die group's bound HAL compiles against |
| `c✗` | hand-written constant that disagrees with that header, or carries another die group's value |
| `c?` | hand-written constant that no header states (authored layout, a GA106 capture, a driver literal) |
| `d` | missing, refused, not decoded, or silently wrong for this die group |
| `s` | not modelled; the BAR0 shadow's last-write / zero read-back satisfies every reader the source shows |
| `·` | this die group never touches it |
| `T` suffix | held to the header by a test added on this branch (`kf_chip::hwref`) |

Verification column: **M:** measured on the named dies (evidence in the cited doc/trace); **S** source-only;
**R** refused by name; **U** unit tests only. Paths: `crates/` omitted for kayfabe; for ogkm-580, `pub/` =
`src/common/inc/swref/published/`, `hw/` = `kernel-open/nvidia-uvm/hwref/`, `uvm/` =
`kernel-open/nvidia-uvm/`, `cls/` = `src/common/sdk/nvidia/inc/class/`, `gen/` = `src/nvidia/generated/`,
`krn/` = `src/nvidia/src/kernel/gpu/`. **kayfabe line numbers are at master `e05ff74d`** (the branch
point); rows this branch changed name the fixing commit.

### 4.1 Identity, PCI function, configuration space

| item | kayfabe | TU | GA100 | GA10x | AD | GH | GB10x | GB20x | ogkm header per die group | verification |
|---|---|---|---|---|---|---|---|---|---|---|
| `NV_PMC_BOOT_0` 0x0 — arch 28:24 (= MC arch >> 4), impl 23:20, rev 7:0; `ARCHITECTURE_1` 8:8 never set | kf-chip/src/bar0.rs:54-56,82,145-150 | b | b | b | b | b | b | b | `pub/nv_ref.h:108-136` (every lineage's root; fields `c✓T`) | M:GA106 (= captured 0x176000A1), AD106, GB203 |
| `NV_PMC_BOOT_1` 0x4 = 0 (VGPU = REAL) | bar0.rs:83,151-156 | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | `pub/nv_ref.h:138-148` | M:GA106, AD106, GB203 |
| `NV_PMC_BOOT_42` 0xA00 — arch 29:24, impl 23:20, major 19:16, minor 15:12 | bar0.rs:61-66,84,157-162 | b | b | b | b | b | b | b | `pub/nv_ref.h:149-171` (fields `c✓T`) | M:GA106 (0x176A1000), AD106, GB203 |
| PCI identity (vendor, device, subsystem, class, revision) | kf-qemu/src/hostfacts.rs:46-60,98-104; device.rs:320-324; qemu/hw/misc/kf3/kf3.c:617-623 | b | b | b | b | b | b | b | board facts, no header | M:GA106, GA104, AD106, GB203 |
| BAR0 size | kf-qemu/src/hostfacts.rs:77-78,105; device.rs:361 | b | b | b | b | b | b | b | none; ⚠ falls back to GA106's 16 MiB (`kf-trap/src/timer.rs:30`) when sysfs reports 0 | M:GA106, AD106, GB203 |
| BAR1 / BAR3 ("RM BAR2") / BAR5 sizes and types; MSI-X in BAR5, 32 vectors | kf3.c:52-53,629-685,738-742; device.rs:127,353-355 | c? | c? | c? | c? | c? | c? | c? | none for BAR types; hardware MSI-X is in BAR0 at 0xB90000 with 6 / 9 / 12 vectors (`pub/turing/tu102/dev_vm.h:222-223`, `pub/hopper/gh100/dev_vm.h:49-53`, `pub/blackwell/gb100/dev_vm.h:569-581`) — a deliberate device-shell choice | M:GA106, AD106, GB203 |
| Configuration space: conventional PCI, 256 B, MSI-X capability only | kf3.c:660,696,766 | · | · | · | · | c? | **d** | c? | GB100/GB102 read config registers by walking the PCIe capability list (`krn/arch/blackwell/kern_gpu_gb100.c:359-393`, bound `gen/g_gpu_nvoc.c:1226-1235`); kf3 has none, so the walk fails | S |
| `NV_XVE_LINK_CAPABILITIES` BAR0 mirror 0x88084 (value = host sysfs max speed/width) | bar0.rs:75-80,86,163-168; kf-abi/src/businfo.rs:295-302 | b | b | b | b | · | · | · | `pub/maxwell/gm107/dev_nv_xve.h:104` + `NV_PCFG` (`pub/turing/tu102/dev_nv_xve.h:26`); offset `c✓T`; served but unread on GH/GB | M:GA106, AD106 (x8 fix) |
| `NV_EP_PCFGM` + `LINK_CAPABILITIES` mirror 0x9206C | bar0.rs:220-227,246-248 | · | · | · | · | c✓T | · | c✓T | `pub/hopper/gh100/dev_xtl_ep_pri.h:26`, `dev_xtl_ep_pcfg_gpu.h:73`; read at `krn/arch/hopper/kern_gpu_gh100.c:99-109` (bound GH100 + GB20x, `gen/g_gpu_nvoc.c:1203-1207`) | M:GB203 (bws2); GH100 S |
| Config-cycle `LINK_CAPABILITIES` dword: 0x6C (GH100/GB20x), 0x4C (GB10x) | bar0.rs:280-292; kf3.c:686-703 | · | · | · | · | c✓T | **d** | c✓T | `pub/hopper/gh100/dev_xtl_ep_pcfg_gpu.h:73`; `pub/blackwell/gb100/dev_pcfg_pf0.h:126`; on GB100/GB102 unreachable (row above); GB110/GB112 read 0x4C directly | S |
| PCIe speed map (sysfs "2.5" … "32.0 GT/s" → generation) | kf-qemu/src/hostfacts.rs:85-92 | b | b | b | b | b | **d** | b | a "64.0 GT/s" (Gen6) host is refused at realize although `kf-abi/src/businfo.rs:160` defines Gen6 (`ctrl2080bus.h:363,375`) | M (GA106, AD106, GB203) |
| VBIOS PROM window 0x300000, 1 MiB, synthetic ROM (host PCI id + host VBIOS version + generated FWSEC) | kf-qemu/src/device.rs:25-27,580-588; bar0.rs:325-340; kf-abi/src/vbios.rs | a | a | a | a | · | · | · | `pub/turing/tu102/dev_ext_devices.h:27`; FSP families read no ROM (`gen/g_kernel_gsp_nvoc.c:1283-1301`) | M:GA106, AD106 |
| `NV_PMC_ENABLE` / `DEVICE_ENABLE(0)` / `PMC_INTR(0)` | not modelled | s | s | s | s | s | s | s | `pub/ampere/ga100/dev_boot.h:28,42`; writers are reset paths | S |
| Family / die group from `MC_GET_ARCH_INFO`; integrated parts refused | kf-chip/src/lib.rs:85-145; `hwref::DieGroup::from_arch` | b | b | b | b | b | b | b | `ctrl2080mc.h:77-151`; ⚠ GB10B (arch GB100, impl 0xB: `pub/nv_arch.h:111`, `gen/g_hal_archimpl.h:88`) was accepted as discrete — **fixed `e47b30e5`** | M:GA106, AD106, GB203 |

### 4.2 GSP / FSP boot, falcon and RISC-V registers

| item | kayfabe | TU | GA100 | GA10x | AD | GH | GB10x | GB20x | ogkm header per die group | verification |
|---|---|---|---|---|---|---|---|---|---|---|
| `NV_USABLE_FB_SIZE_IN_MB` 0x1183A4 = the store's size | bar0.rs:95,176-183 | · | · | b | b | b | b | b | `pub/ampere/ga102/dev_gc6_island_addendum.h:33` (offset `c✓T`; GH/GB inherit it: `kmemsysReadUsableFbSize_GA102` is bound through every GB die, `gen/g_kern_mem_sys_nvoc.c:353-357`); GA100 is served it but binds `_GP102` | M:GA106, AD106, GB203 |
| `NV_PFB_PRI_MMU_LOCAL_MEMORY_RANGE` 0x100CE0 — scale 3:0, mag 9:4 | bar0.rs:99,124-136,184-193 | b | b | · | · | · | · | · | `pub/pascal/gp102/dev_fb.h:26-31` (`c✓T`); ⚠ silently omitted when the size is not `mag << scale` (a Turing guest then reads FB size 0); GA10x is served it, unread. GB100's own LMR (0x1FA3E0, mag 27:4) is read only on the self-hosted C2C path | S |
| `NV_PGC6_BSI_SECURE_SCRATCH_15` 0x1180FC — `SCRUBBER_HANDOFF` 31:29 = DONE | bar0.rs:100-105,233-240 | · | · | · | c✓T | · | · | · | `pub/ada/ad102/dev_gc6_island(_addendum).h:27-29`; `kgspExecuteScrubberIfNeeded_AD102` bound for AD10x only (`gen/g_kernel_gsp_nvoc.c:1376-1391`) | M:AD106 (A/B) |
| GFW boot progress 0x118234 = 0xFF; its PLM 0x118128 = all levels | kf-chip/src/falcon_gsp.rs:141-156 | c✓T | c✓T | c✓T | c✓T | · | · | · | `pub/turing/tu102/dev_gc6_island(_addendum).h`, `pub/ampere/ga102/…` | M:GA106 (cap1), AD106 |
| `NV_THERM_I2CS_SCRATCH` FSP boot-complete = 0xFF (static boot register) | bar0.rs:108-116,204-211 | · | · | · | · | c✓T | c✓T | c✓T | 0x200BC `pub/hopper/gh100/dev_therm.h:26`, `pub/blackwell/gb100/dev_therm.h:27`; 0xAD00BC `pub/blackwell/gb202/dev_therm.h:27`; `gen/g_kern_fsp_nvoc.c:418-436` | M:GB203; GH, GB10x S |
| `FspRow` boot-gate claim (the FSM's copy of the same register) | kf-chip/src/fsp_gsp.rs:185,291-302 | · | · | · | · | c✗ | c✗ | c✓T | HOPPER row: none, though GH100 polls it (`_GH100`); BLACKWELL row: GB20x's 0xAD00BC for GB10x too. Harmless (the static row above serves both); **ratchet test** | U |
| `WPR2_ADDR_LO/HI` 0x1FA824/0x1FA828 — `VAL` 31:4 = addr >> 12; values derived from the FB size | falcon_gsp.rs:145-147,234-273; fsp_gsp.rs:125-135 | a | a | a | a | a | d | d | `pub/turing/tu102/dev_fb.h:34-39`, `pub/hopper/gh100/dev_fb.h:43-52` (offsets/fields `c✓T`); Blackwell: the 580 pair is decoded but never published (reads 0, correct by accident: the only 580 reader is the "already up" gate); the 610 HUBMMU pair 0x88A824/8 is published (610 header, not in the 580 table) | M:GA106 (cap1), AD106 |
| WPR end margin (`kgspGetWprEndMargin`: 0 on a clean boot, > 0 after any failed bootstrap) | falcon_gsp.rs:248-266 assumes 0 | **d** | **d** | **d** | **d** | · | · | · | `krn/gsp/arch/turing/kernel_gsp_tu102.c:776`; `krn/gsp/kernel_gsp.c:3908-3921,5637-5697`; exact LO compare `krn/gsp/arch/turing/kernel_gsp_frts_tu102.c:514-524` ⇒ every retry after one failed boot fails | S (the pre-fix AD106 boot recorded "4 retries") |
| FRTS size 1 MiB; WPR end alignment 0x20000; VGA workspace = `DRF_SIZE(NV_PRAMIN)` | falcon_gsp.rs:211-234 | c✓ | c✗ | c✓ | c✓ | c✓ | c✓ | c✓ | `pub/turing/tu102/dev_gc6_island_addendum.h:29`; GA100's FRTS is 0 (`gen/g_kernel_gsp_nvoc.c:1250-1255`); `PRAMIN` `c✓T` | M:GA106, AD106 |
| Falcon bases `NV_PGSP` 0x110000, `NV_PSEC` 0x840000 | falcon_gsp.rs:54-56; fsp_gsp.rs:32 | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | `pub/*/dev_gsp.h:26`, `dev_sec_pri.h:27` (PSEC TU…AD only) | M:GA106, AD106, GB203 |
| Falcon registers IRQSCLR/IRQSTAT/IRQMASK/IRQDEST/MAILBOX0-1/HWCFG2/CPUCTL/DMATRFCMD and fields STARTCPU 1:1, HALTED 4:4, HWCFG2_RISCV 10:10, BR_PRIV_LOCKDOWN 13:13, DMATRFCMD_IDLE 1:1, SWGEN0 6:6 | falcon_gsp.rs:58-75,157-174; fsp_gsp.rs:36-57,222-250 | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | `pub/turing/tu102/dev_falcon_v4.h`, `pub/ampere/ga100|ga102/…`, `pub/hopper/gh100/…` | M:GA106 (cap1 499/499 GSP reads), AD106, GB203 |
| HWCFG2 served value (0x400; FSP: 0x2400 until WPR2 up) | falcon_gsp.rs:165,453; fsp_gsp.rs:231-234,643-649 | c✓ | c✓ | c? | c? | c✓ | c✓ | c✓ | GA102+ `RESET_READY` (bit 31, `pub/ampere/ga102/dev_falcon_v4_addendum.h:27-28`) never set ⇒ each falcon reset spins its 150 µs timeout (benign) | M:GA106, AD106, GB203 |
| RISC-V block at `NV_FALCON2_GSP_BASE` 0x111000 — GA102 layout: CPUCTL +0x388 (ACTIVE 7:7, HALTED 4:4), IRQMASK/IRQDEST +0x528/+0x52C, BCR_CTRL +0x668 (VALID 0:0); TU102 layout: CORE_SWITCH_RISCV_STATUS +0x240 (0:0), IRQMASK/IRQDEST +0x2B4/+0x2B8, no BCR_CTRL | falcon_gsp.rs:85-138; fsp_gsp.rs:70-93 | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | `pub/turing/tu102/dev_riscv_pri.h:27-33`, `pub/ampere/ga100|ga102/dev_riscv_pri.h`; GH/GB10x state no IRQ pair ⇒ resolved by a **pin** to `ampere/ga102` (`kflcnRiscvReadIntrStatus_GA102`, `gen/g_kernel_falcon_nvoc.c:628-635`) | M:GA106, AD106, GB203 |
| GSP `QUEUE_HEAD(i)` 0x110C00 + 8i, i < 8 | falcon_gsp.rs:77-83; fsp_gsp.rs:59-65 | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | `pub/turing/tu102/dev_gsp.h:38-40` (compiled for all, `kgspSetCmdQueueHead_TU102`) | M:GA106, AD106, GB203 |
| FSP EMEM channel 0: EMEMC 0x8F2AC0 (OFFS 7:2, BLK 15:8, AINCW 24, AINCR 25), EMEMD 0x8F2AC4, QUEUE_HEAD/TAIL 0x8F2C00/04, MSGQ_HEAD/TAIL 0x8F2C80/84 | fsp_gsp.rs:151-198; kf-trap/src/fspemem.rs:36-61 | · | · | · | · | c✓T | c✓T | c✓T | `pub/hopper/gh100/dev_fsp_pri.h:26-60`, bound for GH100/GB1xx/GB20x (`gen/g_kern_fsp_nvoc.c:320-400`) | M:GB203 (bw1: 3 replies) |
| SEC2 Booter: CPUCTL 0x840100, MAILBOX0 0x840040, DMATRFCMD 0x840118; Unload iff MAILBOX0 == 0xFF | falcon_gsp.rs:175-178,369-371 | c✓T | c✓T | c✓T | c✓T | · | · | · | `pub/*/dev_sec_pri.h:27`; the Unload convention misses GC6-entry (0xdeaddead) and suspend unloads (`krn/gsp/arch/turing/kernel_gsp_booter_tu102.c:138-165`) | M:GA106 (cap1), AD106 |
| Zero is the right answer: DEBUGINFO (CrashCat), GSP EMEM, DMACTL, VBIOS_SCRATCH FRTS/SB codes, Ada EROT grant, FSP UCODE_VERSION / FUSE_ERROR_CHECK / scratch | not modelled | s | s | s | s | s | s | s | `THE_BAR0_DISPOSITION_MAP.md` §5.1 (DEBUGINFO = 0 is a design obligation pinned only by an old-tree test) | S |
| Zero is wrong but unreached: `NV_PGSP_FALCON_ENGINE` reset status (GH100+ `kflcnReset`), GA100 MMU_LOCK PLM | not modelled | · | d | · | · | d | d | d | `pub/hopper/gh100/dev_gsp.h:32-38`; `pub/ampere/ga100/dev_fb.h:37-44` | S |

### 4.3 The BAR0 trap plane

| item | kayfabe | TU | GA100 | GA10x | AD | GH | GB10x | GB20x | ogkm header per die group | verification |
|---|---|---|---|---|---|---|---|---|---|---|
| PRAMIN 0x700000–0x7FFFFF (disposition A) | kf-trap/src/trappolicy.rs:74-75; memmap.rs:155; kf3.c:220-230 | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | `pub/maxwell/gm107/dev_ram.h:26`, `pub/turing/tu102/dev_ram.h:26`, `pub/hopper/gh100/dev_ram.h:26`, `pub/blackwell/gb202/dev_ram.h:27` | M:GA106 (297 µs worst move) |
| Window-base register: `NV_PBUS_BAR0_WINDOW` 0x1700 (BASE 23:0, TARGET 25:24) / `NV_XAL_EP_BAR0_WINDOW` 0x10FD40 (BASE 21:0 GH; decoded 24:0 on GB); BASE_SHIFT 16 | kf-trap/src/pramin.rs:26-70 | c✓T | c✓T | c✓T | c✓T | c✓T | c✗ | c? | `pub/maxwell/gm107/dev_bus.h:43-50`; `pub/hopper/gh100/pri_nv_xal_ep.h:25-27`; GB10x header 22:0 (`pub/blackwell/gb100/pri_nv_xal_ep.h:40-43`), GB20x **ambiguous** (22:0 vs 21:0); 24:0 is GB10B's — a superset, asserted | M:GA106 (cap3); E2E AD106, GB203 |
| Pre-Hopper L2 ops: `UFLUSH_L2_FLUSH_DIRTY` 0x70010 (PENDING 0:0), VF `L2_SYSMEM/PEERMEM_INVALIDATE` 0xB80F00/F04 → host `FB_FLUSH_GPU_CACHE` | kf-trap/src/cacheop.rs:80-89 | c✓T | c✓T | c✓T | c✓T | · | · | · | `pub/maxwell/gm200/dev_flush.h:47-53`; `pub/turing/tu102/dev_vm.h:28-30` | M:GA106 (w827) |
| `UFLUSH_L2_CLEAN_COMPTAGS` 0x7000C | absent from cacheop.rs:85-89 | d | d | d | d | · | · | · | `pub/maxwell/gm200/dev_flush.h:40`; a PENDING write reads back busy ⇒ 4 s `NV_ERR_TIMEOUT` on a comptag-clean request (`krn/mem_sys/arch/maxwell/kern_mem_sys_gm200.c:150-162`) | S |
| Hopper+ read-started memops: `XAL_EP_UFLUSH_L2_FLUSH_DIRTY` 0x10F810/14, VF `FUNC_L2_SYSMEM/PEERMEM_INVALIDATE` 0xB80F10/14/18/1C, `XAL_EP_UFLUSH_FB_FLUSH` 0x10F800/04; TOKEN 30:0, STATUS 31:31, MAX_OUTSTANDING 140 | cacheop.rs:95-143 | · | · | · | · | c✓T | c✓T | c✓T | `pub/hopper/gh100/pri_nv_xal_ep.h:28-42`, `dev_vm.h:28-39`, `dev_nv_xal_addendum.h:29`; FB_FLUSH is read only on GB110/GB112/GB20x (GH100/GB100/GB102 take the RPC path) | E2E GB203; U |
| `CLEAN_COMPTAGS` token 0x10F808/0C | not in the token set (owner decision) | · | · | · | · | d | d | d | `pub/hopper/gh100/pri_nv_xal_ep.h:43-48` (a silent no-op) | S |
| MMU invalidate: PRIV base = usermode − 0x30000; `PDB` 0x30A0 (ADDR 31:4, APERTURE 1:1, >> 12), `UPPER_PDB` 0x30A4 (19:0), `INVALIDATE` 0x30B0 (ALL_VA 0, ALL_PDB 1, HUBTLB_ONLY 2, REPLAY 5:3, TRIGGER 31) | kf-trap/src/mmuinval.rs:30-131 | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | `pub/turing/tu102/dev_vm.h:120-188`; `pub/blackwell/gb100/dev_vm.h:327-402` | M:GA106 (cap3 `0x80010005`), GB203 |
| CPU interrupt tree: LEAF 0x1000, EN_SET 0x1200, EN_CLEAR 0x1400, TOP 0x1600, TOP_EN 0x1608/10, LEAF_TRIGGER 0x1640 (11:0); `LEAF__SIZE_1` 8 vs 16; loopback vector 129 | kf-trap/src/cpuintr.rs:40-66 | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | `pub/turing/tu102/dev_vm.h:31-67`, `pub/hopper/gh100/dev_vm.h:44`, `pub/blackwell/gb100/dev_vm.h:77-130`, `dev_ctrl.h:35` | M:GA106 (loopback) |
| Usermode window: base 0xBB0000 (page 0 = C passthrough), doorbell +0x90, TIME_0/1 +0x80/84 | kf-trap/src/memmap.rs:143; trappolicy.rs:233; timer.rs:41-42; kf-rm/src/hostquery.rs:471 | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | `pub/turing/tu102/dev_vm.h:224-230`; `cls/clc361.h:31-33` | M:GA106, AD106, GB203 |
| Usermode window **span** — the userspace-mappable arm | kf-qemu/src/device.rs:776-777 (4 KiB) → **fixed `3cb0175a`** (`memmap::VF_USERMODE_LEN` 64 KiB) | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | `DRF_SIZE(NVC361)` (`cls/clc361.h:29-30`), handed to user clients by `kfifoGetUsermodeMapInfo_GV100` (`krn/fifo/arch/volta/kernel_fifo_gv100.c:170-171`). Before the fix: `c✗` on every die group — **security** (§5 D1) | S |
| Hopper+ BAR1 usermode view: leaf aperture SYS_COH (2) + kind SMSKED_MESSAGE (0xF); VF 0x30000–0x3FFFF; PRIV 0x0–0x2FFFF | kf-chip/src/usermode.rs:35-121; kf-trap/src/bar1db.rs; kf3.c:463-501 | · | · | · | · | c✓T | c✓T | c✓T | `pub/hopper/gh100/dev_mmu.h:50,137`, `dev_vm.h:26-27`; `krn/fifo/arch/hopper/kernel_fifo_gh100.c:90-131` | M:GB203 (T0/T1/T2 + negative control) |
| Timer writes refused by name: `NV_PTIMER_TIME_0/1` 0x9400/0x9410 + PLM 0x9430 (LEVEL0 4:4); `NV_PGC6_SCI_SYS_TIMER_OFFSET_0/1` 0x118DF4/F8 | kf-trap/src/timer.rs:100-142 | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | `pub/volta/gv100/dev_timer.h:26-30`; `pub/hopper/gh100/dev_gc6_island.h:35-41` | M:GA106, AD106; E2E GB203 |
| `NV_PGC6_SCI_SEC_TIMER_TIME_0/1` 0x118F54/58 (Hopper+ read side, a live counter) | not modelled (static 0) | · | · | · | · | d | d | d | `pub/hopper/gh100/dev_gc6_island.h:27,31` (0 passes RM's hi-lo-hi loop) | E2E GB203 |
| Read-exit pages: none TU…AD; GH {0x10F000, 0x8F2000, 0xB80000}; Blackwell adds 0x840000 | kf-trap/src/memmap.rs:113-138 | c✓T | c✓T | c✓T | c✓T | c✓T | c✗ | c✗ | 0x840000 is the SEC2 EMEM page of **integrated** GB10B/GB20B (`pub/blackwell/gb20b/dev_sec_pri.h:40`), which are refused — discrete Blackwell pays read exits for nothing | E2E GB203 |
| Doorbell token: VECTOR 11:0 (TU…GH, `& 0xFFF`); Blackwell index = RUNLIST_ID 22:16 << 11 \| chid; RUNLIST_DOORBELL 30:30 = 1 (GB20x) / 22:22 = 0 (GB10x), ignored | kf-trap/src/tokenindex.rs:25-84; device.rs:362-368 | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | `pub/turing/tu102/dev_ctrl.h:36-37`; `pub/blackwell/gb100/dev_vm.h:622-632`; `gb202/dev_vm.h:28-32`; token HALs `gen/g_kernel_fifo_nvoc.c:636-656` | M:GB203 (bws4) |
| Per-runlist chids (`bUsePerRunlistChram`) and `CHID_BITS` 11 = `NV_CHRAM_CHANNEL__SIZE_1` 2048 | kf-chip/src/lib.rs:174-182; tokenindex.rs:36; hostquery.rs:1183 | c✓ | c✓ | c✓ | c✓ | c✓ | c✓T | c✓T | `gen/g_kernel_fifo_nvoc.c:226-236`; `pub/ampere/ga100/dev_runlist.h:27`, `pub/blackwell/gb202/dev_runlist.h:27` | M:GB203 |
| Fault-buffer registers (VF REPLAYABLE/NON_REPLAYABLE LO/HI/GET/PUT/SIZE, 0xB83000+; GET(1) 0xB83028, PUT(1) 0xB8302C) | not modelled (delivery unbuilt, `kf-abi/src/faultbuffer.rs:118-129`) | d | d | d | d | d | d | d | `pub/turing/tu102/dev_vm.h:72-112`; `pub/blackwell/gb100/dev_vm.h:146-196` — PUT never moves: a real replayable fault would hang UVM | inert path measured on the C artifact (cap3) |
| Access-counter `NOTIFY_BUFFER_SIZE` 0xB83110 = 256 entries | bar0.rs:87-89,117-119,169-174 | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | `pub/turing/tu102/dev_vm.h:209`; GB100/GB110 also have counter 1 at 0xB83210 (`pub/blackwell/gb100/dev_vm.h:511`, `gen/g_uvm_nvoc.c:266-270`), **latent**: UVM 580 clamps `accessCntrBufferCount` to 1 (`uvm/uvm_gpu.c:2936-2938`, WAR bug 5262806) | M (UVM up on GA106, AD106, GB203) |
| VF `BAR1_BLOCK` / `BAR2_BLOCK` binds | not modelled (roots are declared in the static config) | · | · | · | · | · | · | · | `pub/turing/tu102/dev_vm.h:231-272`; `pub/hopper/gh100/dev_vm.h:68-109` — a GSP client never writes them | S |
| RUNLIST / CHRAM PRI bases (authored 0xC00000 + 0x400·rl, 0xC20000 + 0x2000·rl) | kf-rm/src/authored.rs:555-559 | c? | c? | c? | c? | c? | c? | c? | UVM dereferences them as BAR0 (`krn/rmapi/nv_gpu_ops.c:10258-10280` → `uvm/uvm_ampere_host.c:132-142`, CLEAR_FAULTED); the range is plain shadow ⇒ **latent** until faults are delivered | S |

### 4.4 Page tables and the memory plane

| item | kayfabe | TU | GA100 | GA10x | AD | GH | GB10x | GB20x | ogkm header per die group | verification |
|---|---|---|---|---|---|---|---|---|---|---|
| MMU format per family: VER2 (TU…AD), VER3 (GH, GB) | kf-chip/src/lib.rs:153-160; kf-qemu/src/device.rs:260-263 | c✓ | c✓ | c✓ | c✓ | c✓ | c✓ | c✓ | `krn/mmu/arch/pascal/kern_gmmu_gp100.c:39-42`; `krn/mmu/arch/hopper/kern_gmmu_gh100.c:44-47` | M:GA106, AD106 (VER2); GB203 (VER3) |
| VER2 level geometry: PD3[48:47]×4 → PD2[46:38] → PD1[37:29] (512 MiB leaf) → PD0[28:21] dual (2 MiB leaf) → PT 64K[20:16]×32 / 4K[20:12]×512; 49-bit VA | kf-cuda/src/abi.rs:559-684; cuda/walk/kf_walk.cu:1325-1371 | c✗ | c✓ | c✓ | c✓ | · | · | · | `krn/mmu/arch/pascal/kern_gmmu_fmt_gp10x.c:58-104`, `ampere/kern_gmmu_fmt_ga10x.c:46-56`; Turing binds `_GP10X` (no PD1 leaf; 512M unsupported, `gen/g_kern_gmmu_nvoc.c:302-312,679-686`) — reachable only by hostile tables | M:GA106 (suite + 7 005-leaf corpus) |
| VER3 level geometry: PD4[56]×2 → PD3[55:47] → PD2[46:38] → PD1 (512 MiB) → PD0 dual (2 MiB) → PT; 57-bit VA | abi.rs:697-727; kf_walk.cu:1390-1427 | · | · | · | · | c✓ | c✗ | c✓ | `krn/mmu/arch/hopper/kern_gmmu_fmt_gh10x.c:53-118`; GB10x makes PD2 a **256 GiB** leaf (`krn/mmu/arch/blackwell/kern_gmmu_fmt_gb10x.c:51-61`, `bPageSize256gbSupported` `gen/g_kern_gmmu_nvoc.c:314-324`) — not decoded: a vidmem 256G PTE reads as PDE INVALID and is skipped silently | M:GB203; S (GH, GB10x) |
| VER2 PTE/PDE/dual-PDE fields: VALID 0; APERTURE 2:1 + codes; VOL 3, PRIVILEGE 5, READ_ONLY 6, ATOMIC_DISABLE 7; ADDRESS_VID 32:8 / SYS 53:8, << 12; dual big half 32:4 / 53:4, << 8, small half in the high word; KIND 63:56; sizes 8 / 16 | abi.rs:559-684; kf_walk.cu:1325-1371 (+ `synth.rs`, `kf_tables.h`, `kf_real_tables.py`) | c✓T | c✓T | c✓T | c✓T | · | · | · | `pub/pascal/gp100/dev_mmu.h`, `pub/turing/tu102/dev_mmu.h`, `hw/turing/tu102/dev_mmu.h` | M:GA106 corpus; T (`kf-mem/tests/walk_format_vs_ogkm.rs`) |
| VER3 PTE/PDE fields: VALID 0; APERTURE 2:1; ADDRESS 51:12 << 12 (one field); PCF 7:3 (UNCACHED/PRIVILEGE/RO/NO_ATOMIC = enumerant bits 0-3), SPARSE 1; KIND 11:8; dual big 51:8 << 8 | abi.rs:697-727; kf_walk.cu:1390-1427 | · | · | · | · | c✓T | c✓T | c✓T | `pub/hopper/gh100/dev_mmu.h:53-188` (gb100 has none; every GB die binds the GH10X PTE HALs) | M:GB203; T |
| VER3 PDE PCF 5:3; sparse = 1 **or** 3 | kf_walk.cu:316-320 reads the PTE's 7:3 == 1 | · | · | · | · | c✗ | c✗ | c✗ | `pub/hopper/gh100/dev_mmu.h:64-76`; RM's PD2 true-sparse fill writes 3 (`krn/mmu/arch/hopper/kern_gmmu_gh100.c:1029-1037`) — **ratchet test** | S |
| VER3 "unmapped big PTE" (PCF = NO_VALID_4KB_PAGE 3) hides the 16 small PTEs | kf_walk.cu:359-364 (VER2 only) | · | · | · | · | d | d | d | `pub/hopper/gh100/dev_mmu.h:143`; UVM writes it (`uvm/uvm_hopper_mmu.c:239-251`) — stale 4 KiB leaves read as live | S |
| 128 KiB big pages (`NV_VASPACE_FLAGS_BIG_PAGE_SIZE_128K`) | not modelled, not refused | d | d | d | d | d | d | d | `krn/mmu/kern_gmmu.c:620-646`; `gpu_vaspace.c:535-547` (UVM refuses 128K) | S |
| PEER-aperture leaves; sysmem page tables and roots | refused (`KFWR_R_LEAF_OOB`, `FOREIGN_AP`, `SetPageDirRootAperture`) | d | d | d | d | d | d | d | legal per `pub/maxwell/gm107/dev_mmu.h:29-30`, `pub/hopper/gh100/dev_mmu.h:62-63` | R |
| PTE kind values and the compressible → uncompressed map | kf-chip/src/ptekind.rs:18-56 | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | c✗ | kind values identical on every die group (`pub/turing/tu102/dev_mmu.h:97-112`, `pub/blackwell/gb202/dev_mmu.h:26-41`); the **rule** differs on GB20x: every non-PITCH kind → GENERIC and no Z/S kinds (`krn/mem_mgr/arch/blackwell/mem_mgr_gb202.c:81-93`, `pub/blackwell/gb202/kind_macros.h:31`) — latent (a GB20x guest cannot emit them) | M:GA106 corpus, GA102 (Xid 13 fix) |
| SMSKED_MESSAGE (0xF) + SYS_COH leaf = the usermode page (internal MMIO) | kf-chip/src/usermode.rs:35-121; kf-mem/src/apply.rs:235-236 | · | · | · | · | c✓T | c✓T | c✓T | `pub/hopper/gh100/dev_mmu.h:50,137`; `pub/blackwell/gb202/dev_mmu.h:41` | M:GB203 |
| BAR1/BAR2 root entry (`UPDATE_BAR_PDE`, fn 70) copied verbatim into our 4 KiB root; entryLevelShift not checked | kf-rm/src/barpde.rs:27-100; kf-qemu/src/mem.rs:1513-1526 | c? | c? | c? | c? | c? | c? | c? | expected root shift 47 (`kern_gmmu_fmt_gp10x.c:59-60`) / 56 (`kern_gmmu_fmt_gh10x.c:63-64`); `krn/bus/kern_bus.c:826-880` | M:GA106 (0x2efbc302, shift 47) |
| Instance-block / subcontext PDB fields (`NV_RAMIN_*`) | not read (roots arrive by RPC) | · | · | · | · | · | · | · | `pub/pascal/gp100/dev_ram.h:30-50`; `pub/volta/gv100/dev_ram.h:29-63` | — |
| Host map emission: 4 KiB host PTEs for store slices; kind override = uncompressed kind; guest RO/VOL/PRIV/ATOMIC_DISABLE decoded but dropped | kf-host/src/lib.rs:851-896; kf-abi/src/bringup.rs:716-757; kf-mem/src/ledger.rs:14-25 | d | d | d | d | d | d | d | `nvos.h:2036-2177`; a guest read-only mapping becomes host read-write | M:GA106, GA102 (page size, kind) |

### 4.5 Channels, pushbuffers, copy engines, completions

| item | kayfabe | TU | GA100 | GA10x | AD | GH | GB10x | GB20x | ogkm header per class | verification |
|---|---|---|---|---|---|---|---|---|---|---|
| Classes bound (channel / CE / compute / usermode) = newest of (generated family set ∩ host class list): C46F/C5B5/C5C0/C461 · C56F/C6B5/C6C0/C561 · C56F/C7B5/C7C0/C561 · C56F/C7B5/C9C0/C561 · C86F/C8B5/CBC0/C661 · C96F/C9B5/CDC0/C661 · CA6F/CAB5/CEC0/C761 | kf-chip/src/host_classes.rs:61-72; classes.rs:141-210 | a | a | a | a | a | a | a | `gen/g_gpu_class_list.c`; ⚠ the sets are generated from ogkm **610.43.02**, not the 580 the bench runs (every 580 set is contained) | M:GA106, AD106, GB203 |
| GPFIFO entry: 8 B; GET 31:2, GET_HI 7:0 (≤ 2^40), LENGTH 30:10 dwords, FETCH 0, LEVEL 9, SYNC 31 (WAIT refused) | kf-abi/src/submit.rs:1838-1945; kf-chan/src/ring.rs:164-176 | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | `cls/clc46f.h:264-282` … `cls/clca6f.h:52-71` | M:GA104, GB203 (gates 3-6) |
| GP control opcode `SET_PB_SEGMENT_EXTENDED_BASE` (4; pushbuffer VA ≥ 2^40) | ring.rs:168-170 swallows it as a control entry | · | · | · | · | d | d | d | `cls/clc86f.h:175,189`; UVM emits it at every channel init (`uvm/uvm_channel.c:2525-2544`) — latent while the base is below 2^40 (it is on GB203) | S |
| USERD: GP_PUT 0x8C, GP_GET 0x88 (absent from C96F/CA6F), size/alignment 512 | submit.rs:1826-1894; kf-host/src/channel.rs:37-40; kf-qemu/src/chan.rs:255-258 | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | `Nv*6fControl` (`offsetof`); `pub/maxwell/gm107/dev_ram.h:47-50`; on GB kayfabe writes GP_GET into an `Ignored` word for Translated rings (harmless) | M:GA104, GB203 |
| Method header: SEC_OP 31:29 (all 8 forms), ADDRESS 11:0 (× 4), SUBCHANNEL 15:13, COUNT 28:16 | submit.rs:2113-2322 | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | `cls/clc56f.h:286-348` (C86F+ headers state no DMA_* format; they use this one) | M:GA106, AD106, GB203; T |
| Legacy GRP2 non-incrementing header | submit.rs:2308-2310 + kf-chan/src/translated.rs:237-247 expand it as incrementing | c✗ | c✗ | c✗ | c✗ | c✗ | c✗ | c✗ | `cls/clc56f.h:296` (`TERT_OP_GRP2_NON_INC_METHOD`) — no stock emitter | S |
| Host semaphore release: SEM_ADDR_LO/HI 0x5C/0x60, PAYLOAD_LO/HI 0x64/0x68, EXECUTE 0x6C (OPERATION 2:0 = RELEASE, RELEASE_WFI 20:20, PAYLOAD_SIZE 24:24); NON_STALL_INTERRUPT 0x20 | submit.rs:2344-2447; kf-chan/src/host.rs:37-51 | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | `cls/clc46f.h:201-226` … `clca6f.h:36-49` (SEM_ADDR_HI is 24:0 from C86F; our 8-bit mask is a subset) | M:GA104, GB203 (gate 1) |
| MEM_OP_A..D 0x28-0x34; OPERATION 31:27: TLB_INVALIDATE 9/0xA → split, MEMBAR 5 and L2 {0xD,0xE,0xF,0x10,0x11,0x15} forwarded, ACCESS_COUNTER_CLR 0x16 consumed; PDB 31:12 \| 26:0 | kf-chan/src/translated.rs:34-73,334-377 | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | `cls/clc56f.h:119-194`, `clc86f.h:53-129`, `clc96f.h:36-73` (0x11 exists only in C96F) | M:GB203 (split walked); T |
| MEM_OP_C `TLB_INVALIDATE_PDB_APERTURE` 11:10 | ignored (PDB always an FB address) | d | d | d | d | d | d | d | `cls/clc56f.h:174-177`; a sysmem PDB would be walked as FB | S |
| CE methods: LAUNCH_DMA 0x300 (TRANSFER 1:0, FLUSH 2, SEMAPHORE_TYPE 4:3, INTERRUPT 6:5, LAYOUT 7/8, MULTI_LINE 9, REMAP 10, SRC/DST_TYPE 12/13), OFFSET_IN/OUT 0x400-0x40C, LINE_LENGTH/COUNT, PITCH_IN/OUT, PHYS_MODE 0x260/0x264 (TARGET 1:0), SET_SEMAPHORE_A/B/PAYLOAD, REMAP 0x700-0x708 | kf-abi/src/submit.rs:2534-2860; translated.rs:399-560 | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | `cls/clc5b5.h`, `clc6b5.h`, `clc7b5.h`, `clc8b5.h` (C9B5/CAB5 inherit C8B5) | M:GA104, GB203 (gate 3) |
| `OFFSET_*_UPPER` width 16:0 (≤ C7B5) / 24:0 (≥ C8B5) | translated.rs:466-477 (`upper_mask`) | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | `cls/clc7b5.h:161-166`; `clc8b5.h:91-96` | M:GB203 (bws4 Xid 31 fix) |
| LAUNCH_DMA bit 23: MEMORY_SCRUB_ENABLE (≥ C8B5) → virtual zero-fill; VPRMODE 23:22 below | translated.rs:480-520 | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | `cls/clc8b5.h:84-86`; `clc7b5.h` VPRMODE | M:GB203 (Xid 71 fix) |
| CAB5 `DATA_TRANSFER_TYPE_PREFETCH` (3) | treated as a copy (translated.rs:439-441) | · | · | · | · | · | · | c✗ | `cls/clcab5.h:34` | S |
| PHYS_MODE `PEER_ID` 8:6 / `FLA` 9:9 | ignored (translated.rs:88-97) | · | d | d | d | d | d | d | `cls/clc6b5.h:62-64`, `clc7b5.h:72-74`, `clc8b5.h:37-38` | S |
| Compute class, gate 6 only: I2M 0x180-0x1B4, SET_REPORT_SEMAPHORE_A..D 0x1B00 | kf-harness/src/bin/kf-gate6.rs:159-166 | c✓ | c✓ | c✓ | c? | c? | c? | c? | `cls/clc5c0.h`, `clc6c0.h`, `clc7c0.h`; C9C0/CBC0/CEC0 are id-only headers in 580 | M:GA104, GB203 (gate 6) |
| Notifier indices: FIFO_EVENT_MTHD 35, GR0 12, CE(n) = 23+n (n < 10) / **166**+n−10, NVENC(n), NVDEC(n) | kf-host/src/event.rs:57-80; kf-chan/src/host.rs:321 | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | `cls/cl2080_notification.h:48-258` (driver ABI); CE10 was 184 = GSP_PERF_TRACE (`:225`) — GB10x `c✗` before **fix `8a669b81`** | M:GA104, GB203 (gate 1, CE0-9) |
| `NvNotification` (16 B: timestamp 0, info32 8, info16 12, status 14) | kf-abi/src/notifier.rs:69-90; kf-qemu/src/chan.rs:2098-2119 | c✓ | c✓ | c✓ | c✓ | c✓ | c✓ | c✓ | `nvgputypes.h:57-64` (driver ABI; offsets in the table) | S |
| Subchannels: harness CE on 4; Translated 0-4 = CE, 5-7 software (must be bound); compute on 1 | kf-harness/src/lib.rs:40; translated.rs:298,384-396 | c✓ | c✓ | c✓ | c✓ | c✓ | c✓ | c✓ | `cls/cla06fsubch.h:27-31` | M:GA106, GA104, GB203 |
| Host work-submit tokens (twins and rings) and the doorbell store at usermode + 0x90 | kf-host/src/channel.rs:502-520; kf-host/src/lib.rs:625-635 | b | b | b | b | b | b | b | token HALs `krn/fifo/arch/*/kernel_fifo_*.c` | M:GA104 (0x4, 0x18), GB203 (0x40000002) |
| Gates 3/4 `is_ce` literal {C6B5, C7B5, C8B5, C9B5, CAB5} | kf-harness/src/bin/kf-gate3.rs:173-175; kf-gate4.rs:192-194 | c✗ | c✓ | c✓ | c✓ | c✓ | c✓ | c✓ | misses Turing's C5B5 (`gen/g_gpu_class_list.c:145`) — harness only | M:GA104, GB203 (gates) |

### 4.6 Engine, fault and interrupt topology kayfabe authors

| item | kayfabe | TU | GA100 | GA10x | AD | GH | GB10x | GB20x | ogkm header per die group | verification |
|---|---|---|---|---|---|---|---|---|---|---|
| MMU fault ids GRAPHICS / CE0 (+i) / HOST0: 64 / 15 / 0x20 (TU…AD), 384 / 43 / 64 (GH), 384 / 65 / 85 (GB) | kf-rm/src/authored.rs:364-384 | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | `hw/turing/tu102/dev_fault.h`, `pub/ampere/ga100/dev_fault.h`, `pub/hopper/gh100/dev_fault.h:27,39` + `hw/hopper/gh100/dev_fault.h:83`, `pub/blackwell/gb100|gb202/dev_fault.h` — GH was **refused by name** ("no HOST0 in the tree") before **fix `1e37217d`** | M:GA106 capture, GB203; GH S |
| NVENC / NVDEC fault ids: TU enc 11-13, dec 10, 25, 26; GA/AD enc 11-13, dec 25-29; GH enc 35-37, dec 19-26; GB enc 44-47, dec 28-35 | authored.rs:337-362 | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | c✓T | same headers (`hw/hopper/gh100/dev_fault.h:49-56,78-80` for GH, added by the same fix) | M:GA106 (NVENC0 0xB, NVDEC0 0x19) |
| NVJPG / OFA engines | not advertised | d | d | d | d | d | d | d | `pub/ampere/ga100/dev_fault.h:40,57`, `pub/ada/ad102/dev_fault.h:40-43` | M:GA104 (the guest lacks OFA0) |
| PBDMA ids and PBDMA fault ids (GR {0,1}; GRCE k → k mod 2; async/video → runlist + 1; fault id = HOST0 + id) | authored.rs:506-562 | c? | c? | c? | c? | c? | c✗ | c✗ | Blackwell: RM expects both GR entries equal and reserves base…base+63 (`krn/fifo/arch/blackwell/kernel_fifo_gb100.c:58-69`, `pub/blackwell/gb100/hwproject.h:27`) — kayfabe's 85/86 and CE/video ids from 87 fall inside | GA106 stated divergences; GB203 boots |
| Runlist ids, RESET bits (GR 12, CE 2+i / 3+i, video 23…31), INTR/RC_MASK 0, DEV_TYPE (GR 0, LCE 0x13, NVENC 0x0E, NVDEC 0x10), RUNLIST_ENGINE_ID = GRCE ordinal | authored.rs:320-340,473-562 | c? | c? | c? | c? | c? | c? | c✗ | only LCE 0x13 is in a header (`pub/ampere/ga100/dev_top.h:34`); GB20x can have 4 GRCEs but `RLENG_ID` is 2 bits (`pub/blackwell/gb100/dev_top.h:50`, `krn/ce/arch/blackwell/kernel_ce_gb202.c:36-37`) | M:GA106 capture (stated divergences) |
| devicePriBase: GR 0x400000, LCE 0x104000, video = host falcon registerBase | kf-rm/src/hostquery.rs:257-279 | c? | c✓ | c✓ | c? | c? | c✓ | c✓ | `pub/ampere/ga100|ga102/dev_ce.h:26`, `pub/blackwell/gb202/dev_ce.h:26` (no witness in tu102/ad102/gh100) | M:GA106 capture |
| GSP / DISP stall vectors 0x9B / 0x9A | authored.rs:132-166 | c? | c? | c? | c? | c? | c✓ | c? | only `pub/blackwell/gb100/dev_vm.h:123-124` states them (154/155) | M:GA106 capture |
| Engine non-stall vectors (GR0 → 0; async CE/video → its runlist) | authored.rs:184-240 | c? | c? | c? | c? | c? | c? | c? | not checked against the host's subtree map (`ctrl2080mc.h:326-336`) | GA106 capture differs |
| Static interrupt rows and category → subtree map | kf-rm/src/hostquery.rs:428-458 | b | b | b | b | b | b | b | `MC_GET_STATIC_INTR_TABLE` 0x2080170e, `…_SUBTREE_MAP` 0x2080170f | boots only (never captured) |
| GMMU fault-buffer sizes 0x31000 / 0x120C20; 32-byte packet | authored.rs:71-87; kf-abi/src/gmmustatic.rs:77-98 | c? | c? | c? | c? | c? | c? | c? | packet = `NVC369_BUF_SIZE` 32 (`cls/clc369.h:35`); sizes are GA106's reset read-back, stated in no header | GA106 capture |
| Fault-buffer entry format (`NVC369_BUF_ENTRY_*`: INST 9:8 / 31:12, ADDR, TIMESTAMP, ENGINE_ID mw 200:192, FAULT_TYPE, CLIENT, ACCESS_TYPE, GPC_ID …) | not written, not decoded (delivery unbuilt) | d | d | d | d | d | d | d | `cls/clc369.h:34-67` (in the table as `mw:` rows) | — |
| Fault type in `RC_TRIGGERED` (literal 0) | kf-qemu/src/device.rs:1606-1609 | c? | c? | c? | c? | c? | c? | c? | `pub/volta/gv100/dev_fault.h:47-62` | U |
| CE fault-method buffer size 0x5000 | authored.rs:52-64 | c? | c? | c✓ | c? | c? | c? | c? | no header (GSP firmware); GA106 measured 20480 | M:GA106 |
| Channels per runlist 0x800 | hostquery.rs:1183 | c? | c✓ | c✓ | c? | c? | c? | c✓ | `NV_CHRAM_CHANNEL__SIZE_1` in `pub/ampere/ga100/dev_runlist.h:27`, `pub/blackwell/gb202/dev_runlist.h:27` only | M:GA106 capture |

### 4.7 Per-die facts kayfabe reads from the host (all NON_PRIVILEGED unless noted)

⊘ Every fact below is `b` on every die group; the columns are dropped. "Refusal" is what realize does when
the host does not answer.

| fact | host control (cmd) / sysfs | asked at | served at | refusal | verification |
|---|---|---|---|---|---|
| arch / impl / revision → family, PMC_BOOT_0/42, chip sub-revision | `MC_GET_ARCH_INFO` 0x20801701 | kf-host/src/lib.rs:486-494; hostquery.rs:193-196 | kf-chip/src/lib.rs:128-145; bar0.rs:51-66 | realize refused | M:GA106 {0x170,6,0xA1}, AD106 0x190/6, GB203 0x1B0 |
| PCI identity; BAR0 and host BAR1 sizes; PCIe max speed and width | sysfs `vendor`/`device`/`subsystem_*`/`class`/`revision`/`resource`/`max_link_*` | kf-qemu/src/hostfacts.rs:46-105 | kf3.c:617-623; bar0.rs:75-80; cardbudget.rs:54-90 | realize refused (BAR0 0 → 16 MiB fallback; Gen6 unmapped) | M |
| die PCIe generation | `BUS_GET_INFO_V2` 0x20801823 [0x2d] | hostquery.rs:1067-1071 | inittables.rs:2343-2357 | realize refused | M:GA106 |
| class list | NV0080 `GPU_GET_CLASSLIST_V2` 0x00800292 | kf-host/src/lib.rs:500-507 | host_classes.rs:61-72 | `HostLacksKind` | M:GA106, AD106, GB203 |
| engine types and counts | `GPU_GET_ENGINES_V2` 0x20800170 | hostquery.rs:236-245 | authored.rs:468-580 → inittables.rs | realize refused (MIG GR1+ refused) | M:GA106, GA104, AD106, GB203 |
| video falcons (engDesc, registerBase, ctx size) | `GPU_GET_CONSTRUCTED_FALCON_INFO` 0x208001b0 | hostquery.rs:310-318 | inittables.rs:1792-1800 | no video engine advertised | M:GA106 (NVENC0 0x1C8000, NVDEC0 0x848000) |
| LCE present, per-LCE caps, GRCE set | `CE_GET_ALL_CAPS` 0x20802a0a | hostquery.rs:1127-1131 | kf-abi/src/cecaps.rs:343-465 | realize refused | M:GA106 (0x0F), GB203 ({0,1,4,5}) |
| GRCE bit per CE (event plane) | `CE_GET_CAPS_V2` 0x20802a03 | kf-host/src/channel.rs:801-806 | kf-qemu/src/chan.rs:629-651 | that CE gets no event | boots |
| LCE → PCE masks | `CE_GET_CE_PCE_MASK` 0x20802a02 | hostquery.rs:380-417 | inittables.rs:2492-2506 | LCE0 refused → realize refused; a hole is served as `NO_PCE_MASK` | M:GA106, GB203 |
| static interrupt rows; category → subtree map | 0x2080170e; 0x2080170f | hostquery.rs:428-458 | inittables.rs:1728-1734 | realize refused | boots only |
| CMP SKU, SMC mode, GPU info 0x11 | `GPU_GET_INFO_V2` 0x20800102 | hostquery.rs:462-487,1006-1061 | inittables.rs | realize refused | M:GA106 |
| GR litters (58), GPC mask, TPC mask per GPC, ZCULL mask, TPC count per logical GPC, logical → physical map, PES per GPC, gfx GPC/TPC, SM order, GR caps, PPC/ROP/syspipe | 0x20801228, 0x2080122a, 0x2080122b, 0x20801237, 0x20801234, `GRMGR_GET_GR_FS_INFO` 0x20803801, 0x20800168, 0x20801239, 0x2080121b, 0x20801227 | hostquery.rs:496-767 | inittables.rs:2187-2212,2526-2542 | realize refused; map → rule `derive_chiplet_gpc_map` (a); PES → litter (a); physGpcMask (0x20801232 is PRIVILEGED) := gpcMask (a) | M:GA106, GA104, AD104, AD102 (`V3_FLOORSWEPT_GR.md`) |
| GR context buffer sizes; zcull info; SM issue-rate modifier; ZBC ranges | 0x2080122d; 0x20801206; 0x20801230; NV9096 0x90960106 | hostquery.rs:791-997 | inittables.rs:2226-2284 | per id ABSENT / None (guest refused) | GB203 (issue rate); others boots only |
| FB / L2 / LTC / FBP / bus width; cache info | `FB_GET_INFO_V2` 0x20801303; 0x20801315 | hostquery.rs:512-536,815-851,1022-1053 | inittables.rs:1812-1817,2252-2255,2397-2417 | realize refused / None | M:GA106, AD106 (bus 128, LTS 16) |
| GSP features; GPU name; C2C; VBIOS version; perf levels; NVD/GPC clocks; NVENC/NVDEC caps | 0x20803601; 0x20800110/111; 0x2080182b; `BIOS_GET_INFO_V2` 0x20800810; 0x2080200b; GSS-legacy 0x20809064/0x2080a028; 0x00801b02/0x00801c02 | hostquery.rs:322-360,1077-1166 | inittables.rs; staticinfo.rs; bar0.rs:325-340 | refused → None (guest refused) or realize refused | M:GA106 |
| **Not asked** (privileged or kernel-only), authored instead (§4.6): engine runlist PRI base, FIFO device info table, CE fault-method buffer size, engine notification vectors, physGpcMask | 0x20800179 (PRIVILEGED), 0x20801112 (kernel), 0x20802a08 (kernel), 0x2080170d (NOT_SUPPORTED to usermode), 0x20801232 (PRIVILEGED) | — | — | — | — |

### 4.8 Firmware protocol on the boot path (NOT hardware — listed so the boundary is complete)

LibOS region entry `{id8 @0, pa @8, size @16}` stride 32, ≤ 4096 entries (`libos_init_args.h:31,49-56`);
`"RMARGS"` id8; boot-args delivery (falcon: MAILBOX0/1; FSP: COT → `GSP_FMC_BOOT_PARAMS` → `bootArgsOffset`
+48, `gspifpub.h:47-120`; COT `gspBootArgsSysmemOffset` at packet byte 860, `kern_fsp_cot_payload.h:27-55`);
MCTP transport/message header and NVDM types COT 0x14 / FSP_RESPONSE 0x15 (`fsp_mctp_format.h:34-53`,
`fsp_nvdm_format.h:36-50`); the suspend sentinel MAILBOX0 == 0x80000000 (exact compare at 580); the VBIOS
container (generated, `kf-abi/src/generated/vbios.rs`). All correct against ogkm-580 where checked
(`M:GB203` for the FSP path, `M:GA106` cap1 for the falcon path), except one **test label**:
`kf-chip/tests/fsp_cot_sequence.rs:45-47` calls 0x18/0x1A "CAPS_QUERY/CLOCK_BOOST" (they are SMBPBI/ROMREAD;
CAPS_QUERY is 0x16, CLOCK_BOOST 0x20). The GSP message queue and RPC formats are the driver boundary and
out of scope here.

## 5. Discrepancies

### 5.1 Fixed on this branch (each trivially provable from source; separate commit + test)

| # | commit | defect | evidence (kayfabe ↔ ogkm-580) | behaviour change |
|---|---|---|---|---|
| D1 | `3cb0175a` | **Security — the usermode window's span.** Only its first 4 KiB was classified `UserspaceMappable`; BAR0 writes to 0xBB1000–0xBBFFFF entered the privileged arm (shadow store + privileged-ring push; ring full ⇒ `PoisonDevice`) | `kf-qemu/src/device.rs:776-777` ↔ `kfifoGetUsermodeMapInfo_GV100` hands a user client `DRF_SIZE(NVC361)` = 64 KiB (`krn/fifo/arch/volta/kernel_fifo_gv100.c:170-171`, every family); the design's own text (`kf-trap/src/trap.rs:17-20`) and the BAR1 path (`device.rs:892`) already said 64 KiB | only hostile writes to pages 1–15 (they now do nothing); no register of the window lies past +0x94 on any die group — the test scans every `NV_VIRTUAL_FUNCTION_*` offset to prove it |
| D2 | `1e37217d` | **Hopper refused by a false premise.** `authored::fault_ids` refused GH100's whole engine table: *"no `HOST0` anywhere in the tree"* | `kf-rm/src/authored.rs:364-380` ↔ `hw/hopper/gh100/dev_fault.h:83` (`HOST0` = 64; HOST0-44 = 64-108, asserted by `uvm/uvm_hopper_fault_buffer.c:77`); NVDEC0-7 = 19-26, NVENC0-2 = 35-37 (`:49-56,78-80`). The published `gh100/dev_fault.h` lacks them; UVM's copy does not | GH100 only: realize proceeds past the engine table (hardware-unverified) |
| D3 | `8a669b81` | `NV2080_NOTIFIERS_CE10` = 184 | `kf-host/src/event.rs:59` ↔ `cls/cl2080_notification.h:207` (166; 184 is `GSP_PERF_TRACE`, `:225`) | only hosts with > 10 CEs (GB100/GB110): COPY10-19 non-stall events now armed on CE10-19 |
| D4 | `e47b30e5` | GB10B not refused as integrated | `kf-chip/src/lib.rs:129-133` ↔ `pub/nv_arch.h:111`, `gen/g_hal_archimpl.h:88` (`ctrl2080mc.h` omits it) | arch 0x1A0 impl 0xB now refused by name |

### 5.2 Listed, not changed (ranked by impact)

| # | defect | die groups | evidence | why not fixed here |
|---|---|---|---|---|
| L1 | **WPR2 ignores RM's WPR-end margin.** `gsp_fw_wpr_end_for` assumes margin 0; `kgspGetWprEndMargin` is non-zero once `bootAttempts > 0`, which any failed GSP bootstrap increments ⇒ every retry fails the exact `WPR2_LO` compare | TU, GA10x, AD (GA100 refused) | `kf-chip/src/falcon_gsp.rs:248-266` ↔ `krn/gsp/arch/turing/kernel_gsp_tu102.c:776`, `krn/gsp/kernel_gsp.c:3908-3921,5637-5697`, `kernel_gsp_frts_tu102.c:514-524` | GA10x behaviour; fix = take `frtsOffset` from the guest's own FRTS command (`frtsRegionOffset4K`) |
| L2 | **GB100/GB102 read PCIe config by walking the capability list**; kf3 is conventional PCI with MSI-X only ⇒ link caps unreadable, the pre-bws1 *"Unknown PCIe speed"* failure | GB10x (GB100, GB102) | `kf3.c:660,766`; `bar0.rs:283` ↔ `krn/arch/blackwell/kern_gpu_gb100.c:359-393`, `gen/g_gpu_nvoc.c:1226-1235` | a kf3 ABI change (a PCIe capability in config space) |
| L3 | **No fault delivery**: fault-buffer PUT never moves, entries (`NVC369_BUF_ENTRY_*`) never written | all | `kf-abi/src/faultbuffer.rs:118-129` ↔ `pub/*/dev_vm.h` fault-buffer block, `cls/clc369.h:34-67` | a design item (`V3_UVM_DEMAND_PAGING.md`) |
| L4 | VER3 "unmapped big PTE" (PCF `NO_VALID_4KB_PAGE`) not honoured — stale 4 KiB leaves read as live (the VER2 ct4 bug class) | GH, GB10x, GB20x | `cuda/walk/kf_walk.cu:359-364` ↔ `pub/hopper/gh100/dev_mmu.h:143`, `uvm/uvm_hopper_mmu.c:239-251` | walk-kernel (PTX) change |
| L5 | GB10x PD2 256 GiB leaf not decoded — a vidmem 256G PTE is skipped silently | GB10x | `kf-cuda/src/abi.rs:711`, `kf_walk.cu:1398` ↔ `krn/mmu/arch/blackwell/kern_gmmu_fmt_gb10x.c:51-61`, `gen/g_kern_gmmu_nvoc.c:314-324` | walk-kernel change |
| L6 | Host maps drop the guest's RO/VOL/PRIV/ATOMIC_DISABLE — a guest read-only GPU mapping is host read-write (guest-internal isolation) | all | `kf-mem/src/ledger.rs:14-25`, `kf-cuda/src/diffmodel.rs:52-53` ↔ `pub/pascal/gp100/dev_mmu.h:124-136`, `pub/hopper/gh100/dev_mmu.h:144-175` | host map verb + diff protocol change |
| L7 | Blackwell GR PBDMA fault ids 85/86 (RM expects both equal and reserves base…base+63; CE/video ids from 87 fall inside); GB20x `RLENG_ID` is 2 bits for up to 4 GRCEs | GB10x, GB20x | `kf-rm/src/authored.rs:506-562` ↔ `krn/fifo/arch/blackwell/kernel_fifo_gb100.c:58-69`, `pub/blackwell/gb100/dev_top.h:50` | GB20x (measured) behaviour |
| L8 | Hopper+ `SET_PB_SEGMENT_EXTENDED_BASE` GP entry swallowed as a control entry | GH, GB10x, GB20x | `kf-chan/src/ring.rs:168-170` ↔ `cls/clc86f.h:175,189`, `uvm/uvm_channel.c:2525-2544` | latent while the pushbuffer base < 2^40; GB20x behaviour |
| L9 | MEM_OP `TLB_INVALIDATE_PDB_APERTURE` ignored — a sysmem PDB walked as FB | all | `kf-chan/src/translated.rs:354-358` ↔ `cls/clc56f.h:174-177` | GA10x behaviour |
| L10 | `UFLUSH_L2_CLEAN_COMPTAGS` 0x7000C not modelled (a request reads busy ⇒ 4 s timeout); Hopper+ `CLEAN_COMPTAGS` token a silent no-op (owner decision) | TU…AD; GH, GB | `kf-trap/src/cacheop.rs:85-89,125-127` ↔ `pub/maxwell/gm200/dev_flush.h:40`, `pub/hopper/gh100/pri_nv_xal_ep.h:43-48` | no unprivileged host verb cleans comptags |
| L11 | RUNLIST/CHRAM PRI bases (authored) are dereferenced as BAR0 by UVM's fault recovery; the range is plain shadow | all | `authored.rs:555-559` ↔ `krn/rmapi/nv_gpu_ops.c:10258-10280`, `uvm/uvm_ampere_host.c:132-142` | latent until L3 |
| L12 | GB100/GB110 access counter 1 (`0xB83210`) unserved | GB10x | `bar0.rs:169-174` ↔ `pub/blackwell/gb100/dev_vm.h:511`, `gen/g_uvm_nvoc.c:266-270` | latent: UVM 580 clamps the count to 1 (`uvm/uvm_gpu.c:2936-2938`) |
| L13 | `FspRow` boot-gate rows: HOPPER says "no gate" (GH100 polls it), BLACKWELL gives GB10x GB20x's 0xAD00BC | GH, GB10x | `fsp_gsp.rs:185,291-302` ↔ `gen/g_kern_fsp_nvoc.c:418-431` | harmless (the static boot row serves the right offset); `gsp_model` would need the architecture — **ratchet test** |
| L14 | Blackwell's 580 WPR2 pair decoded but never published (reads 0) | GB10x, GB20x | `fsp_gsp.rs:564-565,596-597` ↔ `gen/g_kernel_gsp_nvoc.c:1237-1239` | correct by accident (only the "already up" gate reads it) |
| L15 | 0x840000 (SEC2 EMEM, integrated only) is a read-exit page on discrete Blackwell | GB10x, GB20x | `kf-trap/src/memmap.rs:131-136` ↔ `THE_BAR0_DISPOSITION_MAP.md` §1, `pub/blackwell/gb20b/dev_sec_pri.h:40` | GB20x (measured) behaviour; extra exits only |
| L16 | VER3 PDE sparse: the PDE's PCF is 5:3 and sparse is 1 **or** 3; the walker reads the PTE's 7:3 == 1 | GH, GB | `kf_walk.cu:316-320` ↔ `pub/hopper/gh100/dev_mmu.h:64-76` | census/veto only — **ratchet test** |
| L17 | PRAMIN window BASE decoded 24:0 on Blackwell (GB10x header 22:0; GB20x ambiguous) | GB10x, GB20x | `kf-trap/src/pramin.rs:52` ↔ `pub/blackwell/gb100/pri_nv_xal_ep.h:41` | a superset: harmless (asserted) |
| L18 | GB20x kind rule (every non-PITCH kind → GENERIC; no Z/S kinds) | GB20x | `kf-chip/src/ptekind.rs:44-48` ↔ `krn/mem_mgr/arch/blackwell/mem_mgr_gb202.c:81-93` | latent (a GB20x guest cannot emit them) |
| L19 | Turing decoded with GA10x geometry (PD1 as a 512 MiB leaf) | TU | `kf-cuda/src/abi.rs:663-670` ↔ `kern_gmmu_fmt_gp10x.c:74-80`, `gen/g_kern_gmmu_nvoc.c:302-312` | hostile tables only (span-checked) |
| L20 | 128 KiB big pages neither modelled nor refused | all | `abi.rs:572,576` ↔ `krn/mmu/kern_gmmu.c:620-646` | refuse by name, or model |
| L21 | Translated rewriter forms no stock driver emits: legacy GRP2 non-inc expanded as incrementing; CAB5 PREFETCH treated as a copy; PHYS_MODE PEER_ID/FLA ignored; L2 op 0x11 forwarded on classes without it | all / GB20x | `kf-abi/src/submit.rs:2308-2310`, `translated.rs:53,88-97,237-247,439-441` ↔ `cls/clc56f.h:296`, `clcab5.h:34`, `clc7b5.h:72-74` | GA10x behaviour on hostile input |
| L22 | Booter Unload matched only on MAILBOX0 == 0xFF (GC6-entry and suspend unloads are read as Loads); HWCFG2 `RESET_READY` never set (150 µs per falcon reset) | TU…AD | `falcon_gsp.rs:165,175-178` ↔ `kernel_gsp_booter_tu102.c:138-165`, `pub/ampere/ga102/dev_falcon_v4_addendum.h:27-28` | GA10x behaviour |
| L23 | Turing `LOCAL_MEMORY_RANGE` silently omitted for a size that is not `mag << scale` (FB size 0) | TU | `bar0.rs:184-193` ↔ `krn/mem_sys/arch/pascal/kern_mem_sys_gp102.c:47-56` | should refuse realize by name |
| L24 | A PCIe Gen6 host (64 GT/s) is refused at realize | GB10x hosts | `kf-qemu/src/hostfacts.rs:85-92` ↔ `ctrl2080bus.h:363,375` | trivial, but no Gen6 host to test |
| L25 | The engine class sets are generated from ogkm **610.43.02**, not the 580 the bench runs (every 580 set is contained) | all | `kf-chip/src/classes.rs:141` | regenerate against 580, or state both |
| L26 | Gates 3/4 `is_ce` omits Turing's C5B5 | TU (harness) | `kf-harness/src/bin/kf-gate3.rs:173-175`, `kf-gate4.rs:192-194` | use `kf_chip::classes` |
| L27 | `kf-chip/tests/fsp_cot_sequence.rs:45-47` labels NVDM 0x18/0x1A as CAPS_QUERY/CLOCK_BOOST (they are SMBPBI/ROMREAD; 0x16/0x20) | GH, GB (test label) | ↔ `fsp_nvdm_format.h:42-49` | test comment only |
| L28 | Device-shell choices: MSI-X in BAR5 with 32 vectors (hardware: BAR0 0xB90000, 6/9/12); BAR0 size falls back to GA106's 16 MiB | all | `kf3.c:52-53`, `device.rs:127,361` | deliberate; the fallback should refuse |

**Citation drift found (values right, pointers wrong)** — recorded so a reader does not trust the pointer:
`falcon_gsp.rs:286` ("little-endian ASCII" — it is big-endian char packing); `fsp_gsp.rs:29-31,67-68,273-278`
(link to modules that no longer exist); `fspemem.rs:51` (AINCR is `:36`); `cpuintr.rs:25-27` (gh100's
`dev_vm.h` defines LEAF/EN only); `cacheop.rs:33-35,119-123` (the FB_FLUSH token is read only on GB110,
GB112 and GB20x — GH100/GB100/GB102 take the RPC path); `token.rs:86` (626/630, not 624);
`bar0.rs:85` (0x84 is only in `gm107/dev_nv_xve.h`); `kf_walk.cu:317`, `walk.rs:716`, `abi.rs:529,551`,
`kf_walk.cu:1302-1304` and the "VER3 is a sketch, never run" notes (`kf_walk.h:259-260`, README) —
VER3 ran on GB203; `submit.rs:1836,2111-2117` and the SEMAPHORE_TYPE/VPRMODE lines; `translated.rs:33`;
`kf-chip/src/lib.rs:164`; `kf-rm/src/authored.rs:320-322` (GRAPHICS/NVENC/NVDEC DEV_TYPE are in no ogkm
header).

## 6. Gaps per family

### GH100 (Hopper) — never run; source-only everywhere

- **Realize.** Realize was refused at the engine table until `1e37217d` (D2). ⚠ The next blocker on
  the path is unknown until a box runs it.
- **Boot path.** It is the FSP path GB203 proved:
  - `NV_THERM_I2CS_SCRATCH` 0x200BC is served statically.
  - The `NV_EP_PCFGM` mirror 0x9206C is served.
  - The 0x6C config word is unreached in a VM.
  - The FSP EMEM channel is `gh100`'s own header.
- **Page tables.** Two walker gaps apply:
  - L4: UVM on GH100 writes `NO_VALID_4KB_PAGE`.
  - L16: the PDE sparse encoding.
  - 128 KiB pages (L20) are neither modelled nor refused.
- **Channels.** C86F and C8B5 are trimmed in 580. Their layouts rest on NVIDIA's inheritance, and the
  class checks follow it. `MMU_OPERATION` (0xB) is refused by name. Extended-base GP entries: L8.
- **Registers served as 0 that are wrong but unreached:**
  - `NV_PGSP_FALCON_ENGINE` reset status: a `kflcnReset` path would time out.
  - `NV_PGC6_SCI_SEC_TIMER` is a static 0.
  - `NV_GFW_FSP_UCODE_VERSION` 0 makes RM skip the SYS_TIMER_OFFSET writes, which is harmless.
- **Refused by name, as on every die group.** C2C (Grace Hopper), MIG (GR1+), NVJPG/OFA.

### GB10x (datacenter Blackwell) — never run; source-only everywhere

- **PCIe link capabilities are unreadable on GB100 and GB102** (L2). This is the most likely first
  failure, at `UVM_REGISTER_GPU`.
- **Page tables.** Three gaps:
  - The 256 GiB PD2 leaf is not decoded (L5).
  - The VER3 walker gaps L4 and L16 apply.
- **Topology.** Blackwell's PBDMA fault-id reservation contract (L7).
- **Fixed on this branch:**
  - CE10-19 notifiers (D3); GB100/GB110 have 20 copy engines.
  - GB10B is refused (D4).
- **Latent:**
  - Access counter 1 (L12).
  - The `FspRow` boot-gate row (L13).
  - The 580 WPR2 pair is unpublished (L14).
  - PRAMIN BASE superset (L17).
  - The SEC2 hole (L15).
  - A Gen6 host is refused (L24).
- **Driver boundary, noted only.** These Blackwell GR controls are unserved but tolerated today:
  `SM_ISSUE_RATE_MODIFIER_V2`, `PPC_MASKS`, `ROP_INFO`, `FECS_TRACE_DEFINES`, CCU sample info
  (`V3_FAMILY_PORT_BLACKWELL.md` §7).

### GB20x (consumer Blackwell) — measured on GB203

- **Open items:**
  - L4 and L16 (UVM on GB20x uses the Hopper MMU HAL).
  - L7 (`RLENG_ID` width with 4 GRCEs).
  - L8 (the pushbuffer base is 0 today).
  - L15 (the SEC2 hole costs exits).
  - L17 and L18.
  - L21 PREFETCH.
- **Correct only by accident:** L14. The GB20x boot gate is right. The `FspRow` ratchet concerns GB10x.

### GA10x — measured on GA106 (plus GA104, GA102 lanes)

- L1 (retry after a failed GSP boot).
- L9, L10, L21, L22.
- The shared design gaps: L3, L6, L11, L20.
- Every authored topology value (§4.6) is anchored to one GA106 capture.

### AD10x — measured on AD106 (end to end), AD104 (GR probe)

- The GA10x gaps apply.
- The Ada-only boot register (`SCRUBBER_HANDOFF`) is measured A/B.
- The CE header stops at CE5, and CE6's fault slot is NVJPG0. Latent: current parts have ≤ 5 LCEs.

### TU10x — never run

- **VER2 geometry** is decoded with GA10x's 512 MiB PD1 leaf (L19).
- **L23** (`LOCAL_MEMORY_RANGE` omission), **L1**, **L10**.
- **Harness:** gates 3/4 miss C5B5 (L26).
- **Unmeasured Turing-only paths:**
  - the TU102 RISC-V layout (header-checked, `T`);
  - PIO HS-ucode load;
  - HOST14 as the last HOST id. Authored PBDMA ids have no upper bound; latent.

### GA100 — refused by name (`RowUnbuilt`)

- **Needs a Booter-only boot sequence.** GA100 has no FWSEC-FRTS (FRTS = 0), which makes the FRTS row
  `c✗`.
- **Needs the MMU_LOCK PLM registers.** Zero fails them.
- **Everything else is header-checked with the TU102 RISC-V layout (`T`).**

## 7. Hand constants that should become derived — ranked by risk

"Derived" means read from `kf_chip::hwref`, or from a HAL-binding table generated the same way, instead
of written as a literal. Rows marked `T` in §4 are already *checked*; this ranking concerns what should be
*supplied*.

1. **Page-table level geometry and leaf sets per die group.** Today these are two hand copies
   (`kf-cuda/src/abi.rs:559-727`, `cuda/walk/kf_walk.cu:1325-1427`). They should be derived from the
   `kgmmuFmtInitLevels_*` binding plus `bPageSize512mb/256gbSupported` in `g_kern_gmmu_nvoc.c`.
   ⇒ Turing loses the 512 MiB leaf (L19), and GB10x gains the 256 GiB leaf (L5).
2. **VER2/VER3 field positions.** There are five copies: `.cu`, `abi.rs`, `synth.rs`, `kf_tables.h`,
   `kf_real_tables.py`. Only `abi.rs` is now tied to `dev_mmu.h`. Generate the descriptor from `hwref`,
   and add the missing encodings: VER3 PDE PCF 5:3 with sparse 1/3 (L16), and `NO_VALID_4KB_PAGE` (L4).
3. **WPR2** (`falcon_gsp.rs:248-273`). Take it from the guest's FRTS command instead of re-deriving the
   arithmetic (L1).
4. **MMU fault ids and video fault ids** (`authored.rs:337-384`). They are now checked on every family.
   Generate them, and add per-die bounds: Ada CE0..5, TU HOST0..14.
5. **The Blackwell hole list and cache-op register sets per die group** (`memmap.rs:113-138`,
   `cacheop.rs:80-143`). Generate them from the FSP/SEC2, `kmemsysDoCacheOp` and
   `kbusSendSysmembarSingle` bindings (L10, L15).
6. **One per-die-group row for each of the following:**
   - the FSP boot gate (`fsp_gsp.rs:185` vs `bar0.rs:114-116`, L13);
   - the PRAMIN window register (`pramin.rs:47-54`, L17);
   - the access-counter registers (`bar0.rs:89`, L12);
   - config-space placement (`bar0.rs:280-292`, L2).
7. **CE per-class rules.** These are the upper width, the scrub bit and PREFETCH (`translated.rs:466-520`),
   and `kf_abi::submit::ce`, which is written as C7B5 and used for every class. Replace them with one
   per-class table generated from `cl*b5.h` plus the inheritance map.
8. **GP-entry opcodes per channel class** (`submit.rs:1926-1945`, `ring.rs:168-170`, L8). Also the
   `fifo::SEM_*` widths (SEM_ADDR_HI is 7:0 through C56F, 24:0 from C86F).
9. **Notifier indices** (`kf-host/src/event.rs:57-80`). They are now checked; generate them from
   `cl2080_notification.h`.
10. **Kind uncompression per die group** (`ptekind.rs`). Derive it from the `memmgrGetUncompressedKind_*`
    binding and `PTEKIND_SUPPORTED` (L18).
11. **GSP/DISP and engine non-stall vectors** (`authored.rs:132-240`). Derive them from the host's
    category/subtree map, and check them against its static rows.
12. **GA106 values served to every die group.** These are the GMMU fault-buffer sizes, the CE
    fault-method buffer size, the FECS record size, the memsys fields and the GSS-legacy bodies
    (`authored.rs:52-125`, `hostquery.rs:523-535`, `kf-abi/src/cudartinit.rs:110-130`). Ask the host
    where a control allows it; otherwise state them per die group.
13. **The integrated-part list** (`kf-chip/src/lib.rs:129-137`). Generate it from `g_hal_archimpl.h`.
    It was fixed by hand for GB10B (D4).
14. **The usermode base.** It is held twice (`memmap.rs:143`, `hostquery.rs:471`). The window span now
    has one source (D1).
15. **`CHID_BITS` vs `AUTHORED_FIFO_CHANNELS`** (`tokenindex.rs:36`, `hostquery.rs:1183`). Derive one
    from the other and from `NV_CHRAM_CHANNEL__SIZE_1`.
16. **The PCIe speed map** (`kf-qemu/src/hostfacts.rs:85-92`). Derive it from the ctrl2080bus table,
    which adds Gen6 (L24). **The BAR0 16 MiB fallback** should refuse by name (L28).
17. **Lowest risk.** These are correct on every die group and checked (`T`): falcon, RISC-V, queue and
    GFW offsets; FSP EMEM; MMU invalidate; the CPU interrupt tree; timer refusals; PMC_BOOT layouts; the
    method header. Generate them only so that one source exists.

## 8. What this inventory is not

- **Not a hardware measurement.** "Header" means ogkm-580's header for the die group's bound HAL. For
  registers the guest RM reads, that is the acceptance criterion. For formats the host GPU executes
  (CE methods, page tables), silicon is the truth, and only the `M:` rows have touched it.
- **The lineage approximates HAL bindings.** When no bound HAL is at hand, `kf_chip::hwref` returns the
  nearest ancestor's value. It refuses (`Ambiguous`) only when that tier disagrees with itself.
  - A resolved value says what a name would mean **if** the die group read it.
  - Whether it reads it is the HAL binding's business. The §4 rows cite the binding wherever it decides
    the verdict.
- **The tests are the enumeration.** A hardware literal with no test (a `c` cell without `T`) is still
  unchecked, and a literal that neither the sweeps nor the tests saw is not in this document.
  - The sweeps read every `kf-*` crate, `qemu/hw/misc/kf3/` and `cuda/walk/`.
  - The frozen `kayfabe-*` tree (including the raw client's own GA10x-era assumptions) is out of scope.
- **Line numbers are at `e05ff74d`.** This branch shifted lines in `authored.rs`, `memmap.rs`,
  `device.rs`, `event.rs` and `kf-chip/src/lib.rs`. The fixed rows name their commits.

## Appendix A — the die-group-varying names kayfabe relies on

Generated with `cargo run -p kf-chip --example hwref -- <names>`. Markers: `·` inherited from an
ancestor directory, `📌` pinned by HAL, `⊘amb` ambiguous (no pin), `—` absent from the lineage. A value
in a column does not mean that die group reads the register (for example, `NV_PTIMER_TIME_0` resolves on
GH100, which uses the SCI timer instead). The full list of 1 274 differing names is
`… --example hwref -- --differs`.

| name | TU10x | GA100 | GA10x | AD10x | GH100 | GB10x | GB20x |
|---|---|---|---|---|---|---|---|
| `NV_PRISCV_RISCV_CPUCTL` | 0x268 | 0x268· | 0x388 | 0x388· | 0x388 | 0x388· | 0x388 |
| `NV_PRISCV_RISCV_IRQMASK` | 0x2b4 | 0x2b4 | 0x528 | 0x528· | 0x528📌 | 0x528📌 | 0x528 |
| `NV_PRISCV_RISCV_BCR_CTRL` | — | — | 0x668 | 0x668· | 0x668· | 0x668· | 0x668· |
| `NV_PFB_PRI_MMU_LOCAL_MEMORY_RANGE` | 0x100ce0· | 0x100ce0· | 0x100ce0· | 0x100ce0· | 0x100ce0· | 0x1fa3e0 | 0x1fa3e0· |
| `NV_PFB_PRI_MMU_LOCAL_MEMORY_RANGE_LOWER_MAG` | 9:4· | 9:4· | 9:4· | 9:4· | 9:4· | 27:4 | 27:4· |
| `NV_USABLE_FB_SIZE_IN_MB` | — | — | 0x1183a4 | 0x1183a4· | 0x1183a4· | 0x1183a4· | 0x1183a4· |
| `NV_PGC6_BSI_SECURE_SCRATCH_15` | — | — | — | 0x1180fc | — | — | 0x1180fc· |
| `NV_THERM_I2CS_SCRATCH` | — | — | — | — | 0x200bc | 0x200bc | 0xad00bc |
| `NV_EP_PCFGM` | — | — | — | — | 0x92fff:0x92000 | 0x92fff:0x92000· | 0x92fff:0x92000· |
| `NV_EP_PCFG_GPU_LINK_CAPABILITIES` | — | — | — | — | 0x6c | 0x6c· | 0x6c· |
| `NV_PF0_LINK_CAPABILITIES` | — | — | — | — | — | 0x4c | 0x4c· |
| `NV_XAL_EP_BAR0_WINDOW` | — | — | — | — | 0x10fd40 | 0x10fd40 | 0x10fd40 |
| `NV_XAL_EP_BAR0_WINDOW_BASE` | — | — | — | — | 21:0 | 22:0 | ⊘amb |
| `NV_XAL_EP_UFLUSH_L2_FLUSH_DIRTY` | — | — | — | — | 0x10f810 | 0x10f810 | 0x10f810· |
| `NV_PFSP_EMEMC(0)` | — | — | — | — | 0x8f2ac0 | 0x8f2ac0· | 0x8f2ac0· |
| `NV_VIRTUAL_FUNCTION_PRIV_CPU_INTR_LEAF__SIZE_1` | 0x8 | 0x8 | 0x8 | 0x8· | 0x10 | 0x10 | 0x10· |
| `NV_VIRTUAL_FUNCTION_PRIV_FUNC_L2_SYSMEM_INVALIDATE` | — | — | — | — | 0xf10 | 0xf10 | 0xf10· |
| `NV_VIRTUAL_FUNCTION_DOORBELL_RUNLIST_DOORBELL` | — | — | — | — | — | 22:22 | 30:30 |
| `NV_VIRTUAL_FUNCTION_DOORBELL_RUNLIST_DOORBELL_ENABLE` | — | — | — | — | — | 0x0 | 0x1 |
| `NV_CHRAM_CHANNEL__SIZE_1` | — | 0x800 | 0x800· | 0x800· | 0x800· | 0x800· | 0x800 |
| `NV_PFAULT_MMU_ENG_ID_GRAPHICS` | 0x40 | 0x40 | 0x40· | 0x40· | 0x180 | 0x180 | 0x180 |
| `NV_PFAULT_MMU_ENG_ID_CE0` | 0xf | 0xf | 0xf· | 0xf | 0x2b | 0x41 | 0x41 |
| `NV_PFAULT_MMU_ENG_ID_HOST0` | 0x20 | 0x20 | 0x20· | 0x20· | 0x40 | 0x55 | 0x55 |
| `NV_PFAULT_MMU_ENG_ID_NVDEC0` | 0xa | 0x19 | 0x19· | 0x19 | 0x13 | 0x1c | 0x1c |
| `NV_PFAULT_MMU_ENG_ID_NVENC0` | 0xb | 0xb | 0xb· | 0xb | 0x23 | 0x2c | 0x2c |
| `NV_MMU_VER3_PTE_PCF` | — | — | — | — | 7:3 | 7:3· | 7:3· |
| `NV_MMU_VER2_PTE_KIND` | 63:56 | 63:56· | 63:56· | 63:56· | 63:56 | 63:56· | 63:56· |
| `NV_MMU_VER3_PTE_KIND` | — | — | — | — | 11:8 | 11:8· | 11:8· |
| `NV_PGC6_SCI_SYS_TIMER_OFFSET_0` | — | — | — | — | 0x118df4 | 0x118df4· | 0x118df4· |
| `NV_PTIMER_TIME_0` | 0x9400· | 0x9400· | 0x9400· | 0x9400· | 0x9400· | 0x9400· | 0x9400· |
