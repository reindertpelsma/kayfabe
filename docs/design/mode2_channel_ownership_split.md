# Mode-2 channel ownership: passthrough vs emulated, and how completions cross the line

**Status:** owner ruling, 2026-08-10. Continues `mode2_doorbell_chid.md` (owner, 2026-06-05) and
`mode2_forwarding_model.md`. Written here because this repo already hosts the June ruling it extends;
the `kayfabe` tree should carry a pointer to it.

⚠ **This page records an owner brainstorm plus corrections made against measurement and against
NVIDIA's own driver.** Where a claim is inference it says so. Where the assistant was wrong, it says
that too — several of the corrections below are of the assistant, not of the owner.

---

## 1. The split, and the discriminator

★★★ **QUALIFICATION, 2026-08-11 — read before §1's second bullet: SOME KERNEL CHANNELS ARE GR
CHANNELS, and one of them PUSHES GRAPHICS METHODS.** Enumerated from `ogkm-580.159.04` in
`kernel_gr_channels_and_the_mme_exposure.md`. Three kernel-side channels target `RM_ENGINE_TYPE_GR0`:
golden-image context init (`kernel_graphics.c:2136`, GSP-client-**specific**), the Turing-only Bug
4208224 WAR (`kgraphics_tu102.c:296`), and the **RC watchdog** (`kernel_rc_watchdog.c:437`), which puts
a `FERMI_TWOD_A` object on it and emits `NV902D_*` methods from CPU kernel code (`:1246-1345`).
⊘ The good news, and it is what keeps this page's split intact: **the kernel's ENTIRE GR method
vocabulary is those five `NV902D_*` methods, and there is NO MME anywhere in kernel code** — both
established by tree-wide grep. So the emulated axis is `{CE methods} ∪ {five NV902D_ methods}` and
stays statefully decodable. ⚠ But *"kernel channel"* must no longer be read as *"copy engine"*: the
emulated axis has to model a GR-engine notifier, and a `GR0` channel carrying a `3D`/`COMPUTE` object
that never receives work. ⚠ Do not conclude otherwise from the GA106 bench — Bug 4208224 is
TU102/104/106-only and is invisible there, inside the Turing+ support floor.

> **Guest-userspace-managed channels are always passthrough.** No exception has been found that we
> need to implement.
>
> **Kernel channels are mostly completely emulated.** Passthrough of a kernel channel is possible
> *only* if every action on it is guaranteed unprivileged and executable against host **userspace**
> — no physical addressing, no privileged registers. That is unlikely for most kernel channels.

★ **The discriminator is who PUSHES the commands** — who writes the pushbuffer and advances `GP_PUT`
— **not** whose memory the work touches, and not whose VA space the operands live in.

This dissolves a question the assistant had posed badly ("which side is UVM on, since its channels
are kernel-allocated but operate on user memory?"). Owner's answer, and it is correct: libcuda and
the kernel driver do not both drive one ring — that would be a data race on `GP_PUT` — so **for any
channel it is unambiguous who pushes**. UVM's channels are built and driven by the guest kernel's UVM
module, so they are **kernel** channels; and because UVM's migration copies use physical addressing,
they also fail the "guaranteed unprivileged" test ⇒ **emulated**.

⇒ The C already implements exactly this boundary, in one line
(`src/qemu/nvkvm_gpu_emul.c:4265`):

> *"**User-CE / GR channels are EXCLUDED (the host executes + releases those for real).**"*

The corresponding split for the objects on a channel (`mode2_doorbell_chid.md`, 2026-06-05):

| object | class | treatment |
|---|---|---|
| pushbuffer, GPFIFO ring, referenced data, completion semaphore | **chid-INDEPENDENT** | `OS_DESCRIPTOR`-pin the guest pages, `map_dma` FIXED into the host channel's VAS **at the guest VAs** |
| USERD (`GP_PUT`), doorbell token | **chid-DEPENDENT** | translate |

---

## 2. The doorbell is one lookup and one store

`mode2_doorbell_chid.md` item 3, and the C's implementation
(`nvkvm_gpu_emul.c:4220`, `:9160`):

```c
uint32_t tok = c->token_valid ? c->host_token : s->m2_gr_token;
stl_le_p((uint8_t *)s->m2_usermode_qva + 0x90, tok);   /* the whole fast path */
```

Receive the token, look it up, ring the translated one, VM-enter. **No inspection, no parse.** The
host's usermode doorbell page is mapped by any CUDA process, so the store needs no privilege. The
VM exit + enter is inherent; the body must be the most optimised short code in the system.

⊘ **Correction of the assistant:** "zero exits if we can dictate the chid" was wrong framing. The exit
is inherent to trapping the write. The named optimisation is different and already recorded: make
USERMODE a `KVM_MEM_READONLY` memslot so **reads and PTIMER are native and only writes fault**.

Only channels we own completely — USERD *and* buffers *and* ring *and* semaphores — take the slow
path of manual inspection, and for those **no direct host ring happens at all**.

---

## 3. Wakeup: three routes, and only one of them is hot

⊘ **Correction of the assistant.** The assistant worried that "every completion costs a GSP RPC
round-trip". That is wrong, and the owner's instinct is right. There are three distinct routes:

1. ★ **A semaphore in memory, polled by the waiter.** This is the hot path and the overwhelming
   majority of real work: **zero interrupts, zero VM exits, nothing for us to do.** An LLM in steady
   state lives here — and a kernel launch costs **zero ioctls**, so the control plane is not involved
   at all after `cuCtxCreate`.
2. **GSP message queue + `POST_EVENT (0x1003)` + the GSP stall vector** → guest ISR → `kgspService`
   drains → `osNotifyEvent` → the registered os-event fd wakes. This is the **blocking-sync /
   os-event** path: context setup, explicit `cuStreamSynchronize` with blocking sync, teardown. **Cold
   by construction**, so its cost is not a parity concern.
3. **Non-stall notifier interrupts** (e.g. notifier 35, which is what makes `nvidia-smi`'s process
   list work). A separate vector from (2); do not conflate them.

⇒ The interrupt round-trip is inevitable *when an interrupt is genuinely required*. The optimisation
is not making the round-trip cheaper — it is that **most completions never need one**, because the
guest is polling a semaphore the hardware wrote.

⚠ **Open, and worth checking rather than assuming:** a measured boot showed `completions: 4 announced,
**179 UNVECTORED (work done, nothing told the guest)**`. Before building more announcement machinery,
establish how many of those 179 any guest actually asked to be told about. Announcing work nobody
requested notification for is the *opposite* of the rule above.

---

## 4. Scheduling: the guest expresses intent; the GPU and GSP schedule

Owner's model: *the host schedules the guest, just as the host schedules vCPUs as threads.* A guest
determining relative priority **among its own channels** is a performance property, not a correctness
one — correct workloads synchronise explicitly rather than depending on scheduler behaviour.

**Owner's question — "does the guest preempt channels and do the context switch, or is that the
GSP/GPU's job?" — answer: it is the GPU's and GSP's job**, which is the outcome the owner preferred.
On Turing+ with GSP-RM:

- The CPU-side driver **allocates** TSGs and channels, sets timeslice and preemption **mode**
  (`NV2080_CTRL_CMD_GR_SET_CTXSW_PREEMPTION_MODE` — WFI vs CILP; we already see this control), and
  calls `GPFIFO_SCHEDULE` to make a channel **resident on a runlist**.
- The **GPU's host/FIFO engine executes the runlist** and performs context switches; **GSP-RM** owns
  runlist submission and scheduling policy.

⇒ The guest never performs a context switch itself. It expresses **intent**, and intent is exactly
what we can accept without honouring precisely. This is what makes host-side scheduling — shared with
other host CUDA processes and other VMs, and therefore unpredictable — **hideable**.

⚠ I claimed the real risk was the guest's **watchdog** — that a guest could declare a channel dead
because our scheduling was slow. **That is refuted, and the refutation is structural.**

### ★★★★ The guest CPU-RM cannot time out a channel at all on GSP hardware

`R580 kernel_gsp.c:541-544`, NVIDIA's own comment, verbatim:

> *"**RC error handling ("Channel Teardown sequence") is executed in GSP-RM.** Client notifications, OS
> interaction etc happen in CPU-RM (Kernel RM)."*

The guest side is a **pure receiver** of `NV_VGPU_MSG_EVENT_RC_TRIGGERED`. ⇒ **In Mode 2, GSP-RM is
us. We are the only party that can declare a channel dead.** "The guest times out because our
scheduling was slow" is, for the FIFO path, **structurally impossible**.

Three supporting refutations, all cited:
- **The RC watchdog is OFF by default and never watches app channels.** `RmInitAdapter` enables then
  immediately disables it (`osinit.c:2160,2165`), and NVIDIA's comment says *"**CUDA wants the
  watchdog disabled**"* (`kernel_rc_watchdog.c:136-137`). It only re-arms on an explicit client
  request, and what it watches is RM's *own private* `FERMI_TWOD_A` notifier pushbuffer.
- **UVM has no timeout.** `uvm_spin_loop` returns `NV_ERR_TIMEOUT_RETRY` only to print every 30 s and
  then loops forever; every UVM wait exits solely on a **non-zero error notifier** — i.e. on an RC
  somebody else declared. UVM cannot originate a timeout.
- ⊘ **The emulation 60× timeout multiplier is a dead knob** — `IS_EMULATION`/`IS_SIMULATION` have
  **no writer** anywhere in `src/nvidia/`. We cannot buy slack by claiming to be an emulator.

### ★★★ What DOES have a clock: the GSP RPC poll — and it is a budget on OUR service latency

`_kgspRpcRecvPoll` waits **1.5 × the RM default** (`kernel_gsp.c:2372-2378`). The default is
`osGetTimeoutParams` (`os.c:1961-2014`): **4 s in graphics mode, 30 s in compute mode** — so the poll
budget is **6 s / 45 s**. Mode flips to compute when the guest allocates a `GR_OBJECT_TYPE_COMPUTE`
class object (`kernel_graphics_context.c:3183-3185`), **which is an RPC we service**.

⇒ ★★★★ **The entire `cuCtxCreate` path runs at the 6 s budget**, and only afterwards does it become
45 s. Escalation is: `NV_ERR_TIMEOUT` returned and *"we will soldier on"* → **Xid 119** → on the
**third consecutive** timeout, `gpuMarkDeviceForReset` and **RC of every channel**.

⇒ ★★★★★ **The exposure is our own RPC service tail latency, NOT GPU sharing.** None of these clocks
measures GPU execution time; they all measure round-trip latency **to us**. Contention with other
host processes or VMs affects the former, not the latter — unless our service loop is itself blocked
behind it.

**⇒ THE HARD DESIGN CONSTRAINT THIS PRODUCES, and it binds §5:** *never do long synchronous work
inside an RPC reply path.* A **real scrub** (case 2 above) issued synchronously during `cuCtxCreate`
has **6 seconds**, total, including everything else in that RPC. ⇒ The scrub must start the host
operation and return, with the completion bridged asynchronously — which is the same conclusion the
vCPU-thread constraint reaches from the other direction. Two independent arguments, one rule.

**Recommendation: change nothing about the timeouts; measure instead.** Instrument per-RPC service
latency and assert p99 ≪ 6 s during `cuCtxCreate`, ≪ 45 s after. ⊘ Extending them is not available
anyway: the only lever is a **guest registry key**, which breaks the stock-guest property that is this
project's headline claim. ⚠ And it would be a **guest-side availability loss we inflict**: a guest that
cannot time out on GSP RPC cannot detect that we have stopped answering — Xid 119 and the three-strike
reset are its *last* wedge detector once the above refutations are accounted for.

---

## 5. The scrubber, and the general shape of an emulated kernel channel

★ **Owner's reframing, and it is the right one:** a scrub means **"ensure future access to this memory
reads blank"**, not "make this memory blank now". That turns one operation into three cases:

| the page is… | correct action | why the guest cannot tell |
|---|---|---|
| **unallocated / unbacked** | **no-op** | the next allocation yields empty pages anyway |
| **in use** | **really scrub it** | the very next read must return zero and the page cannot be deallocated |
| **dangling — no references** | **free it on the host** | the next use allocates a fresh empty page |

All three produce the same guest-observable behaviour. ⇒ **Applies to kernel-initiated scrubs only;
guest-userspace scrubs are passthrough and none of this applies.**

**Two invariants this rests on. State them; do not assume them.**
1. **Our backing is zero on first touch.** If a page we hand the guest is not zeroed, case 1 is a
   data leak, not an optimisation.
2. **Host RM scrubs on allocation.** Case 3 depends on it.

⚠⚠ **The one genuinely dangerous part — signalled, because getting it backwards is a security bug,
not a correctness bug.** Cases 1 and 3 are *optimisations* and case 2 is the *only* one that is always
safe. Misclassifying an in-use page as unbacked or dangling hands the guest **stale contents of
somebody else's memory**. ⇒ **The default must be case 2 (really scrub), with cases 1 and 3 requiring
POSITIVE PROOF of unbackedness or of danglingness.** "We have no record of this page" is not proof of
either — this tree has already had orphan-generation lifetime bugs where our record and reality
disagreed.

### ★★★★ OWNER RULING (2026-08-10) — staging, and the constraint proof alone does not give you

> *"Cases 1 and 3 are **pure optimisation** that require **both hard proof and a lock** to prevent a
> race falsifying the proof during the optimised scrub — like a free. **If that fails, case 2 always
> applies.** … Start with 'always really scrub': **yes, 100%**. Cases 1 and 3 are purely a shortcut."*

⇒ **Build order is settled: case 2 only, first.** Cases 1 and 3 are a later, separately-justified
optimisation and must never be the path of first resort.

★★★★ **And the lock requirement is a distinct constraint from the proof, easy to miss.** A proof of
unbackedness or danglingness is a statement about **an instant**; the optimised action (a free, or
skipping the scrub) takes **time**. Between proving and acting, the guest can allocate into the page,
take a reference, or map it — **falsifying the proof after it was correctly obtained**. ⇒ The proof and
the optimised action must be **under one lock**, and **failure to take that lock is not a reason to
retry the optimisation — it is a fall-through to case 2.** A correct proof is not sufficient; it must
still be true at the moment of the act.

⚠ This is a **TOCTOU on a security boundary**, and this tree has already produced the general form of
it once — *"a correct capture can answer the wrong question: the question was about a LIFETIME, the
instrument sampled one instant."* The scrub optimisation is exactly that shape, with a data leak as
the consequence rather than a wasted rung.

### The general pattern for an emulated kernel channel

> We write the semaphore, and optionally send an interrupt if an os-event is set. If a **real** host
> operation is required, we track the **host's** completion separately and bridge it to the guest
> semaphore we manage by hand.

★ This is the general shape, and it makes precise what "manual completion handling" means: **the
guest-facing semaphore is ours to write because the channel is ours; the host-facing completion is a
separate object we observe.** Nothing about it is forgery — forging is writing a **user** channel's
completion, which the host was supposed to release.

⊘ **One constraint the brainstorm did not state, and it is load-bearing: the wait must not run on the
vCPU thread.** "Spin on the semaphore, or an eventfd if it takes too long" is right in shape, but a
doorbell trap runs on the vCPU with the BQL held. A spin or a blocking wait there stalls the guest and
can wedge it. ⇒ The bridge belongs on the reactor (which exists in the tree and has never been
reached), and the doorbell path must return promptly having only *started* the host operation. An
audit found **nine** sites on the doorbell path that can block, only two of which are bounded.

---

## 5b. ⊘ WHAT THE C ACTUALLY BUILT — corrections to §§2, 3 and 6 of this page

★★★★ **Read this before treating anything above as "the C already did it".** A full audit of the C's
source (2026-08-10) found that **four items this page and `mode2_doorbell_chid.md` describe as the C's
solution were planned and not built, or built differently.** The June page is a *build plan*; only some
of it landed.

**⊘ C1 — There is NO KVM memslot anywhere in the Mode-2 data plane, so "the guest's write lands
directly" is false.** BAR1 is fully trapping: every guest USERD access is a **VM exit** →
`bar1_pdb` GMMU walk → `nvkvm_fb_write` → the `m2_fbback` overlay → `stl_le_p` into the host mmap.
One physical host page, two views — but the guest view is **mediated by a trap on every access**.
`nvkvm_mmap_host.c`'s memslot machinery (including its `KVM_MEM_READONLY` support) **is never called**
from the device. ⇒ **The C never achieved the no-exit hot path.** `mode2_forwarding_model.md:114-118`
describes the untrapped shared page as the intended end state; **it was not built.**

**⊘ C2 — The doorbell is NOT a token lookup.** The C **never translates the guest's token** — the
written value is used only for gated logging. Ringing is **GP_PUT-driven demux**: on any doorbell
write, scan every registered channel and ring the host token of each whose `GP_PUT` advanced. This was
deliberate and *measured*: `mode2_doorbell_chid.md:391-399` records that guest token `0x10001` matched
**no** host token, so *"doorbell pass-through is INCORRECT"*. ⇒ The body is a **linear scan over 64
channels**, not an O(1) lookup. Any perf argument that assumes a table lookup is arguing about code
that does not exist.
⚠ And **T18** says the alternative is closed: legacy-vGPU host-allocates-chid needs
`NV_PMC_BOOT_1.VGPU == _VF`; `_PV` falls through to bare-metal and the `IS_VIRTUAL_WITHOUT_SRIOV`
paths are dead in the open build. **Trap-and-translate is mandatory.**

**⊘ C3 — The ring gate of §6 was never built.** `:4162`: *"Unconditional (the **m2ring gate was
removed**)"*. What replaced it is **ordering, not gating**: `nvkvm_m2_exec_doorbell` runs the
working-set sweep and the per-entry pushbuffer walk **before** the ring, in the same function, on the
same doorbell — *"so the full working set, incl. the semaphore the host must write, is FIXED-mapped
into the host VAS before any ring (else a ring faults the host GPU on the SEM_RELEASE target →
**cuInit=999**)"*. Plus a **token defer** that gates on *schedulability*, not on mapping. **"Fully
mapped" is never computed anywhere.** ⇒ §6's principle (don't fault) is right; the mechanism is
*sweep-then-ring in one handler*, and `mode2_forwarding_model.md:148` still lists the real gate as
**future work**.

**⊘ C4 — For userspace channels the C is a DOUBLE WRITER in the shipped config.** `nvkvm_chan_execute`
parses **every** channel including GR and software-writes `COMPUTE_REPORT_SEM` / `SEM_RELEASE` /
`CE_SEM_RELEASE`. Suppression requires `m2hostsem`, **default OFF** — an A/B never promoted. The C
**never observes the host's completion**: it reads host `GP_PUT`/`GP_GET` at ring time only for a
trace-gated log, and delivers the os-event on **its own** bookkeeping. ⇒ The `:4265` line quoted in §1
(*"User-CE / GR channels are excluded"*) scopes **only the kernel finishPayload forge**. It is **not**
a statement that userspace completions were left to the host.
⚠ Worse, this was deliberate: `M5.38` records that letting the host's writes reach the guest semaphore
made *"the LAGGING bridged host channel write stale payloads over the live value ~40 s late"*,
tripping UVM's wrap detector. So in the green config the host's completions were **deliberately kept
out of the guest's semaphore**.

⇒ ★★★★★ **Consequence for the plan.** "Parity, like Mode-2 C did" is a **lower bar than it sounds**:
the C traps every USERD access, scans channels per doorbell, and still writes compute semaphores in
software. Genuine passthrough — an untrapped USERD page and host-owned completions — is **new work,
not a port**. That is achievable and it is the right target; it must simply not be costed as
transcription.

⚠ **And one asymmetry to carry into the Rust port:** in the C **the ring is default-ON and the
observability of the ring is default-OFF**. Every `RANG`, `USERD-WR` and `FORGE` line the design docs
quote came from a **non-default build**.

## 5c. ★★★★★ WHY PASSTHROUGH IS CORRECT BY CONSTRUCTION — the semaphore is embedded in the work

Sourced from NVIDIA, 2026-08-10. This is the load-bearing argument for §1's split, and it is stronger
than any efficiency case.

> **A completion payload is not derivable from the work.** It is a number the guest's *software*
> invented, kept in its own driver state, and embedded as a **literal immediate** in the pushbuffer.
> The hardware reproduces it only because it is in the bytes.

`uvm_channel.c:1508-1512` computes `new_payload = (NvU32)(++tracking_sem.queued_value)`, and that exact
value becomes the `SET_SEMAPHORE_PAYLOAD` immediate at `uvm_turing_ce.c:70-72`. RM does the same from
`lastSubmittedPayload` (`channel_utils.c:839`).

⇒ **If the host GPU executes the guest's bytes, the payload is automatically right. If anything
re-encodes, re-orders or re-generates the work, the payload is unrecoverable** — not hard to compute,
*impossible*: it is private software state we never see.

★★★★ **And the argument does not stop at payloads. Everything that makes a release CORRECT is a
literal in the same bytes:**

| what | where it lives | what happens if we re-derive it |
|---|---|---|
| the **payload** | `SET_SEMAPHORE_PAYLOAD` / `SET_REPORT_SEMAPHORE_C` immediate | unrecoverable; a guess eventually goes backwards |
| the **flush scope** | `LAUNCH_DMA.FLUSH_ENABLE`/`FLUSH_TYPE`, `SET_REPORT_SEMAPHORE_D.FLUSH_DISABLE` | a release becomes visible before the data it gates — NVIDIA fixed a real Ampere bug here |
| the **interrupt arming** | `LAUNCH_DMA.INTERRUPT_TYPE`, `SET_REPORT_SEMAPHORE_D.AWAKEN_ENABLE` | the wake never fires — the exact `cuCtxCreate` hang |
| the **structure size** | `SEMAPHORE_TYPE` / `STRUCTURE_SIZE` | 4 bytes written where 16 were promised; stale timestamp at +8 |
| the **target VA** | the semaphore address fields | resolved in the *executing channel's* VAS — **no aperture override exists in any of the three classes** |

⇒ ★★★★★ **The rule generalises: anything the guest encoded in its bytes is correct only if we run its
bytes.** Re-encoding is not a performance choice with a correctness cost attached — it is a
correctness choice, and it loses every time.

**And it yields a falsifiable invariant we can gate on:** *no code path may compute or write a
semaphore value for a guest-userspace channel.* If we ever find ourselves able to predict a payload,
we are on the wrong path. ⊘ Writing a **kernel** channel's completion stays legitimate (§5) — that
channel is ours, and its payload is ours to know.

⚠ **The failure mode if we get this wrong is not gradual.** UVM keeps the hardware's 32-bit payload in
the low half of a 64-bit counter, so **any decrease is read as a 2³² wrap forward**, exceeds
`UVM_GPU_SEMAPHORE_MAX_JUMP`, and trips `UVM_ASSERT_MSG_RELEASE` — **compiled into release builds** —
which calls `uvm_global_set_fatal_error`. UVM is then dead for that GPU. No retry, no recovery, on the
**first** occurrence. ⇒ **Exactly one writer per semaphore, forever, values non-decreasing.** A bridged
or lagging second writer is a one-shot kill, and that is the measured `M5.38` incident.

## 6. Faults: prevent, don't handle

`mode2_doorbell_chid.md` item 4, owner, 2026-06-05:

> **"Gate the ring** on 'this channel's working set fully mapped' (naive ring today → `cuInit=999`
> because the host faults on unmapped referenced VAs)."

⇒ The design answer to host faults is **do not fault**: refuse to ring until every VA the channel will
reference is mapped in the host channel's VAS. The `kayfabe` port has this gate built and it is
currently **vacuous** — the shell passes an empty working set. Closing it is a **prerequisite** of
passthrough, not a follow-up: under passthrough it is the only thing standing between a hostile ring
and an Xid.

⚠ **Scope, honestly:** prevention covers the **static** working set. It does not cover the guest
changing a mapping under a running channel, nor UVM demand-migration. A host→guest fault **delivery**
path remains undesigned; it is simply not a precondition for first compute.

---

## 7. Standing instruction, from the same June page

> *"**Quarantine** the QEMU-side `nvkvm_chan_execute()` pushbuffer-parse/sema-fake path during the real
> build (**it masks whether the host actually ran the work**)."*

⇒ The `kayfabe` equivalents are the CPU CE executor on the **forwarded** arm, the private GP cursor,
and completion-writing on the forwarded arm. **Quarantine, do not delete** — the emulated arm is
legitimate per §5. Without this, a green run cannot distinguish *"the host ran it"* from *"we faked
it"*, which is precisely the ambiguity that cost this campaign five rungs.
