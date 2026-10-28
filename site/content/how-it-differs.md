+++
title = "How it differs"
description = "The choice is what you share, which driver the guest runs, and what you give up."
template = "detail.html"
[extra]
src = "README.md comparison; docs/FAQ.md; individual sources below"
+++

## Six approaches

### VFIO passthrough

<!-- src: README.md comparison table (VFIO: no, one VM owns it) -->
VFIO gives the whole card to one VM. While it runs, the host cannot use that card. Kayfabe leaves it with the host.

### NVIDIA vGPU

<!-- src: README.md comparison table (vGPU row) -->
NVIDIA vGPU shares a card between VMs. It needs a data-centre GPU and a licence. The host does not use the card in this setup.

### API forwarding

<!-- src: docs/FAQ.md 'How is kayfabe different from nvkvm-pv?'; docs/PROJECT_HISTORY.md section 3; docs/howto/credits.md Acknowledgements -->
nvkvm-pv and virtio-nvgpu leave the card with the host. The VM needs a custom kernel module, so this route is Linux only. The host and guest driver versions must match. nvkvm-pv is the author's earlier project. If you want something that works today, use it.

### gVisor with nvproxy

<!-- src: https://gvisor.dev/docs/user_guide/gpu/ (quoted 2026-10-27); docs/howto/security.md -->
gVisor puts a container in front of the driver, not a VM. Its GPU guide says nvproxy adds no hardware-level isolation beyond what NVIDIA's kernel driver provides. Kayfabe also reaches the host driver like an ordinary program, with a VM in front of it.

### virtio-gpu

<!-- src: docs/FAQ.md 'Why not virtio-gpu (Venus, virgl or a native context)?' -->
Venus and virgl provide a generic graphics path. The guest does not run NVIDIA's own driver. The repository has not compared them with kayfabe in detail.

### kayfabe

<!-- src: README.md comparison table; docs/howto/requirements.md; docs/howto/goals.md 'Hypervisor independence'; docs/FAQ.md 'Does the guest driver have to match the host driver?' -->
The VM uses NVIDIA's own driver. The host keeps the card, with no vGPU licence server and no driver patches. Host and guest drivers need not match: Windows 580.88 has run against host 595.91.07.

The [FAQ](/@docs/FAQ.html) goes deeper on VFIO, API forwarding and guest drivers. For the container route, see [gVisor's GPU guide](https://gvisor.dev/docs/user_guide/gpu/).

## How it is isolated

<!-- src: docs/howto/security.md 'What the isolation is made of' and 'Property P' (docs/design/V3_SECURITY_MODEL.md R6, a comparison and not a safety claim); README.md STATUS line (not audited) -->
The VM reaches the GPU through the host's NVIDIA driver, as an ordinary program does. The driver and GPU keep clients apart. Kayfabe checks what the VM asks to map and run. That is the design, not an audit result.

Read the [security model](/@docs/security.html#what-the-isolation-is-made-of) and [isolation FAQ](/@docs/FAQ.html#how-is-the-vm-kept-apart-from-my-other-gpu-programs) before deciding what to trust.

## What you need

<!-- src: docs/howto/requirements.md; docs/howto/goals.md Hypervisor independence -->
A Linux x86-64 host with KVM, QEMU and an NVIDIA card from Turing on. The recorded runs use NVIDIA's open kernel module. Other hypervisors are not supported yet. [Requirements](/@docs/requirements.html) lists the cards and drivers that have been tried.

## Licences and source

<!-- src: docs/howto/credits.md 'No vendor binaries' and 'Your licences' -->
Kayfabe ships no NVIDIA or Microsoft files. NVIDIA's licence applies to the driver you install on the host and in the VM. If you host VMs for others, you are responsible for those licences. See [Your licences](/@docs/credits.html#your-licences).

<!-- src: docs/howto/credits.md 'Licence', 'Third-party notices', 'Your licences' -->
Kayfabe is dual-licensed Apache-2.0 OR GPL-2.0-or-later. Struct layouts, class ids and register offsets are generated from NVIDIA's open GPU kernel modules, under MIT. Windows must be licensed for the way you use it. See [Credits and notices](/@docs/credits.html) for the full terms.
