# w263 — LEG B FIRED, AND THE ENGINE FETCHED THE GUEST'S RING

> ### STATUS — 2026-08-12 / **LIVE — MEASURED.** Two arms (`off`, `ring`) from **one** binary,
> source revision **`15512d3`**, **stamp-gated against the binary before booting**
> (`STAMP: [kayfabe-rev:15512d32…] WANT: [kayfabe-rev:15512d32…]` → `PASS`).
> Bench `vh`, `NVIDIA GeForce RTX 3060` (GA106), host driver **580.159.04**.
> Graded against `docs/design/leg_b_userd_at_creation_prereg.md`, committed at `04a5744`,
> **before** the build.
> `BUILD_RC=0`, `CAPTURE_RC=0` both arms, `ENOSPC_LLVM=0` from the same invocations.

---

## 0. ★★★★★ THE HEADLINE — and it is not the one the rung was aimed at

Leg B fired: **`userd=GUEST-USERD` 16 on `ring`, 0 on `off`.** That was the target and it is
the smaller half of what the boot says.

**The host engine fetched the guest's GPFIFO entry and walked to the pushbuffer address that
entry named.** Two independent witnesses, from opposite sides:

```
run_w263_ring_hostdmesg.log — 8 lines, 8 Xid, ALL of the form:
  NVRM: Xid (PCI:0000:00:07): 31, … channel 0x01000011, intr 00000000.
  MMU Fault: ENGINE CE2_PBDMA0 HUBCLIENT_ESC faulted @ 0x2_02400000.
  Fault is of type FAULT_PDE ACCESS_TYPE_VIRT_READ
```

| the 8 fault addresses | the 8 pushbuffer VAs the GUEST'S OWN `gp[0]` entries name (`run_w263_ring_qemu.log`) |
|---|---|
| `0x2_02400000` `0x2_02600000` `0x2_02800000` `0x2_02a00000` `0x2_02c00000` `0x2_02e00000` `0x2_03000000` `0x2_03200000` | `gp[0]@0x200218000=0x202400000+0x20` … `gp[0]@0x20022d000=0x203200000+0x20` |

**Byte-exact, all eight, in order.** A PBDMA cannot fault at an address it did not read out of a
GPFIFO entry. ⇒ The host channel was told the guest's ring, the guest advanced the guest's
cursor in the page RM reads, and **hardware consumed it**.

And the second witness, from the cursor side — the first `GP_GET` read in this campaign:

```
fbuserd@0x1026088 GET=1 PUT=1 …      ← ring arm
fbuserd@0x1026088 GET=0 PUT=1 …      ← off arm, same address, same boot pair
```

⇒ `admitted_and_served_are_different_gates` — **this run crosses that gate.** Every previous
rung could say only that RM had been *told*.

⚠ **The Xid is contained** (`gpu_fault_is_contained`): both arms completed, `CAPTURE_RC=0`,
`RmInitAdapter failed` 0, guest `NVRM` 31 on both.

---

## 1. THE PRE-REGISTERED SCORECARD

| # | observable | pred `off` | **meas `off`** | pred `ring` | **meas `ring`** | |
|---|---|---|---|---|---|---|
| P1 | `phys=fb:` on `RING-PROJ` | ≥ 1 | **8** | ≥ 1 | **9** | ✔ ★★★ §2.1 |
| P2 | `adopt=GUEST-RING` | 0 | **0** | 16 | **16** | ✔ |
| P3 | `adopt=DECLINED` | 24 | **24** | 8 | **8** | ✔ |
| P4 | `userd=GUEST-USERD` | **0** | **0** | **≥ 1** | **16** | ★★★★★ ✔ **the rung** |
| P5 | `guest_userd=` tail | 0 | **0** | = P4 | **16** | ✔ |
| P6 | `REFUSED USERD_NOT_A_JOINED_WINDOW` | 0 | **0** | 0 | **0** | ✔ |
| P7 | `REFUSED RING_NOT_A_JOINED_WINDOW` | 0 | **0** | 0 | **0** | ✔ |
| P8 | `RmInitAdapter failed` | 0 | **0** | 0 | **0** | ✔ |
| P9 | host `Xid` | 0 | **0** | **0** | **8** | ⊘⊘ **REFUTED** — §2.2, and it is the best row |
| P10 | guest `NVRM` lines | 31 | **31** | 31 | **31** | ✔ |
| P11 | `fbuserd@` present | ≥ 1 | **8** | ≥ 1 | **9** | ✔ |
| P12 | `fbuserd GET=` non-zero | **0** | **0** | **0** | **1** | ⊘⊘ **REFUTED** — §2.3, favourable |
| P13 | `CUP2_RC` | 124 | **124** | **124** | **124** | ✔ movement predicted **0**, movement **0** |
| P14 | `BAR1 GP_PUT` total | ~188 | **188** | ~188 | **188** | ✔ identical |
| P15 | `doorbells … 0 REFUSED` | true | **FALSE (8)** | true | **FALSE (16)** | ⊘ **REFUTED on BOTH arms** — §3.1 |

`ENGINE-OBJECT` census `seen=34 forwarded=32 refused=2` on **both**. `GR-BIRTH` total **24** on
both. `CE-SUBMIT` **0** on both.

---

## 2. THE THREE ROWS THAT CARRY THE RUN

### 2.1 ★★★★★ P1 — the address really is on the wire, and it is EXACT

```
userd=h0x5c000014/off0x1a000/phys=fb:0x101a000/0x200
userd=h0x5c000014/off0x1d000/phys=fb:0x101d000/0x200
…                     off0x2f000/phys=fb:0x102f000/0x200
```

The rung's single point of failure — *does the guest's channel-alloc params blob reach +188?* —
is answered **yes**, at 0.75 confidence, on the first try. And three things fall out that were
predictions rather than measurements before this boot:

- **`size = 0x200` — 512 bytes exactly**, on every row. The descriptor really is the
  *sub*-memdesc of one USERD slot, which is what `ChannelUserdMemWire`'s doc argued from
  `memdescCreateSubMem(…, userdOffset, userdSize)` and had never seen.
- **`phys = 0x1000000 + userdOffset[0]`**, exactly, on all eight rows. The guest's USERD object
  is based at the joined leaf's own framebuffer base.
- ⇒ ★ **the derived offset and the guest's declared `userdOffset[0]` are EQUAL on this bench**,
  which is precisely the coincidence `the_birth_names_the_guests_ring.rs` sets `0x9000` to break.
  A fixture that had reproduced it would have passed with the guest's offset forwarded into an
  object it is not an offset into. `[measured]` here, in the favourable direction, by luck.

### 2.2 ⊘⊘ P9 REFUTED — 8 host `Xid`, and they are NOT leg B's addressing

The pre-registration named this exact row as the rung's harm case: *"if the offset arithmetic is
wrong by a page, RM programs a runlist entry naming a physical address inside the joined leaf
that is not a USERD… an `Xid` on the `ring` arm and not on `off` **names the address**."*

★★★ **It named the address, and the address is not USERD's.** `ACCESS_TYPE_VIRT_READ`,
`FAULT_PDE`, at eight **virtual** addresses that are byte-exact the pushbuffer VAs the guest's
own GPFIFO entries carry (§0). A USERD misplacement would fault on a *physical* USERD access at
a runlist address, on a different engine, at a different instant.

⇒ **This is §2.2 item 4 of the pre-registration, arriving as hardware rather than as an
argument**: *"the guest's pushbuffer pages are reachable by the host engine at the addresses the
GPFIFO entries name. The ring's leaf is joined; the pages its entries point at are a different
set of leaves and nothing in this rung joins them."* Written before the boot; measured after.

★ **It is the wall moving one hop, and the next hop has an address list.** Eight VAs, `0x200000`
apart, in `Vidmem`, on `vas=0xcafe0005`.

### 2.3 ⊘⊘ P12 REFUTED — `GET` MOVED, on one channel

Predicted `GET = 0` everywhere; measured `GET=1 PUT=1` at `0x1026088` on the armed arm and
`GET=0 PUT=1` at the same address on the control. One channel of nine sampled.

⚠ **One is a small count and this is not a small event** (`a_small_count_is_not_a_small_event`):
`GET` is written by **hardware** and by nothing in this port. There is no path by which our code
could put a 1 there.

⊘ **What it does not say.** Nine `fbuserd` samples is not a census — the token rides the
`RING-PROJ` line, which is emitted on a probe path, not once per channel per instant. *"One
channel fetched"* and *"one channel was sampled at the right moment"* are not separated by this
run. The eight Xid are the stronger statement, because they are eight.

---

## 3. ⊘ WHERE THE RUN IS WEAKER THAN IT LOOKS

### 3.1 ⊘⊘ P15 REFUTED ON BOTH ARMS — and my control is NOT `w262`'s control

`w262` measured *"doorbells: 191 arrived, 191 served, 0 REFUSED"* on both arms. This run:

| | `off` | `ring` |
|---|---|---|
| `Route::NotACopyEngineChannel` | 8 | 8 |
| `FwdFault::PushbufferAperture` | 0 | **8** |

**Two separate findings, and I am reporting both rather than the tidier one.**

1. ⚠ **The `off` arm already differs from `w262`'s `off`** (8 refusals vs 0), and the cause is
   my harness: `w263_run.sh` exports `KAYFABE_FB_JOIN=shared` **unconditionally**, on both arms.
   `w262`'s control did not have the supply side armed. ⇒ **My control is the right control for
   isolating legs A2+B and the wrong one for comparing to `w262` row by row.** Said plainly
   because six rows above are compared to `w262`'s numbers and this one is why that comparison
   is not free.
2. ★ **A fourth number moved between MY arms** (`PushbufferAperture` 0 → 8), so by §3.2's own
   rule — *"if a fourth moves, the comparison is reported as void rather than re-graded"* — the
   arm comparison is **qualified, not clean**. ⊘ The movement is caused by the rung and is
   coherent with §2.2 (eight CE doorbells whose pushbuffer is in the emulated framebuffer stop
   being served locally once their channel is born over the guest's ring), but *coherent with*
   is not *established by*, and I am not re-grading a rule I wrote yesterday to fit today.

### 3.2 ⊘ THE RESIDENCY TOKEN WAS A SYSTEMATIC FALSE NEGATIVE, exactly as pre-registered

```
fbuserd@0x101a088 GET=0 PUT=1 resN-NEVER-WRITTEN   ← ring
fbuserd@0x101a088 GET=0 PUT=1 resY                 ← off, same address
```

`resN-NEVER-WRITTEN` on **every** armed row, on pages whose bytes were read correctly. Cause:
`SparseFb::install_join` removes the local pages for a joined range, and `is_resident` asks
`self.pages.contains_key`. `FbStore::read` checks the join first; `is_resident` was never
widened to match.

★ **Caught before any output was read**, by asking what the instrument would print against the
code path rather than against the intent, and fixed at **`75e8715`** — which this binary
predates. ⚠ The pre-registration had already named `resN-NEVER-WRITTEN` as meaning *"the
instrument is at the wrong address"*, so an unfixed token would have manufactured the
pre-registered failure reading on the arm the rung is about.

⊘ The **values** are unaffected: the join is checked first on the read path, and the `off`/`ring`
pair at the same address proves it (`PUT=1` on both).

### 3.3 P13 held — `CUP2_RC = 124` on both arms, movement 0 as predicted

`cup2` reaches `cuDeviceTotalMem` (`totalMem=11959 MiB`, `compute=8.6`) and then times out, on
both arms, exactly as before. ⊘ `name=` is still empty — the `GPU_GET_NAME_STRING` gap, unrelated.

---

## 4. WHAT THIS RUN CANNOT PROVE

- ⊘ **That the offset is right in general.** It is right *here* because the guest's USERD object
  is based at the joined leaf's base, so the derived offset and the guest's declared
  `userdOffset[0]` coincide (§2.1). A guest that allocated differently would separate them, and
  nothing in this boot exercises that.
- ⊘ **That the fetch is leg B's doing rather than leg A2's.** `w262` had leg A2 alone and no
  Xid; this run has both and eight. That is an **arm difference across two rungs**, not a
  controlled comparison — the clean experiment (leg A2 armed, leg B forced off) was not run.
  ⚠ This is the one I would most want next, and it is cheap.
- ⊘ **Anything about completions.** `CE-SUBMIT → RETIRED` = 0 on both arms, as predicted.
- ⊘ **That `GET` moved on more than one channel**, or on the GR channels specifically (§2.3).
- ⊘ **That a non-zero `userdOffset[0]` is honoured *by RM* rather than merely accepted.** The
  fetch is evidence that *something* consistent happened; only a `PUT`/`GET` pair moving at an
  offset RM was told and we did not write separates the readings.
- ⊘ **That leg B is correct rather than lucky** (`a_green_test_can_hold_a_wall_in_place`).
  Adoption at creation makes the ordering question moot as a property of the code, not of this
  boot.

---

## 5. THE NEXT ADDRESS, and it is written down rather than inferred

The eight VAs the engine faulted on are the pushbuffer leaves:

```
0x202400000  0x202600000  0x202800000  0x202a00000
0x202c00000  0x202e00000  0x203000000  0x203200000     (0x200000 apart, Vidmem, vas=0xcafe0005)
```

⇒ The supply side that joined the ring's leaf has to join **these**, and the join mechanism is
the one that already works (`join_fb_leaf` / `BackingBytes::JoinsGuestWindow`). ★ Note what has
changed about the question: before this boot *"the pushbuffer pages are not reachable"* was a
prediction in a pre-registration; it is now a fault address list from hardware.
