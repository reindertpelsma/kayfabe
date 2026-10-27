# One KVM lease through the complete merge bar

STATUS: LIVE PASS, 2026-10-23. Run `v9-xid-kvm-20261023`, lease57367447,
approved immutable source2c908a0e8dc8965dde7f57ec8ff787d19b28ca09; n=1, no spread.
[Structured receipt](live_run.json), [callback milestones](live_log_excerpt.txt).

MEASURED on 2026-10-23, run `v9-xid-kvm-20261023`: the independent runner accepted
PASS/verified=1 after checking the source SHA, nonce, strict schema and complete log chain.
Suites: provision1; host CUDA4/4; kf tests3749/0; gates9/9 with gate10=PASS;
current-revision kf3; raw30/30; thin30/30 with no crashes/unrun arms; birth census1;
fat CUDA4/4. Every structured row has fail=0/skip=0. These are untrusted GPU-tripwire
observations, not an attested GPU result. Approved code predates rental; a forged verdict
cannot change the code being approved.

MEASURED on 2026-10-23, the same run: one verified GA102/RTX3090 in US, vm:true,
ssh_direct, provider driver metadata580.178.04 and final reported VM driver580.159.04/compute860. No qualifying GA106;
the sole allowed measured-die class was selected without relaxing any quality requirement.
Effective quote0.422847222222 USD/h, compute/storage0.393333333333 USD/h and full
transfer reserve0.088541666667 USD. [Thresholds, rejection counts and per-die verified counts](availability.json),
[chosen instance fields](chosen-fields.json). Country US is in the structured result.

MEASURED on 2026-10-23, the same run: first accepted report121.533s from rent start;
first independent running poll124.910s, sampled every20s. Both blob lookups200 first try.
Cache167141234 bytes in36.455s; provisioning1280.652s to READY; host CUDA13.493s;
merge bar2033.184s; fat CUDA416.590s. The merge watcher's rc6 was admitted only after
its entire delta matched the approved native+guest ledger; the underlying merge script
completed and all its required count rows were accepted. Fat CUDA kept zero new Xids
(10 before/10 after). [Milestone text](live_log_excerpt.txt).

INFERRED from those 2026-10-23 timings: this lease misses the roughly15-minute setup
objective. Its bench-tree critical path is unlocalized. The narrowly scoped own-instance
provider-log diagnostic returned boot output, without an additional bench-tree phase;
it does not establish whether downloads, APT, guest setup or compilation caused the delay.
Separate fresh APT elapsed time and the selected APT mirror remain unverified.
[Diagnostic summary](provider-diagnostic-summary.json).

MEASURED on 2026-10-23, run `v9-callback-replay-20261023`: callback-native records replay
through the same shared S4 matcher: four exact defer CE faults plus the separately exact
missing-page GR control. Wrong channel, VA, type, non-CE client and an extra record each
refuse. Only literal display prefixes/watcher indentation were removed; no fault/client/source
field was rewritten. Public log copies additionally remove trailing ASCII whitespace/EOF blank lines;
the same matcher also passes those copies. [Replay receipt](callback-replay.json), [pre-recorded falsifier](callback_replay_plan.json),
[native defer](native--defer-liveness.log), [native missing-page](native--missing-page-fault.log).
[Guest control callback projections](guest_control_excerpt.log) retain serial/QEMU context.
The prior V8 failed lease's exact journal was absent; this does not reconstruct it.

MEASURED on 2026-10-23, the live run:54 bounded diagnostic frames were retained. The
success-log budget is960KiB, with64KiB reserved for the first refused arm (1MiB total).
Late successful arms can have no frame once that budget is exhausted; their count rows
remain. The full validated/scrubbed stream retained owner-side is1188762 bytes, beyond the
runner UI's200-line display budget. Worker bytes_dropped=47080, tail_bytes_dropped=1036110;
runner scrub drop counter11664. These counters include redaction/sanitization/bounds;
this evidence is not an unabridged independent kernel/artifact capture. No SSH was needed
for these callback records. [Export counters](validated-log-metadata.json).

MEASURED on 2026-10-23, run `v9-cleanup-20261023`: independent exact-id reaper armed
before create;64 actual local scheduled-handler begin/end pairs,60s cadence. Owner-stop
was followed by exact id/label absence; both native Vast API paths independently rechecked
absence. Reaper/tunnel/identity/ingress/Worker processes stopped; local listeners released.
[Independent absence](independent-absence.json), [controller cleanup](controller-cleanup.json),
[temporary runtime/cache/image removal](local_cleanup.json).
INFERRED from recorded rate/lifetime3908.583s plus full transfer reserve: cost upper
0.515590547 USD, not invoice, below stop3/cap4 USD. No additional lease or retry started.

MEASURED on 2026-10-23, run `v9-hosted-20261023`: all exact-source hosted checks passed:
[box push39949647427](https://github.com/reindertpelsma/kayfabe/actions/runs/39949647427),
[box PR39949654242](https://github.com/reindertpelsma/kayfabe/actions/runs/39949654242),
[general CI39949647374](https://github.com/reindertpelsma/kayfabe/actions/runs/39949647374).
[Receipts](hosted.json). Source2c908a0e8; the final evidence commit is docs-only.

CODE READ: no Worker/reporter schema, defaults, deadline, crate code or deployment changed.
Bundle SHA2562402798ce8ab53dcc4db5efca7891b565c0bca7e92723ff0b5050e5bb6e6760f.
The real provider create and callback path used the production tiny-onstart/exact-SHA/env
contract through local wrangler with a localhost test identity. Hosted OIDC and billed
Cloudflare CPU remain NOT DONE. Independent SSH artifact/hardware proof, unabridged
artifacts, separate APT time/mirror and this slower setup's critical-path attribution remain
unverified. Deferred Worker format2/README/review lows remain. Earlier MANUAL retention
proof stays accepted and was not repeated. No deployment or production renting enablement.
