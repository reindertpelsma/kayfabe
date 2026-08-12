# w279 PRE-REGISTRATION — the ring stores LANDED; the refusal was ours

**STATUS: LIVE — 2026-08-12.** Written and committed **before** the boot. Branch off
`54d2ade` (`w278` RESULT).

---

## ⊘⊘ THE BRIEF IS CONTRADICTED BEFORE THE BOOT — from `w278`'s own artefacts

The rung was set as *"a BAR1/vidmem write census — where did those CPU stores go?"*, over
`w278`'s finding that *"our emulated framebuffer has no record of those bytes."*

**That premise is false, and `w278`'s own log says so in a single self-contradicting line**
(`traces/boots/w278/run_w278b_guest_qemu.log.gz`, read this session):

```text
fbRING[p0]@0x41000=0000022001400000…  nz4/4096  resN-NEVER-WRITTEN by?
```

`nz4` = **four non-zero bytes in that page**, and they are the client's own GPFIFO entry:
`0x0000_4001_2002_0000` ⇒ `gpu_va = 0x1_20020000`, `length = 16 dwords = 64 B` — matching
the same line's `gp[0]@0x120021000=0x120020000+0x40` and `pb=V:0x40000 pbm[16w of 64B]`,
whose decoded methods (`SET_OBJECT 0xc7b5`, the `0x120022000` semaphore) the client
independently printed. ⇒ **The CPU stores through `NV_ESC_RM_MAP_MEMORY` landed in our
framebuffer, at the right offset, and read back byte-correct.**

### Where the `NEVER-WRITTEN` came from — three source facts, no inference

1. `crates/kayfabe-device/src/fbwin.rs` `SparseFb::install_join` ends with
   `self.pages.remove(&frame); self.origin.remove(&frame);` for **every frame of a joined
   range** — deliberately: *"a first-writer record for a page that no longer exists would
   attribute a resident-page census entry to memory the census can no longer see."*
2. `SparseFb::read` / `write_tagged` check `joined_at()` **first**, so the bytes are served
   from the joined backing and are live and correct.
3. `SparseFb::is_resident` / `page_origin` do **not** check the join ⇒ answer `Some(false)` /
   `None` for every joined address.

And `w278b`'s log states the ring leaf is joined:

```text
GR-RING-JOIN RING(chan=0 entries=64 engine=Ce) leaf va=0x120020000 len=0x10000
             fb_phys=0x40000 → JOINED (shared) memory=0xcafe0006 … ★ ONE memory
```

`0x40000 ≤ 0x41000 < 0x50000` ⇒ the ring page is inside the join.
`PlaneFbSource::page_written` (`shim.rs`) computed `page_writer(...).is_some()` ⇒ `Some(false)`
⇒ `fetch_ring_bytes` returned `FwdFault::RingFbNeverWritten` (`kayfabe-fwd/src/lib.rs:5162`).

⇒ **The refusal is OURS, about our own bookkeeping, not a statement about the guest.**

★ Of the brief's four candidate answers, **none** is what happened: not a host mapping we do
not model, not a wrong offset, not `w198`'s serviced-and-discarded (the bytes survive), not a
non-FB-backed `RM_MAP_MEMORY`. It is the pre-registered **fifth** arm — *"the census cannot
see CPU stores ⇒ instrument gap"* — in its sharpest form: **it sees them as BYTES and not as
PROVENANCE, and the doorbell decides on provenance.**

### ⊘ And the correction had already been written — for ONE of the three callers

`shim.rs`'s `fb_userd_cursors` carries a 2026-08-11 comment describing this exact failure and
fixing it with a **local** `if joined`. Its last clause — *"no caller had asked it about a
joined address before"* — was **false when written**: `fb_dump_row` and `PlaneFbSource` were
both already asking. ★ A correction implemented at one call site is not a correction.

---

## THE FIX UNDER TEST (built before this boot)

- `kayfabe_device::FbPageStanding` — `JoinedOneMemory | Resident | NeverWritten | Unknown`,
  with `written() -> Option<bool>` mapping `JoinedOneMemory → None` (**unmeasured**, the
  `dlen=0` lesson) and `Some(false)` **only** for `NeverWritten`.
- `RegPlane::fb_page_standing` checks the join first; **`RegPlane::fb_is_resident` is REMOVED**
  so a fourth caller cannot re-acquire the blindness.
- All three callers rewired: both dump rows and `PlaneFbSource::page_written`.
- Tests, both directions: a joined page must never read as unwritten **and** an unwritten page
  off every join must still be named (`crates/kayfabe-device/tests/fb_join.rs` §5).

⚠ **The guard is genuinely weaker on joined pages and must be.** A zero-filled joined page is
indistinguishable from a quiet ring and this store cannot tell. Answering `Some(false)` to keep
a guard alive is inventing a fact about the guest.

---

## ARMS — graded from artefacts, low arms included

| # | prediction | how it is read |
|---|---|---|
| **H1** ★ *(most weighted)* | `RingFbNeverWritten = 0`; the doorbell reaches a **new, different** named refusal | `grep -c RingFbNeverWritten` = 0 **and** a named `DOORBELL-REFUSED` with another `FwdFault` |
| **H2** | the doorbell is **SERVED/FORWARDED** | ⊘ unlikely: `w246` measured route B *enumerates* a ring, `CE-SUBMIT = 0` in all four corners |
| **H3** | `RingFbNeverWritten` **still fires** | ⇒ the join is not the cause, or a second path answers residency. My source reading would be wrong |
| **H4** | a **different, earlier** refusal (`PushbufferAperture`, operand/sema planes) | ⇒ the same blindness exists on more planes than the ring's |
| **H5** | the dump row prints `JOINED-one-memory` for `fbRING[p0]@0x41000` | ⊘ if it still prints `resN-NEVER-WRITTEN`, the **binary is not this HEAD** — check the stamp gate before reading anything else |
| **H6** | the client's arm 1 goes **GREEN in the guest** (`dst[0] → 0xc0ffee33`, `GP_GET 1`) | would be the milestone; not predicted |
| **H7** | nothing changes anywhere | ⇒ stamp/gate or arming failure, **not** a finding |
| **H8** ⚠ *(the regression arm)* | some **other** channel whose ring is legitimately never-written now gets past the guard and forwards **4 KiB of zeros** as a ring | forbidden #2 returning. Read `SERVED-LOCAL` and the kernel channels' rows; a served ring with `nz0` is the signature |
| **H9** | the boot fails / hangs / `ENOSPC` | not a data point |
| **H10** ⊘ *(lowest)* | the ring bytes read back are **not** the client's entry | ⇒ the join serves different memory and my reading of `w278b`'s artefact was wrong |

## ⊘ WHAT THIS BOOT CANNOT DECIDE, stated before it runs

- **It cannot say this is `cup2`'s wall — and the evidence available says it is NOT.** In the
  same `w278b` log the kernel's GR/CE channels report
  `GR-RING-JOIN … → ⊘ NOT A FRAMEBUFFER LEAF: GuestRam { gpa: 629026816 }`. A guest-RAM ring
  takes `PushSrc::Gpa` in `fetch_ring_bytes`, where the `page_written` guard **cannot fire at
  all**. `cup2` is not run on this boot and there is no `CUP2_RC`.
- **It cannot prove the crossing works in the other direction** (engine → guest).
- One workload, one chip (GA106), one driver (`580.159.04`), one boot.
