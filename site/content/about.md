+++
title = "About kayfabe"
description = "What kayfabe is, why it exists, its status, its licence and the related projects it credits."
+++

## What it is

<!-- src: README.md, first paragraph -->
Kayfabe is a QEMU device that shows a guest an emulated NVIDIA GPU. The guest loads NVIDIA's real, unmodified driver (Windows or Linux); kayfabe reads what that driver asks for and has the real host GPU do the work, from an **unprivileged** host process that talks to the host driver as an ordinary client. Written in Rust.

Mechanically, kayfabe plays the part of the GPU's GSP, the on-GPU firmware processor that current NVIDIA drivers send their resource-manager commands to. The manual's [Technical FAQ](/@docs/technical-faq.html) explains how that works and why it is built this way.
<!-- src: docs/howto/technical-faq.md, "What is kayfabe, mechanically?" -->

## Why it exists

<!-- src: README.md, 'Why it is interesting' table -->
The README compares the common ways of giving a virtual machine an NVIDIA GPU. With VFIO passthrough one VM owns the GPU and the host does not keep it. NVIDIA vGPU needs a data-centre GPU and a licence, and the VMs share the GPU without the host. API forwarding lets the host keep the GPU, but it needs a custom guest module and is described as fragile for multi-tenant VMs. In kayfabe's row the host keeps the GPU, the guest driver is stock and unmodified, and untrusted guests are safe by design (not yet audited). The [comparison table](/#compare-h) on the start page has the details.

## Status

<!-- src: README.md 'Status: research, 2026-10-21' -->
Research. Kayfabe runs real applications, but there is no installer, no stable interface and no security audit yet, and no public binary ships. The [README](https://github.com/reindertpelsma/kayfabe#readme) carries the current status, and the manual's [introduction](/@docs/index.html) repeats it with its sources. A short summary of the security model is on the [Security](/security/) page.

## Licence

<!-- src: README.md, docs/howto/credits.md "Licence" -->
Dual-licensed **Apache-2.0 OR GPL-2.0-or-later**. The repository's [`LICENSE`](https://github.com/reindertpelsma/kayfabe/blob/master/LICENSE) lists the exceptions, and a QEMU binary built from the tree is not yet distributable under the GPL. Read [Credits and notices](/@docs/credits.html) before you redistribute anything.

## Related projects

Other projects work on a GPU for a virtual machine that runs the stock NVIDIA driver. They are credited on [Credits and notices](/@docs/credits.html), for readers who want to compare approaches.

## Where to talk to the project

The repository is at [github.com/reindertpelsma/kayfabe](https://github.com/reindertpelsma/kayfabe). Questions, bugs and documentation gaps go to its [issues](https://github.com/reindertpelsma/kayfabe/issues). Report a suspected vulnerability privately, as the [Security](/security/) page describes.

## About this site

The manual's source is in `docs/howto/` in the repository, and every manual page has a "View and edit source on GitHub" link.
