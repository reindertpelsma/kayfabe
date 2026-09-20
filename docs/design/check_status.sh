#!/usr/bin/env bash
# Every doc in docs/design/ must declare its status. A doc with no status reads as current
# forever — this tree's own doc-hygiene rule, and the reason 110 of 219 docs had to be archived.
# Exits non-zero and NAMES the offenders; it does not fix them.
cd "$(dirname "$0")"
bad=0
for f in *.md; do
    grep -qE '^\*\*STATUS|^STATUS:|^> \*\*STATUS|^\*\*\*\*STATUS' "$f" || { echo "⊘ no STATUS: $f"; bad=$((bad+1)); }
done
echo "STATUS_GATE files=$(ls *.md | wc -l) missing=$bad"
[ "$bad" -eq 0 ]
