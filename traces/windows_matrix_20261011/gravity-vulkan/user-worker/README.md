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

Measured GPU-free: six-crate suite1913PASS before final cleanup fix; exact final
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
