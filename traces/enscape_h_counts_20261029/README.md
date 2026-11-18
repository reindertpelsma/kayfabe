# H: actual stitch mmap counts in one Enscape06 diagnostic run

**STATUS: RESEARCH / DEBUG EVIDENCE, 2026-10-29.** One invocation after
[G](../enscape_g_costs_20261028/README.md). No product fix or additional run.

MEASURED on 2026-10-29 at commit `161f6677d966d7f8622014ff1f614aa9a9982143`:
two H windows were emitted; both are retained below. Both had zero refusals,
unmaps and fallbacks; map verbs/batches were 12 and 24 respectively.

| Walk | Target ms | Stitch ms | Fixed attempts | Offered pieces | Runs | us/attempt |
|---|---:|---:|---:|---:|---:|---:|
| 19783 | 775.320437 | 488.896787 | 44274 | 44274 | 44274 | 11.042526 |
| 19789 | 2683.219178 | 2005.787809 | 98304 | 98304 | 98304 | 20.403929 |

The ratio is whole-stitch wall time divided by actual counted per-piece libc
MAP_FIXED attempts: **overhead-inclusive AMORTIZED cost, not measured individual
syscall latency**. Validation, reservation, counters, other stitch work and signal
handling can be included. The initial PROT_NONE reservation is excluded from the
attempt counter. Validated pieces are offered before reservation and may exceed
actual attempts on a failure. Equality with logical runs holds in these rows;
it is not a universal identity. Zero attempts would mean an undefined ratio.

Outer Reservations::mark calls/times were 12 / 144.345998ms and 24 / 400.403721ms;
whole stitch invocations were 12 and 24. Inner Live::mark/allocation count remains
unknown. These are conditional valid target_apply>500ms windows, not all
refreshes, whole asynchronous Batch/request wall, or a per-call latency profile.
All raw rows were retained; no missing/invalid row was assigned zero.
The larger window has 98304 calls and 20.404us amortized, not an assumed 90000
or 28us. No fix, optimization or unpaired performance comparison follows.

The preregistered stopping rule used the FIRST complete H row over 500ms after
launch, regardless of count/value/refusal, then a fixed 60s observation tail.
First observed 13:18:09.344389Z; configured tail target 13:19:09.344389Z. One-second
polling handed capture off at 13:19:10.158316Z. Capture request followed around
13:19:42Z. Observation-tail, capture and physical cutoff times are separate;
all later rows remain retained. [result.json](result.json) gives the full timeline.

The frozen trace archive 70513549 bytes and final metadata 4590652 bytes were copied
to host storage, SHA256-matched and ZIP-checked before QMP shutdown. ETL ended
before final cutoff; logger BuffersLost=0 does not establish complete coverage.
A nonce and newly recorded cmd PID/StartTime guarded the owned tree cutoff.
No named engine remained at its confirmation marker 511.407742s after launch,
within 1800s. No stale PID adoption or kill-by-name was used. XML remained empty;
CHILDREN_EXITED, null wrapper exit and task result 0 are not a benchmark pass.
This is an operator-censored diagnostic outcome. Three new nvlddmkm event 153 records
were queried after saved guest launch. Signals and later outcomes are PERTURBED.
Guest UTC was approximately 2h behind host; queries used guest launch UTC, with
no precise event/counter alignment claim.

Windows shut down through clean QMP powerdown; all owned host PIDs are gone,
lock released, GDM active, cuInit 0, /dev/null intact and sealed parent unchanged.
The whole-VM 2700s bound was met. Raw status VM_RUNNING is the prior phase;
unchanged raw endline/restoration prove closure. Both capture SHAs and 23 private
input hashes are retained in result.json and [provenance.json](provenance.json).

CODE READ at G source ebeaa49: the ordinary accepted scatter path constructs a
temporary host CPU view for synchronous RM OS-descriptor allocation, then maps
the descriptor handle on the GPU (channel.rs:1507/1518/1524). Later CPU-view
cleanup is normally deferred. This reading is not a universal release guarantee
or a theorem about the installed driver's internal page pins.

Configuration: SPECviewperf 15.x, Windows guest, RTX 4070, NVIDIA 580.88,
FullHD 1920x1080, vendor enscape 6 selector, same 23 A7418 flags plus existing debug flag,
both TDR keys absent. [Reviewed source][source] and result.json pin runtime,
binary and vendor identities. No numerical SPEC score, model, screenshot, raw
PC, device identifier or private path is published. This debug run is not an
official SPEC result and is not submission-compliant. SPEC and SPECviewperf are
trademarks of the Standard Performance Evaluation Corporation, https://www.spec.org.

[source]: https://github.com/reindertpelsma/kayfabe/commit/161f6677d966d7f8622014ff1f614aa9a9982143
