# V3 DRIVER MATRIX — both driver axes, measured per ogkm tag

**STATUS: LIVE (in progress), 2026-09-26, branch `v3-drivers`.** The two-axis model, the
inventory, the derivation pipeline and the generated-table design are built on BOTH axes; the guest
axis is being walked on hardware (§6 is the running matrix) and the host axis is wired (kf-host
carries every host struct to the host's measured layout, §2.2). Boxes: vast `52746206` (RTX 3090
GA102, Xeon E5-2673 v4) and `52788835` (RTX 3080 Ti GA102, 38 cores), host driver **580.159.04
open** on both.

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
| state 2026-09-26 | table assembled from **measured** per-tag layouts; 580.x walked (thin 30/30 + ladder 4/4); **590.48.01 and 595.84 initialize** (grader is 580-only); 575/570/565 past the INTR wall at head; 550 → device-info carry; 610 → register-map carry; 535/545 need the capability rows (§6, §7) | R2 gates on **measurement**; every host struct carried by name (`kf_abi::hostabi`, 53 controls pinned to the matrix); subtree map authored per family below 580.65.06 (ruling 3); walker PTX at ISA 8.2 (ruling 4) |

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
| 580.159.04 | thin **30/30**, ladder **4/4** | `47348e3b`, `6de22590` |
| 580.105.08 | thin **30/30**, ladder **4/4** | `47348e3b` |
| 580.65.06 | thin 28/30 (2 × adapter-init flake) | `47348e3b` |
| 580.95.05 | thin 29/30 (`rpc-mixed-allocs`: a SYSMEM object read `0xffffffff`, dead mapping — rerun queued) | `47348e3b` |
| 580.126.09 / 580.173.02 / 580.178.04 | *running* | `47348e3b` |
| **590.48.01** | init ✔, **ladder 4/4** | `6de22590` |
| 595.84 | init ✔, ladder *running* | `6de22590` |
| 575.57.08 | init ✔ (after the element-size fix), ladder *running* | `6de22590` |
| 575.51.03 | INTR wall at `47348e3b`; re-run queued | — |
| 570.148.08 | init ✔, ladder queued | `6de22590` |
| 570.124.06 | INTR wall at `47348e3b`; re-run queued | — |
| 565.57.01 | init ✔ (BIF refused, carried since), ladder queued | `6de22590` |
| 550.54.14 | device-info wall at `6de22590` → carried; re-run queued | — |
| 610.57.04 | register-map wall at `47348e3b` → carried + large RPCs; re-run queued | — |
| 545.23.08 | needs a ≤ 6.6 guest kernel (harness built) + capability row (owner review) | — |
| 535.309.01 | capability row (owner review, `567942c1`) | — |

Host axis, guest **580.159.04**: 580.95.05 / 580.65.06 / 575.57.08 / 570.148.08 / 565.57.01 /
550.54.14 queued on box 2 (§8.3), each with mixed pairs (guest 590.48.01 and 575.57.08 ladders).

★ Every row carries its source revision. Box: vast `52746206`, RTX 3090 (GA102 `0x2204`), Xeon
E5-2673 v4 (nested KVM), host driver **580.159.04 open**. Thin suite = `KF_DEVICE=kf3
fast_suite.sh <tag> 180` (30 arms); gates = `scripts/bench/v3_gates.sh`; ladder = the fat-guest
`cuda_ladder.sh guest` (cup2 CE round trip, cup3 `=43`, cup8 2048² matmul `bad=0 maxerr=0`,
cup8bench, every timed iteration verified).

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

   ⚠ The Ada box's gates 3 and 4 fail **identically with the 8.8 PTX**, so they are not the ISA:
   every check that has a copy engine read or write GUEST-RAM (sysmem) pages fails
   (`sysmem_to_fb_by_engine`, `fb_to_sysmem_by_engine`, a release semaphore in guest RAM reads 0)
   while every vidmem check passes. It was a container box (no `dmesg`, so an IOMMU fault could
   not be seen) — recorded as an open environment-or-Ada question, not a PTX result.
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
