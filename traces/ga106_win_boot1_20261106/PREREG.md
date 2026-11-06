# GA106 Windows gate, boot 1 "QEMU is gone": pre-registration

**STATUS: RESEARCH, 2026-11-06.** Written and pushed before any box was rented for this question. Results go
to `README.md` in this directory; this file is not edited after the first run starts (corrections go into the
README, above the text they correct).

## The failure under study (MEASURED from the job log, 2026-11-05/06)

gpu-vast run 42111518119 attempt 2, tested tree `next` 9269a3a1, Vast machine 22775 (host 79466, Quebec), RTX 3060
(GA106), 21 vCPU and 49 GB RAM as the gate saw them. Every Linux stage passed (provision, cuda_host 4/4, merge_bar
gates 9 + raw 30 + thin 30 + census, cuda_guest 4/4). Host reference green before and after, `xid_delta=0`. The
Windows gate log:

```
GATE_STAGE install EXIT ok 3077s
GATE_STAGE boot1 START 2026-11-05T23:58:24+00:00
GATE_NOTE QEMU is gone while waiting for ssh
GATE_NOTE graceful stop of the guest (gate_...)
WINDOWS_GATE_FAIL diag=KAYFABE stage=boot1 why="the guest never answered ssh after boot 1 (900 s); run log:
  [2026-11-05T23:58:29Z] ENV_FILE ... [2026-11-05T23:58:29Z] KF3_ENV ..." ... secs=3406
WINDOWS_GATE_EXIT rc=1 2026-11-06T00:03:07+00:00
```

The evidence archive was on the box and is lost.

## What the log already says (INFERRED from the job log and the script text of 9269a3a1)

1. The failure text "(900 s)" is a fixed string. The gate's own clock says otherwise: boot1 began at gate second
   3124 (23:58:24 minus the gate start 23:06:20) and the verdict was written at second 3406. The teardown and the
   host reference AFTER (both after the failure) fit in those 282 s. So the ssh wait did not run 900 s.
2. The run log's last two lines at the moment of the failure were `ENV_FILE` and `KF3_ENV` (23:58:29Z). In
   `win_vm.sh run`, `QEMU_START` comes next; it is logged after `run/qemu.pid` is written. In the two passing GA106
   gates of 2026-11-04 (lease 59031382, `traces/die_coverage_20261104/ampere/ga106/dca106{a,b}_win/*_run.log`)
   `KF3_ENV` -> `QEMU_START` took 3 s, and boot1 START -> `QEMU_START` took 6 s and 10 s.
3. The gate runs `win_vm.sh run --detach`, sleeps 5 s, then `wait_ssh`, whose first act is `qemu_alive`: it reads
   `run/qemu.pid`. Before the detached loop has passed the GPU lock, `nvidia-smi -q` and the swtpm start, that file
   does not exist, and `qemu_alive` is false: "QEMU is gone".
4. The teardown right after the failure printed "graceful stop of the guest", which it prints only when
   `qemu_alive` is true. So a QEMU was running AFTER the failure. A QEMU that had crashed or been OOM-killed at
   boot would have ended the run loop (`reason` is not `guest-reset`), and no QEMU would have been running.

## Hypotheses and falsifiers

- **H-RACE (a form of H-GATE-KILL; the leading hypothesis).** The gate read "QEMU is gone" before the detached run
  loop had written `run/qemu.pid`; no QEMU had exited. The gate's own teardown then stopped the QEMU that started
  a few seconds later.
  FALSIFIERS: (a) in the forced reproduction (E2 below) the unmodified gate does not log "QEMU is gone" before the
  run loop's `QEMU_START`, or it reaches `ssh up`; (b) in any run, the 10 Hz observer shows `run/qemu.pid` present
  and its process alive at the second the gate logs "QEMU is gone"; (c) a run log of a failing boot 1 shows a
  `WINVM_BOOT_END boot=1` before the gate's failure line.
- **H-OOM.** The host OOM killer or a cgroup limit killed QEMU (guest RAM 8192 MiB memfd, `WINVM_RAM_MB` default;
  box 49 GB). FALSIFIER: no `oom`/`Killed process` line in `dmesg -wT` or the journal during any run, and every
  `WINVM_BOOT_END` has `rc=0` and a `guest-*` reason. (The original box's dmesg is lost: this can only be judged on
  the reproductions; item 4 above already argues against it.)
- **H-QEMU-CRASH.** kf3's QEMU died (SIGSEGV, abort, assert). FALSIFIER: no `WINVM_BOOT_END` with `rc>128` or
  `rc!=0`, no core file, no `segfault`/`traps:` line in dmesg, and the QEMU logs end in an orderly shutdown.
- **H-SLOW-CPU.** A hard-coded wait was exceeded on the slow machine (Windows first boot, OOBE). FALSIFIER: the
  failure happened before any wait expired: in the original run the ssh wait was not 900 s (item 1), and the
  install (3077 s) was inside `WINVM_INSTALL_WAIT_S=4200`. In the reproductions: every boot-1 failure comes before
  the 900 s ssh wait ends. (A slow CPU may still be the reason the race is lost: that is H-RACE's mechanism, not a
  timeout.)
- **H-DIE.** A GA106-specific kayfabe defect on boot 1. FALSIFIER of H-DIE: the gate with the start-wait fix passes
  boot 1 and the whole gate on a GA106 box (`WINDOWS_GATE_PASS`), n>=2 runs, and the forced reproduction fails only
  the unmodified gate.

## Plan (one GA106 box; machine 22775 only if it is the only offer, or later for machine separation)

Box: one RTX 3060 KVM offer (`vms_enabled=true`, the KVM image per `scripts/bench/box/README.md`), not machine 22775
first, never 17023, 36135 or 103782. Label `kf-ga106win-20261106`, lease <= 3 h, one re-rent allowed, budget 4 USD.
Provision: `provision_full.sh` with `KAYFABE_BRANCH=next` at 9269a3a1 (the tested tree; host driver as the Worker
provisions it). All gate runs: `bash scripts/bench/box/windows_gate.sh <tag> /root/prov` from a checkout, default
flags file `kf3_flags.sample.env`, no `WINVM_INSTALL_CACHE` (the Worker sets none), the first run installs.

- **E1 (Worker-like, unmodified 9269a3a1).** The full gate including the install. Prediction under H-RACE: either
  PASS (the race is won) or the same "QEMU is gone" within seconds of boot 1, with a QEMU running afterwards.
- **E2 (forced reproduction, unmodified 9269a3a1).** Same installed image, new tag. A helper holds the GPU lock
  `/tmp/kayfabe-fastguest.lock` for 30 s from the moment `GATE_STAGE boot1 START` appears (the run loop waits up to
  120 s for it). Prediction under H-RACE: "QEMU is gone" about 5 s into boot 1, `QEMU_START` about 30 s later.
- **E3 (fixed gate, the same forced delay).** This branch's `windows_gate.sh` (waits for `QEMU_START` of the boot),
  `WINVM_QEMU` = the 9269a3a1 kf3 binary (only harness text differs). Prediction: `GATE_NOTE QEMU boot 1 ... started
  after ~30s`, then the gate continues; a PASS of the whole gate is the H-DIE falsifier run 1.
- **E4 (fixed gate, natural).** Prediction: PASS (H-DIE falsifier run 2).
- If budget and an offer allow: E4 again on machine 22775 to separate machine from die.

## Capture plan (every run)

Before the gate starts: `dmesg -wT` and `journalctl -f -o short-iso` to files; `free -m`, `nproc`, `lscpu` head,
`cat /proc/pressure/memory` if present, `ulimit -c unlimited` in the gate's shell and
`core_pattern=/workspace/cores/core.%e.%p.%s.%t` (QEMU inherits both); a 10 Hz observer writing
`EPOCHREALTIME`, the content of `run/qemu.pid`, whether that pid lives, and the qemu/win_vm process list on change;
the gate's stdout through a per-line UTC timestamper; `/usr/bin/time -v` is not used (it would change nothing the
gate sees). After: the `WINVM_BOOT_END` lines (QEMU's exit status and reason as the run loop recorded them),
`stop.log`, the gate's evidence tgz, the observer log, dmesg and journal filtered for `oom|killed process|segfault|
traps:|qemu|kvm|NVRM`, any core (`file`, and a `gdb -batch -ex bt` if one exists). Off the box before teardown, to
`/root/night/ga106win_evidence/` on the coordinator, then into git here without secrets, UUIDs or IPs.
