# Candidate-5 source audit

FALSIFIERS stated before the audit: new unsafe code outside permitted files; a
new wait on a vCPU or while holding a lock a vCPU takes; a diagnostic gate
controlling a display decision; a new GP_PUT polling path.

MEASURED: production Rust/C added lines contain zero disallowed unsafe code and
zero GP_PUT mentions. The complete merged diff passes credential, key, address and private analysis path scans. These scans supplement the source review below.

CODE READ: realize waits for its initial zero before publishing the device.
Reset and unload scrub completion run on the VA worker. va_loop takes the manager
out of its initialization mutex before entering the loop; sync_store and scrub
completion hold no such guard. reset_memory_prepare's added mirrors mutex use
reads a count and releases the guard before GPU work. Channel-plane lock matches
are formatting of existing calls; the CPU-window lock match is a test fixture.
String join calls in the candidate list are formatting, not thread waits.

CODE READ: step meters and event recording use stored construction-time booleans.
The exact environment value 1 enables them; absent or other values leave them off.
Disabled meters invoke no sample clock; the disabled ring has no base timestamp,
allocation, event, dump request or output. The conn dump path calls its logging
sink only after take_dump returns Some. The broker flag starts false in the
zero-initialized configuration; diagnostic clock reads return zero when off.

CODE READ: request sequence/open-credit/tick-age/answer-time state and TickCounters
remain active. Gates do not control credit decisions, buffer release, reset Ack,
BootClean or LUT token identity. declined_for only deduplicates diagnostic events.
The periodic FPS line retains the candidate-4 behaviour. No diagnostic flag is
required by initialization or functional state transitions.

MEASURED: the new flags are absent in full-suite runs; explicit constructors test
both enabled and disabled states without process-environment mutation. Gate
mutants forced each way fail at least two distinct tests; LUT restart/retained-
binding mutants each fail two. All eight cursor-plane blobs match candidate-4.

MEASURED: comparable clippy has 878 diagnostics on candidate-4 and the merged
source. Five apparent additions from whole rendered-line comparison were paired
with five identical old diagnostics after rustfmt line wrapping. Matching actual
highlighted source and its containing function gives zero additions/removals.
A positive control using the historical pre-fix source still detects both real
field-initializer warnings. No lint suppression or product change was used.

CODE READ: GPU-free state/source tests cannot establish live GPU completion time,
boot success or hardware performance. Those are outside this build-only lane.
