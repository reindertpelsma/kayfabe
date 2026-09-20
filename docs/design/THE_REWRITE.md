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

---

## w823 — ★★★ THE RAW CLIENT IS AN AXIS PROBE, NOT ONLY A REGRESSION TEST

> `[owner, 2026-09-21]` *"it's also useful if the raw client 30/30 can be run on other GPUs and
> drivers later, as it's a test for when iterating kayfabe itself on the axis matrix."*

★★★ **Adopted, and it changes what lane 2 is FOR.** The seven axes (§0 of
`THE_ARCHITECTURE_v3.md`) have always had the same problem: they are enumerated but **unmeasured**,
and the only lane that could measure them — the full guest — needs a KVM box per cell. This lane
does not.

### Why it works, stated precisely

**There is no VMM in it.** The suite runs the same binary the guest runs, against the real driver
on the real card, with no QEMU and no KVM. ⇒ It isolates the **`(GPU die × host driver)`** cell
from everything kayfabe does. A red arm here is a fact about the **die or the driver**, never about
our emulation — which is the property that makes the result attributable at all.

### ★★★ Two consequences, and the second is the one that pays

1. **It needs no KVM box.** `[owner's rule]` *"a kvm box can be used for both kvm gpu, non kvm gpu,
   cpu. a container only for non kvm gpu and cpu."* Bare-metal raw client is **non-kvm-gpu** ⇒ a
   plain CUDA container suffices. Containers are cheap and available across far more GPU types than
   VMS-enabled hosts. ⇒ **Die coverage scales on this lane and cannot scale on the guest lane.**
2. ★★★ **Several arms are already per-die discriminators**, so on an unseen die the suite is a
   **difference detector**, not a pass/fail gate:

   | arm | what it discriminates |
   |---|---|
   | `--doorbell-census` | ⚠ `blackwell_doorbell_encoding_differs_per_die_group` — **GB202 sets bit 30 where Ampere does not.** This arm would have found that |
   | `--pce-mask-probe` | copy-engine PCE topology, which is per-die |
   | `--gpga-reserve-probe` | the reservable GPGA window |
   | `--atomics-probe` | atomic support on the memory path |
   | `--bar1-crossing` | BAR1 aperture geometry |

   ⇒ **A red arm on a new die is a FINDING, not a defect** — it is precisely the input
   `DERIVE PER DIE, MAINTAIN PER FAMILY` needs, delivered before any kayfabe code is written for
   that die.

### What was built for it (w823)

- `bare_metal_suite.sh` now **identifies the cell and refuses without one**: `gpu`, `driver_version`,
  **`pci.device_id`** (the die), `compute_cap`, and the **module flavour** (`open` vs `proprietary` —
  ⊘ these are *different host drivers* at the same version string, so the flavour is part of the
  cell, not a footnote). It emits `BARE_CELL_ARM …` per arm and one `BARE_CELL …` summary, both
  machine-readable and both carrying the cell.
  ⚠ It **refuses** when `nvidia-smi` cannot identify the GPU: a row that cannot say which die it
  describes **poisons the matrix it lands in**, and is worse than a missing row —
  `no provenance looks CLEANER than bad provenance`.
- `scripts/fastguest/axis_cell.sh` — one cell end to end on a fresh container box: pull, build, run,
  emit. ⊘ It distinguishes a **build** fault from a **cell** result by name, because a failed build
  reported as a failed cell is the same misattribution that cost tonight two cycles.

### ⊘ The limits, so this is not over-read

- It bounds **one axis pair**. Guest kernel, guest driver, VMM and guest OS are untouched — those
  still need the guest lane.
- A green cell says the **client and the driver agree on that die**. It does *not* say kayfabe will
  work there; it says that when kayfabe fails there, the failure is ours. That is exactly the
  `BARE-METAL PASS + GUEST FAIL ⇒ KAYFABE BUG` rule, extended from one die to a matrix.
- ⚠ Arms are **not** equally portable. `--gpu-info-sweep` and `--bus-info-sweep` assert values that
  may legitimately differ per die; when this first runs on a non-GA10x part, expect reds that are
  **assertions needing a per-family overlay**, not bugs. ⇒ Triage a new cell's reds against that
  table above *before* touching code.

**First cell on the record (w823)** — `0x2504` is **GA106**, verified from the running box, not
inferred. ⚠ I first wrote `0x2522` here from memory of the 3060 family and it was **wrong**; the
device id is the matrix's primary key, so a plausible-looking guess in that field is the single
worst thing this doc could contain. ⇒ **Every cell row is pasted from a run, never typed.**

```
BARE_CELL pci_dev=0x250410DE gpu="NVIDIA GeForce RTX 3060" drv=580.159.04 kmod=open cc=8.6 rev=3dcfd772 pass=30 fail=0 crash=0 arms=30
```

---

## w823 — ⊘ LANE 3 BLOCKED BY THE PROVISIONER, AND THE REFUSAL WORKED

The thin-guest lane scored **30 CRASH / 0 PASS**, every arm at 0 s:

```
--timer   CRASH  0s   the serial log is EMPTY — QEMU wrote nothing at all
```

★ **And this time the harness did NOT accuse the client.** It said the serial log was empty and
named that as its own distinct state. ⇒ The lesson from the bare-metal misattribution was already
encoded in *this* lane (`[measured w812]`), and it paid: the next question was *"what did QEMU
say"*, not *"which thirty arms are broken"*. **That is the whole value of refusing by name.**

QEMU's own log gave the answer in one line, also by name:

```
nvkvm: the register plane refused to build (3): KAYFABE_ISOLATES asked for a host isolate
plane, and this archive was built without the `host-isolates` feature — it does not link
`kayfabe-isolate-host` and cannot spawn anything.
```

### ⊘ The defect is in the RECIPE, not the code

`scripts/bench/provision_bench_tree.sh` builds QEMU with **default features**, and
`scripts/fastguest/run_fast_guest.sh` requires an archive built with **`cuda-scratchpad`** (which
implies `host-isolates`) — a requirement it prints as advice at line 148 and **does not check**.

⇒ **A fresh box provisioned by the documented recipe produces a QEMU that lane 3 cannot use**, and
nothing between the two says so until thirty boots have failed. ⚠ Same class as
`THE NEW DESIGN WAS UNREACHABLE BY DEFAULT` (w760, five arms defaulting to superseded
architectures): the pieces are each correct and their **composition** is not.

**Fix (w823):** the provisioner builds with `KAYFABE_SHIM_FEATURES=cuda-scratchpad`, and
`run_fast_guest.sh` turns its advice into a **precondition** — the archive is checked for the
feature before a single boot, not described in an echo. ⊘ A printed instruction is not a check;
this tree has the rule already (`A CHECK THAT REPORTS IS NOT A CHECK THAT GATES`) and line 148 was
a live instance of it.
