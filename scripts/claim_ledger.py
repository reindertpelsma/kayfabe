#!/usr/bin/env python3
"""The CLAIM LEDGER — separate what we MEASURED from what we INFERRED.

Read `docs/design/claim_ledger.md` for the doctrine. This file is the instrument.

WHAT IT DOES. It sweeps every tracked Rust source and design doc, finds every prose
block that makes an EPISTEMIC CLAIM ("measured", "observed", "verified", "confirmed",
"proven"), and asks one question of each: *does the block name what is behind the
claim?* A block that says "measured" and names a capture, a bench date, a commit or a
test is attributed. A block that says "measured" and names nothing is not, and this
script says so by file and line.

WHY IT IS A SEPARATE SWEEP AND NOT A GREP. The unit is the BLOCK, not the line. A
rustdoc paragraph routinely says "measured" on one line and names the capture three
lines later; a line-oriented grep flags that as unattributed and is ignored within a
day. Blocks are the smallest unit over which "does this carry its evidence" is a real
question.

★ THE UNIVERSE IS DERIVED, NEVER LISTED. Files come from `git ls-files`, so a crate
or a design doc added tomorrow is in scope tomorrow with no edit here. A hand-list is
a smaller true statement and weakens in silence (memory: gates_quantified_over_a_list).

★★ WHAT THIS GATE CANNOT SEE, stated so nobody reads more into a green than is there:

  1. It checks that a claim NAMES its evidence. It cannot check that the evidence
     EXISTS, that it says what the claim says it says, or that it was ever run. A
     block citing `cap9_imaginary` passes. Adjudicating a citation is human work; the
     ogkm-tag gate's 20 phantoms were found by reading, not by grepping.
  2. `MEASURED` vs `INFERRED` is decided by WHICH tokens a block carries, so a block
     that cites ogkm AND names a date is scored MEASURED even if the date belongs to
     the reading rather than to a run. Precision, not recall, is what this buys.
  3. It is blind to a claim made WITHOUT a claim word. "The driver writes 0x4 here"
     asserts a fact and matches nothing below. This gate raises the cost of the
     confident-sounding claim; it does not reach the quiet one.
  4. Code is out of scope. An `assert_eq!` encoding an unmeasured belief is invisible
     here and always will be.

USAGE
  scripts/claim_ledger.py                 # the ledger: counts by class
  scripts/claim_ledger.py --list CLASS    # every site in a class, file:line
  scripts/claim_ledger.py --gate          # CI mode: enforce the pinned bars
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# =====================================================================================
# THE VOCABULARY
# =====================================================================================

# A CLAIM WORD is a word that asserts an experiment happened. `measured` is the one the
# owner's directive names; the rest are the same assertion in different clothes.
#
# ★★★ `observed` IS NOT IN THIS LIST, AND THAT IS A FINDING RATHER THAN A CONVENIENCE.
# The directive names it, and it was in the first draft: it produced 284 Rust hits, and
# reading them showed the great majority are the DOMAIN VERB, not an epistemic claim —
# "a completion was observed for this proc", "last doorbell token observed", "Observed,
# not yet composed into any batch". In a project whose entire mechanism is observing a
# guest, `observed` is a word about the running program far more often than about us.
# Keeping it would have added several hundred sites that no reading can triage, and a
# gate whose output nobody can read is a gate that cannot fire. What the gate can SEE
# here is the strong words; `observed`-as-a-claim is left to human review, and the
# ledger says so rather than implying coverage it does not have.
#
# ★ Deliberately NOT here either: "shown", "clear", "known", "obvious". They assert just
# as much and are ordinary prose.
CLAIM_WORDS = r"measured|measurement|verified|confirmed|proven|proved|demonstrated"
CLAIM_RE = re.compile(rf"\b({CLAIM_WORDS})\b", re.IGNORECASE)

# ★★ A LOCAL PROOF IS NOT AN EMPIRICAL CLAIM, and this too was found by the gate firing
# on someone else's merge rather than by foresight. `// SAFETY: both pointers were just
# proven non-null` is a statement about the three lines above it — a deduction, complete
# where it stands, with no machine and no source involved. There is nothing for it to
# cite and demanding a citation would be noise in the one place (`unsafe` blocks) where
# noise is most expensive. These phrases are STRIPPED before a block is examined, so a
# block containing only local proofs is not a claim site at all, while a block that also
# says "measured on hardware" still is.
LOCAL_PROOF_RE = re.compile(
    r"\b(?:proven|proved|verified|confirmed)\b(?:\s+\w+){0,3}"
    r"\s+(?:above|below|here|already|just now|by the caller|at the call site)"
    r"|\b(?:just|already|therefore|thus)\s+(?:proven|proved|verified|confirmed)\b"
    r"|\bproven\s+(?:non-null|nonnull|live|in-bounds|aligned|valid|unique|disjoint)\b",
    re.IGNORECASE,
)

# --- EVIDENCE OF A RUN --------------------------------------------------------------
# Something that happened at a time, on a machine, at a revision. Any ONE of these in
# the same block is enough to say the claim names what is behind it.
RUN_EVIDENCE = [
    # the C oracle's committed captures (traces/mode2_c_reference/) and the CUDA ladder
    (r"\bcap[123]b?_\w+|\bcap1b\b|\bcup2\b|\bcup8(_iter)?\b|\bcupctx2_min\b", "capture"),
    # a date — a run happened on a day; undated "measured" is the defect this exists for
    (r"\b20\d\d-\d\d-\d\d\b", "date"),
    # a revision the claim was measured at (CLAUDE.md: any bench claim carries its rev)
    (r"\b(?:base|at|commit|rev|revision|HEAD)\s+`?[0-9a-f]{7,40}`?\b", "commit"),
    # the machine, or the driver on it
    (r"\bRTX ?\d{3,4}\b|\bGA10[0-9xX]\b|\b\d{3}\.\d{2,3}\.\d{2}\b", "hardware"),
    # a test that runs and can fail
    (r"\btests?/[\w/]+\.rs\b|\b\w+\.rs::\w+|\bcargo (test|mutants|bench)\b", "test"),
    # a named campaign with a number behind it
    (r"\bmutation (campaign|round|score)\b|\bbite[- ]check\b|\btask #\d+\b|\bissue #\d+\b", "campaign"),
]

# --- EVIDENCE OF A READING ----------------------------------------------------------
# ★★ THE DISTINCTION THIS WHOLE FILE EXISTS TO KEEP. A citation into NVIDIA's source,
# gVisor, or the C artifact tells you what the code SAYS. It is legitimate, it is
# load-bearing, and it is NOT a measurement — nothing ran. Both classes are attributed;
# they are attributed to different things and the ledger never merges them.
#
# ★ THE `[src]` MARKERS ARE THIS REPO'S OWN, NOT AN INVENTION OF THIS SCRIPT.
# `l1_concurrency.md` §… already defines "`[src]` = cited to ogkm/gvisor/the C artifact"
# and `mode2_gsp_port_plan.md` §0.1 already refines it to `[src@580]` / `[src@610]`. The
# gate reads the vocabulary that exists rather than asking authors to learn a second one.
#
# ★★ AND `[measured]` IS DELIBERATELY *NOT* EVIDENCE. The same convention defines
# `[measured]`/`[meas]` as "measured on this tree" — but a marker declares a class, it
# does not name a run, and a marker treated as evidence would be a free pass through the
# exact gate this is. A block tagged `[measured]` still has to say WHICH run. This is the
# one place the ledger deliberately disagrees with an existing convention, and it
# disagrees in the strict direction.
READ_EVIDENCE = [
    (r"\[src(@(580|610))?\]", "src-marker"),
    (r"\bogkm-(580|610):", "ogkm"),
    (r"\bnvproxy\b|\bgvisor\b", "nvproxy"),
    (r"\bC:\s*[\w/]+\.[ch]:\d|\bsrc/qemu/[\w.]+|\bnvkvm_\w+\.c\b", "c-artifact"),
    # ★★ A POINTER TO A DESIGN DOC IS THE WEAKEST ATTRIBUTED FORM, and it is scored as
    # a READING on purpose even when the block says "measured". `l1_concurrency.md
    # §12.27` says where the run is written down; it is not itself the run. This is the
    # read-at-invalidate lesson made mechanical: CLAUDE.md asserted a fact by pointing
    # at a design doc that had already refuted it. A summary is not a source, so a
    # pointer can never promote a block to MEASURED — only naming the run can.
    (r"\b\w+\.md\b|§\s?\d+(\.\d+)*", "design-doc"),
]

# --- AN EXPLICIT EPISTEMIC DOWNGRADE ------------------------------------------------
# ★ The `SET_OBJECT` form the owner named: a block that records the ABSENCE of a
# measurement rather than borrowing one. This is the correct thing to write when you
# have no evidence, and it must not be scored as an unattributed claim — otherwise the
# gate punishes exactly the honesty it is for.
HONEST_RE = re.compile(
    r"not (?:been )?(?:observed|measured|verified|confirmed)"
    r"|never (?:been )?(?:observed|measured|verified)"
    r"|\bunmeasured\b|\bunverified\b"
    # `[unverified]` — again the repo's own marker, used in host_execution_plane.md §1.4
    r"|\[unverified\]|\[unmeasured\]"
    r"|no (?:measurement|evidence|experiment|oracle)"
    r"|\bASSUMED\b|\bassumption\b|\bassumed\b"
    r"|(?:inferred|read) (?:from|off) (?:the )?source"
    # "None of them is verified …", "is not verified against …" — the SET_OBJECT form,
    # written as a negative sentence rather than as a marker. Recording the absence of a
    # measurement must never score as an unattributed claim; that would make the gate
    # punish the one behaviour it exists to encourage.
    r"|\b(?:none|neither) of (?:them|these|which)\b[^.]{0,40}\b(?:verified|measured|checked)\b"
    r"|\bis not (?:\w+ ){0,2}(?:verified|measured)\b"
    # ★ "No Ada card was measured for this number." — found as a FALSE POSITIVE the third
    # time this gate fired on another agent's merge (`kayfabe-chips/src/ad10x.rs`). The
    # site is doing exactly the right thing: it cites ogkm for the number and then states,
    # in one sentence, that no hardware backs it. A gate that scores that as a defect
    # penalises the behaviour it exists to encourage, so the pattern learned the form
    # rather than the bar absorbing the site.
    r"|\bn(?:o|either) (?:\w+ ){0,3}(?:was|were|has been|have been) (?:measured|verified|observed|tested)\b"
    r"|\bnot (?:yet )?(?:run|reproduced)\b",
    re.IGNORECASE,
)

RUN_RE = [(re.compile(p, re.IGNORECASE), n) for p, n in RUN_EVIDENCE]
READ_RE = [(re.compile(p, re.IGNORECASE), n) for p, n in READ_EVIDENCE]

# ★★★ THE STRONG WORDS. "measured / verified / proven" assert that an EXPERIMENT
# SETTLED IT. "confirmed / demonstrated" are softer and routinely used about an
# argument rather than a run, so they count as claims but do not, on their own, put a
# site in the CONFLATED class below.
STRONG_RE = re.compile(r"\b(measured|measurement|verified|proven|proved)\b", re.IGNORECASE)

# ★★★ THE HARDWARE ASSERTION — and the reason this exists is that the FIRST PLANTED BITE
# DID NOT FIRE.
#
# The bite was a rustdoc line reading "Measured on real hardware: this offset is stable
# across every driver revision and the frontend was proven never to reject it", planted
# with nothing behind it. Both ratchets stayed green and the runner exited 0. The cause is
# structural and it is the most important thing in this file: **the unit is the BLOCK**,
# so a claim planted into a paragraph that already carries a citation INHERITS that
# paragraph's attribution. The citation was for a different sub-claim. The gate could not
# see the new one at all.
#
# That block-level design is not a mistake — a line-level gate flags every rustdoc
# paragraph that says "measured" on one line and names its capture three lines later, and
# gets ignored within a day. But it means the two ratchets above are gates on the
# PARAGRAPH, and a paragraph is a place a claim can hide.
#
# So this third rule is LINE-INSENSITIVE IN THE OTHER DIRECTION: the phrase "measured on
# hardware" and its neighbours name an EXPERIMENT ON A MACHINE explicitly. Nothing else
# can be meant by them. A block containing one MUST carry a run token — a date, a capture,
# a revision, a hardware id, a test — and neither a source citation nor an honest-downgrade
# phrase elsewhere in the block buys an exemption, because neither has anything to do with
# whether the machine was switched on.
HW_ASSERTION_RE = re.compile(
    r"\b(measured|verified|proven|proved|reproduced|confirmed)\b"
    r"[^.\n]{0,30}\bon (?:real |this |the )?(?:hardware|silicon|the bench|a real|the real)"
    r"|\bhardware[- ](?:measured|verified|proven|confirmed)\b"
    r"|\bon (?:real|actual) hardware\b",
    re.IGNORECASE,
)

MEASURED, INFERRED, ASSUMED, UNATTRIBUTED = "MEASURED", "INFERRED", "ASSUMED", "UNATTRIBUTED"
# ★★★ CONFLATED — the class this whole exercise exists to name. A block that says
# something was MEASURED, VERIFIED or PROVEN, and whose only evidence is a CITATION
# INTO SOURCE (ogkm / nvproxy / the C artifact / a design-doc pointer). Reading the
# open kernel modules tells you what the driver DOES; only a live boot tells you what
# HAPPENS. A block in this class has done the first and claimed the second, and
# everything downstream inherits it as settled. This is read-at-invalidate's shape.
#
# It is a SUB-CLASSIFICATION of INFERRED, not a fifth bucket: the site is correctly
# attributed, to a reading. What is wrong is the verb.
CONFLATED = "CONFLATED"
# ★★★ BARE-HW — a block that says "measured ON HARDWARE" and names no run whatsoever.
# The narrowest and strongest of the three rules, and the only one held at a hard ZERO.
BARE_HW = "BARE-HW"
CLASSES = [MEASURED, INFERRED, ASSUMED, UNATTRIBUTED]


@dataclass
class Site:
    path: str
    line: int
    cls: str
    words: list[str]
    tags: list[str] = field(default_factory=list)
    text: str = ""
    conflated: bool = False
    bare_hw: bool = False


# =====================================================================================
# BLOCK EXTRACTION
# =====================================================================================


def rust_blocks(text: str):
    """Contiguous runs of `//`, `///`, `//!` lines. Yields (first_line_no, body).

    ★ Only line comments. `/* */` blocks are rare in this tree (checked: the workspace
    uses line comments throughout) and a half-supported form would make the count
    depend on a style choice, which is not a property a gate should have.
    """
    cur, start = [], 0
    for i, ln in enumerate(text.splitlines(), 1):
        s = ln.strip()
        if s.startswith("//"):
            if not cur:
                start = i
            cur.append(re.sub(r"^\s*//[/!]?", "", ln))
        else:
            if cur:
                yield start, "\n".join(cur)
            cur = []
    if cur:
        yield start, "\n".join(cur)


def md_blocks(text: str):
    """Markdown paragraphs, separated by blank lines. Fenced code is skipped whole —
    a code fence is a transcript, not a claim, and its identifiers produce noise."""
    cur, start, fence = [], 0, False
    for i, ln in enumerate(text.splitlines(), 1):
        if ln.lstrip().startswith("```"):
            fence = not fence
            if cur:
                yield start, "\n".join(cur)
            cur = []
            continue
        if fence:
            continue
        if ln.strip():
            if not cur:
                start = i
            cur.append(ln)
        else:
            if cur:
                yield start, "\n".join(cur)
            cur = []
    if cur:
        yield start, "\n".join(cur)


def bare_hardware_claim(body: str) -> bool:
    """A block that says "measured on hardware" and names no run at all."""
    return bool(HW_ASSERTION_RE.search(body)) and not any(r.search(body) for r, _ in RUN_RE)


def classify(body: str) -> tuple[str, list[str], bool]:
    run = [n for r, n in RUN_RE if r.search(body)]
    read = [n for r, n in READ_RE if r.search(body)]
    strong = bool(STRONG_RE.search(body))
    # ★ The honest-downgrade check runs BEFORE the conflation verdict. A block that
    # says "ogkm says X; this has NOT been measured" is doing the right thing with the
    # right words, and must not be scored as claiming a measurement it lacks.
    honest = bool(HONEST_RE.search(body))
    if run:
        return MEASURED, run + read, False
    if read:
        # ★★ CONFLATION IS ABOUT SOURCE CITATIONS, NOT DOC POINTERS. A block that says
        # "measured (`l1_concurrency.md` §12.27)" points at where the run is written
        # down; it is weak attribution, it is counted as INFERRED rather than MEASURED
        # for exactly that reason, but it is not the named failure. The named failure is
        # having READ NVIDIA'S SOURCE and said MEASURED — `ogkm`, `nvproxy`, the C
        # artifact. Only those put a site in CONFLATED, so the number stays small enough
        # to be a list somebody actually reads.
        cited_source = any(t in ("ogkm", "nvproxy", "c-artifact") for t in read)
        return INFERRED, read, strong and cited_source and not honest
    if honest:
        return ASSUMED, ["self-declared"], False
    # ★ An unattributed strong claim is NOT scored CONFLATED. Conflation is the specific
    # act of putting a reading behind a measurement word; a site with nothing behind it
    # is a different (and already counted) defect, and double-counting it would make the
    # conflated number impressive rather than true.
    return UNATTRIBUTED, [], False


def universe() -> list[Path]:
    """★ DERIVED. `git ls-files` is the list; nothing here enumerates crates or docs.

    ★★ **Tracked AND untracked-but-not-ignored**, and the second half is not a nicety.
    `git ls-files` alone lists only what is already in the index, so a run made *before*
    `git add` scores a brand-new file's prose as if it did not exist — the ledger reports
    green, the file is committed, and CI (where everything is tracked) then fails on
    claims nobody was shown. Measured on 2026-07-31 during task #127: a pre-commit run
    read 383 with three new modules on disk and unstaged; the same tree read 385 the
    moment they were added. That is this project's own *"gates quantified over a LIST"*
    failure with the list supplied by git, so the fix is to widen the list rather than to
    remember to stage first. `--exclude-standard` keeps `.gitignore` honoured, so
    `target/` and friends stay out.
    """
    out = ""
    for extra in ([], ["-o", "--exclude-standard"]):
        out += subprocess.run(
            ["git", "ls-files", "-z", *extra, "--", "*.rs", "docs/**/*.md", "*.md"],
            cwd=ROOT, capture_output=True, text=True, check=True,
        ).stdout
    files = []
    seen = set()
    for f in out.split("\0"):
        if f in seen:
            continue
        seen.add(f)
        if not f:
            continue
        p = ROOT / f
        # The ledger and this instrument quote the vocabulary in order to DEFINE it,
        # exactly as mode2_gsp_port_plan.md quotes a bare `ogkm:`. Excluded by path so
        # the charter does not fire on itself.
        if f in ("docs/design/claim_ledger.md", "scripts/claim_ledger.py"):
            continue
        if p.exists():
            files.append(p)
    return sorted(files)


def sweep() -> list[Site]:
    sites: list[Site] = []
    for p in universe():
        rel = str(p.relative_to(ROOT))
        text = p.read_text(errors="replace")
        blocks = rust_blocks(text) if p.suffix == ".rs" else md_blocks(text)
        for line, body in blocks:
            # Strip local proofs first — see LOCAL_PROOF_RE. `body` keeps its original
            # text for the excerpt; only claim DETECTION runs over the stripped copy.
            probe = LOCAL_PROOF_RE.sub(" ", body)
            words = sorted({m.group(1).lower() for m in CLAIM_RE.finditer(probe)})
            if not words:
                continue
            cls, tags, conf = classify(body)
            first = next(
                (ln.strip() for ln in body.splitlines() if STRONG_RE.search(ln)),
                next((ln.strip() for ln in body.splitlines() if CLAIM_RE.search(ln)), ""),
            )
            sites.append(
                Site(rel, line, cls, words, tags, first[:150], conf,
                     bare_hardware_claim(body))
            )
    return sites


# =====================================================================================
# THE BARS — pinned literals, as in scripts/ci_gates.sh
# =====================================================================================
#
# ★★ WHY A RATCHET AND NOT A ZERO. There are hundreds of claim sites in this tree and
# most of them are fine. A hard zero would have to be bought with a retrofit nobody can
# review, so it would be bought with `#[allow]`-shaped noise instead. A ratchet pinned
# to a number someone had to look at makes ADDING an unattributed claim a red build,
# which is the behaviour that matters, and makes REMOVING one a visible commit.
#
# ★★★ THE TWO BARS HAVE DIFFERENT POLARITY, AND THE ASYMMETRY IS THE ARGUMENT.
#
# CONFLATED is EXACT (`!=`). It is the named failure mode, the list is 65 long, and both
# directions must be pinned for the reason the ogkm doc ratchet gives about itself: a
# gate that only catches growth leaves slack a later claim can be added into with nothing
# saying so. 65 is small enough that anyone who moves it can read the whole list.
#
# UNATTRIBUTED is a CEILING (`>`). This is a weaker gate than the one above and it is
# weaker deliberately, so say so plainly rather than dress it up. 383 is a CENSUS, not a
# policy: it counts pre-existing prose across a tree that several agents write into at
# once, and an exact pin would turn an unrelated doc commit red. The predictable outcome
# of that is not better prose — it is people bumping the number without reading it, which
# is how a ratchet decays into a formality. The direction that matters (a NEW claim with
# nothing behind it) is caught either way; what a ceiling gives up is the automatic
# tightening, and the gate compensates by printing the exact line to change whenever the
# real count drops below the bar. That is a worse gate honestly described, not a better
# one described optimistically.
#
# BARE-HW is EXACT and it is the STRONGEST of the three. 17 is not zero, and the reason is
# worth stating rather than hiding: about six of the seventeen are the *legend lines that
# define the notation* — `**[measured]** on hardware, **[src]** read from code` — which say
# "measured on hardware" in order to explain what the phrase means. That is the same
# self-firing-charter problem the ogkm doc ratchet describes, and the same answer: a
# reviewed constant rather than a hand-list of exemptions, because a hand-list is a smaller
# true statement. The other eleven are real claims and each is a one-line fix by whoever
# made the run. Drive this one to zero; it is the only bar here that can reach it.
#
# ★ 383 -> 388 and 65 -> 66 AT THE `gsp-boot-spec` MERGE (`a5d63f8`), and this is the gate
# discriminating on code it was not written against rather than a bookkeeping bump. The
# six sites, read before the numbers moved:
#   docs/design/gsp_boot_gate_spec.md:18   "the isolation verdict this file rests on —
#                                           measured, not estimated"  (no run named)
#   :22 the measurement table's header; :33 "two costs the spike measured";
#   :73 "the spike proved gate G1 only"; :464 "needs, measured above"
#   crates/kayfabe-crec/tests/gsp_boot_gates.rs:111  "Measured: the throwaway C …",
#                                           evidenced by an `ogkm` citation -> CONFLATED
# All six are that branch's to attribute — a spike WAS run, and naming it is one line
# each at the keyboard where it happened. They are on that author's floor, not absorbed.
#
# ★★ AND A THIRD FIRING, ON `554c333` (the arch-axis merge), WHICH WAS A FALSE POSITIVE
# AND MOVED THE PATTERN RATHER THAN THE BAR. `kayfabe-chips/src/ad10x.rs:104` cites ogkm
# for a number and then says, in the same block, *"No Ada card was measured for this
# number."* — the exact honesty this gate exists to encourage, scored as the defect it
# exists to catch, because HONEST_RE knew "not measured" but not "no X was measured".
# The form was added to HONEST_RE and CONFLATED went 67 -> 66 with no other site moving.
# ⊘ Absorbing that site into the bar would have been the cheaper edit and the wrong one:
# a gate that penalises the behaviour it is for teaches people to stop writing it down.
#
# ★ AND A FOURTH FIRING, ON `0ad4c95` — also a false positive, also fixed in the pattern.
# `kayfabe-qemu-raw/src/shim_unsafe.rs` writes `// SAFETY: both pointers were just proven
# non-null`, which is a deduction about the three lines above it and has nothing to cite.
# LOCAL_PROOF_RE now strips that form, which TIGHTENED the ceiling (389 -> 383, fifteen
# blocks turning out to be local proofs only) while CONFLATED and BARE-HW did not move.
# Two false positives in four firings is the honest hit rate; both were pattern defects
# rather than bar pressure, and both were cheaper to fix than to absorb.
# ★ 383 -> 382 on the reachability-on-transition branch (`resume_from_fault.md` §7 step 4).
# Not a campaign, and worth naming exactly because "a site left" is the sort of thing that
# gets attributed to whatever the commit was about: the site was `kayfabe-mocks`'s
# `encode_pde` doc, which called the regime it stands in for "the *measured* regime" and
# named no run. The block was being rewritten anyway (the dual slot grew a second edge) and
# the phrase went with it — the claim word left, the block stopped being a claim site. The
# runner asks for the tightening the moment the ceiling has slack, and the reason it asks is
# that a ceiling nobody tightens stops biting, so it happens in the commit that created the
# slack rather than being left as headroom the next diff can grow into.
UNATTRIBUTED_BAR = 382
CONFLATED_BAR = 66
BARE_HW_BAR = 17


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--list", choices=CLASSES + [CONFLATED, BARE_HW, "all"])
    ap.add_argument("--gate", action="store_true")
    ap.add_argument("--unattributed-bar", type=int, default=None)
    ap.add_argument("--conflated-bar", type=int, default=None)
    args = ap.parse_args()

    sites = sweep()
    counts = {c: sum(1 for s in sites if s.cls == c) for c in CLASSES}
    conflated = [s for s in sites if s.conflated]
    bare_hw = [s for s in sites if s.bare_hw]

    if args.list == CONFLATED:
        chosen = conflated
    elif args.list == BARE_HW:
        chosen = bare_hw
    elif args.list:
        chosen = [s for s in sites if args.list in (s.cls, "all")]
    if args.list:
        for s in chosen:
            print(f"{s.path}:{s.line}  [{s.cls}] ({'+'.join(s.tags) or '-'}) {s.text}")
        return 0

    total = len(sites)
    print("=== CLAIM LEDGER ===")
    for c in CLASSES:
        print(f"  {c:<14} {counts[c]:>5}")
    print(f"  {'TOTAL':<14} {total:>5}")
    print(f"  {'of which':<14}")
    print(f"  {'  CONFLATED':<14} {len(conflated):>5}"
          "   strong claim word, evidence is a READING only")
    print(f"  {'  BARE-HW':<14} {len(bare_hw):>5}"
          "   says 'on hardware' and names no run at all")

    if args.gate:
        ubar = args.unattributed_bar if args.unattributed_bar is not None else UNATTRIBUTED_BAR
        cbar = args.conflated_bar if args.conflated_bar is not None else CONFLATED_BAR
        fail = False

        print(f"\nunattributed claim sites: {counts[UNATTRIBUTED]} (ceiling: {ubar})")
        if counts[UNATTRIBUTED] < ubar:
            print(f"★ The ceiling has slack. Lower it: UNATTRIBUTED_BAR = {counts[UNATTRIBUTED]}")
            print("  (in scripts/claim_ledger.py — a ceiling nobody tightens stops biting)")
        if counts[UNATTRIBUTED] > ubar:
            fail = True
            print("")
            print("★ CLAIM-LEDGER CEILING EXCEEDED (unattributed) — docs/design/claim_ledger.md.")
            print(f"Ceiling is {ubar} unattributed claim site(s); found {counts[UNATTRIBUTED]}.")
            print("")
            print("An UNATTRIBUTED site is a block that says something was measured,")
            print("verified, confirmed or proven and names NOTHING behind it: no capture,")
            print("no date, no revision, no test, no source citation. That is the exact")
            print("shape that turned read-at-invalidate from a reading into a fact and")
            print("cost weeks. Four ways out, all legitimate:")
            print("  - it WAS measured -> name the run: the capture, the date, the")
            print("    revision, or the test that fails if it stops being true.")
            print("  - it was READ from ogkm/nvproxy/the C artifact -> cite it, tagged.")
            print("    That makes it INFERRED, which is a different and honest class.")
            print("  - it is neither -> say so. `not observed to matter`, `assumed`,")
            print("    `unmeasured`. Recording the absence of a measurement is worth more")
            print("    than borrowing one, and the ledger scores it ASSUMED, not red.")
            print("  - you REMOVED one (thank you) -> lower the bar in the same commit.")
            print("")
            print("  scripts/claim_ledger.py --list UNATTRIBUTED")

        print(f"\nconflated claim sites: {len(conflated)} (bar: {cbar})")
        if len(conflated) != cbar:
            fail = True
            print("")
            print("★★★ CLAIM-LEDGER RATCHET MOVED (conflated) — the named failure mode.")
            print(f"Expected {cbar} conflated claim site(s), found {len(conflated)}.")
            print("")
            print("A CONFLATED site says MEASURED / VERIFIED / PROVEN, and the only thing")
            print("behind it is a CITATION INTO SOURCE — ogkm, nvproxy, the C artifact, or")
            print("a pointer at another design doc. Reading the open kernel modules tells")
            print("you what the driver DOES. Only a live boot with real work tells you what")
            print("HAPPENS. A site in this class has done the first and claimed the second,")
            print("and everything downstream inherits it as settled.")
            print("")
            print("The fix is NEVER to delete the claim and never to soften it into vagueness.")
            print("Restate it at the epistemic level actually held, just as specifically:")
            print("  `ogkm-580: X.c:99 sets the bit` — a reading, said as one; or")
            print("  `not observed to matter` — the absence of a measurement, recorded.")
            print("Both are worth more than a false `measured`, and more than silence.")
            print("")
            print("  scripts/claim_ledger.py --list CONFLATED")

        print(f"\nbare hardware claims: {len(bare_hw)} (bar: {BARE_HW_BAR})")
        if len(bare_hw) != BARE_HW_BAR:
            fail = True
            print("")
            print("★★★ BARE HARDWARE CLAIM — the strongest of the three rules.")
            print(f"Expected {BARE_HW_BAR} such site(s), found {len(bare_hw)}.")
            print("")
            print("A block below says something was measured, verified or proven ON")
            print("HARDWARE, and names no run at all: no date, no capture, no revision, no")
            print("machine, no test. There is nothing else the phrase can mean — it asserts")
            print("that a machine was switched on — so unlike the two ratchets above, a")
            print("source citation elsewhere in the block buys NO exemption here.")
            print("")
            print("★ This rule exists because the FIRST PLANTED BITE DID NOT FIRE. A claim")
            print("dropped into a paragraph that already carried a citation inherited that")
            print("paragraph's attribution, both ratchets stayed green, and the runner")
            print("exited 0. The block is the unit of the other two rules, and a paragraph")
            print("is a place a claim can hide. This rule does not use the paragraph.")
            print("")
            print("Name the run, or take the phrase out. If the run is the C artifact's")
            print("rather than ours, say whose it was — an inherited measurement is still")
            print("a measurement and is worth strictly more than an anonymous one.")
            print("")
            for s in bare_hw:
                print(f"  {s.path}:{s.line}  {s.text}")

        if fail:
            return 1
        print("\nclaim ledger: all three attribution rules hold")
    return 0


if __name__ == "__main__":
    sys.exit(main())
