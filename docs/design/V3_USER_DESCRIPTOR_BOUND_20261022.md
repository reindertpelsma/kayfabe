# Oversized USER descriptors: admission and continuation review

**STATUS: RESEARCH / DESIGN-ONLY, 2026-10-22. G1 stopped before implementation.**

**SUPERSEDED, 2026-10-22:** CODE READ: the authoritative
[descriptor decision](V3_USER_DESCRIPTOR_DECISION_20261022.md) resolves this stop.
Pointer admission uses the hardware format and forwards the intact word; the
917504-byte bound applies to copying/staging. The complete-mode-from-birth
recommendation and proposed eight-part ceiling below were rejected. This note
retains the earlier analysis as history; implementation follows the decision.

CODE READ: this note describes T1 base `77ed23eff`, not an implemented fix.
INFERRED: the current Pointer contract creates an unresolved design choice for
the requested authored continuation. Per the lane's instruction to stop and
record options rather than improvise a design change, no runtime change or
test-only EXPECTED RED commit was made. This is not a claim that a safe fix is
impossible. The requested implementation, mutation checks and GPU-free gates
remain outstanding.

## Current semantics

CODE READ:

| Site | Current contract |
|---|---|
| `kf-chan/src/user_storage.rs:18-24` | One 1 MiB private object; PB staging ends at `GP_OFFSET = 0xe0000`. `MAX_SOURCE_BYTES = GP_OFFSET = 917504`. This is allocation policy, not the public FIFO length maximum. |
| `kf-qemu/src/userread.rs:15-19,257-270` | `MAX_BYTES` is that same limit. `snapshot` refuses zero or larger reads; at most 225 placement pieces. Store preparation runs outside the row/RAM guards; copy rechecks the epoch, rows and RAM generation. |
| `kf-qemu/src/userpump.rs:707-708` | The production `Io::max_entry_bytes` returns `userread::MAX_BYTES`. |
| `kf-chan/src/user_stream.rs:43-64` | Recognized CONTROL NOP (including SYNC/LEVEL) fetches no PB. Otherwise decode a direct C56F descriptor; length above the trusted budget returns `Budget`; an unrepresentable end returns `Entry`. This precedes Pointer/Scan selection. |
| `kf-chan/src/user_turn.rs:677-712` | `plan_step` applies that check before software lookup/lifecycle dispatch. Refusal increments the entry-refusal counter and returns `Fail::Refused` with the captured word/cursor/PUT. |
| `kf-chan/src/user_turn.rs:509-517` | A refused step sets the core's `dead` and invalidates lazy state. |
| `kf-qemu/src/userpump.rs:2863-2871` | Production increments `poisoned`, logs no completion/reuse credit, sets `Record::dead`, and calls `Cell::cancel`. |
| `kf-qemu/src/userrelay.rs:145-150` | Cancellation clears retry, waiting and unannounced obligations and marks the cell canceled. It does not complete GPU work. |
| `kf-chan/src/user_storage.rs:522-530` | `write_user_gp` independently applies the same budget before writing a direct descriptor. Changing only admission would still fail here. |

CODE READ: GP acquisition is one owned eight-byte snapshot; `load_dword`'s low
and high results come from `aux.entry`, not two independent guest reads
(`userpump.rs:746-790`). Snapshot admission rechecks mapping identity/topology,
not equality of current guest bytes. It does not promise an atomic guest write
or an immutable live pushbuffer. Any hostile-mutation test must distinguish
captured bytes, mapping changes and later guest writes.

CODE READ: a true zero-length CONTROL NOP is a native control operation, not a
CPU completion. Other zero-length encodings still require format validation.
`user_batch::MAX_OPS` is a separate prototype plan budget; it is not this
production `USER_RING_GP_FORMAT Budget` path.

MEASURED in T1's preserved artifacts: the first refusal word
`0x0faa26023110102c` describes 1026596 bytes, 109092 above the staging limit;
the twelve refusal lengths span 947340..1026596. CODE READ of T1's report:
the failing relay was Pointer mode. INFERRED: the admission defect is a leading
TDR explanation, not proof of exclusive causality or a repaired workload.

## Design question that stopped implementation

INFERRED hypothesis H1, stated before tracing: the staging capacity is used as
a whole-descriptor limit before mode selection. FALSIFIER: the production path
already chunks oversized entries or supplies another limit. CODE READ result:
the table above supports H1; neither exception exists in the inspected path.

INFERRED hypothesis H2, stated before the follow-up: safe segmentation from
Pointer needs parser history that mode does not retain. FALSIFIER: Pointer
retains sufficient packet state, or arbitrary splits preserve the public
segment contract. CODE READ result: `Lifecycle::entry` says a Window cannot
start inside a submission because packet state is unknown
(`user_lifecycle.rs:861-863`); Pointer returns `Plan::Forward`, and reading a
PB there is a counted violation (`:987-1004`). `Scan::fresh_resume` explicitly
forgets pending packet state (`user_scan.rs:138-148`).

CODE READ: `END_PB_SEGMENT` stops interpretation of the rest of its original
segment (`user_scan.rs:215-216`), with the existing test
`end_pb_segment_does_not_interpret_ignored_suffix` (`:696`). INFERRED: splitting
the native descriptor after that instruction could execute a previously ignored
suffix. Starting a new scan at an arbitrary Pointer descriptor could instead
interpret carried argument words as headers. Neither is a valid continuation.
This is a source-level finding; the existing test was read, not run by G1.

CODE READ: the current rare-path staging also assumes a complete bounded
snapshot. It inserts barriers/markers only around its planned software actions,
and has a special contiguous remainder for channel updates inside front-end
atomic sequences (`userpump.rs:438-461,2212-2396`). INFERRED: recycling the
single staging arena at an arbitrary byte boundary requires a new proof: GP_GET
is not execution/byte-consumption proof, and an inserted witness may enter an
open packet or split an atomic sequence. A timer or GP_PUT poll cannot supply
the missing parser or lifetime proof.

INFERRED: these facts reject a naive chunking patch; they do not rule out a
carefully designed continuation. G1 did not establish a complete design within
the instruction to change nothing else in the relay/worker, so stopped instead
of silently widening that scope or presenting a limit increase as the fix.

## Options for the owner

INFERRED, proposed options, none implemented:

1. **Recommend: expand the design to an authored complete mode with parser
   continuity from birth.** Preserve the existing per-read/per-staging-segment
   bound. Specify a fixed, fallibly allocated set of read-only staging segments
   and a VM-wide resource budget before coding. Pre-stage a complete bounded
   descriptor before publication where an open packet or atomic sequence
   prevents safe recycling. This requires reviewing lifecycle and staging
   ownership as part of the fix, rather than adding only a size branch.
2. **Keep native Pointer semantics and design a separate descriptor admission
   bound.** Public direct descriptors could remain intact in the native USER
   VAS, without new PB reads, while Scan uses bounded continuation. This needs
   an explicit determination that intact native references meet the lane's
   authored-continuation requirement. G1 did not assume that determination or
   implement a larger admission constant.
3. **Keep the current refusal while the complete-mode contract is reviewed.**
   The observed admission failure remains; this is a stop disposition, not a
   compatibility fix. No timer, polling change or boot flag is proposed.

## Required continuation contract for option 1

INFERRED, proposed acceptance requirements; not an approved final design:

- **Bounds:** keep each source read and each staging span at or below 917504
  bytes. A possible resource-derived hard ceiling is eight such segments,
  7340032 source bytes, independent of the observed workload sizes. A valid
  direct descriptor above it must refuse before reading its PB or publishing
  any part, stop only its relay, and grant no completion/reuse credit. This is
  a candidate budget, not a changed constant or a compatibility guarantee.
  Allocation failure must also refuse without partial publication.
- **CPU and memory:** at most eight bounded source copies and a fixed number
  of linear validation/authoring passes per admitted descriptor; never retry
  until the guest becomes stable. Account explicitly for immutable source,
  parsed/rewritten words, staging storage, mapping receipts and the channel/VM
  cap. Eight staging spans alone are at most 7340032 bytes; total memory is
  larger and remains an unresolved design item. No unbounded allocation,
  traversal, emit, retry or spin is acceptable.
- **Ordering:** retain original descriptor order, packet argument state,
  bindings, subdevice masks, END semantics, SYNC_WAIT and LEVEL semantics,
  software-trigger order, atomic sequences and source-generation identity.
  A segment boundary creates neither a new guest entry nor a completion.
  Author output only from the exact owned bytes that validation accepted;
  source mutation cannot change an already authored command or software act.
- **Publication:** initialize every span before publishing the corresponding
  host PUT. No host marker may enter a carried packet. Never overwrite staging
  using GP_GET alone as proof of consumption. Keep all lifetime and native
  execution witnesses required by the existing path; never forge completion.
  The final original-descriptor receipt must not let the guest recycle bytes
  that a later continuation has not captured. Distinguish source reuse from
  GPU completion and from staging reuse.
- **Doorbells:** preserve one durable original-descriptor obligation across
  continuations, worker turns and credit waits. Resume from that state using
  existing events. Original guest GP_PUT exposes the complete original entry;
  internal segmentation must not require another guest PUT or doorbell. A
  doorbell is not a packet boundary, execution witness or authority hint.
  No GP_PUT poll or elapsed-time decision is part of correctness.
- **Execution context:** worker-only work and mapping preparation; no vCPU
  waits, host calls or PB parsing, and no blocking under a vCPU-taken lock.
  No required boot flag, no new physical operands or writable private GPU maps.

## Tests and hardware handoff still owed

INFERRED: before any fix, commit EXPECTED RED tests alone using the production
entry path and real snapshot/publication seams, not only a parser model. Cover
1026596 bytes, the other eleven observed sizes, hard-ceiling and ceiling+4,
captured-descriptor mutation, mapping replacement, zero-length native NOP and
invalid control forms. Cover packet/trigger boundaries at a staging cut,
END with an ignored suffix, carried packets from preceding descriptors,
SYNC_WAIT, LEVEL, atomic sequences, partial publication, backpressure and
source migration. Each behavior mutation needs at least two independent killers.

INFERRED: hardware alone can confirm exact-head catia-07 subtest
`03_ThreeLoftJetsDarkMirror` on the run-3440 configuration, preceded by the
whole catia viewset; stable real descriptors, native consumption and interrupt
ordering, application survival, guest event 153/4101 and host Xids, and whether
the other reset without a Budget refusal persists. No such result is claimed.
T1's matched VFIO comparison remains unavailable. GPU-free suites and clippy
cannot establish a repaired TDR or GPU performance.

MEASURED: G1 made no runtime edits, ran no cargo build, tests, clippy or mutants,
and used no VM/GPU/host resource. Product before/after test counts are 0/0
(not run), not a passing suite. See the small
[lane disposition](../../traces/descriptor_bound_20261018/README.md).
