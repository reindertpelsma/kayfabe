# Windows K: same-revision memory-registration-off comparison

**STATUS: RESEARCH, 2026-10-05.** Same exact product `fdc991c9` and runner
`152afe7b` as [J](../probe-j/README.md), on a fresh overlay with only
`--memory-list-probe` omitted. Other experiment flags and display configuration
are identical.

Function4 is refused, initialization stops before display-resource creation,
and Windows remains Code43/NVIDIA-SMI9. There are213 traced/215 serviced RPCs and
zero GPU-channel births. I also has213 trace records: the records and statuses
match as a multiset, with an earlier display-query ordering difference preserved
in `rpc-difference.txt`. No trace identity is claimed.

This supports attributing J's extra initialization to enabled memory registration,
rather than to the accompanying shared RAM-listener and resource-lifetime fixes.
It does not establish successful Windows GPU work, full registration lifecycle
consumption or compatibility on other driver/GPU tuples. Windows shut down
cleanly and QEMU exited0. No hardware rebinding was performed.
