# V3 FAMILY PORT — Ada (AD106, RTX 4060 Ti): the first non-GA106 boot

> **UPDATED 2026-09-26 (branch `v3-families`, off `v3-bar1db`):** §2's second table, §3 and §5 are
> corrected in place — six of the GA10x-only items are now derived (host control / sysfs / ogkm HAL),
> Turing has a GSP model, and GA100 is refused by name. Source-only, **hardware-unverified** (no box).

**STATUS: ANSWERED, 2026-09-26 (branch `v3-family`, head `6ccb4585` + this doc).** kf3 boots a
stock 580.159.04 guest over an **AD106** host and the thin guest passes **30/30** (twice: `ada2`
at `1d6bb323`, `ada3` at `6ccb4585`), every arm `forwarded>0`, `emulated=0`. Bare metal on the
same box: **30/30** after one client fix (29/30 before it). One Ada-only boot blocker was found
and fixed as a per-family data row; A/B-confirmed on hardware.

Box: vast `52660152`, RTX 4060 Ti 16 GB (`0x2803`, AD106, arch `0x190` impl 6), AMD EPYC 7K62
(nested KVM guest), host driver swapped 575.51.03 → **580.159.04 (open)**. Destroyed at the end of
the session.

## 1. Verdict — does the deriving follow v3?

**Mostly yes, and the evidence is that it booted.** The kf3 realize path takes the family from the
host's `MC_GET_ARCH_INFO` (`Family::Ada`), PMC_BOOT_0/42 from the same reply, PCI identity/BAR0
size from the host's sysfs, engine types/counts, LCE/PCE masks, interrupt table, GR geometry,
context-buffer sizes, GPU name, L2/RAM/LTC facts from unprivileged host controls, and the engine
classes as the family's generated set ∩ the host's class list (Ada: `0xc56f`/`0xc561`/`0xc7b5`/
`0xc9c0`). Nothing on that path needed a new die row. What broke were **three GA10x facts sitting
where a family rule or a host fact belongs** (§2), and one **GA10x pin in the raw client** (§3).

The one thing ogkm does differently on Ada that matters to an emulated GSP is small: the full list
of AD10x-only HAL bindings in 580.159.04 is 13 functions (`g_*_nvoc.c`), and only
`kgspExecuteScrubberIfNeeded_AD102` touches a register we serve. `kbifPreOsGlobalErotGrantRequest_AD102`
reads `NV_PBUS_SW_SCRATCH(0)`, which the shadow answers 0 = `VALID_NO` = "no ERoT", returning early.

## 2. GA10x assumptions that broke (or were wrong) — and what was done

| # | where | GA10x assumption | Ada / other families (ogkm-580) | fix |
|---|---|---|---|---|
| 1 | `kf-chip/src/bar0.rs` `boot_regs` | no scrubber handoff register | `kgspExecuteScrubberIfNeeded_AD102` (`kernel_gsp_ad102.c`) runs a SEC2 scrubber HS ucode before FWSEC unless `NV_PGC6_BSI_SECURE_SCRATCH_15` (`0x1180fc`) `[31:29] >= 3`; on Ada the image is **always** allocated (`kernel_gsp.c:3734`, WAR bug 5016200) and the check is re-read after the "run" | **BOOT BLOCKER, fixed** `09a3944b`: Ada row advertises `0x6000_0000`. **A/B on hardware:** the pre-fix binary (`79848341`) fails `RmInitAdapter` with `kgspExecuteScrubberIfNeeded_AD102: failed to execute Scrubber: done bit not set` → `NV_ERR_GENERIC` @ `kernel_gsp_tu102.c:507` (4 retries); the fixed binary boots |
| 2 | `kf-abi/src/fbinfo.rs:266,271,444` via `kf-rm/src/inittables.rs` `FbGetInfoV2` | bus = LTC×32, FBPs = LTC/2, LTS = L2/128 KiB | host AD106 (measured, raw RM ioctl): bus `0x80`, FBPs 2, mask 3, L2 `0x2000000`, LTC 4, **LTS 16**. The GA10x ratio serves **LTS 256** (Ada's slice is 2 MiB) | **fixed** `5d7d471b`: `HostFacts.forwarded_fb_info` asks the host for the seven indices (NON_PRIVILEGED, once at realize) and serves them verbatim; missing index = realize refusal by name |
| 3 | `kf-chip/src/bar0.rs` `boot_regs` | `USABLE_FB_SIZE_IN_MB` (`0x1183a4`) served on Turing…Ada only | `kmemsysReadUsableFbSize_GA102` is bound for GA10x, AD10x, **GH100 and every GB die**; Turing and **GA100** bind `_GP102`, which reads `NV_PFB_PRI_MMU_LOCAL_MEMORY_RANGE` (`0x100ce0`) (`g_kern_mem_sys_nvoc.c:349-357`) | **fixed** `6ccb4585`: `0x1183a4` on all but Turing; `0x100ce0` (exact mag/scale, never rounded) on Turing + Ampere(GA100). Hopper/Blackwell guests would otherwise read FB size 0 |

Not blocking on Ada, left as named items (none reached a failing arm). ⊘ **Corrected 2026-09-26
(`v3-families`, `e8938d03` / `909fcef9`)** — the "status" column now says what was done:

| where | assumption | status |
|---|---|---|
| ~~`kf-chip/src/bar0.rs` `vbios_profile`~~ | ~~`VBIOS_PROFILES.first()` = the GA106 row's version (`0x9418_0000`)~~ | ✔ **FIXED** `e8938d03`: the ROM is the host's PCI identity + the **host's** VBIOS version (`BIOS_GET_INFO_V2` `0x20800810` `[REVISION, OEM_REVISION]`, NON_PRIVILEGED, `HostFacts::vbios_version`) + `kf_abi::vbios::GENERATED_FWSEC` (family-level, the shared `_TU102` FWSEC path). No die row is read |
| ~~`kf-abi/src/cudartinit.rs:214-228`~~ | ~~`PERF_GET_LEVEL_INFO_V2` splices GA106 clock words~~ | ✔ **FIXED** `e8938d03`: realize asks the host libcudart's own question once (`0x2080200b`, NON_PRIVILEGED, flags `0x50048`) → `HostFacts::perf_level_info_v2`; the guest's identical question gets the host's `[OUT]` words (`splice_perf_level_info_v2`), any other question is refused; a host refusal is relayed, not a realize failure. No host verb on the serve path. The GA106 words are now the test oracle |
| ~~`kf-abi/src/cecaps.rs:272`~~ | ~~GA106-measured CE base caps for every family~~ | ✔ **FIXED** `e8938d03`: `HostFacts::ce_caps` = the host's `CE_GET_ALL_CAPS` (`0x20802a0a`, NON_PRIVILEGED) minus the three bits the guest kernel ORs in itself (`KERNEL_OR_CAPS`: `kceAssignCeCaps_GP100/_GB100`). `present` stays the engine list's projection; an LCE the host does not mark present refuses |
| ~~`kf-rm/src/authored.rs:182,350`~~ | ~~`GA10X_GRCE_LCE_MASK = 0x03` for every family~~ | ✔ **FIXED** `e8938d03`: the GRCE set is the **host's** `GRCE` bits from the same reply — per die, so GB20x's `0x0F` (`kernel_ce_gb202.c:36`, and computed at runtime by `kceGetGrceSupportedLceMask_GB202`) and Turing's own HAL (`_4a4dee`) need no row. Engine table: all GRCEs on runlist 0, GRCE *k* on GR's PBDMA *k* mod 2 (`ENGINE_MAX_PBDMA = 2`); GA106 layout unchanged (`host_facts_query_ga106`) |
| ~~`kf-trap/src/trappolicy.rs:194`~~ | ~~Hopper/Blackwell doorbell at BAR1 `page_base 0x9_0000`~~ | ⊘ **REPLACED 2026-09-26** (`V3_BAR1_DOORBELL.md`): no fixed page — the BAR1 view is overlaid where the guest's BAR1 PTEs put it; BAR0 stays live. Per-family data: `kf_chip::Family::usermode_mmio` |
| ~~`kf-abi/src/businfo.rs` via `bar0.rs`~~ | ~~link advertised fully trained at x16~~ | ✔ **FIXED** `e8938d03`: `NV_XVE_LINK_CAPABILITIES` carries the host function's own sysfs `max_link_speed` **and `max_link_width`** (`PcieLinkCaps::host_link`); a width PCIe does not define refuses realize. x16 host → the old word, byte for byte |
| `kf-rm/src/authored.rs:17,22,85-86` | CE fault-method buffer `0x5000`, GMMU fault-buffer sizes, GSP/DISP vectors `0x9b/0x9a` | ours to author by ruling (w827); vectors refuse realize if the host table collides — did not on Ada. **Unchanged** (authored, not GA10x-only) |
| `kf-chip/src/bar0.rs:682-689` | 260 MiB firmware carve-out + BAR1/BAR2 root offsets from a 12 GiB GA106 capture | family-independent layout of OUR GSP; held on AD106 at `fb-mb=8192`. **Unchanged**. ⚠ Turing's WPR heap is smaller (`kgspGetMinWprHeapSizeMB_7185bf` = 64 MiB, OS carve-out 0), so it fits too |
| `kf-chip/src/bar0.rs:173` | `NV_PFB_PRI_MMU_LOCAL_MEMORY_RANGE` served on **all** Ampere | new, noticed 2026-09-26: only Turing/GA100 read it (`_GP102`); GA10x serves an extra, unread register. Harmless; not changed (GA106 behaviour frozen) |

★ **Every host fact added here is asked once at realize** (`kf_rm::hostquery`), each with a
`PROVENANCE` row and a named refusal; on a GA106 the derived values equal the GA10x constants they
replaced, which stay as the **test oracle** (`kf-abi` `cecaps`/`cudartinit` tests,
`kf-rm/tests/host_facts_ga106.rs`, `host_facts_query_ga106.rs` — `ce_caps` is filled from the real
GA106 `CE_GET_ALL_CAPS` capture). `BIOS_GET_INFO_V2` has **no** real-GA106 capture: on the replay host
it is one more "uncaptured" field (`over_the_real_ga106_the_query_refuses_only_the_uncaptured`).
⚠ Behaviour change on GA106, cosmetic: the ROM now carries the board's real VBIOS version instead of
`0x9418_0000` (only the guest's `NVRM` log prints it).
★ **Amended the same day (coordinator):** `vbios_version` is `Option` and **never fails realize** — a
host that does not answer `BIOS_GET_INFO_V2` leaves it `None`, the ROM declares the named
`kf_abi::vbios::NEUTRAL_VBIOS_VERSION` (`00.00.00.00.00`) and realize logs that by name. And the
guest's **own** `BIOS_GET_INFO_V2` (`0x20800810`, routed to physical) is now SERVED from it
(`WantedTable::BiosGetInfoV2`) so guest `nvidia-smi` shows the real VBIOS; refused when `None` or for
an index the header does not define. `cap1b` never reaches it (added to that differential's
named exception set, 32 of 51).

## 3. The client had the GA10x pin, not kayfabe

Bare metal was **29/30** on the first run: `--uvm-mean` P3 allocated `AMPERE_COMPUTE_B 0xc7c0` from
`kayfabe_chips::pinned_host_classes()` (old tree, pinned `Ga10xHostClasses`) and RM answered
`NV_ERR_INVALID_CLASS` (`0x56`). Per the standing rule (bare-metal FAIL ⇒ client bug) this was fixed
in the client (`1d6bb323`): P3 asks `MC_GET_ARCH_INFO` and picks
`kayfabe_chips::host_classes_for_arch` (Ada → `ADA_COMPUTE_A 0xc9c0`). Bare 30/30 after; the guest
arm passes too, because the guest's RM reports the host's family through our device.
⊘ ~~`pinned_host_classes()` still has about a dozen other call sites in the old-tree crates
(`kayfabe-rm-ladder`, `kayfabe-isolate-host`)~~ — ✔ **FIXED 2026-09-26** (`a133bc02`): every raw-client
and isolate-host connection now opens with `RmConnection::open_on_host`, which reads the device's own
`NV0080_CTRL_CMD_GPU_GET_CLASSLIST_V2` (NON_PRIVILEGED) after R6 (rung `R6a`) and picks the newest id
per role from *host list ∩ `kayfabe_doorbell::classgen::FAMILIES`* (generated from ogkm's
`g_gpu_class_list.c`) — RM's own `findDeviceClasses` rule, no family table. On a GA10x list it
reproduces the pin exactly (`kayfabe-chips` `derived_tests`). The P3 site's
`host_classes_for_arch(MC_GET_ARCH_INFO)` (no Turing, GB10x or GA100 row — GA100 is arch `0x170` and
takes `AMPERE_COMPUTE_A`) is gone too. Hardware-unverified.

## 4. Measurements

| run | rev | result |
|---|---|---|
| bare `bare1` | `79848341` | 29/30 (uvm-mean: client's GA10x compute class) |
| bare `bare2 --uvm-mean` | `1d6bb323` | PASS |
| guest `ada1 --timer` | `1d6bb323` | PASS, 22 s |
| guest suite `ada2` | `1d6bb323` | **30/30**, 120 s budget; `ce-client-guest-ram` 94 s (bare 8 s), `gpga-reserve-probe` 56 s (bare 39 s), rest 20–35 s |
| guest suite `ada3` | `6ccb4585` | **30/30** (same shape) |
| guest `adaAB_noscrub --timer` | binary `79848341` | FAIL — `RmInitAdapter` at the scrubber (negative control for §2 row 1) |

The guest's `NVRM` log carries the same pre-existing refusals as GA106 (golden-image channel /
kernel GR = P7 scope, `INTERNAL_INIT_USER_SHARED_DATA`, DECOMP PCE config) — none Ada-specific.

## 5. What is left for "all families first-class"

⊘ **Rewritten 2026-09-26 (`v3-families`).** Was: Turing unbuilt; Hopper/Blackwell FSP + read side
effects; GB20x GRCE mask; `pinned_host_classes()`; the cosmetic GA106 words. The GRCE mask, the
class pin and all four GA106 words are fixed (§2, §3).

1. **Turing** — ✔ **has a GSP model** (`909fcef9`). The old refusal's reason (*"`_TU102` boot has no
   FWSEC-FRTS"*) was wrong: `kgspGetFrtsSize_TU102` = 1 MiB and `kgspBootstrap_TU102`/FWSEC-FRTS/the
   SEC2 Booter are the HALs GA10x binds. Every falcon/queue/GFW-boot/WPR2 offset is identical
   (`turing/tu102` vs `ampere/ga102` `dev_falcon_v4.h`, `dev_gsp.h`, `dev_fb.h`). The one register
   difference is the RISC-V block (`kf_chip::falcon_gsp::RiscvLayout::Tu102`): RISC-V active =
   `CORE_SWITCH_RISCV_STATUS` `+0x240` bit 0 (`kflcnIsRiscvActive_TU102`), `IRQMASK`/`IRQDEST` at
   `+0x2b4`/`+0x2b8`, no `BCR_CTRL`. The HS-ucode load is PIO (`IMEMC`/`IMEMD`/`DMEMC`/`DMEMD`,
   `kgspExecuteHsFalcon_TU102`) — data-port writes; the one read on that path, `DMACTL` scrubbing, is
   `DONE == 0`. ⚠ Unverified on hardware; the remaining Turing-only HALs that touch no served register
   (`kgspGetLibosVersion` = LibOS 2, WPR heap 64 MiB / carve-out 0, `kgspIsDebugModeEnabled_TU102`)
   were read and need nothing from us.
2. **GA100** — ⊘ **refused by name** (new finding). It binds the `_TU102` RISC-V HALs **and** has no
   FWSEC-FRTS (`kgspGetFrtsSize_4a4dee` = 0, `kgspPrepareForFwsecFrts_5baef9`), so its boot goes
   straight to the Booter and the shared FWSEC-first `FalconSecureBooterBoot` FSM does not describe it.
   Before this it was **silently** given the GA102 model. Needed: a Booter-only boot sequence in
   `kf-gsp` (+ its WPR2 ordering) — a `kf-gsp` change, not a register row. `Family::gsp_model` now
   takes the host's `MC_GET_ARCH_INFO` implementation to draw this line.
3. **Hopper / Blackwell**: FSP boot + the falcon PIO auto-increment read side effect (THE_CONSTRAINTS
   §52) and the read-triggered L2 cache op (`v3-reinit`); Hopper's PBDMA fault ids (`HOST0`) unstated
   in the tree (engine table refuses Hopper by name).
   ⊘ **Blackwell (GB20x) ANSWERED 2026-09-26** — `V3_FAMILY_PORT_BLACKWELL.md`: FSP boot (EMEM
   auto-increment served on the vCPU), bare 30/30, thin guest 30/30, CUDA ladder green on a GB203.
   Hopper and GB10x remain hardware-unverified.
4. ~~The guest's own `BIOS_GET_INFO_V2` is unserved~~ — ✔ served from `HostFacts::vbios_version`
   (same day, see §2).
