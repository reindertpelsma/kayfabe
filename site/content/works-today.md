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
The README also reports CUDA in the guest, Blender renders with CUDA and OptiX, and one 4K YouTube playback in Edge that ran to the end. Those are separate observations, not part of that matrix. No all-app pass exists. Only Windows driver 580.88 is covered, and it needs [three registry settings](/@docs/windows-guest.html).

<!-- src: README.md Linux guest; docs/howto/linux-guest.md -->
Linux guests have run a full desktop, display planes, suspend and resume, and a 30-test suite. MEASURED on an RTX 4090 at `144557e43`, 2026-11-07: 70/70 runnable apps passed on the host and 66/70 in the guest. Passing apps met their own correctness predicates; the three digest-producing apps matched host digests. The four UVM apps are explicitly unsupported for now, listed in every report. <!-- src: traces/apps_linux_20261107/README.md; docs/OWNER_RULINGS.md section E -->

<!-- src: README.md screenshot; traces/20261021_131847.jpg -->
![Windows 11 and a Linux desktop using one RTX 4070](/screenshot-windows-desktop.jpg)

The screenshot shows Windows 11 on the right, with Minecraft Bedrock, Unigine Heaven, a YouTube video in Edge and nvidia-smi. Linux is on the left, with Blender, Firefox and nvidia-smi. Both screens plug into the same RTX 4070. One GPU serves every app in the picture.

## Progress by GPU

<!-- src: docs/howto/requirements.md GPU table; README.md GPUs; traces/sw3_die_sweep_20261024/README.md -->
These are recorded stages, not a promise that every card or application works. A passed Linux test does not imply a Windows desktop. The [requirements](/@docs/requirements.html#gpu) list revisions and driver coverage.

| Family / die | Linux evidence | Windows evidence |
|---|---|---|
| Turing TU116 | Guest tests passed | MEASURED: Windows gate PASS, `0c515b3b3`, 2026-11-04; stock 580.88. This is a gate result, not full application qualification |
| Ampere GA106 (RTX 3060) | CUDA application tests and desktop | MEASURED: Windows gate PASS, `0c515b3b3`, 2026-11-04; stock 580.88. This is a gate result, not full application qualification |
| Ampere GA104 | GPU gates and guest tests passed | MEASURED: Windows gate PASS, `0c515b3b3`, 2026-11-04; stock 580.88. This is a gate result, not full application qualification |
| Ada AD104 (RTX 4070) | Guest tests and desktop | MEASURED: Windows gate PASS, `0c515b3b3`, 2026-11-04; stock 580.88. This is a gate result, not full application qualification |
| Ada AD104 (RTX 4000 Ada) | Guest tests passed | MEASURED: Windows gate PASS, `0c515b3b3`, 2026-11-04; stock 580.88. This is a gate result, not full application qualification |
| Ada AD106, AD103 | Passed on AD106; AD103 not documented | Desktops with a borrowed display page |
| Ada AD102 | yes; 30/30 at `0c515b3b3` (580.159.04 guest), 2026-11-04 | MEASURED: Windows gate PASS, `0c515b3b3`, 2026-11-04; stock 580.88. This is a gate result, not full application qualification |
| Blackwell GB205 | Guest tests passed | MEASURED: Windows gate PASS, `0c515b3b3`, 2026-11-04; stock 580.88. This is a gate result, not full application qualification |
| Blackwell GB202 | On a branch: guest tests and CUDA passed | MEASURED: Windows gate intermittent: FAIL, then two PASS on one box, `0c515b3b3`, 2026-11-04; stock 580.88. This is a gate result, not full application qualification |
| Blackwell GB206 | Passed on earlier revisions | MEASURED: Windows gate PASS, `0c515b3b3`, 2026-11-04; stock 580.88. This is a gate result, not full application qualification |

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
The launch scripts still use root; a [run without sudo](/@docs/security.html) is recorded on one host.

## Sharing and unfinished work

<!-- src: docs/howto/technical-faq.md 'What do the gates and 30 arms mean?' (one guest boot per arm); docs/howto/security.md (one guest per GPU); branch claude/rootless-audit-20261016 (no VFIO rebind, no driver change) -->
The test suite boots 30 short-lived VMs in a row, one for each test. There is no GPU to rebind between runs. We recommend one VM per card at a time.

<!-- src: README.md 'Not finished'; traces/windows_code43_walls_20261007/README.md run54 (a88764b3, 2026-10-08); docs/FAQ.md 'Can several VMs share one GPU?' -->
There is no security audit. Two runs on one RTX 4090 recorded a host CUDA loop and two guests with correct results, no GPU resets and per-workload ratios beside the suite's fault arms (`3aa2a75a6`, 2026-11-05). Hostile long kernels and general fault containment remain unverified. <!-- src: traces/trio_vast_20261105/README.md --> The [sharing FAQ](/@docs/FAQ.html#can-several-vms-share-one-gpu) describes the limits.

<!-- src: README.md 'Trying it' and STATUS line; docs/howto/goals.md 'An installer and a stable interface'; docs/howto/credits.md (LICENSE exception 2) -->
There is no installer, public binary or stable interface. Build from source. An installer is planned, with no date.

<!-- src: docs/howto/goals.md; docs/howto/requirements.md -->
Only QEMU on Linux x86-64 is supported today. Proxmox and libvirt integration do not exist yet. Other hypervisors and host platforms are goals, not current features.

<!-- src: docs/design/V3_VIOMMU_NEED_RESEARCH_20261013.md summary item 3; docs/FAQ.md 'Do games with anti-cheat work?' (owner direction 2026-10-28) -->
No game with kernel anti-cheat is recorded. Products such as Vanguard and FACEIT refuse VMs. Working around that is a [non-goal](/@docs/FAQ.html#do-games-with-anti-cheat-work).

## Not measured yet

<!-- src: docs/FAQ.md 'Can several VMs share one GPU?', 'Why a display broker instead of QEMU's own window?', 'Do I need root, or a patched host?', 'Can kayfabe run inside a container?'; docs/howto/goals.md; docs/howto/troubleshooting.md; traces/perf_status_20261017/README.md historical banner -->
This is the one list of things the project has not measured. Other pages link here instead of repeating it.

- Windows speed on current master. A new benchmark is pending, so no figure is published.
- Several VMs on one card: general fairness and containment during hostile long kernels or faults remain unverified beyond the limited two-run trio result above.
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
