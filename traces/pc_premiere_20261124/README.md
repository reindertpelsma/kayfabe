STATUS: LIVE, 2026-11-24. Premiere Pro sequence-crash control on the owner's pc (RTX 4070): bare-metal VFIO vs kf3, for the crash of PR #588 (`traces/imgb_offline_features_20261124/`). Preregistration `PREREGISTRATION.md`, committed at 68abe03b9 before any run, addendum 1 at b12e28321 before the variants. Labels: MEASURED (a file of this directory, 2026-11-24 runs), INFERRED (not tested).

# Result
The Vast crash does **not** reproduce on the pc, under kf3 or on bare metal. MEASURED on 2026-11-24 (`runs/attempts.tsv`): every attempt that reached its action ended PASS, 24 of 24:
- kf3 ed41beea: 17 attempts in three boots (K1, K2, K3);
- bare-metal VFIO passthrough: 7 attempts in one boot (V1).

Each attempt made a new project, imported the same `sintel_trailer_1080p.mp4` (14,621,544 B), created a 1920x1080 sequence from it, made it active and held it 45 to 120 s:
- with and without the Vast harness's ENTER/ESC key cadence;
- with Scene Edit Detection;
- at the real clock and at the Vast run's recorded clock;
- in K3, the Preview-Your-Clips Learn project was opened instead.

After each attempt, Premiere was alive and responding, with no Premiere crash record and no Application 1000 for Premiere. There was no display-driver event in the guest and no host Xid.

**Answer to the owner's question:** bare metal passes, and kf3 on the RTX 4070 passes the same actions too. The owner's rule ("bare metal passes and the guest fails") is therefore not met on this die and host, and no kayfabe defect is shown. The 4090/Vast crash stays UNATTRIBUTED. It is specific to that box, die, host, image checkpoint or flow (list below), and nothing here reproduces it. No code was changed and no test was added.

# Runs (MEASURED, 2026-11-24, pc: AMD Ryzen 9 7900, RTX 4070, host driver 595.91.07, guest driver 580.88 = 32.0.15.8088)
| run | arm | binary | guest clock | time (UTC) | attempts |
|---|---|---|---|---|---|
| K1 | kf3 | `kf3-bins/ed41beea/qemu-system-x86_64`, built on the pc from `origin/next` ed41beea1 (`build_kf3.sh`) | real | 05:59-06:35 | A x3 (A3 with 120 s hold), keys x3, SED+keys x3; 8 Learn-project attempts that never opened the project (OTHER) |
| K2 | kf3 | same | recorded, `-rtc base=2026-11-22T02:43:26` (the Vast boot's base) | 06:36-06:50 | A x3, keys x3 (after 6 void attempts before 06:41Z in which the harness directory was missing in the guest and Premiere was not started; not counted) |
| V1 | VFIO | `/usr/bin/qemu-system-x86_64` (distro QEMU), RTX 4070 01:00.0 and .1 on vfio-pci by `scripts/bench/pc_bind.py` of PR #586 (fc35c9f8a) | real | 06:50-07:02 | A x3, keys x3, SED+keys x1 |
| K3 | kf3 | same as K1 | real | 07:02-07:11 | Learn project x2 (one harness-error attempt, OTHER) |

Common setup (MEASURED, `runs/*/argv_scrubbed.txt`):
- A throwaway thin overlay on `propimage_b/runs/b2_addon_cand/run/clone.qcow2`, which was not written: mtime 2026-11-22 04:26 +0100 before and after. Its OVMF_VARS and swtpm state were copied, and all overlays were removed at the end.
- Offline: `restrict=on` in every argv; `Test-Connection` false in K1. No sign-in.
- 16 GiB, 8 vCPU, and the Vast run's virtual sound card (`ich9-intel-hda` + `hda-duplex`, `audiodev none`).
- The test-assets ISO as a CD-ROM.
- kf3: fb-mb 8192, `KF3_RPC_TRACE=0`, 1920x1080 guest display.
- VFIO: the real monitor on DP-3, guest mode 3840x2160.

Driver:
- `ppsession.py` owns the VM, the GPU lock, PC_ACTIVE, bind/restore and the verification.
- `pp.py`, `pp_run.ps1` and `pp.jsx` run the attempt. It follows PR #588's `pp_sed.jsx`: the Startup stub, working directory = the Premiere folder, and `-Keys` copies `lib.ps1` `Run-StartupScript`'s ENTER/ESC cadence.
- `attempts.py` makes `runs/attempts.tsv`.

# Hypotheses (PREREGISTRATION.md and addendum 1)
| hypothesis | verdict | evidence (MEASURED 2026-11-24, `runs/attempts.tsv`) |
|---|---|---|
| H1 bare-metal VFIO also crashes | **FALSIFIED** for this die/host | V1: action A PASS 3/3, keys PASS 3/3, SED PASS 1/1 |
| H2 bare metal passes and kf3 crashes (kayfabe defect) | **FALSIFIED** on the 4070 (falsifier "K PASS >= 2/3" met) | K1 action A PASS 3/3; K2 PASS 3/3 |
| H3 Software Only removes the crash | NOT-EXERCISED (preregistered: only where A crashes) | - |
| H4 the crash needs the 1080p clip's decode/effects | NOT-EXERCISED (same rule) | - |
| H5 the Vast harness key cadence is needed | FALSIFIED | K1 keys PASS 3/3; ENTER landed 12-13 s after the sequence step each time (`keys_sent` column) |
| H5b SED with keys (the Vast 04:06 run) | FALSIFIED | K1 SED PASS 3/3: 11 clips after detection, 9.1-11.7 s; bare metal V1 4.9 s |
| H6 the recorded clock is needed | FALSIFIED | K2 (guest UTC 2026-11-22T02:44Z at boot) action A PASS 3/3, keys PASS 3/3 |
| H7 opening the Learn project crashes (Vast crash #1) | FALSIFIED under kf3 (2 of 2 PASS); not run on V | K3 L1, L2: project opened by `app.openDocument`, 120 s held, no crash. The Vast run opened it by a double click in the home screen, which was not reproduced. |

# Crash signature
- None on the pc. In all four boots there was:
  - no Premiere SentryIO crash record;
  - no Application 1000/1001/1002 for Premiere;
  - no WER report for Premiere and no LocalDumps file;
  - no System 4101/nvlddmkm event and no LiveKernelEvent.
  The only Application 1001 events are Windows' YourPhone (`combase.dll`, c0000005) and Store update scans, all unrelated (`runs/*/crash_evidence.txt`).
- The Premiere SentryIO directory holds only `"type":"session"` envelopes and `__sentry-event` files of about 2.1-2.3 kB, written while Premiere was alive. INFERRED: the 2,201-byte "fourth crash" file of PR #588 (`g1/premiere_crash_threads.txt`) can be such a session record. Its crash time came from `last_crash` and stays a crash; its thread was never read.
- The signature on record is still the Vast one (PR #588): "Unhandled exception!" on `Display Thread` with renderer CUDA, in 2 reports; one report on `ORT_AudioClipAudioFP16-20250912`. No exception code or module was read there.

Host and kf3 (MEASURED 2026-11-24, `runs/dmesg_session_filtered.txt`, `runs/K*/kf3_log_filtered.txt`):
- no Xid in host dmesg;
- one `NVRM: unknown NVRM ioctl command: 0xf0` at each kf3 start;
- no NVRM line between the bind (07:50:10 +0100) and the restore (08:00:49 +0100);
- kf3: no walk CUT, no CEILING, no DEAD, no VA_SLOW. Refusals were leaf only: `refuse_mask=0x400` K1 0, K2 12, K3 12; `CLEARED OVER ABSENCE` K1 3, K2 9, K3 9.

# What still differs from the Vast run (INFERRED candidates; none tested here)
- **Die and host.** RTX 4090 (AD102) vs RTX 4070 (AD104). Host driver 580.159.04 in nested KVM on a Vast box vs 595.91.07 on bare metal. The guest driver is the same.
- **Image state.** The Vast object `kfimg-adobefull2-20261122T015401Z` is checkpoint 1 of the add-on session (stopped 2026-11-22T01:54:01Z, `b2_addon_cand/run/stopped_checkpoint1.json`). The pc overlay holds one more session on top (02:12Z to 03:26Z on 2026-11-22, `session.status`). So the pc runs are on a later state of the same image, not the identical bytes.
- **Boot history.** On Vast, Premiere ran after about 40 minutes of Photoshop and Camera Raw work, and after its own first crash (Learn project, 03:30Z). Every later Vast attempt therefore started after a crash. An app state written at that first crash (a media cache or preferences) would explain "3 of 3" without any GPU cause; this is untested.
- **Media path.** The clip was on a virtio data volume (`KFDATA`) on Vast and on a CD-ROM here.
- **kf3 revision.** 3581ba07 on Vast vs ed41beea here. The kf3 code diff between them is a pure refactor of one log line plus tests (`crates/kf-qemu/src/userrelay.rs` `route_declined_line`, a `chan.rs` test module), so INFERRED: no behavioural difference. The PR #565 binary (`kf3-bins/9d143630`) was not run, because nothing crashed.

# Guards and restore (MEASURED, 2026-11-24)
- **Dry run without binding** (`runs/guard_dryrun.txt`, 06:36Z): the four `install nvidia* /bin/false` lines were read back by `modprobe --showconfig`. The probe refused `--gpu` while the block was present (rc 3). The gate was quiet in 10 s, and the block was removed.
- **V1 bind** (`runs/V1/bind.txt`): gdm and persistenced were stopped and no holders were left. The audio function was released and the block installed. All four nvidia modules were removed with rc 0, and both functions were on vfio-pci.
- **V1 restore** (`runs/V1/restore.txt`):
  - the gate was quiet and the block removed; the four modprobes returned rc 0;
  - `snd_hda_intel` was back on 01:00.1 and gdm started;
  - `nvidia-smi` gave RTX 4070, 595.91.07; `udevadm settle` returned rc 0; nvidia_uvm was stable for 60 s.
- **Final host state** (07:11:36Z, after K3, lock free):
  - nvidia 595.91.07 on 01:00.0, `snd_hda_intel` on 01:00.1;
  - gdm active with gnome-shell running, `card1-DP-3` connected and enabled;
  - host `cuInit` 0; udev queue empty; no vfio module; no `/run/modprobe.d/kayfabe-vfio-bind-block.conf`.
  - `runs/V1/host_after.json` and the PC_ACTIVE END lines record the same state.
- **Cleanup:** the overlays `/workspace/pc1/winvm/pc_premiere_20261124/{K1,K2,K3,V1}` (4.4 GB) and this session's screenshots were deleted. The sealed image, the zip and `b2_addon_cand` were not touched.
- **Kept on the pc:** `kf3-bins/ed41beea` and `/workspace/bench/pc_premiere_20261124/` (logs, 25 MB).

# Next (not done here)
- A Vast RTX 4090 box under kf3 can replay the Vast flow in order: Photoshop/Camera Raw, then the Learn project from the home screen, then action A. Two controls would separate the image state from the die/host: action A on a fresh boot without the earlier steps, and the same image at checkpoint 1 under VFIO on a bare-metal 4090 (LeaderGPU) if one is rented. Falsifier for "post-crash app state": action A crashes on a fresh boot of the Vast image.
- Premiere's offline AI features can now be built on: Scene Edit Detection works offline under kf3 on the 4070 (11 cuts, K1 S1-S3).

# Files
| file | content |
|---|---|
| `PREREGISTRATION.md` | hypotheses and falsifiers (68abe03b9) and addendum 1 (b12e28321), both before their runs |
| `ppsession.py` | session driver on the pc (lock, PC_ACTIVE, overlay, VM, bind/restore, verification); from PR #587's `crsession.py` plus the sound card and `--rtc-base` |
| `pp.py`, `pp_run.ps1`, `pp.jsx` | the attempt: setup (LocalDumps, Startup stub), one attempt, crash evidence |
| `attempts.py`, `kf3log.sh` | the attempt table, the kf3 log filter |
| `runs/attempts.tsv` | one row per attempt: mode, keys, sequence step, done or process-gone time, alive/responding, outcome, keys sent |
| `runs/<run>/<attempt>_progress.txt` | the script's own step log (ms epoch); K1 A1 was transcribed from the session console (its result was read but not saved to a file) |
| `runs/<run>/crash_evidence.txt` | guest crash evidence since the run start (masked) |
| `runs/<run>/session.log`, `host_before.json`, `host_after.json`, `argv_scrubbed.txt`, `kf3_log_filtered.txt` | per boot |
| `runs/V1/bind.txt`, `runs/V1/restore.txt`, `runs/guard_dryrun.txt`, `runs/dmesg_session_filtered.txt` | the VFIO bind/restore and host kernel log |
| `runs/K1/A3_sequence_shown_cropped.png` | K1 A3 at 06:06Z under kf3: the 1080p sequence in the timeline (cropped; no account data) |

Full kf3 logs stay on the pc under `/workspace/bench/pc_premiere_20261124/runs/` (private). Screenshots of the home screen and the Learn panel show the account name and are not kept.
