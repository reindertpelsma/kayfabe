# Security model

> **Kayfabe has not been security audited.** The model below is a design, and the repository's own review of it (a source reading, 2026-10-03) listed gaps. Do not run untrusted or multi-tenant guests on a machine you cannot afford to lose. <!-- src: README.md, SECURITY.md, docs/design/V3_SECURITY_MODEL.md -->

## Reporting a vulnerability

Report it privately through GitHub's private vulnerability reporting: <https://github.com/reindertpelsma/kayfabe/security/advisories/new>. Do not open a public issue. Include the revision (the `KAYFABE_REV.txt` file or the git hash), what you did and what you saw. Reports are read by the maintainer. <!-- src: SECURITY.md -->

In scope: a guest-reachable unbounded read or emit, a guest-controlled host action, or any way for a guest to reach host memory or devices it was not given. <!-- src: SECURITY.md -->

## The threat model

- **The guest is hostile.** Guest userspace and guest root are both untrusted. The boundary inside the guest is also untrusted: unprivileged guest userspace can write any doorbell token, so no check at the trap can establish identity, and the design limits what such a write can reach. <!-- src: AGENTS.md "Rules", ARCHITECTURE.md "Rules that shape the code" -->
- **Host actions are authored, never forwarded.** Every call kayfabe makes on the host GPU is built by kayfabe from a few verbs, using the host driver's unprivileged interface. Nothing the guest sends is replayed to the host. Answers to the guest's driver come from facts read from the host GPU through unprivileged controls, from generated family rules, or from values kayfabe fabricates so the driver's own code accepts them. <!-- src: ARCHITECTURE.md "The shape" -->
- **The host process is meant to run unprivileged.** The host side uses only controls the host driver marks non-privileged and needs no host kernel module. **Recorded:** one audit ran kayfabe as a throwaway user whose only group was `kvm` (no capabilities, no sudo). The 30-arm Linux suite passed 30 of 30, the GPU gates 9 of 9, and a Windows 11 guest ran Unigine Heaven in Direct3D 11 at 92.55 fps on average against 93.09 as root (-0.6%, inside a 3.3% run-to-run spread), with no VFIO rebind and no kernel module, driver or udev change [measured, kayfabe `0a746117`, RTX 4070, host driver 595.91.07, Ubuntu 26.04, one host]. The record is on the branch `claude/rootless-audit-20261016` (commit `32d7319b2`, [report](https://github.com/reindertpelsma/kayfabe/blob/claude/rootless-audit-20261016/traces/rootless_audit_20261016/REPORT.txt)), not on master. Its qualifiers: it relied on NVIDIA's default mode 0666 for `/dev/nvidia*`; the host desktop ran on an AMD integrated GPU, so a desktop on the NVIDIA card was not tested; a real window needs the desktop user itself; the CUDA application matrix and the other heavy lanes were not run as that user. **Not so yet:** the launch scripts in the repository still start as root, and a repeatable rootless test on master is still to be added. <!-- src: branch claude/rootless-audit-20261016, traces/rootless_audit_20261016/REPORT.txt (verdict paragraph, claim table, run table); scripts/bench/display/interactive.sh -->
- **No CPU executes GPU work**, and no completion is forged for work that reached the GPU. Only emulated channels, which have no GPU work behind them, complete inside the VMM. <!-- src: AGENTS.md, ARCHITECTURE.md -->
- **Only writes to the device's register window trap.** The vCPU never blocks on host work. <!-- src: ARCHITECTURE.md -->
- **No unsafe code** outside three crates and only in files named `*_unsafe.rs`. <!-- src: AGENTS.md -->

### What the isolation is made of

This is the precise version of the home page's "How it is isolated". In short: the VM reaches your GPU through the host's NVIDIA driver, the way an ordinary program does, and the driver and the GPU keep that VM and your other programs apart. The details, each with its source:

- **Kayfabe is an ordinary client of the host driver.** It opens the device files a CUDA program opens (`/dev/nvidiactl`, `/dev/nvidia0`, `/dev/nvidia-uvm` [measured: strace of a Linux guest run, rootless audit]) and, because it is a QEMU device, `/dev/kvm`. Every call it makes to the driver is built by kayfabe from a closed list, never replayed from guest bytes, and in NVIDIA's open headers each is marked non-privileged [code reading; how the closed GPU firmware treats a few of them is not verified]. The design document states the property as: every effect a guest can produce on the host GPU is one a local unprivileged process could produce. That is a comparison, not a safety claim. <!-- src: docs/design/V3_SECURITY_MODEL.md section 4 and R6 (Property P); traces/rootless_audit_20261016/REPORT.txt section 2 (strace) on branch claude/rootless-audit-20261016 -->
- **The guest never supplies or sees a physical GPU address, and it does use real GPU memory.** Its video memory is a real allocation of host VRAM (the store, sized by `fb-mb`), and its system memory is the hypervisor's shared memory file. The addresses in the guest's GPU page tables are offsets into those two. Kayfabe reads the tables, resolves each offset itself, refuses anything outside the store or guest RAM, and asks the host driver to map its own allocations at the GPU virtual addresses the tables name. So the guest chooses a virtual layout; it does not choose what that layout can reach. <!-- src: docs/OWNER_RULINGS.md section Q (address model); ARCHITECTURE.md "Memory and addresses"; docs/design/V3_SECURITY_MODEL.md section 3 link 3 (kf-mem apply_entry) and section 4 (guest-derived inputs are named scalars only) -->
- **Guest application work runs on unprivileged host channels, in an address space that holds only that VM's mappings.** The command buffers of the guest's applications run unparsed on the real engines. The host address space for such a channel holds only rows derived from the guest's own page tables: no window, no kayfabe memory. Physical addressing needs a privileged channel: on a user-privilege channel the copy engine refused a physical operand with a channel reset, while a virtual operand was delivered [measured, kayfabe `8c084ab7`, RTX 4070, driver 595.91.07, 2026-10-08]. For the 3D, compute and video engines this rests on a reading of NVIDIA's open class headers, which expose no physical-operand method; it is not measured. <!-- src: docs/OWNER_RULINGS.md sections Q and AB; docs/design/V3_P1P2_TSPACE.md status; traces/phys_operand_oracle_20261008/README.md -->
- **Who separates what.** Between this VM and your other programs or VMs on the card, the separation is the host driver's and the GPU's: each client has its own address space, and channel privilege decides what a channel may do, as for any two programs on one card. Kayfabe's own responsibility is deciding what goes into the VM's address space. A bug in that address resolution therefore matters more than it would under VFIO, and it is the code that gets hostile-input tests and review. Kayfabe's own GPU programs (the page-table walker and the display copy) run in a CUDA context inside the VMM process, and their bounds checks are a host-memory-safety obligation. <!-- src: docs/design/V3_SECURITY_MODEL.md R3.5, R5.1, R6; docs/FAQ.md IOMMU answer -->
- **What is not claimed.** No audit, and no statement that a guest cannot reach a bug in NVIDIA's own driver: the host driver is trusted. What a guest-triggered GPU fault does to other VMs on the card is not measured. <!-- src: docs/design/V3_SECURITY_MODEL.md sections 9, 12 -->

Details, including why an IOMMU is not part of the argument, are in the [FAQ](../FAQ.md). <!-- src: docs/FAQ.md -->

## Deployment: one process, and an extra layer you add

Kayfabe is linked into the VMM. There are no helper processes, no IPC and no sandbox of its own. <!-- src: ARCHITECTURE.md "One process", docs/design/V3_SECURITY_MODEL.md section 0 -->

**Confining that process is defence in depth, and it is recommended.** The GPU-level separation described above does not rest on your confinement. When kayfabe runs as an ordinary user, it is whatever the host driver gives any unprivileged client. When it runs as root with `CAP_SYS_ADMIN`, as the launch scripts do today, it holds only because kayfabe never asks for a privileged operation (reviewed in code, not enforced), so the unprivileged way is the better one. On top of that, run kayfabe as an ordinary user and, if you can, in a container or sandbox (`systemd-nspawn`, `bubblewrap`, or Docker with `/dev/kvm` and the NVIDIA device nodes passed in; see [FAQ](../FAQ.md)). By design, a compromised kayfabe then has the reach of an ordinary GPU-container tenant on the host driver, plus access to `/dev/kvm`. A guest first has to break kayfabe to get there, and the path through kayfabe's own authored calls is a closed list of calls an unprivileged process may issue. NVIDIA driver bugs and KVM bugs still apply, and none of this is audited. What an attacker can do after taking over the VMM is outside the security model's scope. The [FAQ](../FAQ.md) explains why this matters. <!-- src: docs/design/V3_SECURITY_MODEL.md R3.4, R3 "Out of scope", section 11; owner direction 2026-10-28 (defence in depth) -->

The deployment assumptions the repository's security review lists (as recommendations; it was written to be stated in the support notes) are: <!-- src: docs/design/V3_SECURITY_MODEL.md section 11 -->

- a confined VMM (recommended as an extra layer);
- one uid per VM;
- `CAP_SYS_ADMIN` dropped by the launcher (recommended, not enforced);
- host scrub-on-free left enabled;
- profiling restricted to admin users (the host driver's default);
- no `--privileged` container;
- a PCI topology where the guest cannot unplug the device, until that issue is closed.

This page adds one recommendation of its own: enable the host IOMMU when the host runs untrusted guests. It is not a precondition. It limits a wrong DMA address from a buggy or compromised device to pages already mapped for it, and the argument above does not rely on it (see the [FAQ](../FAQ.md) IOMMU answer). <!-- src: docs/FAQ.md IOMMU answer, "What the host IOMMU still does" -->

The display broker adds a separate small program so a sandboxed VMM need not touch your display server; see [Display and the broker](display.md). Its checks include a peer-credential check on the socket and a grab that ends on focus loss. <!-- src: broker/README.md -->

Harness secrets: the Windows harness generates its password, ssh key and TPM state into a private directory (mode 0700/0600). The TPM state is the secret; never copy or commit it. <!-- src: scripts/bench/windows/win_vm.sh header -->

## What a tenant can still do to others (accepted)

- **Completion interrupts are GPU-wide.** A tenant that runs GPU work asking for interrupts makes other VMs that registered for the same engine's events take extra, spurious interrupts (a minor denial of service). A guest can tell, from when its interrupts arrive, that some tenant used that kind of engine, and roughly how often. It does not learn which tenant, what the work was, or any data. Any unprivileged host process can already observe the same [measured 2026-10-08, RTX 4070]. The project accepted this, since dropping an interrupt would be a correctness bug. An optional pacing setting, `KF3_PT_NSI_MIN_INTERVAL_US` (off by default), can cap how often one guest vector is raised. <!-- src: docs/FAQ.md -->
- **Not measured:** what a guest-triggered GPU fault does to other VMs sharing the card. <!-- src: README.md -->
- **Resources not bounded per VM at the 2026-10-03 source review:** NVENC sessions, runlist time, host BAR1 per VM, GPU time, host RM call rate and host log volume; the page-table walker's work in one refresh is shared across the spaces in a batch, so one guest's layout can make that refresh fail for others. The review's answer for the shared-GPU case is one guest per GPU. These findings are from a source reading dated 2026-10-03; some have since been addressed and the review has not been repeated. <!-- src: docs/design/V3_SECURITY_MODEL.md sections 7, 9, 10 -->

## What the repository does not claim

- No independent audit, no penetration test and no formal proof.
- No statement that a guest cannot reach a host-side bug in NVIDIA's driver. The host driver is trusted. <!-- src: docs/design/V3_SECURITY_MODEL.md section 9 -->
- No claim for a multi-tenant host. The README says the model is designed but not audited, and no public binary ships.

## Related

[Technical FAQ](technical-faq.md) for how the design works; [FAQ](../FAQ.md) for the IOMMU and interrupt questions.

Verified on: the threat model is from the repository's documents of 2026-10-03 (source review at master `7c9234d2`), the README (2026-10-21) and the FAQ (2026-10-08, kayfabe `8c084ab7`, RTX 4070). Not verified end to end: this model was not tested on this page, and the repository states it is not audited.
