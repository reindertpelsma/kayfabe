# Windows user-ring P0

**STATUS: RESEARCH, 2026-10-12 00:35UTC.** Native permission measurement and Linux verification complete.
This is not a Windows migration implementation or GravityMark PASS.

See docs/design/V3_WINDOWS_USER_RING_RELAY_REVIEW_20261012.md for the source
review, owner-approved restrictions and hypotheses/falsifiers written before
hardware. Owner condition: Linux Passthrough forbids host-owned mappings.

Probe: kf-harness/bin/kf-user-ring-probe, read-write then read-only then
write-attempt, each isolated uid65534 process under shared GPU lock. The two
positive cases differ only in GPU mapping permission; the negative case attempts
a GPU store into the read-only command mapping. No VM or personal disk is used.
No product behavior changes. Exactsource91bf09a80f4934c54f333a958adfcde8fcfb5a68;
binarySHAe158896acc417bf1892a5694fcb49bd73a807a3482cbb9accd864b148623f69c.
HostRTX4070/595.91.07; all probe callers uid65534, channels assertedUSER.

- read-write: PASS, realGPUtail1, sentinel unchanged, zeroXid.
- read-only: PASS, identical commands, realGPUtail1, sentinel unchanged, zeroXid.
- write-attempt: DENIAL_OBSERVED. The GPU's attempted write to the read-only
  command mapping yields attributedXid31/FAULT_RO_VIOLATION/ACCESS_TYPE_VIRT_WRITE;
  sentinel unchanged, tail0. Return1 is expected collection failure, not PASS.
  Cleanup succeeds, no external kill. GP_GET=1/GP_PUT=1 despite tail0, directly
  demonstrating why fetch progress is not GPU-work completion.

Original logs/source metadata, per-process timestamps/UID/PID, journal windows
and observations are under native-result; the collector is run_native.py and
its stdout native-session.log. Hypotheses/falsifiers predate the hardware run in
commit91bf09a80. This tests a FIFO virtual write, not all engines/families.
Post00:12:29UTC: noQEMU, GPUlockfree, group11DMA-FQ,122.0GiBfree. Owner image unused.

Second reference discriminator (falsifier written in review before run): exact
b081f705a native deferred-update-probe, binary9acb5976725bbc0fd8a57685038840f121cca4b7950097b7f8452544d61c224c,
same update-hold arguments under uid65534 versusuid0. **Both FAIL the no-Xid
criterion** despite rc0/realGPUtail/2s ownership hold/clean teardown: each has one
PID-attributedXid32 within the hold. GPU channels stayUSER. RootUID did not
establish a usable reference; actual saved RM privilege/inner error unmeasured.
No more wall-clearing variants queued. Full source/PID/time/journal evidence in
privilege-reference, stdout privilege-session.log, runner run_privilege_reference.py.

GPU-free unconnected prototype: kf-chan user_ring Ledger, six new bounds/wrap/
backpressure/completion models; full kf-chan suite green. kf-rm admission permit,
two new Linux/unknown/kernel/UVM/hostile-fact scope tests; full kf-rm suite green
(133lib plus integrations). Red compile logs and green logs retained. Separate
ordering_model.py: four tests, seven late-switch counterexamples, zero with
immutable birth-time inspection under those schedules. These are not a native
migration or backing-lifetime proof; no runtime private ring is enabled.

Linux exact **70aa0d666c22289583d8659e325b0763bef4a982**: gates **9/9 + gate10 PASS**,
USER births12/12; raw client and Linux fastguest rebuilt; **fast30/30 PASS**,
zeroFAIL/CRASH/NOTRUN. QEMU SHA256
`6d4661860bfc1690e22dbb2f40c2ae07d65a64d7fbfa0f2bcfe806a62cd8886e`.
This verifies the unconnected model/permit revision, not a Windows relay.

The first runner passed gates/build but did not start the fast suite: missing
BENCH_DIR and a nine-character binary prefix (installer uses eight). Initial
EXIT2 is retained as a harness failure. A separate immutable retry runner changed
only the harness and ran the original30-arm fast suite on the same binary;
retry EXIT0. Runners are run_linux_initial.sh and run_linux_fast.sh. Original
logs including the failed attempt and per-arm records are linux-result/original-logs.tar.gz;
readable gates/fast summaries and final host receipt are alongside it. No personal
Windows image was used. Post00:35UTC: noQEMU, lockfree, group11DMA-FQ, stop-file
absent,129830002688bytes available (>100GiB). No queued run.
