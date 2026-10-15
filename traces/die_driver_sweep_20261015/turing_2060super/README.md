# Turing RTX2060Super wave

**STATUS: LIVE, 2026-10-15 15:43UTC.** Own instance56117789, created only after
AD10256096336 was verified absent. Offer55223684, Quebec, KVM image,150GB
requested. Actual quote$0.1113888889/hour.175min teardown timerPID1995396,
deadlineabout18:36:22UTC; never extend beyond18:41:16UTC three-hour cap.
Exact die, running kernel, available disk and KVM still need SSH verification.
Own branchcodex/sweep-turing-20261015, parent455d13980 (AD102 candidate).

Order: native Windows580.88 LTSC2024 reference/facts first, restore Linux on the
same rental, then Linux and Windows kayfabe guest suites. This order was reported
to the owner; it avoids shipping unmeasured per-die Windows constants. Text only
returns, no agent forwarding/secrets; binaries stage outbound from trusted builds.

H-T1 BEFORE native installation/run: stock Windows580.88 supports this consumer
Turing board and passes Code0/WDDM, CUDA2/3/8 and D3D11/12 with no TDR during
the suite. Falsifier: nonzero adapter code, failed content/fence oracle or Event4101.
Installation/SSH timeouts are provisioning results, not hardware limits.

H-T2 BEFORE capture: the bounded native module can read the exact display caps
scalar, and GSP capture can validate only demanded small controls2801/2806 at
580.88's public same-CL tables. Falsifier: failed/unstable BAR read, module refusal,
loss/gaps/torn capture, unanswered requests or unsupported public decode. No
captured board table, no852f row; only measured exact-question T3 residue may ship.

H-T3 BEFORE guest runs: generated all-family vocabulary plus the public C57E
Shared32 semaphore path admits Turing display without borrowing modern semaphore
semantics. Falsifier: named display refusal, incorrect acquisition/release, adapter
failure, render mismatch, TDR, or Linux raw/broker regression. First failure is
compared with this board's native oracle before another wall-clearing run.

No Turing hardware pass or complete family acceptance is claimed. Both OS suites
across several supported driver builds remain required; this wave starts at580.


First SSH15:44:57UTC: root146GiB/138GiB available,KVM present,19CPUs,
50,541MiB RAM, GPU8192MiB, PCI device1f06, initial stock575.51.03,
kernel6.8.0-59. Native Windows install queued via pinned
windows-on-vast93b65b3347601f35f4146602ff85950e389b409f and
reinstall80c3d5e175f39c2d2bbd267cd583842140154140. Expected final Windows
SSH public host key was saved before the intentional Linux/installer reboots.
Native oracle now explicitly rejects nonzero adapter code, hash mismatches and
unreadable System logs; zero events is not inferred from a failed log query.

Windows runner now honors WIN_GUEST_DRIVER for a measured public same-CL tag,
records it and refuses malformed/injected properties before launch. Default
remains580.65.06 for Windows580.88. Before/after actual argv is tested with
stubbed QEMU;26harness tests pass. This adds no new driver compatibility claim.


## Native580 reference,16:23UTC

Stock580.88 actual32.0.15.8088: Code0/WDDM, CUDA3/3 (sm75 targets),
hardware D3D11compute/draw andD3D12compute/fences20iterations pass;
Systemlog readable,0Event4101 during16:22:30–16:22:44UTC suite. Actual driver
and adapter health checked before/after;4probe hashes verified before execution.
Default GSPN/A. This is a native reference, NOT a kayfabe guest pass.
Installer complete=true/driver=true/cuda=true/failures0, measured before reboot.

Windows Update raced the pinned installer: System events43/19 show591.86
installation16:07:18–16:09:18; nativepreflight reported32.0.15.9186 while
580installer was active. It later finishes with32.0.15.8088/Code0.
ExcludeWUDriversInQualityUpdate=1 andDriverSearching SearchOrderConfig=0
were set. The OS restarts wuauserv asManual/Running despite the stop/disable
attempt; do not claim a permanently disabled service. Its now-unused oem13.inf
was verified NVIDIA/Display/32.0.15.9186 and removed without uninstall/force,
only after current580Code0 was checked. The exact oracle rejects any build
change before/after. Measured: WU caused the temporary591 install; any claim
that it explains all installer delay would be inference.

H-PIN before reboot: exact580 remains active after removing unused higher-ranked
store package and excluding WU drivers. Falsifier: a different actual build after
reboot or during either native suite. Testsigning was enabled afterSecureBootoff,
VBS/HVCI0/0 preflight; reboot requested normally at16:24 for module measurement.
Native GSP-enabled reference, caps and captures are still pending. No Turing
constant row has shipped and no closed binary reverse engineering was done.

Exact590/595 display-table topic is merged in this branch; CPU source8149f0b53
full3243passed/0failed/14ignored, fresh target-driver590.595removed swap-ready
control is deliberately omitted, not assigned an older ID. No590 or595display-on
hardware validation is inferred from CPU tests or the earlier headless graphics.


## GSP reference and selected facts,16:33UTC

After the normal measurement reboot: testsigningON,SecureBootoff,VBS/HVCI0/0,
actualdriver32.0.15.8088 remainsCode0. Bounded module loads; physical caps
word0x00c=0x100 before and after GSP enable. NativeGSP firmware580.65.05,
public decode tag580.65.06. GSP-enabled native CUDA3/3 and hardwareD3D11/12
pass again, exactdriver/Code0 checked before/after, readableSystemlog and0TDR
in the short16:28:19–16:28:33 suite. No zero-TDR-whole-boot claim.

Capture16:28:08–16:30:08:9210messages,4581command/4629status,208controls;
9227records,bind2/elements9224,loss0/0/0,ENDyes/filecheckOK. Decoderoracle
PASS, no gaps/parsefailures/trailingbytes/unknownfunction; zero unanswered
controls/orphanreplies. PowerShell collector ExitCode printed blank; do not
infer its process rc0 from that field. END/filecheck/decode are independent.

Only two control IDs were body-allowlisted. Four measured4byte bodies:
2801 request3 ->2,status0;2806 request3 ->0,status0. Whole caps page is evidence
only; only its decisive scalar may ship. All three aggregate TSVs are saved as
text, unlike the earlier AD102 run. Raw and scrubbed capture binaries stay on
the rental and will be erased by Linux return. No852f or broad-screened export.
Exact die tuple must still be derived from restored Linux host controls before
registering rows; no unmeasured tuple is presented as native evidence.

Linux return preparation started16:31:49 via the pinned public reinstall payload,
explicitusername/root+SSHport22+publickey and noninteractive stdin. Original
175min rental timer still18:36:22UTC. No Turing kayfabe guest pass yet.

Linuxreturn prep completesReady/LastTaskResult0 by16:38:24UTC. Measurement
service stopped cleanly before erase. Native caps has90nonzero words before
and afterGSP (distinct from AD102's99); only word0x00c100 is eligible to ship.
All native facts/capture aggregates are pushed before the normal Linuxreturn
reboot. Currentdriver580 still verified; next derive hostchiptuple and test/fix
bothguest OSes. Sourcecandidate now includes exact590/595 tables; all Turing
native results remain measured580.88 only.


## Exact tuple and candidate CPU verification,17:07UTC

RestoredUbuntu22.04.5/kernel6.8.0-138,stock open595.91.07. Host RM probe
uid65534/CapEff0/cap_sys_admin=false reportsMC_GET_ARCH_INFO arch0x160,
impl0x6,TU106. Tuple is measured, not inferred from PCI/name. Register only
TU106/580 capsC573/word0xc100 and2801/2806 exact4byte request3 outputs2/0.
No whole page, other selectors, neighbouring dies or unmeasured branches.

Both new positive tests fail before rows. Full isolated CPU candidatec266772aa
passes3245/0/14ignored (target-turing-hw);28bench harness tests pass. GPU device
sourceb8d092b0 has identical production Rust/data; c266 changes tests formatting
and docs/inventory only. Workspace fmt inherited differences remain; noCIgreen
or master promotion. Source before/after logs are saved here.

Windows compressed baseline relay13chunks completes17:00:26UTC, bothends
SHA539aaad2f7280d7fb80fcf67c16297792b7561113c9084978070e9d94ace044b.
Fresh Turing headless preflightSOUND after removing the obsolete Ampere-only
marketing-name rejection; hardware-native workloads are still required by Tier2.
Bench tree completes17:05:17, packed guest580 verified independently of host595.
Hardware tests begin only after provisioning's QEMU/guest build completes.

## Linux hardware partial,17:50UTC

Deviceb8d092b0, host595.91.07, thin/fatguest580.159.04. Gates9/9+gate10PASS,
all12gatebirthsUSER; native raw30/30 andCUDA4/4. Guest raw29PASS/0FAIL/
1CRASH(TIMEOUT): ce-client-guest-ram/R34 exceeds unchanged180s budget.
Independent guestCUDA4/4 passes. No full Linux cell pass or birth-census claim.
Native success makes the guest timeout a kayfabe performance defect, not a
physical GPU/driver limitation.

H-R34 stated before diagnostics: high mapping/cleanup cost rather than a
permanent CE stall. Falsifier: shallow copy/content failure, or no finite
completion at the original13000decoys. Shallow0decoys passes; original13000
passes correct content in201s (client184670ms), diagnostic300s budget. This
supports finite overhead; it does not isolate allocation, walking, GPU callback,
interrupt or cleanup costs, and does not make180s acceptance green. The frozen
client prints its result after cleanup of operands and all decoys. Original
failure logs retained. Ranked next candidates: nested trapped BAR0/IRQ cost,
map/unmap/cleanup cost, host CUDA callback signalling. Next falsifier: profile
those phases against this same native control before selecting a bounded
completion-path change. Never filter owner-required NSI/IRQs to force a pass.

Windows phase1 binds the image's exact580.88 INF, sets GSP flags and shuts down
cleanly; its expected pre-reboot post-start error is not phase2 acceptance.
Fresh production-profile phase2 starts17:49:51UTC; desktop/Code0/apps pending.
Canonical strict Windows validator08f25dd9d checks actual build andCode0 before
and after apps, verifies4trusted hashes, CUDA3, hardwareD3D11/12, and requires
readableSystemlog before treating zeroTDR as evidence.

## Windows580 first failure and fix,18:04UTC

Before deviceb8d092b0: fresh phase2 reports Code0/32.0.15.8088/WDDM,
Explorer+DWM,1920x1080; QGA final evidence reconnect times out once but responds
again standalone. Do not call that a proved kernel hang. Initial app staging
shell escaped the filename variable, so all probes shared one literal path;
strict hash/path preflight rejects before any workload. Corrected paths retain
the failed run. Actual cup2 then fails cuCtxCreate999.

First relevant product refusal: c46f/TURING_CHANNEL_GPFIFO_A channel,
USER_RING_SOURCE_DECLARATION. The relay explicitly accepts only AMPERE c56f,
although the earlier per-family capability/alloc path admits c46f. Same native
Windows580 reference passes these probes. H-CLASS, stated before rebuild:
generated host-derived family ChannelGpfifo classification removes this refusal
and admits correct GPU content; falsifier is the same declaration refusal or a
GPU content/fence failure after rebuild. No privilege/ownership rule changes.

Extracted original predicate plus two tests: both fail atc5ee774cb. One earlier
fixture referenced a missing legacy generated ABI constant and did not compile;
corrected fixture uses the generated-family/public clc46f value before recording
the assertion failures. Fix8e3a9d325 uses family.classes().kind_of(class), retaining
all entry/address bounds, Windows permit, USERD ownership and native USER birth.
Every family class is tested; invalid/family-incompatible classes, unaligned or
wrapping addresses and oversized/non-power-of-two counts refuse. Declaration
acceptance is not Blackwell GPU execution proof. Existing private-ring progress
semantics still require actual hardware checks per family.

Exactfix8e3a9d325 full isolated CPU3247passed/0failed/14ignored,
target-turing-hw; hardware binary8e3a9d32 built18:01:45UTC. Before logs and CPU
results inwindows-before/. Old Windows VM shuts down normally17:57:48UTC.
Fresh fixed boot18:02:30UTC; apps and Linux regressions pending. No family pass.
