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
the only one a real NVIDIA driver has ever accepted end-to-end. It is therefore promoted to a **standing oracle**, and the
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

## 5a. ★★★ MEASURED LIMITS — what a C recording can and cannot witness

An instrumentation audit of `nvkvm_gpu_emul.c` at HEAD (`2899a74`) found four limits that
narrow this oracle sharply. They are recorded here **before** the capture, because each one
is a place a green diff would mean nothing.

**L1 — the completion plane has NO C oracle at all.** `CompletionOp::Observed` is
unproducible: the C never observes a host completion source, it **forges** completions —
`nvkvm_chan_sem_wr32` (`C:5278`), the finishPayload forge (`C:4093-4097`), the `0xFFF508`
guest-kernel backdoor (`C:3545-3587`). A grep over `nvkvm_isolate_*(` finds 17 call sites
and **zero** poll/event verbs. ⇒ **The entire completion-source plane — precisely what L1-M1
is being built around — is invisible to this differential.** A green diff says nothing about
it. This is the single biggest blind spot and the likeliest source of false confidence.

**L2 — the diff will NEVER be green end-to-end, and a green diff would itself be the bug.**
The C has no refusal vocabulary; `nvkvm_m3_service_cmdq` echoes `NV_OK` for essentially
everything (`C:2417-2419`). So `Outcome::Refused` is unproducible from the C, and **every**
MUST-DIFFER row is a position where the C emits a positive event and the Rust emits a
`Refused`. The ledger is not a footnote to the diff — it is the only thing that makes the
diff readable at all.

**L3 — any forwarding-mode trace is non-hermetic BY CONSTRUCTION.** With `m2fwd=on` (the
default, `C:9567`) `nvkvm_m2_share_guest_ram` (`C:6358-6394`) `MAP_FIXED`s the whole guest-RAM
memfd into the stub, and the **host GPU DMAs into guest RAM directly** — bytes that are
guest-visible and pass through neither `nvkvm_dmaw` nor any QEMU path. Replaying such a trace
cannot reproduce the guest's view. This is a stronger limitation than §6.1 acknowledges.

**L4 — the archived logs are UNUSABLE for this; capture must be a fresh instrumented run.**
`s->access_count` is incremented *inside* the log statement (`C:1525`, `C:4309`), so it counts
**logged BAR0 accesses only**; DMA reads/writes, IRQ raises and RPC replies carry no sequence
number and interleave by wall-clock arrival. The ordering information was never recorded.
(Three further defects are why the recorder cannot simply reuse the existing logging: the
BAR0 read trace has **three early returns that bypass it entirely** — PROM/VBIOS and the two
`LINK_*` registers, `C:1500-1515` — so ~1 MiB of streamed VBIOS and the value gating
`UVM_REGISTER_GPU` have never appeared in any C log; `nvkvm_bar0_write` logs itself **last**,
after all side effects, inverting causality; and it is **re-entered internally** with a
fabricated doorbell at `C:3366-3370` that the guest never wrote.)

★ **The good news, and it is load-bearing:** the single-counter total order is *sound*.
`qemu_thread_create|pthread_create|qemu_bh_new|timer_new` return **zero hits** in the device;
all four `MemoryRegionOps` keep `global_locking = true`; the isolate reader thread never
touches `NvkvmGpuEmul`. A plain non-atomic `uint64_t` in the device struct is a genuine total
order — no lock, no atomic. This upgrades the port plan's `[inferred] I2` to **evidenced**.

⇒ **Consequence for the capture order:** the hermetic cold bring-up (`m2fwd=off`,
`m2romregs=off`, BAR0, full mask) is the one trace a replay can be *closed* over, it is small,
and it is exactly what `kayfabe-gsp` S5 needs. **Take it first.** Forwarding-mode traces are
worth capturing for their *decision* planes, not for replayability, and must be marked
non-hermetic in their header.

## 6. Limits — what this oracle does NOT establish

- **One point on each axis.** The C's traces are one GPU (GA106), one host driver
  (580.159.04), one guest driver, one guest OS, one workload. The four axes of variation are
  untouched by it; divergences off that path are invisible to it.
- **The C is a reference, not a correctness oracle.** Matching it is evidence, not proof —
  it shipped the nine `GSP-D*` divergences and the promote-ctx defects. Where the ledger says
  we differ, *the C is the thing that is wrong*.
- ★ **What the C actually proved in MODE 2, stated correctly.** Mode-2's LLM result is
  **49.9 tok/s on a ~0.5 B Qwen2 (469 MB GGUF)** against a **47.5 t/s host-native ceiling on
  the same RTX 3050** — i.e. parity, zero overhead bare-metal (`mode2_baremetal_32`). The
  **63 t/s figure is MODE 1**, on a faster RTX 3060, and the **Qwen2.5-7B** run is likewise
  **Mode 1**, at ~20 tok/s (`llm_7b_inference_done`). `mode2_baremetal_32` flags this exact
  conflation as apples-to-oranges. Do not cite the 7B or the 63 t/s as Mode-2 evidence.
- ★ **A stock guest is evidenced for matmul, NOT for the LLM.** At emulator source `862c7c2`
  a stock, unpatched 580.159.04-open guest passed `cup2`, `cupctx2_min` (#12), `cup8`
  (2048² matmul byte-exact, `bad=0 maxerr=0`) and `cup8_iter` (#13, 5/5). The LLM and PyTorch
  were **not** re-validated stock — they were lost with the original overlay and never
  re-staged. "Stock guest → working LLM" is therefore **unevidenced**, and is the first novel
  risk any capture campaign meets.
- **It cannot cover what the recording did not exercise** — §6's S5 row names exactly this as
  its residual risk.
- It says nothing about the host execution plane, which is a separate ★★★ gap
  (`host_execution_plane.md` §0).
