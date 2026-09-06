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

## §5 THE BOOT — `w380llm`, real GA106, 2026-09-06, HEAD `e01603f`, arm `alias`

**Pre-registered before the run**, in the brief that commissioned it: `SUPERSEDED → 0`,
`⊘ SUPERSEDE CAPPED → 0`, `Xid 31 → 0`, coverage for `proc=3 pdb=0x201000` not regressed, and
`LLM_TOKENS` reported whatever it is.

| | `w376llmd` (supersede) | `w380llm` (alias) |
|---|---|---|
| `SUPERSEDED` | **127** | **0** |
| `⊘ SUPERSEDE CAPPED` | **28 108** | **0** |
| host `Xid` (any) | **1** (`31 FAULT_PDE @ 0x7480_27604000`) | **0** — watermark 4 → 4, **zero new host dmesg lines** |
| `proc=3 pdb=0x201000` | `total=18539 already_host=1226 already_pinned=17300 candidates=7 refused=6` | `total=17436 already_host=9185 already_pinned=8245 candidates=0 refused=0` |
| `LLM_TOKENS` | `0`, outcome **(B)**, `LLM_EXC=CUBLAS_STATUS_NOT_SUPPORTED` | **ABSENT**, outcome **(D) ⊘ UNMEASURED** |

★ **The fix fired on the real workload.** Eight frames — `0x1e00000`, `0x2000000`, `0x2200000`,
`0x2400000`, `0x2600000`, `0x2800000`, `0x2a00000`, `0x2c00000`, the same set `w377` §9 named —
were each aliased at a second VA, all eight `placed_as_asked=true`, each with **its own**
`OS_DESCRIPTOR` (`0xcafe22ac … 0xcafe22b3`) over **one** `memfd`. `THE INSTALL REFUSED` = 0,
`FRAME-NOT-OURS` = 0, `ALIAS BIND REFUSED` = 0. Distilled evidence:
`traces/guest_boots/w380llm_e01603f_alias_evidence.log`.

⊘ **`LLM_TOKENS` is UNMEASURED and that is not a failure value.** The runner loaded 290/290
shards, reported `TORCH_CUDA_AVAILABLE=True TORCH_DEV_COUNT=1`, and was killed by the hook's own
`timeout 600` with no `LLM_TOKENS=` line. Doorbells were still being served **at 15:21:21**,
seconds before the kill, so it was grinding rather than wedged. ⚠ The `CUBLAS_STATUS_NOT_SUPPORTED`
that ended `w376llmd` **did not occur**.

## §6 ★★★★★ THE cuBLAS ERROR WAS A POISONED CONTEXT — `w380llm2`, the discriminator

`w382`'s hook runs a **4×4 fp32 matmul before the model loads**, with the host Xid count read
either side of it:

```
HOST_XID_BEFORE=1        (the pre-existing w379 rmladder Xid, not this boot's)
PROP_CAPABILITY=8.6  PROP_multi_processor_count=28  PROP_L2_cache_size=2359296
MINMM_OK=1  MINMM_SUM=64
HOST_XID_AFTER_MINMM=1
```

`64` is un-forgeable — a 4×4 of ones squared has every element 4. ⇒ **(S): cuBLAS works, the
device properties are right, and `w376`'s `CUBLAS_STATUS_NOT_SUPPORTED` was the sticky error of
a context the `Xid 31` had already poisoned.** The fix removes the Xid, so it covers that.

⊘ `PROP_name` is empty — the known `GPU_GET_NAME_STRING` zero-bytes defect, unrelated and not
what cuBLASLt selects on.

⊘⊘ **`LLM_TOKENS_GRADE=0` in that boot's probe log is the INSTRUMENT, not the measurement.**
`llm_hook2.sh` printed `${TOKENS:-0}`, so *"the runner produced no `LLM_TOKENS=` line"* rendered
as the failure value `0` — the exact conflation `llm_hook.sh`'s own pre-registration forbids in
words (*"(D) ⊘ THE MEASUREMENT DID NOT HAPPEN. It is NOT 0."*). Corrected to `ABSENT` in the
same commit as this section. **The true reading of both boots is UNMEASURED.**

## §6.1 ★★★ AND THE FANOUT IS FIVE, NOT TWO — `w380llm2`, 18 frames

`traces/guest_boots/w380llm2_3cacd43_alias_fanout.log`:

```
0x1e00000 … 0x2800000   -> 4 distinct alias VAs each   (six frames)
0x2a00000, 0x2c00000    -> 2 each
0x3400000 … 0x4600000   -> 1 each                      (ten frames)
38 placements, placed_as_asked=true on 38 of 38, 38 distinct host objects
SUPERSEDED=0  SUPERSEDE_CAPPED=0  INSTALL_REFUSED=0  ALIAS_BIND_REFUSED=0
FRAME_NOT_OURS=198  ORPHAN_RECLAIMED=107   (the cross-process paths, still by name)
```

⇒ **Six frames reached FIVE simultaneous GPU addresses** (one join + four aliases). ⚠ A
hard-coded `N = 2` would have refused **18 of the 38** placements — the brief's insistence that
`N` be unbounded was not caution, it was the measurement waiting to happen. `w377` §9's *"2 or
3, never more"* was a bound on **one boot's** observation, not on the guest.

⊘ `FRAME-NOT-OURS=198` and `ORPHAN-RECLAIMED=107` are the *other two* branches of §4's table
firing on a boot with several GPU-touching processes: a live peer's join is refused by name and
a join nobody names is reclaimed. Neither is an alias and neither was regressed.

## §7 ⚠ THE RESIDUAL THIS OPENS, NAMED

`PT-DECODE` refusals went **255 → 271**, and the sixteen new ones are `RepointsPublished: 8`
and `UnbindsPublished: 8` — exactly the eight aliased frames. They are the *consequence* of the
fix, not a regression in it: a row that keeps its host backing is a row `populate` will refuse
to re-point, and the supersede arm used to dissolve that case by deleting the row.

⇒ **The next question is what to do when the guest re-points an ALIASED VA to a different
frame.** `apply_settlement_as` deliberately refuses a remap of a published row (`w329b1`: doing
it revoked a live translation and broke the bandwidth workload), so the answer is not simply to
relax it. ⊘ Not measured against a fault, and it did not produce one on this boot.
