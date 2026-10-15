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
