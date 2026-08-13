# w262 — LEG A2 FIRED, WITNESSED BY NAME; and the leg-B ordering question is NOT the one that can be asked

> ### STATUS — 2026-08-12 / **LIVE — MEASURED.** Two arms (`off`, `ring`) from **one** binary,
> plus a third boot (`w262b_ring`) from a **second** binary carrying a corrected instrument.
> Bench `vh`, `NVIDIA GeForce RTX 3060` (GA106), host driver **580.159.04**.
> Source revisions **`9d689ec`** (arms 1-2) and **`7f10b01`** (arm 3), each **stamp-gated**
> against the binary before booting.
> Graded against `w262_birth_witness_and_userd_ordering_prereg.md`, committed before any code.

---

## 0. ★★★★★ THE HEADLINE — `w261`'s hole is closed, and the answer is YES

`w261`'s own `RESULT.md` led with: *"Leg A2's firing is NOT directly witnessed … Zero refusals is
consistent with BOTH `adopt: Some` succeeding AND `adopt: None` never being asked — this log
cannot tell them apart."*

**It fired.** The line, verbatim from `run_w262_ring_qemu.log`:

```text
kayfabe-isolate: GR-BIRTH #1 engine=GrCompute vas=0xcafe0005 adopt=GUEST-RING
    memory=0xcafe0006 ring_va=0x200200000 gp_fifo_va=0x200200000 entries=1024 joined=YES
    ⇒ the address table held a JoinsGuestWindow binding at this channel's declared
    gpFifoOffset → alloc_channel_over_guest_ring [births=1 guest_ring=1 declined=0
    not_asked=0 refused=0]
```

**16 such lines on the `ring` arm. Zero on `off`.** Sixteen **distinct** declared `gp_fifo_va`
— `0x200200000`, `0x200203000`, … `0x20022d000`, stride `0x3000` — all resolving into the
**one** joined leaf (`ring_va=0x200200000`, `memory=0xcafe0006`, `entries=1024`), all on
`vas=0xcafe0005`, the address space of client `0xc1d0000c`: **the channel this campaign walls
on.** Eight `engine=GrCompute`, eight `engine=Ce`.

★ **And the control's zero is a MEASURED zero, which is the whole point of the third state.**
The `off` arm prints 24 lines that say, in words, that the armed path *ran*:

```text
kayfabe-isolate: GR-BIRTH #1 engine=Ce vas=0xcafe0005 adopt=DECLINED ⊘ an engine-object
    birth, so the armed path WAS consulted — and the address table held no joined binding
    at this channel's ring VA → RingSource::Ours(None) [births=1 … declined=1 …]
```

⇒ *"no line"* and *"a line saying nothing was adopted"* are now different observations.

---

## 1. THE PRE-REGISTERED SCORECARD

| # | observable | pred `off` | **meas `off`** | pred `ring` | **meas `ring`** | |
|---|---|---|---|---|---|---|
| Q1 | `CUP2_RC` | 124 | **124** | 124 | **124** | ✔ movement predicted **0**, movement **0** |
| Q2 | `GR-BIRTH` total | > 0 | **24** | > 0 | **24** | ✔ |
| Q3 | `adopt=GUEST-RING` | **0** | **0** | **≥ 1** | **16** | ★★★★★ ✔ **the rung** |
| Q4 | `adopt=DECLINED` | > 0 | **24** | ≥ 0 | **8** | ✔ |
| Q5 | `adopt=NOT-ASKED` | ≥ 0 | **0** | ≥ 0 | **0** | ⊘ see §3.2 |
| Q6 | `REFUSED RING_NOT_A_JOINED_WINDOW` | 0 | **0** | 0 | **0** | ✔ |
| Q7 | `GR-RING-JOIN` on `fb_phys=0x1000000` | 0 | **0** | **24** | **25** | ⊘ **MISSED** — §3.1 |
| Q8 | `BAR1 GP_PUT` live lines | > 0 | **8** | > 0, same offsets | **8, identical offsets** | ✔ |
| Q9 | first `GP_PUT` vs first birth | GP_PUT **later** | **later** | GP_PUT **later** | **later** | ✔ — and §2 is the caveat |
| Q10 | `RmInitAdapter` failures | 0 | **0** | 0 | **0** | ✔ |
| Q11 | host `Xid` | 0 | **0** | 0 | **0** | ✔ |
| Q12 | guest `NVRM` lines | 31 | **31** | 31 | **31** | ✔ |
| Q13 | `CE-SUBMIT → RETIRED` | 0 | **0** | 0 | **0** | ✔ |

`CAPTURE_RC=0` both arms. `BUILD_RC=0`, `STAMP_GATE=PASS` against HEAD.
`grep -c 'No space left on device\|LLVM ERROR'` over build + qemu + probe + capture logs of the
**same invocations that produced the statuses** = **0**.

### 1.1 ★★ INSTRUMENT NEUTRALITY — every non-target row is IDENTICAL across the arms

`ENGINE-OBJECT` census `seen=34 forwarded=32 refused=2` on **both**; `doorbells: 191 arrived,
191 served, 0 REFUSED` on **both**; `BAR1 GP_PUT: 188 write(s)` on **both**; `GR-BIRTH` total
24 on **both**; `LOOPBACK BACKEND` 0 on both (so the real isolate plane served every birth).

⇒ **Exactly two numbers moved: Q3 (0 → 16) and Q7 (0 → 25).** The prereg's §3.3 item 5 said an
arm comparison is void if anything else moves. Nothing else moved.

---

## 2. ★★★★★ ITEM 2 — THE ORDER IS FAVOURABLE, AND THE CAVEAT IS WORTH MORE THAN THE RESULT

### 2.1 What was measured

| | `off` | `ring` |
|---|---|---|
| first `ENGINE-OBJECT` of any kind | line 17 | line 19 |
| first host channel **born** (`materialized_channel=true`) | line 19 | line 23 |
| **first `GP_PUT` store** | line 35, **`06:15:26.509239`** | line 50, **`06:20:03.252207`** |
| nearest stamp *before* that first birth | `06:15:00.672650` | `06:19:38.141140` |

⇒ **The guest's first cursor advance comes ~20-25 s AFTER the first host channel is born**, on
both arms. **Q9 as pre-registered, in the favourable direction**, with a large margin.

### 2.2 ⊘⊘ AND THAT IS NOT THE QUESTION LEG B NEEDS — three measured reasons

1. **The advances we can place are not the GR channel's.** All eight printed stores land on
   pages `0xa0000 / 0xc0000 / 0xe0000 / 0x100000` — **`0x20000` apart**. The walling GR
   channels declare `userd=h0x5c000014/off0x2000, 0x5000, 0x8000 …` — **one object, `0x3000`
   apart**. `nvkvm.c:625-632` attributes the recorded pairs, from the driver's own source, to
   nvidia-uvm's `internal_channel_submit_work`.
2. **8 of 188 were placed.** The teardown line says `BAR1 GP_PUT: 188 write(s) at USERD+0x8c,
   8 printed LIVE (cap 8)` on **both** arms. 180 advances were counted and not placed. ⇒ §4's
   follow-up instrument.
3. ★ **All 16 `GUEST-RING` births happen AFTER all 8 placed advances** (`ring`: advances at
   lines 50-79, births at 91-198). So the only advances we can *order* are on the unfavourable
   side of the GR births — while belonging to different channels. **Two facts that point
   opposite ways, and neither settles it.**

⇒ **Reported plainly: the ordering that leg B's safety depends on — does the GR channel write
`GP_PUT` into its OWN USERD before its OWN host channel is born — is STILL UNMEASURED.** What
this rung changed is that it is now unmeasured *for a measured reason* rather than an assumed
one, and §4 is the instrument that can close it.

---

## 3. ⊘ WHERE I WAS WRONG, AND WHAT THE RUN CANNOT PROVE

### 3.1 Q7 missed: predicted 24, measured 25

The prereg registered *"**24** (`w261`'s number, exactly)"*. Measured **25**. Recorded rather
than re-graded. The prediction treated a count that depends on drain scheduling as if it were a
constant; `w261`'s 24 was one sample, not a law.

### 3.2 ⊘ Q5 `NOT-ASKED` is **0 on both arms** and has NO live known-positive

No doorbell materialization occurred — all 191 doorbells were `SERVED-LOCAL`, so
`VerbPlan::Doorbell`'s birth path never ran. ⇒ That state's correctness rests on the unit test
(`the_birth_offer_reads_three_states_and_adoption_dominates`) and the source census
(`the_birth_witness_can_tell_declined_from_never_asked`), **not** on a live positive. Said out
loud because `a_census_zero_needs_a_known_positive` and this zero does not have one.

### 3.3 ⊘ What the `GUEST-RING` line does NOT say

- **Not that the ring is fetched.** `admitted_and_served_are_different_gates`: the line says RM
  was **told** the guest's ring at channel creation. Nothing here reads `GP_GET`.
- **Not that any guest work executed.** `CUP2_RC = 124`, `CE-SUBMIT` 0, completion plane
  untouched — all as pre-registered.
- **Not that leg B's ordering claim is right.** `a_green_test_can_hold_a_wall_in_place`: an
  ordering *observation* cannot distinguish *"the ordering is right"* from *"the ordering is
  wrong and got lucky"*. That needs fault injection, which this rung does not do.

### 3.4 ⊘ Two defects the boots found in MY OWN instruments

Both fixed in `7f10b01`; recorded here because they were only visible from a real boot.

1. **The flat print cap answers the wrong shape** — §2.2 item 2.
2. **`GR-BIRTH #N` was ambiguous across processes.** `[measured, `w262_ring`]` the log carries
   `#1..#8` and `#1..#16` — two isolate children interleaving independent **per-process**
   counters into one file — and nothing on the row said which was which. The counters stay
   per-process (an isolate is a pool, so that is the right granularity); the row now carries
   the `IsolateId`.

---

## 4. THE FOLLOW-UP BOOT — one row per USERD page

`w262b_ring`, revision `7f10b01`, same arm and same environment as `w262_ring`, with the
per-page cursor census beside the flat cap. `CAPTURE_RC=0`, `STAMP_GATE=PASS`, `ENOSPC_LLVM=0`,
`CUP2_RC=124`, `adopt=GUEST-RING` **16**, `adopt=DECLINED` **8**, joins on `0x1000000` **25** —
i.e. it **reproduces `w262_ring` row for row**, which is what makes its new rows comparable.

### 4.1 The measurement

```text
BAR1 GP_PUT pages: 16 distinct USERD page(s) ever advanced a cursor,
                   44 advance(s) DROPPED because the table was full (cap 16)
  page[0] +0xa0000  first_val=0x1     advances=129     ┐
  page[1] +0xc0000  first_val=0x1     advances=1       │ 0x20000 stride —
  page[2] +0xe0000  first_val=0x1     advances=1       │ nvidia-uvm's pool
  page[3] +0x100000 first_val=0x1     advances=1       ┘
  page[4] +0x90000  first_val=0xd801  advances=1       ⚠ SEE §4.3 — NOT A CURSOR
  page[5..15] +0x112000 0x115000 0x118000 0x12a000 0x12d000 0x130000 0x133000
              0x136000 0x139000 0x13c000 0x13f000      ← ★ 0x3000 STRIDE
```

**`0x3000` is the walling GR channels' own stride** — both their declared USERD offsets
(`h0x5c000014/off0x2000, 0x5000, 0x8000 …`) and their declared `gp_fifo_va`
(`0x200200000, 0x200203000 …`).

### 4.2 ★★★ THE ORDERING, for the group that matters

| | line |
|---|---|
| all 16 `adopt=GUEST-RING` births | **96 … 203** |
| first advance on **every** `0x3000`-stride page | **205 … 435** |

⇒ **Every one of the sixteen host channels born over the guest's ring was born BEFORE any
`0x3000`-stride page first advanced.** On this workload, in this direction, adoption at channel
creation would not have wiped a live cursor.

### 4.3 ⊘⊘ AND THE INSTRUMENT FOUND A FALSE-POSITIVE CLASS IN ITSELF — read before citing §4.2

`page[4] +0x90000 first_val=0xd801`. **`0x90000` is a RING page**, not a USERD page
(`BAR1[0] WRITE off=0x90000 val=0x20000000`), and `0xd801` has the shape of a GPFIFO entry's
**high dword** — compare the measured `0x2801` and `0x6801`. ⇒ a ring whose 18th entry pair
lands at `+0x88/+0x8c` **collides with the predicate `(addr & 0xfff) == 0x8c`**.

⚠ **This bites §4.2 directly.** The GR channels' **rings** are `0x3000` apart *and so are their
USERDs*. A `0x3000`-stride page group is therefore consistent with **either**. The only thing
separating them here is the **value shape** — every `0x3000` page has `first_val = 0x1`, which
is what a first `GP_PUT` looks like and not what an entry dword looks like — and **a value shape
is evidence, not a discriminator.**

⇒ **§4.2 is a strong indication and not a proof**, and the reason is the same one this whole
rung keeps arriving at: *nothing joins a BAR1 offset to a channel*. Recorded in the gate itself
(`the_c_gp_put_witness_watches_the_offset_the_abi_names` now carries `0x9008c` as a measured
counterexample) so the "the predicate is clean" reading cannot be made again from the eleven
entry stores that were the only evidence before.

★ Note how it was caught: **by an instrument that printed a value it did not need**, next to a
page it did not need. A census that had printed only counts would have reported *"16 pages
advanced a cursor"* and been wrong about one of them, silently.

### 4.4 ⚠ And the cap is STILL too low — 44 advances dropped

16 distinct pages recorded, **44 advances dropped** because the table was full. There are more
than 16 distinct pages. ⊘ The dropped count exists precisely so this cannot read as *"only
sixteen pages ever advanced"*; it says *"sixteen fit."*

---

## 5. WHAT A NEXT RUNG SHOULD READ BEFORE RE-DERIVING ANY OF THIS

⊘⊘ **`leg_b_userd_adoption_blocker.md` §1's stated blocker was already adjudicated — in the
OTHER repo, the day before — and the two docs disagree about what the blocker is.**

`nvidia-gpu-passthrough@4003eab`, `docs/design/userd_is_not_the_ring.md`, **2026-08-11 15:59**,
one day before the leg-B doc:

- *"`AllocFacts::mem_phys` having no producer is **CONFIRMED — and IRRELEVANT**"*, because RM
  looks `hUserdMemory[0]` up **in the caller's own client**
  (`ogkm-580: kernel_channel_gv100.c:184-187`), so forwarding the guest's handle was never the
  mechanism; and *"the physical page is already resolved, by a different instrument"* — the
  BAR1 GMMU walk.
- *"the guest's `hUserdMemory[0]` **is decoded TODAY**, version-keyed, on every guest channel
  alloc"* (`ChannelUserdWire`, `V580 {32, 64}`) — against the leg-B doc's *"the guest's USERD
  alloc itself carries no params at all"*, which that doc shows is a **C-era, misattributed**
  log row.

★ **The two docs reach the SAME decision — do not build a `UserdSource::Guest(handle)` arm —
for different reasons, and the leg-B doc's reason is the one that does not survive.** The
predecessor's *refusal* was right; its *diagnosis* was not. ⇒ Do not cite §1 of the leg-B doc as
the blocker.

⚠ **And two of that doc's three named prerequisites have since LANDED**, which nobody has folded
back into either doc: its #1 (*"give the emulated framebuffer a shareable host backing (memfd)"*)
is what `join_fb_leaf`'s `ExportSource::Fabricated` + `JoinsGuestWindow` now does, and its #2
(*"**run R32**" — "the whole verdict hangs here"*) ran at `f58473f` and **J2 HOLDS**.

⇒ The gap that actually remains is the one **both** docs independently reach and this rung
reached a third time from a third direction: **USERD is named by handle+offset (and, in the
runlist entry, by raw physical address), never by a VA** — so the page-table walk that gave the
ring its second join source (leg A1) **cannot** be pointed at USERD. That, and not `mem_phys`,
is what a leg-B rung has to solve.

