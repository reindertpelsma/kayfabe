STATUS: STOPPED PARTIAL, 2026-10-22. S3 takeover of the existing S2 lease.

MEASURED: pinned SSH and the API identified the owned running RTX 4090 lease
57144505. The inherited controller had already exited rc=1 at 04:30:12 UTC;
no restart or new workload was launched during takeover. Its original destroy
timer remained armed for 05:05:42 UTC. Evidence commit b9f1abd03 was pushed
and origin-verified before teardown. MEASURED: the supplied helper destroyed
only ID 57144505; API absence was verified 04:45:07 UTC before timer
cancellation, then independently rechecked 04:45:54 UTC. No owned local
process, build target or local lock remains. See ../final_cleanup_receipt.json.

CODE READ: the controller stated its Windows hypothesis and falsifier before
the arm: Code 43, crash, TDR, desktop, CUDA or DX failure falsifies acceptance;
PASS requires a 600-second zero-TDR hold and actual app results.

MEASURED: normal ON arm ad102_on_d1, product source
7f6f7e7a76c141522a47dc79d024d9b3fd4d35e0, QEMU PID 22219, started
04:24:56 UTC. Windows reported actual driver 32.0.15.8088 (580.88), Code 0,
status OK, 1920x1080 and zero event 4101 at 04:28:55 UTC. The launcher's
guest-driver=580.65.06 label does not identify the installed Windows driver.
Evidence: start/process receipts, enable/diag text, and run log in this directory.

MEASURED: the 04:29:11 pre-hold health record had dwm=1, explorer=0,
events4101=0, events153=2 and bugcheck1001=0. Both emitted event-153 records
name Microsoft-Windows-Kernel-Boot, with a virtualization-based-security message.
CODE READ: the guard selects IDs 4101/153/1001 without a provider filter and
requires Explorer. Its failure is recorded; no NVIDIA TDR or fully working
interactive desktop is established by these observations. No safety check was
relaxed and no production defect was inferred from that refusal.

MEASURED: ARM_HOLD_OBSERVED and app-acceptance records are absent; the normal
600-second hold, Windows CUDA/DX smokes and observer arm are UNRUN. The
controller stopped before the normal hold. Cleanup additionally reported the
missing alloc_headers.py helper; allocation-header export is incomplete. Its
existing metadata export retained 4433 RPC records. QEMU stopped cleanly and
the parent image checksum remained OK; ARM_EXIT rc=1 is preserved.

MEASURED: the final exported kernel Xid records exactly equal the frozen ten
approved native/guest negative-control records, in order. Offline comparison
PASS; no later host Xid at export. The pre-destroy process check found zero
QEMU, Windows-controller or guard matches. Native/Linux acceptance remains
as recorded in ../linux_complete/ and ../guest_controls_accepted/: native
30/30, CUDA 4/4, GR PASS, gates 9+10, 12 USER births, Linux guest 30/30 and
30 durable actual-PID completions. Each current-freeze hardware arm has n=1;
spread is N/A. No Windows stability or performance PASS is claimed.

MEASURED: 43 existing text artifacts were exported in two bounded collections,
plus receipts and offline verification. Raw RPC bodies, executables, VM images
and the 1.7 MB QEMU log were excluded; its byte count and SHA256 are retained.
The original raw rental log will not survive lease deletion. The exported
summary's empty allocation sections cannot establish zero allocation failures,
because the allocation-header helper was missing.

CODE READ: inherited saved software evidence records 128 tests PASS, including
the original 23, and 36 manual mutants with at least two distinct failing tests
each (22 matching, 3 image, 6 exit, 3 thread, 2 framing). No software behavior
changed and no software tests or mutants were rerun during takeover.

INFERRED decision 1: retain Linux PASS and Windows PARTIAL. Recommend a future
separately authorized, tests-first review of provider-aware health evidence,
the missing Explorer session and export-helper deployment before any rerental.
