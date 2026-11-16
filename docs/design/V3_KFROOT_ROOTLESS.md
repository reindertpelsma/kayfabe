# V3: the Windows gate and the box harness without root, and the explicit `kfroot` step

STATUS: LIVE as code, 2026-11-15; NOT RUN ON A BOX. GPU-free tests and one hand run of the real `sudo` in a sandbox only (section 3).
Implements OWNER_RULINGS section AR item 6 (rootless proof; owner 2026-11-11 and 2026-11-15) for the Windows stages
(`scripts/bench/box/windows_gate.sh`, `windows_apps.sh`, `scripts/bench/windows/win_vm.sh`). Not done (section 8): the payload
(`vast_payload.py`, `vast_onstart.sh`) still starts every stage as root; nothing here has run on a GPU box.

## 1. What this is

The proof the owner wants is that kayfabe works in real production as an unprivileged user with no capability. This change makes the
Windows gate and the Windows apps stage runnable that way and records, per run, that they were:

- **No blanket root refusal.** `windows_gate.sh` and `windows_apps.sh` no longer say `refuse not-root`. They check what they need as a
  capability and refuse by name: `no-/dev/kvm-access`, `no-/dev/nvidiactl-access`, `not-writable-<dir>`, `no-cargo-for-uid-<n>`,
  `dmesg-unreadable`, then the existing disk, RAM and flags checks.
- **The few real root operations are `kfroot <cmd>` lines** (section 4), each with a comment that says why.
- **The kayfabe tree is dropped.** `kfroot.py run` starts the harness command as the unprivileged user with every capability set empty and
  `NoNewPrivs=1`; inside that tree `kfroot` is refused (rc 126), so a root dependency fails on the first change that adds it.
- **A per-run assertion** reads `/proc` and the sudo log and writes the result into the log, the evidence and the verdict line (section 5).

This is accident prevention in an untrusted sandbox, not security inside the box (the Worker is the boundary).

## 2. The mechanism (owner design, 2026-11-11), `scripts/ci/kfroot.py`

| File | Content |
|---|---|
| `/usr/local/bin/kfroot` (0755) | `#!/bin/bash` `exec sudo -n -E /usr/local/sbin/kfroot-wrapper "$@"` |
| `/usr/local/sbin/kfroot-wrapper` (root:root 0755) | `#!/bin/bash` `exec "$@"` |
| `/etc/sudoers.d/kfroot` (0440) | `Defaults:ci !secure_path`; `ci ALL=(root) NOPASSWD:SETENV: /usr/local/sbin/kfroot-wrapper`; plus `!use_pty`, `logfile=`, `loglinelen=0` |

Only the wrapper is whitelisted, so a third-party script that calls `sudo X` is refused. `install` validates the sudoers text with
`visudo -c -f` in a temporary file BEFORE it is put in `/etc/sudoers.d` (a missing or refusing `visudo` is a refusal; nothing is written).
Additions to the owner's three lines: `!use_pty` (design intent, not measured: no pseudo-terminal between the harness and the command) and
`logfile=/var/log/kfroot-sudo.log` with `loglinelen=0` (the audit file, readable by the CI user, one entry per line; the need for
`loglinelen=0` was seen in the hand run of section 3, 2026-11-15, and is pinned by `scripts/ci/test_kfroot.py`).

`kfroot.py` subcommands: `install`, `verify`, `bootstrap`, `drop-argv`, `run`, `audit-mark`, `audit-check`, `assert-tree`
(`python3 scripts/ci/kfroot.py --help`). `scripts/lib/kfroot.sh` is the shell glue the three harness scripts source: the `kfroot`
function (as root: runs the command directly; unprivileged: the `kfroot` program; in a NoNewPrivs tree: refused, rc 126), `kf_dmesg`, and
the assertion helpers. Logic is Python (CLAUDE.md).

## 3. Measured, in a sandbox, by hand (2026-11-15)

Hand run of 2026-11-15 in the coordinator sandbox of this session at tree `84dff90b6` plus this PR's files; no trace is committed, so
treat the list as a record of the author's hand run, not as evidence. It used real `sudo.ws` 1.9.17 (the original sudo; the box image
`vastai/kvm:ubuntu_cli_22.04` has 1.9.9, not tested) and `sudo-rs` 0.2.13, a throwaway user, real `visudo`, `setpriv`, `prlimit`.
No GPU, no `/dev/kvm` group test, no box. Observed in that run:

- sudo.ws: `kfroot id -u` prints 0; environment variables (`FOO`, `KF3_*`, `DEBIAN_FRONTEND`, `HOME`) and `PATH` arrive; stdin, stdout and
  stderr stay separate; the exit status is returned; argv with spaces, newline, quotes, `$(...)`, `;`, `*`, `-n`, an empty string and `--`
  arrives byte for byte. A stray `sudo -n /usr/bin/id` is refused ("a password is required") and logged.
- **`LD_*` variables do NOT arrive through sudo, even with `SETENV` and `-E`.** `kfroot.py run` therefore hands them to the command AFTER the
  drop (`... setpriv ... -- env LD_LIBRARY_PATH=... CMD`), where they bind only an unprivileged process.
- **sudo wraps a logfile line at 80 columns** unless `loglinelen=0` (`COMMAND=` landed on a continuation line). The sudoers file sets it, and the
  parser also joins continuation lines.
- **sudo-rs ignores `-E`** ("preserving the entire environment is not supported"): the environment and `PATH` do not arrive. It also refuses the
  `logfile` setting and does not log a refused stray `sudo` to `auth.log`. `kfroot.py verify` reports `sudo_flavor sudo-rs` as a problem; on
  such a box `install` falls back to a file without `logfile`, and the audit is `/var/log/auth.log` (unreadable by the dropped user without an ACL:
  then the audit fails closed). The Vast image's sudo (22.04) is the original.
- Inside `setpriv --no-new-privs`, `kfroot` fails ("The 'no new privileges' flag is set, which prevents sudo from running as root").
- `kfroot.py run --user U -- CMD`: CMD ran as U with `Groups: <kvm gid>` only, `CapEff/CapPrm/CapBnd = 0`, `NoNewPrivs: 1`, `ulimit -l` unlimited;
  `assert-tree` in require mode passed with `sudo_entries=1 sudo_violations=0` (the one `kfroot prlimit ... setpriv` call that made the drop), and
  turned to `sudo_violations=1` after a stray `sudo -n id`.
- The gate's preflight, started that way as an unprivileged user, printed `KFROOT_ASSERT mode=require ok=1 uid=1003 capeff=0 capprm=0 capbnd=0 nnp=1`
  and then refused by name (`not-writable-<checkout>`: the checkout was root-owned; `bootstrap --chown` is the fix).

## 4. Which operations really need root (the inventory)

Searched in `windows_gate.sh`, `windows_apps.sh`, `windows_lane.sh`, `win_vm.sh`, `propimage_run.sh`, `appmatrix/*` (`run_windows_apps.sh`,
`winapps.py`, `appdisk.py`, `qga.py`, `sshguest.py`, `fetch_public_bundle.py`, ...), `windows_broker.sh`: mounts, nbd, `qemu-img` on block devices,
`ip`/tap/iptables, `modprobe`, chown to other users, `/dev` nodes, swtpm sockets under `/run`, hugepages, writes to `/etc`, apt, driver install,
`rlimit memlock`, `dmesg`, `sysctl`.

**Truly needs root or a capability (each is a `kfroot` line, or box setup run by `kfroot.py bootstrap`):**

| Operation | Where (current tree) | Why root | Handled by |
|---|---|---|---|
| `apt-get install` of ovmf, swtpm, swtpm-tools, sbsigntool, python3-virt-firmware, xorriso, netpbm, p7zip-full, mingw (was `windows_gate.sh:685-693`) | `windows_gate.sh:672` (`install_prereqs`) | dpkg database and `/usr` | `kfroot`; box setup: `windows_gate.sh --prepare` before the drop. Inside the payload the apt shim is a bash function; the shim's own command (`python3 -I vast_setup.py apt`) is what runs under `kfroot` |
| swtpm AppArmor rule: append to `/etc/apparmor.d/local/usr.bin.swtpm`, `apparmor_parser -r` (was `win_vm.sh:257-267`) | `win_vm.sh:275-276` | `/etc` is root-owned; loading a profile needs CAP_MAC_ADMIN | `kfroot tee -a`, `kfroot apparmor_parser`; `win_vm.sh prepare` (called by `windows_gate.sh --prepare`); no root when the rule exists |
| `dmesg` (host Xid before/after, per boot) while `kernel.dmesg_restrict=1` | `kf_dmesg` (`scripts/lib/kfroot.sh`); callers `windows_gate.sh:475`, `windows_apps.sh:316`, `win_vm.sh:955,973` | CAP_SYSLOG | `bootstrap` sets `kernel.dmesg_restrict=0`; otherwise `kfroot dmesg`. **Before this change an unreadable log was a silent zero (a vacuous Xid check)**; the preflight now refuses `dmesg-unreadable` |
| Drop to the unprivileged user, empty bounding set, `NoNewPrivs`, raise RLIMIT_MEMLOCK above the hard limit | `kfroot.py run` (`kfroot prlimit ... setpriv ...`) | setuid/setgroups, CAP_SETPCAP, CAP_SYS_RESOURCE | one `kfroot` call per run (the audit shows it) |
| The CI user, group `kvm`, `/dev/kvm` group and mode, NVIDIA nodes (`nvidia-modprobe`, blocked once NoNewPrivs is set), node permissions, `limits.d`, the `/workspace` tree, the GPU lock file, the sudo log, the sysctl | `kfroot.py bootstrap` | useradd, chgrp, chmod, mknod, `/etc`, `/var/log`, sysctl | box setup, run once as root before the drop |
| Host driver install, module reload, `/root/...` provisioning | `windows_lane.sh` (whole script; its `apt-get` at `:79`), `provision_*` | the host driver is inherently root | unchanged: box setup |

**Needs only a file permission, a group or a path the user owns (not root):** open `/dev/kvm` (group `kvm`); `/dev/nvidiactl`, `/dev/nvidia*`
(mode 0666 after the bootstrap); `nvidia-smi`; `qemu-img create` on files (`win_vm.sh:345,901`; no block device, no nbd, no loop); `swtpm_setup`
and the swtpm sockets (`win_vm.sh:336,544`; state under `$VM/run`, not `/run`); `sbsign`, `xorriso`, `7z`, `virt-fw-vars`, `openssl`, `ssh-keygen`;
slirp user networking (`win_vm.sh:520`; no tap, bridge or iptables); `memory-backend-memfd,share=on` (`win_vm.sh:510`; no hugepages and no mlock in
the tree, so RLIMIT_MEMLOCK is raised as insurance and its need stays UNSETTLED until a run); the GPU lock `/tmp/kayfabe-fastguest.lock`
(`win_vm.sh:97,931`; a file owned by another user is refused by `fs.protected_regular`, so the bootstrap creates it mode 0666); `chmod 0400` of
the user's own qcow2; `git rev-parse` (needs the checkout owned by the user, `bootstrap --chown`); `cargo build` (needs a toolchain for that user:
NOT PROVISIONED, the gate refuses `no-cargo-for-uid-<n>`); `/workspace/winapps`, `/workspace/bench`, `/workspace/winvm` (ownership).

**Not changed (workstation or build lanes that are root by nature, listed so they are not mistaken for gate paths):**
`windows_broker.sh:62` (`die "run as root"`, `loginctl`, `runuser`, `pkill`), `run_windows_apps.sh:78,83,84` and `winpass_cycle.sh:60`,
`screen_sampler.sh:15` (raw `dmesg`), `propimage_run.sh:23,26,33` (DESIGN-ONLY; `ci` mode could drop its `runs as root` check: kf3 only needs the
device nodes), `appmatrix/appdisk.py:423` (`mount -o loop` of the bundle ISO: a bundle-build tool, in no gate path).

## 5. The per-run assertion

`kfroot.py assert-tree` reads `/proc/<pid>/status` of the named processes and every descendant: `Uid` (all four ids) not 0, `CapInh`, `CapPrm`,
`CapEff`, `CapBnd`, `CapAmb` all zero, `NoNewPrivs` 1; a missing or unreadable field, a named pid that is gone and "no process read" are violations
(fail closed). The gate calls it for its own process tree at the preflight and with QEMU and swtpm (pid files; swtpm daemonizes) after boot 1, and
for the sudo window at the end: the sudo log from the offset `kfroot.py run` recorded (`KF_SUDO_LOG_MARK`) must hold no entry other than a clean
call of `/usr/local/sbin/kfroot-wrapper` (a refused command, a password prompt, an unparsable line, another path: violation; a missing or unreadable
log: refusal, never an empty window).

`KF_ROOTLESS=require|record|off`. Default `record` (the result is recorded, the verdict is unchanged); `kfroot.py run` sets `require`. In require mode
a violation at the preflight is `WINDOWS_GATE_REFUSED why=rootless-violated`, after boot 1 it is an OPERATOR retry, and a PASS with a violated sudo
window becomes `WINDOWS_GATE_RETRY class=OPERATOR`. Recorded: `KFROOT_ASSERT ...` lines in the log, `rootless_preflight.json`, `rootless_boot1.json`,
`rootless_final.json` in the evidence, and `rootless=<ok|violated|off|unchecked>` first in the PASS line (and in the apps verdict). The payload
does not yet copy it into `kf-result.json` (section 8).

## 6. Tests and mutants (GPU-free)

`scripts/ci/test_kfroot.py` (about 200 tests): the launcher and wrapper under a fake sudo that reads the generated sudoers file (refuses a command
that is not whitelisted, `-E` without SETENV, a missing NOPASSWD, a reset PATH without `!secure_path`, a prompt without `-n`, and logs in sudo's
own format including the 80-column wrap); the sudoers content; install, verify and `visudo` handling (a real `visudo` when present); the bootstrap
plan and its execution with a fake runner; the drop argv; the audit parser; the `/proc` assertion on fake `/proc` trees and on the real one; the
shell library; the harness scripts (every root operation is a `kfroot` line with a `# root:` comment, no blanket refusal, no raw `dmesg`); the
real gate script with the assertion stubbed or real (a dropped gate passes the assertion and is refused later by name; an undropped root gate is
refused in require mode: root-only tests, skipped elsewhere). `scripts/ci/kfroot_mutants.py`: 75 mutants, each killed by at least two named
tests (wired in `.github/workflows/ci.yml`). The gate's `--selftest` gained 3 cases (27): a not-rootless PASS becomes a RETRY, a guest failure stays
the guest's verdict.

## 7. First-box-run checklist (nothing below has been run)

1. Box image: `sudo --version` must say the original sudo (not sudo-rs); `kfroot.py install --user ci` then `verify` (visudo accepted?).
2. `kfroot.py bootstrap --user ci --chown <checkout>`; read the `KFROOT_BOOTSTRAP` JSON for each step's rc (nvidia nodes, sysctl, `/dev/kvm`).
   Check by hand: `ls -l /dev/kvm /dev/nvidia*`, the user's groups (`id ci`: kvm), `ulimit -l`, `cat /proc/sys/kernel/dmesg_restrict`.
3. Provision a Rust toolchain the CI user can use (NOT done here; `CARGO_HOME`/`RUSTUP_HOME` or a per-user rustup) and the kf3 binary
   (`kf3-bins/<rev>`) readable by it.
4. `windows_gate.sh --prepare` (as `ci`, outside the drop): packages and the swtpm AppArmor rule through `kfroot`; read `$PROV/prepare*_windows.log`.
5. The gate as `ci`, dropped: `python3 scripts/ci/kfroot.py run --user ci -- bash scripts/bench/box/windows_gate.sh <tag> <prov>`. First look at the
   preflight `KFROOT_ASSERT` line (uid, caps 0, nnp 1), then the stage-by-stage behaviour of the WHOLE flow rootless, in particular:
   the sealed base install (`win_vm.sh install`, 30-60 min: `qemu-img create`, `swtpm_setup`, `sbsign`, `xorriso`, the unattended install with
   `-accel kvm` and memfd RAM), the swtpm socket and its AppArmor profile under the CI uid, QEMU's memory limits (memlock), the nbd/loop absence
   (none expected), `nvidia-smi -q`, the GPU lock, the host `dmesg` reads, the `LD_*` carry (does the gate or kf3 need one?), and the swtpm pid file
   in `rootless_boot1.json`.
6. The sudo log: `cat /var/log/kfroot-sudo.log` after the run: exactly the `kfroot prlimit ... setpriv` call of the drop and nothing else;
   `rootless_final.json` `sudo.violations` empty. Then `windows_apps.sh` the same way, then the mutant by hand: run the gate as root with
   `KF_ROOTLESS=require` (must refuse `rootless-violated`).
7. Record what failed under rootless as a finding with its evidence: the open questions are RLIMIT_MEMLOCK need, `pagemap` blindness in the
   refresh-stress census, and any node a driver (re)install creates after the bootstrap.

## 8. Not done, and follow-ups

- `vast_payload.py` / `vast_onstart.sh` still run every stage as root; the box bootstrap (`kfroot.py bootstrap`) is not yet called from
  `vast_onstart.sh`, and the stage commands are not yet wrapped in `kfroot.py run`. Wiring them is a payload change that must also add
  `kfroot.py` to the copy lists of every harness that copies `vast_payload.py` (`payload_mutants.py`, `spec_stage_mutants.py`) if the payload
  imports it, and copy the result into `kf-result.json`.
- `scripts/ci/install_flow.py` (draft PR #423) has its own `drop_argv`; `kfroot.drop_argv` is the same argv. It should import `kfroot` instead.
- The harness fixes of draft PR #420 (shared lock, git `safe.directory`) are not in this branch; the bootstrap works around the lock and the checkout owner.
- Other stages (merge bar, `build_kf3.sh`, `fast_suite.sh`, `cuda_ladder.sh`, the Linux app matrix) were probed rootless by hand in the rehearsal
  (`traces/release_manual_rehearsal_20261115/`: host CUDA ladder, a kf3 guest boot, `fast_suite` 30/30) but have no `kfroot` wiring here.
