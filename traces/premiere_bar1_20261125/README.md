STATUS: LIVE, 2026-11-25. Result of the Premiere BAR1 and state control (`PREREGISTRATION.md`, commit bdfe77836, written before any box). One Vast RTX 4090 box, one boot. Labels: MEASURED (a file of this directory, run of 2026-11-25), INFERRED (not tested).

# Result
The Premiere crash of PR #588 did NOT reproduce. MEASURED on 2026-11-25 (box 62455544, `runs/`): action A (new project, import `sintel_trailer_1080p.mp4`, 1920x1080 sequence from the clip, active and selected, 45 s hold) PASSED 3 of 3 (A1, A2, A3). After each attempt Premiere was alive and responding. The guest had no Application 1000/1001/1002, no WER report, no LocalDumps file, no display or TDR event, no LiveKernelReport (`box1/runs/ce__evidence.txt`, since the first attempt), and the host Xid count was 0 (`box1/harness_C_rows.txt`). The Premiere SentryIO folder holds only session and small event records (2.1 kB `__sentry-event`, envelopes of 347 to 392 B), no minidump. No exception code exists to read.

# Host (MEASURED 2026-11-25, `box1/host_record_before_image.txt`, before any image byte)
- RTX 4090, FB 24564 MiB, `nvidia-smi -q -d MEMORY` BAR1 total 256 MiB (used 2 MiB), `lspci -vv` Region 1 size 256M, no Resizable BAR capability listed in the guest's view of the function, clocksource `kvm-clock`, AMD Ryzen 9 7950X, 48 GiB, provider driver of the rented VM 575.51.03, then kayfabe's in-VM host driver 580.159.04 (`box1/harness_C_rows.txt`). kf3 QEMU built on the box from origin/next 03d2a1267.
- INFERRED: the BAR1 shown is what the VM firmware gave the passed-through function; the hypervisor host's own BAR is not visible.
- Only one box ran. A second box with BAR1 equal to the VRAM was not rented: the box passed the 3 box-hour cap while the run was still being driven (image fetch 19 min, boot, harness), and the coordinator ordered the run closed.

# Run (MEASURED, 2026-11-25)
- Image `kfimg-adobefull2-20261122T015401Z`, offline (`restrict=on`), recorded clock (RTC base 2026-11-22T02:43:26Z), guest 16384 MiB, fb-mb 8192, virtual sound card, data volume `test-assets-20261116c.iso` (sha256 6b5fa657...0266). Pins verified on the box: `image.qcow2` sha256 ec36a395...ad796 (dl_member.py, `SHA256 OK`), zip sha256 c702ef2d...e7776 (`zip_verify.py`, `ZIP_SHA256 OK`, `VERIFY_EXIT rc=0`). Presigned GET minted with `presign_get.py` defaults (r2ro), files 0600, shredded on the coordinator and on the box after the fetch.
- Boot C (harness `hold_run.py`, hold until file): `gpu` PASS, `verify` PASS, host Xid CLEAN. Driven by `drive.py` with `pp_action.ps1` (this directory's scripts are under `scripts/bench/windows/imgb_offline/`: `pp_action.ps1`, `crash_evidence.ps1`, `pp_setup.py`; the jsx is PR #588's `pp_sed.jsx` in its `nosed` mode). The loop sees the process end, with the Vast ESC/ENTER cadence and dialog sweep.
- The result files of each attempt were pulled by `pull_guest.py`; the attempt table:

| attempt | boot | earlier apps in the boot | outcome | alive and responding after |
|---|---|---|---|---|
| A1 | C | none (first Premiere start of the boot) | PASS | yes |
| A2 | C | A1 (no crash) | PASS | yes |
| A3 | C | A1, A2 (no crash) | PASS | yes |

# Hypotheses
- H1 (crash only with small BAR1): FALSIFIED on this box. BAR1 was 256 MiB and nothing crashed in 3 attempts. The large-BAR1 arm was not run, so the reverse direction is untested.
- H2 (crash needs an earlier Premiere crash in the same boot): this boot's A1 is a fresh-boot action A (no earlier crash, and no earlier apps): PASS. By the preregistered falsifier (a fresh-boot action A crashes) H2 is not falsified by A1, but the crash chain never started (no first crash happened), so the state hypothesis is NOT TESTED: it needs a boot in which Premiere has crashed first.
- H3 (host driver or nested KVM): not separable. The same host class (7950X, nested KVM, driver 580.159.04, kf3) that crashed 3 of 3 on 2026-11-24 passed 3 of 3 here.
- NOT RUN (time and cap): Photoshop and Camera Raw first, the Learn project from the home screen (the Vast 2026-11-22 03:30Z crash trigger), a second boot, a second box with a large BAR1. The crash on 2026-11-24 followed those steps; the difference between that run and this one is exactly that chain, so it remains the leading INFERRED candidate and a different box.
- Exception code: not obtained (no crash).

# Cost and teardown
Box 62455544, label `kf-prembar-250329`, RTX 4090 offer (Spain), 0.66 USD/h, about 3.2 box hours, about 2.1 USD (coordinator figure at 3.04 h: 2.00 USD); cap entry `kf-prembar-` 3 h and 3 USD (exceeded by about 0.2 h because the host reboot of the coordinator interrupted the session during the image fetch window). Destroyed by its own id; `vastai show instances-v1` listed no instance afterwards; `vast_guard.py --enforce` clean before and after. No URL, IP, key, UUID, account name or image content is in this directory.

# Files
| file | content |
|---|---|
| `PREREGISTRATION.md` | hypotheses and falsifiers, before any box |
| `box1/host_record_before_image.txt` | `nvidia-smi -q -d MEMORY`, `lspci -vv` of the GPU, clocksource, CPU, RAM, before the image |
| `box1/harness_C_rows.txt` | harness rows of boot C, Xid watch, attempt timestamps |
| `box1/runs/A*__result.json`, `A*__pp_progress.txt`, `A*__pp_stub.log` | each attempt's own record (guest clock) |
| `box1/runs/ce__evidence.txt` | crash evidence query over the whole session (empty of crashes) |
