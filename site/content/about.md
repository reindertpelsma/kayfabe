+++
title = "About kayfabe"
description = "What kayfabe is, why it exists, where it stands, and how to reach the project."
+++

## What it is

<!-- src: README.md, opening paragraph -->
kayfabe lets a virtual machine use your NVIDIA graphics card with NVIDIA's own driver, while the computer you are sitting at keeps using the same card. It is a QEMU device written in Rust. To the VM it looks like an NVIDIA GPU, and behind it the real work runs on your real card.

<!-- src: docs/howto/technical-faq.md, "What is kayfabe, mechanically?" -->
Under the hood, kayfabe plays the part of the GPU's firmware processor, the GSP, which current NVIDIA drivers send their commands to. The manual's [Technical FAQ](/@docs/technical-faq.html) explains how that works and why it is built this way.

## Why it exists

<!-- src: README.md comparison table; docs/FAQ.md -->
Every common way of giving a VM an NVIDIA card takes something from you. VFIO hands the whole card to one VM. NVIDIA's vGPU needs a data-centre GPU and a licence. API forwarding leaves the card with the host, but the VM needs a custom kernel module. kayfabe tries to keep the card with the host, run NVIDIA's own driver in the VM, and ask for no licence server. The [home page](/#diff-h) lines the options up.

## Where it stands

<!-- src: README.md STATUS line and 'Not finished' -->
Research. It runs real programs, but there is no installer, no public binary, no stable interface and no security audit, and Windows is rough: sleep and wake do not survive. The home page has the straight list under [Where it stands](/#where-h), and the [README](https://github.com/reindertpelsma/kayfabe#readme) carries the current status. A short note on the security model is on the [Security](/security/) page.

## Licence

<!-- src: README.md, docs/howto/credits.md "Licence" -->
Dual-licensed Apache-2.0 OR GPL-2.0-or-later. The repository's [`LICENSE`](https://github.com/reindertpelsma/kayfabe/blob/master/LICENSE) lists the exceptions, and a QEMU binary built from the tree is not yet distributable under the GPL. Read [Credits and notices](/@docs/credits.html) before you redistribute anything.

## Related projects

Other people are working on the same problem. They are credited on [Credits and notices](/@docs/credits.html), for anyone who wants to compare approaches.

## Talk to the project

The code is at [github.com/reindertpelsma/kayfabe](https://github.com/reindertpelsma/kayfabe). Questions, bugs and gaps in the docs go to the [issue tracker](https://github.com/reindertpelsma/kayfabe/issues). A suspected vulnerability goes through GitHub, privately, as the [Security](/security/) page describes.

## About this site

The manual's source is in `docs/howto/` in the repository, and every manual page has a "View and edit source on GitHub" link.
