# The remaining boot surface, and two traps that would otherwise be rediscovered

Facts derived in conversation on 2026-08-07/08 that live nowhere else. ⊘ Each was a
measurement or an owner ruling; none is a plan.

## 1. `[measured]` How much RPC boot surface is left

Counted from `traces/real_ga106/rpc_transcript_real_ga106.txt` (a **real** driver on a **real**
GA106):

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
  empty rows**. Real hardware answers **20480**; the captured row says 0, and RM DMAs CE fault
  records into a buffer of exactly that size. ⊘ Use the measured value, never the row.

### ⚠ Two caveats that bound what this number means

⊘ **25 is a lower bound on work, not an upper bound on effort.** The transcript counts *RPC
control calls only*. It does **not** count the scrubber's real CE copy (execution) or event
delivery for notifier 35 (interrupts). Those two are the expensive items and they sit outside
this list entirely.

⊘ **Never verified whether the 88 entries end at adapter-init success or are truncated.**
Flagged three times, still open. It is the cheapest unanswered question in the project and it
decides whether "25 remaining" means anything at all. Check it before quoting the number.

## 2. ⚠ The trap that will corrupt the first Rust perf measurement

`[measured, C artifact]` Mode-2 reached **49.9 t/s vs 47.5 t/s bare metal** — parity. But that
was the *end of a campaign*, not a property, and the record is specific about where the time
went: compute and DMA overhead were ~0% almost immediately; the tax was **all control-path**.
Perf root was a `0x110094` **poll vmexit storm**; the wins were `m2opaque`/GPGA-index plus
memslot-backing the read-mostly pages and trapping only the observe-write ones.

★ So the Rust's perf work is **pre-diagnosed** — the enemy is control-path chatter, not data
movement. That is weeks we do not spend, in the same way the C's compute bugs are weeks we do
not spend.

⊘ **AND THE TRAP:** the C's own finding was that *"the gap was 100% nesting."* Our bench guest
runs **inside a vast VM**, so any benchmark taken there measures the nesting, not the port. The
first Rust perf figure must be bare-metal or explicitly labelled meaningless — otherwise it
reads as a regression that is not one and somebody spends a day chasing it.

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
