# The rewrite — what is deleted, what is kept, what is rewritten

**STATUS: LIVE, 2026-09-21 (w822).** The operational half of `THE_DESIGN.md`: what happens to the
144 175 lines that exist today, and how the work is kept cheap enough to iterate on.

---

## 1. The three buckets

⊘ **The deletions are not edits inside the existing files.** They remove the *organising ideas*
those files are built around — per-process isolation, an address table, a publication pass. Taking
them out incrementally means running two architectures in one file set for the duration, which is
exactly how the contradictory defaults and stale comments got there.

### 1.1 ⊘ DELETE — the idea goes, so the code goes with it

| what | ~lines | why it cannot survive |
|---|---|---|
| `kayfabe-isolate-host` + `kayfabe-isolate` | **~52 700** | One process (§3). The IPC plane, fd passing, the `Wedged` state and the foreign-handle gate exist only because there was a boundary. ★ And it is the *measured* source of a live defect: a cross-process round trip on a vCPU under a lock |
| the address table | ~8 000 | Replaced by mirroring. There is no reverse-resolution step to hold a table for |
| joins (all four generations) | ~4 000 | An artifact of per-process VA identity |
| VA→phys translation on the submission path | ~3 000 | Operands are rewritten into our VA space, not resolved to physical |
| publication epochs, the dirty gate, sweep-skip | ~4 000 | The publish trigger is the invalidate, not a pass we schedule |
| `Proc` and the per-process container | ~5 000 | ⊘ A process is not a GPU concept. v3 speaks in channels, VA spaces and RM objects |
| the CPU copy executor | ~2 500 | §46 — real work goes to the GPU. It is *how the scrub became a leak* |
| the completion watch and its 250 ms sweep | ~1 500 | Passthrough and translated completions are hardware's; emulated ones are forged at the call |
| the framebuffer probe/rebind | ~800 | The store's size is a command-line parameter |
| every on/off flag for a thing with one side left | ~1 000 | §42(d): an opt-in may move, not disappear — but a flag whose other arm is deleted is not an opt-in |

⇒ **~82 500 lines deleted outright.**

### 1.2 ★ KEEP — and these are why a rewrite is affordable at all

| what | ~lines | why it survives |
|---|---|---|
| **the test suite** | ~113 000 | ⊘ **Not rewritten.** It is the grading instrument, and a rewrite that changes its own grader cannot be graded. ⚠ The *verdict producers* it calls must be re-pointed; the assertions must not move |
| the ogkm-derived constants, **regenerated** | ~14 000 | The values are right; the *source* changes from hand-maintenance to generation |
| the chip/format descriptors | ~3 000 | Per-die and per-family data, already the right shape |
| the PTX page-table walker | ~2 000 | ★ Validated 72/72 on hardware, hostile + racer + real-driver diff. **The PTX is not the risk** |
| the measured numbers in comments | — | §10: keep measurements, audit assertions. A dated measurement does not rot |

### 1.3 ✎ REWRITE — same job, incompatible shape

| what | from → to |
|---|---|
| the vCPU trap path | a classifier with two arms → **three**, with the token table, the wakeup word and the lock-free ring (§5) |
| the register plane | shadow + ad-hoc handlers → **generated descriptors** carrying write- and fence-semantics |
| channels | `Translated` typed but never constructed → **built**, and it is what closes the scrub leak |
| the host plane | isolate RPC → **in-process authored verbs** on two descriptors (§9) |
| the VMM seam | ~19 000 lines of mirror machinery → **dispatch only**, once the exit-site hook lands |

---

## 2. ★★★ How iteration is kept cheap — the thing that decides whether this finishes

⊘ **A full guest boot per idea is how the last architecture took months.** Four lanes, in cost
order; a change is answered by the cheapest one that can answer it.

| lane | cost | answers | when it is NOT enough |
|---|---|---|---|
| **GPU-free unit** | seconds | every protocol interleaving in §5 — lost wakeup, summary race, four-state claim, ring poison, cross-vCPU order | anything touching real RM |
| **raw client, bare metal** | ~2 min | ★ every host-side verb, the mirror, the store, channels, CE — **with no guest at all** | anything about what the *guest driver* does |
| **thin guest** | ~40 s/arm | the guest's own boot and its first channel, five ledger rows | full CUDA |
| **full guest** | ~10 min | the CUDA ladder and the LLM lane | — |

⊘⊘ **And one ordering constraint discovered the moment the lane was run, w822:** the raw client's
binary **lives inside `kayfabe-isolate-host`** — the largest crate on the delete list. ⇒ **It must
move to its own crate before that deletion**, or the grading instrument disappears with the thing
it grades. ★ This is the general shape to watch for: *the tool that measures a subsystem often
lives inside it*, and a delete list built from architecture alone will not see that.

★ **The raw client is the keystone and it is why bare-metal-first is not optional.** It exercises
our host plane through the same verbs the guest lane uses, *without* a guest — so a failure there
is unambiguously ours. ⇒ **A guest failure only indicts kayfabe once bare metal passes**; before
that, the guest lane cannot distinguish our bug from the client's.

⚠ **And the gate must be honest about hardware.** A green ledger does not prove hardware ran the
work — a CPU-only arm passed all five rows. The discriminator is **`forwarded=` per token**.

---

## 3. The path to a passing raw client

1. **Reproduce the baseline** — 27/30 on this box, so the 3 failures are the work, not the noise.
2. **Take the three** in dependency order: they are `--engines`, `--concurrency`,
   `--concurrent-fuzz`, and the first is a prerequisite — ⊘ engine objects have been
   `forwarded=0 refused=8` *ever*, so concurrency arms that need an engine cannot pass beneath it.
3. **Each fix lands with its GPU-free test first**, so the suite proves it rather than the box.
4. ⊘ **Never re-enable a local arm to get a pass.** The number to move is *"a forwarded CE copy
   retires"*, and until it does the honest score is the one the GPU earns.

## 4. The ladders, and what each is allowed to prove

| ladder | proves | ⊘ does not prove |
|---|---|---|
| CUDA: `cuInit` → `cuCtxCreate` → matmul `bad=0 maxerr=0` | the control plane and the data plane end to end | steady-state performance |
| LLM | residence, the allocation path, per-token cost | correctness — it is green on wrong data |

⚠ **Run every gate twice in one process.** A plane that works on the first init and not the second
does not work, and the stock driver **refuses to boot while the protected firmware region is up**.

---

## w823 — LANE 2 IS GREEN: the raw client passes bare metal 30/30

**Measured 2026-09-21**, box 51815459 (RTX 3060, driver **580.159.04**, open module), rev
**`3dcfd772`**, `scripts/fastguest/bare_metal_suite.sh`:

```
BARE_SUITE_PASS=30 BARE_SUITE_FAIL=0 BARE_SUITE_CRASH=0 ARMS=30
```

★ Slowest arms: `--gpga-reserve-probe` 15 s, `--defer-liveness` 14 s, `--ce-client-guest-ram` 10 s;
**whole suite well inside the 120 s/arm budget.** ⇒ The ~2 min lane-2 figure in §4's ladder is
confirmed by measurement, not estimated.

⇒ **This is the licence the plan needed.** `BARE-METAL PASS + GUEST FAIL ⇒ KAYFABE BUG` is only
usable as an indictment if the bare-metal side is actually green; at w814 it was **27/30**, so
three arms could always be blamed on the client. **It is now 30/30**, and every guest-side failure
from here indicts the VMM.

### ⊘⊘⊘ AND THE ROAD THERE IS THE LESSON — the same suite reported `FAIL=30` TWICE

Both prior runs tonight printed:

```
BARE_SUITE_PASS=0 BARE_SUITE_FAIL=30 ... --map-propagation FAIL 0s  client rc=1
```

**The truth was `FAIL=0`. A total inversion — and the harness NAMED THE DEFENDANT**: `client rc=1`
is an accusation against `kayfabe-rm-ladder`, which was innocent thirty times over. The actual
cause was that `/workspace/bench/` does not exist on this box, so `> "$BENCH/bare_$arm.log"`
failed, the redirect took the shell's exit status, and the suite recorded it as the client's.

★★★ **This is worse than a silent instrument, and the difference is the point.** A harness that
reports nothing gets investigated. A harness that reports a **confident, specific, well-formatted
verdict pointing at the thing under test** gets believed — and would have sent the next day into
debugging thirty passing arms. ⚠ Same class as `A CHECK THAT REPORTS IS NOT A CHECK THAT GATES`
and `failed=0 IS NOT "NOTHING REFUSED"`, but inverted: here the instrument did not under-report,
it **manufactured** a failure and attributed it.

⇒ Two fixes, both landed:
1. `bare_metal_suite.sh` now **probes the log directory for writability and refuses by name**
   (`HARNESS precondition, not a client result`, `exit 3`) before running a single arm.
2. ⊘ **The runner must `git pull` on the box.** `bare2.sh` built and ran without fetching, so the
   pushed fix in (1) **was not on the box** and the re-run reproduced the identical output — which
   read as *"the fix didn't work"* rather than *"the fix wasn't there."* `bare3.sh` does
   `git fetch && git reset --hard origin/<branch>` and **echoes the rev**, and the suite header now
   prints `rev=` too. ⚠ Generalise it: **a remote lane must state the revision it measured**, which
   is the rule `docs/BENCH_REBUILD_NOTES.md` already paid for once in the C tree.
