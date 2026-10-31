+++
title = "Security model"
description = "A short summary of kayfabe's security model and how to report a vulnerability. The manual has the full page."
+++

<!-- src: docs/howto/security.md, opening note -->
> **Kayfabe has not been security audited.** The model is a design, and the repository's own review of it (a source reading, 2026-10-03) listed gaps. Do not run untrusted or multi-tenant guests on a machine you cannot afford to lose.

The [security model in the manual](/@docs/security.html) explains the design and its evidence.

## The model in five lines

<!-- src: docs/howto/security.md 'The threat model', 'What the isolation is made of', 'Deployment' -->
- The guest is hostile. Guest userspace and guest root are both untrusted.
- Kayfabe builds the requests it sends to the host driver and checks what the guest asks to map and run.
- The launch scripts still use root; a run without sudo is recorded on one host.
- By design, running the hypervisor as an ordinary user in a container or sandbox adds a layer of protection.
- NVIDIA driver bugs and KVM bugs still apply. Enable the host IOMMU when you run untrusted guests.

## What is not claimed

<!-- src: docs/howto/security.md "What the repository does not claim" -->
No independent audit, no penetration test and no formal proof. No statement that a guest cannot reach a host-side bug in NVIDIA's driver; the host driver is trusted. No claim for a multi-tenant host. What a guest-triggered GPU fault does to other VMs sharing the card is not measured.

## Reporting a vulnerability

<!-- src: docs/howto/security.md "Reporting a vulnerability"; SECURITY.md -->
Report it privately through [GitHub's private vulnerability reporting](https://github.com/reindertpelsma/kayfabe/security/advisories/new). Do not open a public issue. Include the revision (the `KAYFABE_REV.txt` file or the git hash), what you did and what you saw.
