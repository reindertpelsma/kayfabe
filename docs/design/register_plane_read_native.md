# Read-native, write-trap — the register/doorbell BAR pages

> **Owner directive, 2026-07-31:** *"only write should be trapped to translate doorbell tokens."*
>
> Status: **ruled; the blocking question is MEASURED and settled; the memslot caller is not
> built and needs an owner decision.** Task `#128`. What ships today (`5b54494`) is a stopgap
> and §4 says so.
>
> ★★★ **Read `read_native_timer_measured.md` next.** It carries the 2026-08-02 GA106 run that
> settled *"can a capability-less process map the host counter at all?"* (**yes**, two routes,
> live and advancing), and the geography finding that **reshaped §5 of this document**: the
> guest reads its counter at page offset `0x080`, the host's doorbell-free PTIMER page carries
> it at `0x400`, and a memslot cannot re-base within a page — so the only host page that can
> back the guest's timer page is **the one with the doorbell in it**. §1 is therefore one
> policy over one shared page, not two policies over two pages, and `KVM_MEM_READONLY` is
> exactly that policy.

## 1. The rule

On the GPU's register / doorbell BAR pages:

- **Reads are native passthrough** — memslot-backed, no vmexit.
- **Writes are trapped**, and the *purpose* of trapping them is **doorbell-token translation**:
  the guest's `{runlist, chid}` must become the host's chid before it reaches hardware
  (`mode2_doorbell_chid.md` §5 frames the collision this resolves).

This settles the open axis recorded as `#87` ("the read-native write-trap dispatch looks
undesigned").

## 2. ★★★ Why reads must be native: accuracy, not performance

The obvious argument is that trapping a hot register costs vmexits. **That argument is not the
load-bearing one, and reaching for it first led me to the wrong conclusion.**

Trap volume was counted on 2026-07-31 by decoding the committed trace
`traces/mode2_c_reference/cap3_matmul_forwarding.rec.zst` (532 824 records, `n_errors=0`, chip
GA106, captured 2026-07-29) with `scripts/mode2_diag/rec_dump.py`: **225 `Clock` records spanning
`#139879 … #532143`**, ns 58.8 s → 97.0 s — i.e. timer reads continue through the compute phase, and
each was served by the emulator. On that basis I concluded the case for passthrough was weak.
⊘ **Wrong axis.**

> A vmexit costs **microseconds, with high variance**. A **nanosecond** counter read through a trap
> therefore carries jitter **~3 orders of magnitude larger than its own resolution**. It stops being
> a nanosecond timer the instant it is trapped — stale and noisy *by construction*, however rarely
> it is read.

⇒ **Trap volume is irrelevant to this decision.** A once-per-second trapped read is as wrong as a
million. ★ And unlike §3, this argument is **workload-independent**: it holds even for a guest that
never correlates anything.

## 3. The second, independent reason: timebase

Compute is forwarded to a **real host GPU**, so every timestamp the GPU itself produces — event
semaphores, `%globaltimer` inside kernels — is in the **host GPU's timebase**. A synthetic CPU-side
clock (the C used `QEMU_CLOCK_VIRTUAL`; this port used host monotonic) is **unrelated** to it.

Anything that correlates the two — `cudaEventElapsedTime`, every profiler — gets a **wrong answer,
not a slow one**, surfacing as nonsense durations rather than a failure. Read-native passthrough
makes the guest read the host GPU's own counter, so correlation is correct **by construction**, with
no clock to keep in sync.

★★ A synthetic clock can only ever approximate a value we are not allowed to see.

## 4. What this reclassifies

`NanoClock` / `HostMonotonicClock` (`5b54494`) is a **boot-only stopgap**. It was necessary for the
hang it fixed — RM's `gpuCheckTimeout` needs elapsed time to *exist*, and a defaulted zero produced
an **unkillable silent D-state with no dmesg**, observed on a stock unpatched 580.159.04 guest at
`5b54494` on 2026-07-31 and cleared by that commit in the same session — but it is **not the
finished design**, and its rustdoc must say so rather than reading as intent.

⊘ **Standing rule this produces:** *never answer a free-running counter with a constant.* Zero is
**plausible**, so the driver believes it and spins forever with no diagnostic. A refusal is not
plausible, so it surfaces in one boot.

## 5. Two timer registers, and the difference matters

| register | offset | HW mode | role |
|---|---|---|---|
| `NV_PTIMER_TIME_0/1` | `0x9400` / `0x9410` | **`RW-4R`** | what `tmrGetGpuPtimerOffset_GM107` hands to clients; `NV2080_CTRL_CMD_TIMER_GET_REGISTER_OFFSET` exists expressly *"so that clients may map them directly"* (`ogkm-580: ctrl2080tmr.h:107-110`) |
| `NV_VIRTUAL_FUNCTION_TIME_0/1` | `0xBB0080` / `0xBB0084` | **`R--4R`** (read-only in HW) | the VF mirror the kernel polls — the pair whose zero answer hung the guest |

⚠ The PF pair is **writable**, and `tmrSetCurrentTime_GV100` writes it. A guest write must **not**
reach the host GPU — it would perturb the host. Writes are trapped under §1 anyway; the policy there
must be **decided explicitly** (refuse-by-name is this project's house style) rather than falling
through.

★ **DECIDED, 2026-08-02:** `kayfabe_device::plane::PTIMER_WRITE_REFUSED`, counted in
`Counters::ptimer_writes_refused` — its own counter, because `unclaimed_writes` means *"this
port does not model that offset"* and this means *"it models it and says no"*. Rationale and
the two independent mechanisms that also hold it: `read_native_timer_measured.md` §4.

★★ **And a correction to the table above.** Both rows are real registers, but the guest
driver reads only the **second**: `tmrReadTimeLoReg_TU102` goes through the virtual-function
aperture unconditionally, on a virtual function *and* on the physical one (`ogkm-580:
src/nvidia/src/kernel/gpu/timer/arch/turing/timer_tu102.c:130-155`). The `0x9400` pair is what
`NV2080_CTRL_CMD_TIMER_GET_REGISTER_OFFSET` hands a *client* — measured `NV_OK, 0x9000` on a
GA106, and its page is mappable — but it is **not** the pair a bare-metal driver polls, and
its page offset rules it out as the backing page anyway.

## 6. What the trace can and cannot settle

★ **Settled** by the `cap3` decode named in §2: the timer is read during real compute, and those
reads are served by the emulator rather than passing through.

⚠ **Not settled, and it does not need to be:** whether those reads originate in the guest *kernel*
or in a *userspace* mapping. The register aperture is emulator-served, so a userspace `mmap` of BAR0
traps identically — the trace cannot separate them. §2 holds either way.

## 7. Open, to state rather than discover in an audit

A read-only free-running host counter is a **low-risk exposure** but is a **high-resolution timing
side channel**, and it leaks host GPU uptime. Say so in the security model when this lands.

★ **DONE, and the guess about *what* it leaks was wrong.** `guest_blast_radius.md` §5.4.
**[measured]** on a GA106 the counter reads Unix wall-clock nanoseconds — `2026-08-02
00:57:36.684 UTC`, against a host uptime of 14 h — because RM sets it with
`tmrSetCurrentTime`. So the disclosure is the **host's wall clock at ~32 ns resolution**, not
uptime: a channel around a deliberately offset guest clock, and a host-wide shared reference
two co-tenants could use. ⚠ One prerequisite remains `[unverified]` and gates shipping: a
memslot is page-granular, so the guest reads the *whole* 64 KiB usermode window, and what is
in the undocumented remainder has not been measured.
