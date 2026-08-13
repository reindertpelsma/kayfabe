# w281b / RESULT — THE SWEEP BINDS THE OPERANDS, AND THAT MOVES THE WALL TO AN OWNER RULING

**STATUS: LIVE — 2026-08-12.** Boot `w281b_clientsweep` at revision
**`45acca797befc827f755fd2a1016c73883335044`** — stamp gate PASS, tree clean, `cap2b` guard 0,
`ENOSPC_LLVM=0`, `=== W281 EXIT rc=0 ===`. `GUEST_MD5=a28f06884ed3080e7c1d9b9185a46ca2` equal to
the native md5, `total=53 failed=0`, 6/6 carried arms PASS, `PT-SWEEP arm=on` asserted,
`PUSHBUF-VIDMEM: PASS (ON)`.

One variable against `w281_client`, which is committed at `eb20f82` — **and the device source
is identical between them** (`45acca7`'s only diff is `scripts/bench/w281_run.sh`).

---

## ⊘⊘⊘ LEAD — **MY OWN PRE-REGISTERED FALSIFIER WOULD HAVE DECLARED THIS A SUCCESS. IT IS NOT.**

I pre-registered, in `w281`'s RESULT and in the runner's own commit:

> *"`HOST_DMESG_XID` must go **1 → 0** while `CE-SUBMIT` stays **1**."*

Both halves fired. **`HOST_DMESG_XID=0`. `CE-SUBMIT lines = 1`.** By the falsifier as written,
this rung passed. **It did not.** Read the line's *identity*, not its count:

| | `w281_client` | **`w281b_clientsweep`** |
|---|---|---|
| `CE-SUBMIT` | `dst=0x120010000 len=4096 **by=HostCe** gp_get=1 gp_put=1 sem=0x0 want=0x1 → **NEVER-RETIRED**` | `dst=0x120010000 len=4096 **by=Ours** src=Address(4831838208) → **REFUSED BEFORE SUBMISSION** Other(19270)` |
| host Xid | **1** — `Xid 31 CE0 @ 0x1_20010000 FAULT_PTE` | **0** |

⇒ **The Xid went to zero because the submission never happened**, not because the engine could
suddenly reach the operand. `by=HostCe` → `by=Ours`; *submitted and faulted* → *refused before
submission*. **The count stayed 1 and the thing counted changed.**

★ This is `a_count_cannot_see_a_substitution` landing on **the falsifier I wrote one rung
earlier**, and it is the third instance in three rungs (`w280`'s `CE-SUBMIT 0 → 68`; `w281`'s
`gp_get=1` belonging to the host channel; now this). ⇒ **A falsifier over a COUNT is not a
falsifier.** The correct form was always: *"`CE-SUBMIT` must still read `by=HostCe`, and
`HOST_DMESG_XID` must be 0."*

---

## ★★★ THE REAL ADVANCE, AND IT IS REAL — THE OPERANDS NOW RESOLVE

```text
w281_client :  OPERAND-TABLE: 2 page(s) asked, 0 resolved in guest RAM, 2 MISS, 0 NOT-IN-GUEST-RAM
               [va=0x120000000:Address(Miss { pdb: Pdb(24576) })  va=0x120010000:Address(Miss { … })]
w281b       :  OPERAND-TABLE: 2 page(s) asked, 0 resolved in guest RAM, 0 MISS, 2 NOT-IN-GUEST-RAM
               [va=0x120000000:Vidmem@0x10000   va=0x120010000:Vidmem@0x20000]
```

**`2 MISS` → `0 MISS`.** The whole-VAS sweep bound both CE operand VAs, and resolved them to
framebuffer offsets `0x10000` and `0x20000`. The populate gap `w281` named is **closed**.

### ⊘⊘ AND THIS REFUTES `w276`'s HEADLINE FOR THIS CASE, exactly as pre-registered

`w276` measured `bound=0 swept_binds=0` on **all 88** rows and concluded the sweep is *"built,
correct, cheap — and **inert here**"*. That was a **GR** arm on which `parse_pushbuffer` never
ran (`FWD-RING = 0` on both its boots; a GR doorbell answers `ring_content_is_forwardable` =
no), so **nothing ever asked the table**. This arm is **CE**, `parse_pushbuffer` runs, the CE
operand decode asks — and the sweep answers. ⇒ *"The sweep binds nothing"* is **scoped to a case
where nothing ever asked**, and a ruling's architecture is part of its citation.

---

## ★★★★★ THE NEW BLOCKER IS AN OWNER RULING, NOT A BUG — ESCALATING AS INSTRUCTED

The operands resolve to **`Vidmem`** — our **emulated framebuffer**. That is *fabricated* space,
and the partitioner's rule is explicit (`kayfabe-rt/src/ceutils.rs:274`):

> `Representability::Fabricated` ⇒ `CeExecutor::Ours` ⇒ the shell CPU executor

and `HostRmBackend::ce_copy` refuses `CeExecutor::Ours` **unconditionally and by name** —
*"needs the isolate's mapping of the fabricated aperture, which does not exist"* — under a
**standing owner ruling** (`docs/design/ce_executor_tree.md`, owner 2026-08-07), which this
tree's own comment calls **CORRECT and stays**:

> *"the CPU branch cannot execute in the isolate … so `ce_copy(Ours)` must keep refusing there"*

⇒ **The two configurations I can reach are both walls, and neither is a fix:**

| | operands MISS (`w281`) | operands bound as Vidmem (`w281b`) |
|---|---|---|
| executor | `HostCe` — a real host engine | `Ours` — the shell CPU |
| outcome | executes, **faults**: `Xid 31 FAULT_PTE @ 0x1_20010000` | **refused before submission**, by owner ruling |
| why | the host channel's page tables do not map the guest's VA | the operand lives in fabricated space no real engine can be pointed at |

**Both failures are the same missing thing, stated two ways: the guest's CE operands live in our
emulated framebuffer, and a real host engine needs them as real host GPU memory its own tables
can reach.** That is the *join/publish* step the ring already has
(`GR-RING-JOIN … → JOINED`), one object further along — and the ring's join is scoped to **RING
objects only**, measured this boot: the single adopted leaf is `va=0x120020000 len=0x10000
fb_phys=0x40000`, which covers pushbuffer/ring/semaphore and **not** the operands below it.

### ⇒ THE OPTIONS, for the owner. I am not choosing between these unilaterally.

1. **Extend the FB-leaf join to CE operand pages** — make the operand's framebuffer pages
   host-reachable the same way the ring's are, so the executor stays `HostCe` and the host
   engine's tables resolve the VA. ★ Most consistent with what already works; the machinery
   (`fb_join=shared`, `JoinsGuestWindow`) exists. ⚠ It widens what the guest can cause to be
   published, so it is a hostile-guest-isolation question, not only a plumbing one.
2. **Relax `ce_copy(Ours)`** to let the shell CPU execute the copy. ⊘ **I recommend against
   it and did not attempt it**: it contradicts a standing owner ruling by name, and it would
   make the data plane CPU-emulated — precisely what `CLAUDE.md` says the C artifact's green
   already was, and what this rewrite exists to stop being.
3. **Influence the operands' aperture at allocation time** so they land in sysmem (route A's
   shape, one object along), where the existing guest-RAM pin already works.

⊘ **Nothing here forges a completion, reads host memory the guest does not own, needs root, or
weakens isolation** — and I have not widened any boundary to get a green.

---

## THE THREE PASS-CRITERIA — still ZERO of three in the guest

```text
FAIL R33 arm 1 COPY = dst[0] 0x3f0011cc -> 0x3f0011cc (want 0xc0ffee33), semaphore 0x00000000
     (want 0x00000001), GP_GET 0 GP_PUT 1
R33_RC=1
```

1. `GP_GET` catches `GP_PUT` — ⊘ **NO** (`GP_GET 0 GP_PUT 1`).
2. The bytes moved — ⊘ **NO** (`dst[0]` still its pre-fill).
3. The semaphore carries the declared payload — ⊘ **NO** (`0x00000000`).

⚠ **This arm is a REGRESSION in execution terms and an ADVANCE in addressing terms**, and both
must be said: `w281` got a real host engine to *execute* the guest's methods; `w281b` does not
submit at all. The right next tree is `w281`'s (`by=HostCe`) **plus** operand publication, not
`w281b`'s.

## ⊘⊘ WHAT THIS RUN CANNOT PROVE

- **It cannot say the sweep is safe to leave on.** It is armed here for one workload; `w276`'s
  cost/coverage measurements were taken on a different arm.
- **It cannot say operand publication is sufficient** — the semaphore write-back and the guest's
  own `GP_GET` advance are still untested behind it.
- **The completion plane still has no oracle.**
- One workload, one chip (GA106), one driver (`580.159.04`), one boot.
