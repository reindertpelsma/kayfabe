# The VMM executes emulated channels — through a raw client it owns

**STATUS: LIVE — written 2026-09-17 (w755p), from the owner's ruling of the same day.**
Supersedes nothing; it *replaces the executor* behind `DoorbellRoute::CpuCe` and changes no
routing. Constraint: **§37**. Related: §20 (which this must not reopen), §23, §35, §36, and
`route_k_phase_1_holds.md` (whose machinery this reuses wholesale).

---

## 0. The ruling, in the owner's words

> *"For emulated channels it executes in the worker in VMM, from there it can execute the raw
> client. I think raw client is better in VMM than executing in scratchpad isolate because
> **a)** it prevents a hop for that work to another process and **b)** with raw client you can
> obtain an eventfd and do the semaphore polling yourself, keeping the big loop in the worker,
> and still satisfying constraint that the worker cannot block without stopping accepting new
> input. This means the va space of scratchpad land in the VMM, which is also needed for the
> MMIO CPU maps to populate bar1/bar2 anyways."*

**(b) is the load-bearing half.** (a) is a latency argument and would not, on its own, justify
moving RM calls into the VMM. (b) is a *feasibility* argument: §35 forbids a worker from making
a blocking call that stops it accepting new input, and there are exactly two sanctioned blocking
points — the work mutex and `epoll`. A completion that lives in **another process** reaches this
worker as a **reply it must wait for**, which is neither of those. A completion that lives on an
**fd this process holds** goes into the worker's own `epoll` set and is. ⇒ Executing in the
scratchpad does not merely cost a hop; under §35 it has **no legal way to report completion**.

---

## 0.1 ⊘ THE DELTA IS THE RM CLIENT, NOT THE EXECUTION SITE — the first draft overstated it

It is easy to read this ruling as *"guest-derived execution moves into the VMM"*, and it is worth
being exact, because that reading would collide with §26's trust statement (*"the scratchpad does
hold guest RAM, and is trusted to because **it executes no guest-derived work**"*).

★ **The emulated CE already executes in the VMM today.** `route_of_engine` maps
`EngineKind::Ce → DoorbellRoute::CpuCe`, `shell_disposition` maps that to
`ShellDisposition::MayServeLocally` — *"the shell's own CPU copy-engine executor … the copy IS
the workload and its operands are in memory this process holds, so it can run here"* — and
`cpu_ce::execute_ours` runs in the VMM process.

⇒ §37 does **not** move execution into the VMM. Execution is already there. What the VMM gains is
an **RM client**, so that the executor already running there can hand a descriptor to real
hardware instead of moving bytes with the CPU at 48 MiB/s. That is a much narrower change than
*"the VMM starts running guest work"*, and it is the accurate one.

---

## 1. What this does NOT reopen — the §20 line, and it is the line to hold

§20 says the page-table walker runs in the **scratchpad isolate, never the VMM**, and its
argument (`THE_CONSTRAINTS.md`, *"Placement, and why it is not the VMM"*) is precise: the walker
**dereferences guest-authored pointers in a loop**, so it must run at the privilege of the data
it processes, where it *cannot escalate*.

That argument does **not** reach an emulated-channel submission, and the difference is structural
rather than a matter of degree:

| | the walker (§20) | an emulated CE/scrub submission (§37) |
|---|---|---|
| input | a guest-authored **tree**, of guest-chosen depth | a **fixed tuple** — `src`, `dst`, `len` |
| what the code does with it | **follows pointers** until a terminator it does not control | bounds-checks the tuple, then builds a descriptor |
| failure mode | out-of-bounds read that **does not reliably fault** (measured — §20's own note) | a refusal by name, before anything is built |
| ⇒ placement | scratchpad | VMM worker |

⇒ The test for *"may this run in the VMM?"* is **not** *"is the input guest-authored?"* — nearly
every input is — but **"does this code chase guest-authored structure?"**

⚠ **That boundary must be enforced by a gate, not by intent.** Everything the VMM's raw client
submits is built from a fixed number of already-validated fields. The day a loop over
guest-authored data appears on this path it has become §20's problem and belongs in the
scratchpad — and nothing about the code's *location* will announce that it crossed over.

---

## 2. Why the CPU executor was never viable under the single store — and this is not a preference

The emulated CE executes today on the **CPU**: `kayfabe_rt::cpu_ce::execute_ours` reads operands
through `ce.fb().read()`. Under §18 (one reserved device-local object is all of guest video
memory) an `FbStore` read of the store is a **CPU read of video memory**, and that is measured at
**~48 MiB/s** (`vidmem_cpu_reads_are_48_mibs_and_bar1_bounds_views.md`).

⇒ *"the emulated CE executes on the CPU"* and *"the store is the only memory"* are not two
independent design choices that happen to compose badly; the second makes the first unpayable.
This ruling is what lets the single store be paid for. ⊘ The CPU path is **kept as the bottom
rung** of the degradation ladder, in the same spirit as §20's *"every layer degrades into the one
below"* — it is correct, and on a scrub of a few KiB it is also fast enough.

### 2.1 ⊘ AUDITED — the CPU rung does NOT forge completions, so it is a safe bottom rung

The owner flagged this as the thing to check first: *"if it forges completion without actually
doing stuff then thats first to get working."* The worry is well-founded **about the C**, which
completes `finishPayload` for the kernel CeUtils channels because *"the scrub is a no-op for our
backing … complete now if no real work"* (`nvkvm_gpu_emul.c:4228`).

`[audited w755p]` **This tree does not do that.** There are exactly three sites that write a
completion, and each is downstream of the work:

| site | what precedes it |
|---|---|
| `ceutils.rs:1253` | a standalone release method — the launch it reports on retired in an earlier submission |
| `ceutils.rs:1352` | `execute_ours_spans` at `:1307`, **same loop iteration**, with `?` — a failed copy never reaches the release |
| `shim.rs:9504` | the **deferred** drain: the copy ran, and the release was withheld until the publication committed |

And the two guards that make it structural rather than incidental:
- §14.8's guard at `ceutils.rs:1300` refuses the whole submission by name (`CpuCeStraddle`) if
  **any** span is not `CeExecutor::Ours` — a partial copy cannot release a semaphore.
- A submission that decoded no launch is **refused, not completed**: *"A submission that decoded
  no launch moved no byte. It is not served."*
- ⊘ `PushMethod::SemRelease` is deliberately **not** acted on: it is the host-FIFO semaphore four
  bytes *below* the `finishPayload` the guest actually spins on, and advancing it *"would satisfy
  our own counters while the guest spins on the word above it."*

⇒ The C's forgery is the reason the C *"is no oracle at all for engine execution"*. Our CPU rung
is slow, not dishonest — so §37 may keep it as the degradation floor without keeping a lie.

---

## 3. The mechanism — and how much of it already exists

The parts, and their status **as measured in the tree on 2026-09-17**:

### 3.1 A raw RM client in the VMM — ★ the transport already transits the VMM

`RmBackend::mint_birth_client` returns `MintedBirthClient { client, isolate_client, ctl, node }`,
where `ctl` and `node` are **`OwnedFd`s** for `/dev/nvidiactl` and `/dev/nvidia<N>`, minted inside
an isolate and passed out over `SCM_RIGHTS`. The VMM **already receives both fds today** — 
`StoreMapPort::adopt_birth_client` (`crates/kayfabe-qemu-raw/src/storemap.rs`) takes
`minted.ctl` / `minted.node` and forwards them to the scratchpad.

⇒ The VMM is already on the delivery path for exactly the descriptors it needs. What is new is
that it **keeps a set** rather than only forwarding them. Nothing about the fd-passing has to be
invented.

⊘⊘⊘ **CORRECTED before building on it — the first draft of this line said V is minted BY THE
SCRATCHPAD, and that is exactly the mistake route K exists to prevent.** RM stamps `ProcessID`
from **the calling task** at client creation (`client.c:112`), which is why
`HostRmBackend::mint_birth_client` **refuses outright when the caller is the scratchpad**
(`rm.rs:7421`, `BIRTH_CLIENT_NOT_A_PER_PROC_ISOLATE`), with the comment *"every later stamp
would be wrong while every ioctl succeeded."*

⇒ **V is minted BY THE VMM, in the VMM**: open a second `/dev/nvidiactl`, `NV_ESC_REGISTER_FD`
the matching `/dev/nvidia<N>` onto it (without which every later escape answers `0x23
INVALID_CLIENT` — *"a refusal that reads like a permissions problem and is a missing binding"*),
then `OwnClient::allocate_root`. V is then **ours**, with our `ProcessID`, which is correct:
emulated channels belong to no guest process.

⚠ **This does extend F11's approved set, and that must be deliberate.** The standing rule is
*"a per-proc isolate may never name a foreign client; the scratchpad may, and only for a VA space
the VMM handed it."* Duping the **store** into V is the scratchpad naming a foreign client for a
**memory object**, which that sentence does not cover. ⇒ It needs a second arm on the same
newtype, argued and named — **not** a string added to an allowlist, and **not** slipped in under
the existing VA-space arm.

### 3.2 The store and the VA space, duped into V

Identical to route K's `birth_ranges`: the scratchpad dups the store object and the VA space into
V. The owner has already ruled the VA space is **shared**: *"a cuda channel not managed by us but
libcuda and the regular scratchpad channel for kernel ce/scrub/other? both share same scratchpad
va yes was my idea."*

⊘ And the VA space landing in the VMM is **not a new cost**: §23 requires the VMM to hold CPU maps
to populate BAR1/BAR2 with no demand-fill path, so it holds views of the store regardless. The
owner's message makes this point and it is correct.

### 3.3 A channel in V

`BirthConn::birth_channel` already does TSG alloc → channel alloc → `BIND` →
`GET_WORK_SUBMIT_TOKEN`, fail-closed and unwound. ⚠ **One difference from route K, and it
inverts the interesting part**: route K's channel adopts the **guest's** ring and USERD, because
it is a passthrough channel the guest drives. V's channel is **ours end to end** — our ring, our
USERD, our doorbell — because no guest ever drives it. ⇒ The `RingSource::Guest` machinery is
**not** what this reuses; it is `birth_channel`'s allocation sequence that is.

⊘ Per CeUtils' own default (`channel_utils.c`), the pushbuffer and GPFIFO are **SYSMEM**
(`NV01_MEMORY_SYSTEM`, `_LOCATION_PCI`, `_UNCACHED`) unless a registry key overrides it. We are
not obliged to copy that, but it is the shape RM itself ships, and sysmem costs the store nothing.

### 3.4 Completion on a pollable fd — ★ CONFIRMED AVAILABLE, from the open driver's source

This is (b), and it is the half worth checking before building, so it was checked:

- `NV_ESC_ALLOC_OS_EVENT` = `NV_IOCTL_BASE + 6` = **206**
  (`ogkm-580: kernel-open/common/inc/nv-ioctl-numbers.h:33`), params
  `nv_ioctl_alloc_os_event_t { hClient, hDevice, fd, Status }` (`nv-ioctl.h:69`).
- It is handled **entirely in the OS layer**, never reaching RM core:
  `osapi.c:2782` → `allocate_os_event(pApi->hClient, nvfp, pApi->fd)` (`osapi.c:505`), which
  records `{hParent, nvfp, fd, active}` on the device's event list.
- `nvfp` is the file-private of **the fd the ioctl was issued on**, and `escape.c:872-889`
  requires it to be a **control** fd, forbids self-referential links, and forbids linking to an
  fd that itself has a link.
- **The nvidia character device is pollable**: `nvidia_fops.poll = nvidia_poll`
  (`kernel-open/nvidia/nv.c:238`, impl at `:2252`).

⇒ The sequence is: open a fresh `/dev/nvidiactl`; `NV_ESC_ALLOC_OS_EVENT` naming it; allocate
`NV01_EVENT_OS_EVENT` (class `0x79`) bound to the channel's notifier; then **`poll()` that fd**.
That fd drops straight into the worker's `epoll` set. §35 is satisfied structurally, not by
convention.

⚠ **What is NOT yet established** and must be measured on hardware before this is load-bearing:
that a **semaphore release on our own CE channel** is a notifier RM will post this event for, at
the granularity we need. The primitive is proven to exist and to be pollable; *which notifier
index carries our completion* is an empirical question, and until it is answered the completion
path must **keep the semaphore poll** (§35 permits polling always — `epoll` timeout 0, a queue
check, a semaphore check) as the mechanism, with the fd as the optimisation.

⊘ This is the precondition §35 already names for un-promoted semaphores, so nothing new is being
asked for. But it is the difference between *"the design is sound"* and *"the design is built"*,
and this document must not be read as claiming the second.

---

## 4. The seam — where this actually lands in the code

★ It is **small and already factored**, which is the strongest evidence the design fits the tree.

`kayfabe_rt::device::route_of_engine` maps `EngineKind::Ce → DoorbellRoute::CpuCe`, and
`shell_disposition` maps `CpuCe → ShellDisposition::MayServeLocally`. **Neither changes.** The
routing statement is already *"this work is served in this process"*; what this ruling changes is
**who in this process serves it** — a CPU byte-mover, or a raw client's CE channel.

⇒ The change is an executor behind `MayServeLocally`, introduced as a trait with two
implementations, and the CPU one stays as the fallback rung:

- `CpuCeExecutor` — today's `cpu_ce::execute_ours_spans`. Correct, ~48 MiB/s against the store.
- `RawClientCeExecutor` — builds the descriptor, submits on V's channel, rings V's token, and
  reports completion through the fd (or, until §3.4's residual is closed, the semaphore poll).

⚠ The single call site to convert is `ceutils.rs:1307`
(`cpu_ce::execute_ours_spans(ce.fb(), vmm, &spans)`), guarded by §14.8's `CpuCeStraddle` refusal
directly above it. That refusal is about a span the CPU executor cannot serve; under the raw
client it becomes servable, so **the guard's premise has to be re-derived, not carried across.**
⊘ That is the same class of mistake as w755m's leg-B decline, whose premise expired 19 hours
after it was written and cost this campaign a whole wall.

---

## 5. Increments, in order, each with what makes it judgeable

1. **V exists.** The VMM mints and holds a client on its own `ctl`/`node` fds. Gate: the client
   is alive and `ProcessID` matches the VMM. Nothing observable changes.
2. **V can name the store and the VAS.** Scratchpad dups both into V. Gate: a read-back control
   on the duped objects succeeds from V.
3. **V has a channel.** `birth_channel` with our ring, our USERD. Gate: `GET_WORK_SUBMIT_TOKEN`
   returns a token and `GPFIFO_SCHEDULE` succeeds in V.
4. **V executes one copy.** One `LAUNCH_DMA`, semaphore-polled. Gate: the bytes move, and they
   move for a span the CPU executor would have refused as `CpuCeStraddle` — which is the
   **non-vacuity** condition, and without it this rung proves only that a slower path still works.
5. **The completion becomes an fd.** §3.4's residual closed on hardware, the fd in the worker's
   `epoll` set. Gate: the worker never calls a blocking wait, and a §35 source-scan says so.

⊘ **Order matters and is not arbitrary**: 1–3 are capability with no behaviour change, 4 is the
first observable, 5 is the one that discharges §35. Reversing 4 and 5 would mean building the
completion transport for work that has never completed.

---

## 6. What this document does not claim

- It does **not** claim the raw-client executor is faster. That is overwhelmingly likely (a copy
  engine at ~10 GB/s against a CPU read at ~48 MiB/s) but **unmeasured here**, and this campaign
  has been wrong before about which of two terms binds.
- It does **not** claim §3.4's notifier question is answered. It claims the primitive exists,
  with the source cited, and names the measurement that closes it.
- It does **not** retire the scratchpad. The walker stays there under §20; libcuda stays there;
  the store is still held there. What moves is **emulated-channel execution only**.
