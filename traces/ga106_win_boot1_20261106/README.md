# GA106 Windows gate, boot 1 "QEMU is gone": a gate race, not kayfabe

**STATUS: ANSWERED, 2026-11-06.** Pre-registration: [PREREG.md](PREREG.md) (pushed as 04b7eccac before the
rental). Fix: `scripts/bench/box/windows_gate.sh` on branch `claude/ga106-win-boot1-20261106`.

## Verdict

The `WINDOWS_GATE_FAIL diag=KAYFABE stage=boot1` of gpu-vast run 42111518119 attempt 2 is a defect of the gate
script, not of kayfabe and not of the host. `windows_gate.sh` starts `win_vm.sh run --detach`, sleeps 5 s, and
then reads `run/qemu.pid`. On machine 22775 the detached run loop needed 5.28 s to write that file (MEASURED in
E1 below, instance 59343174, rev 9269a3a1, 2026-11-06), so the gate read a missing pid file as "QEMU is gone", failed the guest, and its own teardown then shut
down the QEMU that had just started. The label `diag=KAYFABE` was wrong: no guest had run. With the fix (wait for
the run loop's `QEMU_START` line of the boot) the gate passed on the same box.

## Box (own rental)

| instance | machine / host | GPU | price | created (UTC) | destroyed |
|---|---|---|---|---|---|
| 59343174 | 22775 / 79466, Quebec (the machine of the failing job) | RTX 3060, GA106 | 0.134 USD/h | 2026-11-06T00:29:49Z | see *Teardown* |

Offer search 2026-11-06 00:29Z (`vms_enabled=true`, one GPU, RTX 3060 / A2000 / 3050 / 3060 Ti): three offers.
Machine 35599 has 16 GB RAM (the gate refuses under 20 GB), machine 18856 is in the Worker's `badMachines`
(`ci/vast-broker/defaults.json`), so machine 22775 was the only usable one; no separate machine was available to
split machine from die. Box: Xeon E5-2699 v4 @ 2.20 GHz, 21 vCPU, 49 GB RAM, 117 GB root, host driver 580.159.04
(`provision_full.sh`, `KAYFABE_BRANCH=next`, `READY driver=580.159.04`, kf3 built at 9269a3a1, `e1_unmodified/prov.log`).

Deviation from the Worker path: the Linux stages (merge_bar, cuda_guest) were not run before the gate, so the
host carried 0 Xids instead of 10. The gate was called as the payload calls it: `windows_gate.sh <tag> /root/prov`
from the checkout, default flags file, no `WINVM_INSTALL_CACHE`, the first run installing Windows.
Capture around every run (`box_scripts/`): `dmesg -wT`, `journalctl -f`, `free -m`, a 10 Hz observer of
`run/qemu.pid` and the qemu/swtpm/run-loop pids (`observer.txt`), the gate's stdout with a UTC timestamp per line
(`gate_ts.txt`), `ulimit -c unlimited` with `core_pattern` to a file.

## E1: unmodified gate, tested tree 9269a3a1 (MEASURED, 2026-11-06, `e1_unmodified/`)

The failure reproduced on the first try, in the same words as the Worker job:

```
01:57:05.143 GATE_STAGE install EXIT ok 3522s
01:57:12.572 GATE_STAGE boot1 START
01:57:18.326 GATE_NOTE QEMU is gone while waiting for ssh
01:57:21.385 GATE_NOTE graceful stop of the guest (gate_e1ga106)
01:59:15.808 WINDOWS_GATE_FAIL diag=KAYFABE stage=boot1 why="the guest never answered ssh after boot 1 (900 s); run log: ... ENV_FILE ... KF3_ENV ..."
```

The observer and the run loop's log (`observer.txt`, `gate_e1ga106_run.log`, `run_launch.txt`):

| time (UTC) | event |
|---|---|
| 01:57:13.477 | the detached run loop appears (`run_launch.txt`: `detached` at 01:57:13) |
| 01:57:15 | `WINVM_RUN_START` (GPU lock taken at once; `qemu_bin`, `check_prereqs`) |
| 01:57:16 | `ENV_FILE`, `KF3_ENV` (the two lines the Worker job's failure quoted) |
| **01:57:18.326** | **gate: "QEMU is gone"** (the observer shows `pidfile=none`, no qemu process) |
| 01:57:18.544 | swtpm started |
| **01:57:18.760** | **`run/qemu.pid` = 255834, alive** (0.43 s after the gate's verdict) |
| 01:58:18 | QMP `NIC_RX_FILTER_CHANGED`: the guest's virtio-net driver is up (Windows is booting) |
| 01:59:01.9 | QMP `SHUTDOWN guest=true reason=guest-shutdown` (the gate's `win_vm.sh stop` asked it to) |
| 01:59:07 | `WINVM_BOOT_END boot=1 rc=0 reason=guest-shutdown`, `WINVM_RUN_END` |

Launch to pid file: 5.28 s; the gate looked after 5 s. Install: 3522 s (media fetch 7 min, install boot 48 min,
install check 3 min; the Worker job's was 3077 s), inside `WINVM_INSTALL_WAIT_S=4200`.

## E3: the fixed gate under a forced 30 s delay (MEASURED, 2026-11-06, `e3_fixed_forced_delay/`)

Same box and installed image, branch head 04b7eccac (harness only), `WINVM_QEMU` = the 9269a3a1 kf3 binary.
`box_scripts/hold_lock.sh` held the run loop's GPU lock from 01:59:43.6 until 30 s after boot1 START (released
02:00:20.6; `hold_lock.txt`). Run loop appeared 01:59:50.7, `qemu.pid` 02:00:25.8 (35 s; 5.2 s after the lock was
released). The gate:

```
01:59:50.452 GATE_STAGE boot1 START
02:00:26.756 GATE_NOTE QEMU boot 1 of gate_e3fix started after 35s
02:02:15.803 GATE_NOTE ssh up after 0s
02:39:29.958 WINDOWS_GATE_PASS rev=04b7ecca gpu="NVIDIA GeForce RTX 3060" idle_s=300 hostref=green/green secs=2398
  install=once driver=580.88 gsp=forced code=0 probes=cup8,d3d11,x2 resets=0 known_live_dumps_193=2 ...
```

Boot ends: boot 1 `rc=0 reason=guest-reset` (the gate's `shutdown /r`), boot 2 `rc=0 reason=guest-shutdown`.

## E4: the fixed gate, natural timing

MEASURED, 2026-11-06, `e4_fixed_natural/`: same box, image and binaries as E3, no forced delay. The run loop
appeared at 02:40:14.6 and `qemu.pid` at 02:40:20.26 (5.6 s; a transient `qemu-system-x86` at 02:40:14.6 with no
pid file is the loop's own binary probe, not the guest). The gate logged `QEMU boot 1 of gate_e4fix started after
6s`, `ssh up after 0s`, and:

```
03:23:14.223 WINDOWS_GATE_PASS rev=04b7ecca gpu="NVIDIA GeForce RTX 3060" idle_s=300 hostref=green/green secs=2609
  install=once driver=580.88 gsp=forced code=0 probes=cup8,d3d11,x2 resets=0 known_live_dumps_193=2 ...
```

Boot ends: boot 1 `rc=0 reason=guest-reset`, boot 2 `rc=0 reason=guest-shutdown`; host Xid 0; no OOM, segfault or
core. With natural timing the run loop needed more than the old 5 s sleep in both runs on this box (E1 5.28 s, E4
5.6 s): INFERRED, the unmodified gate would have failed E4 as well.

## E2 (not run)

The pre-registered forced reproduction of the UNMODIFIED gate was skipped: E1 reproduced the failure without any
forcing, and the 3 h lease did not hold E1 (an hour, mostly the install) plus E2, E3 and E4.

## Hypotheses (pre-registered falsifiers, judged)

- **H-RACE: not falsified, MEASURED.** (b) At the gate's "QEMU is gone" (01:57:18.326) the observer had
  `pidfile=none` and no qemu process; the pid file appeared 0.43 s later and its QEMU booted. (c) The run log has
  no `WINVM_BOOT_END` before the failure; boot 1 ended `rc=0 guest-shutdown` after the gate's stop.
- **H-OOM: falsified on this box.** No `oom`, `Killed process` or `out of memory` line in `dmesg -wT` or the
  journal (E1, E3); `available` 49 GB before and after. (The Worker job's own dmesg is lost.)
- **H-QEMU-CRASH: falsified on this box.** Every `WINVM_BOOT_END` has `rc=0` and a `guest-*` reason; no core
  file; no `segfault` or `traps:` line.
- **H-SLOW-CPU: falsified as a timeout.** No wait expired: the gate gave up 5.75 s into boot 1, not after 900 s,
  and the install ended inside its 4200 s bound. The slow CPU is why the race is lost (5.28 s of run-loop work
  before the pid file). The two 2026-11-04 GA106 gates on lease 59031382 (also Quebec) show about 5 s from
  `OVERLAY_CREATED` to `QEMU_START` at 1 s resolution (`../die_coverage_20261104/ampere/ga106/dca106{a,b}_win/`):
  they won the same race narrowly, and lane 2's run 3 on machine 22775 passed boot 1 too. That is H-RACE.
- **H-GATE-KILL:** the gate did stop the QEMU itself, but only after it had wrongly concluded "gone": the cause is
  the race, the stop is its consequence. The Enter-press commit ee64977b4 is not in 9269a3a1 and plays no part.
- **H-DIE: falsified for this failure.** The fixed gate passed the whole gate on GA106 (E3 and E4, n=2, `WINDOWS_GATE_PASS` with resets=0), with the
  same kf3 binary that the unmodified gate failed on.

## Fix (harness only; no crate touched)

`windows_gate.sh`: `wait_qemu_start TAG BOOT SECONDS` waits for the run loop's `QEMU_START tag=<tag> boot=<n>`
line (written after the pid file), as the reboot stage already did. A loop that logged `WINVM_RUN_END` or a
`REFUSED:` line, or vanished, without that line is a harness fault (`fail_op`, RETRY OPERATOR), never
`diag=KAYFABE`. Used at boot 1 (`cap 300`, which covers the run loop's 120 s GPU-lock wait) and at the post-mortem
boot, which had the same `sleep 5`. The failure text now names the real ssh wait instead of a fixed "900 s".
Selftest 18 -> 24 cases; `scripts/ci/test_vast_windows_gate.py` adds two mutants and a call-site test.

Not fixed here (recorded): `wait_ssh` still treats a QEMU that exits on its own during boot 1 (a guest reboot) as
fatal; no run so far has needed that path.

## What the Worker should keep off the box in future

The gate's evidence tgz (`<prov>/<tag>_windows.tgz`, 28 files on a boot-1 failure) holds the run log, the QMP
events, `stop_cleanup.txt` and the QEMU logs, which were enough here; it was lost because nothing copies it off a
Worker box. Recommended (a Worker change, not made here): upload the tgz (bounded, text only, password-scanned by
the gate already) with the result, and add to it the host `dmesg -T` tail and `free -m` at the failure.

## Teardown

Instance 59343174 destroyed by id at 2026-11-06T03:24:03Z (`vastai destroy instance 59343174 -y`); `vastai show
instances-v1 --raw` no longer listed it. The one other instance listed belonged to another session and was not touched.

## Cost

INFERRED from the offer rate: 00:29:49Z to 03:24:03Z is 2.90 h at 0.134 USD/h, about 0.39 USD, plus the
downloads (about 6 GB of media and packages). The account credit is shared, so its change is not this run's alone.
