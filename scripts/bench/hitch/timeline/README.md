# H1 offline timeline tools

STATUS: RESEARCH, 2026-10-19. These tools analyze normalized evidence. The
runtime recorder, raw dump/ETL adapters and live collectors are not implemented.
No file here is authorization to access the owner VM or GPU. Hardware requires
coordinator `go` and the collection/budget decisions in the preregistration.

MEASURED: 84 unit tests, EXPECTED RED stages before implementation; mutation
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
