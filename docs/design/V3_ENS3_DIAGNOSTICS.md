# First153 diagnostic snapshot and apply costs

**STATUS: LIVE diagnostic implementation, 2026-10-26; GPU-free validation only.**
CODE READ: the implementation is on `codex/ens3-20261026`, based on SPR2 `3e52e0161`
and GATELOG `21497cbdd`. No sparse-twin, gate, timeout, completion or GPU scheduling
behavior is changed. Sparse twin remains default OFF. Validation is recorded in
[`traces/ens3_20261018/`](../../traces/ens3_20261018/README.md).

Enable `KF3_DIAG_ENS3=1` (exactly `1`) and set `KF3_DIAG_ENS3_TRIGGER` to a regular,
host-owned file in the arm directory. This enables GATELOG recording too. Keep
`KF3_DIAG_VAHIST=1` and `KF3_DIAG_PROGRESS=1000` matched across diagnostic arms.
The host collector writes this text on the first **new, current-boot nvlddmkm153**:

```text
event=153
```

If the affected host channel/guest VA-space key is independently known, use:

```text
event=153 chid=0x67 space=0x123400005678
```

Numbers above illustrate syntax, not a recorded channel. `chid` is the low16 bits
of the host token; `space` is guest client<<32 | VA-space handle. Selectors are
intersected. Without selectors, the snapshot includes all tracked USER rings.
No invented Xid or operand VA is required. The existing VAHIST Xid trigger remains
separate and unchanged; it can still answer a genuine fault VA lookup.

The new observer is independent of the VA-manager thread. It checks the file at
most10 times/second, reads at most513bytes, accepts at most512bytes of strict
syntax, and consumes one valid trigger per process. Missing, malformed, oversized
or non-regular input does not consume the first valid capture. It performs no GPU
or guest read. With the flag off it starts no observer, opens no file and reads no
diagnostic clock. After a successful snapshot the observer exits. A guest cannot
choose the host path. The directory and file must remain controller-owned.

The `ENS3 FIRST153` block uses the VAHIST monotonic origin and prints its Unix
epoch. It contains:

- Each selected USER ring's last64 GATELOG records, including seen PUT/entry,
  gate start/end and hold duration, publish and held-across-unmap events. An open
  hold also reports its age at capture, even if its start has left the recent64.
- The latest successful private descriptor publication sequence and marker-issued
  sequence, with publication time; the latest ordinary private GET/REFERENCE
  observation and its time/age. These are copied from existing worker reads,
  never polled for the snapshot. PUT and marker sequences are separate proofs;
  GET/fetch alone is not engine completion. Observations may be stale and say so.
- The newest128 selected apply records, including applies still in flight, from a
  fixed1024-record history. Each keeps at most8 explicit run samples with MAP or
  UNMAP, range/backing/aperture/kind, start/end timestamps, omitted-run count and
  phase costs. An unfinished apply is explicitly `finished=false`; its final
  costs are unavailable at that instant, not zero-cost work.
- Existing NSI, engine arm/wake/raise and vector-pacer counters sampled over an
  explicitly timestamped interval. Vector fields are emitted as bounded tokens,
  preserving every field of the existing summary rather than truncating one long
  heartbeat line. This is aggregate interrupt evidence, not packet identity.

Storage is bounded by GATELOG's128 channels,4096 records/channel, and the1024
apply slots with8 runs each. The new main block is at most12288 lines of512bytes
each; current fixed caps fit below that without truncation. NSI output adds at
most2048 bounded tokens and a terminator. Channel/history overflow and omitted
runs are printed; absence in truncated evidence cannot falsify an ordering
hypothesis. Histories are copied under recorder locks, which vCPUs never take,
then formatted/emitted after releasing those locks. This is a one-time, bounded,
cross-thread sample, **not an atomic snapshot**; start/end stamps expose that limit.

For kernel Translated CE work, retain the existing optional
`KF3_COMPLETION_PROBE=1000` in all matched arms. It records real private fences,
their releases, and overdue/guest-silent-after-completion diagnostics. The new
USER-ring snapshot does not invent equivalent receipts for passthrough channels.
Correlate those existing probe records and circular DxgKrnl ETW with the snapshot.
Do not conclude an interrupt was lost from a fleet counter alone.

`ENS3 APPLY` reports these distinct quantities:

| Field | Meaning |
|---|---|
| `wall_ns` | Whole apply wall duration, including recorder overhead |
| `thread_cpu_ns` | Actual Linux calling-thread CPU consumption, or `None` |
| `rm_ns` | Wall duration inside authored HostRm ioctls other than invalidate |
| `invalidate_ns` | Wall duration inside the RM invalidate ioctl |
| `cpu_residual_wall_ns` | Wall minus the two RM buckets; includes locks/scheduling/mmap, not CPU usage |
| `walk_ns` | Batch submit-to-collection wall duration, separate from apply |
| `gpu_walk_us` | GPU-reported walk duration for that batch, separate from apply |

CPU consumption overlaps CPU used inside RM, and is not an additive component of
the RM wall split. A batch's walk times may accompany multiple space applies;
do not add those repeated batch values. In-flight apply costs are incomplete.
Thread CPU clock calls occur only for opted-in applies, through the raw-OS
adapter's checked Linux clock wrapper.

`ENS3 COST` prints per-apply interval calls/time/work for ordinary RM,
invalidate, sparse admission, tree clone and error-path intent restoration.
Admission includes its nested clone/restore/RM costs; do not add them again.
`thread_last_call_ns` and `thread_max_ns` are lifetime gauges on the same VA
thread, including when the interval has zero calls. Counters saturate and use
thread-local storage, so another worker's RM call cannot contaminate this apply.
Errors preserve their existing return values and are timed too.

CODE READ: `Reservations::admit` clones the tree for each sparse admission, not
for every plain backed map. Any admission error restores intent by scanning up to
512 page positions per block, even when an initial budget check refused before
changing layout. That is O(blocks) clone storage and O(blocks×512×log(blocks))
restoration work. A mutation journal or avoiding a full restore when no intent
changed is an obvious candidate to investigate separately; exact transitions and
hostile-guest bounds must remain checked. **No such optimization is in ENS3.**

MEASURED (dev probe, 2026-10-26): the GPU-free probe in `traces/ens3_20261018/admission-probe.txt` used8192
normal blocks,152bytes of block payload each, and an immediately budget-declined
sparse request. It visited4194304 page positions and made zero fake-RM calls;
one dev-box run took81.82ms in admission,1.41ms in clone and80.33ms in restoration.
These are n=1 CPU-model timings under nice19, not Windows or GPU measurements.
They establish a potentially significant per-call cost, not the cause of Enscape.

Before hardware use, the controller must seal the exact combined source SHA,
standard `build_kf3.sh` executable hash and flags. The proposed matched experiment
is three fresh prefix boots ON/OFF/ON, capturing raw153XML, circular DxgKrnl ETW and
the trigger before recovery tears channels down. Neither a hardware build nor a
VM run is authorized by this document.
