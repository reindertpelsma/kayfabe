# Windows S3 sleep/wake under kf3 on a second die: the pc's RTX 4070 (2026-11-25)

STATUS: RESEARCH, 2026-11-25. Run on the owner's pc (pc, 2026-11-25 00:03Z to 00:59Z, kf3 **`7bd968c2`**, tag `pcs3`; tools committed before the
first boot, `PREREGISTRATION.md`). **With the optional hyperv-wake patch, the Windows guest survived 5 of 5 consecutive S3 cycles in one boot on an
RTX 4070 (AD104) under host driver 595.91.07, every cycle CLEAN (H1 not falsified). Without the patch the same Hyper-V flags storm 2 of 2 (H2 not
falsified). Arm A (`host,-vmx`, stock binary) survived 3 of 3 (H3 not falsified).** No GPU bind: kf3 used the host GPU through unprivileged RM and the
owner's desktop stayed up. MEASURED = a file in this directory named here; INFERRED = said so.

## Setup (MEASURED, pc, 2026-11-25, kf3 `7bd968c2`)
- Host: AMD Ryzen 9 7900, kernel 7.0.0-34, `current_clocksource` = `tsc`, no TSC-unstable line in `dmesg`, RTX 4070 (AD104), host driver 595.91.07, gdm
  active (`run2/host_before.txt`, `run2/host_after.txt`). The kernel command line was not changed.
- Guest: the sealed non-proprietary `win11_base` (Windows 11 Pro 25H2, NVIDIA 580.88, not the LTSC of the Vast runs), one fresh thin overlay per boot
  of the `prep.qcow2` of `traces/vfio_s3_control_20261124/` (never written), q35, 8 vCPU, 8 GiB, display plane on (`display=on,gop=on`, headless
  broker), S3 on (`-global ICH9-LPC.disable_s3=0`, which follows the argv builder's `disable_s3=1` and wins), default `unmapped-reads-zero`
  (`run2/*/argv_scrubbed.txt`).
- Binaries (built on the pc from the pinned QEMU 10.2.4 tarball, sha256 checked, in a fresh tree, from kf3 `7bd968c2` = head of `origin/next` when the run
  started): stock `scripts/bench/build_kf3.sh`, sha256 `09fb9f44...`; patched `scripts/bench/build_kf3_patched.py --with-patch hyperv-wake`
  (patch sha256 `47658004...`), sha256 `8241beb2...`. The tree in `/workspace/bench/qemu-10.2.4` already contains the wake hunk (its `hw/i386/pc.c`
  has `msr_hv_hypercall`; the tarball's has none), so `kf3-bins/63e46069` and the binaries of `traces/pc_displayexp_20261124/` and
  `traces/pc_hwgates_20261124/` are not stock (see Findings); a fresh tree was used here and the stock binary was built from it.
- CPU flags: arm A `host,-vmx`; arm B adds `hv-relaxed,hv-vapic,hv-spinlocks=0x1fff,hv-time,hv-vpindex,hv-synic,hv-stimer,hv-crash`.
- Harness: `s3kf3pc.py` (cycle = `s3vfio.cycle`: scheduled-task suspend in the signed-in session, QMP `system_wakeup`, ssh back, 150 s wait, System-log
  events, `nvidia-smi`, explorer; plus kf3 log counts and QMP events from `qmp_events.py`), `summarize.py` makes the tables. The CLEAN rule is the preregistered one.

## Results (MEASURED, pc, 2026-11-25, kf3 `7bd968c2`, `run2/`, tables from `summarize.py`: `summary_run2.md`)
Every CLEAN cycle: same `LastBootUpTime`, kf3 `(S3 WAKE) ... KEPT` and `GSP-RESUME` once each, no cold reset, no display `STOPPED`, no `STALL`, no RM unload,
no `USER_RING_REFUSED ... Topology`, guest System log only Kernel-Power 42 and 107, Kernel-General 1 and Power-Troubleshooter 1 (no 41, 1001, 6008, no
`nvlddmkm` 153), `nvidia-smi` in the guest `NVIDIA GeForce RTX 4070, 580.88`, explorer in the interactive session, QMP events only SUSPEND and WAKEUP after boot.

| arm | binary | cycles (boot) | cycle verdicts | sleep (guest Kernel-Power 42, UTC) | wake (host `system_wakeup`, UTC) | ssh back after wake (s) |
|---|---|---|---|---|---|---|
| H1: B | patched | 5 (H1_b1) | CLEAN x5 | 00:05:52, 00:09:16, 00:12:39, 00:16:07, 00:19:24 | 00:05:59, 00:09:23, 00:12:42, 00:16:12, 00:19:28 | 8, 8, 15, 8, 7 |
| H2: B (control) | stock | 2 (H2_b1, H2_b2) | STORM x2 | request to QMP `suspended` 7 s each | 00:25:25, 00:30:17 | never (150 s) |
| H3: A | stock | 3 (H3_b1) | CLEAN x3 | 00:35:05, 00:38:32, 00:42:02 | 00:35:14, 00:38:40, 00:42:08 | 8, 14, 8 |
| H1x: B, `unmapped-reads-zero=off` (extra) | patched | 3 (H1x_b1) | CLEAN x3 | 00:48:40, 00:52:03, 00:55:24 | 00:48:47, 00:52:08, 00:55:28 | 8, 8, 8 |

- The wait from the sleep request to the QMP `suspended` state was 2-7 s in every cycle; the full per-cycle table (cycle start, QMP wait, events, explorer, `nvidia-smi`)
  is `summary_run2.md`, the raw records are `run2/*/c*_result.json`.
- Both H2 storms: ssh never returned; 60 s after the wake the vCPUs exited 3.68 and 6.05 million times a second (`run2/H2_b*/c*_wedge_sample.json`,
  `c*_result.json` `storm`), the TSC page was byte-identical before the suspend and after the wake (`tsc_page_unchanged` true), kf3 had logged `KEPT` but no
  `GSP-RESUME` (the guest never ran the resume). Same signature as the 2026-11-24 VFIO control (`traces/vfio_s3_control_20261124/`).
- Host RM page-table failure (open finding B1), per boot (`summary_run2.md`, `run2/*/host_dmesg_window.txt`): **`mmuWalkMap: Failed to map VA Range 0x120000000
  to 0x20f9fffff. Status = 0x00000051` appeared in all 5 default boots** (1 `mmuWalkMap` line and 18-20 `NV_ERR_NO_MEMORY` lines each) **and in 0 of the 1 boot
  with `unmapped-reads-zero=off`**. The QEMU log carries no 0x51 line in any boot. The guest was not killed by it in any boot (H1, H3 and H1x ran clean).
  No Xid, `BUG:` or `Oops` in any boot's host `dmesg` window.

## Verdicts (falsifiers of PREREGISTRATION.md)
- **H1: not falsified.** 5 of 5 consecutive CLEAN cycles in one boot with the patch; none stalled, stormed or TDR'd.
- **H2: not falsified.** The unpatched arm B did not wake clean in 2 of 2; it stormed in 2 of 2.
- **H3: not falsified.** Arm A on the stock binary: 3 of 3 CLEAN, no storm.
- The release row `windows-sleep-wake` was backed by 3 cycles on one die (AD102, host driver 580.159.04, kf3 `a2df9a81`, `traces/win11_s3_display_20261123/`). This
  run adds a second die (AD104) and host driver (595.91.07), a different guest edition and a newer kf3 (`7bd968c2`). Row changes are the release run's to make.
- Limits: one boot per arm (H1 5 cycles, H3 3, H1x 3); arms ran in a fixed order, not interleaved; each cycle was judged 150 s after ssh returned, so a TDR later
  than that is not covered; the display plane was on in every boot; H1 (patched) and H3 (stock, no enlightenments) differ in both binary and flags, as the
  documented configurations do; the harness does not run a guest workload during sleep.

## Incident: run1 (aborted, `run1_aborted/`; pc, 2026-11-24 23:53Z to 2026-11-25 00:03Z, kf3 `7bd968c2`)
First attempt, patched binary, H1: cycle 1 CLEAN (`summary_run1_aborted.md`). Cycle 2 was marked FAIL-OTHER "new boot" by the harness, but the guest had not
rebooted: the scheduled task `kfs3` is created with `/sc once /st 23:59`, and at guest time 23:59 (cycle 2 started 23:59:19Z) that task ran on its own and
put the guest to sleep before the harness asked, so the harness's baseline `LastBootUpTime` read failed (ssh rc 255, empty) and the cycle was judged against an
empty boot time. QMP shows SUSPEND, WAKEUP, SUSPEND, WAKEUP; kf3 `KEPT` and `GSP-RESUME` for that wake, no 153. I killed the session at 00:03Z, added a retry of the
baseline read (`s3kf3pc.py`), and re-ran everything from H1 cycle 1 as `run2/` (after midnight the 23:59 task no longer fires inside the run). Run1's cycles are
not counted in the table above. [INFERRED] the 2026-11-23 Vast runs used the same task time and were not hit because they did not run across 23:59 guest time.

## Findings (not fixed here)
- **The shared pc QEMU tree is patched.** `/workspace/bench/qemu-10.2.4` contains the hyperv-wake hunk (the old hardwired one), so every binary built from it
  (`kf3-bins/63e46069`, and the binaries of `traces/pc_displayexp_20261124/` and `traces/pc_hwgates_20261124/`) carries it. A stock run needs a fresh tree
  from the tarball. This run's first build used the shared tree (and refreshed it and `/workspace/bench/qemu-build-kf3` to the `7bd968c2` sources); its binary
  (`kf3-bins/7bd968c2`, patched) was removed and replaced by the two builds above in a private directory. The shared tree was left as found otherwise.
- B1 (`mmuWalkMap` 0x51 at the guest's FB alias with the default fill) reproduces on this pc in 5 of 5 default Windows boots and is absent with
  `unmapped-reads-zero=off` in 1 of 1 (pc, 2026-11-25, kf3 `7bd968c2`); it did not stop S3 in any boot.
- Harness: the `/st 23:59` once-task of the S3 probes fires by itself at guest 23:59 (see Incident). `s3vfio.py`'s cycle has no baseline retry; `s3kf3pc.py` adds one.

## Disk and idle
`/workspace` had 29 GB free before the builds, 25 GB during the runs (builds, thin overlays of about 365 MB in total), 29 GB after cleanup. Removed at the end: this
run's overlays, work directories, both build trees, both binaries, the cargo target on tmpfs. At the end: no `qemu`, `swtpm` or broker process, `pc_gpu.lock` free,
host driver 595.91.07 loaded (`nvidia-smi` names the RTX 4070), gdm and nvidia-persistenced active, no vfio module loaded; `PC_ACTIVE.log` has the START and END lines
(tags `s3-4070` for the builds, `vfio-s3` for the VM runs, the tag of the helper it reuses; nothing was bound).

## Files
`PREREGISTRATION.md`; `s3kf3pc.py`, `qmp_events.py`, `summarize.py`; `summary_run2.md`, `summary_run1_aborted.md`; `run2/` and `run1_aborted/`: per boot `argv_scrubbed.txt`,
`boot_summary.json`, `c*_result.json`, `c*_kf3_keys.txt`, `host_dmesg_window.txt`, `qemu_log_tail.txt`, `powercfg_a.txt`, `qmp_events.jsonl`, `c*_wedge_sample.json` (storms); per run
`host_before.txt`, `host_after.txt`, `results.json`, `s3probe.log`; `run2.stdout`, `run1_aborted.stdout`. Left out for size: each cycle's KVM trace head and boot trace tail
(3 MB each). Scrubbed by the helper: UUIDs, IPs, MACs, serials.
