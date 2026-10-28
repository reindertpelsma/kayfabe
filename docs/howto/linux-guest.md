# Linux guest

A Linux guest uses NVIDIA's stock driver, unmodified, and nothing has to be set in the guest: no registry-style settings, no guest patches, no custom kernel module. <!-- src: README.md "Linux guest", docs/WINDOWS_GUEST_SETUP.md "Linux guests" -->

## What has been run

All of these are the repository's claims, with the revision or box where it states one. <!-- src: README.md, docs/STATUS_DETAIL.md -->

- **The 30-arm suite passes** in the thin guest (also with the relay forced on), with the GPU gates at 9/9. [measured: kayfabe `afb552ea`, RTX 3060, 2026-09-30; GA104 at `714ec29d3`, 2026-10-24] <!-- src: docs/STATUS_DETAIL.md section 0, traces/sw3_die_sweep_20261024/README.md -->
- **CUDA is correct:** `cup3` returns 43, `cup8` (a 2048 x 2048 matrix multiply) returns `BAD=0 MAXERR=0`. <!-- src: docs/STATUS_DETAIL.md section 2 -->
- **61 of 65 CUDA applications** pass (CUDA samples, PyTorch, Hugging Face generation, llama.cpp), with 6 of 6 stream probes and 100 processes in one boot, and outputs equal to bare metal [measured, an earlier revision, a rented RTX 3060]. Four workloads need unified (UVM) memory paging and fail correctness checks, including CUDA error 719, a host Xid 31 and a recorded wrong result despite SUCCESS. <!-- src: README.md, docs/STATUS_DETAIL.md section 2 -->
- **NVENC and NVDEC output is byte-identical to bare metal** in the graded lane [measured, RTX 3060]. <!-- src: docs/STATUS_DETAIL.md section 3 -->
- **Headless graphics:** Vulkan, EGL, and GLX through Xvfb and VirtualGL; 38 of 38 items of the test set, 31 byte-identical to bare metal [measured, GA104]. <!-- src: docs/STATUS_DETAIL.md section 0 -->
- **A desktop** with display planes (overlay, YUV) and 3 of 3 suspend/resume cycles. See [Display and the broker](display.md). <!-- src: README.md -->
- **Driver versions:** see [Requirements](requirements.md).

Not done: demand-paged managed memory (4 applications), a rootless boot record on master (one exists on the branch `claude/rootless-audit-20261016`, see [Security model](security.md)), and any statement about sharing performance between two VMs on one GPU (two VMs have booted together; see the [FAQ](../FAQ.md)). <!-- src: README.md, docs/PRODUCT_POSITIONING.md -->

## Prepare a guest image

The bench builds its guest image once, from an Ubuntu 24.04 (noble) cloud image, on **stock QEMU** (no kayfabe), then reuses it. These are the steps `scripts/bench/provision_bench_tree.sh` performs. The bench scripts assume the host-mode driver: the same `.run` as the host. <!-- src: scripts/bench/provision_bench_tree.sh -->

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

## Boot it on kayfabe

Use a QEMU built by `build_kf3.sh`, and start it with the device as shown in [Running the QEMU device](installing.md). The bench's launcher for a full guest is `scripts/bench/boot_nvkvm.sh <tag> [extra qemu args...]`, run in the foreground; `scripts/bench/boot_capture.sh <tag>` boots and also stores the guest driver's own `dmesg` and the host driver's log for the boot in files. <!-- src: scripts/bench/boot_nvkvm.sh, scripts/bench/boot_capture.sh -->

What the launcher sets, from its source:

- shared memfd RAM and `-machine q35,accel=kvm` (2048 MB unless `NVKVM_RAM_MB`), `-cpu host`, `-smp 3` unless `KF_SMP`;
- a virtio disk from `KF_GUEST_IMG` (default `/workspace/bench/guest.qcow2`) and a virtio-net on the `nvktap0` tap;
- BAR1 of 128 MiB (`KAYFABE_GUEST_BAR1_MB`), BAR2 of 32 MiB, and `fb-mb` from `KF3_FB_MB`, else 8192 MiB, or (VRAM - 2048 MiB) on cards under 10 GiB;
- `KF_FIRMWARE=ovmf` for UEFI firmware instead of SeaBIOS (needed by the boot display, whose option ROM is a UEFI driver).

<!-- src: scripts/bench/boot_nvkvm.sh -->

Then, inside the guest, the driver loads as it would on hardware. `nvidia-smi` shows the single GPU, which the host keeps using. <!-- src: README.md (screenshot caption) -->

The guest driver's `dmesg` is not in the serial log unless the harness captures it, and a serial console is slow: verbose tracing over the 16550 UART once consumed most of a run's time budget, so the harness uses a virtio console. <!-- src: AGENTS.md "Bench traps", scripts/fastguest/run_fast_guest.sh -->

## The thin guest (a test environment, not a desktop)

For testing, a guest with no disk and no services exists: a kernel and initrd that load the driver, run the raw client and power off. Results come over the serial console. Build it with `scripts/fastguest/build_fast_guest.sh` and run it with `scripts/fastguest/run_fast_guest.sh`. See [Running the QEMU device](installing.md) for the checks. <!-- src: scripts/fastguest/build_fast_guest.sh, scripts/fastguest/run_fast_guest.sh -->

## More than one GPU

One kayfabe device is created per host GPU. `KF3_GPUS=0,1` makes the thin-guest harness add one `kf3-gpu,gpu-minor=<m>` per listed host minor. A thin guest with two distinct host GPUs passed copy-engine checks serially and concurrently [measured in `docs/design/V3_MULTI_GPU_AUDIT.md` section 7, revision `74fced87`, lease 52664396, two RTX 3060s on one host]. The full 30-arm regression was a separate single-device run. CUDA inside a two-GPU guest, UVM across two GPUs and peer access were not measured. <!-- src: scripts/fastguest/run_fast_guest.sh, docs/STATUS_DETAIL.md section 3 -->

## Not documented yet

- A supported way to build a guest image on other distributions.
- A supported guest-kernel range. Recorded examples include the thin guest on 6.8.0-59 and the full guest on 6.8.0-139. The branch `codex/sweep-integrate-20261018`, not merged, also records 6.8.0-147-generic in a GB202 desktop attempt whose presentation failed [measured at revision `24906bf33`, lease 56788212; `traces/sweep_integrate_20261018/phase2_blackwell_56788212/B2_RESULT.md` on that branch]. That extends the record; it is not a support promise.
- The proprietary (closed) guest module.
- A guest-side guide for container runtimes.

Verified on: Linux guest results are recorded for kayfabe `afb552ea` (RTX 3060 GA106, 2026-09-30) and `714ec29d3` (GA104, host and guest 595.91.07, 2026-10-24); the application matrix is from an earlier revision on a rented RTX 3060. Not verified end to end: the image-preparation steps above were read from the script, not run for this page.
