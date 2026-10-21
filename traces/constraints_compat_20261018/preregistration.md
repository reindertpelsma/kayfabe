# V2 source audit preregistration

STATUS: RESEARCH, 2026-10-21. Pinned subject: e64b8e5f6d0f64c41c9c4d40775e74be08de7164.

MEASURED: origin/master was fetched and matched the lane's pinned revision before the
V2 worktree was created. Started 2026-10-21 19:49 UTC; deadline 22:49 UTC.
No product edits, GPU, host connections, VMs, closed binaries, or delegation.

These hypotheses and falsifiers precede substantive implementation inspection.
The rules are read with dated owner amendments; an explicit exception is reported
with its scope. A contradiction stops any proposed implementation: this lane only
records it, the options, and proposed tests.

| Hypothesis | Falsifier / check before looking |
|---|---|
| H1: production traps conform to the BAR policy | C MemoryRegion / callback registration or Rust trap classification admits a BAR1/BAR2 trap, or a BAR0 read outside a documented family or diagnostic exception. Trace both registration and handlers. |
| H2: QEMU vCPU/main/input/register paths obey blocking and capability limits | A reachable mutex wait, join, blocking socket/file/logging call, host ioctl/map, CUDA sync, unbounded spin, or wait without the new-input fd. Inventory all sites, inspect callers and lock scopes; distinguish realize, callbacks and worker teardown. |
| H3: channel work, completion, address and privilege boundaries hold | CPU execution of engine work, guest-byte host control forwarding, completion without real work/event, private mapping outside an authorized space, changed FIXED VA accepted, or privilege bracket/reply check bypass. Search all sinks and existing negative tests. |
| H4: constants and compatibility values follow the tier/source rules | A per-die or driver-keyed value lacks an open/generated/host-query/measurement source, an unknown key borrows a neighbour, or a required generator consumes closed binaries. Enumerate tables, generators, selectors and tests. |
| H5: default configuration needs no measurement/behaviour env opt-in | A boot prerequisite or production work route requires a non-default env flag; a measurement flag defaults on. Enumerate environment reads with parser defaults and indirect wrappers; source evidence cannot itself prove a flag-free hardware boot. |
| H6: unsafe and architectural separation are enforced | An unsafe/raw pointer/address sink is outside the named perimeter, kf depends on frozen kayfabe crates, CPU PT walking/snapshots/joins/IPC reappear, or existing structural gates fail. Run available GPU-free gates/tests. |
| H7: documentation and public provenance claims have their evidence | A missing dated STATUS, unrecorded supersession, unavailable cited revision/evidence path in the pinned git tree, untagged/blob table row, or public closed-binary offset/analysis marker. Scan text only; never inspect a closed binary. |
| H8: known family generation rules cover Blackwell without per-die capture | GB20x selectors, constants or generated outputs require a die-specific row or a driver-specific value not derived by a documented mechanism. Inventory chip, consttab, GSS, display and ABI keys and source tags. |

The report separates source readings, executed checks, and inference (2026-10-21).
A grep absence is a
bounded source result, never proof of runtime containment. A test pass applies only
to its exercised paths. No behaviour change means no fix mutation obligation;
existing instrument self-tests may supply known positives without product edits.
