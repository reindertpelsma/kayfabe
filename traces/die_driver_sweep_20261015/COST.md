# Campaign cost and instance ownership

**STATUS: LIVE, 2026-10-15.** Ceiling $40; at most one rental at a time;
each rental at most three hours. Only the instance IDs listed here were created by this campaign.
Hourly rates include requested disk as returned by `show instances`.
Accrued cost is an estimate until account billing is available; transfers may add charges.

| Instance | GPU | USD/hour | Start (UTC, API epoch) | Stop (UTC) | Purpose | Cost |
|---|---|---:|---|---|---|---|
| 56076314 | RTX 3070 / GA104 | 0.3083333333 | 2026-10-15 09:44:43 (1792057483.6353264) | 2026-10-15 12:40:19; timer destroyed, absence verified | Linux580/595; Windows guest stall; native reinstall did not regain SSH before deadline | compute upper bound $0.91; transfers unbilled |
| 56096336 | RTX 4090 / AD102 | 0.6416666667 | 2026-10-15 12:45:54 (1792068354.8935983) | active; original 175-minute timer PID1954479; deadline about15:40:55UTC | native Windows580.88 oracle and exact die facts | compute ceiling $1.88; transfers unbilled |

No other account instances are owned by this campaign. The first rental was
verified absent before the second was created. Compute upper bounds total $2.79,
well below the $40 campaign budget; final transfer charges are not yet known.

AD102 first SSH at12:59UTC: root146GiB/138GiB available, /dev/kvm present,
19 logical CPUs, 75,324MiB RAM, NVIDIA575.51.03, PCI device2684, VRAM24,564MiB.
The API quoted rate includes150GB disk; the search offer rate excluded it.
