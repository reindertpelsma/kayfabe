# w283 / RESULT — THE GUEST'S OWN RELEASE, CARRIED TO HARDWARE; AND WHY CRITERION 1 IS A DIFFERENT RUNG

**STATUS: LIVE — 2026-08-13.** Owner ruling of 2026-08-13: *"go with the third shape"* — option 1
(**hardware writes the semaphore**) **plus** option 2's aside (**`AdoptedGuestUserd` for the CE
channel**), so that criteria 1 and 3 are reached **with no CPU write anywhere**.

⇒ **Half of that ruling is built and booted. The other half is REFUSED BY THE TREE'S OWN CODE, by
name, at two separate sites — and the refusals are right.** That is the lead.

---

## ⊘⊘⊘ LEAD — WHAT CONTRADICTS THE BRIEF

> *"Give the CE channel an `AdoptedGuestUserd` … Then `GP_GET` advances because the cursor
> hardware moves is the guest's own."*

**Both premises the brief asked me to verify are real, and the mechanism still does not port —
for a reason that is neither a "disposition" nor a missing feature.**

### What IS true (verified before writing a line)

- ★ **`AdoptedGuestUserd` exists** (`crates/kayfabe-isolate/src/lib.rs:723`), nested inside
  `AdoptedGuestRing::userd` exactly as the brief says, deliberately unspellable on its own, and
  its own doc already carries the **AT CREATION, NEVER AT THE FIRST DOORBELL** trap.
- ★ **And its `memory` is already its OWN `HostHandle`.** The doc's *"always the same object as
  `AdoptedGuestRing::memory`"* is an **observation about the GR case**, not a constraint of the
  type — which matters, because `[measured, w282b]` the raw CE client's USERD is at framebuffer
  `0x50000` while its ring is in the leaf at `0x40000`. **Different leaves.** So a CE pairing is
  *expressible*; leg 7 would simply have to join `0x50000` as well.
- ★ **`sem_releases` is parsed** (`kayfabe-fwd/src/lib.rs:4551`, `Vec<(GpuVa, u64)>`) and — on the
  forwarding path — was being **dropped on the floor**. That half of the ruling is now built.

### ⊘⊘ WHY IT STILL DOES NOT PORT — and the tree says so itself, twice

The CE forwarding path is **`ce_copy`**, and `ce_copy` **drives its channel**: it composes the
methods into the ring object, writes the GPFIFO entry, writes `GP_PUT`, then reads the cursors
back. Every one of those needs a channel whose ring and USERD are **ours**. On a channel over the
guest's, the tree already refuses — by name, at the top, deliberately:

```rust
// rm.rs, submit_entry:
if parts.owner == RingOwner::HandedIn {
    return Err(RmError::Other(RING_NOT_OURS));
}
```
> *"Composing methods into a ring we do not own is the wrong verb, and it says so by name."*

```rust
// rm.rs, userd_store_u32 / read_gp_cursors:
r.userd.as_ref().ok_or(RmError::Other(USERD_NOT_OURS))?
```

and the reason the USERD view does not exist is **RM's, not ours** — `[measured, R31 arm B]` an
`OS_DESCRIPTOR` over another process's pages **cannot be CPU-mapped at all**
(`NV_ERR_NOT_SUPPORTED`; *"memMap_IMPL: CPU mapping not supported for addressSpace: 0x1"*).

⇒ **`ce_copy` and `AdoptedGuestUserd` are mutually exclusive by construction.** One is *we drive
the channel*; the other is *the guest drives it*. They are not two dispositions of one design —
they are two designs.

★★★ **This is `w248` in its sharpest form.** The brief warned *"the host kind is a FUNCTION of the
guest kind — check before porting"*. Checked: here the host channel's **ring/USERD ownership** is a
function of **who composes the work**, and CE-forwarded-by-decode fixes that answer to *ours*.

### ⇒ WHAT CRITERION 1 ACTUALLY COSTS

Not an `AdoptedGuestUserd` on the CE channel — **CE doorbell passthrough**: the `HandToCore` shape
GR already has, where the guest writes its own pushbuffer, its own GPFIFO entry and its own
`GP_PUT`, and we only ring the host token. Then `GP_GET` advances because hardware is walking the
guest's own ring, which is exactly the owner's intent.

⚠ **It replaces the mechanism that currently makes criterion 2 green.** The brief says *"do not
regress it — it is a known-positive you now own."* Standing up passthrough while keeping
`ce_copy`'s green means running both shapes behind one flag for at least one rung. **That is the
next rung, and I am not starting it inside this one.**

---

## ★★★ WHAT IS BUILT AND BOOTED — option 1, end to end, with no CPU store

`CeSubCopy::guest_release: Option<CeGuestRelease>`, carried core → wire → child → `ce_pushbuffer`,
which appends to the **same pushbuffer**, **behind** the copy's own `LAUNCH_DMA`:

```
SET_SEMAPHORE_A/B/PAYLOAD   ← the guest's VA (split 48:32 / 31:0), the guest's literal payload
LAUNCH_DMA                  ← LAUNCH_TRANSFER_NONE  (release only; moves NO bytes)
```

**⊘ Nothing here is a CPU store, and the no-forgery rule cannot be violated by it.**
`ce_executor_tree.md` rule 1 forbids *"signalling completion for work that did not happen"*.
The release is a method in the same pushbuffer as the copy, behind the copy's own launch,
executed by the same engine in submission order — **there is no ordering of these methods in
which the payload lands and the bytes did not.** And our own release is still emitted **last**,
so `await_semaphore` returning means the guest's release has retired too rather than racing it.

★ Attached to the **last span of that launch only**, and **only when it is `HostCe`** — this
field is the *host engine's* instruction, and putting it on a CPU-executed span would name a
writer that is not the one running the work.

### ⊘⊘ AND A CORRECTION TO MY OWN COMMENT, made before anyone could rely on it

My first draft justified the `HostCe`-only scoping with *"a span on `CeExecutor::Ours` keeps
`kayfabe_rt::cpu_ce::write_completion` as its writer, unchanged"*. **True of the design and false
of the tree.** `[measured 2026-08-13, `git grep write_completion` over the whole workspace]` that
function — the documented `sem_releases` consumer — has **zero call sites**; every hit is its own
definition or a doc reference. So `sem_releases` is populated on the forwarding path and
**dropped**, which is exactly what every control arm measures (`semaphore 0x00000000`, every
boot, including `w282`'s and `w283`'s).

⇒ **There is exactly ONE writer for a guest completion in this tree, it is the engine, and it is
this field.** Left standing, that comment would have read as *"the other arm is already
handled"* — a citation that looks like provenance and resolves to nothing.
★ `u32::try_from` on the payload, and a wider one is **declined, never truncated**: a one-word
release cannot carry it, and writing a *different* value at the address the guest polls is worse
than writing none.

---

## ⊘⊘ AND IT REGRESSED CRITERION 2 ON THE FIRST BOOT — caught by the identity falsifier

`[w283_client, real GA106]`:

```text
CE-SUBMIT dst=0x120010000 len=4096 by=HostCe src=Address(4831838208)
          → REFUSED BEFORE SUBMISSION Other(19274)
R33 arm 1 COPY = dst[0] 0x3f0011cc -> 0x3f0011cc (want 0xc0ffee33)   ⊘ criterion 2 LOST
```

`19274 = 0x4B4A = BAD_ENCODE`. **The pushbuffer slot is 64 bytes and our own push filled it
exactly** — 16 words, to the byte. The guest's release added six more.

⚠ **And note how it presented.** The length check answers with a constant named `BAD_ENCODE`, so
the boot log said *"the encoding is wrong"* when the encoding was right and the **slot** was too
small. A refusal whose name is true of a different cause — a class this campaign has paid for
repeatedly. ★ The site now **prints both numbers** and says which refusal it is.

⇒ **Fix:** `PUSHBUFFER_SLOT_BYTES` 64 → 128, and `PUSHBUFFER_SLOTS` **derived** from
`GPFIFO_OFFSET - PUSHBUFFER_OFFSET` rather than assumed. ★ `next_slot` took the index modulo the
ring's **entry count** alone, which is safe only while `entries × slot_bytes` happens to fit —
true at 64 × 64 and false at 64 × 128. **That was a latent bug at the old numbers too**: nothing
tied `GPFIFO_ENTRIES` to the region size, so a larger ring would have written methods over the
GPFIFO that indexes them, with no check anywhere.

### ★ THE TEST THAT WOULD HAVE CAUGHT IT, now in the tree

`every_push_this_encoder_can_emit_fits_one_pushbuffer_slot` — quantifies over both arms of the
encoder and asserts the **relation**, so raising the slot size is a fix it accepts and a
hard-coded `22` would not be. Beside it,
`the_guest_release_names_the_guests_address_and_moves_no_bytes` grades on **identity**: the copy's
words are byte-identical, the tail carries the **guest's** address split as `SET_SEMAPHORE_A/B`
and the **guest's** literal payload, and its launch is `TRANSFER_NONE`. ⊘ A test that only counted
words would pass on a second launch that **re-ran the copy** (a silent doubling of every transfer)
and on one naming **our** semaphore twice (the guest polls forever with every row green).
Both pass; `cargo test -p kayfabe-isolate-host --lib` = **67 passed, 0 failed**.

---

## ⊘ A VOID BOOT, DECLARED VOID — the guards did their job

`w283b_client` (the re-run with the 128-byte slot) is **VOID and is not a data point**:

```text
WITNESS-ARM: FAIL — route B is UNREACHABLE without it. ⊘ VOID.
RING-PROJ lines = [0]   fbRING rows = [0]   DOORBELL-XLATE = [0]
FAIL RM bring-up failed at R1 openat(nvidia<gpu>): Syscall { call: "openat", errno: Some(5) }
```

The guest's driver failed to open the GPU (`EIO`) before the client issued a single ioctl.
⊘ **Not attributed to the change, and not attributed to anything else either — it is flaky**, and
two same-revision controls say so: the **native** arm from the same binary passed **all three**
criteria minutes earlier, and the **`clientoff`** arm at the same revision booted, reached
`RING-PROJ = 1` and produced its expected `by=Ours … REFUSED BEFORE SUBMISSION Other(19270)`
(`NOT_ON_THIS_RUNG` — `ce_copy(Ours)` refusing exactly as it always has). ⇒ re-run, do not bisect.

⚠ And the grader had a bug of its own this boot: a **backtick pair inside an `echo`** ran as a
command substitution (`line 193: armed: command not found`). The sentence survived mangled and the
grade continued — but a backtick in a graded line is one quoting slip from swallowing the verdict
it prints. Fixed.

---

## THE BAR — where the guest stands

| # | criterion | status |
|---|---|---|
| 1 | `GP_GET` catches `GP_PUT` (**the guest's**) | ⊘ **NO** — and it is a different rung; see the LEAD |
| 2 | the bytes moved | ★ **MET at `w282b`** (`dst[0]`/`dst[last]` read back, `by=HostCe`, `RETIRED`, `HOST_DMESG_XID=0`) |
| 3 | the semaphore carries the declared payload | ◐ the mechanism is **built, tested and shipped**; the boot that proves it is pending the re-run |

⚠ **`guest_rel=CARRIED` means EMITTED, not OBSERVED.** The `CE-SUBMIT` line says what we put in
the pushbuffer; hardware wrote it **iff** that VA resolves in the executor VAS. The only witness
for criterion 3 is the **client's own `semaphore 0x…` read**, and the grader says so inline —
`forwarded_counts_intent_not_work`, one plane over.

## ⊘⊘ WHAT THIS CANNOT PROVE

- **Criterion 3 is not measured yet** — built, unit-tested, and pending a green boot.
- **Criterion 1 is not reachable on this shape at all**, and that is a measurement, not a delay.
- The cleanup design (`docs/design/operand_join_lifetime.md`) is **still unwired**; every joined
  leaf lives for the life of the `Vas`.
- One workload, one chip (GA106), one driver (`580.159.04`).
