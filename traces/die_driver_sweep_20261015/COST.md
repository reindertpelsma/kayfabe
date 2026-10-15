# Campaign cost and instance ownership

**STATUS: LIVE, 2026-10-15.** Ceiling $40; at most one rental at a time;
each rental at most three hours. Only the instance IDs listed here were created by this campaign.
Hourly rates include requested disk as returned by `show instances`.
Accrued cost is an estimate until account billing is available; transfers may add charges.

| Instance | GPU | USD/hour | Start (UTC, API epoch) | Stop (UTC) | Purpose | Cost |
|---|---|---:|---|---|---|---|
| 56076314 | RTX 3070 / GA104 | 0.3083333333 | 1792057483.6353264 | active; 175-minute autodestroy armed immediately | Linux580/595 baseline and regression; Windows guest stall; native-Windows reference reinstall queued11:56UTC | pending; compute ceiling $0.90 |

No other account instances are owned by this campaign.

Snapshot at12:17UTC: the quoted instance price is unchanged and the provider says
running. Compute accrued about$0.78; the175-minute compute ceiling remains$0.90.
API quotes inet_down_cost0.0026041666666666665 and inet_up_cost0.00390625;
final transfer charges are not yet known. No second instance has been created.
Native installation completion is unverified while SSH is unavailable. The
original timer is still alive; it is not reset by the OS conversion.
