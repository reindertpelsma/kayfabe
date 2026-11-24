# Camera Raw "Select Background" on the pc: bare-metal VFIO control and kf3 rerun with PR #565

**STATUS: LIVE, 2026-11-24 (preregistration, written and committed before any run of this session; results in README.md
of this directory).**

## Question
On 2026-11-21 Camera Raw "Select Background" stalled under kf3 on the pc (binary 8a4bd4b0). The trigger was the Camera Raw
Filter on a JPEG in Photoshop. In the kf3 log, walk slot 25 (root 0x1ece5a000) was cut at a per-space limit
(`refuse_mask=0x1000`). kf-mem treated the refusal as absence and completed the guest's invalidate, the host logged real-GPU
PDE faults, and the Camera Raw dialog hung for 12 min. PR #565 (head 9d143630) makes a cut a hard failure: the split
fails, the invalidate is held, and a `walk per-space CEILING` line is logged.

The owner's questions:
- Is the stall kayfabe's?
- Does bare metal complete the same action?

## Arms (one at a time; same guest; same input)
- Guest for every arm: a throwaway thin qcow2 overlay on the image B add-on overlay
  (`propimage_b/runs/b2_addon_cand/run/clone.qcow2`, which is not modified). Copies of its OVMF_VARS and swtpm state, the
  same VM identity, offline (`restrict=on`, no route), the host's real clock, and no sign-in. The test-assets ISO is
  attached read-only as a CD-ROM.
- **V (H1): bare-metal VFIO passthrough.** Stock `/usr/bin/qemu-system-x86_64`. The real RTX 4070 (01:00.0 and .1) on
  vfio-pci, bound and restored by `scripts/bench/pc_bind.py` with the guards of PR #586.
- **K (H2): kf3 at 9d143630.** The head of PR #565, `kf3-bins/9d143630`. It holds the walk-cut ceiling line; confirmed
  with `strings`.
- **C (H3): kf3 at 647ba52f.** The `next` parent of 9d143630's merge, so the same tree without #565
  (`kf3-bins/647ba52f`). The control for K, run only if cheap.
- kf3 settings as on 2026-11-21: fb-mb 8192 and `KF3_RPC_TRACE=0`. The display is QEMU VNC on a unix socket, not the broker
  window.

Order: K, then C, then V. The bind comes last, so its restore ends the GPU work of the session.

## Actions (inputs)
- **D (primary):** Photoshop opens `raw/dji_fc7303_0765.dng` (CC0, raw.pixls.us) from the test-assets ISO. The file opens
  in the Camera Raw dialog. Then Masking > Select Background (mouse; positions read from guest screenshots).
- **J (the 11-21 trigger, secondary):** Photoshop opens `C:\Windows\Web\Wallpaper\ThemeA\img20.jpg`, then Filter > Camera
  Raw Filter (`t_acr.jsx`), then Masking > Select Background.
- Launchers set the working directory to the application's folder. The 2026-11-21 driver scripts are used:
  `drv.py`, `shot.ps1`, `inp.ps1`, `psjs.ps1`.

## Outcome classes (per arm and action, decided from the evidence below)
- **COMPLETE:** within 10 min a mask is shown, or Camera Raw shows its own verdict (e.g. "No background detected"); the
  dialog answers Cancel; no TDR.
- **CLEAN-RESET:** a guest TDR (System event 4101 from nvlddmkm, or a LiveKernelEvent 141/117 report) or an application
  crash/close within 10 min, the guest stays reachable, and the kf3 log shows the failure (`DEAD` or the ceiling line).
- **SILENT-HANG:** the dialog is frozen or black for 10 min with no TDR and no app crash.
- **NOT-EXERCISED:** a kf3 arm whose log has no walk cut (`CEILING` in K, `walk report CUT`/refusal lines in C) for this
  action.

## Hypotheses and falsifiers
- **H1 (bare metal completes):** V-D and V-J are COMPLETE.
  - Falsifier: V is SILENT-HANG or CLEAN-RESET. Then the stall is not kayfabe-only (an application, driver or guest
    issue).
  - If V-J shows "No background detected" as on 11-21 while the UI stays responsive, that message is the application's
    verdict on that image, not a kayfabe defect.
- **H2 (#565 fails closed):** in K, the action whose log shows a walk cut ends CLEAN-RESET, not SILENT-HANG.
  - Additionally: no host Xid 31 in `dmesg` after the cut.
  - Additionally: the ceiling line is logged, and its first line names a root, a slot, `refuse_mask` and a reported need
    (expected: about 0x1000, need of order 10^6).
  - The ceiling line appears once per cut event. Repeats are rate-limited: the first 8, then powers of two.
  - Falsifier: SILENT-HANG with a ceiling line in the log; a host Xid 31 after the cut; or a hang with no ceiling line
    while the walk is cut.
  - K is COMPLETE with no cut: H2 is NOT-EXERCISED for that action. That is not a pass.
- **H3 (the control reproduces 11-21):** C-J (and C-D, if cut) is SILENT-HANG.
  - Expected with it: `walk report CUT` / `CLEARED OVER ABSENCE` lines and host Xid 31 `FAULT_PDE`.
  - Falsifier: C completes. Then the 11-21 stall does not reproduce on this input, and K's comparison is weak.
- **Owner question (inference rule, fixed now):**
  - "The stall is kayfabe's" is supported only if V completes and C hangs on the same action.
  - V hangs too: it is not kayfabe's.
  - V completes but C also completes: undecided (not reproduced).

## Evidence collected per run (scrubbed before commit)
- Host: `dmesg` lines since the run's start, filtered for Xid, NVRM, vfio, DMAR and AER; GPU UUIDs and serials masked.
- kf3: the log lines with `CEILING`, `CUT`, `DEAD`, `refuse`, `CLEARED OVER ABSENCE` and `Xid`, plus their counts.
- Guest:
  - System events 4101 and LiveKernelEvent reports;
  - Application events 1000/1002 for Photoshop;
  - whether the Photoshop process responds.
- Screenshots: small PNGs. The result only, cropped or downscaled. No photo content beyond the dialog, and no account data.
- Timing: action start, first screenshot after it, and the outcome time.

## Stop rules and safety
- Each action is observed for at most 10 min. A frozen dialog is not retried.
- After a stuck action: the dialog's Cancel, then `CloseMainWindow` (60 s), then the Photoshop process tree is ended. This
  is the 11-21 coordinator rule.
- Guest wall cap: 75 min per boot. All guest work ends by 15:30Z, and the restore is verified by 16:00Z.
- The pc GPU lock is held for every arm. Each arm writes PC_ACTIVE.log START/END lines. The host state is recorded before
  the bind.
- No `nvidia-smi` or nvidia load while the GPU is bound; the modprobe block enforces this.
- Restore verification:
  - nvidia 595.91.07 on 01:00.0, `snd_hda_intel` on 01:00.1;
  - gdm on card1 with DP-3 connected;
  - host `cuInit` 0;
  - no vfio modules left;
  - the udev queue empty.
- On a wedge: the SysRq ladder (s, u, b). Never a normal reboot.
- Throwaway overlays are removed at the end. Nothing else on the pc is deleted. The Adobe image never leaves the pc.
