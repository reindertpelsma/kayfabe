# Heartbeat follow-up from existing evidence

STATUS: RESEARCH, 2026-10-20. GPU-free review after the GB202 lease closed;
no new hardware arm, product change or clock/period change.

H_METADATA_ORDER (INFERRED before code inspection): the current-life flag might
become visible before the corresponding system-info body replaces the cell.
Falsifier: the producer stores the body before setting the flag, with the state
mutation excluded from the heartbeat reader by the same state mutex.

CODE READ result: this particular race is rejected. `kf-gsp/src/boot.rs`, the
GspSetSystemInfo NoReply arm, calls `cell.store(payload, sequence)` and then sets
`system_info_current = true`. The device's register application owns its GSP
mutex while servicing the FSM; `heartbeat::try_sample` acquires that same mutex
with try_lock before reading the flag and borrowing the cell. No fix is proposed
for a flag-before-body ordering that this code does not have.

MEASURED from the already-exported `linux595_cuda/rungs/cup3/qemu.txt`: three
fn72 records occur at lines350,1074,1541; the latter two follow Halted -> Booted
and GSP-PUBLISH of fresh queue addresses. The corresponding complete guest log
contains two nonzero-heartbeat warning bursts, 144 and160 records. These counts
are observations; the untimestamped RPC lines do not establish an exact temporal
join with the guest warnings.

INFERRED next hypothesis: an existing mailbox value could survive a GSP start
and be interpreted with a new sysTimerOffsetNs before the next periodic sample.
This is distinct from the rejected metadata ordering race. The current reader
refuses unavailable metadata but that alone does not prove existing register
contents are valid in the next epoch. Publication runs every500ms, unchanged.

Falsifier for a future bounded diagnostic: show that both mailbox words are
cleared or published in the new epoch before the guest's first heartbeat check,
or that a warning's raw value was freshly published using that same epoch.
Correlate actual fn72 epoch, old/new mailbox words and the first guest check;
do not infer the join solely from similar counts. Any diagnostic must be bounded,
off by default, nonblocking and leave the500ms period and workload oracle intact.

No protected lifecycle/reset/scrub/relay change is authorized by this hypothesis.
Any required dependency there must be named under the existing STOP rule before
changing it. Native PTIMER/VF_TIME agreement remains measured and rules out only
that sampled native clock-view mismatch, not this proposed cross-start mechanism.
