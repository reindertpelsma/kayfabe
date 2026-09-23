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
