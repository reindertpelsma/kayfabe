# The Windows test harness

This page is for people who want to test kayfabe with a Windows guest built by the project's own script. To run your own Windows guest, see [Run a Windows guest](windows-guest.md); you do not need the harness for that.

The harness installs Windows 11 Enterprise LTSC 2024 (Microsoft's 90-day evaluation) unattended, boots it on kayfabe, installs NVIDIA's driver with the guest settings applied, and collects evidence. It is written for rented, disposable machines and runs as root.

## The harness script, `win_vm.sh`

`scripts/bench/windows/win_vm.sh` installs a Windows 11 guest once and then boots it on kayfabe as many times as you like. It is one script and it needs a kayfabe QEMU built for the checkout's revision ([Building from source](building.md)). It refuses by name when a command or file it needs is missing. <!-- src: scripts/bench/windows/README.md -->

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
| `--env-file FILE` | Export the `KF3_*` flags listed in FILE (only `KF3_NAME=digits` lines and `#` comments are accepted). Not needed to start Windows (the Windows profile is the device's default); for measurement flags only, and `kf3_flags.sample.env` is a one-line example. |
| `--dry-run` | Print the QEMU command line and exit; start nothing. |

<!-- src: scripts/bench/windows/README.md; scripts/bench/windows/win_vm.sh (cmd_run) -->

The harness environment: `WINVM_ROOT` (default `/workspace/winvm`), `WINVM_NAME`, `WINVM_RAM_MB` (8192), `WINVM_SMP` (8), `WINVM_KF3_FB_MB` (8192), `WINVM_SSH_PORT` (2224), `WINVM_QEMU`. <!-- src: scripts/bench/windows/win_vm.sh header -->

**Opt-in, not yet run on hardware** (for a rented box; the defaults are the older behaviour): `WINVM_LOCK_WAIT_S` (7200, how long `run` waits for the GPU lock), `WINVM_INSTALL_WAIT_S` (14400, how long `install` waits), `WINVM_SSH_CONNECT_TIMEOUT` (8), `WINVM_VM_IDENTITY=1` (a random SMBIOS UUID, serial and MAC per VM, kept in the VM's directory), `WINVM_DRY_RUN=1`, and the sub-command `desktop-prep` (autologon and `OverlayTestMode=5` for the account in `KF_GUEST_USER`, default `kf`; for throwaway guests only, because the password is then stored in the guest registry). The table is in `scripts/bench/windows/README.md`. <!-- src: scripts/bench/windows/README.md -->

**Opt-in, not yet run on hardware:** `WINVM_KF3_DISPLAY=off` makes `run` build the device without the display plane and without the boot display driver (`kf3-gpu,fb-mb=...,id=kf0`), for a box with no monitor; the default (`on`) is the device described in the next paragraph. `WINVM_INSTALL_CACHE=unsafe` runs the disk of the install-time boots only with QEMU's `cache=unsafe` (faster; after a host crash during an install, delete the VM and install again). It is not an image cache and shares nothing between VMs. `scripts/bench/box/windows_lane.sh` runs a fresh rented box up to a sealed image with these set. <!-- src: scripts/bench/windows/README.md; scripts/bench/windows/win_vm.sh header -->

The QEMU command it builds uses the `pc-q35-10.2` machine with SMM, Secure Boot firmware, the TPM, and the device `kf3-gpu,fb-mb=8192,bar1-size=134217728,bar2-size=33554432,display=on,gop=on,gop-efi=<signed driver>,id=kf0`. Every guest reboot starts a fresh QEMU with the same firmware variables and the same TPM state; the harness restarts it for you. <!-- src: scripts/bench/windows/win_vm.sh -->

Rules the script enforces: every step logs `START` and `EXIT rc=` (a log with no `WINVM_RUN_END` belongs to a dead run); GPU work takes the shared lock `/tmp/kayfabe-fastguest.lock`; nothing secret is written into the repository. Keep the swtpm state and the ssh key private; a VM is never restored from another VM's state. <!-- src: scripts/bench/windows/README.md -->

**Remote access:** `ssh -J <gpu-host> -p 2224 kf@127.0.0.1` with a key you passed to `install --authorized-key`. To watch the screen, forward the VNC socket: `ssh -N -L 5905:/workspace/winvm/kfwin/run/vnc.sock <gpu-host>` and point a VNC viewer at `localhost:5905`. <!-- src: scripts/bench/windows/README.md -->


## Known limits

- `nv-install` is slow: it copies the 2.6 GB driver package over user networking at about 2.4 MB/s (18 minutes). <!-- src: scripts/bench/windows/README.md "Known limits" -->
- A failed NVIDIA driver slows everything down. With the driver in Code 43, boot to ssh took about 20 minutes and the guest ignored `shutdown /s` for 5 minutes. <!-- src: scripts/bench/windows/README.md "Known limits" -->
- The guest changes its own Secure Boot variables (it adds the 2023 CAs to `db` and updates `dbx`). The variables file is per-VM live state; never regenerate it or copy it to another VM. <!-- src: scripts/bench/windows/README.md "Known limits" -->

## Preparing the open application disk

The permanent public disk is winapps-open.iso, label KFAPPS. Build it from a reviewed checkout using Python
3.11+ standard library; the official Zig compiler is pinned in manifest.json. No shell extractor, ISO utility,
distro mingw or pip resolution is needed. App originals stay intact and are staged inside Windows.

```text
python3 -I scripts/bench/windows/appmatrix/package_bundle.py fetch --dl downloads
python3 -I scripts/bench/windows/appmatrix/package_bundle.py build --dl downloads --out out-a
python3 -I scripts/bench/windows/appmatrix/package_bundle.py build --dl downloads --out out-b
python3 -I scripts/bench/windows/appmatrix/package_bundle.py compare-image --iso out-b/winapps-open.iso --receipt out-a/winapps-open.manifest.json --out out-b
```

For manual downloads, also copy the tagged upstream tokenizers licence to
Downloads/_compliance/hf_wheels/tokenizers-v0.22.2-LICENSE as shown by Layout. The wheel itself contains
no legal text; the builder retains this pinned fallback alongside any bundled notices.

Level 1 is the reproduction gate: identical file paths and SHA-256/size values, including the repository
programs. Level 2 compares ISO bytes too, with a metadata WARNING when the files match but the container
differs. Both ISO directory namespaces must expose the same files and bytes; a primary-only payload
fails the check. Rock Ridge/System Use and hybrid filesystem records are unsupported and refused.
`verify-image --out out-a` checks one object's integrity and its embedded recipe against
the matching checkout, including pinned package/source bytes, guest files and licence notices. A receipt
with freshly recalculated hashes cannot approve different inputs. Run it from the reviewed checkout used
to build the image. Publication additionally requires that checkout's input digest before any store request.
The GUI offers the same build, integrity and reproduction checks. No program currently uses the supported
inventory-only/functional-check qualification. A vendor changing pinned bytes is INPUT_DRIFT, not flakiness.

There is no permanent private bundle. The Linux box host obtains required CUDA/torch/llama, driver and
conditional Microsoft/LunarG artifacts from official_fetch.json using J4 downloads with SHA-256 checks. The
transient KFAPPSP disk and local manifest live only for the run and are removed after all guests stop.
Nothing NVIDIA is uploaded to R2/GitHub; no Actions media artifact/cache is created. The guest stays offline.
Both disks are read-only, fixed K: (KFAPPS) and P: (KFAPPSP).

J2 must select the exact Windows driver with --official-driver VERSION or KF_WINAPPS_OFFICIAL_DRIVER
(the box stage `windows_apps.sh` defaults it to 580.88, the driver its gate installs).
Only known catalogue versions are accepted; currently 580.88 is pinned. Driver staging/install belongs
before the parent Windows gate; the app harness checks that selected version and does not change it after
the gate. --guest-driver is the separate kf3 ABI setting. A missing pin or wrong installed version is
MEDIA_DRIFT/NOTRUN. Missing official media never passes; --no-official-fetch leaves required apps NOTRUN.
The downloader/person or CI operator must accept the CUDA EULA for private use before official downloads.

The first deliverable is build + verify, followed by coordinator manual upload of public content. CI
publication remains optional and disabled; no secrets, domain, environment or enable flag is configured.
The GUI supports selecting the receipt and original archives in a file manager. Keep its licence texts,
notices and mirrored copyleft source together; source retention remains our obligation while distributing
the binaries. Native cross-platform builds and live upload have not been measured by this change.
<!-- src: scripts/bench/windows/appmatrix/package_bundle.py, scripts/bench/windows/appmatrix/bundle_repro.py, scripts/bench/windows/appmatrix/official_runtime.py -->

