#!/usr/bin/env bash
# Build the headless-graphics workloads from source into <out>.  Same sources on bare metal
# and in the guest, so the two sides' hashes are the same program's output.
#   usage: build_gfx.sh <out-dir>     needs: gcc, glslangValidator, libvulkan-dev, libegl-dev, libopengl-dev
set -euo pipefail
SRC="$(cd "$(dirname "$0")" && pwd)"; OUT=${1:?out dir}
mkdir -p "$OUT"; rm -f "$OUT"/vk_gfx "$OUT"/egl_gfx "$OUT"/glx_gfx "$OUT"/*_spv.h   # ⊘ [ -x ] cannot tell fresh from stale
for s in scene.vert scene.frag blit.vert blit.frag compute.comp; do
    n=$(echo "$s" | tr . _)_spv
    glslangValidator -s -V "$SRC/$s" --vn "$n" -o "$OUT/$n.h" >/dev/null
done
gcc -O2 -Wall -I"$OUT" -I"$SRC" -o "$OUT/vk_gfx" "$SRC/vk_gfx.c" -lvulkan -lm
gcc -O2 -Wall -I"$SRC" -o "$OUT/egl_gfx" "$SRC/egl_gfx.c" -lEGL -lOpenGL -lm
gcc -O2 -Wall -I"$SRC" -o "$OUT/glx_gfx" "$SRC/glx_gfx.c" -lGL -lX11 -lm
echo "BUILD_GFX_OK $(md5sum "$SRC"/vk_gfx.c "$SRC"/egl_gfx.c "$SRC"/glx_gfx.c "$SRC"/refcheck.h | md5sum | cut -c1-12)"
