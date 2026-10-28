+++
title = "kayfabe"
description = "Run Windows or Linux in a VM on your NVIDIA card with NVIDIA's own driver, while your desktop keeps using it. A research project: build from source."

# Every statement below is taken from README.md, the manual (docs/howto/, docs/FAQ.md) or a committed
# trace, and each `src` field is written into the page as an HTML comment (a src string must not
# contain two hyphens in a row). site/test_build.py fails if a banned claim phrase or first-screen
# jargon appears here. The strict evidence tone lives in the manual, the FAQ and the "Where it
# stands" section; the rest of the page is written for a first-time visitor.

[extra]
page_title = "kayfabe: share one NVIDIA GPU between your desktop and your VMs"
tagline = "Share an NVIDIA GPU with virtual machines that run the stock NVIDIA driver, while the host keeps using it."
tagline_src = "README.md first line (owner's lead sentence)"
lede = "Windows or Linux in a VM runs NVIDIA's own driver on your real card, and your desktop keeps using that card at the same time. kayfabe does this with a QEMU device that presents a virtual NVIDIA GPU to the VM."
lede_src = "README.md opening paragraph; docs/FAQ.md 'Does the host keep the GPU?'"
status = "kayfabe is a research project. There is no installer yet, so you build it from source. [Where it stands](#where-h) is the straight list of what works and what does not."
status_src = "README.md STATUS line (research, no installer, build from source)"

[extra.screenshot]
src = "README.md screenshot and caption; traces/20261021_131847.jpg (downscaled copy)"
alt = "A Windows 11 desktop on one monitor running Unigine Heaven, Minecraft, nvidia-smi in PowerShell and a YouTube video in Edge, next to a Linux desktop on a second monitor running Blender and nvidia-smi."
caption = "Windows 11 on the right, running Unigine Heaven, Minecraft Bedrock, nvidia-smi and a YouTube video in Edge. The Linux host on the left, with Blender, Firefox and an nvidia-smi that lists one GPU used by every app in the picture. Both screens are plugged into that card. One machine, one RTX 4070."

[extra.uses]
heading = "What it is for"
src = "see each scene"

[[extra.uses.items]]
title = "One Windows program on a Linux desktop"
text = "Some programs only exist for Windows and want NVIDIA's driver. Run Windows 11 in a VM, install NVIDIA's normal driver in it, and the program uses your real card while your Linux desktop keeps running on the same one. On an RTX 4070 we have run Minecraft, Unigine Heaven, GravityMark, 4K YouTube with hardware decoding and Blender renders this way."
src = "README.md 'What runs today' Windows table and screenshot caption"

[[extra.uses.items]]
title = "CUDA work in a real VM"
text = "You want a VM with its own kernel and its own driver version, and you do not want to hand the GPU over to it. A Linux VM on kayfabe uses NVIDIA's stock driver, and that driver does not have to match the host's. On a rented RTX 3060, 61 of our 65 CUDA test programs, PyTorch and llama.cpp among them, gave the same output as the bare machine. Big jobs kept pace with the host; programs that fire off many tiny GPU calls fall behind, and [the numbers](#where-h) say by how much."
src = "README.md Linux guest paragraph and CUDA table; docs/howto/requirements.md 'Guest'"

[[extra.uses.items]]
title = "Throwaway VMs on one machine"
text = "Boot a VM for a build or a test run, give it the GPU, delete it afterwards. The host keeps its normal NVIDIA driver, so there is no card to rebind and no vGPU licence to manage. kayfabe's own test suite works like this, with 30 short-lived VMs in a row, one per test. For now we recommend one VM per card at a time."
src = "docs/howto/technical-faq.md 'What do the gates and 30 arms mean?' (one guest boot per arm); docs/howto/security.md (one guest per GPU); branch claude/rootless-audit-20261016 (no VFIO rebind, no driver change)"

[extra.different]
heading = "What is different"
intro = "There are a few ways to put an NVIDIA GPU in a VM today, and each one costs you something."
src = "README.md comparison table; docs/FAQ.md; docs/PROJECT_HISTORY.md section 3; https://gvisor.dev/docs/user_guide/gpu/"
isolated_heading = "How it is isolated"
isolated = "The VM talks to your GPU the way any normal program does, through your NVIDIA driver, so the driver's usual separation between programs applies. That is the design, and nobody has audited it yet. The precise version, with sources, is in [the manual](@docs/security.html#what-the-isolation-is-made-of) and the [FAQ](@docs/FAQ.html)."
isolated_src = "docs/howto/security.md 'What the isolation is made of' and 'Property P' (docs/design/V3_SECURITY_MODEL.md R6, a comparison and not a safety claim); README.md STATUS line (not audited)"

[[extra.different.items]]
name = "VFIO passthrough"
text = "gives the whole card to one VM. While it runs, your desktop has no GPU. With kayfabe the host keeps the card."
src = "README.md comparison table (VFIO: no, one VM owns it)"

[[extra.different.items]]
name = "NVIDIA vGPU"
text = "shares a supported GPU between VMs and needs a licence. The host does not use it."
src = "README.md comparison table (vGPU row)"

[[extra.different.items]]
name = "API forwarding"
text = "([nvkvm-pv](https://github.com/reindertpelsma/nvkvm-pv), virtio-nvgpu) leaves the card with the host, but the VM needs a custom kernel module, so it is Linux only, and the driver versions on both sides have to match. nvkvm-pv is the author's earlier project, and if you want something that works today, use it."
src = "docs/FAQ.md 'How is kayfabe different from nvkvm-pv?'; docs/PROJECT_HISTORY.md section 3; docs/howto/credits.md Acknowledgements"

[[extra.different.items]]
name = "gVisor with nvproxy"
text = "puts a container in front of the driver, not a VM. [Its own docs](https://gvisor.dev/docs/user_guide/gpu/) say it adds no hardware-level isolation beyond what NVIDIA's kernel driver provides. kayfabe also reaches the host driver like an ordinary program, with a VM in front of it, and that has not been audited."
src = "https://gvisor.dev/docs/user_guide/gpu/ (quoted 2026-10-27); docs/howto/security.md"

[[extra.different.items]]
name = "virtio-gpu"
text = "(Venus, virgl) is a generic graphics path, and the VM does not run NVIDIA's own driver. We have not compared the two in detail."
src = "docs/FAQ.md 'Why not virtio-gpu (Venus, virgl or a native context)?'"

[[extra.different.items]]
name = "kayfabe"
text = "gives the VM NVIDIA's own driver, keeps the card with the host, needs no licence server and patches nothing. The VM's driver does not have to match the host's: Windows on 580.88 has run against a 595.91.07 host. In return you need an NVIDIA card from the Turing generation on, the open NVIDIA kernel module, a Linux x86-64 host with KVM, and QEMU. No other hypervisor is supported yet."
src = "README.md comparison table; docs/howto/requirements.md; docs/howto/goals.md 'Hypervisor independence'; docs/FAQ.md 'Does the guest driver have to match the host driver?'"

[extra.where]
heading = "Where it stands"
intro = "kayfabe is a research project with one maintainer. It runs real programs, and it is not ready for your daily machine."
src = "README.md STATUS, What runs today and Not finished; docs/howto/index.md"

[[extra.where.groups]]
title = "What runs"
[[extra.where.groups.items]]
text = """
Windows 11 with NVIDIA's own 580.88 driver, on an RTX 4070: a desktop next to your Linux one,
Minecraft, Unigine Heaven, GravityMark in Direct3D 12 and Vulkan, 4K YouTube with hardware
decoding, CUDA, and Blender renders with CUDA and OptiX. Windows needs three registry settings,
listed in the [manual](@docs/windows-guest.html). These are separate demonstrations and reports,
not a graded all-app pass. Edge failed the cited matrix video run after about seven minutes.
"""
src = "README.md 'What runs today' Windows table; traces/win11_userring_20261014/matrix-eb9f2aa3-20261014-2321.md (revision eb9f2aa3, latest session 89fc8a222); docs/howto/windows-guest.md"
[[extra.where.groups.items]]
text = "Linux VMs on NVIDIA's stock driver: a 30-test suite, a full desktop, suspend and resume, and 61 of 65 CUDA programs matching the bare machine (on a rented RTX 3060 inside a cloud VM). Linux VMs have run on Turing, Ampere, Ada and Blackwell cards, though not on every card in those families."
src = "README.md Linux guest and GPUs paragraphs; docs/howto/requirements.md 'GPU'; docs/howto/linux-guest.md"

[[extra.where.groups]]
title = "What is rough"
[[extra.where.groups.items]]
text = "Windows is not stable. Sleep and wake do not survive, and Edge can make the driver reset after 7 to 16 minutes of video."
src = "README.md 'Not finished'; docs/howto/windows-guest.md"
[[extra.where.groups.items]]
text = "Windows runs real 3D applications. Speed is being benchmarked again on current master, and the results will be posted here."
src = "README.md speed bullet (re-measurement on current master pending); traces/perf_status_20261017/README.md historical banner; commits lazy-prod 1/5 to 5/5 on master (2026-10-17)"
[[extra.where.groups.items]]
text = "On Linux, big CUDA jobs ran at the host's speed on that RTX 3060: matrix multiply at 0.997 of the host, llama.cpp prompt processing at 0.998. Many small launches hurt. PyTorch decoding token by token ran at 0.29 of the host, creating a CUDA context takes about eight times longer, and 4 programs fail because they need unified memory paging, which is not supported yet. The same tests on the RTX 4070 without a cloud VM in between have not been run."
src = "README.md Linux CUDA table and note; docs/design/V3_LINUX_PERF_BASELINE_6692e621.md"
[[extra.where.groups.items]]
text = "Windows with a display has run on three Ada cards (AD104, AD106, AD103). On an Ampere card (GA104) the VM gets the GPU but no display, and one Blackwell card (GB206) stops with Code 43. Only NVIDIA driver 580.88 has been tried."
src = "README.md 'GPUs:' paragraph; docs/howto/requirements.md 'GPU' table"
[[extra.where.groups.items]]
text = "SolidWorks, OBS recording and 3DMark runs are reported in the README, but their result files are not committed yet, so treat them as claims."
src = "README.md SolidWorks, OBS and 3DMark rows ('evidence not yet committed')"
[[extra.where.groups.items]]
text = "In one audit on one machine, the Linux test suite and a Windows benchmark ran as an ordinary user, not root. The record is on a branch, and the launch scripts in the repository still start as root."
src = "branch claude/rootless-audit-20261016 commit 32d7319b2, traces/rootless_audit_20261016/REPORT.txt; docs/howto/security.md"

[[extra.where.groups]]
title = "What does not exist yet"
[[extra.where.groups.items]]
text = "An installer, a public binary and a stable interface. You build from source, and an installer is planned, with no date."
src = "README.md 'Trying it' and STATUS line; docs/howto/goals.md 'An installer and a stable interface'; docs/howto/credits.md (LICENSE exception 2)"
[[extra.where.groups.items]]
text = "A security audit. Nobody has measured what one VM's crash does to the other VMs on the same card, and sharing a card is untested beyond booting two VMs together."
src = "README.md 'Not finished'; traces/windows_code43_walls_20261007/README.md run54 (a88764b3, 2026-10-08); docs/FAQ.md 'Can several VMs share one GPU?'"
[[extra.where.groups.items]]
text = "Support for Proxmox or libvirt, for hypervisors other than QEMU, for hosts other than Linux on x86-64, and for cards older than Turing, which cannot work."
src = "docs/howto/goals.md; docs/howto/requirements.md"
[[extra.where.groups.items]]
text = "Games with kernel anti-cheat. The project's own research notes that products such as Vanguard and FACEIT refuse VMs, and getting around that is a non-goal."
src = "docs/design/V3_VIOMMU_NEED_RESEARCH_20261013.md summary item 3; docs/FAQ.md 'Do games with anti-cheat work?' (owner direction 2026-10-28)"

[extra.licence]
text = "kayfabe ships no NVIDIA or Microsoft files. NVIDIA's driver licence applies to the driver you install on the host and in the VM, and if you host VMs for other people, that is your responsibility. See [Your licences](@docs/credits.html#your-licences)."
src = "docs/howto/credits.md 'No vendor binaries' and 'Your licences'"

[extra.start]
heading = "Try it"
src = "README.md 'Trying it'; docs/howto/building.md; docs/howto/installing.md; docs/howto/goals.md"
text = "There is no installer. If you are happy building Rust and QEMU from source, the manual lists the steps the project's own scripts perform. Read it first, because those scripts were written for throwaway test machines. You need a Linux x86-64 host with KVM and an NVIDIA card from Turing on; `cargo test` runs without a GPU."
command = "cargo build --workspace --release"
after = "That builds the Rust part. The QEMU binary with the kayfabe device is a second step, and the manual covers it."

[extra.community]
heading = "Follow along"
src = "docs/howto/credits.md; docs/howto/security.md 'Reporting a vulnerability'"
licence = "Dual-licensed Apache-2.0 OR GPL-2.0-or-later. Struct layouts, class ids and register offsets are generated from NVIDIA's open-source GPU kernel modules (MIT). Windows must be licensed for the way you use it."
licence_src = "docs/howto/credits.md 'Licence', 'Third-party notices', 'Your licences'"
+++
