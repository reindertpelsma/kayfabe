# host_tools — scripts that lived only on the Windows bench host

STATUS: LIVE, 2026-10-10. Copied verbatim (text only, each < 200 KB) from the trusted host's
`/var/lib/kf-windows-20261005` and `/root`, so the record of how each run was launched is in git. Passwords and
host addresses are scrubbed; everything else is as it ran. These are the **record**, not maintained tools: paths
(`W=/var/lib/kf-windows-20261005`), revisions and flags are the ones of the day. The host keeps the logs, dumps
and per-run state (`winprod/runN/`, `windows-broker-runN.state`, `wr-launch-N.log`); none of that is in git
(see `docs/design/V3_NOT_IN_GIT.md`).

## Secrets rule

The throwaway test-guest password (the `vast` account of the Windows guest) and the throwaway Ubuntu test-VM
password used to be hard-coded in some runners. In this copy they are read from the environment and the script
aborts if unset: `KF_GUEST_PW` (Windows guest, used by `tdr-run*.sh`, `winprod-run*.sh`, `gl-run-signin*.sh`,
`owner-session.sh`) and `KF_GUEST_SSH_PW` (Ubuntu test VMs, `misc/root_home/do*.sh`, `pc*.sh`). Set them to the
values you chose when the guest image was built (`windows_baseline` docs); they are not recorded in git.

## Common dependencies

bash, `flock` (the caller holds `/tmp/kayfabe-fastguest.lock`), QEMU with the kf3 device (binary under
`kf3-bins/<rev>/`, built by `scripts/bench/build_kf3.sh`), the Windows baseline qcow2 plus a per-run overlay,
the QMP/QGA helpers in `scripts/bench/windows/` (`qmp.py`, `qga_run_ps.py`, `win_vm.sh`), python3, ffmpeg for
`bars.mp4` (below), a GPU bound to vfio-pci with the IOMMU group switched by `iommu_type.sh`.

## Subdirectories and run numbers

Run numbers (approximate ranges inferred from the host's state and log names, not from per-script records) are the `windows-broker-runN.state` / `wr-launch-N.log` / `winprod/runN` numbers on the host.

| dir | what | runs produced |
|---|---|---|
| `runners/winprod/` | production-profile interactive Windows boot (`winprod-run*.sh`, launchers), `hardstop.sh` (owner deadline stop), the two `windows_broker_prod*.sh` and `dxg_etw_stop_tail.ps1` that were untracked in the 6fafcc6e checkout | 260-296, 380-385 |
| `runners/` (top level) | `wr-run.sh`/`wr-go.sh` (windows-reset boots, flock wrapper), `run105-launch.sh`, `drd-run*.sh` (display-reply-diff), `flip-run.sh`, `chain*.sh`, `irqflood-*.sh` (IRQ-flood bisect), `pti-*.sh` (PTI/Windows cycle), `ceint-*.sh` (completion-audit integration evidence), `deferred-gates.sh`, `dm-restart.sh`, `iommu_type.sh` / `pti-iommu_nogdm.sh` (the IOMMU group type switch helpers), `screen_sampler.sh` | wr: 100-118, 130-153, 160-245; drd 97-99; flipv 93-96; irqflood 104-115 |
| `runners/guestlogs/` | scripted sign-in plus guest log collection (`gl-run*.sh`, `gl-launch*.sh`, one per iteration of the sign-in/snapshot recipe), `gl-monitor.py` | 116-181 |
| `runners/click/` | click-trace harness (`click_run.sh`, `click_ctl.py`, VFIO DVI reference click) | 131, 140-145 |
| `runners/broker-interactive/`, `c7v/`, `fixfs/` | Linux-guest broker/interactive proofs, c7 validation, fast-suite regression bisect (`bisect_build.sh`, `bisect_run.sh`, `verify.sh`) | Linux lanes, not Windows runs |
| `runners/winpass/`, `winflip/`, `wintimeout/`, `boundary-tools/` | winpass cycles and ETW arm/stop scripts; flip and D3D probes; wintimeout timing/debugger helpers; boundary experiment tools (QMP-driven) | 60-96, 77-84 |
| `tdrhunt/` | `tdr-run.sh` and every numbered variant `tdr-run2..24.sh` (kept all: each is how a hunt run was launched), `irq_sampler.py`, `evx.py` (event decode), `kfplay.ps1`/`kfbars.ps1`/`kfprobe.ps1`/`kfocc.ps1` (guest playback/occlusion probes). The `*.bundle` files (git bundles of the hunt branches) and `sys263.txt` stay on the host: branch `claude/tdr-hunt-20261010` holds the code | 264-296, 381-415 |
| `tdropus/` | opus hunt queue (`queue.sh` waits for an idle GPU then runs a `tdr-run*.sh` under flock), `owner-session.sh` (run 500, interactive owner session), `pwatch*.sh`, `rwatch.py`, `flipwatch.py`, `gdbwatch.py`, `seq.py`, `vfioseq.py`, `vsrace.py`, `revmap.{c,py}` (reverse page-table map of guest RAM), `elfpa.py` (read GPA from a QEMU dump ELF), `qga_attach_copy.py`, `etwid.py`/`etwwin.py` (DxgKrnl ETW event filters), `invalstats.py`, `pair450.py`, `suspwin.py`, `tslog.py`, `ui.sh` | 276-296, 400-415, 500 |
| `volatility_plugins/` | host-only Volatility 3 plugins over Microsoft structures: `windows/{waitunwind,waitfast,waitscan,tdrctx,reslocks,regunwind,modof,kevents}.py`; `symz*.py` (symbolise addresses from MS PDB JSON), `bm2raw.py` (Windows summary/kernel-bitmap dump to a sparse raw image), `setdump.ps1`/`q223.ps1` (guest crash-dump configuration and query) | 223, 226-232, 260, 264-266, 270, 415 |
| `build/` | host build scripts (`build*.sh` per revision/branch, `tdrhunt-build.sh`, `tdrhunt-build-pb.sh`) | n/a |
| `misc/` | `gsp_alloc_scan.py`, `gsp_ctrl_scan.py`, `gspwin.py`, `irqflood-*.py`, `flipsum.py`, `rm_instloc.ps1`, `sched_probe.ps1`; `root_home/` = the older `/root` scripts of the host (bar1 cycle/sampler, kata builds, PC-matrix A/B, UVM/VA probes, vast job runner) | various |

### Volatility venv recipe (the venv itself is not in git)

```
python3 -m venv dumpcfg/venv && dumpcfg/venv/bin/pip install volatility3 pefile capstone
# plugins: copy volatility_plugins/windows/*.py into <vol3>/volatility3/plugins/windows/ (or pass -p <dir>)
# symbols: Microsoft PDBs -> JSON with volatility3's pdbconv, fetched by build id from the Microsoft symbol server
#   (see V3_NOT_IN_GIT.md); ntoskrnl symbols are fetched by volatility3 itself on first use.
```

### bars.mp4 (not committed)

`tdrhunt/bars.mp4` (6 KB) is the synthetic playback clip used by `kfplay.ps1`. Regenerate:

```
ffmpeg -f lavfi -i testsrc2=size=1280x720:rate=30 -t 30 -c:v libx264 -pix_fmt yuv420p bars.mp4
```

(The exact original parameters were not recorded; any short H.264 colour-bar clip serves.)

## Not copied (kept on the host, names only)

Closed-driver analysis (standing rule: diagnosis only, nothing derived or referenced): `re/fn.py`, `tdropus/vdis.py`
(PE function disassembler), `re/ghidra.zip` and `re/ghidra_12.1.4_PUBLIC/`, `re/img/`, `re/img2/` (module images
carved from dumps). Secrets: `windows-desktop/secrets/`. Logs, dumps, `*.state`, `*.bundle`, venvs, `target*` dirs.

## Files

### `build/` (25 files)

| file | x | what (first comment line, if any) |
|---|---|---|
| `build-d3d.sh` | x |  |
| `build-ilut.sh` | x |  |
| `build-irqflood.sh` | x | build-irqflood.sh — build kf3 for claude/debug-irq-flood-20261009 into kf3-bins/<rev> (own worktree and target dir). |
| `build-irqflood2.sh` | x | build-irqflood2.sh — kf3 of claude/debug-irq-flood-20261009 in a PRIVATE copy of the observer-patched QEMU tree |
| `build-irqflood4.sh` |  | build-irqflood2.sh — kf3 of claude/irqflood-bisect-20261009 in a PRIVATE copy of the observer-patched QEMU tree |
| `build-linuxreg.sh` |  |  |
| `build-observer.sh` | x |  |
| `build-olut.sh` | x |  |
| `build-reenable.sh` | x | build-wintimeout.sh — Windows-reenable agent 2026-10-08: GPU-free tests then kf3 for the checked-out revision. |
| `build-tmo-surface.sh` | x |  |
| `build-windisplay.sh` | x | build-windisplay.sh REV/BRANCH — Windows-display agent 2026-10-08: build kf3 for a revision into |
| `build-wintimeout.sh` | x | build-wintimeout.sh — Windows-timeout agent 2026-10-08: GPU-free tests then kf3 for the checked-out revision. |
| `build96.sh` |  |  |
| `build_c7.sh` |  |  |
| `build_disp.sh` |  |  |
| `build_enb.sh` |  |  |
| `build_int.sh` |  |  |
| `build_int2.sh` |  |  |
| `build_perf_6692e621.sh` | x |  |
| `build_perf_906a76a4.sh` | x |  |
| `build_pieces.sh` |  |  |
| `build_split.sh` |  |  |
| `build_win_6fafcc6e.sh` | x |  |
| `tdrhunt-build-pb.sh` |  |  |
| `tdrhunt-build.sh` |  |  |

### `misc/` (8 files)

| file | x | what (first comment line, if any) |
|---|---|---|
| `flipsum.py` |  | flipsum.py QEMU_LOG -- summarise a kf3 Windows boot for the H-flip/H-loadv record (display write trace lines |
| `gsp_alloc_scan.py` |  | SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later |
| `gsp_ctrl_scan.py` |  | SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later |
| `gspwin.py` |  | Print a window of a kayfabe-gsp-text/1 export: per record dir, fn, ctrl cmd, POST_EVENT fields. |
| `irqflood-analyze.py` |  | analyze.py RUNDIR [LOCK_THRESHOLD] -- one-page summary of an irq-flood boot (run on the bench host). |
| `irqflood-tl.py` |  |  |
| `rm_instloc.ps1` |  | SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later |
| `sched_probe.ps1` |  | SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later |

### `misc/root_home/` (70 files)

| file | x | what (first comment line, if any) |
|---|---|---|
| `bar1-cycle.sh` | x | Distinguish a genuine leak from non-shrinking retained capacity: |
| `bar1-sampler.sh` | x | BAR1 leak sampler. Appends one row per sample; never truncates. |
| `blankwatch.sh` | x | Read-only: samples the guest's output state until it blanks, then captures why. |
| `build-fb8150ed.sh` |  |  |
| `build-kernel-kata.sh` | x |  |
| `build-qemu-kata.sh` | x |  |
| `build.sh` | x | Pinned to a SHA, not a branch name: the Dockerfile clones in a plain RUN, so |
| `cap_sampler.sh` | x |  |
| `chain.sh` | x |  |
| `cuprobe.c` |  | include <stdio.h> |
| `cuprobe2.c` |  | include <stdio.h> |
| `cutest.sh` | x |  |
| `deploy.sh` | x |  |
| `deploy2.sh` |  |  |
| `diag.sh` |  |  |
| `disc.sh` |  |  |
| `docompose.sh` | x |  |
| `doctr.sh` | x |  |
| `dotarball.sh` | x | Fully contained: nothing written to /opt, /usr/lib, or the shared guest dir. |
| `dotb2.sh` | x | stop the previous test VM (match on comm, never on our own cmdline) |
| `drmcap.c` |  | define _GNU_SOURCE |
| `drmcaps.c` |  | include <stdio.h> |
| `evtx_tail.py` |  |  |
| `exp.sh` |  |  |
| `exp2.sh` |  |  |
| `fmt.sh` |  |  |
| `fmt2.sh` |  |  |
| `fmt3.sh` |  |  |
| `frames.sh` |  |  |
| `gs_test.sh` | x | gs_test.sh <label> — bring up nvkvm-steamos and report on the gamescope session. |
| `held-suite-wait.sh` | x |  |
| `kill-rdr2.sh` |  |  |
| `leaksample.sh` | x |  |
| `lintest.c` |  |  |
| `mapva.py` | x | Read QEMU's g_mapva table live out of /proc/<pid>/mem. |
| `modq.c` |  | define _GNU_SOURCE |
| `mon.sh` | x | sample the guest's scanout every 60s; a frozen fb id == the black-after-idle bug |
| `nsi-build.sh` |  | Host-side, GPU-free: check out the branch tip in /root/kf-nsi-nogate-20261008, build the stamped raw |
| `nvsample.sh` | x |  |
| `nvsample2.sh` | x |  |
| `pc-ab.sh` | x | A/B on the reference machine: a guest in the TRUE pre-fix state (manual |
| `pc-ab2.sh` | x | A leg, done properly: move aside EVERY libcuda the loader can resolve, so the |
| `pc-b.sh` | x | B leg: same guest, same broken state (no libcuda anywhere), now under the |
| `pc-b2.sh` | x | B leg v2. Fail FAST if the checkout does not actually land the fix, instead |
| `pc-before.sh` | x | Boot the EXISTING (pre-fix) guest and record its state. Read-only. |
| `pc-respec.sh` | x | Restart the SteamOS guest with more CPU and RAM, to test whether the RDR2 |
| `pc39.sh` |  |  |
| `pc_matrix.sh` |  | Two arms, both as a SECOND vm (desktop VM on 2222 untouched). |
| `pc_native.sh` |  |  |
| `pin-vcpus.sh` | x | Pin each guest vCPU thread 1:1 to a host hardware thread, and confine QEMU's |
| `pin-vcpus2.sh` |  | Pin ONLY the real vCPU threads 1:1. Worker threads spawned by a vCPU inherit |
| `qmp.py` |  |  |
| `resume-native4070-219b4ee7.py` |  | Resume the preserved 2026-10-04 native RTX 4070 fixture in an independent clone. |
| `run-vfio4070.py` |  |  |
| `run_test.sh` | x |  |
| `runmint-diff.sh` | x |  |
| `runmint.sh` | x |  |
| `sampler.sh` | x |  |
| `timeout_ab.sh` | x | A/B the TimeoutStartSec fix by making gamescope deliberately slow to report ready. |
| `uvm2.sh` |  |  |
| `uvmprobe.sh` |  |  |
| `uvmtest.c` |  | include <stdio.h> |
| `va_capacity.c` |  |  |
| `va_probe.sh` | x |  |
| `va_probe2.sh` | x |  |
| `vast-windows-test-job.py` |  |  |
| `vecadd.c` |  |  |
| `vk_import_host_ptr.c` |  |  |
| `wake.sh` |  |  |
| `watch2.sh` | x | Every 5 min: is there a picture, and is the capture still being refreshed? |

### `runners/` (29 files)

| file | x | what (first comment line, if any) |
|---|---|---|
| `ceint-evidence.sh` |  | Host-side: stage the text evidence of the --ce-interrupt runs (filtered of per-ioctl trace noise). |
| `ceint-flagsweep.sh` |  | For every tag of the driver matrix: the value of the two nvos.h event flags the raw client ORs into |
| `ceint-guestrun.sh` |  | Host-side: run the fast-guest arms; optionally wait until no other tenant's QEMU is on the GPU. |
| `ceint-hostbuild.sh` |  | Host-side: check out the pushed branch head in the ceint worktree and build the stamped client. |
| `ceint-hostfinal.sh` |  | Host-side: the final-revision cycle (GPU-free parts): checkout, tests, fmt check, gates, clippy, stamped build, initrd. |
| `ceint-runall.sh` |  | Host-side: the hardware cycle for revision $1 (bare 30 + bare ce-interrupt x3 + unprivileged + kf3 fast guest). |
| `ceint-runall2.sh` |  | Host-side: the hardware cycle for revision $1 (bare 30 + bare ce-interrupt x3 + unprivileged + kf3 fast guest). |
| `chain.sh` |  | chain.sh REV N1:FLOOD1 N2:FLOOD2 ... — serial boots (FLOOD "-" = flood off), each under the fastguest flock |
| `chain239.sh` |  |  |
| `deferred-gates.sh` |  | Serial: v3 gates, then the GR-tier native oracle (with the §U gate check), at the checked-out commit. |
| `dm-restart.sh` |  |  |
| `drd-run.sh` |  | drd-run.sh N REV "EXTRA_FLAGS" — one Windows boot on kf3 (display reply diff batch, 2026-10-09; copy of flip-run.sh). |
| `drd-run2.sh` |  | drd-run2.sh N REV "EXTRA_FLAGS" — one Windows boot on kf3 (display reply diff, boot 3 of the 2026-10-09 time-box). |
| `flip-run.sh` |  | flip-run.sh N REV "EXTRA_FLAGS" — one Windows boot on kf3 (H-flip / H-loadv session, 2026-10-08). |
| `iommu_type.sh` |  | iommu_type.sh TYPE — switch the RTX 4070's IOMMU group (01:00.0 + 01:00.1) to TYPE (identity / DMA-FQ) |
| `irqflood-collect.sh` |  | collect.sh N — small evidence files for run N into /var/lib/kf-windows-20261005/irqflood-evidence/runN |
| `irqflood-launch-3e9.sh` | x | irqflood-launch.sh N [FLOODWORD] — run N of the interrupt-flood matrix (run 104 flags; binary 3e9bcdce) |
| `irqflood-launch.sh` | x | irqflood-launch.sh N [FLOODWORD] — run N of the interrupt-flood matrix (run 104 flags; binary b728b480) |
| `irqflood-launch2.sh` |  | irqflood-launch2.sh N REV [FLOODWORD] — run N of the bisect matrix: irqflood-launch.sh with the binary as an argument |
| `pti-check.sh` |  | Host-side: sync the pti worktree to the pushed branch and run the GPU-free checks. |
| `pti-iommu_nogdm.sh` |  | iommu_nogdm.sh TYPE — switch the RTX 4070's IOMMU group to TYPE WITHOUT touching gdm (GNOME runs on |
| `pti-run.sh` |  | Host-side: the passthrough-interrupt hardware cycle. Every step takes the SHARED GPU lock |
| `pti-win-cycle.sh` |  | winpass cycle: one Windows boot on kf3 (windows_broker.sh run N), guest work through QGA, clean stop. |
| `pti-win-run.sh` |  |  |
| `run105-launch.sh` | x |  |
| `screen_sampler.sh` | x | SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later |
| `start-linux-demo-when-free.sh` | x | Start the Linux demo (interactive.sh, kf3 4bc62999) once the GPU lock is free (another agent held it at 20:09). |
| `wr-go.sh` | x | wrapper: flock -o around one wr-run.sh boot |
| `wr-run.sh` | x | wr-run.sh N REV "EXTRA_FLAGS" — one Windows boot on kf3 for the windows-reset record (2026-10-09). |

### `runners/boundary-tools/` (4 files)

| file | x | what (first comment line, if any) |
|---|---|---|
| `capture_display_caps.py` |  | SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later |
| `pc_boundary_experiment.py` |  | SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later |
| `qmp.py` | x | SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later |
| `recover_pc_watchdog.py` |  | SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later |

### `runners/broker-interactive/` (4 files)

| file | x | what (first comment line, if any) |
|---|---|---|
| `ab.sh` | x | A/B: which kf3 display config stalls the scanout copy on this host (595.91.07) |
| `e1.sh` | x | e1.sh <name> <qemu-bin> <kf3-props> <broker:none/pre> <shot-at-s/-> <paused-s/0> [extra env...] |
| `s3.sh` |  | subpixel run s3: against the running demo guest (run-20261008-151825), broker swapped per phase |
| `uinput_mouse.c` |  | SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later |

### `runners/c7v/` (3 files)

| file | x | what (first comment line, if any) |
|---|---|---|
| `run12.sh` |  |  |
| `run3.sh` |  |  |
| `run_gates2.sh` |  |  |

### `runners/click/` (8 files)

| file | x | what (first comment line, if any) |
|---|---|---|
| `click_ctl.py` | x | SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later |
| `click_launch.sh` | x | click_launch.sh MODE N REV [GESTURE] — click_run.sh under the fastguest flock (waits for any other holder, e.g. a coordinator run). |
| `click_run.sh` | x | click_run.sh MODE N REV [GESTURE] — one scripted-input boot (README section 18, 2026-10-09). MODE = kf3 / vfio. |
| `gl-monitor.py` |  | SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later |
| `guest_logs_collect.ps1` |  | SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later |
| `guest_logs_events.ps1` |  | SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later |
| `guest_logs_light.ps1` |  | SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later |
| `vfio_dvi_reference_click.sh` | x | SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later |

### `runners/fixfs/` (3 files)

| file | x | what (first comment line, if any) |
|---|---|---|
| `bisect_build.sh` |  | Build kf3 QEMU for each revision given (serial; no GPU). Bins land in fixfs/kf3-bins/<rev>/. |
| `bisect_run.sh` |  | Run the --timer arm for each built revision (fast_suite/run_fast_guest take the GPU flock). |
| `verify.sh` |  | Hardware verification of ONE revision: build_kf3 -> raw client -> fast guest -> fast suite (30 arms) |

### `runners/guestlogs/` (52 files)

| file | x | what (first comment line, if any) |
|---|---|---|
| `gl-launch-flags.sh` | x | gl-launch.sh N [REV] — run N of the guest-log collection under the fastguest flock, then the usual evidence collector. |
| `gl-launch-quiet.sh` | x | gl-launch.sh N [REV] — run N of the guest-log collection under the fastguest flock, then the usual evidence collector. |
| `gl-launch-quietobs.sh` | x | gl-launch.sh N [REV] — run N of the guest-log collection under the fastguest flock, then the usual evidence collector. |
| `gl-launch-quietsnap.sh` | x | gl-launch.sh N [REV] — run N of the guest-log collection under the fastguest flock, then the usual evidence collector. |
| `gl-launch-signin.sh` | x | gl-launch.sh N [REV] — run N of the guest-log collection under the fastguest flock, then the usual evidence collector. |
| `gl-launch-signinsnap.sh` |  | gl-launch.sh N [REV] — run N of the guest-log collection under the fastguest flock, then the usual evidence collector. |
| `gl-launch-signinsnap10.sh` |  | gl-launch.sh N [REV] — run N of the guest-log collection under the fastguest flock, then the usual evidence collector. |
| `gl-launch-signinsnap11.sh` |  | gl-launch.sh N [REV] — run N of the guest-log collection under the fastguest flock, then the usual evidence collector. |
| `gl-launch-signinsnap12.sh` |  | gl-launch.sh N [REV] — run N of the guest-log collection under the fastguest flock, then the usual evidence collector. |
| `gl-launch-signinsnap13.sh` |  | gl-launch.sh N [REV] — run N of the guest-log collection under the fastguest flock, then the usual evidence collector. |
| `gl-launch-signinsnap14.sh` |  | gl-launch.sh N [REV] — run N of the guest-log collection under the fastguest flock, then the usual evidence collector. |
| `gl-launch-signinsnap15.sh` |  | gl-launch.sh N [REV] — run N of the guest-log collection under the fastguest flock, then the usual evidence collector. |
| `gl-launch-signinsnap2.sh` |  | gl-launch.sh N [REV] — run N of the guest-log collection under the fastguest flock, then the usual evidence collector. |
| `gl-launch-signinsnap3.sh` |  | gl-launch.sh N [REV] — run N of the guest-log collection under the fastguest flock, then the usual evidence collector. |
| `gl-launch-signinsnap4.sh` |  | gl-launch.sh N [REV] — run N of the guest-log collection under the fastguest flock, then the usual evidence collector. |
| `gl-launch-signinsnap5.sh` |  | gl-launch.sh N [REV] — run N of the guest-log collection under the fastguest flock, then the usual evidence collector. |
| `gl-launch-signinsnap6.sh` |  | gl-launch.sh N [REV] — run N of the guest-log collection under the fastguest flock, then the usual evidence collector. |
| `gl-launch-signinsnap7.sh` |  | gl-launch.sh N [REV] — run N of the guest-log collection under the fastguest flock, then the usual evidence collector. |
| `gl-launch-signinsnap8.sh` |  | gl-launch.sh N [REV] — run N of the guest-log collection under the fastguest flock, then the usual evidence collector. |
| `gl-launch-signinsnap9.sh` |  | gl-launch.sh N [REV] — run N of the guest-log collection under the fastguest flock, then the usual evidence collector. |
| `gl-launch.sh` |  | gl-launch.sh N [REV] — run N of the guest-log collection under the fastguest flock, then the usual evidence collector. |
| `gl-monitor.py` |  | SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later |
| `gl-run-flags.sh` |  | gl-run.sh N REV — one Windows boot on kf3 (flood OFF; the flags of run 114, KF3_NO_BATCHED_MAP included) with the guest-log collector |
| `gl-run-quiet.sh` | x | gl-run.sh N REV — one Windows boot on kf3 (flood OFF; the flags of run 114, KF3_NO_BATCHED_MAP included) with the guest-log collector |
| `gl-run-quietobs.sh` | x | gl-run.sh N REV — one Windows boot on kf3 (flood OFF; the flags of run 114, KF3_NO_BATCHED_MAP included) with the guest-log collector |
| `gl-run-quietsnap.sh` | x | gl-run.sh N REV — one Windows boot on kf3 (flood OFF; the flags of run 114, KF3_NO_BATCHED_MAP included) with the guest-log collector |
| `gl-run-signin.sh` | x | gl-run.sh N REV — one Windows boot on kf3 (flood OFF; the flags of run 114, KF3_NO_BATCHED_MAP included) with the guest-log collector |
| `gl-run-signinsnap.sh` |  | gl-run.sh N REV — one Windows boot on kf3 (flood OFF; the flags of run 114, KF3_NO_BATCHED_MAP included) with the guest-log collector |
| `gl-run-signinsnap10.sh` |  | gl-run.sh N REV — one Windows boot on kf3 (flood OFF; the flags of run 114, KF3_NO_BATCHED_MAP included) with the guest-log collector |
| `gl-run-signinsnap11.sh` |  | gl-run.sh N REV — one Windows boot on kf3 (flood OFF; the flags of run 114, KF3_NO_BATCHED_MAP included) with the guest-log collector |
| `gl-run-signinsnap12.sh` |  | gl-run.sh N REV — one Windows boot on kf3 (flood OFF; the flags of run 114, KF3_NO_BATCHED_MAP included) with the guest-log collector |
| `gl-run-signinsnap13.sh` |  | gl-run.sh N REV — one Windows boot on kf3 (flood OFF; the flags of run 114, KF3_NO_BATCHED_MAP included) with the guest-log collector |
| `gl-run-signinsnap14.sh` |  | gl-run.sh N REV — one Windows boot on kf3 (flood OFF; the flags of run 114, KF3_NO_BATCHED_MAP included) with the guest-log collector |
| `gl-run-signinsnap15.sh` |  | gl-run.sh N REV — one Windows boot on kf3 (flood OFF; the flags of run 114, KF3_NO_BATCHED_MAP included) with the guest-log collector |
| `gl-run-signinsnap2.sh` |  | gl-run.sh N REV — one Windows boot on kf3 (flood OFF; the flags of run 114, KF3_NO_BATCHED_MAP included) with the guest-log collector |
| `gl-run-signinsnap3.sh` |  | gl-run.sh N REV — one Windows boot on kf3 (flood OFF; the flags of run 114, KF3_NO_BATCHED_MAP included) with the guest-log collector |
| `gl-run-signinsnap4.sh` |  | gl-run.sh N REV — one Windows boot on kf3 (flood OFF; the flags of run 114, KF3_NO_BATCHED_MAP included) with the guest-log collector |
| `gl-run-signinsnap5.sh` |  | gl-run.sh N REV — one Windows boot on kf3 (flood OFF; the flags of run 114, KF3_NO_BATCHED_MAP included) with the guest-log collector |
| `gl-run-signinsnap6.sh` |  | gl-run.sh N REV — one Windows boot on kf3 (flood OFF; the flags of run 114, KF3_NO_BATCHED_MAP included) with the guest-log collector |
| `gl-run-signinsnap7.sh` |  | gl-run.sh N REV — one Windows boot on kf3 (flood OFF; the flags of run 114, KF3_NO_BATCHED_MAP included) with the guest-log collector |
| `gl-run-signinsnap8.sh` |  | gl-run.sh N REV — one Windows boot on kf3 (flood OFF; the flags of run 114, KF3_NO_BATCHED_MAP included) with the guest-log collector |
| `gl-run-signinsnap9.sh` |  | gl-run.sh N REV — one Windows boot on kf3 (flood OFF; the flags of run 114, KF3_NO_BATCHED_MAP included) with the guest-log collector |
| `gl-run.sh` |  | gl-run.sh N REV — one Windows boot on kf3 (flood OFF; the flags of run 114, KF3_NO_BATCHED_MAP included) with the guest-log collector |
| `guest_logs_collect.ps1` |  | SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later |
| `guest_logs_events.ps1` |  | SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later |
| `guest_logs_light.ps1` |  | SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later |
| `manual-run.sh` | x | gl-run.sh N REV — one Windows boot on kf3 (flood OFF; the flags of run 114, KF3_NO_BATCHED_MAP included) with the guest-log collector |
| `until-alive.sh` | x | until-alive.sh START_N [MAX_TRIES] [HOLD_S]: run 118-style boots (gl-launch.sh, binary 66eeebb6, QGA monitor) until one is still alive HOLD_S |
| `until-alive2.sh` | x | until-alive.sh START_N [MAX_TRIES] [HOLD_S]: run 118-style boots (gl-launch.sh, binary 66eeebb6, QGA monitor) until one is still alive HOLD_S |
| `until-alive3.sh` | x | until-alive.sh START_N [MAX_TRIES] [HOLD_S]: run 118-style boots (gl-launch.sh, binary 66eeebb6, QGA monitor) until one is still alive HOLD_S |
| `until-alive4.sh` | x | until-alive.sh START_N [MAX_TRIES] [HOLD_S]: run 118-style boots (gl-launch.sh, binary 66eeebb6, QGA monitor) until one is still alive HOLD_S |
| `until-alive5.sh` | x | until-alive.sh START_N [MAX_TRIES] [HOLD_S]: run 118-style boots (gl-launch.sh, binary 66eeebb6, QGA monitor) until one is still alive HOLD_S |

### `runners/winflip/` (4 files)

| file | x | what (first comment line, if any) |
|---|---|---|
| `d3d11_clear_probe.ps1` |  | SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later |
| `d3d12_signal_probe.ps1` |  | SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later |
| `vdr_monitor_info.ps1` |  | SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later |
| `vdr_user_session.ps1` |  | SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later |

### `runners/winpass/` (12 files)

| file | x | what (first comment line, if any) |
|---|---|---|
| `cycle.sh` |  | winpass cycle: one Windows boot on kf3 (windows_broker.sh run N), guest work through QGA, clean stop. |
| `cycle2.sh` |  | winpass cycle: one Windows boot on kf3 (windows_broker.sh run N), guest work through QGA, clean stop. |
| `cycle3.sh` |  | winpass cycle: one Windows boot on kf3 (windows_broker.sh run N), guest work through QGA, clean stop. |
| `cycle4.sh` |  | winpass cycle: one Windows boot on kf3 (windows_broker.sh run N), guest work through QGA, clean stop. |
| `dxg_decode_kept.ps1` |  | winpass: decode the newest C:\kf\kfdxg-stall-*.etl (stopped by etwstopnow.ps1 DURING the stall, so it |
| `dxg_etw_arm.ps1` |  | SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later |
| `dxg_etw_stop.ps1` |  | SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later |
| `etwstopnow.ps1` |  | keep this boot's trace: the autologger (Append Off) would overwrite C:\kf\kfdxg.etl at the next boot |
| `hags.ps1` |  | winpass: the guest's hardware-scheduling (HAGS) state and display adapters, from dxdiag (read-only) |
| `hws.ps1` |  |  |
| `hwsch_off.ps1` |  | winpass run91 variable: HwSchMode = 1 (hardware-accelerated GPU scheduling OFF) for the next boot |
| `tdroff_arm.ps1` |  | winpass (diagnostic, guest-side): TdrLevel = 0 (Windows' GPU timeout detection OFF) for the NEXT boot, so a |

### `runners/winprod/` (10 files)

| file | x | what (first comment line, if any) |
|---|---|---|
| `dxg_etw_stop_tail.ps1` |  | SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later |
| `hardstop.sh` | x | hardstop.sh — owner deadline 2026-10-10 03:41 CEST: stop the interactive guest cleanly, then by TERM, release the GPU. |
| `windows_broker_prod.sh` |  | SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later |
| `windows_broker_prod2.sh` | x | SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later |
| `winprod-launch.sh` | x | winprod-launch.sh N — queue behind /tmp/kayfabe-fastguest.lock (blocks), then run the production Windows guest. |
| `winprod-launch2.sh` | x | winprod-launch.sh N — queue behind /tmp/kayfabe-fastguest.lock (blocks), then run the production Windows guest. |
| `winprod-launch3.sh` | x | winprod-launch.sh N — queue behind /tmp/kayfabe-fastguest.lock (blocks), then run the production Windows guest. |
| `winprod-run.sh` | x | winprod-run.sh N — ONE interactive Windows 11 guest on kf3, PRODUCTION profile (class-B behaviour flags only, |
| `winprod-run2.sh` | x | winprod-run.sh N — ONE interactive Windows 11 guest on kf3, PRODUCTION profile (class-B behaviour flags only, |
| `winprod-run3.sh` | x | winprod-run.sh N — ONE interactive Windows 11 guest on kf3, PRODUCTION profile (class-B behaviour flags only, |

### `runners/wintimeout/` (12 files)

| file | x | what (first comment line, if any) |
|---|---|---|
| `ce_blocksync.c` |  |  |
| `dbg_analyze.ps1` |  |  |
| `dbg_install.ps1` |  |  |
| `dbg_log.ps1` |  |  |
| `dbg_status.ps1` |  |  |
| `dbg_tail.ps1` |  |  |
| `etl_status.ps1` |  |  |
| `tdr_delay.ps1` |  |  |
| `tdr_read.ps1` |  |  |
| `timing.sh` |  | Per run: async-preempt DISABLE_CHANNELS (refused or served), RUNLIST_PREEMPT_COMPLETE posts, the |
| `tl.py` |  | Timeline of a kf3 qemu.log: every rpc-trace / refusal / relay / birth / free / RC line with the last |
| `wpr_gpu.ps1` |  |  |

### `tdrhunt/` (30 files)

| file | x | what (first comment line, if any) |
|---|---|---|
| `evx.py` |  |  |
| `irq_sampler.py` |  | SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later |
| `kfbars.ps1` |  | kfbars.ps1: run in the interactive session; opens the colour-bar clip in Edge. |
| `kfocc.ps1` |  | kfocc.ps1: an opaque topmost cmd window over the video's lower middle (logical px; the guest is DPI-scaled 1.25) |
| `kfplay.ps1` |  | kfplay.ps1 TAG : run in the INTERACTIVE session (schtasks /it). Guest-side playback evidence: |
| `kfprobe.ps1` |  |  |
| `tdr-run.sh` | x | tdr-run.sh N — TDR hunt runner (host-only tool, copy kept in git as the record of how runs 264+ were launched). |
| `tdr-run10.sh` |  | tdr-run.sh N — TDR hunt runner (host-only tool, copy kept in git as the record of how runs 264+ were launched). |
| `tdr-run11.sh` |  | tdr-run.sh N — TDR hunt runner (host-only tool, copy kept in git as the record of how runs 264+ were launched). |
| `tdr-run12.sh` |  | tdr-run.sh N — TDR hunt runner (host-only tool, copy kept in git as the record of how runs 264+ were launched). |
| `tdr-run13.sh` |  | tdr-run.sh N — TDR hunt runner (host-only tool, copy kept in git as the record of how runs 264+ were launched). |
| `tdr-run14.sh` |  | tdr-run.sh N — TDR hunt runner (host-only tool, copy kept in git as the record of how runs 264+ were launched). |
| `tdr-run15.sh` |  | tdr-run.sh N — TDR hunt runner (host-only tool, copy kept in git as the record of how runs 264+ were launched). |
| `tdr-run16.sh` |  | tdr-run.sh N — TDR hunt runner (host-only tool, copy kept in git as the record of how runs 264+ were launched). |
| `tdr-run17.sh` |  | tdr-run.sh N — TDR hunt runner (host-only tool, copy kept in git as the record of how runs 264+ were launched). |
| `tdr-run18.sh` |  | tdr-run.sh N — TDR hunt runner (host-only tool, copy kept in git as the record of how runs 264+ were launched). |
| `tdr-run19.sh` |  | tdr-run.sh N — TDR hunt runner (host-only tool, copy kept in git as the record of how runs 264+ were launched). |
| `tdr-run2.sh` | x | tdr-run.sh N — TDR hunt runner (host-only tool, copy kept in git as the record of how runs 264+ were launched). |
| `tdr-run20.sh` |  | tdr-run.sh N — TDR hunt runner (host-only tool, copy kept in git as the record of how runs 264+ were launched). |
| `tdr-run21.sh` |  | tdr-run.sh N — TDR hunt runner (host-only tool, copy kept in git as the record of how runs 264+ were launched). |
| `tdr-run22.sh` |  | tdr-run.sh N — TDR hunt runner (host-only tool, copy kept in git as the record of how runs 264+ were launched). |
| `tdr-run23.sh` |  | tdr-run.sh N — TDR hunt runner (host-only tool, copy kept in git as the record of how runs 264+ were launched). |
| `tdr-run24.sh` |  | tdr-run.sh N — TDR hunt runner (host-only tool, copy kept in git as the record of how runs 264+ were launched). |
| `tdr-run3.sh` | x | tdr-run.sh N — TDR hunt runner (host-only tool, copy kept in git as the record of how runs 264+ were launched). |
| `tdr-run4.sh` | x | tdr-run.sh N — TDR hunt runner (host-only tool, copy kept in git as the record of how runs 264+ were launched). |
| `tdr-run5.sh` | x | tdr-run.sh N — TDR hunt runner (host-only tool, copy kept in git as the record of how runs 264+ were launched). |
| `tdr-run6.sh` | x | tdr-run.sh N — TDR hunt runner (host-only tool, copy kept in git as the record of how runs 264+ were launched). |
| `tdr-run7.sh` | x | tdr-run.sh N — TDR hunt runner (host-only tool, copy kept in git as the record of how runs 264+ were launched). |
| `tdr-run8.sh` |  | tdr-run.sh N — TDR hunt runner (host-only tool, copy kept in git as the record of how runs 264+ were launched). |
| `tdr-run9.sh` |  | tdr-run.sh N — TDR hunt runner (host-only tool, copy kept in git as the record of how runs 264+ were launched). |

### `tdropus/` (21 files)

| file | x | what (first comment line, if any) |
|---|---|---|
| `elfpa.py` |  | Read guest-physical bytes from a QEMU dump-guest-memory ELF (paging=false). usage: elfpa.py dump.elf GPA LEN""" |
| `etwid.py` |  | Print DxgKrnl CSV events whose id is in a list, inside a UTC window. usage: etwid.py etwwin.py csv HH:MM:SS HH:MM:SS id,id,...""" |
| `etwwin.py` |  | Dump DxgKrnl ETW CSV rows (tracerpt) in a UTC time window, optionally filtered by a substring. |
| `flipwatch.py` |  | Host-side watcher (TDR hunt, shape F): tail a kf3 qemu.log with KF3_DISPLAY_WRITE_TRACE. A window LATCH whose vblank the |
| `gdbwatch.py` |  | gdbwatch.py — loaded by `gdb -batch -x gdbwatch.py`; env GW_PORT, GW_ADDRS (comma list of "tag:addr"), GW_LOG. |
| `invalstats.py` |  | Per run: VA-thread invalidate statistics from kf3 status lines (2 s windows): total, window avg, cumulative max and the |
| `owner-session.sh` |  | owner-session.sh — queue an interactive Windows run for the owner (no scripted scrolling), then hot-plug the app disk once signed in. |
| `pair450.py` |  | Pair DxgKrnl 450 (submit to hw, hContext, fenceId) with 451 (completed, hContext, fenceId); list unpaired before T.""" |
| `pwatch.sh` |  | PREEMPTDUMP=1 (H-S, shape S): watch kayfabe's log for a preempt-all burst the guest never re-enables: >= PD_MIN (6) |
| `pwatch2.sh` |  | PREEMPTDUMP=1 (H-S, shape S): watch kayfabe's log, line by line in order, for a disable list (DISABLE_CHANNELS(bDisable=true)) of |
| `qga_attach_copy.py` |  | SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later |
| `queue.sh` | x | queue.sh N BIN HOLD RUNNER [extra env...] — wait until the GPU is idle (no QEMU, no other tdr-run), then run under the flock. |
| `revmap.c` |  | revmap.c — every kernel VA (PML4 256..511) that maps guest-physical page PA, walking the guest's 4-level page |
| `revmap.py` |  | revmap.py QPID CR3 PA [LOWMEM] — every kernel VA that maps guest-physical page PA, by walking the guest's |
| `rwatch.py` |  | rwatch.py RUN_DIR QPID OUT_DIR [SLOTS] — follow kayfabe's qemu.log; for every new window-notifier page (the |
| `seq.py` |  | Compact sequence of disable(+)/enable(-)/suspend(S)/schedule events in a qemu.log, with the last seen time stamp.""" |
| `suspwin.py` |  | For each FLUSHSCHEDULER_SUSPEND->RESUME window in a DxgKrnl CSV: duration, paging-op starts (324), op stops (325), |
| `tslog.py` |  | Print qemu.log lines a..b (or a time window) prefixed with the last WTRACE t= stamp; drop VSync/ack lines.""" |
| `ui.sh` | x | ui.sh RUN cmd args — drive a running guest: shot NAME / key QCODE / click PX PY (pixels of 1920x1080) / combo K1 K2 |
| `vfioseq.py` |  | List GSP RPCs of a VFIO observer trace touching DISABLE_CHANNELS (0x2080110b) and POST_EVENT 139, in order.""" |
| `vsrace.py` |  | For every window LATCH in a kf3 display write trace: the VSync raise before it, the guest's ack of that VSync |

### `volatility_plugins/` (6 files)

| file | x | what (first comment line, if any) |
|---|---|---|
| `bm2raw.py` |  | Diagnosis helper (host only): Windows summary/kernel bitmap dump -> sparse raw physical image for Volatility. |
| `q223.ps1` |  |  |
| `setdump.ps1` |  |  |
| `symz.py` |  |  |
| `symz2.py` |  |  |
| `symz3.py` |  |  |

### `volatility_plugins/windows/` (8 files)

| file | x | what (first comment line, if any) |
|---|---|---|
| `kevents.py` |  | Diagnosis helper, host-only: read _KEVENT objects at given kernel addresses (public ntoskrnl types) and their waiters. |
| `modof.py` |  | Diagnosis helper, host-only, never committed: waiting threads of a Windows memory image with a stack-scan of module return addresses. |
| `regunwind.py` |  | Diagnosis helper, host-only, never committed: real x64 unwind of waiting kernel threads from a memory image. |
| `reslocks.py` |  | Diagnosis helper, host-only, never committed: contended ERESOURCEs and their owner threads. |
| `tdrctx.py` |  | Diagnosis helper, host-only, never committed: waiting threads of a Windows memory image with a stack-scan of module return addresses. |
| `waitfast.py` |  | Diagnosis helper, host-only, never committed: waiting threads of a Windows memory image with a stack-scan of module return addresses. |
| `waitscan.py` |  | Diagnosis helper, host-only, never committed: waiting threads of a Windows memory image with a stack-scan of module return addresses. |
| `waitunwind.py` |  | Diagnosis helper, host-only, never committed: real x64 unwind of waiting kernel threads from a memory image. |
