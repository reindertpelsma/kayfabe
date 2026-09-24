# OVERNIGHT NOTES — w825 (2026-09-23/24)

STATUS: LIVE — the running log of the overnight autonomy session. Box 52236011 (RTX 3060,
GA106, 580.159.04). Baseline `w825base`: thin guest 17/30 (honest 15/30).

## Measured and fixed

1. **The doorbell ledger never saw a WORKER forward** (`c61f4a49`). A deferred doorbell is
   recorded on the trap as `Scheduled ⇒ emulated`; the worker then stored it to hardware
   (`DOORBELL-STORE … WROTE`) and nothing updated the ledger. Five arms whose client said
   `ALL ARMS MET rc=0` were graded FAIL "never reached hardware": `blockage-coverage`,
   `late-map-race`, `executor-vas`, `dictated-ring`, `ce-client`. **All five now PASS**
   (w825b), each token `forwarded>0`.
   ⇒ The `forwarded=0` cluster was a GRADER defect, not a data-path one. `route_of_engine`
   (`CpuCe` for CE) is NOT on the user-channel path: user CE channels are `Passthrough` and
   forward through the store (`STORE-BIRTH … BORN IN B`).
2. **REAL cross-client leak, two layers** (`eaf05f50`, `4bc8e7be`). Per-proc isolates and
   per-proc birth clients are separate RM clients, so handle NUMBERS repeat. `StoreMapPort::
   adopted` keyed by `.raw()`, and every `BirthConn` minted from `0xB147_0000`; proc 2's
   channel ran in proc 1's host VAS and A's copy landed in B's object. Keyed by the whole
   handle + one process-wide birth-handle counter. `cross-client-leak` **PASS** (w825d),
   distinct ranges `0xb1470003` / `0xb147000a`.

3b. **Full suite at `4bc8e7be`: 22/30** (from 17, honest 15). Every remaining failure is a
   timeout except `rpc-mixed-allocs`.
4b. **Guest RAM as ONE RM object — BUILT** (`a0e43134`, `d622fd2c`): the scratchpad maps the
   whole memfd as one `OS_DESCRIPTOR` (2 GiB in ~1.5 s) and sysmem rows are FIXED slices of it
   at the guest's VA; a slice the guest's rows no longer back is unmapped (handle-ledger diff).
   The per-page pin path is skipped on the K arm. `engines` **PASS** (was a 17 s worker grind).
5b. **The raw client raced itself** (`88b628bc`): `w381_engine_write` reused one staging word
   without waiting for its copy to retire. `rpc-mixed-allocs` **PASS**, identity 12/12,
   ordering 6/6. ⚠ A CLIENT fix — the guest's deferred first doorbells exposed it.
6b. **The invalidate hold** (the guest spins on TRIGGER for all of it) was ~15 ms per map:
   BAR-mirror revalidation on EVERY invalidate (5.3 ms), BAR premap on every invalidate
   (6.8 ms), PT refresh (2.6 ms). Scoped both BAR phases to BAR-relevant invalidates
   (`4d3d10d6`, `89026348`) and memoised page-table reads within a revalidation pass
   (`47029f0c`). ⚠ `refresh` (the CPU walk of user page tables through vidmem device views)
   is what remains — that is §9 step 4 (the GPU walker) and is the next build.

★★★ **Full suite at `47029f0c`: 26/30, FAIL=0** (w825i). Remaining four are all TIMEOUTs —
`concurrency`, `ce-client-guest-ram`, `gpga-reserve-probe`, `concurrent-fuzz` — and all four are
bound by the invalidate hold's CPU page-table sweep (§ step 4) or, for `gpga-reserve-probe`,
by CPU reads of vidmem through BAR1 device views (by construction).

⊘ **Refuted, measured:** SSE4.1 streaming loads (`MOVNTDQA`) on the device view — FB-IO-RATE
24.94 vs 24.95 MB/s on AMD Zen 2. The CPU read rate of vidmem is not a load-instruction
problem; it stays ~25 MB/s. ⇒ The sweep's cost can only fall by reading LESS (step 4, the GPU
walker), not by reading faster.

## Diagnosed, in progress

3. **Guest RAM (sysmem) is never mapped into the host VAS on the K arm** — v3's second
   ground truth is not built on the live path. Every sysmem row publish is refused
   (`map_gpu_va` refuses a bare space). Causes `rpc-mixed-allocs`, `ce-client-guest-ram`,
   and — through the per-page pin path re-trying every sysmem row on every publish
   (~99k worker requests, ~17 s) — the `engines`/`concurrency` stalls. Being built: one
   OS_DESCRIPTOR over guest RAM in the scratchpad, sysmem rows FIXED-mapped as slices.
4. `concurrent-fuzz`: no longer wedges; RM verb p50 163 ms ⇒ budget-bound.
5. `defer-liveness`: the rung PASSes at 34 s; the guest then hangs in close past 45 s.
6. `gpga-reserve-probe`: a 256 MiB CPU read sweep through BAR1 device views (~24 MB/s) —
   performance-bound by construction.

## Step 4 (walk-at-invalidate on the GPU) — mapped, NOT built tonight, and why

The seam exists (`PtSweepDecider::decide`, `kayfabe-rt/src/device.rs:9175`), but the in-place
GPU walk cannot replace the CPU sweep as-is:
- the kernel's report is **coalesced runs** — no table pages, levels, child edges or sparse
  slots — so the `SubtreeDecode` every downstream consumer reads (`ReachShadow::observe` →
  `settle` → `AddressTable::bind`) cannot be rebuilt from it;
- four wiring defects on the in-place path: `cudawalk::run` refuses without a staged image it
  then ignores (`cudawalk.rs:288`); the decider sends relocated roots, not real PDBs
  (`walkshadow.rs:332`); `arm_store_device_pointer` imports the store in a DIFFERENT CUDA
  context (`rm.rs:10535`, the kernel cannot use it — `walk.rs:669`); a foreign-aperture table
  refuses the whole report instead of one branch (`kf_walk.cu:512`);
- ⚠ **and it would not be faster yet**: Rust launches only the SERIAL kernel (1.6–4.5 ms),
  vs today's CPU sweep ≈ 3.5 ms per invalidate. Only the parallel kernel (205 µs) wins, and no
  Rust code calls `kf_run_parallel`.
⇒ Plan (8 steps, file:line anchored) is in the session log; the order is: page-record section
in the kernel report → per-branch foreign-aperture → same-context import → real-PDB verb →
`subtree_from_report` → decider-first execute → wire + keep the Shadow arm as a field-by-field
check → parallel launches. **Owner question:** is it acceptable to keep the CPU sweep (reads of
vidmem PT pages through device views) on the hold until that lands, given v3 declares CPU reads
of vidmem dead? Tonight's work kept it and cut everything around it instead.

## CUDA ladder (`cup3`, full guest via `boot_capture.sh` + `cup3_hook.sh`) — progress, in order

Run with `NVKVM_RAM_BACKEND=memfd NVKVM_RAM_MB=4096 KAYFABE_CUP3_TIMEOUT=120`. Each step was
measured on box 52236011 and each fix is its own commit:
1. `cuInit` hung in `UVM_REGISTER_GPU` (never `CTX OK`). Two defects on the CPU-CE path for
   kernel/UVM channels: a new channel inherited a dead one's GPFIFO cursor (`6edddcfd`), and an
   entry below `GP_PUT` not yet VISIBLE refused the doorbell forever (`d930d4a8`). ⇒ **`cuInit`
   completes.**
2. `cuCtxCreate`'s GR channels were refused birth: libcuda declares engineType 0 (NULL) and a
   re-bind of the identical store slice was refused as "taken" (`7ff4cf12`).
3. Then `NoMemory` on the 3rd GR birth: ⚠⚠ **the GPGA reservation (11760 MiB of 12 GiB)
   leaves host RM no vidmem for per-channel GR context.** Measured with
   `KAYFABE_SCRATCHPAD_START_MB=10240`: all 16 births succeed. **OWNER QUESTION — sizing
   policy**: the reservation must leave headroom proportional to the channels the guest can
   create; today it takes the maximum. 10 GiB is a measured working value, not a policy.
4. CE channels declared NULL inside a CE TSG were born as GR — host RM refused their copy
   object (runlist conflict). Now born on the TSG's engine (`6f6d87e4`).
5. **Current wall:** `cuCtxCreate → 999`, host `Xid 31 CE2_PBDMA0 FAULT_PTE @ 0x2_0440f000`
   (virt read — a pushbuffer fetch). That page is the native oracle's completion page
   (`0x2_0440_fff0`, host RAM). All 12 801 guest-RAM rows of the process's VAS are slices
   by then, so either the page is not a sysmem row we see or it is used before its row is
   published. A `KAYFABE_PROBE_VA` run is in flight to say which.
