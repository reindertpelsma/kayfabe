# Attach your GPU to a VM you already have

You already run a Linux or Windows VM under QEMU/KVM or libvirt. This page is only what changes to give it the kayfabe GPU: QEMU arguments, host permissions, a few guest settings. It does not install an operating system.

**[untested]** marks a step the repository has no script or record for. Everything unmarked comes from the launch scripts. Read the status on the [introduction](index.md) first: research stage, no installer, no public binary.

## What the host needs

- Linux on x86-64 with KVM, the open NVIDIA kernel module and a loadable `libcuda`: see [Requirements](requirements.md). <!-- src: docs/howto/requirements.md -->
- A QEMU 10.2.4 binary built with the `kf3-gpu` device: see [Building from source](building.md). A distribution's QEMU does not contain it. <!-- src: scripts/bench/build_kf3.sh -->
- The QEMU process must be able to open `/dev/kvm`, `/dev/nvidiactl`, `/dev/nvidia<minor>` and `/dev/nvidia-uvm`. A run as an ordinary user relied on NVIDIA's default mode 0666 for those nodes. `nvidia-modprobe -c 0 -u` creates them if they are missing. <!-- src: docs/FAQ.md "Can kayfabe run inside a container?", scripts/bench/rmladder_suite.sh -->
- Keep VFIO, iommufd, vfio-user, `nvme://` drives and SEV out of the VM: the device refuses to share a VM with anything that pins guest RAM. <!-- src: qemu/hw/misc/kf3/kf3.c (kf3_dev_realize) -->

## The arguments to add

Take your VM's command line and make these changes. The binary is the one `build_kf3.sh` installed under `kf3-bins/<rev>/`, and its `pc-bios` folder must stay beside it. <!-- src: scripts/bench/build_kf3.sh -->

```
-object memory-backend-memfd,id=ram0,size=8192M,share=on
-machine q35,accel=kvm,memory-backend=ram0 -m 8192 -cpu host
-uuid <a fixed uuid for this VM>
-device kf3-gpu,id=kf0,fb-mb=8192,bar1-size=134217728,bar2-size=33554432
```
<!-- src: scripts/fastguest/run_fast_guest.sh, scripts/bench/boot_nvkvm.sh, scripts/bench/windows/win_vm.sh -->

- **RAM must be a shared memfd, and `-m` must equal its `size=`** or QEMU refuses. The device does not check this at start: on ordinary RAM the VM boots, and the first guest memory mapping fails with `no fd-backed guest RAM (the VM needs memory-backend-memfd,share=on)` in QEMU's log. <!-- src: scripts/bench/boot_nvkvm.sh, crates/kf-qemu/src/mem.rs (place_ram) -->
- **Machine and CPU:** every recorded run is `q35` with `-cpu host` on KVM, and the device refuses without KVM. Other machine types are not documented. <!-- src: qemu/hw/misc/kf3/kf3.c -->
- **`-uuid`:** the per-VM GPU UUID is derived from it. Without `-uuid` (or `vm-id=`) the guest gets a random GPU UUID on every boot, and the device says so in its log. libvirt passes the domain's `<uuid>` [untested]. <!-- src: crates/kf-rm/src/gpuuid.rs -->
- **Display:** with `display=off` keep your VM's normal VGA. With `display=on`, remove the VGA (`-vga none`) because the kayfabe device is the display. <!-- src: scripts/bench/boot_nvkvm.sh, scripts/bench/display/interactive.sh -->

### The device properties you will touch

The full list, with defaults, is in [Running the QEMU device](installing.md). These are the ones an existing VM needs to decide.

| Property | Default | Change it when |
|---|---|---|
| `bar1-size` | 256 MiB | Always pass `134217728` (128 MiB) unless your card's BAR1 aperture is larger. The default plus BAR2, 1 MiB and 16 MiB headroom does not fit a 256 MiB aperture, and the device refuses at start with the largest value that fits. |
| `bar2-size` | 32 MiB | Pass `33554432`, as every script does. |
| `fb-mb` | 8192 | The card has less than 10 GiB: use VRAM minus 2048 MiB in whole GiB (6144 booted on an 8 GiB card). Too big is refused as `store of N MiB refused: NoMemory`. |
| `gpu-minor` | 0 | The host has several GPUs: the number in `/dev/nvidia<minor>`. |
| `guest-driver` | the host's version | The guest's NVIDIA driver is another version. The Windows harness's example for driver 580.88 is `580.65.06`. An unmeasured version is refused by name. |
| `display`, `gop` | off | You want a virtual monitor and boot screens. `gop` needs UEFI firmware, and the harnesses set both together. |
| `gop-efi` | none | The VM has Secure Boot: the boot-display driver must be signed. |
| `display-broker`, `display-broker-uid` | unset | You want a desktop window: see [Display and the broker](display.md). |
| `gpu-uuid` | `auto` | You need a fixed value: `gpu-uuid=GPU-<uuid>`. |

<!-- src: qemu/hw/misc/kf3/kf3.c (kf3_properties), docs/howto/installing.md, docs/howto/troubleshooting.md, scripts/fastguest/run_fast_guest.sh, crates/kf-qemu/src/cardbudget.rs -->

The recorded shapes are two. A **compute-only** Linux guest keeps the default VGA and uses `display=off` (`boot_nvkvm.sh`). A **desktop** guest uses `-vga none` and `display=on,gop=on` (`interactive.sh`, `win_vm.sh`). Windows with `display=off` reached driver Code 0, then reset and later stopped responding; no application results exist for it. <!-- src: scripts/bench/boot_nvkvm.sh, scripts/bench/display/interactive.sh, docs/howto/requirements.md -->

## libvirt [untested]

No script or record of ours starts a kayfabe VM through libvirt. This is the shape the arguments above take; expect to adjust it.

```xml
<domain type='kvm' xmlns:qemu='http://libvirt.org/schemas/domain/qemu/1.0'>
  <memory unit='MiB'>8192</memory>
  <memoryBacking><source type='memfd'/><access mode='shared'/></memoryBacking>
  <os><type arch='x86_64' machine='q35'>hvm</type></os>
  <cpu mode='host-passthrough'/>
  <devices>
    <emulator>/opt/kayfabe/kf3-bins/REV/qemu-system-x86_64</emulator>
    <memballoon model='none'/>
  </devices>
  <qemu:commandline>
    <qemu:arg value='-device'/>
    <qemu:arg value='kf3-gpu,id=kf0,fb-mb=8192,bar1-size=134217728,bar2-size=33554432'/>
  </qemu:commandline>
</domain>
```

- `<memoryBacking>` asks libvirt for the memfd backend itself. If it does not produce a shared memfd, put the `-object memory-backend-memfd,id=ram0,...` and `-machine memory-backend=ram0` arguments in `<qemu:commandline>` instead, with `-m` equal to `size=` (the Windows harness runs without a balloon device, hence `memballoon`). <!-- src: scripts/bench/windows/win_vm.sh -->
- The kayfabe QEMU is built with `--without-default-features`, no GTK or SDL, so libvirt's probing may reject devices it adds by default. With `display=on` use `<video><model type='none'/></video>`. <!-- src: scripts/bench/build_kf3.sh -->
- libvirt runs QEMU as its own user in cgroups and, on Ubuntu, under AppArmor. Check each of these when the VM will not start:
  1. The emulator path and its folder are readable and executable by that user (not under a `0700` home).
  2. **cgroup device ACL:** add `/dev/nvidiactl`, `/dev/nvidia0`, `/dev/nvidia-uvm` and `/dev/kvm` to `cgroup_device_acl` in `/etc/libvirt/qemu.conf`, keeping the default entries.
  3. **AppArmor or SELinux:** look for `DENIED` or AVC lines. The profile must allow the emulator path, the `/dev/nvidia*` nodes, `/dev/kvm`, `libcuda.so.1`, `/proc/driver/nvidia` and the card's `/sys/bus/pci` entries. (The Windows harness had to relax the AppArmor profile of `swtpm` for its folder, so these profiles do bite.) <!-- src: scripts/bench/windows/win_vm.sh (apparmor_swtpm) -->
  4. **Memory:** all guest RAM is pinned on the host for the VM's life, so host RAM and any memory cgroup limit must cover it plus QEMU's overhead. Whether `RLIMIT_MEMLOCK` matters is not documented: compare `prlimit --memlock --pid <qemu pid>` if the start fails. <!-- src: crates/kf-qemu/src/mem.rs, crates/kf-linux-raw/src/scratch.rs -->
  5. **Seccomp:** if QEMU dies at start with a seccomp line in the audit log, `seccomp_sandbox = 0` in `qemu.conf` rules it out. Treat that as a diagnostic, not a setting to keep; see [Security model](security.md).

## Proxmox [untested]

Proxmox generates its own `-machine` and `-m`, starts its own QEMU, and keeps extra arguments in the `args:` line of `/etc/pve/qemu-server/<vmid>.conf`. The `args:` line can carry the `-device kf3-gpu,...` argument, but Proxmox's QEMU has no such device and the repository has no recipe for pointing Proxmox at another binary or for a shared memfd. Treat Proxmox as not supported today.

## In the guest

**Linux.** Install NVIDIA's `.run` driver with `-m=kernel-open` (the open module is what every run used). Nothing else is set. See [Linux guest](linux-guest.md). <!-- src: docs/howto/linux-guest.md -->

**Windows.** Use NVIDIA's stock 580.88 installer and set these three values (DWORD, restart to apply). Setting the first two *before the driver's first start* avoids Code 43. The commands and the index lookup are in [Windows guest](windows-guest.md); `scripts/bench/windows/gsp_on.ps1` does the first two while installing the driver. The harness gives Windows a TPM 2.0 and Secure Boot; a guest without them has no recorded acceptance run. <!-- src: docs/howto/windows-guest.md -->

| Key (`<index>` is the display adapter's own key: `0000` on Windows 11 Pro, `0001` on LTSC; read it, never assume) | Value |
|---|---|
| `HKLM\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}\<index>` `EnableGpuFirmware` | `1` |
| `HKLM\SYSTEM\CurrentControlSet\Services\nvlddmkm` `EnableGpuFirmware` | `1` |
| `HKLM\SOFTWARE\Microsoft\Windows\Dwm` `OverlayTestMode` | `5` |

<!-- src: docs/howto/windows-guest.md, scripts/bench/windows/gsp_on.ps1 -->

A Secure Boot VM can only show the boot screens if the VM's variable store trusts the key that signed `gop-efi`. `win_vm.sh sign` signs the driver for the harness's own VM; enrolling that key in an existing VM is not documented. <!-- src: scripts/bench/windows/win_vm.sh, scripts/bench/windows/README.md -->

## Check that it works

- **Host, QEMU's log** has a line like `kf3: host GPU minor=0 bdf=0000:00:08.0 gpuId=0x8 rm_device_instance=0 cuda_device="NVIDIA GeForce RTX 3060" store=8192 MiB`. A refusal names its cause. <!-- src: crates/kf-qemu/src/device.rs, traces/v3_windows/discovery_20261004/basic1/basic1_b1_qemu.trimmed.log -->
- **Linux guest:** `nvidia-smi` names the card and exits 0 (`SMI_RC=0` in `boot_capture.sh`). For CUDA, the samples `deviceQuery` and `vectorAdd` were part of the 65-application matrix. <!-- src: scripts/bench/boot_capture.sh, docs/design/V3_APP_MATRIX.md -->
- **Windows guest:** `nvidia-smi`, and `Get-PnpDevice -Class Display` showing `problem=CM_PROB_NONE` (the last line `gsp_on.ps1` prints is `GSP_ON after enable: ... status=... problem=...`). `dxdiag /t C:\dxdiag.txt` records `Card name`, `Driver Version` and `Device Problem`. <!-- src: scripts/bench/windows/gsp_on.ps1, scripts/bench/windows/hags_status.ps1, scripts/bench/windows/check.ps1 -->
- **Host log while it runs:** `bash scripts/bench/host_xid_watch.sh <tag> -- <command>` exits 6 if a new `NVRM: Xid` appeared. A host GPU fault does not raise anything in the guest. <!-- src: scripts/bench/host_xid_watch.sh -->

## What is supported today

Linux guests have run CUDA applications, headless graphics and a desktop, on some cards from Turing to Blackwell; Windows 11 runs desktops, games and professional applications on the author's RTX 4070. Per-card evidence is the GPU table in [Requirements](requirements.md), mirrored on the site's [What works today](https://kayfabe.dev/works-today/) page; no separate support matrix file exists. <!-- src: docs/howto/requirements.md, site/content/works-today.md -->

Known issues, plainly:

- **Windows can reset the display driver (TDR) under heavy scenes**, and on some cards it loops. Edge video can reset after 7 to 16 minutes. Sleep and wake do not work. This is open. <!-- src: README.md "Not finished", docs/howto/troubleshooting.md -->
- Four CUDA applications that need unified-memory demand paging fail, one with a wrong result printed as SUCCESS.
- One VM per GPU is the recommended shape. Several VMs on a card are not measured, and the security model is not audited ([Security model](security.md)).
- Windows 580.88 is the only Windows driver covered. GB206 under Windows is Code 43. <!-- src: docs/howto/windows-guest.md -->

## If it does not start or misbehaves

[Troubleshooting](troubleshooting.md) lists the device's refusals by their message and the Windows Code 43 and TDR cases. First rule: if the same workload passes on the host with no VM and fails in the guest, the defect is kayfabe's. <!-- src: AGENTS.md "Verification" -->

## Not documented yet

- libvirt, Proxmox and other managers (everything marked [untested] above).
- Putting the boot-display key into an existing Secure Boot VM.
- Machine types other than `q35`, and a guest without KVM.
- Moving an existing Windows install onto the device (the order of driver and registry steps outside `gsp_on.ps1`).

Verified on: the QEMU arguments and properties were read from `run_fast_guest.sh`, `boot_nvkvm.sh`, `interactive.sh`, `win_vm.sh` and `kf3.c` at `next` (`e1dbdbd41`). Not verified end to end: nothing on this page was run, and no libvirt or Proxmox start is recorded.
