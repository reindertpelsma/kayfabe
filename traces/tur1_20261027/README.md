# TUR1 — kayfabe on Turing (TU106, RTX 2060 SUPER), 2026-10-27

STATUS: LIVE, 2026-10-27 — iteration 1 of lane TUR1. One rented box, Vast lease 57882297 (machine 13058, host 79466, Quebec CA, offer 57797054, about 0.12 USD/h). Branch `claude/tur1-20261027`.

All sentences below carry their lease and evidence path. A guest result counts against kayfabe only because the host reference on the same box was green (see "Host reference").

## Host reference (bare metal, same box)

MEASURED (2026-10-27, lease 57882297, `run1/host_reference_before.txt`): stock driver 575.51.03, `nvidia-smi` RTX 2060 SUPER, BAR1 256 MiB, Xid count 0, a driver-API host-to-device-to-host roundtrip of 1 MiB equal.

MEASURED (2026-10-27, lease 57882297, `run1/tur_gates.log`): after the host driver swap to 595.91.07 open, `v3_gates.sh` ends `V3_GATES_SUMMARY pass=9 fail=0 gate10=PASS`, births 12 of 12 USER.

MEASURED (2026-10-27, lease 57882297, `run1/tur1a_suites.run`): `bare_metal_suite.sh` 30/30 (`BARE_SUITE_PASS=30 BARE_SUITE_FAIL=0 BARE_SUITE_CRASH=0`); host CUDA ladder cup2, cup3, cup8, cup8bench 4/4 PASS (`run2_ladder/cl_tur1_host.out`).

MEASURED (2026-10-27, lease 57882297, `run1/tur1a_xid_after_fast.txt`): the host Xid sequence after bare suite and fast suite is ten Xid 31 records, all `HUBCLIENT_HSCE0` for the CE faults (the Turing spelling of the copy client) and one `HOST_CPU` PDE write fault. The five-and-five negative-control pattern of the Ampere and Ada cells, with Turing's spelling. The SW5 guard fix for `HSCE` (cherry-picked, `scripts/bench/native_xid_guard.py`) is what admits it.

## Linux guest on kayfabe

MEASURED (2026-10-27, lease 57882297, `run1/tur1a_suite.out`): thin guest `fast_suite.sh` 29/30 at the 180 s oracle; the one miss is `--ce-client-guest-ram` TIMEOUT (a timeout is a crash by rule).

MEASURED (2026-10-27, lease 57882297, `run1/tur1b_suite.out`, `run2_ladder/tur1c_suite.out`): the same arm alone times out again at 180 s and PASSES at a 600 s budget in 203 s (client 186.7 s). GA106 in SW5 lease 57726516 needed 129 s for it. The arm is slow, not hung (the device's invalidate and map counters keep advancing, `run1/ce-client-guest-ram_qemu_tail.log`). Classified PERF, not a correctness failure; the 180 s oracle is the owner's and is unchanged.

MEASURED (2026-10-27, lease 57882297, `run2_ladder/cl_tur1g_guest.out`): fat guest (guest driver 580.159.04, host 595.91.07) CUDA ladder 4/4 PASS **only with `KF3_DEV_EXTRA=guest-driver=580.159.04`**. Without it the device defaults the guest ABI to the host's 595.91.07 and the 580 guest's GSP queue init times out (`run2_ladder/fat_ladder_without_guest_driver_prop_cup2.log`). That is operator configuration, not a kayfabe defect; master's `provision_bench_tree.sh` no longer writes the staged-driver receipt that the lane branches had.

## Windows guest on kayfabe (LTSC 2024, driver 580.88, `guest-driver=580.65.06`)

MEASURED (2026-10-27, lease 57882297, `run3_windows/tur1_w4.run`): without the winprod flag set the guest driver starts, issues `GFX_POOL_QUERY_SIZE` (0x2080121f), is refused, and unloads (`Code 43`, 25 distinct refused controls, `run3_windows/w4_refused_set_no_flags.txt`).

MEASURED (2026-10-27, lease 57882297, `run3_windows/tur1_w6.run`, `run3_windows/nvidia_smi_windows_tu106_after_fixes.txt`): with the winprod flag list of `scripts/bench/windows/host_tools/runners/winprod/windows_broker_prod2.sh` (20 `KF3_*=1`) and `display=on`, Windows reaches `ConfigManagerErrorCode 0`, `nvidia-smi` shows RTX 2060 SUPER, architecture Turing, driver 580.88, GSP firmware 580.88, 6144 MiB, WDDM, display attached.

MEASURED (2026-10-27, lease 57882297, `run3_windows/matrix_after_userring_fix_fresh_boot.txt`): in the first two minutes of a fresh boot, with the two fixes below, `cuda_callback` and `cuda_occupancy` print `RESULT OK`, `dxprobe_d3d11` (compute plus draw readback) and `dxprobe_d3d12` (DIRECT and COPY queue fences) exit 0, `cup3` exits 0 and `cup8` ends `bad=0 maxerr=0 ... PASS` (a 2048x2048 matmul on the GR engine, byte exact). `cup2` fails `cuCtxCreate` with out-of-memory after a TDR.

MEASURED (2026-10-27, lease 57882297, `run3_windows/w7_nvlddmkm153_bugcheck_xml.txt`, `run3_windows/w8_host_fence_overdue_probe_block.txt`): a few minutes into a boot, nvlddmkm logs `Resetting/Reset/Restarting TDR` triples (66 events by 01:35 UTC), the RM unloads and reloads (nine lives in one boot) and one boot ended in bugcheck 0x113 with parameter 1 0x2c. The completion probe (`KF3_COMPLETION_PROBE=1000`) names one Translated copy-engine channel (token 0x809, host 0x8003b, engine 0xb) with `HOST-FENCE-OVERDUE`: the guest `GP_PUT` is 12028, the guest `GP_GET` 12026, the host ring is drained (GET equals PUT, 230) and fence 765 never completes. No host Xid. This is the "completion lost until TDR" class already tracked in `docs/STATUS_AND_HANDOFF.md`; nothing here shows it is Turing specific.

## Defects found and fixed on this branch

1. `display=on` could not realize on Turing: `NVC57E_SET_SEMAPHORE_CONTROL_PAYLOAD_SIZE is not derived`. C57E's window semaphore is 32-bit with one context DMA and control word. `kf-disp` `Vocab` now carries the payload bit, release mode and separate acquire semaphore as absent on Turing. Test first: `crates/kf-disp/tests/vocab_all_families.rs` (EXPECTED RED commit 267d93a74), fix in the next commit. The Turing ACQUIRE value is NOT modelled (a Turing window never waits on an acquire); that is a stated gap, not a verified behaviour.
2. The Windows USER ring relay refused every Turing channel (`USER_RING_SOURCE_DECLARATION`, class 0xc46f), so `cuCtxCreate` returned 999 and `D3D11CreateDevice` 0x8876017c on a Code 0 device. It now admits `TURING_CHANNEL_GPFIFO_A` next to `AMPERE_CHANNEL_GPFIFO_A`. Test first: `userrelay::tests::the_user_ring_admits_the_turing_and_ampere_gpfifo_classes_only` (EXPECTED RED commit, no predicate yet).
3. `scripts/bench/windows/win_vm.sh`: `WINVM_KF3_DISPLAY=off` builds a headless device (no display plane, no boot GOP) for boxes without a monitor.

## Not done

- Mutant checks for the two fixes (the vocabulary tests and the predicate test each have a second killer only by construction; no hand mutation was run).
- The full Linux fast suite on the fixed binary is in `run4_regress/` if the run finished; see that directory.
- The Windows TDR above, the `cup2` out-of-memory after it, `d3d12_signal_probe.ps1`, `opencl_icd`, Heaven and any application.
- TU106 native Windows reference (lane WC1, machine 39235). The only native Windows Turing reference is TU116 (`docs/design/V3_TURING_DIE_TABLE.md` on `origin/codex/wc1-20261026`).
