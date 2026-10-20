# H1 offline timeline tools

STATUS: RESEARCH, updated 2026-10-20. These tools analyze normalized evidence.
K1 owns the single runtime profiler/hooks/collectors; H1 builds no runtime ring.
The K1 importer awaits a concrete JSONL format on origin; raw ETL is not parsed.
No file here is authorization to access the owner VM or GPU. Hardware requires
coordinator `go` and the collection/budget decisions in the preregistration.

MEASURED: 115 unit tests, EXPECTED RED stages before implementation; mutation
tables and read-only evidence in `traces/hitch_timeline_20261019/`.

```sh
python3 -B -m unittest discover -s scripts/bench/hitch/timeline -p 'test_*.py'
python3 -B scripts/bench/hitch/timeline/long_acts.py QEMU_LOG --out long_acts.json
python3 -B scripts/bench/hitch/timeline/analyze.py \
  --events events.jsonl --broker broker.jsonl --guest guest.jsonl \
  --calibration calibration.json --vm-start-ns VM_START_MONOTONIC_NS \
  --context-coverage coverage.json --out result.json
```

All event timestamps use integer host CLOCK_MONOTONIC nanoseconds. The broker
input is a JSONL stream of actual presentation observations with `ts_ns`,
content/request `seq` and boolean `new`. It is not the aggregate `--stats-line`.
The sequence must come from a verified content/commit correlation; compositor
output sequence and VMM frame sequence are not interchangeable.

Guest input uses QPC `qpc` (and optional `end_qpc`), with per-boot calibration
`qpc0`, `host_ns0`, positive `frequency`, and measured `error_ns`. The integer
conversion does not assume QPC equals TSC. A measured VM-start anchor produces
event/gap uptime and ten-minute bins using actual observation time. A gap is
assigned to the bin of its preceding new-content presentation. Device reset
does not reset VM uptime. Missing VM start produces UNKNOWN uptime, not zero.

Normalized interval events for causal candidates have `ts_ns`, `end_ns`,
`kind`, correlated `frame_seq`, ordered `stage`, and host `tid`. The existing
S1–S7 kinds and S8 `rpc_reply_wait`/`held_reply_wait` describe provided stage
intervals. S8 additionally requires `guest_thread_wait_verified: true`, positive
`guest_waiter_pid`/`guest_waiter_tid`, and bounded clock uncertainty. Never set
verification from host timestamp overlap, a depth=1 line, or an RM handle.

MMU phase events: `mmu_arrive`, `mmu_walk`, `mmu_plan`, `mmu_apply`, `mmu_clear`;
scope `inval_seq` with `device` and `generation`. Walk/plan/apply intervals need
`end_ns`. Missing clears are pending; missing phases are UNKNOWN. A batch that
answers multiple invalidates must be expanded only from its recorded request
membership, with shared work identified; do not infer membership by proximity.

RPC phase events: `rpc_arrive`, `reply_held`, `rows_applied`, `act_resolved`,
`reply_posted`, `reply_post_attempt`; scope `rpc_id` with `device`/`generation`.
Record `function`, `hold_reason`, `guest_client`, `guest_object`, `object_class`,
`row_count`, `page_count`, and guest waiter identity only where actually known.
`rm_call` intervals with the same RPC identity supply separate host-call time.
Only a successful `reply_posted` closes a wait. Posting is not consumption.
The longest-wait inventory preserves early/boot waits even when their overlap
with later stalls is zero. Missing posts stay censored. Open intervals require
complete coverage to compute an exposure lower bound.

Observed `completion` events carry `engine` (CE2/CE3/GR0) and optional `count`;
these are completion/interrupt-edge observations, not inferred GPU execution
timestamps or a count of presented frames. `guest_vram` rows preserve measured
fields such as dedicated used/budget bytes, shared bytes, PID/LUID, sample source
and app-running state. The output retains before/inside/after sample positions;
it does not interpolate a post-exit sample into a gameplay peak.

`coverage.json` is an assertion of validated collection, for example
`{"mmu":true,"completions":true,"guest":true,"replies":true}`. Missing or lost
coverage produces observed counts with UNKNOWN exact rates/latencies. `--lost`
records known loss. Causal `--coverage` must be supplied only after the entire
collection, guest ETW/active-workload and correlation gates pass. Without it,
classification stays UNKNOWN. Static desktop counters cannot create a gap.

The long-act tool inventories only printed acts strictly above 20 ms. A
`SET_TIMESLICE 400 us` parameter is not its call duration. The timer covers the
whole act closure and omits queue wait; quiet/requeued acts may omit lines.
Existing lines cannot establish actual guest waiter/group mode/uptime. Only
an explicit `act_end_mono_ns` event stamp supplies act time; a tailer's arrival
timestamp is not equivalent. Missing identities remain UNKNOWN in JSON/TSV.

NOT VERIFIED: real-data collection compatibility, instrumentation overhead,
three-hour recorder operation, peak residency or any cause of the owner stalls.
The three-hour unit case and CLI smoke are synthetic analyzer checks only.

## S9 pre-TDR observations

S9 is a separate degradation phenotype, not a replacement for the first-stage
S1–S8/UNKNOWN causal class. The result includes `pre_tdr_windows`, a status
summary and `pre_tdr_window_indices` on overlapping gaps. Every supplied
`guest_tdr` remains visible, including unknown/unverified cases. Generic Event
153 rows are not converted to TDRs. These are H1 normalized fields, not a claim
about the pending K1 JSON schema:

- `guest_tdr`: aligned `ts_ns`, actual positive integer `pid`, explicit
  `tdr_verified: true` only from a verified reset observation.
- `guest_present`: aligned individual `ts_ns` and that PID.
- `workload_active`: observed `ts_ns`, `end_ns`, PID, covering the entire
  ten-second window plus clock-error margins; contiguous segments are allowed.
- Context coverage: `guest: true`, measured `guest_start_ns` / `guest_end_ns`
  covering the window plus margins. `broker: true` and bracketing broker
  observations are required for host rates. Known loss invalidates exactness.

Ten one-second half-open bins retain guest Present and broker new-content
observations separately. First/last two-second counts are compared using
conservative bounds from twice the measured calibration error. Printed rates
use nominal aligned timestamps; count bounds preserve boundary uncertainty.
Missing coverage yields null rates and bounds, including observed zero counts.
No positive early baseline, unclear clock/identity/activity or ambiguous count
bounds yields UNKNOWN. DECLINE_OBSERVED describes a covered rate decrease;
NO_DECLINE_OBSERVED describes a covered nondecrease for this window only.
Neither establishes the cause of the TDR or rules out other kinds of hitch.

CLI guest rows use `qpc` / `end_qpc` as above; the same measured calibration
aligns TDR, activity and Presents. Normalized Python callers provide all rows
in CLOCK_MONOTONIC ns plus `clock_error_ns`. No wall-clock correction or
per-second aggregate is accepted as individual Present evidence.
