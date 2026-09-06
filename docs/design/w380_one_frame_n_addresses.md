# ★★★★★ ONE FRAMEBUFFER FRAME, N GPU ADDRESSES — the LLM wall's fix

**STATUS — 2026-09-06 — LIVE.** Built at `w380`. Supersedes the *"take the frame over"*
mechanism `w329` leg 2 introduced and `w330` made the default; that arm survives **by name
only**, as this rung's negative control. Boot result at §5.

Read with: `w377_the_llm_wall_is_our_own_refusal.md` (its correction block and §9 are the
diagnosis this builds on), `fb_join.md`, `fb_cpu_view.md` §4.

## §1 THE DEFECT, IN ONE SENTENCE

The framebuffer join store was keyed by **physical frame alone** —
`install_join(phys, region)` / `release_join(phys)` / `fb_join_installed_at(phys)` — so one
frame could be host-backed at exactly **one** GPU virtual address. The guest maps frames at
several VAs at once, so publishing either alias *revoked the other*.

`[measured w376llmd, real GA106]` **127 supersedes over 17 frames**, then **28 108
`⊘ SUPERSEDE CAPPED`** once `SUPERSEDE_CAP_PER_FRAME = 4` was reached. The `Xid 31 FAULT_PDE`
landed at `0x7480_27604000` — a VA we had unpublished ourselves — and the supersede path's own
log line predicted it verbatim:

> *"⊘ The old VA is still DESCRIBED by the guest and now resolves with no host backing — an
> engine still pointed there takes a CONTAINED fault."*

⚠ **The cap did not stop the ping-pong; it FROZE it.** That is why the fix is a key and not a
number: a bound on a symptom leaves one live VA of each pair permanently unbacked.

## §2 WHY *ALIASING* AND NOT *"OBSERVE THE UNMAP"* — two independent measurements

Two models fitted the data and they select different fixes: either the guest genuinely holds
both VAs live (⇒ support aliasing), or one is stale in our decode and we never learned to drop
it (⇒ observe the unmap).

- **Our own decode, `w377` §9.** A supersede **TARGET later becomes a SOURCE**, repeatedly and
  stably — frame `0x2000000` alternates between two VAs *eight* times. **A stale row we merely
  failed to drop can never be re-declared**: staleness is monotone. Only a guest holding both
  VAs mapped produces an alternating pair. ⇒ model (a) confirmed.
- **The driver itself, bare metal, 5/5 at two revisions**
  (`traces/real_ga106/w379_mapping_plane_real_ga106.txt`): RM maps ONE allocation at TWO GPU
  VAs; both stay live; **unmapping VA_A leaves VA_B live**. ⇒ keying host backing by frame
  alone was **strictly weaker than the driver we stand in for**, and the release rule is
  *last VA out*, not first.

⊘ The first is our decode and therefore not an independent observer of guest intent — what
carries it is the **shape** (alternation). The second is independent and is about hardware and
ogkm, not about our port. Neither alone would have settled it.

## §3 THE MECHANISM

A new verb, `RmBackend::alias_fb_leaf(vas, len, at, phys)`, and a new plan/reply pair
(`VerbPlan::AliasFbLeaf` → `VerbReply::FbLeafAliased`), selected by a new
`FbLeafBacking::Aliased`.

```
join  (first VA)  : mint memfd → mmap in isolate → OS_DESCRIPTOR → map_dma_both @ VA₀
                    → hand the backing up → VMM mmaps it → FbStore::install_join(phys)
alias (VA₁ … VAₙ) : token_for(phys,len) → lend the SAME memfd → a fresh mmap of it
                    → OS_DESCRIPTOR → map_dma_both @ VAᵢ         (no backing crosses)
```

### ★★★ ONE MEMORY, N DESCRIPTORS — and the asymmetry is deliberate

The **pages** are minted once. Every alias is described to RM over a second mapping of the
*same `memfd`*, so all aliases of a frame are the same bytes — which is exactly what
`BackingBytes::JoinsGuestWindow` asserts, preserved unchanged.

The **`OS_DESCRIPTOR` is per-VA**, and that is not an implementation accident. Every reclaim
path in this tree — `apply_settlement_as`'s `RevokeWholeJoins`, the retired-proc sweep, the
unadopted release — disposes of a host object **per address-table row, unconditionally**. One
object shared by N rows would invert that: the first row to go would free memory the survivors
still name. ⊘ A per-VA descriptor makes each row's release **local and complete**, so no
refcount has to be right for the tree to be safe — and it is what RM does (§2, *"unmapping VA_A
left VA_B live"*).

⚠ **N is unbounded.** Nothing counts aliases and nothing refuses the *k*-th one. 8 of the 17
frames already reach three VAs, and a hard-coded 2 would be `SUPERSEDE_CAP_PER_FRAME`'s mistake
one level up.

### ★★ THE RELEASE RULE: LAST VA OUT, and it governs the STORE

Before aliasing, *"this row is going"* and *"this frame is finished"* were the same fact. They
are not any more. `release_revoked_joins` and the retired sweep now ask
`SharedDevice::fb_join_namers(phys)` first: while any row still names the frame, the **store's
join is kept** and only this row's own host object is unmapped and freed. Giving the join back
early puts the guest's framebuffer window on fabricated pages while the engine still reads the
`memfd` — `w228`'s two memories, silently, in both directions.

### ⊘ THE REFUSAL THAT MAKES THE SELECTION SAFE

The two mistakes are not symmetric, and the code is built around that:

| mistake | consequence | how it is handled |
|---|---|---|
| `Joined` where `Aliased` was wanted | a **second memory** for one frame — `w228`, self-concealing | made unrepresentable: the STORE is asked first (`fb_join_installed_at`), and it is the only authority on whether a frame has pages |
| `Aliased` where `Joined` was wanted | nothing is allocated | left loud: `FB_ALIAS_NO_JOIN`, refused by name, never a fallback to minting |

## §4 WHAT THE SHELL ASKS, AND IN WHICH ORDER

`join_one_fb_leaf` step 0, on a frame the store already serves out of joined pages:

| this VAS names it | who owns the pages | action |
|---|---|---|
| yes | us | **ALIAS** |
| no, nobody does | nobody | reclaim the orphan (`w366`), then join fresh |
| no, a live peer does | another proc | refuse **by name** — a peer's backing is never taken |

The cheap per-VAS question (`SharedDevice::fb_join_va_in_vas`) is asked **before** the
device-wide namer census, which is O(procs × VASes × rows) and which `w364` measured costing
the GPU when it ran on every refusal.

## §5 THE BOOT

*(filled in below by the run itself)*
