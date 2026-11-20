# V3: the audit of every `kfroot` step (are you really sure a user has to run this?)

STATUS: LIVE as an audit, 2026-11-20; read from the tree at `7fb1c98a0` (`origin/next`), NOT RUN ON A BOX. Every verdict below is code or
documentation read, with the file and line it rests on; a verdict that needs a run says NOT YET RUN and names the smallest hardware check
(section 7). Nothing here was measured on hardware by this audit. The data form of this table is `STEPS` and `STAGE_POLICY` in
`scripts/ci/kfroot.py` (`python3 scripts/ci/kfroot.py steps`); `scripts/ci/test_kfroot_payload.py` fails when the two disagree.
Parent: `docs/design/V3_KFROOT_ROOTLESS.md` (the mechanism). Ruling: `docs/OWNER_RULINGS.md` section AR item 6.

## 1. What the owner asked, and how it is read

The rootless run is a TEST. If every kayfabe QEMU of the CI runs as an unprivileged user, no NVIDIA ioctl needed root for anything tested; if
one did not, the test failed. `kfroot` is the explicit approval of one root step ("this command was needed to run this CI and needed
root"). The goal is to REDUCE `kfroot`: each step is asked "are you really sure users do not have to run this to get that app working
rootlessly?". In production there is no `kfroot`. The run of the VM itself is rootless; setup steps may use `kfroot`, minimal and documented.

Classes (the first three are the owner's; the fourth is added because three steps are neither setup nor convenience):

| Class | Meaning | What happens to it |
|---|---|---|
| `SETUP-ONCE` | a user does this once to get kayfabe or the app working (driver, packages, group, AppArmor rule) | stays a `kfroot` step; the unprivileged equivalent is listed in section 6, "to be documented in the manual" |
| `VM-RUN` | a root need during the VM run | a kayfabe BUG or a harness bug: the row says which; never a `kfroot` step that is accepted |
| `CI-CONVENIENCE` | the CI does it for its own sake (ownership of a work tree, an instrument, an insurance setting) | to be removed; the row says how, and what keeps it today |
| `TEST-MECHANISM` | the test's own apparatus: the account, the drop, the sudo log | stays, outside what a user does; it is what makes the run a test |

A step is also left out of a run when its effect is already there (`kfroot.py bootstrap --only-needed`, section 8): a step that is not run is
not a `kfroot` step of the run.

## 2. A. The explicit `kfroot <cmd>` lines of the harness (today: 4 call sites and the drop)

The owner's note counted about 26 lines in 7 files; `git grep` of the tree at `7fb1c98a0` finds these call sites (the other mentions are
comments, tests and documents):

| Step | Where | What it does | Why root | Class | Evidence and verdict |
|---|---|---|---|---|---|
| `apt_prereqs` | `scripts/bench/box/windows_gate.sh:677` (`install_prereqs`, :662) | `apt-get install` of ovmf, swtpm, swtpm-tools, sbsigntool, python3-virt-firmware, xorriso, netpbm, p7zip-full, mingw | dpkg database and `/usr` | `SETUP-ONCE` | A user needs these packages for the Windows lane (`windows_gate.sh:663-667` checks them). The payload's provision profile `ci` does NOT install them (`scripts/bench/provision_box.sh:42`: `ci` prints only `required`; the `windows` list is :33-34), so in the payload flow this is the only place they arrive. Verdict: really needed, once, as root. Now run by `windows_gate.sh --prepare` before the drop (:684-690) |
| `swtpm_apparmor_rule` | `scripts/bench/windows/win_vm.sh:288` (`kfroot tee -a`, `apparmor_swtpm` :276) | appends `$WINVM_ROOT/** rwk,` to `/etc/apparmor.d/local/usr.bin.swtpm` | `/etc/apparmor.d` is root-owned | `SETUP-ONCE` | Comment at :277: "jammy's swtpm runs under an enforced AppArmor profile scoped to libvirt paths". Skipped when the profile is absent (:282, `[ -e "$prof" ]`) or the rule exists. A user on an AppArmor distro needs it once. NOT YET RUN: whether a swtpm state path inside the profile's allowed set exists that a user may write (which would remove the step): read `/etc/apparmor.d/usr.bin.swtpm` on the box image |
| `swtpm_apparmor_load` | `scripts/bench/windows/win_vm.sh:289` (`kfroot apparmor_parser -r`) | loads the profile into the kernel | CAP_MAC_ADMIN | `SETUP-ONCE` | as the row above |
| `dmesg_via_kfroot` | `scripts/lib/kfroot.sh` `kf_dmesg` (`kfroot dmesg`; the callers are `windows_gate.sh`, `windows_apps.sh`, `win_vm.sh`) | reads the kernel log for the host Xid check | `kernel.dmesg_restrict=1` needs CAP_SYSLOG | `CI-CONVENIENCE` | An app does not need the kernel log; the Xid watch is the CI's evidence instrument (`scripts/bench/host_xid_watch.sh` header: an unreadable log must be "unwatched", never clean). Kept as the one instrument, but the dropped tree cannot use `kfroot` (NoNewPrivs), so it only works with `dmesg_readable` below. A user who wants Xid diagnostics sets `kernel.dmesg_restrict=0` or reads `journalctl -k` as group `adm`/`systemd-journal` (not measured) |
| `drop` | `scripts/ci/kfroot.py` `run_prefix` (`kfroot prlimit ... setpriv ...`; :199 `drop_argv`, :217 `run_prefix` at the base tree) | starts the stage as the user, every capability set empty, NoNewPrivs | setuid, setgroups, bounding set | `TEST-MECHANISM` | This is what turns "runs as root" into "runs rootless"; a user is already unprivileged. As root (the payload) no `sudo` is involved |

## 3. B. The box bootstrap (`kfroot.py bootstrap`, `plan()`): 17 steps, now needed-checked

Source: `plan()` of `scripts/ci/kfroot.py` (base tree :439-481). "Needed-check" = left out with `--only-needed` when its effect is already there.

| Step | What | Class | Evidence and verdict | Needed-check |
|---|---|---|---|---|
| `kfroot_files` | `/usr/local/bin/kfroot`, `/usr/local/sbin/kfroot-wrapper`, `/etc/sudoers.d/kfroot` | `TEST-MECHANISM` | exists so a NON-root caller can name and log its root steps; a user has none. The payload is root and `kfroot` then runs the command directly (`scripts/lib/kfroot.sh`: as root, `"$@"`), so the payload flow does not install it (a box without sudo is not a failure) | no (not run by the payload) |
| `group_kvm` | `groupadd --system kvm` if missing | `CI-CONVENIENCE` | a distro with KVM has the group; only a minimal image lacks it | yes: skipped when the group exists |
| `user` | `useradd`/`usermod -aG kvm` for the CI account | `TEST-MECHANISM` | a user is the account they log in as; the account is the test's subject | no |
| `dev_kvm_group`, `dev_kvm_mode` | `chgrp kvm /dev/kvm`, `chmod 0660` | `SETUP-ONCE` | a user needs group membership and a `/dev/kvm` they may open (`windows_gate.sh:710` refuses by name `no-/dev/kvm-access`). Rehearsal: "`/dev/kvm` group `kvm`" on the Vast image (`traces/release_manual_rehearsal_20261115/README.md:36`) | yes: skipped when the node is group kvm and group read/write |
| `dev_kvm_missing` | records an absent `/dev/kvm` | `SETUP-ONCE` | not a command; a provider fault otherwise (`require_provider_kvm`, `scripts/ci/vast_payload.py`) | n/a |
| `nvidia_node` (per GPU), `nvidia_uvm`, `nvidia_modeset` | `nvidia-modprobe -c N`, `-u -c 0`, `-m` | `CI-CONVENIENCE` | `nvidia-modprobe` is a setuid helper that a user's first `nvidia-smi` or CUDA call runs; the test's NoNewPrivs defeats setuid, which is why the bootstrap pre-creates the nodes (`windows_gate.sh:711` comment). In the payload flow the nodes already exist after `provision`, which runs `nvidia-modprobe -c 0 -u` as root (`scripts/bench/provision_host_driver.sh:148,300`). Verdict: induced by the test's own NoNewPrivs, probably redundant after provision. NOT YET RUN (check: section 7, item 1) | yes: only a missing node |
| `nvidia_node_perms` | `chmod a+rw` of the nodes | `CI-CONVENIENCE` | "`/dev/nvidia*` were mode 0666" on the rehearsal image (`README.md:36`, MEASURED there) | yes: only a node that is not world read/write |
| `dmesg_readable` | `sysctl -w kernel.dmesg_restrict=0` | `CI-CONVENIENCE` | the Xid instrument (`host_xid_watch.sh`, every stage) needs a readable log; not an app need. Kept until the Xid watch reads the log another way | yes: only when restricted |
| `memlock_limit` | `/etc/security/limits.d/kfroot.conf`: `memlock unlimited` | `CI-CONVENIENCE` | The question "do users have to raise RLIMIT_MEMLOCK" is UNSETTLED but the evidence points to no: the rootless rehearsal ran `cuda_ladder.sh` host and guest and `fast_suite.sh` 30/30 at the default limit of 12966280 KiB (`README.md:34-37`); the tree has no mlock and no hugepages (`docs/design/V3_KFROOT_ROOTLESS.md` section 4, `win_vm.sh:552` memfd). Not shown for a Windows guest with large RAM. Verdict: insurance; the payload no longer raises it (`--rootless-memlock` to turn it back on) so a run that needs it fails and says so | off unless asked |
| `workspace` | `install -d -o ci /workspace /workspace/bench /workspace/winvm /workspace/winapps /workspace/prov` | `CI-CONVENIENCE` | a user works in their own home or a tree they own; this is the CI's layout (`/workspace` is created by root). Removal: move the work trees under the user's home (many scripts hard-code `/workspace`: not done) | yes: only when a directory is missing or not the user's |
| `chown_tree` | `chown -R ci:ci <checkout, bench, prov, evidence, apps>` | `CI-CONVENIENCE` | the root `provision` stage builds `target/`, QEMU, the fast guest as root (`provision_full.sh:54-66`), so the user cannot write there. Removal: split provision into a root half (apt, driver, modules) and a user half (cargo, QEMU, fast guest built as the user). NOT YET RUN (check: section 7, item 2) | no (also re-run after an approved root stage) |
| `gpu_lock` | creates `/tmp/kayfabe-fastguest.lock` mode 0666 | `CI-CONVENIENCE` | rehearsal harness defect (1): a root-created lock was refused to the user (`README.md:39`). With every stage the same uid the first stage creates it (`scripts/bench/cuda_ladder.sh:78`, `fastguest/run_fast_guest.sh:69`) | yes: only a missing or non-0666 lock |
| `sudo_log`, `sudo_log_acl` | the root-owned audit file the CI user may read (sudo.ws) or an ACL on `auth.log` (sudo-rs) | `TEST-MECHANISM` | the audit of the test's own `kfroot` calls; a user has none | no |

## 4. C. What the payload does as root without a `kfroot` line (now one line per stage, in `STAGE_POLICY`)

Before this audit `scripts/ci/vast_payload.py` started every stage as root (`execute_stages` ran each command directly). Now, with
`--rootless require` (`docs/CI_V3.md`, "Rootless payload run"), each stage has a kind:

| Stage | Kind | Class of its root need | Evidence | Status |
|---|---|---|---|---|
| `provision` | `setup` (root, one recorded step `provision`) | `SETUP-ONCE` for apt, the host driver, modules and services (`provision_host_driver.sh:85,119,125,143-148,189-200,276,300`; `provision_box.sh:126`); `CI-CONVENIENCE` for the part that builds kayfabe as root (`provision_full.sh:54-66`: `provision_bench_tree.sh`, `cargo build`, `build_fast_guest.sh`, `build_kf3.sh`), which forces `chown_tree` and `toolchain_for_user` | the scripts cited | NOT YET RUN split |
| `cuda_host`, `cuda_guest` | `user` | none found: `cuda_ladder.sh` has no root operation (the static check reads it); rehearsal ran `cuda_ladder.sh host` cup2/cup8 and `guest` cup2 as uid 1002 with no capability: PASS (`README.md:34`) | `traces/release_manual_rehearsal_20261115/README.md:31-37` | `probed` by hand, not through the payload |
| `merge_bar` | `root_stage` (needs `--rootless-root-stage merge_bar`) | the rebuild of the fast guest per run: `scripts/bench/box/merge_check.sh:132` runs `scripts/fastguest/build_fast_guest.sh`, which does `modprobe nbd` (:177), `qemu-nbd --connect` (:179) and `mount -o ro` (:207, :214, :234). This is a build step, not the VM run: harness, not kayfabe. Class: `SETUP-ONCE` in nature (provision already builds the same artifact, `provision_full.sh:65`), `CI-CONVENIENCE` as a per-run repeat. The 30-arm suite itself ran rootless (`README.md:35`). Also: `merge_check.sh:56` lock, `:58` `$HOME/.cargo`, git ownership | read | NOT YET RUN (section 7, item 3) |
| `apps` | `root_stage` | `scripts/apps/apps_matrix.sh:3` says "(as root)"; its host side does `mkdir -p /opt/apps` and `cp` into it (:25-34) and writes `/workspace/apps/results` (:15), which the bootstrap does not create. Harness | read | NOT YET RUN |
| `apps_discovery`, `kfml` | `user` | none found in the entry scripts (`apps_discovery_stage.py`, `kfml_stage.py`); they reuse the `apps` bundle under `/workspace/apps` (given back to the user after a root stage) | static check | NOT YET RUN |
| `install_flow` | `self_drop` | a root launcher that drops its own VM tree: `scripts/ci/install_flow.py:418` (`not_root` refusal), `:316-334` (`guest_access`: key copy, `chmod` traversal, `nvidia-modprobe`), `:427` `require_unpriv=True`. The VM is dropped (it has its own `drop_argv`, :165); the launcher steps are setup. It records its own root steps (`:318`) | read | NOT YET RUN |
| `windows_gate`, `windows_gate_2`, `windows_apps`, `windows_open_rows` | `user` with `windows_gate.sh --prepare` first | section 2 rows `apt_prereqs`, `swtpm_apparmor_*`; the rest of the Windows harness is already rootless-ready by `docs/design/V3_KFROOT_ROOTLESS.md` section 4 (no nbd, loop, tap, hugepages; `win_vm.sh` runs `qemu-img create`, `swtpm_setup`, `sbsign` as the user). Needs the user's Rust toolchain (`windows_gate.sh:719`: `no-cargo-for-uid-<n>`) | `docs/design/V3_KFROOT_ROOTLESS.md` section 4 | NOT YET RUN |
| `propimage_apps`, `propimage_apps_b`, `matlab` | `root_stage` | all three call `build_swtpm` of `scripts/ci/propimage_apps.py:661`: `apt-get install` (:669) and `make install` under `/opt/tpm` (:86). `SETUP-ONCE` for the packages; `CI-CONVENIENCE` for the prefix (a `$HOME` prefix needs no root). Harness | read | NOT YET RUN |
| `specviewperf` | `root_stage` | `scripts/bench/box/specviewperf_stage.sh:485`: `[ "$(id -u)" = 0 ] || abort OPERATOR "not root"`, and a raw `dmesg` at :315. Nothing cited shows the stage needs root: a blanket refusal of the kind `windows_gate.sh` dropped on 2026-11-15. This is a harness bug (class `VM-RUN` if it stays: it runs a VM). Conversion = what `windows_apps.sh` got | read | NOT YET RUN |

Six stages stay `root_stage` (`merge_bar`, `apps`, `propimage_apps`, `propimage_apps_b`, `matlab`, `specviewperf`); each needs the owner's
or the coordinator's explicit approval by name in a run, is recorded as the `VM-RUN` step `root_stage`, and makes the run's verdict
`ROOT_STAGES_APPROVED` and not `ROOTLESS`. `install_flow` is `self_drop`: it needs no approval and is recorded.

## 5. D. Root that is the box's own lifecycle (not counted: outside what the test proves)

`scripts/ci/vast_onstart.sh` runs as root by design (Vast starts the container as root): `apt-get install curl python3 git` (:36), the DNS
fallback (:32), `install` of `kf-box-kill` (:58), the reporter and the evidence bundle. The payload launcher itself (`vast_payload.py`) makes
`/root/kayfabe` and copies `provision_box.sh` to `/root` (`vast_payload.py:2156-2161`). The stages are what this work drops; the lifecycle is not a
kayfabe step and is not part of the claim "kayfabe's QEMU needs no root".

## 5b. Findings of the audit (defects, each with the file and line)

| # | Finding | Evidence | Class | Status |
|---|---|---|---|---|
| X1 | `host_xid_watch.sh` read the kernel log with raw `dmesg` (:94, :111 at `7fb1c98a0`). The payload wraps EVERY stage in it (`watched()` in `vast_payload.py`), so a dropped stage with `kernel.dmesg_restrict=1` ended as UNWATCHED (exit 7) before its command ran: the rootless mode of the payload could not have worked for any stage | `scripts/bench/host_xid_watch.sh:94,111`; `scripts/ci/vast_payload.py` `watched` | harness bug (`CI-CONVENIENCE` instrument) | fixed in the code PR: `kf_dmesg`, UNWATCHED (never clean) with a `KFROOT_REFUSED dmesg` line when it cannot read |
| X2 | `specviewperf_stage.sh` refuses `not root` with no cited need | `scripts/bench/box/specviewperf_stage.sh:485` | harness bug (`VM-RUN` class while it stays) | open: `root_stage`, conversion listed in section 7 |
| X3 | `merge_check.sh --fast` rebuilds the fast guest per run through `qemu-nbd` and `mount`, a root build step that provision already did | `scripts/bench/box/merge_check.sh:132`; `scripts/fastguest/build_fast_guest.sh:177-234`; `scripts/bench/box/provision_full.sh:65` | `CI-CONVENIENCE` repeat of a `SETUP-ONCE` step | open: `root_stage` |
| X4 | `apps_matrix.sh` host side writes `/opt/apps` and `/workspace/apps/results`, neither of which the bootstrap makes the user's | `scripts/apps/apps_matrix.sh:15,25-34` | harness | open: `root_stage` |
| X5 | the swtpm build of the image stages installs packages and writes `/opt/tpm` as root | `scripts/ci/propimage_apps.py:661-669,86` | `SETUP-ONCE` (packages), `CI-CONVENIENCE` (prefix) | open: `root_stage` for three stages |
| X6 | the count: the tree has 4 `kfroot` call sites plus the drop, not about 26 lines (the rest are comments, the library and tests) | `git grep` at `7fb1c98a0` | n/a | recorded |
| X7 | no kayfabe bug is claimed or excluded: this audit read the harness only, not `qemu/hw/misc/kf3`, `crates/` or the QEMU command lines; a root need during a VM run is found by a run (section 7, item 7), and the one hand run that exists (`traces/release_manual_rehearsal_20261115/README.md:31-37`) needed none for the bare-metal and kf3-guest ladders and the 30-arm suite | `README.md:34-37` | `VM-RUN` | NOT YET RUN for the Windows lane |

## 6. Counts and the unprivileged equivalents

Registered steps (`STEPS`): 25. By class: `SETUP-ONCE` 7 (`apt_prereqs`, `swtpm_apparmor_rule`, `swtpm_apparmor_load`, `dev_kvm_group`,
`dev_kvm_mode`, `dev_kvm_missing`, `provision`), `CI-CONVENIENCE` 12 (`dmesg_via_kfroot`, `group_kvm`, `nvidia_node`, `nvidia_uvm`,
`nvidia_modeset`, `nvidia_node_perms`, `dmesg_readable`, `memlock_limit`, `workspace`, `chown_tree`, `gpu_lock`, `toolchain_for_user`),
`TEST-MECHANISM` 5 (`drop`, `kfroot_files`, `user`, `sudo_log`, `sudo_log_acl`), `VM-RUN` 1 (`root_stage`, covering six stages). Stage kinds
(`STAGE_POLICY`, 16 stages): `setup` 1, `user` 8 (2 probed by hand, 6 NOT YET RUN), `self_drop` 1, `root_stage` 6.

To be documented in the manual (flagged, not written here: `docs/howto/` is the published manual and nothing in it changes with this audit):

| Step a user does | The unprivileged equivalent |
|---|---|
| packages | `sudo apt-get install ovmf swtpm swtpm-tools sbsigntool python3-virt-firmware xorriso netpbm p7zip-full gcc-mingw-w64-x86-64 g++-mingw-w64-x86-64`, once |
| `/dev/kvm` access | `sudo usermod -aG kvm $USER` and log in again; a udev rule `KERNEL=="kvm", GROUP="kvm", MODE="0660"` where the distro lacks it |
| GPU device nodes | none: the first `nvidia-smi` or CUDA call creates them through the setuid `nvidia-modprobe` (only a NoNewPrivs tree needs them pre-created) |
| swtpm under AppArmor | one line `<state dir>/** rwk,` in `/etc/apparmor.d/local/usr.bin.swtpm`, then `sudo apparmor_parser -r /etc/apparmor.d/usr.bin.swtpm` (NOT YET RUN: whether a state directory already inside the profile removes it) |
| locked memory | none shown to be needed; if a run fails on a locked allocation: `<user> - memlock unlimited` in `/etc/security/limits.d/`, or `LimitMEMLOCK=infinity` in the systemd unit (to be documented only when measured) |
| kernel log for Xid diagnostics (optional) | `sudo sysctl -w kernel.dmesg_restrict=0`, or `journalctl -k` as a member of `adm` |
| host driver | the distro's or NVIDIA's own driver install, once; kayfabe is built and run as the user |

## 7. NOT YET RUN: the smallest hardware check for each open point

1. **GPU nodes and `nvidia_*` steps.** On a box after `provision`: `ls -l /dev/nvidia*`; run `kfroot.py bootstrap --only-needed --dry-run`. If no `nvidia_node*`, `nvidia_uvm`, `nvidia_modeset` or `nvidia_node_perms` step is listed and `cuda_ladder.sh host` passes dropped, those four classes are removed from the audit.
2. **Provision split.** Build `kayfabe-rm-ladder`, the fast guest and the kf3 QEMU as `ci` in `provision_full.sh` steps 54-66 instead of root (the driver and apt stay root); if `cuda_host` and `windows_gate` then pass with no `chown_tree` and no `toolchain_for_user` step, both go.
3. **merge_bar.** Run `merge_check.sh --gates --kf3 --host --census` as `ci`; run `fast_suite.sh` as `ci` against the provision-built fast guest (already PASS by hand, `README.md:35`); then decide whether `merge_check.sh --fast` may skip `build_fast_guest.sh` or replace `qemu-nbd` plus `mount` by `qemu-img convert -O raw` and `debugfs -R rdump` (not tried).
4. **Windows stages.** `python3 scripts/ci/vast_payload.py --rootless require --only windows_gate` (with the plan variables) on a rented box; read `KFROOT_ASSERT` in the log, the `rootless` block of `kf-result.json` (`verdict`, `steps`, `refused`) and `/var/log/kfroot-sudo.log` (must hold nothing for a root-launched payload). First failures expected: `no-cargo-for-uid` (the toolchain copy), memlock, swtpm AppArmor.
5. **memlock.** The Windows gate dropped without `--rootless-memlock`: its `GATE_NOTE host: ... memlock_kb=` line (`windows_gate.sh` preflight) and any `ENOMEM` on a locked allocation decide.
6. **The unconverted stages.** For each `root_stage`: run it dropped by hand, take the first refusal, convert, repeat; `specviewperf_stage.sh:485` first (a guard, not a need).
7. **VM-run.** Any `KFROOT_REFUSED` in a dropped stage's log, or an `EACCES` on `/dev/kvm` or `/dev/nvidia*`, is a root need during the VM run: decide kayfabe bug or harness bug by the first failing path (a path under `qemu/hw/misc/kf3` or `crates/` is kayfabe's).

## 8. What this change did (and did not)

Done (GPU-free, tested by `scripts/ci/test_kfroot_payload.py`, killed by `scripts/ci/kfroot_mutants.py`):
- `scripts/ci/kfroot.py`: the audit as data (`STEPS`, `STAGE_POLICY`), the static check (`unmarked_root_ops`, `lint_stage`, `stage_refusals`), the record (`Record`, extended not duplicated: it holds the steps, the stages and the refusals), the orchestration (`Rootless`), `plan(only_needed=, memlock=)` and `bootstrap(install_files=False)` (the payload is root: no launcher, no sudoers file).
- `scripts/ci/vast_payload.py`: `--rootless require` (flag or `KF_PAYLOAD_ROOTLESS`), `--rootless-user`, `--rootless-root-stage`, `--rootless-memlock`. Each non-setup stage starts through `kfroot.py run` as the user; a stage that needs root and was not approved is refused by name before anything starts; the record goes into `kf-result.json` as `rootless` (read by `scripts/ci/vast_report.py` `validate_rootless`).
- `scripts/lib/kfroot.sh`: `KF_KFROOT_RECORD`, one line per `kfroot` call (root, sudo or refused).
- `scripts/bench/host_xid_watch.sh`: reads the kernel log through `kf_dmesg`. Finding X1: before this change it called raw `dmesg` at :94 and :111, and it wraps every payload stage, so every dropped stage would have reported UNWATCHED (exit 7) with the log restricted.

Not done: the Worker does not send the new variables (`scripts/ci/vast_report.py` `SPEC`, `ci/vast-broker` untouched); the provision split (section 7, item 2); the six root stages; the manual text; nothing ran on a box.
