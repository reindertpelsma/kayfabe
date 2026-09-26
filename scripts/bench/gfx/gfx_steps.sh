#!/usr/bin/env bash
# ★ The five headless-graphics steps (V3_HEADLESS_GRAPHICS.md §6), run ON THE MACHINE THAT HAS
# THE GPU — bare metal (the baseline) or the kayfabe guest.  Every graded line is at column 0:
#   GFX_S1=PASS|FAIL   nvidia-drm modeset=1 loaded, card* + renderD* present
#   GFX_S2=PASS|FAIL   vulkaninfo names the NVIDIA device AND vk_gfx compute VKC_BAD=0
#   GFX_S3=PASS|FAIL   vk_gfx render: CPU-reference check clean (hashes graded vs host by the caller)
#   GFX_S4=PASS|FAIL   EGL device platform display + egl_gfx reference check clean
#   GFX_S5=PASS|FAIL   Xvfb + VirtualGL (EGL back end): glxinfo renderer is NVIDIA AND glx_gfx (GLX in an X
#                      window) renders the scene correctly (CPU reference; hashes graded vs host by the caller).
#                      ⊘ glxgears' frame count is printed but NOT graded: a client-side count stays >0 while
#                      the GPU faults every frame (measured gfx7).
# A step that could not run prints GFX_Sn=NOTRUN with the reason (never a silent pass).
#   usage: gfx_steps.sh <host|guest> <build-dir>
set -uo pipefail
ROLE=${1:?host|guest}; OUT=${2:?build dir}
SRC="$(cd "$(dirname "$0")" && pwd)"
SUDO=""; [ "$(id -u)" -ne 0 ] && SUDO="sudo -n"
T=${GFX_STEP_TIMEOUT:-120}
echo "GFX_STEPS_START role=$ROLE at $(date -Is) kernel=$(uname -r)"
echo "GFX_DRIVER=$(cat /sys/module/nvidia/version 2>/dev/null || echo none)"

# ── S1: nvidia-drm modeset=1, displayless ────────────────────────────────────────────────────
if [ "$ROLE" = guest ]; then
    $SUDO modprobe nvidia-drm modeset=1 2>&1 | sed 's/^/  modprobe: /'
fi
MS=$($SUDO cat /sys/module/nvidia_drm/parameters/modeset 2>/dev/null || echo absent)   # 0400: root-only
CARDS=$(ls /dev/dri 2>/dev/null | tr '\n' ' ')
echo "GFX_S1_MODESET=$MS"; echo "GFX_S1_DRI=$CARDS"
$SUDO dmesg 2>/dev/null | grep -i 'nvidia-drm\|nvidia-modeset\|nvkms\|displayless' | tail -8 | sed 's/^/  dmesg: /'
if [ "$MS" = Y ] && echo "$CARDS" | grep -q card && echo "$CARDS" | grep -q renderD; then echo "GFX_S1=PASS"
else echo "GFX_S1=FAIL"; fi
# the render node must be usable by this (unprivileged) user for the UMDs
ls -la /dev/dri/ 2>/dev/null | sed 's/^/  dri: /'

# ── build ────────────────────────────────────────────────────────────────────────────────────
if ! bash "$SRC/build_gfx.sh" "$OUT" 2>&1 | tail -3; then echo "GFX_BUILD=FAIL"; fi
[ -x "$OUT/vk_gfx" ] && [ -x "$OUT/egl_gfx" ] && [ -x "$OUT/glx_gfx" ] && echo "GFX_BUILD=OK" || echo "GFX_BUILD=FAIL"

# ── S2: vulkaninfo + Vulkan compute ─────────────────────────────────────────────────────────
VI=$(timeout "$T" vulkaninfo --summary 2>&1); VIRC=$?
echo "$VI" | grep -E 'deviceName|driverVersion|apiVersion|deviceType|driverID|ERROR|error' | head -12 | sed 's/^/  vulkaninfo: /'
echo "GFX_VULKANINFO_RC=$VIRC"
VC=$(cd /tmp && timeout "$T" "$OUT/vk_gfx" compute 2>&1); VCRC=$?
echo "$VC" | sed 's/^/  /'; echo "$VC" | grep -E '^VKC_(HASH|BAD)='
echo "GFX_VKC_RC=$VCRC"
if [ "$VIRC" -eq 0 ] && echo "$VI" | grep -q 'deviceName.*NVIDIA' && echo "$VC" | grep -q '^VKC_BAD=0$'; then echo "GFX_S2=PASS"
else echo "GFX_S2=FAIL"; fi

# ── S3: Vulkan offscreen render with depth ──────────────────────────────────────────────────
VR=$(cd /tmp && timeout "$T" "$OUT/vk_gfx" render "/tmp/vkr_$ROLE" 2>&1); VRRC=$?
echo "$VR" | sed 's/^/  /'; echo "$VR" | grep -E '^VKR_(HASH_[AZB]|REF_FAILS)='
echo "GFX_VKR_RC=$VRRC"
if echo "$VR" | grep -q '^VKR_REF_FAILS=0$'; then echo "GFX_S3=PASS"; else echo "GFX_S3=FAIL"; fi

# ── S4: EGL headless GL ─────────────────────────────────────────────────────────────────────
EI=$(timeout "$T" eglinfo -B 2>&1); EIRC=$?
echo "$EI" | grep -iE 'platform|renderer|vendor|version string' | head -16 | sed 's/^/  eglinfo: /'
echo "GFX_EGLINFO_RC=$EIRC"
ER=$(cd /tmp && timeout "$T" "$OUT/egl_gfx" 2>&1); ERRC=$?
echo "$ER" | sed 's/^/  /'; echo "$ER" | grep -E '^EGLR_(HASH_[AZB]|REF_FAILS)=|^EGL_RENDERER='
echo "GFX_EGLR_RC=$ERRC"
if echo "$ER" | grep -q '^EGLR_REF_FAILS=0$' && echo "$ER" | grep -q '^EGL_RENDERER=.*NVIDIA'; then echo "GFX_S4=PASS"
else echo "GFX_S4=FAIL"; fi

# ── S5: headless desktop — Xvfb + VirtualGL with the EGL back end ──────────────────────────
if ! command -v Xvfb >/dev/null || ! command -v vglrun >/dev/null; then
    echo "GFX_S5=NOTRUN (Xvfb=$(command -v Xvfb || echo missing) vglrun=$(command -v vglrun || echo missing))"
else
    pkill -x Xvfb 2>/dev/null; sleep 1
    Xvfb :7 -screen 0 1280x720x24 -nolisten tcp >/tmp/xvfb7.log 2>&1 & XP=$!
    sleep 2
    GI=$(DISPLAY=:7 timeout "$T" vglrun -d egl glxinfo -B 2>&1); echo "$GI" | grep -iE 'renderer|vendor string|version string|error' | head -6 | sed 's/^/  glxinfo: /'
    GG=$(DISPLAY=:7 timeout 12 vglrun -d egl glxgears 2>&1); echo "$GG" | grep -iE 'frames|error' | head -4 | sed 's/^/  glxgears: /'
    FR=$(echo "$GG" | grep -o '^[0-9]* frames' | head -1 | cut -d' ' -f1)
    echo "GFX_S5_RENDERER=$(echo "$GI" | grep -i 'OpenGL renderer string' | sed 's/.*: //')"
    echo "GFX_S5_FRAMES=${FR:-0}"
    GX=$(cd /tmp && DISPLAY=:7 timeout "$T" vglrun -d egl "$OUT/glx_gfx" 2>&1); GXRC=$?
    echo "$GX" | sed 's/^/  /'; echo "$GX" | grep -E '^GLXR_(HASH_[AZB]|REF_FAILS)=|^GLX_RENDERER='
    echo "GFX_GLXR_RC=$GXRC"
    kill $XP 2>/dev/null; wait $XP 2>/dev/null
    if echo "$GI" | grep -qi 'renderer string.*NVIDIA' && echo "$GX" | grep -q '^GLXR_REF_FAILS=0$' \
       && echo "$GX" | grep -q '^GLX_RENDERER=.*NVIDIA'; then echo "GFX_S5=PASS"; else echo "GFX_S5=FAIL"; fi
fi
echo "GFX_STEPS_END role=$ROLE at $(date -Is)"
