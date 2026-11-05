# w269 — ★★★★★ **THE GUEST POLLS THE EXACT SLOTS WE WRITE.** Arming CONSUMED seven of eight; the wait moved from *eight GR slots awaiting `1`* to **one CE slot, `0x2_0440ff70`, awaiting `2` and holding `1`** — and the doorbell store is now WITNESSED, 17/17

> ### STATUS — 2026-08-12 / **LIVE — MEASURED.** Four boots on bench `vh`, `NVIDIA GeForce RTX
> 3060` (GA106), host driver `580.159.04`. **Pass 1** (`w269_refuse`, `w269_pass`) ran the
> **`w268` binary unchanged**, `kayfabe-rev:70463ae…`, asserted before booting and **built by
> nothing** — the only change was a guest-side `ptrace` probe. **Pass 2** (`w269b_refuse`,
> `w269b_pass`) added the owner's doorbell-store witness and rebuilt at
> `kayfabe-rev:4b23d0b…`. Graded against the pre-registration (removed, see the correction below),
> committed at **`92aa6da`, before `guest_spinprobe.c` existed**. Branch
> `w269-the-spin-address`.
>
> **Correction, 2026-10-31 (public-tree hygiene).** The probe's source
> (`scripts/bench/guest_spinprobe.c`) and its pre-registration (`docs/archive/w269_the_spin_address_prereg.md`,
> committed at `92aa6da`) were removed from the tree, and the per-run probe output beyond the
> process maps was redacted: they carried offsets into, and an object layout read out of, a closed
> NVIDIA library, which `docs/design/V3_PROVENANCE.md` section 2 does not allow in this tree. The
> probe logs under `traces/boots/w269..w280` keep their line numbers; the affected lines read
> `[line redacted ...]`. What stays is the behaviour measured: which slots the guest polled, the values
> read there, and how the wait changed between the two arms. The pre-registered predictions that
> depended on the library layout (P3-P6) are summarised in section 3 only by their outcome.

---

## 0. ⊘⊘ LEAD WITH WHAT CONTRADICTS THE BRIEF — three things, and the first two cost no GPU time

### 0.1 ★★★★★ **THE BRIEF'S ITEM 1 WAS ALREADY ANSWERED ON DISK.** `cuCtxCreate` blocks — not the copies

The brief proposed *"`launches=3` on a CE channel is consistent with the memcpys — the wall may
have moved past `cuCtxCreate` into the copies"* and asked which call is the last to print.
⊘ **`traces/boots/w268/run_w268_{pass,refuse}_probe.log:51-62`, both arms, already said:**

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

### 0.2 ★★★★ **"NOBODY KNOWS WHAT IT IS" WAS TOO STRONG — `w215` had already characterised the loop on 2026-08-10**

`f5f55ad`. The `cuCtxCreate` wait is a userspace spin loop. It re-arms a one-second clock each time
`0x20801702` `MC_SERVICE_INTERRUPTS` (named in the open kernel modules) returns `NV_OK`, which we
answer `NV_OK`, so no longer deadline can ever answer "does it terminate." `w216` showed that
constructively: with `KAYFABE_MC_SERVICE_BUDGET=20`, `cuCtxCreate` returned. ⊘ How the loop was
characterised is not restated here (see the correction above); the behavioural evidence is `w216`'s
run and the repeated `0x20801702` calls in this boot's RPC census.
What was genuinely unknown was **which memory the loop polls**, and that is what this rung read.

### 0.3 ⊘ **THE INSTRUMENT THE BRIEF NOMINATED CANNOT ANSWER A HANG**

`tests/mode2/gcup2_gdb.sh` is built for a **`SIGSEGV`** (`handle SIGSEGV stop nopass`, then
`$_siginfo…si_addr`). On a hang it prints nothing. ⊘ And **`gdb` is not installed in the guest
and could not be installed** — the C probe stands alone; the cross-check does not exist.

---

## 1. ★★★★★ THE ANSWER — the polled address, read out of the spinning process

Seen in `run_w269b_refuse_probe.log`, eight polled addresses (the per-item
decode lines are redacted, see the correction above):

```
POLLED ADDRESS = 0x20440fff0   pageoff=0xff0   nvidiactl+0x400fff0  <rw-s /dev/nvidiactl>
                 VALUE AT IT = 0x00000000 (0)
                 SLOT-JOIN: MATCHES a GR SET_REPORT_SEMAPHORE slot page-offset
… 0x20440ffe0 … 0x20440ffd0 … 0x20440ffc0 … 0x20440ffb0 … eight in all, descending at a 16-byte stride
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

### 1.2 ★★★★★ THE `pass` ARM POLLS A **CE** SLOT, WANTS `2`, AND READS `1`

`[measured, w269b_pass, `run_w269b_pass_probe.log`]`:

```
one wait item, awaiting progress 2, holding 1
POLLED ADDRESS = 0x20440ff70   pageoff=0xf70   nvidiactl+0x400ff70  <rw-s /dev/nvidiactl>
                 VALUE AT IT = 0x00000001 (1)
                 SLOT-JOIN: MATCHES a CE release-semaphore slot page-offset (0xf00..0xf70/16)
```

⇒ ★★★★★ **THE WALL, NAMED EXACTLY.** Arming the route satisfies all eight **GR** waits, and the
guest then advances to a **single wait on a COPY-ENGINE release semaphore**: `0x2_0440ff70`,
**wanting progress `2`, holding `1`**.

★★★ **And `0x2_0440ff70` is not a new address — it is chan 8's own declared CE release target.**
`w268` §2 recovered it from that channel's own pushbuffer:
`SEMA-SOURCE-CE token=0x0001000f chan=8 → release_target(s)=1 [0x20440ff70]`.
⇒ **The CE channel released ONCE (the slot reads 1) and the guest is waiting for it to release a
SECOND time.**

★★★★ **This re-opens the brief's item 2 on a measured chain rather than a hypothesis** — and by a
completely different route than the one the brief proposed (which §0.1 killed). `w268` §3.2
measured that channel's **second** submission as `methods=11 launches=3` over two GPFIFO entries
— the first substantive CE work of the boot — and it took `Xid 31 ENGINE CE2 HUBCLIENT_CE0
FAULT_PTE ACCESS_TYPE_VIRT_WRITE @ 0x2_04420000`, **reproduced identically here** on `w269_pass`.
A submission that faults never reaches its release. ⇒ *"The guest wants release #2; release #2's
submission faulted"* is a coherent, same-channel, correctly-ordered chain.
⊘ **It is NOT proven.** I have not shown the faulting submission is the one that would have
written `2` into `0x2_0440ff70`; the correlation is by channel and by ordering only.

⊘ The spin was the pause-only flavour and the one-second `MC_SERVICE_INTERRUPTS` leg was live, as `w215`
characterised (register and field values redacted, see the correction above).
⚠ **I cannot say WHICH of the eight channels the survivor object is** by address — different ASLR
and heap layout across arms. The **slot** identifies it (`ff70` = chan 8's target); the pointer
does not.

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
### 2.1 ★★★★★ AND ON THE ARMED ARM — task #243 settled in **BOTH** directions

`[measured, w269b_pass]` `DOORBELL-XLATE` **17**, `DOORBELL-VERB` **17**,
`DOORBELL-STORE … WROTE` **17**, `NOT REACHED` **0** — of which **8 are `engine=GrCompute`,
`proc=2`, the user process**, and 9 are `Ce`.

⇒ ★★★ **Task #243 (*"user-proc `GrCompute` doorbells never reach it at all"*) is settled in both
directions and by measurement rather than by reading**: **zero** on the shipping arm (the route
refuses them above the verb, as designed), **eight** when armed — and all eight **rang**. That is
the transport half of `w268`'s `GET` catching `PUT`, now witnessed at the store instruction
instead of inferred from its effect.
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
| **P4** | the one-shot snapshot fires inside the known wait loop | .85 | ★★★ **FIRED**, every sample, both arms |
| **P5** | the wait list has 1 or 2 items | .70 | ⊘ **HALF** — `pass` N=1; `refuse` N=**8**, outside the range |
| **P6** | an item of the semaphore-progression kind is present | .60 | ⊘ **FAILED** — every item is of the other kind |
| **P7** | same address, same value, on both arms | .55 | ⊘⊘ **FAILED, and this is the rung's headline** — `refuse` polls eight **GR** slots reading `0`; `pass` polls one **CE** slot reading `1` |
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

1. ★★ **`resolve()` reported offsets against the containing MAP ROW, not the module base.** Every
   histogram offset in pass 1 was therefore short of the file offset that an offline tool would use
   (the `.text` segment does not start at file offset 0). Fixed for pass 2.
2. ★ **`MAX_BUCKETS = 96` filled**: 5250 of 6000 hits fell outside, so pass 1's "top 24" was
   really *"the first 96 distinct RIPs, all at 24 hits"* — a biased sample presented as a
   ranking. ⊘ **The dropped-count line is what exposed it, which is the entire reason it
   exists.** Raised to 512. (One loop iteration ≈ 250 instructions: 24 iterations in 6000 steps.)
3. ★ **The known-positive caught a third before any boot**: the first version read
   `/proc/tid/stat` **after** `PTRACE_INTERRUPT` and so printed `state=t` for a process
   demonstrably spinning in userspace — which would have voided the owner's item 4, the single
   reading that decides whether this is a memory poll at all.

⊘ **And the probe's central discipline paid**: pass 1 met an item kind it had not decoded, and printed
*"⊘ NESTED WAIT … the polled address is NOT reported. A partial answer, said as one."* A probe that
had guessed would have named the wrong word, wrong by two dereferences. Pass 2 decoded that kind
and answered.

⚠ **A trap that did NOT bite but would have**: `process_vm_readv` cannot read a `VM_PFNMAP`
mapping (two runs elsewhere concluded a ring was not in the address space; it was). This probe
uses `/proc/<pid>/mem`, which has the same limitation — the polled slot happens to live in a
`rw-s /dev/nvidiactl` mapping and reads fine. ⊘ **Had it read `UNREADABLE`, that would have been
a statement about the probe, not about the page**, and the probe says so in those words.

---

## 5. ⊘ WHAT THIS RUN CANNOT PROVE

1. ⊘ **That the polled slot is the ONLY thing the wait needs.** The wait covers every item in the
   list and the caller spins until all are satisfied; one unsatisfied item explains the spin
   without being the only one.
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

## 6. THE NEXT RUNG — and it has a measured address for the first time

1. ★★★★★ **`0x2_04420000`, and now on a chain rather than a hunch.** The guest waits for CE
   release **2** on `0x2_0440ff70`; that channel's **second** submission is the `methods=11
   launches=3` one; that submission takes `Xid 31 CE2 HUBCLIENT_CE0 FAULT_PTE VIRT_WRITE @
   0x2_04420000` — **reproduced identically on three consecutive armed boots** (`w268_pass`,
   `w269_pass`, `w269b_pass`). A faulting submission never releases. ⇒ Find who *names*
   `0x2_04420000`: the second GPFIFO entry's own methods, decoded from the guest's declaration.
   ⚠⚠ **Derive the page from what the guest declared, never from the remembered literal** — a
   remembered address read out of guest RAM is the `cap2b` class, this tree's own
   guest-reachable-defect fixture.
2. ★★★ **Prove or break the chain before fixing it.** Two cheap discriminators: (a) does
   `0x2_0440ff70` ever reach `2` on any boot? (b) is the faulting submission the one carrying
   the `SET_REPORT_SEMAPHORE` for `ff70`, or a different entry of the same pushbuffer? ⊘ Today
   the link is correlation by channel and ordering only.
3. ★★ **Measure `RcTriggered` and the per-channel event-leaf mask** (`w212` F6: 179 completions
   unvectored, 1 stall vector into a masked leaf). ⚠ **Do not adopt F6 as the cause** — it is an
   unfixed candidate the moving wall orphaned, which is the shape that has misled this campaign
   before. ★ The two `poll()`ing helper threads are exactly where it would show, and this rung
   has now proved the **main** thread is not among them.
4. ⊘ **Do NOT default the route.** Four boots is not a posture change; `refuse` stays default —
   though it is now measurably the *worse* configuration for the guest, which is new.
