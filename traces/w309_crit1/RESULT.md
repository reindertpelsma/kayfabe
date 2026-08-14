# w309 RESULT — the isolating arm BUILDS, and the thing it was to be read against is BROKEN AT HEAD

`[measured 2026-08-14, vh2, real GA106, driver 580.159.04]`
Bench tree `/workspace/kayfabe_w309`, clean, stamp gate passed on **every** boot below.

> ## ★★★★★ THE HEADLINE, AND IT IS NOT THE ONE THE BRIEF ASKED FOR
>
> **R33 arm 1 — the in-boot known-positive that makes the whole criterion-1 contrast readable —
> is BROKEN AT MASTER `74200b2b`, and the first bad commit is `d2c58075` (merge w304, "FIVE
> INERT RELAXATIONS CONFIRMED AND DELETED").** It is bisected, bracketed by repeats on the
> passing side, and it is **not mine**: it reproduces at `74200b2b` with none of my commits
> applied.
>
> ⇒ Every arm of the matrix I was sent to run would have printed `CONTROL-NEVER-LANDED` at HEAD,
> and **I would have had no way to tell "this confound is the blocker" from "nothing works on
> this rev at all"** — because the arm that discriminates those two is exactly the one that
> regressed. ⊘ **The matrix is built and it did not run**, deliberately: an arm read against a
> dead known-positive is not a measurement.

---

## 1. The pre-registered letter

| letter | verdict |
|---|---|
| **(A)** one confound isolated | ⊘ **NOT REACHED** — see §4; blocked by the regression, not by the arm |
| **(B)** the contrast does not reproduce | ⊘ **REFUTED.** It reproduces at `c7c058a3`, **2/2** |
| **(C)** criterion 1 MET | ⊘ not in the guest. ★ **MET NATIVELY on the isolating arm** — §3 |
| **(D)** the isolating arm will not build | ⊘ **AVERTED** — it builds, runs, and is a native known-positive |
| **(E)** VA identity broken | ⊘ **NOT OBSERVED.** Native: guest-side plane D and host `Xid` name the *same* address. No guest boot produced two addresses to compare, so this is **unmeasured in the guest**, not "holds" |

★ **A sixth outcome the brief did not enumerate, and it is the result:** the *instrument's own
known-positive* regressed under us. Naming it is worth more than any arm I could have run.

---

## 2. ⊘⊘⊘ w305'S "ONE-LINE FIX" IS WRONG — AND THE REAL CAUSE IS A DESTRUCTIVE READ IN A DIAGNOSTIC

The brief said to verify this rather than inherit it. **It does not survive.**

w305 ruled: *"`alloc_channel_in` wants the RANGE and arm 1's `vas` handle is the SPACE
(`0xcafe0005`), whose paired range is `0xcafe0009`. The fix is one line — pass the range."*

**Refuted from source, three ways:**

1. `alloc_vaspace` **returns the range** (`rm.rs:4082-4127`) — it allocates `FERMI_VASPACE_A`,
   then an `NV01_MEMORY_VIRTUAL` over it, `pair()`s them, and hands back **the range**. The
   handle was never the space.
2. `ce_control_placement` prints `guest_space = narrow(vas)` (`rm.rs:6832-6840`) — so the
   `0xcafe0005` in the log **is** that same range handle, printed from `vas` itself.
3. `0xcafe0009` is **not** its pair. It is the **executor VAS** that `map_dma_both` mints
   lazily on first publish (`rm.rs:4149-4162`) — the w229 split, a third object. The log's own
   words: *"two different spaces — the w229 executor split"*.

### ★★★ The actual cause — and it had been in every committed R33 log for weeks

`pde_info` resolved its `hVASpace` with **`companion_of`** (`rm.rs:5729`), the accessor that
**REMOVES** the pairing and whose one legitimate caller is `free`. Its peeking twin `space_of`
exists three lines away and its doc-comment already names this exact hazard:

> *"Reading it with the removing accessor would make allocating a channel silently un-free the
> address space."*

**Arm 6 runs before arm 4 and ate the pairing.** Two symptoms, one cause, never joined:

| symptom | how it read | what it was |
|---|---|---|
| arm 6's **first** `pde_info` answered, every later one printed `Other(19270)` | *"⊘ NOT ASKED — the control refused or the reply did not decode"* | the control never refused; **this function had eaten its own handle** |
| `--ce-client-fault-shared-vas` died `BadHandle(0xcafe0005)` | *"pass the range, not the space"* | `alloc_channel_in`'s `space_of(range) → None` |

`19270` = `0x4B46` = `NOT_ON_THIS_RUNG` (`rm.rs:158`) — `pde_info`'s own refusal constant,
printed as if hardware had spoken.

**Fix: `companion_of` → `space_of`. One line — but not the line w305 named, and in a different
function.**

⊘ **AND FIXING IT REVEALED A SECOND, INDEPENDENT DEFECT UNDERNEATH** — reported, not smoothed.
Native, after the fix, every row now *answers*, and arm 6's calibration **still fails, with the
opposite polarity**:

```
R33 arm 6 CAL+ ring  0x0000000120020000 = PDE PRESENT (pageSize 0x20000000, ptePhysAddr 0x3112000, pdbAddr 0x3110000)
R33 arm 6 CAL- free  0x0000000900000000 = PDE PRESENT (pageSize 0x20000000, ptePhysAddr 0x3112000, pdbAddr 0x3110000)
FAIL R33 arm 6 CALIBRATION = ring -> Some(true) (want Some(true)), free -> Some(true) (want Some(false))
```

**Every address returns the byte-identical block** — a mapped VA and one 30 GB away. ⇒ the reply
is not address-dependent, so `pde_info` is **not an oracle for any address** and never was.
⚠ **Two defects stacked, the first hiding the second**: while the destructive read was in place
the rows printed `NOT ASKED`, so the constant answer could not be seen. Had I stopped at *"the
rows answer now"* I would have shipped a working-looking oracle that says `PRESENT` for
everything. ⊘ **Out of scope for w309 and NOT fixed here** — filed, with the evidence above.

---

## 3. ★★★★★ THE ISOLATING ARM IS BUILT, AND IT IS A NATIVE KNOWN-POSITIVE

`[measured, vh2, NO QEMU, rev 891cc7b9]` `--ce-client-fault-shared-vas`, which in w305 could not
be constructed at all:

```
info  RMLADDER ARGV     = [--ce-client-fault-shared-vas]
info  R33 arm 4 CONFIG  = vas=SHARED (arm 1's, already carried retired work) addr=DICTATED
                          notifier=PRESENT chan-ordinal=2 (HELD, not settable) engine=COPY0
                          fault-va=0x0000000900000000
★     R33 CRIT1 STATE   = FAULT-PROVOKED-ADDRESS-READ | VA-IDENTITY MEASURED = yes
★     R33 arm 5 NOTIFIER= PLANE A FIRED — status 0xffff, info32 0x0000001f, info16 engine 0x0001
★     R33 arm 5 WHERE   = GET_MMU_FAULT_INFO addr=0x0000000900000000 faultType=0x0
                          faultString="FAULT_PDE" | VA-IDENTITY HOLDS
host: Xid 31 … name=kayfabe-rm-ladd, channel 0x00000005 … ENGINE CE0 HUBCLIENT_CE1
      faulted @ 0x9_00000000. Fault is of type FAULT_PDE ACCESS_TYPE_VIRT_READ
```

⇒ **Outcome (D) is averted**: the arm builds, runs, provokes the fault, reads the address, and
two independent observers name the same one. **Both arms of the VAS contrast are now native
known-positives**, so a guest divergence on either is attributable to our device and nothing else.

⚠ **Scoped:** this is the criterion-1 machinery working **natively**. It is not criterion 1,
which is about the **guest** observing the fault.

---

## 4. ★★★★★ THE REGRESSION — BISECTED, WITH REPEATS ON BOTH SIDES

Everything held constant — same clone, same `CARGO_TARGET_DIR`, same box, same harness, same
boot arm (`drain`), boots minutes apart. **The only variable is the source revision**, and the
QEMU stamp gate passed on every one (`STAMP == HEAD`).

| revision | what it is | arm 1 (the known-positive) |
|---|---|---|
| `c7c058a3` | w305's exact rev | ★ **PASS** — 4096 bytes moved, whole four-fact bar |
| `c7c058a3` | **repeat** | ★ **PASS** — 2/2, so the pass is confirmed, not one boot |
| `8d258daa` | merge w305 | ★ **PASS** |
| **`d2c58075`** | **merge w304 — "FIVE INERT RELAXATIONS … DELETED (-2850 lines)"** | ⊘ **FAIL** ← **first bad** |
| `74200b2b` | **master HEAD** | ⊘ **FAIL** |
| `c73e015e` | mine (master + w309) | ⊘ **FAIL** |
| `c73e015e` | **repeat** | ⊘ **FAIL** — byte-identical numbers |

**The failure, verbatim, identical on every failing rev:**

```
FAIL  R33 arm 1 COPY = dst[0] 0x3f0011cc -> 0x3f0011cc (want 0xc0ffee33),
      dst[last] 0x3f0011cc (want 0xc0fff232), semaphore 0x00000000 (want 0x00000001),
      GP_GET 1 GP_PUT 1 — the entry WAS fetched and the methods did nothing:
      SET_OBJECT class, subchannel, or an operand that does not resolve
```

★★★ **AND NOTE THE SHAPE: THAT IS ARM 4'S FAILURE, ON ARM 1.** Cursors caught up, semaphore
never written, nothing moved. w305's contrast was *arm 1 works / arm 4 does not*; at HEAD
**both fail with one fingerprint**. ⇒ The contrast is real — it just does not exist at HEAD any
more, because the working side of it died.

### ⊘ Why this was invisible to w304's own gate

w304 confirmed all five relaxations inert on **`^CUP3_VAL=43`**, `n=2`, ladder 8/8, `Xid=0` —
a real and careful gate. **But `43` is cup3: libcuda, a GR compute launch.** R33 arm 1 is a
**raw CE client** — no libcuda, its own VAS, its own operands, a copy engine. ⇒ *inert for cup3*
was read as *inert*, and the two workloads do not exercise the same publication paths.

★ This is the tree's own banked lesson firing again — **"a refusal scoped by a workload"** — and
the memory index's **"a census ZERO needs a KNOWN-POSITIVE"**: five ablations each showed *no
change in 43*, and none of them asked whether anything **other than cup3** still worked.

⚠ **And the deletion is the reason this is expensive rather than a one-line revert.** w304
deleted the code — *"DELETED, not defaulted: `pin_pushbuffer_guest_ram`,
`pin_completion_guest_ram`, `ce_release_pages`, `pin_operand_guest_ram`, `sweep_cpu_pt_tables`
…"* — so the relaxation cannot be re-armed from the environment at HEAD to test it. That is why
the ablation sweep below runs at `8d258daa`, the last rev where all five are still env-gated.

---

## 5. THE ARM MATRIX — BUILT, PRE-REGISTERED, AND NOT RUN

| arm | vas | ring + operand VAs | notifier | ordinal | what it discriminates |
|---|---|---|---|---|---|
| `fresh` | FRESH | DICTATED | PRESENT | 2 | ⊘ **nothing — it is the CONTROL.** Does the contrast reproduce? |
| `shared` | **SHARED** | DICTATED | PRESENT | 2 | ★ **VAS freshness, alone** |
| `rmplaced` | FRESH | **RM-PLACED** | PRESENT | 2 | ★ **address dictation, alone** |
| `nonotif` | FRESH | DICTATED | **ABSENT** | 2 | ★ **notifier presence, alone** |

⊘⊘ **THE FOURTH CONFOUND — CHANNEL ORDINAL — IS HELD AT 2 AND IS NOT SETTABLE**, said plainly
rather than left out: arms 2/3/6 read `ce_control_placement`, which does not exist until arm 1
has built a channel, so making the probe the process's **first** channel is a different program,
not a flag. **Three of the four move; the fourth does not**, and no arm may be read as testing it.

★★★ **What one arm could have settled.** If `shared` lands, VAS-freshness is the discriminator
**and** dictation, the notifier and the ordinal are simultaneously ruled out as sufficient
blockers, because all three were held at arm 4's values while it landed. One arm, four answers.

⊘ **Only `fresh` ran** (×2, at HEAD). It printed `CONTROL-NEVER-LANDED` — but so did arm 1, so
the boot grades nothing. The harness caught this itself, by design:

```
arm 1 met the whole four-fact bar = [NO ⊘ THE IN-BOOT KNOWN-POSITIVE DID NOT FIRE]
```

---

## 6. THREE INSTRUMENT DEFECTS THIS RUNG FOUND, TWO OF THEM ITS OWN

1. ★★★ **A DESTRUCTIVE READ INSIDE A DIAGNOSTIC, REPORTED AS THE HARDWARE REFUSING** — §2.
   Same class as `get_mmu_fault_info`, except that one is destructive *in RM* and says so, while
   this was destructive in **our own bookkeeping** and printed the damage as an RM refusal.
   ⇒ **`Other(19270)` was our own constant.** An error path that cannot distinguish *"they said
   no"* from *"I broke it"* will always be read as the first.

2. ⊘ **A HARNESS SELF-CHECK THAT WENT LOOKING FOR A STRING NOTHING EMITS.** My grading block
   printed `w309 grading lines = [0] (MUST be >= 1)` while the banner it counts was six screens
   above it — I renamed the banner and not the `grep` literal. ★ **The mirror of w305's anchor
   trap**: that one produced a false *"UNMEASURED"* on a measured field; this produced a false
   *failure* of a check that had passed. ⚠ Both fail toward the reading this tree treats as
   safe, which is exactly why they get believed. Fixed by making the banner **one shared
   variable**, so a rename cannot separate the emitter from the check.

3. ⊘ **AN ARM RECOVERED FROM THE HARNESS'S OWN BELIEF.** w305's *"the ARM ACTUALLY PASSED to the
   client"* field grepped the probe log and printed `[]` on both boots. Now **two independent
   lines that must agree**: `RMLADDER ARGV` (the client echoes its own argv before parsing a
   flag) and `R33 arm 4 CONFIG` (built from the values handed to the probe). If they disagree
   with each other or with the harness's intent, the boot measured a different experiment than
   the one it is filed under.

---

## 7. WHAT THE NEXT RUNG SHOULD DO, IN ORDER

1. ★★★★★ **Land the arm-1 regression.** It is bisected to `d2c58075`. §4's ablation sweep names
   which of the five deleted relaxations R33 arm 1 depends on; the fix is to restore that one,
   **with the raw CE client as its gate** rather than `43`.
2. ★★ **Add R33 arm 1 to whatever gate declares a relaxation inert.** w304's gate was sound and
   still missed this, because it had exactly one workload. Two workloads that stress different
   planes is the cheapest possible fix.
3. ★ **Then run the matrix** — `shared` first; it is built, pre-registered, and a native
   known-positive on both arms.
4. ⊘ **`pde_info` answers the same block for every address** (§2). It is cited as an oracle in
   arm 6 and in `road_to_v1_after_cup2.md`'s neighbourhood. Until it varies with its argument,
   nothing may be concluded from it.
