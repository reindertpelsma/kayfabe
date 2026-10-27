# Vast check capacity and spending settings

CODE READ from source at rev 9a995ad97 plus this branch's count-ceiling change: `DAILY_RENTALS`
accepts integer1..40. Its default stays4; deployments choose their own value. Raising the
configuration ceiling changes no deployed setting. Daily rental counts apply per profile and
reset at UTC midnight; real and uncertain attempts count, confirmed no-instance failures do not.
CODE READ from Ledger.reserve and budgetSummary in the combined source: a bounded provider
replacement shares its original attempt/day; both legs retain dollar receipts.

Daily count, daily dollar liability, rolling30-day dollars, remaining prepaid credit reserve
and concurrency are independent checks. An extra count slot never overrides a dollar refusal.
Settled rentals release unused reserved duration. The check and sweep daily dollar limits are
separate; rolling dollars and prepaid credit are shared across the account.

INFERRED from source reservation math: at $1.32 estimated per full check, a $10 daily limit
admits7 full-length runs before prior spend, not40. Twenty PR pushes plus twenty merge runs
need40 attempt slots and about $52.80 if every attempt incurs that full estimate.
A provider replacement shares the original deadline and configured dollar allowance; it can
have a different eligible price, but adds no new daily attempt slot.
Actual short settled runs can cost less. A $30 rolling limit also restricts sustained usage;
raising daily count alone cannot promise every busy-day push gets a box. Keep dollar caps
until the owner chooses an explicit budget; inspect BUDGET reason/numeric diagnostics first.

CODE READ from source at rev 9a995ad97: account concurrency2 permits two simultaneous check
rentals only when MAX_CONCURRENT_CHECK is also2 or unset. A third gets BUSY, and the runner
waits for first admission only while its remaining job budget can hold a complete rental
plus admission margin. A proved replacement uses the original remaining deadline instead.
It can fail visibly before either long rental completes. No durable waiting queue is implied.
