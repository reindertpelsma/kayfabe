+++
title = "How it differs"
description = "The choice is what you share, which driver the guest runs, and what you give up."
template = "detail.html"
[extra]
src = "README.md comparison; docs/FAQ.md; individual sources below"
+++

## How the options differ

### VFIO passthrough

<!-- src: https://www.kernel.org/doc/html/latest/driver-api/vfio.html (binding a device to vfio-pci); README.md comparison table -->
VFIO gives the whole card to one VM. While it runs, the host cannot use that card. Kayfabe leaves it with the host.

### NVIDIA vGPU

<!-- src: https://docs.nvidia.com/vgpu/latest/grid-vgpu-user-guide/index.html; https://docs.nvidia.com/vgpu/gpus-supported-by-vgpu.html (supported workstation and data-centre cards); https://open-iov.org/index.php/System_Overview and https://open-iov.org/index.php/Install_and_First_Run (retrieved 2026-10-28) -->
NVIDIA vGPU shares a card between VMs. It needs a GPU on [NVIDIA's supported list](https://docs.nvidia.com/vgpu/gpus-supported-by-vgpu.html), plus vGPU software and a licence. Supported cards include workstation and data-centre models.

Open-IOV is a community project using stock NVIDIA guest drivers without a vGPU licence, with its own host module bound to the GPU.

### API forwarding

<!-- src: https://github.com/reindertpelsma/nvkvm-pv README; https://github.com/nestrilabs/virtio-nvgpu/blob/dev/README.md; docs/PROJECT_HISTORY.md section 3 -->
API forwarding leaves the card with the host. Examples include nvkvm-pv and virtio-nvgpu. They use custom guest drivers and support Linux guests. nvkvm-pv needs matching host and guest NVIDIA driver versions.

### CUDA container

<!-- src: https://github.com/NVIDIA/nvidia-container-toolkit/blob/main/README.md; https://docs.docker.com/engine/security/ (namespaces); https://gvisor.dev/docs/user_guide/gpu/ (nvproxy Security); docs/howto/requirements.md Guest -->
A CUDA container shares the host's NVIDIA driver and Linux kernel. Namespaces separate its processes; it has no VM boundary or separate guest kernel. Kayfabe adds a VM with its own operating system, including Windows.

[gVisor with nvproxy](https://gvisor.dev/docs/user_guide/gpu/) is the sandboxed container variant; its guide says the GPU still relies on NVIDIA's driver for hardware isolation.

### virtio-gpu

<!-- src: https://www.qemu.org/docs/master/system/devices/virtio/virtio-gpu.html (virglrenderer and Venus); docs/FAQ.md 'Why not virtio-gpu (Venus, virgl or a native context)?' -->
Venus and virgl provide a generic graphics path. The guest does not run NVIDIA's own driver. No speed or feature comparison with kayfabe is recorded.

### kayfabe

<!-- src: README.md comparison table; docs/howto/requirements.md; docs/howto/goals.md 'Hypervisor independence'; docs/FAQ.md 'Does the guest driver have to match the host driver?' -->
The VM uses NVIDIA's own driver. The host keeps the card, with no vGPU licence server or driver patches. Kayfabe adds no host module of its own. It needs a QEMU build that includes its device. Host and guest drivers need not match: Windows 580.88 has run against host 595.91.07.

<!-- src: docs/howto/security.md; branch claude/rootless-audit-20261016 commit 32d7319b2, traces/rootless_audit_20261016/REPORT.txt -->
The launch scripts still use root; a run without sudo is recorded on one host.

## How it is isolated

<!-- src: docs/howto/security.md 'What the isolation is made of' and 'Property P'; README.md STATUS line -->
The VM reaches the GPU through the host's NVIDIA driver, as an ordinary program does. Kayfabe checks what the VM asks to map and run. The [security model](/@docs/security.html) describes how the driver and GPU keep clients apart. It has not been audited.

<!-- src: site/content/works-today.md 'Not measured yet'; docs/FAQ.md comparison and multi-VM answers -->
Speed and security comparisons with other approaches are [not measured yet](/works-today/#not-measured-yet).

## What you need

<!-- src: docs/howto/requirements.md; docs/howto/goals.md Hypervisor independence -->
A Linux x86-64 host with KVM, QEMU and an NVIDIA card from Turing on. The recorded runs use NVIDIA's open host driver. Only QEMU is supported today. The [requirements](/@docs/requirements.html) list the cards and drivers that have been tried.

## Licences and source

<!-- src: docs/howto/credits.md 'Licence', 'No vendor binaries', 'Your licences', 'Third-party notices' -->
Kayfabe is dual-licensed Apache-2.0 OR GPL-2.0-or-later. It ships no NVIDIA or Microsoft files. NVIDIA's licence applies to the driver you install on the host and in the VM. Windows must be licensed for the way you use it. If you host VMs for others, you are responsible for those licences. The [credits and notices](/@docs/credits.html) have the full terms.
