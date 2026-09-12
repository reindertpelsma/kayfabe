# w488 — memslot churn is NOT what the millisecond traps wait on

**STATUS: LIVE, 2026-09-12.** One binary, two arms, both `W392D_GUEST_OUTCOME=(P)`.

## The experiment

`bar1-passthrough=off,bar2-passthrough=off` removes the BAR mirror and therefore **every**
memslot fill a boot performs. If memslot updates — which take the kernel's `slots_lock` and
force a slot-table rebuild — were what the traps waited on, slow traps had to collapse.

| | passthrough **on** | passthrough **off** |
|---|---|---|
| mirror fills | **4188** | **0** |
| `slow_traps(>1000us)` | 245 | **1823** |
| `worst_trap` | 26 156 us @ `bar0+0xb81208` | 25 005 us @ `bar0+0xb81600` |
| 1-10 ms / 10-100 ms | 233 / 12 | 1605 / **218** |
| `off_trap_claims` | 668 | 668 |
| `inline_exceptions` | 2 | 2 |
| client | (P) | (P) |

## The verdict

★ **Refuted.** Removing ~90 % of the memslot operations made slow traps **7.4x worse** and
left the worst case unchanged. The extra slow traps in the `off` arm are volume — BAR1 exits
return (~88k) and more exits means more slow ones — but the *worst* trap does not care either
way.

⊘ This was the fifth hypothesis to die tonight. The others: the big lock was throttling the
guest (it was a stale atomic mirror); the mirror walk was over tens of thousands of slots (62);
PRAMIN is unused in the open driver (it bootstraps BAR2); inline host verbs were what
`bar0+0x110094` waited on (a pre-registered falsifier fired — that register got slightly
*worse*).

## What survives, and it is the useful part

- The worst trap is **~25 ms in both arms**, at an **interrupt-tree register**, with
  `off_trap_claims` and `inline_exceptions` **identical** on both sides. Our own deferral is
  the same and the mirror is irrelevant to it.
- `bar0+0x110094` has **no decode arm anywhere in this tree** and still measures milliseconds.
  Code that does not exist cannot be slow ⇒ **the vCPU is waiting, not executing.**
- `kayfabe-vmm-qemu` contains **zero** `bql_lock` calls by design (`lib.rs:99-104`) and never
  mutates the hypervisor's region tree, so the QEMU global lock is not the waiter either.
- Both arms run **identical verb plans**: 670 plans, `JoinFbLeaf n=402`, `Release n=131`
  (worst **235 ms**), `ChannelBirth n=12`, **1.6-2.1 s** of host verb time per boot, all on
  the worker.

## The next thing to test — and it is the sixth hypothesis, so it gets tested, not assumed

If the worker holds a lock across any of that verb work which the **vCPU trap path also
takes**, every vCPU stalls — and **the R1 witness would not see it**. `lockwitness.rs` states
in its own module doc that it watches only **ranked** locks and that a bare `Mutex` is
invisible to it. An unranked lock held across a host RM verb is therefore invisible to every
assertion in the tree and would produce exactly this symptom.

⚠ Do not act on this before the lock intersection is actually read. Five have already died.
