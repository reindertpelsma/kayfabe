# Constants measurement lane C, resumed (2026-10-14 daytime, branch `claude/constants-measure-20261014b`)

**STATUS: LIVE, 2026-10-14 (in progress; written as the run goes).** Labels: **[M]** measured by a command whose output is in this
directory, **[I]** inferred, **[U]** not measured. Every hypothesis has its falsifier written before the run.
No UUID, serial, VBIOS string, MAC or PCI subsystem id is in any file here (grep before each commit). Capture files stay on the host
(`/root/kfwc-captures-20261014/`, not in git; `/data` is full), only census text, scrubbed body tables and sidecars are committed.

## Resumption notes

The first lane-C agent died of the weekly limit at ~03:15 CEST. Its Windows rentals (GA104 55835609, GB203 55838542) were destroyed by the
auto-destroy timers with **no capture brought back** (no `/data/kfwc-*` directory exists; the AD103 capture files of
`traces/real_windows_20261014/` are likewise gone from the host, only their census text survives). This directory is the second attempt.

## Windows run on GA104 (RTX A4000, vast KVM VM, Windows 11 LTSC 26100)

Box: vast contract 55921418 (own id), RTX A4000, PCI device 0x24B0 = GA104, host (Linux side, before the reinstall) driver 575.51.03.
**Finding W0 [M]**: `windows-on-vast`'s driver step failed (`setup.exe exited -436207360`) because Windows Update had already installed
NVIDIA **596.51** (`32.0.15.9651`, WHQL, dated 2026-05-07) on its own. So this box runs a driver *newer* than the 580.88 of the AD103/AD104 captures,
i.e. a different GSP branch. That is a bonus for F-4 (595-branch request shapes) and a deviation from the like-for-like 580.88 reference.

| # | hypothesis | falsifier (written before the run) |
|---|---|---|
| W1 | the 596.51 GeForce driver runs the A4000 on GSP when `EnableGpuFirmware=1` is in both keys and the adapter is restarted | `nvidia-smi -q` GSP Firmware Version still `N/A` after the restart, or the collector attaches to nothing |
| W2 | the mailbox route attaches from `seqNum` 0 on this die and driver | `prefix_missing`, first `seqNum` != 0, `seq_gap` or `parse_failure` on the mailbox-attached instance (`kf-census`) |
| W3 | F-4: the controls 852e/853a/a618/852f/853b/b201 changed between firmware branches (host 595.91.07 answers them INVALID_ARGUMENT to the 580-branch requests) | the 596.51 driver sends the same all-zero requests as 580.88 does: then the branch explanation is wrong and the 595.91.07 refusal is a host-driver property |
| W4 | GA104 and AD104 GMMU static info differ (`0x20800a59`); kayfabe's authored GA106-equal fault-buffer sizes are not the GA104's | the GA104 reply equals the authored/AD104 values |
