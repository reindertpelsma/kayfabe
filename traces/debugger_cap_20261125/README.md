# Debugger sessions on host RM: cost, refusal and a cross-VM cap, 2026-11-25

**STATUS: RESEARCH, 2026-11-25.** Hypotheses, falsifiers and the cap-derivation rule were committed before the run
(`PREREGISTRATION.md`, commit `c17445deb`). MEASURED = a file under `run/`; INFERRED = reasoning, said so.

Run: Vast instance 62458542 (RTX 4090, 24564 MiB, 26 vCPUs, 49 GB, nested KVM, kernel 6.8.0-59), host driver **580.159.04**
(open module, stock tier), idle GPU (FB used 0 MiB at every arm start). Tool: `kf-debugger-cap-probe` and
`scripts/bench/debugger_cap_arms.py` at revision `71614d548` (the tree was sent as a tarball; the box clone is not a git checkout).
Host only, no VM: each process is one host RM client, device and subdevice, as kf3 holds per VM; sessions are made by
`HostRm::alloc_debugger`, the call kf3 makes. Files: `run/run1_a1a2_a3crash/` (a1, a2; its a3 was lost to a controller bug, an
`nvidia-smi` failure, fixed before `run/run2/`), `run/run2/` (a3, a4, a5, k1, k2), `run/run3/` (x1), `run/box_dmesg_nvrm.txt`
(every NVRM line of the box's life, scrubbed), per arm `.log` (every worker line), `_timeline.tsv`, `_dmesg.txt`, `.json`.

## Result (MEASURED, 2026-11-25, box 62458542, revision `71614d548`)

| arm | procs x GR | sessions reached | refusal | per process | FB used: before / after GR setup / peak | after freeing the sessions | after exit |
|---|---|---|---|---|---|---|---|
| a1 | 1 x 1 | **11734** | `NoMemory` (0x1a or 0x51) | 11734 | 0 / 5 / 5 MiB | 5 (= setup) | 0 |
| a2 | 1 x 4 | 11719 | 4 of 4 GR objects: `NV_ERR_INVALID_STATE` (0x40) after the first | 2929-2930 each | 0 / 18 / 18 | 18 | 0 |
| a3 | 1 x 16 | 11659 | `NoMemory`, all 16 objects | 728-729 each | 0 / 72 / 72 | 72 | 0 |
| a4 | 4 x 1 | **11718** (all 4 processes refused) | `NoMemory` | 2885-2961 | 0 / 18 / 18 | 18 | 0 |
| a5 | 16 x 1 | **11652** (all 16 refused) | `NoMemory` | 680-799 | 0 / 72 / 72 | 72 | 0 |
| k1 | 1 x 64, at most 4 per GR | 256 (all admitted) | none | 256 | 0 / 286 / 286 | 286 | 0 |
| k2 | 1 x 448, at most 4 per GR | 1792 (all admitted) | none | 1792 | 0 / 2001 / 2001 | 2001 | 0 |
| x1 | a1, then a second process while a1 holds | 11734; the second process built **0 of 1** GR objects | its VA-space alloc: `NoMemory` | | | | 0 |

- No Xid in any arm (`new_xids` 0). Stops: host RM refused every GR object (a1-a5, x1); k1/k2 ended at their per-GR limit.
  Neither the 20000-session ceiling nor the 4 GiB FB limit was reached.
- The refusal is `GspRmAlloc failed ... hClass=0x83de ... status=0x1a` in dmesg (`NV_ERR_INSUFFICIENT_RESOURCES` from GSP).
  a2's later objects read 0x40, after the first refusal.
- Time per alloc: mean 0.79 ms (p99 1.3 ms) for one process; with 4 / 16 processes 3.2 / 13 ms (the host RM lock serialises them).
  Freeing 11.7k sessions took 12 s (one process), 14 s (4), 18 s (16).
- Host kernel memory (`/proc/meminfo` `SUnreclaim`, start to max): +111.6 MB for a1, **about 9.7 KB per session**
  (a2 9.7, a3 9.8, a4 9.7, a5 9.6 KB). k1/k2 include their GR contexts (GR object setup alone is about 4.5 MiB of FB each:
  286 MiB / 64, 2001 MiB / 448). The worker's RSS stayed 2.7-3.1 MB.
- Leak check: FB after freeing every session equals the value after GR setup, and after the processes exit it is 0 MiB again, in all
  arms: **no FB leak**. Not clean: while freeing, host RM logs `nvAssertFailedNoLog: Assertion failed:
  listCount(&pKernelGraphicsContextShared->activeDebuggers) == 0 @ kernel_graphics_context.c:3571` (3 lines in a3 and x1, none
  in a1/a2/a4/a5/k1/k2 in the arm's own dmesg slice) and once `pGpu->numSubdeviceBackReferences == 0 @ gpu.c:1601`
  (`a3_dmesg.txt`, `x1_dmesg.txt`). Free order and cause not investigated.

## Verdicts
- **H1 (host refuses by itself at a comparable number): HOLDS, in a different form.** The limit is real and **global**:
  about 11.7k sessions in total, whether one process makes them over 1, 4 or 16 GR objects, or 4 or 16 processes share the work
  (a1 11734, a4 11718, a5 11652: the processes of a4/a5 split one budget, not one each). It is not per client, not per GR object.
  The per-VM budget x VMs is already past it for 7 VMs at the 1789 default.
- **H2 (tens of KiB of host FB per session): FALSIFIED.** FB growth per session is below the 1 MiB resolution of `nvidia-smi`
  (peak equals the after-setup value over 11.7k sessions, so under 0.1 KiB). The cost is host kernel memory, about 9.7 KB per
  session (run2, 2026-11-25, revision `71614d548`), plus the GSP side (not observable from the box; INFERRED to be what is exhausted).
- **H3 (leak): FB part HOLDS** (no FB leak); the host assertions above are a finding.
- **H4 (global): HOLDS** (a4/a5 sum to the a1 count within 1 percent).
- **Denial of service (not preregistered, MEASURED once, x1):** with one client holding the 11.7k sessions, a fresh process could
  not allocate even a VA space (`NoMemory`). One hostile VM that reaches the global limit therefore blocks every other client of
  the die from building GR work until it ends. Which other allocations fail was not tested (the dmesg shows only the 0x83de refusal).

## Recommended global cap (the owner decides; derived by the rule fixed in `PREREGISTRATION.md`)
R = 11652 (the smallest global total, a5). **Cap = floor(R / 4) rounded down to 64 = 2880 host debugger sessions per GPU, across
all VMs.** At the default channel budget (1789) that allows **1 VM** by budget (2880 / 1789); at the 256 minimum, **11 VMs**.
Headroom is 4x. Reading for the decision (INFERRED): a per-VM budget of 1789 is also the wrong size for sessions, because real use
is one per CUDA context (`docs/design/V3_GUEST_INFLUENCE_AUDIT.md`: 145 traces), and a policy-compliant VM at the default budget (k2:
1792 sessions over 448 GR objects) costs 2 GiB of FB for the GR contexts and 15 percent of the global limit; a separate, smaller
per-VM debugger cap (for example 256, 11 VMs at 2880) would fit the shared limit with room for the host's own users.

## Cost and teardown
Instance 62458542 (label `kf-dbgcap-1`), rented about 03:55 UTC and destroyed by its id at about 04:15 UTC (about 20 minutes, 0.456 USD/h,
about 0.2 USD); `vastai show instances` afterwards lists no id of ours. `vast_guard.py --enforce` before and after: OK. Scrubbed: no GPU
UUID, PCI address, box IP or serial is in these files (`run/` was grepped; dmesg lines pass through `scrub`).
