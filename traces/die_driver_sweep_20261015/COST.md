# Campaign cost and instance ownership

**STATUS: LIVE, 2026-10-15.** Ceiling $40; at most one rental at a time;
each rental at most three hours. Only the instance IDs listed here were created by this campaign.
Hourly rates include requested disk as returned by `show instances`.
Accrued cost is an estimate until account billing is available; transfers may add charges.

| Instance | GPU | USD/hour | Start (UTC, API epoch) | Stop (UTC) | Purpose | Cost |
|---|---|---:|---|---|---|---|
| 56076314 | RTX 3070 / GA104 | 0.3083333333 | 2026-10-15 09:44:43 (1792057483.6353264) | 2026-10-15 12:40:19; timer destroyed, absence verified | Linux580/595; Windows guest stall; native reinstall did not regain SSH before deadline | compute upper bound $0.91; transfers unbilled |
| 56096336 | RTX 4090 / AD102 | 0.6416666667 | 2026-10-15 12:45:54 (1792068354.8935983) | destroyed/absence verified by2026-10-15 15:39:44UTC | native Windows580.88 facts/oracle; Windows580 and Linux580 validation PASS; Linux595 raw/CUDA/graphics PASS | compute ceiling $1.88; transfers unbilled |
| 56117789 | RTX2060Super / TU106 (host-derived) | 0.1113888889 | 2026-10-15 15:41:16 (1792078876.69137) | revised timer destroyed18:38:26; absence verified18:39:01UTC, below18:41:16UTC hard cap | nativeWindows580/nativeLinux595 PASS; Windows580 guest PASS afterfix; Linux580 CUDA4 beforefix/broker afterfix PASS, raw29 of30 before/afterfix (R34timeout180s) | compute upper bound$0.34; transfers unbilled |
| 56137859 | RTX5060Ti / Blackwell, exactdie pending RM | 0.2861111111 | 2026-10-15 18:44:29 (1792089869.567632) | destroy requested19:01:35; absence verified19:06:18UTC | provider never createdVM; native/guest hardware NOTRUN | compute upper bound$0.11; transfers unbilled |

| 56140371 | RTX5090 / Blackwell, exact die pending RM | 0.4283333333 | 2026-10-15 19:08:39 (1792091319.0049653) | active; timer2045281 deadline22:03:40UTC, hardcap22:08:39UTC | replacement nativeWindows580 reference, then both guestOS suites | compute ceiling$1.29; transfers unbilled |

AD102 absence was verified at15:39:44UTC before Turing was rented. Turing absence
verified18:39:01UTC, before Blackwell56137859 was created18:44:29. Failed56137859 absence verified19:06:18 before56140371 creation19:08:39. Sole active own rental is56140371. No other account instances are owned by this campaign. The first rental was
verified absent before the second was created. Compute upper bounds for five leases total $4.53,
well below the $40 campaign budget; final transfer charges are not yet known.

AD102 first SSH at12:59UTC: root146GiB/138GiB available, /dev/kvm present,
19 logical CPUs, 75,324MiB RAM, NVIDIA575.51.03, PCI device2684, VRAM24,564MiB.
The API quoted rate includes150GB disk; the search offer rate excluded it.

At13:43UTC the native oracle and exact die facts were pushed; measurement module
stopped cleanly and Windows rebooted into Linux restoration. Original timer stays
armed. No new rental was created, and no AD102 guest result is claimed yet.

Final AD102: device da0569a6; Windows580 Code0/DWM/D3D+CUDA passes, Linux580 raw/CUDA/graphics/broker passes, Linux595 raw/CUDA/graphics passes (broker595 not run). Retirement verified before original timer; compute upper bound remains$1.88.
