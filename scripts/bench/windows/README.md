<!-- SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later -->
# Windows 11 on kayfabe: `win_vm.sh`

`win_vm.sh` installs a Windows 11 guest once, then boots it on kayfabe's `kf3-gpu` device as many times
as you need. You don't have to repeat the manual steps. The owner asked for it on 2026-10-04: *"a
script to later boot windows + kayfabe … also exposing a ssh to windows"*. It is one script for now; a
container can wrap it later. The design, and what the first run found, are in
[`docs/design/V3_WINDOWS_DISCOVERY.md`](../../../docs/design/V3_WINDOWS_DISCOVERY.md).

## What it sets up

| piece | how |
|---|---|
| Windows | Windows 11 Enterprise LTSC 2024, the 90-day evaluation (build 26100), from Microsoft's own fwlink. Installed unattended. |
| Secure Boot | ON. The VM has its own OVMF variable store. PK and KEK are generated for the VM. `db` holds Microsoft's keys plus **this host's kayfabe db certificate**. kf3's boot-display GOP driver is signed with that key (`gop-efi=`). |
| TPM 2.0 | swtpm. The state is **manufactured once** per VM (`swtpm_setup`) from swtpm's CSPRNG, then reused on every boot. The harness refuses to manufacture it again. |
| BitLocker | Prevented from the first boot: `PreventDeviceEncryption` is set both in the unattend component and in the registry. `check` verifies the volume is FullyDecrypted with protection Off. |
| drivers | The Red Hat virtio drivers (virtio-win 0.1.302): `viostor` is loaded in WinPE; NetKVM, vioserial, viorng, pvpanic, fwcfg and smbus are installed too. **No balloon and no viomem.** The QEMU guest agent is installed. |
| ssh | Win32-OpenSSH, key only. The key is the harness's own (`secrets/ssh_ed25519`) plus any you pass with `--authorized-key`. PowerShell is the default shell. |
| network | QEMU user networking with `restrict=on`. The guest has no route out, so there is no Windows Update and no Windows Update drivers. Only ssh is forwarded: `127.0.0.1:2224` → guest `:22` by default. |

## Prerequisites (on the GPU host)

You need the bench tree from `scripts/bench/provision_box.sh`, `provision_host_driver.sh` and
`provision_bench_tree.sh`, so that there is a kf3 QEMU for this checkout's revision under
`/workspace/bench/kf3-bins/<rev>/`. `build_kf3.sh` has configured `--enable-tpm --enable-slirp` since
2026-10-04, and `win_vm.sh` refuses an older binary by name. Also install:

```sh
apt-get install -y ovmf swtpm swtpm-tools sbsigntool python3-virt-firmware xorriso netpbm p7zip-full
```

`win_vm.sh` checks every command and file it needs and refuses by name if one is missing.

## Use

```sh
S=scripts/bench/windows/win_vm.sh
$S install                                  # once; about an hour on a nested box
$S run --detach --tag basic                 # kf3 + signed GOP, display on; restarts QEMU on each guest reboot
$S ssh 'Get-PnpDevice -Class Display'       # or: ssh -i /workspace/winvm/kfwin/secrets/ssh_ed25519 -p 2224 kf@127.0.0.1
$S shot                                     # PNG of the kf3 console under logs/shots/
$S check --kf3                              # WIN_* lines: Secure Boot, db, TPM + EK hash, BitLocker, virtio …
$S nv-install                               # NVIDIA 580.88, GSP forced on before nvlddmkm's first start
$S desktop-prep                             # autologon + DWM OverlayTestMode=5 for a signed-in desktop at the next boot
$S collect nv1                              # Windows-side evidence → logs/evidence/nv1/
$S stop                                     # guest shutdown → QMP powerdown → QMP quit (never SIGKILL)
$S status
```

`run` options:

| option | effect |
|---|---|
| `--overlay NAME` | The disk to boot: `disk/NAME.qcow2` over the sealed `base.qcow2`. It is created on first use. The default is `main`. |
| `--guest-driver V` | kf3's `guest-driver=` (for example `580.65.06` for Windows 580.88). |
| `--kf3-extra P` | Extra kf3 device properties. |
| `--no-kf3` | Boot on std VGA without kf3. |
| `--arm A` / `--arm B` | Without (A, the default) or with (B) Hyper-V enlightenments. |
| `--once` | Do not restart after a guest reboot. |
| `--ssh-port N` | The host port for ssh. |
| `--ssh-bind ADDR` | Bind the ssh port wider than `127.0.0.1`. **Explicit only.** |
| `--detach` | Run in the background. |
| `--no-rpc-trace` | Do not set `KF3_RPC_TRACE=1` (on by default for kf3 runs: one log line per command kf3 answers). |
| `--env-file FILE` | Export the `KF3_*` flags in FILE for this run (added 2026-10-31). Only blank lines, `#` comments and `KF3_NAME=digits` lines are accepted; any other line refuses the file. The run log records the file's name, sha256 prefix and flag count (`ENV_FILE`) and the resulting `KF3_*` environment (`KF3_ENV`). A sample is [`kf3_flags.sample.env`](kf3_flags.sample.env): the 19 flags of the recorded Windows Code 0 holds, **not confirmed as the current production list** (its header says what to check against `next`). |
| `--dry-run` | Print the QEMU command line (`DRYRUN_QEMU`) and the `KF3_*` environment (`DRYRUN_ENV`) and exit; nothing is started, locked, signed or created (same as `WINVM_DRY_RUN=1`; `install --dry-run` prints the install boot). |

**Options for a rented Vast box (added 2026-10-31, from [`V3_WINDOWS_ON_VAST.md`](../../../docs/design/V3_WINDOWS_ON_VAST.md) section 3).**
All are opt-in. The default column is today's behaviour for the old host workflow; the last column is what the Vast lane
sets. **None of this has run on hardware yet.**

| setting | default (unchanged) | Vast lane | what it does |
|---|---|---|---|
| `WINVM_LOCK_WAIT_S` | 7200 | 120 | How long `run` waits for the GPU lock before refusing (the refusal names the holder when `fuser` can). |
| `WINVM_INSTALL_WAIT_S` | 14400 | 4200 | How long `install` waits for the install boot. On timeout it screendumps, quits QEMU and the step fails. |
| `WINVM_SSH_CONNECT_TIMEOUT` | 8 | 100 | ssh/scp `ConnectTimeout`. `wait_ssh` lets one attempt last this long plus 12 s (it was a fixed 20 s). |
| `WINVM_VM_IDENTITY=1` | off | on | A random SMBIOS UUID, serial and MAC per VM, stored once in `$WINVM_ROOT/<name>/identity.env` (0600) and passed as `-uuid`, `-smbios type=1,serial=` and `mac=` on every boot of that VM. It makes kf3's `gpu-uuid=auto` (which hashes `-uuid`) the same on every boot of one VM and different between VMs. An `identity.env` that exists is always used. Turn it on **before `install`**: adding it to an installed VM changes the hardware identity Windows sees. |
| `WINVM_INSTALL_CACHE` | unset (QEMU's default) | `unsafe` through `windows_lane.sh` | A QEMU `cache=` mode (`none`, `off`, `writeback`, `writethrough`, `directsync`, `unsafe`; anything else is refused) for the **base disk of the two install-time boots only** (the install boot and its check boot); `run` never reads it. **It is not an image cache**: it is the write-cache mode of this VM's own disk, and nothing is saved, shared or restored between VMs (the swtpm state, the ssh key, the Windows password and the Secure Boot variables stay per VM). With `unsafe` a host crash during the install can leave a corrupt qcow2, so after one delete the VM directory and install again instead of resuming. `windows_lane.sh` sets `unsafe`; `WINLANE_INSTALL_CACHE=` (set, empty) turns it off. Added 2026-10-31 (from draft PR #165), not yet run on hardware. |
| `WINVM_KF3_DISPLAY=off` | `on` | not used (the Vast bar needs a console screenshot) | `run` builds a headless `kf3-gpu` (no display plane, no boot GOP, so no GOP is built or signed). Anything but `on` or `off` is refused by name. Added 2026-10-31 (from draft PR #163), not yet run on hardware. |
| `KF_GUEST_USER` | `kf` (`desktop-prep`); `vast` (`signin.py`) | `kf` | The account `desktop-prep` sets up for autologon, and the account `signin.py` expects to own the session. |
| `desktop-prep` | not run | run once in the bind boot | Sets Winlogon autologon for `KF_GUEST_USER` (the password is read from `secrets/win_password` and goes to the guest on ssh's stdin, never in an argument), `DevicePasswordLessBuildVersion=0` and `OverlayTestMode=5`, and reads each back (`desktop_prep.txt`-style output in `logs/desktop_prep_*.txt`, last line `DESKTOP_PREP_DONE fail=0`). They apply at the next boot of that disk. The password then sits in the guest registry: use it on a throwaway guest on an untrusted box only. The installed image itself still ends first logon password-less (`firstlogon.ps1` is unchanged). |

**Fresh Vast box to a sealed Windows image: `scripts/bench/box/windows_lane.sh`** (added 2026-10-31 from draft PR #165; not yet run on hardware in this form). It provisions the box, fetches the pinned media in parallel with the host driver and the kf3 build, and runs `win_vm.sh install` with every core (at most 16) and `WINVM_INSTALL_CACHE=unsafe`. It prints `STAGE <name> START|EXIT rc=N`, a `WINLANE_TIMING` line and a last `WINLANE_EXIT rc=N` line to `/root/prov/winlane.log`; a log without `WINLANE_EXIT` belongs to a dead run. It does not run the Linux suites and does not restore a prebuilt image.

**The merge-bar Windows gate: `scripts/bench/box/windows_gate.sh`** (added 2026-11-01; `KF_MERGE_WINDOWS=1` in
`merge_check.sh`). It drives this script through install, `run`, `check --kf3`, `nv-install --no-enable`,
`desktop-prep`, a guest reboot, a one-time device enable (`gate_enable.ps1`), `nvidia-smi`, two probes
(`cup8`, `kf_dxprobe d3d11`), a 300 s idle and the reset counters (`gate_state.ps1`, `gate_run.ps1`), and ends in one
`WINDOWS_GATE_*` verdict line; the same-box host GPU reference decides whether a failure is kayfabe's. Design
and the first real run: [`docs/design/V3_WINDOWS_GATE.md`](../../../docs/design/V3_WINDOWS_GATE.md).

**Remote access.** Use `ssh -J <gpu-host> -p 2224 kf@127.0.0.1` with a key you passed to
`install --authorized-key`. No public port is opened.

## Files (`$WINVM_ROOT`, default `/workspace/winvm`)

```
cache/              pinned media; sha256 and size are checked on every install
secureboot/         db.key db.crt db.guid (this host's), kf-gop.<rev>.efi, kf-gop.<rev>.signed.efi
kfwin/firmware/     OVMF_VARS.fd: per-VM Secure Boot state, created once, never regenerated
kfwin/tpm/          the swtpm state: THE SECRET (owner, §K). 0700, never copied, never committed
kfwin/disk/         base.qcow2 (sealed 0400 after the install check) and overlays
kfwin/secrets/      win_password, ssh_ed25519; unattend.iso until the install check passes
kfwin/run/          qmp.sock, qmp-ev.sock, qga.sock, swtpm.sock, vnc.sock, pids, locks
kfwin/logs/         install.log, <tag>_run.log, per boot: _qemu.log _qmp_events.jsonl _serial.log
                    _host_dmesg.txt _host_smi_{before,after}.txt _cmdline.txt; shots/; evidence/
```

**Rules the script enforces:**
- Every step logs `START` and `EXIT rc=`. A log with no `WINVM_RUN_END` belongs to a dead run, not a
  running one.
- The kf3 revision and the host driver are stamped on every run (`WINVM_RUN_START`).
- GPU work takes `/tmp/kayfabe-fastguest.lock`, the same lock as the Linux lanes. `run` waits for it up to
  `WINVM_LOCK_WAIT_S` seconds (default 7200).
- Nothing secret is in this directory or the repo. A VM is never restored from another VM's state: if
  the box is lost, run `install` again.

**To watch the screen live:** `ssh -N -L 5905:/workspace/winvm/kfwin/run/vnc.sock <gpu-host>`, then
point a VNC viewer at `localhost:5905`.

## Known limits (first run, 2026-10-04, `docs/design/V3_WINDOWS_DISCOVERY.md`)

- **`nv-install` is slow.** It copies the 2.6 GB `Display.Driver` over QEMU user networking, at about
  2.4 MB/s (18 minutes). Handing the driver to the guest as an ISO is the planned fix.
- **A failed NVIDIA driver slows everything down.** With nvlddmkm in Code 43, boot to ssh took about
  20 minutes, and the guest ignored `shutdown /s` for 5 minutes, so `stop` fell back to QMP `quit`.
- **The guest changes its own Secure Boot vars.** Windows adds the 2023 CAs to `db` and updates `dbx`. KEK
  also changed during the first install, which is not yet explained. The vars file is per-VM live state:
  it is never regenerated, and it is never copied to another VM.
- **GSP is forced on.** `nv-install` writes `EnableGpuFirmware=1` before nvlddmkm's first start, because
  kayfabe serves only a GSP client.
