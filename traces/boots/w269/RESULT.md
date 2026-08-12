# w269 — ★★★★★ **THE POLLED ADDRESS IS `0x2_0440fff0` ITSELF.** The guest polls the exact slot we write, the arm CONSUMED seven of eight, and the wall moved from *"8 awaiting 1"* to *"1 awaiting 2"*

> ### STATUS — 2026-08-12 / **LIVE — MEASURED.** Four boots on bench `vh`, `NVIDIA GeForce RTX
> 3060` (GA106), host driver `580.159.04`. **Pass 1** (`w269_refuse`, `w269_pass`) ran the
> **`w268` binary unchanged**, `kayfabe-rev:70463ae…`, asserted before booting and **built by
> nothing** — the only change was a guest-side `ptrace` probe. **Pass 2** (`w269b_refuse`,
> `w269b_pass`) added the owner's doorbell-store witness and rebuilt at
> `kayfabe-rev:4b23d0b…`. Graded against `docs/design/w269_the_spin_address_prereg.md`,
> committed at **`92aa6da`, before `guest_spinprobe.c` existed**. Branch
> `w269-the-spin-address`.

---

## 0. ⊘⊘ LEAD WITH WHAT CONTRADICTS THE BRIEF — three things, and the first two cost no GPU time

### 0.1 ★★★★★ **THE BRIEF'S ITEM 1 WAS ALREADY ANSWERED ON DISK.** `cuCtxCreate` blocks — not the copies

The brief proposed *"`launches=3` on a CE channel is consistent with the memcpys — the wall may
have moved past `cuCtxCreate` into the copies"* and asked which call is the last to print.
⊘ **`traces/boots/w268/run_w269_{pass,refuse}_probe.log:51-62`, both arms, already said:**

```
ok   cuInit … ok   cuDeviceTotalMem      totalMem=11959 MiB
CUP2_RC=124                               ← and NOTHING in between. No `ok   cuCtxCreate`.
```

`cup2.c`'s `CK()` prints **after** the call returns, so the call that never returned is
**`cuCtxCreate`**; `cuMemAlloc` and both memcpys are never reached. ⇒ The `methods=11
launches=3` CE work is **inside `cuCtxCreate`** (context-buffer setup).
★ **Two independent refutations now**: this one, and the native `cup2` capture
(`7c90f89`, C repo) showing a 4-byte `cuMemcpyHtoD` uses **no copy engine at all** — it is
`NVC7C0` inline-to-memory, `ce_launch_dma = 0`. Either alone kills the memcpy hypothesis.
⇒ ⊘ **The brief's item 2 (pin `0x2_04420000`) never opened**, and the owner's conditional
resolves to *no*.

### 0.2 ★★★★ **"NOBODY KNOWS WHAT IT IS" WAS TOO STRONG — `w215` disassembled the loop on 2026-08-10**

`f5f55ad`. Re-verified independently today at the shipped `libcuda.so.580.159.04` (md5
`10e2dd6c…`): the wait is `libcuda+0x22bdeb…+0x22c145`, the deadline constant at `0x1185a90` is
`0x447a0000` = **1000.0f**, and the only non-completion exit is a **non-`NV_OK`** from
`0x20801702` `MC_SERVICE_INTERRUPTS` — which we answer `NV_OK`, resetting the clock forever.
⇒ ⊘ **No longer deadline can ever answer "does it terminate."** `w216` proved it constructively.
What was genuinely unknown was the **predicate's operand**, and that is what this rung read.

### 0.3 ⊘ **THE INSTRUMENT THE BRIEF NOMINATED CANNOT ANSWER A HANG**

`tests/mode2/gcup2_gdb.sh` is built for a **`SIGSEGV`** (`handle SIGSEGV stop nopass`, then
`$_siginfo…si_addr`). On a hang it prints nothing. ⊘ And **`gdb` is not installed in the guest
and could not be installed** — the C probe stands alone; the cross-check does not exist.

---

## 1. ★★★★★ THE ANSWER — the polled address, read out of the spinning process

`[measured, w269b_refuse, `run_w269b_refuse_probe.log`]`, all eight items, verbatim:

```
item[0] KIND=1 [0x08]=0x7f84706fe010 [0x10]=0x1
    S[0x10] limit  = 0x1     (wanted is within the limit)
    S[0x18] cached = 0x0     ★ wanted > cached ⇒ NOT satisfied; it reads
    S[0x20] desc   = 0x5b62341a82c0
    POLLED ADDRESS = 0x20440fff0   pageoff=0xff0   nvidiactl+0x400fff0  <rw-s /dev/nvidiactl>
                     VALUE AT IT = 0x00000000 (0)
                     SLOT-JOIN: ★★★ MATCHES a GR SET_REPORT_SEMAPHORE slot page-offset
    chain          : item[0x08] +0x9410 -> S +0x20 -> 0x5b62341a82c0 +0x10 -> 0x20440fff0
item[1] … 0x20440ffe0     item[2] … 0x20440ffd0     item[3] … 0x20440ffc0
item[4] … 0x20440ffb0     … eight in all, descending at a 16-byte stride
```

⇒ ★★★★★ **The guest polls `0x2_0440fff0 … 0x2_0440ff80` — the eight GR
`SET_REPORT_SEMAPHORE` slots, at the identical numeric address the device knows them by**, in a
`rw-s /dev/nvidiactl` mapping, and on the shipping arm **every one reads `0`**.

★★ **The join is not by page offset — it is the same number.** The pre-registration §4.2
conceded only a page-offset match was available (guest VA ≠ GPU VA); the guest maps this
identity, so the identification is exact. ⊘ That is stronger than promised, and it is
independently corroborated by the native capture, which lands `+0xff0` in host RAM at the same
address with `AWAKEN_ENABLE=0` ⇒ **polled, not interrupt-driven**.

### 1.1 ★★★★★ AND THE ARM MOVED THE WALL — the first measured progress in this campaign

| arm | `N` items | each awaits | polled | value |
|---|---|---|---|---|
| `refuse` (shipping) | **8** | `1` | `0x20440fff0 … ffb0 …` (8 slots) | **0** |
| `pass` (`GR_ROUTE=passthrough`) | **1** | **`2`** | (see §1.2) | (see §1.2) |

⇒ ★★★ **Seven of the eight items DROPPED OUT of the wait list and the survivor's awaited value
ADVANCED from 1 to 2.** The eight completions `w268` measured landing (payload `1`, distinct GPU
timestamps) were **consumed by the guest**. `cuCtxCreate` got past what it was stuck on.
⊘ **And `CUP2_RC = 124` on all four boots anyway.** The wall **moved**; it did not fall. That is
a far more informative negative than *"nothing changed"*, and it is the first time this arm has
been shown to buy the guest anything at all.

### 1.2 ⊘ WHAT THE `pass` ARM'S SINGLE ITEM POLLS — *(filled from `w269b_pass`; see §6 if absent)*

`[w269_pass, pass 1]` `N items = 1`, `KIND=1`, `[0x10] = 2`, wait object **on the stack**
(`0x7ffe3c98fbf8`) rather than the heap array of eight. `rbx = 0` ⇒ the pause-only spin;
`ctx[0x40]+0x72b8 = 0` ⇒ the 1-second `MC_SERVICE_INTERRUPTS` leg is live, exactly as `w215`
disassembled.
⚠ **I cannot say WHICH of the eight channels the survivor is.** The arms have different ASLR
bases and different heap layouts, so object addresses are not comparable across arms.

---

## 2. ★★★★★ THE OWNER'S ITEM 0 — **THE STORE INSTRUCTION EXECUTES.** Proof, not inference

The owner: *"do you have proof this piece of write instruction is hit for unprivileged guest
passthrough channel"*. ⊘ **We did not** — every *"the doorbell forwarded"* statement rested on
reading call order, while every doorbell line in `w268` read `DOORBELL-REFUSED`.

`[measured, w269b_refuse, the SHIPPING arm]`:

| witness | count |
|---|---|
| `DOORBELL-XLATE` (guest token → host token, with engine) | **8** |
| `DOORBELL-VERB` (the `ring_doorbell` call site, with engine) | **8**, all `engine=Ce` |
| ★★★ `DOORBELL-STORE … WROTE` | **8** |
| `DOORBELL-STORE … NOT REACHED` (window `Err`) | **0** |
| `DOORBELL-STORE … STORE ITSELF WAS REFUSED` | **0** |

```
DOORBELL-XLATE proc=2 chan=12 vchid=VChid(0x13) engine=Ce guest_token=0x00020013 host_token=0x20015 schedule=true
DOORBELL-XLATE proc=2 chan=8  vchid=VChid(0xf)  engine=Ce guest_token=0x0001000f host_token=0x10011 schedule=true
…eight, every one proc=2 (the USER process)
```

⇒ ★★★ **Leg C is real for `Ce`**: a user process's guest token is translated to a *different*
host token and the 32-bit store into the usermode window executes. The translation is
non-identity, so this measures the translation and not merely the call.
⇒ ★★ **Task #243 is CONFIRMED on the shipping arm**: **zero** `GrCompute` doorbells reach
`plan_doorbell` — as designed, the route refuses them above the verb. `w269b_pass` is what says
whether arming changes that. *(§6.)*
⊘ **Not proven**: that the store had the intended *effect*. There is no completion to check at
the store; `GP_GET` and the semaphore are the only downstream evidence, and `w268` already
carries those.

---

## 3. GRADED AGAINST THE PRE-REGISTRATION (`92aa6da`)

| # | prediction | p | outcome |
|---|---|---|---|
| **P1** | `CUP2_RC = 124` both arms; last print `totalMem=11959 MiB`; no `ok cuCtxCreate` | .93 | ★★★ **FIRED**, all four boots, and its **size** too |
| **P2** | userspace spin, not a blocking syscall | .85 | ★★★ **FIRED** — `state=Rl`, `/proc/pid/syscall` = `running`, `orig_rax=228` (vDSO `clock_gettime`, the elapsed-ms leg) |
| **P3** | histogram mass in the `w215` loop | .80 | ★★ **FIRED** |
| **P4** | snapshot reaches `libcuda+0x22be1f`/`+0x22bedc` | .85 | ★★★ **FIRED** at `+0x22be1f`, every sample, both arms |
| **P5** | `N` is 1 or 2 | .70 | ⊘ **HALF** — `pass` N=1; `refuse` N=**8**, outside the range |
| **P6** | a `kind == 3` item is present | .60 | ⊘ **FAILED** — every item is `kind 1` |
| **P7** | same address, same value, on both arms | .55 | *(§6)* |
| **P8** | the address is **NOT** a GR or CE slot offset | .65 | ⊘⊘ **FAILED — it IS `0x20440fff0`** |
| **P9** | mapping is anon or `/dev/nvidia-uvm` | .55 | ⊘ **FAILED** — `/dev/nvidiactl`, `rw-s` |
| **P10** | ★ the deliberately-widened low arm: **the address IS a GR slot** | **.15** | ★★★★★ **FIRED** |
| **P11** | gdb present or installable | .90 | ⊘ **FAILED** — absent, uninstallable; the C probe stands alone |

★★★★★ **P10 IS THE FOURTH CONSECUTIVE RUNG WHOSE LEAST-WEIGHTED ARM FIRED** (`w268`'s A5 was
`p=0.07`). The brief's calibration note said to widen the low arms so a surprise stays
interpretable; that is the only reason this outcome had a registered meaning at all.
⊘ **And I was wrong in a specific, instructive direction**: P8/P9 encoded a belief that the
completion plane had been *eliminated* as the wait's subject because `w268` satisfied it. The
right inference was that satisfying it would let the guest **advance within the same plane**,
which is exactly what happened.

---

## 4. ⊘ THE INSTRUMENT'S OWN DEFECTS — three, all found by its own output

1. ★★ **`resolve()` reported offsets against the containing MAP ROW, not the module base.** The
   snapshot at file offset `0x22be1f` printed as `libcuda+0xc5e1f` (`.text` begins `0x166000`
   into the file), so **every histogram offset in pass 1 is `0x166000` short of what `objdump`
   uses**. An offset that cannot be joined to a disassembly is not an offset. Fixed for pass 2.
2. ★ **`MAX_BUCKETS = 96` filled**: 5250 of 6000 hits fell outside, so pass 1's "top 24" was
   really *"the first 96 distinct RIPs, all at 24 hits"* — a biased sample presented as a
   ranking. ⊘ **The dropped-count line is what exposed it, which is the entire reason it
   exists.** Raised to 512. (One loop iteration ≈ 250 instructions: 24 iterations in 6000 steps.)
3. ★ **The known-positive caught a third before any boot**: the first version read
   `/proc/tid/stat` **after** `PTRACE_INTERRUPT` and so printed `state=t` for a process
   demonstrably spinning in userspace — which would have voided the owner's item 4, the single
   reading that decides whether this is a memory poll at all.

⊘ **And the probe's central discipline paid**: pass 1 met `KIND=1`, a handler it had not
decoded, and printed *"⊘ NESTED WAIT … the polled address is NOT reported. A partial answer,
said as one."* A probe that had guessed would have named `item[0x08]` and been wrong by two
dereferences. `4029e0` was then disassembled and pass 2 answered.

⚠ **A trap that did NOT bite but would have**: `process_vm_readv` cannot read a `VM_PFNMAP`
mapping (two runs elsewhere concluded a ring was not in the address space; it was). This probe
uses `/proc/<pid>/mem`, which has the same limitation — the polled slot happens to live in a
`rw-s /dev/nvidiactl` mapping and reads fine. ⊘ **Had it read `UNREADABLE`, that would have been
a statement about the probe, not about the page**, and the probe says so in those words.

---

## 5. ⊘ WHAT THIS RUN CANNOT PROVE

1. ⊘ **That the polled slot is the ONLY thing the wait needs.** `f9df90` walks every item and
   the caller spins unless the composite status reaches 5; one unsatisfied item explains the
   spin without being the only one.
2. ⊘ **Which physical channel the `pass` arm's surviving item belongs to.** Different ASLR and
   heap layout across arms; not comparable.
3. ⊘ **That the store had the intended effect.** §2 witnesses the instruction, not the outcome.
4. ⊘ **Anything about the error/event plane.** The two helper threads sit in `poll()`
   (`orig_rax=7`) — that is the os-event plane, and this rung neither measured `RcTriggered`
   nor rechecked `w212`'s F6 masked leaf per channel. ★ But it **does** discharge the owner's
   precondition: **the main thread is NOT in `poll`/`epoll`/`ioctl`** — it is `state=R`,
   `syscall = running`, in a userspace memory poll — so the spin-address hunt was not void, and
   an fd-side answer cannot be the *whole* answer.
5. ⊘ **The guest's `cuMemcpyHtoD` pushbuffer still cannot be decoded from `cup2`** — it never
   reaches the copies (§0.1). Closing that hole needs a workload that gets past `cuCtxCreate`.
6. ⊘ **Correctness of the GR work**; **ordering finer than one sample**; `GR-CURSOR-READER
   stopped` and `PAGE-READER ASSERTION` still FAIL (unpaid a third time, as pre-registered).
7. ⊘ **Two-channel scope**: the native capture shows `HtoD` and `DtoH` use *different* channels
   and slots (`+0xff0`, `+0xf30`). This probe is **not** per-channel — it enumerates the guest's
   whole wait list — so it is not subject to that gap; but nothing here observed `+0xf30`.

---

## 6. THE `pass` ARM OF PASS 2, AND THE NEXT RUNG

★★★ **The one question left open by §1.2**: the `pass` arm's single item awaits **2**. If its
polled address is `0x20440fff0` and the value there is **1**, then **we write the payload the
guest's first wait wanted and not the one its second wait wants** — and that is immediately
actionable. If the value is `2` and it still spins, the wait is not on this word.

1. ★★★★★ **Find what produces the payload `2`.** The guest asked for `1` eight times and got
   it; it now asks for `2` once. Whatever advances that counter a second time is the wall.
2. ★★★ **Measure `RcTriggered` and the per-channel event-leaf mask** (`w212` F6: 179
   completions unvectored, 1 stall vector into a masked leaf). ⚠ **Do not adopt F6 as the
   cause** — it is an unfixed candidate the moving wall orphaned, which is the shape that has
   misled this campaign. The two `poll()`ing helper threads are where it would show.
3. ★★ **Re-run the `DOORBELL-XLATE` census on the armed arm** to settle task #243 in both
   directions.
4. ⊘ **Do NOT default the route.** Four boots is not a posture change; `refuse` stays default.
