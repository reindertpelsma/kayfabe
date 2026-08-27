# S1 — what does the terminal refusal actually protect?

**Answer: nothing. It is a ROUTING statement, and it never guarded the invariant it was cited for.**
The thing that actually makes GR execution impossible is an **absence**: no code copies the guest's
commands into the host channel.

**Status:** read-only audit, 2026-08-10. `kayfabe` @ `ae73f6b`.

> ### ✔ Verified by me
> - **`doorbells: 191 arrived, 183 served, 8 REFUSED by name`** (`run_w228a_82f9aa5_fbback_qemu.log`).
>   ⊘ **My repeated "all 86 GrCompute doorbells" is WRONG** — 86 is the *method* count of one
>   pushbuffer, and separately the GrCompute *doorbell* count of a **different, earlier** boot
>   (`s51`, `GrCompute=86 Ce=362`). I fused two numbers from two boots. **It is 8.**
> - `fn ce_channel(&mut self, vas: HostHandle)` (`kayfabe-isolate-host/src/rm.rs:3501`) — our CE
>   channel is created **in an address space passed to it**. ✔

## 1. Provenance — there are TWO decisions with the same name

| commit | when | what | stated reason |
|---|---|---|---|
| **`d502ac6`** | 08-10 **04:41** | ★ **introduced** the refusal | *"a **routing statement, not a plane change**"* — the CPU copy-engine executor is the wrong executor for a GR pushbuffer; refuse **by name before reading a byte of a ring** |
| **`184df5f`** | 08-10 **21:47** | declined to **open** it | the MME/containment argument — **17 hours and three rungs later** |

⇒ ★★ **Retiring the MME clause (`07227a4`) refutes an argument for KEEPING S1 SHUT. It does not
touch the reason S1 EXISTS.** Downstream text cites "S1" for both interchangeably. **Say which.**

## 2. What is behind it — eleven gates, and none of them is why nothing runs

On the shipping configuration (`ce_executor=local`, every measured boot), de-refusing sends the
doorbell into the **CPU copy-engine executor** — the very path `d502ac6` removed it from — where the
class-gated codec decodes `0xc7c0` to `Opaque` and returns `SubmissionDecodedNoWork`.
⇒ **Removing S1 changes one refusal's name into another refusal's name.**

★★★ **The decisive fact is not a gate. It is a MISSING VERB**, stated in three independent places:
- `device.rs:1793-1797` — *"It rings the isolate's host channel, **which the guest's methods are
  never copied into**."*
- `gr_execution_boundary.md` §0.1 — *what puts methods in it: the guest, directly (the C) |
  **nothing** (kayfabe)*; *"Scheduling an empty host ring makes the host engine consume nothing,
  correctly and forever."*
- S1's own comment — GR forwarding *"needs a host channel that SHADOWS the guest's … **neither of
  which is built**"*.

⊘ **Do not build a guard for the missing method-copy. It is not a gate; it is an unbuilt feature.**

⚠ Two real consequences of opening it anyway: on the host-executor path a guest doorbell would
**allocate and schedule host GR state** for an empty ring; and the `GR-ADDRESS-CENSUS` /
`COMPLETION-WATCH` instrumentation currently **rides the refusal arm** and would have to move first.

## 3. ★★★ THE CO-LOCATION VERDICT — the invariant is VIOLATED as PLACEMENT, today

> **Owner's invariant: VMM state must never be placed where a guest VA can name it.**

There is **one host address space per guest address space** (`Vas::host_vas`), and **everything**
goes in it: fabricated publishes, guest-RAM pins, w228's FB leaves — all FIXED **at guest-chosen
VAs** — the guest's own materialized host channel, **and our CE ring + USERD + completion
semaphore** (`plan_ce` → `ce_channel(vas)` → `alloc_channel_on(vas, …)` → `raw_map_dma`).

The code already knows: *"RM's own VA allocator and our fixed publishes **share one address
space**"* (`rm.rs:1263-1274`). And the sentence the invariant actually rests on is
`rm.rs:1259-1263`: *"memory the isolate allocated for itself, **which no guest ever names**."*
⊘ **That is an assertion in a doc comment, gated by nothing, and true only while no engine executes
guest-authored methods in that space.** It is a property of the missing verb, not of the placement.

⚠ **w228 made this MORE real:** three 2 MiB vidmem objects are now FIXED at **guest-chosen VAs** in
the same space that holds our semaphore.

**Verdict, split three ways because the answer differs per question:**
- *Is VMM state placed where a guest VA can name it?* — **YES, decidably, today.** The address is
  RM-chosen, which makes it **unpredictable, not unnameable.** Unpredictability is not a boundary.
- *Could a guest GR channel, once executing, name it?* — **YES**, same fact. **[NOT MEASURED]** — no
  boot has ever executed a guest-authored method on a host engine.
- *Is it exploitable today?* — **NO, and S1 is not why.** `forwarded=0` in every boot, no host CE
  channel was ever created, single-writer census 7/7 green, and the guest's semaphores read
  `0x00000000` over 81-88 samples × 8 channels × 4 boots. **The missing method-copy is what makes it
  inert.**

⇒ ★★ **S1 is not the guard on this invariant and never was. Retiring it would not breach the
invariant — and keeping it does not uphold it.** The placement defect is live now, independent of
S1, and fixable **on the CE path**, where there is a working executor to regress against and nothing
to do with graphics.
