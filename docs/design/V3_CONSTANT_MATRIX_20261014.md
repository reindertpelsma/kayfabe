# V3 constant matrix: family x constant class x tier x status

**STATUS: LIVE, 2026-10-14 (night run, lane C; branch `claude/constants-night-20261014`).** Owner asleep: every decision taken tonight is in
§6 with its reason. Inputs: `V3_CONSTANT_STRATEGY.md` (tiers, §3 table rules), `V3_CONSTANT_LISTS_20261011.md` (140-row inventory classes),
`V3_WINDOWS_ONLY_CONSTANTS_20261013.md` (the 40 rows no ogkm tag defines), `V3_GSP_CAPTURE_FORMAT.md` / `V3_WINDOWS_CAPTURE_MODULE.md`
(kfwc, kf-census), the hardware evidence named per row. Labels: **[M]** measured (a command whose output is in the repo), **[R]** read in
source (file:line), **[I]** inferred. An inference is never written as evidence.

## 0. How to read it

**Tiers** (owner-binding order): **T1** derive (ogkm, generated tables, kayfabe's own state) → **T2** unprivileged host RM control at
realize/runtime (`CAP_SYS_ADMIN` is the privilege line, not uid) → **T3** privileged measurement per (die, driver branch), published in the
table → **T4** reverse engineering, private, minimal, never in the repo.

**Status words** (one per cell):

| status | meaning |
|---|---|
| DERIVED | T1: generated from ogkm or computed from kayfabe state; nothing per die is kept |
| HOST | T2: kayfabe asks the host GPU (unprivileged) at realize, with a request it authors; per die by construction |
| RULE | hand-kept family row with an ogkm mechanism (allowed: "only family rows are maintained by hand") |
| AUTHORED | a value kayfabe states about the device it presents (w827 "we are the GSP"), with a stated reason |
| TABLE | T3: served from the published table `kf_rm::consttab` only on the (die, branch) it was measured on |
| CAPTURED | a value captured on one die and served to others: **the defect v3 exists to end** |
| MEASURE | not obtainable at T1/T2; needs a privileged measurement per die (KVM rental / bare metal / kfwc) |
| REFUSED | the clean fallback: refused by name (and counted), nothing claimed |
| RE? | only reverse engineering is left (none tonight; §7) |
| n/a | the family has no such thing |

## 1. Summary

- **The boot-critical per-die facts are already derived or host-answered on every family** [R]: the family and die come from the host's own
  `MC_GET_ARCH_INFO`; class sets are generated from ogkm and intersected with the host's class list; `HostFacts` (39 fields, `kf-rm/src/hostfacts.rs`
  PROVENANCE) is 31 host controls, 2 family rules, 4 authored, 2 advertised, 0 unsourced. Linux guests passed the bar on TU116, GA106, GA104,
  GA102, AD106, AD104, GB203, GB205 with no per-die table (§2 evidence).
- **What was still CAPTURED and served to every die** (the list this night started from, §2.3): libcudart's init gate (GA106 rows) — **fixed tonight**
  (host first, then the published table per die, else refused); and `0x20808162` (GA106's byte 1 was wrong on the AD104 at 595, which answers 0 [M]) — **fixed tonight** too; still open: ZBC default entries (GA102), and several
  AUTHORED numbers that were *chosen* equal to a GA106 (`GMMU_STATIC`, `CE_FAULT_METHOD_BUFFER_SIZE`, stall vectors, compression page, BAR0-size
  fallback). Authored values are allowed (we are the GSP), but where a real per-die value exists it is now being measured (§5).
- **The constant table exists** (`kf_rm::consttab`, `crates/kf-rm/data/measured_constants.tsv`): exact (die, branch) rows, mechanism-tagged
  wildcards only, `blob`/board data refused, request words part of the key, refusals named and counted. It is the only home of a measured per-die value.
- **Tier 2 grew tonight** [M]: at host 595.91.07 every GSS row kayfabe asks (cudart gate, cudart/NVENC clocks) answers an unprivileged client, so the
  "not served" lines of the owner host are gone; seven Windows-only GSS queries whose real request is all-zero are answered with the host's bytes.
- **Windows guests**: only the home AD104 has run Windows under kayfabe [R]. On other dies the same derived facts apply; the Windows-only rows are
  per-die unknowns until measured (kfwc lane, §5) — the answer there is the counted refusal, never another die's row. Lane B is booting Windows
  under kayfabe on GA104/GB206 tonight (`V3_DIE_SWEEP_20261014.md`); it found a kf-disp code gap on GB20x (display=on refused: no ctxdma methods
  in CA7D/CA7E), which is not a constant.
- **Reverse engineering: none needed tonight**; the RE list stays empty (§7).

## 2. The matrix

Columns: **TU** Turing TU10x/TU11x · **GA10x** Ampere consumer · **GA100** · **AD** Ada AD10x · **GH** Hopper GH100 · **GB20x** Blackwell consumer ·
**GB10x** Blackwell datacenter. Cells: status word + note. Sources: the research pass over the code at `80f4d02b8` (file:line below), re-checked
where tonight's commits changed a row. Hardware column at the end of each row group: what actually ran.

### 2.1 Identity, classes, registers, boot

| class | TU | GA10x | GA100 | AD | GH | GB20x | GB10x | tier | where |
|---|---|---|---|---|---|---|---|---|---|
| family + die | DERIVED | DERIVED | DERIVED (then refused: no GSP model) | DERIVED | DERIVED (SOC refused) | DERIVED | DERIVED (GB10B refused: integrated) | T1/T2 | `kf-chip/src/lib.rs:91-173`, `die.rs`, `hostfacts.rs` `die` |
| engine class sets | DERIVED | DERIVED | DERIVED | DERIVED | DERIVED (no NVENC) | DERIVED | DERIVED | T1 ∩ T2 | `kf-chip/src/classes.rs:170-300` (generated from ogkm 610.43.02), `host_classes.rs` |
| register offsets | RULE (hand literals checked against generated `dev_*.h`) | RULE | RULE | RULE | RULE | RULE (7 disagree with the header) | RULE (7 disagree) | T1 check | `kf-chip/src/hwref.rs`, `data/hwref-580.159.04.tsv`, `V3_HW_BOUNDARY_INVENTORY.md:123-133` |
| GSP boot model | RULE `Tu102` RISC-V, FWSEC V2, no heartbeat | RULE `Ga102`, FWSEC V3, heartbeat | **REFUSED** `RowUnbuilt` | RULE (= GA10x) | RULE FSP | RULE FSP, thermal gate | RULE FSP (carries GB20x gate) | T1 | `kf-chip/src/lib.rs:225-339`, `fsp_gsp.rs:280-302` |
| MMU format / USERD / chids | RULE VER2 | RULE VER2 | RULE VER2 | RULE VER2 | RULE VER3 (some PDE encodings not honoured) | RULE VER3, GP_GET not written back [M GB203] | RULE VER3 | T1 | `kf-chip/src/lib.rs:183-215`, `ptekind.rs` |
| synthetic VBIOS | DERIVED (FWSEC V2) | DERIVED (V3) | unreachable | DERIVED (V3) | inert (FSP) | inert | inert | T1+T2 (host version) | `kf-chip/src/bar0.rs:318-361` |
| display row | DERIVED v0400 (never run on hw) | DERIVED v0401 | REFUSED | DERIVED v0404 | REFUSED | DERIVED v0502, boot layer only; **display=on refused at realize** [M lane B GB206] | REFUSED | T1 | `kf-chip/src/display.rs:73-87`, `kf-disp` |

### 2.2 Facts the emulated GSP serves (static info, init tables, controls)

| class | TU | GA10x | GA100 | AD | GH | GB20x | GB10x | tier | where |
|---|---|---|---|---|---|---|---|---|---|
| GR geometry, GR info, context buffers, zcull, SM order | HOST | HOST | HOST† | HOST | HOST† | HOST | HOST† | T2 | `hostfacts.rs:311-366` |
| FB info, L2/LTC, cache info | HOST | HOST | HOST† | HOST | HOST† | HOST | HOST† | T2 | `hostfacts.rs:275-281`, forwarded indices |
| CE caps, PCE masks, engine types/counts | HOST (slots AUTHORED per family) | HOST | HOST† | HOST | HOST† | HOST (GRCE ×4) | HOST† | T2 | `hostfacts.rs:233-239`, `authored.rs:535-560` |
| interrupt table / subtree map | HOST (+RULE map) | HOST | HOST† | HOST | HOST† | HOST (no family map: refused if the host cannot report) | HOST† | T2 | `kf-chip/src/lib.rs:392-400` |
| names, VBIOS version, GID input, GSP features, PCIe gen, SMC mode | HOST | HOST | HOST† | HOST | HOST† | HOST | HOST† | T2 | `hostfacts.rs` |
| video caps, FIFO latency, perf level info | HOST | HOST | HOST† | HOST | HOST† (no NVENC) | HOST | HOST† | T2 | `hostfacts.rs:442-469` |
| GSP static info (fn 65) | DERIVED/HOST | DERIVED/HOST | — | DERIVED/HOST | DERIVED/HOST† | DERIVED/HOST | DERIVED/HOST† | T1+T2 | `gspstaticinfo.rs:661-694`, `staticinfo.rs:403-459` |
| GMMU fault-buffer sizes | AUTHORED (GA106-chosen) | AUTHORED | AUTHORED | AUTHORED (AD104 real 0x54000/0x220c20 [M]) | AUTHORED | AUTHORED | AUTHORED | ours (§7 D-1) | `authored.rs:71-87` |
| CE fault-method buffer size | AUTHORED (GA106-chosen 0x5000) | same | same | same | same | same | same | ours; real value KERNEL_PRIVILEGED (T3) | `authored.rs:52-64` |
| FECS record size, stall vectors, MMU fault ids | AUTHORED / RULE | same | same | same | RULE (384/43/64) | RULE (384/65/85) | RULE | T1 | `authored.rs:94-151,436-444` |
| memsys compression page / comptags | AUTHORED (GA106-equal [I] elsewhere) | AUTHORED | AUTHORED | AUTHORED | AUTHORED | AUTHORED | AUTHORED | ours | `hostquery.rs:741-781` |
| ZBC table ranges / default entries | HOST / **CAPTURED** (GA102 defaults) | same | same | same | same | same | same | T2 / defect | `kf-rm/src/zbc.rs:20-27,89-90` |
| report-semaphore / notifier `OFFSET_UPPER` | RULE 7:0 ✓ | ✓ | ✓ | ✓ | **wrong: ogkm `clcb97.h:42,3913` is 24:0**; refused above 0xff, never truncated | report semaphore REFUSED; compute notify: `clcdc0.h:39` is 24:0, kayfabe 7:0 | REFUSED | T1 (fix: §7 F-2) | `kf-chan/src/grtables.rs:120-160,399-403` |
| gfx pool sizes | AUTHORED (virtual, ≤4096 slots, flag `KF3_GFX_POOL_PROBE`; 64 is the SDK's add/remove batch array, not a pool limit) | same | same | same | same | same | same | ours | `kf-abi/src/gfxpool.rs` |

† source-derived only: no die of that column has run kayfabe (GA100 is refused at realize before any of it is asked).

### 2.3 GSS-legacy and other opaque controls (no open header)

| class | all families | tier | where |
|---|---|---|---|
| cudart init gate `0x20809009/9001/9064/a001` | **HOST** (cudart's own request, asked at realize) → **TABLE** (GA106/580, AD103/575, AD104/595) → REFUSED. Was CAPTURED (GA106) until tonight | T2 → T3 | `gssreplay.rs`, `consttab.rs`, `inittables.rs` cudart arm |
| RC watchdog info `0x20802209` | RULE: RM software state (INITIALIZED\|DISABLED = 5), ogkm `kernel_rc_watchdog_ctrl.c:30-60` | T1 | `measured_constants.tsv` (wildcard row with mechanism) |
| cudart / NVENC clocks `a084/a026/a028`, encoder listing `9064 {0,1,8}` | HOST (hosts in [580.65.06,581) and, since tonight, 595.91.07) | T2 | `gssreplay.rs`, `hostabi.rs` |
| encoder session `8163/8164` | HOST state (acquire/release on the host's own slot) | T2 | `chanlink.rs` |
| Windows-only exact-zero queries `852a/8524/a70a/a088/852e/853a/a618` | **HOST** (new tonight; refused where the host refuses, e.g. 852e/853a/a618 at 595.91.07) | T2 | `gssreplay.rs` exact rows |
| `0x20808159` | echo of the request (behaviour measured on GA106 only) | T1 [I elsewhere] | `kf-abi/src/gsslegacy.rs:187` |
| `0x20808162` | **HOST** (exact 1-byte row) → TABLE (GA106/580 = 1, AD104/595 = 0) → REFUSED. Was CAPTURED (GA106's 1 on every die) until tonight, and **wrong on the AD104 at 595.91.07, which answers 0** [M] | T2 → T3 | `gssreplay.rs`, `measured_constants.tsv`, `inittables.rs` 8162 arm |
| other Windows-only rows | REFUSED (counted) or probe-flag answers; §3 | T2/T3 | `V3_WINDOWS_ONLY_CONSTANTS_20261013.md` |

### 2.4 Driver axes

| axis | state | where |
|---|---|---|
| guest driver | 31 ogkm tags carry generated tables (535.309.01..615.71.09), exact-tag lookup; proven on GA102: 580.159.04/580.105.08 (thin + ladder), 590.48.01/595.84/575.57.08/610.57.04 (ladder) | `kf-abi/src/generated/matrix.rs`, `V3_DRIVER_MATRIX.md:523-541` |
| host driver | encoders written for [580.65.06, 581); other hosts only as exact measured layouts; measured hosts 575.57.08, 580.65.06, 580.95.05, 580.159.04 (GA102) and 595.91.07 (AD104); GSS rows: all closed at 595.91.07 tonight except the three INVALID_ARGUMENT rows | `kf-abi/src/hostabi.rs`, `host_driver.rs` |
| guest OS | Windows only on the home AD104 (580.88; Win11 and LTSC); Linux on every die in §2.5 | `STATUS_AND_HANDOFF.md` |

### 2.5 Hardware evidence per die (what actually ran kayfabe)

| die | family | ran | evidence |
|---|---|---|---|
| TU116 GTX 1660 SUPER | Turing | tests 1742/0, gates 9/9, bare 30/30, thin 30/30, CUDA ladder 4/4+4/4 (2026-09-30) | `traces/v3_turing_master/` |
| GA106 RTX 3060 | Ampere | merge bar, gates, 30/30, CUDA/LLM/display lanes (to 2026-10-08) | `traces/merge_bar_20261008/` |
| GA104 RTX 3060 Ti (floor-swept) | Ampere | thin 30/30, cup3/cup8 (2026-09-26) | `traces/real_ga104/` |
| GA102 RTX 3080 Ti | Ampere | driver matrix: 4 hosts x 15 guests (2026-09-26/27) | `V3_DRIVER_MATRIX.md` |
| AD106 RTX 4060 Ti | Ada | bare 30/30, thin 30/30 (2026-09-26) | `V3_FAMILY_PORT_ADA.md` |
| AD104 RTX 4000 Ada | Ada | 1744/0, 9/9, 30/30, 30/30, ladder (2026-09-30) | `traces/v3_families_master/` |
| AD104 RTX 4070 (home) | Ada | Linux bar at 595.91.07; **all Windows-guest runs** | `traces/windows_*` |
| AD103 RTX 4070 Ti SUPER | Ada | no kayfabe run: unprivileged probes (Linux 575.51.03) and kfwc under stock Windows (2026-10-13/14) | `traces/real_hw_20261013/`, `traces/real_windows_20261014/` |
| GB203 RTX 5080 | Blackwell | bare 30/30, thin 30/30, ladder, gates (2026-09-26) | `V3_FAMILY_PORT_BLACKWELL.md` |
| GB205 RTX 5070 | Blackwell | 1742/0, 9/9, 30/30, 30/30, ladder (2026-09-30) | `traces/v3_families_master/` |
| GA100, GH100, GB10x | — | none (GA100/GB10B refused by name; GH100/GB10x source-derived only) | `STATUS_AND_HANDOFF.md:2253` |

## 3. The 40 "Windows-only" rows after tonight

The 40 rows of `V3_WINDOWS_ONLY_CONSTANTS_20261013.md` (no ogkm tag defines them; 34 distinct control ids). Family columns are not split
here because every measurement so far is Ada (AD104 home host, AD103 rental); the per-die columns of the GA104/GB203 rentals are in §5.
"kayfabe now" is this branch.

| row(s) | what | real-driver request [M] | host answers it unprivileged [M] | kayfabe before | kayfabe now | tier |
|---|---|---|---|---|---|---|
| `0x2080852a` (1784 B), `0x20808524` (4), `0x2080a70a` (26), `0x2080a088` (12) | GSS-legacy, purpose unknown | all-zero (Windows 580.88 / GSP 580.65.05, AD103; a088 6 of 12 calls) | AD104 595.91.07 OK; AD103 575.51.03 OK | refused | **HOST** (exact-zero `gssreplay` rows; any other request refused) | T2 |
| `0x2080852e` (528), `0x2080853a` (1456), `0x2080a618` (8620) | GSS-legacy, purpose unknown | all-zero | AD103 575.51.03 OK; **AD104 595.91.07 INVALID_ARGUMENT** | refused | **HOST** where the host answers (inside [580.65.06,581) and any measured version that does), REFUSED at 595.91.07 | T2 / gap |
| `0x2080852f` (776) | GSS-legacy | NOT a zero query: word 1 = 3 plus ~80 non-zero words, several shaped like user-space addresses (read host-side from capture C with a one-off scrub allow; bodies not kept) | AD103 OK, AD104 INVALID_ARGUMENT (zero request) | refused | REFUSED | needs the request layout (T3), then maybe T2 |
| `0x20809001`, `0x20809009` (8) | cudart init gate | all-zero (Windows and cudart) | AD104 595.91.07 OK; AD103 575 OK; GA106 580 OK | **CAPTURED** (GA106 row to every die) | **HOST**, then TABLE (GA106/580, AD103/575, AD104/595), else REFUSED | T2 (+T3 fallback) |
| `0x20809064` (520) | clock-domain listing; reply depends on the request | cudart `{0,2,1,..,4}`; encoder `{0,1,8}`; Windows `{0,3,1,..,4,..,8}` | cudart and encoder requests OK at 595.91.07 | **CAPTURED** (cudart reply to every request) | **HOST** for the cudart and encoder requests; TABLE (cudart request only, GA106/580, AD104/595); the Windows 3-entry request REFUSED | T2 |
| `0x2080a084`, `0x2080a026`, `0x2080a028`, `0x2080a001` | cudart / NVENC clock queries | measured requests (gssreplay) | **all OK at 595.91.07 tonight** | HOST at 580.x, REFUSED at 595.91.07 ("not served") | **HOST at 595.91.07 too** | T2 |
| `0x20809004` (1544) | GSS-legacy, telemetry-like (118 distinct replies in 640 calls) | non-zero selectors (`{_,3,_,1,..}`) | zero request INVALID_PARAM_STRUCT | refused | REFUSED | T2 if a request layout is measured; state, not a constant |
| `0x2080a060`, `0x2080a630`, `0x2080a801`, `0x2080b201` | GSS-legacy | non-zero request words (a060 values, a630 0xffffffff fills, a801 `{1,0x40003,0x38}`); b201 zero in 1 of 3 calls | zero request OK on AD104 (a060/a630/a801); b201 zero INVALID_ARGUMENT at 595.91.07 | refused | REFUSED (a zero-request answer would answer a different question) | MEASURE layout |
| `0x2080853b`, `0x2080a619` | GSS-legacy | 853b: **issued once on the AD103** (1832 B, request `{0, 1, 0…}`; never in the five AD104 captures, and 1832 is not a size the probe ladder tried); a619: not seen | INVALID_ARGUMENT at every ladder size | refused | REFUSED | T2 candidate for 853b: probe `{0,1}` at 1832 B |
| `0x2080a02f` | GSS-legacy | — | NOT_SUPPORTED (the real GPU too) | refused | REFUSED (equals the real answer) | done |
| `0x00809908`, `0x20810108`, `0x2081010d`, `0x20801111` | NV0080 / NV2081 binary-API / sw-runlist submit | — | not probed (no such object in the probe) | refused / zero-fill / echo | unchanged | MEASURE (kfwc per die) |
| `0x20802806` | privileged, absent from open tables | — | INSUFFICIENT_PERMISSIONS unprivileged, OK as root (AD103) | refused | REFUSED | T3 (group P/A) |
| `0x00730122`, `0x00730128`, `0x007302a3`, `0x007302a5`, `0x0073011a` | display-common, absent from open tables | Windows-only traffic (not on Linux) | not probed (needs the display-common object) | probe-flag only / echo / refused / authored | unchanged | T3 per die with a head (kfwc on a rental has none) |
| core latch, frame edge, LOADV, sw-runlist admission | behaviour/timing, not RM traffic | — | — | authored / flags | unchanged | oracle tests on display machines (§7) |

## 4. What was implemented tonight

Branch `claude/constants-night-20261014` = `integration/prod-candidate-2-20261013` (`bf510bdf4`) + the evidence branches
`claude/kfwc-real-windows-20261014` (with gsp-census, windows-capture-module, constant-strategy), `claude/ctl-probe-20261013`,
`claude/default-route-evidence-20261013`, `claude/constant-inventory-20261011` (inventory TSV conflict: the integration copy kept, it already
held the inventory's last row) + these commits. GPU-free: `cargo test -p kf-abi -p kf-rm -p kf-chip -p kf-host` 1486/0 after the first commit,
kf-qemu/kf-harness/kf-census 334/0; every later commit re-ran kf-abi/kf-rm/kf-host/kf-harness green.

| commit | what |
|---|---|
| `80f4d02b8` | `kf_chip::DieId` (die from `MC_GET_ARCH_INFO`, ogkm names, discrete dies only); `kf_rm::consttab` + `data/measured_constants.tsv` (the published table, §0 rules enforced by tests); `HostFacts.die`; the cudart arm answers from the table, not from the GA106 rows; gssreplay asks cudart's own requests (9009, 9001, 9064 `{0,2,1,..,4}`, a001) at realize; hostabi carries 9064/a084/a026/9009/9001 at 595.91.07; tests `cudart_gate_per_die` (neighbour die refused, host answer serves any die, the request is part of the question, the mechanism row answers every die) |
| `a0ecfb17b` | gssreplay `exact` rows: 852a, 8524, a70a, a088, 852e, 853a, a618 asked once with zeros (no background probe) and answered only to an all-zero guest request; tests `gss_exact_zero_rows` |
| `527a125de` | corrections folded above `V3_CONSTANT_STRATEGY.md`, `V3_WINDOWS_ONLY_CONSTANTS_20261013.md`, `V3_DRIVER_MATRIX.md` |
| `df45a72cb` | `tools/derive_die_names.sh`: re-derives `DIE_NAMES` from any ogkm tree and diffs (OK at 595.84) |
| `7788750b9` | `kf-ctl-probe` `KF_PROBE_GSS_ROWS=1` (asks exactly kayfabe's realize-time GSS requests); a028 and a001 measured at 595.91.07 → `MeasuredOpaque` |
| (next) | `0x20808162`: probe at 595.91.07 answers `00` where the GA106 capture says `01`; exact 1-byte gssreplay row, hostabi `MeasuredOpaque` 595.91.07, table rows GA106/580 = 1 and AD104/595 = 0, the 8162 arm answers from the table; byte-granular table rows |
| (this doc) | the matrix; `kf-ctl-probe` extra rows with measured request words (`KF_PROBE_EXTRA_ONLY`) |

Also fixed on the way: `kf-abi/tests/truncated_row_reads.rs` was red on the merged tree (`kf-ctl-probe` mentions `0x20800b03`); the probe is a
measurement, not a reader of the captured row, so it is listed with the other self-references.

## 5. Measurements tonight, per die

### 5.1 AD104 (home host, RTX 4070, host driver 595.91.07) — tier 2, unprivileged (uid 1000, CapEff 0)

Every run had its falsifier written in the evidence header before it ran; the Xid count was taken before and after each run (unchanged by every
probe; the Xids in the window belong to lane A's `qemu-system-x86` pids). Evidence: `traces/constants_night_20261014/`.

| run | what | result [M] |
|---|---|---|
| `host_cudart_probe_595.txt` | libcudart 13.3 (`cudaGetDeviceCount` + two clock attributes) under the nvdiff recorder | 9009/9001/9064/a084/a026 issued and answered NV_OK; cudart's 9064 request is `{0,2,1,0,0,0,4}` (the inference from the GA106 reply holds); 9001/9009/9064 replies equal the GA106 rows; libcuda's own `0x20808162` gets `00` |
| `gss_rows_probe_595.txt` | `kf-ctl-probe KF_PROBE_GSS_ROWS=1`: exactly kayfabe's realize-time GSS requests | 20 OK; 852e/853a/a618 INVALID_ARGUMENT; a028's background probe refused (zero probe kept, by design) |
| `probe_8162_595.txt` | `0x20808162`, 1 byte, request `00` | **`00`, not GA106's `01`** — the captured byte was wrong on this die/driver |
| `probe_853b_595.txt` | `0x2080853b` with the Windows request shape (1832 B, word 1 = 1) | INVALID_ARGUMENT |
| `kit_ad104_595.txt` | the rental kit's dry run | as above; a001 = `{1,5,2,0x11}` (= GA106) |

### 5.2 Windows request shapes (AD103 kfwc captures of 2026-10-14, read host-side, bodies not kept)

852a/8524/a70a/852e/853a/a618 all-zero; a088 zero in 6 of 12 calls; 9064 `{0,3,1,0,0,0,4,0,0,0,8}` (a third request shape); 852f carries ~80
non-zero words incl. user-space-address-shaped values; 853b `{0,1}` at 1832 B (issued once; never in the AD104 captures); b201 zero in 1 of 3.

### 5.3 Other dies (rentals)

<!-- RENTALS -->

## 6. Decisions taken tonight (owner asleep)

| # | decision | why |
|---|---|---|
| D1 | The table lives in **kf-rm** (`kf_rm::consttab`, data `crates/kf-rm/data/measured_constants.tsv`), not kf-chip | kf-chip's charter is "never a per-die constants file"; kf-rm is the crate that answers the guest and already carries `HostFacts`. The table is the published measured table §AF allows, and its rules are tests, so it cannot quietly become a captured table |
| D2 | Key = (ogkm die name from `MC_GET_ARCH_INFO`, driver **branch** = major), plus the request words | §3: measured per die and per firmware/driver branch; a family is never a key. The branch is the **guest's** (the GSP version the guest believes it talks to). Exact-patch keys would never hit; whole-branch keys follow §3's "branch" wording |
| D3 | A wildcard row needs `ogkm:`/`rule:`/`authored:`; a `measured:` row is never a wildcard | §3 "a family rule only with a mechanism"; "consistent so far" (9001/9009 equal on GA106, AD103, AD104) is recorded as three rows, not as a rule |
| D4 | The cudart gate on an unmeasured die with no host answer is **refused** (counted), although refusing it is measured fatal to libcudart | the brief and §AE: refuse-and-log, never invent. Mitigation, not exception: the host path now answers on every host driver measured (580 interval and 595.91.07), so the refusal is reached only on an unmeasured host driver on a die with no row — where the old code served GA106 bytes |
| D5 | `0x20802209` gets a wildcard RULE row (5 = INITIALIZED\|DISABLED) | ogkm answers it in kernel RM from RM software state (`kernel_rc_watchdog_ctrl.c`), not from the die; refusing it on non-GA106 dies would break libcudart for no physical reason |
| D6 | Exact-zero GSS rows are asked once with zeros, never with the 0xa5 background | the layout is unknown: a background fill is a different question to the host, with unknown effect |
| D7 | No change to authored values that were *chosen* equal to a GA106 (GMMU, CE fault buffer, vectors, compression page) | they are statements about OUR device (w827) and changing fault-buffer or vector numbers under lane A's live Windows work is a behaviour change with no measured need; the real per-die values are being measured (§5) for the owner to decide (§7 D-1) |
| D8 | Windows display-common rows (`0x73xxxx`) and the 5070 caps unchanged | they need a lit head or a privileged display object; rentals have neither; and lane A is stabilising Windows display tonight |
| D9 | Hopper/Blackwell `OFFSET_UPPER` width not fixed tonight | code in `kf-chan` (lane A's channel plane); the defect refuses, it never truncates, and no Hopper/GB10x has run; recorded as F-2 |
| D10 | Host probes were run on the owner host beside lane A's running VM | the night plan allows short host probes; they are unprivileged RM queries kayfabe itself issues at realize; Xid counts were taken before and after each run, and every Xid in the window belongs to a `qemu-system-x86` pid |
| D11 | Measurement rentals: Windows (kfwc) on GA104 and GB203; no Turing | no Turing KVM offer existed at 02:15 (lane B saw the same); Blackwell under $0.60/h was available |

## 7. Open: owner decisions, measurement, reverse engineering

**Owner decisions**

- **D-1 Authored vs measured per-die values.** `GMMU_STATIC` (fault-buffer sizes), `CE_FAULT_METHOD_BUFFER_SIZE` and the compression page are authored
  numbers chosen equal to a GA106. Real values differ per die (AD104 GMMU 0x54000/0x220c20 [M]). Options: (a) keep authored (ours, w827); (b) serve the
  die's measured value from `consttab` when a row exists, authored otherwise. Recommendation: (b) for GMMU only after a Windows A/B on the AD104
  shows no regression; nothing tonight.
- **D-2 Strict refusal of the cudart gate on an unmeasured (die, host driver).** Accept D4 as the rule, or allow a "consistent so far" row class for
  values equal on ≥3 dies of ≥2 branches (9001/9009 today). Recommendation: keep strict; extend the host path instead (it costs one probe per host
  driver, `kf-ctl-probe KF_PROBE_GSS_ROWS=1`).
- **D-3 Are the seven exact-zero Windows rows wanted** (they change what the Windows guest sees, toward the real GPU: the real GPU answers them)? Default
  taken: yes, because it is the VFIO-reference behaviour; revert by deleting the rows if a Windows A/B regresses.
- ~~D-4 `0x20808162`~~: done tonight (host path + table per die), because a measurement showed the captured byte wrong on the AD104.

**Fixes found (not done tonight)**

- **F-1 ZBC default entries are a GA102 capture** served to every die (`kf-rm/src/zbc.rs:20-27,89-90`). No ogkm mechanism (physical RM installs
  them; nothing in the 595.84 tree). Impact today: none [R] — every host-twin mapping is an uncompressed kind, so no ROP ever resolves a ZBC index
  (`zbc.rs` module docs); it re-opens with compression. Route then: read the host's own entries (`GET_ZBC_CLEAR_TABLE_ENTRY` on kayfabe's ZBC
  object, T2) — but the host table is GPU-global and other host processes add entries, so "which slots are the defaults" needs a rule first; or
  table them per die. Not changed tonight.
- **F-2 `OFFSET_UPPER` width**: Hopper 3D (`clcb97.h` 24:0) and Blackwell compute (`clcdc0.h` 24:0) are 25 bits in ogkm 595.84; kayfabe re-authors 8
  bits on every family (`kf-chan/src/grtables.rs`); a family row by ogkm header, T1.
- **F-3 Register offsets**: GB10x and GB20x each have 7 hand literals that disagree with the generated header (`V3_HW_BOUNDARY_INVENTORY.md`).
- **F-4 GSS rows at 595.91.07**: 852e, 852f, 853a, 853b, a618, b201 answer `INVALID_ARGUMENT` to the requests that AD103/575.51.03 and the Windows
  driver's GSP 580.65.05 accept [M]: these controls changed between firmware branches. They stay refused at 595.91.07 until a 595 request shape
  is measured (a kfwc capture of a 595-branch Windows driver, or a Linux 595 driver trace).
- **F-5 Stale text**: `V3_HW_BOUNDARY_INVENTORY.md:151-156` says TU10x has no measured row (TU116 ran on 2026-09-30).

**Needs measurement (T3)** — per die, per branch; the kfwc lane (§5) is the route for everything the Windows driver sends to the GSP:
group P (`0x731369`, `0x20802801`, `0x20802a12`, `0x50700709`, `0x50700118`: admin), group K (`0x73014b`, `0x7302a2`, `0x2080121f`, CE fault-method
buffer, gfx pool sizes: kernel-only), group I (GMMU static, user register access map, `0x20800a3f/a80/a2e/a87/ab8/b03`: internal), group A
(`0x730122/128/2a3/2a5`, `0x20802806`: absent ids; display ones need a head). Display-state rows need a display machine (§9 of the strategy).

**Reverse engineering**: none requested or done tonight. The RE list stays empty; the behaviour rows (core latch, frame edge, LOADV, sw-runlist
admission) go to oracle tests on the owner's display machines first, per `V3_CONSTANT_LISTS_20261011.md` revision.

## 8. Falsifiers

| claim | falsified by |
|---|---|
| a measured row is never served to another die | the tests `consttab::a_neighbour_dies_row_is_never_served` and `cudart_gate_per_die::a_neighbour_die_is_refused_not_served_ga106s_bytes`. **Mutation check run [M]:** `lookup_in` changed to match any die of the same family → both tests fail (the integration test bites through GA104, same family and branch as the GA106 rows; it was added for this) |
| the table carries no blob and no board data | `consttab` tests (`blob_and_untagged_rows_are_refused`, `board_data_is_refused`); the published file re-validated on every build |
| the host answers cudart's gate at 595.91.07 | a 595.91.07 host whose realize log prints "GSS 0x2080900x … refused by the host" |
| the cudart 9064 request is `{0,2,1,..,4}` | a libcudart whose 9064 request differs (the guest is then refused, by name: `W349REFUSE … consttab … not the measured question`) |
| exact-zero rows answer the real driver's question | a Windows boot whose request for one of the seven is non-zero (then not answered: unchanged behaviour), or a reply that changes within a boot (one distinct reply each over three AD103 captures today) |
| DIE_NAMES is ogkm's | `tools/derive_die_names.sh <ogkm>` printing a difference |
