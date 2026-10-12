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
