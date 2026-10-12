# Resume Windows apps and GravityMark Vulkan

**STATUS: LIVE, 2026-10-12 01:15UTC.** This compact current state supersedes earlier
pending paragraphs in the chronological HANDOFF_FOR_CODEX_20261011.md.

## Branch and source

Public <https://github.com/reindertpelsma/kayfabe>, branch
**codex/windows-matrix-20261011**, controller worktree **/data/kf-inval-absent**.
Last confirmed push **f3a670f20**; this final evidence checkpoint may be newer.
Fetch the branch tip to obtain all reachable source/evidence. Do not edit the
separate /workspace/kayfabe or Claude worktrees. Do not merge master/integration
without CI/review/exact-revision hardware requirements.

Latest Windows hardware product **b081f705a7502380d584f37ab7da80e668f5018f** adds a
**default-OFF negative control**, KF3_WIN_REFUSE_PT_FIFO_UPDATE=1. It returns
NOT_SUPPORTED only for FIFO_UPDATE_CHANNEL_INFO registrations on live
Passthrough user-work parents; whole5080/other APIs/default behavior unchanged.
Native answers OK, so this is an explicit diagnostic divergence, not compatibility
support. Full kf-rm tests (131 lib, including new model, plus integration tests)
and kf-qemu161 lib tests pass. No migration/default/host-privilege change.

Private <https://github.com/reindertpelsma/kayfabe-private>, branch
codex/gravity-vulkan-20261011, last confirmed push1dc9c0e, controller checkout
/tmp/kayfabe-private-codex. Entry codex/gravity-vulkan-20261011/README.md.
Owner authorized private diagnosis; interfaces/constants stay public-source or
validated measurement derived. Do not copy private driver analysis into product.

Read AGENTS.md, OWNER_RULINGS §§U,V,AD–AI, V3_CONSTANT_STRATEGY.md,
V3_PROVENANCE.md, V3_FEATURE_GAPS.md. Current branch lacks §AH/strategy document:
read origin/integration/windows-20261010 versions with git show, do not invent
or silently import the entire newer integration. Hostile guest; authored
unprivileged host verbs; no vCPU blocking or unsafe GPU completion; no heuristics.

## Approved restricted user-ring work, 2026-10-12

Owner approved §AI: GPU-read-only private commands/staging only for declared
Windows user-work, workers and immutable inspection from birth; **Linux
Passthrough forbids host-owned mappings**. No writable private fence, inline
copy, privileged host GPU channel or late parsing conversion. Imported original
integration328fe4bf8 proposal with superseding corrections at its top. Read
[restricted review](design/V3_WINDOWS_USER_RING_RELAY_REVIEW_20261012.md).

Exact70aa0d666 adds an **unconnected** bounded ring Ledger and Windows-only
construction permit. Full kf-chan/kf-rm tests pass; failing-first and exhaustive
ordering/model logs published. Native unprivileged exact91bf measurement: USER
channel fetches read-only commands successfully; attempted FIFO write faults
RO with unchanged sentinel. GP_GET1 despite tail0 disproves GET as work completion.
A separate UID reference comparison did not provide a valid migration reference;
that route is ended. No further privileged probes or private diagnosis queued.

Linux exact70aa **gates9/9+gate10 and fast30/30 PASS**, USER births12/12. Initial
fast attempt was a documented harness failure, corrected on the same binary.
[Evidence and original logs](../traces/windows_matrix_20261011/gravity-vulkan/user-ring-p0/README.md).
No VM relay/migration/default changes and no new Windows app result. Need bounded
source/backing ownership, stream parser, permitted real completion and native
cursor semantics before runtime wiring. Registration refusal below remains failed.

Post00:35UTC noQEMU/shared lockfree/DMA-FQ/stop-file absent,129830002688bytesfree.
Personal image retained; no active loop or Vast rental. Latest hardware source
70aa at /data/kf-user-ring-host-70aa0d666; results under
$W/codex-user-ring-linux-70aa0d666. Do not replay completed runners.

Latest native reference (exact9b65e3c9e) **immediate and held PASS**: USER C56F,
read-only commands, unmapped USERD, virtual CE copy then WFI_ALL/SET_REFERENCE.
Held48 polls show reference0/CE semaphore0 while GP_GET advances; after explicit
signal real copy/reference arrive. ZeroXid, clean teardown, no personal image.
[Original facts](../traces/windows_matrix_20261011/gravity-vulkan/user-ring-reference/README.md).
New unconnected receipt-wrap/parser models pass full kf-chan70lib/all integrations;
no VM call or Windows app result. Parser rejects unsupported forms; source pins,
cross-entry packets, native migration cursors, broader completion evidence and
independent review still gate runtime integration. Latest host job is complete.

## UPDATE 2026-10-12 01:15UTC — native allocation reference PASS; Windows scope clarified

Last confirmed pushf3a670f20 (this evidence checkpoint may be newer). Native
reference source5a407dcd8/retry harnessf15da21bc Gravity Vulkan PASS183s, image
and timings, zero guest153/4101/WER/reboot and bounded hostXid0. Strict passive
export validates18923 records, all65 observedC56F alloc params complete. Four
successful FIFO registrations request32768→65536→131072→262144→524288 entries,
new GP ring addresses and SYSTEM USERD handles/slots. Registration is not GPU
execution; cursor continuity/backing retirement remain unmeasured. Evidence:
gravity-vulkan/vfio-ring-p0. DirectGP parser guards now fullGPU-free kf-abi/
kf-chan green; VM runtime unchanged. Linux hardware remains exact70aa9+10/30.
Owner permits necessary Windows-specific mapping exception, keeps Linux host
mapping prohibition; sysmem USERD alone is not an admission discriminator.
After Gravity fix: finish full Windows app matrix, then compatibility driver
sweep and derivation checks. Vast checked through API01:12UTC: no instances.
Trusted GPUidle/noQEMU/lockfree/DMA-FQ/stopabsent,123491311616bytesfree;
personal Windows chain preserved. Next native cursor sampling; no app fix yet.

## Hardware is idle; preserve the personal image

Trusted **root@172.22.1.20**, one RTX4070/AD104, host595.91.07/guest580.88.
W=/var/lib/kf-windows-20261005. Owner released manual1380 and authorized stopping
it/further runs. It stopped UNCLEAN-kill21:33:57UTC; qcow2 check0 does not prove
NTFS clean shutdown. Keep these **personal convenience** files on this host:

- $W/boundary-kayfabe-1380/windows.qcow2, original retained0444.
- $W/baseline/windows.qcow2, its backing parent.
- $W/owner-state/manual1380/resume.qcow2, separate writable personal child;
  copied OVMF_VARS.fd and KEEP.json record the chain.

Do not test on/delete/rebase that personal chain. User says all contents are
regeneratable/redownloadable/re-signin; no offsite disk backup requested. Password
is only in trusted-host $W/windows-desktop/secrets/win_password, accountvast;
never print/commit it or place disks/credentials on Vast.

Earlier refusal campaign **completed23:13:01UTC**. Root
$W/codex-fifo-refusal-20261011-b081f705, log $W/codex-fifo-refusal-campaign.log,
isolated source /data/kf-fifo-refusal-host-b081f705, unique Cargo target
/data/kf-fifo-refusal-target-b081f705. Run1540 stopped ACPI; final check23:13:44UTC:
**noQEMU, shared GPU lock available, group11DMA-FQ, stop-file absent,122.6GiB free**.
Recheck these before any GPU job; lock /tmp/kayfabe-fastguest.lock. Keep >100GiB.
Only completed own1490/1495/1510 test disks were reclaimed after162 backing-header
checks. Their full evidence is published; personal1380 was excluded and retained.
Do not replay completed runner IDs. No active loop or queued GPU job remains.

No Vast instance remains rented by this session. Vast is untrusted, no secrets,
rent vms_enabled=true/KVM, evidence in Git, teardown own IDs only. No need to rent
for the completed experiment. Kayfabe and VFIO cannot share this single GPU at once.

## Latest exact-revision results

Exact **b081f705a**: Linux gates **9/9 + gate10 PASS**, USER births12/12,
client/fastguest rebuilt, **fast30/30 PASS**, zeroFAIL/CRASH/NOTRUN.
Original logs and binary SHA in gravity-vulkan/registration-refusal/linux-result.

Windows **1540**: **TIMEOUT605s/rc124**, original600s app workload, no image/timing
proof. Rendered asteroids followed by Vulkan device-lost/cannot-submit dialog.
One FIFO registration refused: clientc1d0007d/objectff1fe010/API40000030,
inner20801116, status0x56; VM refusal counter1. **HostXid32/channel4e at23:02:37UTC**,
about19.7s after app start. Final kernel window query0/Xid1 including shutdown.
**Guest153/4101/WER0, no reboot, per-appGSPcycles0**. This differs from earlier
recovery but is not a pass. Complete boot log has one Running→Suspending during
ACPI shutdown; do not count it as app recovery. Actual later refused-handle
trigger/faulting method remain unmeasured; missing samples are not zero triggers.

No walk-ceiling/slot-pool/OverDeclaredCap/HELD lines in1540; these do not prove
mapping/TLB correctness. Boot invalidate max51373us is not fault-time causality.
The clean registration fallback hypothesis is **FALSIFIED**. No alternate status
run: no specific status-handling error measured. Diagnostic remains default OFF.

[Full final facts, screenshots, logs and falsifiers](../traces/windows_matrix_20261011/gravity-vulkan/registration-refusal/README.md).
Recording EXIT0 means completed collection, not application PASS. Gzip full QEMU
log4247648bytes verified against source SHA; marker-authoritative ACPI shutdown.

## GravityMark cause and next decision

Same RTX4070/Windows580.88 VFIO **PASS183s**, original600s workload/image/timing
and NVIDIA activity; zero guest153/4101/WER/reboot. Native physical12GiB versus
Kayfabe declared4GiB explicit; peak2247MiB. Reference GSPcycles unmeasured.
Observer has two successful FIFO-update registrations, no plain20801116 RPC;
GSP can execute deferred work internally, so absence of a plain RPC does not
prove native skipped migration. Original alloc params/USERD aperture/size absent.

Fault twin4e in1361 is **Passthrough/user-work**, scheduled and4096+ forwards
before registration. Two Translated registering channels have no FIFO updates.
Late conversion at registration is invalid. Successor1510 captures submitted
subch6/method200/API4000002c matching FIFO-update bundle, old32768→65536 entries,
newFIFO0x42be000/USERDoffset0x600. Earlier binding and GPU execution unmeasured;
not formal direct-doorbell/actual-execution H1 falsification. The target is not
Translated, so the proposed Translated-only software fix is inapplicable.

Exact89e8ddf8 Linux9+10/30 PASS; control1495 FAIL772s/Xid32/unclean stop;
probe1510 TIMEOUT602s/Xid32/three153/per-appGSPcycle1/ACPI stop. Earlier native
USER deferred update faults, NOP clean; hold fault300us into2s ownership hold
falsifies early cleanup for that native case, not GravityMark cause. Public
source saves caller privilege; inner FIFO update remains privileged. PBDMA
interrupt is cleared BEFORE FIFO update execution (earlier plan reversed it).

**No migration implemented.** [Concrete design options/cost limits](design/V3_WINDOWS_USER_FIFO_MIGRATION_DECISION_20261011.md):
restricted mediated user channels chosen at birth (stable host ring, own user VAS,
physical operands denied), optional validated H8 host tier, or explicitly unsupported
stock scope. Recommendation: stock-mode bounded model and public/measurement
cursor/lifetime semantics work first; H8 separate. Registration refusal did not
fix GravityMark; relay-only pointer change cannot re-point the native FIFO.
Close actual-trigger/cursor/lifetime gaps before migration code; no doorbell
boundary, late conversion, privileged stock verb or guessed cursor semantics.

## Broader app status and open defects

Canonical combined104 rows: **79 cleanPASS,8 PASS_WITH_RECOVERY,11 FAIL,6 TIMEOUT**;
Linux counterpart59/65. Mixed explicit revisions, not one-revision full-green.
This failed experiment does not change canonical counts. OpenGL gears passes.

Exact89 Windows1490 gears/vectorAdd/CUP8 PASS; TorchAIbench FAIL411s/reboot/
GSPcycle1 with hard walk need1049632 >1048576. This differs from optional growth
headroom fixed65530e6a; walk pool itself2^20, cannot simply remove the guard or
raise a threshold. Need bounded capacity design preserving invalidate latency.
Full reboot cause unmeasured. Linux exact89 was green.

Native PTIMER default3252a087 passed Linux9+10/30 and Windows1390/1460 smokes.
**19 other behavior flags remain; no zero-flag stock Windows claim.** Removing
captured PDISP caps probe gives BOOT_FAIL/Code43; derive correct public-source
capabilities rather than promoting a captured page. Manual1380 Minecraft/GPU
freeze is owner-reported; bounded records lack Gravity's newXid/FIFO signature,
matching edition/workload Windows reference unmeasured. Keep it separate.

Push at least every30min; update this compact state and the chronological
handoff at each block. Filter large logs on host; no compilation during app timing.
