# SPR2: hardware validation of selective sparse twin mappings

STATUS: LIVE, 2026-10-26. Text evidence of task 4 of lane SPR2 (continuation of SPR1, whose tasks 1-3 are in
`traces/sparse_spr1_20261018/`). Host: RTX 4070 (AD104), host driver 595.91.07, stock host tier. Revision under test
`0053c83b4` (branch `claude/spr2-20261026`); the QEMU binary built from it has SHA-256 in `binary-sha256.txt`. The host lock
was taken only through `flock -n` polled every 300 s, never broken, and every VM was stopped by its own exit or QMP
`system_powerdown`. Labels: MEASURED (a file named in the same sentence), CODE READ, INFERRED.

## Linux guest reproducer (Vulkan sparse bind, unbind, CE write into the unbound tile)

MEASURED on 2026-10-26 (RTX 4070, rev `0053c83b4`) in `h6-final-off-sparse-1.txt` and `h6-final-off-sparse-2.txt` (flag OFF, 2 boots): host Xid 31 `FAULT_PTE
VIRT_WRITE` from copy engine CE3 at the unbound tile, and the guest sees `VK_ERROR_DEVICE_LOST`. This is the product
behaviour before this work, identical in both boots.

MEASURED on 2026-10-26 (RTX 4070, rev `0053c83b4`) in `h6-final-on-full-1.txt`, `h6-final-on-full-2.txt`, `h6-final-on-full-3.txt` (flag ON, 3 boots, 300 iterations each
for the sequential and the chained submit): `ABSORBED_ZERO_NO_FAULT`, unbound reads 16384/16384 zero words, the write is not
stored (`writes_landed_in_memory=0`), 0 host Xids, no `DEVICE_LOST`, as on bare metal (experiment E3 of branch `claude/sparse-hw-20261026`).

## Negative control (plain invalid VA must still fault)

MEASURED on 2026-10-26 (RTX 4070, rev `0053c83b4`) in `h6-final-on-invalid-1.txt` and `h6-final-on-invalid-2.txt` (flag ON) and `h6-final-off-invalid-1.txt`,
`h6-final-off-invalid-2.txt` (flag OFF): a wild CUDA store at `0x20_00000000` raises Xid 31 `FAULT_PDE VIRT_WRITE` in all
4 boots, CUDA reports `LAUNCH_FAILED`. The selective path does not absorb a plain-invalid access (the blanket
`KF3_DIAG_SPARSE_RESV=1` did).

## Native gates

MEASURED on 2026-10-26 (RTX 4070, rev `0053c83b4`) in `gates-flag0.txt` (flag OFF) and `gates-flag1.txt` (flag ON): `V3_GATES_SUMMARY pass=9 fail=0 gate10=PASS`, 12 USER
births, 0 refusals, 0 Xids in the bounded kernel-journal interval. Gate 9 contains the sparse fixtures for small and big
leaves on VER2 and VER3 with the switch OFF and ON.

MEASURED on 2026-10-26 in `gate4-before-fix.txt` (RTX 4070, SPR1 build at `162a72a6a`, flag OFF): `kf-gate4` FAILed
`teardown_is_ranges_and_frees_the_batch` with `frees=0`, so the failure is independent of the flag. Cause (CODE READ,
`crates/kf-harness/src/publish.rs`): the FX1 deferred-free merge made batch frees wait for `BatchedVas::invalidate`, but the
gate wrapper `Batching::invalidate` called the inner space directly and `Recorded` did not forward `pending_invalidate`.
Fixed by `0053c83b4` with 3 GPU-free tests and 2 mutants, each killed by at least 2 tests; the gate passes in `gates-flag0.txt` and
`gates-flag1.txt`.

## Fast suite (30 arms, one boot each)

MEASURED on 2026-10-26 (RTX 4070, rev `0053c83b4`) in `fast-suite-flag0.txt` (OFF) and `fast-suite-flag1.txt` (ON): `FAST_SUITE_PASS=30 FAIL=0 CRASH=0 NOTRUN=0` in both.
`fast-xid-census.txt`: the only Xids are the declared fault primes of `--missing-page-fault` (1) and `--defer-liveness` (4),
0 unexpected, in both flag states. The budget wrapper allows one Xid per declared `FAULT_MARK`, plus one more at the placed
VA when the defer-liveness rung itself prints `copy=DEAD` (its test channel is killed by design; CODE READ
`crates/kayfabe-rm-ladder/src/main.rs`, rung `defer_liveness`). The first OFF attempt stopped at that arm because the wrapper
had no allowance for it (arm PASS, wrapper exit 51); the wrapper was amended and the suite rerun from the start.

MEASURED on 2026-10-26 (RTX 4070, rev `0053c83b4`) in `fast-flag1-sparse-counters.txt` (ON): every `kf-host: SPARSE` line has `refused=0 overflow=0`. The reservation
objects made per session are about 8000 (4000 live), of the 32768 session cap (see the open question in the report).
MEASURED on 2026-10-26 (RTX 4070, rev `0053c83b4`) in `h6-final-on-full-*.txt`: the reproducer boots end with `refused=0 overflow=0 unhonoured_bytes=131072`; the 128 KiB
is sparse intent inside a populated normal block, the counted closed fallback (it stays invalid).

## Not covered

No Windows run (follow-up, owner's one-boot SPEC sequence). Shader (SM) loads through a sparse tile are untested; the CE
data-channel check E2 is bare metal only. The committed PTX was built by SPR1 on the host (SHA-256 equal to the host copy);
SPR2 did not rebuild it.

## Round 2 (2026-10-26): plain maps are never refused (owner ruling), re-validated on Linux

Trigger: the first Windows campaign at `a5d709d5` (flag ON) failed 3 of 3 boots at the start of `catia-07` with a refused map
followed by Xid 31 (coordinator report, evidence on the host in the coordinator's campaign directory, not in this
repository). Cause (CODE READ, `crates/kf-host/src/sparse.rs`, rev `a5d709d5`): the map path ran `Reservations::normal()` and
refused the map when that refinement was declined (per-space object budget spent, a populated coarse block, an RM refusal).
The same declines also incremented `overflow` once per attempt, which is why it read 8.3 M.

Fix at `59f09133d`: `Reservations::route_map` makes refinement best-effort and places the map in the coarse block as with the
twin OFF; `overflow` counts declined SPARSE admissions only (a declined admission is now unhonoured, not an error); the new
`coarse_maps` counts plain maps placed unrefined; micro reservations no longer count against the session cap; routing
survives a poisoned layout. Tests: EXPECTED RED `118cdbeb2` (12 of 130 kf-host unit tests fail), green at `59f09133d`
(143 tests after the follow-up commit). `plain-map-mutations.json`: 13 mutants, each killed by at least 2 tests.

MEASURED on 2026-10-26 (RTX 4070, rev `59f09133d`, files in `round2/`): `gates-flag0.txt` and `gates-flag1.txt` 9/9 with 0 Xids;
`h6-r2-off-sparse-*.txt` Xid 31 and `DEVICE_LOST` (2 of 2), `h6-r2-on-full-*.txt` absorbed with 0 Xids (3 of 3),
`h6-r2-on-invalid-*.txt` and `h6-r2-off-invalid-*.txt` the plain-invalid control faults (4 of 4);
`fast-suite-flag0.txt` and `fast-suite-flag1.txt` 30/30; every `/dev/null` check before and after each host step read a
character device (`DEVNULL_CHAR_OK` in the session log). Counters in `fast-flag1-sparse-counters.txt` and the reproducer
files: `refused=0 overflow=0 coarse_maps=0`. The Linux runs never reach the block budget, so they do NOT exercise the
fixed path (`coarse_maps=0`); that path is covered by the GPU-free catia-like simulation (64 distinct 2 MiB regions against a
budget of 4 to 10, byte-identical to the twin-OFF mapping through map and unmap). The Windows campaign is the hardware proof.
