#!/usr/bin/env bash
# ★ DERIVE the display channel classes' METHOD and FIELD layouts, and the caps registers, by COMPILING
# ogkm's class headers — the display-plane counterpart of tools/derive_hwref.sh (same technique:
# macro NAMES from the preprocessor's own table (`gcc -E -dM`), VALUES from the compiler, fields via
# the `(1?hi:lo)` / `(0?hi:lo)` trick nvmisc.h's own DRF_EXTENT/DRF_BASE use).
#
# Rows (one per macro whose name matches the spec below):
#     V   <name> <value>                 a plain value (a method offset, an enum value)
#     F   <name> <hi> <lo>               a bit field `hi:lo`
#     A   <name> <base> <stride>         a one-parameter macro `X(i)` = base + i*stride
#     A2  <name> <base> <s1> <s2>        a two-parameter macro `X(a,b)` = base + a*s1 + b*s2
#     FA  <name> <hi0> <lo0> <stride>    a one-parameter bit field `X(i)` = (hi0+i*s):(lo0+i*s)
# Classes: every family's core/window/window-immediate/cursor channel and caps class the display plane
# serves (Turing C57x, Ampere C67x, Ada C77D + C67x, Blackwell GB20x CA7x), plus NV_DISP_NOTIFIER.
# ⊘ Names are matched on PREFIX, never on bodies; the compiler evaluates every body. A body is taken
#   as a bit field when the preprocessor's own text of it holds a ':' (`31:30`, `(1*32+13):(1*32+0)`).
#
# usage: tools/derive_display_classes.sh [path-to-ogkm] > crates/kf-disp/data/classes-<version>.tsv
set -euo pipefail
OG=${1:-/workspace/nvidia-gpu-passthrough/research_clones/ogkm-580.159.04}
INC="$OG/src/common/sdk/nvidia/inc"
VER=$(sed -n 's/^NVIDIA_VERSION = //p' "$OG/version.mk")
CLASSES="c373 c57a c57b c57d c57e c573 c67a c67b c67d c67e c673 c77d c773 ca7a ca7b ca7d ca7e ca73"
SPEC='UPDATE SET_CONTEXT_DMA_NOTIFIER SET_NOTIFIER_CONTROL SET_INTERLOCK_FLAGS SET_WINDOW_INTERLOCK_FLAGS
HEAD_SET_PIXEL_CLOCK_FREQUENCY HEAD_SET_RASTER_ HEAD_SET_VIEWPORT_ HEAD_SET_CONTEXT_DMA_CURSOR HEAD_SET_OFFSET_CURSOR
HEAD_SET_CONTROL_CURSOR HEAD_SET_DISPLAY_ID HEAD_SET_CONTROL_OUTPUT_RESOURCE HEAD_SET_SURFACE_ADDRESS_HI_CURSOR
HEAD_SET_SURFACE_ADDRESS_LO_CURSOR WINDOW_SET_CONTROL SOR_SET_CONTROL SET_SIZE SET_STORAGE SET_PARAMS
SET_PLANAR_STORAGE SET_CONTEXT_DMA_ISO SET_OFFSET SET_POINT_IN SET_POINT_OUT SET_PRESENT_CONTROL SET_COMPOSITION_
SET_CONTEXT_DMA_SEMAPHORE SET_SEMAPHORE_CONTROL SET_SEMAPHORE_RELEASE SET_SEMAPHORE_ACQUIRE
SET_CONTEXT_DMA_ACQ_SEMAPHORE SET_ACQ_SEMAPHORE_CONTROL SET_ACQ_SEMAPHORE_VALUE SET_SCAN_DIRECTION
SET_SURFACE_ADDRESS_ FREE SET_CURSOR_HOT_SPOT_POINT_OUT GET PUT SYS_CAP HEAD_CAP HEAD_CLK_CAP SOR_CAP
SOR_CLK_CAP PRECOMP_WIN_PIPE_HDR_CAP POSTCOMP_HEAD_HDR_CAP IHUB_COMMON_CAP WINDOW_CAP MISC_CAP
SET_CONTEXT_DMA_ILUT SET_ILUT_CONTROL SET_CONTEXT_DMA_TMO SET_TMO_CONTROL SET_TMO_LOW_INTENSITY_
SET_TMO_MEDIUM_INTENSITY_ SET_TMO_HIGH_INTENSITY_ HEAD_SET_OLUT_CONTROL HEAD_SET_OLUT_FP_NORM_SCALE
HEAD_SET_CONTEXT_DMA_OLUT HEAD_SET_OFFSET_OLUT'
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
{ echo '#include "nvtypes.h"'; for c in $CLASSES; do echo "#include \"class/cl$c.h\""; done; } > "$tmp/all.h"
gcc -E -dM -I"$INC" "$tmp/all.h" > "$tmp/macros"
# The preprocessor's table is one `#define NAME[(params)] body` per line; split it into NAME, arity,
# and whether the body text holds ':' — then let the COMPILER evaluate every body.
python3 - "$tmp/macros" "$CLASSES" "$SPEC" > "$tmp/d.c" <<'PY'
import sys, re
macros, classes, spec = sys.argv[1], sys.argv[2].split(), sys.argv[3].split()
prefixes = tuple(f"NV{c.upper()}_{s}" for c in classes for s in spec) + ("NV_DISP_NOTIFIER",)
head = re.compile(r'#define ([A-Za-z_][A-Za-z0-9_]*)(\(([^)]*)\))? ?(.*)$')
print('#include <stdio.h>'); print('#include "all.h"'); print('int main(void){')
for line in open(macros):
    m = head.match(line.rstrip('\n'))
    if not m or not m.group(1).startswith(prefixes):
        continue
    n, params, body = m.group(1), m.group(3), m.group(4)
    if not body.strip():
        continue
    arity = 0 if params is None else len([p for p in params.split(',') if p.strip()])
    u = '(unsigned long long)'
    if arity == 0 and ':' in body:
        print(f'  printf("F\\t{n}\\t%llu\\t%llu\\n", {u}(1?{n}), {u}(0?{n}));')
    elif arity == 0:
        print(f'  printf("V\\t{n}\\t%llu\\n", {u}({n}));')
    elif arity == 1 and ':' not in body:
        print(f'  printf("A\\t{n}\\t%llu\\t%llu\\n", {u}({n}(0)), {u}({n}(1))-{u}({n}(0)));')
    elif arity == 1 and '?' not in body:
        # an indexed bit field `X(i)` = (hi0+i*s):(lo0+i*s) — e.g. INTERLOCK_WITH_WINDOW(i)
        print(f'  printf("FA\\t{n}\\t%llu\\t%llu\\t%llu\\n", {u}(1?{n}(0)), {u}(0?{n}(0)), {u}(0?{n}(1))-{u}(0?{n}(0)));')
    elif arity == 2 and ':' not in body:
        print(f'  printf("A2\\t{n}\\t%llu\\t%llu\\t%llu\\n", {u}({n}(0,0)), {u}({n}(1,0))-{u}({n}(0,0)), {u}({n}(0,1))-{u}({n}(0,0)));')
print('  return 0; }')
PY
cp "$tmp/all.h" "$tmp/all.h.keep"
gcc -w -I"$INC" -I"$tmp" -o "$tmp/d" "$tmp/d.c" 2>"$tmp/err" || { echo "⊘ did not compile:" >&2; head -20 "$tmp/err" >&2; exit 3; }
echo "# derived by tools/derive_display_classes.sh from ogkm $VER — do not edit by hand"
echo "VERSION	$VER"
"$tmp/d" | sort
