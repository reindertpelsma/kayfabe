# G: mark and stitch wall time during one Enscape06 diagnostic run

**STATUS: RESEARCH / DEBUG EVIDENCE, 2026-10-28.** One additional invocation after
[E11](../enscape_e11_stack_20261028/README.md); no product fix or extra run.

MEASURED on 2026-10-28 at commit `ebeaa49b842cb4d77217549d3aebcf5ba1c02f55`:
one complete synchronous target apply took 3008.184730ms. Outer
Reservations::mark used 107.007069ms; whole stitch used 2612.129799ms. Their
nonoverlapping sum is 90.391286percent, meeting the preregistered >50percent
local-overhead criterion for this debug window. Stitch accounts for 86.834089percent.
The <20percent falsifier is not met. This is one per-space apply_entry window,
not the whole asynchronous Batch, split/request wall, or historical TDR cause.
Signal/backtrace work can be included inside the timed intervals.

| Walk | Target ms | Outer mark calls / ms | Stitch calls / ms | Combined % |
|---|---:|---:|---:|---:|
| 16081 | 803.552284 | 24 / 293.634126 | 24 / 355.322462 | 80.760966 |
| 16397 | 3008.184730 | 24 / 107.007069 | 24 / 2612.129799 | 90.391286 |

Both rows have zero refused applies and zero fallbacks. The 803ms row is below
the >1s exposure threshold. These are two emitted valid >500ms windows, not all
refreshes; absent/invalid/smaller windows are not zero. Full summaries report
98304 maps/24 verbs/24 batches carrying 98304 logical runs for walk 16081, and
93454 maps/24 verbs/24 batches carrying 93454 logical runs for walk 16397, both 0 unmaps.
The raw space key is private; both rows refer to the same observed space.

Mark_calls counts OUTER Reservations::mark invocations, each of which may call
inner Live::mark many times across blocks. Inner Live::mark/allocation count is
UNKNOWN. Stitch_calls counts whole stitch invocations; actual pieces and mmap
syscall count are UNKNOWN. All 32 existing map_scattered phase receipts were
printed during boot, before the launch log frontier, with no reliable join to
these windows. Logical runs must not be substituted for mmap/piece counts.

MEASURED on 2026-10-28: 22 stack headers resolve against the sealed binary.
21 top own frames point to MappedRegion::stitch at mapping_unsafe.rs:593 (raw PC)
and :584 (return PC minus one), the per-piece libc::mmap MAP_FIXED placement path. One
other sample reaches allocator growth through Vec::push at apply.rs:1066.
These are sampled positions, not a 21/22 duration profile or exact ticket join.
[Reviewed source][source], [result.json](result.json) and
[provenance.json](provenance.json) bind the source, binary and 18 private inputs.

CODE READ: ledger.rs:492–500 already coalesces adjacent file-contiguous backing
pieces on the canonical path. A generic stitch-coalescing idea was stopped
before code. No speculative fix is included.

MEASURED outcome on 2026-10-28: no completed XML result was recorded. At final
capture the registered PT30M task was Ready, but both named engine children
were present in the final capture, whose guest-QPC marker is 1886.416s after
launch. Thus the requested 1800s
process-tree bound was not enforced. The first final-metadata read failed on
the open vendor log; a read-only shared-file retry succeeded. These deviations
are retained, not counted as a completed benchmark. No resampling occurred.
Three new nvlddmkm event 153 records were queried after the saved guest launch.
All post-sample behavior is PERTURBED. Guest UTC was approximately 2 h behind
host UTC; queries used guest launch UTC. No precise event/counter alignment or
independent unperturbed TDR conclusion is claimed.

Both logger sessions stopped after the observed events. The whole 77480579-byte
trace archive and 4591921-byte final metadata archive were copied to host storage,
SHA256-matched and ZIP-checked before cleanup. ETL ended before final cutoff;
logger-reported BuffersLost=0 is not complete-event coverage. Windows shut down
cleanly through QMP powerdown; owned processes are gone, the lock is released,
GDM is active, cuInit returns 0, /dev/null is a character device, and the sealed
parent is unchanged. Whole-arm 2700s bound was met; the 1800s child limit was not.

Configuration: SPECviewperf 15.x, Windows guest, RTX 4070, installed NVIDIA 580.88,
FullHD 1920x1080, exact vendor selector enscape 6, original 23 A7418 flags plus
KF3_DIAG_REFRESH_STACK=1, both TDR keys absent. Source/binary and vendor hashes
are in result.json. Raw ETL, PCs, private paths, device identifiers, models,
screenshots and numerical SPEC scores stay private. This debug run is not an
official SPEC result and is not submission-compliant. SPEC and SPECviewperf are
trademarks of the Standard Performance Evaluation Corporation, https://www.spec.org.

[source]: https://github.com/reindertpelsma/kayfabe/commit/ebeaa49b842cb4d77217549d3aebcf5ba1c02f55
