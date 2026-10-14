# Real Windows 580.88 GSP traffic on dies other than Ada (2026-10-14, constants measurement lane)

**STATUS: LIVE, 2026-10-14 (in progress; written as the run goes so that a crash loses little).**
Labels: **[M]** measured by a command in this directory, **[I]** inferred, **[U]** not measured.
Procedure: the AD103 run, `traces/real_windows_20261014/README.md` and `docs/design/V3_WINDOWS_CAPTURE_MODULE.md`
12a/12b, repeated on other dies. Capture files (`*.kfgspcap`) stay on the trusted host under
`/data/kfwc-<die>-20261014/`; only census text, scrubbed-body comparisons, sidecars and scripts are here. No UUID,
serial, VBIOS string, MAC or PCI subsystem id is in any file (scripts never print them; grep before each commit).

## Decisions taken without the owner (asleep), with reasons

1. **Which bodies may leave the host.** Lane C asked for per-control reply values of static-info controls and of
   the Windows-only rows. Values are taken only from `kf-census scrub` output (default + `--allow-screened` lists
   + `allow_static_20261014.txt`), read back with the new `kf-census --bodies` (commit `0f7f4b3c4`; it refuses any
   input without the scrubbed flag). `allow_static_20261014.txt` adds 27 static-info controls that carry die-level
   facts only. Kept **out** on purpose (sizes and status only): `GPU_GET_INFO_V2` (ECID indices: a per-chip unique
   id), `INTERNAL_GPU_GET_CHIP_INFO` (PCI subsystem ids), the floorsweeping controls (`GET_GLOBAL_SM_ORDER`,
   `GET_FLOORSWEEPING_MASKS`, `GET_PPC_MASKS`, `GRMGR_GET_GR_FS_INFO`: per chip instance), `FB_GET_INFO_V2`,
   `BUS_GET_INFO_V2`, `BUS_GET_PCI_BAR_INFO` (machine/slot data), and everything on `scrub::BOARD_DATA_CONTROLS`.
   The task's "class list (0x2080014a)" is `GPU_GET_GID_INFO` (the UUID; deny-listed); the class list is
   `NV0080_CTRL_CMD_GPU_GET_CLASSLIST_V2` (0x00800292), compared here.
2. **Equality** is over the set of distinct reply bodies (and separately request bodies) per control. Run-to-run
   noise on AD103 itself (capture E vs C, same box, two different boots): every static-info row and every
   Windows-only row is byte-identical **except** `0x20809004`, `0x2080b201`, `0x20801111` (reply sets differ) and
   `0x2080a088` (request set differs) [M, `ad103_baseline/ad103_E_vs_C_values.tsv`]. A "no" on those four rows
   across dies therefore says nothing about the die.
3. **Reference** for the die comparison: AD103 capture C (one complete GSP boot after a device restart, `kf-census`
   PASS), the same shape as the captures taken here.
4. **Rentals.** Vast KVM offers on 2026-10-14 02:20: one RTX 3070 (GA104) at $0.17/h, an RTX 5060 Ti (GB206) at
   $0.16-0.22/h (taken by lane B before this lane could rent it), no Turing at all, other Blackwell >= $0.63/h
   (over the $0.60 limit).

## Falsifiers, stated before the runs

| # | hypothesis | falsifier |
|---|---|---|
| G1 | The Windows 580.88 GeForce driver runs GA104 on GSP when `EnableGpuFirmware=1` is in both keys and the adapter is restarted | `nvidia-smi -q` GSP Firmware `N/A` after the restart, or the collector attaches to nothing |
| G2 | The mailbox route gives a complete capture from `seqNum` 0 on this die too | first `seqNum` != 0, `prefix_missing`, `seq_gap` or `parse_failure` in `kf-census` on the mailbox-attached instance |
| G3 | `INTERNAL_GMMU_GET_STATIC_INFO` is byte-identical to AD103 (Ampere and Ada share the page-table format) [I before the run] | reply bytes differ |
| G4 | The class list (`GPU_GET_CLASSLIST_V2`) differs from AD103 (Ampere has its own 3D/compute class ids) [I before the run] | reply bytes equal |
| G5 | The 27 static-info rows that are stable across AD103 boots are die-specific in at least the GR rows (`KGR_GET_INFO`, `GET_CONTEXT_BUFFERS_INFO`) | those replies are byte-identical to AD103 |

## Boxes

| die | box | offer | $/h | rented | destroyed | cost |
|---|---|---|---|---|---|---|
| GA104 (RTX 3070) | 55835609 | 55562112 | 0.17 | 02:19 CEST (created *stopped*: lane B rented the same machine's GPU in the same minute; `vastai start` at 02:38 got it) | | |

Box facts [M]: KVM VM, 15 vCPU, 24 GiB RAM, 146 GB root (Linux side before the reinstall), `lspci`: GA104 [GeForce RTX 3070].
windows-on-vast `install.sh` as the on-start script (image `vastai/kvm:ubuntu_cli_22.04-2025-05-16`, disk 150).

## Files

`allow_static_20261014.txt` (scrub allow-list addition), `rows_lane_c.txt` (the rows lane C asked for),
`compare_values.py` (die vs reference from census + bodies TSVs only), `gsp_on.ps1` (registry only; finds the
NVIDIA class subkey), `restart_gpu.ps1` (never prints the instance id: it carries the subsystem id),
`dieinfo.ps1` (device id, driver, GSP firmware; `nvidia-smi -q` filtered on the box), `start.ps1` (detached collector).
