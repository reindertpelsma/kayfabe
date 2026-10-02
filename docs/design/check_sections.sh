#!/usr/bin/env bash
# ★★★ NO TWO SECTIONS MAY SHARE A NUMBER — and one doc had FIVE §6.1s.
#
# `[measured w823]` `THE_DESIGN.md` carried **§6.1 five times**, including a 52-line block
# duplicated VERBATIM, and — the dangerous one — a section whose PROSE had been updated to the
# current design while its TABLE still stated the superseded one. §9 ran `9.2, 9.4, 9.1, 9.2`.
#
# ⊘ This is the same failure the comment audit found in the source (~50 contradiction pairs, three
# of them a file disagreeing with itself), in the document written to ESCAPE it. A design that
# contradicts itself produces code that contradicts itself, and the cost lands at implementation
# time when it is most expensive to unpick.
#
# ⚠ `10.4a` is NOT a duplicate of `10.4`, and a cross-reference heading like "§41 is amended" is
# not a section declaration. Both were false positives in the first version of this gate — a gate
# that cries wolf is a gate people switch off.
set -uo pipefail
cd "$(dirname "$0")"
# ⊘ 2026-10-02: this gate passed vacuously on a known duplicate. Two reasons:
#   - GNU grep (the CI runner's grep) never matches the old `(?= )` lookahead pattern; it warns
#     "? at start of expression" and returns 1, so only the fallback grep ever ran.
#   - That fallback's `§?` binds `?` to the LAST byte of the two-byte `§` in a C locale, so no
#     heading without a `§` matched.
# Hence `(§)?`, one GNU-compatible grep, and the self-test below: report one before reporting zero.
dup_numbers() {
    grep -oE '^#{2,4} +(§)?[0-9]+(\.[0-9]+)*[a-z]?' "$1" 2>/dev/null \
        | sed 's/^#* *//' | grep -v '^$' | sort | uniq -d | tr '\n' ' '
}
probe=$(mktemp)
printf '## 7. a\n### 7.x b\n## 7.1 c\n## 7.1a d\n' > "$probe"
seen=$(dup_numbers "$probe"); rm -f "$probe"
if [ "$seen" != "7 " ]; then
    echo "SECTION_GATE self-test FAILED: expected the known duplicate '7' only, got '$seen'"
    exit 2
fi
bad=0
for f in *.md; do
    dups=$(dup_numbers "$f")
    if [ -n "${dups// /}" ]; then
        echo "⊘ $f  duplicate section numbers: $dups"
        bad=$((bad+1))
    fi
done
echo "SECTION_GATE files=$(ls *.md | wc -l) offenders=$bad"
[ "$bad" -eq 0 ]
