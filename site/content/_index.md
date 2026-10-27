+++
title = "kayfabe"
description = "Share an NVIDIA GPU with virtual machines that run the stock NVIDIA driver, while the host keeps using it."

# Every statement below is taken from README.md or the manual (docs/howto/). The `src` fields are
# written into the page as HTML comments, so the source of each sentence stays next to it.

[extra]
tagline = "Share an NVIDIA GPU with virtual machines that run the stock NVIDIA driver, while the host keeps using it."
tagline_src = "README.md, first line"
intro = "Kayfabe is a QEMU device that shows a guest an emulated NVIDIA GPU. The guest loads NVIDIA's real, unmodified driver (Windows or Linux); kayfabe reads what that driver asks for and has the real host GPU do the work, from an **unprivileged** host process that talks to the host driver as an ordinary client. Written in Rust."
intro_src = "README.md, first paragraph"

[extra.status]
heading = "Status: research"
src = "README.md 'Status: research, 2026-10-21'; docs/howto/index.md 'Status: read this first'"
lead = "Kayfabe runs real applications, but there is no installer, no stable interface and no security audit yet. No public binary ships: you build it from source."
more = "Not finished, as the README lists it: Windows sleep/wake does not survive; other GPU dies need more work under Windows; the security model is designed but not audited, and what a guest-triggered GPU fault does to other VMs sharing the card is not yet measured."
dated = "These statements are from the README of 2026-10-21. The README carries the current status."

[[extra.properties]]
title = "The host keeps using the GPU"
text = "With VFIO passthrough one VM owns the GPU. With kayfabe the host keeps it. In the project's screenshot, nvidia-smi shows a single GPU used by all the apps, with Firefox on the host in the background."
src = "README.md comparison table (VFIO: 'no, one VM owns it'; kayfabe: 'yes') and the screenshot caption '(host keeps gpu)'"

[[extra.properties]]
title = "The guest runs NVIDIA's stock driver"
text = "The guest loads NVIDIA's real, unmodified driver, on Windows or Linux, installed with NVIDIA's own installer. Nothing is patched: no host or guest driver or kernel is modified."
src = "README.md 'Nothing is patched'; docs/howto/windows-guest.md and linux-guest.md"

[[extra.properties]]
title = "The guest is treated as hostile"
text = "Every host action is authored by kayfabe from a few unprivileged verbs, never forwarded from guest bytes. The host process is unprivileged by design and talks to the host driver as an ordinary client. The design is not audited, and the launch scripts in the repository run as root."
src = "README.md 'The guest is treated as hostile'; docs/howto/security.md 'The host process is unprivileged by design'"

[extra.path]
heading = "How a guest application reaches the GPU"
src = "README.md first paragraph; docs/howto/security.md 'Deployment: one process' (kayfabe is linked into the VMM)"
note = "Kayfabe is linked into the VMM: the QEMU device and the host-side code are one process, with no helper processes."
steps = [
  "A guest application uses the GPU.",
  "NVIDIA's stock driver in the guest, unmodified, loads on the emulated NVIDIA GPU.",
  "That GPU is emulated: a QEMU device that kayfabe provides.",
  "Kayfabe reads what the driver asks for and authors the host actions itself, from unprivileged verbs.",
  "The host's NVIDIA kernel driver receives those calls as it would from any ordinary client.",
  "The real GPU does the work, and the host keeps using it.",
]

[extra.compare]
heading = "How it differs from other approaches"
src = "README.md 'Why it is interesting' table, unchanged"
columns = ["", "host keeps the GPU", "safe for untrusted guests", "guest driver", "needs"]
[[extra.compare.rows]]
cells = ["VFIO passthrough", "no, one VM owns it", "yes", "stock", "any GPU"]
[[extra.compare.rows]]
cells = ["NVIDIA vGPU", "no, VMs share it, the host does not", "yes", "vendor vGPU driver", "data-centre GPU, licence"]
[[extra.compare.rows]]
cells = ["API forwarding ([nvkvm-pv](https://github.com/reindertpelsma/nvkvm-pv) / virtio-nvgpu)", "yes", "possible, but fragile for multi-tenant VMs", "a custom guest module", "any NVIDIA GPU"]
[[extra.compare.rows]]
highlight = true
cells = ["**kayfabe**", "**yes**", "**yes, by design (not yet audited)**", "**stock, unmodified**", "any NVIDIA GPU is the goal; tested on consumer cards (Turing to Blackwell); the open NVIDIA host driver"]

[extra.screenshot]
src = "README.md screenshot and caption; traces/20261021_131847.jpg (downscaled copy)"
alt = "A Windows 11 desktop on one monitor running Unigine Heaven, Minecraft, nvidia-smi in PowerShell and a YouTube video in Edge, next to a Linux desktop on a second monitor running Blender and nvidia-smi."
caption = "Windows 11 on kayfabe: desktop and nvidia-smi under the stock 580.88 driver. On Windows, displayed using nvkvm broker on an Ubuntu desktop: Minecraft Bedrock/Windows only edition, Unigine Heaven benchmark, nvidia-smi in powershell, Big Buck Bunny Blender on Youtube in Edge. On Linux left screen: Blender, nvidia-smi showing a single gpu being used for all the apps in the screenshots and Firefox in background (host keeps gpu). There is no PCIe/DMA copy of frames for this windows desktop (i.e both screens are directly plugged in the GPU)."

[extra.today]
heading = "What runs today"
src = "docs/howto/index.md 'What has been run'"
intro = "These are the repository's own claims, with their limits. Each manual page repeats the evidence for what it describes."
items = [
  "**Windows 11 guest** (Pro and Enterprise LTSC, NVIDIA's stock 580.88 driver): desktop, games, benchmarks, video decode, CUDA and professional applications on an RTX 4070 (AD104). A few registry settings are required: see [Windows guest](@docs/windows-guest.html).",
  "**Linux guest** (NVIDIA's stock driver, nothing to set): a 30-arm test suite, a desktop with display planes, suspend/resume, and 61 of 65 CUDA applications with outputs equal to bare metal on a rented RTX 3060. The 4 failures use unified (UVM) memory paging and fail loudly. See [Linux guest](@docs/linux-guest.html).",
  "**GPUs:** the families from Turing to Blackwell have been run in some form. The detail is on [Requirements](@docs/requirements.html).",
]

[extra.start]
heading = "Get started"
src = "README.md 'Trying it'; docs/howto/building.md; docs/howto/installing.md"
text = "There is no user install path yet; it is still being built. Developers can build from source (`cargo test` runs without a GPU)."
command = "cargo build --workspace --release"

[extra.community]
heading = "Contribute and community"
src = "docs/howto/credits.md; docs/howto/security.md 'Reporting a vulnerability'"
+++
