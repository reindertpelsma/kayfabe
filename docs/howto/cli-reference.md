# Command-line reference

**Assumes:** QEMU 10.2.4 with the kayfabe device ([Install](installing.md)). The table describes the device of the source tree this manual was generated from.

Everything you set for a kayfabe VM goes on the QEMU command line. A normal Windows or Linux run needs no environment variable.

Kayfabe adds one QEMU device, `kf3-gpu`, to the QEMU binary you built ([Install](installing.md)). A VM without a `-device kf3-gpu` argument is plain QEMU.

## What the VM needs besides the device

```
-object memory-backend-memfd,id=ram0,size=<N>M,share=on
-machine q35,accel=kvm,memory-backend=ram0 -m <N> -cpu host
-uuid <a fixed uuid for this VM>
-device kf3-gpu,id=kf0
```

- Guest RAM must be a shared memfd, and `-m` must equal its `size=`. On ordinary RAM the VM boots and the first GPU memory mapping fails.
- Use `q35` with KVM and `-cpu host`. Other machine types are not documented.
- `-uuid` gives the VM a stable identity: the GPU UUID the guest sees is derived from it (`gpu-uuid=auto`). Without `-uuid` (or `vm-id=`) the guest gets a random GPU UUID at every boot, and the device says so in its log.
- With `display=on`, add `-vga none`: the kayfabe device is the display.
- Keep VFIO, iommufd, vfio-user, `nvme://` drives, SEV and a memory balloon (`virtio-balloon`) out of the VM. The device refuses to share a VM with them.

[Attach to a VM you already have](attach-to-existing-vm.md) explains each line.

## Device properties

Properties are given on the device, `-device kf3-gpu,id=kf0,fb-mb=8192,bar1-size=134217728,bar2-size=33554432`, or for one property on every device of the type with `-global kf3-gpu.<property>=<value>`. PCI placement is QEMU's own: use `bus=` and `addr=`.

This table is generated from the device's source (`qemu/hw/misc/kf3/kf3.c`) and a test fails when it differs from it. Types and defaults are the source's. The Status column says what is known about the property in words: "Used in recorded runs" means the maintainers' test runs set or rely on it; "Not yet run on hardware" means it exists in the code and in GPU-free tests only.

<!-- kf-generated: device-properties begin (scripts/ci/doc_cli_reference.py; do not edit by hand) -->

| Property | Type | Default | Status | What it does |
|---|---|---|---|---|
| `gpu-minor` | uint32 | `0` | Used in recorded runs | Which host GPU to use: the number in `/dev/nvidia<minor>`. One device per host GPU. |
| `fb-mb` | uint64 | `8192` | Used in recorded runs | Guest video memory in MiB. It is a real allocation of host VRAM and must fit the card; a value that does not fit is refused at start. |
| `bar1-size` | uint64 | `268435456` (256 MiB) | Used in recorded runs | Size of the guest's BAR1 in bytes. The recorded runs pass 134217728 (128 MiB), because 256 MiB plus headroom does not fit a 256 MiB host aperture; the device refuses a size that does not fit and names the largest that does. |
| `bar1-overlays` | uint32 | `64` | Not yet run on hardware | Hopper and newer only: how many BAR1 doorbell views can be trapped at once. No Hopper hardware run exists. |
| `bar2-size` | uint64 | `33554432` (32 MiB) | Used in recorded runs | Size of the guest's BAR2 in bytes. The recorded runs pass 33554432 (32 MiB). |
| `msix-vectors` | uint32 | `32` | Used in recorded runs | Number of MSI-X vectors the device offers. |
| `guest-driver` | string | `unset` (the host driver's version is used) | Used in recorded runs | The guest's NVIDIA driver version, as `major.minor[.patch]`. Unset means the host driver's own version, and the version the guest driver announces is checked against it. A guest on another driver branch than the host needs this property; the refusal names the value to give (see Troubleshooting). |
| `dummy-bar` | bool | `off` | Diagnostic | Adds synthetic MMIO test pages for trap-cost measurements. It submits no work to the GPU. Leave it off. |
| `display` | bool | `off` | Used in recorded runs | Gives the guest a virtual NVIDIA display that its own driver drives. Off means a headless GPU. See Display and the broker. |
| `gop` | bool | `off` | Used in recorded runs | Boot display: a UEFI driver in the card's expansion ROM shows the firmware and boot screens. The recorded desktop runs set it together with `display=on`. |
| `x11-dispsw` | bool | `off` | Experimental | A display class that X11 desktops need. Needs `display=on`. |
| `gop-efi` | string | `unset` (the built-in driver is used) | Used in recorded runs | Path of the boot display driver, signed, for a Secure Boot guest. Refused with `gop=off`. |
| `gpu-uuid` | string | `unset` (behaves as `auto`) | Default in use; other values not verified | The GPU UUID the guest sees: `auto` (derived from the VM id, the host GPU's UUID and the PCI slot, so each VM differs), `random`, `host`, or an explicit `GPU-xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`. |
| `vm-id` | string | `unset` (QEMU's `-uuid` is used) | Default in use; other values not verified | The VM identity text that `gpu-uuid=auto` derives from. |
| `channel-budget` | uint32 | `0` (derived from the host) | Used in recorded runs | GPU channels per runlist that the guest is told. Refused below 256 and above what the host can give (at most 2048). Leave it at `0`. |
| `user-view-keep` | uint32 | `0` (behaves as 16) | Used in recorded runs | Windows tuning: CPU views of video memory that each channel keeps armed, up to 256. |
| `windows-support` | bool | `on` | Default in use for Windows; `off` not measured | The Windows profile (the behaviours a Windows driver needs and Linux does not). It is chosen by what the guest announces about itself, so it is not a security boundary. `off` makes the VM Linux-only; set it on hosts that run only Linux guests or several tenants. |
| `report-utilization` | bool | `on` | Not yet run on a GPU | Shows the guest the physical GPU's utilisation in `nvidia-smi` and NVML. On a GPU shared by several VMs every guest sees the whole GPU's load, including the other VMs'; set `off` for tenants that must not see each other's load. |
| `unmapped-reads-zero` | on/off/auto | `auto` | Linux guest measured with `on` (Vulkan sparse residency, RTX 4090); Windows boots measured with it on and off; `auto` itself not yet run on a GPU | Where it is on, a GPU read of an unmapped guest address returns zero and a write is dropped, with no fault. `auto` turns it on for a Linux guest and off for a Windows guest (a guest that announces the Windows driver while `windows-support` is on); `on` and `off` apply to every guest. The guest's own announcement decides, so `auto` is a compatibility default and not a protection: any guest that does not announce Windows gets it on, with the cost below. Only `off` removes that cost. Vulkan sparse residency needs it on; with it off those cases hang the guest's GPU channel (host `Xid 109`) and the application loses the device. The cost of on: every fault the guest expects becomes a silent zero, including application bugs that read freed memory and CUDA managed memory or HMM pages that need demand paging (kayfabe does not serve them: with it off they fail with an error, with it on they read zero and the result can be wrong), and while a VM with it on runs the host driver holds about 2 GiB of video memory for page tables (one RTX 4090 run). Set `off` for a Linux VM that uses managed memory and no sparse resources. On a Windows guest `on` made the host refuse a large mapping of the guest's driver in every boot that was run (RTX 4070 and RTX 4090), and the device then refuses that mapping by name instead of leaving it silently absent, so leave Windows on `auto`. |
| `doorbell-ioeventfd` | bool | `off` | Experimental | Doorbell fast path that avoids a VM exit per doorbell write. Off until measured. |
| `doorbell-ioeventfd-max` | uint32 | `256` | Experimental | Cap on the number of doorbell fast-path event files. |
| `display-broker` | string | `unset` (no broker) | Used in recorded runs | Absolute path of the display broker's unix socket. Unset means no broker. See Display and the broker. |
| `display-broker-uid` | int64 | `-1` (none) | Used in recorded runs | One more user id accepted as the broker. Needed only when the broker runs as another user than QEMU; root and QEMU's own user are always accepted. Any value outside 0..4294967294 is refused. |
| `display-broker-vram` | string | `unset` (behaves as `auto`) | Used in recorded runs | GPU-copy path for frames to the broker: `auto`, `on` or `off`. |
| `display-max-fps` | uint32 | `0` (a cap of 75 Hz) | Default in use; other values not verified | Cap on the emulated monitor's refresh rate in Hz, 24 to 75. Refused outside that range and without `display=on`. |
| `x-gsp-observer` | string | `unset` (off) | Diagnostic | Records the guest driver's GSP traffic to a new file at this path (it must not exist). For bug reports from the maintainers' tooling; not needed for normal use. |
| `x-gsp-observer-seconds` \* | uint32 | `300` | Diagnostic | How long the recording runs, from 1 to 7200 seconds. |

\* Not described in earlier versions of this manual.

<!-- kf-generated: device-properties end -->

The `unmapped-reads-zero` default in the table above is `auto`, as the device source has it: on for a Linux guest, off for a Windows guest. Set `off` to get the host fault back for a Linux VM that uses managed memory and no sparse resources. Set `on` for a Windows guest only to try sparse resources there; see [Troubleshooting](troubleshooting.md) for what was seen with it on.

A property that is refused at start is refused by name; see [Troubleshooting](troubleshooting.md#the-device-refuses-to-start).

## The display broker

A headless VM needs no broker. For a window on your desktop, start the broker, then give QEMU its socket (see [Display and the broker](display.md)):

```
nvkvm-display-broker --socket <path> [--backend auto|wayland|x11] [--persist] [--title TEXT]
-device kf3-gpu,...,display=on,display-broker=<path>
```

The broker's options today, from its own usage text: `--socket PATH`, `--socket-mode OCTAL` (default 0600), `--socket-group NAME`, `--socket-fd N`, `--no-peercred`, `--backend auto|wayland|x11|test`, `--size WxH` (default 1920x1080), `--title TEXT`, `--allow-user NAME`, `--allow-group NAME`, `--allow-uid UID`, `--wayland-display NAME`, `--runtime-dir DIR`, `--x11-display :N`, `--xauthority FILE`, `--drop-user WHO`, `--fullscreen`, `--scale aspect|stretch|none`, `--persist`, `--clipboard off|guest-to-host|consent`, `--clipboard-trigger LIST`, `--refresh auto|none|HZ`, `--resolution auto|none|WxH`, `--present-mode`, `--trace-frames`, `--linear-only`, `--stats-line`, `--verbose`, `--help`. `nvkvm-display-broker --help` prints the full text.

A broker started from outside your desktop session needs no environment variable: `--wayland-display NAME` (or an absolute path), `--runtime-dir DIR`, `--x11-display :N`, `--xauthority FILE` and `--allow-uid UID` replace `WAYLAND_DISPLAY`, `XDG_RUNTIME_DIR`, `DISPLAY`, `XAUTHORITY` and `SUDO_UID`; the variables stay as the fallback. The broker creates a missing socket directory (mode 0700, one level). These flags were tested against stand-in sockets, not yet in a real Wayland or X11 session.

## Environment variables

A full-performance run needs none. Variables beginning with `KF3_` exist for development and benchmarking. They are not part of the supported interface and are not listed here.

## The Windows guest script

`kayfabe-windows-setup.bat` sets the guest settings a Windows guest needs. See [Run a Windows guest](windows-guest.md).
