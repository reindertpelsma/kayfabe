# Campaign cost and instance ownership

**STATUS: LIVE, 2026-10-17.** Ceiling $40; at most one rental at a time;
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
|56246613|RTX5070 / GB205 (unprivileged RM confirmed)|0.2416666667|2026-10-16T12:40:48.060853+00:00 conservative requesttime|destroyed/absence verified14:42:21UTC; timer2319249 canceled after absence|B3first, newdie nativeWindows reference/B4 while GB206KVM unavailable|2h01m33s; compute ceiling0.49USD; campaign ceiling4.85USD+unbilledtransfers|

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

Final56246613: provider153389 went offline during native Linux restoration after Ubuntu22 was identified and provisioning PID1400 launched. SSH NoRoute/actual_status offline persisted; no kayfabe Windows guest started. All completed evidence was pushed before reimage. Destroy/absence verified14:42:21UTC; owned timer canceled only after absence/ownership check. No own rental remains. Native Windows580 legacy hold/GSP short suite pass, Linux595raw30 pass; GB205Windowsguest UNRUN due provider loss. Exclude machine153389 from the next search.

## Ada passing display reference and missing lanes, 2026-10-16 15:05UTC

| Instance | GPU | USD/hour incl150GB | StartUTC | StopUTC | Purpose | Ceiling |
|---|---|---:|---|---|---|---|
|56261059|RTX4090, exactdie pending unprivilegedRM|0.5083333333|2026-10-16T15:05:43.770452+00:00|destroyed/absence verified15:41:34UTC; timer2383207 canceled after absence|A1Linux595broker/A2Windows580hold and Ada passing display-subtree reference for Blackwell|35m50s; compute ceiling0.31USD; campaign ceiling5.16USD+transfers|

Prior own56246613 absence verified14:42:21 before creation. Single-GPU
unverified offer56242118/machine154064 advertises24CPU/48263MiBRAM/719GBdisk;
actualKVM/disk/die remainUNRUN until bootstrap passes. No rentable5060/5070KVM
consumer offer in narrow and wide/default/no-default searches; do not widen to
P100/RTX6000D. Earlier provider153389 is excluded. Own startup deadline15min;
original timer stays. Native Windows reference already exists per AD102; do not
repeat green native installs. Validate native Linux on this box before guest work.

Final56261059: VM never reached SSH/KVM within the15min startup deadline,
controller exited15:20:51. No hardware test or exact die measurement ran.
Contract destroyed/absence verified15:41:34, timer canceled after absence.

| Instance | GPU | USD/hour | StartUTC | StopUTC | Purpose | Ceiling |
|---|---|---:|---|---|---|---|
|56265118|4xRTX4060Ti advertised; exactdie unmeasured|0.735|2026-10-16T15:44:50.024520+00:00|destroyed/absence verified16:01:36UTC; timer2392895 canceled after absence|A4 startup attempt; no usable SSH or hardware test|16m47s; compute ceiling0.21USD; campaign ceiling5.37USD+transfers|

Verified offer51136946/machine146370,vms_enabled=true, advertises80CPU/
386836MiBRAM/3186GBfree disk;150GBrequested. GPU0/BDF is selected explicitly;
no workload is assigned to the other three GPUs. New bootstrap fixes shell
precondition/launch fail-fast before launch and caps startup at15min. No native
or guest fact may be claimed until its own command completes. No data/values
are borrowed across Ada dies. Prior56261059 absence precedes this creation.

Final56265118: the startup controller exited1 at16:00:02UTC after its15minute
deadline. No usable SSH, actual die identification, native suite or guest suite
was obtained. Retirement and filtered startup text are saved under
provider_startup_failures_20261016/. API absence was checked before cancelling
the exact owned timer. Machine146370 is excluded from subsequent searches.

## Verified AD102 missing lanes and passing display reference, 2026-10-16 16:03UTC

| Instance | GPU | USD/hour incl150GB | StartUTC | StopUTC | Purpose | Ceiling |
|---|---|---:|---|---|---|---|
|56267214|RTX4090 / AD102, unprivileged RM verified|0.61|2026-10-16T16:03:21.897190+00:00|destroyed/API absence verified18:43:36UTC; own timer2396076 cancelled after absence|A1Linux595broker/A2Windows580hold/passing Ada display trace for B1|2h40m15s; compute ceiling1.63USD; campaign ceiling7.00USD+transfers|

MEASURED: verified offer48798435/machine43132, single GPU. Actual146GiB root,
KVM,30logical CPUs,24389MiB RAM. Cloud-init regenerated its initial SSH host key;
the final key was verified against this owned instance's provider console before
pinning. Fresh UID65534/CapEff0 RM probe reports arch0x190/impl2 AD102 and stock
595.91.07. Native gates9/9 plus gate10 PASS,12/12USER births at16:10:50UTC.
Private binary2feb22a9 built16:11:25UTC. No guest pass is claimed yet.

Final56267214: Linux595 native gates9+10/CUDA4, guest raw30/30 and broker pass;
Windows580 production421second hold and short Blender CUDA/Edge hardware WebGL
apps with survivor CUDA/D3D pass. Passing Ada connected-display trace exported
as public metadata/display bodies before retirement. Linux raw retains1418
startup heartbeat0warnings; broker retains9STALL/9nvAssert. Only screened text
returned. Destroyed/API absence verified18:43:36UTC, exact timer cancelled after
absence. Conservative duration9615seconds at0.61USD/h, estimate1.629214USD,
rounded-up ceiling1.63USD; retired campaign ceiling7.00USD+unbilled transfers.
Windows581.51 public ABI/package prerequisites pass, hardware remainsUNRUN.

## GB206 display-phase follow-up, 2026-10-16 18:49UTC

|Instance|GPU|USD/hour incl150GB|StartUTC|StopUTC|Purpose|Ceiling|
|---|---|---:|---|---|---|---|
|56284522|RTX5060advertised, exactdie pending unprivilegedRM|0.1680556quote|2026-10-16T18:49:33.182026+00:00|destroyed/API absence verified19:07:54UTC; timer2463545 cancelled after absence|B3GB206clock revalidation, B1displayon/off, B2one virtual-tiling group|18m21.7s; compute ceiling0.06USD; campaign ceiling7.06USD+transfers|

Prior own56267214 absence verified18:43:36UTC before creation. Verified
machine46454 advertises6effectiveCPU/64244MiBRAM/KVM/523GBfree; actual disk,
KVM and die pending. Startup deadline15min; no green hardware claim yet.

Final56284522: PCI/sysfs NVIDIA device count0, installer No NVIDIA GPU found;
no native/guest hardware work. Partial Windows image transfer cancelled, no
full hash PASS. Text saved under blackwell_5060/display_phase_20261016/. Rental
destroyed/absence19:07:54UTC before timer/hard cap. Machine46454 excluded.
Compute estimate0.051431USD/ceiling0.06USD; no own rental remains.

## GB206 second display-phase attempt, 2026-10-16

|Instance|GPU|USD/hour incl150GB quote|StartUTC|StopUTC|Purpose|Ceiling|
|---|---|---:|---|---|---|---|
|56286918|RTX5060Tiadvertised, exactdie pendingRM|0.7222222|2026-10-16T19:11:25.724680+00:00|ACTIVE; timer2478432 at22:06:25, hard22:11:25|B1/B2/B3; earlyPCIcheck, baseline then one derived tiling group|max2.17USD; campaign ceiling9.23USD+transfers|

Previous56284522 API absence verified19:07:54UTC before this creation.
Verified offer56266289/machine152012 advertises64effectiveCPU/257596MiBRAM/
6705GBfree/KVM. These remain claims until the bootstrap checks them. Reject
missing NVIDIA PCI before driver installation and image transfer. No native
Windows green repeat; per-die reference already exists if GB206 is measured.


## Final GB206 follow-up retirement, 2026-10-16 21:32UTC

Only own56286918 destroyed after all GPU work stopped; API absence verified21:32:48UTC,
original timer2478432 canceled only after checking its owned destroy command and absence.
Duration ceiling8483seconds, rate0.7222222USD/h including150GB disk, compute ceiling1.71USD.
Previous retired ceiling7.06USD; campaign ceiling8.77USD plus unbilled transfers.
No own rental active at this checkpoint. Full evidence pushed before retirement: B3raw30,
B2one-group mode progress but presentation FAIL, Windows on/off and CMC Code43 negatives,
exacte1bd candidate raw30, native/guest595CUDA4 and graphics5/5+10matchinghashes.
Retirement receipt: blackwell_5060/display_phase_20261016/retirement.json.


## Ada second actual driver builds, 2026-10-16 21:51UTC

|Instance|GPU|USD/hour incl150GB|StartUTC|StopUTC|Purpose|Ceiling|
|---|---|---:|---|---|---|---|
|56302930|RTX4090 actualAD102/PCI2684/KVM/root146GiB|0.61|2026-10-16T21:51:00.774332UTC|RETIRED/APIabsent2026-10-17T00:40:19UTC; timer2571615 canceledafterabsence|A3Windows581.51/Linux590; A4AD102three-row ablations|10159sec/computeceil1.73USD; retiredcampaign10.50USD+transfers|

Prior own56286918retired/APIabsence21:32:48 beforecreation. PublicSSHkeyattached,
timer2571615armed immediately. NoBlackwell5070contract created: preparedofferdisappeared.
Own onlythisnewID; allotheraccountinstancesareuntouched. Startup15minKVM/disk/actualGPUchecks.

Final56302930: exact9d AD102595/590raw30, threeWindows580one-rowablations,
590CUDA4/gfx5+10nativehashes/CinnamonSHM PASS, Windows581guestCode43,
sameboardnative581GSP/420.59s0TDR PASS andnativeB29712byteallocationmetadata.
AllGPUjobs/observerstopped andtextPUSHED beforeAPIabsence00:40:19UTC2026-10-17.
OnlythisIDdestroyed; exacttimer2571615 checked/canceledAFTERabsence00:43:02UTC.
Computeconservative10159s at0.61/h incldisk ceil1.73USD; retiredtotal10.50USD+transfers.
No ownrentalactive. No thirdGB206B1lease; anynextlease onlyafterthisverifiedabsence,
newreceipt/timer andliveprice/PCI/KVM/diskchecks.

## GA104 Linux595 completion, 2026-10-17

| Instance | GPU | USD/hour incl150GB | StartUTC | StopUTC | Purpose | Ceiling |
|---|---|---:|---|---|---|---|
|56326220|RTX3070/GA104arch170impl4 unprivilegedRMverified|0.1555555556|2026-10-17T01:30:28.677605+00:00|RETIRED/APIabsent2026-10-17T04:02:45UTC; timer2714225+sleep2714228 canceledafterabsence|M1 nativecaps/M3 Linux595CUDA,gfx,broker; exact9d6f210ba|9137s/computeceil0.40USD; retiredcampaign10.90USD+transfers|

All prior campaign-owned IDs were checked absent before creation; other account
instances were not acted on. Immediately armed175minute timer and attached only
the publicSSHkey. Startup/KVM/rootcapacity passed; native unprivileged MC_GET_ARCH_INFO proves GA104.
Exact9d Linux595 native+guest raw/CUDA/gfx/broker functional acceptance PASS (warnings retained).
FreshstockLTSC baseline active; Windows adapter/binding stillUNVERIFIED. No shared-host use.

## Final GA104 retirement, 2026-10-17 04:02UTC

Only56326220 destroyed/APIabsent04:02:45UTC afterallGPU/VMjobs stopped/NBD8detached and
fulltextPUSH98dc717ec. Own timer2714225+verifiedsleepchild2714228 canceledAFTERabsence,
nonrunningrechecked. Durationceil9137s/rate.1555555556USDh/computeceil.40USD; campaign10.90
plusunbilledtransfers. Beforemanual04:15/original04:25/absolute05:00UTC deadlines.
No ownrentalactive/no retiredaliasSSH. Linux595fullfunctionalPASS/warningsretained; Windows
stockbaselinePASS, actualNVIDIA8088bindingUNVERIFIED/BasicDisplay; apps/holdUNRUN.
No furtherboots/installerreplay. Nextnative/VFIOreference+verifiedINF/GSP-first-start setup.
