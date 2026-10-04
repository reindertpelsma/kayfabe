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
- GPU work takes `/tmp/kayfabe-fastguest.lock`, the same lock as the Linux lanes.
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
