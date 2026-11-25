# Camera Raw "Select Background" control on the pc: bare-metal VFIO vs kf3 with PR #565

**STATUS: LIVE, 2026-11-24 (run 02:53Z-03:24Z on the owner's pc, RTX 4070, host driver 595.91.07; preregistration
`PREREGISTRATION.md`, committed at deca27fc0 before any run, plus an addendum at 87e8a6f89 before the exploratory run).**

## Result in one paragraph
The offline image cannot run Camera Raw's AI selection. Select Background and Select Subject both end at once with
"Something went wrong. Unable to load AI selection model". This happens on the DNG and on the 11-21 JPEG path, under kf3
and on **bare-metal VFIO passthrough alike**. There is no mask, no hang and no TDR. So the action never reaches the GPU
work that cut walk slot 25 on 2026-11-21, and **H1, H2 and H3 are NOT-EXERCISED**. The owner's question ("is the stall
kayfabe's; does bare metal complete it") stays open: it needs the action to load its model, which the offline,
signed-out image does not allow.

What was measured (2026-11-24, RTX 4070):
- kf3 and bare metal behave identically on this action offline.
- In the exploratory arm, Camera Raw AI Denoise on the real raw DNG **completes under kf3 9d143630**. The guest's GPU
  was busy during it, which is an inference from the host trap counters. No walk cut, no host Xid.
- The first bind and restore with the guards of PR #586 was clean. During the bind no nvidia module loaded, and at the
  restore the udev queue was quiet at once.

## Runs (revisions and binaries)
| run | arm | binary | revision | time (UTC) |
|---|---|---|---|---|
| K1 | kf3 | `kf3-bins/9d143630/qemu-system-x86_64` | 9d1436305 (head of PR #565; holds the `walk per-space CEILING` line, checked with `strings`) | 02:53-03:04 |
| V1 | VFIO | `/usr/bin/qemu-system-x86_64` (distro QEMU) | real RTX 4070 01:00.0 and .1 on vfio-pci; bind and restore by `scripts/bench/pc_bind.py` at 2b4f74778 (PR #586) | 03:04-03:13 |
| K2 | kf3 (exploratory E) | `kf3-bins/9d143630` | 9d1436305 | 03:13-03:24 |
| C | kf3 at 647ba52f (next without #565) | built, `kf3-bins/647ba52f` | 647ba52f1 | **not run** (addendum: Select Background cannot reach the walk) |

All runs used the same guest setup:
- A throwaway thin qcow2 overlay on the image B add-on overlay, which was not modified (mtime still 2026-11-22). All
  three overlays were removed after the runs.
- Copies of the overlay's OVMF_VARS and swtpm state, and the same VM identity.
- **Offline**: `restrict=on`, no route. The host's real clock. No sign-in.
- The test-assets ISO as a read-only CD-ROM: `raw/dji_fc7303_0765.dng`, CC0, raw.pixls.us.
- kf3 settings: fb-mb 8192 and `KF3_RPC_TRACE=0`, as on 2026-11-21; VNC on a unix socket instead of the broker window.
- Driver: `crsession.py` (VM, GPU lock, PC_ACTIVE lines, bind and restore), and `cr.py` / `cr_open.ps1` (UI steps
  through the 2026-11-21 helpers `drv.py`, `inp.ps1`, `shot.ps1`, `psjs.ps1`, `t_acr.jsx`). Photoshop is started with
  its install folder as the working directory.

## Per action (MEASURED 2026-11-24 unless marked)
| arm | action | outcome | evidence |
|---|---|---|---|
| K1 | D: DNG > Camera Raw > Masking > Background (02:57:01Z) | Message "Unable to load AI selection model" within 20 s; the dialog answers. Select Subject gives the same message. | `runs/K1/D_dng_select_background.png`, `runs/K1/kf3_log_filtered.txt` |
| K1 | J: img20.jpg > Camera Raw Filter > Masking > Background (03:02:07Z) | Same message within 30 s; Photoshop responding | `runs/K1/J_acr_filter_select_background.png` |
| V1 | D (03:07:26Z) | **Same message on bare metal** | `runs/V1/D_dng_select_background.png` |
| V1 | J (03:10:21Z) | **Same message on bare metal** | `runs/V1/J_acr_filter_select_background.png` |
| K2 | E: Lens Blur > Apply on the DNG (03:17:18Z; not preregistered: tried first because it was on screen) | "Unable to load Lens Blur AI depth estimation model" | `runs/K2/E_lens_blur.png` |
| K2 | E: Detail > Denoise on the DNG (03:19:34Z; the addendum's action) | **COMPLETE**: "Applying Denoise" was at 90 % at 03:21:28Z. By 03:22:46Z the dialog was gone and Denoise was checked at amount 50. | `runs/K2/E_denoise_done_panel.png` |

Measured in the same runs (2026-11-24, kf3 at rev 9d1436305):
- kf3 logs (both kf3 runs):
  - no `walk per-space CEILING`, no `walk report CUT`, no `refuse_mask=0x1000`, no `DEAD`, no `VA_SLOW`;
  - leaf refusals only: `refuse_mask=0x400`, K1 143 and K2 179;
  - `CLEARED OVER ABSENCE` K1 29 and K2 32. This is the absence path that #565 keeps for leaf refusals, not a cut.
- Guest, all runs (`runs/*/guest_events.txt`):
  - no display TDR (System 4101) and no LiveKernelEvent report;
  - no Application 1000/1001/1002;
  - Photoshop `Responding=True` at the end. The IDs 14 and 153 in those files are boot messages caught by the broad
    filter.
- Host `dmesg` for the session window (`runs/dmesg_session_filtered.txt`):
  - **no Xid**;
  - one `NVRM: unknown NVRM ioctl command: 0xf0` at the start of each kf3 run (K1 and K2). [inferred: from kf3's or the
    cuInit probe's start-up, not from the actions]

INFERRED:
- The model-load failure is the image's offline, signed-out state, not kayfabe. The basis is that bare-metal
  passthrough gives the same message, and that the same overlay loaded these models online on 2026-11-21 (Sky mask,
  Lens Blur, and the Select Background that stalled).
- Which check refuses (entitlement, model decryption or a manifest refresh) was not determined. Camera Raw's own log
  stayed empty.
- That AI Denoise ran on the GPU under kf3 rests on host counters during it: trapped +520k, walks +2,175, 18 new
  Windows user rings. The guest-side GPU census was not taken.

## Hypotheses (PREREGISTRATION.md)
- **H1 (bare metal completes Select Background): NOT-EXERCISED.** V1 shows an application refusal, not COMPLETE,
  SILENT-HANG or CLEAN-RESET.
- **H2 (#565 fails closed on a cut): NOT-EXERCISED.** No walk cut in K1 or K2. The ceiling line never appeared, so its
  numbers are unverified.
- **H3 (the control reproduces the 11-21 hang): NOT RUN.** By the addendum, it could not reach the walk either.
- **Owner question:** undecided. By the preregistered rule, "kayfabe's" needs V to complete and C to hang on the same
  action. Neither ran the action's GPU work.

## Guard verification (PR #586, first real use)
Dry run before any bind (`runs/guard_dryrun.txt`):
- `modprobe --showconfig` read back all four `install nvidia* /bin/false` lines;
- a `/run/modprobe.d` install line was honoured (a dummy module's modprobe ran `/bin/false`, rc 1);
- the gate was quiet in 10 s;
- the block was removed;
- the probe said `bind_active=yes` and refused `--gpu` (exit 3) while the block was present.

V1 bind (`runs/V1/bind.txt`):
- the block was installed before `rmmod nvidia_uvm`;
- all four nvidia modules unloaded with rc 0;
- both functions went to vfio-pci;
- no nvidia module was loaded while bound (lsmod checked at 03:11Z, no NVRM line in dmesg between the bind and the
  restore).

V1 restore (`runs/V1/restore.txt`):
- the gate was quiet without the ladder, the block was removed, and nvidia, modeset, drm and uvm loaded with rc 0;
- `snd_hda_intel` was back and gdm started;
- the post-restore `nvidia-smi` gave RTX 4070, 595.91.07;
- `udevadm settle` returned rc 0 (it timed out after the 2026-11-24 00:54Z bind) and nvidia_uvm settled.

One gap: `vfio_iommu_type1` and `vfio` stayed loaded. They were unloaded by hand at 03:14Z (rc 0, a PC_ACTIVE line),
and the restore now unloads them (fc35c9f8a in PR #586, test `test_restore_unloads_every_vfio_module`, which fails
before the fix).

## Restore verification (final host state, 03:24Z; `runs/K2/host_after.json`, PC_ACTIVE END lines)
- nvidia 595.91.07 on 01:00.0, `snd_hda_intel` on 01:00.1.
- gdm active and gnome-shell running; `card1-DP-3` connected and enabled.
- Host `cuInit` 0. No vfio module loaded.
- Udev queue empty; no modprobe block present.
- GPU lock released.

## Files
| file | what it holds |
|---|---|
| `PREREGISTRATION.md` | the plan and falsifiers, with the addendum |
| `crsession.py` | session driver: lock, VM, bind/restore, verification |
| `cr.py`, `cr_open.ps1` | the UI steps |
| `runs/<run>/` | `session.log`, `host_before.json`, `host_after.json`, `argv_scrubbed.txt` (identity, GPU UUID, ports and subnet masked), `guest_events.txt`, the click times, the cropped result PNGs (only the message dialog or panel; no account data), `kf3_log_filtered.txt` (counts and the matching lines) |
| `runs/V1/bind.txt`, `runs/V1/restore.txt` | the bind and restore logs |
| `runs/dmesg_session_filtered.txt` | host `dmesg` for the session window |
| `runs/guard_dryrun.txt` | the dry run of the guards |

Full kf3 logs and screenshots stay on the pc under `/workspace/bench/pc_camera_raw_20261124/` (private; Adobe UI).

## Next (not done here)
- Exercising Select Background needs the model to load: an online, signed-in session, which the owner has to decide on;
  this brief forbade it. Alternatively, an offline-loadable heavy ML path that reaches about 10^6 runs in one space. AI
  Denoise on the 9 MP DNG did not; Denoise with Super Resolution on a larger raw, or a batch, might [untested].
- The C arm becomes useful only once some action reaches a walk cut.
