# Running the QEMU device

**There is no installer.** Kayfabe is started today by the repository's own bench scripts, which were written for rented, disposable test hosts and run as root. This page describes what those scripts do and the QEMU device they start, so you can reproduce it on a machine you dedicate to the purpose. Every command names the file it comes from. <!-- src: README.md "Trying it", scripts/bench/display/interactive.sh -->

> **Warning.** The provisioning scripts purge NVIDIA driver packages, stop the host's display manager, load and unload kernel modules, create a tap network device, change `ip_forward` and add `iptables` rules. They are written for throwaway test machines. Do not run them on a machine you care about; read them first. <!-- src: scripts/bench/provision_host_driver.sh, scripts/bench/provision_bench_tree.sh -->

## Overview

1. Put the **open** NVIDIA driver on the host.
2. Build the QEMU binary with the `kf3-gpu` device ([Building from source](building.md)).
3. Prepare a guest ([Linux guest](linux-guest.md), [Windows guest](windows-guest.md)).
4. Start QEMU with a `kf3-gpu` device. One GPU job at a time.

## 1. The host driver

The recorded runs use the open kernel module. The script that installs it for the bench is `scripts/bench/provision_host_driver.sh` (run as root; default version 580.159.04):

```sh
HOST_DRIVER=580.159.04 bash scripts/bench/provision_host_driver.sh
```
<!-- src: scripts/bench/provision_host_driver.sh -->

What it does, in order: blacklists `nouveau`; un-holds and purges every `nvidia-*` / `libnvidia-*` package; stops a running display manager (`display-manager`, `sddm`, `gdm3` or `lightdm`) and `nvidia-persistenced`; unloads the NVIDIA modules; removes the dkms tree; installs the compiler the running kernel was built with if needed; runs

```sh
sh NVIDIA-Linux-x86_64-<version>.run --silent --no-x-check --no-nouveau-check --no-questions --dkms -m=kernel-open -j8
```

then loads the module and, if `nvidia-smi` does not name the GPU, tries up to three unload / PCI reset / reload recoveries. The `.run` comes from `https://us.download.nvidia.com/XFree86/Linux-x86_64/<version>/`, with the `tesla/<version>/` path as a fallback. <!-- src: scripts/bench/provision_host_driver.sh -->

**Check the result on content, not on the installer's exit status:** `/proc/driver/nvidia/version` must say "Open Kernel Module". <!-- src: scripts/bench/provision_host_driver.sh header -->

## 2. The bench tree

`scripts/bench/provision_bench_tree.sh` (run as root after the driver step) prepares everything under `/workspace/bench` (override with `BENCH_DIR` in the launchers): <!-- src: scripts/bench/provision_bench_tree.sh -->

- installs the packages of `provision_box.sh --packages bench`;
- downloads QEMU 10.2.4 and the Ubuntu 24.04 (noble) cloud image;
- creates a tap device `nvktap0` with host address `192.168.77.1/24`, enables IPv4 forwarding and adds a masquerade rule and two `FORWARD` accepts for it; the guest sits at `192.168.77.2`;
- generates an ssh key `guest_key`;
- builds the kayfabe QEMU with `build_kf3.sh` (into `qemu-build-kf3`, and `kf3-bins/<rev>/`);
- builds a guest disk `guest.qcow2` (the cloud image plus 30 GB), seeds it with cloud-init, boots it on stock QEMU, and installs the guest driver (see [Linux guest](linux-guest.md));
- builds the guest-side raw client (musl).

The script expects the **guest driver `.run` of the same version as the host** at `/root/NVIDIA-Linux-x86_64-580.159.04.run`; that version string is fixed in the script. <!-- src: scripts/bench/provision_bench_tree.sh -->

The tap network is the bench's choice. The Windows harness uses user-mode networking instead, so it needs no root network device. <!-- src: scripts/bench/windows/README.md -->

## 3. One GPU job at a time

The launch scripts take an exclusive lock, `/tmp/kayfabe-fastguest.lock` (override with `KF_LOCK`), and wait for it. GPU runs are strictly serial: two concurrent runs of the fast lane kill each other's QEMU. <!-- src: scripts/fastguest/run_fast_guest.sh -->

Between runs the launcher also waits (up to about 30 s) for the previous VM's GPU memory to drain (`KF_GPU_IDLE_MIB`, default 512). <!-- src: scripts/fastguest/run_fast_guest.sh -->

## 4. The `kf3-gpu` device

Add the device to a QEMU command line built from the binary in `kf3-bins/<rev>/`. The device is a PCI display device. It takes its identity (vendor and device ids, the die's facts) from the host GPU at start. <!-- src: qemu/hw/misc/kf3/kf3.c, crates/kf-qemu/src/device.rs -->

### Machine arguments it needs

```
-object memory-backend-memfd,id=ram0,size=<N>M,share=on
-machine q35,accel=kvm,memory-backend=ram0 -m <N> -cpu host
```
Use a shared memfd for guest RAM: it is the tested configuration. With plain `-m` the device still starts, but the paths that need a file offset refuse by name and the file-less mapping has not been run on a GPU (`requirements.md`). Use `-vga none` when the device provides the display. <!-- src: scripts/fastguest/run_fast_guest.sh, scripts/bench/display/interactive.sh -->

### Device properties

Every property below is declared in `qemu/hw/misc/kf3/kf3.c`. Defaults are as declared there. <!-- src: qemu/hw/misc/kf3/kf3.c (kf3_properties) -->

| Property | Default | What it does |
|---|---|---|
| `gpu-minor` | `0` | Which host GPU (the `/dev/nvidia<minor>` number). One device per host GPU; the same card twice is refused at start if the BAR1 budget does not fit. |
| `fb-mb` | `8192` | Size of the guest's video memory in MiB. It is a real allocation of host VRAM and must fit the card. |
| `bar1-size` | 256 MiB (bytes) | Guest BAR1 size. The harnesses pass `134217728` (128 MiB) because 256 MiB plus headroom does not fit a 256 MiB host aperture. |
| `bar2-size` | 32 MiB (bytes) | Guest BAR2 size; the harnesses pass `33554432`. |
| `bar1-overlays` | built-in default | Hopper and newer only: how many BAR1 doorbell views can be trapped at once. |
| `msix-vectors` | `32` | MSI-X vectors. |
| `guest-driver` | host's driver version | The guest's driver version, for the layouts kayfabe must speak. The guest driver's own version string is checked against it when the guest driver starts; a wrong declaration is a named refusal. Example from the Windows harness: `580.65.06` for Windows driver 580.88. |
| `display` | off | Provide a virtual NVIDIA display the guest's driver drives. See [Display and the broker](display.md). |
| `gop` | off | Boot display: a UEFI driver in the card's expansion ROM shows the firmware and boot screens. The harnesses always set it together with `display=on`. |
| `gop-efi` | none | Path of the boot display driver, signed (for Secure Boot). Refused with `gop=off`. |
| `x11-dispsw` | off | Experimental; needs `display=on`. |
| `gpu-uuid` | `auto` | The guest-visible GPU UUID: `auto`, `random`, `host`, or `GPU-xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`. Built to give each VM a distinct UUID. [Code and GPU-free tests only as of 2026-10-08; not run on hardware when written.] |
| `vm-id` | QEMU's `-uuid` | The VM's identity text for the above. |
| `channel-budget` | `0` (derived from the host) | GPU channels per runlist told to the guest. Refused below 256 or above what the host can give (or 2048). |
| `user-view-keep` | `0` (= 16) | Windows user-ring tuning: CPU views of the store that each channel keeps armed, up to 256. |
| `doorbell-ioeventfd` | off | Doorbell fast path; off until measured. |
| `doorbell-ioeventfd-max` | `256` | Cap for the fast path. |
| `display-broker` | unset | Absolute path of the display broker's socket. See [Display and the broker](display.md). |
| `display-broker-uid` | `-1` | One more uid accepted as the broker. |
| `display-broker-vram` | `auto` | GPU-copy path to the broker: `auto`, `on`, `off`. |
| `display-max-fps` | `0` | Cap on the emulated vblank, 24 to 75 Hz; `0` means a 75 Hz cap. Refused outside 24..75 and without `display=on`. |
| `dummy-bar` | off | A synthetic MMIO trap-floor probe: it adds lockless, BQL-held and ioeventfd test pages in the MSI-X BAR. It does not submit work to the GPU. [code reading: `qemu/hw/misc/kf3/kf3.c`, `docs/design/V3_DOORBELL_BASELINE.md`] |
| `x-gsp-observer`, `x-gsp-observer-seconds` | unset, `300` | Diagnostic: record the guest driver's GSP traffic to a fresh file for 1 to 7200 s. |

Properties are given as `-device kf3-gpu,fb-mb=8192,bar1-size=134217728,bar2-size=33554432,id=kf0`, or for one property on an existing device `-global kf3-gpu.<property>=<value>`. <!-- src: scripts/fastguest/run_fast_guest.sh, scripts/bench/windows/win_vm.sh -->

Environment variables beginning with `KF3_` also exist (for example `KF3_RPC_TRACE=1` logs every command the emulated GPU answers). They are for development and benchmarking. A complete, supported list is **not documented yet**; the harness sets what it needs. <!-- src: scripts/bench/windows/win_vm.sh -->

### Example: the Linux desktop guest's device line

Assembled from `scripts/bench/display/interactive.sh` (not run as written for this page):

```
-device kf3-gpu,id=kf0,guest-driver=580.159.04,fb-mb=8192,bar1-size=134217728,bar2-size=33554432,display=on,gop=on,display-broker=<socket>,display-broker-uid=<uid>
```
(The script also sets `x11-dispsw=on`; see [Display and the broker](display.md).) <!-- src: scripts/bench/display/interactive.sh -->

## 5. Checks you can run

All of these assume the bench tree above. They are strictly serial, one box at a time. <!-- src: AGENTS.md "Verification" -->

| Check | Command | Passing |
|---|---|---|
| GPU-free unit tests | `cargo test -p kf-<crate>` | tests pass |
| GPU gates (no QEMU) | `bash scripts/bench/v3_gates.sh [logfile]` | `V3_GATES_EXIT pass=9 fail=0 ...` and exit status 0. It also runs a tenth probe, `kf-micro-reserve-probe reserve`. |
| Host baseline, no VM | `bash scripts/fastguest/bare_metal_suite.sh <tag> [budget] [arms...]` | 30 arms pass on the host's own driver |
| 30-arm thin guest | `KF_DEVICE=kf3 bash scripts/fastguest/fast_suite.sh <tag> 180` | `FAST_SUITE_PASS=30 FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=0 NOTRUN=0` |
| One thin-guest run | `KF_ARMS="--timer --engines" bash scripts/fastguest/run_fast_guest.sh <tag> <budget-seconds>` | `FAST_VERDICT=PASS` |
| CUDA ladder (full guest) | `bash scripts/bench/cuda_ladder.sh host\|guest <tag> [reps] [rungs]` | the four rungs `cup2`, `cup3`, `cup8`, `cup8bench` pass |

<!-- src: scripts/bench/v3_gates.sh, scripts/fastguest/bare_metal_suite.sh, scripts/fastguest/fast_suite.sh, scripts/fastguest/run_fast_guest.sh, scripts/bench/cuda_ladder.sh -->

Rules the harness applies, which are worth copying to any setup of your own:

- **A timeout is a crash.** The thin guest gets one hard budget for the whole run (QEMU start to power-off). <!-- src: scripts/fastguest/run_fast_guest.sh -->
- **A green run must have reached hardware.** The thin guest run fails if a guest channel was never forwarded to the GPU (`KF_REQUIRE_FORWARD=0` turns that off for a deliberate control run). <!-- src: scripts/fastguest/run_fast_guest.sh -->
- **If bare metal passes and the guest fails, the defect is kayfabe's.** Run the host baseline first. <!-- src: AGENTS.md -->
- **Watch the host's kernel log for Xid messages across a run.** A host GPU fault does not show up in the guest. `bash scripts/bench/host_xid_watch.sh <tag> -- <command>` runs a command and exits 6 if a new host Xid appeared across it. <!-- src: scripts/bench/host_xid_watch.sh -->
- **Write a start marker and an exit line** for every job; a killed job and a running one look alike otherwise. <!-- src: AGENTS.md "Bench traps" -->

## 6. Stopping a guest

The launchers stop a guest by asking it to power down (ACPI), then waiting, and only then killing QEMU. The interactive Linux launcher uses `scripts/bench/display/interactive.sh stop`; the Windows harness uses `scripts/bench/windows/win_vm.sh stop` (guest shutdown, then QMP `system_powerdown`, then QMP `quit`, never SIGKILL). <!-- src: scripts/bench/display/interactive.sh, scripts/bench/windows/README.md -->

## Not documented yet

- A supported rootless installation guide. The branch `claude/rootless-audit-20261016` has [launch notes and permission checks](https://github.com/reindertpelsma/kayfabe/blob/32d7319b2/traces/rootless_audit_20261016/REPORT.txt) in sections 1, 2 and 7, with scripts under `scripts/bench/rootless/`. That audit is not merged, and its host-specific launchers are not a general installer. The scripts described here still run as root. See [Security model](security.md).
- libvirt, systemd units, container images and sandbox profiles. The design leaves sandboxing the (single) QEMU process to the deployer: see [Security model](security.md).
- A complete reference for the `KF3_*` environment variables.
- Several VMs on one GPU as a supported configuration.

Verified on: the scripts and properties above were read from the repository at master (`04773c348`). Their results are recorded in the repository for kayfabe `afb552ea` (gates 9/9, bare metal 30/30, thin 30/30, RTX 3060, 2026-09-30) and `714ec29d3` (GA104, 2026-10-24). Not verified end to end for this page: no script on it was run.
