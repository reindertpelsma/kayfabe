# Recovered owner directives: what was kept and what was dropped (2026-10-08)

**STATUS: LIVE, 2026-10-08 (audit table).** The record of one filtering pass over the directives
recovered by the earlier branch `docs/owner-directives-20261007` (commits `d548a7a8`, `49b99d1e`: a
156-line §T in `docs/OWNER_RULINGS.md` and an edit to `docs/design/THE_CONSTRAINTS.md`). The owner's
instruction, 2026-10-08 (`docs/OWNER_RULINGS.md` §W): *"§T (recovered directives): later always wins
from recovered, only what's useful and non conflicting. Be very careful with directions during v1/v2
kayfabe architecture, I would rather avoid them."* The kept text is §T in `docs/OWNER_RULINGS.md`.
This file lets the owner audit what was dropped. Nothing here is a ruling.

## The filter

A directive is **KEPT** only if all three hold: it is still true for v3, it is not stated elsewhere
already, and an agent working today can use it. It is **DROPPED** if any of these holds:

- it conflicts with a later or current ruling (`docs/OWNER_RULINGS.md` §A-§W), with the v3
  architecture (`ARCHITECTURE.md`, `docs/design/THE_ARCHITECTURE_v3.md`,
  `docs/design/the_three_channel_kinds.md`) or with `CLAUDE.md`;
- its subject is the v1/v2 kayfabe architecture (the pre-v3 `kayfabe-core`/`-fwd`/`-rt` tree,
  isolates, the scratchpad, the old executor and mode-2 designs, or anything that only makes sense
  in that tree);
- it duplicates a statement already in the repo;
- the filter cannot tell (when in doubt, drop).

**Dating rule used.** `docs/design/THE_ARCHITECTURE_v3.md` is dated 2026-09-20 (w819). A directive
the owner gave before that date was given under the v1/v2 architecture. It was dropped when its
content depends on that architecture or on a design a later document replaced, or when the filter
could not tell. It was kept only when the subject is not the architecture at all. Nothing dated before
2026-09-20 was kept, so every entry kept below is from 2026-10-01 or later.

**Counts.** 13 recovered entries plus the `THE_CONSTRAINTS.md` edit. Of the 13 entries, 4 are kept
(T.1 in part, T.2, T.3, T.11, and T.12 restored 2026-10-08) and 8 are dropped (T.4 to T.10, T.13). The `THE_CONSTRAINTS.md`
edit (three supersession marks) is dropped. The table has 20 rows (entries split where they hold separate directives): 4 kept, 16 dropped.

## The table

Row numbers are the original entry numbers of the recovered §T.

| # | Directive (short) | Source dates | Verdict | Reason |
|---|---|---|---|---|
| T.1a | 172.22.1.20 (RTX 4070) is trusted hardware; the 2026-10-04 rules: destructive VFIO and display changes need no permission, no firmware changes or bricking | 2026-10-04, correction 2026-10-07 | **KEEP** | Dated within v3; no later ruling or v3 document states it; useful. Its ⚠ about "temporarily borrow" is kept as an open point for the owner (see Unsure, below). |
| T.1b | Kiosk PCs (172.18.30.21-32): not trusted, no keys on them, leave them booting | 2026-08-31 | DROP | v1/v2-era bench rule (2026-08-31). The kiosk machines' addresses are not recorded elsewhere in the repo (`docs/STATUS_AND_HANDOFF.md`), so the filter cannot tell it applies. The general "untrusted, no secrets" is already `OWNER_RULINGS.md` §F and `scripts/bench/box/README.md`. |
| T.1c | Vast boxes: absolutely not trusted | 2026-10-07 | DROP | Duplicate of §F ("Boxes: untrusted ... no secrets") and `scripts/bench/box/README.md`. |
| T.2 | Outside repos are untrusted; clone them, do not web-fetch | 2026-10-01, 2026-10-04 | **KEEP** | v3-era; not stated elsewhere; useful. Its "do not build or run it" clause was cut from the How-to-apply, because the quotes support only "untrusted" and "clone, not web-fetch". |
| T.3 | Search for an existing solution before building one | 2026-10-04, 2026-10-05 | **KEEP** | v3-era; not stated elsewhere; useful. |
| T.4 | A doorbell is registered before it can be used (block the register RPC until the token is in the table; no vCPU blocking; zero refused doorbells in test runs) | 2026-09-11, 2026-09-14 | DROP | v1/v2-era doorbell-table design (the "refused doorbells" counter belongs to the old tree). v3 treats a ring of an unregistered token as a no-op: `THE_ARCHITECTURE_v3.md` §2 (the bit-31 paragraph), "It degenerates to the same no-op as any other unowned token". The two cannot be reconciled without a decision, so when in doubt: drop. The non-blocking half is already `CLAUDE.md` and §A.4. |
| T.5 | When a VA space may be freed (no mappings, no table reference, no channel uses it; VMM-coordinated) | 2026-09-18, 09-19 | DROP | Dated before the v3 architecture (2026-09-20). It is phrased in terms of the old VMM-coordinated per-process VA bookkeeping; v3 has a single VA-manager thread that does all mapping (`THE_ARCHITECTURE_v3.md` §1) and `V3_P5_PORT_MAP.md` item 10 covers VA-space retire and recycle. The filter cannot tell the v3 meaning. |
| T.6a | Rare paths are correct by construction, written against the spec | 2026-07-30 | DROP | v1/v2-era (nvkvm start). Its performance-versus-correctness stance is already `CLAUDE.md` ("Correctness before cost") and §A. |
| T.6b | Push back; the owner's ideas are brainstorms and may be replaced by the better version | 2026-08-11, 2026-09-06 | DROP | v1/v2-era. "You are free to do the better version" is in tension with §A.11 ("Don't silently change a constraint — tell the owner") and the `CLAUDE.md` falsifier rule. The filter cannot tell which governs, so it drops. |
| T.7a | One GPU UUID per GPU per VM, hash(vm id + host GPU UUID), settable by the user | 2026-08-08 | DROP | Superseded and duplicated: §V (2026-10-08) rules the UUID (`gpu-uuid=`, `vm-id=`), and `docs/design/V3_GPU_UUID.md` implements it. The recovered entry's facts ("synthetic today", "same UUID for the same chip row") are stale. |
| T.7b | The user may set the GPU name; hide or regenerate persistent hardware IDs | 2026-08-08 | DROP | The GPU-name part is already in `docs/design/V3_GPU_UUID.md` (line 124, "user-settable GPU NAME"). The ID-hiding part is an unverified 2026-08-08 (v1/v2-era) wish with no check behind it. |
| T.8 | Snapshot, pause/resume and live migration are future goals; do not design them out | 2026-09-04, 2026-09-13 | DROP | v1/v2-era (before 2026-09-20), and its examples (host twins, the single store, the missing reset path) are about the old tree. The filter cannot tell it still holds for v3. |
| T.9 | Write about licensing by mechanism and outcome, never "the licence not being paid" | 2026-09-14 | DROP | v1/v2-era. A later text, `docs/PRODUCT_POSITIONING.md` (2026-10-01, "after issue #1 and an owner discussion"), says "without passthrough or licensing". The filter cannot tell which the owner wants, so it drops. The recovered entry itself flagged that file for the owner. |
| T.10 | AMD is out of scope (separate kayfabe-amd project, deferred) | 2026-09-14, 2026-09-16 | DROP | v1/v2-era; stated in the context of an earlier project plan ("winapps-nviidia"). Its "build no vendor abstraction" is the original author's addition, stronger than the quotes. Dropped on the dating rule; the owner can re-rule it in one line. |
| T.11 | Docs for agents (verbose, step by step, rulings) are wanted; prominent human-facing docs are written for people | 2026-10-07 | **KEEP** | v3-era, not stated elsewhere, useful. Two clauses of the original How-to-apply ("states limitations up front instead of burying them", "does not read as AI-written") were cut, because the owner's quote does not say them. |
| T.12 | Rent only Vast "verified" hosts | 2026-07-28, 2026-07-30 | KEEP (restored 2026-10-08, owner-side review) | v1/v2-era (nvkvm start). The later box README's rent command (`vms_enabled=true ...`, `scripts/bench/box/README.md`) has no verified filter, and §C records VM offers by die, not by verification. The filter cannot tell it still holds. |
| T.13 | Isolates were removed on 2026-09-20 (the recorded owner quotes) | 2026-09-20 | DROP | Subject is the v1/v2 architecture (isolates). Duplicate: `THE_ARCHITECTURE_v3.md` §1 ("One process ... No isolate children") already records it with the owner's quote, and `THE_CONSTRAINTS.md` line 7-8 already says isolates and the scratchpad are deleted in v3. |
| C.3 | `THE_CONSTRAINTS.md` item 3 ("Multiple concurrent workers in isolates"): SUPERSEDED mark citing §T.13 | edit of 2026-10-07 | DROP | Isolates are the v1/v2 subject; the mark cites T.13, which is dropped. The file is not edited. |
| C.14 | `THE_CONSTRAINTS.md` item 14 ("Isolates can have multiple threads"): SUPERSEDED mark citing §T.13 | edit of 2026-10-07 | DROP | Same reason as C.3. |
| C.26 | `THE_CONSTRAINTS.md` item 26 (ownership split: the isolate borrows, the scratchpad holds): SUPERSEDED mark citing §T.13 | edit of 2026-10-07 | DROP | Same reason as C.3. |

`docs/design/THE_CONSTRAINTS.md` is therefore **unchanged** by this pass. Its three items stay as
they are on the integration branch; the file's own header (lines 7-8) already says the isolate and
scratchpad mechanisms are deleted in v3.

## Unsure

- **T.1a.** It is the only kept entry that grants permission (destructive changes without asking). It
  depends on the coordinator-relayed correction of 2026-10-07 and carries an unresolved conflict with
  the owner's own 2026-10-04 words ("I temporarily borrow this machine"). It was kept because the
  correction is the later statement and says the machine is trusted; the owner should confirm.
- **T.9, T.10 (and T.12, restored).** Their subjects are not the architecture. They were dropped on the dating rule
  and on a later text that may disagree. These are the three entries most likely to be wanted back.
- **Stale pointers.** `docs/STATUS_AND_HANDOFF.md` still lists "approval of §T" as an owner decision
  waiting and the old branch as left out. This pass does not edit that file.
