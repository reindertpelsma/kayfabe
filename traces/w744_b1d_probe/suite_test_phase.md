# w744 — workspace suite ⊘⊘⊘ CORRECTED: I REPORTED A NUMBER READ MID-FLIGHT

## ⊘⊘⊘ THE FIRST VERSION OF THIS FILE WAS WRONG, AND THE ERROR WAS MINE, NOT THE SUITE'S

It recorded `RESULT_LINES=86  FAILED_TARGETS=0  FAILED_TESTS=0` and I reported that as the
result — twice, once in a report and once in commit `baaef113`. **The suite had not
finished.** 86 was a count taken while `cargo test` was still running, and the run's own
terminator had not been written. The real answer:

    RESULT_LINES=127   FAILED_TARGETS=1   SUITE_EXIT_STATUS=101

★ This is the campaign's own most-repeated lesson, committed by me while quoting it: a
partial artefact reads as a complete one, and **only the terminator distinguishes "finished"
from "not yet"**. I had the terminator check — `grep -q SUITE_EXIT_STATUS` — in the waiter,
and then answered from a *different*, un-gated count because the waiter was slow. ⊘ The
instrument was right; I went around it.

## ★★★★★ AND THE ONE FAILURE IS A REAL REGRESSION FROM THIS BRANCH — a SECURITY invariant

`kayfabe-isolate-host --test own_client_invariant`:

    every_rm_escape_in_rm_rs_stamps_the_isolates_own_client
    ★★★ F11 VIOLATED — an RM escape in rm.rs names a client that is not this isolate's
        own minted `OwnClient` (4 site(s)):
      rm.rs (code line 1141): `src_client: u32`
      rm.rs (code line 1150): `h_client_src: src_client`
      rm.rs (code line 2048): `src_client: u32`
      rm.rs (code line 2079): `src_client: u32`

**Attribution is exact, not inferred:** `git show 1221d6ae:…/rm.rs | grep -cE
"src_client|h_client_src"` → **0**. At HEAD → **8**. The test was green at the base of this
branch and **my probe turned it red.**

### ⇒ AND THE TEST'S OWN DOCS PREDICTED THIS EXACT CHANGE, BY NAME

> *"Adding `h_client_src` to an ABI struct and then using it in `rm.rs` turns this gate red
> without anyone remembering it exists."*

It is not a nuisance and it is not a false positive. Read F11's safety argument:

> *"That widening is **latent and not live** for exactly one reason: **we have no way to name
> a client we did not mint.**"*

`NVOS55_PARAMETERS::hClientSrc` **is, by definition, a client we did not mint** — that is what
`DupObject` means. ⇒ **The (d) design requires the first verb in this codebase that names a
foreign RM client, and F11 says precisely that capability is what keeps the euid widening
latent rather than live.** On a root VMM the isolate's kernel-visible euid is 0 and RM's
cross-user check is an OR on euid (`ogkm-580: os.c:3844-3868`), so a client handle we can name
is one we would be *permitted to drive*.

## ⊘ NOT SILENCED, AND DELIBERATELY LEFT RED

- ⊘ **The allowlist was not widened.** The brief's standing rule is that a constraint is never
  relaxed to pass, and this is a security gate, not a style rule.
- ⊘ **The probe verbs were not moved out of `rm.rs` either.** That would make the gate go
  green by shrinking the universe it quantifies over — the `gates_quantified_over_a_list`
  failure this very file's docs name as a standing lesson. Dodging a gate is worse than
  failing it, because the next reader sees green.

⇒ **This is a THIRD owner-level question stacked on (d)**, alongside the `ForeignHandle` gate
and `RING_NOT_A_JOINED_WINDOW`: *may an isolate name a foreign RM client at all, and if so,
what narrows `hClientSrc` to the one VA-space handle the scratchpad is entitled to dup?*
A plausible shape — **not built, not chosen here** — is an `OwnClient`-like newtype for
*"a client id handed to us by the VMM for a proc we are servicing"*, so the gate's universe
keeps its meaning and the approved forms grow by a type rather than by a string.

## What the suite says about the REST of the change

126 of 127 targets pass. The failure is one source-text gate over `rm.rs`, with the cause
named above. ⊘ And the brief's stated baseline — *"11 failing targets / 30 failing tests in
this environment"* — is **not** what this tree produces: base `1221d6ae` is green apart from
nothing, and HEAD fails exactly one target, mine. The stated baseline and this tree disagree;
recorded, not reconciled.
