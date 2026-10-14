**STATUS: RESEARCH, 2026-10-12 04:50UTC.**

VA-worker cancellation prerequisite for the approved Windows immutable USER relay.
No VM birth caller or software migration is connected; Gravity remains unresolved.
Source is the commit containing this evidence, based on pushed6c255beb2.

Reply states Pending/Ready/Taken/Canceled serialize handout versus cancellation.
Cancellation before handout does metadata work only. A popped canceled request
returns its pending mirror pin without any host operation. Prepared/ready storage
is released by the VA worker only because no caller ever received it and no
native channel could exist. Once take wins, cancellation refuses: the caller
must confirm native free or never-born state before explicit lease release.
Canceled replies cannot be overwritten by a late successful mapping result.

Automatic error/cancellation cleanup gets one attempt. Failed cleanup retains
storage, mirror pin and VA exclusion; no timer or retry loop grants reuse.
Retained constructor failures are already marked as cleanup-attempted.
Canceled request churn still yields an initialization turn to older work.

MEASURED: failing-first tests did not compile before the API existed; after the
implementation full cargo test -p kf-abi -p kf-rm -p kf-qemu -p kf-chan PASS.
The12 userva tests include cancellation before prepare, prepared/ready handout
exclusion, failed cleanup retention, cancellation during clear/map errors,
during cleanup and after failed cleanup, and canceled request scheduling fairness.
Independent source review found both an error-path second cleanup attempt and
an initialization-starvation path, corrected before final review. Final review
found no remaining source blocker. Filtered suite output retained here.

No hardware run or Linux exact-revision claim for this change. The prior exact
cec428397 Linux9/9+gate10+fast30/30 PASS remains in user-lifecycle/linux-result.
Linux Passthrough still has no caller for private Windows storage or mappings.
Next: bind the reply to an exact pending channel/subtree identity, then source
and USERD-object ownership adapters and actual ordered Windows migration.
Vast API rechecked04:42UTC:[]; no instance to retire. Personal image untouched.
