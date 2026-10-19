# Remaining protected dependency: USER channel family admission

STATUS: LIVE, 2026-10-19. STOP for this edit; proposal only, product unchanged.

CODE READ: the current candidate-4/candidate-5 `Birth::new` declaration check
accepts only generated AMPERE_CHANNEL_GPFIFO_A. The sweep's retained Turing
fix needs `crates/kf-qemu/src/userrelay.rs`, which the latest coordinator
instruction explicitly leaves protected after approving F-2's two files.

Exact upstream proof/fix history: `0b1a1fe7e` (extract unchanged declaration
predicate and add failing tests), `c5ee774cb` (generated-family fixture),
`8e3a9d325` (use the actual family's generated ChannelGpfifo class membership).
The bounded proposal is `userrelay_family_gate_PROPOSAL.patch`, derived only
from that three-commit topic; it is NOT applied to the product. No GPU or host
session was used to establish this CODE READ finding. Current-revision Windows
hardware consequences remain UNRUN, not a measured failure claim.

Proposed scope: only source_declaration and its call in Birth::new, and two
pure declaration tests in userrelay.rs. Class membership uses
family.classes().kind_of(class) == Some(ChannelGpfifo). Preserve entry bounds,
power-of-two/alignment/overflow checks, USERD and subdevice ownership, native
birth permissions, lifecycle/lazy/scan-off behavior, existing tests, all counters,
publication/doorbell paths and channel groups. No old USER batching/measurement
or other userrelay hunks are imported.

Recommendation: authorize this narrow generated-family declaration topic with
failing-first evidence, >=2 killing tests per mutant, before/after release
counts and clippy delta added=0. Alternatively explicitly defer it and leave
Turing's required USER admission unresolved; do not borrow the historical PASS.
This asks for a new protected file only; F-2 and the original exclusions are
already approved and are not being asked again.

The protected-path commit audit of the combined sweep source found this
family topic as the only non-F2 retained product edit under kf-chan,
userpump/userrelay, channel groups/latejoin or the drainer paths. All other
candidate-5 raw preflight collisions in mem.rs/device.rs arise from incidental
old formatting at current reset/budget code; those formatting edits need not be
imported. Preserve current integration behavior there.
