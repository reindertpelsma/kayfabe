# Windows guest

**What a Windows 11 guest needs to run NVIDIA's stock driver on kayfabe.** Labels: [measured] = a run on named hardware, [inferred] = reasoning. <!-- src: docs/WINDOWS_GUEST_SETUP.md (STATUS: LIVE, 2026-10-17) -->

## What was tested

- **Windows 11 Pro, build 26200 (the full retail edition)** [measured, 2026-10-14, kayfabe `eb9f2aa3`, RTX 4070]: the desktop and the application matrix (the benchmark output names "Microsoft Windows 11 Pro", kernel 10.0.26200). <!-- src: docs/WINDOWS_GUEST_SETUP.md, README.md -->
- **Windows 11 Enterprise LTSC 2024, build 26100** (Microsoft's 90-day evaluation) [measured]: the image that `scripts/bench/windows/win_vm.sh install` builds, and the one used on rented hosts. <!-- src: docs/WINDOWS_GUEST_SETUP.md, scripts/bench/windows/README.md -->
- **Driver:** NVIDIA's stock Windows driver, version 580.88, installed with NVIDIA's own installer or INF, unmodified. No driver, kernel or Windows binary is patched. <!-- src: docs/WINDOWS_GUEST_SETUP.md -->
- Applications run unmodified from the vendors' complete installers; the README's table lists the results (Minecraft, Unigine Heaven, GravityMark, Edge with hardware video decode, CUDA, Blender, OBS, Office, SolidWorks and 3DMark). Several of those rows say their result files are not committed yet. <!-- src: README.md "What runs today" -->

## The settings it needs (and why)

The stock GeForce driver on Windows runs its resource manager on the CPU by default. Kayfabe serves only the GSP firmware path, so the driver has to be told to use GSP. Without the first setting the driver stops with **Code 43** [measured on Windows 11 and on LTSC]. All values are DWORDs and take effect after a restart. <!-- src: docs/WINDOWS_GUEST_SETUP.md -->

| # | Key | Value | Why |
|---|---|---|---|
| 1 | `HKLM\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}\<index>` `EnableGpuFirmware` | `1` | forces GSP; `<index>` is the display adapter's own key (`0000` on Windows 11 Pro, `0001` on the LTSC image: read it, never assume) |
| 2 | `HKLM\SYSTEM\CurrentControlSet\Services\nvlddmkm` `EnableGpuFirmware` | `1` | the same flag; the service key alone is not enough, set both |
| 3 | `HKLM\SOFTWARE\Microsoft\Windows\Dwm` `OverlayTestMode` | `5` | without it, opening the Start menu reset the display driver on Windows 11 Pro (the cause is not known) |

<!-- src: docs/WINDOWS_GUEST_SETUP.md -->

From an elevated command prompt in the guest. First read the display adapter's driver key, then substitute it:

```
powershell -NoProfile -Command "(Get-PnpDeviceProperty -InstanceId (Get-PnpDevice -PresentOnly | Where-Object InstanceId -like 'PCI\VEN_10DE*').InstanceId -KeyName DEVPKEY_Device_Driver).Data"
reg add "HKLM\SYSTEM\CurrentControlSet\Control\Class\<the key printed above>" /v EnableGpuFirmware /t REG_DWORD /d 1 /f
reg add "HKLM\SYSTEM\CurrentControlSet\Services\nvlddmkm" /v EnableGpuFirmware /t REG_DWORD /d 1 /f
reg add "HKLM\SOFTWARE\Microsoft\Windows\Dwm" /v OverlayTestMode /t REG_DWORD /d 5 /f
shutdown /r /t 0
```
<!-- src: docs/WINDOWS_GUEST_SETUP.md -->

`scripts/bench/windows/gsp_on.ps1` does settings 1 and 2 while it installs the driver (it is what `win_vm.sh nv-install` runs): it disables the device, adds and installs the INF, writes both values, reads them back, then re-enables the device. Setting them **before the driver's first start** avoids the Code 43. <!-- src: docs/WINDOWS_GUEST_SETUP.md, scripts/bench/windows/gsp_on.ps1 -->

These are settings for a development guest, not a complete installation guide for a production system.

## The Windows harness: `win_vm.sh`

`scripts/bench/windows/win_vm.sh` installs a Windows 11 guest once and then boots it on kayfabe as many times as you like. It is one script and it needs a bench tree and a kayfabe QEMU built for the checkout's revision ([Building from source](building.md), [Running the QEMU device](installing.md)). It refuses by name when a command or file it needs is missing. <!-- src: scripts/bench/windows/README.md -->

Install the host tools it names:

```sh
apt-get install -y ovmf swtpm swtpm-tools sbsigntool python3-virt-firmware xorriso netpbm p7zip-full
```
<!-- src: scripts/bench/windows/README.md -->

### What it sets up

| Piece | How |
|---|---|
| Windows | Windows 11 Enterprise LTSC 2024, the 90-day evaluation (build 26100), from Microsoft's own download link, installed unattended. |
| Secure Boot | On. The VM has its own OVMF variable store; PK and KEK are generated for the VM; `db` holds Microsoft's keys plus this host's kayfabe `db` certificate, which signs kayfabe's boot-display driver (`gop-efi=`). |
| TPM 2.0 | swtpm. The state is manufactured once per VM and then reused. The harness refuses to manufacture it again. |
| BitLocker | Prevented from the first boot; `check` verifies the volume is decrypted. |
| Drivers | The Red Hat virtio drivers (virtio-win 0.1.302), the QEMU guest agent. No balloon, no viomem. |
| ssh | Win32-OpenSSH, key only; PowerShell is the default shell. |
| Network | QEMU user networking with `restrict=on`: the guest has no route out, so no Windows Update. Only ssh is forwarded, `127.0.0.1:2224` to the guest by default. |

<!-- src: scripts/bench/windows/README.md -->

### Commands

```sh
S=scripts/bench/windows/win_vm.sh
$S install                                  # once; about an hour on a nested box
$S run --detach --tag basic                 # kayfabe + signed boot-display driver, display on
$S ssh 'Get-PnpDevice -Class Display'
$S shot                                     # PNG of the console
$S check --kf3                              # WIN_* lines: Secure Boot, db, TPM, BitLocker, virtio...
$S nv-install                               # NVIDIA 580.88, GSP forced on before nvlddmkm's first start
$S collect nv1                              # Windows-side evidence into logs/evidence/nv1/
$S stop                                     # guest shutdown, then QMP powerdown, then QMP quit
$S status
```
<!-- src: scripts/bench/windows/README.md "Use"; scripts/bench/windows/win_vm.sh header -->

Other sub-commands in the script's header: `scp-to`, `scp-from`, `sign` (export and sign this checkout's boot-display driver) and `fetch` (download and verify the pinned media). <!-- src: scripts/bench/windows/win_vm.sh -->

`run` options:

| Option | Effect |
|---|---|
| `--overlay NAME` | The disk to boot: `disk/NAME.qcow2` over the sealed `base.qcow2`, created on first use. Default `main`. |
| `--guest-driver V` | The device's `guest-driver=` (for example `580.65.06` for Windows 580.88). |
| `--kf3-extra P` | Extra device properties. |
| `--no-kf3` | Boot on the standard VGA without kayfabe. |
| `--arm A` / `--arm B` | Without (A, the default) or with (B) Hyper-V enlightenments. |
| `--once` | Do not restart after a guest reboot. |
| `--ssh-port N`, `--ssh-bind ADDR` | The host port for ssh; binding wider than `127.0.0.1` is explicit only. |
| `--detach` | Run in the background. |
| `--no-rpc-trace` | Do not set `KF3_RPC_TRACE=1` (on by default for kayfabe runs). |
| `--tag`, `--max-boots` | Name the run; limit restarts (default 12). |
| `--env-file FILE` | Export the `KF3_*` flags listed in FILE (only `KF3_NAME=digits` lines and `#` comments are accepted). Added 2026-10-31; the sample `kf3_flags.sample.env` is not confirmed as the current production list. |
| `--dry-run` | Print the QEMU command line and exit; start nothing. |

<!-- src: scripts/bench/windows/README.md; scripts/bench/windows/win_vm.sh (cmd_run) -->

The harness environment: `WINVM_ROOT` (default `/workspace/winvm`), `WINVM_NAME`, `WINVM_RAM_MB` (8192), `WINVM_SMP` (8), `WINVM_KF3_FB_MB` (8192), `WINVM_SSH_PORT` (2224), `WINVM_QEMU`. <!-- src: scripts/bench/windows/win_vm.sh header -->

**Added 2026-10-31, opt-in, not yet run on hardware** (for a rented box; the defaults are the old behaviour): `WINVM_LOCK_WAIT_S` (7200, how long `run` waits for the GPU lock), `WINVM_INSTALL_WAIT_S` (14400, how long `install` waits), `WINVM_SSH_CONNECT_TIMEOUT` (8), `WINVM_VM_IDENTITY=1` (a random SMBIOS UUID, serial and MAC per VM, kept in the VM's directory), `WINVM_DRY_RUN=1`, and the sub-command `desktop-prep` (autologon and `OverlayTestMode=5` for the account in `KF_GUEST_USER`, default `kf`; for throwaway guests only, because the password is then stored in the guest registry). The table is in `scripts/bench/windows/README.md`. <!-- src: scripts/bench/windows/README.md -->

The QEMU command it builds uses the `pc-q35-10.2` machine with SMM, Secure Boot firmware, the TPM, and the device `kf3-gpu,fb-mb=8192,bar1-size=134217728,bar2-size=33554432,display=on,gop=on,gop-efi=<signed driver>,id=kf0`. Every guest reboot starts a fresh QEMU with the same firmware variables and the same TPM state; the harness restarts it for you. <!-- src: scripts/bench/windows/win_vm.sh -->

Rules the script enforces: every step logs `START` and `EXIT rc=` (a log with no `WINVM_RUN_END` belongs to a dead run); GPU work takes the shared lock `/tmp/kayfabe-fastguest.lock`; nothing secret is written into the repository. Keep the swtpm state and the ssh key private; a VM is never restored from another VM's state. <!-- src: scripts/bench/windows/README.md -->

**Remote access:** `ssh -J <gpu-host> -p 2224 kf@127.0.0.1` with a key you passed to `install --authorized-key`. To watch the screen, forward the VNC socket: `ssh -N -L 5905:/workspace/winvm/kfwin/run/vnc.sock <gpu-host>` and point a VNC viewer at `localhost:5905`. <!-- src: scripts/bench/windows/README.md -->

## Other things a Windows guest needs (not settings)

- *(2026-10-31)* `win_vm.sh` gives each VM its own SMBIOS UUID, serial and MAC when `WINVM_VM_IDENTITY=1` is set before `install` (off by default; it has not run on hardware yet). Without it every VM has the same MAC and QEMU's default SMBIOS identity. <!-- src: scripts/bench/windows/README.md -->
- A **unique SMBIOS UUID, serial and MAC per VM**, and offline setup until after OOBE (Windows Autopilot once matched a default-identity VM to an unrelated company tenant); an explicit `gpu-uuid`. <!-- src: docs/WINDOWS_GUEST_SETUP.md -->
- **Do not snapshot an image with a Windows update staged**: it crash-loops `dwm.exe`. Block updates in test images. <!-- src: docs/WINDOWS_GUEST_SETUP.md -->
- You supply and license Windows. The scripts download Microsoft's evaluation image, the virtio drivers and NVIDIA's installer from their vendors, each pinned by SHA-256. See [Credits and notices](credits.md). <!-- src: scripts/bench/windows/win_vm.sh, docs/NOTICES.md -->

## Known limits

- **Windows sleep/wake is not supported yet.** <!-- src: docs/WINDOWS_GUEST_SETUP.md, README.md -->
- Edge can reset the display driver after 7 to 16 minutes of playback. <!-- src: README.md "Not finished" -->
- Other GPU dies need more work under Windows; GB206 under Windows is open (Code 43). <!-- src: README.md "GPUs:" -->
- `nv-install` is slow: it copies the 2.6 GB driver package over user networking at about 2.4 MB/s (18 minutes). <!-- src: scripts/bench/windows/README.md "Known limits" -->
- **A failed NVIDIA driver slows everything down.** With the driver in Code 43, boot to ssh took about 20 minutes and the guest ignored `shutdown /s` for 5 minutes. <!-- src: scripts/bench/windows/README.md "Known limits" -->
- The guest changes its own Secure Boot variables (it adds the 2023 CAs to `db` and updates `dbx`). The variables file is per-VM live state; never regenerate it or copy it to another VM. <!-- src: scripts/bench/windows/README.md "Known limits" -->

See also [Troubleshooting](troubleshooting.md) and [Display and the broker](display.md).

## Not documented yet

- A Windows install on a machine without the harness (a plain ISO install onto a kayfabe-backed VM by hand).
- Windows 10, or other Windows 11 builds than the two above.
- A successful guest run with another NVIDIA driver version. Windows 581.51 was attempted on AD102 and failed with Code 43 [measured in the sweep matrix, revision `24906bf33`, P19 profile; historical source `9d6f210ba`]. This is recorded on the branch `codex/sweep-integrate-20261018`, not merged: [matrix](https://github.com/reindertpelsma/kayfabe/blob/0bbf3c55f/traces/sweep_integrate_20261018/MATRIX.md). Its same-board native Windows control passed, which is not a kayfabe guest pass.

Verified on: Windows 11 Pro 26200 with driver 580.88 on an RTX 4070 (kayfabe `eb9f2aa3`, 2026-10-14; further results up to master `89fc8a222`, 2026-10-21, in the README); LTSC 26100 on rented hosts. Not verified end to end: the `reg add` snippet is the repository's own text and is not a recorded run of exactly those lines. Settings 1 and 2 are the part that `gsp_on.ps1` applies and reads back in the harness.
