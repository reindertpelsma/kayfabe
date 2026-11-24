STATUS: LIVE, 2026-11-24. Preregistration of the Premiere Pro sequence-crash control on the owner's pc (RTX 4070), written and committed before any run of this session. Results go to `README.md` beside this file; this file is not edited after the first run except by a dated addendum at the end.

# Question
PR #588 (`traces/imgb_offline_features_20261124/`, Vast box, RTX 4090, kf3 at 3581ba07) measured: Premiere Pro 27.0 in the sealed image B, booted offline with the revoked login, crashes about 2 s after a 1080p sequence made from `sintel_trailer_1080p.mp4` is shown, 3 of 3 ("Unhandled exception!" on `Display Thread`, renderer tag CUDA, in the 2 reports read). No TDR, no guest GPU event, no host Xid. Owner rule: bare metal first; if bare metal passes and the guest fails, the defect is kayfabe's.

1. Does the same action crash on a real GPU with VFIO passthrough (stock QEMU, no kf3), same image, same offline state?
2. Does it crash under kf3 on the pc's RTX 4070 (is it die- or host-specific)?

# Setup (identical in every arm unless named)
- Image: a throwaway thin qcow2 overlay on `propimage_b/runs/b2_addon_cand/run/clone.qcow2` (that file, its backing chain and the zip it backs onto are never written, moved or deleted). Copies of its OVMF_VARS and swtpm state. Overlays removed at the end.
- Offline: `-netdev user,...,restrict=on`, no route. Host's real clock. No sign-in to anything.
- Guest RAM 16384 MiB, 8 vCPU, and the virtual sound card of the Vast run (`ich9-intel-hda` + `hda-duplex`, `audiodev none`, as `propimage.py` `AUDIO_ARGS`), in every arm, so the audio device is not a difference between Vast and pc.
- Media: the test-assets ISO (read-only CD-ROM), the same `sintel_trailer_1080p.mp4` (14,621,544 B) the Vast run used.
- Action A (the repro, from PR #588's `pp_sed.jsx` in its `nosed` mode): Premiere started with its install folder as the working directory; through the Startup stub, a new project, import the clip, `createNewSequenceFromClips`, make it the active sequence, select the clip, hold 45 s, write a done marker. No Scene Edit Detection.
- Arms:
  - K: kf3 QEMU built from `origin/next` ed41beea1 (`kf3-bins/ed41beea/`), fb-mb 8192, `KF3_RPC_TRACE=0`.
  - V: stock `/usr/bin/qemu-system-x86_64`, the RTX 4070 (01:00.0 and 01:00.1) on vfio-pci, bound and restored by `scripts/bench/pc_bind.py` of PR #586 (fc35c9f8a) after a dry run without binding.
  - S (optional, only if time remains): kf3 from PR #565 head 9d143630 (already built on the pc), action A only.

# Outcome classes per attempt (decided from data, not from the screen alone)
- CRASH: the Premiere process ends within 120 s after the script logged the sequence step, without the done marker, and a new Premiere crash record appears (SentryIO `last_crash` time or a new report) or an Application 1000 event for `Adobe Premiere Pro.exe`.
- PASS: the done marker is written after the 45 s hold, the process is still alive and responding, and no new crash record exists.
- HANG: neither within 300 s of the sequence step. OTHER: anything else (for example no sequence was made); OTHER attempts do not count.
- Captured per attempt: `pp_progress.txt`, the crash record (time, size, `gpu-renderer` tag, crashing thread breadcrumb), Application 1000/1001/1002 and WER report headers (exception code, faulting module, offset), dump size and exception code only (dumps stay in the guest overlay and are deleted with it), System 4101/nvlddmkm events, host dmesg/Xid for the window, kf3 log counts (K).

# Hypotheses and falsifiers
- H1 (bare-metal VFIO also crashes; cause is the application/offline state or the guest setup, not kayfabe). Prediction: V gives CRASH in at least 2 of 3 attempts of action A with the same crashing thread class. Falsifier: V gives PASS in at least 2 of 3 attempts.
- H2 (bare metal passes and kf3 crashes; a kayfabe defect). Prediction: V PASS >= 2/3 AND K CRASH >= 2/3. Falsifier: V CRASH >= 2/3 (then H1), or K PASS >= 2/3 on the 4070 (then the Vast crash is not reproduced on this die/host: report as die- or host-specific and not attributed; the 4090 has no bare-metal arm here).
- H3 (with the Mercury Playback Engine set to Software Only the crash disappears; isolates the CUDA render path). Run only in the arm(s) where action A crashes. Precondition: the renderer switch is verified by a readback (the project's renderer setting or the crash/telemetry `gpu-renderer` tag reading software); without the readback H3 is NOT-EXERCISED. Prediction: action A with Software Only gives PASS in at least 2 of 2. Falsifier: CRASH in at least 1 of 2 with the readback showing Software Only (then the CUDA renderer is not required; the display path is suspect).
- H4 (the crash needs the 1080p clip's GPU decode/effects). Run only where action A crashes. Variants of action A with a different source, 1 attempt each, 2 if the first is ambiguous: (a) `bbb_trailer_720p.mov` (720p, other codec container); (b) the still image `lady_fern_4845x2980.jpg` from the ISO (no video decode, no audio); (c) `smptehdbars_2160p25.mp4` (video decode, expected without an audio stream; checked with the clip's properties). Prediction: (b) PASS while action A crashes. Falsifier: (b) CRASH (then neither decode nor the clip is required: any shown sequence crashes).

# Decision rule for the owner question
- "kayfabe's defect" only if H2 holds: V PASS >= 2/3 on the same action and K CRASH >= 2/3. Then capture the crash signature and look for the kf3-side cause; a fix needs the smallest GPU-free Rust test that fails before and passes after.
- If H1 holds: not kayfabe's (the bare-metal reference fails too).
- If K passes on the 4070: not reproduced; the Vast result stays unattributed and die/host-specific (4090 box, nested KVM).
- Measured and inferred are kept apart in the README.

# Safety (binding for this session)
- GPU lock `/root/prov/pc_gpu.lock` held for each arm, PC_ACTIVE START/END lines. Restore and verify before releasing the lock and by 16:00Z at the latest: nvidia 595.91.07 on 01:00.0, `snd_hda_intel` on 01:00.1, gdm active with `card1-DP-3` connected, host `cuInit` 0, udev queue empty, no vfio module, no modprobe block file.
- No `nvidia-smi` or any nvidia tool while a bind may be active. SysRq s,u,b only for a wedge.
- Nothing on the pc is deleted except this session's own overlays and build scratch.
