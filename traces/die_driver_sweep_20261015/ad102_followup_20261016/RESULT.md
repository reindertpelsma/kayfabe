# AD102 missing lanes and passing Windows display reference

**STATUS: LIVE, 2026-10-16 18:44 UTC. Windows580 hold/apps and Linux595 raw/broker pass; rental retired.**

MEASURED A1: broker595 passes at17:37:30UTC, private6265b980f. Cinnamon1/Xorg1,
NVIDIA OpenGL renderer, guest flip_timeouts0/Xid0, and host Xid delta0 (5before,
5after). X11 SHM presentation contains13506colors at1920x1080. Actual process
environment has only KF3_REV, the binary selector. Its caps page has23T1 words,
zero T3 words and one unmeasured word; no595caps row was borrowed from580.
Nine host display STALL messages and nine guest nvAssert lines are retained as
limitations; functional presentation is not a latency-clean claim. See broker595/.
The first presentation launch was NOTRUN after a previous provisioning PID
refusal; the explicit-parent-wait retry exercises that handoff and succeeds.

MEASURED A5: native Blender4.5 CUDA passes17:16:24UTC on stock595, NVIDIA
RTX4090, mean0.6405. Windows580.88 production apps pass18:38:30UTC: Blender CUDA
exit0/NVIDIA/mean0.6405, Edge hardware WebGL2 correct pixels/722frames/12seconds,
zero TDR, and CUDA cup2/3/8 plus hardware D3D11+12 before and after. Host Xid
5before/5after, delta0. Source77a72da9e, actual19flags/diagnostics off, same
product code and binary SHA as the production hold. Earlier file-sync failures
and the false PowerShell null-exit-code failure are retained as harness negatives;
see windows_apps/RESULT.md. Images stayed on rental. Bounded file staging has
six passing CPU tests; mutating commands are never automatically replayed.

MEASURED A2 complete for this build: production QEMU103998 uses exactly19flags,
no RPC diagnostics. Actual Windows32.0.15.8088, Code0, DWM/Explorer, CUDA cup2/3/8
and hardware D3D11+12 pass before and after the421second/13round hold. Readable
System4101 count is0during the hold and since boot. Both1920x1080screenshot
receipts contain2072803nonblack pixels of2073600. Owned VM stops cleanly17:08:25UTC.
Diagnostic process102707 separately passes the short suite with21flags and
provides the connected-display trace; see passing_windows_display_trace/RESULT.md.
The QGA helper's framed synchronization recovered probe staging; both staging
failures remain recorded and are not mislabeled as GPU/Code43 failures.

MEASURED update: private7d1be8882/e9744a23ab38ff45afbf788d9e426d71913d1f1e6954fa1ea0da1ea1920b25a6,
host/guest595.91.07, actual KF3 environment empty, normal loglevel6: raw30/30 at
180seconds PASS16:27:04..16:33:20UTC. Thirty actual process receipts,161/161USER
suite births,12/12USER gate births and90CUDA thread posture lines pass. Native
CUDA4 also passes. The newer revision differs from native-gate2feb22a9f only in
bench scripts/docs, with crates/qemu/cuda byte-identical. Raw client and thin
guest were freshly rebuilt before this run; their hashes are in the wave receipt.

MEASURED limitation:1418heartbeat formula warnings across all30serial logs,
all with heartbeat0, before the first valid heartbeat. Concurrency warnings span
guest6.945..7.323seconds, then stop. No nonzero-heartbeat formula warnings occur.
This is the remaining startup readiness gap, not a warning-free result or a
controlled claim that the candidate regressed Ada. The heartbeat PR remains draft.
Five host Xid31 records map by exact QEMU PID to defer-liveness (four) and
missing-page-fault (one); the frozen grader deliberately faults GPU channels in
those arms and both pass. This does not waive zero-new-Xid checks for broker/apps.

MEASURED: the99nonzero words of this board's native Linux595 page are identical
across all4096bytes to the earlier native AD102 Windows580 oracle. Comparison
and page SHA are in native580_vs595_caps.json. This observation admits no wildcard
branch row or unmeasured die.

First Windows diagnostic attempt is UNVERIFIED: QGA timed out synchronizing the
third probe upload, before the acceptance suite. Owned QEMU100839 shut down
cleanly at16:37:47UTC; no Code43 or suite failure is inferred. A retry keeps the
same driver/GPU flags and stages hash-checked files from a read-only owned CD.

Owned instance56267214, verified machine43132. MEASURED: actual146GiB root,
30logical CPUs,24389MiB RAM and KVM. Unprivileged UID65534/CapEff0 RM probe
identifies arch0x190/impl2 AD102 and stock Linux595.91.07. Native gates9/9,
gate10 PASS and12/12USER births at16:10:50UTC. Private source2feb22a9f built
at16:11:25UTC. The native595 caps page is an oracle only.

| Die | OS / lane | Actual driver | Flags | Verdict | Evidence |
|---|---|---|---|---|---|
|AD102|native Linux gates|595.91.07|gate harness|PASS9/9+gate10|adaref2_gates.log|
|AD102|native Linux CUDA|595.91.07|stock host|PASS4/4|linux595/cl_codex_adaref2_native595_host.out|
|AD102|Linux guest raw|595.91.07|actual KF3={}, loglevel6|PASS30/30, startup heartbeat warnings remain|linux595/codex_adaref2_full595_suite.out|
|AD102|Linux guest broker|595.91.07|actual KF3_REV selector only|PASS;9STALL/9nvAssert retained|broker595/|
|AD102|Windows guest short / display trace|580.88 /32.0.15.8088|production19 plus bounded metadata/body diagnostics|PASS|windows580/adaref2_windows_baseline.log; passing_windows_display_trace/|
|AD102|Windows guest seven-minute hold|580.88 /32.0.15.8088|production19, diagnostics off|PASS421seconds/13rounds/0TDR|windows580/adaref2_windows_baseline.log|
|AD102|Windows guest Blender/Edge and survivor suite|580.88 /32.0.15.8088|production19, diagnostics off|PASS;0TDR/0new host Xid|windows_apps/RESULT.md|
|AD102|Windows second build|581.51 /32.0.15.8151|planned production19|UNRUN; public ABI/package prerequisites pass|../driver58151_public_abi_20261016/RESULT.md|

The Windows diagnostic boot collected an ordered passing connected-display
reference for prompt section2; see passing_windows_display_trace/RESULT.md. Native GB206 was headless and cannot supply that
reference. Hypothesis H-display-reference (INFERRED): presence of73010c and
c3700201 in the native sequence may identify missing display initialization.
Falsifier, stated before this run: a passing Ada guest also skips these controls;
then their presence in native alone cannot identify the Blackwell abort.
Also report whether a89,ab2,ab4,cbcb,2801 and2806 are served or never issued.

The diagnostic short suite and production hold use separate QEMU processes.
Each must assert the actual source, private binary SHA, host595, guest580 public
ABI580.65.06, actual Windows32.0.15.8088 and every process KF3 variable. The hold
requires Code0, DWM/Explorer, hardware D3D11+12, at least420seconds, zero readable
System4101 events, before/after CUDA cup2/3/8 and nonblank screenshot receipts.
Screenshots and unscreened reply bodies stay on the rental. No new native Windows
installation is needed because the per-die reference already exists.

CPU harness correction: draft PR26 adds explicit KF_GUEST_DRIVER and backing-disk selection to
interactive.sh; its previous hardcoded580 ABI would make a595 disk's receipt
wrong. Failing-first assertions are saved; three test methods now pass. Hardware broker595 passed on6265b980f with this change. Separate draft PR27 records and
verifies owned QEMU identities and uses ACPI then bounded TERM cleanup. Its
actual-launcher slow-cleanup fixture fails on the old code; four CPU tests pass
after the fix. Both changes are hardware-broker verified on6265b980f, including the explicit
parent-wait handoff correction; nine STALL/nvAssert warnings remain visible.

No die family is complete. Owned56267214 destroyed/API absence verified
18:43:36UTC, before its original timer and hard cap. Exact timer2396076 cancelled
after absence and command-identity verification. Conservative lease duration
2h40m15s, compute estimate1.6293USD/ceiling1.63USD. Campaign compute ceiling
7.00USD plus unbilled transfers. No own rental remains. See retirement.txt.
