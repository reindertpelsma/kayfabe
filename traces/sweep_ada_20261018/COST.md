STATUS UPDATE: CLOSED PARTIAL, 2026-10-22 02:07 UTC; supersedes previous
closeout for the newly authorized rerun. MEASURED: controller fix f98ceae65 is
pushed; new create calls0, new lease ids[], new spend0USD. Prior spend0.21USD
(owner accounting) remains inside the4USD lane cap; no new timer was needed.
MEASURED: complete verified/rentable/single-GPU KVM search returned15 offers,
one Ada candidate (RTX4090,0.8766666667USD/h), exceeding0.60USD/h. A separate
preceding Ada query returned two4090offers at0.8766666667and0.95USD/h, also
ineligible. No verified AD106/AD104/AD103 offer was returned. No deploy failed;
no deploy retry occurred. The price guard refused provisioning; common STOP
honored. Current shared credit42.743520USD is not exclusive lane attribution.
Evidence: image_path/availability.json. Prior owned57138158 remains API-absent.

STATUS UPDATE: CLOSED PARTIAL, 2026-10-22. One approved AD102 lease57138158.
MEASURED: created2026-10-22T01:15:51.043786+00:00; API-absent2026-10-22T01:41:10.658354+00:00; rate0.49USD/h,
170-minute destroy timer armed FIRST and cancelled only after absence. Runtime
estimate0.206836USD; own invoiced-so-far0.193USD, not final
settlement. The1USD delayed-billing reserve is a guard, not spend; runtime+guard
1.206837USD stays below4USD cap. Shared credit43.239299->42.811353USD is not
exclusive attribution.5USD reserve intact. No second lease. See lease receipt,
invoices_observed.json and final_cleanup_receipt.json under lease_57138158/.
CODE READ: nativePASS; first guest armNOTRUN due omitted controller guest-image
path; common STOP honored, WindowsUNRUN. Older cost statuses below are historical.

STATUS UPDATE: LIVE, 2026-10-22. Owner resumed S2; exact guest guard is implemented,
verified and pushed. No eligible listed die is available, and an AD102 substitution
question is pending. Created IDs [], rental spend 0 USD, no destroy timer started.
Fresh observed shared credit 43.531724 USD, distinct from the briefing value.
The 4 USD lane cap and 5 USD reserve remain binding. Historical initial stop below.

STATUS: LIVE, 2026-10-22. Lane S2; stopped before rental.

CODE READ: this lane's hard cap is 4 USD, with 5 USD account reserve untouched;
its briefing reports shared account credit 43.85 USD. This is a new S2 budget,
not an extension of historical SW2 spending.

MEASURED: created instance IDs []; lease-create calls 0; rental spend 0 USD;
auto-destroy processes 0. No paid resource was created or used by S2.
MEASURED: no instance needed teardown or API-absence verification, because this
lane never created an instance. No other lane's instance was accessed or acted on.
CODE READ: a future rental still requires a fresh credit/offer check, a 170-minute
auto-destroy timer armed first, own-ID-only teardown with API absence, the lane's
one-lease rule and its complete guard proof. No future spend is committed here.
