# v3-roperm evidence (2026-09-26)

**STATUS: LIVE (2026-09-26).** Evidence for branch `v3-roperm`: guest PTE permissions
(READ_ONLY, ATOMIC_DISABLE, VOLATILE) are carried into the walker's diff key and into the host
map. The defect and the probe come from `docs/design/V3_UVM_DEMAND_PAGING.md` §6 / E3 on branch
`v3-uvm-research`.

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
