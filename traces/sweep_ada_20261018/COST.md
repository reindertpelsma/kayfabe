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
