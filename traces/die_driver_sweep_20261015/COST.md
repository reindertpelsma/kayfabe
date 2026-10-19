
## Integration stretch continuation — 2026-10-19

STATUS: LIVE. New stretch append; earlier campaign cost history remains on
`codex/sweep-blackwell-20261015` (not imported with old evidence trees). Owner
raised the whole-stretch cap to25USD, stop renting/report at20USD. Phase1
accepted before rental. Starting own spend0USD, own active IDs0, fresh observed
account credit43.559771576861976USD. Credit is separate from spending authority.
One own box at a time, <=180min, auto-destroy armed first, teardown API-verified.
Entries below will record every created ID and actual/conservative cost.

- CREATED56765201 at2026-10-19T17:42:43Z, RTX5060 offer56180125,
  source24a67f4c3. Auto-destroy180min armed/verified before SSH. Requested180GB;
  observed rate0.175USD/h (quote0.13449 at search defaults). Still pulling KVM
  image; no hardware test yet. Cost pending settlement; maximum runtime charge
  at observed rate0.525USD plus transfer. One active own instance.

- DESTROYED56765201; API absence verified2026-10-19T17:49:06Z. Preflight
  HOST_FAULT:5CPUs(<8), missing NVIDIA device nodes, NVIDIA-SMI unavailable.
  No product build or guest test. Actual attribution pending invoice: runtime
  estimate0.018653USD, observed shared-account credit delta0.005332USD. The
  temporary budget guard reserves1USD for delayed transfer billing, not a claim
  of actual spend. Own timer cancelled only after verified absence. ActiveIDs0.

- Invoice observation after teardown:56765201 has two current charge rows,
 0.006+0.005=0.011USD. These are invoiced-so-far, not asserted final settlement.
 Guard reserve remains conservative; actual spend is not the1USDdelay reserve.

- CREATED56766467 (own RTX4090/AD102 candidate), auto-destroy180min armed
  immediately/verified (ownPID60974). Frozen source24906bf33. Observed rate
 0.49USD/h with180GBrequest; offer54367435 reserved32effective CPU cores.
  First box absence verified before creation. SSH/OS startup pending, no hardware
  result yet. Maximum runtime at observed rate1.47USD plus transfer.

- DESTROYED56766467, API absence2026-10-19T20:44:59.768674Z, elapsed2.86571h,
  observed0.49USD/h, runtime estimate1.404197USD. TimerPID60974 cancelled only
  after absence. Exported all cited results first; no executable/image copied back.
  Invoice observation:56765201 now0.017USD;56766467 compute1.232+disk0.142
  =1.374USD. Own invoiced-so-far1.391USD, attribution not final. Conservative
  cost guard includes1USD delayed-transfer reserve per lease; this is not spend.
  Active own IDs0. Owner directs next lease Blackwell B1/B2/B3 (any Blackwell
  permitted); no Ampere lease created. Cap25USD, stop renting/report20USD unchanged.

- CREATED56788212 at2026-10-19T20:48:41.537703Z, RTX5090offer56778084,
  creationheadf9849e500, runtimeproductfrozen24906bf33. Auto-destroy180min
  immediately armed/verifiedPID144987 before SSH. Observed rate0.783333USD/h
  with180GBdisk, maximum180min runtime2.35USDplus transfer. Reserved32effective
  CPUs/63GBRAM; actual health/die not yet verified. PreviousownID absence verified
  first. Owner explicitly directed anyBlackwell when GB206 unavailable. Only this
  own ID active, harddeadline23:48:41UTC, cleanup reserve23:43UTC.
