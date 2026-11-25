# Rootless proof: kf3 QEMU as an unprivileged uid with no capability, Linux and Windows guests (2026-11-25)

**STATUS: RESEARCH, 2026-11-25. MEASURED on one Vast box, one die (AD102), one host driver, one run per stage.**
Hypotheses, falsifiers and procedure were committed before the run (`PREREGISTRATION.md`, commits `2b5ff7a6` and
`ae1ebd74`; addendum 1 in `f00cf99f`, written after the first Windows install failed and before the runs it governs).
Measured = a run named here; inferred = said so.

- Box: Vast instance 62458518, label `kf-rootless-20261125b`, RTX 4090 24564 MiB (AD102), 40 vCPUs, 131 GB RAM, 146 GB
  root (`df -h /` at start: 138 GB free), image `vastai/kvm:ubuntu_cli_22.04-2025-05-16`, nested KVM, 0.635 USD/h. Rented
  03:58Z, destroyed 05:33Z (1.6 h, about 1.0 USD from the rate, not an invoice); absence checked with
  `vastai show instances-v1` (own id only). A first rental, 62455597, stayed in `loading` for 26 minutes and was destroyed
  unused. Host driver after `provision_full.sh`: 580.159.04 (`READY`, `ev/prov.log`).
- Trees: the Linux stages ran at `ae1ebd74` (this branch: `origin/next` `03d2a126` plus the preregistration). The Windows
  stages and the second N2 control ran at `f00cf99f` = `ec079dca` (the two fixes below) plus addendum 1. Each kf3 binary
  was built by the unprivileged user (`ev/build.log`, `ev/build_kf3.log.gz`: `KF3_BUILT .../kf3-bins/<rev>/`).
- User: `useradd --create-home --groups kvm kfrun` (uid 1002); `sudo -l -U kfrun`: "not allowed to run sudo"; no
  `kfroot install`. Every stage was started by root as `prlimit --memlock=8388608:8388608 python3 -I scripts/ci/kfroot.py run
  --user kfrun --no-memlock -- bash stage.sh <stage>` (`run_proof.py`), i.e. `setpriv --reuid kfrun --regid kfrun --groups kvm
  --inh-caps=-all --ambient-caps=-all --bounding-set=-all --no-new-privs`, with no sudo anywhere.
- What the box gave the user, unchanged (`ev/setup_root.txt`): `/dev/kvm` `crw-rw---- root kvm`; `/dev/nvidia0`,
  `/dev/nvidiactl`, `/dev/nvidia-uvm`, `/dev/nvidia-uvm-tools` `crw-rw-rw- root root` (the driver's default; no node mode
  was changed); `/dev/nvidia-caps/*` root-only and never needed. `sudo` 1.9.9 (the original, not sudo-rs).

## Results

| stage (as `kfrun`) | tree | result | evidence |
|---|---|---|---|
| `toolchain` (per-user rustup through `kf_trust.py`) | `ae1ebd74` | rc 0, cargo 1.99.0 | `ev/toolchain.log` |
| `build` (`build_kf3.sh`) | `ae1ebd74`, then `f00cf99f` | rc 0 both | `ev/build.log` (second build), `ev/build_kf3.log.gz` |
| N1 `/dev/nvidiactl` 0600 | `ae1ebd74` | QEMU refused: `kf3: realize refused: RM bring-up failed at R0 openat(nvidiactl): Syscall { call: "openat", errno: Some(13) }` | `ev/fast_rlneg_N1_qemu.log` |
| N2 `/dev/nvidia-uvm` 0600 | `ae1ebd74` | QEMU refused: `... GPU walker bring-up on 0000:00:07.0: cuInit refused: 999 (CUDA_ERROR_UNKNOWN)` (node not named) | `ev/neg_N2a_refusal.txt` |
| N2 again | `f00cf99f` | `... cuInit refused: 999 (CUDA_ERROR_UNKNOWN); /dev/nvidia-uvm does not open read-write for this process (Permission denied (os error 13)): QEMU needs it (docs/howto/installing.md, Permissions)` | `ev/fast_rlneg_N2b_qemu.log` |
| N3 no group `kvm` | `ae1ebd74` | QEMU refused: `Could not access KVM kernel module: No such file or directory` / `failed to initialize kvm` | `ev/fast_rlneg_N3_qemu.log` |
| `suite` (`fast_suite.sh rlsuite 180`, beside the first install) | `ae1ebd74` | **`FAST_SUITE_PASS=30 FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=0`**, no TIMEOUT, so no re-run | `ev/rlsuite_suite.out`, `ev/suite.log` |
| `wininstall` first try | `ae1ebd74` | stopped at the TPM manufacture: `Need read/write rights on statedir /var/lib/swtpm-localca for user kfrun` | `ev/wininstall_a_tpm_refusal.txt` |
| `wininstall` | `f00cf99f` | rc 0 in 31 min (`TPM certificates from the per-VM CA ...`, install boot, check boot, seal) | `ev/wininstall.log` |
| `wingate` (`windows_gate.sh`, `KF_ROOTLESS=require`, `KF_WINGATE_IDLE_S=120`, default flags) | `f00cf99f` | **`WINDOWS_GATE_PASS rev=f00cf99f gpu="NVIDIA GeForce RTX 4090" idle_s=120 hostref=green/green secs=1594 rootless=ok ... driver=580.88 gsp=forced code=0 probes=cup8,d3d11,x2 resets=0`** | `ev/rlwin_windows.log`, `ev/wingate_rlwin.log` |

Windows details (`ev/`): the kf3 device line of both boots is `kf3-gpu,fb-mb=8192,...,display=on,gop=on,gop-efi=...`
(`gate_rlwin_b1_cmdline.txt`); guest `nvidia-smi -L`: `GPU 0: NVIDIA GeForce RTX 4090` (`nvidia_smi_L_guest.txt`);
display adapter `code=0 status=OK` (`display_final.txt`); the gate's host CUDA reference green before and after.
The memlock limit the gate saw: `memlock_kb=8192` (`GATE_NOTE host: ... uid=1002 memlock_kb=8192`).

## Verdicts (falsifiers from PREREGISTRATION.md)

- **H1 (works rootless): CONFIRMED** for this box and these trees. (a) 30/30 at `ae1ebd74`; (b) `WINDOWS_GATE_PASS` with
  `display=on,gop=on` and the guest's `nvidia-smi` naming the card at `f00cf99f`; group `kvm` only, driver-default node
  modes, memlock 8 MiB (the fallback was not needed). Two refusals by name came up, both in the Windows harness and
  not in QEMU: the TPM CA directory (fixed in `ec079dca`, see below) and nothing else. Nothing in QEMU or kf3 needed
  root, a capability, another group or a larger memlock limit.
- **H2 (no privilege in the tree): CONFIRMED for every QEMU and swtpm thread.** `procsample.py` read 550 QEMU thread
  records over the 30 suite boots, 629 QEMU and 8 swtpm/swtpm_setup/swtpm-localca records over the installs, 123 QEMU
  and 2 swtpm records over the gate: all `Uid 1002 1002 1002 1002`, `CapInh/CapPrm/CapEff/CapBnd/CapAmb
  0000000000000000`, `NoNewPrivs 1` (`ev/suite.procs.jsonl.gz`, `ev/wininstall.procs.jsonl.gz`,
  `ev/wingate_rlwin.procs.jsonl.gz`). No QEMU or swtpm of any other uid existed during the stages. The gate's own
  assertion agrees: `KFROOT_ASSERT mode=require ok=1 uid=1002 capeff=0 capprm=0 capbnd=0 nnp=1 procs=5` at boot 1 with the
  QEMU and swtpm pid files (`rootless_boot1.json`). The broker was not used (no stage starts it).
  Two sampler records are flagged and are not kayfabe processes: the `setpriv` launcher itself, seen once in the build
  stage and once in the first install, between its `setresuid` and its `execve` (uid 1002, `CapPrm/CapEff
  000001ffffffffff`, `CapBnd 0`, `NoNewPrivs 1`). [inferred, from the capability rules of `execve(2)`] with an empty bounding
  set, no file capabilities and no ambient set, the program it executes starts with empty permitted and effective sets;
  every `bash` the sampler saw after it reads 0.
  Kernel-side reading [inferred, from the citation in `crates/kf-cuda/src/posture.rs`: `ogkm-580: kernel_channel.c:277-291`,
  `escape.c:304`]: with `CapEff` 0 on every QEMU thread, host RM's `capable(CAP_SYS_ADMIN)` checks are false for every call
  kf3 made, so every RM call of these runs was one an unprivileged client may make. The kf3 logs agree: 161 channel births
  in the suite and 151 in the Windows boot 2 log, all `PRIVILEGED_CHANNEL=0 privilege=USER`; 90 `cap_sys_admin=not-held`
  posture lines (`ev/gate_b2_kf3_keylines.txt`, `ev/fast_rlsuite_timer_qemu.log.gz`).
  S1-22 (under a root QEMU only births and libcuda threads drop `CAP_SYS_ADMIN`) is sidestepped by running QEMU rootless;
  this run says nothing about a root QEMU and does not close S1-22.
- **H3 (a missing permission is named): CONFIRMED for N1 and N3 at `ae1ebd74`; N2 at `ae1ebd74` met the written
  expectation (the CUDA init error) but named neither the node nor the permission**, so the falsifier's second clause
  applies to that run; at `f00cf99f` N2 names `/dev/nvidia-uvm` and the errno. No control passed (no silent fallback).
  N3's QEMU text says "No such file or directory" for a node that exists and is denied; that wording is upstream QEMU's
  and was not examined further. Hugepages and VFIO: not in the run path; no control.
- **B1 (recorded):** one host line `NVRM: mmuWalkMap: Failed to map VA Range 0x120000000 to 0x30f9fffff. Status =
  0x00000051` at 05:20:41Z, in the Windows boot 2 window (`ev/gate_rlwin_b2_host_dmesg.txt`), preceded there by RM
  `nvAssertFailedNoLog` lines. The gate passed regardless. Also in the kernel log: 5 `Xid 31` (CE0 MMU fault, `qemu-system-x86`)
  at 04:19-04:22Z, inside the suite window, none in a Windows window [inferred: the suite's deliberate-fault arms].

## Fixes made here (both found by this run)

1. `crates/kf-qemu/src/device.rs` (`walker_refusal`, `open_uvm_rw`): a `cuInit` refusal at realize now names
   `/dev/nvidia-uvm` and the io error when the node does not open read-write. GPU-free tests:
   `device::walker_refusal_tests` (3; the first fails when the check is disabled, run 2026-11-25). Security note: the new
   code opens and closes `/dev/nvidia-uvm` once, only after `cuInit` already refused, on QEMU's own realize path; no guest
   input reaches it.
2. `scripts/bench/windows/win_vm.sh` (`tpm_manufacture`): when `/var/lib/swtpm-localca` is not writable by this uid,
   `swtpm_setup` gets `--config` with a CA kept under the VM directory (`$VM/tpm-ca`). Ubuntu 22.04's `swtpm_setup` 0.6.3 has
   no `--create-config-files`. Harness only; the EK and platform certificates of such a VM are signed by that per-VM CA.

## What still needs root (box setup, not the VM run)

Host driver install and module load (`provision_full.sh`), `apt-get` of the Windows packages and the swtpm AppArmor rule
(`windows_gate.sh --prepare`), `useradd`. For the bench harness only: `kernel.dmesg_restrict=0` (the gate reads host Xid
lines), the shared GPU lock file mode 0666, ownership of `/workspace/bench`. Nothing in the VM run itself.

## Caveats

- One box, one die, one driver, one run per stage. The sudo-window check of the gate (`rootless_final.json`: 0 entries)
  is vacuous here: no sudoers entry exists and `NoNewPrivs` blocks setuid.
- The QEMU objects outside the Rust archive were compiled by root's provisioning build and relinked by `kfrun`
  (`build_kf3.sh` reuses `/workspace/bench/qemu-build-kf3`); the Rust archive and the binary were built by `kfrun`.
- The first N2 and the first install wrote their logs to paths the second runs reused; their key lines were copied from
  the operator's terminal into `ev/neg_N2a_refusal.txt` and `ev/wininstall_a_tpm_refusal.txt` before that.

## Files

`PREREGISTRATION.md`; `procsample.py`, `run_proof.py`, `stage.sh`, `setup_root.sh` (the procedure); `ev/` (text only,
scrubbed of GPU and VM UUIDs and IP addresses).
