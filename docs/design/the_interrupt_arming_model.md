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

## What this means for each channel kind

| kind | today | under this model |
|---|---|---|
| **Passthrough** | we announce only if the channel has a bound engine | ⊘ **do not inspect the channel at all.** Forward the guest's interrupt *arming* to eventfd arming on the host; the host's completion drives the wake |
| **Emulated (scratchpad wait)** | CPU-polls; `raise_os_event` is gated on `ServedLocally` | must **also** support an eventfd fallback rather than polling forever — the owner names this as required, not optional |
| **Either, guest-armed** | vector derived from the bind, dropped when absent | ★ **fire what the guest armed, on completion, regardless of channel** |

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
