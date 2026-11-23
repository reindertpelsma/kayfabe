# VFIO control session on the pc (RTX 4070), 2026-11-18

STATUS: LIVE, 2026-11-18 (session FINAL: Resolve rows DONE, Wukong kf3 arm DONE, Wukong VFIO arm NOT RUN (no second bind cycle without a new owner decision); the pc is idle). Text evidence only.
Run: owner-approved supervised VFIO control session (ONE session); the pc is the only machine with the same die for VFIO and kf3 (RTX 4070, host driver 595.91.07, guest driver 580.88).
kf3 revision of every kf3 arm AND of the QEMU binary used by the VFIO arm: `6bfa2dbc` (newest installed on the pc, built 2026-11-16 18:59, binary sha256
`e30ece557b31994bc651065a7ca37a9688da9dffdc2d5c17b3ca69721f7cdaca`, `ev/rev.txt`). Image A offline throwaway overlays (restrict=on, recorded guest clock 2026-11-12), one fresh clone per attempt.
Rule: kf3 FAIL + VFIO PASS = kayfabe defect; both FAIL = application/harness; both PASS = not a kayfabe finding on this die (flaky elsewhere).
MEASURED = a line in a file named here. INFERRED = reasoning, marked.

## Row 1: Resolve.exe crash in nvoglv64.dll about 3-4 min after project load (seen 3 of 3 on a kf3 RTX 4090 Vast box)

Falsifier stated before the run (`HYPOTHESES.md`): Resolve.exe crashes in nvoglv64.dll on VFIO in at least 1 of 3 attempts, then it is not a kayfabe finding.

| row | image | die | platform | kf3 rev | attempts | pass count (no crash in the window) | durations | kf3 census (Xid, refused/unserviced) | key app log lines | classification |
|---|---|---|---|---|---|---|---|---|---|---|
| resolve | A | 4070 | vfio | 6bfa2dbc (QEMU only) | 3 (s1v) | 3 of 3 | 1139, 1148, 1122 s; Resolve alive at the last poll 936, 940, 915 s after start | n/a | no Application Error 1000, no Resolve/nvoglv64 event, no LiveKernelEvent/nvlddmkm after the recorded clock base | no crash on VFIO |
| resolve | A | 4070 | kf3 | 6bfa2dbc | 3 (s2k) | 3 of 3 | 1318, 1334, 1328 s; alive 1131, 1142, 1142 s | Xid 0; unserviced 3036/3045/3041 (code 76); refused 856/873/786 | same: none | no crash on kf3 |

Per-attempt table: `resolve_table.md` (made by `scripts/analyze_resolve.py` from the full per-attempt evidence including the guest event logs, which are not committed; the committed `ev/` holds the soak, census, dmesg and verdict files).
**Classification: both pass on this die. Not a kayfabe defect, and the crash is not reproduced on the 4070; the 4090 crash is Vast-side (die, nested KVM, or host driver 580.159 against 595.91), which this session does not separate.**
Caveats (MEASURED): the row's own verdict is FAIL on both platforms at the render steps (UI driver coordinates; VFIO fails already at the h264 Format step, kf3 passes h264 and fails dnxhr/prores/png), so the control covers load plus idle plus a 600 s soak, not a finished export. 3 attempts per platform on one die.
The Xid column: `dmesg_new_filtered.txt` holds 0 `Xid` lines in all six attempts.

## Row 2: Wukong benchmark (image A): kf3 36 FPS against 93 FPS VFIO earlier, and the Wukong TDR (LiveKernelEvent 141/117, nvlddmkm 153)
Falsifier (HYPOTHESES.md): VFIO shows a TDR/LKE event in at least 1 of 3 attempts, then not kayfabe.
**The VFIO arm of this row was NOT run in this session** (it needs a second bind cycle; the first one ended in the nvidia-uvm hang below, so it needs a new owner decision). The kf3 arm ran; the VFIO reference is the OLDER session, not this session.
VFIO reference (older session, previous agent, same image and settings Very High, DLSS + frame generation on, 3 of 3 results, 0 TDR): 93 / 92 / 93 FPS, cited from `pcnight2_20261115/README_final.md` line 25 and `pc_wukong_20261115/PC_WUKONG_20261115.md` lines 9-11 (coordinator evidence, not committed; the figure is also in `docs/OWNER_RULINGS.md`, "36 vs 93 FPS"). kf3 at 10240 MiB earlier: 36, 35, 36, 36 FPS.

kf3 arm, session s3k (image A, 4070, kf3 rev 6bfa2dbc, image defaults: FG on, guest framebuffer at the harness default 4096 MiB, one boot, 3 attempts inside it, 08:01-08:25Z; evidence `ev/s3k_wukong_kf3/`):

| row | image | die | platform | kf3 rev | attempt | result | duration | TDR events | kf3 census in the attempt window | key app log lines |
|---|---|---|---|---|---|---|---|---|---|---|
| wukong | A | 4070 | kf3 | 6bfa2dbc | 1 | game exited, no result | 19-22 s after Confirm | YES: LiveKernelEvent 141 x2, nvlddmkm 153 x3 | M1 OFA channel dead x1 (`chan token ... ForeignClass`), chan DEAD x1, SLOW host unmap x1 | `LKE count 0->2: first GPU watchdog event`, `END game_exited` |
| wukong | A | 4070 | kf3 | 6bfa2dbc | 2 | result screen reached | 462 s after Confirm (612 s since launch) | none new | SLOW host unmap x4, SPLIT-LATE x7, SLOW-APPLY x17, no DEAD | live FPS readings 3 to 5 FPS (OCR, `ev/s3k_wukong_kf3/summary.json` fps_series 4); result screen shows "Detected low VRAM. Benchmark results may significantly differ" |
| wukong | A | 4070 | kf3 | 6bfa2dbc | 3 | game exited 77 s after launch, before the benchmark | n/a | none new (no GPU watchdog event; the two LKE lines are attempt 1's) | none | `game process gone`, VRAM peak 431 MiB; cause not diagnosed (INFERRED nothing) |

Boot totals (MEASURED, `analysis.txt` and the kf3 census of the whole boot): unserviced 2053 (code 76), refused 934, dead channel 3, foreign class 3, host Xid 0, walk capacity ceiling 0, walk report CUT/TRUNCATED 0.
Pass count: 1 of 3 reached the result screen (a slow one, 3 to 5 FPS at 4096 MiB), 1 of 3 TDR, 1 of 3 exited at start. Attempt 1 repeats the known kf3 signature (OFA channel dead then watchdog, 22 s after Confirm), the same as a1 of the 2026-11-16 boots on rev fb274c4b.
**Classification (rule applied to the older VFIO reference, which is not a same-session control): kf3 fails and VFIO passed (3 of 3, 93 FPS, 0 TDR) = kayfabe defect candidate; the kf3-side signature (dead OFA channel, then watchdog) supports it. Not confirmed by a same-session VFIO arm.** The 4096 MiB guest framebuffer is a known confounder for FPS (10240 MiB gives 36 FPS); the TDR in attempt 1 is not explained by that.

## Finding: late udev unload of nvidia-uvm after a CLEAN restore (the likely cause of the earlier pc wedges and of "cuInit 999 right after a restore")
MEASURED (`ev/s1_after_restore_uvm_hang.txt`, `ev/s1k_cuda_ready.txt`): after the VFIO session s1v restored cleanly at 04:02:16Z (all rmmod/modprobe rc=0, nvidia-smi OK), `nvidia_uvm` was in initstate `going`, refcnt -1, with `modprobe -r nvidia-uvm` (child of a udev-worker) in D state at `rcu_barrier <- kmem_cache_destroy <- uvm_tools_exit <- uvm_exit <- delete_module`, new instances every few seconds. A kf3 session started in that minute got `cuInit 999`.
The unload comes from the stock Ubuntu rule `/usr/lib/udev/rules.d/71-nvidia.rules`: line 18 `ACTION=="add", DEVPATH=="/bus/pci/drivers/nvidia", RUN+="/sbin/modprobe nvidia-uvm"`, line 19 `ACTION=="remove", DEVPATH=="/bus/pci/drivers/nvidia", RUN+="/sbin/modprobe -r nvidia-uvm"`.
INFERRED: the remove event from our `rmmod nvidia` is handled late and races the reload; it unloads uvm under a freshly loaded, in-use driver. Falsifier: after the fix below a restore still leaves uvm `going` longer than 120 s.
Action taken: SysRq ladder (s, wait 15 s, u, wait 5 s, b) at about 04:05Z with coordinator approval; the pc came back at 04:06:39Z with uvm live. Host config untouched (no udev drop-in).
Fix in `scripts/pcsession.py` (24 tests, `scripts/test_pcsession_bind.py`, GPU-free): after every restore and before every kf3 start `wait_host_modules_settled` runs `udevadm settle`, then waits until no nvidia modprobe/rmmod runs and `nvidia_uvm` is live with refcount >= 0 (or absent, then one `modprobe nvidia_uvm`) for 60 s; `going` or refcount < 0 for more than 120 s raises a wedge (stop, SysRq ladder, no second unload). Measured after the reboot: `ev/s2k_host_settle.txt` (settled), `ev/s2k_cuda_ready.txt` (cuInit 0). The fix is NOT yet proven on a restore after a VFIO bind (that needs a second bind cycle).

## Other changes made to the session driver (coordinator-approved, 2026-11-18)
- The GPU's audio function 01:00.1 is bound to snd_hda_intel, and the rewrite aborted on any bound function after the nvidia unload. Now `release_audio` releases that one function first through snd_hda_intel's own unbind file (60 s child, D state = wedge-lite, abort otherwise, no nvidia unbind anywhere), the restore binds it back, and a bind abort after a module unload runs the restore (`recover_after_bind_abort`).
- Dry runs before the first VFIO boot: `ev/dryrun_services.txt` (gdm off, nvidia_drm refcount 0 at once), `ev/dryrun_audio.txt` (audio function released, re-probed, card back, nvidia-smi OK).
- VFIO bind of session s1v: `ev/s1v_bind.txt` (clean); restore: `ev/s1v_restore.txt` (clean, audio back on snd_hda_intel).

## Files
`HYPOTHESES.md` (falsifiers before each row, the finding), `queue.txt`, `resolve_table.md`, `ev/s3k_wukong_kf3/` (Wukong kf3 arm), `scripts/` (session drivers, analysis, tests), `ev/` (per-attempt summary.json, census.json, soak.txt, filtered dmesg, row verdict table).
Not committed: qemu logs, guest event logs, screenshots, argv with identifiers, anything from the proprietary images beyond the row verdict lines.
