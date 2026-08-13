# w279 / RESULT — THE STORES LANDED, AND THE REFUSAL MOVED ONE STAGE

**STATUS: LIVE — 2026-08-12.** Branch `w279-the-ring-stores-landed`. Boot `w279_guest` at
source revision **`0a4ce497ffbb358f4a5aeabadd530e0526442524`** — stamp gate PASS, the QEMU
binary's `kayfabe-rev` equal to the bench tree's HEAD, both printed. All six carried arms
PASS. Every number below was read from an artefact opened in this session; none is carried.

Pre-registration: `PREREGISTRATION.md`, committed before the boot, **and corrected before the
boot** (the `cup2` correction at its head — I was wrong, see §4).

---

## ★★★★★ LEAD — THE BRIEF'S PREMISE IS FALSE, AND THE ANSWER WAS IN `w278`'s OWN LOG

The rung was set as *"a BAR1/vidmem write census — **where did those CPU stores go?**"*, over
`w278`'s reading that *"our emulated framebuffer has no record of those bytes."*

**They went exactly where they should.** `w278b`'s log says so in one self-contradicting line
that was read as the wall:

```text
fbRING[p0]@0x41000=0000022001400000…  nz4/4096  resN-NEVER-WRITTEN by?
```

`nz4` = **four non-zero bytes in that page**. Decoded against `kayfabe_abi::submit`'s own
field layout (`GP_ENTRY0_GET` 31:2, `GP_ENTRY1_GET_HI` 7:0, `GP_ENTRY1_LENGTH` 30:10), the
qword `0x0000_4001_2002_0000` is a well-formed GPFIFO entry: **`va = 0x1_2002_0000`, 16 dwords
= 64 bytes** — matching the same line's `gp[0]@0x120021000=0x120020000+0x40` and
`pb=V:0x40000 pbm[16w of 64B]`, whose decoded methods (`SET_OBJECT 0xc7b5`, the semaphore
`0x120022000`) the client independently printed.

⇒ **The CPU stores through `NV_ESC_RM_MAP_MEMORY` landed in our framebuffer, at the right
offset, and read back byte-correct.** `nz4` and `resN-NEVER-WRITTEN` are in the same line and
contradict each other.

### None of the brief's four candidates is what happened

| candidate | verdict |
|---|---|
| stores landed in a host mapping we do not model | ⊘ no — they landed in the joined leaf our own `FbStore::read` serves |
| stores landed at a **wrong FB offset** | ⊘ no — `0x41000`, and the bytes decode to the client's own pushbuffer |
| **serviced-and-discarded** (`w198`'s class) | ⊘ no — the bytes survive and are read back |
| `RM_MAP_MEMORY` returned something not FB-backed | ⊘ no |

★ It is the pre-registered **fifth** arm — *"the census cannot see the stores ⇒ instrument
gap"* — in its sharpest form: **it sees them as BYTES and not as PROVENANCE, and the doorbell
decided on provenance.**

### The cause, from source, no inference

1. `SparseFb::install_join` ends `self.pages.remove(&frame); self.origin.remove(&frame);` for
   **every frame of a joined range** — deliberately, and its own comment says why.
2. `SparseFb::read` / `write_tagged` check `joined_at()` **first** ⇒ the bytes are live.
3. `is_resident` / `page_origin` do **not** ⇒ `Some(false)` / `None` for every joined address.
4. `w278b`'s own log: `GR-RING-JOIN RING(chan=0 entries=64 engine=Ce) leaf va=0x120020000
   len=0x10000 fb_phys=0x40000 → JOINED (shared) … ★ ONE memory`, and
   `0x40000 ≤ 0x41000 < 0x50000`.

⇒ `PlaneFbSource::page_written` computed `page_writer(..).is_some()` = `Some(false)`, and
`fetch_ring_bytes` (`kayfabe-fwd/src/lib.rs:5162`) refused the doorbell.

⊘ **The trait contract was already right and its single implementation contradicted it.**
`FbSource::page_written`'s doc says *"`None` = cannot tell, which is unmeasured and must never
be read as no"*, and `FwdFault::RingFbNeverWritten`'s says *"raised **only** when
`page_written` answers `Some(false)`"*. There is exactly **one** production impl of that
trait, and it was the one that turned "cannot tell" into "no".

---

## ★★★★★ THE MEASUREMENT — the refusal moved, and only the ADDRESS shows it

One variable: the join-aware residency question. Same runner, same arming, same client.

| | `w278b_guest` | **`w279_guest`** |
|---|---|---|
| `RingFbNeverWritten` | **fired** (`va 0x1_2002_1000`, `phys 0x41000`) | **0** |
| first doorbell refusal | `RingFbNeverWritten` | `PushbufferAperture` |
| refusal **VA** | `0x1_2002_1000` — **the RING** | ★ `0x1_2002_0000` — **the PUSHBUFFER** |
| `fbRING[p0]@0x41000` | `nz4/4096 resN-NEVER-WRITTEN by?` | `nz4/4096` **`JOINED-one-memory`** |
| `resN-NEVER-WRITTEN` in log | present | **0** |
| `JOINED-one-memory` | 0 | **2** |

⇒ **The ring is now read out of our own framebuffer through the join, and the forwarding plane
advanced one stage** — from *fetching the ring* to *fetching the pushbuffer the ring points
at*. The bytes at `0x41000` are byte-identical between the two boots; only the label changed.

### ⚠⚠ A COUNT CANNOT SEE THIS MOVE — THE FAULT NAME IS THE SAME AS `w278` ARM 1's

`w278_guest` (route B off) also refused `PushbufferAperture`. **That refusal was at
`va 0x1_2002_1000` — the ring.** This one is at `0x1_2002_0000` — the pushbuffer. Same
variant, different address, different stage, opposite meaning. A grader counting
`PushbufferAperture` would have called this rung a no-op. **Every arm here is graded by the
VA, printed by the program.**

### The new blocker is DELIBERATE, and the source names it

`crates/kayfabe-fwd/src/lib.rs:4750-4757`, unchanged by this rung:

```rust
// ⊘ `Refuse`, explicitly: this rung wires the RING out of the framebuffer, not
// the pushbuffer the ring points AT. Widening both at once would make a boot
// unable to say which of the two reads produced the bytes.
for (src, at, take) in push_range_gpas(table, pdb, r, len, VidmemRoute::Refuse)? {
```

⇒ Not a defect and not a surprise: the pushbuffer read is hard-coded `VidmemRoute::Refuse` so
that exactly this boot could attribute the bytes. **The named next step is to widen it — as
its own flag, for the reason the comment gives.**

---

## ★★★ THE CONTROLS — both polarities, on this run

- **Native arm, same binary, same run, minutes before the boot** (`xid_w279_native.log`):
  `★ R33 arm 1 COPY = 4096 bytes moved: dst[0] 0x3f0011cc -> 0xc0ffee33 … engine semaphore
  0x00000001 … GP_GET 1 caught GP_PUT 1`. **GREEN on bare metal.**
- **Same program in the guest**: `total=53 failed=0 logged=53 dropped=0`, `GUEST_MD5 =
  ccb3ccf9504cb68f95110c3c6203ccb9` = the native md5, `GUEST_EXECUTABLE=yes`, `GUEST_NVRM_LOADED=1`.
- **Arms 2 and 3 green in the guest**; arm 1 still `FAIL … GP_GET 0 GP_PUT 1`.
- Guest `dmesg` persisted and non-empty: **31 `NVRM` lines**. Host Xid across the boot: **0**.

⊘ **The client's own verdict did NOT change (`R33_RC=1`), and that is expected**, not a
disappointment: `w246` measured that route B *enumerates* a ring and does not submit work
(`CE-SUBMIT = 0` in all four corners). This rung moved the **device's** wall, not the
client's.

---

## PRE-REGISTERED ARMS — how they fell

| # | prediction | outcome |
|---|---|---|
| **H1** | `RingFbNeverWritten = 0` + a **new, different** named refusal | ★★★★★ **FIRED** — and only the VA distinguishes it |
| H2 | doorbell SERVED/FORWARDED | ⊘ did not fire — as `w246` predicted |
| H3 | `RingFbNeverWritten` still fires | ⊘ did not fire |
| H4 | a different, earlier refusal | ⊘ partly — same *name*, but **later**, not earlier |
| **H5** | the dump prints `JOINED-one-memory` | ★★★ **FIRED** — 2 rows, `resN-NEVER-WRITTEN` = 0 |
| H6 | client arm 1 green in the guest | ⊘ did not fire |
| H7 | nothing changes (stamp/arming failure) | ⊘ did not fire — stamp PASS, 6/6 carried arms |
| **H8** ⚠ | **regression**: a zeros ring now forwarded | ⊘ **did not fire** — the `nz0`-ring grep printed nothing; the 13 `SERVED-LOCAL` are the kernel's CeUtils channels, as before |
| H9 | boot fails / hangs / ENOSPC | ⊘ did not fire |
| H10 | the ring bytes are not the client's entry | ⊘ did not fire — `gp[0]@0x120021000=0x120020000+0x40` |
| H11 | the fix is not `cup2`-specific | ⊘ **not measurable here** — `cup2` was not run |
| ★ unregistered | a **void boot printed this rung's predicted success** | ⊘⊘ **FIRED** — §3 |
| ★ unregistered | the `GP_PUT` witness has false positives | ★★ **FIRED and MEASURED** — §5 |

---

## ⊘⊘ 3. THE FIRST BOOT WAS VOID, AND IT PRINTED `RingFbNeverWritten = 0`

`[measured, /workspace/w279_run.log, first attempt — discarded]`

```text
=== CLIENT BUILD RC=0 ===
=== CLIENT md5= ===                       ← EMPTY, and nothing checked it
…/kayfabe-rm-ladder: No such file or directory
=== NATIVE ARM RC=127 ===
```

`$CARGO_TARGET_DIR` is exported before the client build, so cargo wrote the musl binary there
and **not** under `$REPO/target`, the only path `$CLIENT` named. The native arm never ran,
`KAYFABE_R33_BIN` pointed at nothing, and **the guest ran no client at all** — and that boot
reported `RingFbNeverWritten = 0`, which is **H1, this rung's predicted success**.

⊘ What separated them was the known-positive placed on my own grep **before** the boot:

```text
fbRING rows in this log = [0]  ⊘ 0 ⇒ VOID, not 'no join'
```

★ That single line is why this is a discarded run and not a false green. **An absent artefact
reads as favourable; run a known-positive on your own grep pattern.** ⚠ And note the shape of
the miss: `cargo`'s `RC=0` means *"I had nothing to do"*, which is also what it says when it
built somewhere else. The runner now asserts the **artefact** (`[ -s "$CLIENT" ]`, non-empty
md5), never the exit status.

---

## ★★★★★ 4. THE `cup2` CORRECTION — MADE BEFORE THE BOOT, AND IT IS THE BIGGER FINDING

My own pre-registration ruled *"the evidence says it is NOT `cup2`'s wall"*, from the kernel
channels' `GuestRam` rings. **That checked the wrong channels.**

`[measured, traces/boots/w277/run_w277_off_qemu.log.gz — a `cup2` boot]`

```text
GR-RING-JOIN RING(chan=0 entries=1024 engine=GrCompute) leaf va=0x200200000
             len=0x200000 fb_phys=0x1000000 → JOINED
first doorbell refusal [FwdFault::PushbufferAperture] … ring=0x200224000 rng=V:0x1024000
  fbRING[p0]@0x1024000=0000c00202220000 nz4/4096 resN-NEVER-WRITTEN
```

⇒ **`cup2`'s own compute channel ring is a joined framebuffer leaf carrying the same false
label**, and its four bytes decode as a valid GPFIFO entry (`va 0x2_02c0_0000`, 8 dwords). The
same defect is on `cup2`'s walling channel in **all six** `cup2` boots (`w268`, `w270`,
`w274`–`w277`).

★ **Why `RingFbNeverWritten` nonetheless reads 0 in all six:** the guard sits **downstream** of
the aperture check and all six ran **route B OFF**, so they stop at `PushbufferAperture` and
never reach it. ⊘ **A count of zero for a refusal that is unreachable is not evidence** — the
same shape as `w246`'s *"the count is 0 BECAUSE route B is on"*. **The defect was LATENT for
`cup2`, not absent.**

⚠ This rung still **cannot say the two walls are the same**: `cup2` was not run, there is no
`CUP2_RC`, and this boot's own next blocker (the pushbuffer's deliberate `Refuse`) is a stage
`cup2` has never reached. What it can say is that they **share a defect**, and that *"it is not
`cup2`'s wall"* is unsupported.

---

## ★★ 5. THE `BAR1 GP_PUT` WITNESS HAS MEASURED FALSE POSITIVES

Second, independent finding out of the same artefacts. The detector is `page offset == 0x8c &&
size == 4` on **any** BAR1 page — as strong as the assumption that every BAR1 page a guest
writes is a USERD, which `w278`'s workload broke by CPU-mapping its own data buffers
(`+0x9008c val=0xc0ffee56`, `+0xa008c val=0x3f0011cc` — the client's payload magics, on the
pages whose offset 0 carries `0xc0ffee33` / `0x3f0011cc`).

This boot, with the labelling in place:

```text
BAR1 GP_PUT: 2 of those 12 carried a value that CANNOT be a put pointer (>= 4096, the
largest GPFIFO this tree has seen) ⇒ they are guest DATA at offset 0x8c of a page that is
not a USERD.
```

⊘ **A label, not a filter**, and a **lower** bound on the false positives — a small data word
is indistinguishable from a cursor here. The state field's comment *"ONE ROW PER USERD PAGE,
i.e. per channel"* is now false and says so.

---

## ⊘⊘ WHAT THIS RUN CANNOT PROVE

- **It cannot say the guard is as strong as it was.** It is deliberately weaker on joined
  pages: `fbFIN@0x49004 … nz0/4096 JOINED-one-memory` is exactly the case — an all-zero joined
  page, where *"nothing wrote it"* and *"it was written with zeros"* are now indistinguishable.
  That is honest (the store genuinely cannot tell) and it is a real loss of forbidden #2's
  detector **inside joins only**; off every join the arm still fires, asserted by test.
- **It cannot say `cup2` reaches the same refusal.** No `cup2`, no `CUP2_RC`. See §4.
- **It is not the first forwarded work.** Route B enumerates a ring (`w246`: `CE-SUBMIT = 0`);
  the client's copy still does not happen and `R33_RC=1`.
- **The completion plane still has no oracle.**
- One workload, one chip (GA106), one driver (`580.159.04`), one boot per arm.

## ★ THE NEXT ONE FACT

The pushbuffer read at `va 0x1_2002_0000` refuses by a **hard-coded `VidmemRoute::Refuse`**
that exists so this boot could attribute the ring's bytes. Widen it — as its **own** flag,
never folded into route B — and the same 53-ioctl client says whether the methods arrive. ★ And
run `cup2` with route B **on**: it has never once reached the ring-fetch guard, so H11 is
answerable in one boot.

## ARTEFACTS

| what | where |
|---|---|
| pre-registration (+ the `cup2` correction, pre-boot) | `PREREGISTRATION.md` |
| the whole run, incl. the VOID first attempt's lesson | `w279_run.log` |
| the boot | `run_w279_guest_qemu.log.gz`, `run_w279_guest_probe.log`, `run_w279_guest_dmesg.log` |
| the native arm, same binary, same run | `xid_w279_native.log` |
| the fix | `kayfabe-device/src/fbwin.rs` (`FbPageStanding`), `plane.rs` (`fb_page_standing`; `fb_is_resident` **removed**), `kayfabe-qemu-raw/src/shim.rs` (3 call sites) |
| the tests, watched RED | `kayfabe-device/tests/fb_join.rs` §5 |
