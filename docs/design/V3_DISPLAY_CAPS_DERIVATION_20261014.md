# V3 display capabilities page — derived per die, no captured table (H01)

**STATUS: LIVE, 2026-10-15 (branch `claude/display-caps-derive-20261014`; every number below is a measured result with its
run directory on the host, or is marked INFERRED).** Closes the *code* half of H01 (`V3_FEATURE_GAPS.md`): the captured
AD104 page behind `KF3_DISPLAY_CAPS_PROBE` is gone from the product. Still open and stated in §7: the derived page has
not yet been booted as the final binary (host VM runs are paused for the owner's SolidWorks test), and no die other than
AD104 has been measured.

## 0. Result in one screen

- **What "the display caps page" is.** `NV_PDISP_FE_SW`, BAR0 `0x640000..0x640fff`, the register page the family's
  `NV*73_DISP_CAPABILITIES` object maps (`kdispGetDisplayCapsBaseAndSize`, `kern_disp_0300.c`): C573 Turing, C673 Ampere,
  C773 Ada, CA73 Blackwell. 1024 words; 99 are non-zero on the owner's AD104. It is **not an RM control**: the guest reads
  it as BAR0 memory. kayfabe's display plane serves it from a static word list (`DisplayPlane::static_words`).
- **The captured page was `CAPS_PROBE_WORDS` (99 words) in `crates/kf-qemu/src/display.rs`**, applied only for
  (class `0xC773`, base `0x640000`). Its fixture copy is `crates/kf-disp/tests/fixtures/caps_page_ad104_580.65.06.txt`
  (test oracle; no product code reads it). Reading the live page of the host's own AD104 as root
  (`tools/read_display_caps_page.py`, 2026-10-15, host driver 595.91.07) gives **the same 99 words**: the page is a static
  property of the die, not of the driver that read it.
- **What the Windows driver needs of it (measured on AD104, §3):** two things only, together — the **main-layer windows'
  `SCLR_PRESENT`** (windows 0/2/4/6 claim a precomp scaler) and the **unnamed word `0x00c` (= `0x100`)**. The other 100 of
  the 101 differing words (head caps, SOR caps, clocks, IHUB, widths, precisions, overlay-window presence, the other unnamed
  words) are not needed for a desktop and are not claimed.
- **Tiers.** T1 (rule over the ogkm class header + family row): the base page and the window-layer rows. T2 (unprivileged host
  control): **does not exist for this page** (§4). T3 (privileged per-die measurement): word `0x00c`, one row per
  (die, guest-driver branch) in `crates/kf-disp/data/caps_measured.tsv`. A die with no row gets the clean authored page and
  a loud notice (ruling §AH), never a neighbour's row.
- **Code.** `kf_disp::capsderive::derive` (new), family rows `data/caps_rows.tsv`, T3 rows `data/caps_measured.tsv`,
  generator/check `tools/derive_display_caps.sh` → `data/caps_family_pages.tsv`, reader `tools/read_display_caps_page.py`.
  `KF3_DISPLAY_CAPS_PROBE` is removed from `kf-qemu` and from the winprod launchers (older run scripts still list it; it is
  now ignored).
- **Tests (GPU-free, all green on the host):** `cargo test -p kf-disp` 150 unit + `tests/caps_derive.rs` 11 pass;
  `cargo test -p kf-qemu` 258 + integration pass (with the unrelated `force_migratable` initializer in `twin.rs` patched on
  the host copy only: the committed `twin.rs` at `1d845d50` does not compile its own test, the owner's working copy has the fix).

## 1. The layout (ogkm 580.159.04, `clc773.h` and siblings)

All four family classes share one layout; offsets are identical, fields differ slightly (`tools/derive_display_classes.sh`
output `data/classes-*.tsv`).

| offset | register | notes |
|---|---|---|
| `0x000` | `SYS_CAP` | `HEAD_EXISTS(i)` bits 0..7, `SOR_EXISTS(i)` bits 8..15, `DSI_EXISTS` 20..23 |
| `0x004` | `SYS_CAPB` | `WINDOW_EXISTS(i)` bits 0..31 (NVKMS asserts contiguity from 0) |
| `0x008`, `0x00c` | **no header names them** | `0x432`, `0x100` on AD104 |
| `0x010`, `0x018` | `IHUB_COMMON_CAPA`, `_CAPC` | mempool entries/width, MSCG/MCLK, request size; CAPC: lines buffered, pitch, `SUPPORT_SEMI_PLANAR`, `SUPPORT_HOR_VER_FLIP` (CA73 adds CAPE/CAPF) |
| `0x048/058/068/078`, `0x0c0/0d0/0e0/0f0` | **no header names them** | four per-head-stride words each (`0xa00`, `0x10`) |
| `0x144 + 8i` | `SOR_CAP(i)` | TMDS A/B, dual TMDS, DP A/B, 8 lanes, HDMI FRL |
| `0x5e4 .. 0x604` | **`HEAD_CLK_CAP(i)`** at `0x5e8 + 4i` (v05_01 `dev_disp.h`) + one unnamed word at `0x5e4` | `PCLK_MAX` 7:0 (INIT `0x77`), `PCLK_MIN` 15:8 |
| `0x608 + 4i` | `SOR_CLK_CAP(i)` | `DP_MAX` 7:0 (INIT 81), `TMDS_MAX` 23:16 (INIT 60), `LVDS_MAX` |
| `0x680 + 32i` | `POSTCOMP_HEAD_HDR_CAPA..F(i)` | head scaler, OCSC, DSC, dither, OLUT (not in CA73) |
| `0x780 + 32i` | `PRECOMP_WIN_PIPE_HDR_CAPA..F(i)` | `CAPA`: `FULL_WIDTH` 4:0, `UNIT_WIDTH` 9:5, `ALPHA_WIDTH` 13:10, `CSC00/0LUT/01` 16..18, **`SCLR_PRESENT` 19**, `TMO_PRESENT` 20, `GMA` 21, `CSC10/1LUT/11` 22..24; `CAPB` ILUT + `FMT_PRECISION`; `CAPC` CSC0 precision/LUT; `CAPD` TMO + scaler params; `CAPE` CSC1; `CAPF` vertical-scaler line limits |

The unnamed words are not in any ogkm tag, nouveau, or open-gpu-doc header (`V3 tier_checks/tier1_ogkm.md`: "layout is
published, the per-die values are hardware reads" — true for the named registers only).

## 2. Authored page vs the AD104 page, field by field (the table)

`kf_disp::caps::diff_pages` decodes both pages with the header's own field names. The production profile's authored page
(`sdr_page`, `KF3_DISPLAY_SDR_COLOR=1`) differs from the measured AD104 page in **101 words / 299 named fields**
(`cargo test -p kf-disp --test caps_derive -- --ignored dump_derived_vs_measured` prints them). Grouped (the head index is
irrelevant, the window parity is not):

| group | measured AD104 vs authored | tier that could produce it | used by the Windows driver? (§3) | derived page |
|---|---|---|---|---|
| `SYS_CAP`, `SYS_CAPB` | equal (`0xf0f`, `0xff`) | T1 (family row: heads/SORs/windows) | – | T1, equal |
| **`0x00c`** | `0x100` vs 0 | **T3** only (no header, no unprivileged control) | **yes, decisive** | **T3 row** |
| `0x008` | `0x432` vs 0 | T3 | no (e16: with it alone the guest hangs; e17b: `0x00c` without it works) | not claimed |
| `0x048..0x078`, `0x0c0..0x0f0` (8 words) | `0xa00`, `0x10` vs 0 | T3 | no (e9, e4/e8 pass without) | not claimed |
| `IHUB_COMMON_CAPA/C` | mempool 11040 entries, 128 B, MCLK/MSCG/VGA, 4 lines, flip vs 0 | T3 (per-die IMP inputs); NVKMS reads only PLANAR/SEMI_PLANAR/FLIP | no (e3) | base page keeps SEMI_PLANAR only |
| `SOR_CAP(0-3)` | `DP_A`, `DP_B`, `DP_8_LANES` set, `DUAL_TMDS` clear vs `DUAL_TMDS` | connector policy (kayfabe's connectors are TMDS-only) | no (e5) | not claimed |
| `SOR_CLK_CAP` | `DP_MAX` 81 on 8 SORs, `TMDS_MAX` on SORs 4-7 vs TMDS on 0-3 | T1 INIT values for the SORs kf presents | no (e5) | not claimed |
| `HEAD_CLK_CAP` | `0x86` ×8 (+`0x5e4`) vs `0x77` ×4 | per-SKU; T1 INIT is `0x77` | no (e5) | base page (`0x77`) |
| `POSTCOMP_HEAD_HDR_CAPA-F(0-3)` (24 words) | scaler, OCSC0/1, DSC, dither, HCLPF, VFILTER, OSCAN, widths vs 0 | T3 numbers; each "present" bit is a feature kf must implement | no (e2) | not claimed |
| `PRECOMP_WIN_PIPE_HDR_CAPA`, even windows | **`SCLR_PRESENT`**, + `ALPHA/FULL/UNIT_WIDTH` 8/16/14 | `SCLR_PRESENT`: T1 row (kf's console scales a window); widths: T3 numbers, no open source | **`SCLR_PRESENT` yes (f5)**; widths no (f3 passes without) | **`SCLR_PRESENT` only** |
| `PRECOMP_WIN_PIPE_HDR_CAPA`, odd windows | CSC11 only, no TMO, + widths vs full CSC chain + TMO | T3 / hardware layer design | no (f6 fails alone, f5 passes without) | not claimed (authored overlay keeps its chain) |
| `PRECOMP_WIN_PIPE_HDR_CAPB..F(0-7)` | `FMT_PRECISION` 16, `ILUT_LOGNR` 6, CSC precisions 14, CSC LUT 2^9 ×(5/6), scaler precisions/limits (4× max, 5120/2560 px), `TMO_LOGNR` 6 vs 0 (odd windows: TMO fields cleared) | T3 numbers | no (e19b fails alone; e18b/f3/f5/e20 pass without) | not claimed |

What the base page already agrees with the hardware on, derived from the headers alone: `SYS_CAP/B`, ILUT `LOGSZ`/`DIRECT`/
`SFCLOAD`, TMO `LOGSZ`/`SFCLOAD` on windows that have TMO, CSC presence, the `SEMI_PLANAR` bit, `TMDS_MAX`/`SINGLE_TMDS`/
`HDMI_FRL`.

## 3. The bisection on hardware (measured)

Setup: the owner host's own AD104 (RTX 4070, host driver 595.91.07), the Windows 11 LTSC 580.88 baseline
(`/var/lib/kf-windows-20261005/baseline`, a COW overlay per run), the die-sweep harness
(`scripts/bench/diesweep/` of `origin/claude/die-sweep-20261014b`, with a `WIN_FLAGS_ADD` hook), kf3 `1d845d50` + a
**temporary probe** (`KF3_DISPLAY_CAPS_GROUPS=<groups>`: the authored page with the named groups of the measured AD104 page
laid over it; it is in the branch history at `bdcd65f15` and removed in the product commit). Production flag profile
without `KF3_DISPLAY_CAPS_PROBE`. Verdict per run, from kf3's own log: **desktop** = "the console shows head 0 1920x1080:
[window 0 …]" within ~11 s of the first arm and 0 resets; **reset loop** = "display life ENDED" repeating, 0 windows
scanned; **hang** = the guest stops pushing at `puts=35 methods=5338 updates=0` (no first UPDATE, no reset).

| run | groups laid over the authored page | verdict |
|---|---|---|
| h0 | none (probe flag removed) | **reset loop**: 10 `Running -> Suspending` in 2 min, 96 window channels allocated (8 per life), 8589 methods at the first update — reproduces the die sweep's AD106/GA104 failure on AD104 |
| e6 | `all` (= the old probe) | **desktop** (control: the mechanism reproduces the captured page) |
| e7 | everything **except** the window groups | reset loop (the windows are necessary) |
| e1, e1b | all window words | **hang** (twice) |
| e2, e3, e5 | window + head caps / + IHUB / + SOR/clock groups | hang |
| e4 | window + all unnamed words | **desktop** |
| e9 | window + `0x48..0x78` | hang |
| e16 | window + `0x008` | hang |
| e8 | window + `0x008`,`0x00c` | desktop |
| e17b | window + `0x00c` only | **desktop** |
| e18b | `0x008`,`0x00c` + window CAPA words only | desktop |
| e19b | `0x008`,`0x00c` + window CAPB-F only | reset loop |
| e20 | `0x00c` + window CAPA words only | desktop |
| f3 | `0x00c` + CAPA **presence bits** only (widths stay 0) | **desktop** |
| f4 | `0x00c` + CAPA widths only | reset loop |
| f5 | `0x00c` + **even-window** CAPA presence only | **desktop** |
| f6 | `0x00c` + odd-window CAPA presence only | reset loop |

Reading (measured): the set {even-window `SCLR_PRESENT`, word `0x00c`} is sufficient, and neither alone is (windows without
`0x00c` hang; `0x00c` with authored windows resets). The even-window CAPA presence difference is exactly one bit
(`SCLR_PRESENT`), because the base page already has the CSC chain and TMO on every window. **The derived AD104 page is
word-for-word the page of run f5** (asserted by a test). INFERRED, not shown: *why* — the driver appears to treat a window
with a precomp scaler as a main-layer candidate and initialise only those (the hardware run initialised windows 0/2/4/6 only,
`display_reply_diff_20261008` row P17), and `0x00c` bit 8 gates that path; no closed code was read for this and none is
needed.

Invalid runs, discarded and re-run: e10/e11 (stopped unfinished), and one block (e17, e18, e19, the first e8c) that died with
`kvm run failed Bad address` because three kf3 VMs plus another agent's had exhausted the host GPU's 12 GiB (each kf3 VM
holds its 4 GiB store plus ~0.7 GiB; `nvidia-smi` 11.2 of 12.3 GiB). The runs from e8c on use `fb-mb=2048`, 4 GiB guest RAM,
4 vCPUs. The hang/desktop/reset verdicts of the earlier block (e1..e16) used `fb-mb=4096`, 8 GiB, 6 vCPUs; the control e6
(desktop) and e1 (hang) were in that block, and the later block's e8c control also gives a desktop.

Evidence: `traces/display_caps_derive_20261014/` (`bisection_runs.txt`: every run's counts and display lines; `exp.sh`/`exp2.sh`: the runner; `caps_live_ad104_host_20261015.txt`: the live read).

## 4. Why not T2

An unprivileged host client can allocate neither the display object nor its caps object: `NVC770_DISPLAY`/`NVC673_DISPLAY`…
and every `NV*73_DISP_CAPABILITIES` are `RS_FLAGS_ALLOC_PRIVILEGED` (ogkm 580.159.04
`src/nvidia/src/kernel/rmapi/resource_list.h:1060-1130, 1261-1340`; `alloc_free.c:600-660` requires `RS_PRIV_LEVEL_USER_ROOT`).
The one display fact an unprivileged subdevice control carries is `NV2080_CTRL_GPU_INFO_INDEX_DISPLAY_ENABLED`. Everything
else (`NV2080_CTRL_CMD_INTERNAL_DISPLAY_GET_STATIC_INFO` with `feHwSysCap`, `windowPresentMask`, `numHeads`) is an internal
control only RM may issue. Not tried, listed as a possible follow-up: the host's own NVKMS ABI (`nvkms-api.h`,
`NVKMS_IOCTL_ALLOC_DEVICE` reply: heads, layers, per-layer scaler/TMO/LUT caps) which would give the parsed result for the
**named** fields on the host's own die; it cannot give `0x00c` (no header names it, NVKMS never reads it). Root on the box
(`CAP_SYS_ADMIN`, a user-space tool, no kernel module) is tier 3 in the ruling's order, and a root tool can also read the page
itself, which is what `tools/read_display_caps_page.py` does.

## 5. What changed

- `kf-disp`: `caps::diff_pages` (field-level page diff); `capsderive` (`derive`, `parse_measured`, `parse_rows`, `Tier`,
  `Die`, `HostFacts` (the empty T2 hook), `Derived { page, provenance, unmeasured }`); `class::rows_under`; data
  `caps_rows.tsv` (family rows: one row today, the main-layer scaler), `caps_measured.tsv` (T3: AD104/580 word `0x00c`),
  `caps_family_pages.tsv` (generated).
- `kf-qemu`: `DisplayPlane::build` takes the die `(architecture, implementation)` and derives the page; the captured table,
  `KF3_DISPLAY_CAPS_PROBE` and its test are deleted. Realize prints `caps page derived — N words: a T1 rule, b T3 measured
  (c unmeasured)` and, for a die with no row, `NO MEASURED caps word 0x00c for die arch … impl …, driver branch …`.
- Tools: `tools/derive_display_caps.sh` (checks every row name against ogkm's headers with `gcc -E -dM`, regenerates
  `caps_family_pages.tsv`), `tools/read_display_caps_page.py` (T3 reader).
- Launchers: `KF3_DISPLAY_CAPS_PROBE` dropped from `windows_broker_prod*.sh`.
- Rule kept: nothing about hostile-guest handling changed. The page is a static set of aligned, in-page, distinct words
  published read-only through the BAR0 shadow; nothing in it comes from guest bytes (tested).

## 6. Measuring another die (the T3 reader) and how the constants kit would call it

On any box that is ours, as root, with the stock driver up and the display engine initialised (a GeForce die with no monitor
is fine):

```
python3 tools/read_display_caps_page.py --list          # the NVIDIA display functions: "0000:01:00.0 0x2786"
python3 tools/read_display_caps_page.py 0000:01:00.0 > caps_<die>_<driver>.txt
```

It maps `resource0` `PROT_READ` over `0x641000` bytes, does 1024 aligned 32-bit loads at `0x640000`, writes nothing and
issues no ioctl. Expected output (the AD104 host, 2026-10-15):

```
# kf-display-caps-page v1
# pci 0000:01:00.0 vendor 0x10de device 0x2786 bar0 0x640000+0x1000 (PROT_READ, 32-bit loads)
DECISIVE 0x00c 0x00000100
EVENCAPA 0 0x780 0x01df21d0
EVENCAPA 2 0x7c0 0x01df21d0
EVENCAPA 4 0x800 0x01df21d0
EVENCAPA 6 0x840 0x01df21d0
WORD 0x000 0x00000f0f
WORD 0x004 0x000000ff
… (every non-zero word, ascending)
# end words=99
```

Exit 1 with a message when the BAR cannot be mapped read-only or the page is empty/all-ones (display engine not up);
exit 2 on usage. A row for kayfabe = the `DECISIVE` value + the die's RM `(architecture, implementation)` (kf3 prints them at
realize) + the guest-driver branch + caps class:
`<arch>\t<impl>\t<branch>\t<class>\t0x00c\t<value>\tmeasured:<pci id>,<how>,<date>` appended to
`crates/kf-disp/data/caps_measured.tsv` (a source not starting `measured:` is refused by the parser).

**Kit integration (documentation only; the kit's branch is not edited).** `scripts/bench/constants/run_container_probe.sh`
(`origin/claude/constants-measure-20261014b`) copies a kit directory to `/root/kit` on a rented *container* box and runs
`sh /root/kit/tier2_probe_box.sh`. A container box has no root BAR0 path (that script is deliberately tier 2), so the reader
belongs to the privileged lane: put `read_display_caps_page.py` in the kit directory and have the privileged
(KVM/bare-metal) variant run, after the tier-2 probes,
`for b in $(python3 /root/kit/read_display_caps_page.py --list | cut -d' ' -f1); do python3 /root/kit/read_display_caps_page.py $b; done > /root/out/display_caps.txt`
and bring `display_caps.txt` back as text (it holds capability words only).

## 7. What is known about AD106, AD103, GA104, TU106, GB206 (measured vs inferred)

- **Measured (die sweep, `traces/die_sweep_20261014/`):** with `KF3_DISPLAY_CAPS_PROBE=1` (the AD104 page served unchanged,
  caps class `0xc773`/`0x640000`) Windows reached a desktop on AD106 (RTX 4060 Ti) and AD103 (RTX 4070 Ti SUPER), 0 resets in
  a 7-minute hold; with the probe removed AD106 went black with 37-39 resets in 5 minutes; GA104 (RTX A4000) got no
  matching page (`REFUSED: caps class 0xc673 is not the measured one`), armed a head and never scanned a window, 10
  resets in 7 minutes; TU106 `display=on` is refused at realize for a different reason (single-semaphore window class);
  GB206's Windows RM unloads after `GetGspStaticInfo` for a different reason.
- **Inferred, not measured:** that AD106/AD103 need only the same two facts as AD104 (main-layer scaler + `0x00c = 0x100`).
  It is evidence for a family constant — the AD104 page worked there — but it is **not a reading of those dies**, and the
  policy is that a value measured on one die is never served to another. So **the interim for AD106/AD103 (and every
  unmeasured die) is the authored fallback plus the notice**, which means the sweep's AD106/AD103 desktops are *not*
  reproduced until their own `0x00c` is read with the T3 reader. That is a regression of those two dies against the old probe
  flag and is an owner decision (fallback vs a clearly named neighbour row); the code path for either is one line in
  `capsderive::derive`'s T3 lookup.
- GA104/TU106/GB206: the T1 half (window-layer rows) derives for every family class — C573, C673, C773, CA73 all resolve
  the row and place `SCLR_PRESENT` on windows 0/2/4/6 (tested). Whether the same two facts are enough on Ampere is
  **unknown**; falsifier: a GA104 run with the derived T1 page plus GA104's own measured `0x00c` still scans no window.
- A family rule needs every measured die to agree plus a mechanism (ruling §AH). Today: one measured die (AD104), no
  mechanism for `0x00c` (no header names it).

## 8. Risks and what still needs hardware

1. **Not yet booted as the final binary.** The page the product serves equals bisection run f5 word for word (tested), and f5
   gave a desktop on AD104 with the probe binary. Prepared step: `exp2.sh`-style run of the product binary on the host's
   AD104 (≈5 min VM), expected `console shows head 0 …` within ~11 s and 0 `life ENDED`. Waiting for the owner's SolidWorks
   test to finish.
2. **Linux regression check needed.** The page now claims `SCLR_PRESENT` on the main-layer windows for Linux guests too. The
   scaler limit fields stay 0, so NVKMS bounds out every scaling use (`EvoParsePrecompScalerCaps5`, `IsScalingUsageBounded`)
   and should stay 1:1; this is reading, not a run. Per the standing rule, retest the Linux guest with the broker
   (`desktop_smoke.sh`, `planes_oracle.sh`) before merging.
3. **Second die.** No rental was made (budget unused). Plan if wanted: ONE vast KVM Ampere/Ada box, auto-destroy armed,
   `provision_full.sh` for the driver, run the T3 reader first (cheap: a `0x00c` row for that die), then the die-sweep phase
   1/phase 2 with the product binary and no probe flag; the pass condition is a desktop screenshot on that die.
4. The unnamed words `0x008`, `0x48..`, `0xc0..` are not needed on AD104 under Windows 580.88; another driver branch or
   workload (HDR, MPO, DSC, DP) may read them. If a Windows branch shows a new need, bisect with the probe in `bdcd65f15`.
