# Goals and plans

These are **goals**, not promises. Kayfabe is a research project with no release schedule, and the owner can change any of them. Each item says where it comes from and, in one line, what exists **today**. Where the repository gives no date or no evidence, the item says so. <!-- src: README.md status line; docs/OWNER_RULINGS.md section AO -->

## Hypervisor independence

**Goal (the owner's stated goal, 2026-10-27):** kayfabe should not depend on one hypervisor. The design keeps the core independent of the hypervisor, and only a thin shim knows the hypervisor it runs under. <!-- src: docs/OWNER_RULINGS.md section AO; docs/design/THE_ARCHITECTURE_v3.md (version-axis table, row V); docs/design/THE_DESIGN.md -->

**Status today:** not implemented. Kayfabe is a QEMU device (`kf3-gpu`), and **only QEMU is supported**; a second hypervisor needs a shim that does not exist. By design the core reaches the hypervisor only through two traits (`Vmm` and `HostOps`) and has no QEMU dependency, and the display broker is a separate program written to be VMM-neutral. <!-- src: docs/howto/requirements.md "Operating system and CPU"; ARCHITECTURE.md (kf-core and kf-broker rows); crates/kf-core/Cargo.toml and src/lib.rs -->

## arm64 hosts

**Goal:** arm64 (aarch64) support later, after x86-64. The owner's ruling of 2026-10-03 says x86-64 ships first and the boot-display driver stays architecture-neutral so nothing x86-specific creeps in. The owner's priority order of 2026-10-22 puts all GPU families first, then arm64 and several GPUs in one guest. <!-- src: docs/OWNER_RULINGS.md section K (aarch64 later) and section AO -->

**Status today:** no run on arm64 is documented. The continuous-integration job only cross-checks that the core builds for aarch64 and that the boot-display driver builds for an aarch64 UEFI target. Hosts are Linux on x86-64 only. <!-- src: docs/howto/requirements.md "arm64"; docs/CI_V3.md; .github/workflows/ci.yml (aarch64 job) -->

## Several GPUs in one guest

**Goal:** a guest with several kayfabe devices, one per host GPU, including Windows guests, and a way to choose which GPU serves the display. In the owner's priority order of 2026-10-22 it comes after all GPU families work, Blackwell included; that is a priority, not a rule that nothing else may be started. <!-- src: docs/OWNER_RULINGS.md section AO; docs/design/V3_MULTI_GPU_AUDIT.md section 1 (one kf3 device per host GPU, several in one VM); docs/design/V3_FEATURE_GAPS.md H20 -->

**Status today:** one kayfabe device is created per host GPU. A Linux thin guest with two distinct GPUs passed copy-engine checks serially and concurrently [measured in `docs/design/V3_MULTI_GPU_AUDIT.md` section 7, revision `74fced87`, lease 52664396, a rented 8x RTX 3060 box]. The full 30-arm regression was a separate single-device run. Not measured: CUDA inside a two-GPU guest, unified memory across two GPUs, and Windows or the display with several GPUs. Peer-to-peer access between guest GPUs is refused by name and has no decision yet. <!-- src: docs/STATUS_DETAIL.md section 3; docs/howto/linux-guest.md "More than one GPU"; docs/design/V3_FEATURE_GAPS.md H20, L10 -->

## Kayfabe on NVIDIA MIG partitions

**Goal (the owner's, 2026-10-28):** run a kayfabe process bound to one slice of a GPU that NVIDIA's Multi-Instance GPU (MIG) has partitioned. NVIDIA documents that each MIG instance has its own compute and memory, with separate paths through the memory system. By design, a kayfabe process on one slice would be an ordinary client of that slice, and its VM would see only the slice, which would give each VM a hard resource boundary. Time-slicing does not: NVIDIA's GPU Operator documentation says of it that there is "no memory or fault-isolation between replicas". Sharing a GPU by time-slicing is sharing, not isolation, and this page does not call either a guarantee. <!-- src: owner request 2026-10-28; https://docs.nvidia.com/datacenter/tesla/mig-user-guide/introduction.html and https://docs.nvidia.com/datacenter/cloud-native/gpu-operator/latest/gpu-sharing.html (retrieved 2026-10-28) -->

**Status today:** not tested. No MIG-capable GPU has been available to the project. NVIDIA's table of MIG-supported GPUs (Ampere and later) lists data-centre products and RTX PRO products, and no GeForce card appears in it [retrieved 2026-10-28, [supported GPUs](https://docs.nvidia.com/datacenter/tesla/mig-user-guide/supported-gpus.html)]. Kayfabe reads per-die facts from the host GPU, so by design a slice would be described by what the host reports for the slice; that has not been run. Of the dies in NVIDIA's table, GA100 and GB10B are refused by name today, and Hopper has never run on hardware. This goal comes after the GPU families in the owner's priority order. <!-- src: docs/howto/requirements.md "GPU" notes; docs/STATUS_DETAIL.md section 0; docs/OWNER_RULINGS.md section AO (priority order) -->

## All NVIDIA families from Turing on

**Goal:** every family from Turing to Blackwell, Hopper included, is a first-class target, not only the one the author owns. Per-die facts are read from the host GPU, and only a small per-family table is kept by hand. Pre-Turing cards cannot work (there is no GSP firmware to stand in for). <!-- src: docs/OWNER_RULINGS.md "All GPU families are first-class" (principle 7); docs/howto/technical-faq.md "Why only the GSP path?" -->

**Status today:** runs are recorded on Turing (TU116), Ampere (GA106, GA104), Ada (AD104, AD106, AD103, AD102) and Blackwell (GB205), mostly with Linux guests. Hopper and datacentre Blackwell were never run on hardware, and two parts are refused by name (GA100, GB10B). See [Requirements](requirements.md). <!-- src: docs/howto/requirements.md "GPU"; docs/STATUS_DETAIL.md section 0 -->

## Windows guests and display

**Goal:** Windows is the end goal of the Mode-2 design (the guest runs NVIDIA's own driver), with the guest's screen shown on the host desktop through the display broker. Windows sleep/wake and more GPU dies are on the list to fix. <!-- src: docs/PROJECT_HISTORY.md sections 1 and 3; docs/OWNER_RULINGS.md section C item 6; README.md "Not finished" -->

**Status today:** Windows 11 runs a desktop and a set of applications on the author's RTX 4070 with NVIDIA's 580.88 driver and three registry settings. Sleep/wake does not survive, and one Blackwell die (GB206) fails under Windows with Code 43. See [Windows guest](windows-guest.md) and [Display and the broker](display.md). <!-- src: README.md "What runs today" and "Not finished"; docs/howto/windows-guest.md; docs/howto/requirements.md -->

## Driver versions beyond the tested set

**Goal:** a driver matrix over the same range nvkvm-pv supports (535 to 610), host and guest versions chosen independently, on every family. <!-- src: docs/OWNER_RULINGS.md section C item 5 (driver matrix) -->

**Status today:** host drivers 570.148.08, 575.57.08, 580.65.06, 580.95.05, 580.159.04 and 595.91.07, and guest drivers 575.57.08, 580.105.08, 580.159.04, 590.48.01, 595.84 and 610.57.04, have passed the recorded checks, on a few boxes each. Hosts older than 570 are untested. Display layouts exist for one Linux guest version, and Windows has run with 580.88 only. The exact lists are on [Requirements](requirements.md). <!-- src: docs/howto/requirements.md "NVIDIA driver on the host" and "Guest"; docs/design/V3_DRIVER_MATRIX.md section 6.0 -->

## An installer and a stable interface

**Goal:** an install path that is easier than VFIO (packaging first, then integration with common virtual-machine managers), and a stable interface for the device. The pre-release checks include a security audit before the first public binary. <!-- src: docs/PRODUCT_POSITIONING.md (status update 2026-10-01, "Conversion needs"); docs/design/V3_FEATURE_GAPS.md H19 -->

**Status today:** none of these exist. There is no installer, no stable interface, no public binary and no security audit. You build from source with the repository's scripts. See [Running the QEMU device](installing.md). <!-- src: README.md status line and "Trying it"; docs/howto/installing.md; docs/howto/security.md -->

## Optional enhancements

**Goal:** optional cooperation stages on top of the stock path: a helper module in the guest, and patched guest or host drivers, for things such as doorbell writes without a VM exit and demand-paged unified memory. They are planned and optional, and the stock path never requires them. <!-- src: docs/design/V3_COOPERATIVE_TIERS.md section 1; docs/design/V3_HOST_PATCH_LIST.md section 1 (owner policy 2026-10-10) -->

**Status today:** design only, nothing in the stock path depends on it; one research patch to the host's `nvidia-uvm` exists. <!-- src: docs/STATUS_DETAIL.md section 0 -->

## Where plans change

The owner's rulings, with their dates, are kept in the repository. This page is rewritten when a ruling changes; an item here is only as current as the date of its source. The status of what works today is on the [introduction](index.md) and [Requirements](requirements.md). <!-- src: docs/OWNER_RULINGS.md; docs/STATUS_DETAIL.md -->

Verified on: every item was read from the repository files named in its comments as of master (2026-10-27). Not verified end to end: nothing on this page was run, and the goals have no delivery dates.
