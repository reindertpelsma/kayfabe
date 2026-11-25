# Security review 3 of 4: the CUDA side that runs on the GPU

STATUS: RESEARCH, 2026-11-13 (committed 2026-11-25; scrubbed copy). Point-in-time report, not maintained.

> Findings status in this report is as of 2026-11-13 at master `921c39444`. Current status of each finding is in the
> coverage table of the [stage-2 audit](2026-11-23-v3-stage2.md). Scrub: local paths, worktree names, host and account
> details and references to working notes or proof-of-concept files that are not in the repository were removed or
> replaced by "not retained"; the technical findings, severities, code locations and reasoning are unchanged.

Review status: Source review, read-only, no GPU. Tree: `origin/master` `921c39444`. Notes, scripts and rebuilt PTX:
(not retained). No product file was changed, nothing was pushed.

Labels: **CODE READ** (read in source and/or PTX), **MEASURED** (a script or build run here, CPU
only), **INFERRED** (reasoned, not observed). Line numbers are for `921c39444`.

## 0. Verdict in five lines

- **No guest breakout found** (no CRITICAL). Every guest-steered load in the walker is bounded
  before the dereference, in the `.cu` and in the committed PTX; every store I traced is bounded
  by a kayfabe-owned capacity. The display kernels have one 2-byte over-read that stays inside
  the guest's own store.
- The safety of several writes rests on **host-side invariants only** (entry count, layout, slot
  uniqueness, whole-report refusal). They hold today; none has an in-kernel backstop.
- **Open and guest-triggerable:** stale VER3 translations (intra-guest), a guest process failing
  every space's walk, GPU work not bounded by the budget, and an **unbounded host log line per
  walk** (new).
- **The committed PTX cannot be reproduced byte for byte here** (NVRTC 12.2.140 is not on this
  box) and **no CI gate ties it to the `.cu`**. A structural comparison against a 12.9 rebuild
  matches on every entry.
- All ten stage-1 walker findings (S1-60 to S1-69) and S1-05 are **still open** on master.

## 1. Falsifiers, stated first, and results

| # | Hypothesis | What would prove it wrong | Result |
|---|---|---|---|
| H1 | Every load of guest memory is bounded before the dereference, in the PTX too | A `ld.volatile` in `kf_walk.ptx` not dominated by the `off > len` / `len - off < 8` compare | **Not falsified, MEASURED.** 17 of 17 volatile loads carry the guard (`ptx_guard.py` plus reading each window). |
| H2 | No store in any kernel can leave its buffer for any guest input | A store whose index is not bounded by a capacity we allocated, reachable with host invariants intact | **Not falsified, CODE READ.** Falsified only if a host invariant breaks (F-9). |
| H3 | A coalesced or joined run can leave the store although each leaf was contained | A join that adds a length without VA+GPGA+flags contiguity, or a containment test after a coalesce | **Not falsified, CODE READ** (three join sites; `check_s39.py` (c) passes). |
| H4 | The committed PTX is what the `.cu` produces | Different entries, `.param` lists, or load/store/atomic counts per entry against a rebuild | **Not falsified structurally, MEASURED; byte identity UNVERIFIED** (§5). |
| H5 | A walk always terminates | A loop whose bound is a guest or device value with no compile-time cap | **Not falsified, CODE READ** (`make check-invariants` passes here). The amount of work is not bounded by the budget (F-3). |
| H6 | `ComposeLayer::check` / `YuvLayer::check` bound every display kernel address | A CPU model finds an accepted layer with an access outside the extent | Compose: **not falsified, MEASURED** (~145k layers). YUV: **falsified** for one class (F-6). |
| H7 | `walk-v2-frozen` is not loaded by v3 | A `kf-*` crate or script that embeds it | **Not falsified.** Only `crates/kayfabe-cuda/src/walk.rs:29`, `kayfabe-isolate-host/build.rs:170` and archive tests reference it; no `kf-*` crate does. `kayfabe-cuda` still builds as part of the frozen grader closure (UNVERIFIED whether it is ever run against a GPU today). |

## 2. Findings

Severity follows the owner addendum: a breakout is CRITICAL; unbounded DoS counts unless a
planned limit is documented.

### F-1 · HIGH · VER3 (Hopper, Blackwell): stale 4 KiB PTEs under an unmapped big PTE are published (S1-60, S1-67: not fixed; VER3 is accepted in production)
- **Where:** `cuda/walk/kf_walk.cu:409-414` (`kf_big_pte_unmapped` returns false unless VER2),
  `:432-435`, used at `:734` and `:2074`; `:359-371` (VER3 sparse compares only `PCF == 1`);
  `crates/kf-cuda/src/walk.rs:756-763` ("KNOWN GAP"); production selects the VER3 descriptor at
  `crates/kf-qemu/src/device.rs:505`.
- **Input:** stock guest UVM on a VER3 card: big PTE `VALID=0, PCF=0x3 (NO_VALID_4KB_PAGE)` over
  sixteen stale valid 4 KiB PTEs. A compromised guest kernel is not needed.
- **Failure:** the host twin keeps translations to pages the guest kernel unmapped and may hand to
  another guest process. Intra-guest isolation (model R1.1), not a host breakout: every leaf is
  still contained. **CODE READ; UNVERIFIED on hardware.**
- **Fix:** a VER3 arm in `kf_big_pte_unmapped` (`PCF == 3`, valid clear) and in `kf_slot_sparse`
  (`PCF == 3` on the PDE's big half); the two C-suite cases S1-60/S1-67 name.

### F-2 · MEDIUM (DoS across guest processes) · One space can fail every space's walk (S1-61: not fixed; documented open, no limit in place)
- **Where:** global budget `kf_walk.cu:1952-1953`, `:2161-2162`; shared frontier `:1619`, `:1975`;
  whole report refused `crates/kf-mem/src/vasmgr.rs:432-470`; documented as open in
  `docs/design/V3_SECURITY_MODEL.md:302`.
- **Input:** a guest process whose VA layout charges more than `entry_budget` (4 194 304):
  about 7 700 PD0 slots each with a small table (544 entries charged per task).
- **Failure:** every batch containing that space is TRUNCATED and refused; other processes' maps
  stop. **CODE READ.** **Fix:** per-entry budget and caps, or re-walk without the offender.

### F-3 · MEDIUM (DoS, shared GPU, R5) · The entry budget does not bound GPU work; fan-in is untested (S1-63: not fixed; "planned, not in place")
- **Where:** `kf_walk.cu:1927-1985` (no abort check in `kf_par_expand_one`), `:2038` (compact
  skipped after abort, so the next level expands a **stale** frontier), `:2007-2025` (`*total`
  unclamped). No `FRONTIER` test in `kf_tests.cu` (grep count 0).
- **Input (VER2, guest root):** one page `T` in vidmem filled with PDEs `0x2 | ((T >> 12) << 8)`
  (aperture vidmem, pointing at itself), root = `T`, in 64 address spaces. Levels reach 131 072
  frontier entries; each level then loads 131 072 x 512 words (512 MiB of 8-byte loads) whether or
  not the budget fired. Up to 3 such levels plus the stale ones per walk; the guest re-arms a walk
  with every invalidate.
- **Failure:** the walker context holds the GPU for a bounded but large slice per walk, at the
  guest's pace. Termination holds. The time is not measured: **INFERRED** from the loop bounds.
  **Fix:** warp-uniform early exit on `walk_abort` at the top of `kf_par_expand_one`; clamp `nout`;
  the `hostile/frontier_fan_in` case of S1-63 with an event-time ceiling.

### F-4 · MEDIUM (DoS, host log flood; NEW) · One host log line per walk, at the guest's pace, with no cap
- **Where:** `crates/kf-mem/src/vasmgr.rs:485-494` (`eprintln!` whenever `header.refusals > 0`),
  `:401-403` (one line per capacity event: every ceiling and every re-walk,
  `crates/kf-cuda/src/walk.rs:1446`, `:1463`, `:1476`, `:1504`). The refusal *samples* are capped
  at 32 lines (`refusal_samples.rs:27`); these are not.
- **Input (VER2):** one big PTE `0x0000000000000101` (valid, vidmem, address field 1: GPGA 0x1000,
  not 64 KiB aligned) gives `MISALIGNED_LEAF` on every walk; then loop TLB invalidates. A foreign
  aperture PDE (`0x4`, aperture 2 = sysmem) does the same.
- **Failure:** unbounded stderr volume (disk or journal exhaustion), and a blocking write on the VA
  manager thread if stderr is a full pipe (owner rule "quiet by default on input-serving threads").
  **CODE READ**; rate not measured. I found no document planning a limit.
- **Fix:** print the first N and count the rest into the status line, as `refusal_samples` does.

### F-5 · MEDIUM (supply chain) · The shipped walker PTX is tied to its source by nothing that runs (S1-68: not fixed)
- **Where:** `crates/kf-cuda/src/walk.rs:63` (`include_bytes!`); `cuda/walk/Makefile:21-64`;
  no `cuda/walk`, `check_s39`, `make_ptx` or NVRTC step in `.github/workflows` or `scripts/ci`
  (grep); `crates/kf-cuda/tests/walk_abi_matches_the_cu.rs:845-873` compares `.param` sizes only.
- **Also:** `kf_walk.cu` was edited after the last PTX regeneration (`0adcd861a` after
  `b0db7f35b`). **MEASURED:** both sources compile to the same PTX under NVRTC 12.9, so that edit
  did not change device code.
- **Scenario:** an edit to a bounds check without regeneration, or a hand edit of the PTX, ships a
  program the suite never graded. **Fix:** CI step: `make -C cuda/walk check-invariants`, then
  regenerate with the pinned NVRTC 12.2.140 and `cmp`. Same for `cuda/display` (F-8).

### F-6 · LOW · Packed YUV 4:2:2: chroma read 1-2 bytes past the checked extent (NEW, display)
- **Where:** `cuda/display/kf_yuv.cu:93-100` (reads `cx*4+1`, `cx*4+3`); host bound
  `crates/kf-cuda/src/display.rs:532` (`x_end * 2`); same formula `crates/kf-disp/src/scanout.rs:851`.
- **Input:** a `Y8_U8__Y8_V8_N422` or `U8_Y8__V8_Y8_N422` window, pitch surface, odd
  `POINT_IN.x + SIZE_IN.width` (model example `sx0=37, sw=32, sy0=11, sh=4, pitch=202`: extent
  2966, bytes 2966 and 2967 read).
- **Failure:** a read past the context-DMA limit, still inside the store (plane starts are
  256-aligned and the store is 4-aligned). Not a breakout; it becomes one if either alignment
  changes. **MEASURED** (CPU model) and CODE READ. **Fix:** bound packed rows as
  `x_end.div_ceil(2) * 4` in both places; add the odd-width case to `tests/yuv_kernel.rs`.

### F-7 · MEDIUM (structural) · Display kernels have no in-kernel bound; HMM is not queried (S1-05: not fixed)
- **Where:** `cuda/display/kf_scanout.ptx:142` (source load), `:156-216` (frame stores): no extent
  compare; `kf_color`, `kf_yuv`, `kf_sum` likewise. No `PAGEABLE_MEMORY_ACCESS` query in
  `crates/kf-cuda/src` (grep). `compose_yuv` and `launch_compose`
  (`crates/kf-cuda/src/display.rs:942`, `:1506`) do not compare `w*h*4` with the staging length;
  the only caller pairs them correctly (`crates/kf-qemu/src/display.rs:5418-5470`).
- **Scenario:** any drift in a host formula (F-6 is one) is a GPU access to VMM address space on a
  host with pageable memory access. Not reachable today. **CODE READ.** **Fix:** pass `extent`,
  one compare per pixel; port the model as a proptest; check the staging length.

### F-8 · LOW (supply chain) · `kf_scanout.ptx` and `kf_sum.ptx` are hand-written, with no source and no gate (2026-11-13)
- No `.cu` exists in the tree, `archive/` or history. The three generated display PTX files
  regenerate **byte-identically** after the header with local clang (**MEASURED**), but nothing in
  CI checks it; their FNV header ties the `.cu`, not the PTX body.

### F-9 · LOW (defence in depth; partly NEW) · Kernel write bounds that exist only on the host
- `kf_par_seed` stores `tbl_run_count[t]` for `t < npdb` with no `KF_MAX_PDB` clamp
  (`kf_walk.cu:1795`; PTX `kf_walk.ptx:8372`). Bounded only by `walk.rs:1150`.
- `kf_commit_kernel` writes `B[nb+ex]` before the fit check (`kf_walk.cu:1358`, check `:1363`;
  S1-66, not fixed). Bounded by "the diff never emits more MAPs than the previous `walk_cap`".
- The diff reads stale walk-table runs after a truncation or abort (`:2281-2298`, `:2320`,
  `:1036`; S1-62, not fixed). Safe only because `vasmgr.rs:432` refuses the whole report.
- Two entries naming one slot would race two blocks on one `com` region; refused at `walk.rs:1161`.
- **No guest input reaches these today (CODE READ).** **Fix:** `t < KF_MAX_PDB` in seed, bound
  before write in commit, zero `tbl_run_count` on abort.

### F-10 · LOW (latent; NEW) · The shared-memory snapshot is smaller than the geometry the descriptor may declare, and production has no geometry check
- **Where:** `KF_SHWORDS = 576` words per warp (`kf_walk.cu:1624`); the expand loads
  `i < KF_SHWORDS` (`:1956`) but decodes `sh[2*i+1]` for `i < entries <= 512` (`:1837`, `:1872`);
  the leaf's big table has 64 words (`:2248`) for `big_entries <= 512` (`:2168`).
  `kf_format_check` (`:1517-1553`) is host code outside `KF_DEVICE_ONLY`, so it is **not** in the
  PTX, and `WalkKernel::bring_up_on` checks only `abi_version` (`walk.rs:732`).
- **Scenario:** a future family row with a dual level above 288 entries, or a big table above 64,
  makes a warp decode its neighbour's snapshot (another table, possibly another address space) and
  read past the block's shared allocation. Both shipped descriptors are inside (256 dual, 32 big),
  and the descriptor is host-authored, never guest data. **Fix:** port `kf_format_check` to Rust
  with `entries * entry_bytes / 8 <= KF_SHWORDS` and `big_entries <= 64`; `static_assert` in the `.cu`.

### F-11 · LOW · Test gaps (S1-64, S1-65, S1-69: not fixed)
- Race tests run with an unbounded span, so they never assert containment
  (`cuda/walk/kf_tests.cu:135`; only `:1375-1436`, `:1834` use `cfg_span`).
- No hostile case reaches the production launch path (`walk.rs:1752-1938`), and there is no CPU
  model of the walk decode; `crates/kf-cuda/tests` has no hostile or property test of it.
- `Report::validate` (`walk.rs:360-434`) does not check op, page-size code or alignment.

### F-12 · INFO · Fault handling
- A stream failure surfaces at `cuEventQuery` and latches `completion_failed` (`walk.rs:1368-1372`,
  `:1138`); the report is not read and every later submit is refused. No error is turned into
  success (**CODE READ**). There is no path that recreates the context, so a faulted walker means a
  dead guest GPU until the VM restarts (guest-only effect).
- Walker and display use separate contexts per device (`walk.rs:782`, `display.rs:578`). Whether
  a device fault in one poisons the other in the same process is **INFERRED** from CUDA's
  documented sticky errors, not observed.

## 3. Answers to the questions

**(a) Loads, stores, atomics.** The only guest memory access is `KF_GPGA_DEREF` in `kf_win_load`
(`kf_walk.cu:463-470`), guarded by `off > len || 8 > len - off`. In the PTX: serial walk
`kf_walk.ptx:508-516` and five more sites; expand `:8691-8700`, `:8759-8803` (unrolled, with a
running remainder); leaf `:10368-10376`, `:10410-10418`. Roots: `.cu:1803-1804`, PTX `:8383-8387`
(alignment) and `:8404-8412` (extent). Table pointers: `kf_win_table_ok` `.cu:472-479`, called at
`:1861`, `:1886`, `:1898`. Leaves: containment before coalescing at `.cu:549-557` and `:1730-1738`
(PTX refusal sites `:10713`, `:11033`, `:12040`). Vidmem leaves are bounded to the store span, PEER
is refused, sysmem leaves are passed through for the host to bound against guest RAM
(`crates/kf-mem`, not in this scope). All writes go to walker pools, bounded as listed in H2/F-9.
The pinned report is written at `.cu:1254`, `:1259`, `:1269`, bounded by `run_capacity` and
`pdb_capacity`, matching `PinLayout` (`walk.rs:487-512`). **A crafted PTE cannot move a read
outside the store or a write outside the walker's buffers.**

**(b) Bounds.** Depth is the host launch loop (`walk.rs:1786`, at most 5). Fan-out is capped at
512 per table and 131 072 per level; scratch at 4 Mi runs. A self-pointing PDE only fills the
frontier. Grid-stride indices are `u32` and stay below 2^27; `total` in `kf_diff_emit` is below
2^21 with the default pools (`64 x 2 x walk_cap`, pool 2^20), so it cannot wrap. Output capacity:
`.cu:1217` (report), `:2290` (per entry), `:1131` (slot). No watchdog exists and none is needed
for termination; the work bound is F-3.

**(c) Races.** Counters use atomics; output order comes from scans, not from the atomics. The two
"store then add" joins are separated by `__syncwarp` (`.cu:2225-2229`) or a kernel boundary
(`kf_par_join`). `__syncthreads` in the diff and commit is reached by all threads on every path
that does not return for the whole block (the early returns depend only on block-uniform values).
Guest writes during a walk: each table is read once into shared memory, both sweeps read that
copy, and the containment test runs on register copies. The host re-validates the report
(`vasmgr.rs:428-432`).

**(d) PTX vs source.** §5.

**(e) Host side.** `npdb <= 64`, slots in range and distinct (`walk.rs:1150-1170`); one walk in
flight (`:1144`); store pointer from `cuMemImport` + `cuMemMap` with the same length RM reserved
(`device.rs:538-563`, `driver_unsafe.rs:1745-1796`); store accesses bounded by `store_at`
(`walk.rs:2023-2035`); collect clamps counts to capacities (`:1387-1388`). The store is mapped
read-write into the walker context although the walk only reads (needed by `write_store` and the
scrub).

**(f) Leaks.** Every device buffer is zero-filled at allocation (`walk.rs:833-837`), the pinned
buffer too (`:896`). Report structs have no padding; the header is `memset`. The report goes to
the VMM only and carries store offsets, never host addresses. Refusal samples carry the guest's
own entry. The store itself is not scrubbed at VM start (owner-accepted premise,
`crates/kf-qemu/src/scrub.rs:12-16`, S1-31, UNVERIFIED).

**(g) Corpus and `check_s39.py`.** `check_s39.py` is three text checks on the `.cu`: distinct load
offsets, containment before coalesce, refusal before return. It does not look at the PTX and runs
only by hand. `corpus/` is a differential corpus of benign and a few hostile VER2 images plus a
real GA106 capture (263 leaves), compared on a GPU box; it is not a fuzz corpus. The C suite has
26 hostile and 4 race cases and needs a GPU.

Smallest additions (GPU-free unless marked):
1. `hostile/frontier_fan_in` (F-3 pattern): assert `FRONTIER_CAP`, zero runs, and an event-time
   ceiling (GPU).
2. `hostile/seed_npdb_65` against the kernel with a canary after `tbl_run_count` (F-9; GPU).
3. VER3 `PCF=3` big PTE over sixteen valid small PTEs, and PDE big half `PCF=3` (F-1; GPU, and a
   Rust decode-model test).
4. Race tests with `cfg_span(f.g.size())` and a `KF_BREAK_BOUNDS` racer (S1-64; GPU).
5. A Rust test that refuses a `KfFormat` with a 512-entry dual level or 128 big entries (F-10).
6. A vasmgr `ModelWalker` test: 10 000 reports with `refusals = 1` produce a bounded number of
   log lines (F-4).
7. `yuv_kernel.rs`: packed layouts with odd `sx0 + sw` (F-6).
8. CI: regenerate and `cmp` all PTX; PTX-level guard scan (`ptx_guard.py` is a start) (F-5, F-8).

## 4. False positives (checked, safe)

- **`runstage[st + excl - 1]` underflow** (`.cu:2228`): `excl >= 1` whenever `head == 0`, because
  the first lane with a run always has `head == 1`.
- **`total` wrap in `kf_diff_emit`** making `base + k` exceed the report: needs 2^32 staged runs;
  the pool caps it at 2^21.
- **Missing atomics in the committed PTX** (0 `atom.` in `kf_par_seed`): NVRTC 12.2 emits
  `red.global.*` for result-unused atomics; `atom + red` equals the 12.9 rebuild on every entry.
- **Stale frontier entries dereferenced after an abort:** every load is re-checked against the
  window, so a stale address cannot leave the store.
- **Compose kernel 32-bit wraps** (`kf_scanout.ptx:86`, `:117`, `:122`): only for field widths the
  planner cannot produce, and a wrap lands inside the extent.
- **Leaves into the firmware carve-out:** refused for host GPU VA spaces by default
  (`crates/kf-qemu/src/tspace.rs:95-97`, `vasmgr.rs:952`); the kernel itself bounds to the whole store.

## 5. PTX against source

- Committed `cuda/walk/kf_walk.ptx`: sha256 `33c1e5a8...`, header "release 12.2, V12.2.140",
  `.version 8.2`, `.target sm_75`, 14 entries. The stage-1 audit's byte-identical rebuild was at
  `12a526df` with a different hash; the PTX has been regenerated twice since.
- NVRTC 12.2.140 is not installed here (12.9.86 and 13.0 are), and installing was out of bounds.
  **Byte identity is UNVERIFIED.**
- **MEASURED** with NVRTC 12.9.86 into the notes directory: the 14 entry names and `.param` lists
  are identical; per entry, `ld.global`, `ld.volatile.global`, `st.shared` and `atom + red` counts
  are identical, `st.global` differs by one in `kf_diff_emit` (69/70), `setp` by up to 7.
- Makefile flags (`nvcc -O2 -gencode arch=compute_75`) build the test binary, not the shipped PTX;
  the shipped PTX comes from `make_ptx.py` (`-DKF_DEVICE_ONLY=1 -default-device --std=c++14`).
- Display: three generated files reproduce byte for byte after the header; two are hand-written.

## 6. Owner points and earlier review items

| Point | Where it applies here | Status | Evidence |
|---|---|---|---|
| Hostile guest; guest-reachable unbounded read is a bug (CLAUDE.md) | walker loads, display reads | HOLDS (walker); VIOLATED by 2 bytes inside the store (YUV) | H1; F-6 |
| Nothing blocks on a vCPU | walk submit/collect | HOLDS | `walk.rs:1108-1235` queues only; `try_collect` uses `cuEventQuery` (`:1362`) |
| Input-serving threads never block; quiet by default (owner 2026-10-09) | VA manager logging | VIOLATED | F-4 |
| No CPU executor for GPU work | walker | HOLDS | no CPU decode of tables on the walk path; `refusal_dump` is a measurement flag (`vasmgr.rs:409`) |
| Measurement flags never required (owner 2026-10-09) | `KF_VAS_CENSUS`, `KF3_WALK_REFUSAL_DUMP` | HOLDS | `vasmgr.rs:409`, `:448`, `:471` gate reads only |
| Unsafe only in `*_unsafe.rs` | kf-cuda | UNVERIFIED (not swept) | — |
| §20 I1 no loop on guest data | `.cu` | HOLDS | `make check-invariants` run here |
| §20 I2 bounds before every dereference | `.cu` + PTX | HOLDS | H1 |
| §20 I3 loud truncation | `.cu`, vasmgr | HOLDS | `.cu:1217-1227`, `vasmgr.rs:432` |
| §20 placement in an unprivileged isolate | walker runs in the VMM | VIOLATED (stale text, S1-26 open) | `device.rs:527-536` |
| §21 one program, format as data, skew fails loudly | abi gate | HOLDS for `abi_version`; geometry unchecked | `walk.rs:732`; F-10 |
| §39(a) copy, check, use | loads | HOLDS | `check_s39.py` (a) passes |
| §39(c) containment before coalesce; span vs window | emit sites | HOLDS | `.cu:549`, `:1730` |
| §39(e) refusal dominates every return | leaf, expand | HOLDS | `.cu:2143`, `:2213` |
| Owner §Q: host addresses reach a GPU only in kayfabe's contexts; kernel bounds are host-safety obligations with tests that can fail | walker, compose | PARTIAL | walker has a known-positive that needs a GPU; compose has none in-kernel (F-7) |
| No guest value used as a device pointer | launches | HOLDS | `walk.rs:1049-1094`: all pointers are walker allocations or the import |
| R5.1 no guest-steered access to other tenants | kernels | HOLDS | §3(a) |
| R5.2 shared-resource exhaustion bounded per VM | walk work, logs | VIOLATED | F-2, F-3, F-4 |
| R5.3 residual VRAM scrubbed | store, walker pools | pools HOLD; store UNVERIFIED (S1-31) | `walk.rs:835`; `scrub.rs:12-16` |
| 2026-10-02: no MMIO CPU reads of guest tables | walker | HOLDS | tables are read only by the kernel |
| Passthrough spaces hold only guest PT leaves (2026-10-10) | report content | NOT-APPLICABLE here (kf-mem applies) | — |
| Unchanged VAs stay mapped during refresh | diff keeps covered placements | HOLDS in the kernel | `.cu:947-971`, `:1158` (only non-kept placements are unmapped) |
| Single-channel fault containment (2026-10-22) | walker fault | UNVERIFIED | F-12 |
| Derive, never capture | format descriptors | HOLDS | two family rows, `abi.rs:940`, `:1078` |
| RE boundary / provenance | cuda/ | HOLDS | citations are to ogkm; nothing else seen |
| S1-05 | compose bounds, HMM query | NOT FIXED | F-7 |
| S1-22 CUDA contexts de-privileged | bring-up threads | narrowed: `on_cuda_thread` drops CAP_SYS_ADMIN (`device.rs:522-527`); not re-audited | — |
| S1-26, S1-27 | documents | NOT FIXED (per audit plan table) | `V3_SECURITY_AUDIT_PLAN.md:198-199` |
| S1-60 / S1-67 | VER3 vetoes | NOT FIXED | F-1 |
| S1-61 | shared budget | NOT FIXED | F-2 |
| S1-62 | stale runs after truncation | NOT FIXED | `.cu:2281-2298` |
| S1-63 | budget is not a work bound | NOT FIXED | F-3 |
| S1-64 | race tests without containment | NOT FIXED | `kf_tests.cu:135` |
| S1-65 | no hostile test on the production path | NOT FIXED | F-11 |
| S1-66 | commit writes before the check | NOT FIXED (an earlier `o > pcap` guard was added at `.cu:1318`; the `B` write is unchanged) | `.cu:1358-1363` |
| S1-68 | gates not in CI | NOT FIXED | F-5 |
| S1-69 | weaker Rust validator | NOT FIXED | `walk.rs:360-434` |

No regression of a previously fixed item was found.

## 7. Not reviewed

- `crates/kf-mem` apply/ledger (sysmem leaf bounds, carve-out policy, PRIV withholding) beyond the
  report gate; `crates/kf-chan`; the QEMU C device; `broker/`.
- `kf_tests.cu`, `kf_bench.cu`, `kf_real_tables.py`, `kf_report_emit.c`, `run_on_box.sh` line by
  line (case names and span configuration only).
- `crates/kf-cuda`: `diffmodel.rs`, `synth.rs`, `selftest.rs`, `posture.rs`, graph capture details
  in `driver_unsafe.rs` (parameter update correctness was read, lifetime handling was not).
- `kf-disp/src/color.rs`, frame and slot lifetime in `kf-qemu/src/display.rs`, the console
  reader of pinned frames (`kf3.c`).
- The 5 000 lines of generated colour and YUV PTX (covered by byte-identical regeneration).
- VER3 decode correctness beyond the two veto gaps.
- The frozen `walk-v2-frozen` kernel's content (only that v3 does not load it).

## 8. What a GPU run would add

- The 58-case suite and `check-negative` on the current PTX, and the cases in §3(g).
- The time of the worst-case fan-in walk (F-3) and its effect on a second tenant.
- `CU_DEVICE_ATTRIBUTE_PAGEABLE_MEMORY_ACCESS` on the target hosts (F-7).
- What a walker fault does to the display context in the same process (F-12).
- JIT of the committed PTX against a 12.2 rebuild on the same card (F-5).
