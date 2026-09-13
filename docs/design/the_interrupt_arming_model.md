# THE INTERRUPT ARMING MODEL — fire what the guest armed, not what the channel is bound to

> Status: **LIVE, 2026-09-13.** Owner design + the C artifact's mechanism, which implements it.
> Opened by `[measured w682a]` `INTR-CENSUS nonstall[raises=4 unvectored=39]` — **nine of every
> ten completions the guest waits for were never announced.**

## The owner's rule, 2026-09-13

> - *"passthrough remains passthrough, we do not inspect them. Instead we forward the interrupt
>   arming to eventfd arming. I think mode 2 C has this mechanism correct"*
> - *"for emulated channels, if we wait something in scratchpad, we do not poll on CPU forever,
>   we can also fallback to eventfd (this path must be supported). Also the guest can ask us to
>   arm an interrupt for an emulated channel, and then we need to fire it on completion,
>   **regardless which channel it is**."*

## ⊘ WHAT WE DO TODAY, AND WHY IT FAILS

`RegPlane::announce_completion(engine: Option<u32>)` derives a **per-engine non-stall vector**
from the channel's bound engine:

```rust
let Some(rm_engine_type) = engine else { nonstall_no_engine += 1; return false; };
let Ok(vector) = non_stall_vector(self.chip.intr_table, rm_engine_type) else { … };
```

⇒ **No bind ⇒ no vector ⇒ no interrupt.** `[measured w682a]` that path declined **39** completions
against **4** announced. The guest then sits in `NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS`
(`0x20801702`) — which `[oracle, real GA106]` it calls **175 times until killed**, and which
**hardware calls ZERO times in the whole program**.

★ A guest servicing interrupts in a loop is a guest waiting for one that never arrives.

## ★★★★★ WHAT THE C DOES — the oracle, and it is the owner's rule

`src/qemu/nvkvm_gpu_emul.c:1849-1866`:

```c
static void nvkvm_gsp_deliver_events(NvkvmGpuEmul *s)
{
    if (s->osevent_n <= 0)      return;   /* nothing the guest armed */
    if (s->gsp_swgen0_pending)  return;   /* one signal in flight is enough */
    for (int i = 0; i < s->osevent_n; i++)
        nvkvm_m3_post_event(s, s->osevents[i].hclient, s->osevents[i].hevent,
                            s->osevents[i].notify_index, 0);
    nvkvm_gsp_raise_swgen0(s);            /* ONE fixed vector: SWGEN0, vec 155 */
}
```

Triggered at `:4363` on **`any_completed`** — *"a channel finished: deliver the os-event
completion so libcuda's blocking-sync `poll()` wakes"*:

```c
if (any_completed) { nvkvm_gsp_deliver_events(s); }
```

### The four properties that matter, each different from ours

1. **The trigger is "something completed", not "this channel's engine".** No bind is consulted,
   so a channel with no engine cannot be silently skipped.
2. **The vector is FIXED — SWGEN0 (155)** — not computed per engine. There is no table to miss.
3. **The list is what the GUEST ARMED** (`s->osevents[]`), so this is the *"forward the interrupt
   arming"* half of the owner's rule, held as state rather than derived.
4. ⚠ **Ordering: write the semaphore, THEN signal** (`:4358-4360`) — *"so the payload is already
   visible when the guest re-checks"*. A signal that lands before the payload makes the guest wake,
   look, find nothing, and sleep again — a lost wakeup that looks like a slow device.

⊘ And `nvkvm_m2_osevent_drop` (`:1868`) retires entries when the guest frees the client or the
event, matching on **either** `hEvent` or the client itself. Without it a freed handle keeps being
posted to — which is a use-after-free of a guest object, not merely noise.

## ★★★★★ THE C IS NOT THE TARGET — arm-directed beats broadcast, and closes a race

**Owner, 2026-09-13, refining it:**

> *"is the C better than what you have? I mean should you interrupt always on every completion,
> if the guest didn't ask for it, then its spinning on semaphore and we don't interrupt. if the
> guest asks for interrupt on an already completed channel we immediately interrupt"*

★ **Correct, and it improves on the C.** The C **broadcasts**: `if (any_completed)` posts *every*
armed event. Two consequences:

1. ⊘ **It fires when nobody asked.** A guest spinning on a semaphore wants no interrupt; posting
   anyway is an exit per completion with no consumer.
2. ⚠ **It hides the arm-after-completion race rather than closing it.** An event armed *after* its
   work finished fires only when some **unrelated** later completion sweeps it up. If none comes,
   it never fires. ⇒ Same hang as ours, but **load-dependent** — which is strictly worse to debug
   than a deterministic one.

⊘ And that race is the common case, not an edge: libcuda's blocking sync **spins first, then arms
and blocks**. The arm therefore lands *after* the work completed much of the time.

### The rule

- **Nothing armed for this completion ⇒ raise nothing.** The guest is polling; a semaphore write
  it can read is the whole contract.
- **An arm on an already-completed channel ⇒ fire immediately**, at arm time.
- **Otherwise ⇒ fire on completion**, to the armed event only.

### ⚠ THE ORDERING, WHICH IS THE WHOLE CORRECTNESS ARGUMENT

**Register the arm FIRST, then check for an already-complete state.**

| order | a completion landing between the two steps | verdict |
|---|---|---|
| register → check | seen by the registration **or** by the check — at worst both | ★ **safe**: at most a duplicate wake |
| check → register | seen by **neither** — the check ran too early, the registration too late | ⊘ **lost wakeup**, and it hangs forever |

★ Same asymmetry every other gate in this tree rests on: **over-report, never under-report.** A
spurious wake costs the guest one re-check of a semaphore it was going to read anyway; a missed
wake costs the process.

⊘ It also makes the counters honest. Under a broadcast, `raises` counts sweeps rather than
answered waits, so it cannot distinguish *"we woke the right waiter"* from *"we woke everyone and
one of them happened to be right"*.

## What this means for each channel kind

| kind | today | under this model |
|---|---|---|
| **Passthrough** | we announce only if the channel has a bound engine, and drop it otherwise | ⊘ **do not inspect the channel at all** — ogkm's own model for userspace channels. Arm a host eventfd, relay its signal, decode nothing. The per-engine vector is out of scope, not missing |
| **Emulated (scratchpad wait)** | CPU-polls; `raise_os_event` is gated on `ServedLocally` | must **also** support an eventfd fallback rather than polling forever — the owner names this as required, not optional |
| **Either, guest-armed** | vector derived from the bind, dropped when absent | ★ **fire what the guest armed, on completion, regardless of channel** |

## ★★★★★ PASSTHROUGH: WE DO NOT INSPECT THE CHANNEL AT ALL

**Owner, 2026-09-13:**

> *"for passthrough: we do not care about semaphores at all, we just forward the eventfd as
> interrupt assuming the GPU did everything correctly. Its the exact same how the ogkm driver
> handles userspace channels, it also just registers interrupt for channel and forward of bare
> metal GPU without inspecting the channels userspace set up"*

★ **This is the real driver's own model**, and it is a deletion rather than a feature. For a
userspace channel, RM registers an event against the **channel** and relays the GPU's non-stall
interrupt. It never parses the pushbuffer and never reads the semaphores — **userspace owns what
it wrote there**, and RM has no business knowing it.

⇒ For a passthrough channel we must:
- **arm** the host-side event (eventfd) when the guest arms its interrupt — the *"forward the
  arming"* half;
- **relay** the host's signal to the guest's event;
- ⊘ **decode nothing**: no semaphore read, no completion tracking, no per-engine vector derived
  from a bind we should not need.

⚠ That last line is where today's defect lives. `announce_completion` tries to vector completions
for **every** channel kind, passthrough included, and gives up when the bind is absent
(`[measured w682a]` 39 of 43). For passthrough the whole computation is **out of scope** — the
host GPU completed the work and the host driver knows it; our job is to carry the signal.

★ It also removes an entire failure class by construction: a passthrough completion we cannot see
is no longer a completion we can fail to announce.

## ★★★ EMULATED: THE COMPLETION IS VISIBLE BEFORE THE INTERRUPT, GUARANTEED

**Owner:** *"for emulated channels: set completion before doing any interrupt, guaranteed."*

The C pins the same order at `nvkvm_gpu_emul.c:4358-4360` — *"write sema THEN signal, so the
payload is already visible when the guest re-checks"*.

⊘ **Inverted, it is a lost wakeup and not a slow one.** The guest wakes, re-reads the semaphore,
sees the OLD value, concludes the work is unfinished and blocks again — with no further interrupt
coming, because the one that would have told it already fired. The symptom is a hang; the cause is
two stores in the wrong order.

⚠ It is not enough to write them in source order: the completion is a write to **guest memory**
and the signal is a **device interrupt**, two different paths. The store must be *ordered before*
the signal, not merely written above it.

## ⚠ Why this is a CORRECTNESS issue and not a latency one

A completion we decline to announce is not slow — it is **lost**. The guest's blocking-sync path
waits on an event that will never be posted, and no error is logged anywhere: `HOST_DMESG_XID=0`
on every one of these boots. ⇒ It presents as a hang with a clean log, which is the most expensive
shape a defect can have.

★ And it was invisible until `intr_census` existed: `Regs::audit()` has carried these counters
since they were written and **had no consumer outside tests**.

## The split that made it actionable

`nonstall_unvectored` covered **three** causes with three different fixes — no bound engine, no
vector in the chip's `intr_table`, and an out-of-range vector. It now reports them separately
(`no_engine` / `no_vector` / `out_of_range`), because *"39 unvectored"* names a symptom and the
three arms name three bugs. ⊘ The w625 lesson, on the path that matters most.
