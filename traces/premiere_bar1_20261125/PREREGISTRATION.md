STATUS: LIVE, 2026-11-25. Written and committed before any Vast box of this experiment was rented and before any guest boot. Labels used in the result: MEASURED (a file or log line of this experiment, named), INFERRED (not tested).

# Question
PR #588 (`traces/imgb_offline_features_20261124/`, Vast box, RTX 4090, kf3 at 3581ba07, host driver 580.159.04) recorded a Premiere Pro 27.0 crash about 2 s after a 1080p sequence was shown, 3 of 3, offline, sealed image B, "Unhandled exception!" on `Display Thread`, renderer CUDA. PR #589 (`traces/pc_premiere_20261124/`) ran the same actions on the owner's pc (RTX 4070, host 595.91.07): 24 of 24 passed (17 kf3, 7 bare-metal VFIO). The exception code was never read.

The owner suggests a limit of the rented host's BAR1 (no Resizable BAR). PR #589 lists other differences: die, host driver, nested KVM, image checkpoint, and the boot history (on Vast, Premiere ran after Photoshop and Camera Raw work and after its own first crash, so every later attempt started after a crash).

This experiment runs the sealed image B offline on Vast RTX 4090 boxes (real 24 GB boards, `gpu_ram` 24564), records each host's BAR1 before anything else, and drives the Vast flow in order, plus a fresh-boot control.

# Records on every box, before the image is fetched
`nvidia-smi -q -d MEMORY` (BAR1 total and used), `lspci -vv` for the GPU (Region 1 size, Resizable BAR capability and state), host driver version, the current clocksource, CPU model, RAM. No serial, UUID, IP or PARTUUID is committed.

# Procedure per boot (the harness holds the guest up; all actions go through `drive.py` and the scripts of `scripts/bench/windows/imgb_offline/`)
- Boot C (chain): Photoshop with the real DNG (Camera Raw opens), held; then Premiere Pro started and the "Preview-Your-Clips" Learn project opened from the home screen by a double click (the Vast 03:30Z flow); then action A three times.
- Boot F (fresh control): a new boot of a fresh overlay; action A three times, nothing else started first.
- Action A: new project, import `sintel_trailer_1080p.mp4` from the data volume, create the 1920x1080 sequence from the clip, make it active, select the clip, hold 45 s (the `nosed` mode of `pp_sed.jsx`). An attempt CRASHES if the Premiere process ends before the hold marker is written, PASSES if the marker is written and the process is alive and responding afterwards.
- After every attempt: guest evidence since the attempt start (Application 1000/1001/1002 with exception code and faulting module, WER report headers, Premiere SentryIO records with the crashing thread and the minidump's exception code and the module that contains the faulting address, LocalDumps sizes, display-driver events, host Xid). Dumps stay on the box; only sizes and the exception code leave it.

# Hypotheses and falsifiers
H1: the crash occurs only on a host with a small BAR1 (256 MiB, no Resizable BAR).
- Prediction: action A crashes on the small-BAR1 host and passes on a host whose BAR1 equals the VRAM.
- FALSIFIED if it crashes with a large BAR1 too, or if it does not crash on a small-BAR1 host. A result with only one kind of host is reported as "not testable" for H1, never as support.
- Caveat written in advance: `lspci` inside a Vast KVM guest shows the BAR the VM firmware gave the passed-through function, which can differ from what the hypervisor host has. The host's own BAR1 is therefore INFERRED from what the box reports; a box that reports one size while the guest uses another is recorded as such.

H2: the crash needs an earlier Premiere crash in the same boot (state).
- Prediction: boot F attempt 1 passes; boot C attempts after the Learn-project crash crash; if the Learn project does not crash, boot C attempts are the test.
- FALSIFIED if a fresh-boot action A (boot F, before any crash) crashes.

H3: neither; the cause is the host driver or the nested-KVM environment (recorded, not separated).
- Not falsifiable by this design alone. It is the residual verdict when H1 and H2 are both falsified and the crash still reproduces on some box.

If nothing crashes anywhere (every attempt passes, including the Photoshop-then-Learn chain), the result is "not reproduced on these boxes": H1 and H2 are not tested, and the open differences are the checkpoint of the image and the host.

# Exception code
Read from Windows Application event 1000 if WER saw the crash, and otherwise from the minidump in the Premiere SentryIO folder (exception stream: code, address; module list: the module containing the address). INFERRED rule: a code of 0x80000003 is a breakpoint (an `int 3` or a debug assert); 0xc0000005 is an access violation; the result states what was read, not what it means.

# Stop and report rules
- Cap 3 USD and 3 box-hours in total for all boxes of this experiment (label prefix `kf-prembar-` in the cost guard). One more box is rented only for a second BAR1 class and only if the first box's hours and cost (read from the cost guard) leave room; if the first box is destroyed first, that is fine.
- Boxes are destroyed by their own instance ids and their absence is verified. `vast_guard.py --enforce` runs before and after.
- A kayfabe defect, if one shows up, gets the smallest GPU-free test that fails before the fix and passes after it. None is expected from this design.

# What is NOT claimed
One or two boxes, one image, a few boots. A crash that reproduces on a box is not attributed to its BAR1 unless a second box with the other BAR1 passes. A pass is a pass of these actions only.
