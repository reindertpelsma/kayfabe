# w272 — ⊘⊘⊘ **F6 IS NOT ON THIS PATH.** `RcTriggered` cannot fire, because nothing calls it and nothing observes the fault; the leaf is not masked; the event index an RC would use was never armed

> ### STATUS — 2026-08-12 / **LIVE — REFUTED, NO BOOT SPENT.**
> Method: **static analysis at `416088c`** (this branch's base) + **re-reading committed
> boot evidence** from `traces/boots/w270/`, which is the boot that took the live `Xid 31`.
> ⊘ **No GPU time was spent and no vast box was created** — see §0.4 for why that is the
> honest answer rather than a shortcut, and §4 for what it therefore cannot prove.
>
> **Provenance.** The w270 binary was built at source revision **`1b64729`**; the source
> read here is **`416088c`**. `git diff --name-only 1b64729 416088c` touches only
> `crates/kayfabe-abi/src/capability.rs`, `scripts/bench/w270_run.sh` and
> `traces/boots/w270/**` — ⇒ **every file on the fault path is byte-identical between the
> binary that took the `Xid` and the source adjudicated here.** Box identity of the cited
> evidence: bench **`vh`**, `NVIDIA GeForce RTX 3060` (GA106), host driver **580.159.04**.
> Branch `w272-the-announcement`, worked in a **separate git worktree** (§0.5).

---

## 0. ⊘⊘ LEAD WITH WHAT CONTRADICTS THE BRIEF — five things, and the first cost nothing

### 0.1 ★★★★★ **THE BRIEF'S PREMISE WAS REFUTED THE DAY BEFORE THE BRIEF WAS WRITTEN**

The brief hands me task #235 as *"still open, measured `w212`"*: **"F6 — THE ANNOUNCEMENT IS
MASKED … the enable bookkeeping is proven SOUND, so the masked leaf is REAL."**

⊘ `33c66f0`, **2026-08-11**, `w257`: *"the MASKED LEAF is NOT the wall — refuted from
NVIDIA's source, from the guest's own words, and by a boot at HEAD's own revision."* Its
argument is a comment in the driver we are impersonating:

> `intrGetPendingStallEngines_TU102`, `ogkm-580: intr_tu102.c:895-899` — *"We skip checking
> if it is enabled in the leaf register since we mess around with the leaf enables in the
> interrupt disable path"*

and `intrGetLeafStatus_TU102` (`:1108-1141`) fills its array with `intrReadRegLeaf_HAL` and
**never calls `intrReadRegLeafEnSet`**. The stall test is `leaf & NVBIT(leafBit)` — the raw
latch, ANDed with nothing.

⇒ ★★★ **`would be masked` is a TRUE reading of a predicate the guest never evaluates** on the
stall path. This is the sixteenth consecutive lane to find its premise stale, and the
staleness was **one day old and already committed in this repository**.

### 0.2 ★★★★ **ALL THREE OF THE BRIEF'S F6 NUMBERS ARE `w212`'s, AND `w270` INVERTS THEM**

The brief quotes *"1 stall vector raised / 1 would be masked, 179 completions UNVECTORED."*
That is `w212` (`aae43e7`, 2026-08-10), **~99 commits back**. `[measured, the boot the brief
itself hands me]` `traces/boots/w270/run_w270_pin_qemu.log:1613-1615`, verbatim:

```
completions:       4 announced (non-stall vector raised), 193 UNVECTORED, 4 would be masked
os-events:         3 registered / 3 retired / 0 live (0 malformed, 0 refused-full);
                   48 POST_EVENT posted in 16 batch(es);
                   gate: 99 gated, 0 not-running, 0 failed, 15 IRQSCLR cleared
os-event announce: 16 GSP stall vector(s) raised, 0 UNVECTORED, 3 would be masked;
                   0 batch(es) WOKE WITH NOTHING
```

| reading | `w212` (the brief's) | `w270` (the live-fault boot) | |
|---|---|---|---|
| os-event stall vectors raised | **1** | **16** | ★ |
| of those, UNVECTORED | — | **0** | ★★ |
| `IRQSCLR` cleared | **0** | **15** | ★★★ **the guest's ISR attributed and cleared them** |
| `POST_EVENT` posted | 1 in 1 batch | **48 in 16 batches** | ★ |
| gated | 348 | **99** | the gate is open |

⇒ ★★★★ **The announcement plane is not broken and not masked — it is demonstrably
round-tripping.** `15 IRQSCLR cleared` is the guest's *own* acknowledgement: you cannot clear
a leaf you did not attribute. The brief's *"right now we produce neither"* is false for the
os-event half.

★ **And the instrument's own known-positive passes in the same line** (`:1612`): `interrupt
tree: 9962 register accesses, 2 guest LEAF_TRIGGER raises, 0 of them would be masked`. The
report's own falsifier is *"if that equals the raises, the ENABLE BOOKKEEPING is blind"* —
`0 ≠ 2`, so the masking numbers mean what they say.

### 0.3 ★★★★★ **THE `Xid` IS A *HOST-PROCESS* FAULT. THE GUEST IS NOT A PARTY TO IT**

`[measured, `traces/boots/w270/run_w270_{off,pin}_hostdmesg.log`, both files non-empty and
asserted so]`:

```
off: NVRM: Xid (PCI:0000:00:07): 31, pid=2244266, name=memfd:kayfabe-i, channel 0x01000011,
     … faulted @ 0x2_04420000 … FAULT_PTE ACCESS_TYPE_VIRT_WRITE
pin: NVRM: Xid (PCI:0000:00:07): 31, pid=2246177, name=memfd:kayfabe-i, channel 0x01000011,
     … faulted @ 0x2_04428000 … FAULT_PTE ACCESS_TYPE_VIRT_WRITE
```

⇒ ★★★ **`pid=…, name=memfd:kayfabe-i` is OUR ISOLATE**, and `channel 0x01000011` is a **host**
channel handle. This is the *host* driver telling the *host* kernel log that a channel the
isolate owns walked the *host* MMU into nothing. The guest's emulated GPU performed no walk
and has nothing to report.

⇒ ⊘⊘ **The brief's framing — *"maybe a fault is signalled and we never deliver it"* — has the
wrong subject.** Nothing signals us. There is no fault *in the guest's world* that is being
withheld; there is a fault in **ours** that no component of ours observes (§1.3).

### 0.4 ⊘ **NO BOX WAS SPUN, AND THAT IS THE FINDING, NOT A SHORTCUT**

Every one of the brief's three questions is answerable from artefacts already committed —
and, decisively, **from the very boot that produced the live fault the brief wants measured
against**. A new boot would re-derive `w270`'s own numbers at the cost of GPU time and a
second provenance surface the brief itself flags as the risk of running two boxes.

★ This is `check_whether_the_question_is_already_answered` and `the_bar_was_already_met_
before_the_rung` firing together. The measurement the brief asks for **had already been
taken and committed**; what was missing was reading it.

### 0.5 ⊘⊘ **I NEARLY BUILT OVER THE OTHER LANE'S UNCOMMITTED WORK**

`/workspace/nvkvm-rs` is a **shared checkout** and it was sitting on branch
`w271-the-extent-key` with **three uncommitted modified files** (`kayfabe-fwd/src/lib.rs`,
`kayfabe-fwd/src/trace.rs`, `kayfabe-qemu-raw/src/shim.rs`, 387 insertions) — the extent-key
lane's in-flight work. The brief says *"work in `/workspace/nvkvm-rs`"* and says nothing
about it being occupied.

I created a branch there and ran `cargo check`, which **failed with `BASE_RC=101`** —
`PinnedRun` and `kayfabe_arch` unresolved in `shim.rs`. ⚠ **That failure is the other lane's
work-in-progress, not a defect of `416088c`**, and reading it as *"HEAD does not compile"*
would have been a false and alarming report. Restored to `w271-the-extent-key` with all three
files intact, and all subsequent work done in a **separate `git worktree`**
(`/workspace/nvkvm-rs-w272`) reading committed blobs via `git show`/`git grep <rev>`.

⇒ ★★ **"Parallel lanes get their own box" does not imply their own checkout.** A dirty shared
tree makes *any* build result unattributable — the same class as the `862c7c2` provenance
trap, arriving through the working tree instead of the binary.

---

## 1. LEG 1 — **Does `RcTriggered` fire for the channel that took the `Xid`? NO, AND IT CANNOT.**

Three independent proofs; the first is structural and the strongest.

### 1.1 ★★★★★ The emitter's REQUIRED ARGUMENT has no producer anywhere

`rc_triggered_for` (`crates/kayfabe-rmrpc/src/fault.rs:165`) takes `except_type: u32`, and its
own doc names the only legal value for an MMU fault:
`kayfabe_abi::generated::rpc::ROBUST_CHANNEL_FIFO_ERROR_MMU_ERR_FLT`.

`[measured, `git grep -n ROBUST_CHANNEL_FIFO_ERROR_MMU_ERR_FLT 416088c -- 'crates/*.rs'`]` —
**8 hits, and every one is the generator, the constant's own definition, or a doc comment.**
Zero runtime uses.

⇒ ★★★ **No code anywhere ever holds the argument the emitter cannot be called without.** That
is a stronger statement than "no call site": it survives any indirection, because the value
would still have to be produced somewhere.

### 1.2 ★★★★ The whole chain is orphaned, and grep is COMPLETE here

`[measured at `416088c`]`, the #111 chain end to end:

| verb | file | non-test callers |
|---|---|---|
| `kayfabe_fwd::fault_facts` (the derivation site) | `kayfabe-fwd/src/lib.rs:6676` | **0** — and **0 in tests too**; only its own definition |
| `kayfabe_core::fault::verdict` (the decision) | `kayfabe-core/src/fault.rs:277` | **0** — every other hit is a doc comment |
| `kayfabe_rmrpc::rc_triggered_for` (the encoder) | `kayfabe-rmrpc/src/fault.rs:165` | **0** — definition, one `pub use` re-export (`lib.rs:197`), one comment |
| `FaultEmission::deliver` (the transport) | `kayfabe-rmrpc/src/fault.rs:119` | **0** |

⚠ **The gate's own warning is that text search is not the verdict** (`scripts/orphan_gate.sh`:
*"`MapGuestRam` greps as zero callers and runs 8× per boot"*). ★ **That warning does not
reach here, and the reason is checkable:** it applies to trait methods and dispatched verbs.
These four are **free functions and one inherent method** — Rust has no way to call one
without the identifier appearing at the call site. The only escape hatch is macro-constructed
names, and `git grep -n "concat_idents\|paste!\|paste::" 416088c -- 'crates/*.rs'` returns
**empty**. ⇒ The identifier search is complete over this revision *for these symbols
specifically*.

⊘ **What I did NOT do, stated plainly:** I did not run the compiler adjudication
(`scripts/orphan_gate.sh`). It needs a build, the only warm `target/` is the other lane's
(19 GB, 9.1 GB free on the box), and exhausting that disk would have broken a lane that is
mid-flight. The structural proof in §1.1 does not depend on it.

### 1.3 ★★★★★ Nothing in this process can even LEARN that the host GPU faulted

`[measured]` over `crates/kayfabe-isolate/src/` and `crates/kayfabe-isolate-host/src/`, for
`NV01_EVENT` / `EVENT_OS_EVENT` / `SET_NOTIFICATION` / `NV2080_NOTIFIERS` / `0x0079` /
`errorNotifier` / `error_notifier`: **zero hits.**

⇒ ★★★ **The isolate registers no event with the host driver and reads no host error
notifier.** The only reader of a host `Xid` in this repository is a **shell script**,
`scripts/bench/host_xid_watch.sh`, scraping host `dmesg` out-of-band.

⇒ Even if the entire #111 chain were wired, **there is no input that would trigger it for this
fault.** The design's trigger is *our own* refusal — `simulated_gpu_fault.md` §1: *"an unmapped
VA in a submission's working set is refused by the #14 ring-gate (`kayfabe_fwd::plan_doorbell`
→ `VerbPlan::gated_doorbell` → `FwdFault::Address(AddressFault::Miss)`)"*. On the `w270` arms
we did **not** refuse: we admitted, pinned and forwarded, and hardware faulted *downstream* of
our gate. **The #111 path is designed for the case where we say no. This is the case where we
said yes and were wrong.**

### 1.4 ★★★ And the boot itself says so, with a known-positive beside it

`[measured, `run_w270_pin_qemu.log`]` — grep for `escalate|NotAttributable|AddressFault|
RcTriggered|RC_TRIGGERED|error notifier|ErrorNotification|quarantine`: **one hit**, and it is
the device's own audit line (`:1845`) confirming the design, not an event:

> *"⊘ shadow-queue PUSH is UNBUILT: … this port never enqueues a fault packet, so a
> non-replayable fault surfaces as an RC on the channel plus an error notifier
> (`simulated_gpu_fault.md` 5.2, the deliberate choice) and NEVER as a queue entry"*

★ **A census zero with a known-positive in the same report**: the same log carries `48
POST_EVENT posted in 16 batch(es)`. ⇒ The device posts events; it posted **no RC**.
⚠ Note the `faults 2` in `:1613`'s `registers:` line is **not** this — it is the
register-plane's `GspFault` counter (`shim.rs:1873`), a different plane entirely. A count
named `faults` beside a fault investigation is exactly the substitution trap; graded, and it
is not ours.

---

## 2. LEG 2 — **Is that channel's event leaf masked? NO — and the question is not evaluated anyway.**

Two answers, and they agree.

1. **Numerically**, on the live-fault boot: `os-event announce: 16 … raised, 0 UNVECTORED,
   3 would be masked`, with **`15 IRQSCLR cleared`**. Not a masked plane — a serviced one.
   ⚠ The `3 would be masked` is a real, non-zero reading and I am not hiding it; but it sits
   beside `0 UNVECTORED`, so nothing was withheld.
2. **Semantically**, per `w257` (§0.1): the guest's **stall** scan never consults `LEAF_EN`.
   ⇒ `would be masked` is unfalsifiable-by-consequence on this path.

⊘ **Per-channel was not re-checked, because there is nothing per-channel to check**: the
device raises **one** GSP engine stall vector for the os-event plane, not a per-channel leaf.
The brief's *"is that channel's event leaf masked"* presupposes a per-channel leaf that this
device does not model.

★ The one place `LEAF_EN` **is** consulted by the guest is the **non-stall** scan
(`intr_nonstall_tu102.c:254-255, :305-306, :455-456, :486-487`), which is the
**completions** plane — `4 announced, 193 UNVECTORED, 4 would be masked`. ⊘ **And that plane
is irrelevant to this wall**, because `w269` measured the guest **polling**, not waiting on an
interrupt: `AWAKEN_ENABLE=0`, corroborated by the native capture.

---

## 3. LEG 3 — **Is an event registered? YES — but never index 37, the one an RC would use.**

### 3.1 ★★★★★ The arming census, from the live-fault boot

`[measured, `run_w270_pin_qemu.log:1826-1831`]` — **six** armings, all accepted
(`result 0x00000000`):

```
arming event 194 action 2 client 0xc1e00002 object 0xcaf00001 result 0x0 x1
arming event  35 action 2 client 0xc1e00005 object 0x0000000b result 0x0 x1
arming event  35 action 2 client 0xc1e00006 object 0x0000000c result 0x0 x1
arming event 194 action 2 client 0xc1e0000d object 0xcaf00001 result 0x0 x1
arming event  35 action 2 client 0xc1e00010 object 0x0000000b result 0x0 x1
arming event  35 action 2 client 0xc1e00011 object 0x0000000c result 0x0 x1
```

Resolved against `ogkm-580: src/common/sdk/nvidia/inc/class/cl2080_notification.h`:

| index | name | armed here |
|---|---|---|
| 35 | `NV2080_NOTIFIERS_FIFO_EVENT_MTHD` | **4×** |
| 194 | `NV2080_NOTIFIERS_POWER_RESUME` | **2×** |
| **37** | **`NV2080_NOTIFIERS_RC_ERROR`** | ⊘ **ZERO — never armed, in either arm** |

⇒ ★★★★ **The guest never asks to be told about an RC in this boot.** So the brief's
mechanism — *"an unmasked `RC_TRIGGERED` would make libcuda return an error instead of
spinning"* — has **no armed consumer at the client layer** even if every earlier link were
built. ⚠ Stated precisely: an `RC_TRIGGERED` RPC goes to the guest *kernel* RM, which would
still tear the channel down; but the client-visible `RC_ERROR` callback the argument leans on
is not armed.

### 3.2 ★★★★ What the helper threads are waiting on — and why it is not this

`[measured, `traces/boots/w269/run_w269b_refuse_probe.log:77-93`]`:

```
    TID STAT %CPU WCHAN                  COMMAND
   1758 R    …    (main)                 cup2        orig_rax=228  RIP=[vdso]+0xbb2
   1759 Sl   0.0  do_poll.constprop.0    cuda00001400006   orig_rax=7   (poll)
   1978 Sl   0.1  do_poll.constprop.0    cup2              orig_rax=7   (poll)
```

Syscall numbers resolved against `/usr/include/x86_64-linux-gnu/asm/unistd_64.h` on the box
rather than from memory — **`__NR_poll 7`, `__NR_restart_syscall 219`, `__NR_clock_gettime
228`**, and `__NR_clock_nanosleep` is **230**.

- ★★ **`orig_rax=228` is `clock_gettime`, and the `RIP` is in the vDSO** — `state=R`, a
  userspace spin, not a sleep and not a poll.
  ⚠ ⊘ **I first wrote `clock_nanosleep` here from recall and it is wrong**; the brief calls it
  *"a userspace poll"*, which is right in spirit and wrong in mechanism. Both readings would
  have survived review, because *"228 = a waiting syscall"* is the kind of claim nobody
  re-derives. `suspect_the_instrument_first` applies to the analyst's memory too.
- ★★★★ **And the corrected reading CORROBORATES `w215`.** A spin loop calling `clock_gettime`
  through the vDSO on every iteration is a loop **reading its own deadline** — precisely the
  `0x447a0000` = `1000.0f` deadline constant `w269` §0.2 disassembled at
  `libcuda+0x22bdeb…+0x22c145`. Two independent instruments, the disassembly and the live
  `orig_rax`, describing the same loop.
- **The two helper threads are in `poll(2)`** (`orig_rax=7`, `WCHAN=do_poll`) — one is
  libcuda's event thread `cuda00001400006`. ★ **This is the os-event plane, and it is
  exactly where an announcement would land.** It is armed (§3.1, index 35) and the plane
  round-trips (§0.2, `15 IRQSCLR cleared`).

⇒ ★★★★★ **The blocked thread and the stuck thread are different threads.** Waking the
`poll()`-ing helper does not advance `cuCtxCreate`: the main thread is not waiting on an
os-event at all — it is re-reading `0x2_0440ff70` until it holds `2`. **An announcement,
delivered perfectly to a thread that is armed for it, is delivered to the wrong thread.**

---

## 4. ⊘ WHAT THIS RUN CANNOT PROVE — stated in full

1. ⊘ **No boot of my own.** Every number here is `w270`/`w269`'s, re-read. If those captures
   are wrong, so is this. They carry stamp-gated provenance; I did not re-verify the stamps
   beyond reading them.
2. ⊘ **No compiler adjudication** (§1.2). The orphan claim rests on identifier completeness
   for free functions plus the absence of name-constructing macros — sound, but it is not
   `scripts/orphan_gate.sh`'s verdict, and the gate exists because text was wrong seven times.
3. ⊘ **I cannot prove an RC would NOT help if it were wired.** I prove it does not fire, has
   no trigger, and has no armed client consumer. Whether a hypothetical, fully-wired RC would
   unstick `cuCtxCreate` is **untested** — §3.2 is a strong argument that it would not reach
   the spinning thread, not a measurement of a delivered RC.
4. ⊘ **`3 would be masked` is unexplained.** Non-zero, on a plane whose consumer ignores the
   predicate. It is consistent with everything here but I did not chase it.
5. ⊘ **Nothing here addresses the extent-key hypothesis** the other lane is testing. These
   were independent by construction and this result does not bear on it either way.
6. ⊘ **`NV2080_NOTIFIERS_RC_ERROR` absence is scoped to this workload** (`cup2` under
   `w270`'s two arms). A different CUDA program, or a later stage of the same one, may arm it.

---

## 5. ★★ THE ADJACENT FACT, NAMED AND **NOT** BUILT

The brief's real goal is *"make libcuda return an error instead of spinning."* ⊘ `RcTriggered`
is not the short route to it. `w269` §0.2 already disassembled the loop
(`f5f55ad`/`w215`, re-verified at `libcuda.so.580.159.04`, md5 `10e2dd6c…`):

> the wait is `libcuda+0x22bdeb…+0x22c145`; the deadline constant at `0x1185a90` is
> `0x447a0000` = **1000.0f**; and **the only non-completion exit is a non-`NV_OK` from
> `0x20801702` `MC_SERVICE_INTERRUPTS`** — which we answer `NV_OK`, resetting the clock
> forever.

⇒ The loop already **has** a bounded-failure exit, and this port holds it shut. `w270`'s
pre-registration explicitly values that outcome: *"`1` — `cuCtxCreate` returns an error. ★
Also progress and it must be reported as such: a bounded failure names the next thing, where a
hang does not."*

⊘ **I am not building it, and it is not obviously right to build**: answering non-`NV_OK` to
`MC_SERVICE_INTERRUPTS` is a lie about a control that succeeded, and this port's `0x56`
vocabulary is `NOT_SUPPORTED`, not "your channel died". It is recorded here so the next rung
starts from the *measured* exit rather than the assumed one.

---

## 6. ⇒ THE VERDICT ON #235

**F6 is refuted as a cause of the `cuCtxCreate` hang, and task #235 should be closed as
NOT-ON-PATH rather than left open.** The four claims that carried it are each false at
`416088c`:

| #235's claim | status |
|---|---|
| the announcement is masked | ⊘ **false** — the guest clears `IRQSCLR` 15× (§0.2); the stall scan never reads `LEAF_EN` (§0.1) |
| the stall vector is not raised | ⊘ **false** — 16 raised, 0 unvectored (§0.2) |
| the error path is BUILT | ⊘ **half-true and the dangerous half** — it is *visible* (`pub`, re-exported, unit-tested) and **entirely uncalled** (§1.2). ★ `the_orphan_gate_asks_visibility_not_reachability`, again |
| an unmasked `RC_TRIGGERED` would unstick libcuda | ⊘ **unsupported** — no trigger observes the fault (§1.3), index 37 is unarmed (§3.1), and the spinning thread is not the armed one (§3.2) |
