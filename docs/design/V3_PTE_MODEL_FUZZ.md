# V3 — guest PTE → host PTE: a GPU-free model-based differential fuzz

**STATUS: LIVE, 2026-11-18 (harness and oracle; findings PF-1 and PF-2 are fixed by the two changes stacked on the harness, PF-3 is pinned, PF-4 is recorded; the PF-2 walker change has NOT run on a GPU).**

Why this target: the two guest-only failures found before this date both lived in the guest-PTE →
host-PTE translation. The depth-surface KIND was flattened by `host_pte_kind` (PR #480,
hardware-confirmed 2026-11-18, `traces/open_rows_guest_20261117/confirm/README.md`), and guest SPARSE PTEs are counted and dropped, so the host range is INVALID
(`V3_SPARSE_RESIDENCY_RCA.md`, PR #493/#494). Owner rule (`V3_REPLAY_TESTS.md`): a kayfabe-core
failure gets a GPU-free Rust reproduction first.

## 1. What is under test

Production's pipeline, end to end, without the GPU (`crates/kf-mem/src/sim/pte_fuzz.rs`):

1. Guest page-table bytes, built and edited by a seeded generator (`Gen`, xorshift from `sim::Rng`;
   no new dependency).
2. The walk kernel's own source, `cuda/walk/kf_walk.cu`. Its serial walk (`kf_walk_one`) runs on the
   CPU through `cuda/walk/kf_pte_fuzz_walk.cpp`, which the test builds with `g++` the way
   `kf_cpu_walk.cpp` is built. The descriptor the walk is handed is production's:
   `kf_mem::apply::walk_format(family)`, the same function `kf_qemu::device` realize calls.
3. The commit-on-ack diff `kf_cuda::diffmodel::diff_with` / `commit` (the kernel's Rust spec), then
   `PermPolicy::diff_run`, then `apply_entry`.
4. The real `BatchedVas` over the host-RM model (`SimMirror` / `SimRm`), optionally as a user twin
   (it withholds privileged leaves). Since this date, each model mapping records the kind and
   permissions its map verb carried (`SimMap::attr`, `SimRm::host_attr`).

⚠ The PARALLEL walk (`kf_par_*`, what production runs) is not executed here: its warp intrinsics are
CPU stubs. It shares every decode function with the serial walk (`kf_valid`, `kf_leaf_flags`,
`kf_addr`, `kf_pte_sparse`, `kf_big_pte_owns_slot`, `kf_acc_emit`/`kf_emit`). `kf_tests.cu` grades
it on a GPU.

## 2. The oracle

`crates/kf-mem/src/sim/pte_oracle.rs` is a reference translator written from the ogkm field
definitions, not from kayfabe's constants. Each field is cited to `ogkm-580` (580.159.04) by file and
line:

| what | source |
|---|---|
| level geometry per chip | `kern_gmmu_fmt_gp10x.c:48-105` (TU10x), `_ga10x.c:46-56` (GA10x/AD10x: PD1 512 MiB, `:55`), `_gh10x.c:63-116` (GH100), `_gb10x.c:50-61` (Blackwell: PD2 256 GiB, `:60`); chip binding `generated/g_kern_gmmu_nvoc.c:670-690` |
| PTE vs PDE at a dual-role level | `src/nvidia/src/libraries/mmu/gmmu_fmt.c:64-86` |
| VER2 PTE | `pascal/gp100/dev_mmu.h:120-157` (VALID, APERTURE, VOL, PRIVILEGE, READ_ONLY, ATOMIC_DISABLE, ADDRESS_VID 32:8, ADDRESS_SYS 53:8, KIND 63:56); apertures `:80-83` |
| VER2 PDE / dual PDE | `pascal/gp100/dev_mmu.h:98-119`; PDE apertures `:27-30` |
| VER3 PTE | `hopper/gh100/dev_mmu.h:130-188` (PCF 7:3 and its enumerants `:140-159`, KIND 11:8, ADDRESS 51:12) |
| VER3 PDE / dual PDE | `hopper/gh100/dev_mmu.h:52-128` |
| SPARSE PTE / PDE | `kern_gmmu_gm200.c:44-69`; VER3 `gh100/dev_mmu.h:141` |
| unmapped big PTE hides the 4 KiB PTEs | `kernel-open/nvidia-uvm/uvm_mmu.h:201-210` |
| a SPARSE big half ends the 2 MiB | `gmmu_trace.c:430-466` |
| KIND values and uncompressed layouts | `turing/tu102/dev_mmu.h:97-112`, `hopper/gh100/dev_mmu.h:35-50`, `blackwell/gb202/dev_mmu.h:26-41` |

The oracle answers per VA: a leaf, SPARSE, INVALID, unreadable by kayfabe (a table in system memory
or outside guest vidmem), or `Undecided` when the sources do not say what the MMU does. `Undecided`
cases are a sparse big PTE, an invalid big PTE with stray bits, and PF-4 below; they are never
asserted on. The encoder in `pte_fuzz.rs` (`enc_pte`, `enc_pde`, `enc_big`) and the decoder are
written separately from the same citations, and `findings::oracle_round_trips_every_field` checks
each against the other.

## 3. Properties

After every refresh of a case (4 refreshes, with 1 to 3 guest edits between them):

- **P1 fields**: every host page has the guest's backing, the uncompressed KIND (the documented
  `per_map_kind = false` PITCH is accepted for PITCH/GENERIC), READ_ONLY, VOLATILE, and
  ATOMIC_DISABLE exactly when the policy carries it.
- **P2 hostile guest**: no host page outside what the guest's translation names and the host
  authorised: the store below the carve-out, and guest RAM through the VMM layout. No privileged
  leaf on a user twin, no peer.
- **P3 unmap unmaps**: no host page where the guest's translation is INVALID or SPARSE.
- **P4 idempotence**: re-walking an unchanged guest gives an empty diff when nothing was refused,
  and leaves the host unchanged.
- **P5 bounded work**: host verbs ≤ 2 × runs + 2 per refresh.
- **No silent absence**: a guest-mapped page the host lacks must be accounted to a FAILED ack or a
  walker `KFWR_R_*` bit. A carve-out-clipped tail is accounted when it is placed.

Generator coverage: every KIND value 0x00–0xFF (VER3: the 4-bit field) with the depth and
compressible kinds over-weighted; all four apertures; RO/VOL/PRIV/ATOMIC and VER3's ACD bit; one
stray bit anywhere above the flags; addresses at the store end, at the carve-out, straddling it, at
the guest-RAM chunk ends and in the hole between them, and at 2^37/2^40/2^52; misaligned big pages;
4 KiB, 64 KiB, 2 MiB and 512 MiB leaves plus the 256 GiB level; SPARSE, UNMAPPED and random invalid
words; VAs inside and outside the guest reservation, at 0 and at the top of the VA space; and the
knobs per case: family, `per_map_kind`, atomic carry, user twin, batching, low reservations.

## 4. Commands

- Quick, part of `cargo test -p kf-mem` (2000 cases, about 3 s in debug):
  `cargo test -p kf-mem --lib pte_fuzz`.
- Campaign: `KF_FUZZ_SECONDS=600 cargo test -p kf-mem --release --lib -- --ignored pte_fuzz_campaign --nocapture`.
  `KF_FUZZ_SEED=<hex>` moves the seed, `KF_FUZZ_CASES=<n>` runs a fixed count, and
  `KF_FUZZ_DIGEST=<file>` writes `<seed> <family> <digest>` per case.
- Before/after equivalence of a kernel change: `KF_PTE_FUZZ_CU_DIR=<dir with another kf_walk.cu,
  kf_walk.h, kf_cpu_shim.h, kf_pte_fuzz_walk.cpp>`. `KF_PTE_FUZZ_NO_UNCODED=1` hands the walk the
  descriptor without PF-2's level.

## 5. Results

### 5.1 Campaigns (MEASURED 2026-11-18, `pte_fuzz_campaign` in release, GPU-free, base a60cc24b2)

- 120 s: 88,911 cases, 4,713,929 leaf-steps, all five families. Classes after PF-1/PF-2: the two
  KNOWN classes below, plus PF-4 and a harness accounting fix, both since resolved in the oracle and
  harness.
- PR #480 check (2026-11-18). This tree's base does not contain #480 (`git merge-base
  --is-ancestor fa87599a6` is false for next, master and the base). With #480's `apply.rs` change
  applied temporarily, a 40 s campaign (36,524 cases, all families, both `per_map_kind` settings,
  every aperture) showed ZERO depth/stencil-on-guest-RAM mismatches. So #480 closes the class; it
  is not a hole. Without #480 the class is reported KNOWN, and the entry goes when #480 is merged
  here.

### 5.2 Findings

| id | class | families | what | status |
|---|---|---|---|---|
| PF-1 | fidelity | Turing | The VER2 descriptor gave every VER2 family GA10X's 512 MiB page at PD1. GP10X (TU10x) has none, so a Turing PD1 entry with bit 0 set became a 512 MiB host mapping where the guest's MMU sees a PDE. | FIXED: `Family::pd1_holds_page`, `kf_format_ver2_no_pd1_page`, `walk_format`. Host side only, no PTX change. Tests `findings::pf1_*`. |
| PF-2 | fidelity, fail-closed | Blackwell | GB10X's PD2 holds a 256 GiB page that the walk's two-bit page-size code cannot name. The VER3 descriptor read a VALID PD2 entry as a PDE: a VIDEO page vanished with no refusal, and a PEER page (PTE aperture 1 = PDE VIDEO) sent the walk into the page's data as if it were a PD1 table. | FIXED (fail closed): descriptor code `KF_PS_UNCODED`, refusal `KFWR_R_UNCODED_PAGE` (bit 15, sampled), in the serial and the parallel walk. `kf_walk.ptx` was rebuilt with the pinned NVRTC 12.2.140 (`walk_ptx_repro.py walk: ok`). Test `findings::pf2_*`. ⚠ Not run on a GPU. |
| PF-3 | fidelity | all | A VALID PTE whose KIND the family does not define is placed PITCH, where the hardware raises a kind fault (`[inferred]`: `NV_PFAULT_FAULT_TYPE_UNSUPPORTED_KIND` exists, but which values fault is not in the sources). Covers INVALID 0x07, VER2 values above 0x0F, and SMSKED over SYS_COHERENT on Turing–Ada. | PINNED, no change: the guest reaches only its own memory (`findings::pf3_*`). |
| PF-4 | fidelity (loud) | all | A dual PDE whose big half names a table kayfabe cannot read (system memory) is refused `KFWR_R_FOREIGN_AP`, but its small table is still published, although an unread big PTE might own the slot. | RECORDED. The space already fails by name. Option: withhold the small half too (a kernel change). |

PF-2 reachability (ogkm-595, read 2026-11-18): RM selects a 256 GiB page only on GB100, GB102, GB110
and GB112 (`bPageSize256gbSupported`, `generated/g_kern_gmmu_nvoc.c:319-328`; consumer GB20x is
false). It does so from the user allocation attribute `NVOS32_ATTR2_PAGE_SIZE_HUGE_256GB`
(`mem_mgr/mem.c:318-319`, `system_mem.c:73-74`, `gpu/mem_mgr/mem_mgr.c:1912-1915`) or from a fabric
import whose page size is 256 GiB (`mem_fabric_import_ref.c:218`). Either way the backing is
256 GiB-aligned and at least 256 GiB, which no kf guest has. Mapping such a page (a page-size code
the ABI lacks) stays DESIGN-ONLY; refusing it by name is what ships.

Isolation analysis of PF-1 and PF-2's PEER variant: a misread entry produces leaves through the same
chokepoints as any leaf. Table reads are bounded by the window (`kf_win_load`, `kf_table_ok`). A
vidmem leaf is bounded by the span at both emit chokepoints (`kf_walk.cu` `kf_emit` and
`kf_acc_emit`, `KFWR_R_LEAF_OOB`), then by `store_bytes` (`ledger.rs` `desired_from_leaves`) and the
carve-out (`apply.rs` `prepare_row`). A sysmem leaf is bounded by the VMM's guest-RAM layout
(`ram_offset`). So the host mapping names only the guest's own store below the carve-out, or its own
RAM. P2 checks this on every case and found no violation (MEASURED 2026-11-18 in the model, the
`pte_fuzz_campaign` runs of §5.1 and the equivalence run of §5.5).

### 5.5 Before/after equivalence of the PF-2 walker change (MEASURED 2026-11-18, `pte_fuzz_campaign`)

`KF_FUZZ_SEED=e9017a1e KF_FUZZ_CASES=60000`, all five families with per-case knobs, 3,179,449
leaf-steps. Run A used the base walker (`kf_walk.cu`/`.h` at a60cc24b2, `KF_PTE_FUZZ_CU_DIR`) and
run B the new walker, both with the descriptor without the uncoded level
(`KF_PTE_FUZZ_NO_UNCODED=1`). Their per-case digest tables (walk runs, refusal/sparse/stop counts,
ack codes and host image after each of 4 refreshes) are byte-identical: sha256
`8467d8beb526907f76766f5d4f6759ae006012e62347a53799b69fa746939a9a`. Run C, the production
descriptor (sha256 `38b3abdfc7eff4515424332b53f72574f8659787188e02c814c485418076ca75`), differs from
B in 1,356 cases, all Blackwell. `[inferred]` each of them holds a VALID PD2 entry, since the
descriptor delta is `dir[2].leaf_ps` alone.

### 5.3 Known, not new

The guest-SPARSE → host-INVALID gap (PR #493/#494) appears as absence over SPARSE ranges and is not
counted as a finding.

### 5.4 Not covered (INFERRED or open)

- The parallel walk (§1).
- 128 KiB big pages (`bigPageShift` 17, `gp10x.c:56`): the generator and the descriptor use 64 KiB
  only.
- Host RM's own acceptance of each kind/permission (the model accepts what it is given). The host
  carry's `kindOverride` refusal on an old host is outside the model.
