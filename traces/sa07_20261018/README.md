# SA-07 shared guest diagnostic admission

INFERRED hypothesis before runs: a shared budget limits guest-driven diagnostics
and preserves replies/actions. FALSIFIERS: a flood exceeds the budget, a new
shared admission lock blocks, formatting runs after suppression, or functional
policy/host actions change.

MEASURED tests-only EXPECTED RED commit a49fd0847:100000 calls to each real
InitTablePolicy unknown-GSS and ChannelPolicy root-client entry point each produce
100000 stderr lines; both assertions fail against the128-line edge-bucket bound.
Restored measurements (kind, lines, bound):[('root', '32', '128'), ('gss', '32', '128')].
Both policy floods execute successfully; these are output-count assertion reds,
not child crashes or missing targets.

CODE READ: one process-wide helper in kf-gsp, reused across47 audited/adjacent
sites. One CAS admission attempt; contention suppresses rather than retries.
At most32 payloads and32 swallowed-count summaries per clock bucket. Clock
rollback cannot replenish the budget; overflow is clamped before packing.
Arguments format only after admission. A pending swallowed count is reported
on a later admitted message; no timer/background logger is introduced.
Admission adds no explicit lock or lazy initialization. Accepted payloads and
summaries retain existing stderr I/O; this is not a nonblocking-I/O claim.

CODE READ: call-site edits change the diagnostic macro only; argument expressions
are read-only formatting/inspection. Replies, status, channel/VA statements,
resource accounting, host verbs and completion order remain outside admission.
BORN text is emitted through the per-act logger; both act success and refusal
are covered. Optional dedicated census/classification/replay/debug announcements
are retained. This inventory does not claim all workspace logs are now limited.
The process-wide budget is shared across devices in one process, so a flood may
suppress other participating diagnostics; swallowed counts remain aggregate.

|Source at base e64b8e5f6|Calls|
|---|---:|
|crates/kf-gsp/src/boot.rs|4|
|crates/kf-qemu/src/chan.rs|4|
|crates/kf-qemu/src/device.rs|2|
|crates/kf-qemu/src/mem.rs|2|
|crates/kf-rm/src/barpde.rs|1|
|crates/kf-rm/src/chanlink.rs|12|
|crates/kf-rm/src/guestsysinfo.rs|5|
|crates/kf-rm/src/inittables.rs|16|
|crates/kf-rm/src/rmrpc/mod.rs|1|

MEASURED whole kf-gsp/rm/qemu GPU-free release suite1194 passed /0 failed /
0 ignored; KVM-GATE RAN0/SKIPPED0, KAYFABE_NO_KVM=1. Bypass-admission mutant2
killing tests; bypass-macro-guard mutant2. Restored floods2 passed. Clippy all
targets257 ->257 source fingerprints, added0. No assertions, deny sets or lint
baselines loosened; restored tests additionally print their observed counts.
All Cargo runs held the shared owner lock, release/two jobs/nice19/own target.

INFERRED hardware obligations later: normal boot/work/reset diagnostics remain
useful under the shared budget, stderr latency in the real deployment, and hostile
RPC churn alongside normal workloads. No VM/GPU or production throughput result
is claimed. Existing stderr writes on admission can still block as before.
