# Security model

> **Kayfabe has not been security audited.** The model below is a design, and the repository's own review of it (a source reading, 2026-10-03) listed gaps. Do not run untrusted or multi-tenant guests on a machine you cannot afford to lose. <!-- src: README.md, SECURITY.md, docs/design/V3_SECURITY_MODEL.md -->

## Reporting a vulnerability

Report it privately through GitHub's private vulnerability reporting: <https://github.com/reindertpelsma/kayfabe/security/advisories/new>. Do not open a public issue. Include the revision (the `KAYFABE_REV.txt` file or the git hash), what you did and what you saw. Reports are read by the maintainer. <!-- src: SECURITY.md -->

In scope: a guest-reachable unbounded read or emit, a guest-controlled host action, or any way for a guest to reach host memory or devices it was not given. <!-- src: SECURITY.md -->

## The threat model

- **The guest is hostile.** Guest userspace and guest root are both untrusted. The boundary inside the guest is also untrusted: unprivileged guest userspace can write any doorbell token, so no check at the trap can establish identity, and the design limits what such a write can reach. <!-- src: AGENTS.md "Rules", ARCHITECTURE.md "Rules that shape the code" -->
- **Host actions are authored, never forwarded.** Every call kayfabe makes on the host GPU is built by kayfabe from a few verbs, using the host driver's unprivileged interface. Nothing the guest sends is replayed to the host. Answers to the guest's driver come from facts read from the host GPU through unprivileged controls, from generated family rules, or from values kayfabe fabricates so the driver's own code accepts them. <!-- src: ARCHITECTURE.md "The shape" -->
- **The host process is unprivileged by design.** The host side uses only controls the host driver marks non-privileged and needs no host kernel module. A recorded end-to-end run without root is not documented yet, and the launch scripts in the repository run as root. <!-- src: README.md, docs/PRODUCT_POSITIONING.md, scripts/bench/display/interactive.sh -->
- **No CPU executes GPU work**, and no completion is forged for work that reached the GPU. Only emulated channels, which have no GPU work behind them, complete inside the VMM. <!-- src: AGENTS.md, ARCHITECTURE.md -->
- **Only writes to the device's register window trap.** The vCPU never blocks on host work. <!-- src: ARCHITECTURE.md -->
- **No unsafe code** outside three crates and only in files named `*_unsafe.rs`. <!-- src: AGENTS.md -->

### What the isolation is made of

Hardware: the GPU's memory management unit and the privilege of the channel. Software: kayfabe's own checks and the host driver. The guest's work runs on host channels that are unprivileged and live in a GPU address space holding only that VM's mappings. Physical addressing needs a privileged channel: on a user-privilege channel the copy engine refused a physical operand with a channel reset, while a virtual operand was delivered [measured, kayfabe `8c084ab7`, RTX 4070, driver 595.91.07, 2026-10-08]. A bug in kayfabe's address resolution therefore matters more than it would under VFIO. Details, including why an IOMMU is not part of the argument, are in the [FAQ](FAQ.md). <!-- src: docs/FAQ.md -->

## Deployment: one process, you confine it

Kayfabe is linked into the VMM. There are no helper processes, no IPC and no sandbox of its own. **Sandboxing is the deployer's job: confine the whole hypervisor process.** <!-- src: ARCHITECTURE.md "One process", docs/design/V3_SECURITY_MODEL.md section 0 -->

The deployment assumptions the repository's security review lists (as recommendations; it was written to be stated in the support notes) are: <!-- src: docs/design/V3_SECURITY_MODEL.md section 11 -->

- a sandboxed VMM;
- one uid per VM;
- `CAP_SYS_ADMIN` dropped by the launcher (recommended, not enforced);
- host scrub-on-free left enabled;
- profiling restricted to admin users (the host driver's default);
- no `--privileged` container;
- a PCI topology where the guest cannot unplug the device, until that issue is closed.

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

[Technical FAQ](technical-faq.md) for how the design works; [FAQ](FAQ.md) for the IOMMU and interrupt questions.

Verified on: the threat model is from the repository's documents of 2026-10-03 (source review at master `7c9234d2`), the README (2026-10-21) and the FAQ (2026-10-08, kayfabe `8c084ab7`, RTX 4070). Not verified end to end: this model was not tested on this page, and the repository states it is not audited.
