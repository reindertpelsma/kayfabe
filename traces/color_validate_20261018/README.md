# CL1 round 1: example assertion failure; BGRA interpretation corrected

CODE READ: PR #92 head `e636236cf34621b0c9e53170638c54a55a22f979` corrects the interpretation below: the kernel stores B,G,R,A bytes, so the first readback `[64,160,0,255]` is logical RGB `[0,160,64]`. The round-1 example compared BGRA memory as RGB. The kernel and PTX are unchanged; the earlier inference of a kernel mismatch is superseded. Round-2 validation is recorded separately under [round2](round2/).

MEASURED: on 2026-10-22, lease 57217748 at revision `81626530eb1aadd42976a699bb4a881497f74794` built all three `kf-cuda` examples successfully with Rust 1.99.0, release mode, two jobs and CPU niceness 19; see [gpu-failure.log](gpu-failure.log).

MEASURED: lease 57217748 (GTX 1660 SUPER / compute capability 7.5 / driver 610.43.02) failed the first `color_mirror` assertion: expected RGBA `[0,160,64,255]`, observed `[64,160,0,255]` on all eight pixels, exit 101 at 2026-10-22 13:36:23 UTC; see [gpu-failure.log](gpu-failure.log).

MEASURED: on 2026-10-22, lease 57217748 at revision `81626530e` executed one of four mirror cases and failed it; the remaining expected RGB cases `64,160,64`, `192,0,0`, `192,64,64` were not reached. The hypothesis and falsifier were recorded in [hypotheses.txt](hypotheses.txt) before the run; the result is in [gpu-failure.log](gpu-failure.log).

CODE READ: the hardware regression examples selected were `color_sdr` (eight named checks) and `color_tmo` (eight named checks); neither executed because the fail-fast validation script stopped at `color_mirror`. The `cargo test --locked --release -j2 -p kf-cuda` step also did not execute. No before/after unit-test count or mutation result is claimed; this lane changed evidence only.

CODE READ: the lane's stop rule ended execution at the falsifier. The trusted host's CPU build/install was not attempted, no restart binary was installed, no merge was performed, and no Windows install or VM/GPU operation was issued.

MEASURED: lease 57217748 was destroyed by its own numeric ID; authenticated inventory verification at 2026-10-22 13:39:08 UTC found it absent. Its own Rust target was removed and its validation process was gone before teardown; see [cleanup.txt](cleanup.txt).

MEASURED: on 2026-10-22, lease 57217748's pre-armed 50-minute systemd timer was active before rental, then stopped only after verified absence; quoted cost and reserve evidence are in [rental.txt](rental.txt) and [cleanup.txt](cleanup.txt). Account credit was 37.719872290561284 USD before and 37.710500883061286 USD after; the 0.0093714075 USD account change includes any concurrent account use and is not an itemized charge for this lease.

INFERRED (historical round-1 recommendation, superseded by the BGRA correction above): the owner should return PR #92 to its author with this exact mismatch before installing or restarting on this revision; this lane does not diagnose its cause. Real display-engine MIRROR semantics and Windows Hue/Digital Vibrance console recovery remain unverified.

MEASURED: on 2026-10-22, the trusted host's owner QEMU process remained PID 2123004 with the same start time before and after lease 57217748's run; no lane-owned host directory was created, no fastguest lock was taken, no local Rust target was created, and no lane job or timer remained active after round 1; see [cleanup.txt](cleanup.txt).
