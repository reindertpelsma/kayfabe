# SA-04: indexed free paths; broader allocation scans remain

MEASURED status: graph traversal and policy free paths implemented; broader SA-04
is PARTIAL. Draft only. Whole kf-rm release suites: 765 passed /0 failed /0 ignored.
Clippy all targets57 ->57 source-bound fingerprints, added0. No lint baseline,
deny set or assertion loosened; a test return-type alias and collapsed if address
new style diagnostics without changing behavior.

INFERRED hypothesis before tests: parent/children and reverse-name indices remove
unrelated-state scans while preserving alias lifetimes and retirement order.
FALSIFIERS: quadratic growth, changed retirement counts/order, early retirement
under a live alias, stale parent edges, or unresolved old orphan after alias reuse.

MEASURED tests first:99f147772 imports the original SA-04 test unchanged. It PASSED
under the prescribed release profile (3000 frees132.5ms;6000 frees667.6ms). Its5s
oracle was measured in debug by the audit and does not prove release red.
bc359a2b0 supplies two release-scale assertion failures; b2e97a870 supplies six
policy assertion failures. Four added lifetime controls pass on master as well
as on the indexed implementation. No setup failure is counted as red.

MEASURED release timings (one run per listed inventory, dev box, nice19; no
hardware benchmark or spread claim):

| Work | Inventory | Before | After |
|---|---:|---:|---:|
| Graph leaf frees |30000|21.415s|15.46ms|
| Graph leaf frees |45000|48.826s|23.63ms|
| ChannelPolicy leaf frees |30000|1.917s|3.57ms|
| ChannelPolicy leaf frees |45000|3.318s|8.31ms|
| PageDirPolicy leaf frees |30000|2.051s|7.02ms|
| PageDirPolicy leaf frees |45000|2.653s|10.96ms|
| Orphan plus last-alias frees |1600|5.730s|1.41ms|
| Orphan plus last-alias frees |2400|11.192s|2.01ms|

CODE READ: graph indices contain only live origin parent edges; dup-kept resource
identity and refcounts are unchanged. Client-root removal uses a namespace range.
Policies use exact/range name removal, reverse alias references, device-share
users, and parent/children lookup. Orphan alias overwrites enqueue candidates for
the NEXT free, preserving the old timing and sorted retirement order. A transient
GPU_DEVICE VAS name still does not retire its device-owned space.

MEASURED hand mutants, unique top-level failing tests (nested child stdout is
not counted twice): master graph2; master ChannelPolicy2; master PageDirPolicy4;
omitted parent index2; omitted overwritten-orphan candidate2; lost alias refs4.
Restored graph3/3 and policy10/10; final whole suite above includes two additional
cache-scope controls. Timing budgets have orders of magnitude of observed headroom.

CODE READ remaining scope: resolve_pending_dups still scans parked entries;
free_subtree still scans the capped parked-dup table; cache_targets still scans
resources on Device allocation. This PR does not claim bounded total RM request
work or close every site in the audit. Those paths are unchanged deliberately.

MEASURED safety counterexample for narrowing cache_targets to the incoming
Device's subtree: allocate a VAS before its non-Device parent, later declare that
parent under Device A, then allocate unrelated Device B. The existing backfill
caches A on the VAS. Free its original name and A; a foreign alias retains the
sticky target. Without the unrelated backfill event it has no sticky target.
Both controls pass. A subtree-only rewrite would alter routing/lifetime on this
accepted event order. INFERRED: further optimization needs an explicit dependency
index over live resources, including alias-kept ghosts and late parents, with
separate tests/proof; a handle-only child index is insufficient. Under the lane's
production-behavior stop rule, that remaining optimization is deferred for review.

INFERRED hardware obligations later: Linux/Windows process churn, GPU_DEVICE
references, cross-client dup/free and actual host-resource retirement. These
GPU-free timings and models do not establish host ioctl latency or end-to-end
production throughput. All Cargo runs held the shared owner lock, used two jobs,
release mode and the lane's own target directory.
