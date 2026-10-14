# Independent review of late FIFO-update sampling

**STATUS: RESEARCH, 2026-10-11 21:16UTC.** Read-only independent review by
`/root/review_late_fifo_probe`, after the owner's request for an independent
review. No reviewer edits, hardware commands or private diagnosis.

Final review: no blocking security, default-off behavior or lock-order finding.
The probe is suitable for bounded candidate capture, with the limitations below.

Resolved during review:

- Multi-datum packets hid triggers: three failing-first tests and diagnostic
  packet expansion now cover Incrementing, NonIncrementing and IncrementOnce.
  Iteration is bounded by the already capped input; method arithmetic fits u32.
- A reusable guest handle could misattribute a registration: a shared allocation
  lifetime flag now invalidates on guest object/subtree free. The flag publishes
  under the PtChan mutex at admission before the action queues and is never
  revived/replaced by later objects. Final follow-up review also verified Free
  before action start and concurrent Free during host allocation are covered.
- Bundle-prefix output is bounded by `len().min(64)`, once at arming, with no
  short-bundle panic. No guest registration bytes reach native host calls.

Remaining measurement limits: earlier software-object binding is not captured;
registration can race with an already forwarded batch; only the first authored
5080 is associated; Legacy packet forms are sized-only; post-doorbell guest-byte
sampling races with GPU fetch/guest rewrites. No hit cannot rule out a trigger.
The probe alone cannot prove hardware execution or formally falsify H1.
