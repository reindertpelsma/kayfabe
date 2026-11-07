# Status and handoff, history part 2: the 2026-10-12 decision, "0. Current resumption" (2026-10-14) and the 2026-10-11 checkpoints

**STATUS: SUPERSEDED, 2026-11-07** by [`../STATUS_AND_HANDOFF.md`](../STATUS_AND_HANDOFF.md) (current resume point) and [`../STATUS_DETAIL.md`](../STATUS_DETAIL.md). The section "0. Current resumption" as it stood on 2026-10-14, the 2026-10-12 restricted-FIFO decision and the 2026-10-11 checkpoints of `docs/STATUS_AND_HANDOFF.md`. Moved here, unchanged, on 2026-11-07 when the 223 KB file was split (lines 1105-1601 of the file at `a9dee7b2c`; `git show a9dee7b2c:docs/STATUS_AND_HANDOFF.md` returns the unsplit text). The only edit is that relative links gained a `../` prefix so they still resolve from this directory. Every statement below carries its own date and is a record of that day, not a description of today.

---

## Decision resolved — restricted Windows FIFO model, 2026-10-12

Owner §AI approves the restricted birth-time/read-only model. Completion and
source-lifetime evidence still gate runtime wiring. The former alternatives below
are history, not a pending request.

### Earlier alternatives, 2026-10-11

[Concrete design options and recommendation](../design/V3_WINDOWS_USER_FIFO_MIGRATION_DECISION_20261011.md).
Run1540 narrow NOT_SUPPORTED registration refusal reaches counter1 but still
hostXid32/channel4e and a Vulkan device-lost dialog; final TIMEOUT605s, no output. Guest153/4101/WER0, no reboot/per-appGSPcycles0; ACPI stop.
No clean fallback demonstrated. Choose priority: restricted mediated user
channels at birth for stock hosts; optional validated H8 host tier; or explicitly
unsupported migration while other app gaps proceed. No migration/host patch
implemented. Late conversion and relay-only edits do not solve the observed case.

## 0. Current resumption — start here

**Win11 USER ring, 2026-10-14 (branch `claude/win11-userring-20261014`, evidence `traces/win11_userring_20261014/`).**
Host-side fixes, GPU-free tests green (kf-chan, kf-qemu 259, kf-disp 150, kf-rm 148); Windows verification pending
(the VM was the owner's). (a) **Edge/YouTube TDR** [measured cause, `0c5fedff`]: the kernel NVDEC ring had no T-mode
tier, its first segment (`NVC9B0_SEMAPHORE_A-D`, a release + a non-stall trap) was decoded as CE work and refused
`NoCeObject` → DEAD → TDR. Fix `1c503f87`: VIDEO tier `kf_chan::vtables`, method space DERIVED from open-gpu-doc
`classes/video` (`tools/derive_video_methods.py`), semaphores admitted (address authored through rows + perimeter),
every other named method refused by name. (b) **Minecraft in-world Xid 32 PBENTRY**: [measured] the Xid precedes the
guest's `RESET_CHANNEL` by ~2 s (journal vs kf3 display clock; `dmesg -T` is ~4 s off here). Leading candidate H1
[read + inferred]: the SYNC_WAIT gate's 4-byte zero witness is inserted even when a packet is open (legal per
`dev_pbdma.ref` GP_ENTRY1_SYNC; straddles measured on every channel) → one-word shift → PBENTRY. Fix `1c503f87`:
control-NOP witness while a packet is open, `USER_RING_SYNC_CARRIED` counted; falsifiers and H2-H4 in the README.
(c) `GET_RG_SCAN_LINE` `0x5070020c` served live (`a8424ae2`, layout derived). (d) GravityMark Vulkan "relay flag":
the relay is already default-on (`KF3_WIN_USER_RING_RELAY=0` is only a diagnostic falsifier); verify on Windows.
**RESET_CHANNEL `0x906f0102` decision: stays refused.** The measured requests are nvlddmkm's engine-reset step inside
its TDR (kayfabe armed no RC notifier for USER twins: `rc[armed=0]`). The only rule-conforming act is the host's own
NON_PRIVILEGED `RESET_CHANNEL` (flags `0x10008`, `g_kernel_channel_nvoc.c`) on our twin; what it does to a PRIVATE
ring/USERD (GP_GET/PUT, PBDMA state) is closed GSP-RM and unmeasured, so the relay cannot know where to resume (a wrong
resume re-faults or drops a fence → repeated TDRs → bugcheck 0x116, worse than today's recovered adapter reset); an
NV_OK without the act is a forged recovery. Containment is already VM-bounded. A served form needs: RC notifiers armed
for USER twins, a forced-fault lane with a VFIO reference of what Windows does after an OK, and a test that the verb
is authored only for an own twin with a host RC observed, once per RC. Both measured triggers are fixed at source.

**Win11 OpenGL, 2026-10-14 (branch `claude/win11-opengl-20261014`, evidence `traces/win11_opengl_20261014/`).**
FIXED [measured]: the Minecraft/OpenGL 300 s vsync hang ("Unable to communicate with the display driver, error 2")
was the frozen GSP-client vblank counter: kf3 now keeps `kHeadVblankCount[head]` in the MAP_SHARED_DATA page
(`d86fea1f`); `kf_glvsync --swap 1` hang -> 60 fps, Minecraft vsync on loads. Also live core GET_RG_SCAN_LINE /
window GET_LINE (`af2b87fd`, fidelity). Walk "misaligned leaves": proposed AA amendment (stale reads of recycled
tables, not leaves). OPEN [measured]: (a) Minecraft in-world Xid 32 PBENTRY on javaw's USER-ring twin ->
RESET_CHANNEL refused -> TDR (3/3); (b) Edge/YouTube: kernel NVDEC translated channel refused `NoCeObject subch 4
method 0x240` on its first segment -> DEAD -> TDR (pre-existing on 0c5fedff); (c) `0x5070020c` GetScanLine control
refused. VM 2990 left on `d86fea1f`, production profile.


**TODO, owner-requested, 2026-10-14: Windows app-matrix failures (fix, or keep queued here).** Branch `claude/win11-opengl-20261014`.
(1) GravityMark **Vulkan** passes only with a user-ring relay flag; the default-route result is pending (evidence on
`origin/claude/default-route-evidence-20261013`): hardwire the needed behaviour so the flag disappears (H-relay).
(2) GravityMark **D3D12** fails (TDR, 3x nvlddmkm 153, no image); candidate shared with the Minecraft in-world defect
(Xid 32 PBDMA PBENTRY on a USER-ring twin, guest RESET_CHANNEL `0x906f0102` refused 0x56, then TDR) (H-d3d12).
(3) **OpenGL**: gears and FurMark pass; Unigine **Heaven exits early** (H-heaven: the frozen vblank counter, fixed
below — retest). (4) **Vblank counter / scan line** (the OpenGL branch's open work): DONE on this branch —
`GET_VBLANK_COUNTER` via the MAP_SHARED_DATA page (`d86fea1f`, fixed the vsync hang: `kf_glvsync --swap 1` and
Minecraft vsync on), live `GET_RG_SCAN_LINE`/`GET_LINE` (`af2b87fd`); still open: `NV5070_CTRL_CMD_GET_RG_SCAN_LINE`
(`0x5070020c`) is refused NOT_SUPPORTED, so `D3DKMTGetScanLine` reads line 0 / never in vblank.
(5) **Minecraft in-world** (vsync on or off): Xid 32 PBENTRY on javaw's GR user-ring twin a few seconds after a world
loads, then RESET_CHANNEL refused and a TDR (2/2 reproduced, `traces/win11_opengl_20261014/`).


**Windows/Codex current resume point:** [RESUME_CODEX_WINDOWS_20261011.md](../RESUME_CODEX_WINDOWS_20261011.md), branch `codex/windows-matrix-20261011`. Owner session is stopped; preserve its convenience disk chain described there.

## Latest checkpoint, 2026-10-11 22:54 UTC

Last confirmed push0e8d4ba62; private1dc9c0e unchanged. Prepared a default-OFF
`KF3_WIN_REFUSE_PT_FIFO_UPDATE` experiment: NOT_SUPPORTED only for the inner
FIFO-update registration on live Passthrough user-work parents, after normal
validation. 5080/other APIs/default behavior unchanged; deliberate divergence
from native OK. Full kf-rm tests and kf-qemu161 pass. No hardware result yet.
Prior exact89 campaign completed with Linux9+10/30 PASS, Gravity control/probe
FAIL/TIMEOUT/Xid32; matching submitted migration candidate captured. Formal
GPU execution remains unmeasured. Manual1380 lacks the same observed signature;
personal image chain retained. Next job needs lock/disk headroom;108GiB free.
See compact resume and gravity-vulkan/registration-refusal/README.md.

## Latest checkpoint, 2026-10-11 22:29 UTC

Last confirmed public push8358dabab, private1dc9c0e. Exact89 Linux9+10/30 PASS;
Windows1490 gears/vector/CUP8 PASS, TorchFAIL/new hard ceiling/reboot.
Matched Gravity control1495 FAIL772s/unresponsive/Xid32, UNCLEAN-kill;
probe1510 TIMEOUT602s/three153/per-appGSPcycle1/Xid32, acpi stop.
Probe captures GP[4b63] subch6/method200 payload4000002c matching registered
FIFO-update API on Passthrough token27/host4e. Bundle grows32768→65536 and
relocatesFIFO/USERD. Earlier binding/GPU actual execution unmeasured; sample is
not formal H1 falsification. Translated-only implementation guard NOT satisfied;
late registration also cannot safely convert a token-used/scheduled twin.
No migration/product fix/privileged host operation added. Full original logs,
public-layout decode, marker-authoritative shutdowns in late-fifo-89e8ddf8.
Campaign finished; verify noQEMU/DMA-FQ/lock before next job. Personal1380chain
retained. Owner options/next decision in final evidence README and compact resume.

## Latest checkpoint, 2026-10-11 22:01 UTC

Last confirmed public push05271adf3 (Windows1490 smoke + capacity failure), private
1dc9c0e. Fresh same-source control1495 reproduces hostXid32/channel4e intr00800000
at21:55:37UTC and GSPcycle1. Original600s final oracle pending; probe1510 follows.
Fault target c1d0007a:ff040025 remains Passthrough/Windows-user-work; FIFO update
API4000002d on its ff1fe010 object registers0. Default-off sampler records no late
candidate in control. Saved live host journal query0/Xid1 and selected sequence
in gravity-vulkan/late-fifo-89e8ddf8/control-live-observation. No translated-only
migration justified; actual software trigger/initiating cause still unmeasured.
No walk capacity ceiling in that prefix; keep separate from smoke1490TorchFAIL.

## Latest checkpoint, 2026-10-11 21:55 UTC

Last confirmed public pushee8764481 (exact89 Linux9+10/30 green), private1dc9c0e.
Windows1490 exact89: gears/vectorAdd/CUP8 PASS, torch_ai_bench FAIL411s with
unexpected guest reboot/GSPcycle1; precise-window kernel journal Xid0/query0.
The no-capacity-refusal hypothesis is FALSIFIED: slot48 needs1049632 rows versus
hard walk ceiling1048576, split9194 TRUNCATED/refuse_mask0x10. This differs from
optional-headroom exhaustion fixed earlier; no full Torch fix claimed. Pool
itself has2^20 walk rows, so removing per-space guard alone is invalid. Complete
reboot cause/bugcheck code unmeasured. Final log/original oracle/launch metadata
published in gravity-vulkan/late-fifo-89e8ddf8/smoke-result. Matrix104 counts not
changed by this confirming FAIL. Gravity1495control/1510probe continue; no fix
or actual software trigger claimed yet. Personal1380chain retained untouched.

## Latest checkpoint, 2026-10-11 21:43 UTC

Last confirmed public push1b2d3fb22 (saved owner image, takeover/runners), private
1dc9c0e. Exact product89e8ddf8 Linux gates9/9+gate10 and fast30/30 PASS, zero
FAIL/CRASH/NOTRUN in the corrected suite. Original fast runner precondition
NOTRUN/exit1 retained separately. Per-revision binary SHA and build/client/
fastguest/fast logs published in gravity-vulkan/late-fifo-89e8ddf8/linux-result.
Windows1490 smoke starting, then matched Gravity1495control/1510probe. No new
Windows verdict yet; Vulkan unfixed. Owner personal1380chain remains preserved.

## Latest checkpoint, 2026-10-11 21:39 UTC

Last confirmed public push89e8ddf8b, private1dc9c0e. Owner releasedthe trusted test host;
manual1380 stopped UNCLEAN-kill21:33:57 after failed ACPI/QGA shutdown. Installed
image + backing chain retained; writable personal child/firmware/KEEP.json in
owner-state/manual1380. qcow2 check0 is not NTFS shutdown proof. User explicitly
calls it convenience only; no VM disk/credentials in Git or on Vast.
Owner-reported Minecraft/GPU freeze recorded separately, matching workload and
Windows reference unmeasured. Guest three153, no queried Display4101; no new
hostXid. No canonical104 matrix counts changed.

Old allocator queue cancelled before running, combined into exact89 campaign.
Linux gates9/9+gate10 PASS, build/client/fastguest succeeded; initial fast suite
NOTRUN due missing runner output directory, exit1 preserved. Corrected archived
resume runner started21:38UTC under shared GPUlock; fast30 pending, then fresh
Windows1490 allocator smoke,1495 Gravity control,1510 opt-in late peek probe.
Probe is observation only, no migration/native registration or Vulkan fix.
See compact resume for paths/PID/decision guards. Personal1380chain never tested.

## Latest checkpoint, 2026-10-11 21:19 UTC

Last confirmed public pusha7719952a, private1dc9c0e. Reviewed late diagnostic
source passes161 GPU-free tests. This follow-up moves lifetime publication to
admission before queuing, covering hostile Free even before the allocation act
starts; later objects cannot replace the pending flag. Independent re-review
found no blockers; final161/161 PASS. Mode stays opt-in, candidate capture only.
No GPU work was performed; manual1380 remains reserved for the owner. Linux
hardware regression and matched GravityMark control/probe remain pending.
See RESUME_CODEX_WINDOWS_20261011.md and gravity-vulkan/h1-channel-audit.

## Latest checkpoint, 2026-10-11 21:16 UTC

Last confirmed public pushff1e3cd7e (offline H1 audit), private1dc9c0e.
This source checkpoint adds only an opt-in late measurement mode:
KF3_RELAY_PB_PEEK=after-fifo-update. It samples after registration using the
existing hard bounds, tracks 5080 allocation lifetime through concurrent free,
and never migrates/registers/executes guest bundles. Independent review found
and resolved lifetime and diagnostic multi-datum decoding gaps. Three new tests
failed first, final kf-qemu lib161/161 PASS. Probe still cannot prove GPU execution
or alone falsify H1; earlier binding and timing/coverage gaps are documented.
No hardware validation or GravityMark fix claimed. Protect owner manual1380;
after owner ends it, allow queued allocator job and honor the shared GPU lock.
Then exact-revision Linux gates9+10/fast30, fresh matched Gravity control/probe.
Resume doc and gravity-vulkan/h1-channel-audit contain the full decision guard.

## Latest checkpoint, 2026-10-11 21:07 UTC

Last confirmed public push249e4e248, private1dc9c0e; this commit publishes the
next offline audit. Owner is trying manual1380 and will notify when done.
No GPU job, QGA access or owner-session change during this audit.

Run1361 fault channel4e is Passthrough/user-work; schedule and over4096 relay
submissions precede FIFO-update registration. Three registering channels audited:
two Translated register no FIFO update, one Passthrough registers it. VFIO also
registers after scheduling. Actual0x200 trigger remains UNMEASURED, so H1 is
inconclusive and the Translated-only implementation guard is not met.
Registration-time conversion before token/schedule does not fit this workload.
Public595.84/595.91.07 source clears PBDMA interrupt before this update, contrary
to the plan's wording. Audit script, compiler-derived public layout, input hashes
and selected original log lines are in gravity-vulkan/h1-channel-audit.
No product migration, privileged host call or new hardware result is claimed.
Next: bounded post-registration trigger evidence, after owner's session ends.

## Latest checkpoint, 2026-10-11 20:23 UTC

Last confirmed public push fc6f85b4a; private1dc9c0e. Owner explicitly reaffirms
that the solution must follow v3: authored unprivileged host operations,
validated ownership/lifetime, real GPU ordering/completion, no blocking vCPU.
Native deferred registration is not permission to execute a privileged action.
No production deferred-update caller or Vulkan fix was added.

Native uid65534 USER discriminator: NOP clean; update and update-hold each
Xid32 despite rc0/GPU tail. HOLD_BEGIN20:17:17.639216UTC, Xid17.639516UTC,
HOLD_END19.639287UTC: fault300us into ownership hold falsifies early cleanup
for this native case. Root read-only diagnostic queries return counts0/1/2,
97-byte records, privately backed up. Exact public schema leaves nested fields
opaque, so no inner status/faulting method/VA is claimed. Permissions remain
a public-source hypothesis, not a measured cause. GravityMark's exact trigger
still needs measurement; identical Xid bits alone do not identify it.

Exact3252 Linux gates9/9+gate10/fast30 and both normal Windows controls1390/
1460 PASS gears/vectorAdd/CUP8, zero recorded guest recovery/GSP and precise
hostXid. Captured PDISP caps-page ablation1400 BOOT_FAIL/Code43; aborted1401
before app execution, unclean shutdown recorded, DMA-FQ restored, hostXid0.
This is separate from system GET_CAPS_V2 feature bits. No caps default promoted.

Manual1380 READY20:17:45UTC, product3252a087, NVIDIA Code0, vast account,
autologon disabled, no input/apps automated, no expiry. Owner notified with
credentials; password stays solely in trusted host secrets file. GPU lock
parent209237 remains held throughout the VM. Do not start hardware tests or
another QGA client in this session. Stop request, only when owner wants it:
/tmp/kf-stop-codex-manual-1380. Allocator65530 validation still waits for its
actual exit. Remaining19 behavior flags are explicit; no zero-flag boot claim.

Canonical104:79cleanPASS,8PASS_WITH_RECOVERY,11FAIL,6TIMEOUT; Linux59/65,
mixed revisions explicit. Same-GPU VFIO GravityVK PASS183s remains strongest
reference. Public/private results pushed separately; raw RAM/disks excluded.
Open: Vulkan causal method and valid v3 fix, remaining Windows failures,
derived caps/defaults, allocator hardware validation after owner session.

## Latest checkpoint, 2026-10-11 20:02 UTC

Last confirmed public push428ad66d7; this checkpoint records later evidence.
Same-GPU RTX4070 VFIO GravityMark Vulkan PASS183s/rc0: image864279bytes,
118694timing lines, NVIDIA3D98.43%/copy44.80%, guest153/4101/WER/reboot0.
Original600s workload/oracle. Reference physical12GiB versus kayfabe-declared4GiB
explicit; reference measured peak2247MiB. GSPcycles unmeasured: observer has
21837 records, generations1/2, no reported missing records, incomplete header;
whole-boot generations are not app recovery counts. Kayfabe1361 remains
TIMEOUT603s/hostXid32/GSPcycle1. No Vulkan fix or common causal method claimed.

Reference captures2successful50800104 registrations, inner20801116 channel-info
update. This deferred operation changes GPFIFO/USERD channel metadata; it is
separate from mapping/unmapping batches with a final invalidate. Registration
stores an API handle; the5080software method triggers execution. Public source
classifies FIFO update non-stalling, clearing PBDMA interrupt before execution;
a later GPU tail alone therefore does not validate asynchronous action success.
Native update currently faults while NOP does not. Next native discriminator
retains resources2s after real GPU tail before cleanup; a fault during the hold
rules out early cleanup for that native case. All channels USER/uid65534. Global
RC journal query as uid65534 refused0x1b INSUFFICIENT_PERMISSIONS; no empty-journal
claim. Separate root diagnostic read is queued, supplies no implementation data.

Native diagnostic now runs before manual VM creation, under the original serial
lock, after production controls; it cannot touch an already-ready owner VM.
The old queued native waiter was canceled before lock acquisition. Manual source
remains frozen under a copied path; no automation/expiry in the actual session.
Manual1380 NOT READY. Allocator65530 validation still waits for its actual exit.

Exact3252 Linux gates9/9+gate10, build/raw client/fast30/30 completed successfully,
USER birth checks pass. Windows timer-default control1390: gears/vectorAdd/CUP8
allPASS, zero guest events/GSP and precise-window hostXid0. Display-capability
ablation1400 has BOOT_FAIL/no NVIDIA adapter; subsequent requested rows still
running, no default promotion or zero-flag boot claim. Final production control
follows. No active runner was edited.

FurMark stress1370 cleanPASS with fresh renderer evidence is now folded into
canonical checkpoint:104executed,79cleanPASS,8PASS_WITH_RECOVERY,11FAIL,6TIMEOUT;
Linux counterpart parity59/65. Mixed product/harness revisions remain explicit.
Public observer/benchmark artifacts backed up; password-input traces and guest
RAM/disk excluded. Full private dump stays trusted-host only and regeneratable.

## Latest checkpoint, 2026-10-11 19:39 UTC

Last confirmed public push fd54eef40. Canonical matrix remains104/104 executed:
78cleanPASS,8PASS_WITH_RECOVERY,12FAIL,6TIMEOUT; Linux parity58/65, mixed
revisions explicit. Additional FurMark OpenGL stress1370 passes using fresh
renderer-output evidence, zero guest recovery records; canonical integration
pending evidence review. Same-GPU RTX4070 VFIO GravityMark Vulkan reference
started19:37:51UTC, original600s workload/oracle, guest580.88; result pending.

Diagnostic1361 Vulkan TIMEOUT603s: hostXid32 at19:09:06.062454UTC,
PID302124/channel0x4e, first guest153 about28.576s after app start, three153,
no4101/live-kernel/WER/reboot, oneGSPcycle. Mid-run screenshot shows asteroid
rendering468.3FPS before failure. Diagnostic1360 nbody rc0/29s but FAIL for
missing GPU proof; survival alone is not a pass. Neither replaces canonical.
PT snapshot hypothesis INCONCLUSIVE: only free-time captures, no stall capture
and zero decoded segments; global capture budget and segment window missed
needed fault-channel coverage. Zero refusal/held counters do not prove all
PT/PD mappings or lifetime/invalidate synchronization correct.

Private1362 hardware-hang breakpoint hit after hostXid32. Guest CPUs paused
about33s for a trusted-host dump and resumed; original app deadline retained,
final TIMEOUT. This confirms the later hang path, not the initiating GPU cause.
Raw memory and closed-image diagnostic notes remain private; no implementation
interface or constant is sourced from them. Bounded public-header RC journal
reader builds; not hardware-tested before VFIO detached the host driver.
Read-only journal may expose earlier fault details; absent/denied records are
inconclusive. No production caller, GPU work, or watchdog/history modification.

Frozen3252 Linux regression, boot-default controls and manual1380 follow VFIO;
manual NOT READY. No auto-input will run in the manual session. Capacity65530
hardware validation still waits for that session to exit. Public and private
scripts/evidence are backed up separately; regeneratable RAM dump stays on
trusted host, excluded from Git and Vast. GravityMark Vulkan fix remains open.

**2026-10-11 19:10 UTC, Codex Windows campaign:** last confirmed push773e818e5.
Latest combined104/104:78cleanPASS,8PASS_WITH_RECOVERY,12FAIL,6TIMEOUT;
Linux parity58/65, mixed revisions explicit. Direct stream1350 cleanPASS;
CUDAclpeak1310 TIMEOUT1504s/hostXid109, no guest driver/recovery records.
Native unprivileged deferred update faults with Xid32 while same-binary NOP
is clean; hypothesis failed, no production caller. Precise journal endpoint
fix prevents missing current-second faults. Native video FIFO positive control
fails, no notification implementation admitted. Vulkan snapshot1361 active;
private hang diagnosis follows. Event153 alone is not TDR proof. Manual1380
and exact3252 regression queued; allocator65530 validation waits for manual exit.
[Detailed handoff](../HANDOFF_FOR_CODEX_20261011.md).

**2026-10-11 18:43 UTC, Codex Windows campaign:** last confirmed pushc30b7f679.
Run1288 final Vulkan TIMEOUT611s with device-lost/hostXid32/GSPcycle1; cause
still unmeasured. Canonical103/104:77cleanPASS,8PASS_WITH_RECOVERY,12FAIL,
6TIMEOUT,1pending; Linux parity57/65. Native unprivileged authored deferred
channel-update probe queued; 66GPU-free host testsPASS, no product caller.
CUDAclpeak1310 active; owner manual1380 still queued. Compatibility axes/source
tiers follow integration's V3_CONSTANT_STRATEGY; reference measurements do not
establish unmeasured dies/branches. [Detailed handoff](../HANDOFF_FOR_CODEX_20261011.md).

**2026-10-11 18:21 UTC, Codex Windows campaign:** last confirmed push8d46a807.
Run1288 GravityMarkVK device-lost/submit/present dialog confirmed after owner
report; hostXid32/channel0x4e at18:17:10UTC, GSPcycle1. Specific method/cause
unmeasured; stock timeout does not establish same error. Original600s verdict
pending. Canonical102/104 executed:77cleanPASS,8PASS_WITH_RECOVERY,12FAIL,
5TIMEOUT,2pending; Linux parity57/65. Existing serial queues unchanged, manual
1380 not ready. [Detailed handoff](../HANDOFF_FOR_CODEX_20261011.md).

**2026-10-11 18:07 UTC, Codex Windows campaign (`codex/windows-matrix-20261011`):**
Last confirmed push65530e6a fixes false walk-slot capacity refusal under optional
headroom pressure,421GPU-free testsPASS/3ignored; hardware pending. Run1284 AI
rebooted (FAIL450s, GSPcycle1) after capacity refusal/channel death. Complete
causality unproven; exact diff demand inferred upper bound, not a captured fact.
Latest candidate100/104 executed:77cleanPASS,8PASS_WITH_RECOVERY,10FAIL,5TIMEOUT,
4pending; Linux parity57/65. Fresh GravityD3D12/OpenGL also fail with recovery,
GL hostXid32. Coverage and existing probes remain serial. Frozen3252 timer-default
Linux/control campaign32315 precedes owner manual1380, still queued, no automatic
input/login/apps or expiry. New capacity validation80736 waits for personal VM
exit before locking/building/running1480+; cannot interrupt it. Reference580.88
complete103PASS/1GravityVKTIMEOUT, owned Vast retired/evidence backed up. Other
boot defaults still open, captured/placeholder replies not promoted. Integration
and master unchanged. No full-green or zero-flag Windows claim. Resume detailed
[handoff](../HANDOFF_FOR_CODEX_20261011.md) and [evidence](../../traces/windows_matrix_20261011/README.md).

**2026-10-11 17:52 UTC, Codex Windows campaign (`codex/windows-matrix-20261011`):**
[Current evidence and run instructions](../../traces/windows_matrix_20261011/README.md).
Last confirmed push3252a087: native timer mapping default,906 GPU-free testsPASS;
new hardware validation queued. Previous exact product72741e2b passes Linux gates9/9+10 and
fast30/30, all births USER. Windows gears/vectorAdd/CUP8 pass cleanly; managed
still times out at translated SYNC_WAIT. Native authored video semaphore/tail
passes; notification experiment is queued, production kernel video still refuses.
Combined canonical97/104 executed:77cleanPASS,8PASS_WITH_RECOVERY,7FAIL,5TIMEOUT,
7pending. Per-row mixed product/harness revisions are explicit in current TSVs.
Linux counterparts57/65 all clean. Normal-launch FurMark1270 now passes without
recovery/Xid; old failure unroot-caused. Fresh nbodyGUI times out with host Xid32
and one GSPcycle. ValleyOpenGL also TIMEOUT/Xid32; vkcube renders but fails
GPU activity proof. FurMarkstress also proof-only FAIL; fresh logfile collection
bench0c5fa450 queued, product unchanged. Stock counterparts PASS; bounded nbody snapshot queued. Ordinary Edge launch only proves welcome-window survival;
owner-observed black recycle remains open. Main lane MPOoff, eight overlay rows separate.
Matching stock580.88 reference complete104/104 across batches: latest103PASS,
GravityMarkVKTIMEOUT, zero recorded guest driver/reset events in passing rows;
native GSPcycles UNMEASURED. Different GPU(RTX4090) from candidate(RTX4070).
Corrected direct CuPy stream reference passes; candidate fresh retry queued.
Owner manual1380 queued after verified3252 regressions/controls, no automatic
input/sign-in/apps or diagnostics. Other required boot defaults remain under audit.
Owned reference Vast55385171 retired after evidence backup, exactID absence verified.
IPC WDDM-unsupported; Geekbench packaging gap. Integration/master unchanged;
no full-matrix or complete Linux parity success claimed. Resume latest detailed handoff.

**TODO, owner-requested, 2026-10-14: Win11 S3 sleep/resume ("the sleep fix").** The owner's Win11 "crash after idle" was ACPI S3: the guest sleeps on its idle timeout and the wake never completes.
Reproduced on the Linux/ogkm desktop guest [measured]: suspend is clean (fn47 `bInPMTransition=true newLevel=3`, GSP Halted), QEMU's wakeup is a system reset (kf3 device reset, GSP Halted -> Cold), and the guest's
resume-mode GSP boot (`KGSP_BOOT_MODE_SR_RESUME`, `kgspBootstrap_TU102`) fails with `Failed to boot GSP` / NV_ERR_NOT_READY, then Xid 119. Cause [inferred]: `kf-gsp` resume S6-S8 is unbuilt and the wake reset discards
the suspended state. Evidence and the repro (`scripts/bench/display/desktop_s3.sh`): branch `claude/s3-resume-20261014`, `traces/linux_s3_resume_20261014/`. Work in flight: an Opus agent on that branch (resume in the FSM +
device-reset path; Linux S3 x3, gates, fast suite, planes oracle on the stock module). **Interim:** Win11 runs with sleep disabled (`powercfg /change standby-timeout-ac 0`) so Edge/YouTube testing can proceed;
**undo that when the fix lands and retest Win11 S3** (idle sleep + wake, 3 cycles, no TDR). Also open from the same day: the 1001 ms plane-update "ready, waiting for a vblank" STALL report
(`traces/linux_planes_oracle_20261014/`), 3-plane / HDR YUV, ILUT/CSC/TMO on YUV windows, the lane-A scanner/RESET_CHANNEL fixes (`claude/user-ring-scan-20261014`) not yet validated on Win11 Edge+YouTube for 15 min.

**Windows app matrix lane built (GPU-free), 2026-10-10 (branch `claude/windows-appmatrix-20261010`, design [`V3_WINDOWS_APP_MATRIX.md`](../design/V3_WINDOWS_APP_MATRIX.md), STATUS DESIGN-ONLY).**
⊘ *This supersedes the "Windows app-matrix lane is TODO and does not exist" sentence of the next entry:* the inventory (100 Windows apps; all 71 Linux rows mapped or explained),
a 6.8 GB read-only app disk (`/var/lib/kf-windows-20261005/appmatrix/image/kfapps.iso`, 44 sha256-pinned downloads, no guest internet needed), the QGA driver
(`scripts/bench/windows/appmatrix/run_windows_apps.sh`: detached supervised apps, TDR/WER/GSP-cycle accounting, fresh-guest recovery, phase-2 isolation, the Linux
`APPRES` vocabulary) and a mock guest with 66 unit tests exist. **Nothing has run on a Windows guest**; the design doc's §5 gives the exact steps once the desktop stays
up and §6 the untested list (start with the USB CD-ROM hot-plug and `kf_guest_setup.ps1`). Open owner decision: a VFIO Windows arm as the per-app baseline (§7).

**Host patch tier plan and an owner requirement on the Windows app matrix, 2026-10-10 (docs only, branch `claude/host-patch-list-20261010`).**
(1) The owner policy of 2026-10-10 is binding: the STOCK install works with no kernel patch; a patched host kernel/driver is an
optional better tier, and the owner now plans to implement it. Every candidate (in-kernel doorbell, mdev shim, b3 nvidia-uvm,
host-owned UVM, the USERD-in-sysmem address-size fix, exact partial unmap, scatter-map verb), with its stock fallback, runtime
probe, security bound, performance label (reasoned vs measured), validation plan, order, stock-path guarantees and the DKMS /
version-pin / signing / unload story, is in `docs/design/V3_HOST_PATCH_LIST.md`. Recommended first patch: the in-kernel doorbell as a
separate kayfabe module (no `nvidia.ko` rebuild), behind a bare-metal wake-to-ring measurement that can drop it. `[measured, 2026-10-10]`
the b3 patch (written for 580.159.04) passes `patch --dry-run` against an ogkm 595.84 tree (textual only; the trusted host runs
595.91.07 and no hardware re-run exists). (2) **Owner requirement: the app matrix must eventually also run on Windows guests.** A
Windows app-matrix lane is TODO and does not exist. It needs: an app inventory (none yet; `docs/design/V3_APP_MATRIX.md` is Linux
only: 61/65 apps + 6/6 probes at `6692e621`), Windows builds or binaries of the CUDA apps and probes (only `scripts/bench/windows/cuda/build_cup_win.sh`
exists), a runner with per-app timeouts and a pass/fail oracle compatible with `scripts/apps/apps_matrix.sh` / `summarize.py`, a
non-interactive way to start the guest and sign in (the scripted sign-in harness exists **host-only** at
`/var/lib/kf-windows-20261005/winprod`, not in the repo), a host-native baseline per app, and TDR resilience (the guest still TDRs
today, so each app needs a recovery and result-collection path through QGA, `scripts/bench/windows/qga_run_ps.py`). It is gated on the
Windows desktop staying up; it is not a merge-bar item yet.

**Windows reset after the first flips, 2026-10-09 (branch `claude/windows-reset-20261009` = `claude/display-reply-diff-20261008`
+ `claude/kf3-read-trace-20261008` + `claude/passthrough-nsi-nogate-20261008`; [record](../../traces/windows_reset_20261009/README.md);
4 hardware boots, runs 100-103, RTX 4070).** ⊘ The trace-mode ruling of the read-trace branch is renumbered **§Y** in
`OWNER_RULINGS.md` (§X is the non-stall ruling); KF3 ABI 25 = the trace verbs + the input-sink verbs. `[measured, runs 98-103,
2026-10-09]` (1) Runs 98/99's "reset ~19.6 s after driver start" is TDR + bugcheck **0x116**: kf3 refused Windows' mirrored
DIRECT10 colour program (`ILUT/OLUT_CONTROL 0x4050a`, MIRROR=1; ILUT offset in 256-byte units) and halted its display engine;
fixed behind `KF3_DISPLAY_LUT_MIRROR=1 KF3_DISPLAY_ILUT_OFFSET_256=1` (default off; GPU-free test with the measured program):
flips then run at 60 Hz and Windows draws its lock screen through kf3 (frame in the record). (2) The guest still TDRs (0x116,
runs 100-103): in runs 101/103 the D3D process's Passthrough GR/CE twins take **host Xid 31 (FAULT_PTE at `0x4034000`/`0x4036000`)**
after kf3 applies page-table walks that UNMAP other rows of the same 2 MiB region (H-pde, stated, not run); in runs 100/102
there is no Xid and the new **stall snapshot** (`KF3_PT_STALL_SNAPSHOT=1`, default off) shows every twin `GPGet == GPPut` with its
fences in guest memory. Falsified: the armed-rule non-stall relay as the fix (run 102), H-flipdone (run 103: every flip's
release/notify published Ok in its latch's millisecond). (3) Every TDR recovery then fails at the T-space rule because the
recovery's own copy channel (`ProcessID=4`) is classified user work — owner decision proposed in the record §11 (no policy
changed). Host left at DMA-FQ, `nvidia` bound, lock released.

**kf3 BAR0 trace mode with the VFIO reference's own tracer, 2026-10-09 (branch `claude/kf3-read-trace-20261008` from
`claude/display-reply-diff-20261008`; owner ruling `OWNER_RULINGS.md` §X; design `docs/design/V3_BAR0_TRACE_MODE.md`;
[record](../../traces/kf3_read_trace_20261009/README.md)).** Default off. `KF3_BAR0_READ_TRACE=1` makes kf3's selected BAR0
reads exit (display range + `KF3_READ_TRACE_RANGES`, or `all`) and records reads, writes and MSIs through QEMU's own
`vfio_region_read`/`vfio_region_write`/`vfio_msi_interrupt` trace events; `x-gsp-observer=` on `kf3-gpu` attaches the GSP
observer, now one device-agnostic helper shared with vfio-pci (`tools/vfio-gsp-observer`). Caps on records and bytes with a
drop counter. `[measured, runs rtoff/rton/rtcap/rtobs at 9cebeea9, RTX 4070, 2026-10-09]` off 30/30 with `read_exits` 0 as
before; on (all ranges) 30/30, +12 % client time; a 4 MiB cap held; the VFIO analysers read kf3's outputs unchanged. For
the Windows display trace: env and `win_vm.sh` switches in the design doc §7; the bench QEMU tree with the shared observer
is `/var/lib/kf-windows-20261005/readtrace/qemu-10.2.4` (the older `/var/lib/kf-windows-20261005/qemu-10.2.4` has the
VFIO-only observer, so a kf3 built there refuses `x-gsp-observer` by name). Not run yet: Windows.

**Display reply diff, 2026-10-09 (GPU-free; branch `claude/display-reply-diff-20261008` from `claude/windows-flip-vsync-20261008`;
[record](../../traces/display_reply_diff_20261008/README.md)).** Every display RPC reply and display register read-back the
Windows KMD sees on the real RTX 4070 before its first window PUT (VFIO DVI reference boot3), diffed field by field against
kf3's (bodies by replaying the hardware requests through kf's display link: `cargo run -p kf-rm --example display_replay`).
From the modeset to the DDI both guests send the same RPCs and the same modeset pushbuffer sizes; the differences are ranked
in the record's §2/§5. Five default-off flags for ONE batched boot (record §6): `KF3_DISPLAY_PRIVATE_PROBE`,
`KF3_DISPLAY_HOTPLUG_EDID_SEEN`, `KF3_DISPLAY_BLANK_STATE`, `KF3_DISPLAY_ARMED_DEFAULTS` (new) + `KF3_DISPLAY_HDCP_STATE`;
prediction: a window-0 PUT after the modeset (`puts` > 39). Not yet implemented, next if falsified: an HDMI_A-in-DVI-mode
connector (H-hdmi) and the crossbar / `DFP_ASSIGN_SOR` path (H-xbar). ⊘ The record's §0 (ordered stream alignment,
boot3 vs run 93) corrects "same decisions until the DDI": the first display WRITE that differs is at driver start
(11.0746 s: the hardware KMD initialises only windows 0/2/4/6, kf3's guest all eight, with different push sizes), right
after it read the caps page (101 of 1024 words differ, incl. even/odd window caps) and the firmware's ARMED state. **Runs 97-99
(2026-10-09, RTX 4070, record §7-§11):** the reply/read-back batch (commit-time controls, hotplug mask, blank/armed defaults)
is falsified (run 97, `puts=39`); the real GPU's caps page (`KF3_DISPLAY_CAPS_PROBE`, a captured table: probe only) is
sufficient on its own (run 99) — Windows then initialises windows 0/2/4/6 like the hardware and programs and flips window 0
after the modeset (`puts=63` at the marker, ~18 s of flips and VSyncs). Next wall: the display re-initialises ~19.6 s after
driver start and the guest stops answering QGA (D3D11/D3D12/nvidia-smi probes could not run). Owner question: author the
deciding caps fields (per-field bisection first) — the captured page cannot ship.

**Windows flip / vsync, 2026-10-08 (branch `claude/windows-flip-vsync-20261008` = `claude/windows-pass-20261008` + the
Passthrough completion-interrupt commits; runs 93-96; [record](../../traces/windows_flip_vsync_20261008/README.md)).**
`[measured, runs 93-96 at 68673e6e/2c77140b/5f0e3b37/61b95494, RTX 4070, 2026-10-08]` kf3 already raises LAST_DATA at
every frame edge while enabled (now one tested function), so H-flip's premise does not hold; the first divergence is
earlier: **Windows never programs a window into kf3's display after the modeset** (display PUT count 39 from the modeset to
the TDR in every boot), so no flip can be retired by a VSync. Falsified as the cause: EVT_STAT LOADV (`KF3_DISPLAY_LOADV`),
the refused GET_HDCP_STATE (`KF3_DISPLAY_HDCP_STATE`), instant core-update completion (`KF3_DISPLAY_CORE_AT_VBLANK`); all
three stay default-off experiments. `KF3_DISPLAY_WRITE_TRACE` (diagnostic) logs every guest display write. Next candidates
and the owner question (a display-aperture read trap) are in the record's §9.

**Windows with NVIDIA, 2026-10-08 fourth session, runs 60-72 (branch `claude/windows-display-20261008`; record: `traces/windows_code43_walls_20261007/README.md`
"Stop (fourth session, ~17:15)").** `[measured, runs 67-72, 2026-10-08]` with `KF3_WIN_USER_CHANNELS_PASSTHROUGH` (classification + USERD relay,
`docs/design/V3_USERD_RELAY.md`) and the diagnostic `KF3_TWIN_VA_BASE=10000`, the compositor's per-process channel runs on an unprivileged host twin
(all its work consumed, no Xid) and Windows reports a 1920x1080@60 mode; D3D devices created by a test process RC their twins (Xid 32, PBDMA DEVICE)
and the guest TDRs; no window surface is presented yet. Next: the D3D twins' RC (subchannel bindings vs binds answered without binding), a real rule
for the twin space's VA start, the window surface.

**Windows user work as Passthrough (OWNER_RULINGS §V), 2026-10-08 fourth session, runs 60-64 (branch `claude/windows-display-20261008`;
[record](../../traces/windows_code43_walls_20261007/README.md#loop-2026-10-08-fourth-session-windows-user-work-as-passthrough-owner_rulings-v)).**
`[measured, runs 60/61/64 at 3a578d50/883f878e/ac3456ac, 2026-10-08]`: every Windows channel is PRIVILEGE=KERNEL and `PRIVILEGED_CHANNEL`; the kernel
driver's 13 channels declare no context share and the driver's own `ProcessID`, the compositor/D3D channels a `FERMI_CONTEXT_SHARE_A` and their own
process — `kf_rm::chanlink::windows_user_work` (flag `KF3_WIN_USER_CHANNELS_PASSTHROUGH`, default off) classifies by both and was right on every
channel. Kernel channels measurably use physical operands (29 `DST_TYPE=PHYSICAL` launches on one display-side CE, run64); the one user-work segment
seen (10 words) had none (sample too small). **Wall:** a Passthrough twin cannot adopt Windows' USERD (guest system memory; host IOMMU `DMA-FQ` gives
IOVA `0x7ff9...`, wider than a USERD: host RM `NV_ERR_INVALID_ADDRESS`); `RMInstLoc` in the guest registry does not move it. Next: a USERD relay
(kayfabe-owned USERD, GP_PUT/GP_GET copied, ring/push buffers unparsed) or a host IOMMU identity domain; decisions listed in the record.
`windows_broker.sh` runs instances side by side and stops cleanly.

**Windows display loop, 2026-10-08 third session (branch `claude/windows-display-20261008` from `ac5d086f`, runs 54-58;
[record](../../traces/windows_code43_walls_20261007/README.md#loop-2026-10-08-third-session-branch-claudewindows-display-20261008-windows-on-screen-and-the-d3d-wall)).**
New launcher `scripts/bench/windows/windows_broker.sh` (Windows in a second broker window, its own socket/title; `desktop`, `autologon`,
`nvidia disable|enable`, `reboot`, `stop`). `[measured, runs 54-58 at a88764b3/03932e9f/40230e23, 2026-10-08]`: kf3 `gop=on` without the std VGA shows
OVMF and Windows' boot in the window; the 7 FB indices are served (AdapterRAM/qwMemorySize now 4 GiB, was 0); H-preempt-bind is FALSIFIED as the
cause of `0x8876017c` (binds answered, same HRESULT), the create then stops at a cross-client `ZCULL_BIND` and the software-runlist submit; display
probes (IMP_ENABLE TRUE + six echoed NV0073 controls) start mode validation, and with the GR refusals also answered (run57, labelled experiment) the
NVIDIA driver **commits 1920x1080@60** (head armed, CCD configuration stored) — then the desktop's first 3D segment dies in the rewriter (subchannel 0
never bound in the ring), TDR, bugcheck 0x116. Owner decision needed: WDDM user work runs on guest-KERNEL channels (options a/b/c, recommendation (c),
draft ruling in the record). A visible interactive desktop exists today only as the **Basic Display fallback** (run58: NVIDIA disabled, software
rendering on kf3's GOP framebuffer). Two kf3 VMs share the GPU without Xid, but the Linux guest's 8 GiB store leaves no room for a 2-4 GiB Windows store.
