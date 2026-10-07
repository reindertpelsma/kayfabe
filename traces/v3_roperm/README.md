# v3-roperm evidence (2026-09-26)

**STATUS: LIVE (2026-09-26).** Evidence for branch `v3-roperm`: guest PTE permissions join the
walker's diff key and the host map under ONE host policy (`kf_mem::apply::PermPolicy`) —
READ_ONLY and VOLATILE carried, ATOMIC_DISABLE only with `KF3_CARRY_ATOMIC_DISABLE=1`, PRIVILEGE
withheld from user twins. ⊘ The first round (below, `a1a82903`) carried ATOMIC_DISABLE too; the
follow-up section at the end supersedes that part. The defect and the read-duplication probe come
from `docs/design/V3_UVM_DEMAND_PAGING.md` §6 / E3.

## The defect

The walk kernel decoded a guest leaf's `READ_ONLY` bit (`kf_leaf_flags`) and then dropped it:

- `kf_hkey` / `kf_cuda::diffmodel::host_key` keyed a placement on aperture class and KIND only,
  so a guest RW→RO downgrade over the same backing produced **no diff**;
- `HostRm::map_kind` never set an `NVOS46` access flag, so every host twin mapping was
  **read-write**, whatever the guest asked for.

Stock UVM read duplication (`cudaMemAdviseSetReadMostly`) maps the GPU duplicate read-only and
relies on a GPU write faulting. Without the fault the write lands in the stale duplicate, guest
UVM still believes the CPU copy is valid, and the CPU reads the old value. No error is reported.

## Files

- `readmostly_probe.cu`: the probe, copied verbatim from `traces/v3_uvm_research/` on branch
  `v3-uvm-research`. Build: `nvcc -O2 -arch=sm_86 -cudart static`.
- `scripts/bench/readmostly_hook.sh` (in the tree): the `POST_CAPTURE_HOOK` that runs every mode
  in the booted kf3 fat guest, one process per mode, and records the new guest and HOST Xid
  lines after each one.
- The before/after captures and the verification numbers are listed below.

## Before the fix — master `283a5304`, measured 2026-09-26

vast 52732498, RTX 3060 (GA106), host driver 580.159.04, kf3 fat guest (stock 580.159.04),
`kf3-bins/283a5304`. Probe binary md5 `4ab506fc…`. Full capture: `before_283a5304_probe.log`.

| mode | result | CUDA error | host Xid |
|---|---|---|---|
| `reprefetch` | `ok bad=0` | none | none |
| `gpuwrite` | ⊘ **`FAIL bad=1048576`**: every one of the n = 2^20 elements is wrong | **none** | **none** |
| `downgrade` | ⊘ **`FAIL bad=1048576`** | **none** | **none** |
| `fault` | `gpu_sum -> 719` | 719 | `31 … FAULT_PDE ACCESS_TYPE_VIRT_READ` (no fault servicer; expected) |
| `reprefetch` (again) | `ok bad=0` | none | none |

⇒ The research note's E3 prediction holds exactly. A GPU write to a read-only duplicate lands in
the stale copy with **no error, no Xid, and every one of the 1 048 576 values wrong** (the probe
counts mismatches; it does not print which value the CPU saw). That is
silent corruption, and it happened in both the fresh-RO-map shape (`gpuwrite`) and the in-place
RW→RO revoke shape (`downgrade`).

## After the fix — `v3-roperm` `a1a82903`, measured 2026-09-26

Same box, same boot recipe, the same probe binary (md5 `4ab506fc…`), `kf3-bins/a1a82903`. The
AFTER binary contains the new code (its strings include `same-kind same-permission`; the BEFORE
binary's do not). Full capture: `after_a1a82903_probe.log`.

| mode | result | CUDA error | host Xid |
|---|---|---|---|
| `reprefetch` | `ok bad=0` | none | none |
| `gpuwrite` | ✔ **fails loudly** | **719** | ✔ **`31 … FAULT_RO_VIOLATION ACCESS_TYPE_VIRT_WRITE`** |
| `downgrade` | ✔ **fails loudly** | **719** | ✔ **`31 … FAULT_RO_VIOLATION ACCESS_TYPE_VIRT_WRITE`** |
| `fault` | `gpu_sum -> 719` | 719 | `31 … FAULT_PDE ACCESS_TYPE_VIRT_READ` (unchanged) |
| `reprefetch` (again) | `ok bad=0` | none | none |

⇒ The host twin now holds the guest's read-only duplicate **read-only**, so the GPU write faults
on the host as a read-only violation instead of landing. Guest CUDA gets 719, and no wrong value
is ever returned. Both shapes are covered: the fresh read-only map (`gpuwrite`) and the in-place
RW→RO revoke, which now reaches the host as UNMAP + MAP read-only (`downgrade`). The read-only
modes (`reprefetch`, twice) still pass with correct values. The device survives each 719: the
next process in the same boot passes.

⊘ **This is not bare-metal behaviour yet.** On bare metal these two modes PASS: the write faults
as a *replayable* fault, guest UVM collapses the duplicate and the value is right
(`bm_rtx3060ti_580.159.04_hmm1.out` on branch `v3-uvm-research`: ~15 k replayable faults each).
kf3 delivers no replayable faults (`V3_UVM_DEMAND_PAGING.md`), so the loud 719 is the correct
outcome until fault delivery exists. It replaces a silent wrong answer.

⚠ Instrument note: `boot_capture.sh`'s *"archive rev STAMPED IN THE BINARY"* line reads
`kayfabe-rev:1da1425e…` in BOTH captures. That stamp belongs to the old-tree `nvkvm` archive,
which is linked into the same QEMU build and was not rebuilt. It does not describe the kf3
archive. The authoritative provenance here is the `kf3-bins/<rev>` path, cargo's `Compiling kf-*`
lines in the kf3 build log, and the strings check above.

## Verification at `a1a82903` (same box)

| check | result | file |
|---|---|---|
| `cargo test --workspace --no-fail-fast` | 5 139 passed, 82 failed. Master `283a5304` on the same box: 5 132 passed, 82 failed, and **the 82 failing names are identical** (`comm` of the two sets is empty). All 82 are in the old tree (`kayfabe-*` crates, `tests/`); the sample checked reads source files the v3 archive moved. Every `kf-*` crate passes. The +7 are this branch's new tests | — |
| CUDA walk suite, `make check` + `./kf_tests` | `make check` rc 0; **71/71**, incl. `diff/permission_edit_remaps` and `correctness/owning_big_pte_carries_its_permissions` | `walk_make_check_a1a82903.log`, `kf_tests_a1a82903.log` |
| v3 gates (`scripts/bench/v3_gates.sh`) | **9/9** — gate 9: VER2 3 298 and VER3 3 208 runs compared, **0 mismatches**, with 452 / 424 in-place permission edits | `gates_a1a82903.log` |
| `KF_DEVICE=kf3 fast_suite.sh … 180` | **30/30 PASS** | `fast_suite_kf3_a1a82903.out` |
| CUDA ladder, fat guest (`cuda_ladder.sh guest … 2 cup3,cup8`) | `cup3` **`CUP3_VAL=43`** ×2; `cup8` **`BAD=0 MAXERR=0`** ×2 | `cuda_ladder_a1a82903_guest.out` |
| read-duplication probe | see "After the fix" above | `after_a1a82903_probe.log` |

## Follow-up (coordinator review, 2026-09-26) — ATOMIC_DISABLE off by default, PRIVILEGE withheld

Box vast 52742061, RTX 3090 (GA102), host driver 580.159.04, kf3 fat guest (stock 580.159.04)
(destroyed mid-run by the idle reaper, which could not reach it: no ssh alias was registered for
it; the final-head suite, ladder and apps ran on vast 52755785, same model, below).
Revisions: master `283a5304` (before any of this branch), `303d0929` (first round: ATOMIC_DISABLE
carried), `6a985dbb` (follow-up, before the rebase), and — after master moved — base `dd3aed08` and
the rebased head `5fead67d`. Bare metal is the same box's host, no QEMU (`phase1_6a985dbb.log`).

### ATOMIC_DISABLE — `atomics_probe.cu` (GPU atomics on CPU-resident managed memory)

Every mode checks all 2^20 elements and one fully contended counter against exact expected values.

| mode | bare metal | guest `283a5304` / `dd3aed08` (not carried) | guest `303d0929` (carried) | guest `6a985dbb` / `5fead67d` (default) | guest `5fead67d` + `KF3_CARRY_ATOMIC_DISABLE=1` |
|---|---|---|---|---|---|
| `devmem` (control, vidmem) | ok | ok | ok | ok | ok |
| `write` (control, plain stores) | ok | ok | ok | ok | ok |
| `accessedby` (device-scope atomics) | ok | ok | ok | ok | ok |
| `accessedby_sys` (`atomicAdd_system`) | ok | ok | ⊘ **719**, host Xid 31 `FAULT_INFO_TYPE_ATOMIC_VIOLATION ACCESS_TYPE_VIRT_ATOMIC` | ✔ **ok** | ⊘ 719, same Xid |
| `prefcpu` (preferred location CPU) | ok | ok | ok | ok | ok |
| `prefetchcpu` (GPU → CPU prefetch) | ok | ok | ok | ok | ok |

⇒ Carrying ATOMIC_DISABLE regressed exactly one shape: **system-scope** atomics on a sysmem-resident
managed page (device-scope atomics through the same mapping did not fault). Off by default, every
mode matches bare metal again; the knob reproduces the regression, so the plumbing is intact for
the day fault delivery exists.

`systemWideAtomics` (cuda-samples v12.5): bare metal `returned OK`; guest **719 at every revision**,
host Xid 31 `FAULT_PDE`. It takes the pageable path (`pageableMemoryAccess=1`, HMM): a demand fault
kf3 cannot deliver (class C), not ATOMIC_DISABLE. It cannot pass in the guest before fault delivery.

The read-duplication probe is unchanged by the follow-up: `gpuwrite` / `downgrade` fail loudly
(719, `FAULT_RO_VIOLATION`) at `6a985dbb` and `5fead67d`, silently (`bad=1048576`) at
`283a5304` and `dd3aed08`; `reprefetch` ok everywhere.

Captures: `probes_guest_<rev>[_knob|_adcarried].log`, `phase1_6a985dbb.log` (bare metal).

### PRIVILEGE — withheld from user twins, counted

A mirror is a guest-KERNEL space when its client is an RM-internal client (`0xC1E0xxxx`, a handle
range no guest process can hold) or a Translated channel is born in it; every other mirror is a
user twin and withholds privileged leaves. The status line counts both sides: `priv_withheld`
(withheld runs, per diff they appear in: a withheld run is never committed, so every walk of the
space re-emits it, as the protocol re-emits any refused map) and `priv_mirrored`.

| workload (boot) | `priv_withheld` | `priv_mirrored` | result |
|---|---|---|---|
| probes, 11 CUDA processes (`5fead67d`, `d6959acb`) | **0** | 72 | as in the tables above |
| fast suite, 30 boots (`d6959acb`) | **0** | 78 | 30/30 |
| CUDA ladder, 4 boots (`d6959acb`) | **0** | 12 per boot | cup3 = 43 ×2, cup8 `BAD=0 MAXERR=0` ×2 |
| PyTorch, llama.cpp ×2, NVENC ×2, NVDEC, vectorAdd, simpleAtomicIntrinsics (`d6959acb`) | **0** | 18 per boot | all PASS |
| **vkpeak** (Vulkan, `d6959acb`) | **135 637** (≈30 per walk over 4 599 walks) | 18 | **PASS**, every figure within 1% of the host |

- CUDA's own VA spaces never carried a privileged leaf. Every privileged leaf of the CUDA workloads
  was in an RM-internal client's space (`0xc1e0xxxx:0xbaba0042`), which stays mirrored.
- ★ **Vulkan's does.** In the vkpeak boot the user client `0xc1d00016` (`kernel=false
  internal=false`) had six privileged runs in RM's internal VA window of its own space:
  `0x120000000+0x10000` (sysmem), `0x120010000+0x196000`, `0x1201ab000+0x3000`,
  `0x120230000+0xa0000`, `0x121000000+0xa00000` (vidmem). These are GR context and global buffers
  (RM sets `MEMDESC_FLAGS_GPU_PRIVILEGED` on them, `kernel_graphics_context.c:1127,1277`,
  `kgraphics_gm200.c:173,199`). All were withheld, and vkpeak still passed at host speed
  (`apps_d6959acb/vkpeak_withheld.txt`, `ro3g/vkpeak.guest.log` vs `ro3/vkpeak.host.log`).
  The twin runs on host RM's own context buffers, so no legitimate path read the guest's. Before
  this branch, a Vulkan shader could reach those buffers through the twin.

⚠ Known limit of the classification: a space becomes KERNEL at its first Translated birth, so a
privileged leaf walked in it before that birth was withheld and is placed at the space's next walk
(its next invalidate or split), not at the birth. Measured above, no privileged leaf was ever
walked in a space that turned kernel that way (UVM's 11 spaces had none); every privileged leaf was
in an RM-internal client's space, which is kernel from creation.

## Final verification — `d6959acb` against master `dd3aed08` (vast 52755785, RTX 3090 GA102)

`d6959acb` is the rebased code (`5fead67d`) plus evidence files only. Later commits change only
comments, docs and traces.

| check | result | file |
|---|---|---|
| `cargo test --workspace --no-fail-fast` | 3 387 passed / 4 failed; master `dd3aed08`: 3 376 / 4; **identical failing sets** (4 old-tree tests); every `kf-*` crate 797 / 0 | `cargo_failing_{dd3aed08,d6959acb}.txt` |
| CUDA walk suite (`make check`, `kf_tests`) | rc 0; **72/72** | `walk_make_check_d6959acb.log`, `kf_tests_d6959acb.log` |
| v3 gates | **9/9**; gate 9 VER2 3 377 / VER3 3 395 / VER2-keyall 3 558 runs, **0 mismatches**, 430 / 444 / 448 permission edits | `gates_d6959acb.log` |
| `KF_DEVICE=kf3 fast_suite.sh … 180` | **30/30** | `fast_suite_kf3_d6959acb.out` |
| CUDA ladder, fat guest | cup3 **43** ×2; cup8 **`BAD=0 MAXERR=0`** ×2 | `cuda_ladder_d6959acb_guest.out` |
| app samples, one per boot | host **9/9**, guest **9/9**; digests equal to the host's (`torch_correct` `763c693a5a53f948`, `llama_cpp_gen` `30451301af8e011d`) | `apps_d6959acb/` |
| probes | as the tables above | `probes_guest_d6959acb.log` |

⚠ On the first box (`5fead67d`) one extra test failed once,
`spawn_unsafe::tests::a_child_runs_from_an_image_with_no_path_at_all` (`execve` errno 26,
ETXTBSY). It passed 5/5 when re-run there and did not recur at `d6959acb`, so it is flaky, not
caused by this branch.

## Re-verification after master moved — head `e5f13f28` on master `02b27c2a` (vast 52789433, RTX 3090 GA102)

Master moved to `02b27c2a` (the refusal audit, floor-swept GR) after the run above. The only
conflict in the rebase was the kf3 status line, which now carries both sides' counters. The PTX
regenerated byte-identical. Everything in `reverify_e5f13f28/`.

| check | result |
|---|---|
| `cargo test --workspace` | 3 414 passed / 4 failed; master `02b27c2a` 3 402 / 5. The head's 4 are master's 4 stable old-tree failures; master's 5th is the flaky `spawn_unsafe` ETXTBSY test. `kf-*` crates 805 / 0 |
| walk suite | `make check` rc 0; **72/72** |
| v3 gates | **9/9**; gate 9's four differentials 0 mismatches |
| `KF_DEVICE=kf3 fast_suite.sh … 180` | **30/30**; `priv_withheld=0`, `priv_mirrored=84` over 30 boots |
| CUDA ladder, fat guest | cup3 **43** ×2; cup8 **`BAD=0 MAXERR=0`** ×2; `priv_withheld=0` |
| probes (bare metal, guest default, guest `KF3_CARRY_ATOMIC_DISABLE=1`) | atomics: bare 6/6 ok; default 6/6 ok; knob → `accessedby_sys` 719 (`FAULT_INFO_TYPE_ATOMIC_VIOLATION`). Read-dup: `gpuwrite` / `downgrade` 719 (`FAULT_RO_VIOLATION`), `reprefetch` ok |
