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

MEASURED (2026-10-27, lease 57882297, `run3_windows/matrix_second_fresh_boot_tdr_storm.txt`, `run3_windows/tur1_w10.run`): a second fresh boot of the same image met a TDR storm (a `nvlddmkm` 153 triple every 30 s from 02:09 UTC) as soon as the desktop was up; `cuda_callback`, `cuda_occupancy`, `cup2` and `cup3` passed in the gaps, `dxprobe_d3d11`, `dxprobe_d3d12` and `cup8` failed (`D3D11CreateDevice` 0x887a0005, `cuCtxCreate` 999). The matrix is therefore n=2 with different outcomes: the Windows guest on TU106 is NOT yet stable.

MEASURED (2026-10-27, lease 57882297, `run3_windows/tur1_w11_nd.run`, `run3_windows/tur1_w11_idle.txt`): with `WINVM_KF3_DISPLAY=off` and no workload the guest was quiet for four minutes (no new 153 event after the boot, `nvidia-smi` 580.88, 61 MiB used). MEASURED (2026-10-27, lease 57882297, `run3_windows/tur1_w13_nd.run`): with `display=off` and the matrix tools running, the RM unloaded four times and the guest stopped answering ssh for more than six minutes. So the resets follow GPU work, and not only the emulated display.

## Linux regression on the fixed binary

MEASURED (2026-10-27, lease 57882297, `run4_regress/tur1d_suites.run`, `run4_regress/tur1d_suite.out`): with the display and user-ring fixes built in (`kf3-bins/67339dc0-dirty`, branch head `e3c4483a0` plus the test additions), `bare_metal_suite.sh` is 30/30 and the thin-guest `fast_suite.sh` at a 300 s budget is 30/30 (`FAST_SUITE_PASS=30 FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=0`); `--ce-client-guest-ram` takes 187 s.

MEASURED (2026-10-27, lease 57882297, `run4_regress/host_reference_after.txt`): host reference after every run: driver 595.91.07 open, 1 MiB used, `cuprobe.py` roundtrip equal, 20 Xid records, all Xid 31 from the approved negative controls of four bare-and-fast suite passes (five each), none from any Windows, ladder or mutant run.

## Defects found and fixed on this branch

1. `display=on` could not realize on Turing: `NVC57E_SET_SEMAPHORE_CONTROL_PAYLOAD_SIZE is not derived`. C57E's window semaphore is 32-bit with one context DMA and control word. `kf-disp` `Vocab` now carries the payload bit, release mode and separate acquire semaphore as absent on Turing. Test first: `crates/kf-disp/tests/vocab_all_families.rs` (EXPECTED RED, commit 267d93a74), fix in the next commit. The Turing ACQUIRE value is NOT modelled (a Turing window never waits on an acquire); that is a stated gap, not a verified behaviour.
2. The Windows USER ring relay refused every Turing channel (`USER_RING_SOURCE_DECLARATION`, class 0xc46f), so `cuCtxCreate` returned 999 and `D3D11CreateDevice` 0x8876017c on a Code 0 device. It now admits `TURING_CHANNEL_GPFIFO_A` next to `AMPERE_CHANNEL_GPFIFO_A`. Test first: `userrelay::tests::the_user_ring_admits_the_turing_and_ampere_gpfifo_classes_only` (EXPECTED RED: no predicate yet).
3. `scripts/bench/windows/win_vm.sh`: `WINVM_KF3_DISPLAY=off` builds a headless device (no display plane, no boot GOP) for boxes without a monitor.
4. SW5's `HUBCLIENT_HSCE[0-9]+` guard fix (two commits, EXPECTED RED then fix) cherry-picked from the local `codex/sw5-20261024`, which was never pushed.

MEASURED (2026-10-27, lease 57882297, `run4_regress/mutants.run`): hand mutants of the fixes, each killed by two tests: payload bit required again (1 kf-disp lib test and 1 vocabulary test fail), acquire group required again (the same two), predicate Ampere-only (2 kf-qemu tests fail); baseline and restored: 0 failures.

## Operator findings (not product defects)

- Master's `provision_bench_tree.sh` wants `[PRIVATE_PATH]` even when the host is 595.91.07; the lane branches fetched it. Fetch it by hand.
- The fat guest needs `KF3_DEV_EXTRA=guest-driver=<guest version>` (above).
- `win_vm.sh run` does not set the Windows production flags; the 20-flag list lives in `scripts/bench/windows/host_tools/runners/winprod/windows_broker_prod2.sh`. `fb-mb` must be at most about 6144 on an 8 GB GPU (`WINVM_KF3_FB_MB=6144`).
- Ubuntu 22.04's mingw lacks `D3D_FEATURE_LEVEL_12_2`; `kf_dxprobe.cpp` builds with `-D'D3D_FEATURE_LEVEL_12_2=((D3D_FEATURE_LEVEL)0xc200)'`. `kf_overlayprobe.cpp` does not build there (`SetProcessDPIAware`).
- A strictly serial harness: a Windows install QEMU running makes every Linux guest arm refuse to start.

## Open hypotheses for the next iteration (INFERRED, each with its falsifier)

- H-CE2 (INFERRED): the Windows TDRs are lost completions of Translated copy-engine fences on LCE2 (engine 0xb), the class in `docs/STATUS_AND_HANDOFF.md`; evidence is the single `HOST-FENCE-OVERDUE` dump (`run3_windows/w8_host_fence_overdue_probe_block.txt`: host ring drained, fence 765 not seen). FALSIFIER: a reset with no overdue Translated fence in the probe, or an overdue fence that completes on a later wake. Next measurement: the same Windows flow on an Ada or Ampere box with `KF3_COMPLETION_PROBE=1000`, to see whether the overdue dump is Turing-specific.
- H-DISP (INFERRED): the Turing window flip semaphore (acquire not modelled, release on the shared control word) is wrong, which keeps flips from completing (the engine stops at about 30 to 40 updates). FALSIFIER: the headless run (`tur1_w13`) resets too, which it did, so display alone is not the cause; display may still add a second fault. A Turing plane oracle from WC1's TU106 native capture would settle the semaphore semantics.

## Not done

- Windows applications, Heaven, the 600 s hold, `driver_status` and `opencl_icd` rows (the lease ended first), and the TDR root cause.
- TU106 native Windows reference (lane WC1, machine 39235). The only native Windows Turing reference is TU116 (`docs/design/V3_TURING_DIE_TABLE.md` on `origin/codex/wc1-20261026`), so the Windows failures above are classified as kayfabe's only provisionally.


## Iteration 2 (2026-10-27): PR split, shorter lease, Ampere control, lost ring

### PRs (all base master; each with an EXPECTED RED commit before the fix)

| PR | Content | State |
|---|---|---|
| #161 | `HUBCLIENT_(CE\|HSCE)` in the native Xid guard (SW5's two commits) and the payload mutants that named the old line | ready; stable, firmware, broker and aarch64 pass, the `gpu-vast` jobs fail on the Worker's UNAVAILABLE state |
| #162 | Windows USER ring admits `TURING_CHANNEL_GPFIFO_A` (0xc46f) | ready; every check passes, `gpu-vast` included |
| #163 | `WINVM_KF3_DISPLAY=off` (`kf3_device_props`) | draft |
| #164 | `kf-disp` Turing NVC57E window semaphores (acquire not modelled) | draft, needs owner review |
| #165 | `windows_lane.sh` and `WINVM_INSTALL_CACHE` | draft |
| #168 | a worker that finds its Translated slot locked gives the ring back (lost wake) | draft, needs owner review |
| #169 | completion probe: verdict, cause and token word | draft |

### Shorter Windows lease: what the setup-cache route is, and what it is not

CODE READ: `setup-cache-*` blobs (`docs/design/VAST_SETUP_CACHE_20261023.md`) are the Linux dependency cache (.debs, `cargo vendor`). A sealed Windows base image would be a different blob in `win-gpu-image-private-store`, with three obstacles: `win_vm.sh` never restores a VM from another VM's state (swtpm state, harness ssh key, Windows password are per VM), the store's write path is the `win-gpu-image` environment only, and the store README keeps the licence review open. I did not publish one. That is an owner decision.

MEASURED (2026-10-27, lease 57908255, `iter2_ga106/winlane.log`): `windows_lane.sh` took a fresh box to a sealed Windows base image in 3407 s (box 219, guest run 9, host driver 264, bench tree 405, kf3 build 18, prerequisites 36, install 2456). The install was NOT faster: 41 min with 16 cores and `cache=unsafe`, against 32 min on lease 57882297 and 19 min on a 16-core box in BW11. INFERRED: the install is bound by one nested-virtualization thread, so cores and disk cache do not help; the only large saving left is a sealed image. Falsifier: an install on a bare-metal KVM host with the same knobs finishing in about 20 min.

### Control: the same Windows flow on Ampere (GA106, RTX 3060)

Host: Quebec CA, host 79466 (the same host as the TU106 box), Xeon E5-2698 v4, 38 vCPUs, offer 54502208, machine 13666, 0.107 USD/h, kayfabe head `385c9285` to `340c6b5a`.

MEASURED (2026-10-27, lease 57908255, `iter2_ga106/hostref_before.txt`, `hostref_after.txt`): host reference green before and after: stock 575.51.03 then 595.91.07 open, BAR1 256 MiB, 0 Xid, roundtrip equal, 1 MiB used at the end.

MEASURED (2026-10-27, lease 57908255, `iter2_ga106/ga106_o1_timeline.csv`, `ga106_o1_matrix.txt`): with the winprod flags, `display=on` and the completion probe, Windows reached Code 0, and then reset in a storm: 54 `nvlddmkm` 153 events 6 minutes after boot and 96 at 10.5 minutes (about 9 events a minute, one triple every 20 s), 116 RM unloads in 17 minutes. In the first minutes `cuda_occupancy`, `dxprobe_d3d11`, `cup2`, `cup3` and `cup8` (matmul byte exact) passed; `cuda_callback` and `dxprobe_d3d12` failed. TU106 showed the same storm (lease 57882297, 66 events in 8 minutes). So the storm is not Turing specific.

MEASURED (2026-10-27, lease 57908255, `iter2_ga106/ga106_o1_overdue_block.txt`): the Ampere run printed the same `HOST-FENCE-OVERDUE` dump as TU106 (ring drained, fence word 1464 equal to the in-flight sequence 1464). H-CE2 (a Turing-specific lost LCE2 fence) is falsified.

MEASURED (2026-10-27, lease 57908255, `iter2_ga106/ga106_h1_*`, `ga106_h2_*`): headless (`display=off`) and idle, the Ampere guest was quiet after one early triple (3 events in 8 minutes). Headless with the matrix running it went quiet in a different way: after 4 RM unloads the guest stopped answering ssh for more than eight minutes while the log kept filling with `NV2080_CTRL_CMD` 0x20801111 requests (572 in 10 minutes, answered "NV_OK, request echoed, contents ignored" by the `KF3_SW_RUNLIST_HOST_OWNED` path that awaits the owner's confirmation). The same control is asked about 1000 times in every storm run, so it is a lead, not a cause.

### The lost ring (fixed on #168)

MEASURED (2026-10-27, lease 57908255, `iter2_ga106/ga106_o2_overdue_block.txt`, `ga106_h1_overdue_block.txt`): with the new probe lines, the overdue dump reads `HOST-DONE-NOT-SEEN marked`, cause `WAKE-NOT-PUMPED` (the last wake 1001 ms ago, the channel's last pump 1002 ms ago, which read fence word 299 for head 300), token word `Idle`, and `chan[contended=1]`. CODE READ: `ChanPlane::serve` `try_lock`ed the slot and dropped the ring when another thread held it. With the ring given back (#168), MEASURED (2026-10-27, lease 57908255, `iter2_ga106/ga106_o3_*`): the contended counter reached 3 in 17 minutes and no `HOST-FENCE-OVERDUE` dump appeared, against one or more in every earlier run. The storm itself continued (116 unloads), and also with a console consumer pulling a frame a second (`ga106_o4_*`, 328 screendumps, 16 unloads in 3 minutes): a missing display consumer is not the cause either.

### What is left, and the TU106 native reference

The native TU106 reference (WC1, machine 39235) passed, so a green-host-reference failure on TU106 is kayfabe's, but it held no continuous workload and is not a stability reference. The storm is therefore classified as kayfabe's and generic (TU106 and GA106). Its cause is not found. Ruled out here: Turing-specific LCE2 fence (H-CE2), the dropped ring (still storms), a missing console consumer, the completion probe itself (storms without it, TU106 `w7`). Open leads, each INFERRED: the display-on desktop (idle headless is quiet), the ignored software-runlist control, and the unmodelled Turing flip semaphore (H-DISP; WC1's table lists GSP controls and classes only, no flip semaphores, so it cannot settle it). Next measurement: ETW or the guest's dxgkrnl scheduler events on the first reset of a boot, then bisect the production flag list on GA106 (cheap and fast) before spending another TU106 lease.
