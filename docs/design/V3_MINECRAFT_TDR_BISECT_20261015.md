**STATUS: RESEARCH, 2026-10-15.** INTERIM: the series stopped at the owner's request (the owner started playing in VM 2990 at 17:53 host time); ten of the planned cells are not run (see "Not done").

# Minecraft TDR bisect (VM 2990, Win11 guest, RTX 4070 Ti SUPER host)

Question: is the Minecraft guest TDR seen on `a07a9ba0` (run 2990, 16:28 host time) (1) caused by the kayfabe builds after `64449dc5`, (2) a launch-parameter effect, or (3) a pre-existing intermittent defect? It gates the cursor branch.

## Verdict so far

**Not a regression introduced by a07a9ba0 / 666ea966, with one open check.** Minecraft idle at spawn, no PresentMon, fresh guest boot: 3 of 7 trials TDR'd, on three different builds (`a07a9ba0`, `64449dc5`, `666ea966`), always about 101-106 s after the launch, with the same guest signature. `eb9f2aa3` (the pre-cursor production build) is 0 of 1, which is not enough to say anything: at the observed rate (3/7) four clean runs would still happen 1 time in 10 by chance. The check that decides it: 4-5 idle controls on `eb9f2aa3` and `2f99e673` (see "Next").

A second finding: starting PresentMon in the guest while Minecraft runs brought the TDR forward to 3-4 s after PresentMon started (3 of 3 fresh-boot trials, builds `a07a9ba0`, `64449dc5`, `64449dc5`; 0 of 1 on `666ea966` in a guest that had already TDR'd). The first crash was this: the earlier agent started PresentMon at 14:28:45 guest time.

## What the first crash actually was

[read, guest PowerShell log + kfagent queue] at 14:28:45.2 guest time, 41 s after javaw started, the earlier agent ran `powershell -ep bypass -File C:\kf\presentmon_minecraft.ps1 -Timed 12 -Tag test4` (PresentMon 2.6, ETW session on javaw.exe and dwm.exe). The first `nvlddmkm` 153 came at 14:28:48. [measured, qemu.log of that run] a new guest RM client (7 RmAlloc) appears in the same 5 s heartbeat window, then 37 `DISABLE_CHANNELS` RPCs in a row, then the refusal below.

The refusal that looked like the cause is a consequence: `ring REFUSED segment gp 34 ... NoCeObject { subch: 4, method: 576 }` on kernel channel token `0x806` (class `0xc56f`, engine `0xb`, a copy-engine kernel channel; the 6 words are a CE `SET_SEMAPHORE_A/B/PAYLOAD` + `LAUNCH_DMA` on subchannel 4 with no `SET_OBJECT` before it). [measured] The identical refusal (same channel, same gp 34, only the payload word differs) is also in the log of a **clean guest shutdown** (`vm-C2-a07a9ba0-noprof`, 3000 lines after the last trial, no TDR in that life). So it is the guest driver's teardown/reset push, which kf3 refuses; it is not the trigger of the TDR. (Earlier `NoCeObject` hits in 50 older boot logs are all the NVDEC kernel channel, engine 0x13, gp 0 - a different, already fixed defect `1c503f87`.) A refused teardown push on a channel that is being torn down is probably harmless but deserves its own look (tier at birth for kernel CE channels).

Signatures seen: S1 = 3x `nvlddmkm` 153, javaw crash in `nvoglv64.dll` 0xc0000409, host Xid none, kf3: `ring REFUSED ... token 0x806` + `DEAD`. S2 (C4_t2 only, a guest that had TDR'd 3.5 min before) = the documented open signature: `USER_RING_MIGRATED`, `GSP REFUSED fn76/0x906f0102` (RESET_CHANNEL refused 0x56), a full GSP Running -> Suspending -> Running cycle, no `ring REFUSED`. In no trial was there a host Xid (`journalctl -k` Xid lines: 0 in all 15 trials).

## Matrix (host time of the trial = 16:39-17:53 on 2026-10-15; evidence in `/data/claude-mctdr/ev/trials/<trial>/`, kept on the host)

`t` = seconds from javaw start (guest clock) to the first `nvlddmkm` 153. All trials: production profile, guest 16 GiB / fb 8192, the launch is the earlier agent's (`-Xlog:gc`, `--quickPlaySingleplayer "New World"`, no `--quickPlayPath`) unless "owner". "idle" = nothing typed in Minecraft; "drive" = walking/looking script. PROF = KF3_PROF=1. Each trial ran after a fresh guest boot unless marked "same life".

| trial | build | PROF | load | PresentMon | result | t | signature |
|---|---|---|---|---|---|---|---|
| (live4, earlier agent) | a07a9ba0 | yes | idle | yes, +41 s | TDR | 43 s | S1 |
| C1_t1 | a07a9ba0 | yes | drive, **game paused by focus loss** | no | survived 3 min - INVALID (not a load) | - | - |
| C1_t2 | a07a9ba0 | yes | drive (same life, after the TDR) | no | survived 3 min | - | - |
| C1_t3 | a07a9ba0 | yes | idle (same life) | no | survived 150 s | - | - |
| C2_t1 | a07a9ba0 | no | idle | no | survived 150 s | - | - |
| C2_t2 | a07a9ba0 | no | drive (same life) | no | survived 3 min | - | - |
| C2_t3 | a07a9ba0 | no | drive, "owner" launch (saved launcher command + fresh `--quickPlayPath`, no `-Xlog`) | no | survived 3 min | - | - |
| C3_t1 | 64449dc5 | no | idle | yes, +36 s | TDR | 42 s | S1 |
| C3_t2 | 64449dc5 | no | idle | no | survived 120 s | - | - |
| C3_t3 | 64449dc5 | no | idle (same life as C3_t2) | yes, +36 s | TDR | 42 s | S1 |
| C4_t1 | 666ea966 | no | idle | no | **TDR** | 106 s | S1 |
| C4_t2 | 666ea966 | no | idle (same life, after C4_t1's TDR) | yes, +36 s; PresentMon ran 12 s and wrote 202 KB of frames, no TDR then | TDR | 148 s | S2 |
| F644c_t1 | 64449dc5 | no | idle | no | **TDR** | 101 s | S1 |
| F666c_t1 | 666ea966 | no | idle | no | survived 150 s | - | - |
| Fa07c_t1 | a07a9ba0 | no | idle | no | **TDR** | 105 s | S1 |
| Feb9c_t1 | eb9f2aa3 (broker badf2d7-TRACE, the pre-stream viewer) | no | idle | no | survived 150 s | - | - |

(Table fields come from `summarize.sh` over the trial directories; "t" for the live4 run is from its guest event log.)

Counts: idle controls on a fresh boot, no PresentMon: **TDR 3** (C4_t1, F644c_t1, Fa07c_t1), survived 4 (C2_t1, C3_t2 at 120 s, F666c_t1, Feb9c_t1). All three TDRs at 101-106 s.

## Hypotheses against their falsifiers (PREREG: `/data/claude-mctdr/ev/PREREG.txt`, written before the first run; the PresentMon addendum is dated 17:14, before the first PresentMon trial)

- **H-A deterministic on a07a9ba0+PROF**: falsified. Of three trials there (the original crash, C1_t2, C1_t3) two survived (C1_t2/t3 ran in a guest that had TDR'd; C1_t1 invalid). Without PROF a07a9ba0 survived 3 of 4 trials (only Fa07c_t1 TDR'd).
- **H-B PROF matters**: falsified. TDRs without PROF on `a07a9ba0`, `64449dc5`, `666ea966`.
- **H-C the a07a9ba0 relay/worker wake matters**: falsified. `64449dc5` crashes the same way, with and without PresentMon.
- **H-D launch parameters**: not decided. The owner's own launch could not be reconstructed (the launcher logs the JVM arguments but not its quick-play game arguments; `launcher_quick_play.json` only names the world). The "owner" form (saved launcher command, fresh `--quickPlayPath`, no `-Xlog`) survived once (C2_t3); the agent form crashes with the same signature on three builds. One survival does not separate them.
- **H-E pre-existing on every build**: supported for the three builds after `eb9f2aa3`; **eb9f2aa3 and 2f99e673 not decided** (0 of 1 and not run).
- **H-F the NoCeObject refusal is the trigger**: falsified as a cause. It also appears in the clean-shutdown log (C2) and follows the 37-channel disable burst.
- **H-P (added after reading the guest logs) PresentMon causes the TDR**: not necessary (controls TDR without it), but it precipitates it: 3/3 fresh-boot trials at +3-4 s after its start (a07a9ba0 original, 64449dc5 twice). Not reproduced on 666ea966 in a guest that had TDR'd (C4_t2, no TDR for ~110 s after PresentMon ended, then S2). Falsifier "3 trials survive": not met.

Caveats [stated, not hidden]: (a) every trial shares my harness, which runs a PowerShell status query as SYSTEM every ~9 s; a "quiet" variant (no guest commands during the run) was written but not run. (b) The TDRs in the idle controls cluster at 101-106 s after the launch and, in two of them, a kfadmin-owned process queried WMI (`Select Name From Win32_Processor`, cancelled) ~3 s before the TDR at about 3 minutes after boot; survivors spanning the same minute exist (C3_t2), so this is a correlation, not a finding. (c) Fresh boots take about 25 s; trials ran 25-45 s after the guest agent answered. (d) Sample sizes are small.

## Code delta (read, `git diff --stat`)

`eb9f2aa3` -> `a07a9ba0`: only display/cursor crates changed (kf-disp `scanout.rs`/`engine.rs`, kf-qemu `display.rs`, kf-cuda `display.rs`, kf-broker); kf-chan, kf-rm, kf-mem untouched. `a07a9ba0` -> `666ea966`: kf-broker only (`conn.rs`, `lib.rs`). The refused push is decoded in kf-chan (untouched since `eb9f2aa3`).

## Next (needs the VM back)

1. Idle controls, fresh boot, on `eb9f2aa3` (4 more) and `2f99e673` (2): if they TDR at the same rate the defect is pre-existing on every build; if 0 of 6 the display delta (eb9f2aa3 -> 64449dc5) is the suspect. `matrix2.sh` holds this queue.
2. The quiet control (`QUIET=1 trial.sh`) on `a07a9ba0` and `666ea966`, to rule the status poll out.
3. Instrumentation missing: a host-side marker for "guest TDR started" (the host only sees it as a DISABLE_CHANNELS burst, ~37 RPCs, then the refused teardown push), wall-clock stamps on qemu.log lines, the per-RM-control command ids of the new client that appears 3 s before the burst in the production profile (`rpc-trace` is off), and GP_GET/GP_PUT per user channel at the burst. Without those the stall that dxgkrnl timed out on (~2 s before the 153) is invisible: no host Xid, no refusal before the burst.
4. The NoCeObject refusal on CE kernel channel 0x806 (a teardown push without `SET_OBJECT`): decide whether kf3 should admit it or refuse it quietly; separate from the TDR.

## Not done / state

Not run: eb9f2aa3 beyond 1 trial, 2f99e673, PresentMon on fresh `666ea966`/`eb9f2aa3`, quiet controls, the 3 h host harvest (no trial survived a 3-minute window in a state worth harvesting; skipped), PresentMon frame statistics (one 12 s capture exists in the guest for C4_t2, not analysed). Shutdown-only counters per VM life are in `vm-*/` (`user_ring_loop_summary.txt`, `late_max_us_top3.txt`, `retired.txt`, full log gz). The guest still has `pauseOnLostFocus:false` in the owner's Minecraft `options.txt` (original saved as `C:\kf\mctdr\options.txt.orig`), scheduled tasks `mct-launch`/`mct-drive`, and files in `C:\kf\mctdr`; and the VM is at `eb9f2aa3` with the `badf2d7-TRACE` broker, not at the requested `666ea966`.
