# Technical FAQ

How kayfabe works and why it is built this way, at the level a user or deployer needs. The full design lives in the repository's design documents, which are not part of this manual. Questions about the IOMMU, isolation and interrupts are in the [FAQ](../FAQ.md). Each answer names its source. <!-- src: ARCHITECTURE.md, docs/FAQ.md -->

## What is kayfabe, mechanically?

A QEMU device (`kf3-gpu`) that appears to the guest as an NVIDIA GPU. The guest's stock driver talks to it the way it talks to a real card. Kayfabe plays the part of the card's **GSP**, the on-GPU firmware processor that current NVIDIA drivers send their resource-manager commands to. It answers those commands itself, and has the real host GPU do the work behind them. One host process holds all of it: QEMU, with the kayfabe core linked in. Only QEMU is supported today; the core is designed to be independent of the hypervisor, and that independence is a goal (see [Goals and plans](goals.md)). <!-- src: ARCHITECTURE.md "The shape" -->

## What does the guest actually execute on?

Three kinds of guest channel, three treatments: <!-- src: ARCHITECTURE.md "Channels - three kinds" -->

| Kind | Who | What kayfabe does | Who completes it |
|---|---|---|---|
| **Passthrough** | guest user channels (CUDA, Vulkan, NVENC/NVDEC) | creates a host "twin" channel in a matching host channel group; the guest's command buffer runs on the real engine, unparsed | the GPU writes the completion; kayfabe does nothing |
| **Translated** | guest kernel channels (the driver's scrubber, UVM's copy engine) | rewrites the physically addressed copy-engine work onto kayfabe's own window and rings a host ring; the real engine executes it | the GPU writes the semaphore that was forwarded |
| **Emulated** | channels with no GPU work behind them (for example the virtual display) | executes in the VMM | the VMM, because no GPU work was involved |

There is **no CPU executor for GPU work.** Completions of work that reached the GPU are never forged. <!-- src: ARCHITECTURE.md, AGENTS.md -->

## Why only the GSP path? Which GPUs can that cover?

Turing and newer. Below Turing there is no GSP at all: the guest driver would run its resource manager on the CPU and drive the hardware directly, and there would be no firmware interface to impersonate. On Windows the stock GeForce driver runs its resource manager on the CPU by default [measured: Code 43 without the setting], which is why the Windows guest needs the `EnableGpuFirmware` setting. <!-- src: docs/design/THE_ARCHITECTURE_v3.md section 0.0.1, docs/WINDOWS_GUEST_SETUP.md -->

## How does the guest's memory get to the host GPU?

- **One store.** The guest's video memory is one allocation of host VRAM, sized by `fb-mb`. A guest framebuffer offset is an offset into that allocation. Guest system memory is the hypervisor's shared memfd. <!-- src: ARCHITECTURE.md "Memory and addresses" -->
- **Mirror, never adopt.** The host twin's address space is filled with kayfabe's own host mappings. Kayfabe never points the host at the guest's page tables. <!-- src: ARCHITECTURE.md -->
- **The diff.** A small GPU program (the "walk kernel", committed as PTX) reads the guest's live page tables in place in video memory and emits only the difference from what the host has confirmed. The host applies it as batched maps and unmaps with one TLB invalidate, and acknowledges run by run. A refused map stays a difference and is retried. <!-- src: ARCHITECTURE.md -->
- **The guest's TLB invalidate completes only after the host mapping is committed** (gate 8). <!-- src: ARCHITECTURE.md "Verification" -->

## Why does only BAR0 trap? Does the vCPU ever wait on the host?

Only writes to the device's register window (BAR0) trap. BAR1 and BAR2 are never trapped. The vCPU never blocks on host work: privileged writes go into one ordered ring drained by one thread, and a doorbell write sets a bit in a table that worker threads scan. Nothing blocks on a vCPU or under a lock that a vCPU takes. <!-- src: ARCHITECTURE.md, AGENTS.md -->

## What does "authored, never forwarded" mean?

Every call kayfabe makes to the host driver is built by kayfabe from named values; none takes a guest-supplied flag word. The host driver's version is measured on the host and decides the layout of those calls; the guest's version, which the guest declares, only decides how kayfabe decodes the guest's requests. A false declaration can at most make that guest's own requests fail. <!-- src: ARCHITECTURE.md, docs/design/V3_WINDOWS_SUPPORT_PROFILE.md (the two version axes) -->

## Where do per-GPU facts come from?

"Derive, never capture." A GPU's facts are read from the host through unprivileged controls; register offsets and class sets are generated from NVIDIA's open driver headers; only a small family table is maintained by hand. A captured per-GPU table is the thing the design exists to avoid. <!-- src: AGENTS.md, ARCHITECTURE.md -->

The generated parts come from NVIDIA's open-gpu-kernel-modules (MIT) and open-gpu-doc; some capability allowlists derive from gVisor's `nvproxy`. See [Credits and notices](credits.md). <!-- src: THIRD_PARTY_NOTICES.md -->

## Do I need to patch the host or guest driver or kernel?

No. Nothing is patched. An optional research patch to the host's `nvidia-uvm` exists to prove unified-memory demand paging; it is not part of the stock path. Optional enhancements (a helper module in the guest, patched guest or host drivers) are planned, and the stock path will not require them. <!-- src: README.md, docs/STATUS_DETAIL.md section 0 -->

## How does this differ from the alternatives?

From the README's comparison: <!-- src: README.md "Why it is interesting" -->

| | host keeps the GPU | guest driver | needs |
|---|---|---|---|
| VFIO passthrough | no, one VM owns it | stock | any GPU |
| NVIDIA vGPU | the VMs share it, the host does not | the vendor's vGPU driver | a GPU on [NVIDIA's supported list](https://docs.nvidia.com/vgpu/gpus-supported-by-vgpu.html) (workstation and data-centre cards), vGPU software and a licence |
| API forwarding (nvkvm-pv, virtio-nvgpu) | yes | a custom guest module | any NVIDIA GPU |
| kayfabe | yes | stock, unmodified | goal: any NVIDIA GPU; tested on consumer cards, Turing to Blackwell; the open host driver |

## What does running as an ordinary process change?

To the host driver, kayfabe is a GPU client that also holds `/dev/kvm`, so no host kernel patch or custom host driver is needed. By design, a bug in kayfabe is meant to stay inside that one process, several VMs rely on the host driver's multi-process support, and a compromised kayfabe has the reach of an ordinary GPU-container tenant plus `/dev/kvm`. None of this is audited, and driver and KVM bugs still apply. The reasons, sources and limits are in the [FAQ](../FAQ.md#why-does-it-matter-that-kayfabe-is-an-ordinary-unprivileged-gpu-process). What has not been measured is on the site's [list](https://kayfabe.dev/works-today/#not-measured-yet). <!-- src: docs/howto/security.md "What the isolation is made of"; docs/FAQ.md -->

## What does the guest see?

The model and architecture stay truthful: the device takes its PCI identity and die facts from the host GPU. The GPU UUID is a per-VM value so that two VMs do not report the same one (`gpu-uuid=`; as of 2026-10-08 that was code and GPU-free tests only). <!-- src: docs/design/V3_GPU_UUID.md, qemu/hw/misc/kf3/kf3.c -->

## Why do the results come from rented, nested hosts?

Most hardware results were measured on rented hosts that are themselves KVM guests, so every kayfabe guest on them is nested; a VM exit costs about 10 to 40 times as much as on bare metal. Ratios are per box. <!-- src: AGENTS.md, docs/STATUS_DETAIL.md -->

## Why is performance uneven?

It depends on how often an application rings the doorbell. Every doorbell write from the guest is a trapped write, which is a VM exit. Applications that submit large batches run close to host speed, and applications that launch many small kernels pay for each launch. An optional guest doorbell module that would avoid the exit is designed but not built. Performance numbers, with their revisions and boxes, are in the README. <!-- src: docs/STATUS_DETAIL.md section 4, README.md -->

## Why is the CUDA walk kernel inside the VMM?

So that the guest's page tables are read where they live, in video memory, by the GPU, instead of being copied and interpreted by a CPU. Kayfabe's own GPU kernels therefore run in a CUDA context inside the VMM, which is why `libcuda` has to be present on the host, and why the security review calls the walker's bounds the only barrier on that path. <!-- src: ARCHITECTURE.md, docs/design/V3_SECURITY_MODEL.md section 8 -->

## Why a frozen `kayfabe-*` set of crates next to `kf-*`?

The `kf-*` crates are the product. The older `kayfabe-*` tree is frozen reference code plus the test client (`kayfabe-rm-ladder`) that the 30-arm suite and the bare-metal baseline use; nothing in `kf-*` depends on it. <!-- src: AGENTS.md "Layout", ARCHITECTURE.md "Crates" -->

## What do the "gates" and "30 arms" mean?

- **Gates** (`kf-gate1` to `kf-gate9`, plus a tenth probe) play the guest against the real planes on a real GPU with no QEMU: a real copy on kayfabe's own ring completing through a host event; the host address space matching the guest's tables in both page-table formats; translated kernel copies; doorbells driving workers; passthrough on copy and graphics engines; invalidate completing only after the host map is committed; and the walk kernel's diff protocol. <!-- src: ARCHITECTURE.md "Verification", scripts/bench/v3_gates.sh -->
- **The 30 arms** are 30 checks run by a raw resource-manager client, one guest boot per arm under a hard time budget, with bare metal as the baseline. <!-- src: scripts/fastguest/fast_suite.sh, ARCHITECTURE.md -->

## What is not decided yet?

Windows sleep/wake, host-side fault containment between VMs, and a public install path. See the status list on the [introduction](index.md) and in [Requirements](requirements.md).

Verified on: the descriptions come from the architecture page and the FAQ (master at 2026-10-21, kayfabe `89fc8a222`); the gates and suite are recorded for `afb552ea` (2026-09-30, RTX 3060). Not verified end to end: this page explains design and was not tested.
