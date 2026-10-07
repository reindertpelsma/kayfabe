# The remaining boot surface, and two traps that would otherwise be rediscovered

Facts derived in conversation on 2026-08-07/08 that live nowhere else. ⊘ Each was a
measurement or an owner ruling; none is a plan.

## 1. How much RPC boot surface is left

`[measured 2026-08-07]` — counted from `traces/real_ga106/rpc_transcript_real_ga106.txt`, the
transcript of a **real** 580.159.04 driver on a **real** GA106 (RTX 3060, the bench box):

| | |
|---|---|
| total `KAYFABE-RPC` entries in the capture | **88** |
| `cmd=0xa06f0104` (the bind) | line **63** |
| entries after the bind | **25** |

⇒ at the bind, the guest is roughly **72% through the real driver's RPC boot sequence**.

The 15 distinct commands after the bind, in order of first appearance:

```
0xa06f0103  0x20800301  0x90f10106  0x20802a08  0x2080012b  0x2080013f
0x402c0101  0x00730107  0x0073028b  0x00730211  0x20800a70  0x20800a6c
0x00730151  0x20800a38  0x20800afe
```

Triage:
- **Already served** — `0xa06f0103` (schedule), `0x20800a6c` (L2 evict), `0x20800a70`.
- **Already in the unserviced list** — `0x20800a38`, `0x20800afe`, `0x20800a70`. Known rows.
- ★ **`0x90f10106` is the current blocker.** It is the CeUtils **page-directory root**;
  unclaimed by `ObjectPolicy`, `PageDirNotModelled` in the ABI. E10e's first step.
  ★★ Note the differential *predicted* this before we hit it — the list is not just a
  checklist, it named the wall.
- **`0x0073…` family** (`0107`, `028b`, `0211`, `0151`) — four commands on a class this port has
  never touched. Confirmed **downstream of the CE wall**, not a defect.
- **`0x402c0101`** — `0x402c` is the class the probe boot showed a failing `GspRmAlloc` for.
- ⚠ **`0x20802a08`** is `CE_GET_FAULT_METHOD_BUFFER_SIZE`, one of the C oracle's **contradicted
  empty rows**. A real GA106 answers **20480** (`[measured 2026-08-01]`, task #157, recorded with
  a real body by task #178); the captured row says 0, and RM DMAs CE fault records into a buffer
  of exactly that size. ⊘ Use the hardware answer, never the row.

### ⚠ Two caveats that bound what this number means

⊘ **25 is a lower bound on work, not an upper bound on effort.** The transcript counts *RPC
control calls only*. It does **not** count the scrubber's real CE copy (execution) or event
delivery for notifier 35 (interrupts). Those two are the expensive items and they sit outside
this list entirely.

✓ **ANSWERED 2026-08-08 — the capture is COMPLETE, so "25 remaining" is a real number.**
(This was flagged as open five times. It needed one set-difference, and both halves of the
answer were already committed.)

★ **It is a derivation over two committed artefacts, not a hardware run** — labelled that way
deliberately, since nothing was executed on a machine and a marker claiming otherwise would be
the exact defect this file's §3 criticises. Anyone can re-run it: take the distinct `cmd=` words of
`traces/real_ga106/rpc_transcript_real_ga106.txt`, take the `ctl_<cmd>` row names of
`C: src/qemu/mode2_initctrl_ga106.h`, and difference the two sets.

The transcript's **55 distinct commands are a strict subset of the C oracle's 56-row table**:
`rows − transcript = {0x20800a4c}`, and `transcript − rows = {}`. The single missing row is the
one `traces/real_ga106/README.md` **independently** identifies as reachable only by widening the
capture with `nvidia-smi -q` — *"a client-driven query rather than an init control."*

⇒ The transcript covers the init set exactly, missing precisely the one command that is by
definition not part of init.

★★★★ **⊘⊘ AND THE INFERENCE DRAWN FROM THAT SENTENCE WAS THE WALL** (`[measured 2026-08-08]`,
boot `gis1_e6ed6bc`, `execution_plane_increments.md` §14.29). Every word of the derivation
above is right, and `0x20800a4c` `INTERNAL_GPU_GET_SMC_MODE` really is not an init control.
It is reached by `libcuda`, through `GPU_GET_INFO_V2` index `0x2a`, and refusing it returned
`cuInit(0) -> 100` for four rungs while ten other indices of the same request were answered
correctly. ⇒ *"Not reached during `RmInitAdapter`"* and *"not needed"* are different
statements, and every oracle this file reasons over was `nvidia-smi`-driven, so none of them
could distinguish them. **The single unexplained row in a set-difference is not residue — it
is the one place a shared blind spot can show through.** ★ That asymmetry is the proof: a capture cut short drops an
*arbitrary* tail, not exactly the one row its own author had already classified as post-init.
Two independent artefacts agree, and neither was written to answer this question.

## 2. ⚠ The trap that will corrupt the first Rust perf number

`[measured 2026-06-16, C artifact, bare-metal box .32, RTX 3050]` — Mode-2 ran a `qwen.gguf`
generation at **49.9 t/s** against a **47.5 t/s** host-native ceiling *on that same card*. Within
noise ⇒ **~zero forwarding overhead**. But that was the *end of a campaign*, not a property, and
the record is specific about where the time went: compute and DMA overhead were ~0% almost
immediately; the tax was **all control-path**. Perf root was a `0x110094` **poll vmexit storm**;
the wins were `m2opaque`/GPGA-index plus memslot-backing the read-mostly pages and trapping only
the observe-write ones.

★ So the Rust's perf work is **pre-diagnosed** — the enemy is control-path chatter, not data
movement. That is weeks we do not spend, in the same way the C's compute bugs are weeks we do
not spend.

⊘ **AND THE TRAP:** the same run settled that *the gap was 100% nesting* — the C's earlier vast
numbers were ~20 t/s against the same 50 t/s bare metal, a **2.5× tax that belonged entirely to
nested virt**, not to the design. Our bench guest also runs **inside a vast VM**, so any
benchmark taken there measures the nesting. The first Rust perf figure must be bare-metal or
explicitly labelled meaningless — otherwise it reads as a regression that is not one and
somebody spends a day chasing it.

★ And the corollary cuts the other way too: the `0x110094` remedy above is a **nested-deployment
fix**. On bare metal the C found nothing to fix. ⊘ So do not port that work as if it were
general — check first whether the deployment is nested.

## 3. Owner-relevant assessment of the instrument apparatus (2026-08-07)

Asked directly which piece is most overengineered. The honest answer, with evidence:

★ **The claim ledger's ENFORCEMENT** — not the idea. Separating MEASURED / INFERRED / ASSUMED is
excellent and has visibly shaped good habits. But it is implemented as a **regex classifier over
English prose with three numeric ceilings**, gating 1608 sites. It fired twice in one night on
text that was *already honest* (`"a row I transcribed rather than measured"` scored red because
the word "measured" appeared; a heading carrying `[measured]` plus a boot, box and revision
scored red because headings are their own block). Both fixes were **rewording, not measuring** —
the marginal work is prose engineering, and a ceiling creates pressure to phrase *around* it.
**Recommendation: demote to a report** — run it, print the delta, do not fail the build. Keep
the vocabulary, drop the ceilings.

★ **Runner-up: mutation testing.** 8738 mutants, self-contaminating via accumulated proptest
seeds, and its headline number was a **false green** — 100% reported while ENOSPC killed its
workers (`#181`, open). Either fix it or delete it: a broken instrument that still *answers* is
worse than an absent one, which is the same lesson the unserviced ledger taught twice.

✓ **Keep without hesitation: the bite-check requirement.** It repeatedly found gates that did not
bite — the unranked-lock scanner missing qualified paths, the two-VM test satisfied by
coincidence, `#[should_panic]` matching the wrong site. It earns its cost every time.

★ For contrast: **the census — ~40 lines of instrumentation — settled in one boot what the claim
ledger could not have told us in a year.** One measures the system; the other measures our
sentences about it.
