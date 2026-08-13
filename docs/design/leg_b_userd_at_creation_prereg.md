# w263 PRE-REGISTRATION — leg B, and the first boot on which all three legs are present

> ### STATUS — 2026-08-12 / **LIVE — PRE-REGISTERED, NOT YET RUN.**
> Written and committed **before** the build. Source revision to be stamped from the binary.
> Bench `vh`, RTX 3060 (GA106), host driver 580.159.04. Two arms from **one** binary:
> `KAYFABE_GUEST_RING` unset (`off`) and `=ring`.
> Graded in `traces/boots/w263/RESULT.md`.

---

## 0. ⊘ WHAT I ALREADY REFUTED, BEFORE ANY CODE — the brief's central premise

The brief said leg B is *"a MISSING CONSUMER, not a missing producer"*, and that the decision
should be `Some` iff the address table holds a `JoinsGuestWindow` binding **"at the channel's
declared USERD address"**.

**There is no declared USERD address.** The guest declares a *handle* in its own client
namespace and an *offset* into it, and neither is an address — which three documents had
already established and which is why all three concluded leg B was blocked. The producer
exists anyway, and it is not `mem_phys` and not a page-table walk:
`NV_CHANNEL_ALLOC_PARAMS.userdMem`, filled by the **guest's own CPU-RM** before the GSP RPC.
`docs/design/userd_mem_is_on_the_wire.md`; landed at `af2dd2e`.

⇒ Both halves were missing. The brief was right that a consumer was needed and wrong that a
producer was not.

---

## 1. THE PREDICTIONS

Every row is scored against `run_w263_{off,ring}_qemu.log` unless stated.

| # | observable | pred `off` | pred `ring` | confidence |
|---|---|---|---|---|
| P1 | `phys=fb:` tokens on `RING-PROJ` lines | **≥ 1** | **≥ 1** | ★★ 0.75 — see §2.1 |
| P2 | `adopt=GUEST-RING` | 0 | **16** | ★★★★ 0.9 (w262 measured 16) |
| P3 | `adopt=DECLINED` | 24 | 8 | ★★★ 0.85 |
| P4 | `userd=GUEST-USERD` | **0** | **≥ 1** | ★★ 0.6 — **the rung**, and it is gated on P1 |
| P5 | `guest_userd=` in the census tail | 0 | **= P4** | ★★★★ 0.95 (arithmetic) |
| P6 | `REFUSED USERD_NOT_A_JOINED_WINDOW` | 0 | **0** | ★★★ 0.85 |
| P7 | `REFUSED RING_NOT_A_JOINED_WINDOW` | 0 | 0 | ★★★ 0.9 |
| P8 | `RmInitAdapter failed` | 0 | **0** | ★★★ 0.8 — see §3.2 |
| P9 | host `Xid` in `run_w263_ring_hostdmesg.log` | 0 | **0** | ★★ 0.7 — see §3.1 |
| P10 | guest `NVRM` lines | 31 | 31 | ★★★ 0.85 |
| P11 | `fbuserd@…` tokens present | **≥ 1** | **≥ 1** | ★★ 0.75 (= P1) |
| P12 | `fbuserd GET=` non-zero anywhere | **0** | **0** | ★★★ 0.8 — see §2.3 |
| P13 | `CUP2_RC` | 124 | **124** | ★★ 0.7 — see §2.2 |
| P14 | `BAR1 GP_PUT` total writes | ~188 | ~188, **same order of magnitude** | ★★ 0.7 |
| P15 | `doorbells … 0 REFUSED` | true | true | ★★★ 0.85 |

`CAPTURE_RC`, `BUILD_RC`, `STAMP_GATE`, and `grep -c 'No space left on device\|LLVM ERROR'`
over the build + qemu + probe + capture logs **of the same invocations that produced the
statuses** are recorded but are not predictions.

---

## 2. ★★★ THE THREE THAT MATTER, AND WHAT EACH OUTCOME WOULD MEAN

### 2.1 P1 — does `userdMem` reach us at all?

This is the rung's single point of failure and it is **not** something I can reason my way to.
The decode is additive: if the guest's channel-alloc params stop before +188, every row reads
`phys=UNREADABLE`, leg B is `None` by construction, and the boot measures **only** that.

- **Prior for yes:** the *error-notifier* decode needs **268** bytes and is in production;
  `ChannelEngineWire` reads +128; the offset is `ChannelNotifierWire::V580.internal_flags - 76`
  and that number has been in the tree for weeks.
- **Prior for no:** ⊘ no committed log shows **either** of those decodes returning `Some` on
  this bench. The prior is entirely indirect, which is why P1 is 0.75 and not 0.95.

⇒ **`phys=UNREADABLE` on every row is a clean, cheap, complete answer** and it would say the
port cannot see past ~+72 of a channel alloc — which is a fact about far more than leg B.

### 2.2 P13 — ★★★ WHY I PREDICT `CUP2_RC` DOES NOT MOVE, on the first boot where it could

The brief says plainly that this is the first boot on which a `CUP2_RC` change is *genuinely
possible*, and asks for the expected **size** of any movement. **I predict zero movement, and
I want to be explicit that this is a prediction against the rung's own optimism.**

`CUP2_RC = 124` is the launcher's timeout: `cup2` never finishes. For it to finish, every one
of these must hold, and this rung supplies exactly one of them:

1. the host channel names the guest's ring — **leg A2, landed, measured `w262`**;
2. the host channel names the guest's cursor — **leg B, this rung**;
3. the doorbell reaches the host channel — **leg C, landed `b734995`**;
4. ⊘ **the guest's pushbuffer contents are reachable by the host engine at the addresses the
   GPFIFO entries name.** The ring's *leaf* is joined; the pages its entries **point at** are a
   different set of leaves and nothing in this rung joins them. `[measured, w262b]` the
   walling channels' `gp[0]` entries name `0x200400000`, `0x200800000`, … — VAs **outside**
   the `0x200200000` leaf;
5. ⊘ the completion path — `CE-SUBMIT → RETIRED` was **0** on both `w262` arms.

⇒ **Necessary-not-sufficient, exactly as `w260` measured for the supply side.** If `CUP2_RC`
moves to 0 I will have been wrong in the favourable direction and will say so; anything between
(a hang at a different point, a different rc) is more informative than either.

⚠ `the_join_landed_and_the_wall_did_not_move` is the precedent and it is the reason this row is
0.7 rather than higher: three arms all read 124 and the temptation was to read that as the
change not landing.

### 2.3 P12 — ★★★★★ `GP_GET` IS FINALLY READ, AND I PREDICT IT READS ZERO

`w262`'s own `RESULT.md` §3.3 says *"Nothing here reads `GP_GET`"*, and the brief asks for it
by name. `07817e3` adds `fbuserd@0x… GET=n PUT=m` on the `RING-PROJ` line, read out of the
**framebuffer** because after leg B the isolate cannot CPU-map USERD at all.

⚠⚠ **The three readings of `GET=0 PUT=0` are not the same finding and the residency token is
what separates them:**

| line | reading |
|---|---|
| `GET=0 PUT=0 resN-NEVER-WRITTEN` | **nobody has written this page** — not even RM's zeroing at channel creation. ⇒ we are reading the wrong address, **or** the channel was never born over it |
| `GET=0 PUT=0 resY` | the page is live and both cursors are zero: the honest wall |
| `GET=0 PUT=n≠0` | ★★★★★ **the wall, located exactly**: the guest advanced its cursor in the page RM is reading and the engine fetched nothing |
| `GET=n≠0` | ★★★★★ **the engine FETCHED.** This would be the first such observation in the campaign |

⇒ P12 predicts `GET` stays 0 everywhere, for §2.2's reason 4. **The row I most want is the
`PUT` beside it**, because `GET=0 PUT≠0 resY` is the first direct measurement that the ring and
the cursor RM holds are the ones the guest is moving.

---

## 3. ★★ THE TWO PLACES THIS RUNG CAN DO HARM, named before the run

### 3.1 P9 — a host `Xid` is the outcome that would teach the most

Leg B hands host RM a **non-zero `userdOffset[0]`** into an object it does not own the layout
of. If the offset arithmetic is wrong by a page, RM programs a runlist entry naming a physical
address inside the joined leaf that is **not** a USERD, and the engine DMAs into it.

⇒ **A host `Xid` on the `ring` arm and not on `off` names the address**, and per the brief's own
instruction that is an *addressing* finding, not a doorbell one. It is contained
(`gpu_fault_is_contained`) but it is real, and P9 is 0.7 rather than 0.9 because I have no
hardware evidence for the arithmetic — only the four watched-red mutations at `ad1aa8b`.

### 3.2 P8 — and the arm comparison is void if anything else moves

`w262`'s §3.3 rule: `ENGINE-OBJECT` census, `doorbells`, `BAR1 GP_PUT` totals and `GR-BIRTH`
totals must be **identical across the arms**. Exactly three numbers are expected to move:
`adopt=GUEST-RING` (P2), `userd=GUEST-USERD` (P4) and `guest_userd=` (P5). If a fourth moves,
the comparison is reported as void rather than re-graded.

---

## 4. ⊘ WHAT THIS BOOT CANNOT PROVE, WHATEVER IT SAYS

- **That the offset is right.** `USERD_NOT_A_JOINED_WINDOW = 0` says the *handle* was one we
  minted; nothing checks that the 512 bytes at that offset are the ones the guest is writing.
  The only instrument that could is `fbuserd`'s `PUT` **agreeing with a BAR1 `GP_PUT` store**,
  and ⊘ nothing joins a BAR1 offset to a channel, so the agreement can be observed but not
  attributed. `a_witness_that_covers_one_writer`.
- **That RM honours a non-zero `userdOffset[0]` on this part.** Source says it does
  (`kernel_channel_gv100.c:204-206`, `:234-237`); a green boot with `GET=0 PUT=0` is consistent
  with it being ignored. Only a moving `PUT` at the adopted offset distinguishes them.
- **Anything about the completion plane.** `CE-SUBMIT → RETIRED` was 0 on `w262` and is
  expected to stay 0.
- **That leg B is correct rather than lucky.** `a_green_test_can_hold_a_wall_in_place`: an
  observation cannot separate *"the ordering is right"* from *"the ordering is wrong and got
  away with it"*. Adoption at **creation** is what makes the ordering question moot, and that
  is a property of the code, not of this boot.
- ⚠ **`GET=0 PUT=0 resN-NEVER-WRITTEN` on the armed arm would look like the wall and would in
  fact mean the instrument is reading the wrong page.** It is called out here so the reading
  cannot be made after the fact.
