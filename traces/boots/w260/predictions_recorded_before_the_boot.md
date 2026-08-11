# w260 — PREDICTIONS, recorded BEFORE the boot

**Branch:** `fb-join-port`, based at `096a730`. **Nothing on this branch has ever booted.**
**Bench:** `vh`, RTX 3060 GA106, driver 580.159.04.
**Harness:** `scripts/bench/boot_capture.sh`, repo helpers, `KAYFABE_ISOLATES=real`.

**Arms, all from ONE binary and ONE stamp:**

| arm | `KAYFABE_FB_JOIN` | what it is |
|---|---|---|
| `w260_off`     | unset / `off` | the **arming control** — materializes nothing, prints no `GR-FB-JOIN` operand line |
| `w260_shared`  | `shared`      | ★ **the arm** — `MAP_SHARED` over the isolate's sealed memfd; one backing, two mappings |
| `w260_private` | `private`     | ★★ **the negative control** — VMM side is `MAP_PRIVATE\|MAP_ANONYMOUS`; the two views MUST disagree |

Plus one **pre-boot, guest-free** probe: `rmladder --fb-memfd-join` / `--fb-memfd-join-negative`
(R32). ⊘ `traces/real_ga106/` carries R18–R30 and **no R32** — that probe has never been run on
hardware, so the mechanism the whole branch rests on is currently argued, not measured.

---

## ★★★ 0. THE PREDICTION THAT CONTRADICTS MY OWN BRIEF — recorded first, on purpose

The brief names four unknowns. **I predict this boot can answer only ONE of them (#4), plus
the probe's detection power, and that #1, #2 and #3 are STRUCTURALLY UNREACHABLE on this
workload.** Reasoning, from the code and from `w228`'s measured census:

- **#1 (install→bind ordering)** is observable *only on a failing install*. When the install
  succeeds, "bind before install" and "bind after install" have **identical end states**. A
  green boot does not discriminate them.
- **#2 (`release_unadopted_fb_leaf`, `0x51` avoidance)** runs on **failure paths only** — dup
  refused, `mmap` refused, install refused. A green boot never enters one.
- **#3 (attempt-once `refused` set)** needs **two census operands in ONE leaf**. `w228`
  measured three FB operands in **three distinct leaves**:
  `SET_VALID_SPAN_OVERFLOW_AREA` va=`0x200000000` phys=`0x400000`;
  `SET_TEX_SAMPLER_POOL` va=`0x10002000000` phys=`0x800000`;
  `SET_TEX_HEADER_POOL` va=`0x10000000000` phys=`0x600000` — all len=`0x200000`.
  With one operand per leaf the `refused` set is **never consulted for a second time**.

⇒ **If P10 below holds, three of the brief's four unknowns are not answered by this boot, and
saying so is the deliverable.** They need fault injection or a different workload, not a boot.
⊘ The boot is still worth spending: #4 is the branch's load-bearing claim, it is *cited and
never reproduced*, and the `private` arm converts it from an assertion into a measurement.

---

## 1. Provenance and arming

**P0 — the binary is THIS revision.** `strings qemu-system-x86_64 | grep kayfabe-rev:` equals
this commit's own sha, with **no `-dirty` suffix**, on all three tags.
⊘ **REFUTED BY:** any other sha, `unknown`, `-dirty`, or more than one distinct stamp.
(The bench once served a binary from an old revision for weeks. This is the gate, not a note.)

**P1 — the arm is stated by the boot's own on-disk evidence.** Each `run_<tag>_qemu.log`
contains exactly one `kayfabe: FB-JOIN arm=<arm> exports_directory=true` line, and the arm
matches the tag.
⊘ **REFUTED BY:** `exports_directory=false` (this build has no route from a backing token to
a descriptor — the whole chain is then inert and every green below is vacuous), a missing
line, or an arm that disagrees with the tag.

**P2 — the census shape does not move.**
`kayfabe: GR-ADDRESS-CENSUS proc=2 chan=0 class=0xc7c0 operands=5 bound=4 unbound=1 mme_dwords=39`,
identical on all three arms. The census counts **binding**, not backing.
⊘ **REFUTED BY:** any different `operands`/`bound`/`unbound`.

**P3 — three FB operands, three DISTINCT leaves,** at exactly the VAs and phys listed in §0.
⊘ **REFUTED BY:** any two FB operands resolving to one `leaf va` — which would make unknown
#3 reachable, and I would want that. This is a prediction I would be glad to lose.

---

## 2. The headline — unknown #4

**P4 — the ported chain reaches JOINED on real hardware.** On the **first** GR doorbell of
`w260_shared`, exactly **three** lines of the form:

```
kayfabe: GR-FB-JOIN proc=2 chan=0 <OPERAND> leaf va=0x… len=0x200000 fb_phys=0x… →
  JOINED (shared) memory=0x… host_va=0x<the same VA> placed_as_asked=true established=… bytes
  over … page(s), of which … NON-ZERO
```

with `placed_as_asked=true` on **all three**.
⊘ **REFUTED BY:** any `⊘ REFUSED BY NAME`, any `THE BACKING CROSSED AND THE VMM COULD NOT
CLAIM IT`, any `THE VMM'S OWN MAPPING FAILED`, any `THE INSTALL REFUSED`, any `THE BIND
REFUSED`, any `placed_as_asked=false`, or fewer than three JOINED lines.
⚠ If the refusal is `Rm(NoMemory)` that is status `0x51`, which is
collision-or-exhaustion and **cannot be told apart** — I will report it as undiscriminated,
not as exhaustion.

**P5 — the probe runs EXACTLY ONCE per boot, on doorbell 1.** `live` is pushed only on a
non-replay install; doorbells 2..8 take the `ALREADY JOINED (idempotent replay)` arm and
`continue` before `live.push`. So each `shared`/`private` boot prints **1** `DIRECTION 1`
line, **1** `DIRECTION 2` line, and **7** × `⊘ NO PROBE: no leaf reached a live view this
doorbell`.
⊘ **REFUTED BY:** ≠1 probe, or ≠7 `NO PROBE` lines, or any `⚠ PROBE MISS` (the isolate
holding no joined range covering a leaf it just said it joined — that would be a direct
self-contradiction and gets reported first).

**P6 — `shared`: BOTH DIRECTIONS AGREE over 1024 words.**
`★ DIRECTION 1 (guest view → isolate view) … AGREES over 1024 words` **and**
`DIRECTION 2 (isolate view → guest view) … AGREES over 1024 words`.
⊘ **REFUTED BY:** either `DISAGREES at word i`, or a `PROBE ABORTED`.

**P7 — ★★ `private`: BOTH DIRECTIONS DISAGREE, at word 0, with SPECIFIC values.** This is the
prediction whose failure would make P6's green worthless, so it is stated to the word.
With `g2h = (fb_phys as u32) ^ 0x5a5a_5a5b` and `h2g = !g2h`:
- The three `JOINED (private)` lines still appear — the arm changes **only** the VMM's `mmap`
  backing, one line of code, downstream of everything RM does.
- `DIRECTION 1 … DISAGREES at word 0 (got 0x00000000, want 0x<g2h>)` — the isolate's mapping
  was never written by the VMM, so it reads zeros.
- `DIRECTION 2 … DISAGREES at word 0 (got 0x<g2h>, want 0x<h2g>)` — the guest side still holds
  what **direction 1** wrote into its private page, and the isolate's poke never crossed.
⊘ **REFUTED BY:** `private` AGREEING in either direction ⇒ the probe measures nothing and P6
is not evidence of a join. ⊘ Also refuted, more weakly, by `got` being zeros in direction 2
(that would mean the private page is not even holding direction 1's own write, i.e. the
guest-side path is broken in a second, unrelated way).

**P8 — the establishment copy is VACUOUS on all three leaves.** `nonzero=0` on every JOINED
line, and the `⊘ the establishment copy was VACUOUS for this leaf` note fires for each leaf
whose `established=0`. Rationale: these are texture/span pools the guest has not written
before the first GR doorbell. ⚠ **Low confidence** — recorded because a guess I would
otherwise rationalise afterwards.
⊘ **REFUTED BY:** any `NON-ZERO` count > 0. That would be *better* news than the prediction:
it would mean real guest bytes crossed into the host object, which is the copy doing work.

---

## 3. The SIZE of the change — the standing pre-registration (task #231)

**P9 — this boot moves the execution and completion planes by EXACTLY ZERO.** Specifically:
`CE-SUBMIT` stays **0**; `COMPLETION-WATCH … → NOT-OBSERVED samples=88` × 8 unchanged from
`w251`/`w256`; `Route::NotACopyEngineChannel` still refuses every `GrCompute` doorbell; `cup2`
does not advance one rung; `RING-PROJ` 0.

★ And the reason this is **not** evidence against #231's *"if passthrough is right, advances
should be large and discontinuous"*, stated **before** the log exists so it cannot be a
rationalisation: #231 quantifies over changes that touch the **execution** path. This branch
adds **no route to an engine at all** — the doorbell refusal is upstream of everything here
and is untouched by the diff. A supply-side precondition landing with zero execution movement
is the predicted shape, not a disappointment.
⊘ **REFUTED BY:** any movement on the completion or submission plane. If joining three
framebuffer leaves causes a semaphore to be observed, or `CE-SUBMIT` to become non-zero, **my
model of this branch is wrong, it is enormous, and it is the first thing reported.**

**P10 — ★★★ zero failure-path lines, therefore unknowns #1/#2/#3 UNANSWERED.** Across all
three tags: **zero** occurrences of `RELEASED and NOT bound`, `THE INSTALL REFUSED`, `THE BIND
REFUSED`, `COULD NOT CLAIM IT`.
⊘ **REFUTED BY:** any of those lines — which would *answer* unknown #1 or #2 and would be
reported ahead of P4.

---

## 4. The pre-boot mechanism probe (R32, no guest)

**P11 — `rmladder --fb-memfd-join` PASSES on this GA106, and its negative FAILS.** J1 (write
through the VMM's mapping, describe the isolate's, GPU reads the VMM's bytes) and J2
(GPU-write → CPU-read through the memfd) both agree; `--fb-memfd-join-negative`
(`OsDescSeed::Never`) reports the disagreement.
⊘ **REFUTED BY:** J2 failing. ★ That would refute the branch's **mechanism** before the guest
is involved at all — it is the direction the stuck completion semaphore needs and the
direction no `OS_DESCRIPTOR` evidence in this tree has ever run. If J2 fails, **I do not spend
the boot on P4**, because P4 would then be measuring an integration over a broken primitive.
⊘ Also refuted by the *negative* passing, which would mean the probe cannot detect.

---

## 5. Scoring rule, fixed in advance

Every prediction above gets **HELD / REFUTED / UNMEASURED**, in that vocabulary, including
P8/P9/P10 which I expect to be the interesting ones. ⊘ `UNMEASURED` is a real verdict and is
**not** a pass: it means the boot produced no line that could discriminate — the `dlen=0`
lesson, applied to my own predictions.
