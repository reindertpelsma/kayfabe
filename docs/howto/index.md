# kayfabe user manual

Kayfabe shares an NVIDIA GPU with virtual machines that run the stock NVIDIA driver, while the host keeps using it. It is a QEMU device that shows a guest an emulated NVIDIA GPU. The guest loads NVIDIA's unmodified driver on Windows or Linux; kayfabe reads what that driver asks for and has the real host GPU do the work, from an unprivileged host process. It is written in Rust. <!-- src: README.md -->

This manual is in three parts. **Part 1** is for everyone who wants to install kayfabe, run a Linux or Windows guest, fix a problem and report one. **Part 2** is for those who want to test it themselves with open software or with their own images. **Part 3** is for operators who run their own CI. You do not need to know how NVIDIA's driver works to use Part 1.

## Status: read this first

<!-- src: README.md, "Status: research, 2026-11-08" -->
- **Research stage.** It runs real applications, but there is no installer yet, no stable interface and no security audit.
- **No public binary ships yet.** You build it from source ([Install](installing.md)). A QEMU binary built from this tree is not yet distributable under the GPL (see [Credits and notices](credits.md)). <!-- src: README.md, LICENSE exception 2 -->
- **The security model is designed but not audited.** See [Security model](security.md). <!-- src: README.md "Not finished" -->
- **Windows sleep/wake and Edge playback stability are unfinished.**

Nothing is patched: no host or guest driver or kernel is modified, and the guest uses NVIDIA's own installer. Kayfabe ships no NVIDIA or Microsoft binaries. <!-- src: README.md "Nothing is patched"; docs/NOTICES.md -->

## What has been run

- **Windows 11 guest** (Pro and Enterprise LTSC, NVIDIA's stock 580.88 driver): desktop, games, benchmarks, video decode, CUDA and professional applications on an RTX 4070. A few registry settings are required, and a script sets them: see [Run a Windows guest](windows-guest.md). <!-- src: README.md -->
- **Linux guest:** 66 of 70 runnable test applications pass in the guest, against 70 of 70 on the host. The four failing ones need demand-paged unified memory, which is not supported yet. See [Run a Linux guest](linux-guest.md). <!-- src: traces/apps_linux_20261107/README.md; docs/OWNER_RULINGS.md section E -->
- **GPUs:** the families from Turing to Blackwell have been run in some form. See [Requirements](requirements.md).

## Part 1. Use kayfabe

| Page | What it answers |
|---|---|
| [Requirements](requirements.md) | Which GPU, driver, kernel, QEMU and CPU the project has been run on |
| [Install](installing.md) | The four steps: get the QEMU, install your guest, start it with the kayfabe device |
| [Building from source](building.md) | The exact steps that build the kayfabe QEMU |
| [Run a Linux guest](linux-guest.md) | Linux guests: driver, device, what has been run |
| [Attach to a VM you already have](attach-to-existing-vm.md) | The QEMU arguments, libvirt notes and guest settings for a VM you already run |
| [Run a Windows guest](windows-guest.md) | The guest settings, the setup script and what has been tested |
| [Display and the broker](display.md) | Showing a guest's screen on your desktop, and the two-command quickstart |
| [Command-line reference](cli-reference.md) | Every device property, its type and default, and the broker's options |
| [Troubleshooting](troubleshooting.md) | The messages and symptoms you will meet, and what to do |
| [Report a problem](report-a-problem.md) | What to attach to a report, and what to leave out |
| [Security model](security.md) | What is trusted, what is not, and how to report a vulnerability |
| [Technical FAQ](technical-faq.md) | How it works and why it is designed this way |
| [Goals and plans](goals.md) | What the project aims at later, as goals with their status today |
| [Credits and notices](credits.md) | Licences, trademarks, acknowledgements |
| [FAQ](../FAQ.md) | Common questions: vGPU, VFIO, GPUs and drivers, sharing, performance, security, the IOMMU and interrupts |

## Part 2. Test it yourself

| Page | What it answers |
|---|---|
| [What you can test](test-yourself.md) | What runs out of the box, and how to choose what to test |
| [Run everything in one go](full-standalone-run.md) | The standalone full run on a rented or local machine |
| [Run the open FFmpeg and DCGM rows](open-linux-rows.md) | Video encode and decode, and NVML and DCGM sweeps, with open software |
| [Run the Vulkan conformance subset](vulkan-cts.md) | The Vulkan conformance tests on the host and in a guest |
| [Run the open ML rows (vLLM, TensorRT, JAX)](open-ml-rows.md) | Machine-learning workloads with open software |
| [Run the open Windows rows](windows-open-rows.md) | VK-GL-CTS and DirectX samples on a Windows guest |
| [Rebuild the Linux apps image](linux-apps-image.md) | Building the Linux guest image of the application tests |
| [The Windows test harness](windows-harness.md) | The script that installs and boots a Windows 11 test guest |
| [Bring your own Windows images](run-windows-images.md) | Running applications on Windows images you built |
| [Proprietary test images](proprietary-test-images.md) | What the maintainers' private test images are and how they are used |
| [Recreate the proprietary Windows image](recreate-the-proprietary-image.md) | Building your own copy of the application image with your own accounts and licences |

## Part 3. Run your own CI (operators)

For people who deploy the Cloudflare Worker and rent GPU boxes. You do not need these pages to use kayfabe. [Overview](ops/index.md).

## Where else to look

- [Repository README](https://github.com/reindertpelsma/kayfabe#readme): the landing text and current status.
- [Design and internals](https://github.com/reindertpelsma/kayfabe/tree/master/docs/): design documents, status records and evidence. They are for contributors, are not part of this manual and change faster than it does.
- To report a suspected vulnerability, see [Security model](security.md).
