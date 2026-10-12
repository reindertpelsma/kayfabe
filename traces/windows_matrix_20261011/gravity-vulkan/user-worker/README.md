# Restricted USER worker diagnostic

STATUS: RESEARCH, 2026-10-12. Source review cleared one default-OFF diagnostic;
no new Windows compatibility claim. Source revision is the containing commit.
Previous pushed source checkpoint32312b0c6.

Implemented worker-only native USER submission from the first source entry,
bounded own-VAS source admission, exact software-object classification, actual
private-ring descriptor-fetch receipts, immutable staging only for intercepted
triggers, and host-authored own-object deferred actions. Linux Passthrough and
kernel Translated retain their routes. No pushbuffer work on a vCPU.

`KF3_WIN_USER_RING_RELAY=1` is an experiment, never needed to boot. Initial
migration source cursor zero remains a hypothesis. The native private cursor is
never reset. Before the Windows run: hypothesis is that the registered new source
starts at zero and software source replacement at the intercepted stream point
preserves work. Falsifier: refusal, missing progress, wrong output, device loss,
Xid32 or guest141/4101. Native VFIO GravityMark Vk reference PASS183s. This does
not establish cursor semantics or compatibility before that run.

Measured GPU-free: six-crate suite1915PASS before final cancellation gating adjustment; exact final
kf-qemu suite (retirement-tests.log) passes, including two new cleanup tests.
Actual Plane admission tests exercise shared Cell entry/liveness under Record
and Pt contention and pause/enable. Source work is a shim, not the native Pump.
Full native worker path needs hardware tests. Descriptor-fetch receipt regression
was deliberately made to fail by removing the receipt (receipt-red.log), then
restored and passed. Fetch is not GPU engine completion.

Independent review fixed classification changes, fetch/staging lifetime, birth
RAM generation checks, lost final wakes, optional notification constructor
refusal, uncertain native-Free notifier retention and final wake cancellation.
Permanent cancellation plus exact Record exclusion across index publication
prevents retained failed-cleanup records from waking a reused guest index.
Two actual Cell tests cover retained cleanup and a concurrent wake. No claim of
an exhaustive thread interleaving proof; atomic dispatch finite Loom tests are a
separate existing primitive.

Next: exact source Linux gates9+10, fast30, desktop smoke; then a single-variable
Windows relay experiment on a disposable child of the retained personal image.
No host privileged FIFO-update verb and no host patch.

## Resolved initial USERD correction, 2026-10-12 18:02UTC

Windows1550 clean-refused birth with USER_RING_USERD_ACCESS Gone. No app or
migration verdict. ACPI stopped; zero hostXid, noQEMU/DMA-FQ. See windows-1550.
CPU-local birth handles are independent of GSP MemoryList handles. Corrected
initial access uses the public resolved Sysmem descriptor, bounded512 bytes,
exact channel lifetime and guarded RAM topology. Migrated USERD still requires
its MemoryList lease. No vCPU work added. Independent source review passed;
full six-crate1919PASS/0fail/exit0 (resolved-userd-tests.log). New tests exercise
real RAM copies, bounds, topology revocation, channel reuse and graph/channel
admission order. Native worker execution still needs the next VM experiment.
Candidate channels are worker-routed from birth, not converted by registration.
Source GET credit remains actual descriptor-fetch based; private native ring
and USERD remain fixed. Owner permits a separate opt-in Linux relay diagnostic
if useful; ordinary Linux Passthrough unchanged. Sysmem-only admission explicit.

## Exact Linux result and Windows start

MEASURED on full source30c887c8651c3e79b6b62c6785eda30f443205d1, trusted
RTX4070/595.91.07: gates9/9+gate10, USERbirths12/12/refused0; rebuilt QEMU/client
and fresh fastguest; fast30/30, crashes0, NOTRUN0, exit0. Run17:32:55–17:37:28UTC.
Five Xid31 correspond to deliberate negative read/write arms; no Xid32. Evidence
linux-result (binary SHA256 and full per-arm scoreboard), linux-campaign.log.

Linux Cinnamon/Xorg/NVIDIA580.159.04 desktop/gears ~59–60FPS PASS_WITH_WARNINGS:
shared-data NOT_SUPPORTED assertions and one DRM flip timeout reproduce earlier
baseline warnings. No host Xid. Disposable child stopped, no QEMU remained and
IOMMUgroup11DMA-FQ. This is desktop smoke, not full render parity.

Windows relay experiment launched run1550 on the same pinned source. Original
600s GravityMark Vk app/oracle/media; only added relay flag. Fresh baseline child;
personal1380 chain untouched. ABORTED_ADMISSION as corrected above; no app verdict. run_windows.sh is the executed
runner (copied outside host checkout); Linux desktop runner likewise retained.

## Registration-order review on6ff5aaf29, 2026-10-12

Independent source review: route/token fixed at birth. First5080 allocation
publishes exact immutable ready=false identity under Pt; private PUT rechecks
same identity and readiness under that same Pt before setting user_published.
Allocation wins: publication retries until native child ready. FirstPUT wins:
late5080 allocation refuses by name. No opaque native history changes mode.
Bundle registration is a separate synchronous registry insertion before NV_OK;
a trigger lookup after observing success sees that insertion. A malicious or
unordered trigger racing before registration commits may named-refuse. This
is not an assertion that registration acknowledges unregistered earlier work.
Native child readiness precedes allocation acknowledgement; registration and
allocation return claims are distinct. Full native execution remains unmeasured.

Doorbell review: BUSY_RUNG retains simultaneous rings, cumulative PUT/cursor plus
again drains submitted entries; pre-turn contention/paused demand is durable and
copy Busy keeps tick/staged waits. Producer must publish GP/PB and PUT before
MMIO and obey one-empty/no-overwrite-before-GET. PUT after its final ring is not
covered. Migration continuity remains unresolved: software swaps source/cursor0
and does not drain arbitrary later old-ring entries. The driver's quiesce/copy
contract/native cursor semantics must establish what survives. New-source PUT
and rings racing the swap are observed later; this is distinct from old-source
work preservation. Do not promote the diagnostic until native/VM evidence agrees.

## Exact corrected Linux result and Windows1560 start

Exact6ff5aaf293e9fc04ac8780983f26dc04d0f3b735 Linux9+10PASS, USERbirth12/12,
fast30/30/crash0/NOTRUN0/exit0; five intended negative Xid31, no32. Desktop
NVIDIA580.159.04 RTX4070 Cinnamon/Xorg/gears59–60FPS PASS_WITH_WARNINGS, no hostXid.
Stopped/noQEMU/DMA-FQ. Evidence resolved-userd-linux. Windows1560 launched with
only relay flag added to unchanged production setup/original600s oracle/media;
result pending. No new Windows compatibility claim or cursor-continuity proof.
