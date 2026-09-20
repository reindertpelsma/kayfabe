#!/usr/bin/env bash
# Rebuild docs/pdf/kayfabe_architecture_v3.pdf from the v3 design documents.
#
# ⊘ The font matters: this tree's documents carry meaning in ⊘ ★ ⚠ ⇒, and the default
# LaTeX font has none of them — pandoc emits "Missing character" warnings and silently drops
# them, which turns a refusal into a blank. DejaVu covers all four.
#
# ⚠ Wide reference tables: longtable in xelatex will overflow the text block rather than
# wrap, so the column model below keeps tables narrow. If a table clips, fix the SOURCE
# table's column count — do not widen the geometry, because that reflows every other page.
set -uo pipefail
cd "$(dirname "$0")/.."
OUT=docs/pdf/kayfabe_design.pdf
mkdir -p docs/pdf
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

part() { printf '\n\\newpage\n\n# %s\n\n' "$1"; tail -n +2 "$2"; }

{
    cat docs/design/THE_PDF_FRONT_FINAL.md
    part 'Part 1 — The design'                                docs/design/THE_DESIGN.md
    part 'Part 2 — The plan'                                  docs/design/THE_V3_PLAN.md
    part 'Part 3 — The surface, at constant level'            docs/design/THE_SURFACE_v3.md
    part 'Part 4 — What the oracles tell us'                  docs/design/THE_OGKM_RESIDUE.md
    part 'Part 5 — The guest-OS axis'                         docs/design/THE_WINDOWS_AXIS.md
    part 'Part 6 — What is still open'                        docs/design/THE_OPEN_QUESTIONS.md
} > "$TMP/bundle.md"

pandoc "$TMP/bundle.md" -o "$OUT" --pdf-engine=xelatex --toc --toc-depth=3 \
    -V geometry:margin=1.6cm -V colorlinks=true \
    -V documentclass=extarticle -V fontsize=9pt \
    -V mainfont="DejaVu Serif" -V sansfont="DejaVu Sans" -V monofont="DejaVu Sans Mono" \
    -V monofontoptions="Scale=0.72" -H docs/design/pdf_header_v3.tex \
    2>&1 | grep -viE 'missing character' || true

ls -la "$OUT"
