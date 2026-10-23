# Q4 K3 perf investigation

STATUS: K3 CORE ANSWERED WITH LIMITS, 2026-10-23. Candidate correctness remains PARTIAL/HOLD.

MEASURED runs4620/4621: paired diagnostic CATIA/Creo arms each completed once on
candidate01a171e63 and physical VFIO, with no reset observer failure. See
[run4620](run-4620.json), [run4621](run-4621.json), and the
[stamped research comparison](profile-comparison-nc.md). These are profiling arms.

MEASURED 2026-10-23, telemetry-4620.json / telemetry-4621.json: CATIA GPU busy averaged74.93% through kayfabe
and85.78% through VFIO; Creo averaged59.34% and58.57%. See
[4620 telemetry](telemetry-4620.json) and [4621 telemetry](telemetry-4621.json).
INFERRED from runs4620/4621: H1 is supported for CATIA but its stated falsifier
is met for Creo; a universal starvation explanation is insufficient. Driver
sampling windows, store capacities and display carriers differ; one boot per arm.

MEASURED 2026-10-23, kvm-4620-catia.json / kvm-4620-creo.json: non-HLT exit elapsed was0.290 and0.198 core-equivalents
in short CATIA/Creo prefixes; run4621 gave0.047 and0.070. See the KVM JSON files.
HLT idle waits are excluded. Exit elapsed is not serial critical-path time.
The original perf-kvm record wrapper aborted at its byte limit in run4620;
those files are explicitly partial. Later plain tracepoint captures closed cleanly.

CODE READ at70f7fb591: the optional direct instrument timestamps before trap
publication and after the actual native doorbell returns; it preserves native
results and has no product decision role. First pending kick to next native
completion is not one-to-one pushbuffer causality or worker dispatch time alone.
MEASURED 2026-10-23, controls/software-controls.json and tests-k3.json:19 focused tests passed,
1097 broader executions passed with4 existing ignores, five mutants killed by
at least two assertions each, clippy added0. See [test proof](tests-k3.json).

MEASURED 2026-10-23, run4622 and linux-profile.json: direct histograms and CPU
stacks are complete; Windows Passthrough was zero and a separate Linux positive
control exercised one real native Passthrough call. See [analysis](analysis.md)
for hypothesis outcomes, attribution limits and one proposed optimization.
[The original falsifiers](plan.json) precede all runs. No optimization is claimed.
