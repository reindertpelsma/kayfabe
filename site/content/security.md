+++
title = "Security model"
description = "A short summary of kayfabe's security model and how to report a vulnerability. The manual has the full page."
+++

<!-- src: docs/howto/security.md, opening note -->
> **Kayfabe has not been security audited.** The model is a design, and the repository's own review of it (a source reading, 2026-10-03) listed gaps. Do not run untrusted or multi-tenant guests on a machine you cannot afford to lose.

This page only summarises. The full, sourced text is [Security model in the manual](/@docs/security.html).

## The model in four lines

<!-- src: docs/howto/security.md "The threat model" -->
- The guest is hostile. Guest userspace and guest root are both untrusted.
- Host actions are authored, never forwarded: every call kayfabe makes on the host GPU is built by kayfabe from a few verbs, using the host driver's unprivileged interface. Nothing the guest sends is replayed to the host.
- The host process is meant to run unprivileged. In one audit on one machine, the Linux test suite and a Windows benchmark ran as an ordinary user, not root. The record is on the branch `claude/rootless-audit-20261016`. The launch scripts in the repository still start as root.
- Confining the VMM, as an ordinary user and ideally in a container or sandbox, is an extra layer we recommend. kayfabe is linked into the VMM, so you confine the whole hypervisor process.

## What is not claimed

<!-- src: docs/howto/security.md "What the repository does not claim" -->
No independent audit, no penetration test and no formal proof. No statement that a guest cannot reach a host-side bug in NVIDIA's driver; the host driver is trusted. No claim for a multi-tenant host. What a guest-triggered GPU fault does to other VMs sharing the card is not measured.

## Reporting a vulnerability

<!-- src: docs/howto/security.md "Reporting a vulnerability"; SECURITY.md -->
Report it privately through [GitHub's private vulnerability reporting](https://github.com/reindertpelsma/kayfabe/security/advisories/new). Do not open a public issue. Include the revision (the `KAYFABE_REV.txt` file or the git hash), what you did and what you saw.
