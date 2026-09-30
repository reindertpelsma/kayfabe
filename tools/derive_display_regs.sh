#!/usr/bin/env bash
# ★ DERIVE the display engine's BAR0 register vocabulary — `NV_PDISP_*` / `NV_UDISP_*` registers and
# fields, and the `NV_DMA_*` context-DMA object format in display instance memory — by COMPILING
# ogkm's published `disp/<version>/dev_disp.h` headers. The register counterpart of
# tools/derive_display_classes.sh (same technique: macro NAMES from the preprocessor's own table
# (`gcc -E -dM`), VALUES from the compiler, fields via the `(1?hi:lo)` / `(0?hi:lo)` trick nvmisc.h's
# own DRF_EXTENT/DRF_BASE use). ⊘ Nothing here parses C.
#
# ⚠ One row per (header directory, macro): which directories a family's kernel display HAL compiles
# against, newest first, is decided in ONE place — `kf_disp::regs::lineage` — never here.
#
# Rows (tab-separated):
#     <dir> V   <name> <value>                  a plain value (a register offset, an enum value)
#     <dir> F   <name> <hi> <lo>                a bit field `hi:lo`
#     <dir> A   <name> <base> <stride>          a one-parameter register `X(i)` = base + i*stride,
#                                               LINEAR — checked at i = 0, 1, 2
#     <dir> FA  <name> <hi0> <lo0> <stride>     a one-parameter field `X(i)` = (hi0+i*stride):(lo0+i*stride)
#     <dir> N   <name>                          a one-parameter macro that is NOT linear in i (e.g.
#                                               NV_UDISP_FE_CHN_ASSY_BASEADR(i)): named, never evaluated
#                                               as base+stride — the Rust side refuses to use it
#
# usage: tools/derive_display_regs.sh [path-to-ogkm] > crates/kf-disp/data/regs-<version>.tsv
set -euo pipefail
OG=${1:-/workspace/nvidia-gpu-passthrough/research_clones/ogkm-580.159.04}
PUB="$OG/src/common/inc/swref/published/disp"
[ -d "$PUB" ] || { echo "⊘ no published display headers at $PUB" >&2; exit 2; }
command -v gcc >/dev/null || { echo "⊘ gcc is required — it IS the parser here" >&2; exit 2; }
VER=$(sed -n 's/^NVIDIA_VERSION = //p' "$OG/version.mk")
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
echo "# derived by tools/derive_display_regs.sh from ogkm $VER — do not edit by hand"
echo "VERSION	$VER"
for d in "$PUB"/v*/; do
  dir=$(basename "$d")
  h="$d/dev_disp.h"
  [ -f "$h" ] || continue
  gcc -E -dM "$h" > "$tmp/macros.$dir"
  python3 - "$tmp/macros.$dir" "$dir" "$h" > "$tmp/d_$dir.c" <<'PY'
import sys, re
macros, dirname, header = sys.argv[1], sys.argv[2], sys.argv[3]
prefixes = ("NV_PDISP_", "NV_UDISP_", "NV_DMA_")
head = re.compile(r'#define ([A-Za-z_][A-Za-z0-9_]*)(\(([^)]*)\))? ?(.*)$')
print('#include <stdio.h>')
print(f'#include "{header}"')
print('int main(void){')
u = '(unsigned long long)'
for line in open(macros):
    m = head.match(line.rstrip('\n'))
    # the prefixes, and the display aperture itself (`NV_PDISP`, a bare name)
    if not m or not (m.group(1).startswith(prefixes) or m.group(1) == "NV_PDISP"):
        continue
    n, params, body = m.group(1), m.group(3), m.group(4)
    if not body.strip():
        continue
    arity = 0 if params is None else len([p for p in params.split(',') if p.strip()])
    d = dirname
    # a bit field's body holds ':' at its top level and no '?' (a ternary is a value, never a field)
    is_field = ':' in body and '?' not in body
    if arity == 0 and is_field:
        print(f'  printf("{d}\\tF\\t{n}\\t%llu\\t%llu\\n", {u}(1?{n}), {u}(0?{n}));')
    elif arity == 0:
        print(f'  printf("{d}\\tV\\t{n}\\t%llu\\n", {u}({n}));')
    elif arity == 1 and is_field:
        # an indexed field: (hi0+i*s):(lo0+i*s)
        print(f'  printf("{d}\\tFA\\t{n}\\t%llu\\t%llu\\t%llu\\n", {u}(1?{n}(0)), {u}(0?{n}(0)), {u}(0?{n}(1))-{u}(0?{n}(0)));')
    elif arity == 1:
        # linear iff X(2)-X(1) == X(1)-X(0)
        print(f'  if ({u}({n}(2))-{u}({n}(1)) == {u}({n}(1))-{u}({n}(0)))')
        print(f'    printf("{d}\\tA\\t{n}\\t%llu\\t%llu\\n", {u}({n}(0)), {u}({n}(1))-{u}({n}(0)));')
        print(f'  else printf("{d}\\tN\\t{n}\\n");')
print('  return 0; }')
PY
  gcc -w -o "$tmp/d_$dir" "$tmp/d_$dir.c" 2>"$tmp/err_$dir" || { echo "⊘ $dir did not compile:" >&2; head -20 "$tmp/err_$dir" >&2; exit 3; }
  "$tmp/d_$dir" | sort
done
