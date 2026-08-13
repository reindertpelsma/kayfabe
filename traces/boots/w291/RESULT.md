# w291 STEP 1 — THE RELAXATIONS ARE LOAD-BEARING. ⊘ NOT A SHIPPING RESULT.

Two boots at `bee436b6`, stamp gate PASS on both, **same binary, same client md5**, differing
only in `KAYFABE_PT_SWEEP` / `KAYFABE_OPERAND_JOIN`. `KAYFABE_VAS_PUBLISH=off` on **both** —
leg 8 is a third relaxation and arming it would have made this question unanswerable.

## ⊘⊘⊘ LEAD: THE ANSWER TO THE SHIPPING QUESTION IS **NO**

The brief flagged *"if arm 1 passes with no relaxations, that is a SHIPPING result."* **It does
not pass.** Stated plainly so it cannot be read the hopeful way.

## `met_the_whole_bar()` BY ITS FOUR FACTS — never a pass/fail word

| fact | `relaxed` (`PT_SWEEP=on`, `OPERAND_JOIN=join`) | `bare` (both unset) |
|---|---|---|
| 1 — bytes moved | **`4096 bytes moved`** | ⊘ **none** (no `bytes moved` clause at all) |
| 2 — dst first/last | `dst[0] 0x3f0011cc -> 0xc0ffee33`, `dst[last] 0xc0fff232` (want `0xc0fff232`) | `dst[0] 0x3f0011cc -> 0x3f0011cc` (want `0xc0ffee33`), `dst[last] 0x3f0011cc` (want `0xc0fff232`) — **unchanged, still the sentinel** |
| 3 — semaphore vs declared | `engine semaphore 0x00000001 (declared 0x00000001)` | `semaphore 0x00000000 (want 0x00000001)` — **never released** |
| 4 — `GP_GET` vs `GP_PUT` | `GP_GET 1` = `GP_PUT 1` | `GP_GET 1` = `GP_PUT 1` — ★ **holds on BOTH arms** |
| line kind | **`★`** (all four) | **`FAIL`** |

★★ **Fact 4 holds on the failing arm too**, and that is the whole value of reporting four
numbers rather than a word: the entry **was** fetched. The client's own named diagnosis says so
— *"the entry WAS fetched and the methods did nothing: SET_OBJECT class, subchannel, or an
operand that does not resolve."*

## THE DELTA IS ATTRIBUTABLE BY IDENTITY, NOT BY COUNT

| arm | host `Xid`s |
|---|---|
| `bare` | `CE0 HUBCLIENT_CE1 @ **0x1_20000000** FAULT_PTE READ` ch `0x6` — **arm 1's own `src`**; and `@ 0x7_00100000` ch `0x7` |
| `relaxed` | **only** `@ 0x7_00100000` ch `0x7` |

The client prints `R33 arm 1 OPERANDS = src 0x0000000120000000 dst 0x0000000120010000`, so the
`bare` fault is **arm 1's source operand by identity**, and the `relaxed` arm does not fault
there at all. The surviving `0x7_00100000` on both arms is **arm 4's control** operand
(`R33 arm 4 control … src 0x0000000700100000`) — a different, already-known failure.
⇒ The relaxations remove exactly one fault, and it is arm 1's.

## ★★★ `R33_RC = 1` ON **BOTH** ARMS — "arm 1 passes" is NOT "the client passes"

Anchored `[R33_RC=1]`, unanchored `[R33_RC=1 R33_RC=1]`. Unchanged by the relaxations, because
other arms fail on both:

- **arm 4** — `??  the POSITIVE CONTROL did not land (sem 0x00000000, GP_GET 1 GP_PUT 1, moved
  0xdead0000 want 0x5ea1c071)`. ⊘ So arm 4 says **nothing** about whether `0x9_00000000`
  resolves. Its `CRIT1 STATE = CONTROL-NEVER-LANDED | VA-IDENTITY MEASURED = no` — **the owner's
  still-owed criterion-1 address half, still blocked, and blocked identically on both arms.**
- **arm 6** — `FAIL CALIBRATION = ring -> Some(true) (want Some(true)), free -> None (want
  Some(false))`. An oracle that cannot show both polarities cannot distinguish *"nothing is
  mapped"* from *"I cannot see mappings"*, so its rows are not measurements this run.
- **arm 5** — `PLANE A FIRED` (the driver wrote the client's own notifier: `status 0xffff,
  info32 0x0000001f`) and `PLANE C SPEAKS` (`Other(19270)`), but `PLANE D UNMEASURED` — the
  fault's **code** without its **address**.
- arms 2 and 3 are `★` on both.

⚠ The instrument's own known-positive fired first on both arms: `IN-BAND CAL = KNOWN-POSITIVE
FIRED … [Refused(86)]`, so "zero refusals" would have been a measurement rather than a blind
spot.

## ⊘ A HARNESS DEFECT CAUGHT BY ITS OWN NAMED EXIT

The first `bare` attempt exited **`95`** in 0 s — the musl client had only ever been built under
`cargo-target-w289`, and `[ -x "$CLIENT" ]` alone would happily have run a **stale binary from
another rung** had one been present. The harness now deletes and rebuilds the client too:
**no build ⇒ no file ⇒ no run**, applied to the client and not only to the shim.
