# V3 DRIVER MATRIX — both driver axes, measured per ogkm tag

> ### ⏸ WHERE I STOPPED — 2026-09-27 ~02:00 UTC (owner: weekly usage limit; stopped mid-walk, boxes destroyed)
> ★ **2026-09-27 ~19:40 UTC — merged** (a later note; the stop note below is kept as written):
> everything on `v3-drivers` but its tip is **on master**. `8f9bdd14` (this note's own commit) went in
> through merge candidate v3-mc20 (`5018bb57` = master `0d3ecde9` + `v3-drivers~1`), which passed the
> merge bar at `c0ef7b75` — `kf-*` tests 1639 / 0, gates 9/9, `KF3_RC=0`, thin 30/30 (vast 53004208,
> RTX 3060 GA106, host 580.159.04 open; `traces/v3_mc20/`); master and v3 are `db038f5f`. The 535/545
> capability commit, now **`a50265f8`** (the same change as `ee35ca4a`, which was an earlier
> (pre-rebase) SHA of it; §6's rows still cite `ee35ca4a` as the revision they were measured at), **remains held**
> for owner review and is the only commit on the branch that is not on master. Step 5 below is
> therefore only partly done: the bar ran at the merged head without the allowlist and without the CUDA
> ladder (`merge_check.sh` does not run it), and the 570 / 565 ladders were not re-run. Steps 1–4 stand.
>
> **Branch `v3-drivers`**, the 535/545 capability commit is LAST (owner review); everything below it
> is mergeable (coordinator merging `42b25354`+ onto master — ⊘ *done, through `8f9bdd14`: see the ★
> note above*). All evidence is committed under
> `traces/driver_matrix/walk/` (both boxes' queue logs, suites, ladders, failure points, host
> installer logs); §6.0's grid is derived from it by `scripts/drivermatrix/matrix_table.py`.
>
> **Last measured (not yet folded into §6 rows):**
> - `bb9b67a9` (kfd, host 580.159.04): guests **595.84 and 610.57.04 ladder 4/4 with ZERO
>   "heartbeat timed out" lines** — the GSP heartbeat (`46d9bf37`) verified on hardware; tests
>   1618/0; the thin suite at that head was interrupted (not measured).
> - Host **570.148.08** (kfh, `1f3e4fc4`, guest 580.159.04): gates 9/9, thin **30/30**, ladder **4/4**,
>   mixed pairs 590.48.01 **4/4** and 575.57.08 **4/4** (its `.run` is on the `tesla/` path only).
> - 535.309.01 / 545.23.08 past the chip-info carry: next wall `INTERNAL_GET_CONSTRUCTED_FALCON_INFO`
>   (`0x20800a42`) — the ≤545 id of what 550+ asks as `GPU_GET_CONSTRUCTED_FALCON_INFO`
>   (`0x208001b0`, served as `WantedTable::ConstructedFalconInfo`): the same answer under another id.
>
> **Next steps, in order:**
> 1. **570 UVM first-channel wall** (on master after `v3-mc19` = the init-race fix): nvidia-uvm 570's
>    first channel (`UVM_OWNED`, GPFIFO VA `0x121010000`) is read before the mirror places it
>    (`traces/v3_initrace/570/`). Measured difference: the UVM internal VA space's root is stated
>    `0x1efa74000` on 570 and every walk under it finds **0 rows**; on 575 it is `0x4000` and the walks
>    find its rows — and no fn 54 arrives on 570 before the channel dies. Run the 570 fat ladder with
>    `KF_VAS_CENSUS=1` (`b87d9882` now names each root's carrier) beside a 575 run and compare which
>    statement roots `0xc1d0000a:0xcaf00005` on each (queued as `q11`, interrupted before it ran).
> 2. Host **565.57.01 / 550.54.14**: root-caused from the kept installer logs — the old `.run` builds
>    with `cc` (gcc-11) while Linux 6.8 was built by gcc-12 (`cc: error: unrecognized command-line
>    option '-ftrivial-auto-var-init=zero'`, `Failed CC version check`). Run the installer with
>    `CC=x86_64-linux-gnu-gcc-12` in `provision_host_driver.sh`; the 550 fat-guest `.run` (image kernel
>    6.8.0-139) most likely fails the same way (`stage_fat_guest.sh` now prints its errors).
> 3. The remaining host walk: 535.309.01 (the ISA 8.2 JIT floor), 590.48.01, 595.84, 610.57.04 hosts
>    (`hostwalk3`, interrupted after 570).
> 4. 535/545: serve `0x20800a42` from the `ConstructedFalconInfo` answer (the INTERNAL/GPU struct
>    pair measured identical in the matrix before relying on it) — behind the owner's review of the
>    capability rows.
> 5. The final bar at the merged head (tests, gates 9/9, default thin 30/30, ladder 4/4), then the
>    570 / 565 ladders again.

**STATUS: LIVE (in progress), 2026-09-27, branch `v3-drivers`** (the 535/545 capability commit is
always the LAST one — owner review; everything below it is mergeable — ⊘ *and since 2026-09-27 on
master via v3-mc20 `5018bb57`; the held commit is `a50265f8`: the ★ note at the top*). Both axes are built and
walked on hardware; §6.0 is the running grid, DERIVED from `traces/driver_matrix/walk/`.
**Guest axis (host 580.159.04):** every 580.x thin 27–30/30 (the reds are the adapter-init flake);
CUDA ladder **4/4 for 580.159.04, 580.105.08, 590.48.01, 595.84, 575.57.08 and 610.57.04**;
570.148.08 / 565.57.01 initialise but every adapter RE-init fails at CeUtils (the flake's mechanism,
deterministic — handed to `v3-initrace`); 550.54.14 initialises (fat image unstaged: its `.run`
does not build on the image's kernel); 535.309.01 / 545.23.08 (the latter on a staged 6.5 guest
kernel) reach `_gpuInitChipInfo`, carried since. **Host axis (guest 580.159.04):** 575.57.08,
580.95.05, 580.65.06 — gates 9/9, thin 30/30, ladder 4/4, mixed pairs (590 / 575 guests) 4/4; the
first sub-580 host needed the per-map-kind fix (§2.2 H3). 570/565/550/535/590/595/610 hosts in the
third walk. Boxes: vast `52746206` (RTX 3090 GA102) and `52788835` (RTX 3080 Ti GA102).

> Owner roadmap item (2026-09-26): kayfabe v3 must support the NVIDIA driver range nvkvm-pv
> supports — 535 → 610 — on BOTH axes, with per-version values **derived from source into
> generated tables keyed by version ranges, never hand rows**, and the guest must see exactly
> what its version expects. Measure cheapest first: host fixed at 580.159.04, walk the guest
> (580.x points → 570/575 → 550/565 → 535 → 590/595 → 610); then fix the guest and walk the
> host; then mixed pairs. Stop and report a version that needs an owner decision, with the size
> of its gap.

## 0. One screen

| | guest axis (`Dg`) | host axis (`Dh`) |
|---|---|---|
| who chooses it | the operator, inside the VM | the operator, on the host |
| what varies | the GSP firmware interface our fake GSP must speak: queue element, init args, RPC numbers and payloads, static info, every RM control the guest's CPU-RM forwards | the RM ioctl ABI kf-host authors: NVOS escape bodies, control params, alloc params, plus the PTX ISA the host's libcuda JITs |
| selected by | `guest-driver=` (defaults to the host's), **cross-checked** against the guest's own `NV_VERSION_STRING` at fn 1 | the host RM's own `NV_ESC_CHECK_VERSION_STR` |
| state 2026-09-27 | table assembled from **measured** per-tag layouts; CUDA ladder 4/4 for 580.159.04 / 580.105.08 / 590.48.01 / 595.84 / 575.57.08 / 610.57.04 guests; ≤575 guests needed the fn 54/79 page-directory carrier (G11), 610 the `(runlist, chid)` token index and the large RPC; 570/565 blocked by adapter re-init (`v3-initrace`); 535/545 reach chip-info (carried) — capability rows under owner review | R2 gates on **measurement**; every host struct carried by name (`kf_abi::hostabi`, 53 controls pinned to the matrix); subtree map authored per family below 580.65.06 (ruling 3); walker PTX at ISA 8.2 (ruling 4); per-map PTE kind only from 580.65.06 (H3 correction); hosts 575.57.08 / 580.95.05 / 580.65.06 green on thin + ladder + mixed pairs |

★ **The finding that shapes everything:** NVIDIA moves ABI **inside** a branch. Measured:
`GspSystemInfo` gains a field at 580.95.05 and another at 580.105.08, `NV0080_CTRL_MSENC_GET_CAPS_V2_PARAMS`
grows 8 → 12 bytes at 580.95.05, `g_rpc-structures.h` changes at 580.126.09, 580.159.04 and
580.173.02, the vGPU handshake pair differs between 570.124.06 and 570.148.08. ⇒ A key of
"major" (or of "newest hand row ≤ version") is wrong by construction; the table is keyed on the
**exact measured tag**.

## 1. The two-axis model

`THE_ARCHITECTURE_v3.md` §0 names the axes; this is how the code now carries them.

- **Guest (`Dg`)** — the stock guest driver talks to the fake GSP (`kf-gsp`) and the emulated RM
  (`kf-rm`). Every fact it expects — queue element and init-args shape, RPC function/event
  numbers, `rpc_*_v` payload layouts, `GspStaticConfigInfo`, the VGX handshake pair, the params
  of every control it forwards, the alloc params of every class it allocates through us — is a
  function of **its** version. Carried by `kf_abi::versions::DriverAbiTable`, assembled per
  measured tag (§4).
- **Host (`Dh`)** — kf-host authors every host call from unprivileged RM verbs; the layouts of
  those verbs are a function of the **host** version. ⊘ Until 2026-09-26 one pinned interval
  (`[580.65.06, 581)`); now **measured**: `kf_abi::hostabi::HostAbi` resolves the host's tag,
  kf-host keeps writing the bench layout and every struct is carried to the host's own layout by
  field name at the ioctl boundary (§2.2) — the same transcoder the guest axis uses.
- **Independence.** The two versions are read from two different places and never assumed equal:
  the guest's version is declared (property) and verified against what the guest says; the host's
  is read from the host RM. `author_host_flags_never_forward_them` keeps guest flag words off host
  calls, so no guest-version fact leaks onto the host wire.
- **The band** (`support_matrix_asymmetry`, owner 2026-08-09): exact match, same-branch minors,
  and n−1 must work; more mismatch is an extra. Out-of-band pairs must be refused by name — the
  fn-1 cross-check (§4.2) is the first mechanism that can say *which* pair it is looking at.

## 2. Inventory — where 580.159.04 was hard-coded

Two read-only sweeps (2026-09-26), every cross-version claim compiled against real tags. Condensed;
the full tables (file:line for every item) are in this doc's history note §2.9.

### 2.1 Guest axis (`kf-gsp`, `kf-rm`, `kf-chip`, `kf-abi`, `kf-qemu`, `kf3/`)

| # | item | where | was | measured truth |
|---|---|---|---|---|
| G1 | the version table's floor | `kf-abi/src/versions.rs` `TABLES` | hand rows from 550.54.04 (not an ogkm tag) | every tag measurable; 535/545 refused at realize |
| G2 | VGX handshake pair | `versions.rs` rows | `Some` only on 580.65.06 and 610 rows | per tag, moves inside 570 (§0) — every 550–575/590/595 guest refused at fn 1 |
| G3 | `rpc_gsp_rm_control_v` header | `kf-abi/src/view.rs:198` `HEADER = 40` | fixed 40 | **24 bytes through 570.x**, 40 from 575.51.02 |
| G4 | `GspStaticConfigInfo` | `kf-abi/src/gspstaticinfo.rs:65-248`, `kf-rm/src/staticinfo.rs:226` | 1792 bytes, offsets pinned to cap1b | 6 layouts 535→580 (2168/2168/2184/1640/1656/1792), 1808 at 590, 1592 at 595, 1600 at 610 |
| G5 | channel-alloc tail offsets | `versions.rs` rows + `kf-abi/src/notifier.rs` | `None` on every 550–575 row | V580 offsets exact at every tag 535.309.01…595.84 |
| G6 | served RM controls' params | `kf-rm/src/inittables.rs` exact-size gate + `kf-abi/src/*.rs` encoders | 580 sizes | e.g. `INTR_GET_KERNEL_TABLE` 2068 ≤575 (2112 at 580); `KGR_GET_INFO` 3392/3584/3712/3776/4032; `GET_GLOBAL_SM_ORDER` 23072 ≤570 … 73760 at 610 (above the 64 KiB element max) |
| G7 | version plumbing | `kf-qemu/src/device.rs:198-201` | lenient parse (bad patch → 0), guest defaults to host, no cross-check | strict parse; cross-check at fn 1 |
| G8 | RM engine numbering | `kf-abi/src/submit.rs:1106-1140`, `kf-rm/src/authored.rs` | 580 values | `RM_ENGINE_TYPE_SW` 0x2c at 535/545, 0x2d from 550; MC indices lower at 535/545 |
| G9 | `rpc_rc_triggered_v17_02` | `kf-abi/src/generated/rpc.rs:710-744` | 48 bytes | five layouts 535–565 (20 bytes, `exceptType`@8 at 535) |
| G10 | MSENC caps params | `kf-abi/src/videocaps.rs` | 12 bytes | 8 bytes through 580.65.06 (580.82.07 host side) |
| G11 | the page-directory statements' CARRIER | `kf-gsp/src/rpc.rs` `FunctionCodes`, `kf-rm/src/barpde.rs` | only `GSP_RM_CONTROL` `0x00801813`/`0x00801814`; fn 54/79 refused as unknown functions | through **575.64.05** `deviceCtrlCmdDma(Un)SetPageDirectory_IMPL` sends the dedicated RPCs **fn 54 / fn 79** (`rpc_set_page_directory_v` 48 B, `rpc_unset_page_directory_v` 16 B, embedding the control structs field for field at all 29 tags); from 580.65.06 the control. The two tags' `NV_RM_RPC_*` sets differ in exactly this pair (+ Tegra's `DCE_RM_INIT`). **Found by the 575 fat ladder** (§6): UVM's `nvUvmInterfaceSetPageDirectory` rides it inside `cuInit`. Fixed: both carriers share one statement function (§6 triage) |

Stable 535 → 610, verified: the 48-byte GSP element (until 610's 16-byte MCTP one), `msgq` headers,
`rpc_message_header_v`, every RPC function/event number kayfabe uses (only
`INIT_GSP_TRACE_CRASH_BUFFER` = 228 is new at 580.159.04), `LibosMemoryRegionInitArgument`,
`rpc_gsp_rm_alloc_v`, `rpc_free_v`, `rpc_dup_object_v`, `UpdateBarPde_v15_00`,
`rpc_set_guest_system_info_v`, `rpc_post_event_v`, `NV_CHANNEL_ALLOC_PARAMS` prefix,
`NV0080_ALLOC_PARAMETERS`, TSG/ctxshare params, `SET_PAGE_DIRECTORY`, `PROMOTE_CTX`.

### 2.2 Host axis (`kf-host`, `kf-linux-raw`, `kf-cuda`, `kf-chan`, `kf-mem`, `kf-qemu` host facts)

| # | item | where | host size by version (kf value first) |
|---|---|---|---|
| H1 | version gate | `kf-abi/src/host_driver.rs`, `kf-host/src/lib.rs:158-161` | refuses outside [580.65.06, 581); **refused two-field versions (595.84) as Unparsable** — fixed |
| H2 | `NV0080 GET_CLASSLIST_V2` | `kf-host/src/lib.rs:499-507` | 804; 644 at 535/545, 700 at 550, 404 at 555–575 |
| H3 | `NVOS46` map-memory-DMA | `kf-host/src/lib.rs:831-919` (every GPU map) | 64; **56 below 580.65.06** |
| H4 | `NVOS47` unmap | `lib.rs:922-976` | 48; 40 at 535/545 (no range unmap before 550.40.07) |
| H5 | `GPFIFO_SCHEDULE` | `kf-host/src/channel.rs:741-751` | 3; **2 bytes ≤ 570.x** |
| H6 | walk kernel PTX | `cuda/walk/kf_walk.ptx` (was `.version 8.8`, NVRTC 12.9) | JIT failed on every host driver < 575 (CUDA 12.9) — "no walker, no device". **Regenerated with NVRTC 12.2 → ISA 8.2** (ruling 4): same 14 entries, same `.param` lists, `.target sm_75`; `make_ptx.py` now refuses an ISA above 8.2 |
| H7 | `NV_CHANNEL_ALLOC_PARAMS` | `kf-abi/src/submit.rs:407-541` | 368; 376 at 610 with `hHandleVASpace`@32 — **silent** (RM copies its own sizeof) |
| H8 | HostFacts controls | `kf-rm/src/hostquery.rs` | `GPU_GET_ENGINES_V2` 340 (252/256/260 ≤555), `GPU_GET_INFO_V2` 564 (516/524/532 ≤575, 580 at 610), `GR_GET_INFO_V2` 488 (448/472 ≤565, 496 at 590/595, 528 at 610), `FB_GET_INFO_V2` 1028 (436/444/460 ≤575), `GR_GET_GLOBAL_SM_ORDER` 9240 (6168 ≤570, 7192 at 575) |
| H9 | controls absent on old hosts | `hostquery.rs:402-432` | `MC_GET_INTR_CATEGORY_SUBTREE_MAP` absent < 580.65.06; `MC_GET_STATIC_INTR_TABLE` absent at 535/545 — **needs another source**, not a size fix |
| H10 | `NV2080_NOTIFIERS_CE10` | `kf-host/src/event.rs:59` | was **184** (= `GSP_PERF_TRACE`); 166 at every tag that has it — **fixed** (a bug on every host) |

★★ **BUILT 2026-09-26 — the host axis, measured (`kf_abi::hostabi`, `kf-host`).** A read-only
inventory of every host-side struct (12 escape wrappers, 18 allocations, 53 controls, every literal
size) drove it; 31 of those structs were not in the matrix and are now consumed (+ `host.spec` for
the four no spec reached). The rule is one encoder, carried at the boundary:

- **R2 gates on measurement**, not on a pinned interval: the host's tag must be measured, and every
  wrapper kf-host sends without a carry (`NVOS00/02/21/33/34/54`, the fd wrappers, `REGISTER_FD`,
  `CARD_INFO`, `ALLOC_OS_EVENT`, the version query — identical at all 29 tags) must have the
  bench's layout there (H1).
- **Controls** — `raw_control` looks the command up in `HOST_CONTROLS` (53 rows; a test pins every
  id to the SDK's measured value at every tag and requires the struct wherever the id exists) and
  carries the payload out and the reply back (H2, H5, H8). GSS-legacy ids (no header) pass only on
  the old interval; an unlisted control is refused by name outside it.
- **Allocations** — every `raw_alloc` names its params struct (H7: the 610 channel params, the 535
  memory params, the ≤575 VA-space params…); MSENC/BSP params and caps are carried under their 610
  names (NVENC/NVDEC — the old names survive only as `#define` aliases DWARF cannot see).
- **NVOS46/47** carried (H3, H4); a field the host lacks that the request sets — a kind override
  on a 575 host, a range unmap on a 545 host — is `HOST_ABI_REFUSED`, printed by name.
  ⊘⊘ **CORRECTED 2026-09-26 (host 575.57.08, measured): refusing the kind override was right as a
  statement and fatal as a policy** — every guest vidmem leaf carries a kind (GENERIC_MEMORY), so
  the carry refused 609 maps in one suite and no guest got past CeUtils (thin **0/30**, all
  timeouts). A host below 580.65.06 has no per-map kind at all (no `kindOverride`, bit 19
  undefined; the PTE takes the memory object's own kind), so kf-mem now asks the host ABI
  (`HostAbi::per_map_pte_kind`, measured by the field) and maps PITCH/GENERIC there with NO
  override — the pre-v3-gfx mapping every compute rung ran under — while a depth/stencil kind is
  kept and still refused by name (mapping a Z surface as the memory's kind is host Xid 13).
- **H9** — the subtree map is authored per family from ogkm where the host's driver measurably
  lacks the control (ruling 3; `kf_chip::authored_intr_subtree_map` = the GA106 host's own answer);
  `MC_GET_STATIC_INTR_TABLE` exists at every measured tag (the earlier "absent at 535/545" was a
  misreading).
- ⊘ **Found on the way:** `MSENC_GET_CAPS_V2` is 8 bytes with `instanceId` at +4 on 580.65.06 and
  580.82.07 — hosts the old gate ACCEPTED — while kf-host sent 12 bytes with it at +8. Carried now.

At the bench host every carry is the identity; not yet run on a non-580 host (§8.3).

★ **Cross-checked against nvkvm-pv (coordinator, 2026-09-26: nvkvm-pv is the source of truth for
these user↔kernel ioctl structs).** nvkvm-pv measured its nine profile fields plus the five
`UVM_REGISTER_GPU` fields at all 216 published tags 515→610 with `sizeof`/`offsetof` probes
(`nvkvm-pv/tests/abi_parity/ogkm_abi_sweep_20260826.tsv`); this matrix measured the same structs
from gcc's DWARF. `tools/drivermatrix/crosscheck_nvkvm_pv.py` compares every cell both measured:
**167 tags × 14 fields = 2 338 cells, 2 338 agree, 0 disagree**
(`traces/driver_matrix/nvkvm_pv_crosscheck.tsv`). Two independent instruments agree on every
host-axis cell they share, including both in-branch boundaries (535.54.03 → 535.86.05 channel,
550.40.07 → 550.40.53 UVM). nvkvm-pv's profile boundaries therefore apply to H3/H4/H7 as stated:
`NVKVM_ABI_525` (channel 304) … `NVKVM_ABI_610` (channel 376).

The raw client (`kayfabe-rm-ladder`, the thin guest's grader, running against the **guest**
driver) has its own copy of H1/H2/H3/H4/H5/H7 and the UVM layouts (`UVM_MAP_EXTERNAL_ALLOCATION`
1200 before 550.40.53; `UVM_FREE`/`UVM_UNREGISTER_CHANNEL` shrink at 590.44.01) — so the thin
guest cannot grade a guest outside [580.65.06, 581) until the grader is version-aware (§7.1).

## 3. The derivation pipeline — `tools/drivermatrix/`

★ **The compiler is the parser.** Owner, 2026-09-21: *"use proper parsers/compilers"*;
`tools/derive_classes.sh` is the precedent.

| step | tool | what it does |
|---|---|---|
| fetch | `dm.py fetch` | blobless sparse clone of one ogkm tag's header trees (~110 MB, ~7 s) |
| measure | `dm.py probe` | per spec entry, ONE translation unit compiled with the tag's own `src/nvidia/Makefile` `-I`/`-D` flags: struct layouts read out of the DWARF `gcc -g` emits (every member, recursively), enumerators from DWARF, `#define` names from the preprocessor's macro table (`gcc -E -dM`, minus an empty TU's) and values from a compiled `printf` gated by `_Generic` to integers, batch-bisected so one bad name costs one row |
| sweep | `dm.py sweep` | every tag in `tags.txt`, parallel, resumable (`.done` per tag) |
| collapse | `collapse.py ranges / report / boundaries / gaps` | runs of consecutive identical tags per item; the per-struct boundary report; the per-version work list against the bench tag |
| emit | `emit_rust.py` | `crates/kf-abi/src/generated/matrix.rs` — data only |
| all | `regen.sh [tag…]` | the above, end to end; outputs committed (the diff is the review) |

Specs: `gsp.spec` (guest only — queues, boot args, static/system info, RPC numbers and every
`rpc_*_v`, engine numbering), `sdk.spec` (both axes — every control id and params struct in the
SDK `ctrl/` tree, class ids, alloc params, `NVOS*`), `os.spec` (host only — escapes, `nv-ioctl.h`,
UVM). `consumed.txt` is the machine-readable inventory: what kayfabe reads or writes; only those
items reach `ranges.tsv` and the Rust.

Evidence discipline (from `nvkvm-pv/tools/abi_derive.sh`): every cell is measured or MISSING with
its compiler error kept; nothing is interpolated between tags; nothing is typed. Measured on the
box: one tag ≈ 2 min for ~1 500 struct layouts and ~2 600 values.

### 3.1 What moved, and where (committed: `traces/driver_matrix/boundaries.txt`, `report.md`)

| transition | structs changed | values changed |
|---|---:|---:|
| 535.309.01 → 545.23.08 | 86 | 62 |
| 545.23.08 → 550.54.14 | 239 | 132 |
| 550.54.14 → 565.57.01 | 270 | 281 |
| 565.57.01 → 570.124.06 | 159 | 163 |
| 570.124.06 → 570.148.08 | 3 | 8 |
| 570.148.08 → 575.51.03 | 68 | 61 |
| 575.51.03 → 575.57.08 | 1 | 0 |
| 575.57.08 → 580.65.06 | 119 | 78 |
| 580.65.06 → 580.95.05 | 11 | 5 |
| 580.95.05 → 580.105.08 | 3 | 1 |
| 580.105.08 → 580.126.09 | 2 | 3 |
| 580.126.09 → 580.159.04 | 3 | 4 |
| 580.159.04 → 580.173.02 | 1 | 2 |
| 580.173.02 → 580.178.04 | 0 | 0 |
| 580.178.04 → 590.48.01 | 130 | 115 |
| 590.48.01 → 595.84 | 65 | 67 |
| 595.84 → 610.43.02 | 108 | 71 |
| 610.43.02 → 610.57.04 | 1 | 0 |
| 610.57.04 → 615.71.09 | 79 | 56 |

(all measured structs/values, not only consumed ones — the consumed subset is §7's table)

## 4. The generated-table design in the code

`crates/kf-abi/src/matrix.rs` (lookup rules) + `crates/kf-abi/src/generated/matrix.rs` (data).

- **`MEASURED`** — every tag the committed sweep compiled. **Exact membership, no nearest
  neighbour**: a version not in it is `AbiError::Unmeasured`, whose message is the one command that
  fixes it (`tools/drivermatrix/regen.sh <tag>`). ⊘ "Newest measured ≤ version" is exactly the
  borrow the intra-branch moves make wrong.
- **`StructRuns` / `ValueRuns`** — per consumed item, the runs of tags over which it is identical;
  a struct's value is a deduplicated `Layout` holding only consumed fields (+`sizeof`). Absent is
  an answer (`None`), unmeasured is an error — different findings, different types.
- **`Resolved::need` / `maybe`** — a consumer asks for a field by path; a missing required field
  is `LayoutError::Missing{struct, path, version}`: the unit of the per-version gap report.
- **`DriverAbiTable` is assembled per measured tag** (`versions::table_for` → `derive_table`),
  cached once per process. Each field is READ or DERIVED from what was read:

| field | rule |
|---|---|
| `map_dma` | `NVOS46_PARAMETERS` `(sizeof, status)` = (56, 48) or (64, 56); else `NoEncoding` |
| `gsp_element` | `GSP_MSG_QUEUE_ELEMENT` checked field by field against the 48-byte or the 16-byte MCTP shape; a third shape (615.71.09's encryption union) is `NoEncoding` by name |
| `gsp_init_args` | the four shared fields at 0/8/16/24 checked; `queueElementHdrSize` present → nine-field |
| `gsp_static_info` | the hand encoder is claimed only where the measured layout **equals 580.159.04's**; 610 → the 610 variant; anything else → `Unencoded` (fn 65 refused by name) |
| `vgx` | the measured `VGX_{MAJOR,MINOR}_VERSION_NUMBER` |
| `channel_*` | the measured offsets of `internalFlags`, `errorNotifierMem`, `hUserdMemory`, `userdOffset`, `userdMem`, `engineType` |
| `rm_control` | the measured `rpc_gsp_rm_control_v` (`params`, the flags word, `rmctrlFlags`/`AccessRight` where present, `paramsSize`, `status`) |
| `caps` | ⊘ **policy, not layout**: nvproxy's reviewed allowlist rows, "newest row ≤ version"; below the oldest row (535/545) `NoCapabilityRow` by name |

### 4.1 Version strings

`DriverVersion::parse` — two or three all-digit fields, nothing else (`595.84` is a real release;
`580.65.06-x` is not a version). Replaces kf-qemu's lenient parse (a bad patch silently became 0,
selecting another release's row) and the host gate's three-field-only parse (which refused 595.84
as unreadable).

### 4.2 The fn-1 cross-check

The guest's version is **declared** (`guest-driver=`, defaulting to the host's). `SET_GUEST_SYSTEM_INFO`
carries the guest's own `NV_VERSION_STRING`; `kf-rm`'s policy compares it with the table's version
and refuses by name on a mismatch (`kf-rm: SET_GUEST_SYSTEM_INFO refused: the guest driver says it
is "580.105.08" but this device's layouts were selected for 580.159.04 — set the device property
guest-driver=580.105.08`). ★ The VGX pair alone could not catch that: every 580.x speaks 0x2B/0x13.
⚠ Limitation: fn 1 is not the first message — `GSP_SET_SYSTEM_INFO` and `SET_REGISTRY` (both
ignored today) and the queue geometry come first, so a declared version on the wrong side of the
610 element break fails before fn 1.

★ **RULED and BUILT (2026-09-26): re-selection at fn 1.** `kf_rm::ReselectAtFn1` wraps the served
chain; kf3 builds the chain through a recipe (`device.rs`) so it can be rebuilt for another table.
When the device's version was **defaulted** (no `guest-driver=`), fn 1 carries the guest's own
version, and the pair's pre-fn-1 surface is identical (`kf_abi::versions::pre_fn1_surface_differs`:
element and init-args shapes, element size maximum, VBIOS path, the numbers of fns 1/64/72/73 and
`GSP_INIT_DONE`, and the fn-1 / message-header layouts — all read from the matrix), the chain is
rebuilt for the guest's version before fn 1 is answered. A **declared** version is never
overridden; a pair whose surface differs (e.g. 580.x → 595.84: nine-field init args; → 610: MCTP
elements) or an unmeasured reported version is refused by name. Measured pairs, from the matrix:
every 580.x ↔ 580.159.04, 575.57.08 and 570.148.08 share the surface; 595.84 and 610.x do not.

⚠ 535–550 publish a SIX-field `MESSAGE_QUEUE_INIT_ARGUMENTS` (two lockless-queue offsets at +32/+40).
The four fields kf-gsp reads sit at the same offsets, and the Linux client writes the two extra
ones as 0 — `bIsTaskIsrQueueRequired` defaults to `NV_FALSE` (`ogkm-550.54.14:
src/nvidia/generated/g_kernel_gsp_nvoc.c:271`, consumed at `kernel_gsp.c:1979, 3394-3403`), i.e.
no second (ISR) RPC queue pair exists — so the table's four-field reading is right there. A guest
that ever wrote non-zero lockless offsets would need that queue pair served (not built).

⊘ **Detecting the version from the guest's firmware is not structurally available.** `[measured
2026-09-26, gsp_ga10x.bin of 580.105.08]` the version string lives in the firmware container's
`.fwversion` section (11 bytes, `580.105.08\0`), which the guest's CPU-RM checks and never hands to
the GSP; the `.fwimage` that reaches guest memory is not an ELF (magic `0x0006c297`) and carries the
string only inside GSP-RM's own rodata (two hits ~16 MB in, at no fixed offset). Finding it would be
a pattern scan of a 74 MB guest-supplied image — sniffing, not reading. ⇒ Declared + checked stays
the design; the cheaper follow-on is to **re-select at fn 1** when the guest's reported version shares
the provisional one's pre-fn-1 surface (element, init args), which the matrix can state per pair.

### 4.3 The RM-control header

`decode_rpc_control` slices `params[]` at the measured offset (24 through 570.x, 40 from 575), and the
sticky-answer neutraliser zeroes `rmctrlFlags`/`rmctrlAccessRight` only where the version has
them — before 575 those bytes were `params[0..8]` and were being zeroed in every accepted reply.

## 5. The hardware walk — mechanics

- **Guest driver, thin guest:** `scripts/drivermatrix/stage_guest_driver.sh <v>` builds that tag's
  open modules for the host's running kernel and takes that version's GSP firmware from its `.run`
  (CUDA-repo deb fallback: 545.23.08 and 575.51.03 have no public `.run`).
  `build_fast_guest.sh` with `KF_FROM_HOST=1 KF_GUEST_DRIVER_DIR=…` puts them in the initrd
  (the kernel and non-NVIDIA deps stay the host's); `run_fast_guest.sh` takes the build via
  `KF_FASTGUEST_DIR`; the device gets `KF3_DEV_EXTRA=guest-driver=<v>`.
- **Guest driver, fat guest:** `scripts/drivermatrix/stage_fat_guest.sh <v>` — a qcow2 overlay on
  the bench image with `<v>`'s `.run` installed (open modules), verified on content (`modinfo`,
  libcuda); `boot_nvkvm.sh` takes it via `KF_GUEST_IMG`.
- **Guest kernel (545 only):** `scripts/drivermatrix/stage_guest_kernel.sh 6.5.0-45-generic`
  EXTRACTS (never installs) a jammy HWE kernel — image, depmod'ed modules, build headers — for a
  guest driver that does not build on the host's 6.8 (545.x: `libspdm_shash.c`,
  `crypto_tfm_ctx_aligned`); `stage_guest_driver.sh` builds against it (`KREL=`, `KBUILD=`) and
  `build_fast_guest.sh` boots it (`KF_GUEST_KROOT=`).
- **Host driver:** `scripts/bench/provision_host_driver.sh` with `RUN_URL=` for the version; the
  guest stays at 580.159.04 (the bench image, and a thin guest staged from 580.159.04 with
  `guest-driver=580.159.04` declared so a defaulted device cannot take the host's version).
- **Non-580 guests are graded by the fat ladder** (ruling 1): the thin guest's raw client is a
  580-only RM client and refuses them at R2, so their thin row is `failure_point.sh` — did the
  guest's RM initialise, and if not, where it stopped — and the fat-guest CUDA ladder is the verdict.

## 6. The matrix (host × guest), measured

### 6.0 At a glance (latest result per cell; the rows below carry every measurement and revision)

Guest axis, host **580.159.04** (GA102). *thin* = the 30-arm suite (580.x guests only — the grader
is a 580 RM client, ruling 1); *init* = the guest's RM initialised (`failure_point.sh`); *ladder* =
the fat-guest CUDA ladder (cup2 / cup3 / cup8 / cup8bench).

| guest | result | rev |
|---|---|---|
| 580.159.04 | thin **30/30**, ladder **4/4** | `47348e3b`, `6de22590`, `13b25624` (rebased on master `59cc98a9`) |
| 580.105.08 | thin **30/30**, ladder **4/4** | `47348e3b` |
| 580.65.06 | thin 28/30 (2 × adapter-init flake) | `47348e3b` |
| 580.95.05 | thin 29/30 (`rpc-mixed-allocs`: a SYSMEM object read `0xffffffff`, dead mapping — rerun queued) | `47348e3b` |
| 580.126.09 | thin 29/30 (1 × adapter-init flake, the `memmgrInitCeUtils` `NV_ERR_INVALID_STATE` variant) | `47348e3b` |
| 580.173.02 | thin **30/30** | `47348e3b` |
| 580.178.04 | thin 27/30 — all three reds (`concurrency`, `late-map-race`, `executor-vas`) are the adapter-init flake (`RmInitAdapter failed` in each; 6 in 213 boots of this revision's walk) | `47348e3b` |
| **590.48.01** | init ✔, **ladder 4/4** | `6de22590` |
| **595.84** | init ✔, **ladder 4/4** | `6de22590` |
| **575.57.08** | init ✔, **ladder 4/4** at `ee35ca4a` (cup2 `0xabcd1234`, cup3 `43`, cup8 `bad=0 maxerr=0`, cup8bench verified) — the fn 54 carrier (G11). Before it: ladder **0/4** at `6de22590`, `cuInit` → `CUDA_ERROR_NOT_INITIALIZED`. ⊘ **CORRECTED (same day): not the GSS-legacy `0x2080a637`** this row first blamed — that 96 KB control is asked during the boot-time adapter init, before the module is reloaded cold, and a bare-metal 575.57.08 host's `cup2` never asks it. ★ **The wall was the page-directory carrier (G11):** inside `cuInit` UVM registers the GPU, brings up its six channels, then calls `nvUvmInterfaceSetPageDirectory` — which a ≤575.64.05 RM sends as the dedicated RPC **fn 54**, refused as an unknown function (`GSP rpc UNSERVICED { code: 54 }`; `traces/driver_matrix/walk/kfh/ladder_boots.tar.xz`); UVM tears the registration down | `ee35ca4a` |
| 575.51.03 | INTR wall at `47348e3b`; re-run queued | — |
| 570.148.08 | init ✔ (the FIRST `RmInitAdapter` of every boot); **ladder 0/4** at `ee35ca4a`, `cuInit` → 999: every adapter **RE-init** fails at `memmgrInitCeUtils` (`memmgrMemCopy` TIMEOUT, `0x25:0x65:1128`). The reborn CeUtils channel (same token, same host channel, same GPFIFO VA, a new VA space) retires `forwarded=1 submissions=0`: the ring reader fetched the guest's entry and produced NO words, while the completion tail still retired it (host CE2 non-stall 1→2→3, `GP_GET=1`) — the guest's copy and its `finishPayload` release never ran. Deterministic here because 570's CeUtils self-test OPENS with the sysmem copy (575+ first memsets vidmem). The adapter-init flake's mechanism, reproducible — handed to `v3-initrace` (coordinator, 2026-09-26); re-run when its fix lands | `ee35ca4a` |
| 570.124.06 | INTR wall at `47348e3b`; re-run queued | — |
| 565.57.01 | init ✔ (BIF refused, carried since); **ladder 0/4** at `ee35ca4a` — the same adapter RE-init wall as 570 (first init passes; the reborn CeUtils channel retires `forwarded=1 submissions=0`; `v3-initrace`) | `ee35ca4a` |
| 550.54.14 | device-info wall at `6de22590` → carried; failure point re-run queued. Fat guest **unstaged**: the 550.54.14 `.run` does not install on the fat image's kernel (6.8.0-139; the thin guest's 6.8.0-59 builds it) — `stage_fat_guest.sh` now prints the installer's errors | — |
| **610.57.04** | init ✔, **ladder 4/4** at `1837166d` (cup2 `0xabcd1234`, cup3 `43`, cup8 `bad=0 maxerr=0`, cup8bench verified). Two 610-only facts made it: the **large RPC on hardware** — `GET_GLOBAL_SM_ORDER` (73 800 B) joined from 2 fragments and answered in 2 replies (ruling 2) — and **per-runlist chids on Ampere** (`bUsePerRunlistChram`, GA10x from 610.43.02): at `ee35ca4a` the user channel's chid 1 collided with CeUtils' on the chid-indexed token table (`OverDeclaredCap` → `0x1a`, `cuInit` → 3); the device now indexes by `(runlist, chid)` on every family (`84967e3a`). The GSP heartbeat a 610 guest reads is published since `46d9bf37` (hardware check queued) | `1837166d` |
| 545.23.08 | boots the staged **6.5.0-45** guest kernel (`stage_guest_kernel.sh`; the 6.8 build gap closed) with the capability row (owner review); RmInitAdapter stops at `_gpuInitChipInfo`: `INTERNAL_GPU_GET_CHIP_INFO` is 92 bytes there (`bar1Size` at +12) — carried by name since `ee35ca4a`+1 (see 535) | `ee35ca4a` |
| 535.309.01 | with the capability row (owner review — the LAST commit on the branch): RmInitAdapter stops at `_gpuInitChipInfo` (`0x23:0x56:907`) — `INTERNAL_GPU_GET_CHIP_INFO` is 92 bytes at ≤545 (`bar1Size` at +12; it has no reader in the 535/545 RM). Carried by name at the next commit (a unit test pins the carry at both versions); re-run queued | `ee35ca4a` |

Host axis, guest **580.159.04** (box 2, RTX 3080 Ti):

| host | result | rev |
|---|---|---|
| **575.57.08** | at `ee35ca4a`: gates **9/9**, thin **30/30**, ladder **4/4** (guest 580.159.04); mixed pairs: guest **590.48.01 ladder 4/4**, guest **575.57.08 ladder 4/4**. Refused by name, as designed: `MC_GET_INTR_CATEGORY_SUBTREE_MAP` (absent at 575 — the Ampere family rule answers, ruling 3) and the four GSS-legacy clock rows (no public header, unmeasured below 580 — §7). Before the per-map-kind fix (`9339ee6b`): gates 9/9 but thin **0/30**, ladder 0/4 — every kinded map refused (§2.2 H3 correction) | `ee35ca4a` (`9339ee6b`) |
| **580.95.05** | at `ee35ca4a`: gates **9/9**, thin **30/30**, ladder **4/4**; mixed: guest 590.48.01 **4/4**, guest 575.57.08 **4/4** | `ee35ca4a` |
| **580.65.06** | at `ee35ca4a`: gates **9/9**, thin **30/30**, ladder **4/4**; mixed: guest 590.48.01 **4/4**, guest 575.57.08 **4/4** | `ee35ca4a` |
| 570.148.08 | not swapped: its `.run` is 404 under `XFree86/` (it is on the datacenter path `tesla/`) — the third walk tries both | — |
| 565.57.01 / 550.54.14 | not swapped: the `.run` failed "Building kernel modules" on the host's Linux 6.8.0-59, and the next swap overwrote the one log that said why — `provision_host_driver.sh` now keeps each version's installer log and prints its errors; retried in the third walk | — |

**The host × guest grid** — DERIVED, not typed: `scripts/drivermatrix/matrix_table.py` reads every
queue log committed under `traces/driver_matrix/walk/` and keeps each cell's latest measurement
(thin = the 30-arm suite, 580.x guests only; ladder = the fat-guest CUDA ladder; revision in
backticks). Regenerate after every refresh of the walk evidence.

| guest \ host | 570.148.08 | 575.57.08 | 580.65.06 | 580.95.05 | 580.159.04 |
|---|---|---|---|---|---|
| *gates* | 9/9 | 9/9 | 9/9 | 9/9 | 9/9 |
| 550.54.14 |  |  |  |  | ladder unstaged |
| 565.57.01 |  |  |  |  | ladder 0/4 `ee35ca4a` |
| 570.148.08 |  |  |  |  | ladder 0/4 `ee35ca4a` |
| 575.57.08 | ladder 4/4 `1f3e4fc4` | ladder 4/4 `ee35ca4a` | ladder 4/4 `ee35ca4a` | ladder 4/4 `ee35ca4a` | ladder 4/4 `1837166d` |
| 580.65.06 |  |  |  |  | thin 28/30 `47348e3b` |
| 580.95.05 |  |  |  |  | thin 29/30 `47348e3b` |
| 580.105.08 |  |  |  |  | thin 30/30, ladder 4/4 `47348e3b` |
| 580.126.09 |  |  |  |  | thin 29/30 `47348e3b` |
| 580.159.04 | thin 30/30, ladder 4/4 `1f3e4fc4` | thin 30/30, ladder 4/4 `ee35ca4a` | thin 30/30, ladder 4/4 `ee35ca4a` | thin 30/30, ladder 4/4 `ee35ca4a` | thin 30/30, ladder 4/4 `1837166d` |
| 580.173.02 |  |  |  |  | thin 30/30 `47348e3b` |
| 580.178.04 |  |  |  |  | thin 27/30 `47348e3b` |
| 590.48.01 | ladder 4/4 `1f3e4fc4` | ladder 4/4 `ee35ca4a` | ladder 4/4 `ee35ca4a` | ladder 4/4 `ee35ca4a` | ladder 4/4 `1837166d` |
| 595.84 |  |  |  |  | ladder 4/4 `bb9b67a9` |
| 610.57.04 |  |  |  |  | ladder 4/4 `bb9b67a9` |

★ Every row carries its source revision. Box: vast `52746206`, RTX 3090 (GA102 `0x2204`), Xeon
E5-2673 v4 (nested KVM), host driver **580.159.04 open**. Thin suite = `KF_DEVICE=kf3
fast_suite.sh <tag> 180` (30 arms); gates = `scripts/bench/v3_gates.sh`; ladder = the fat-guest
`cuda_ladder.sh guest` (cup2 CE round trip, cup3 `=43`, cup8 2048² matmul `bad=0 maxerr=0`,
cup8bench, every timed iteration verified).

> ★★★★★ **ROOT-CAUSED 2026-09-26 (`v3-initrace`) — the adapter-init flake below (`RmInitAdapter
> 0x25:0x65` after `memmgrMemSet … NV_ERR_TIMEOUT`, CeUtils token `0x2` `forwarded=1 GP_GET=1`) is
> NOT a completion-plane race and NOT version-specific.** A Translated channel was born over an
> UNINITIALISED guest USERD: kayfabe is the physical RM for the guest's kernel channels, physical RM
> zeroes an FB USERD at allocation (the guest's CPU-RM clears only a SYSMEM one,
> `kernel_channel.c:2344-2356`), and kayfabe did not. A stale `GP_PUT = 1` left in the slot is read
> by the `GPFIFO_SCHEDULE` pump as queued work: the guest's still-zero entry 0 is fetched as a NOP
> (`submissions=0`), `GP_GET = 1` is authored, and the guest's REAL entry 0 then arrives at
> `GP_PUT = 1` = the cursor and is never fetched. Every re-open finds the previous channel's stale
> cursors (299/300 opens measured); only a stale `1` fails outright. Reproduced 20/20 with
> `KF3_INJECT_STALE_USERD=1`; fixed by `kf_chan::host::zero_userd` at birth — 0/300 natural and
> 0/300 injected after, at `7f271349` (vast `52792102`, the same RTX 3090 machine as `52746206`).
> Mechanism and evidence: `THE_TRANSLATED_PLANE.md` §7 item 3. ⊘ The row's *"the PMA scrubber's CE
> token never forwarded"* reading is superseded by this. ⚠ The second variant reported later
> (`memmgrInitCeUtils` `NV_ERR_INVALID_STATE`, both CeUtils submissions retired: the self-test's
> data check failed) is **not explained by it and stays open**: a clean slot (`forwarded=2`) rules
> the stale cursor out, and it did not recur on `52792102` in ~1 600 self-tests (every open runs
> one) nor in the targeted 580.65.06 arms. ★ Measured on the way: CeUtils' self-test reaches FB
> **virtually**, through its FB alias (`memmgrMemUtilsMapFbAlias`, `LAUNCH_DMA 0x218e` — SRC
> VIRTUAL, DST PHYSICAL sysmem), so the engine's source is whatever OUR host-space rows map at the
> alias VA. `KF3_COMPLETION_PROBE=<ms>` now prints, per completed CeUtils fence, each operand's
> bytes as the host holds them (a virtual side resolved through our rows) and through every guest
> CPU window (BAR1/BAR2/PRAMIN) showing that store page — the first occurrence under the probe will
> say whether the guest's write and the engine's read met the same memory.
> ★★ **The 570.148.08 guest's deterministic re-init wall (§6.0) IS this mechanism**
> `[measured, vast 52792102, v3-drivers 607f290f ± the fix]`: without it, every re-init's CeUtils
> is born over the first life's `(GP_PUT, GP_GET) = (1, 1)` and retires `forwarded=1 submissions=0`
> (three incarnations per boot, all failed); with it the re-init passes `memmgrInitCeUtils`
> (`forwarded=1 submissions=1`, both releases landed). ⊘ **The 570 fat ladder is still 0/4, at a
> different, later wall:** nvidia-uvm 570's first channel (`KERNEL+UVM_OWNED`, GPFIFO VA
> `0x121010000`, 1024 entries) is read before the mirror has placed that VA — `DEAD: ring: Read …
> not placed by us` → `REFUSED-AND-POISONED`, and `cup2` hangs in `uvm_channel_manager_create`
> (`uvm_push_end_and_wait`). Refused by name, never retired — a version-gap item for this matrix.
> ⊘ Also measured: on master `59cc98a9` (and the thin guest on either binary) a 570.148.08 guest
> oopses in its FIRST init (`memmgrMemCopyWithTransferType`, NULL dereference) — the v3-drivers
> head does not.

| host | guest | rev | gates | tests | thin guest | fat guest ladder | notes |
|---|---|---|---|---|---|---|---|
| 580.159.04 | 580.159.04 | `283a5304` (master) | 9/9 | — | **30/30** | — | baseline on this box |
| 580.159.04 | 580.105.08 | `283a5304` (master) | — | — | 29/30 → **30/30** | — | the one red was the harness (a half-written initrd from a killed build booted `Failed to execute /init`; fixed: `guest_walk.sh` builds atomically); the arm re-run PASS |
| 580.159.04 | 580.159.04 | `67e7eadb` | 9/9 | 801 (+1 stale test, fixed in `0d3ce125`) | **30/30** | **4/4** | cup2 `0xabcd1234`, cup3 `43`, cup8 `bad=0 maxerr=0`, cup8bench verified |
| 580.159.04 | 580.105.08 | `67e7eadb` | — | — | **30/30** | 0/4 → *staging fault* | the overlay was staged without the seed ISO; fixed in `f72f9a58` |
| 580.159.04 | 580.159.04 | `f72f9a58` (rebased on master `e05ff74d`) | **9/9** | **1512 / 0** | *not run* (harness: the default initrd was built without the musl client ⇒ NOTRUN=30; re-run at `47348e3b`) | **4/4** | cup2 `0xabcd1234`, cup3 `43`, cup8 `bad=0 maxerr=0`, cup8bench verified |
| 580.159.04 | 580.105.08 | `f72f9a58` | — | — | **29/30** | **4/4** | the fat guest re-staged with the seed ISO (`f72f9a58`); ladder identical to the default guest's. The one thin red is an **adapter-init flake**, see below |
| 580.159.04 | 580.159.04 | `47348e3b` (rebased on master `02b27c2a`; walker PTX ISA 8.2) | **9/9** | **1533 / 0** | **30/30** | **4/4** | the early-merge candidate — green on the whole bar |
| 580.159.04 | 580.105.08 | `47348e3b` | — | — | **30/30** | **4/4** | + **fn-1 re-selection on hardware 3/3**: the 580.105.08 initrd on a DEFAULTED device logs `RE-SELECTED at fn 1: 580.159.04 (defaulted) -> 580.105.08` and passes `--timer`, `--engines`, `--ce-client` |
| 580.159.04 | 580.159.04 | `6de22590` (rebased on master `f8c68286`; host axis carried) | **9/9** (RTX 3080 Ti, box 2) | **1591 / 0** | **30/30** | — | at the bench host every host carry is the identity |
| 580.159.04 | 580.159.04 | `13b25624` (the branch REBASED on master `59cc98a9`, 535 commit last; mergeable head `7538aabf`; walker PTX regenerated by `make_ptx.py` + NVRTC 12.2 — byte-identical) | **9/9** | **1612 / 0** | **30/30** | **4/4** | green on the whole bar after the rebase. ⚠ This box's GPU had wedged just before (GFW boot `progress 0xff` on the first adapter init after a QEMU exit, 19:37); an FLR (`rmmod`, PCI `reset`, `modprobe`) recovered it — the queues now health-check and FLR before every step |
| 580.159.04 | 580.65.06 | `47348e3b` | — | — | 28/30 | — | both reds are the adapter-init flake (§6 triage note) — one a NEW variant: `memmgrInitCeUtils` `NV_ERR_INVALID_STATE` with BOTH CeUtils submissions retired (the self-test's data check failed) |

★ **At `6de22590` (element sizes in the matrix) the 575.57.08, 570.148.08 and 565.57.01 guests'
RM INITIALISES** (`failure_point.sh`: no `RmInitAdapter` failure; the grader refuses them by
design). 565 was still refused `INTERNAL_BIF_GET_STATIC_INFO` (16 bytes at ≤565; its status is
discarded by the guest) — carried at `d687eb53`. 550.54.14 stopped at the device-info table
(carried at `b6574262`) and BIF. Their fat ladders are the verdict (§8.2 ruling 1).

★ **How far each other guest version got at `47348e3b`** (`failure_point.sh`: one `--timer` boot,
host 580.159.04; the 580-only grader refuses every non-580 guest by design, so "client rc=1" with
no guest error means the guest's RM initialised):

| guest | where it stopped | cause | status |
|---|---|---|---|
| 590.48.01 | **nowhere in the guest** — RmInitAdapter completes; the grader refuses | — | fat ladder queued (ruling 1) |
| 595.84 | **nowhere in the guest** — RmInitAdapter completes; the grader refuses | — | fat ladder queued |
| 575.57.08 / 575.51.03 | `intrInitInterruptTable` | `INTR_GET_KERNEL_TABLE` carry: the matrix had no element sizes, so `subtreeMap` read as one scalar that narrowed | fixed at `1b4aedaf` (element sizes) |
| 570.148.08 / 570.124.06 | `intrInitInterruptTable` | same | same |
| 565.57.01 | `intrInitInterruptTable` | same | same |
| 550.54.14 | `gpuConstructDeviceInfoTable` | `INTERNAL_GET_DEVICE_INFO_TABLE` (9 220 bytes there) unported | carry added `b6574262` |
| 610.57.04 | `RmInitAdapter 0x23:0x56` | `USER_REGISTER_ACCESS_MAP` (20 492 bytes) unported; next: SM order 73 760 > one message | carry `b6574262`; large RPCs `5b577f5d` (ruling 2) |
| 535.309.01 | realize | no capability row below 550.54.04 | ruling 5: owner review |

★ **Past RmInitAdapter, the next wall of every ≤575 guest is `cuInit` — and it is one fact, G11.**
`[measured 2026-09-26, 575.57.08 fat guest at 6de22590, box 2]` the QEMU log shows UVM's six
channels born and scheduled, then `GSP rpc Other(54)` → `UNSERVICED { code: 54 }` → `0x56`, then
UVM's frees: `nvUvmInterfaceSetPageDirectory` arrived as the dedicated `SET_PAGE_DIRECTORY` RPC,
because through 575.64.05 `deviceCtrlCmdDmaSetPageDirectory_IMPL` issues `NV_RM_RPC_SET_PAGE_DIRECTORY`
(`575.57.08: mem_mgr/dma.c:441`, `vgpu/rpc.c:9238`) where 580.65.06+ issues `NV_RM_RPC_CONTROL`
(`580.65.06: dma.c:443-453`). Every ≤575 tag in the range (535 … 575.64.05) has it; nothing else in
the two tags' `NV_RM_RPC_*` sets differs. ⇒ The fix is a carrier, not a new behaviour: fn 54's
48-byte wrapper and fn 79's 16-byte one are consumed from the matrix (`rpc_(un)set_page_directory_v`,
identical at all 29 tags and embedding the control structs field for field —
`kf-abi/tests/driver_matrix.rs`), and `barpde::PageDirPolicy` answers them with the SAME statement
function, capability gate, echo and reply hold as `0x00801813`/`0x00801814`
(`kf-rm/src/barpde.rs` test: fn 54 at 575.57.08 states exactly what the control states at
580.159.04). The ids are read from the matrix at compile time (`ValueRuns::everywhere_u32`): a sweep
in which they move is a build failure.

★ **The 580.105.08 red at `f72f9a58`, measured.** Arm 1 (`--timer`): guest `RmInitAdapter failed!
(0x25:0x65:1236)` after `memmgrMemSet … NV_ERR_TIMEOUT` (`mem_mgr.c:463`) and
`pCeUtils->lastCompletedPayload == lastSubmittedPayload` (`ce_utils.c:349`). The QEMU log names the
mechanism: CeUtils channel token `0x2`'s FIRST submission was forwarded and executed on the host
(`forwarded=1`, `GP_GET=1`, one host `CE2` non-stall observed), the guest never observed its
completion, so its second submission never came (a passing arm shows `forwarded=2`). Count on this
box: **1 adapter-init failure in 149 thin boots** — 0/60 at 580.159.04, 1/89 at 580.105.08, which
this sample cannot separate. Every consumed layout and the whole pre-fn-1 surface are identical
between the two 580.x versions, so nothing version-specific is on that path: it is recorded as a
**channel/completion-plane race**, handed off, and every later boot of the walk is counted for it
(`q3`: `INITFLAKE boots=… adapter_failures=…`).

**Triage note (for the hunt; coordinator 2026-09-26: a real completion-plane race even if rare).**
Per occurrence, `scripts/drivermatrix/initflake_evidence.sh --all <run prefix>` prints the guest's
assertion chain, every translated token's `RETIRED` line and the device counters before the first
retire. The first occurrence (`rf72f9a58_g58010508_timer`):

| | failing boot | passing boots (same version, same revision) |
|---|---|---|
| token `0x2` (CeUtils `memmgrInitCeUtils`) | `forwarded=1 serves=3 last_put=1 GP_GET=1` | `forwarded=2 serves=5 last_put=2 GP_GET=2` |
| host non-stall events observed / raised to the guest | `CE2:1/0raised` | `CE2:2/0raised` |
| guest IRQs `raised` | 0 | 0–18 (CeUtils polls memory; no interrupt is expected for it) |
| guest | `memmgrMemSet` TIMEOUT, then `lastCompletedPayload != lastSubmittedPayload` | — |

So the host fetched the entry and the copy engine raised its non-stall at the release; the
guest's poll of the completion payload never saw the value. ⊘ What no current log line can say,
and is the first thing to capture: **where the release was written** (kayfabe does not parse
pushbuffers, so the semaphore GPU VA is not logged), **what the host backing held at that address
after the release**, and **which backing the guest's CPU view of that page pointed at while it
polled** (`views[armed/held]` is only a count). A boot that re-reads the completion page through
both the host store and the guest's view at the timeout would split "the release landed elsewhere"
from "the view was stale".

## 7. Gaps per version (consumed items that differ from 580.159.04)

`collapse.py gaps --ref 580.159.04 --only consumed.txt` (both axes' consumed structs):

| guest/host version | consumed structs ≠ 580.159.04 | values ≠ | what they are |
|---|---:|---:|---|
| 580.65.06 | 3 | 2 | `GspSystemInfo` (ignored by kf-rm), MSENC caps (now measured), `rpc_init_gsp_trace_crash_buffer_v` (absent; the guest never sends fn 228) |
| 580.95.05 – 580.126.09 | 1–2 | 2 | as above |
| 580.173.02 / 580.178.04 | 0 | 2 | — |
| 575.x | 15 | 29 | static info (1656), `GET_CLASSLIST_V2`, FB/GPU info V2, GR FS info, INTR table, KGR floorsweeping / SM order / PPC masks, `NVOS46` (56), VASPACE params |
| 570.x | 23 | ~50 | 575's + the 24-byte RM-control header, `GPFIFO_SCHEDULE` (2), … |
| 565.57.01 | 27 | 84 | + static info 1640 |
| 550.54.14 | 32 | 110 | + `GET_DEVICE_INFO_TABLE`, static info 2184, rc_triggered layout |
| 545.23.08 | 39 | 133 | + engine numbering, no capability row |
| 535.309.01 | 42 | 141 | + `NVOS47` (40), no capability row |
| 590.48.01 | 6 | 28 | static info 1808, KGR_GET_INFO 3776, MSENC caps, VGX 0x2C/0x07 |
| 595.84 | 11 | 46 | static info 1592, nine-field init args, KGR info/floorsweeping, GPU name 68, `rpc_run_cpu_sequencer_v` |
| 610.x | 18 | 72 | 16-byte MCTP element (encoded), static info 1600, `USER_REGISTER_ACCESS_MAP` 20492, SM order 73760 (> 64 KiB element max), GPU info 580, … |

**Guest axis — a GSP behaviour the newest guests expect and we do not provide (measured
2026-09-26):** from **595.84** the guest RM reads two GSP heartbeats after every RPC poll —
`NV_PGSP_MAILBOX(0)` (GSP-RM) and `(1)` (LibOS), GPU time in ms (`595.84` / `610.57.04:
kernel_gsp.c` `_kgspHeartbeatIsGspRmHeartbeatTimedOut`, supported on GA102 and later per
`kgspIsHeartbeatSupported`). Ours stay 0, so a 595/610 guest logs *"GSP RM heartbeat timed out"* /
*"LibOS heartbeat timed out"* after every RPC. Not fatal (the 595.84 ladder is 4/4 with it), but an
RPC that DOES time out is then classified as a hung GSP (`_kgspIsTimeoutFatal`). Up to 590 the
registers are read only by `kgspDumpMailbox` (a failure dump), so publishing a heartbeat is
version-independent. ★ **Built:** the drainer stores the host GPU's time in ms (`HostRm::gpu_time_ns`
— the usermode page's `TIME_1:_0`, the counter the guest itself reads through the aliased page)
into both words every 0.5 s, on GA102-and-later families (`kf_chip::Family::gsp_heartbeat_mailboxes`,
offsets pinned to each die group's `NV_PGSP_MAILBOX(i)`). Hardware check pending (a 595/610
guest's dmesg must lose the *"heartbeat timed out"* lines).

**Host axis — what a sub-580 host cannot be asked, measured on host 575.57.08 (all refused by
name, none silent):**

| gap | hosts | effect in the guest | path to close |
|---|---|---|---|
| per-map PTE kind (`NVOS46` `kindOverride`) | < 580.65.06 | none for compute: PITCH/GENERIC map with the memory's own kind (the pre-v3-gfx mapping); a depth/stencil kind is refused by name (graphics on such a host: Z surfaces unmapped) | a per-kind virtual range (`PAGE_KIND_VIRTUAL`) — only if graphics on an old host is wanted |
| `MC_GET_INTR_CATEGORY_SUBTREE_MAP` | < 580.65.06 | none — the family rule answers (ruling 3) | — |
| GSS-legacy clock rows `0x20809064` / `0x2080a028` / `0x2080a084` / `0x2080a026` | outside [580.65.06, 581) | `cudaDevAttrClockRate` falls back to `cudartinit`'s constant; NVENC's clock query refused ("unsupported device") | probe each host version's answers (the rows are asked with requests WE author) and widen the interval per measured version |
| host 545.x | — | not measurable on these boxes: the 545 kernel module does not build on the hosts' Linux 6.8 (same conftest gap as the guest, §5) | a ≤ 6.6 host kernel |

### 7.1 Index-keyed lists: append-only, with one exception (measured)

The info-list controls (`GPU_GET_INFO_V2`, `FB_GET_INFO_V2`, `KGR_GET_INFO`) carry
`(index, value)` pairs, and a guest built with a shorter list can only name older indices. Across
all 171 measured tags, every `NV2080_CTRL_GPU_INFO_INDEX_*` and `NV2080_CTRL_GR_INFO_INDEX_*` name
keeps its value (only `*_INDEX_MAX` grows). ⊘ **`FB_INFO` is the exception:** index **60** is
`HEAP_RECLAIMABLE` at 595.x and `PARTITION_MASK_2` at 610.x (`HEAP_RECLAIMABLE` moved to **68**).
So an FB-info answer keyed on the host's numbering is right for guests ≤ 590 (max index 59) and
needs the index translated BY NAME for a 595 guest on a 610 host or vice versa.

## 8. What is next, and what needs an owner decision

### 8.1 Mechanical (no decision needed — the matrix already states the answer)

Every item below is "an encoder written for 580.159.04's layout, fed the measured layout instead",
the same move `GspStaticConfigInfo` already made (§4, byte-identical at every 580.x tag).

⚠ **Not always only offsets.** The matrix also shows where a field changed MEANING, which needs a
conversion rule, not a layout: `INTR_GET_KERNEL_TABLE.subtreeMap[]` is `{subtreeStart, subtreeEnd}`
(two `NvU8`, 14 bytes) at every tag ≤ 575.64.05 and `{subtreeMask}` (`NvU64`, 56 bytes) from
580.65.06 — kf3 has the host's masks, so a ≤575 guest needs mask → (start, end), refused by name
when a mask is not one contiguous run; and `table[].engineIdx` is an `MC_ENGINE_IDX` value, whose
numbering is itself per version (lower at 535/545) — translated by NAME through the matrix's
`mc_engine_idx` values, never by number.

| guest version | encoders to move onto the matrix (beyond what §4 did) | est. |
|---|---|---|
| 575.x | `INTR_GET_KERNEL_TABLE` (2068), `FB_GET_INFO_V2` (460), `GPU_GET_INFO_V2` (532), `GRMGR_GET_GR_FS_INFO`, `KGR_GET_FLOORSWEEPING_MASKS` (2368), `KGR_GET_GLOBAL_SM_ORDER` (26912), `KGR_GET_PPC_MASKS` | 1–2 days |
| 570.x | 575's + `KGR_GET_INFO` entry count, `GET_GLOBAL_SM_ORDER` 23072 (12-byte entries) | +0.5 day |
| 565 / 560 / 555 / 550 | + `INTERNAL_GET_DEVICE_INFO_TABLE` (9220/11268), `rpc_rc_triggered_v` (no `gfid` at ≤560), static info's `SM_info` block at ≤550 (a VALUE the guest may read — needs the 550 driver's reader checked, not only a layout) | 2–3 days |
| 590.48.01 | `KGR_GET_INFO` (3776), MSENC caps table grows to 6 | 0.5 day |
| 595.84 | + nine-field init args (the element header size cross-check becomes live), `GPU_GET_NAME_STRING` 68 | 0.5 day |

### 8.2 Decisions — RULED 2026-09-26 (coordinator, under the owner's standing rule: best-bet experiments on sub-branches; anything security-policy is flagged for the owner before merge)

1. **The grader is a 580-only RM client** — `kayfabe-rm-ladder` refuses any guest driver outside
   [580.65.06, 581) at rung R2 and carries 580 layouts (`GET_CLASSLIST_V2`, `NVOS46`, `NVOS47`,
   `GPFIFO_SCHEDULE`, UVM), so the thin guest cannot grade a 570/575/590/595/610 guest.
   **RULED: don't block on it.** Non-580 guests are graded by the fat-guest CUDA ladder
   (cup2/cup3/cup8) plus a few app samples. The grader lift (`rm.rs` onto a version-aware
   kf-host — one mechanism for both RM clients) is **its own task after the first guest versions
   land; not started here.**
2. **610: an RPC reply larger than one queue element** (`KGR_GET_GLOBAL_SM_ORDER` is 73 760 bytes
   at 610, above the 64 KiB element maximum). **RULED: implement when the walk reaches 610** — the
   queue writer mirrors the continuation reassembly kf-gsp already has; the split/reassembly
   round trip gets a unit test.
3. **Hosts below 580.65.06 lack `MC_GET_INTR_CATEGORY_SUBTREE_MAP`** (535/545 also
   `MC_GET_STATIC_INTR_TABLE`). **RULED: derive from the host's own interrupt table where the host
   exposes it; where it does not, author per family from ogkm source** (owner principle: per-arch
   values from source). Each cell of the matrix records which source it used.
4. **The walk kernel's PTX ISA 8.8** (host drivers < 575 cannot JIT it). **RULED: rebuild with
   NVRTC 12.2 (ISA 8.2) on this branch**, keep `.target` low enough that newer drivers JIT it
   forward (Blackwell JITs to sm_120 — verify on at least one Ada or Blackwell box as well as
   GA10x), re-run gates 7–9 and the 30-arm suite on 580.159.04. If a feature the kernel uses
   needs ISA > 8.2: stop and report which.
   ⇒ **DONE (2026-09-26):** NVRTC 12.2 compiles `kf_walk.cu` unchanged — no feature needs ISA > 8.2.
   The generator pins the floor (`PTX_ISA_REFUSED` on a newer NVRTC; checked against 12.9's 8.8).
   Hardware verification (bare metal, `scripts/bench/v3_gates.sh`, logs in
   `traces/driver_matrix/ptx82/`), 2026-09-26:

   | GPU (die, sm) | host driver | revision | walker gates 7–9 | all gates | control at ISA 8.8 (`2d40da64`) |
   |---|---|---|---|---|---|
   | RTX 3090 (GA102, sm_86) | 580.159.04 | `47348e3b` | PASS | **9/9** | 9/9 (`f72f9a58`); gate 9 walk p50 @13 000 rows **413 µs at both ISAs** |
   | RTX 5080 (GB203, sm_120) | 580.173.02 | `3855338c` | PASS | **9/9** | — |
   | RTX 4070 (AD104, sm_89) | 580.159.04 | `3855338c` | PASS | 7/9 | **7/9, the same two** |

   ⊘⊘ **ANSWERED 2026-09-26 (coordinator, `v3-adasys`, `traces/v3_adasys/FINDING.txt`): a kayfabe
   bug — neither the box nor Ada.** kf-host mapped system memory WITHOUT
   `NVOS46_FLAGS_CACHE_SNOOP_ENABLE`, so RM built non-coherent (PCIe No-Snoop) PTEs; on a
   bare-metal host with no GPU pass-through a copy engine reads zeros under the CPU's dirty lines
   (a nested-VM box can never show it). Fixed on `v3-adasys` (v3-mc18, which also sets the bit in
   the frozen grader). What stood here, kept because it is what the correction corrects:
   ⚠ The Ada box's gates 3 and 4 fail **identically with the 8.8 PTX**, so they are not the ISA:
   every check that has a copy engine read or write GUEST-RAM (sysmem) pages fails
   (`sysmem_to_fb_by_engine`, `fb_to_sysmem_by_engine`, a release semaphore in guest RAM reads 0)
   while every vidmem check passes. It was a container box (no `dmesg`, so an IOMMU fault could
   not be seen) — recorded as an open environment-or-Ada question, not a PTX result.
   ⇒ The PTX result stands either way: the two failing gates were the snoop bit, the walker gates
   7–9 passed at ISA 8.2 on Ada. The snoop flag's position (`CACHE_SNOOP` field of `NVOS46`
   `flags`) is the same at every host tag this branch walks (535 → 610), so the host carry passes
   it unchanged.
5. **535/545 capability allowlist.** **RULED: port nvproxy's 535.104.05 / 545.23.06 blocks as a
   separate, clearly marked commit**, list every entry that differs from the 580 allowlist here —
   ⊘ **a security-policy change: explicit owner review before it merges.**
6. **Default `guest-driver=`.** **RULED: re-select at fn 1** for (declared, reported) pairs whose
   pre-fn-1 surface is identical — the matrix states it per pair — and refuse by name otherwise.
7. **615.71.09** (a third GSP element shape): **RULED: out of the asked range; stays refused by
   name** (`NoEncoding`).

### 8.3 The host walk

**Code built (§2.2, 2026-09-26); hardware next.** Box `52788835` arrived with host driver 575.51.03
(the vast VM image's), was swapped to 580.159.04 so its bench guest image is the fixed guest
(580.159.04), and walks the host axis after its guest-axis queue: 580.x hosts first (inside the old
interval — every carry the identity except MSENC caps at 580.65.06/580.82.07), then 575.51.03
(back to the box's own driver: the first host below 580.65.06 — NVOS46 56 bytes, no subtree-map
control, 100-class classlist), then 570/565/550. Guest pinned by `guest-driver=580.159.04` (a
defaulted device would take the HOST's version as the guest's and re-select only on an identical
pre-fn-1 surface).
