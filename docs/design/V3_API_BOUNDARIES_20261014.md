# V3 API boundaries per ogkm release tag: where a driver sweep must cross, and where it may interpolate

**STATUS: RESEARCH, 2026-10-14** (branch `claude/api-boundaries-20261014`; tooling `tools/api_boundaries/`; derived tables `docs/design/v3_api_boundaries/`).
Nothing here changes behaviour: it is a measurement of NVIDIA's open sources and a plan derived from it. Labels: **[M]** measured by a script whose output is committed,
**[R]** read in a committed file, **[I]** inferred (never evidence). Every hypothesis states its falsifier before the result (§8). Only NVIDIA's open source
(`github.com/NVIDIA/open-gpu-kernel-modules`, "ogkm") was read; no closed binary was read or disassembled. The kfwc / observer census tables that are quoted
come from earlier lanes' committed traces (they list message sizes only, no bodies).

**The owner's rule under test (2026-10-14).** (1) Every meaningfully changed ogkm API boundary can also affect the Windows driver, so it is a place where we must
measure or handle. (2) If driver versions A < B < C and A and C have the same API signature, B very likely has the same, so one driver per interval of identical
signature needs a measurement.

## 0. Result on one screen

- **Tags [M]:** 221 release tags exist, 515.43.04 ... 615.78.08, 17 R-branches (515 to 615). None is missing: 595.91.07 (the owner host's driver) is there, and so are 595.99.02 and 595.104.02 after it. Newest tag: 615.78.08.
- **Intervals [M]:** over all 221 tags the RM controls kayfabe serves or probes fall into 29 intervals (28 change points + 1), the controls
  a Windows driver issued into 35, alloc/NVOS parameter structs 14, class defines and headers 41, GSP structs kayfabe names 54
  (the whole GSP/msgq/WPR/FSP/rpc surface 63). Inside kayfabe's supported window (535.309.01 .. 615.78.08, 134 tags) the plan below needs
  **15 host-driver groups and 16 Windows-driver groups**, of which 4 and 2 are already measured, so **11 host versions and 14 Windows versions remain** (§7).
- **F-4 from source [M]:** *the source does not show the 580 to 595 change, because the six controls are not in the source at all.* `0x2080852e/852f/853a/853b/a618/b201`
  (and `0x20808162`) are defined in none of the 221 tags (headers, RM export tables) and the literal ids occur in no file of the complete 595.91.07 tree (3810 files).
  What does show the change is the **Windows captures [M, earlier lanes' census files]**: the same controls changed request size from the 580.88 driver to the 596.51 driver
  (`852e` 528 to 596 B, `853a` 1456 to 15352 B, `853b` 1832 to 14320 B, `a618` 8620 to 8632 B, `b201` 2188 to 2444 B). That supports the "shape changed between firmware branches" reading of F-4: the 595.91.07 refusal is not only a host-driver quirk, the guest-side
  request changed too (the sixth, `852f`, was not captured on the GA104/GA106 boxes). Whether the host accepts the new shapes is a one-probe test, proposed in §5. The ogkm signature is blind to the 13 controls whose size changed and that
  have no ogkm definition (§5).
- **The rule [M]:** the falsifier fires. 12 of 201 changing served controls, 10 of 121 Windows-issued controls, 23 of 310 named GSP items and 14 of 335 class items are non-monotone in version order; the rest are monotone. The violations look mostly like *parallel lineages* [I] (patch tags
  such as 525.47.x vs 525.53, 535.43.x, 580.94.02/03 and 580.95.05) rather than real regressions (§6). Items with a violation are excluded from the interval partition and need a
  per-tag measurement; with the proposed measured set the rule then mis-infers 19 (host) and 4 (Windows) of about 61 000 and 89 000 (item, tag) inferences, all on excluded items.
- **Where the rule cannot reach [I]:** it covers declared interfaces only. It does not cover values, behaviour, firmware behaviour, or the 97 control ids that kayfabe names and ogkm never defines (§4, §6.2).

## 1. Method

**What a signature is.** Text signatures, no compilation: comments are stripped and whitespace collapsed, so a comment-only edit is not a change. A parameter struct's signature is the hash of its
definition together with the transitive closure of every struct, union, enum and `#define` it names (an array bound change in a macro shows up in the parent). One script per step, all committed:

| step | script | what it does |
|---|---|---|
| served set | `derive_served.py` | control names and ids used in code (comments stripped) in every `crates/kf-*` crate, plus the ids in `extra_ids.txt` (matrix section 2.3/3/7: the six F-4 controls, `0x20800a59`, `0x20808162`, groups P/K/I/A, the GSS-legacy rows) and `windows_ids.txt`; allocation / NVOS parameter struct names |
| Windows set | `windows_controls.py` | merges the kf-census `*.controls.tsv` of the AD104 observer captures, the AD103 kfwc captures, GA106 580.88 and GA104 596.51 into `windows_controls.tsv` (249 control ids a Windows driver issued; no bodies) |
| per tag | `sweep_setup.sh`, `sweep.py`, `extract_tag.py` | blobless sparse clone (about 60 MB on the host, `/data/claude-ogkm-hist`), one checkout per tag, then the signatures below |
| analysis | `analyze.py` | change points, intervals, the rule test, the measurement plan; `kf_idents.py` marks which GSP items kayfabe names |
| reproduce | `run_all.sh` | the whole chain |

**Items per tag** (about 2 700 per tag; per-tag JSON stays on the host, the lossless run-length form `item_runs.json.gz` is committed):

| item | signature |
|---|---|
| `ctrl:0xID` | wire view: the RM class that exports it, `paramSize` expression, closure hash of the params struct, **privilege/routing flag names** (`PRIVILEGED`, `NON_PRIVILEGED`, `PRIVILEGED_IF_RS_ACCESS_DISABLED`, `INTERNAL`, `ROUTE_TO_PHYSICAL`); `ABSENT` when no header and no export table defines the id. Source: the generated `g_*_nvoc.c` export tables (`flags`, `methodId`, `paramSize`) and the `ctrl*.h` defines |
| `ctrlall:0xID` | the same plus every other flag name and the access right (locking, caching, big payload) |
| `allocstruct:NAME` | closure hash of the alloc / `NVOS*_PARAMETERS` struct kayfabe uses |
| `classdef:NAME`, `classhdr:0xID` | the class id define, and the normalised `cl<id>.h` header, for every class named in any chip's class list (kayfabe's family class sets are the union of those lists) |
| `chipclasses:CHIP`, `mcarch:NAME` | per-chip engine class list (`g_gpu_class_list.c`), `NV2080_CTRL_MC_ARCH_INFO_{ARCHITECTURE,IMPLEMENTATION}_*` |
| `rpcfn:UNIT:NAME` | ordinal in `rpc_global_enums.h` (function and event enums) |
| `gsp:FILE:def:NAME`, `gsp:FILE:struct:NAME` | defines and struct closures in the GSP / message-queue / WPR-meta / SR-meta / init-args / libos / FSP / rmgspseq / LSF headers, the `g_rpc-*` generated headers (every rpc payload struct), and the struct typedefs of `kernel_gsp_fwsec.c` / `kernel_gsp_booter*.c` |

Three corrections found while building it, kept because they are traps for the next reader: (a) **RMCTRL flag bits were renumbered in R555** [M] (`NON_PRIVILEGED` 0x10 to 0x8, `ROUTE_TO_PHYSICAL`
0x200 to 0x40, ...), so raw masks across tags are wrong and the flags are decoded by name with each tag's own `control.h`; without that, 318 served controls "changed" at 555.42.02 for no reason. (b) `ROUTE_TO_VGPU_HOST`
(vGPU-host plugin routing) is kept out of the wire view: neither kayfabe nor a GeForce Windows driver is a vGPU host. (c) `rpc_generic_union` lists every rpc payload struct; following it from the message header
would make the header change with every payload, so it is not followed and not an item.

**Which boundaries are "meaningful" (touch something kayfabe serves).** By construction the control areas contain only ids kayfabe's code or the matrix names (`controls_served`) or that a Windows driver was observed issuing
(`controls_windows`). Of 485 control ids named, **388 resolve in at least one tag and 97 never do** (§4). Class areas are kayfabe's generated class tables (every class in any chip list); the GSP "referenced" area keeps items whose C name
(or camel/snake variant) appears in `crates/kf-*` including doc comments, 310 of 953 varying GSP items. Boundaries that only ADD an item are reported separately (they cannot break a driver that does not use the item, but they can
enable a call a newer driver makes).

## 2. The tags

221 tags, none missing; ordered by version number (the order the rule uses), not by release date.

| branch | tags | | branch | tags | | branch | tags |
|---|---|---|---|---|---|---|---|
| 515 | 17 | | 550 | 34 | | 580 | 26 |
| 520 | 3 | | 555 | 5 | | 590 | 2 |
| 525 | 27 | | 560 | 3 | | 595 | 20 |
| 530 | 2 | | 565 | 2 | | 610 | 3 |
| 535 | 39 | | 570 | 25 | | 615 | 2 |
| 545 | 5 | | 575 | 6 | | | |

Full list with item counts: `docs/design/v3_api_boundaries/tags.tsv`. `595.91.07` is the owner host's driver; the Windows 596.51 capture was decoded with the `595.84` tables (its `cell.json` says `decode_tag: 595.84`).

## 3. Intervals per area (all 221 tags) [M]

| area | what | items that ever change | change points | intervals (221 tags) | intervals without add-only boundaries |
|---|---|---|---|---|---|
| controls_served | RM controls kayfabe serves or probes (code + matrix lists) | 201 | 28 | 29 | 27 |
| controls_windows | RM controls a Windows driver issued (kfwc / observer censuses) | 121 | 34 | 35 | 30 |
| controls_all_union | union of the two | 226 | 37 | 38 | 33 |
| controls_all_flags | controls, every RMCTRL flag name (locking, caching) counted | 326 | 49 | 50 | 47 |
| alloc_structs | alloc / NVOS parameter structs | 25 | 13 | 14 | 14 |
| classes | class defines + class headers (all classes in any chip list) | 335 | 40 | 41 | 38 |
| chip_lists_arch | per-chip class lists + MC arch/impl ids | 103 | 21 | 22 | 20 |
| gsp_rpc_enum | RPC function / event enum | 238 | 25 | 26 | 26 |
| gsp_interface_referenced | GSP structs / defines kayfabe names | 310 | 53 | 54 | 54 |
| gsp_interface_all | all GSP / msgq / WPR / FSP / rpc structs and defines | 953 | 62 | 63 | 54 |

Exact change points (which items changed between which tags) are in `changepoints_<area>.tsv`; maximal runs of identical area signature in `intervals_<area>.tsv`. Boundaries inside one R-branch (a branch is not uniform):

| branch | tags | first..last | controls (served) | controls (Windows-issued) | alloc structs | classes | GSP (named by kayfabe) |
|---|---|---|---|---|---|---|---|
| 515 | 17 | 515.43.04..515.105.01 | 0 | 0 | 0 | 0 | 1 |
| 520 | 3 | 520.56.06..520.61.07 | 0 | 0 | 0 | 0 | 0 |
| 525 | 27 | 525.47.04..525.147.05 | 2 | 2 | 0 | 3 | 7 |
| 530 | 2 | 530.30.02..530.41.03 | 1 | 1 | 0 | 0 | 0 |
| 535 | 39 | 535.43.02..535.309.01 | 4 | 5 | 3 | 8 | 10 |
| 545 | 5 | 545.23.06..545.29.06 | 0 | 0 | 0 | 0 | 0 |
| 550 | 34 | 550.40.07..550.163.01 | 1 | 2 | 0 | 5 | 5 |
| 555 | 5 | 555.42.02..555.58.02 | 0 | 0 | 0 | 0 | 0 |
| 560 | 3 | 560.28.03..560.35.03 | 0 | 0 | 0 | 0 | 1 |
| 565 | 2 | 565.57.01..565.77 | 0 | 0 | 0 | 0 | 0 |
| 570 | 25 | 570.86.15..570.211.01 | 0 | 0 | 0 | 0 | 7 |
| 575 | 6 | 575.51.02..575.64.05 | 0 | 0 | 0 | 0 | 0 |
| 580 | 26 | 580.65.06..580.178.04 | 4 | 4 | 0 | 5 | 6 |
| 590 | 2 | 590.44.01..590.48.01 | 0 | 0 | 0 | 0 | 0 |
| 595 | 20 | 595.44.02..595.104.02 | 0 | 3 | 0 | 3 | 0 |
| 610 | 3 | 610.43.02..610.57.04 | 0 | 1 | 0 | 0 | 0 |
| 615 | 2 | 615.71.09..615.78.08 | 0 | 0 | 0 | 0 | 0 |

Reading: the RM *control* surface is nearly uniform inside 545, 555 to 575 (served controls), 590, 595 (host side) and 610 to 615; the big control boundaries sit at the first tag of a branch
(520.56.06, 525.47.04, 530.30.02, 535.43.02, 545.23.06, 550.40.07, 555.42.02, 560.28.03, 565.57.01, 570.86.15, 575.51.02, 580.65.06, 590.44.01, 595.44.02, 610.43.02, 615.71.09). The **GSP surface is not**: it changes
at patch level inside 525, 535, 550, 570 and 580 (and once inside 515 and 560) (`GspSystemInfo`, `MESSAGE_QUEUE_INFO`, `GspFwWprMeta` and the rpc payload structs), because the GSP firmware ships in lock-step with every tag.
This is also where the earlier kayfabe analysis stopped: `V3_SWEEP_AND_INSTALL.md` section 1.4 derived 22 guest-axis and 17 host-axis classes from the *consumed layouts* over the 29 matrix tags; this document extends the
same idea to all 221 tags, to the Windows-issued controls and to the GSP interface.

## 4. The part ogkm cannot see: 97 control ids with no open definition

Of the 485 distinct control ids (421 literals plus 282 names from code, the matrix lists, and the 249 a Windows driver issued), **97 have no definition in any of the 221 tags** [M] (no `#define`, no export-table row): 74 of them are ids a Windows driver
issued (36 only seen on Windows, 38 also named in kayfabe), 23 are named only in kayfabe code or the matrix. They are the GSS-legacy range `0x20808xxx` to `0x2080bxxx`, `0x0073xxxx` display-common rows, and a few NV0080 / NV2081 ids. For
these, a signature can only say "absent"; the earlier census labelled them `unknown` / `absent_all_tags` too. Whether a boundary exists for them must be measured, and §5 shows it does.

## 5. F-4: does the source show the change between 580 and 595?

*Hypothesis F4-S:* the six controls whose requests the 595.91.07 host answers `INVALID_ARGUMENT` (`0x2080852e/852f/853a/853b/a618/b201`) have a different definition in the 595 sources than in the 580 sources.
*Falsifier (written first):* any of the six ids is undefined at both ends, or defined identically at both. **Result: falsified [M].** All six are undefined in every tag, including 580.159.04 and 595.91.07; the literal
`0x2080852e|852f|853a|853b|a618|b201|8162` occurs in no file of the full 595.91.07 checkout (3810 files; `grep -rIil`) nor in the sparse 580.159.04 tree; in every other tag the id is merely undefined (no `#define`, no export row). The source therefore says nothing about these
controls: they are outside ogkm's declared interface. `0x20800a59` (GMMU static info) and the 8162 row behave as the matrix says: `a59` has the same signature from 535.43.02 to 615.78.08; `8162` is also undefined.

What the source does show between 580.159.04 and 595.91.07 [M]: 26 served or Windows-issued controls change signature (`0x0000013a`, `0x00730280/282`, `0x00800106`, `0x00801306` (a flapping item, §6), `0x00801b02`, `0x20800110`, `0x2080012f`,
`0x20800a00`, `0x20800a26/2a/30/87/ff`, `0x20801228`, `0x20801320`, `0x20801344`, `0x20802a0f`, `0x20803002/083`, `0x208081ea`, `0x50700117/118`, `0x5070020d`, `0x50700709`, `0xc3700202`), 4 class defines, 13 class headers, and 99 GSP items
(`GspSystemInfo`, `GspStaticConfigInfo`, `GSP_ARGUMENTS_CACHED`, `MESSAGE_QUEUE_INFO`, `GSP_FMC_BOOT_PARAMS`, `rpc_run_cpu_sequencer_v`, the `GSP_MSG_QUEUE_*` constants). None of those is one of the six. Notably the
group-P ids `0x50700118` and `0x50700709` do change (so the owner's "privileged" rows are boundary-sensitive).

**What shows the F-4 change instead (measured from the committed census tables, not from source):**

| control | name in census | request bytes 580.88 (GA106) | request bytes 596.51 (GA104) | ogkm |
|---|---|---|---|---|
| 0x00730282 | (no name) | 2600 | 2608 | signature changed |
| 0x00801b02 | NV0080_CTRL_CMD_MSENC_GET_CAPS_V2 | 8 | 12 | signature changed |
| 0x2080012f | (no name) | 1464 | 1664 | signature changed |
| 0x20800a26 | NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_FLOORSWEEPING_MASKS | 3008 | 5568 | signature changed |
| 0x20800a2a | NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_INFO | 3712 | 3776 | signature changed |
| 0x20800a30 | NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_PPC_MASKS | 512 | 1024 | signature changed |
| 0x20800a87 | (no name) | 1056 | 1064 | signature changed |
| 0x20802a0f | (no name) | 24 | 28 | signature changed |
| 0x2080852b | (no name) | 2836 | 3324 | no definition in any tag |
| 0x2080852e | (no name) | 528 | 596 | no definition in any tag |
| 0x20808530 | (no name) | 912 | 980 | no definition in any tag |
| 0x20808539 | (no name) | 6824 | 51624 | no definition in any tag |
| 0x2080853a | (no name) | 1456 | 15352 | no definition in any tag |
| 0x2080853b | (no name) | 1832 | 14320 | no definition in any tag |
| 0x20809019 | (no name) | 10800 | 12336 | no definition in any tag |
| 0x2080a0a7 | (no name) | 14988 | 17676 | no definition in any tag |
| 0x2080a0a8 | (no name) | 19724 | 23180 | no definition in any tag |
| 0x2080a0c8 | (no name) | 22924 | 27148 | no definition in any tag |
| 0x2080a612 | (no name) | 11680 | 54352 | no definition in any tag |
| 0x2080a618 | (no name) | 8620 | 8632 | no definition in any tag |
| 0x2080b201 | (no name) | 2188 | 2444 | no definition in any tag |

*Hypothesis F4-W* (from the constants-measure lane, W3): the five controls captured on both sides sent a different request on 596.51 than on 580.88. *Falsifier:* equal sizes. **Result: not falsified [M]** for 852e, 853a, 853b, a618, b201
(each changes size; 852f was not captured on the GA104/GA106 boxes, only on AD103/AD104 at 580.88). The 580.88 sizes are identical on AD104, AD103 and GA106 (528 B for 852e on all three), so die is not what changes them at 580.88 [M]; the 596 shapes were
captured on one die (GA104), so "branch, not die" for 596 is **[I]**, falsified by a 596.51 capture on an AD10x die showing 528 B. The 8 controls whose size changed and that ogkm defines at 595.84 are all flagged by the ogkm signature (7 changed definition, 1 was first defined after 580.65.06; wire view), and the signature also flags 4 whose size did not change,
while the 13 undefined ones are invisible to it: **the signature has full recall where ogkm defines the control and none where it does not**. Not run, proposed: the owner host at 595.91.07 should accept zero requests of the 596-branch
sizes (852e 596 B, 853a 15352 B, 853b 14320 B, a618 8632 B, b201 2444 B); an `INVALID_ARGUMENT` there falsifies "the 580 shape is the only thing wrong".

## 6. The interpolation rule, tested on the data

*Hypothesis R:* for any served item, if sig(A) == sig(C) for tags A < B < C then sig(B) == sig(A). *Falsifier (written before computing):* there exists a tag B strictly between A and C with sig(A) == sig(C) but sig(B) != sig(A) for any served item.
**Result: the falsifier fires [M].** Counts, by area (an item is a "violation" when its sequence of signatures in version order has a value that returns after a different one):

| area | items that change | items with a violation | monotone items | monotone share | re-entry events | violating (item, B tag) pairs |
|---|---|---|---|---|---|---|
| controls_served | 201 | 12 | 189 | 94.0% | 15 | 362 |
| controls_windows | 121 | 10 | 111 | 91.7% | 23 | 319 |
| alloc_structs | 25 | 1 | 24 | 96.0% | 2 | 15 |
| classes | 335 | 14 | 321 | 95.8% | 40 | 342 |
| chip_lists_arch | 103 | 31 | 72 | 69.9% | 123 | 546 |
| gsp_rpc_enum | 238 | 9 | 229 | 96.2% | 21 | 292 |
| gsp_interface_referenced | 310 | 23 | 287 | 92.6% | 55 | 1050 |
| gsp_interface_all | 953 | 108 | 845 | 88.7% | 190 | 6689 |

The non-monotone served controls and alloc structs, with one witness triple each (`rule_violations.tsv` has every item of every area):

| item | A | B (differs) | C (equals A) | runs | re-entries |
|---|---|---|---|---|---|
| ctrl:0x00730109 | 525.47.35 | 525.53 | 525.60.11 | 5 | 2 |
| ctrl:0x00730124 | 525.147.05 | 530.30.02 | 555.42.02 | 3 | 1 |
| ctrl:0x007301a3 | 545.29.06 | 550.40.07 | 555.42.02 | 3 | 1 |
| ctrl:0x00731140 | 520.61.07 | 525.47.04 | 530.30.02 | 5 | 1 |
| ctrl:0x00801801 | 535.288.01 | 535.309.01 | 545.23.06 | 5 | 1 |
| ctrl:0x0080200a | 575.64.05 | 580.65.06 | 610.43.02 | 4 | 1 |
| ctrl:0x20801110 | 580.94.03 | 580.94.06 | 580.95.05 | 4 | 2 |
| ctrl:0x50800103 | 530.41.03 | 535.43.02 | 555.42.02 | 6 | 1 |
| ctrl:0xc3700102 | 570.211.01 | 575.51.02 | 580.65.06 | 4 | 1 |
| ctrl:0xc3700103 | 570.211.01 | 575.51.02 | 580.65.06 | 4 | 1 |
| ctrl:0xc3700104 | 570.211.01 | 575.51.02 | 580.65.06 | 3 | 1 |
| ctrl:0xc56f010b | 535.43.02 | 535.43.08 | 535.54.03 | 7 | 2 |
| ctrl:0x00801306 | 595.44.13 | 595.44.14 | 595.45.04 | 7 | 4 |
| ctrl:0x20803400 | 535.230.02 | 535.247.01 | 545.23.06 | 6 | 4 |
| ctrl:0x20803401 | 535.154.05 | 535.161.07 | 545.23.06 | 8 | 5 |
| ctrl:0x208081ea | 570.211.01 | 575.51.02 | 590.44.01 | 3 | 1 |
| ctrl:0x90e70113 | 535.43.23 | 535.43.24 | 535.54.03 | 4 | 2 |
| allocstruct:NV_CHANNEL_ALLOC_PARAMS | 535.43.02 | 535.43.08 | 535.54.03 | 7 | 2 |

Causes read off the witnesses' run lengths (the runs are [M], the word "lineage" is [I]): (i) **parallel lineages**: 525.47.04..18 and 525.47.22..35 differ while 525.53 equals the first, 535.43.02 / 535.43.08 / 535.54.03 flip back and forth, 580.94.02/03 and 580.95.05 are the odd ones out of 580.94.06 .. 580.105.08
(`0x20801110` is undefined at 580.95.05 only), for the `0xc370` controls 570.211.01 equals 580.65.06 while 575.51.x differs; (ii) **real reversions** across branches (`0x0080200a`: 575.64.05 = 610.43.02, different at 580.65.06); (iii) flapping by patch tag
(`0x00801306` alternates between two shapes at 595.44.14, 595.45.04, 595.91.07, 610.43.02 and 610.57.04: its change points are what would split the Windows partition inside 595 if it were not excluded). Version number is a total order for the rule but not for API ancestry, so the rule holds
for the clear majority of items (88.7% to 96.2% of the items that change; chip class lists only 69.9%, because every backport re-orders them) and fails for a named minority.

**Consequence adopted in the plan.** Items with a violation do not place boundaries (the rule cannot interpolate them) and need a measurement per tag they matter at; they are listed with their witnesses and not hidden.
Excluded: 27 items on the host axis, 48 on the Windows axis (10 of them controls: `0x00730109`, `0x00731140`, `0x00801306`, `0x0080200a`, `0x20801110`, `0x20803400/401`, `0x208081ea`, `0x90e70113`, `0xc3700104`). The rule is then
applied to the proposed measured set (§7) over every item of the axis, including the excluded ones:

| axis | measured tags | items | correct inferences | wrong inferences | wrong items | which |
|---|---|---|---|---|---|---|
| host | 17 | 561 | 61242 | 19 | 5 | classdef:NVENC_SW_SESSION, classdef:NVFBC_SW_SESSION, classhdr:0xa0bc, classhdr:0xa0bd, ctrl:0x20801110 |
| winguest | 16 | 791 | 88874 | 4 | 3 | ctrl:0x00801306, ctrl:0x20803400, ctrl:0x20803401 |

All 23 wrong inferences are on excluded (violating) items; there are none on the monotone items [M]. (Reading the numbers: a "correct inference" is an (item, unmeasured tag B) pair with measured neighbours A < B < C of equal signature
where B's source signature equals theirs; source gives B's signature directly, so this is the rule's self-consistency on the data, not a measurement of the closed Windows driver.)

### 6.1 Mapping Windows driver versions to ogkm tags (assumption, not measured)

ogkm is the Linux R-branch; the Windows driver is the same RM built for another OS. **[I] Assumption:** Windows driver N.xx belongs to the highest ogkm branch whose number is <= N (581.xx and 580.88 to R580, 59x.xx to R590/R595, 596.51 to R595).
The repository already keys the Windows build 580.88 as its Linux twin 580.65.06 (`V3_VGPU_COMPAT_RESEARCH_20261013.md`; the GSP firmware reported on the captures is 580.65.05), and the 596.51 capture is decoded with the 595.84 tables. Inside a branch, patch-level placement
needs the **GSP firmware version string**: the kfwc captures record `SET_GUEST_SYSTEM_INFO` / `GET_GSP_STATIC_INFO`, so the string can be read from a capture and located on the tag axis (a firmware version that is not a tag, such as 580.65.05, sits between its neighbours and is placed by the
twin rule). *How to verify:* compare the capture's rpc payload sizes per function (`functions.tsv`) with the struct sizes of the candidate tag (needs `sizeof` per tag, not computed here: signatures are text hashes); a mismatch falsifies the placement.

### 6.2 Limits of the rule [I unless stated]

1. Signatures cover **declared interfaces**: struct layout, ids, flags, enums. They do not cover values a control returns (the matrix's per-die constants), behaviour or timing, GSP firmware behaviour, or runtime privilege decisions (RM access rights tokens are not in the flag names).
2. **97 ids have no open definition** (§4); 13 of them changed request size between 580.88 and 596.51 [M]. The rule is silent there, so those controls need one measurement per branch (580, 595, ...), not per ogkm interval.
3. Version order is not lineage order (§6, causes).
4. The Windows driver is closed: that its GSP interface equals the ogkm twin's is the §6.1 assumption.
5. The census is one die per driver at 596.51 (GA104) and three at 580.88; `852f` and the `0x0073` display rows were not captured on the 596.51 box.

## 7. Measurement plan

Window: kayfabe's supported range, ogkm 535.309.01 .. 615.78.08 (134 tags; `V3_DRIVER_MATRIX.md`; 615.71.09 is out of the asked host range by ruling and is kept as the last row). Groups are the intervals of the *monotone* items of the axis' areas
(host: served controls, alloc structs, classes; Windows: Windows-issued controls, alloc structs, classes, GSP structs kayfabe names), with boundaries that are ADD-only or class-header-only merged into the previous group ("material" groups).
Tier A: a control, alloc struct, rpc function, or a GSP boot / message-queue struct is changed, removed or re-shaped at the group start. Tier B: only class ids or non-boot GSP items (group 0 is simply the first tag of the window: no boundary lies inside it). Measured points are in `measured_points.tsv` (host: 575.51.03 AD103; 575.57.08, 580.65.06, 580.95.05, 580.159.04 GA102; 580.159.04 GA106; 595.91.07 AD104 and AD106;
Windows: 580.88 on AD104, AD103 and GA106, placed at 580.65.06 by the twin rule; 596.51 on GA104, placed at 595.84 by its decode tag; both Windows placements are [I]).

**Host driver (Linux host, 15 groups; 4 covered, 11 need a version)**

| # | tier | tags | n | covered by (measured) | needs: representative tag |
|---|---|---|---|---|---|
| 0 | B | 535.309.01 | 1 | needs | 535.309.01 |
| 1 | A | 545.23.06..545.29.06 | 5 | needs | 545.23.08 |
| 2 | A | 550.40.07 | 1 | needs | 550.40.07 |
| 3 | A | 550.40.53..550.163.01 | 33 | needs | 550.90.07 |
| 4 | A | 555.42.02..555.58.02 | 5 | needs | 555.42.02 |
| 5 | A | 560.28.03..560.35.03 | 3 | needs | 560.28.03 |
| 6 | A | 565.57.01..565.77 | 2 | needs | 565.57.01 |
| 7 | A | 570.86.15..570.211.01 | 25 | needs | 570.148.08 |
| 8 | A | 575.51.02..575.64.05 | 6 | 575.51.03@AD103(H1); 575.57.08@GA102(H2) | - |
| 9 | A | 580.65.06..580.82.09 | 4 | 580.65.06@GA102(H3) | - |
| 10 | A | 580.94.02..580.178.04 | 22 | 580.159.04@GA102(H5); 580.159.04@GA106(H6); 580.95.05@GA102(H4) | - |
| 11 | A | 590.44.01..590.48.01 | 2 | needs | 590.48.01 |
| 12 | A | 595.44.02..595.104.02 | 20 | 595.91.07@AD104(H7); 595.91.07@AD106(H8) | - |
| 13 | A | 610.43.02..610.57.04 | 3 | needs | 610.57.04 |
| 14 | A | 615.71.09..615.78.08 | 2 | needs | 615.71.09 |

**Windows guest driver (16 groups; 2 covered, 14 need a version)**

| # | tier | tags | n | covered by (measured) | needs: representative tag |
|---|---|---|---|---|---|
| 0 | B | 535.309.01 | 1 | needs | 535.309.01 |
| 1 | A | 545.23.06..545.29.06 | 5 | needs | 545.23.08 |
| 2 | A | 550.40.07 | 1 | needs | 550.40.07 |
| 3 | A | 550.40.53..550.163.01 | 33 | needs | 550.90.07 |
| 4 | A | 555.42.02..555.58.02 | 5 | needs | 555.42.02 |
| 5 | A | 560.28.03..560.35.03 | 3 | needs | 560.28.03 |
| 6 | A | 565.57.01..565.77 | 2 | needs | 565.57.01 |
| 7 | A | 570.86.15..570.86.16 | 2 | needs | 570.86.15 |
| 8 | B | 570.123.01..570.211.01 | 23 | needs | 570.148.08 |
| 9 | A | 575.51.02..575.64.05 | 6 | needs | 575.64.05 |
| 10 | A | 580.65.06..580.82.09 | 4 | 580.88@AD103(W2); 580.88@AD104(W1); 580.88@GA106(W3) | - |
| 11 | A | 580.94.02..580.178.04 | 22 | needs | 580.178.04 |
| 12 | A | 590.44.01..590.48.01 | 2 | needs | 590.48.01 |
| 13 | A | 595.44.02..595.104.02 | 20 | 596.51@GA104(W4) | - |
| 14 | A | 610.43.02..610.57.04 | 3 | needs | 610.57.04 |
| 15 | A | 615.71.09..615.78.08 | 2 | needs | 615.71.09 |

Representative tag = the latest tag of the group that already has a layout row in `tools/drivermatrix/tags.txt`, else the latest tag. Public `.run` availability is decided by the sweep's availability table (`V3_SWEEP_AND_INSTALL.md` section 1.4: 545.23.08 and 580.94.02 have no public `.run`; take another tag of the same group). For Windows the entry is the branch plus the placement
rule of §6.1; the concrete build must be one whose GSP firmware string lies inside the group.

**Minimal driver lists**

- Host driver, to add: 535.309.01, 545.23.08 (or another 545.x), 550.40.07, 550.90.07, 555.42.02, 560.28.03, 565.57.01, 570.148.08, 590.48.01, 610.57.04, 615.71.09. That is **11 versions** on top of the 4 groups already measured.
- Windows guest driver, to add: R535 (535.309.01 twin), R545, 550.40.07 and 550.90.07 twins, R555, R560, R565, 570.86.15 and 570.148.08 twins, R575 (575.64.05 twin), 580.178.04 twin (the 580.94 to 580.178 half; the measured 580.88 is the 580.65.06 half), R590 (590.48.01), 610.57.04 twin, 615.71.09 twin. That is **14 versions**; the measured 596.51 covers 595.44.02 .. 595.104.02 only through its decode-tag placement (inferred).
- Against the raw sweep: 134 in-range tags per die and OS; the groups reduce it about 9-fold for the host axis and 8-fold for Windows. Per-die constants (the `consttab` rows) are **not** collapsed by this rule: it bounds how many branches a die must see, not whether a die's own values equal another die's.

**Per die family.** A family only needs the groups its chips can run. First tag whose class lists carry the family's chips [M]: Turing TU102/TU116 and Ampere GA100..GA107 515.43.04; Ada AD102..AD107 and Hopper GH100 520.56.06; Blackwell datacenter GB100 560.28.03; **consumer Blackwell GB202/GB203/GB205/GB206/GB207
570.86.15**. So Blackwell consumer applies to host groups 7..14 (8 of 15) and Windows groups 7..15 (9 of 16); the other families apply to all. Cells = 3 families x 15 + 8 = 53 for the host axis and 3 x 16 + 9 = 57 for Windows (Turing, Ampere, Ada, Blackwell consumer; Hopper has no Windows
driver in scope). Family-specific items (chip class lists, MC arch ids) change at 12 to 19 of the tags per family (`summary.json`, `family_chip_lists`), which are class-set changes the generated tables already follow by exact tag; they do not add driver versions.

**Order of work (risk first).** (1) the five Windows controls whose size changed at 596 (`F4-W`): cheapest is the §5 proposal on the owner host; (2) Windows 590 / 595-branch boot, because 596.51 is covered only by placement; (3) the GSP boot-path structs that vary inside a branch
(`GspSystemInfo`, `MESSAGE_QUEUE_INFO`, `GspFwWprMeta`: excluded from the partition because they are non-monotone, so they need the build's own GSP string checked, not an interval); (4) the remaining branch representatives in the table.

## 8. Falsifiers and what was run

| claim | falsified by | result |
|---|---|---|
| F4-S: the six controls differ between the 580 and 595 sources | undefined at both ends, or identical | **falsified [M]**: undefined at all 221 tags; ids absent from the full 595.91.07 tree |
| F4-W: the six controls changed request shape at 596 | equal sizes in the 580.88 and 596.51 censuses | not falsified for 5 of 6 [M]; sixth not captured |
| R: equal signature at A and C implies equal at B | one B with a different signature | **falsified [M]** for 12/10/1/14/23/9 items (served controls / Windows controls / alloc / classes / GSP named / rpc enum), holds for the rest |
| the plan misinfers monotone items | a wrong inference on a monotone item | none [M] (23 wrong, all on excluded items) |
| signatures see every wire change of a defined control | a defined control whose Windows request size changed and whose signature did not | none among 8 [M] (one pair of captures) |
| Windows N.xx maps to the highest ogkm branch <= N | a capture whose GSP string or payload sizes belong to another branch | **not tested [I]**; method in §6.1 |
| "branch, not die" for the 596 shapes | a 596.51 capture on an AD10x die with 528 B for 852e | not run [I] |
| the flag decode is right | a control whose decoded names disagree with the header text | spot-checked on `0x00000102` and `0x20800a59` at 515, 535, 550, 555, 580 [M] |

## 9. Reproduce, files, caveats

`tools/api_boundaries/run_all.sh <kayfabe-tree> <work> <out>` (needs git, python3, network once; the sweep over 221 tags takes about 80 s on the host, the clone is 60 MB). The served set in this document was derived from `claude/constants-night-20261014` @ `93ff3aa0f`
(it carries `kf-ctl-probe` and `consttab`; the branch this document sits on predates them). Derived tables (all small, 0.5 MB total): `docs/design/v3_api_boundaries/` (`summary.json`, `tags.tsv`, `item_runs.json.gz`, `changepoints_*`, `intervals_*`, `plan_*`, `rule_violations.tsv`,
`size_crosscheck.tsv`, `f4.json`). Host working files (not in git): `/data/claude-ogkm-hist` (clone), `/data/claude-ogkm-hist-out` (per-tag JSON).

Caveats: signatures are text hashes (a reordered but equivalent definition counts as a change; a `sizeof` was not computed, though the hash of the closure includes every member); `classhdr` changes are real header edits but may be
documentation-only in effect (they are merged away in the "material" groups when they are the only change); the served set comes from names and 8-hex literals in code, so a control referenced only through a computed id is missed; the Windows-issued set is the union of six census files (AD104 observer, AD103 kfwc, GA106 C/E, GA104 C/E), not all Windows traffic.
