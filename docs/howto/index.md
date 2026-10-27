# kayfabe user manual

Kayfabe shares an NVIDIA GPU with virtual machines that run the stock NVIDIA driver, while the host keeps using it. It is a QEMU device that shows a guest an emulated NVIDIA GPU. The guest loads NVIDIA's unmodified driver on Windows or Linux; kayfabe reads what that driver asks for and has the real host GPU do the work, from an unprivileged host process. It is written in Rust. <!-- src: README.md -->

This manual is for developers and system administrators who want to build, run and deploy it. It describes what exists in the repository today and says so plainly where something does not exist yet.

## Status: read this first

<!-- src: README.md, "Status: research, 2026-10-21" -->
- **Research stage.** It runs real applications, but there is **no installer**, **no stable interface** and **no security audit**.
- **No public binary ships.** You build it from source. A QEMU binary built from this tree is not yet distributable under the GPL (see [Credits and notices](credits.md)). <!-- src: README.md, LICENSE exception 2 -->
- **The security model is designed but not audited.** What a guest-triggered GPU fault does to other VMs sharing the card is not yet measured. See [Security model](security.md). <!-- src: README.md "Not finished" -->
- **Windows sleep/wake does not survive**, Edge can reset after 7 to 16 minutes of playback, and other GPU dies need more work under Windows. <!-- src: README.md "Not finished" -->
- Master is the current working version; newer experiments live on branches. <!-- src: README.md -->

Nothing is patched: no host or guest driver or kernel is modified, and the guest uses NVIDIA's own installer. <!-- src: README.md "Nothing is patched" --> Kayfabe ships no NVIDIA or Microsoft binaries. <!-- src: docs/NOTICES.md -->

## What has been run

These are the repository's own claims, with their limits. Each page of this manual repeats the evidence for what it describes. <!-- src: README.md "What runs today" -->

- **Windows 11 guest** (Pro and Enterprise LTSC, NVIDIA's stock 580.88 driver): desktop, games, benchmarks, video decode, CUDA and professional applications on an RTX 4070 (AD104). A few registry settings are required: see [Windows guest](windows-guest.md). <!-- src: README.md -->
- **Linux guest** (NVIDIA's stock driver, nothing to set): a 30-arm test suite, a desktop with display planes, suspend/resume, and 61 of 65 CUDA applications with outputs equal to bare metal on a rented RTX 3060. The 4 failures use unified (UVM) memory paging and fail loudly. See [Linux guest](linux-guest.md). <!-- src: README.md -->
- **GPUs:** the families from Turing to Blackwell have been run in some form. The detail is on [Requirements](requirements.md).

## Pages

| Page | What it answers |
|---|---|
| [Requirements](requirements.md) | What GPU, driver, kernel, QEMU and CPU the repository's evidence covers |
| [Building from source](building.md) | The exact steps the build scripts perform |
| [Running the QEMU device](installing.md) | There is no installer: how the device is provisioned and started today, and its properties |
| [Linux guest](linux-guest.md) | Preparing and booting a Linux guest |
| [Windows guest](windows-guest.md) | The guest settings and the Windows harness |
| [Display and the broker](display.md) | Showing a guest's screen on the host |
| [Troubleshooting](troubleshooting.md) | Symptoms that are documented in the repository |
| [Security model](security.md) | What is trusted, what is not, and what is unfinished |
| [Technical FAQ](technical-faq.md) | How it works and why it is designed this way |
| [Goals and plans](goals.md) | What the project aims at later (other hypervisors, arm64, several GPUs in one guest), as goals with their status today |
| [Credits and notices](credits.md) | Licences, trademarks, acknowledgements |
| [FAQ](FAQ.md) | Common questions: vGPU, VFIO, GPUs and drivers, sharing, performance, security, the IOMMU and interrupts |

## How to read the evidence labels

- **[measured]** means a run on named hardware at a named revision.
- **[inferred]** means reasoning, not a measurement.
- **not documented yet** means the repository does not state a verified answer. The manual does not guess.
- Every page ends with a **Verified on** line. Where the repository's evidence does not cover a page end to end, it says **Not verified end to end**.

## Where else to look

- [Repository README](https://github.com/reindertpelsma/kayfabe#readme): the landing text and current status.
- [Design and internals](https://github.com/reindertpelsma/kayfabe/tree/master/docs/): design documents, status records and evidence. They are for contributors, are not part of this manual and change faster than it does.
- To report a suspected vulnerability, see [Security model](security.md).

Verified on: the status lines are copied from the repository README of 2026-10-21 (master `89fc8a222`). Not verified end to end: this page is a map, not a procedure.
