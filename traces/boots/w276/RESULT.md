# w276 — RESULT: the sweep is AIMED RIGHT and BINDS NOTHING, and the completion plane is CLEARED

**STATUS: LIVE — 2026-08-12.** Branch `w276-port-the-whole-vas-sweep`, base `fffde60`.
Two boots at `b637d2e` (`w276_on` / `w276_off`), one confirming boot at `67025ca` (`w276b_on`).
Arming asserted from each boot's own log. Every number below was read from an artefact opened
in this session.

---

## ★★★★★ LEAD — THREE THINGS CONTRADICT THE BRIEF, AND THE FIRST ONE IS THE RUNG'S OWN RESULT

### 1. ⊘⊘ THE SWEEP RUNS PERFECTLY AND PUBLISHES **ZERO**. The brief's premise survives; its expectation does not.

`[measured, w276_on, run_w276_on_qemu.log]` — 88 `PT-SWEEP` rows, first one verbatim:

```
PT-SWEEP tasks=4 skipped=0 ran=4 truncated=0 pages=79 reasons={NeverSwept: 4}
  → bound=0 swept_binds=0 unbound=0 unwitnessed=0 published=0 faults=0 reach_faults=0
    refusals=255 first=StraddlesLiveBinding { va: GpuVa(8655536128) }
```

The walk works: 4 address spaces, 79 page-table pages, **zero** faults, **zero** truncation,
**zero** reach-faults. `unwitnessed=0` — the gate the rung existed to move **did move**. And
`bound=0 swept_binds=0`: the relaxation admitted pages and **not one leaf in the desired set
came from a page the witness had not already seen.**

⇒ **The whole-VAS sweep is not what this port lacks** *in the sense the rung expected*.

⊘⊘ **AND THE CONFIRMING BOOT CORRECTS THAT SENTENCE — READ IT BEFORE QUOTING THE ONE ABOVE.**
`[measured, w276b_on, build `67025ca`, the same first doorbell]` with `unchanged`, `dropped`
and the refusal histogram printed:

```
PT-SWEEP tasks=4 skipped=0 ran=4 truncated=0 pages=79 reasons={NeverSwept: 4}
  → bound=0 unchanged=0 repointed=0 swept_binds=0 swept_only_pages=1 dropped=0
    unbound=0 unwitnessed=0 published=0 faults=0 reach_faults=0 refusals=255
    by_kind={"StraddlesLiveBinding": 255}
    refused_vas=[0x203e90000,0x203ea0000,…,0x203f00000,0x203fb0000,…,0x203fbc000,…]
```

Three numbers change the reading:

- **`unchanged=0`** ⇒ the 255 leaves are **not** re-statements of bindings we already held.
  The sweep genuinely **added 255 mappings to the desired set** that the witness-driven pass
  had never published — because that pass only ever decodes pages that are **dirty**, so a
  page witnessed once and never rewritten has its leaves read by **nobody**. The sweep reads
  them all. ⇒ *"the sweep found nothing"* is **wrong**; it found 255 things.
- **`by_kind={"StraddlesLiveBinding": 255}`** ⇒ **every single one** was refused by the address
  table, for one reason: the leaf's range overlaps a **differently-shaped live binding** placed
  by another populate source. Not a walk failure, not a gate — a **granularity collision**
  between populate sources.
- **`swept_only_pages=1`** ⇒ of 79 root-reachable page-table pages, **78 were already
  witnessed**. The witness transport's coverage of the root-reachable set is ~98.7 %, measured.
  ⇒ the *relaxation* is near-inert; the *sweep* is not.

### 2. ★★★★★ THE COMPLETION PLANE AT `0x2_0440_fff0` IS **CLEARED**, at value level, with times

`[measured, w276_on]` — **8 of 8** `COMPLETION-WATCH … OBSERVED`, **0** `NOT-OBSERVED`:

```
COMPLETION-WATCH proc=2 chan=0 va=0x20440fff0 payload=0x00000001 → OBSERVED after=99ms samples=3
COMPLETION-WATCH proc=2 chan=1 va=0x20440ffe0 payload=0x00000001 → OBSERVED after=33ms samples=2
COMPLETION-WATCH proc=2 chan=2 va=0x20440ffd0 payload=0x00000001 → OBSERVED after=77ms samples=3
COMPLETION-WATCH proc=2 chan=3 va=0x20440ffc0 payload=0x00000001 → OBSERVED after=72ms samples=3
```

and the new write census shows **the transitions themselves**, with the engine's own clock:

```
SEMA-WRITE t=+78973ms gpa=0x58fe000+0xff0 0x00000000 → 0x00000001[GR-REPORT p2c0] n=4  back=0
SEMA-WRITE t=+79905ms gpa=0x58fe000+0xff0 0x00000001 → 0x00000002[GR-REPORT p2c0] n=29 back=0
```

**`CUP2_RC=124`.** ⇒ *"cuCtxCreate waits for a completion that never arrives at
`0x2_0440_fff0`"* is not merely retired (w268 retired it by slot count); it is **refuted at
value level, at the declared address, with the declared payload, inside 100 ms**.

### 3. ★★★★★ AND THE WALL NOW HAS AN ADDRESS THE GUEST **DESCRIBES**

`[measured, w276_on, run_w276_on_hostdmesg.log]`:

```
Xid (PCI:0000:00:07): 31, pid=…, name=memfd:kayfabe-i, channel 0x00000009,
  MMU Fault: ENGINE GRAPHICS HUBCLIENT_FE faulted @ 0x7461_86e00000. Fault is of type FAULT_PDE
```

and the sweep's own picture of the **guest's** tables, same boot, same proc:

```
GUEST-DESCRIBES [proc=2 gpu=0 pdb=0x201000 sweeps=88 trunc=0 runs=6
  0x200000000+0x40aa000, 0x204400000+0xc00000, 0x10000000000+0x200000,
  0x10002000000+0x200000, 0x746186000000+0xc00000, 0x746186e00000+0x400000]
```

The grader's own offline join:

> `0x7461_86e00000: runs_printed=21 → ★★★ LEAF-PRESENT in a run 0x746186e00000+0x200000`

⇒ **ARM 2.1 FIRES ON THE "SWEEP IS RIGHT" SIDE.** The guest's own page tables, walked from the
guest's own installed root, **describe the faulting address** — it is the *base* of a described
run. The sweep story is **not** dead. What is dead is the assumption that discovering the
mapping was the missing step.

★ And the run **grows while the guest runs**: sweeps 86 and 87 read `0x746186e00000+0x200000`,
sweep 88 reads `+0x400000`. The dirty-driven re-sweep is tracking the guest live.

---

## THE SWEEP, AS A MECHANISM — every claim the port makes about it, measured

| claim | measurement (`w276_on`) |
|---|---|
| it walks from the guest's own root | 4 root tasks on the first doorbell, `reasons={NeverSwept: 4}` |
| the dirty trigger **discriminates** | later doorbells: `tasks=2 skipped=2 reasons={Dirty: 2}` — 67×, 15×, 4×, 1× |
| it never truncates at this scale | `truncated=0` on **all 88** rows |
| the budget is oversized, measured | `[w276b_on]` `truncated_total=0 pages_total=4616 max_pages_one_doorbell=79` over 88 doorbells ⇒ the largest single walk is ≈40 448 entries against `PT_SWEEP_BUDGET = 300_000` **per VAS**. ⊘ The C's constant is ~7× larger than this workload needs; it is **not** re-derived downward here, because one workload's address space is not a bound on any other's |
| ⊘ the fault VA is not in the refusal set either | `[w276b_on]` `0x7e6b_42e00000 in a refused_vas list? 0 row(s)` — it is neither bound nor refused |
| the walk is clean | `faults=0 reach_faults=0` on every row |
| the witness gate is no longer the blocker | `unwitnessed=0` on every row (it is a **non-zero** count when the gate bites) |
| the relaxation is inert **here** | `swept_binds=0` on every row |
| the control is silent | `w276_off`: `PT-SWEEP arm=off`, **0** `PT-SWEEP` lines, all 7 arm assertions PASS |
| ⊘ and the control's join REFUSES to answer | `0x7de8_e2e00000: runs_printed=0 → ⊘ NOT MEASURED` — the grader will not say `LEAF-ABSENT` from a boot that published no picture |

★★ **The control's fault is the same fault.** `w276_off` faults at `0x7de8_e2e00000`,
`w276_on` at `0x7461_86e00000` — different ASLR bases, **identical low 24 bits (`0xe00000`)**,
identical `Xid 31 / ENGINE GRAPHICS HUBCLIENT_FE / FAULT_PDE / channel 0x00000009`, identical
`CUP2_RC=124`. ⇒ the fault is **unmoved**, graded by identity and by relative position rather
than by a count.

⊘ **The 255 refusals are the interesting residue.** `StraddlesLiveBinding` at
`GpuVa(8655536128)` = `0x2_0440_0000` — the 2 MiB region containing the completion-semaphore
page. The sweep's leaves collide with bindings **another populate source already placed at a
different granularity**. That is a real, specific finding about the address table's
multi-source discipline, and it is not what the sweep was built to fix.

⚠ **The first armed boot could not say WHICH null it was.** `bound=0 swept_binds=0
refusals=255` is consistent with *"found leaves, table refused them"* and with *"found leaves
already bound"*, and `unchanged` — the only discriminator — **was not printed**. Fixed in
`67025ca` (`unchanged`, `repointed`, `dropped`, `swept_only_pages`, plus the refusal kind
histogram and the distinct refused VAs). Same class as every other absence this campaign has
paid for.

---

## ⊘⊘ THE COORDINATOR'S QUESTION, ANSWERED FROM THE ARTEFACT — and it is worse than "unverified"

**Does any producer of `Vas::pt_pages` see CE-ENGINE writes to page-table pages?**

**The producer exists and it did not run.** `[measured, w276_on and w275_pin]` `FWD-RING` reads
**0 lines** in both. `SharedDevice::doorbell` gates `forward_ring` on
`ring_content_is_forwardable(engine)`, which a **GR** doorbell answers *no* to
(`crates/kayfabe-rt/src/device.rs:2235-2237`); `forward_ring` is the only caller of
`parse_pushbuffer`, which is the only producer of `PtWrite` via `classify_ce` →
`CeOperands::PhysOperand` → `latch_pt_writes`. ⇒ on the armed passthrough arm that producer
**never fires**, and `Vas::pt_pages` is fed by the CPU/BAR2 witness alone.

★★★ **And the C's hook has no analogue here, by construction.** `C: nvkvm_gpu_emul.c:8785`,
read verbatim: *"one hook for every **CPU-emulated CE write** that lands in FB. These writes go
through `nvkvm_fb_host_ptr` / `phys_wr32` and BYPASS `nvkvm_fb_write`."* ⇒ the C could latch
those pages **because the C's own CPU performed the copy**. On our passthrough arm a **real
host engine** does the DMA, which is the uninstrumented `pci_dma_map` channel. There is nothing
to hook.

★ **What we DO have, and it is the C's other half.** The C's `cpt_sync_at_release` decodes each
dirtied page **directly — from the page itself, not a root walk**. That is exactly
`plan_pt_decode` → `run_pt_decode` → `decode_subtree(task.page)`, which starts **at** the page.
It is live and it binds: `[measured, w276_on]` one doorbell read `bound=19618`, others `512`,
`1026`, `2050`. So the mechanism the C calls load-bearing is **not** missing.

⚠ **What IS missing is the C's ORDERING, and this boot does not test it.** The C decodes
*before the push's semaphore release is written*; we decode at the next doorbell. On our arm
the release is written by the real host engine, so there is no point at which we could
interpose it — the same architectural fact, one plane over. **Stated as the next gap, not
built.**

The correspondence, in full:

| the C's producer | ours | live on this arm? |
|---|---|---|
| `nvkvm_fb_write` M5.10 dirty-arm (guest CPU → FB window) | G1 `RegPlane::drain_pt_witness` (`/byBAR2`) | ✔ **yes** — `PT-DECODE drained=156` per doorbell |
| `nvkvm_m2_ce_fb_write_hook` (the **emulator's own** CE copy loop) | `classify_ce` → `PtWrite`, driven by `forward_ring` | ⊘ **no** — `FWD-RING` = 0, and a real engine's DMA cannot be hooked |
| `nvkvm_m2_cpt_sync_at_release` (decode the dirty page **directly**) | `plan_pt_decode` → `decode_subtree(task.page)` | ✔ built, ⊘ **at the doorbell, not at the release** |
| `nvkvm_m2_pt_enum` root sweep | **this rung** | ✔ built, and measured **inert** |

---

## ARM 2.2 — THE LIVE WRITE CENSUS: WRITTEN, MONOTONICALLY, BY SOMETHING WITH A GPU CLOCK

`[measured, w276_on]` **30** `SEMA-WRITE` transitions; `w276_off` **50**. **`back=0` on both.**

- All eight declared GR report semaphores go `0x00000000 → 0x00000001` — the **declared**
  payload (`COMPLETION-DECLARE … payload=0x00000001`) at the **declared** address.
- Each carries a four-word report whose high timestamp word is `0x18cb19e6` on every row and
  whose low word advances monotonically (`0xc451e400`, `0xc642bc00`, `0xc8551800`, …) — the
  engine's own clock, not a CPU write.
- Channel 0 then advances `1 → 2`.
- ★ A slot **no declaration names** moves too: `+0xf70 [UNCLAIMED]` goes `1 → 2 → 5`. There
  **is** a second writer on this page — and it is **monotonic**, so it is not the M5.38 shape.

⇒ **No payload word ever went backwards, across all three boots** ⇒ the M5.38 second-writer
corruption story is refuted on this page. The page is neither frozen nor corrupted: it is
written, in order, and the guest hangs anyway.

### ⊘⊘ AND THE DETECTOR FIRED ONCE — ON ITS OWN FALSE POSITIVE. That is the honest headline.

`[measured, w276b_on]` `BACKWARDS transitions = 1`:

```
+0xf70 0x00000001 → 0x00000002 [UNCLAIMED]  n=19 back=0
+0xf78 0xff109e00 → 0x1dc832e0 [UNCLAIMED]  ⚠⚠ BACKWARDS …   n=20 back=1
+0xf7c 0x18cb1a69 → 0x18cb1a6a [UNCLAIMED]  n=21 back=1
```

`+0xf78` is a **timestamp low word** and `+0xf7c` its **high word**, and they moved in the
**same sample**: the low word wrapped and the high word **carried**. That is a 64-bit GPU clock
advancing — the *same* writer, one tick later — not a second writer.

⇒ **My predicate (`w < p`, un-scoped) turns the normal behaviour of a `FOUR_WORDS` report into
this campaign's most alarming signature, roughly once every 2³² clock ticks.** An instrument
that cries its loudest word on a schedule is worse than a quiet one: the *next* real decrease
would be read as another wrap. `[a-falsifier-that-flags-its-own-good-news]`

★ **Fixed** (`09d4203`, compiles, ⊘ **not re-booted** — stated rather than implied): the
predicate is now scoped by the attribution the reader already computes, so only a **declared
payload** slot counts as `backwards`; a decrease anywhere else is counted and printed
separately as `decreases_elsewhere`, never folded in. ⊘ `[UNCLAIMED]` words are excluded too —
we cannot say what role they play, and *"we do not know"* must not be spelled *"corruption"*.

⇒ **Re-read the three boots under the corrected predicate: `backwards_on_payload = 0` on all
three.** The one firing was a clock, and every payload transition in every boot went up.

⚠ **It is a SAMPLER, not a watchpoint.** Two writes inside one tick read as one transition, and
a write undone within a tick is invisible. `transitions=0` would have bounded *"no persistent
change at this cadence"*, never *"nobody wrote"*. (A DMA write is invisible to x86 debug
registers, so a watchpoint is not the stronger instrument it looks like.)

---

## ARM 2.3 — THE HOST CHANNEL'S STATE: **NOT ANSWERABLE TODAY**, and the reason is one constant

Researched, not measured — **and I did not build it**, per the brief's `⊘ do not build delivery
on this rung`.

- There is **no** cheap unprivileged RM control that reports *"this channel was RC'd"*.
  `NV2080_CTRL_CMD_RC_GET_ERROR_COUNT` (`0x20802205`) and `…_GET_ERROR_V2` (`0x20802213`) are
  admin-gated in the `_IMPL` despite `NON_PRIVILEGED` flags
  (`ogkm-580: src/nvidia/src/kernel/gpu/rc/kernel_rc_ctrl.c:72-99, :113-118`) **and are
  GPU-wide, not channel-scoped**. `bIsRcPending` is exposed only by
  `NV906F_CTRL_CMD_RESET_CHANNEL` (`0x906f0102`), which is a **write** verb
  (`kernel_channel.c:3056-3057`). `NV906F_CTRL_CMD_GET_DEFER_RC_STATE` (`0x906f0105`) is
  unprivileged and channel-scoped but answers *"is RC deferred pending an SM debugger"*, which
  is `NV_FALSE` on an ordinary Xid 31.
- **The intended mechanism is the error notifier, and we disable it.** RM writes a 16-byte
  `NvNotification` (`nvgputypes.h:57-64`) at index 0 on RC — `status = 0xffff`, `info32` = the
  `ROBUST_CHANNEL_*` code (31 = MMU fault), `info16` = the engine
  (`method_notification.c:154-186`, via `krcErrorSetNotifier_IMPL`,
  `kernel_rc_notification.c:234-350`). It is designed to be polled by CPU userspace with zero
  ioctls. **We pass `h_object_error: 0` on every host channel and TSG**
  (`crates/kayfabe-isolate-host/src/rm.rs:4773`, `:4814`) — and it is a *stated* decision:
  `crates/kayfabe-abi/src/submit.rs:278-281` says *"the isolate learns about a wedged channel
  from the verb that does not complete, not from here."*
- ⇒ **"Did the host channel RC?" is answered today only by a human reading `dmesg`.** The fix
  is ~one field plus a mapping, and the `NvNotification` decoder **already exists**
  (`crates/kayfabe-abi/src/notifier.rs`) — used only in the write direction, as the fake GSP.

⊘ Note what this means for the freeze hypothesis the brief raised: *"a killed channel produces
no further completions"* is **untestable here** — and this boot supplies the evidence that
matters more anyway, because the completions **did** arrive (8/8 OBSERVED).

---

## PRE-REGISTERED ARMS — how they fell

| arm | outcome |
|---|---|
| `CUP2_RC=0` | ⊘ did not fire |
| `RC=1`, bounded progress | ⊘ did not fire |
| `124`, GR fault **gone**, page frozen ⇒ corruption story | ⊘ did not fire — the page is **not** frozen and **not** corrupted |
| `124`, fault **moved** | ⊘ same fault kind and engine; the address is ASLR'd per boot |
| `124`, fault **unmoved** | ★ **FIRED** |
| leaf **ABSENT** ⇒ the story is dead | ⊘ **did not fire** — **LEAF-PRESENT**, and it is a run's base |
| the sweep **too expensive** | ⊘ did not fire — `truncated=0`, 79 pages max |
| ⊘ *the sweep publishes nothing* | ★★★★★ **FIRED — and it was not in the table.** |

★ Six of the last eight rungs had their least-weighted arm fire. This one had an arm fire that
was **not weighted at all**, which is worse: the pre-registration did not contain the outcome.
It is recorded as a gap in `PREREGISTRATION.md` rather than back-filled as a prediction.

---

## ⊘⊘ WHAT THIS RUN CANNOT PROVE

- **It cannot refute the whole-VAS invariant.** `C: :8676-8688` measured a root walk as
  insufficient *at this exact point* (*"root re-walk read `runs=0` while the host CE faulted
  one page past the last-backed leaf"*). A null result from a root sweep is **expected by the
  C**. What this run adds is that the null here has a *different* cause: not "the walk cannot
  reach it" but "the walk reaches it and the witness already had it".
- **It does not test the C's ordering** — decode-at-release vs decode-at-doorbell. That is the
  one half of the C's `#13` fix this port does not have, and on a passthrough arm there is no
  interposition point for it.
- **`LEAF-PRESENT` does not say why nothing backs the address — and the confirming boot shows
  it is not the refusals either.** `[measured, w276b_on]` the join reads
  `0x7e6b_42e00000 in a refused_vas list? 0 row(s)` while the same boot reports
  `LEAF-PRESENT in a run 0x7e6b42e00000+0x200000`. ⇒ the faulting address is **neither bound
  nor refused**: the sweep's desired set never contained it. Why it did not is the **next
  question**, and this rung does not answer it.
- **One workload, one chip (GA106), one driver (`580.159.04`), one boot per arm.** The write
  census is a sampler; the sweep numbers are one guest's address-space shape.
- **Nothing here measures doorbell delivery, engine execution, or throughput.**
- The `w276_off` control shares this build; it is a control for the **flag**, not for the code.

---

## ARTEFACTS

| what | where |
|---|---|
| armed + control boot logs | `traces/boots/w276/` |
| pre-registration | `traces/boots/w276/PREREGISTRATION.md` |
| the runner | `scripts/bench/w276_run.sh` |
| the sweep | `crates/kayfabe-{mmu,core,fwd,rt,qemu-raw}` @ `b858fcb` |
| the tests | `tests/tests/pt_sweep.rs` (8, all pass) |
| the §6 ruling | `nvidia-gpu-passthrough@791578e:docs/design/mode2_address_table.md` |
