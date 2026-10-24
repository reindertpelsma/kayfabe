# Run 3440 TDR evidence — T1

**STATUS: LIVE-RESEARCH, 2026-10-22.**

MEASURED: `snapshot2-summary.json` contains bounded line-numbered excerpts and
counter summaries, hashes of the original supplied files, the eight refusal
words, and the nominal last ninety status heartbeats before first display loss.
`snapshot3-and-workloads.json` adds the third snapshot's hash/prefix verification,
twelve refusal words, thirteen unload markers, T-space retirement comparison,
and four already-exported workload run logs. Original large logs remain on the
trusted host; private paths are in the local T1 report, not committed here.
Run 3440 extraction at commit e826ddb4d: [snapshot2](snapshot2-summary.json)
and [snapshot3/workloads](snapshot3-and-workloads.json).

MEASURED: collection used Python text extraction through SSH stdin, with no
remote file writes. Line numbers count original log lines from one. Status lines
were recognized by `kf3: family=`; fields were extracted by their named brackets.
Refusal length uses the public C56F entry1 LENGTH field, bits 30:10 in dwords.
The HELD reply balance adds each logged HELD and subtracts POSTED n; this is a
log-coverage check, not a latency measurement or a complete queue invariant.
Collection attribution: T1 run 3440, 2026-10-21/2026-10-22 read-only extraction,
recorded at commit 77ed23eff; input hashes and output fields are in
[snapshot2](snapshot2-summary.json) and [snapshot3/workloads](snapshot3-and-workloads.json).

CODE READ: status heartbeats are emitted after at least two seconds and can be
late. The "ninety heartbeat" sample must not be assigned exact three-minute
operation timestamps. The first and last clock anchors instead bound 212.444 s.
Cumulative maxima and bounded refusal sample lists do not become per-window
maxima or total refusal counters by subtraction.

MEASURED: thirty offline consistency assertions passed (counts, descriptor
arithmetic, reply checkpoints and local links). No Rust tests, mutation tests,
clippy or hardware actions were run; no source behavior changed. Diagnostic
acceptance cases in the design are planned, not measured passes.

CODE READ: VF1's final published checkpoint `584ebaede` reports a safety refusal
before native launch and zero VFIO samples. No comparison score is inferred.
The complete interpretation and falsifiers are in
[the research note](../../docs/design/V3_TDR_SPECVIEWPERF_20261022.md).
