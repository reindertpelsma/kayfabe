# E11: sampled VA refresh stacks during isolated Enscape06

**STATUS: RESEARCH / DEBUG EVIDENCE, 2026-10-28.** One diagnostic execution;
post-sample behavior is perturbed. This evidence branch contains no runtime fix.

MEASURED on 2026-10-28 at commit `fa0829d75c3f630266afd375fc0a7fd3cfae9d06`:
the [reviewed E11 debug source][source] build printed three VA-thread stacks
during one vendor
`enscape 6` invocation at FullHD 1920x1080. The first resolves to
`Mapping::anywhere` at `crates/kf-linux-raw/src/mapping_unsafe.rs:303` (raw PC),
`:294` (return PC minus one), called by `MappedRegion::stitch` at line 551.
The second and third resolve to `Live::mark` at
`crates/kf-host/src/sparse/state.rs:30`, inside Vec::with_capacity allocation,
through `Reservations::mark` at `sparse.rs:950`. All three share the
`apply_entry` (`apply.rs:1060`) and `VaManager::on_walk_ready`
(`vasmgr.rs:1743/1744`) path. [result.json](result.json) records source/binary
identity, sample locations and limits. [provenance.json](provenance.json) hashes
12 retained private source artifacts using safe aliases. Paths here are
repository-relative.

CODE READ at the pinned source: stitch at line 551 reserves an anonymous PROT_NONE,
MAP_NORESERVE user address range via libc::mmap; Live::mark at line 30 allocates the next
occupancy interval vector. These sampled locations are in our mapping and sparse
bookkeeping path. No sample shown here identifies an NVIDIA RM ioctl. A sampled
call site does not establish its duration, unique ticket, continuous execution,
or the initiating cause of the historical ENS12 timeout. Signal delivery might
be delayed, and the handler itself perturbs execution.

MEASURED outcome on 2026-10-28: the real engine exited and emitted one Test Index 6
XML record,
but its vendor log reports `waitForEnscapeProcEnd: ERROR: Enscape process was
unresponsive!`. This is not a clean pass. The wrapper's exit field was null;
no pass was inferred from it. The retained System event query since launch
returned zero event 153 records. That query is not an independent complete stall
oracle. No numerical score is published. No second run or TdrDelay comparison
occurred, so the earlier [preregistered intervention][prereg] remains untested.

[prereg]: ../enscape_vfio_reference_20261028/PREREGISTRATION.md
[source]: https://github.com/reindertpelsma/kayfabe/commit/fa0829d75c3f630266afd375fc0a7fd3cfae9d06

Both circular ETL sessions stopped successfully. Raw ETL, EVTX, logs, runtime
maps, image paths and stack addresses remain private. Retention and loss have not
been decoded. The archive was copied to host storage with matching SHA256 and
valid ZIP checks. Windows shut down cleanly through QMP powerdown; owned
processes are gone, the lock is
released, GDM is active, cuInit returns 0, /dev/null is a character device, and
the sealed parent is unchanged. Details are recorded in result.json;
no complete trace coverage or full image byte identity is claimed.

Configuration: SPECviewperf 15.x, Windows guest, RTX 4070, installed NVIDIA 580.88,
production A7418 flags plus only KF3_DIAG_REFRESH_STACK=1, absent TdrDelay and
TdrDdiDelay, pinned source and binary in result.json. This debug run is not an
official SPEC result and is not submission-compliant. SPEC and SPECviewperf are
trademarks of the Standard Performance Evaluation Corporation, https://www.spec.org.
