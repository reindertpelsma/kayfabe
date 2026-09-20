#!/usr/bin/env bash
# Rebuild docs/pdf/kayfabe_architecture_v2.pdf from the four design documents.
#
# ⊘ The font matters: this tree's documents carry meaning in ⊘ ★ ⚠ ⇒, and the default
# LaTeX font has none of them — pandoc emits "Missing character" warnings and silently drops
# them, which turns a refusal into a blank. DejaVu covers all four.
set -uo pipefail
cd "$(dirname "$0")/.."
OUT=docs/pdf/kayfabe_architecture_v2.pdf
mkdir -p docs/pdf
TMP=$(mktemp -d)
{
    sed -n '1,/^\\newpage$/p' docs/design/THE_ARCH_PDF_FRONT.md 2>/dev/null || true
    cat docs/design/THE_ARCH_PDF_FRONT.md
    printf '\n\\newpage\n\n# Part 1 — The v2 architecture (the proposal)\n\n'; tail -n +2 docs/design/THE_ARCHITECTURE_v2.md
    printf '\n\\newpage\n\n# Part 2 — The machine, as it is\n\n';               tail -n +2 docs/design/THE_MACHINE.md
    printf '\n\\newpage\n\n# Part 3 — The surface we present\n\n';              tail -n +2 docs/design/THE_SURFACE_v2.md
    printf '\n\\newpage\n\n# Part 4 — The scrub question (open owner ruling)\n\n'; tail -n +2 docs/design/the_scrub_is_the_last_thing_on_the_cpu.md
} > "$TMP/bundle.md"
pandoc "$TMP/bundle.md" -o "$OUT" --pdf-engine=xelatex --toc --toc-depth=2 \
    -V geometry:margin=2cm -V colorlinks=true \
    -V mainfont="DejaVu Serif" -V sansfont="DejaVu Sans" -V monofont="DejaVu Sans Mono" \
    2>&1 | grep -viE 'missing character' || true
rm -rf "$TMP"
ls -la "$OUT"
