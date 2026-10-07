# w280 PRE-REGISTRATION — `cup2` under the join-aware residency fix (H11)

**STATUS: LIVE — 2026-08-12.** Written and committed **before** the boots. Branch off `116cc9b`
(`w279` RESULT). The rung: convert `w279`'s **H11** from hypothesis to measurement — run `cup2`
on the fixed tree with **route B armed**, and read whether its ring relabels, whether the
refusal moves, and `CUP2_RC`.

---

## ⊘⊘⊘ LEAD — **THE RUNG'S HEADLINE WAS ALREADY MEASURED, ON 2026-08-11, AND IT IS COMMITTED**

`[measured, traces/boots/w246/README.md + traces/guest_boots/run_w246d_acbb9a3_witon_rbon_qemu.log,
both opened this session]`

`w246` **corner D** is a `cup2` boot at revision `acbb9a3` with `KAYFABE_PT_WITNESS_EXEC=on`
**and `KAYFABE_RING_VIDMEM=1`** — the exact arming this rung was briefed to run for the first
time. Its own log:

```text
kayfabe: RING-VIDMEM KAYFABE_RING_VIDMEM=1 ⇒ route B ON
FWD-RING proc=2 chan=12 key=0xc1d0000c:0x5c00004b pdb=0x201000
         RING bytes=65536 cursor=0 live=1 spans=0 → NOTHING FORWARDED
fbRING[p0]@0x1024000=0000c00202220000… nz4/4096  resY
CUP2_RC=124
```

⇒ Four things, all already on disk and none of them mine:

1. **`cup2`'s ring at `fb 0x1024000` HAS been read out of our emulated framebuffer**, on the
   walling channel — `key=0xc1d0000c:0x5c00004b`, the same key `w277` refuses on — with the
   same GPFIFO entry (`gp[0]@0x200224000=0x202c00000+0x20`, `nonzero=[0]=0x0000220202c00000`).
2. **It was labelled `resY`, not `resN-NEVER-WRITTEN`.** `RingFbNeverWritten` = 0 that boot, and
   **so was `PushbufferAperture`** — because `cup2`'s pushbuffer is **sysmem** (see §2).
3. **`CUP2_RC=124` anyway.** Eight `FWD-RING` lines, `spans=0`, `CE-SUBMIT=0`.
4. ⇒ **The `resN-NEVER-WRITTEN` label on `cup2`'s ring is a REGRESSION, introduced AFTER
   `w246d`** — by the FB-leaf join (`3159bfb`, forward-port of `fb-join`). `w246d`'s log
   contains **zero** `GR-RING-JOIN` lines and **zero** `JOINED-one-memory`; `w277`'s contains
   97 and the label flipped on the same page with the same bytes.

⇒ **`w279`'s fix does not open new ground on `cup2`; it RESTORES ground the join took away.**
And the outcome it restores is already known to be `CUP2_RC=124`. ⚠ So the honest headline of
this rung is pre-committed, before the boot: **the fix is necessary-not-sufficient for `cup2`,
and that was measurable from `w246d` without booting anything.** The brief's own low arm.

### ⊘ What w280 therefore still measures — and it is NOT nothing

`w246d` read the ring out of **local resident `SparseFb` pages** (`resY`). The current tree
reads it **through a join**, out of a **host sysmem leaf**, and `w277`'s own line says the
establishment copy for that leaf was **vacuous**:

```text
GR-RING-JOIN RING(chan=0 entries=1024 engine=GrCompute) leaf va=0x200200000 len=0x200000
             fb_phys=0x1000000 → JOINED (shared) … established=0 bytes over 0 page(s)
  ⊘ the establishment copy was VACUOUS for this leaf: no page of it was resident
```

⇒ **The bytes `fetch_ring_bytes` will serve are a different memory from the ones `w246d`
served.** Whether they are the *same bytes* is unmeasured, and it is this rung's only genuinely
open device question. The falsifier is exact and already in hand: `gp[0]@0x200224000` must read
`0x202c00000+0x20` and `nonzero=[0]` must read `0x0000220202c00000`. **A join that serves stale
or zero memory would produce `live=0` or a different entry, and that would be a NEW defect —
one the `nz4` dump row cannot see, because `fb_peek` and `fetch_ring_bytes` read through the
same join and would agree while both being wrong about the guest.**

---

## ⊘⊘ 2. THE BRIEF'S "EXPECT THE PUSHBUFFER TO BECOME THE NEXT BLOCKER" IS PROBABLY FALSE FOR `cup2`

`[measured, traces/boots/w277/run_w277_off_qemu.log.gz, the `RING-PROJ` descent for the walling
channel, opened this session]`

```text
ring=0x200224000 rng=V:0x1024000  gp[0]@0x200224000=0x202c00000+0x20
pb=S:0x25d78000  pbm[8w of 32B]: [0]sub4/m0x0/…=[0xc7b5] [1]sub4/m0x240/…=[0x2,0x440ff30,0x1]
                                 [2]sub4/m0x300/…=[0x14]
```

`pb=**S**:` — `cup2`'s pushbuffer is in **guest RAM**, not vidmem. The raw client's is
`pb=**V**:0x40000`. ⇒ `read_pushbuffer`'s hard-coded `VidmemRoute::Refuse`
(`crates/kayfabe-fwd/src/lib.rs:4752`) can only fire on a **vidmem** pushbuffer, so it is
**probably not on `cup2`'s path at all**.

### ⇒ THE DECISION, STATED BEFORE THE BOOT: I AM NOT LIFTING IT, AND HERE IS WHY

The brief required this be decided deliberately and never silently. **`lib.rs:4752` stays
`Refuse`, unchanged, on both arms.** Reasons, in order:

- It is measured to be **off `cup2`'s path** (`pb=S:`), so lifting it would change nothing for
  the arm this rung exists to measure — while making the device non-identical to `w279`'s and
  destroying the one-variable property of the client control.
- The variable this rung is entitled to vary is the **workload** (`cup2` vs the raw client).
  The device stays byte-identical to `w279_guest`'s.
- It is reported **by name** in the grade, per the brief, and the boot itself will say whether
  it was ever reached (`PushbufferAperture` with a VA equal to a `pb=V:` address).

⚠ If `cup2`'s pushbuffer turns out to be vidmem on this boot (it was sysmem on all of `w246d`
and `w277`), the refusal will appear and this decision is what sequenced it — that is arm
**H6**, pre-registered, not a surprise.

---

## THE ARMING — one variable, and it is the WORKLOAD

Device source unchanged from `116cc9b`. Both arms carry `w271_pin`'s six arms byte for byte
plus route B:

| arm | order | workload | `RING_VIDMEM` | hook | why |
|---|---|---|---|---|---|
| `w280_cup2` | **first** | `cup2` (`libcuda`, `cuCtxCreate`) | **on** | `cup2_hook_gdbspin.sh` | the rung |
| `w280_client` | second | `kayfabe-rm-ladder --ce-client`, 53 ioctls | **on** | `r33_hook_ce_client.sh` | the control, `w279`'s arm re-run at this revision |

★ The armed arm runs **first**, deliberately (`w277`'s rule): if the session is cut short, the
rung's own measurement exists and the recoverable half is what is missing.

⊘ **Route B is UNREACHABLE with `KAYFABE_PT_WITNESS_EXEC` off** (`plan_gpfifo_ring` returns
`RingVaUnbound` before `VidmemRoute` is computed). Both arms carry `witness=on`. Never measure
this flag with the witness disarmed.

---

## ARMS — graded from artefacts, by IDENTITY, low arms widened

⚠ **Every arm below is graded by an ADDRESS or a KEY, never by a count.** `w278`→`w279` is the
case in point: same fault variant, different VA, opposite meaning.

| # | prediction | how it is read | weight |
|---|---|---|---|
| **H11a** ★★★ *(the rung's own question)* | `cup2`'s ring page **relabels**: `fbRING[p0]@0x1024000` prints **`JOINED-one-memory`**, `resN-NEVER-WRITTEN` gone **from that address** | the `RING-PROJ` row for `@0x1024000` specifically | most likely |
| **H11b** ★★★★ *(the honest headline)* | **`CUP2_RC=124` regardless** ⇒ the fix is **necessary-not-sufficient**; `w246d` already implies it | anchored `^CUP2_RC=` out of the probe log | most likely |
| **H12** ★★★ | `FWD-RING` fires for `key=0xc1d0000c:0x5c00004b` with `bytes=65536 live=1 spans=0` — `w246d`'s line, restored **through the join** | the `FWD-RING` line, graded on the **key** and on `live=`/`spans=` | likely |
| **H13** ★★★★★ ⚠ *(the one genuinely open device question)* | ⊘ the join serves **different bytes**: `live=0`, or `gp[0]` ≠ `0x202c00000+0x20`, or `nonzero=[0]` ≠ `0x0000220202c00000` | the descent's own fields, against `w246d`/`w277` verbatim | **a NEW defect if it fires** |
| **H14** ⚠ *(the regression arm — route B is newly reachable for OTHER channels)* | `RingFbNeverWritten` fires at a phys **≠ `0x1024000`** — e.g. the kernel CE channels whose joins were `REFUSED BY NAME SystemDataPlane` and whose pages are genuinely unwritten | every `RingFbNeverWritten` graded by `phys`/`va` | ⇒ **a non-zero count is NOT a refutation of H11a**; only a hit at `0x1024000` is |
| **H15** | the refusal **moves off the ring**: no `PushbufferAperture` at `va 0x200224000` | distinct `PushbufferAperture { va: GpuVa(N)` values | likely |
| **H6** | the pushbuffer becomes the wall for `cup2` (`pb` is vidmem after all) | a `PushbufferAperture` at a `pb=V:` address | ⊘ contradicted in advance by `pb=S:` |
| **H16** | `GP_GET` **moves** on the walling channel | `fbuserd@0x1026088 GET=… PUT=…` rows | ⊘ **already 1/1 in `w277` with route B OFF** — this is not a discriminator and must not be reported as one |
| **H17** ⊘ *(the milestone; not predicted)* | `CE-SUBMIT > 0` or `CUP2_RC=0` | anchored | ⊘ `w246`: `CE-SUBMIT` 0 in all four corners |
| **H18** ⊘ *(lowest, and it must be named)* | `cup2`'s ring does **NOT** relabel ⇒ its page is outside the join and **H11 is REFUTED** | the row at `@0x1024000` | ⇒ the two walls are different cases |
| **H19** | the **client** arm diverges from `w279_guest` at this revision | `PushbufferAperture` VA, `R33_RC`, `total=53` | a divergence is itself the finding |
| **H20** ⚠ | **a VOID boot** — no `cup2`, no client, empty artefact reading as favourable | the known-positives below | the trap |
| **H21** | boot fails / hangs / `ENOSPC` | not a data point | |

### ⊘ THE VOID GUARDS — asserted before any zero is read

`w279`'s first attempt was a void boot that printed **this rung's predicted success**. Every
zero in this run is gated on a known-positive **on its own grep**:

- `fbRING rows in this log = [N]` — **0 ⇒ VOID**, not "no join".
- `RING-PROJ lines = [N]` — 0 ⇒ no doorbell descent happened at all.
- `cup2` ran: `CUP2_OUT_BYTES=`, `totalMem=`, and `GCC_CUP2_RC` all present in the probe log.
- the client ran: `GUEST_MD5` equal to the host-side md5, `GUEST_EXECUTABLE=yes`, `total=53`.
- the runner asserts the **artefact** (`[ -s "$CLIENT" ]`, non-empty md5), never `cargo`'s exit
  status.
- `^CUP2_RC=` **anchored** — `GCC_CUP2_RC=0` matches the unanchored form and has reported the
  campaign's headline success value on a hanging arm.

## ⊘ WHAT THESE BOOTS CANNOT DECIDE, stated before they run

- **They cannot make `cup2` work, and are not built to.** `w246d` already measured the same
  arming ending at `CUP2_RC=124`.
- **They cannot say the two walls are the same defect.** They can say only whether the *same
  false label* was present and is now gone; `w246d` already shows removing it does not move
  `cup2`'s wall, so *"same defect"* is close to refuted **before** the boot.
- **They cannot see the completion plane** — it still has no oracle.
- ⚠ The `BAR1 GP_PUT` witness has **measured false positives** (client data at page offset
  `0x8c`). Labelled, never filtered.
- One workload each, one chip (GA106), one driver (`580.159.04`), one boot per arm.
