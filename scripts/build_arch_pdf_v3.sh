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
OUT=docs/pdf/kayfabe_architecture_v3.pdf
mkdir -p docs/pdf
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

part() { printf '\n\\newpage\n\n# %s\n\n' "$1"; tail -n +2 "$2"; }

{
    cat docs/design/THE_ARCH_PDF_FRONT_V3.md
    part 'Part 1 — The v3 architecture (the proposal)'        docs/design/THE_ARCHITECTURE_v3.md
    part 'Part 2 — The surface, at constant level'            docs/design/THE_SURFACE_v3.md
    part 'Part 3 — The Windows axis'                          docs/design/THE_WINDOWS_AXIS.md
    part 'Part 4 — ogkm residue: monolithic, pre-Turing, Windows' docs/design/THE_OGKM_RESIDUE.md
    part 'Part 5 — The machine, as it is'                     docs/design/THE_MACHINE.md
    part 'Part 6 — The surface we present (v2 plan-level)'    docs/design/THE_SURFACE_v2.md
    part 'Part 7 — The scrub question (open owner ruling)'    docs/design/the_scrub_is_the_last_thing_on_the_cpu.md
} > "$TMP/bundle.md"

pandoc "$TMP/bundle.md" -o "$OUT" --pdf-engine=xelatex --toc --toc-depth=3 \
    -V geometry:margin=1.6cm -V colorlinks=true \
    -V documentclass=extarticle -V fontsize=9pt \
    -V mainfont="DejaVu Serif" -V sansfont="DejaVu Sans" -V monofont="DejaVu Sans Mono" \
    -V monofontoptions="Scale=0.72" -H docs/design/pdf_header_v3.tex \
    2>&1 | grep -viE 'missing character' || true

ls -la "$OUT"
