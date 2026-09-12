# The production contract

**STATUS: LIVE, 2026-09-12.** Owner ruling, this session: *"dont chase the llm. the code is bit
rot … make it for prod now, kill the things I repeatedly say to cut and wire the code that was
intended in the architecture and kill a lot of behavioural flags."*

This is the one page that says what kayfabe **is**, so that the deletions which follow are
reviewable instead of a series of silent judgement calls. It is short on purpose.

---

## 1. Why this exists — the rot is measured, not felt

Three things happened in one night, all of them consequences of a configuration matrix nobody
runs end to end:

- **A whole arm stopped booting and nobody knew.** `KAYFABE_DOORBELL_ASYNC=off` cannot
  initialise the guest driver. `start_doorbell_publish_worker` returns immediately when the arm
  does not defer, so that arm has **no worker at all** — and `drain_mirror_revalidation` had
  exactly one caller, inside the worker loop. Queued BAR-mirror fills are therefore never
  drained, so no memslot is ever installed and the passthrough plane never engages.
  `[measured w529]` the boot's own log shows `BAR2-PASSTHROUGH MISS #5, #6, #7 … no memslot
  covered this page yet` at successive pages.
- **A recorded conclusion expired.** w383 measured three arms on hardware and concluded
  `nocoalesce` fixes the LLM. `[measured w528]` it does not: `nocoalesce` fails identically to
  `on`. That conclusion tracked a **version**, not a mechanism.
- **A refusal was read as a cause.** `[measured w527]` I reported the guest's `NV_ERR_NOT_SUPPORTED`
  refusals as the LLM's blocker. `a_refusal_needs_a_negative_control` had already tabulated the
  same refusals in boots where compute is **bit-exact green**. They are the modprobe-time
  baseline.

⇒ **A flag whose second value is never exercised is not a flag. It is dead code with a branch,
and a measurement hazard.** The rot runs both ways: the alternative rots because nothing runs
it, and the *default* rots because the bench pins past it — which is exactly how the publication
dirty gate ran disarmed for a month (`the_llm_passes_and_the_cause_was_an_unreachable_default`).

---

## 2. The production configuration

`[audited 2026-09-12]` the tree reads **57** `KAYFABE_*` environment variables. The bench pins
**nine** of them to the same value on **every graded boot**, and has for months. For those nine
the surviving value is the one every measurement was taken on, which is the whole test for
whether a deletion is safe.

| variable | other values, to be deleted | production behaviour |
|---|---|---|
| `KAYFABE_ISOLATES` | `stillborn` | isolates are **always** used |
| `KAYFABE_GUEST_RAM` | `None` | guest RAM is a hypervisor **memfd** |
| `KAYFABE_FB_JOIN` | `off`, `private` | framebuffer join is **shared** |
| `KAYFABE_GUEST_RING` | `off` | the guest's own **ring** is adopted |
| `KAYFABE_GR_ROUTE` | `refuse` | graphics routes **passthrough** |
| `KAYFABE_OPERAND_JOIN` | `off`, `assert` | operands are **joined** |
| `KAYFABE_PT_SWEEP` | `off` | the page-table sweep is **on** |
| `KAYFABE_PT_WITNESS_EXEC` | `off` | the executor's writes are **witnessed** |
| `KAYFABE_CE_EXECUTOR` | `Local` | copy engines execute on the **host** |

Also collapsing, with work attached rather than a pure deletion:

| variable | other values | production behaviour | prerequisite |
|---|---|---|---|
| `KAYFABE_DOORBELL_ASYNC` | `off`, `nocoalesce` | trap validates, **enqueues, returns**; a worker always exists | rework five test files' seam (§5) |
| `KAYFABE_VAS_PUBLISH` | `off`, `assert`, `publish`, `pinrate`, `both` | **drain** | none |
| `KAYFABE_MMU_INVAL` | `Off` | invalidate completes **off-thread** | none |

**BARs.** BAR1 and BAR2 are passthrough. BAR0 takes **write traps only**; reads are served from
a read-only memslot. ⊘ BAR0 read passthrough **does not exist yet** — there is no BAR0 memslot
anywhere in the tree — so this line is a *commitment*, not a description. See §4.

**Doorbells.** Passthrough rings the host inline, one MMIO store. Emulated channels enqueue and
wake. ⊘ The lock-free `DoorbellTable` is **built and unwired** (316 lines, 9 green tests, zero
callers); making it the only path is §4 work.

### Progress, and the evidence the deletions produce as they go

| arm | state | refs in `shim.rs` | integration tests | parser unit tests |
|---|---|---|---|---|
| `KAYFABE_PT_SWEEP` | **deleted** w533 | — | 0 | **0** |
| `KAYFABE_PT_WITNESS_EXEC` | **deleted** w534 | — | 0 | **0** |
| `KAYFABE_MMU_INVAL` | **deleted** w535 | — | 0 | 2 |
| `KAYFABE_OPERAND_JOIN` | **deleted** w536 | — | 0 | yes |
| `KAYFABE_GR_ROUTE` | pending | 14 | 2 | yes |
| `KAYFABE_GUEST_RING` | pending | 21 | 3 | yes |
| `KAYFABE_VAS_PUBLISH` | pending | 37 | 2 | yes |
| `KAYFABE_FB_JOIN` | pending | 66 | 2 | yes |

⊘⊘ **CORRECTED w535, and the correction matters.** I first published *"not one test ever
covered either deleted arm"* off a grep of `crates/*/tests/*.rs` — which does not see
**in-crate `#[cfg(test)]` modules**. The gate caught it: deleting `KAYFABE_MMU_INVAL` broke two
unit tests my audit had said did not exist. **A test-coverage claim is only as wide as the
places you looked**, which is this tree's census rule pointed at itself.

★★★ **The re-audit makes the thesis SHARPER, not weaker.** What those tests cover is the
**parser**: *"`off` parses to `Off`, and `OFF`/`1`/`true` are refused"*. None of them exercises
the arm's **behaviour**. So deleting an arm deletes a parser test for a parser that no longer
exists, which is correct rather than a loss of coverage. ⊘ And `KAYFABE_MMU_INVAL`'s own test
doc said it outright: *"`off` remains SPELLABLE … It is scheduled for deletion with that path's
landing."* The code had already scheduled its own removal.

★★★★★ **GRADED ON HARDWARE, w537 — the four deletions hold.** Fresh box, fully provisioned,
driver 580.159.04 both sides:

```
W392D_GUEST_OUTCOME=(P)   MEAN_FALSIFIER=PASS   THREADS 8 of 8   panics=0
inline_exceptions=0   VCPU-BLOCKING none   worst_trap=1154us   slow_traps=1
```

⊘ `boot_capture rc=5` is the **evidence-persist** check (*"only 4/3 evidence files reached
traces/guest_boots"*), not a boot failure — the client ran and graded. ⚠ Read it before reading
the numbers; `rc=2` on the previous attempt meant the observation was never made at all.

⚠ **The 1154 µs worst trap is NOT attributable to these deletions.** This is a different
machine from the w525/w530 measurements (16 cores against 24, different CPU), and the trap
figure moves with the box. What the boot establishes is the thing it was run for: **the client
still passes and every invariant still holds with four arms removed.**

⇒ The measured claim, stated correctly: **no arm on this list has a test of its alternative
BEHAVIOUR, and none has a graded boot on it.** That is a branch nobody can vouch for.

⚠ **The ones with integration tests are the ones to slow down on**, not speed up. Those pin the
arm to observe something, so the arm's deletion takes their subject with it (§5).

---

## 3. An arm, a degradation, and a debug flag are three different things

This is the distinction the deletions turn on, and getting it wrong is how the `off` arm rotted.

- **An arm** is a behaviour someone can select. ⇒ **Delete all but one.** A second value that no
  graded boot uses cannot be trusted to work, so offering it is a lie.
- **A degradation** is what the one behaviour does under pressure. It is **not selectable** and
  it **must be counted**. The owner's own ruling is the template: *"if the queue is full then a
  flag must be set that the refresh considers the entirety of PTE/PDB dirty (i.e it rescans
  everything). then its correct, only a bit slower on full queue."*
  ⚠ A degradation with no counter is indistinguishable from a path that never runs — which is
  precisely how `drain_fills` being worker-only survived since w472a.
- **A debug or metric flag** observes without changing behaviour. ⇒ **Keep.** `KAYFABE_KFTIME*`,
  `KAYFABE_SLOW`, `KAYFABE_DECLARED`, `KAYFABE_DOORBELL_VAS_CENSUS`, the build stamps, and the
  environment facts (`KAYFABE_NO_KVM`, `KAYFABE_NO_SANDBOX_NS`, `KAYFABE_ISOLATE_BIN`).

⚠ **The temporary instruments go LAST, not first.** `KAYFABE_STALL_ALARM_US` / `_AT` / `_WORKER`,
`KAYFABE_TRAP_FATAL*` and `TRAP-CPU` are on the cut list and must eventually go — but the
targeted alarm is what named the last two stalls of the campaign, and it is the tool that would
diagnose a bad consolidation. Cut them after the consolidation is green.

---

## 4. What is a commitment, not a cleanup

⊘ Two items on the production list **do not exist yet**. Collapsing to them means building them,
and each needs its own boot so a regression can be attributed:

1. **The doorbell table.** Wire it, measure it, then the old routing becomes deletable.
2. **BAR0 reads from a memslot.** Today all thirteen read arms are computed per access. Sorted by
   whether a page can hold the answer: **static** (boot registers, VBIOS — the cold-boot trace
   has 139 821 VBIOS reads); **producer-updated** (queue head, invalidate completion, interrupt
   leaves, window latch — only we change these, so the page can be written when they change);
   and the **free-running counter**, which advances continuously.
   ★ Owner, this session: the counter **is mappable from host userspace**. Its page
   (`0x00BB_0000`) holds, of what we decode, only `TIME_0` at `+0x80`, `TIME_1` at `+0x84` and
   the **doorbell** at `+0x90` — so a read-only memslot over it yields reads-pass-through and
   writes-trap, which *is* the BAR0 contract, and the doorbell write still traps as it must.
   ⚠ **Two things to design for, not discover.** (a) **Timebase coherence**: everything we
   synthesise must then come from the same counter, or a guest computing a timeout from hardware
   time while reading timestamps we forged from CPU time will disagree with itself. The native
   trace measured GPU against CPU wall time at **43 ppm** — close, not equal. Source the clock
   from the same mapping. (b) **Plumbing**: `install_file_window` takes an fd, and
   `WindowBacking` has three variants, none of which is a mapped address. A memslot only needs a
   userspace address, so this needs a fourth backing, not a redesign.

---

## 5. What the deletions cost, stated up front

**Five test files pin `KAYFABE_DOORBELL_ASYNC=off`** so they can observe a synchronous answer:
`e2_doorbell.rs`, `gr_route_passthrough.rs`, `reap_composition_root.rs`,
`pending_latch_epoch.rs`, `no_worker_still_drains.rs`.

⚠ Deleting that arm takes their observation seam with it. Reworking them carelessly turns green
tests into **vacuous** ones, which is this tree's most catalogued failure
(`a_green_test_can_hold_a_wall_in_place`, `every_row_verified_over_zero_rows`). The seam must be
replaced deliberately: drive the worker and assert the deferred outcome, or expose a test-only
constructor. ⊘ **A test seam is not a production arm** — it belongs in a constructor parameter,
not in an environment variable read at the composition root. The same applies to
`KAYFABE_ISOLATES=stillborn`, which is what lets the suite run with no GPU.

---

## 6. How a deletion is graded

Every step keeps the raw client green, which is the only end-to-end proof there is:

```
W392D_GUEST_OUTCOME=(P)   MEAN_FALSIFIER=PASS   THREADS 8 of 8   panics=0
VCPU-BLOCKING none        inline_exceptions=0   slow_waits=0 on every rank
```

⚠ **Read `W392D_GUEST_RC` and the census line before reading any number.** `[measured w523]` a
boot that died part-way produced *better* trap figures than the fixed one, because a shorter run
has fewer slow traps for free.

⊘ **And say plainly what the raw client does not cover.** It exercises the MMIO contract, the
locks, doorbell routing, the sweep and publication. It never launches a compute kernel, so it is
no evidence about the compute plane, multi-process, or the cudart control surface. Consolidating
on it is a deliberate trade, taken because the LLM lane is currently uninterpretable.

**Deletions and wiring never share a boot.** A pure deletion that still passes is meaningful;
new behaviour needs its own grade. `[measured w523]` the lock split and the reap move landed
separately, and that is the only reason the dead boot could be attributed to the split.

**Prevent the recurrence:** once an arm is gone, **refuse to start if its variable is set**. A
stale script exporting a removed flag must fail loudly, not be ignored while someone reads the
results as though it applied.

---

## 7. Retired by this document

⊘ **The `off`-arm no-worker hypothesis (w532) is OVERTAKEN, not confirmed and not refuted.** I
pre-registered that adding a vCPU-side mirror drain would let that arm initialise the adapter,
and the fix is in the tree, ungraded. §2 deletes the arm, so the path cannot rot because it will
not exist. Recording it here rather than letting an unmeasured claim sit in the history.
