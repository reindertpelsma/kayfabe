# kayfabe FAQ

**STATUS: LIVE, 2026-10-28 (rootless, containers, display broker, two-VM, anti-cheat and isolation answers revised; the IOMMU and interrupt answers are from 2026-10-08).** Questions the VM community asks first, answered from the design and the
evidence in this repository. A statement carries its evidence or says it is inferred. Where the repository has no
verified answer, the answer says **not documented yet**. Plans and goals are on [Goals and plans](goals.md).
<!-- src: README.md and docs/howto/*.md; each answer below names its sources -->

## The basics

### Is this vGPU or SR-IOV?

No. Kayfabe is a QEMU device that shows the guest an emulated NVIDIA GPU, and the guest's stock NVIDIA driver talks to it. (QEMU is the only hypervisor supported today; independence from one hypervisor is a goal, see [Goals and plans](goals.md) and *Does it only work with QEMU?* below.) There is no hardware partitioning and no vendor vGPU driver or licence server. In the README's comparison, NVIDIA vGPU needs a data-centre GPU and a licence, while kayfabe's hardware results so far are on consumer cards, Turing to Blackwell. Using NVIDIA's driver is still subject to NVIDIA's own licence: see [Credits and notices](credits.md).
<!-- src: README.md "Why it is interesting"; docs/howto/technical-faq.md; docs/howto/credits.md "Your licences" -->

### Does the host keep the GPU?

Yes, that is the point of the design. The host driver stays loaded on the card and the host keeps using it; the guest does not own the device. The README's screenshot shows a Windows guest and a Linux desktop using one GPU at the same time. One caveat for today: the repository's provisioning scripts for test machines stop the host's display manager and purge NVIDIA packages, so they are for throwaway machines, not for your desktop. See [Running the QEMU device](installing.md).
<!-- src: README.md (headline, comparison table, screenshot caption); docs/howto/installing.md warning -->

### Why not VFIO passthrough?

With VFIO one VM owns the whole GPU and the host does not keep it (README comparison). Kayfabe lets the host and the guest use the GPU together. What you give up is hardware-enforced isolation by an IOMMU: kayfabe's boundary is the GPU's own memory management unit plus kayfabe's checks, and it has not been audited (see the IOMMU answer below and *Is it safe to run untrusted guests?*). VFIO is also the native reference the README measures kayfabe against. Whether VFIO is safe for untrusted guests is itself contested: Microsoft's Discrete Device Assignment documentation says "You should only use Discrete Device Assignment where the tenants of the VMs are trusted", and Xen's `SUPPORT.md` says it is "generally not safe to use this feature to expose a physical device to completely untrusted guests".
<!-- src: README.md comparison table; docs/PRODUCT_POSITIONING.md section 2 (verbatim Microsoft DDA and Xen SUPPORT.md quotes); this file, IOMMU answer; docs/howto/security.md -->

### Why not virtio-gpu (Venus, virgl or a native context)?

The repository does not compare them in detail: **not documented yet**. What it does say is that kayfabe is aimed at running NVIDIA's own, unmodified driver stack in a guest, including Windows, and the project's positioning note of 2026-10-01 names generic multi-vendor Linux-guest graphics (the virtio-gpu native context case) as not what kayfabe is for. Because the guest runs the stock NVIDIA driver, `nvidia-smi` and CUDA are those of the stock driver [measured, README, 2026-10-21].
<!-- src: docs/PRODUCT_POSITIONING.md (status update 2026-10-01); README.md "What runs today" -->

### How is kayfabe different from nvkvm-pv?

The project's history page (written 2026-09-06) describes it this way: nvkvm-pv forwards the guest's driver calls to the host and needs a custom kernel module in the guest, so the guest is Linux and the host and guest driver versions must match. Kayfabe emulates the device instead, so the guest runs NVIDIA's stock kernel driver, the guest driver version is decoupled from the host's by design, and Windows guests are possible. The same page says that if you want GPU forwarding that works today, use nvkvm-pv; kayfabe is a research project.
<!-- src: docs/PROJECT_HISTORY.md section 3; README.md status line -->

### Is there an installer or a download?

No. There is no installer, no stable interface and no public binary; you build it from source. A QEMU binary built from the tree is not yet distributable under the GPL. See [Building from source](building.md) and [Credits and notices](credits.md).
<!-- src: README.md "Trying it" and status line; docs/howto/credits.md (LICENSE exception 2) -->

## Hardware and drivers

### Which GPUs work?

Turing and newer is the hard line. Below Turing there is no GSP firmware interface for kayfabe to stand in for, so older cards cannot work. Of the newer families, the repository records runs on Turing (TU116), Ampere (GA106, GA104), Ada (AD104, AD106, AD103, AD102) and Blackwell (GB205), mostly with Linux guests. Windows guests have run on fewer dies, and one Blackwell die (GB206) fails under Windows today. Hopper and datacentre Blackwell are derived from the open driver's source only and were never run on hardware. Consumer GPUs are the tested class; "any NVIDIA GPU" is the goal, not a claim. The per-die table is on [Requirements](requirements.md).
<!-- src: docs/howto/requirements.md "GPU"; README.md "GPUs:" paragraph; docs/STATUS_DETAIL.md section 0 -->

### Which host driver versions work?

Every recorded run used the open NVIDIA kernel module. Host drivers that passed the repository's checks are 570.148.08, 575.57.08, 580.65.06, 580.95.05, 580.159.04 and 595.91.07. Hosts older than 570 are untested, and the closed host module is supported in principle but untested. The exact checks and revisions are on [Requirements](requirements.md).
<!-- src: docs/howto/requirements.md "NVIDIA driver on the host" -->

### Does the guest driver have to match the host driver?

No, by design. The host driver version is read from the host and decides how kayfabe talks to the host driver, and the guest's declared version only decides how kayfabe decodes the guest's requests. In the recorded runs, guest drivers 575.57.08, 580.105.08, 580.159.04, 590.48.01, 595.84 and 610.57.04 passed the CUDA ladder against a 580.159.04 host [measured, September 2026, one GA102 box]. A guest version outside the measured set is refused by name. Two limits: the 30-arm suite is graded for 580.x guests only, and the display layouts were derived for Linux guest 580.159.04 only. Windows has been run with NVIDIA's 580.88 driver.
<!-- src: docs/howto/technical-faq.md "authored, never forwarded"; docs/howto/requirements.md "Guest"; docs/howto/display.md -->

### Does the guest need a special driver or kernel module?

No. The guest uses NVIDIA's own installer and driver, unmodified; nothing is patched on the host or in the guest. A Linux guest needs nothing set. A Windows 11 guest needs three registry settings, listed on [Windows guest](windows-guest.md); without the first one the driver stops with Code 43.
<!-- src: README.md "Nothing is patched"; docs/howto/windows-guest.md; docs/WINDOWS_GUEST_SETUP.md -->

### Which guest operating systems and Linux kernels work?

Windows 11 Pro (build 26200) and Windows 11 Enterprise LTSC 2024 (build 26100), both with the 580.88 driver, and Linux guests with the stock driver. For Linux the recorded guest is Ubuntu 24.04 with kernel 6.8; other distributions and kernels are **not documented yet**. Windows sleep/wake does not survive.
<!-- src: docs/howto/windows-guest.md; docs/howto/linux-guest.md "Not documented yet"; README.md "Not finished" -->

### Do I need root, or a patched host?

No host patches are required: the stock path uses the host's stock driver. Optional enhancements (a helper module in the guest, patched guest or host drivers) are planned, never required, and none is built into the stock path today; an optional research patch to the host's `nvidia-uvm` exists, and the four of the 65 recorded CUDA applications that need unified-memory demand paging fail without it.

Root is not required by the design, and one run without root is recorded, but the scripts in the repository still start as root. **Recorded:** an audit on one host ran kayfabe as a throwaway user whose only group was `kvm` (no capabilities, no sudo). The 30-arm Linux suite passed 30 of 30 and the GPU gates 9 of 9, and a Windows 11 guest ran Unigine Heaven (Direct3D 11) at 92.55 fps on average against 93.09 as root, with no VFIO rebind and no kernel module, driver or udev change [measured, kayfabe `0a746117`, RTX 4070, host driver 595.91.07, one host]. The record is on the branch `claude/rootless-audit-20261016` (commit `32d7319b2`, [report](https://github.com/reindertpelsma/kayfabe/blob/claude/rootless-audit-20261016/traces/rootless_audit_20261016/REPORT.txt)), not on master. **Not recorded:** the heavy lanes (the CUDA application matrix and similar) as that user, a desktop on the NVIDIA card itself, and a repeatable rootless test on master, which is still to be added. See [Security model](security.md).
<!-- src: branch claude/rootless-audit-20261016, traces/rootless_audit_20261016/REPORT.txt; docs/howto/requirements.md "Privileges"; docs/design/V3_COOPERATIVE_TIERS.md section 1 and docs/design/V3_HOST_PATCH_LIST.md section 1 (optional tiers, stock tier needs no patch) -->

## Sharing and performance

### How are resources divided between VMs?

**Not documented yet** as a feature: no per-VM GPU time quota is described. What is described: each kayfabe device has its own video-memory allocation on the host card, sized by the `fb-mb` property, and the security review of 2026-10-03 lists NVENC sessions, runlist time, host BAR1 per VM and GPU time as not bounded per VM. Its answer for the shared-GPU case is one guest per GPU.
<!-- src: docs/howto/requirements.md "Memory and apertures"; docs/howto/security.md "What a tenant can still do to others" -->

### Can several VMs share one GPU?

Only boot-level co-residence is recorded, and no sharing performance. In one run a Linux guest and a Windows guest ran together on one RTX 4070 for 3 min 20 s with no new host Xid; the Windows device had to be given a 1024 MiB video-memory store to fit beside the Linux one, and that run was a boot and display check only [measured, kayfabe `a88764b3`, 2026-10-08]. A design audit also lists several VMs on one GPU as "works as-is (measured, 2 VMs)"; that line is not a workload result. Not measured: throughput, fairness, what a guest-triggered GPU fault does to the other VMs, or host BAR1 accounting across VMs. A repeatable two-VM test is being added; until it exists, do not rely on sharing, and the review's advice is one guest per GPU.
<!-- src: traces/windows_code43_walls_20261007/README.md "Run54"; docs/design/V3_MULTI_GPU_AUDIT.md section 5; docs/STATUS_DETAIL.md section 6; README.md "Not finished"; docs/design/V3_FEATURE_GAPS.md H21 -->

### Can one guest use several GPUs?

One kayfabe device is created per host GPU, and a Linux thin-guest test with two distinct host GPUs passed, one at a time and concurrently. CUDA inside a two-GPU guest, unified memory across two GPUs and peer-to-peer access were not measured, and multi-GPU on Windows or with the display has not been run. See [Goals and plans](goals.md).
<!-- src: docs/howto/linux-guest.md "More than one GPU"; docs/STATUS_DETAIL.md section 3; docs/design/V3_FEATURE_GAPS.md H20, L10 -->

### What is the performance cost?

It depends on the workload. Large batches run close to host speed; many small launches pay for each one, because every doorbell write from the guest is a trapped write (a VM exit), and that cost is much higher on the nested rented hosts most results come from. Examples from the README, each with its limits: a matrix multiply in a Linux guest reached 0.997 of host speed on a rented, nested RTX 3060 box (older revisions), while eager PyTorch decoding ran at 0.29 of host speed on the same box. Windows speed is being benchmarked again on current master; results will be posted in the README.
<!-- src: README.md "Windows speed is being benchmarked again"; docs/howto/technical-faq.md "Why is performance uneven?" -->

### Does CUDA give the same results as the host?

For the 61 of 65 CUDA applications that pass, the recorded outputs equal bare metal [measured, an earlier revision, a rented RTX 3060], and NVENC and NVDEC output was byte-identical to bare metal in the graded lane [measured, RTX 3060]. The 4 failures use unified-memory demand paging and fail loudly. Nothing is recorded for a guest with several GPUs.
<!-- src: docs/howto/linux-guest.md "What has been run"; README.md "Linux guest" -->

### Do games with anti-cheat work?

**Not documented yet.** The recorded games and benchmarks are Minecraft (Java and Bedrock), Unigine Heaven and 3DMark; no anti-cheat protected title is recorded. Whether a given anti-cheat accepts a virtual machine is that anti-cheat's decision, and the repository does not test it. The project's own research notes that products such as Vanguard and FACEIT detect and refuse virtual machines. **Anti-cheat is a non-goal:** when an anti-cheat fights VMs, kayfabe will not be changed to work around it.
<!-- src: README.md "What runs today"; docs/design/V3_VIOMMU_NEED_RESEARCH_20261013.md summary item 3; owner direction 2026-10-28 (anti-cheat is a non-goal) -->

## Security and trust

### Is it safe to run untrusted guests?

Not yet; treat it as research. The design treats the guest as hostile and builds every host action itself from a few unprivileged verbs, but nothing has been independently audited and there is no penetration test. What a guest-triggered GPU fault does to other VMs sharing the card is not measured. Read [Security model](security.md) before deciding: it lists what is trusted, what is accepted and what is unfinished, and how to report a suspected vulnerability privately.
<!-- src: README.md status line and "Not finished"; docs/howto/security.md -->

### How is the VM kept apart from my other GPU programs?

The VM reaches the GPU through the host's NVIDIA driver as an ordinary client, and the driver and the GPU keep clients apart, as for any two programs on one card. Precisely: kayfabe opens the same device files a CUDA program does (plus `/dev/kvm`) and makes only calls that NVIDIA's open headers mark non-privileged, each built by kayfabe and never replayed from guest bytes. The guest does use real GPU memory, but it never supplies or sees a physical GPU address: its video memory is one allocation of host VRAM and its system memory is the hypervisor's shared memory file, its GPU page tables hold offsets into those two, and kayfabe resolves them itself and maps them at the virtual addresses the tables name. The guest's applications run on unprivileged host channels, in an address space holding only that VM's mappings. Deciding what goes into that address space is kayfabe's own job, and a bug there matters more than it would under VFIO. All of this is design and code reading plus one hardware check (the copy engine refuses a physical operand on a user channel); none of it is audited. Details and sources: [Security model](security.md).
<!-- src: docs/howto/security.md "What the isolation is made of"; docs/design/V3_SECURITY_MODEL.md sections 3, 4, R3, R5, R6; docs/OWNER_RULINGS.md section Q; traces/phys_operand_oracle_20261008/README.md -->

### Does kayfabe need an IOMMU? Is VM isolation weaker than with VFIO?

**Short answer:** kayfabe's isolation does not depend on an IOMMU, and that is not a weakness in the
way it would be for VFIO, because the guest never drives the physical GPU.

**Why VFIO needs one.** With VFIO the guest's driver programs the real device and can write any bus
address into a descriptor or register. The device then DMAs wherever it is told. The IOMMU is the only
hardware that stops a malicious guest driver from pointing the GPU at arbitrary host memory, and with
one device per VM its domain is a per-VM domain.

**What kayfabe does instead.**
- The guest talks to an emulated device. A guest-chosen address is an input that kayfabe checks
  against that VM's own memory or store, then maps through its own tables. Host RM, the trusted
  driver on the host, builds the GPU page tables from the memory objects kayfabe registered. A guest
  value never reaches the device as a raw bus address.
- The VM's work runs on host channels that are unprivileged and live in a GPU virtual address space
  holding only that VM's mappings. The GPU's own memory management unit enforces the address space.
  Physical addressing needs a privileged channel: [measured, physical-operand oracle at 8c084ab7
  (RTX 4070, driver 595.91.07), 2026-10-08] on a USER-privilege channel the copy engine refuses a
  physical operand, source and destination, with a channel reset, while a virtual operand is
  delivered; the 3D, compute and video classes expose no physical-operand method in OGKM's class
  headers (a reading, not a measurement). Details: `traces/phys_operand_oracle_20261008/`.
- All VMs share one physical GPU, so they share one IOMMU domain for it. An IOMMU cannot tell VM A's
  DMA from VM B's here, so it could not provide VM-to-VM isolation even if we leaned on it. The
  isolation between VMs is the per-channel address space.

**What the host IOMMU still does.** The GPU's DMA on the host goes through the host IOMMU where the
host runs in translated mode (it does on the reference host, `DMA-FQ`). That limits the effect of a
wrong bus address, such as a bug in the driver or the hardware, to pages already mapped for that GPU.
It protects the host from any process using the GPU, kayfabe included, and it is configuration
dependent (many hosts run identity mode). We do not rely on it, and it is not part of the argument.

**Would a guest IOMMU (vIOMMU) help?** Not for security. The guest kernel works in CPU physical
addresses and converts to an IOVA only when it writes a PTE or sends a command to the device; kayfabe
converts it back at once. Translation happens at the two ends only and protects almost no code path;
the "device" it would guard against is kayfabe itself. It would be a compatibility feature for guests
that ask for DMA remapping. Windows 11 does not require one. Cost and design: `design/V3_VIOMMU.md`.

**Is the IOMMU a strong boundary against a malicious device even on bare metal?** Only against DMA to
pages the driver did not map. Drivers generally trust what the device writes into pages that are
mapped for it (rings, message queues, notifiers) and parse it, and GPU drivers are not designed
against a hostile device (inferred; Linux's own model treats internal PCI devices as trusted and has a
separate bounce-buffer path for untrusted external ones). That is why our own hostile party is the
guest and why kayfabe's input handling, bounds checks and authored host actions carry the weight.

**What the boundary is made of.** Hardware: the GPU's memory management unit and the channel
privilege. Software: kayfabe's checks and host RM. So a bug in kayfabe's address resolution matters
more than it would under VFIO, which is why that code gets hostile-input tests and review
(`docs/OWNER_RULINGS.md` §V and the review notes in `docs/STATUS_AND_HANDOFF.md`).

**Paravirtual projects in general.** Other paravirtual GPU projects (the owner reports virtio-nvgpu
says the same of itself; not re-checked here) and nvkvm-pv have no IOMMU-based isolation, and for them
it likewise does not decide the boundary: the guest does not own the device.

### Can one VM's GPU work wake another VM? Does a VM learn when other tenants use the GPU?

**Short answer:** yes, a little, on purpose — the same as for processes on one bare-metal GPU. Decided by
the owner on 2026-10-08 (`docs/OWNER_RULINGS.md` §X).

**What happens.** A "non-stall" interrupt (a completion signal a program asks for, for example after a
copy) is raised by the physical GPU and serviced by the host driver. The host driver's notification is
GPU-wide: it says "an engine of this kind finished something", not whose work it was, and the host driver
itself wakes every client registered on that engine. kayfabe passes the notification on to every VM whose
guest registered for that event (a non-stall event its own driver allocated, which kayfabe records), and
to no other VM. It never drops one: a lost completion interrupt leaves a program waiting for work that
already finished, which is a correctness bug.

**What a tenant can do to others (accepted).** By running GPU work that asks for interrupts, a tenant makes
the other VMs that registered for the same engine's events take extra interrupts. Each is spurious for them:
the waiting program checks its own completion value, finds it unchanged and waits again. This is a minor
denial of service, accepted for correctness, and it is the host driver's own behaviour between processes.
An optional pacing setting (`KF3_PT_NSI_MIN_INTERVAL_US`, off by default) can cap how often one guest
vector is raised; it delays an interrupt, it never drops one.

**What a guest learns (accepted).** A guest that registered for an engine's events can tell, from when its
interrupts arrive, that some tenant used that kind of engine and roughly how often. It does not learn which
tenant, what the work was, or any data. Any unprivileged process on the host can already observe this
[measured 2026-10-08, RTX 4070, `traces/rawclient_ce_interrupt_20261008/bare_tenant_noise_*`]: with another
VM on the GPU, an idle raw client saw these notifications in every observation window.

**What is not shared.** Memory, address spaces, channels and results stay per VM; see the IOMMU answer
above. Design: `design/the_three_channel_kinds.md` §1.2.

## Running it

### Can kayfabe run inside a container?

That is a goal, and it is expected to work with `/dev/kvm` and the NVIDIA device nodes passed into the container, but it is **not tested or documented for kayfabe yet**: no recorded kayfabe run is in a container, and there is no container image or guide. Kayfabe is linked into the QEMU process and has no sandbox of its own, so a container is one way to add the extra layer described in the [Security model](security.md).

What can be said from the repository: in the rootless audit, a Linux guest run opened only `/dev/kvm`, `/dev/nvidiactl`, `/dev/nvidia0` and `/dev/nvidia-uvm`, read `/proc/driver/nvidia` and the card's `/sys/bus/pci` entries, and needed `libcuda` loadable on the host; the display broker additionally uses `/dev/udmabuf` [measured, branch `claude/rootless-audit-20261016`]. The sister project nvkvm-pv documents the same idea for its CUDA container: `docker run --device /dev/kvm --gpus all` with no `--privileged` and no added capabilities, because `/dev/kvm` is designed for unprivileged use, and because a container boundary outside the VMM means a VMM breakout lands in the container instead of on the host. Those two reasons apply to kayfabe's single QEMU process as well. nvkvm-pv's other container advice (its guest image, its isolate processes, the `NVIDIA_DRIVER_CAPABILITIES` list it needs for graphics) is specific to nvkvm-pv and has not been checked for kayfabe. The only container advice written for kayfabe is for the display broker: [Display and the broker](display.md).
<!-- src: https://github.com/reindertpelsma/nvkvm-pv docs/faq.md "Can nvkvm itself run inside a container?" and "Is --device /dev/kvm as dangerous as mounting the Docker socket?"; traces/rootless_audit_20261016/REPORT.txt section 2 (R0 strace) on branch claude/rootless-audit-20261016; docs/howto/requirements.md (libcuda); docs/howto/security.md "Deployment"; docs/howto/display.md "Containers" -->

### Does it only work with QEMU?

Today, yes: kayfabe is a QEMU device (`kf3-gpu`), and **only QEMU is supported**. Independence from one hypervisor is a goal: the core is designed to be VMM-agnostic and only a thin shim knows the hypervisor, but no other hypervisor has a shim. See [Goals and plans](goals.md).
<!-- src: docs/OWNER_RULINGS.md section AO item 1; docs/howto/goals.md "Hypervisor independence" -->

### Why a display broker instead of QEMU's own window?

Kayfabe's QEMU is built without the GTK and SDL front ends on purpose, and the display broker (a small separate program) is the way to get a desktop window; VNC stays as a second path. The reasons the repository records:

- **No lock-in to one hypervisor.** The owner's direction of 2026-10-01 is that frames leave through a hypervisor-agnostic broker, not QEMU's SDL or GTK UI. In the broker relay, only about 230 lines in the QEMU device are QEMU-specific; the connection logic, the frame ring and the input policy are VMM-neutral Rust.
- **The VMM stays away from your display server.** The broker owns the window, the input and the clipboard, and QEMU holds no connection to your display server at all. That matters most when QEMU is confined; it is the extra layer the [Security model](security.md) recommends.
- **One display path to maintain.** Pointer grab, absolute or relative input, buttons, the wheel and re-sync are decided in one place (`kf-broker`), and only a thin shim names the VMM, so a second hypervisor adds a shim rather than a second display path.
- **The broker owns the cursor and the pacing.** It draws the guest's cursor, including true inverting cursors such as the Windows text caret, and kayfabe composes none; the design makes the broker the frame clock (not built yet), with kayfabe's own clock as the fallback after 50 ms without a tick. The display design notes record problems with QEMU's own front ends: SDL and GTK read a guest cursor with different conventions, GTK warps the host pointer under a broker grab, and GTK's window resizing re-modes the guest.
- **Frames are copied on the GPU into a frame kayfabe owns.** For a compositor on the same NVIDIA GPU the frame is exported as a dma-buf and never crosses to the CPU; the guest cannot change it after the copy. Compositors on another GPU get a copy in host RAM, with shared memory as the last fallback.

<!-- src: docs/OWNER_RULINGS.md section C item 6 (2026-10-01), section L (frames), section V "Display broker and input" (2026-10-08), sections AJ, AK, AL (cursor and pacing), section AO; docs/design/V3_DISPLAY.md section 8.0 (the piece table, kf3.c about 230 lines) and the cursor-format notes (SDL/GTK conventions, GTK pointer warp, ui_info resize); broker/README.md; docs/howto/security.md -->

### Do I still need licences for NVIDIA's driver and Windows?

Yes. The NVIDIA driver licence applies to the driver on the host and in the guest, including its limits on datacentre deployment and on hosting, and Windows must be licensed for the way you use it. Kayfabe ships no NVIDIA or Microsoft binaries. See [Credits and notices](credits.md).
<!-- src: docs/howto/credits.md "Your licences"; docs/NOTICES.md -->
