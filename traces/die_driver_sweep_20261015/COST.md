
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

- DESTROYED56788212; API absence verified2026-10-19T23:41:48.173963Z,
  elapsed2.885458h (<3h), observed0.783333USD/h, runtime estimate2.260276USD.
  Auto-destroyPID144987 cancelled only after absence. All cited Linux/Windows
  evidence was exported and pushed first. Final host health was good; no QEMU
  or forensic NBD remained before destruction. All three own IDs API-absent.
  No next lease has been created at this closeout.

- Closeout invoices, observed2026-10-19:56765201=0.017USD,
  56766467=1.391USD,56788212=2.190USD; total **3.598USD invoiced so far**.
  This is not asserted final settlement. Combined runtime estimate3.683126USD;
  conservative budget guard6.683126USD includes three1USD delayed-billing
  reserves and is not actual spend. Last shared-account credit observation
  39.961149USD is not exclusive attribution. Cap25USD, stop renting/report20USD.
  Sanitized own-id-only receipt:
  `../sweep_integrate_20261018/phase2_cost_closeout_20261020.json`.

- CREATED56808180 at2026-10-19T23:48:46.516496Z, RTX3070offer56311549,
  frozen sourcee8553e6ce7ec557aef9ff96c3c876e3060a6b9f8 (accepted product trees).
  Previous own IDs API-absent first. Auto-destroy180min armed/verifiedPID224025
  before SSH. Observed rate0.16USD/h with180GBdisk, maximum runtime0.48USD plus
  transfer. Harddeadline2026-10-20T02:48:46Z, cleanup reserve02:40UTC. Resource
  and driver-health preflight pending; no hardware result is implied.

2026-10-20 01:05UTC owner Windows-night amendment: API credit39.7773407655USD. Cap30USD total sweep, stop/report25USD; reserve5USD; at most Ada+Blackwell Windows two boxes, <=180min each, ALL teardown by2026-10-20 04:30UTC (06:30CEST). Credit>=35 qualifies conservatively. Own settled invoices so far3.782USD including partial GA1040.184USD; not final charges. Shared credit is not own spend attribution.

MEASURED GA10456808180 API-absent2026-10-20T01:05:56.954502Z; observed rate0.16USD/h, elapsed1.286671h, runtime estimate0.205867USD, conservative guard1.205867USD including1USD reserve for delayed billing. New Ada56816419 quote0.561667USD/h, Blackwell56816449 quote1.001389USD/h; timers294763/295834180min armed before SSH, expires before04:30UTC. Shared credit39.77734USD pre-pair; final invoices pending.

2026-10-20T01:41:11UTC APIcredit38.9078384275USD, reserve5USD intact; no budget threshold reached. AD102current580PASS426.09shold; GB202twoCode43arms complete. Conditional native/VFIOreference on same owned56816449 next, within unchanged180mintimer and04:30UTCglobaldeadline; no third product initialization.

2026-10-20T02:48:20UTC API credit37.0302557975USD;5USDreserve intact. Two original own timers stay armed; no new rental. Shared credit delta is not exclusively attributed sweep spend.

FINAL paired-night teardown: Blackwell56816449 API-absent02:56:39.710069UTC; Ada56816419 API-absent02:56:45.488082UTC. Observed rates1.05/0.62USD/h, runtime estimates1.917916/1.138010USD, each<3hours; all six sweep-owned ids absent. Timers cancelled only after absence, well before06:30CESTglobaldeadline. Own invoices so far6.773000USD; runtime total6.944920USD; conservative runtime+six1USDreserves12.944920USD is a guard, NOT spend. Shared credit36.768756USD;5USDreserve intact; cap30/stop25 not reached. No new lease. Invoices may lag; per-ID details in phase2_cost_final_20261020.json. Shared credit deltas for the paired boxes overlap and MUST NOT be added as actual spend.

CREATED Ada broker follow-up 56829120 at2026-10-20T03:09:26.440012+00:00, approved newlease after missing-reference STOP. Auto-destroy70min PID644909 armed/verified first; observed actualrate0.62USD/h, quote0.561667, source7f22d18df, previous6IDs absent. Credit36.683573USD reserve5 intact; cap30/stop25. Own directSSH/KVM verified175GBroot/30CPU/~49GBRAM/RTX4090, stock575; standard595 provisioning active. Allteardown04:30UTC, timer earlier.
