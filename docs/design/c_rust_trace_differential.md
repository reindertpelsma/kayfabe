# The C↔Rust trace differential — decision, 2026-07-29

> **Status: DECIDED (owner, 2026-07-29).** Owner's framing: *"in the C we always compared
> real host to emulated host. now we can also trace C against rust because the llm app
> worked end to end in C. the invariant of two virtualizors can be compared to know exactly
> which thing nvidia chokes on for rust but not C."*
>
> Every "exists"/"missing" claim below was measured against the tree on 2026-07-29 and
> carries the command that established it.

## 0. The decision

The C artifact is **not** history. It is a second implementation of the same contract, and
the only one a real NVIDIA driver has ever accepted end-to-end (Qwen2.5-7B at 63 t/s,
`llm_7b_inference_done`). It is therefore promoted to a **standing oracle**, and the
C↔Rust trace differential becomes **rung zero of the bring-up ladder**
(`host_execution_plane.md` §4) rather than a later tool.

## 1. ★★ Why this is stronger than the C's own oracle

The C could only compare *emulated host* against *real host* — a comparison that answers
"is this reply plausible?" but not "which reply broke the guest?".

Two virtualizers facing the **same guest driver** answer a sharper question. Both see
identical guest MMIO **iff** they behave identically, so:

> if the guest's access sequence diverges at access *N*, our reply to *N−1* differed.

That localises a wedge to **one message** instead of a layer. Which matters because this
project's characteristic failure is *silence* — a hang at first doorbell, no error, no log.
The differential converts "debug a hang" into "read line 4,182 of a diff". ★ Note the
direction: **diff the guest's behaviour, not ours.** Ours is the input; the guest is the
detector.

## 2. What already exists — verified, not assumed

The design anticipated this; the mechanism is largely built.

| Piece | State | Evidence |
|---|---|---|
| Stream format spec | **specified** | `mode2_gsp_port_plan.md` §6 — one interleaved stream, totally ordered by a single counter, MMIO reads *with the value served*, guest-RAM reads *with the bytes returned*, IRQ, clock |
| Decoded-projection rule | **specified** | §6.3 — diff the projection, **never raw bytes**, explicitly so it does not enshrine the C's zero padding, uninitialised element tails, or `rpc.length = 36` for a 32-byte header |
| `TraceEvent` vocabulary | **built** | `kayfabe-trace/src/event.rs` — wire plane (MMIO/RAM/IRQ/clock) + decision planes (rmgraph apply, routing, address bind/resolve, doorbell dispatch, completion, isolate verb) |
| `diff()`, `check_dense_order()` | **built** | `kayfabe-trace/src/replay.rs`; positional, sequence-number-blind, so two recorders are comparable |
| MUST-DIFFER ledger | **populated** | §6.3 — 9 entries `GSP-D1..D9`, each carrying `c_site`, `c_behaviour`, `our_behaviour`, `guest_visible_consequence`, `independent_oracle`; *"it is cleaner" is NOT admissible* |
| Negative-trace class | **specified** | §6.3 — replay the stale-state bring-up that produced 508 lines of `-> echo NV_OK`, assert **exactly one** `Refused(QueueNotBound)` and **zero** `ElementPosted` |
| Per-row bite check | **required** | every MUST-DIFFER row must turn red when reverted to the C's behaviour |

★ The ledger is what makes the diff *readable*. A raw C↔Rust diff shows divergences that
are **correct** — the C's promote-ctx handler alone has seven named defects
(`c_bug_regression_matrix.md`). Without an enumerated admissible-divergence set the
differential is noise; with it, every unexplained divergence is a finding.

## 3. What is missing — three items, measured

1. **No C-side recorder emitting the §6 stream.** The C logs *"every access (offset, size,
   value, R/W) — the ground-truth trace"* (`C: nvkvm_gpu_emul.c:12`) plus an `m2_trace`
   mode, and `tests/mode2/` has `ioctl_trace.c` and ad-hoc `*_trace.sh` — but nothing emits
   the ordered, value-carrying stream §6 specifies. **This is the gap.**
2. **The planes do not emit.** `grep -rn '\.emit('` over `crates/*/src` returns hits in
   `kayfabe-trace` only (3 sites). The vocabulary exists and is unwired; today it is driven
   from the conformance suite's seam observer, not from production call sites.
3. **The ledger has no recorded trace to run against.** §6's S5 row states it plainly:
   *"trace replay harness — **no** (needs recorded traces)"*.

## 4. ★★★ The finding that changes the order — the oracle is PERISHABLE

Items 2 and 3 are pure work: they need no hardware, no bench, no driver, and can be done at
any time by anyone. **Item 1 cannot.** Recording a C trace requires a *running C
deployment*: built QEMU, booted guest, kernel pinned to 6.8.0-117
(`mode2_guest_kernel_pin`), the GA106 VBIOS, and a host driver at 580.159.04.

**That deployment does not currently exist.** Measured 2026-07-29: the live bench (`vh`,
RTX 3060 = GA106, the target part) carries `/root/kayfabe` and **no C tree**. The bench that
had one died earlier in this session.

It decays further with time, not less:

- vast hosts churn — one already died mid-session;
- driver versions drift, and `ogkm_is_versioned` records that the vendored tree (610.43.02)
  and the bench (580.159.04) **already disagree** on the GSP queue;
- the guest kernel pin is a patched-module vermagic, i.e. it gets harder to reconstitute,
  not easier.

⇒ **Recorded traces are the durable artifact; a bootable C on a rented box is not.** Capture
must be treated as a perishable input and scheduled accordingly — not deferred until the
Rust needs it, because at that moment the C may no longer boot.

## 5. The order this implies

Unchanged for the Rust build queue (`host_execution_plane.md` §3), with one **parallel**
track added, because it competes for no shared resource — different repo, different tree,
and it can run on a second box:

- **Track A (unchanged):** deterministic blocking mock → real isolate → L2-Q.
- **Track B (new, time-boxed, do early):** redeploy the C on a GA106 bench; teach it to emit
  the §6 stream; capture the bring-up and the LLM run; **commit the traces**. Then Track A
  gains an oracle it can use forever, with no live bench required.

Thread the plane emit sites (§3 item 2) whenever convenient — it gates nothing and blocks on
nothing.

★ `#47` (*kayfabe-gsp: trace-replay harness + fake-boot FSM*) was recorded as blocked on
`#46` (*kayfabe-abi codegen*). **`#46` is complete, so `#47` is unblocked** and was mis-parked;
its true remaining dependency is Track B's recorded traces.

## 6. Limits — what this oracle does NOT establish

- **One point on each axis.** The C's traces are one GPU (GA106), one host driver
  (580.159.04), one guest driver, one guest OS, one workload. The four axes of variation are
  untouched by it; divergences off that path are invisible to it.
- **The C is a reference, not a correctness oracle.** Matching it is evidence, not proof —
  it shipped the nine `GSP-D*` divergences and the promote-ctx defects. Where the ledger says
  we differ, *the C is the thing that is wrong*.
- **It cannot cover what the recording did not exercise** — §6's S5 row names exactly this as
  its residual risk.
- It says nothing about the host execution plane, which is a separate ★★★ gap
  (`host_execution_plane.md` §0).
