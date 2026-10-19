# Sweep budget authority

STATUS: LIVE, 2026-10-18. Owner budget update; Phase 1 STOP still applies.

OWNER UPDATE, 2026-10-18: the owner added USD 25 to the shared Vast account and
reported USD 43.56 credit with no running instances. The cap for this entire
stretch is now **USD 25**; **stop renting and report at USD 20 spent**. These
replace the USD 12 cap and USD 9 rental-stop threshold in the 2026-10-18
campaign steering. This is an increase to the existing stretch budget, not a
reset of its spending ledger. This lane has spent USD 0 in the stretch.

The fresh, filtered API observation is in `budget_update_receipt.json`; the
older USD 18.56 receipt remains historical evidence of its observation time.
Available account credit is separate from this lane's spending authorization.

Everything else in the steering remains binding: Phase 1 integration before
any rental; one owned instance at a time; at most three hours per instance;
arm auto-destroy before touching the instance; act only on IDs created by this
lane; verify teardown; no wall loops; no owner-host access. Future lease costs
must be appended to the campaign's existing owned cost ledger.

This update does not resolve the protected conflicts in `CONFLICTS.md`.
No conflict resolution, product change, hardware run or rental is authorized
by this budget amendment alone. The coordinator subsequently approved excluding the obsolete USER topic; the
F-2 two-file edit is also approved and verified. The distinct userrelay family
admission topic remains pending under the protected-scope rule in CONFLICTS.md.
