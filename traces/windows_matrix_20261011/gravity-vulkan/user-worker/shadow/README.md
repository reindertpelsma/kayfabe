# Bounded Windows USER scanner diagnostic

STATUS: LIVE DIAGNOSTIC, 2026-10-12. Default OFF; no migration/Windows compatibility
claim. Source revision is the commit containing this checkpoint.

KF3_WIN_USER_RING_SHADOW=1 alone selects private USERD/ring opaque GP publication
on already eligible Windows SYSUSERD USER-work channels. Native privilege, own
restricted/refreshed VAS, physical-CE deny, owned source admission and actual
native descriptor-credit receipts remain. No CPU deferred actions, private PB
staging or witnesses. Ordinary Linux Passthrough remains unchanged.

Observer: 4 MiB attempted PB-read quota per channel; one first sample per class,
at most128 words, saturating attempt counts. Unsupported GP/parser history or
publication failure stops subsequent CPU PB reads. Logs are mutable observations,
not evidence of execution. Initial maskbit1 for declared0 is an explicit observer
hypothesis only. Ordinary native birth retains default0 in this case.

GPU-free: eight project crates2155PASS plus native-probe6PASS. Both review blockers
have semantic failing-first regressions (raw GP writer and failed-publication
parser history). Independent source review cleared exclusive single-VM diagnostic;
shared reset/other-VM availability remains unmeasured. Extra linux-raw run157PASS/
3FAIL, also failing serial: this controller's /dev/null is a regular file, while
those unchanged tests require char1:3. Failures are retained, not called green.

run_campaign.sh requires caller-held global GPU lock and full source revision:
exact Linux9+10, rebuild/client/fresh fast30, Linux desktop, then run_windows.sh.
Windows workloads: desktop implicit, Edge H264 video, GravityMark D3D12 and Vulkan
in one guest, original media/oracles. Production19flags plus shadow alone; normal
migration interception remains OFF. A native unsupported deferred trigger may
still RC; diagnostic is intended to batch observations, not fix it. Hardware
results pending. Personal Windows disk chain is not opened by this campaign.
