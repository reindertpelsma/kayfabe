# w280 / RESULT — `cup2`'s RING RELABELS, THE REFUSAL LEAVES THE RING, AND `CUP2_RC=124` ANYWAY

**STATUS: LIVE — 2026-08-12.** ⊘ **Recovered, not run by me.** These three boots were executed
by the lane that was OOM-killed before it could commit them; the artefacts sat on `vh` and are
committed here. Every number below was read from a log opened in this session.

| arm | revision (stamp gate) | route B | workload |
|---|---|---|---|
| `w280_cup2` | `0b58dd321ff8857a9e3c2b930d845ac8bca6fce7` | **ON** | `cup2` |
| `w280_client` | `0b58dd321ff8857a9e3c2b930d845ac8bca6fce7` | **ON** | raw CE client (53 ioctls) |
| `w280b_cup2off` | `bf9f13e831da7e19078b2f3d5c25be1138fcc105` | **OFF** | `cup2` — the same-revision control |

⚠ **The two revisions differ only in `scripts/bench/w280_run.sh`.** `bf9f13e`'s diff is 36
insertions / 24 deletions in that one file (`git show --stat`), so **the device source is
byte-identical across all three arms** and the control is one-variable. Both stamp gates PASS
(binary `kayfabe-rev` == tree HEAD, both printed), all six carried arms PASS on every arm,
`ENOSPC_LLVM=0`, and both runs carry `=== W280 EXIT rc=0 ===`.

---

## ★★★★★ THE RESULT — three facts, and the control is what makes them attributable

| | `w280b_cup2off` (route B **OFF**) | `w280_cup2` (route B **ON**) |
|---|---|---|
| `fbRING[p0]@0x1024000` | `nz4/4096` **`JOINED-one-memory`** | `nz4/4096` **`JOINED-one-memory`** |
| `resN-NEVER-WRITTEN` | **0** | **0** |
| `JOINED-one-memory` | 123 | 123 |
| first doorbell refusal | `PushbufferAperture { va: GpuVa(8592179200) }` = **`0x200224000`, THE RING** | `RmError::Other(19270)` — **and no VA `0x200224000` anywhere** |
| `FWD-RING` lines | **0** | **8** (`bytes=65536 live=1 spans=0`), incl. `key=…:0x5c00004b` |
| `DOORBELL-REFUSED` | 24 | 24 |
| **`CUP2_RC`** (anchored) | **124** | **124** |

1. ★★★ **H11a FIRED, and it is the JOIN FIX's doing, not route B's** — the relabel is present
   on **both** arms. `cup2`'s walling-channel ring page carries `JOINED-one-memory` where all
   six prior `cup2` boots carried `resN-NEVER-WRITTEN`, and `resN-NEVER-WRITTEN` is **0**
   log-wide. ⇒ `w279`'s fix reaches `cup2`. The control is what licenses this attribution: had
   only the armed arm been run, the relabel would have been confounded with route B.
2. ★★★ **H15 FIRED — the refusal leaves the ring, and THAT is route B's doing.** With route B
   OFF the first refusal is `PushbufferAperture` at `0x200224000`, the ring. With it ON, the
   decoded VAs are `0x0 0x203e90000 0x799fed000000` — **`0x200224000` is not among them** — and
   the first refusal is a different kind entirely (below).
3. ★★★★ **H11b FIRED — `CUP2_RC=124` on both arms.** ⇒ **The join-aware residency fix is
   NECESSARY-NOT-SUFFICIENT for `cup2`.** Stated plainly, as the brief asked: **`cup2` did not
   move.** The pre-registration predicted exactly this from `w246d` before the boot.

### ★★★★★ H13 — THE ONE GENUINELY OPEN DEVICE QUESTION — DID NOT FIRE. The join serves the SAME BYTES.

Pre-registered falsifier, verbatim: `gp[0]@0x200224000` must read `0x202c00000+0x20` and
`nonzero=[0]` must read `0x0000220202c00000`. Both arms print **exactly** that, matching `w246d`
(before the join existed) and `w277`. ⇒ **The FB-leaf join is byte-faithful on the read path.**
A stale or zeroed join was the new-defect arm and it is refuted.

---

## ⊘⊘ THREE THINGS THAT CONTRADICT THE BRIEF AND THE PRE-REGISTRATION

### 1. ⊘ `CE-SUBMIT lines = 68` is NOT "route B submitted work"

The grader prints `CE-SUBMIT lines = 68 (w246: 0 in all four corners)`, which reads as route B
having crossed the line `w246` never crossed. **It did not.** All 68 lines are one string:

```text
CE-SUBMIT dst=0x204420000 len=32768 by=Ours src=Constant(0)
  → REFUSED BEFORE SUBMISSION Other(19270) (no ring store, no doorbell, no semaphore)
```

`by=Ours`, `src=Constant(0)`, `dst=0x204420000` ⇒ **the kernel CeUtils scrubber**, ours, not the
guest's ring, and **every one of them was refused before submission**. `spans=0` on all 8
`FWD-RING` lines says the guest's rings decoded to **no CE span at all**. ⇒ `w246`'s finding
stands: **route B enumerates a ring and does not submit the guest's work.** ⚠ A grader line
whose parenthetical invites a comparison across two different producers is the
`a_count_cannot_see_a_substitution` shape — the number moved 0 → 68 and the *thing being
counted* changed.

### 2. ⊘ `Other(19270)` is OUR OWN sentinel, not a driver error

`19270 = 0x4B46 = "KF"` = `kayfabe_isolate_host::rm::NOT_ON_THIS_RUNG` (`rm.rs:157-158`), the
marker for a verb this rung does not implement. So the route-B-ON arm's first doorbell refusal
is **self-inflicted and named**, not RM refusing us. ⚠ It prints as `[RmError::Other] Rm { err:
Other(19270) }`, which reads like a driver status; only the constant's definition says
otherwise.

### 3. ⊘ The pre-registration's §2 reasoning was right for the wrong channel

It ruled `lib.rs:4752`'s `VidmemRoute::Refuse` off `cup2`'s path because `cup2`'s pushbuffer is
sysmem, citing `w277`'s `pb=S:0x25d78000`. **Confirmed** — all 16 of this boot's pushbuffers are
`pb=S:` (`0x11412000 … 0x42f12000`). ⇒ H6 correctly did not fire, and the decision not to lift
the gate for *this* rung was correct. ⚠ But it is **on the raw client's path**: the client arm
prints `pb=V:0x40000` — **vidmem** — which is the next rung's whole subject.

---

## THE CLIENT ARM — an exact re-run of `w279` at this revision (H19: no divergence)

`GUEST_MD5=21c577865dd733f22dd9ecb08c5fb1f1` = the native md5, `GUEST_EXECUTABLE=yes`,
`GUEST_NVRM_LOADED=1`, `total=53 failed=0 logged=53 dropped=0`, guest `dmesg` 5379 bytes with
**31 `NVRM` lines**, host Xid **0** (watermark `1010 → 1010`, `HOST_DMESG_XID=0`).

```text
first doorbell refusal [FwdFault::PushbufferAperture]
  PushbufferAperture { va: GpuVa(4831969280), aperture: Vidmem }   ← 0x1_2002_0000, THE PUSHBUFFER
  ring=0x120021000 rng=V:0x41000  gp[0]@0x120021000=0x120020000+0x40  pb=V:0x40000
  fbuserd@0x50088 GET=0 PUT=1 resY   scan=64/64 declared (COMPLETE)
FAIL R33 arm 1 COPY = dst[0] 0x3f0011cc -> 0x3f0011cc (want 0xc0ffee33), semaphore 0x00000000
     (want 0x00000001), GP_GET 0 GP_PUT 1 — the entry was NEVER fetched
R33_RC=1
```

⇒ `RingFbNeverWritten = 0`, refusal at `0x1_2002_0000`, `R33_RC=1` — **identical to `w279`**.
Arms 2 (OCCUPIED) and 3 (FREE) green. **Zero of the three pass-criteria are met in the guest.**

★ And the client's `FWD-RING lines = 0` while `cup2`'s is 8: the client's doorbell never reaches
the ring reader, because it refuses **earlier**, at the pushbuffer.

---

## ARMS — how they fell

| # | prediction | outcome |
|---|---|---|
| **H11a** | `cup2`'s ring relabels to `JOINED-one-memory` | ★★★ **FIRED** — and on **both** arms ⇒ the join fix, not route B |
| **H11b** | `CUP2_RC=124` regardless | ★★★★ **FIRED** — necessary-not-sufficient, on both arms |
| **H12** | `FWD-RING` for `key=…:0x5c00004b`, `live=1 spans=0` | ★★★ **FIRED** — 8 lines, 1 on that key, `w246d`'s line restored **through the join** |
| **H13** ⚠ | the join serves **different** bytes | ⊘ **did not fire** — `gp[0]` and `nonzero=[0]` match `w246d`/`w277` verbatim |
| **H14** | `RingFbNeverWritten` at a phys ≠ `0x1024000` | ⊘ did not fire — count 0 |
| **H15** | the refusal moves off the ring | ★★★ **FIRED**, and the control attributes it to route B |
| H6 | `cup2`'s pushbuffer is vidmem after all | ⊘ did not fire — 16/16 `pb=S:` |
| H16 | `GP_GET` moves | ⊘ not a discriminator, as pre-registered |
| H17 | `CE-SUBMIT > 0` or `CUP2_RC=0` | ⊘ **did not fire** — see contradiction 1: the 68 lines are ours and all refused |
| H18 | `cup2`'s ring does NOT relabel ⇒ H11 refuted | ⊘ did not fire |
| H19 | the client diverges from `w279` | ⊘ did not fire — identical |
| H20 | a VOID boot | ⊘ did not fire — `RING-PROJ` 88, `fbRING` 89, `CUP2_OUT_BYTES=330`, `reached-cuCtxCreate = yes`, client md5 matched |
| H21 | boot fails / ENOSPC | ⊘ did not fire |

## ⊘⊘ WHAT THESE BOOTS CANNOT DECIDE

- **They do not make `cup2` work.** `CUP2_RC=124` on both arms.
- **They cannot say the two walls are the same defect.** They say the two walls **shared the
  false label**, that removing it moved neither workload to a pass, and that the two now wall at
  **different places**: `cup2` at our own `NOT_ON_THIS_RUNG`, the client at
  `PushbufferAperture` on a **vidmem** pushbuffer. ⇒ *"same defect"* is now closer to refuted.
- **The completion plane still has no oracle.**
- One workload each, one chip (GA106), one driver (`580.159.04`), one boot per arm.

## ★ THE NEXT ONE FACT — and it is the client's, not `cup2`'s

The client refuses at `pb=V:0x40000`, a **vidmem** pushbuffer, under the hard-coded
`VidmemRoute::Refuse` at `kayfabe-fwd/src/lib.rs:4752`. `cup2`'s pushbuffers are all `pb=S:`, so
that gate is **only** on the client's path. ⇒ Widening it is the raw client's next rung, and it
cannot be measured on `cup2` at all. See `traces/boots/w281/`.
