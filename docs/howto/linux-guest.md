# Run a Linux guest

A Linux guest uses NVIDIA's stock driver, unmodified, and nothing has to be set in the guest: no registry-style settings, no guest patches, no custom kernel module. <!-- src: README.md "Linux guest", docs/WINDOWS_GUEST_SETUP.md "Linux guests" -->

## The steps

1. **Install a Linux guest** the way you always do, on QEMU/KVM or libvirt. Any distribution that NVIDIA's driver supports is expected to work; the project has run Ubuntu, Rocky and Linux Mint guests, and does not document a supported distribution list.
2. **Install NVIDIA's driver in the guest**: the `.run` installer with `-m=kernel-open`, or your distribution's package of the open kernel module. The guest needs a C compiler and the kernel headers for the running kernel if the installer builds the module.
3. **Start the VM with the kayfabe device** instead of the VFIO device ([Install](installing.md), [Command-line reference](cli-reference.md)). If you already have a VM, [Attach to a VM you already have](attach-to-existing-vm.md) lists what changes.
4. **Check it**: `nvidia-smi` in the guest names the GPU, and the host keeps using the card.

Not verified yet: that an image prepared for VFIO boots unchanged with only the device swapped. That is the aim; an equivalence test that boots the same image both ways is not in the tree yet.

Which driver version: a guest driver close to the host's is recommended. A guest on another driver branch needs `guest-driver=<version>` on the device; see [Troubleshooting](troubleshooting.md#the-guest-driver-and-the-host-driver-do-not-match). The versions that have been run are listed in [Requirements](requirements.md).

## What has been run

All of these are the project's claims about its own test runs; per-card detail is on [Requirements](requirements.md). <!-- src: README.md, docs/STATUS_DETAIL.md -->

- **The 30-arm test suite passes** in the thin guest (also with the relay forced on), with the GPU gates at 9 of 9. It passed on an RTX 3060 and on a GA104 card. <!-- src: docs/STATUS_DETAIL.md section 0, traces/sw3_die_sweep_20261024/README.md -->
- **CUDA is correct:** `cup3` returns 43, `cup8` (a 2048 x 2048 matrix multiply) returns `BAD=0 MAXERR=0`. <!-- src: docs/STATUS_DETAIL.md section 2 -->
- **Application matrix:** 70 of 70 runnable applications passed on the host (RTX 4090) and 66 of 70 in the Linux guest. Passing applications met their own correctness checks; three digest-producing applications matched the host. **Unsupported for now (unified memory demand paging):** `UnifiedMemoryStreams`, `UnifiedMemoryPerf`, `conjugateGradientUM` and `attach_verify`. Reports list them as `unsupported (UVM)`. `conjugateGradientUM` printed SUCCESS with a wrong numerical result, so an exit code alone is not a correctness check. <!-- src: traces/apps_linux_20261107/README.md; docs/OWNER_RULINGS.md section E -->
- **NVENC and NVDEC output is byte-identical to bare metal** in the graded lane (RTX 3060). <!-- src: docs/STATUS_DETAIL.md section 3 -->
- **Headless graphics:** Vulkan, EGL and GLX through Xvfb and VirtualGL: 38 of 38 items of the test set passed, 31 of them byte-identical to bare metal (GA104). <!-- src: docs/STATUS_DETAIL.md section 0 -->
- **A desktop** with display planes (overlay, YUV) and 3 of 3 suspend and resume cycles. See [Display and the broker](display.md). <!-- src: README.md -->

Not done: demand-paged managed memory (four applications) and any statement about sharing performance between two VMs on one GPU (two VMs have booted together; see the [FAQ](../FAQ.md)). <!-- src: README.md, docs/PRODUCT_POSITIONING.md -->

## More than one GPU

One kayfabe device is created per host GPU: add one `-device kf3-gpu,id=...,gpu-minor=<m>` per host GPU. A thin guest with two distinct host GPUs passed copy-engine checks serially and concurrently (two RTX 3060s on one host). CUDA inside a two-GPU guest, unified memory across two GPUs and peer access were not measured. <!-- src: scripts/fastguest/run_fast_guest.sh, docs/STATUS_DETAIL.md section 3, docs/design/V3_MULTI_GPU_AUDIT.md section 7 -->

## Not documented yet

- A supported way to build a guest image on other distributions, and a supported guest-kernel range. Guests on kernels 6.8.0-59 and 6.8.0-139 have run, which is a record and not a promise.
- The proprietary (closed) guest kernel module.
- A guest-side guide for container runtimes.

## Appendix. How the project's own test guest is prepared

The project's tests use a throwaway guest built from an Ubuntu 24.04 (noble) cloud image on stock QEMU, then reused. You do not need these steps for your own guest. They are what `scripts/bench/provision_bench_tree.sh` performs, written out so that you can follow them with another distribution. <!-- src: scripts/bench/provision_bench_tree.sh -->

```sh
qemu-img convert -O qcow2 noble-server-cloudimg-amd64.img guest.qcow2
qemu-img resize guest.qcow2 +30G
cloud-localds seed.iso user-data meta-data      # an ubuntu user, your ssh key, a static address
```
Boot it once on stock QEMU (`-enable-kvm -cpu host -m 8G -smp 8`, two virtio drives, user networking with an ssh forward), wait for cloud-init to finish, then in the guest:

```sh
sudo apt-get install -y build-essential linux-headers-$(uname -r)
# copy NVIDIA-Linux-x86_64-<version>.run to the guest, then:
sudo sh nv.run --silent --no-x-check --no-nouveau-check --no-questions -m=kernel-open -j8
modinfo nvidia | grep -E '^version|^license|^vermagic'     # check the version on content
sudo poweroff
```
<!-- src: scripts/bench/provision_bench_tree.sh (track B) -->

Notes from the script:

- The guest needs a C compiler and kernel headers for the running kernel; if the installer's log stops at "Kernel module compilation complete" with no error, the driving ssh session was probably killed mid-install and the modules were built but never installed. <!-- src: scripts/bench/provision_bench_tree.sh -->
- All recorded runs use `-m=kernel-open`, the open kernel module.
- The script also installs a `cuda.h` into the guest for the CUDA test programs (the CUDA runtime archive, version 12.6.77). <!-- src: scripts/bench/provision_bench_tree.sh -->
- The cloud-init seed in the script gives the guest the address `192.168.77.2/24` on the host's `nvktap0` network. Edit it if your network differs.

For other guest driver versions the driver-matrix scripts build one guest image per version (`scripts/drivermatrix/stage_fat_guest.sh`); the launcher takes the image from `KF_GUEST_IMG`. <!-- src: scripts/bench/boot_nvkvm.sh -->


### Booting it with the helper scripts

Use a QEMU built by `build_kf3.sh` ([Install](installing.md)). The test launcher for a full guest is `scripts/bench/boot_nvkvm.sh <tag> [extra qemu args...]`, run in the foreground; `scripts/bench/boot_capture.sh <tag>` boots and also stores the guest driver's own `dmesg` and the host driver's log for the boot in files. <!-- src: scripts/bench/boot_nvkvm.sh, scripts/bench/boot_capture.sh -->

What the launcher sets, from its source:

- shared memfd RAM and `-machine q35,accel=kvm` (2048 MB unless `NVKVM_RAM_MB`), `-cpu host`, `-smp 3` unless `KF_SMP`;
- a virtio disk from `KF_GUEST_IMG` (default `/workspace/bench/guest.qcow2`) and a virtio-net on the `nvktap0` tap;
- BAR1 of 128 MiB (`KAYFABE_GUEST_BAR1_MB`), BAR2 of 32 MiB, and `fb-mb` from `KF3_FB_MB`, else 8192 MiB, or (VRAM - 2048 MiB) on cards under 10 GiB;
- `KF_FIRMWARE=ovmf` for UEFI firmware instead of SeaBIOS (needed by the boot display, whose option ROM is a UEFI driver).

<!-- src: scripts/bench/boot_nvkvm.sh -->

Then, inside the guest, the driver loads as it would on hardware. `nvidia-smi` shows the single GPU, which the host keeps using. <!-- src: README.md (screenshot caption) -->

The guest driver's `dmesg` is not in the serial log unless the harness captures it, and a serial console is slow: verbose tracing over the 16550 UART once consumed most of a run's time budget, so the harness uses a virtio console. <!-- src: AGENTS.md "Bench traps", scripts/fastguest/run_fast_guest.sh -->

### The thin guest (a test environment, not a desktop)

For testing, a guest with no disk and no services exists: a kernel and initrd that load the driver, run the raw client and power off. Results come over the serial console. Build it with `scripts/fastguest/build_fast_guest.sh` and run it with `scripts/fastguest/run_fast_guest.sh`. See [What you can test](test-yourself.md) for the checks. <!-- src: scripts/fastguest/build_fast_guest.sh, scripts/fastguest/run_fast_guest.sh -->

