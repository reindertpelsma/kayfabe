# V3 FAMILY PORT — Ada (AD106, RTX 4060 Ti): the first non-GA106 boot

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

Not blocking on Ada, left as named items (none reached a failing arm):

| where | assumption | status |
|---|---|---|
| `kf-chip/src/bar0.rs` `vbios_profile` | `VBIOS_PROFILES.first()` = the GA106 row's version (`0x9418_0000`); PCI ids overridden from host, FWSEC geometry generated | cosmetic (FWSEC parse is the shared `_TU102` path); the AD106 row at `kf-abi vbios.rs` is unused |
| `kf-abi/src/cudartinit.rs:214-228` | `PERF_GET_LEVEL_INFO_V2` splices GA106 clock words | cosmetic; host has an unprivileged answer |
| `kf-abi/src/cecaps.rs:272` | GA106-measured CE base caps for every family | Ada uses the same copy class; unverified; host has an unprivileged answer |
| `kf-rm/src/authored.rs:182,350` | `GA10X_GRCE_LCE_MASK = 0x03` for every family | right for Ampere/Ada/Hopper; **wrong for GB20x** (`kernel_ce_gb202.c:36`, `0x0F`) — a per-die-group fact inside Blackwell |
| ~~`kf-trap/src/trappolicy.rs:194`~~ | ~~Hopper/Blackwell doorbell at BAR1 `page_base 0x9_0000`~~ | ⊘ **REPLACED 2026-09-26** (`V3_BAR1_DOORBELL.md`): no fixed page — the BAR1 view is overlaid where the guest's BAR1 PTEs put it; BAR0 stays live. Per-family data: `kf_chip::Family::usermode_mmio` |
| `kf-abi/src/businfo.rs` via `bar0.rs` | link advertised fully trained at x16 | AD106 is x8; cosmetic |
| `kf-rm/src/authored.rs:17,22,85-86` | CE fault-method buffer `0x5000`, GMMU fault-buffer sizes, GSP/DISP vectors `0x9b/0x9a` | ours to author by ruling (w827); vectors refuse realize if the host table collides — did not on Ada |
| `kf-chip/src/bar0.rs:682-689` | 260 MiB firmware carve-out + BAR1/BAR2 root offsets from a 12 GiB GA106 capture | family-independent layout of OUR GSP; held on AD106 at `fb-mb=8192` |

## 3. The client had the GA10x pin, not kayfabe

Bare metal was **29/30** on the first run: `--uvm-mean` P3 allocated `AMPERE_COMPUTE_B 0xc7c0` from
`kayfabe_chips::pinned_host_classes()` (old tree, pinned `Ga10xHostClasses`) and RM answered
`NV_ERR_INVALID_CLASS` (`0x56`). Per the standing rule (bare-metal FAIL ⇒ client bug) this was fixed
in the client (`1d6bb323`): P3 asks `MC_GET_ARCH_INFO` and picks
`kayfabe_chips::host_classes_for_arch` (Ada → `ADA_COMPUTE_A 0xc9c0`). Bare 30/30 after; the guest
arm passes too, because the guest's RM reports the host's family through our device.
⚠ `pinned_host_classes()` still has about a dozen other call sites in the old-tree crates (`kayfabe-rm-ladder`, `kayfabe-isolate-host`) (all arms that
passed on Ada use classes Ada also lists). Any Hopper/Blackwell bare run will meet them.

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

1. **Turing**: GSP model still `RowUnbuilt` (no FWSEC-FRTS); the FB-size register is now served.
2. **Hopper / Blackwell**: FSP boot + the falcon PIO auto-increment read side effect (THE_CONSTRAINTS
   §52) and the read-triggered L2 cache op (`v3-reinit`); BAR1 doorbell offset unverified; GB20x GRCE
   mask; `pinned_host_classes()` in the raw client.
3. The cosmetic GA106 words in §2's second table should become host forwards the way `FB_GET_INFO`
   now is — same pattern, one control each.
