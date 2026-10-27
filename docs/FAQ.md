# kayfabe FAQ

**STATUS: LIVE, 2026-10-27 (general questions added; the IOMMU and interrupt answers are from 2026-10-08).** Questions the VM community asks first, answered from the design and the
evidence in this repository. A statement carries its evidence or says it is inferred. Where the repository has no
verified answer, the answer says **not documented yet**. Plans and goals are on [Goals and plans](goals.md).
<!-- src: README.md and docs/howto/*.md; each answer below names its sources -->

## The basics

### Is this vGPU or SR-IOV?

No. Kayfabe is a QEMU device that shows the guest an emulated NVIDIA GPU, and the guest's stock NVIDIA driver talks to it. There is no hardware partitioning and no vendor vGPU driver or licence server. In the README's comparison, NVIDIA vGPU needs a data-centre GPU and a licence, while kayfabe's hardware results so far are on consumer cards, Turing to Blackwell. Using NVIDIA's driver is still subject to NVIDIA's own licence: see [Credits and notices](credits.md).
<!-- src: README.md "Why it is interesting"; docs/howto/technical-faq.md; docs/howto/credits.md "Your licences" -->

### Does the host keep the GPU?

Yes, that is the point of the design. The host driver stays loaded on the card and the host keeps using it; the guest does not own the device. The README's screenshot shows a Windows guest and a Linux desktop using one GPU at the same time. One caveat for today: the repository's provisioning scripts for test machines stop the host's display manager and purge NVIDIA packages, so they are for throwaway machines, not for your desktop. See [Running the QEMU device](installing.md).
<!-- src: README.md (headline, comparison table, screenshot caption); docs/howto/installing.md warning -->

### Why not VFIO passthrough?

With VFIO one VM owns the whole GPU and the host does not keep it (README comparison). Kayfabe lets the host and the guest use the GPU together. What you give up is hardware-enforced isolation by an IOMMU: kayfabe's boundary is the GPU's own memory management unit plus kayfabe's checks, and it has not been audited (see the IOMMU answer below and *Is it safe to run untrusted guests?*). VFIO is also the native reference the README measures kayfabe against.
<!-- src: README.md comparison table and Unigine Heaven table; this file, IOMMU answer; docs/howto/security.md -->

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

No host patches are required: the stock path uses the host's stock driver. The design runs the host side as an unprivileged process using only controls the host driver marks non-privileged. The repository's launch scripts, however, run as root, and a recorded end-to-end run without root is **not documented yet**. An optional research patch to the host's `nvidia-uvm` exists; four of the 65 recorded CUDA applications need unified-memory demand paging and fail without it.
<!-- src: docs/howto/requirements.md "Privileges"; README.md; docs/STATUS_DETAIL.md section 0 -->

## Sharing and performance

### How are resources divided between VMs?

**Not documented yet** as a feature: no per-VM GPU time quota is described. What is described: each kayfabe device has its own video-memory allocation on the host card, sized by the `fb-mb` property, and the security review of 2026-10-03 lists NVENC sessions, runlist time, host BAR1 per VM and GPU time as not bounded per VM. Its answer for the shared-GPU case is one guest per GPU.
<!-- src: docs/howto/requirements.md "Memory and apertures"; docs/howto/security.md "What a tenant can still do to others" -->

### Can several VMs share one GPU?

That is the design, but it is not established as a verified configuration. The status record lists two VMs sharing one GPU as not measured, and the README says that what a guest-triggered GPU fault does to other VMs sharing the card is not yet measured. The gap list mentions a two-VM case without cross-process accounting of host BAR1. Do not rely on it yet.
<!-- src: docs/STATUS_DETAIL.md section 6; README.md "Not finished"; docs/design/V3_FEATURE_GAPS.md H21 -->

### Can one guest use several GPUs?

One kayfabe device is created per host GPU, and a Linux thin-guest test with two distinct host GPUs passed, one at a time and concurrently. CUDA inside a two-GPU guest, unified memory across two GPUs and peer-to-peer access were not measured, and multi-GPU on Windows or with the display has not been run. See [Goals and plans](goals.md).
<!-- src: docs/howto/linux-guest.md "More than one GPU"; docs/STATUS_DETAIL.md section 3; docs/design/V3_FEATURE_GAPS.md H20, L10 -->

### What is the performance cost?

It depends on the workload. Large batches run close to host speed; many small launches pay for each one, because every doorbell write from the guest is a trapped write (a VM exit), and that cost is much higher on the nested rented hosts most results come from. Examples from the README, each with its limits: a matrix multiply in a Linux guest reached 0.997 of host speed on a rented, nested RTX 3060 box (older revisions), while eager PyTorch decoding ran at 0.29 of host speed on the same box. In a Windows guest on the author's RTX 4070, Unigine Heaven in Direct3D 11 ran at 89 to 102 fps on the production path against 330 fps natively (VFIO), and at 296 to 299 fps on an experimental build; those are Heaven's own scores at one window size and one quality setting.
<!-- src: README.md "Near-native speed is within reach"; docs/howto/technical-faq.md "Why is performance uneven?" -->

### Does CUDA give the same results as the host?

For the 61 of 65 CUDA applications that pass, the recorded outputs equal bare metal [measured, an earlier revision, a rented RTX 3060], and NVENC and NVDEC output was byte-identical to bare metal in the graded lane [measured, RTX 3060]. The 4 failures use unified-memory demand paging and fail loudly. Nothing is recorded for a guest with several GPUs.
<!-- src: docs/howto/linux-guest.md "What has been run"; README.md "Linux guest" -->

### Do games with anti-cheat work?

**Not documented yet.** The recorded games and benchmarks are Minecraft (Java and Bedrock), Unigine Heaven and 3DMark; no anti-cheat protected title is recorded. Whether a given anti-cheat accepts a virtual machine is that anti-cheat's decision, and the repository does not test it.
<!-- src: README.md "What runs today" -->

## Security and trust

### Is it safe to run untrusted guests?

Not yet; treat it as research. The design treats the guest as hostile and builds every host action itself from a few unprivileged verbs, but nothing has been independently audited and there is no penetration test. What a guest-triggered GPU fault does to other VMs sharing the card is not measured. Read [Security model](security.md) before deciding: it lists what is trusted, what is accepted and what is unfinished, and how to report a suspected vulnerability privately.
<!-- src: README.md status line and "Not finished"; docs/howto/security.md -->

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

**Not documented yet.** Kayfabe is linked into the QEMU process and has no sandbox of its own; confining that whole process (a container is one way) is the deployer's job. No recorded run puts it in a container, and the guide for container runtimes does not exist yet. The only container advice in the repository is for the display broker. See [Security model](security.md) and [Display and the broker](display.md).
<!-- src: docs/howto/security.md "Deployment"; docs/howto/display.md "Containers"; docs/howto/linux-guest.md "Not documented yet" -->

### Do I still need licences for NVIDIA's driver and Windows?

Yes. The NVIDIA driver licence applies to the driver on the host and in the guest, including its limits on datacentre deployment and on hosting, and Windows must be licensed for the way you use it. Kayfabe ships no NVIDIA or Microsoft binaries. See [Credits and notices](credits.md).
<!-- src: docs/howto/credits.md "Your licences"; docs/NOTICES.md -->
