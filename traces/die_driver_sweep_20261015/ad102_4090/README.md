# AD102 native Windows reference wave

**STATUS: LIVE, 2026-10-15.** Instance56096336, RTX4090/AD102, auto-retire
about15:40:55UTC. Working branch codex/sweep-ada-20261015; no acceptance claim yet.

The owner requires both Linux and Windows guest suites across several supported
builds before a family is covered. A native reference does not satisfy a guest cell.
GA104 Windows remains unresolved; Turing and Blackwell hardware remain unverified.

## Order and hypotheses, before the run

Order change reported to the owner: allocate this rental's first useful window to
the stock Windows oracle. GA104 conversion started with only44minutes remaining
and never regained SSH; that is an installation timeout, not a GPU/driver limit.
This run allows more installation time and preserves the original175-minute timer.

H1: the public, pinned windows-on-vast installer can establish a stock Windows11
EnterpriseLTSC2024/580.88 oracle on this AD102 rental. Falsifier: no verified SSH,
installer completion and NVIDIA adapter Code0 before the lease deadline. A timeout
falsifies provisioning on this run, not AD102 support.

H2: the bounded measurement module can read the physical display caps page and
capture GSP initialization after explicit GSP enablement. Falsifier: module fails
to load, page read rejects, no queue attach during a bounded restart, or a lost
capture prefix prevents attributing the requested controls. No T3 row will be
added without a successful stock Windows read keyed to this die and build.

H3: the cross-built Windows census decoder reproduces its golden fixture result
on native Windows. Falsifier: runtime failure or output mismatch. Raw captures
remain on the untrusted rental; only scrubbed, explicitly selected text returns.

Public installer: reindertpelsma/windows-on-vast93b65b3347601f35f4146602ff85950e389b409f;
upstream bin456789/reinstall80c3d5e175f39c2d2bbd267cd583842140154140.
Measurement module source32973658a; census decoder4ff31a249. Their CPU/build
checks are recorded on their topic branches; runtime remains unverified here.

## Measured reference at13:19UTC

Stock Windows11EnterpriseLTSC2024Evaluation build26100, NVIDIA580.88
(32.0.15.8088), RTX4090/AD102 device2684: adapter Code0, WDDM.
CUDA cup2/cup3/cup8 all pass, including2048x2048 matrix bad0/maxerr0.
D3D11 hardware compute/draw and D3D12 hardware compute/fences pass,20iterations
each; Microsoft Basic Render was enumerated but excluded. System Event4101
count0 during the short oracle window. GSP firmware N/A in this default driver
configuration; a separate explicitly enabled-GSP reference is still required.
No interactive desktop/DWM claim was made for this SSH session.

Windows census golden runtime passes:10command+11status messages,7controls,
zero pairing/gap/parse anomalies. controls/functions/anomalies TSV SHA256 all
match Linux exactly (native-preflight-golden.txt). H3 passes. NVIDIA/CUDA
installer complete=true, failures0, reboots0 before our test-signing reboot.

H1's native installation/adapter oracle succeeds. H2 measurement remains pending.
This is native Windows, without kayfabe; Linux and Windows guest cells on AD102
remain not tried in this campaign.

## GSP-enabled native reference and exact small controls

Same stock580.88, explicitly enabled GSP firmware580.65.05: adapter Code0,
CUDA3/3 and D3D11/12 pass again; Event4101 count0 during the short workload.
The decoder uses the public580.65.06 table, the same-CL Linux twin; its runtime
transport/layout validation succeeds. The measured firmware version is580.65.05.

Capture580b: mailbox attach,10009messages (4992command/5017status),
10041records, END/filecheck OK; zero loss records, sequence gaps, torn reads,
invalid elements, unreadable records or sink drops. Decoder oracle PASS, zero
unanswered/orphan control pairs. Only controls2801/2806 were explicitly
allow-listed; raw and scrubbed binary captures stay on the rental. Measured
2801 request3 ->2 and2806 request0xb ->0, four-byte bodies, status0 each.
GA106's request3 is not accepted for AD102's2806 row. Caps word0x00c remains
0x100 with GSP enabled. No board-specific852f value was exported or installed.

The first capture preparation aborted before restart because its readiness check
mistook a buffered, zero-length output file for failure. It is not evidence.
The successful replacement capture's independent END/filecheck/oracle establish
completion; PowerShell printed an empty collector ExitCode, so no process rc0
is claimed from that field. CPU caps checks after the row:192pass/0fail/2ignored.

## Merged candidate, before guest runs

Source56f32dc0e: full kf-* CPU suite 3241pass/0fail/14ignored,
checkout-specific /data/codex/target-ada. The first merged run caught only a
stale42-row inventory assertion after adding two rows; corrected count44,
then the complete suite reran green. Native requests are served exactly by the
production responder, with wrong questions/dies/branches refused.

H4 (before reboot to Linux): the same pinned public reinstall tool can restore
Ubuntu22.04 minimal with root public-key SSH on this owned rental. Falsifier:
no verified Linux SSH/KVM/GPU within the remaining lease. A conversion timeout
is provisioning evidence, not a kayfabe GPU limit. Original timer stays armed.
Windows-to-Linux preparation first failed its Cygwin certutil download; supplied
the official public HTTPS installer (SHA256
2c9f2fb56e1fb687b5d9680afa8f8b06e6214f0e483096af0eae1946431226c5),
then retried. All native evidence was pushed before conversion.

H5 (before subsequent kayfabe run): native AD102 caps and exact small controls
remove the known per-die measurement gaps and Windows580.88 reaches Code0,
DWM/desktop and hardware D3D with zero TDR under the production profile.
Falsifier: any missing acceptance criterion. Additional refused controls or
interrupt/display-progress gaps remain candidates, not assumed root causes.
Hardware order: stock Linux gates/raw/CUDA oracle, exact-revision guest raw,
CUDA/graphics and broker regression, then Windows guest. No family coverage
until both OS suites pass across several supported driver builds.

## Linux pre-run checkpoint,14:07UTC

Ubuntu22.04.5 restored, kernel6.8.0-138-generic, persistent root147GiB,
KVM present, host595.91.07 open loaded. Bench READY at14:05:29UTC,
QEMU binarykf3-bins/3ab534ca (product code identical to verified56f32dc0e).
Running-kernel headers and the independent fat-guest580 package had to be
installed on this minimal image; NAT additionally needed iptables. These were
provisioning defects, not GPU/driver limits. The source image relay's uutils dd
pipe failure is fixed with a bounded pread producer; full image hash is pending.

H6, before hardware runs: exact candidate3ab534ca passes native Linux595 gates,
raw/CUDA oracle, then guest580 raw30/30 and CUDA4/4 with USER births; subsequent
graphics and broker must preserve those results. Falsifier: any gate/arm/rung,
birth-privilege or display validation failure. A native failure is measured
separately before a guest failure is attributed to kayfabe. No changed grader.
The driver/guest/kayfabe axes remain independently recorded.

## First Linux hardware result and timer falsifier

Native595 at3ab534ca: gates9/9+gate10PASS,12/12USER channel births,
raw30/30, CUDA4/4. Guest580 first raw suite29/30: timer mapping read pairs
advance, but the mirror read follows the page read by3,138,400ns, beyond the
unchanged1,000,000ns grader bound. Other29arms pass; no full guest pass yet.
The image relay completed13/13chunks and full source/destination SHA256
539aaad2f7280d7fb80fcf67c16297792b7561113c9084978070e9d94ace044b matches.

H7, before replay: scheduling during the image relay may explain the single
late timer pair. This is inference. Falsifier: unchanged replay repeatedly
exceeds the bound after relay completion, or the clock implementation/model
shows divergent updates independent of guest scheduling. Retain every replay
and rerun the full suite; never loosen or modify the frozen grader.

Unchanged timer replays1/2/3 all pass: mirror deltas71,200/82,080/67,136ns.
The source confirms production maps the unprivileged native host timer page;
QEMU log says BAR0=0x9000, read-only4096bytes, matched host/guest layout.
The grader prints a line between its second page reading and mirror reading;
I/O/scheduling delay is consistent with the first failure but not proven.
The original failure remains evidence. Full suite rerun follows, unchanged.

Full unchanged guest580 rerun:30/30, CUDA4/4, birth census161/161USER
(+12/12USER gates), at3ab534ca. Initial timer failure and all three successful
replays remain inlinux/. Native595raw30/30+CUDA4/4; image relay complete/hash
matches. At14:26UTC, Windows580 guest takes priority before Linux graphics/broker
regression so a failing cell can be investigated within the original lease.
Order change reported to owner. No AD102 desktop/full-cell or family claim yet.

Windows first launch at14:28UTC exits before guest boot: qcow2 "unknown
compression type:1". Full image hash matches. The source image is zstd-compressed;
kf3's --without-default-features build did not explicitly enable its zstd reader.
No NVIDIA adapter or T3-fix verdict was obtained from that launch.

H8, before the rebuild: explicitly enabling QEMU's public zstd decoder permits
this same verified baseline to open. Falsifier: the image still fails to open,
or stock qemu-img identifies another compression type. Install libzstd-dev in
canonical bench deps; add --enable-zstd to the recorded configure flags, rebuild,
then retry the same overlay. Retain failure and final-image integrity evidence.

QEMU zstd probe: image-open rc1 before, rc0 after explicit--enable-zstd;
new binaryda0569a6, product code unchanged. Windows phase1 now QGA-boots,
binds/flags580.88 and shuts down via QGA. Fresh phase2 reports NVIDIA
Code0/CM_PROB_NONE,1920x1080@60Hz, DWM1/Explorer1. No app pass yet: first
probe deployment accidentally used a literal filename; strict hash/path checks
refused. Correcting the deployment and rerunning acceptance before any cell pass.
