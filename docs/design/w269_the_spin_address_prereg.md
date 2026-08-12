# w269 PRE-REGISTRATION — **READ THE POLLED ADDRESS.** No device change, no rebuild, no trap

> ### STATUS — 2026-08-12 / **LIVE — PRE-REGISTRATION.** Branch `w269-the-spin-address`, off
> `w268-the-cursor-and-the-arm` = `24ea823`. Predecessor: `traces/boots/w268/RESULT.md`
> (rev `70463ae`, 2 arms, real GA106). Bench `vh` = `NVIDIA GeForce RTX 3060` (GA106), host
> driver `580.159.04`, `libcuda.so.580.159.04` md5 `10e2dd6c89409898ba8c68533cde1432`.
> ⊘ **This rung changes ZERO lines of Rust and rebuilds NOTHING.** The binary under test is the
> one `w268` measured, stamped `kayfabe-rev:70463ae…`, asserted before booting. The only new
> code is a guest-side ptrace probe.

---

## 0. ⊘⊘ WHAT CONTRADICTS THE BRIEF — and the first one costs the rung its item 1

### 0.1 ★★★★★ **ITEM 1 IS ALREADY ANSWERED, ON DISK, IN THE TRACES THE BRIEF HANDS ME.** `cuCtxCreate` blocks — NOT the copies

The brief's item 1 is *"`cup2` is five CUDA calls with prints between them — which one is the
last to print?"*, and its lead hypothesis is that `launches=3` on a CE channel means *"the wall
may have moved past `cuCtxCreate` into the copies."*

⊘ **Refuted by `w268`'s own committed probe logs, both arms, at zero cost:**

```
[traces/boots/w268/run_w268_pass_probe.log:51-62]      [and run_w268_refuse_probe.log:51-62, identical]
ok   cuInit(0)
ok   cuDeviceGetCount(&n)              devices=1
ok   cuDeviceGet(&d,0)
ok   cuDeviceGetName(nm,256,d)         name=
ok   cuDeviceGetAttribute(... MAJOR)   ok   cuDeviceGetAttribute(... MINOR)   compute=8.6
ok   cuDeviceTotalMem(&tot,d)          totalMem=11959 MiB
CUP2_RC=124                            ← and NOTHING between. No `ok   cuCtxCreate`.
```

`tests/mode2/cup2.c`'s `CK()` prints `ok   <expr>` **after** each call returns. The last line is
`totalMem=`, so the call that never returned is **`cuCtxCreate`** — on **both** arms, exactly as
`RESULT.md` §0.4 already states (*"It still reaches `cuDeviceTotalMem` and hangs"*) and exactly as
`scripts/bench/cup2_hook_w232.sh`'s own header has said since `w232` (*"`CUP2_RC=124` (the 180 s
timeout at `cuCtxCreate`)"*).

⇒ ★★★ **The wall has NOT moved into the copies.** `cup2` never reaches `cuMemAlloc`,
`cuMemcpyHtoD` or `cuMemcpyDtoH`. The `methods=11 launches=3` CE work `RESULT.md` §3.2 saw is
**inside `cuCtxCreate`**, not the memcpys — which is unsurprising: context init scrubs and
initialises through CE.
⇒ ⊘ **The brief's item 2 (pin `0x2_04420000`) is therefore NOT gated open by item 1.** The
owner's redirect makes that conditional explicit (*"item 2 waits until this says the copies are
where it stops"*), and the answer is **no**.

★ **Thirteenth consecutive lane to find its brief's premise already answered.** ⚠ And note the
shape: the answer was in a file the brief *names as required reading*, six lines from a number the
brief quotes. Reading `CUP2_RC=124` and not the twelve lines above it is the failure.

### 0.2 ★★★★★ **THE SPIN LOOP WAS DISASSEMBLED AT `w215` AND THE CAMPAIGN STOPPED CITING IT**

The brief says *"The hang is something else, and nobody knows what it is."* ⊘ The **loop** is
known, by disassembly, since `2026-08-10` (`f5f55ad`, "the wall's loop is UNBOUNDED — and OUR
`NV_OK` is what feeds it"). What is unknown is **the predicate's operand**, which is a much
narrower and much more answerable question.

`[verified 2026-08-12, independently re-disassembled at the shipped
`libcuda.so.580.159.04`]` — the wait is `libcuda+0x22bdeb … +0x22c145`:

```
22bde3:  call 318580              ; RESET the elapsed-time anchor at -0x80(%rbp)
22bdeb:  test %ebx,%ebx ; je 22be17 ; two spin flavours: pause-only, or sched_yield+pause (22becf)
22be17:  pause
22be1f:  call f9df90              ; ★ THE PREDICATE.  rdi=%r12, rsi=%r15 (the WAIT OBJECT)
22be28:  je   22bdf8              ; eax == 0  ⇒ NOT DONE
22be05:  mov 0x40(%r13),%rax ; mov 0x72b8(%rax),%edx ; test ; je 22c0d0
22c0d0:  call 23d790 / cudbgUseExternalDebugger  → if set, spin on without the deadline check
22c0f7:  call 3185a0              ; elapsed ms  → %xmm0
22c0fc:  comiss 0x1185a90(%rip)   ; ★ the constant is 0x447a0000 = 1000.0f  (ONE SECOND)
22c103:  jbe  22bdeb              ; ≤ 1 s  ⇒ keep spinning
22c124:  call *0x438(%rcx)        ; the RM control — NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS 0x20801702
22c12c:  je   22bde3              ; ★★★ NV_OK ⇒ jump to the TIMER RESET.  Forever.
```

⇒ ★★★★★ **There is no overall timeout.** The only exit short of the awaited work completing is a
**non-`NV_OK`** from `0x20801702`, which this port answers `NV_OK`. `w216` proved that
constructively: with `KAYFABE_MC_SERVICE_BUDGET=20`, `cuCtxCreate` **returned** (`CUP2_RC=1`,
`CUDA_ERROR_NOT_SUPPORTED`) — the only time in this campaign it has.
⊘ **So no longer deadline can ever answer "does it terminate".** Three rungs paid 150 s hangs for
a falsifier that could not fire; do not pay a fourth.

### 0.3 ★★★★★ **AND THE PREDICATE'S DATA STRUCTURE IS FULLY DECODED — the polled address is READABLE**

`f9df90` is not a black box. `[disassembled 2026-08-12, `f9df90…f9e360`]`:

```
f9df90:  r14 = rsi                       ; the WAIT OBJECT (= %r15 at the call site)
f9dff4:  r11d = 0x10(%r14)               ; ★ N, the number of wait ITEMS
f9e009:  r15 = 0x5915f54                 ; the 17-entry jump table
f9e010:  rdx = 0x18(%r14) + i*40         ; ★ the ITEM ARRAY: base 0x18(%r14), STRIDE 40 (0x28)
f9e01e:  eax = (%rdx)                    ; ★ item[0x00] = KIND, 0..0x10, dispatched
```

The seventeen table entries resolve to exactly **five** distinct handlers (the other twelve are the
`f9e0d0` "not ready / next item" continue):

| kind | handler | what it reads |
|---|---|---|
| 6, 16 | `f9e0c0` / `f9e170` | `v = *(u32*)item[0x08]`; **done when `v - item[0x10] >= 0`** (signed) |
| 3 | `f9e1b0` | ★★★ `v = *(u32*)( *(u64*)( *(u64*)(item[0x08]+0x18) + 0x10 ) )`; **done when `v - (4*item[0x10] + 2) >= 0`** (signed). Caches `v` at `item[0x08]+0x20`; a 64-bit monotonic mirror lives at `item[0x18]+0x9428` under a `lock cmpxchg`, with a second word pointer at `item[0x18]+0x9430` |
| 4 | `f9e190` | `call 4029e0(item[0x08]+0x18, item[0x10])` — a nested wait |
| 1 | `f9e320` | `call 4029e0(item[0x08]+0x9410, item[0x10])` — a nested wait |

⇒ ★★★★★ **The polled address, the value read, and the value compared against are all three
recoverable from the target's own memory with `ptrace`, at the instant of the spin.** That is
precisely the owner's items 2 and 3, and it needs **no trap, no device change and no rebuild**.

### 0.4 ⊘ THE INSTRUMENT THE BRIEF NOMINATES IS THE WRONG ONE, AND ITS SUCCESSOR IS ALREADY IN THIS TREE

`tests/mode2/gcup2_gdb.sh` (the C repo) is written for a **`SIGSEGV`**: `handle SIGSEGV stop
nopass` + `run` + `$_siginfo._sifields._sigfault.si_addr`. It never interrupts a *hang* and it
prints nothing at all if the program does not fault. ⊘ It cannot answer this question as written.

`scripts/bench/guest_userstack.c` (this tree, `79ed443`) **does** ptrace a hung `cup2` and resolve
`RIP` through the target's own `/proc/<pid>/maps` — and its header already records why
`/proc/<pid>/stack` is the wrong instrument here (`state=Rl`, empty kernel stack: a **userspace**
spin). ⇒ `w269`'s probe is that program **extended**, not a new one.

---

## 1. THE INSTRUMENT — `scripts/bench/guest_spinprobe.c`, and its rate limit is STRUCTURAL

★ The owner's warning is the load-bearing half: *"do NOT enable a read trap in the device … a
busy-poll would emit millions of lines."* This probe **cannot** emit a million lines, by
construction and not by hope:

1. **`PTRACE_SINGLESTEP` with a fixed budget** (default 6000 steps, `argv`-settable). The loop
   body is ~10 instructions plus `f9df90`'s ~60, so a few hundred steps suffice; the budget is a
   ceiling, and **the number of steps actually taken is printed**.
2. **A RIP histogram capped at 96 distinct buckets**, top 24 printed, **and the count of buckets
   dropped is printed**. ⊘ A silent truncation reads as coverage; this one names what it dropped.
3. **One register snapshot**, at the first step whose `RIP` equals a wanted offset. Not per-access.
4. It **never writes** to the target: no `POKETEXT`, no breakpoint insertion, so it cannot corrupt
   `libcuda`'s code and cannot be blamed for a behaviour change.

⚠ It *does* perturb timing — single-stepping a spin is ~1000× slower. That is bounded to the
budget and, because the wait has **no overall timeout** (§0.2), it cannot change the outcome.
⊘ What it cannot rule out: a *timing-sensitive* race elsewhere in the process.

**What it prints**, per thread and then for the chosen thread:
`RIP` module+offset, `RSP`, `orig_rax` (⚠ the owner's item 4: **if `orig_rax >= 0` and the state
is `S`, it is a blocking syscall and the whole memory-poll reading is void — that is asserted, not
assumed**), the histogram, and at the snapshot the decoded **wait-item array** with, for every
item, `kind`, the five words, the **polled address**, **the value at it**, **the threshold**, and
the **`/proc/<pid>/maps` row the address falls in**.

---

## 2. THE ARMS — the same two as `w268`, one variable, ZERO rebuild

| tag | `GR_ROUTE` | why |
|---|---|---|
| `w269_refuse` | `refuse` (unset) | the shipping configuration; the GR completions are **NOT** written |
| `w269_pass` | `passthrough` | `w268`'s arm; all eight GR completion slots **ARE** written, payload `1` |

Everything else (`FB_JOIN=shared`, `GUEST_RING=ring`, `GUEST_PUSHBUF=pin`, `PT_WITNESS_EXEC=on`,
`GUEST_SEMA=pin`) is identical on both arms and identical to `w268`.

★★★ **Why both arms and not just `pass`:** the polled address is only interpretable against a
control. If the two arms poll the **same address** with the **same value**, then the guest is not
looking at the page we filled and `w268`'s eight satisfied slots are irrelevant to this wait — a
much stronger statement than either arm alone can make.

⊘ **No build.** `w269_run.sh` asserts `strings qemu-system-x86_64 | kayfabe-rev:` is exactly
`70463ae329adac543de59b36da38112a4044fdeb` and **refuses to boot otherwise**. ⇒ this rung cannot
be the *"the bench silently served a binary built from `862c7c2`"* trap, because it never builds.

---

## 3. PREDICTIONS — registered before the probe exists

⚠ The brief asks me to think about `CUP2_RC` fresh rather than inherit the eight-rung streak of
predicting zero. Having read §0.2, the streak is not a habit: **the loop has no exit**. A
prediction of `124` here is a *derivation*, not an inheritance.

| # | prediction | p |
|---|---|---|
| **P1** | `CUP2_RC = 124` on **both** arms, and the last printed line is `totalMem=11959 MiB` on both. **Size: `124` exactly, and `cup2`'s stdout is 8 lines / ~150 bytes, byte-identical between arms modulo nothing.** ⊘ Derived from §0.2: no non-`NV_OK` reaches `0x20801702`, so no exit exists. | **0.93** |
| **P2** | The spinning thread is `cup2`'s **main** thread, `state=R`, `orig_rax` = `-1`/`228` (`clock_gettime`, via the vDSO on the elapsed-ms path), **not** a blocking syscall ⇒ the memory-poll reading is valid. | 0.85 |
| **P3** | The RIP histogram's mass lands in `libcuda+0x22bd80…+0x22c150` and `+0xf9df90…+0xf9e380`, i.e. **the loop `w215` named, unchanged at this revision**. | 0.80 |
| **P4** | The snapshot fires: `RIP` reaches `libcuda+0x22be1f` **or** `+0x22bedc` within the budget. | 0.85 |
| **P5** | `N = *(u32*)(%r15+0x10)` is **small and non-zero** — `1` or `2`. | 0.70 |
| **P6** | ★★★ At least one item has **`kind == 3`** (the semaphore-progression handler). | 0.60 |
| **P7** | ★★★★★ **THE POLLED ADDRESS IS THE SAME ON BOTH ARMS, AND THE VALUE AT IT IS THE SAME.** ⇒ the wait is **not** on the page `w268` filled. | **0.55** |
| **P8** | ★★★★ The polled address's **page offset** (`addr & 0xfff`) is **NOT** in `{0xf80,0xf90,…,0xff0}` (the eight GR `SET_REPORT_SEMAPHORE` slots) and **NOT** in `{0xf00…0xf70}` (the CE slots). ⇒ a **mismatch**, which the owner's brief names as equally informative. | 0.65 |
| **P9** | The polled address falls in a mapping that is **anonymous or `/dev/nvidia-uvm`**, not `/dev/nvidia0`. | 0.55 |
| **P10** | ⊘ **The low-probability arm, deliberately widened** (three consecutive rungs had their least-weighted arm fire): the address **IS** one of the eight GR slots and the value read is **`1`** — i.e. the completion landed and the compare still fails because the threshold is `4*item[0x10]+2 > 1`. That outcome would mean the guest asked for a payload we do not produce, and would be the single most actionable result available. | **0.15** |
| **P11** | ⊘ `gdb` is present in the guest or installable; if not, the C probe alone carries the rung. Either way **something** is measured. | 0.90 |

### ★ WHAT EACH OUTCOME OF THE ADDRESS READ MEANS — named before the run

- **Address ∈ the eight GR slots, value ≥ threshold** ⇒ the wait is satisfied and the guest is not
  re-reading it: a **cache/coherency** defect on the guest's CPU view. `RESULT.md` §4.5's unpaid
  limit (*"`gpa_read` reads the VMM's view of the memfd"*) becomes the finding.
- **Address ∈ the eight GR slots, value < threshold** ⇒ ★ we write the slot but not the *payload
  the guest wants* (P10). Actionable immediately.
- **Address ∉ any slot, same on both arms** ⇒ ★★★ **a THIRD kind of wall** (the brief's item 3):
  not addressing, not completion. Naming the object it belongs to is then worth more than any fix.
- **`kind == 4` or `1` (a nested wait)** ⇒ the poll is one level deeper and this rung's decode
  stops at the handler; that is a **partial** answer and will be reported as one.
- **`orig_rax >= 0` in state `S`/`D`** ⇒ ⚠ **not a memory poll at all**; items 2 and 3 are void and
  the rung reports the syscall instead.

---

## 4. ⊘ WHAT THIS RUN WILL NOT BE ABLE TO PROVE — written before it runs

1. ⊘ **That the polled address is the *only* thing the wait needs.** `f9df90` walks `N` items and
   returns non-zero only when *every* one is satisfied; a single unsatisfied item explains the
   spin without being the only unsatisfied one.
2. ⊘ **Anything about the GPU-side VA.** The probe reads a **guest process VA**. Mapping it to the
   `0x2_0440ff80` GPU VA is done by **page offset and mapping name only** — suggestive, never
   conclusive.
3. ⊘ **That the value we read is the value the CPU sees at full speed.** A single-stepped read is
   still the CPU's view, but ordering/caching effects finer than one sample are invisible.
4. ⊘ **Correctness of the GR work.** Carried unpaid from `w268` §4.1.
5. ⊘ **That `refuse` and `pass` differ only in the route.** They differ in *everything downstream*
   of the route; the arm is one variable, its consequences are not.
6. ⊘ `GR-CURSOR-READER stopped` / `PAGE-READER ASSERTION` are **still** expected to FAIL (`w267`
   §3.2, `w268` §3.3, unpaid a third time). This rung does not fix teardown and says so up front.
