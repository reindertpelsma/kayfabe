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
user twin and withholds privileged leaves. `[measured 5fead67d, pf_head]` 11 CUDA processes:

- `priv_withheld=0` — **no user twin was ever handed a privileged leaf**;
- `priv_mirrored=72` — every privileged leaf the walker reported lives in an RM-internal client's
  VA space (`0xc1e0xxxx:0xbaba0042`, 11 of them), mirrored as before (`privilege_census_5fead67d.txt`);
- 11 spaces turned kernel at a Translated birth (UVM's), 11 user spaces had Passthrough births, and
  no space was both.

⇒ Nothing legitimate reached a user twin through a privileged leaf in these workloads, so the
withholding has nothing to take away from them today. It is the guard for a guest kernel that
does place one there. The RM-internal range being kernel from creation is what keeps the 72
leaves mirrored.
