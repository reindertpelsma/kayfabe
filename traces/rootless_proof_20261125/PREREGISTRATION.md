# Rootless proof on one box: hypotheses and falsifiers written before the run (2026-11-25)

**STATUS: RESEARCH, 2026-11-25. Written and committed BEFORE any hardware run of this directory.** Results go in
`README.md` beside it; this file is not edited after the first run except by a dated addendum at the end.

Purpose: OWNER_RULINGS §BC item 6 (the rootless run proves the VM run needs no root) and the open half of the stage-2
audit item S2-04d (`docs/audits/2026-11-23-v3-stage2.md`). `docs/design/V3_KFROOT_ROOTLESS.md` has not run on a box. The
earlier record is a bonus probe of 2026-11-15 (`traces/release_manual_rehearsal_20261115/README.md`, rev `a76d1397`, Vast
60728222): a dropped uid in group `kvm` passed `fast_suite.sh` 30/30 and one guest CUDA arm; the Windows guest was not run
rootless and no QEMU `/proc` status was recorded (only the wrapper's own).

## What the documents require of the user (read 2026-11-25 at `origin/next` `03d2a126`)

- `docs/howto/installing.md`, "Permissions": QEMU opens `/dev/kvm`, `/dev/nvidiactl`, `/dev/nvidia<n>`, `/dev/nvidia-uvm`;
  run it as an ordinary user; the earlier run's user had group `kvm` only and relied on NVIDIA's default mode 0666 for the
  `nvidia*` nodes; `nvidia-modprobe -c 0 -u` creates missing nodes. **No `render` or `video` group is named.** This run
  therefore gives the user group `kvm` only and does not change any `nvidia*` node mode; it records the modes it found.
- `docs/howto/building.md`: only the package install (`apt-get`) needs root; the build kit needs none.

## The run (procedure fixed here)

One Vast KVM box (RTX 4090, 24 GB; `vms_enabled=true`, image `vastai/kvm:ubuntu_cli_22.04`), label prefix `kf-rootless-`,
cap 3.5 USD and 3 box-hours. Tree: this branch's head when the box clones it (named in `README.md`).

**Box setup, as root (not part of the claim; each step listed with why):** `provision_full.sh` (host driver 580.159.04,
QEMU source tree, fast guest, a kf3 build); `windows_gate.sh --prepare` (apt packages and the swtpm AppArmor rule, the two
root steps `V3_KFROOT_ROOTLESS.md` §4 lists); then `setup_root.sh` of this directory: record `sudo --version`, the
`/dev/kvm` and `/dev/nvidia*` modes, `dmesg_restrict`, `protected_regular`; `nvidia-modprobe -c 0 -u` only if
`/dev/nvidia-uvm` is missing (the documented step); `useradd --create-home --groups kvm kfrun` (no sudoers entry, no
`kfroot install`, no other group); a copy of the checkout at the same revision owned by `kfrun`; `chown` of the bench tree
to `kfrun`; the harness's GPU lock file mode 0666 and `kernel.dmesg_restrict=0` (both needed by the bench harness, not by
QEMU: the gate reads host Xid lines); an empty `/var/log/kfroot-sudo.log` readable by `kfrun` (the gate's final sudo-window
check reads it; with no sudoers entry and `NoNewPrivs` no sudo can elevate, so that check is vacuous here and is not
counted as evidence); root's kf3 build is moved aside so that the kf3 binary under test is the one `kfrun` builds.

**Every kayfabe stage runs dropped:** `prlimit --memlock=8388608:8388608 python3 -I scripts/ci/kfroot.py run --user kfrun
--no-memlock -- bash stage.sh <stage>`. As root, `kfroot.py run` execs `setpriv --reuid kfrun --regid kfrun --groups kvm
--inh-caps=-all --ambient-caps=-all --bounding-set=-all --no-new-privs` (no sudo). RLIMIT_MEMLOCK is set to 8 MiB, the
systemd default a login session gets, not raised (the audit's open memlock question). Stages, in order: `toolchain` (a
per-user rustup through `kf_trust.py`), `build` (`build_kf3.sh`), `neg_*` (H3), `wininstall` (`win_vm.sh install`, no kf3
device) with `suite` (`fast_suite.sh rlsuite 180`) beside it, then `wingate` (`windows_gate.sh` with
`KF_ROOTLESS=require`, `KF_WINGATE_IDLE_S=120`, default flags: `display=on,gop=on`). `procsample.py` (as root, reading
`/proc` only) records every thread of every `kfrun` process and every `qemu-system-*`, `swtpm`, `kf-broker*` process of any
uid during each stage.

## Hypotheses and falsifiers

**H1 (works rootless).** As `kfrun` (group `kvm` only, no capability, NoNewPrivs, memlock 8 MiB, nvidia node modes as the
driver left them), the kf3 QEMU (a) boots the Linux fast guest and `fast_suite.sh` reports `FAST_SUITE_PASS=30`, and (b)
boots the Windows 11 LTSC guest with `display=on,gop=on` and the gate's guest `nvidia-smi` check names the GPU
(`WINDOWS_GATE_PASS`, or at least the gate's boot-1 and driver checks passing with `nvidia-smi` naming the card).
*Falsifier:* any step of the dropped stages that fails because it needs root, a capability, a group other than `kvm`, a
node mode other than the driver's default, or a memlock limit above 8 MiB. Each such refusal is recorded by its message.
A failure with another cause (rental, media download, a guest defect seen also as root) is not a falsification and is
recorded as such. *Fallback fixed now:* if memlock is the only cause, the stage is re-run once with
`--memlock=unlimited` and both results are reported (H1 is then false as stated).
*Concurrency rule:* the suite runs while the Windows install (CPU only, no GPU) runs; an arm that ends `TIMEOUT` is re-run
alone once after the install and both rows are reported.

**H2 (no privilege in the tree).** For every QEMU thread, every swtpm thread and every other `kfrun` thread seen:
`CapInh`, `CapPrm`, `CapEff`, `CapBnd`, `CapAmb` = `0000000000000000`, `NoNewPrivs: 1`, and no uid field 0; no
`qemu-system-*` or `swtpm` process of any uid other than `kfrun` exists during the stages (the broker is not used by these
stages: `display=on` without a broker). The gate's own `KFROOT_ASSERT` lines (boot 1: QEMU and swtpm pid files) agree.
*Falsifier:* any `PROCSAMPLE_SUMMARY` violation, a `KFROOT_ASSERT ... ok=0`, or a root-owned QEMU or swtpm.
Consequence noted in advance: with `CapEff` 0 on every QEMU thread, host RM's admin checks (`capable(CAP_SYS_ADMIN)`,
cited in `crates/kf-cuda/src/posture.rs` as `ogkm-580: kernel_channel.c:277-291`, `escape.c:304`) are false for every call,
so a PASS means every RM call kf3 made succeeded without the admin privilege [inferred, from that citation; not a separate
measurement]. The kf3 log's birth lines (`PRIVILEGED_CHANNEL=0 privilege=USER`) are counted as a second reading.
The open item S1-22 (under a root QEMU only channel births and libcuda threads drop `CAP_SYS_ADMIN`) is sidestepped by a
rootless QEMU, not fixed; this run says nothing about a root QEMU.

**H3 (a missing permission is named, not silently worked around).** Each control is one fast-guest arm (`--timer`) as
`kfrun`, with one change made by root before it and undone after it:
- N1: `/dev/nvidiactl` mode 0600 (root-owned). Expect: kf3 refuses at realize with a message that names `nvidiactl` and the
  permission error; the arm does not pass.
- N2: `/dev/nvidia-uvm` mode 0600. Expect: a failure that names `nvidia-uvm` or the CUDA init error, or the arm passes and
  no kf3 path needed UVM for it (recorded as "not needed by this arm", not as a pass of H3).
- N3: the user without group `kvm` (`setpriv` with `--clear-groups` instead of `kfroot.py run`). Expect: QEMU refuses with
  the KVM permission message.
- Hugepages and VFIO are not in kayfabe's run path (`V3_KFROOT_ROOTLESS.md` §4: memfd RAM, no hugepages, no VFIO); no
  control is run for them, and that is said.
*Falsifier:* a control in which the arm passes while kf3 or QEMU logged nothing about the denied node (a silent fallback),
or fails with a message that names neither the node nor the permission.

**B1 (recorded, not a hypothesis):** the host kernel log's `mmuWalkMap ... 0x51` lines during the Windows boots are
counted (finding B1 of 2026-11-24, `traces/pc_hwgates_20261124/README.md`; not this run's to fix).

## Evidence kept

Text only: `prov.log` heads, `setup_root.txt`, per-stage logs with the wrapper's `/proc/self/status` lines and the
`ROOTLESS_STAGE_EXIT` line, the `procsample` JSONL (compressed) and summaries, the suite output, the gate log and its
`rootless_*.json`, kf3 key lines. Scrubbed: GPU and VM UUIDs, PARTUUIDs, serials, IP addresses.
