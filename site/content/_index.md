+++
title = "kayfabe"
description = "Run Windows or Linux in a VM that uses your NVIDIA GPU with NVIDIA's own driver, while your Linux host keeps using the same card. Research project: build from source."

# Every statement below is taken from README.md, the manual (docs/howto/, docs/FAQ.md) or a committed
# trace, and each `src` field is written into the page as an HTML comment (a src string must not
# contain two hyphens in a row). site/test_build.py fails if a banned claim phrase appears here.
# Defaults applied from the coordinator's direction are listed in the pull request description.

[extra]
page_title = "kayfabe: one NVIDIA card, shared with a VM that runs NVIDIA's real driver"
tagline = "One NVIDIA card. Your Linux desktop keeps it. The VM gets NVIDIA's real driver."
tagline_src = "README.md first paragraph and 'host keeps gpu' screenshot caption"
intro = "Kayfabe is a QEMU device that shows a virtual machine an emulated NVIDIA GPU. Windows or Linux in the VM installs NVIDIA's normal driver on it. The real work runs on your real card, from an unprivileged host process, and your Linux desktop keeps using that card while it does. Written in Rust."
intro_src = "README.md first paragraph; docs/howto/index.md"

[extra.status]
heading = "Status: research"
src = "README.md 'Status: research, 2026-10-21'; docs/howto/index.md 'Status: read this first'; the one-maintainer wording is the project's stated default (task direction), not written elsewhere in the repository"
items = [
  "**Research project, one maintainer.**",
  "**No installer yet.** You build it from source. There is no public binary and no stable interface.",
  "**Windows is not stable.** Sleep and wake fail, and some apps make the GPU driver reset (Edge after 7 to 16 minutes of playback).",
  "**Not security audited.**",
]
more = "Everything on this page names the hardware and revision it was measured on, or says it was not measured. The table [What works today](#today-h) is the short version."

[extra.screenshot]
src = "README.md screenshot and caption; traces/20261021_131847.jpg (downscaled copy)"
alt = "A Windows 11 desktop on one monitor running Unigine Heaven, Minecraft, nvidia-smi in PowerShell and a YouTube video in Edge, next to a Linux desktop on a second monitor running Blender and nvidia-smi."
caption = "Right screen: Windows 11 with NVIDIA's stock 580.88 driver, running Unigine Heaven, Minecraft Bedrock, nvidia-smi in PowerShell and a YouTube video in Edge. Left screen: the Linux host, with Blender, Firefox and an nvidia-smi that lists one GPU used by all the apps in the picture. Both screens are plugged into that GPU, and there is no PCIe copy of frames for the Windows desktop. One machine, one RTX 4070."

[extra.pitch]
heading = "In twenty seconds"
note = "Design intent, not a claim of safety: every host action is authored by kayfabe from a few unprivileged calls and never forwarded from guest bytes, so the guest cannot drive the hardware. The design has not been audited, and the project recommends one guest per GPU. [Security model](@docs/security.html)."
note_src = "README.md 'The guest is treated as hostile'; docs/howto/security.md (not audited; one guest per GPU for the shared case)"

[[extra.pitch.items]]
title = "The host keeps the card"
text = "A VM uses the GPU while your Linux desktop keeps using it. With VFIO passthrough one VM owns the card and the host does not keep it. The screenshot above is the author's RTX 4070 doing both at once."
src = "README.md comparison table (VFIO: 'no, one VM owns it') and screenshot caption '(host keeps gpu)'"

[[extra.pitch.items]]
title = "No VFIO rebind, no vGPU licence, no host driver swap"
text = "The host stays on its normal NVIDIA driver, nothing in it is patched, and there is no vGPU driver or licence server. In one audit the host side ran as an ordinary user in group kvm, with no capabilities and no VFIO rebind. That was measured on one host (RTX 4070, revision `0a746117`), and the record is on the branch `claude/rootless-audit-20261016`, not on master. The shipped launch scripts still run as root, and the provisioning scripts for rented test machines replace the driver: use them only on throwaway machines."
src = "README.md 'Nothing is patched'; docs/FAQ.md 'Is this vGPU or SR-IOV?' and 'Does the host keep the GPU?'; branch claude/rootless-audit-20261016 commit 32d7319b2, traces/rootless_audit_20261016/REPORT.txt (verdict paragraph); docs/howto/security.md (launch scripts run as root)"

[[extra.pitch.items]]
title = "Big CUDA jobs at host speed, in a real VM"
text = "In a Linux guest, large kernels ran at the host's speed: matrix multiply 0.997 of the host, llama.cpp prompt processing 0.998. That was measured on a rented RTX 3060 that is itself a VM, at older revisions. Work made of many small launches is slower: eager PyTorch decoding ran at 0.29 of the host."
src = "README.md Linux CUDA table (docs/design/V3_LINUX_PERF_BASELINE_6692e621.md)"

[[extra.pitch.items]]
title = "Guest and host drivers need not match"
text = "A Windows guest on NVIDIA 580.88 ran against a 595.91.07 host. Linux guests from 575.57.08 to 610.57.04 passed against a 580.159.04 host, and host drivers from 570.148.08 to 595.91.07 passed with a 580.159.04 guest, on a few boxes each. Linux guests have run on Turing, Ampere, Ada and Blackwell dies, not on every card in those families. Windows with a display has run on Ada dies only (AD104, AD106, AD103). Cards older than Turing cannot work."
src = "docs/howto/requirements.md 'NVIDIA driver on the host', 'GPU'; docs/FAQ.md 'Does the guest driver have to match the host driver?'; README.md 'GPUs:' paragraph"

[extra.audiences]
heading = "Who it is for"
intro = "Four kinds of reader, each with what kayfabe promises them, what backs that up, and what to know first."
src = "see each card"

[[extra.audiences.cards]]
name = "Linux desktop users who need Windows"
promise = "A Windows VM that uses the real GPU while your Linux desktop keeps it, with no second card and no reboot."
proof = "Windows 11 with NVIDIA's 580.88 driver ran Minecraft Java and Bedrock, Unigine Heaven, GravityMark (Direct3D 12 and Vulkan) and a 10 minute 34 second 4K60 YouTube video on hardware video decode, beside the Linux desktop on the same card."
caveat = "Shown on one machine. Direct3D 11 in the production path runs at about 0.3 of native speed (table below). Sleep and wake fail. There is no installer."
src = "README.md 'What runs today' table; docs/howto/windows-guest.md"

[[extra.audiences.cards]]
name = "Windows applications that need NVIDIA's driver"
promise = "NVIDIA's own driver in the guest, so CUDA and OptiX come from NVIDIA's stack rather than an emulation of it."
proof = "`nvidia-smi` in the guest shows CUDA 13.0, and Blender 5.2 Cycles rendered with CUDA and OptiX (OptiX 5.3 s against 3.4 s native, measured 2026-10-15 on branch `claude/perf-win-20261015`)."
caveat = "SolidWorks, OBS recording and 3DMark runs are reported in the README, but their result files are not committed yet. No CAD or viewport benchmark is recorded. Not endorsed by NVIDIA or Microsoft."
src = "README.md CUDA, SolidWorks, 3DMark and OBS rows; docs/howto/credits.md Trademarks"

[[extra.audiences.cards]]
name = "CUDA and ML developers who want a real VM"
promise = "A Linux VM with its own kernel and its own driver version, on a card the host also uses."
proof = "61 of 65 CUDA applications (CUDA samples, PyTorch, Hugging Face generation, llama.cpp) gave outputs equal to bare metal, and 100 processes ran in one boot."
caveat = "Measured on a rented RTX 3060 that is itself a VM, at older revisions; the same set has not been run on the author's RTX 4070. Creating a CUDA context takes about 8 times longer. The 4 failures use unified (UVM) memory paging and fail loudly."
src = "README.md Linux guest paragraph and CUDA table; docs/design/V3_APP_MATRIX.md"

[[extra.audiences.cards]]
name = "Homelab and CI for your own code"
promise = "A GPU in a VM without giving the card to VFIO, without a licence server, and without touching your host's driver."
proof = "The rootless audit above: a throwaway user in group kvm ran the Linux suite 30 of 30 and the 9 GPU gates, with no capabilities."
caveat = "Not audited. Do not run untrusted or multi-tenant guests on a machine you cannot afford to lose; the recommendation is one guest per GPU. What a guest-triggered GPU fault does to other VMs is not measured. NVIDIA's driver licence applies to the driver you install on the host and in the guest, and hosting VMs for other people is the operator's responsibility: see [Your licences](@docs/credits.html#your-licences)."
src = "docs/howto/security.md; README.md 'Not finished'; docs/howto/credits.md 'Your licences'; branch claude/rootless-audit-20261016 commit 32d7319b2"

[extra.speed]
heading = "How fast, with the numbers"
intro = "Windows first, because it is the slow one. Unigine Heaven 4.0 at 1920x1080 windowed, Ultra, 260 second runs, Heaven's own score, Windows 11 guest, same RTX 4070. The percentages are the fps divided by the native fps."
src = "README.md Unigine Heaven table; traces/perf_status_20261017/README.md"
columns = ["", "Direct3D 11", "OpenGL"]
note = "The experimental build skips reading the guest's command buffers once a channel is set up. It is research code on branches, not on master, and only Heaven was run with it: no application matrix exists for it. The production path is what master runs. Details and limits: [`traces/perf_status_20261017/`](https://github.com/reindertpelsma/kayfabe/blob/master/traces/perf_status_20261017/README.md). Other window sizes and quality settings give other numbers. 3DMark Steel Nomad is reported at roughly 0.5 to 0.8 of native, with result files not yet committed."
note_src = "traces/perf_status_20261017/README.md sections 1 and 2; README.md 3DMark row (Steel Nomad 2050 to 2991 against about 3800 to 3960 native, so roughly 0.5 to 0.8; derived)"
[[extra.speed.rows]]
cells = ["Native Windows (VFIO, measured 2026-10-15)", "330 fps", "342 fps"]
[[extra.speed.rows]]
highlight = true
cells = ["**kayfabe, production path**", "**89 to 102 fps (27 to 31%)**", "**212 to 233 fps (62 to 68%)**"]
[[extra.speed.rows]]
cells = ["kayfabe, experimental build (unmerged, Heaven only)", "296 to 299 fps (90 to 91%)", "338 to 362 fps (99 to 106%)"]

[extra.speed.linux]
intro = "Then Linux guests running CUDA, on a rented RTX 3060 that is itself a VM, at older revisions (host and guest on the same box):"
src = "README.md Linux CUDA table; docs/design/V3_LINUX_PERF_BASELINE_6692e621.md"
columns = ["Workload", "Guest compared with host"]
note = "The cost is in many small launches: eager PyTorch decoding runs at 0.29 of host speed, creating a CUDA context takes about 8 times longer, and NVENC 1080p reached 0.72. The same set on the author's RTX 4070 without nesting has not been run."
[[extra.speed.linux.rows]]
cells = ["Matrix multiply, N=2048", "0.997 (784 against 786 GFLOPS)"]
[[extra.speed.linux.rows]]
cells = ["llama.cpp Qwen2.5-1.5B Q4, prompt processing", "0.998"]
[[extra.speed.linux.rows]]
cells = ["llama.cpp, token generation", "0.877"]
[[extra.speed.linux.rows]]
cells = ["Device-to-device copy, 32 MB", "0.98"]
[[extra.speed.linux.rows]]
cells = ["App matrix, 65 apps, total time", "1.11 times the host's time; 61 pass"]

[extra.compare]
heading = "How it differs from other approaches"
intro = "What each approach costs you, using each project's own documentation where there is one."
src = "README.md comparison table; docs/FAQ.md; docs/PROJECT_HISTORY.md section 3"
columns = ["", "What it costs you", "Source"]
[[extra.compare.rows]]
cells = ["VFIO passthrough", "One VM owns the card; the host does not keep it while the VM runs. The guest driver is stock. This is also the native reference for the speed tables above.", "README comparison table"]
[[extra.compare.rows]]
cells = ["NVIDIA vGPU", "The VMs share the card but the host does not use it; it needs a data-centre GPU and a licence.", "README comparison table"]
[[extra.compare.rows]]
cells = ["API forwarding ([nvkvm-pv](https://github.com/reindertpelsma/nvkvm-pv) / virtio-nvgpu)", "The host keeps the card, but the guest needs a custom kernel module, the guest is Linux, and guest and host driver versions must match. If you want something that works today, use nvkvm-pv.", "[FAQ](@docs/FAQ.html)"]
[[extra.compare.rows]]
cells = ["Containers with gVisor's nvproxy", "Not a VM. gVisor's documentation says it \"doesn't introduce any additional hardware-level isolation beyond that which is configured by the NVIDIA kernel-mode driver\". Kayfabe also reaches the host driver as an ordinary client; what it adds is a VM boundary and authored calls, which are not audited.", "[gVisor GPU guide](https://gvisor.dev/docs/user_guide/gpu/); [security model](@docs/security.html)"]
[[extra.compare.rows]]
cells = ["virtio-gpu (Venus, virgl)", "A generic graphics path, not NVIDIA's own driver in the guest. The repository has not compared the two in detail.", "[FAQ](@docs/FAQ.html)"]
[[extra.compare.rows]]
highlight = true
cells = ["**kayfabe**", "**The host keeps the card; the guest runs NVIDIA's stock driver, Windows or Linux; no licence server. In return: Turing or newer, the open NVIDIA host kernel module, a Linux x86-64 host with KVM, QEMU only, no installer, not audited.**", "[Requirements](@docs/requirements.html)"]

[extra.today]
heading = "What works today"
src = "README.md 'What runs today' and 'Not finished'; docs/howto/index.md; docs/howto/requirements.md; docs/howto/goals.md"
intro = "Three groups. \"Works today\" means a run on named hardware at a named revision; \"Partly\" says what is missing; \"Not yet\" is not built or not shown. Each manual page repeats the evidence for what it describes."
columns = ["", "What", "Evidence and limits"]
[[extra.today.rows]]
status = "works"
what = "**Windows 11 guest on NVIDIA's stock 580.88 driver, while the host keeps the card.** Desktop, Minecraft Java and Bedrock, Unigine Heaven (Direct3D 9, 11, OpenGL), GravityMark (Direct3D 12, Vulkan), Edge with YouTube 4K60 on hardware video decode, CUDA, Blender Cycles (CUDA, OptiX)."
evidence = "The author's RTX 4070 (AD104), host driver 595.91.07. Matrix at revision `eb9f2aa3`, latest session `89fc8a222`. Three registry settings are required: [Windows guest](@docs/windows-guest.html)."
src = "README.md 'What runs today' Windows table; traces/win11_userring_20261014/matrix-eb9f2aa3-20261014-2321.md; docs/howto/requirements.md"
[[extra.today.rows]]
status = "works"
what = "**Linux guest on the stock driver, nothing to set.** A 30-arm test suite, 9 of 9 GPU gates, a desktop with display planes, 3 of 3 suspend and resume cycles, 61 of 65 CUDA applications with outputs equal to bare metal."
evidence = "Rented RTX 3060 (nested) for the CUDA set, older revisions; see [Linux guest](@docs/linux-guest.html)."
src = "README.md Linux guest paragraph; docs/design/V3_APP_MATRIX.md"
[[extra.today.rows]]
status = "works"
what = "**Different guest and host driver versions**, and Linux guests on Turing, Ampere, Ada and Blackwell dies."
evidence = "Dies run: TU116, GA106, GA104, AD104, AD106, AD102, GB205 (Linux). Per-die table: [Requirements](@docs/requirements.html)."
src = "docs/howto/requirements.md 'GPU' and 'Guest'; README.md 'GPUs:' paragraph"
[[extra.today.rows]]
status = "partly"
what = "**Windows speed.** It runs, and it is slower than native in the production path."
evidence = "Heaven at 27 to 31% (Direct3D 11) and 62 to 68% (OpenGL) of native; the faster experimental build is not merged. See the table above."
src = "traces/perf_status_20261017/README.md section 1; README.md Unigine Heaven table"
[[extra.today.rows]]
status = "partly"
what = "**Windows beyond the author's card.** Windows desktops have run on AD106 and AD103 with a borrowed display page; an Ampere die (GA104) gets the GPU but no display; one Blackwell die (GB206) fails with Code 43."
evidence = "Only NVIDIA's 580.88 Windows driver has been run. [Requirements](@docs/requirements.html)."
src = "docs/howto/requirements.md 'GPU' table; README.md 'GPUs:' paragraph"
[[extra.today.rows]]
status = "partly"
what = "**Running without root.** The host side ran as an ordinary user in group kvm."
evidence = "One host, one revision, record on a branch. The shipped launch scripts still run as root, and `/dev/kvm` access is needed."
src = "branch claude/rootless-audit-20261016 commit 32d7319b2, traces/rootless_audit_20261016/REPORT.txt; docs/howto/security.md"
[[extra.today.rows]]
status = "partly"
what = "**Professional and creative apps.** SolidWorks 2026 (student edition), OBS recording with NVENC, 3DMark and Office are reported in the README."
evidence = "Reported, not proven: the result files are not committed yet."
src = "README.md SolidWorks, OBS, 3DMark and Office rows ('evidence not yet committed')"
[[extra.today.rows]]
status = "no"
what = "**An installer, a public binary, a stable interface.** You build from source. An installer is planned, with no date."
evidence = "[Building from source](@docs/building.html), [Goals and plans](@docs/goals.html). A QEMU binary built from this tree is not yet distributable under the GPL."
src = "README.md 'Trying it' and status line; docs/howto/goals.md 'An installer and a stable interface'; docs/howto/credits.md (LICENSE exception 2)"
[[extra.today.rows]]
status = "no"
what = "**Windows sleep and wake**, and a Windows session that never resets the driver."
evidence = "Edge can reset the driver after 7 to 16 minutes of playback. [Windows guest](@docs/windows-guest.html)."
src = "README.md 'Not finished'; docs/WINDOWS_GUEST_SETUP.md"
[[extra.today.rows]]
status = "no"
what = "**A security audit**, and several VMs sharing one card with measured throughput, fairness or isolation."
evidence = "A Linux and a Windows guest have booted together; nothing more is measured. What a guest-triggered GPU fault does to other VMs is not measured."
src = "traces/windows_code43_walls_20261007/README.md run54 (a88764b3, 2026-10-08); README.md 'Not finished'; docs/howto/linux-guest.md"
[[extra.today.rows]]
status = "no"
what = "**Proxmox or libvirt, other hypervisors than QEMU, non-Linux hosts, arm64.**"
evidence = "Goals only: [Goals and plans](@docs/goals.html)."
src = "docs/howto/goals.md; docs/howto/requirements.md"
[[extra.today.rows]]
status = "no"
what = "**GPUs older than Turing** (they cannot work), and Hopper or data-centre Blackwell (never run on hardware)."
evidence = "[Requirements](@docs/requirements.html)."
src = "docs/howto/requirements.md 'GPU'; docs/STATUS_DETAIL.md section 0"

[extra.games]
text = "Games: Minecraft runs, and so do a few benchmarks. Games with kernel anti-cheat are not a target. We have no results for them, and the project's own research notes that products such as Vanguard and FACEIT detect and refuse virtual machines."
src = "README.md 'What runs today'; docs/design/V3_VIOMMU_NEED_RESEARCH_20261013.md summary item 3"

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

[extra.honest]
heading = "How it stays honest"
src = "AGENTS.md 'Working rules' and 'Verification'; docs/howto/index.md 'How to read the evidence labels'; docs/PROJECT_HISTORY.md section 5; .github/workflows/ci.yml claim-ledger gate"
items = [
  "Every number names its hardware and revision, or says it has none. The manual marks each statement as measured, inferred or **not documented yet**.",
  "Claims the README carried and later had to withdraw are kept in a public corrections log in the repository (`docs/PROJECT_HISTORY.md`), not deleted.",
  "A check in the project's CI fails a change that adds a \"measured\" claim without naming what is behind it.",
  "The evidence (traces, logs, tables) is in the repository, and the tests that need no GPU run in CI.",
  "Attack the design: [the security model](@docs/security.html) lists what is trusted, what is not, and what is unfinished, and the [issue tracker](https://github.com/reindertpelsma/kayfabe/issues) takes bug reports and corrections.",
]

[extra.start]
heading = "Try it, or help"
src = "README.md 'Trying it'; docs/howto/building.md; docs/howto/installing.md; docs/howto/goals.md"
text = "There is no installer today; an installer is planned, with no date. You can build from source on a Linux x86-64 host with KVM and an NVIDIA card from Turing on; `cargo test` runs without a GPU."
command = "cargo build --workspace --release"
after = "That builds the Rust workspace. The QEMU binary with the kayfabe device is a second step; read the pages below first, because the repository's provisioning scripts are for throwaway machines."

[extra.community]
heading = "Contribute and community"
src = "docs/howto/credits.md; docs/howto/security.md 'Reporting a vulnerability'"
licence = "Dual-licensed Apache-2.0 OR GPL-2.0-or-later. Struct layouts, class ids and register offsets are generated from NVIDIA's open-source GPU kernel modules (MIT). Kayfabe ships no NVIDIA or Microsoft binaries. Windows must be licensed for the way you use it."
licence_src = "docs/howto/credits.md 'Licence', 'Third-party notices', 'No vendor binaries', 'Your licences'"
+++