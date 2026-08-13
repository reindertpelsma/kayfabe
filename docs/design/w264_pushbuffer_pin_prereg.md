# w264 PRE-REGISTRATION — leg 4, and ⊘ THE BRIEF'S MECHANISM IS THE WRONG ONE

> ### STATUS — 2026-08-12 / **LIVE — PRE-REGISTERED, NOT YET RUN.**
> Written and committed **before** the code. Branches off `leg-b-userd-adoption-at-creation`
> = `0134a0a`. Bench `vh`, RTX 3060 (GA106), host driver 580.159.04.
> Graded in `traces/boots/w264/RESULT.md`.

---

## 0. ⊘⊘ WHAT I REFUTED BEFORE WRITING A LINE OF CODE — and it changes the whole rung

The brief says, in its own words:

> ⇒ **Leg 4 (the pushbuffer pages) has arrived as hardware**, and hardware named the addresses:
> **`0x202400000 … 0x203200000`, `0x200000` apart, Vidmem, `vas=0xcafe0005`** — and
> `join_fb_leaf` **already reaches them**.
>
> ## ITEM 1 — JOIN THE PUSHBUFFER LEAVES (the rung)

**Both halves of that are false, and they are false in the committed `w263` log the brief is
quoting from.**

### 0.1 The pushbuffer pages are in GUEST RAM, not Vidmem

`[measured, `traces/boots/w263/run_w263_ring_qemu.log`, all 8 channels, both arms]`

```
gp[0]@0x200218000=0x202400000+0x20 pb=S:0x3d45f000
gp[0]@0x20021b000=0x202600000+0x20 pb=S:0x3d65f000
…                                  pb=S:0x3e25f000
```

`CeResolve::tag`'s own doc (`crates/kayfabe-device/src/ceresolve.rs:441-443`) is the
authority on that letter: *"its aperture letter (`V` = this device's framebuffer, **`S` =
guest RAM**, `P` = peer)"*. Every one of the eight pushbuffer VAs resolves **`S`**. The
`Vidmem` in the brief is the aperture of the **ring** — the same line's `rng=V:0x1024000`, and
the `FwdFault::PushbufferAperture { va: GpuVa(8592179200), aperture: Vidmem }` whose `va`
decodes to `0x200224000`, the **ring's** VA, not a pushbuffer's.

⚠ The off arm resolves the **same eight VAs** to **different** GPAs
(`S:0x2bc63000`, `S:0x1dce3000`, …). Same VAs, different physical pages, different boot —
which is itself the reason no address here may ever be hard-coded.

### 0.2 ⇒ `join_fb_leaf` cannot reach them, and the refusal is by construction

`kayfabe_rt::ceutils::resolve_leaf_of` (`:1002-1018`) answers
`(Site::GuestRam { gpa }, **None**)` for a `CpuPlane::GuestRam` resolution, and its own comment
says why: *"it is not this source's to join: **the guest-RAM pin owns that plane**."* Leg A1
passes exactly that `None` to its *"NOT A FRAMEBUFFER LEAF"* arm. ⇒ An `ITEM 1` built as
written would have added a third source to the join, run it over eight guest-RAM addresses,
printed eight refusals, and **joined nothing** — a rung that cannot fire, indistinguishable
from a rung that fired and did not help.

### 0.3 ★★★ THE MECHANISM IS THE GUEST-RAM PIN, AND IT IS ALREADY BUILT — with no source

`SharedDoorbell::pin_ring_guest_ram` (`shim.rs:5307`) is the complete chain: VA → the core's
address table → GPA → **aperture check** → the hypervisor's own stated layout → file offset →
`GuestRamGrant` → `pin_guest_ram`, one `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` per contiguous run,
each mapped **FIXED at the guest's own VA**.

It is asked about exactly one address: **the ring's**. And on `w263` it refused all eight, by
name and correctly:

```
GUEST-RAM PIN … ring=0x200224000 → NOT IN GUEST RAM (the table binds this VA in aperture
Vidmem at 0x1024000; `Binding::phys` is a guest-physical address ONLY for sysmem …)
```

⇒ ★★★ **The pin has never pinned one byte on a live guest, and not because it is broken —
because the only address anybody hands it is in the wrong aperture.** The addresses in the
*right* aperture are on the very same log line, eight of them, and nothing presents them.

**That is leg A1's shape exactly, one plane over**: *the primitive works; the source list is
short*. So `ITEM 1` survives as an instruction and changes its verb:

> **give the guest-RAM PIN a second source: the pushbuffer VAs the ring's own GPFIFO entries
> name.**

### 0.4 ⊘ What this does NOT refute

The brief's *hardware* reading of `w263` stands untouched: eight `Xid 31`
`FAULT_PDE ACCESS_TYPE_VIRT_READ` at eight addresses byte-exact the pushbuffer VAs the guest's
`gp[0]` entries name, so the host PBDMA **did** fetch the guest's GPFIFO entry and walk to what
it named. `FAULT_PDE` says the host VAS has no page directory entry covering `0x202400000` —
which is precisely what a pin at that VA would install. The brief's diagnosis is right; only
its named mechanism is wrong.

---

## 1. WHAT THIS RUNG BUILDS

1. **`pin_pushbuffer_guest_ram`** — beside `pin_ring_guest_ram`, on the same forwarding
   fall-through, **before** `SharedDevice::doorbell`. For each **non-zero, decodable** GPFIFO
   entry of the channel's own ring: derive `[gpu_va, gpu_va + len_bytes)` **from that entry**,
   resolve each host page through the address table, refuse a non-sysmem binding **by name**,
   coalesce contiguous GPAs into runs, ask the hypervisor's layout per run, pin FIXED at the
   guest's own VA.
   - ⚠ **The stride is never assumed.** `0x200000` is what this workload produced. Every
     address is derived from its own entry, and the report prints the observed stride so a
     future reader can see it was observed and not encoded.
   - ⚠ **Bounded, and the bound is loud.** `MAX_PUSHBUFFER_EXTENTS = 64` distinct extents and
     `MAX_PUSHBUFFER_PAGES = 512` pages per doorbell. On overflow the report names
     `⚠ CAPPED` with the count dropped **and the first dropped VA**. ⊘ A silent truncation
     would be a false green; this one cannot be read as a complete pass.
2. **`KAYFABE_GUEST_PUSHBUF={off|pin}`** — its own arm, defaulting `off`, so the pin source is
   a *third* selector and not a silent rider on `KAYFABE_GUEST_RING`.
3. **`scripts/bench/w264_run.sh`** — per-arm env, so **each arm differs from the previous in
   exactly one variable**, which `w263_run.sh` did not (it exported
   `KAYFABE_FB_JOIN=shared` unconditionally, so its `off` arm was not `w262`'s control).

⊘ **NOT built, and named here rather than discovered later:** the third arm the brief asks for
(*A2 armed, leg B forced off*). Leg B's arming is **inherited by construction** — it is
`adopted_guest_userd`, reached only from inside `adopted_guest_ring`'s `Some`, in
`kayfabe-fwd`, which reads no environment by design. Forcing it off needs a boolean threaded
through `plan_engine_object`'s public signature and two `kayfabe-rt` call sites. The brief
called this *cheap* on the predecessor's word; it is cheap **only if a flag exists**, and none
does. §5 records what that costs and why it is not in this rung.

---

## 2. THE PREDICTIONS

Scored against `run_w264_{off,ring,pin}_qemu.log` unless stated. Three arms, one binary:

| arm | `KAYFABE_GUEST_RING` | `KAYFABE_GUEST_PUSHBUF` |
|---|---|---|
| `off` | unset | unset |
| `ring` | `ring` | unset |
| `pin` | `ring` | `pin` |

★ Each arm differs from the one above it in **exactly one** variable. `off`→`ring` is
therefore *the same comparison `w263` made*, and `ring`→`pin` is this rung's.

| # | observable | `off` | `ring` | `pin` | conf |
|---|---|---|---|---|---|
| Q1 | `PB-PIN` lines present | **0** | **0** | **≥ 1** | ★★★★ 0.95 — arming, arithmetic |
| Q2 | `PB-PIN … NOT IN GUEST RAM` | 0 | 0 | **0** | ★★★ 0.8 — §3.1 |
| Q3 | `PB-PIN … → PINNED` runs | 0 | 0 | **≥ 1** | ★★ 0.6 — **the rung**, §3.1 |
| Q4 | `PB-PIN … placed_as_asked=true` | — | — | **true on every pinned run** | ★★ 0.7 — §3.2 |
| Q5 | `PB-PIN … UNRESOLVED` | 0 | 0 | **0** | ★★ 0.55 — §3.1, the likeliest single failure |
| Q6 | `PB-PIN ⚠ CAPPED` | 0 | 0 | **0** | ★★★ 0.85 — `w263` shows `nonzero=[0]` only, 1 extent/ring |
| Q7 | host `Xid` count | **0** | **8** | **0** | ★★ 0.6 — ⊘ §3.3, and a *drop* is this rung's clearest possible win |
| Q8 | `fbuserd GET=` non-zero | 0 | **≥ 1** | **≥ 1** | ★★ 0.6 — `w263` measured 1 on `ring` |
| Q9 | `GUEST-RAM PIN … NOT IN GUEST RAM` (the **ring** pin) | 0 | **8** | **8** | ★★★ 0.85 — unchanged, and it must be |
| Q10 | `adopt=GUEST-RING` | 0 | 16 | **16** | ★★★ 0.85 |
| Q11 | `userd=GUEST-USERD` | 0 | 16 | **16** | ★★★ 0.85 |
| Q12 | `RmInitAdapter failed` | 0 | 0 | **0** | ★★★ 0.8 |
| Q13 | **`CUP2_RC`** | 124 | 124 | **124** | ★★ 0.7 — §3.4, **and I predict ZERO movement** |
| Q14 | `CE-SUBMIT → RETIRED` | 0 | 0 | **0** | ★★★★ 0.95 — leg 5 is unbuilt |
| Q15 | `ENGINE-OBJECT` census `seen/forwarded/refused` | 34/32/2 | 34/32/2 | **34/32/2** | ★★★ 0.8 — the arm-comparison guard |
| Q16 | `BAR1 GP_PUT` total | 188 | 188 | **188** | ★★ 0.7 — guard |

`BUILD_RC`, `CAPTURE_RC`, `STAMP_GATE`, and `grep -c 'No space left on device\|LLVM ERROR'`
**over the same invocations that produced the statuses** are recorded, not predicted.

⚠ **The arm-comparison guard, inherited from `w263` §3.2 and NOT re-graded to fit.** Exactly
these may move between `ring` and `pin`: Q1–Q6 and Q7. If a **seventh** number moves, the
comparison is reported **qualified**, in the RESULT, in those words. ⊘ `w263` already carries
one such qualification (`PushbufferAperture` 0→8 between its arms); it is inherited, not
discharged, and the `off`→`ring` column here is qualified for that reason before the boot runs.

---

## 3. ★★★ THE FOUR THAT MATTER, AND WHAT EACH OUTCOME MEANS

### 3.1 Q3/Q5 — ★★★ THE SINGLE POINT OF FAILURE IS **WHICH RESOLVER**

`pb=S:0x3d45f000` was produced by the **descent** (`RegPlane::resolve_va_from_root`, walking
the guest's real page tables through the framebuffer store). The pin asks a **different**
authority — `SharedDevice::resolve`, the core's forward-populated **address table** — because
that is the one `no_real_phys_only_gpga_or_gpa` and `mode2_address_table_of_truth` make
authoritative, and because reverse-resolving is forbidden.

★ **Two projections of one fact, and they can disagree** (`two_projections_of_one_fact_disagreeing`).
The table binds `bound=6275` addresses on `w263`'s first doorbell, so the prior is good — but
the *only* address anyone has ever confirmed it binds in this VAS is the ring's, and the ring's
binding is Vidmem.

| outcome | reading |
|---|---|
| `→ PINNED`, runs ≥ 1 | ★★★★★ **the rung**: the guest's pushbuffer pages are host-backed at the guest's own VAs |
| `→ UNRESOLVED … Miss` | ⊘ **the table does not bind the pushbuffer VA.** NOT a refutation of the mechanism — a statement that the *populate* side has a gap the descent does not. The next rung is then the populate pass, and it has an address list |
| `→ NOT IN GUEST RAM` | ⊘⊘ **the two resolvers DISAGREE about the aperture** of an address measured `S` by one of them. That is the most valuable outcome on this page and the one I would drop everything for |
| `→ REFUSED BY NAME` (layout) | the hypervisor states no run covering that GPA — a `layout`/`-m` fact, cheap to read |
| `→ REFUSED SystemDataPlane` | §12.26, an **owner** boundary, not a defect. ⊘ `w263`'s channels are `proc=2`, client `0xc1d0000c` — a **user** proc — so I predict this does **not** fire, unlike every prior pin boot |

### 3.2 Q4 — ⚠ `placed_as_asked` IS THE ONLY THING THAT MAKES A PIN MEAN ANYTHING

`Worker::execute`'s `#102` identity check unwinds a mapping RM placed anywhere other than
`at`. A pinned run whose `host_va != rva` would be **unwound**, so `PINNED` and
`placed_as_asked=false` cannot co-occur — the row is predicted `true` and is a **tripwire**: a
`false` beside a `PINNED` means the check regressed, not that the address is nearly right.

### 3.3 Q7 — ★★★ THE `Xid` PREDICTION IS THE ONE I AM LEAST SURE OF, AND I NAME BOTH READINGS

`FAULT_PDE ACCESS_TYPE_VIRT_READ` at `0x202400000` says the **host** VAS has no PDE there.
A successful pin installs a mapping at exactly that VA in the pdb the channel names. So:

| outcome | reading |
|---|---|
| `Xid` **0** on `pin`, 8 on `ring` | ★★★★★ the pin **is** what the engine was faulting for. The strongest possible result and I do not expect it |
| `Xid` **8** on `pin`, same addresses | ⊘ the pin landed in a VAS the host engine is not walking. *Which* VAS the host channel is bound to is `[NOT MEASURED]` and nothing in this rung measures it |
| `Xid` **> 8**, new addresses | ★★ the engine got **past** the first fetch and faulted on something the pushbuffer's own methods name — leg 4 partially discharged, and the new addresses are the next list |
| `Xid` **0** on both | ⊘ ambiguous, and it would mean the `ring` arm did not reproduce `w263`. Report as a **failure to reproduce**, never as a win |

⊘ **The `off` arm is not a control for `Xid`.** `w263` measured 0 there and 8 on `ring`; a
0 on `off` here re-measures an already-measured thing and attributes nothing.

### 3.4 Q13 — ★★★ WHY I PREDICT `CUP2_RC` DOES NOT MOVE, AGAIN, AND SAY SO PLAINLY

The brief asks me to be honest about whether this rung can move `CUP2_RC` at all. **It cannot,
and the reason is arithmetic, not pessimism.**

Leg 5 — the completion path — is unbuilt. `CE-SUBMIT → RETIRED` is **0 in every log ever
recorded**, ~127 of them. `cup2` returns 124 because the launcher times out waiting for a
completion that nothing in this tree produces. Supplying the engine with pushbuffer bytes it
can read moves the wall from *"the fetch faults"* to *"the fetch succeeds and nothing retires"*
— a real hop, and **not one `cup2`'s exit code can express**.

⇒ **Predicted movement: ZERO. Predicted size of movement: ZERO.** If it moves I was wrong in
the favourable direction and will say so in those words. ⚠ And per
`the_join_landed_and_the_wall_did_not_move`, a 124 on all three arms is **not** evidence the
rung did not land — that is precisely the inference `w260` proved wrong, and the rows that
adjudicate this rung are Q1–Q7, none of which is `CUP2_RC`.

---

## 4. ⊘ WHAT THIS BOOT CANNOT PROVE, WHATEVER IT SAYS

- ⊘ **That the pinned bytes are the bytes the engine reads.** The pin establishes a host RM
  object over the guest's pages at the guest's VA. Whether the host **channel** is bound to a
  VAS in which that VA resolves is not measured here and is not measurable from a `PINNED`
  line. `a_route_with_no_server_reads_as_a_routing_bug`.
- ⊘ **That reading `gp[i]` at pin time reads what hardware later fetches.** The guest may
  advance `GP_PUT` after this pass. This pins the extents present **at this doorbell**, and a
  later entry naming a new page is a **miss**, not a fault we would see here.
- ⊘ **Anything about completions** (Q14), by construction.
- ⊘ **That the fetch is leg B's doing rather than leg A2's** — the brief's ITEM 2. Still open,
  still uncontrolled, and §1 says why it is not in this rung.
- ⊘ **That the cap is adequate.** `w263` shows one non-zero entry per ring. `MAX_PUSHBUFFER_*`
  is sized off that single workload and Q6 predicts it is never reached — so a green Q6 is
  evidence the cap was **not exercised**, not evidence it is right.
- ⊘ **That a `w263` number reproduced here means `w263` was right.** Six of `w263`'s rows are
  compared across boots; this run repeats that only where an arm here differs in one variable.

---

## 5. ⊘ THE THIRD ARM (the brief's ITEM 2) — COSTED, NOT DONE

`adopted_guest_userd` is called from exactly one place: inside `adopted_guest_ring`'s
constructed `Some`, in `crates/kayfabe-fwd/src/lib.rs:3709`. Its own doc states the design:
*"There is no flag here and there must not be one… With the supply side disarmed this function
is `None` by construction."* That property is what makes leg B's arming un-driftable, and it
is also exactly what makes *"A2 armed, B off"* unspellable.

Forcing it off requires a boolean on `plan_engine_object`'s **public** signature
(`kayfabe-fwd:3571`), threaded from `kayfabe-rt/src/device.rs:3343` and `:3382`, sourced from a
fourth environment arm at the composition root. That is a real change to a public API, in the
crate the opacity and orphan gates guard, for one comparison.

⇒ **Named, costed, and left to its own rung with its own pre-registration.** ⚠ It should not be
folded into a boot whose subject is a different plane; `a_correct_capture_can_answer_the_wrong
_question`, and an arm added in passing is how the `w263` control stopped being `w262`'s.
