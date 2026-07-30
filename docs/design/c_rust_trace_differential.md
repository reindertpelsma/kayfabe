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

---

## 7. ★★★ BUILT AND RUN — the cap1 differential, 2026-07-29 (task #47)

`crates/kayfabe-crec` is the harness; `traces/cap1_coldboot_hermetic.rec` is the artifact,
committed uncompressed so the harness needs no decoder, no external binary and no
third-party crate. `cargo run -p kayfabe-crec --example cap1_report` prints the whole run;
`crates/kayfabe-crec/tests/{decoder_matches_reference,cap1_differential}.rs` and
`tests/tests/c_trace_differential.rs` assert it (24 tests, in the default suite, 1.5 s).

**The instrument was validated before the results were believed.** The Rust decoder is a
second implementation of the recorder's format, so it is pinned against the *first* —
`rec_dump.py`'s exact census, header counters, provenance offset and dense-order verdict.
Two instrument defects surfaced and were fixed before any divergence was interpreted: a
truncation assertion that had the wrong failure mode, and three wrong RPC function ids
caught by the `bench_abi() == P580.abi()` mechanism (`cap1` independently confirms
`gsp_rm_alloc = 103` — 45 of its 178 commands carry it).

### 7.1 What the replay does

One transaction = one MMIO record plus every guest-RAM access, IRQ raise and clock reading
it caused, up to the next MMIO record. A transaction is projected **only if its driving
access decodes to a `GspReg`**, symmetrically on both sides — `cap1` also contains the
channel/pushbuffer plane (`0xbb0090`, 66 records) and the CPU interrupt tree, which this
crate does not implement and reports as a number. Guest-RAM reads from unprojected
transactions are still installed into the oracle. Nothing is sampled. Both the global
positional diff and the per-transaction one go through `kayfabe_trace::diff`.

### 7.2 The result

**Within the oracle's reach — cold boot → bind → `GSP_INIT_DONE` → four RPC round-trips —
the Rust GSP reproduces the C exactly in decoded projection**: all 498 GSP register reads
in that span, the published status tx header, the write pointers and the command
read-pointer acknowledgements. Nine divergences, one of them a ledger row:

| | count | what |
|---|---|---|
| **GSP-D1** | 1 | found in the artifact without being told where: the C's `GSP_INIT_DONE` declares `rpc.length = 36` for a bare 32-byte header; we declare 32 |
| **F-2** | 1 | we publish a command read-pointer ack **on the bind**; the C waits for a doorbell. B4 drain-on-publish — and the capture *proves the premise*: the guest's own command tx header reads `writePtr = 2` at bind time |
| **F-3** | 5 | we ask the shell to announce the status queue; the C announces nothing, ever |
| **F-4** | 2 | two replies agree on every matched field and differ in the **body** — the C models fn 65 and fn 76 rather than echoing them |

`beyond the closure limit: 544`, counted and never interpreted.

### 7.3 ★★★ The structural finding — why `cap1` cannot be closed

**The C's guest-RAM read set is a strict subset of ours, in three independent places, and
each is one of the ledger's own rows.** A hermetic capture answers the reads its subject
made and no others, so the C's defects bound the differential's reach:

| row | the read the C never makes | consequence |
|---|---|---|
| **GSP-D8** | the region's own page table (`sharedMemPhysAddr`, 129 × 8 bytes) — the C computes `base + offset` instead | strict replay **stops at the bind** |
| **GSP-D2** | the peer's status-queue `readPtr` at `cmdQueueBase + rxHdrOff` — the C has no flow control. ★ It sits **one byte past** the end of the 32-byte tx-header read the C *does* make | every `post` |
| **GSP-D6** | continuation elements of a multi-element command — the C reads element 0 and advances past the rest | **the closure limit** |

The first two are reconstructible under assumptions the harness names out loud
(`ReconKind::{RegionPageTable, PeerStatusReadPtr}`) and counts. **The third is not.** The
run stops at the first multi-element command — record 141976, `GSP_RM_CONTROL`,
`rpc.length = 8276`, `elemCount = 3` — because the capture contains no observation of
command slots 7 and 8 while they were live, and the ring has since been rewritten. Every
later observation of slot 7 is >150 000 records away, i.e. a different generation.

⇒ **A capture of a defective implementation cannot fully close a replay of a correct one.**
Closing the remaining 173 doorbells needs a re-capture with the C patched to *read* the
continuation elements it already skips — a one-site change (`C:3341-3350`) that alters no
reply, only what the recorder witnesses.

### 7.4 ★★ Correction to §5a's limit 4

The pre-registered limit said *"the C raises SWGEN0 once for `INIT_DONE`"*. **It does not.**
`cap1` contains exactly one `IrqRaise` in 359 062 records and it is the driver's own
`INTR_LEAF_TRIGGER` self-test (`0xb81640 <- 129`, `_osVerifyInterrupts`), *not* a GSP
interrupt: `nvkvm_gsp_raise_swgen0` is reachable only from `nvkvm_gsp_deliver_events`,
which returns immediately with no os-event registered, and no CUDA process runs in `cap1`.
So the C posts **202 status elements and announces none of them**; the guest picks up
`GSP_INIT_DONE` and every RPC reply by polling, and writes `IRQSCLR` **zero** times.

Two consequences. This capture constrains the GSP interrupt plane **not at all** — an even
sharper statement of limit 1. And **F-3 is undecidable from it**: our `post` latches
`swgen0_pending`, which only an `IRQSCLR` write clears (E10), so we re-announce on every
service; whether a real guest clears the edge cannot be learned from a capture in which it
never received one.

### 7.5 What landed alongside

`kayfabe-crec::ga10x` is the **first non-fake `GspModel`** — the GA10x register map, every
constant carrying `ogkm-580`'s swref header or the C's arch header, and none of it read off
the capture. It is what makes the Axis-B seam's claim ("one FSM, several register models")
a measurement rather than a design note, and `tests/tests/c_trace_differential.rs` asserts
it shares no encoding with either fake model.

## §7 — cap1b measured: GSP-D6's capture gap is CLOSED, and the wall moved to GSP-D2

**[measured 2026-07-30]** The D6 witness patch (`C: nvkvm_gpu_emul.c` ~`:2578`, commit
`819282d` in the C repo) and its re-capture `cap1b_coldboot_hermetic_d6` were already
committed. Running the existing differential against cap1b via the `KAYFABE_C_TRACE_CAP1`
override — **no repo change, no bench, no GPU** — gives the like-for-like:

| `Fill::Reconstructed` | `cap1` (359 062 rec) | `cap1b` (360 725 rec) |
|---|---|---|
| closure limit | txn **978** | txn **1028** |
| max lookahead | 2373 records | **1035** |
| FSM refusals | 169 | 219 |
| divergences (beyond limit) | 553 (544) | 773 (688) |

★★★ **The headline is not the +50 transactions — it is the CHANGE OF KIND at the wall.**

On `cap1` the run died on a **missing observation**: txn 978 read
`gpa=0x127209000 len=4096 -> Unobserved`, a continuation element the C consumed without
ever reading. On `cap1b` **every read at the wall is `Observed`** — including all nine
continuations `0x127602000 … 0x127609000`. The witness patch did precisely what it was
written to do, and **GSP-D6 is no longer a closure limit.**

What stops it now is a **refusal by our own GSP**:

```
txn 1028  -> QueueFull { needed: 9, free: 1 }
txn 1030+ -> QueueFull { needed: 1, free: 0 }   (215 more)
```

### Why this is oracle blindness and NOT a Rust defect

`QueueFull` is status-queue flow control: our GSP will not post 9 elements into a ring it
believes has 1 slot free. `free` is derived from the **peer's status readPtr** — the pointer
the *guest* advances as it consumes status elements.

**The C never reads that pointer.** Verified in the source, not inferred: the only accesses
to the status queue header are **writes** — `nvkvm_dmaw(… q_stat_base + 16 …)` for our
writePtr (`C: :1783`) and `nvkvm_dmaw(… q_stat_base + 0x20 …)` for the command-readPtr echo
(`C: :3576`). That is ledger row **GSP-D2** exactly: *no flow control*. The C simply posts.

So the guest **is** advancing its readPtr (the boot succeeds, and the C posts 202 status
elements), our GSP is **right** to consult it, and **no capture taken from this C can ever
contain it** — the value passes through no recorder chokepoint. The replay is blind here by
construction, not wrong.

★ This is the fourth measured limit of §5a arriving in concrete form: *a capture of an
implementation that does not perform a read cannot close a replay of one that does.* D6 was
the first instance and it was fixable by witnessing. **D2 is the same shape and fixable the
same way.**

### The next witness patch, specified — and what NOT to guess

To move the limit past 1028, the C must **witness** the guest's status-queue readPtr so the
recorder sees it. Same discipline as D6: recorder-gated (`nvkvm_rec_on()`), a pure read of
guest RAM, bytes discarded, **no reply changed** — the divergence stays real, it merely
becomes observable.

Two things that make this **not** a copy of the D6 patch:

1. **It is not a one-time read.** The value is live — it changes as the guest consumes — so
   it must be witnessed **at each status post** (the path around `C: :1774-1783`), not once
   at init. Extending the 32-byte `txh[32]` init read at `C: :3659` would witness the wrong
   thing: that read is of the **command** queue header, taken once, at bind.
2. ⊘ **Do not guess the offset.** The status queue's RX header is located via `rxHdrOff`
   (`txh+24` in the header the C already logs); read the layout rather than assuming the
   readPtr sits at a fixed displacement from `q_stat_base`. A witness read at the wrong
   address would record a plausible number and silently validate a wrong flow-control model
   — worse than the blindness it replaces.

★★ **Do not treat `QueueFull` as a bug to be tuned away.** Relaxing our flow control to get
a greener diff would delete a correctness property that the oracle simply cannot see. The
diff is the instrument; the instrument does not get to edit the subject.
