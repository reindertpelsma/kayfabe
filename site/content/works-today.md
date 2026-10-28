+++
title = "What works today"
description = "Real programs run. Coverage still depends on the guest, driver and GPU."
template = "detail.html"
[extra]
src = "README.md What runs today; docs/howto/requirements.md GPU table"
+++

## Windows and Linux

<!-- src: traces/win11_userring_20261014/matrix-eb9f2aa3-20261014-2321.md; docs/howto/windows-guest.md -->
Windows 11 was tested in a matrix on 14 October 2026, at revision `eb9f2aa3`, on the author's RTX 4070 with NVIDIA's 580.88 driver. Seven of nine rows passed: Unigine Heaven in Direct3D 11 and OpenGL, windowed and fullscreen, GravityMark in Direct3D 12 and Vulkan, and Minecraft. A pass means the program finished with no host Xid and no GPU reset. Edge video failed after about 7.3 minutes, and the idle row was not run.

<!-- src: README.md Windows table (CUDA, Blender and Edge rows); docs/howto/windows-guest.md -->
The README also reports CUDA in the guest, Blender renders with CUDA and OptiX, and one 4K YouTube playback in Edge that ran to the end. Those are separate observations, not part of that matrix. No all-app pass exists. Only Windows driver 580.88 is covered, and it needs three registry settings, listed in the [Windows guide](/@docs/windows-guest.html).

<!-- src: README.md Linux guest; docs/howto/linux-guest.md -->
Linux guests have run a full desktop, display planes, suspend and resume, and a 30-test suite. On a rented RTX 3060, 61 of 65 CUDA programs gave the same output as the host. These include PyTorch and llama.cpp. The four failures need memory paging that is not supported yet. One failure printed SUCCESS with a wrong result.

<!-- src: README.md screenshot; traces/20261021_131847.jpg -->
![Windows 11 and a Linux desktop using one RTX 4070](/screenshot-windows-desktop.jpg)

The screenshot shows Windows 11 on the right, with Minecraft Bedrock, Unigine Heaven, a YouTube video in Edge and nvidia-smi. Linux is on the left, with Blender, Firefox and nvidia-smi. Both screens plug into the same RTX 4070. One GPU serves every app in the picture.

## Progress by GPU

<!-- src: docs/howto/requirements.md GPU table; README.md GPUs; traces/sw3_die_sweep_20261024/README.md -->
These are recorded stages, not a promise that every card or application works. A passed Linux test does not imply a Windows desktop. See [Requirements](/@docs/requirements.html#gpu) for revisions and driver coverage.

| Family / die | Linux evidence | Windows evidence |
|---|---|---|
| Turing TU116 | Guest tests passed | Not documented yet |
| Ampere GA106 (RTX 3060) | CUDA application tests and desktop | On a branch: Code 0, then repeated driver resets |
| Ampere GA104 | GPU gates and guest tests passed | GPU enumeration only; no display or application result |
| Ada AD104 (RTX 4070) | Guest tests and desktop | Desktop and applications |
| Ada AD104 (RTX 4000 Ada) | Guest tests passed | Not documented yet |
| Ada AD106, AD103 | Passed on AD106; AD103 not documented | Desktops with a borrowed display page |
| Ada AD102 | Guest driver 590 passed | On a branch: 13 of 18 recorded rows passed, 4 failed, 1 not run |
| Blackwell GB205 | Guest tests passed | One idle ten-minute pass with unmerged fixes; no apps tested |
| Blackwell GB202 | On a branch: guest tests and CUDA passed | Open: a crash on one machine was not repeated on another |
| Blackwell GB206 | Passed on earlier revisions | Open: Code 43 |

<!-- src: docs/howto/requirements.md GPU table (2026-10-28); branches claude/blackwell-fixes-20261024, codex/bw6b-20261024, claude/bw11-20261026, codex/sweep-dies-20261022, claude/tur1-20261027 and codex/sweep-integrate-20261018 -->
Rows marked "On a branch" come from records on branches that are not merged. Each rests on one machine. The [requirements table](/@docs/requirements.html#gpu) links the records.
The GB205 Windows pass used two fixes that are not merged yet. Without them, the driver crashes the guest. GB202 is still open; the different outcomes do not explain the crash.

<!-- src: docs/howto/requirements.md GPU notes; docs/STATUS_DETAIL.md sections 0 and 1 -->
Earlier Linux runs also passed on GA102, GB203 and an RTX 3060 Ti. Cards older than Turing cannot use this design. Hopper and data-centre Blackwell have not been run on hardware. GA100 and GB10B are refused by name.

## What is rough

<!-- src: README.md 'Not finished'; docs/howto/windows-guest.md -->
Windows is not stable. Sleep and wake do not survive, and Edge can make the driver reset after 7 to 16 minutes of video.

<!-- src: README.md SolidWorks, OBS and 3DMark rows ('evidence not yet committed') -->
SolidWorks, OBS recording and 3DMark runs are reported in the README. Their result files are not committed yet, so treat them as claims.

<!-- src: branch claude/rootless-audit-20261016 commit 32d7319b2, traces/rootless_audit_20261016/REPORT.txt; docs/howto/security.md -->
One audit on one machine ran the Linux suite and a Windows benchmark as a normal user, without root. The record is on a branch. The launch scripts still start as root. See the [security model](/@docs/security.html).

## Sharing and unfinished work

<!-- src: docs/howto/technical-faq.md 'What do the gates and 30 arms mean?' (one guest boot per arm); docs/howto/security.md (one guest per GPU); branch claude/rootless-audit-20261016 (no VFIO rebind, no driver change) -->
The test suite boots 30 short-lived VMs in a row, one for each test. There is no GPU to rebind between runs. We recommend one VM per card at a time.

<!-- src: README.md 'Not finished'; traces/windows_code43_walls_20261007/README.md run54 (a88764b3, 2026-10-08); docs/FAQ.md 'Can several VMs share one GPU?' -->
There is no security audit. Two VMs have booted together on one card, but sharing performance is untested. Nobody has measured what one VM's GPU fault does to the other VMs. Read the [sharing FAQ](/@docs/FAQ.html#can-several-vms-share-one-gpu).

<!-- src: README.md 'Trying it' and STATUS line; docs/howto/goals.md 'An installer and a stable interface'; docs/howto/credits.md (LICENSE exception 2) -->
There is no installer, public binary or stable interface. Build from source. An installer is planned, with no date.

<!-- src: docs/howto/goals.md; docs/howto/requirements.md -->
Only QEMU on Linux x86-64 is supported today. Proxmox and libvirt integration do not exist yet. Other hypervisors and host platforms are goals, not current features.

<!-- src: docs/design/V3_VIOMMU_NEED_RESEARCH_20261013.md summary item 3; docs/FAQ.md 'Do games with anti-cheat work?' (owner direction 2026-10-28) -->
No game with kernel anti-cheat is recorded. Products such as Vanguard and FACEIT refuse VMs. Working around that is a non-goal. See the [anti-cheat FAQ](/@docs/FAQ.html#do-games-with-anti-cheat-work).

## Not measured yet

<!-- src: docs/FAQ.md 'Can several VMs share one GPU?', 'Why a display broker instead of QEMU's own window?', 'Do I need root, or a patched host?', 'Can kayfabe run inside a container?'; docs/howto/goals.md; docs/howto/troubleshooting.md; traces/perf_status_20261017/README.md historical banner -->
This is the one list of things the project has not measured. Other pages link here instead of repeating it.

- Windows speed on current master. A new benchmark is pending, so no figure is published.
- Several VMs on one card: throughput, fairness, and what one VM's GPU fault does to the others.
- The display path: frame rate and latency compared with other approaches, and PCIe traffic itself.
- Running as an ordinary user on current master, including the CUDA application matrix and a desktop on the NVIDIA card.
- Running inside a container.
- CUDA, unified memory and peer access in a guest with two GPUs, and several GPUs with Windows or the display.
- NVIDIA MIG partitions. No MIG hardware has been available.
- How close real guests come to the limit of 8192 live placements per CPU window.
- Hosts older than 570, and the closed NVIDIA host module.
- Any speed or security comparison with other projects.

## Build and try it

<!-- src: README.md Trying it; docs/howto/building.md; docs/howto/installing.md -->
Build both Rust and QEMU from source. This command builds the Rust part:

```sh
cargo build --workspace --release
```

The QEMU binary with the kayfabe device is a separate step. The [build guide](/@docs/building.html) and [running guide](/@docs/installing.html) list it. Read them before using the scripts: those scripts target throwaway test machines. `cargo test` runs without a GPU.

<!-- src: README.md STATUS line -->
This is a research project with one maintainer. It is not ready for your daily machine. [Goals and plans](/@docs/goals.html) cover the work ahead.
