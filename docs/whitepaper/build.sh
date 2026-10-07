#!/bin/sh
# Build the kayfabe architecture whitepaper.
#
# Toolchain: xelatex (TeX Live) + fontspec + TikZ. Chosen because every diagram
# is native TikZ, so the output is vector at every zoom level and carries no
# rasterised boxes-and-arrows. lualatex is NOT used: this TeX Live install has
# no luaotfload font cache and cannot load its own default fonts.
#
# Two passes are required for the table of contents and the cross-references.
set -e
cd "$(dirname "$0")"
xelatex -interaction=nonstopmode kayfabe_architecture.tex >/dev/null
xelatex -interaction=nonstopmode kayfabe_architecture.tex >/dev/null

# ---------------------------------------------------------------- log gate --
# xelatex under -interaction=nonstopmode EXITS 0 AND WRITES A PDF over LaTeX
# errors, undefined macros and dropped glyphs. Measured 2026-08-20: a bare `^`
# inside \code{} put TeX into math mode -- six errors, one swallowed \item,
# three bullets of the executive summary typeset in the monospace font -- and
# the build reported success. `\text{}` without amsmath did the same seven more
# times, typesetting the argument and discarding the macro.
#   => THE EXIT STATUS IS NOT THE RESULT. THE LOG IS.
# Three classes, each of which has actually bitten this document:
#   `^!`                    a real LaTeX error
#   Undefined control...    a macro that silently did nothing
#   Missing character       a glyph absent from the font, dropped in silence
#                           (see preamble.tex: this cost the (\oslash) marker,
#                            whose loss INVERTS the sentence it qualifies)
bad=0
for pat in '^!' 'Undefined control sequence' 'Missing character'; do
  n=$(grep -c "$pat" kayfabe_architecture.log || true)
  if [ "$n" -ne 0 ]; then echo "BUILD GATE: $n x '$pat' in the log" >&2; bad=1; fi
done
if [ "$bad" -ne 0 ]; then
  echo "BUILD GATE FAILED -- keeping kayfabe_architecture.log for inspection" >&2
  exit 1
fi

pages=$(pdfinfo kayfabe_architecture.pdf 2>/dev/null | awk '/^Pages:/{print $2}')
rm -f *.aux *.log *.out *.toc
echo "wrote $(pwd)/kayfabe_architecture.pdf (${pages:-?} pages, log clean)"
