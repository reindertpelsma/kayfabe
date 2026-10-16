# Campaign cost and instance ownership

**STATUS: LIVE, 2026-10-16.** Ceiling $40; at most one rental at a time;
each rental at most three hours. Only the instance IDs listed here were created by this campaign.
Hourly rates include requested disk as returned by `show instances`.
Accrued cost is an estimate until account billing is available; transfers may add charges.

| Instance | GPU | USD/hour | Start (UTC, API epoch) | Stop (UTC) | Purpose | Cost |
|---|---|---:|---|---|---|---|
| 56076314 | RTX 3070 / GA104 | 0.3083333333 | 2026-10-15 09:44:43 (1792057483.6353264) | 2026-10-15 12:40:19; timer destroyed, absence verified | Linux580/595; Windows guest stall; native reinstall did not regain SSH before deadline | compute upper bound $0.91; transfers unbilled |
| 56096336 | RTX 4090 / AD102 | 0.6416666667 | 2026-10-15 12:45:54 (1792068354.8935983) | destroyed/absence verified by2026-10-15 15:39:44UTC | native Windows580.88 facts/oracle; Windows580 and Linux580 validation PASS; Linux595 raw/CUDA/graphics PASS | compute ceiling $1.88; transfers unbilled |
| 56117789 | RTX2060Super / TU106 (host-derived) | 0.1113888889 | 2026-10-15 15:41:16 (1792078876.69137) | revised timer destroyed18:38:26; absence verified18:39:01UTC, below18:41:16UTC hard cap | nativeWindows580/nativeLinux595 PASS; Windows580 guest PASS afterfix; Linux580 CUDA4 beforefix/broker afterfix PASS, raw29 of30 before/afterfix (R34timeout180s) | compute upper bound$0.34; transfers unbilled |
| 56137859 | RTX5060Ti / Blackwell, exactdie pending RM | 0.2861111111 | 2026-10-15 18:44:29 (1792089869.567632) | destroy requested19:01:35; absence verified19:06:18UTC | provider never createdVM; native/guest hardware NOTRUN | compute upper bound$0.11; transfers unbilled |

| 56140371 | RTX5090 / Blackwell, exact die pending RM | 0.4283333333 | 2026-10-15 19:08:39 (1792091319.0049653) | destroy19:17:58; absence verified19:19UTC | provider never startedVM; native/guest hardware NOTRUN | compute upper bound$0.08; transfers unbilled |

| 56141654 | RTX5060 / GB206 (unprivileged RM confirmed) | 0.1288888889 | 2026-10-15 19:20:04 (1792092004.016716) | destroyed22:05:44UTC; absence verified immediately; timer canceled after absence | nativeWindows580 PASS; Linux580 raw/CUDA/headlessgfx PASS; desktopFAIL; guestWindows580 Code43; Linux595raw29/30 | compute ceiling$0.39; transfers unbilled |

AD102 absence was verified at15:39:44UTC before Turing was rented. Turing absence
verified18:39:01UTC, before Blackwell56137859 was created18:44:29. Failed56137859 absence verified19:06:18 before56140371 creation19:08:39. 56140371 absence verified19:18:29 before56141654 creation19:20:04. Historical state: account instance list empty22:05:44UTC; superseded by new56165326 below. No other account instances are owned by this campaign. The first rental was
verified absent before the second was created. Compute upper bounds for six leases total $3.71,
well below the $40 campaign budget; final transfer charges are not yet known.

AD102 first SSH at12:59UTC: root146GiB/138GiB available, /dev/kvm present,
19 logical CPUs, 75,324MiB RAM, NVIDIA575.51.03, PCI device2684, VRAM24,564MiB.
The API quoted rate includes150GB disk; the search offer rate excluded it.

At13:43UTC the native oracle and exact die facts were pushed; measurement module
stopped cleanly and Windows rebooted into Linux restoration. Original timer stays
armed. No new rental was created, and no AD102 guest result is claimed yet.

Final AD102: device da0569a6; Windows580 Code0/DWM/D3D+CUDA passes, Linux580 raw/CUDA/graphics/broker passes, Linux595 raw/CUDA/graphics passes (broker595 not run). Retirement verified before original timer; compute upper bound remains$1.88.

## Native trace replay wave, 2026-10-16 local date

| Instance | GPU | USD/hour | StartUTC | StopUTC | Purpose | Cost |
|---|---|---:|---|---|---|---|
| 56165326 | RTX5060 / GB206, fresh unprivileged RM verified | 0.1288888889 | 2026-10-15T22:44:19.841586+00:00 | destroyed; absence/account IDs[] verified2026-10-16T01:13:59UTC; own timer canceled after absence | native complete-prefix H-BC4 PASS; host CCU negative cross-ABI query; virtual FMC prefixes recovered, strict oracle FAIL |2h29m40s; compute ceiling0.33USD; campaign conservative ceiling4.04USD, transfers unbilled |

## W3 Turing hold and R34 replay, 2026-10-16

| Instance | GPU | USD/hour | StartUTC | StopUTC | Purpose | Cost |
|---|---|---:|---|---|---|---|
|56180232|RTX2060S / TU106, fresh unprivileged RM verified|0.1113888889|2026-10-16T01:16:45.814656+00:00|destroyed; absence/account IDs[] verified04:08:27UTC; own timer2167738 canceled after absence|Native-first Windows580 reference, nested7min hold, callback/event R34 at180s|2h51m42s; compute ceiling0.32USD; campaign conservative ceiling4.36USD, transfers unbilled|

Prior own56165326 absence verified01:13:59UTC before this creation. No other
own rental remains. Timer armed immediately. Updated03:47UTC: native Windows
580.88 seven-minute reference and Linux595 raw30/30/native gates pass; nested
Windows580.88 short suite passes, corrected-deadline hold running. R34 A/B
pending. UID65534/CapEff0 RM tuple arch0x160/impl6 verifies TU106.

Final W3 retirement04:08:27UTC verified before original04:11:46 timer and
04:16:45 hard cap. No own rental remains (account IDs[] at this checkpoint).
Windows native and both guest seven-minute holds pass; R34 callback/event
both fail180s. No lease extension or full-family acceptance. Only own timer
canceled after absence; owner GPU/VM untouched. Text retirement receipt in
turing_hold_20261016/retirement.txt. Screenshots/executables were not returned.

## Additional Blackwell die, 2026-10-16 12:40 UTC

| Instance | GPU | USD/hour | StartUTC | StopUTC | Purpose | Ceiling |
|---|---|---:|---|---|---|---|
|56246613|RTX5070 / GB205 (unprivileged RM confirmed)|0.2416666667|2026-10-16T12:40:48.060853+00:00 conservative requesttime|ACTIVE, timer2319249 at15:35:50UTC; hardcap15:40:48UTC|B3first, newdie nativeWindows reference/B4 while GB206KVM unavailable|max3h0.73USD; campaign ceiling5.09USD+unbilledtransfers|

MEASURED APIaccount IDs[] before creation; only own new56246613 may be retired.
Immediately armed175minute timer. Offer54561458/machine153389 advertises16CPU,
32109MiBRAM/600.5GBfree disk;150GBrequested. These are offer claims, not measured
hardware. Exact die/PCI/INFsupport/native suite remainUNRUN. No GB206constant
or pass carried across. GB206Code43 remains red. Step0alignment is complete;
newcode-onlyPR21 and22 staydraft, no owner/master merge. Shared ownerGPU/VM
unused; trustedCPU54GSPtests at most2jobs, rentalbinary built onrental because
trustedhostUbuntu26.04/glibc2.43 cannot promise Ubuntu22.04runtime compatibility.

Update13:30UTC: measured GB205 arch0x1b0/impl5, UID65534/CapEff0. Stock595
native gates9+10/concurrency pass. B3 normal180s timeout versus quiet18s pass;
first PTIMER-selector candidate failed180s too. Corrected fn72-offset candidate
304ca9adb running. ActualPCI2f04 matches public580.88INF. Windows suites UNRUN.
