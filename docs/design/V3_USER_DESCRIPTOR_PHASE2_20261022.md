# USER-ring Phase 2: worker boundary and ring audit

**STATUS: LIVE, 2026-10-22. GPU-free implementation validated; hardware pending. Owner correction at 12:40 governs this implementation.**

CODE READ: the audit bases are master `e9c6d39e5` and accepted Phase 1 `468a35c9d`.
The descriptor decision remains authoritative; this note records the scheduling
and wrap obligations added by the owner. Phase 1 remains unchanged. The Phase 2
branch is separate; no inline stage 2 is being built.

## Inline boundary today

CODE READ: `kf-trap/src/trap.rs::TrapPath::doorbell` returns
`Action::RingHostInline` only for `Route::Passthrough`. That is an opaque host
doorbell store, without a GP entry copy or PB inspection. The ioeventfd drainer
uses the same routing (`kf-chan/src/dbfast.rs`). USER-ring birth in
`kf-qemu/src/userrelay.rs::Birth::run` installs no fast-register route;
`ChanPlane::serve` calls `user_ring_serve`, then `Pump::turn_body` drives
`kf_chan::user_turn::Relay` on a worker. This boundary and the worker loop are
identical in both audit bases.

CODE READ: `kf-trap/src/relay_dispatch.rs` contains an unconnected prototype
(`MAX_INLINE_GP = 4`), not a VM path. INFERRED: a future inline copy may handle
only a small fixed number of Pointer entries with pre-existing views and
try-locks, preserving each raw word. It must never scan, wait or block a vCPU or
drainer. SYNC_WAIT, a scan requirement, absent 5080 object, possible migration,
large delta, failed lock or any uncertainty transfers ownership to the worker.
A sync wait always belongs to the worker. Phase 2 changes only worker-owned
classification and keeps all of those future handoff requirements intact.

## Structural turn bound and todo ordering

CODE READ: `Pump::turn_body` ends on `Step::Idle`; a streamed part returns that
step with `Need::Again`, after copying/scanning at most 917504 bytes. Publication
alone commits parser metadata. PUT advances do not extend a captured part or
re-plan the descriptor in progress. Ten parts bound a whole descriptor, not a
single turn. No timer or deadline determines progress.

CODE READ: `user_ring_arm` re-rings `Again`. `Plane::worker_pass` may serve a hot
token up to `REACT_ROUNDS + 1 = 9` times before republishing and moving on, so a
worker pass can copy at most nine parts per visited relay, even with continuous
PUT advances. The original H-fair wording (one part per *pass*) was too strong;
the actual bound is one part per Pump turn, nine per pass. This is the existing
structural per-visit bound, retained by Phase 2, not a new retry policy.

CODE READ: `worker::run_with_relay` runs `on_tick`, reads the wake sequence,
serves the bounded token pass, drains ready fds without waiting, dispatches
completion/other events, evaluates recorded relay waits, then decides whether
another iteration or a legal park is needed. Busy work cannot skip fd draining
or wait evaluation. Source-order pins and the existing `relay_fairness` model
check this order. INFERRED H-fair-2: two ready relays progress within K=1 passes,
with at most nine parts from a hot relay before the loop regains control.
FALSIFIER: the production Plane/Relay fixture exceeds either bound while the
first relay's PUT keeps advancing; removing the part-return bound must fail it.

## Ring arithmetic and characterization

CODE READ: production `Relay::put_step` uses `(put - cursor) mod N` for pending
entries (wrapping subtraction and a power-of-two mask). Each turn loads only
`cursor`, then computes `next_get = (cursor + 1) mod N`. A range spanning the ring
end is copied as tail slots then head slots; the PB VA/length is independent of
the GP ring and is never wrapped or split. Ring birth admits powers of two up to
2^20 entries, with 8-byte alignment; no individual GP word straddles that end.

CODE READ: the current production contract reserves one slot. `put == cursor` means no unrelayed
entry; `put == visible GP_GET` means producer-empty. Full is
`(put + 1) mod N == visible GP_GET`, capacity N-1. Equality after an attempted
N-slot producer lap is indistinguishable from empty: there is no wrap/phase bit
in the index. It cannot authorize an invented N-entry copy. A hostile producer
may overwrite its own unconsumed slots but gains no host credit or new authority.
The private `Fetch` ring also reserves one slot, retains monotone issued/fetched
counts and refuses GET outside its ring or beyond issued work.

INFERRED H-wrap, stated before characterization: these cases preserve order and
bounded copies on master, Phase 1 and Phase 2. FALSIFIER: any wrap/equal/full/u32
case overreads, loses a legal entry, duplicates it, panics or invents a lap.
Tests are characterization, not EXPECTED RED unless an actual defect is found.
New guest-derived arithmetic uses checked operations and `.get()`; the qemu
adapter's inherited deny set and `user_chunk`'s deny set remain active.

MEASURED: the six `wrap_characterization_*` tests in
`tests/user_turn_unit.rs` pass on master `e9c6d39e5` and Phase 1 `468a35c9d`
(6 passed, 0 failed on each, 2026-10-22). These cases found no defect, so this
characterization commit is green. Phase 2 must retain the same assertions.

MEASURED (G1 validation, 2026-10-22): Phase 2 retains all six wrap assertions.
The actual Plane/Relay fairness tests pass with K=1, including reversed token
order; removing the per-turn part bound fails both tests. All fourteen source
mutations have at least two independent killers. Full suites pass 1087/0
(four existing ignores), clippy adds zero emissions/sites/errors and the
repository claims gate reports new=0. The complete commands, mutation table and
hardware handoff are in [the Phase 2 proof](../../traces/descriptor_bound_20261018/phase2.md).
