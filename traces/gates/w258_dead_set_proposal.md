# w258 — the orphan gate's four defects, fixed; and a PROPOSAL for the 164

⊘ **Nothing was deleted by this rung.** §3 is a proposal and it recommends **against** the bulk
deletion it was asked to scope. Read-only analysis plus builds; no boot, no GPU, `CE-SUBMIT` 0.

Base revision `d55187a` (the w256 sweep). Gates at this rung, **inner exit codes asserted**:
`cargo clippy --workspace --all-targets -- -D warnings` **RC=0** (0 errors);
`cargo clippy --workspace --all-targets --features kayfabe-qemu-raw/host-isolates -- -D warnings`
**RC=0**; `cargo fmt --all --check` **RC=0**.

---

## 1. The four defects are fixed, and D1 is verified closed by the test that exposed it

Each defect was **re-measured at w258 before it was fixed** — all four reproduced exactly.

| # | defect | fix | verification |
|---|---|---|---|
| **D1** | `crates/kayfabe-abi/gen/` detaches from the workspace, so `cargo check --workspace` never compiles it; 14 verdicts were vacuous and landed in `INTERNAL_CALLER`, the bucket that reads as healthy | every candidate is mapped to the manifest that actually compiles it; **every scope is proven live by garbage injection before it is trusted**; a scope that swallows garbage yields `UNADJUDICATED`, never `ORPHAN`, and the gate exits **4** | see §2 |
| **D2** | `trap restore_all EXIT INT TERM` restored and then **resumed the loop** | `trap restore_all EXIT` + `trap 'restore_all; exit 130' INT TERM` | SIGTERM to the gate mid-run: process **stopped**, **zero** `.orphan_gate_bak` left, tree clean |
| **D3** | three non-default features were outside the quantifier | adjudication is the **conjunction over 4 axes**, short-circuited on first failure | full 4-axis run on `kayfabe-util` below |
| **D4** | the regex missed 130 public verbs — ⊘ **but its obvious fix is WRONG**, see §1.1 | enumerate `pub const fn`; enumerate `extern` but classify `#[no_mangle]`/`#[export_name]` as **FFI_EXPORT**, never `ORPHAN` | enumeration diff below |

**Enumeration, same tree, old regex vs new: exactly +148 = 128 `pub const fn` + 20 `extern`, with
ZERO unexplained additions.** (1734 → 1882.)

⊘ **The sweep's own 1725 is CORRECT and was not a fifth defect.** Running the *old* regex against
`a517402`'s actual content (`git archive a517402 crates`) returns **1725** exactly, and the five
scoped runs sum to 276+458+281+177+533 = 1725. The 1725→1734 gap is tree drift between `a517402`
and `d55187a`, nothing more. ★ Recorded because "the number moved" is the shape that invites an
invented defect.

### 1.1 ⊘⊘ D4's stated fix would have INTRODUCED the defect the gate exists to prevent

`[measured w258]` all **20** `extern` verbs in the tree are in
`crates/kayfabe-qemu-raw/src/shim_unsafe.rs` and **all 20 carry `#[unsafe(no_mangle)]`** — they are
the C ABI entry points QEMU's shim calls. Making one `pub(crate)` still compiles:

```
#[unsafe(no_mangle)] pub(crate) extern "C" fn kayfabe_shim_abi_version()   → cargo check RC=0
```

`kayfabe_shim_abi_version` (`shim_unsafe.rs:725`) is the shim's **very first handshake call**.
⇒ adding `extern` to the regex without the FFI bucket would have reported **20 live C entry points
as orphans**. That is `MapGuestRam` again — a caller that text *and now rustc* cannot see, because
it is in another language.

## 2. D1, verified closed by the same garbage-injection test that exposed it

The self-test is no longer a one-off; it runs on every invocation. Simulating the **old** scope
mapping (force `gen/` to adjudicate via `--workspace`) reproduces the defect and the gate now
catches it:

```
== self-test: garbage injection per scope (1 scope/s)
   ⊘⊘ VACUOUS  scope=workspace  witness=crates/kayfabe-abi/gen/src/ctype.rs — GARBAGE COMPILED.
★ UNADJUDICATED: 14 candidate/s in a VACUOUS scope — NOT orphans, NOT clean (D1)
★ EXIT 4: at least one scope could not adjudicate. The numbers above are INCOMPLETE.     (RC=4)
```

With the **correct** mapping the same scope is proven live and the 14 get real verdicts:

```
   ✓ live     scope=/workspace/.../crates/kayfabe-abi/gen/Cargo.toml
```

⊘ **The fix is NOT "attach `gen/` to the workspace".** The detachment is deliberate and documented
at `gen/Cargo.toml:28` — a broken offline ABI generator must never break a customer's
`cargo build`. The gate had to learn to ask the right compiler, not the crate to change shape.

**Regression check — the rewrite changes soundness, not verdicts.** Full 4-axis run over
`crates/kayfabe-util`: **11 of 37**, and the 11 are the *same symbols at the same lines in the same
order* as the w256 sweep's 11 rows for that crate. RC=0, tree clean.

## 3. PROPOSAL for the 164 `NO_CALLER_ANYWHERE` — ⊘ recommend NOT deleting in bulk

★★ **"164 genuinely dead" overstates the actionable set by roughly half**, and the reason is
structural rather than a miscount.

| slice | n | why it is not a deletion candidate |
|---|---|---|
| `kayfabe-fwd`, `kayfabe-rt` | **11** | out of scope — another lane is live there |
| `kayfabe-abi`, `kayfabe-linux-raw`, `kayfabe-mocks` | **76** | **deliberate completeness**, see below |
| — of which under `src/generated/` | *10* | deleting is meaningless: the generator re-emits them |
| remainder, in scope | **77** | of which the large majority are unconsumed accessors |

- **`kayfabe-abi` (26)** is an **ABI mirror**. It exists to carry NVIDIA's message surface whether
  or not today's code sends every message; `encode_into`/`decode`/`len_bytes` on
  `generated/{classes,ctrl,rpc}.rs` are the surface, not residue. Deleting the unused half makes
  the mirror lie about the ABI.
- **`kayfabe-linux-raw` (45)** is the **OS portability seam** (`docs/design/portability_arm64.md`,
  `vmm_portability_seam_audit.md`). `ioctl.rs`'s `read`/`write`/`none` are the three direction
  constructors of the ioctl encoding; deleting whichever two are unused today leaves an asymmetric
  API and a trap for the next caller.
- **`kayfabe-mocks` (5)** are test doubles — "no production caller" is their definition.

⇒ ⊘ **For these three crates, `NO_CALLER_ANYWHERE` is the DESIGNED state, not a defect.** A gate
that cannot say so will report them forever and train readers to ignore it.

### 3.1 What the remaining 77 actually are

Top names: `is_empty` ×10, `len_bytes` ×6, `decode` ×6, `new` ×5, `encode_into` ×5, `why` ×4,
`len` ×4. ⇒ **unconsumed accessors, i.e. tidying, not architecture.** This matches the brief's own
assessment and it is **low value**: each removal is a few lines, none unblocks anything, and each
one is a chance to delete something a test or a future call site wanted.

⚠ **I could not substantiate a "refuted experiment" set.** The non-accessor names were traced with
`git log -S` and the ones checked (`waits_for_fb_pull` `fb35691`, `published_walk_trace`
`9a446e9`/`57bd756`/`dcd096c`, `batch_outstanding` `cc85ea5`, `refusal_census` `c93930d`,
`attribution_note` `aabd389`) are **instruments and served features**, not refuted hypotheses.
⊘ Per this tree's rule, absence of evidence is not refutation: **they stay.**

### 3.2 Recommended action — the sweep's own recommendation, unchanged

1. **Report three numbers, never their union.** The union is what made "1033 orphans" read as a
   backlog when 76 of the dead alone are by design.
2. **Enforce `NO_CALLER_ANYWHERE` as a RATCHET off a committed baseline** — green on day one by
   construction, so the gate never goes red on day two and gets disabled.
3. **Exempt the ABI mirror, the OS seam and the mocks by manifest**, with the reason recorded, so
   the designed state stops being reported as a finding.
4. **Re-file `INTERNAL_CALLER` (498) as visibility hygiene**, out of the orphan report entirely.
5. ⊘ **Do not schedule an accessor-deletion pass.** If it happens at all it should ride along with
   work already touching those files.

## 4. Route B (`KAYFABE_RING_VIDMEM`) — ⊘ its premise did NOT lapse

A standing summary says route B "exists to remove a refusal whose count is now 0".
**`traces/boots/w246/README.md` refutes that.** Corner **C** (witness on, route B **off**) reads
`PushbufferAperture` **8**; corner **D** (witness on, route B **on**) reads **0**. One variable.
⇒ **the count is 0 BECAUSE route B is on.** Removing the code restores the 8.

★ Reading a mechanism's own output as evidence the mechanism is unnecessary is the same shape as
*"A DIAGNOSTIC gated on the failure"*. What actually lapsed is the **citation** on
`RING_VIDMEM_ENV`'s doc comment: it cited `[w237]`, predating the four-corner square, so it never
recorded that route B has **fired** (64 KiB read, `spans=0` the *correct* decode of a
semaphore-release-only `LAUNCH_DMA`) nor that it is **unreachable unless `KAYFABE_PT_WITNESS_EXEC`
is armed**. Both are now folded into that comment, above the text they correct. The code is
unchanged.
