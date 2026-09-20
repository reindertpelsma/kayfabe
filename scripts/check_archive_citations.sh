#!/usr/bin/env bash
# ★★★★★ THE ARCHIVE-CITATION GATE — how many comments still cite a SUPERSEDED design?
#
# `[measured w822/w823]` 1 340 `.md` citations live in source comments. **895 of them (67 %)
# resolve to `docs/archive/`** — files whose own header says *"SUPERSEDED — reference, not
# current"*. `l1_concurrency.md` alone is cited **148 times**.
#
# ★★★ THE ARCHIVE MOVE DID NOT CREATE THIS; IT REVEALED IT. Before w822 those same citations
# pointed into `docs/design/` and were **indistinguishable from live ones** — a reader following
# one landed in a superseded architecture with nothing marking it. Moving 190 docs converted an
# invisible problem into a **greppable** one. ⇒ That is the answer to *"how do I ensure old
# architectures in old code read as reference, not current"*: not by editing 895 comments, but by
# making the staleness **mechanically visible** and then ratcheting it down.
#
# ⊘ This gate does NOT demand zero. Most of those citations are legitimately historical — a
# comment explaining why a thing was built cites the doc that argued for it, and that doc is
# SUPPOSED to be archived. Demanding zero would delete the provenance this tree runs on.
# ⚠ What it forbids is the number **rising**: a NEW comment citing an archived doc is a new
# assertion of a dead architecture, which is exactly the class this whole exercise is about.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 2
# ⊘⊘⊘ THE BASELINE IS THIS SCRIPT'S OWN MEASUREMENT, NOT THE SURVEY'S.
# The w823 comment survey reported `895 / 1340`; this gate measures `1011 / 1521` on the same
# tree at the same revision. Both are right: the survey de-duplicated and resolved citations,
# this counts every OCCURRENCE (a file cited twice in one function counts twice) and includes
# `.md` strings that are not doc references. ⚠ **They are different quantities and must never be
# quoted interchangeably.** A gate seeded with a number produced by a different method fires on
# the METHOD GAP, not on a regression — which is a false alarm that trains people to ignore it.
# ⊘⊘⊘ **ARCHIVING A DOC RAISES THIS COUNT WITHOUT ANY COMMENT CHANGING — measured w823.**
# Moving `SINGLE_STORE_PLAN.md` to `docs/archive/` took the number from **1011 to 1110**: exactly
# its **99** citations, none of them edited. ⇒ The gate cannot distinguish *"someone wrote a new
# citation to a dead doc"* (what it exists to catch) from *"a doc the comments already cited was
# correctly archived"* (which is the archive working as intended, and is GOOD).
# ⚠ So a rise is a PROMPT, not a verdict. Before rebasing, answer which of the two happened:
#     git log --diff-filter=R --name-status -1 -- docs/archive/   # was a doc just moved?
# If a doc moved, rebase the baseline and say so here. If not, a new comment cited a dead
# architecture as current — fix the comment instead.
#
# w823: 1011 -> 1110 (SINGLE_STORE_PLAN.md archived; it said STATUS: LIVE while describing the
#       scratchpad-isolate plane §10 deletes).
BASE=${ARCHIVE_CITE_BASELINE:-1110}

tmp=$(mktemp)
grep -rhoE '[A-Za-z0-9_./-]+\.md' --include='*.rs' crates/ 2>/dev/null \
  | sed 's|.*/||' | sort > "$tmp"
total=$(wc -l < "$tmp")
arch=0
while read -r f; do
    [ -n "$f" ] || continue
    if [ -e "docs/archive/$f" ]; then arch=$((arch+1)); fi
done < "$tmp"
rm -f "$tmp"

echo "ARCHIVE_CITE total=$total archived=$arch baseline=$BASE"
if [ "$arch" -gt "$BASE" ]; then
    echo "⊘ REFUSING — archived-doc citations ROSE ($arch > $BASE)."
    echo "  A new comment citing docs/archive/ asserts a superseded architecture as current."
    echo "  Cite a LIVE doc, or say in the comment that the reference is historical."
    exit 1
fi
[ "$arch" -lt "$BASE" ] && echo "✔ ratcheted down by $((BASE-arch)) — lower ARCHIVE_CITE_BASELINE to $arch"
echo "ARCHIVE_CITE_GATE=PASS"
