# The comment audit — what the source says that the design no longer means

**STATUS: LIVE, 2026-09-21 (w823).** Answers the owner's directive *"Scan through comments, what is
incompatible? How do I ensure the rotten docs are cleaned up so old architectures in old code is
reference, not current, no confusion?"* Six parallel lanes over all 24 crates in
`/workspace/kf-master/crates/*/src`, read against `THE_DESIGN.md` (w821).

⊘ **Comments only.** No code was judged. A rotten comment beside correct code is still rotten —
this tree's most expensive recurring failure is *a correct document that stopped being true and did
not say so*, and a comment is a document with the shortest possible distance to a reader.

## What was sampled

- Every `//!` module-doc block in ~190 files, read in full.
- Every comment line grep-driven for the §10 deleted vocabulary (isolate / IPC / sandbox / `Proc` /
  address table / join / epoch / dirty gate / `CeExecutor` / completion watch / route K-B /
  `KAYFABE_*` selectors), for `default`, for measurement signals, and for citations. **~600 hit
  sites read in context.**
- 24 citations verified against `research_clones/ogkm` (610.43.02) and `ogkm-580.159.04`.
- ⊘ Not sampled line-by-line: `qemu-raw/shim.rs` (~10k `///` lines), `rt/device.rs`,
  `device/plane.rs`, `isolate-host/rmladder.rs`, `abi/generated/*`. **Rot fractions are
  paragraph-level estimates; `file:line` rows are exact.**

---

## 1. ★★★ The five findings that should shape the plan

### 1.1 — 67 % of doc citations in comments point at SUPERSEDED docs

**1 340 `.md` citations: 895 → `docs/archive/`**, 269 → live `docs/design/`, 176 → docs that exist
only in the C repo or nowhere. `l1_concurrency.md` alone is cited **148 times**;
`execution_plane_increments.md` **102**.

★★★ **The w822 archive move did not create this. It REVEALED it.** Before the move those citations
pointed into `docs/design/` and were **indistinguishable from live ones** — a reader following one
landed in a superseded architecture with nothing marking it. Moving 190 docs converted an invisible
property into a **greppable** one.

⇒ **That is the answer to the owner's question.** Not by editing 895 comments — most are legitimate
provenance, and deleting them would delete the reasoning this tree runs on — but by making
staleness *mechanically visible* and then **ratcheting it down**:
`scripts/check_archive_citations.sh` forbids the number **rising**. A new comment citing an archived
doc as current is a new assertion of a dead architecture; an old one is history.

### 1.2 — The scrub contradiction is live in FOUR files

| says | where |
|---|---|
| the CeUtils scrub is **forged**, never forwarded, *"on every plane, under every value of `CE_EXECUTOR_ENV`"* | `fwd/lib.rs:1303`, `qemu-raw/shim.rs:21133` (citing **archived** `l1_concurrency.md §12.26`) |
| scrub and CE are **never executed on the CPU, always on the GPU** | `qemu-raw/shim.rs:21301` (w800, owner 2026-09-19), `rt/device.rs:341` (w806) |

⇒ The new design (§7/§8) sides with the **latter**. ⚠ And `rt/device.rs:9111` still states the owner
ruling as a total function `KERNEL → emulate`, which §7 also supersedes.

### 1.3 — FIVE defaults are stated BOTH WAYS inside one crate

`kayfabe-qemu-raw/src` — the class `CLAUDE.md` already names as this tree's most expensive:

| selector | one side | the other |
|---|---|---|
| `KAYFABE_FB_STORE` | `arena` (`shim.rs:16450`, `deviceview.rs:1134`) | `device` (`shim.rs:16245`, `scratchpad.rs:414`) |
| `KAYFABE_ISOLATES` | `Stillborn` (`shim.rs:20503`) | `Real` (`:20772`) — ⊘ *and* "since w760" vs "since w763" |
| `KAYFABE_VAS_OWNER` | `isolate` (`scratchpad.rs:261`) | `k` (`scratchpad.rs:339`) |
| `KAYFABE_SCRATCHPAD` | off (`lib.rs:100`, `shim.rs:15921`) | on (`scratchpad.rs:413`) |
| `CE_EXECUTOR` | `Local` (`shim.rs:21214`) | *"ABSENT IS `Host`"* (`:21259`) |
| dirty gate / PT sweep | on, *"always armed"* (`shim.rs:22055`, `:12184`) | *"default off"* (`:22982`) |

⚠ **A default stated twice is worse than a default stated nowhere**, because each statement makes
the reader stop looking. Cf. `THE NEW DESIGN WAS UNREACHABLE BY DEFAULT` — five arms silently
defaulting to superseded architectures.

### 1.4 — ★ ogkm citations are RELIABLE; doc citations are NOT

| corpus | exact | overdrawn | drifted | contradicted |
|---|---|---|---|---|
| **ogkm** (20 checked) | **17** | 2 | 1 | 0 |
| **docs** (4 checked) | **0 valid paths** | 2 misrepresent | 4 archived | — |

⇒ ★★★ **The hardware oracle is trustworthy and our own prose is not.** That is a reassuring result
in the direction that matters — the facts we cannot re-derive are the ones that held up.

⊘ The three ogkm misses are each **copy-pasted 3–6×**, which is the real lesson: a wrong citation
does not stay where it was written.

| miss | truth | copies |
|---|---|---|
| `ce_utils.c:349` = *"the shortest named guest timeout"* | `:349` is an `NV_ASSERT` in `ceutilsDestruct`; the timeout is raised in `channel_utils.c:344-365` and **discarded** at `ce_utils.c:341` | ×3 |
| `rpc.c:11085` copies reply params *"**whenever** `paramsSize != 0`"* | the copy sits under `status == NV_OK`, `COPYOUT_ON_ERROR` and non-`FINN_SERIALIZED` guards. **Conclusion survives; "whenever" does not** | ×6 |
| `kernel_graphics.c:2420` golden-ctx channel has *"zero entries"* | `gpFifoEntries = 32` (`:2152`, *"power-of-2 random choice"*). **The discriminator is the OFFSET, never the count** | ×1, **fixed w823** |

⊘ And two doc citations **misrepresent the doc they cite**: `fwd/lib.rs:5217` says §2.2 *"refused the
BAR1 route"* when §2.2 is titled *"THE LIVE ROUTE — the BAR1 trap"* and **endorses** it; and
`device/nonstall.rs:2` presents a **paraphrase as a verbatim quotation** — the sentence appears
nowhere under `/workspace/kf-master`. ⚠ Exactly the class `CLAUDE.md` names: **citing the oracle is
not the oracle being right**, and a citation gate checks a claim is *sourced*, never that the source
says what the claim says.

### 1.5 — The lock rank is numbered THREE different ways

`util/lock.rs:105-110` (`Device=2, Proc=3`) vs `util/lock.rs:343-344` (`rank 1 = per-Proc,
rank 2 = leaf`) vs `util/lockwitness.rs:37-38` (`plane=0 device=1 proc=2 leaf=3`).

⚠ And the vCPU-blocking rule is stated both as *"`inline_exceptions` must be 0, the gate is
absolute"* (`util/trapwitness.rs:62`, `rmladder.rs:9518`) and *"passthrough doorbells are inline on
the vCPU, no queue, no worker"* (`linux-raw/window_unsafe.rs:434`, `isolate/lib.rs:2913`, owner
2026-09-13). ⇒ **The design sides with the latter** (§48.1): the question is never *"did anything run
inline"* but *"who owns the blocking"*.

---

## 2. Crate ranking — where to rewrite, where to preserve

*Rotten* = share of comment prose asserting deleted architecture or a superseded default
(paragraph-level estimate). *Keep* = measurement / gate / RM-semantics lines. *Cite* = archived-doc
citation share.

| crate | lines | rotten | keep | cite | verdict |
|---|---|---|---|---|---|
| **kayfabe-isolate** | 3 549 | **~70 %** | ~25 % | 48/75 | **IS the deleted plane.** Salvage ~25 dated rows and the ogkm lock cites |
| **kayfabe-completion** | 280 | **~70 %** | 1 row | 1/3 | delete; keep `:129-135` |
| **kayfabe-isolate-host** | ~17 100 | ~55 % | ~40 % | 81/126 | plane is rot — but ⭐ **`rm.rs` is the richest RM-semantics KEEP set in the tree** |
| **kayfabe-fwd** | 5 586 | ~50 % | ~280 rows | 60/84 | densest dated record; ⊘ **worst citation reliability** (2 of 3 flawed) |
| **kayfabe-mocks** | 1 808 | ~45 % | ~15 % | 16/27 | `MockIsolate*`, joins, *"the scrub is a no-op"* |
| **kayfabe-qemu-raw** | 17 031 | ~35–45 % | ~40 % | 104/187 | every module doc is old-plane; **all five default contradictions live here** |
| **kayfabe-rt** | 6 838 | ~35–45 % | ~5 % | 45/69 | every `//!` built on Proc / isolate / CPU-CE / watch |
| **kayfabe-mmu** | 4 311 | ~35–40 % | ~4 % | 42/48 | ★ `walkdiff`/`walkreport`/`walkshadow`/`refresh` describe **the walker the design KEEPS** |
| **kayfabe-core** | 8 485 | ~33 % | ~4 % | 87/114 | `gpa.rs` wholly deleted; `rmgraph`/`project`/`fault` survive |
| **kayfabe-linux-raw** | 6 833 | ~30 % | **~60 %** | 24/64 | rot = the sandbox/spawn/scm halves; ★ `cache`/`memtype`/`kvm`/`mapping` measurements are **gold** |
| kayfabe-util | 1 863 | ~20 % | ~60 % | 23/34 | lock/trap census (w447–w524) keeps; three-way rank contradiction |
| kayfabe-device | 14 079 | ~15–20 % | **~560 rows, densest** | 93/128 | rot concentrated in `fbwin`/`plane`/`gpgaview`/`twoworlds`/`doorbell` |
| kayfabe-cuda | 1 054 | ~5 % | ~50 rows | **1/15** | only the *"inside the scratchpad isolate"* framing is stale |
| kayfabe-rmrpc | 3 911 | **~1.2 %** | ~110 rows | 40/45 | `lib.rs` sound |
| kayfabe-arch | 2 109 | ~0.7 % | ~30 rows | 29/42 | |
| **kayfabe-abi** | 18 820 | **<0.5 %** | **~330 rows** | 101/137 | ★ **the cleanest large crate, and the hardware oracle.** Surgical sentence edits only |
| kayfabe-gsp | 3 099 | ~0.4 % | ~40 rows | 16/26 | ⚠ but **the rot IS its stated defaults** (`EchoOk`, `defer_commands` off, *"reproduce the C"*) |
| kayfabe-chips | 2 373 | **<0.4 %** | ~35 rows | 19/21 | cleanest |

⊘ **"Clean of architecture" is not "clean of stale pointers."** Four of the seven cleanest crates
have **≥85 %** of their doc citations pointing at archived docs — `kayfabe-trace` and
`kayfabe-crec` are at **12/12** and **7/7**.

---

## 3. ~50 contradiction pairs — the tree answers the same question both ways

The full table is in the audit appendix. The ones that change work:

| the question | A | B |
|---|---|---|
| is the scrub forged or executed? | `fwd/lib.rs:1303` forged | `shim.rs:21301` GPU-executed ✔ |
| are BAR1/BAR2 traps served? | `device/plane.rs:1615` default `Serve` | owner 09-11: *"no traps in bar1/bar2 at all, ever"* ✔ |
| one FB store or two worlds? | `device/twoworlds.rs:3-7` two, strict | `:13-21` *"one GPGA store, one RM object"* ✔ — ⊘ the file **contradicts itself** |
| is `SET_PAGE_DIRECTORY` ever seen? | `gvaspub.rs:9-22` measured **zero** | `setpagedir.rs:120` **2 ACCEPTED, physAddress 0x201000** |
| are the invalidate transports absent? | `ceresolve.rs:41` both absent | `mmuinval.rs` 377 triggers/boot; `[w784] 530` — ⚠ *"a zero with no known-positive is not a measurement"* |
| is the granule 64 KiB or 4 KiB? | `rt/device.rs:3862` 64 KiB | `fbwin.rs:1680` 4 KiB since w392q — **both spellings live** |
| does `NoMemory` mean success or capacity? | `rm.rs:404` already-mapped ⇒ success | `rm.rs:7226` fragmented card ⇒ capacity — **same variant, opposite readings** |
| did the PTX walk land in place? | `rm.rs:10455` *"IT IS NOT"* | `cudawalk.rs:331` *"THE PTX WALKS ALL OF GPGA, IN PLACE"* ✔ |
| is there a C oracle for completions? | `osevent.rs:323` *"NO C oracle"* | `cpuintr.rs:89` cites the C **as** the oracle — ⊘ the CE-vs-GR overdraw from `CLAUDE.md`, reproduced in-tree |
| is the hold ceiling 40 ms or 1 ms? | `mmuinval.rs:115` 40 ms | `:147` owner 09-11: the budget *"encoded the OLD design"*, **1 ms** ✔ |

★★★ **Three of these are a file disagreeing with ITSELF** (`twoworlds.rs`, `pubqueue.rs`,
`cudawalk.rs`) — which is the strongest possible argument that this is a *documentation-process*
failure, not a knowledge failure. Nobody was confused; the file was appended to and the earlier
paragraph was left standing.

---

## 4. What to do, in order

1. ⭐ **Preserve Table C before touching anything.** ~1 500 dated measurement rows are the one thing a
   rewrite cannot regenerate. The `wNNN` provenance tags (~600 more) are recoverable with
   `grep -nE '\bw[0-9]{3}[a-z]?\b'` — **carry them as provenance, never as measurements.**
2. **Fix the five two-sided defaults first.** They are cheap, they are in one crate, and each one is
   a live trap for the next reader.
3. **Fix the three copy-pasted ogkm misses** — a wrong citation propagates.
4. **Rewrite module docs, not line comments**, in `isolate`, `completion`, `rt`, `fwd`, `core`,
   `mmu`: a `//!` header is what a reader takes as the crate's current architecture.
5. ⊘ **Leave the archived-doc citations alone** except where the comment asserts the archived claim
   as *current*. The ratchet keeps the number from rising; the provenance is worth keeping.

⚠ **Two per-die literal comparisons** to watch against `DERIVE PER DIE, MAINTAIN PER FAMILY`:
`chips/gb20x.rs:49-50` + `chips/lib.rs:57-58` hand-state the Ampere refusal mask `!0x007F_0FFF`
that Blackwell must override, and `rmrpc/policy.rs:1868` states the engine set as hand-supplied
per chip.
