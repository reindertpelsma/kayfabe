**STATUS: RESEARCH, 2026-10-12 05:01UTC.**

Exact pending allocation/VA reply adapter for the Windows USER relay. Source is
this commit, based on854533ec1. No runtime allocate/poll/commit caller yet; the
production channel Free hook now invalidates pending scopes before live removal.

Book admits only a birth carrying the existing WindowsUserRingPermit. The caller
must already count its USER twin in statement order and check live collision.
Weak pending rows are bounded512, not a die/driver fact. The act retains the
exact Birth identity; poll and commit reject an old allocation even when every
numeric guest/native handle reuses. Scope covers client/channel/parent/TSG/device.
Commit additionally requires successful lease handout and performs only bounded
attachment metadata under the Book lock. Native birth/free stays outside it.
Free may win during native birth: failed commit then requires confirmed native
cleanup before release of that act's exact lease and USER count.

A pre-handout dropped/orphaned Birth cancels its reply. Production Book signals
the VA eventfd on allocation, cancellation and orphan Drop. It captures the Inbox
only for lifetime-safe notifier access; Queue owns Reply/Lease, not Birth, and
Book owns Weak Birth rows, so there is no strong ownership cycle. After handout,
Drop/cancellation cannot automatically free storage. The notifier is nonblocking;
a last temporary Arc can Drop/signal inside cancel's Book lock, with no re-entry.

MEASURED: failing-first tests lacked the Book module, then full GPU-free cargo
test -p kf-abi -p kf-rm -p kf-qemu -p kf-chan PASS; userva16/16. Tests use the
real Queue/adopt/reply path for subtree Free/reuse, stale poll/commit, pre-handout
commit refusal, Free after handout, once-only commit, orphan cancel/wake, and
no-permit/no-allocation. Independent source review found no blocker. Full adapter
still must bind the native channel to its exact lease; this evidence does not
claim a Windows relay, migration, or Gravity PASS. No hardware run for this change.
Linux Passthrough receives no private mapping. Prior exactcec428397 Linux9+10/30
PASS is separate. Gravity Kayfabe FAIL/TIMEOUT; same-GPU VFIO PASS179/183s.

Next: shared validated USERD-object lifetime access, bounded immutable source/VA
adapter, and stream-position migration; exact Linux regression then Gravity.
Public generated driver identity already associates Windows580.88/r580_78-7
with audited ABI580.65.06; use that exact selector, not a guessed closest build.
Vast04:42UTC empty; personal image untouched. GitHub origin all branches fetched;
stale bx SSH remote was unreachable, no refs or checkout edited to repair it.
