# Requirements

This page lists only what the repository's evidence states. Where the repository gives no answer, the line says **not documented yet**. Nothing here is a promise of support: kayfabe is a research project. <!-- src: README.md -->

## Host

### Operating system and CPU

- **Linux on x86-64 with KVM** (the host OS is Linux only).  The device is a QEMU device (only QEMU is supported today; independence from one hypervisor is a goal, see [Goals and plans](goals.md)), and every recorded run used `/dev/kvm` with `-cpu host`. <!-- src: scripts/fastguest/run_fast_guest.sh, scripts/bench/host_preflight.sh, docs/design/THE_ARCHITECTURE_v3.md section 0 ("Host OS is Linux only") -->
- The CPU needs hardware virtualisation (`vmx` or `svm`). <!-- src: scripts/bench/host_preflight.sh -->
- Hosts that are themselves KVM guests (nested virtualisation) work, and most hardware results come from such rented hosts. A VM exit costs about 10 to 40 times as much on them as on bare metal. <!-- src: docs/STATUS_DETAIL.md section 4 and AGENTS.md "Bench traps" --> Results on non-nested hardware: the author's RTX 4070 host runs the Windows results in the README. <!-- src: README.md -->
- Intel and AMD (EPYC 7452) hosts appear in the recorded results. <!-- src: docs/design/V3_APP_MATRIX.md, docs/STATUS_DETAIL.md -->
- **Host kernel version:** the recorded rented hosts ran Linux 6.8.0-59. A minimum or maximum supported kernel is **not documented yet**. <!-- src: docs/design/V3_DRIVER_MATRIX.md (SW3 GA104 cell) -->
- **arm64:** no run on arm64 is documented. CI only cross-checks that the workspace builds for aarch64. <!-- src: .github/workflows/ci.yml (aarch64 job) -->
- **An IOMMU is not required** for kayfabe's isolation: the guest never drives the physical GPU. See [FAQ](../FAQ.md). <!-- src: docs/FAQ.md -->

### NVIDIA driver on the host

- **The open NVIDIA kernel module** (`-m=kernel-open`) is what every recorded run used, and the provisioning script refuses a host whose `/proc/driver/nvidia/version` does not say "Open Kernel Module". <!-- src: scripts/bench/provision_host_driver.sh, scripts/bench/provision_bench_tree.sh -->
- Closed (proprietary) host modules: supported in principle, untested. <!-- src: docs/design/V3_SECURITY_MODEL.md section 10 -->
- **Host driver versions that passed** the GPU gates (9/9), the 30-arm thin-guest suite and the CUDA ladder with a 580.159.04 guest: 570.148.08 (kayfabe `1f3e4fc4`), 575.57.08, 580.65.06 and 580.95.05 (`ee35ca4a`) and 580.159.04 (`1837166d`) [measured, GA102 boxes, September 2026; nested rented hosts]. <!-- src: docs/design/V3_DRIVER_MATRIX.md section 6.0 (host x guest grid) -->
- **595.91.07 (open)** on the host and in the guest: gates 9/9 plus gate 10, bare metal 30/30 and thin guest 30/30 on one GA104 box [measured, kayfabe `714ec29d3`, 2026-10-24, n=1]. The author's RTX 4070 host runs the same driver. <!-- src: traces/sw3_die_sweep_20261024/README.md, docs/FAQ.md -->
- Host drivers 565.57.01 and 550.54.14 were not swapped successfully in the recorded walk. The 545.x host attempt was blocked because its kernel module did not build on Linux 6.8 [code reading: `docs/design/V3_DRIVER_MATRIX.md` section 7, host-axis table]. No 535 host hardware result is recorded. Treat hosts older than 570 as untested. <!-- src: docs/design/V3_DRIVER_MATRIX.md sections 6.0 and 7 -->
- The host driver version is **measured** by kayfabe from the host, never taken from a guest. An unmeasured host driver is refused by name. <!-- src: docs/design/V3_DRIVER_MATRIX.md (2026-09-30 check) -->
- `libcuda` (the CUDA driver library) must be loadable on the host: kayfabe opens it with `dlopen` to run its page-table walker on the GPU. The walker's PTX is committed in the repository, so no CUDA toolkit is needed to build. <!-- src: ARCHITECTURE.md (kf-cuda row), cuda/walk -->

### GPU

Kayfabe reads the host GPU's facts from the host driver. Marketing names do not decide the die: the RM architecture and implementation numbers do. <!-- src: traces/sw3_die_sweep_20261024/README.md -->

What has been run (the README's list; revisions and boxes are in the repository's status records):

| Family / die | Linux guest | Windows guest |
|---|---|---|
| Ada AD104 (RTX 4070) | yes; the author's GPU | yes; the author's GPU |
| Ada AD106, AD103 | yes (AD106) | desktops run, with a borrowed display page |
| Ada AD102 | yes, guest driver 590 | branch-only, overall FAIL with partial coverage: Code 0 and a 600 s idle hold; matrix 13 PASS, 4 FAIL, 1 NOT RUN, 94 of 112 planned rows not run [measured in the S4 receipt, revision `170054339`, lease 57193877, host 595.91.07, guest 580.88, n=1]. [Record](https://github.com/reindertpelsma/kayfabe/blob/94d58c6da/traces/sweep_dies_20261018/lease_57193877/ADA_RESULT.md) on `codex/sweep-dies-20261022`, not merged <!-- src: branch codex/sweep-dies-20261022 commit 94d58c6da --> |
| Ada AD104 (RTX 4000 Ada) | yes | not documented yet |
| Ampere GA106 (RTX 3060) | yes; the usual bench GPU | branch-only: Code 0 and some early CUDA and D3D probes pass, followed by repeated driver resets [measured in the TUR1 report, lease 57908255, host 595.91.07, guest 580.88, one host]. [Report](https://github.com/reindertpelsma/kayfabe/blob/0b44315ef/traces/tur1_20261027/README.md#control-the-same-windows-flow-on-ampere-ga106-rtx-3060) on `claude/tur1-20261027`, not merged; no Windows stability pass <!-- src: branch claude/tur1-20261027 commit 0b44315ef --> |
| Ampere GA104 | yes (exact cell: kayfabe `714ec29d3`, 595.91.07) | the README records GPU enumeration only. Historical A4000 logs record 12 GSP unload transitions and unclean stops [code reading: revision `6c6bb92b`, `traces/die_sweep_20261014/ga104_a4000/counts.txt` and `run_marker_flags.txt`]. No accelerated-display or application acceptance follows from those records <!-- src: README.md "GPUs:" paragraph; traces/die_sweep_20261014/ga104_a4000/ --> |
| Turing TU116 | yes | not documented yet |
| Blackwell GB205 | yes | an idle ten-minute run passed once, with two fixes that are not merged yet; no apps run; without the fixes the driver crashes the guest <!-- src: branch claude/blackwell-fixes-20261024, one run (n=1) --> |
| Blackwell GB202 | branch-only: thin 30/30 and CUDA 4/4 with guest and host 595.91.07 [measured at revision `24906bf33`, lease 56788212, n=1]. The [record](https://github.com/reindertpelsma/kayfabe/blob/0bbf3c55f/traces/sweep_integrate_20261018/phase2_blackwell_56788212/B3_RESULT.md) on `codex/sweep-integrate-20261018` is not merged. Heartbeat warnings are retained, and a desktop attempt with guest 580.159.04 showed a black frame | **open:** one run crashed within seconds, not yet explained; a run on another machine did not repeat it <!-- src: branches codex/bw6b-20261024 and claude/bw11-20261026 --> |
| Blackwell GB206 | passed at earlier revisions | **open: Code 43** |

<!-- src: README.md "GPUs:" paragraph; docs/STATUS_DETAIL.md section 0; traces/sw3_die_sweep_20261024/README.md -->

- Earlier revisions also passed Linux on GA102, GB203 and a floor-swept RTX 3060 Ti. <!-- src: docs/STATUS_DETAIL.md section 0 and section 1 -->
- **Hopper, GA100 and datacentre Blackwell (GB10x)** are derived from the open driver's source only; no hardware was available. GA100 and GB10B are refused by name. <!-- src: docs/STATUS_DETAIL.md section 0 -->
- **Pre-Turing GPUs cannot work.** Below Turing there is no GSP firmware interface for kayfabe to impersonate: the guest driver would drive the hardware directly. <!-- src: docs/design/THE_ARCHITECTURE_v3.md section 0.0.1 -->
- Consumer GPUs are the tested class; "any NVIDIA GPU" is the goal, not a claim. <!-- src: README.md table -->
- The table above is a map of historical and branch-only results, with those scopes stated. A larger inventory of recorded family evidence is in the internal design document `docs/design/V3_FAMILY_COVERAGE.md` (merged 2026-10-28). Neither table establishes that every die passes on current master. <!-- src: docs/design/V3_FAMILY_COVERAGE.md -->

### Memory and apertures

- **Video memory:** the guest's video memory is a real allocation of host VRAM, sized by the device property `fb-mb` (default 8192 MiB). On a card with less than 10 GiB the bench harness sets it to (total VRAM minus 2048 MiB) rounded down to whole GiB, and the harness script records that 6144 MiB boots on an 8 GiB card. A store the card cannot hold is refused at start with `store of N MiB refused: NoMemory`. <!-- src: qemu/hw/misc/kf3/kf3.c, scripts/fastguest/run_fast_guest.sh, crates/kf-qemu/src/cardbudget.rs -->
- **Host BAR1:** the sum, over the kayfabe devices on one card, of guest BAR1 + guest BAR2 + 1 MiB (PRAMIN window) + 16 MiB of headroom must not exceed the host's BAR1 aperture. With the common 256 MiB host aperture, a guest BAR1 of 256 MiB does not fit and the harness uses 128 MiB. The device refuses at start, by name, a configuration that does not fit. A larger, resizable host BAR1 is not required for that reduced guest aperture; the required host aperture follows from the stated budget [code reading: `crates/kf-qemu/src/cardbudget.rs`, `scripts/fastguest/run_fast_guest.sh`]. <!-- src: crates/kf-qemu/src/cardbudget.rs, scripts/fastguest/run_fast_guest.sh -->
- **Guest RAM** should be a shared memfd (`memory-backend-memfd,share=on`), which is what every launcher in the repo passes and what the tested configuration uses. With or without one, the host GPU maps the guest's system memory from one host object per RAM block over the VMM's own mapping, one map per run (this changed 2026-10-31 and is untested on a GPU; `KF3_STITCHED_MAP=1` selects the previous path, one object over the whole memfd with stitched batches, as a measurement arm, and needs a memfd). With ordinary anonymous RAM the device still realises. The paths that need a file offset still refuse by name without a memfd and are GPU-side, not CPU accesses: a system-memory USERD or error notifier handed to the host driver, the T-space RAM window, PRAMIN pointed at system memory. CPU reads of guest memory (a Translated channel's, the USERD relay's) work without a memfd. The file-less configuration has been exercised by unit tests and a model of the host only, not on a GPU. <!-- src: scripts/fastguest/run_fast_guest.sh, crates/kf-qemu/src/mem.rs, docs/design/V3_GUEST_MEMORY.md -->
- **Host RAM and disk:** the bench's own host checks demand at least 16 GB RAM, 8 cores and 100 GB free disk, but those are limits for a build-and-test box, not product requirements. The harness defaults for a Windows guest are 8192 MB and 8 vCPUs, with an 80 GB disk. A product minimum is **not documented yet**. <!-- src: scripts/bench/host_preflight.sh, scripts/bench/windows/win_vm.sh -->

- **CPU-window admission:** each CPU window permits at most 8192 live guest placements across RAM and store, plus its separate boot seed. Further placements are refused by name; successful unmap returns capacity. <!-- src: crates/kf-mem/src/cpuwin.rs, commit a61ed3c6e (default placement budget) -->

### Privileges

- The design runs the host side from an unprivileged process using only resource-manager controls the host driver marks non-privileged. <!-- src: ARCHITECTURE.md, README.md -->
- The provided launch scripts, however, run as root: they open `/dev/nvidia*`, create a tap network and take a lock. <!-- src: scripts/bench/display/interactive.sh -->
- **One run without root is recorded**, on one host: a user whose only group was `kvm` (no capabilities) ran the 30-arm Linux suite 30 of 30, the GPU gates 9 of 9 and a Windows 11 guest running Unigine Heaven, with no VFIO rebind and no kernel module, driver or udev change [measured, kayfabe `0a746117`, RTX 4070, driver 595.91.07]. It relied on NVIDIA's default mode 0666 for `/dev/nvidia*`, and the user needed permission to open `/dev/kvm`. The record is on the branch `claude/rootless-audit-20261016` (`traces/rootless_audit_20261016/REPORT.txt`), not on master; a repeatable test on master is still to be added. See [Security model](security.md). <!-- src: branch claude/rootless-audit-20261016 commit 32d7319b2, traces/rootless_audit_20261016/REPORT.txt -->
- **No host patches are required.** An optional patch to the host's `nvidia-uvm` exists as research; it is not part of the stock path. <!-- src: docs/STATUS_DETAIL.md section 0 --> Four of the 65 CUDA applications need unified-memory demand paging and fail without it. Optional enhancements (a guest helper module, patched guest or host drivers) are planned and never required. <!-- src: README.md; docs/design/V3_COOPERATIVE_TIERS.md section 1, docs/design/V3_HOST_PATCH_LIST.md section 1 -->

## Build host

See [Building from source](building.md) for the package list, the pinned Rust toolchain and QEMU 10.2.4.

## Guest

### Linux

- NVIDIA's stock driver, unmodified, from the `.run` installer or the distribution package. Nothing to set. <!-- src: README.md, docs/WINDOWS_GUEST_SETUP.md -->
- Every recorded Linux run used the **open kernel module** flavour in the guest. Other flavours are **not documented yet**. <!-- src: scripts/bench/provision_bench_tree.sh -->
- Guest driver versions that passed the CUDA ladder with host 580.159.04: 575.57.08, 580.105.08, 580.159.04, 590.48.01, 595.84 and 610.57.04 [measured, September 2026, GA102 box]. A declared guest version outside the measured set is refused by name. <!-- src: docs/design/V3_DRIVER_MATRIX.md section 6.0 -->
- The 30-arm thin-guest suite is graded for 580.x guests only. <!-- src: docs/design/V3_DRIVER_MATRIX.md section 6.0 -->
- **Display** with the guest's own display driver: the wire layouts were derived for guest 580.159.04 only (as of 2026-09-30); any other guest version is refused. Check the current state before relying on another version. <!-- src: docs/design/V3_DRIVER_MATRIX.md (2026-09-30 check) -->

### Windows

- Windows 11 Pro (build 26200) and Windows 11 Enterprise LTSC 2024 (build 26100) with NVIDIA's stock 580.88 driver, plus three registry settings. See [Windows guest](windows-guest.md). <!-- src: docs/WINDOWS_GUEST_SETUP.md -->
- The Windows harness normally gives the guest a TPM 2.0 (swtpm), Secure Boot firmware (OVMF) and kayfabe's boot display driver. A headless device without display or boot-display driver reached Code 0 before resets and a later loss of guest responsiveness [measured at revision `b439ae82`, lease 57908255, GA106, guest 580.88; branch `claude/tur1-20261027`, not merged, `traces/tur1_20261027/iter2_ga106/ga106_h1_ndtimeline.csv`]. That establishes initialisation without the boot-display driver, not a stability pass. A kayfabe guest without TPM or Secure Boot has no recorded acceptance run. <!-- src: scripts/bench/windows/README.md; branch claude/tur1-20261027 commit 0b44315ef -->
- You supply and license Windows yourself. <!-- src: docs/NOTICES.md -->

## Not supported or not finished

- Windows sleep/wake. <!-- src: README.md, docs/WINDOWS_GUEST_SETUP.md -->
- GA100, GB10B (refused by name); Hopper and GB10x (never run on hardware). <!-- src: docs/STATUS_DETAIL.md -->
- Multi-GPU: the two-GPU thin-guest record covers copy-engine checks, serially and concurrently; the full 30-arm regression was a separate, single-device run [measured in `docs/design/V3_MULTI_GPU_AUDIT.md` section 7, revision `74fced87`, lease 52664396, two RTX 3060s on one host]. CUDA inside a two-GPU guest, UVM across two GPUs and peer access were not measured. <!-- src: docs/STATUS_DETAIL.md section 3; docs/design/V3_MULTI_GPU_AUDIT.md section 7 -->
- Several VMs sharing one GPU as a verified, isolated configuration: the effect of a guest-triggered GPU fault on other VMs is not yet measured. <!-- src: README.md -->

Verified on: the lists above are taken from the repository README (2026-10-21), the GPU sweep record of 2026-10-24 (kayfabe `714ec29d3`, one GA104 box) and the driver-matrix record of September 2026 (kayfabe `ee35ca4a`, `1f3e4fc4`, `1837166d`; GA102 boxes). Not verified end to end: no single machine was checked against this page as a whole.
