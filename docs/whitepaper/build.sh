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
rm -f *.aux *.log *.out *.toc
echo "wrote $(pwd)/kayfabe_architecture.pdf"
