#!/usr/bin/env bash
# build_extra.sh <out-dir> — the test set's helper programs, built IN THE GUEST IMAGE (provision.sh), so
# the bare-metal side (hostroot.sh) runs the very same binaries. Prints BUILD_<name>=ok|FAIL per program.
#   nvkvmpv/  nvkvm-pv's probes, verbatim (validate.sh's embedded vk/gl probes, tests/repro, tests/perf/apps,
#             tests/integration — candidate branch `integration/candidate-2026-09-18` where it differs)
#   wl_scene.c the deterministic Wayland client of this set
#   glmark2 2023.01 from source (nvkvm-pv build_glmark2.sh: flavors wayland-gl,wayland-glesv2,x11-gl,x11-glesv2)
set -uo pipefail
S="$(cd "$(dirname "$0")" && pwd)"; B=${1:?out}; mkdir -p "$B"; T=$(mktemp -d); P=$S/nvkvmpv
WP=/usr/share/wayland-protocols
ok(){ if [ "$2" -eq 0 ]; then echo "BUILD_$1=ok"; else echo "BUILD_$1=FAIL rc=$2"; fi; }
cc(){ gcc "$@" 2>>"$T/cc.err"; }
EGLGL="-lEGL -lGLESv2"; GBMX="$(pkg-config --cflags --libs gbm libdrm 2>/dev/null)"
cc -O2 -o "$B/vk_probe" "$P/vk_probe.c" -ldl;                              ok vk_probe $?
cc -O2 -o "$B/gl_probe" "$P/gl_probe.c" -ldl;                              ok gl_probe $?
cc -O1 -o "$B/vk_device_extensions" "$P/vk_device_extensions.c" -ldl;      ok vk_device_extensions $?
cc -O2 -o "$B/vk_create_device" "$P/vk_create_device.c" -lvulkan;          ok vk_create_device $?
cc -O2 -o "$B/signal_restart_export" "$P/signal_restart_export.c" -ldl;    ok signal_restart_export $?
cc -O0 -o "$B/fbo_formats_probe" "$P/fbo_formats_probe.c" -ldl;            ok fbo_formats_probe $?
cc -O2 -o "$B/egl_dmabuf_export_probe" "$P/egl_dmabuf_export_probe.c" $EGLGL; ok egl_dmabuf_export_probe $?
cc -O2 -o "$B/dmabuf_import_probe" "$P/dmabuf_import_probe.c" $GBMX $EGLGL;  ok dmabuf_import_probe $?
cc -O2 -o "$B/xiso_import_probe" "$P/xiso_import_probe.c" $GBMX $EGLGL;      ok xiso_import_probe $?
cc -O2 -o "$B/xiso_bytes_probe" "$P/xiso_bytes_probe.c" $GBMX $EGLGL;        ok xiso_bytes_probe $?
cc -O2 -o "$B/gbm_egl_import" "$P/gbm_egl_import.c" $GBMX $EGLGL;            ok gbm_egl_import $?
cc -O2 -o "$B/gbmprobe" "$P/gbmprobe.c" $GBMX $EGLGL;                        ok gbmprobe $?
cc -O2 -o "$B/gbmshot" "$P/gbmshot.c" $GBMX $EGLGL -lm;                      ok gbmshot $?
cc -O2 -o "$B/gl_decompose" "$P/gl_decompose.c" $EGLGL -lm;                  ok gl_decompose $?
cc -O2 -o "$B/gl_drawrate" "$P/gl_drawrate.c" $EGLGL;                        ok gl_drawrate $?
cc -O2 -o "$B/gl_finishrate" "$P/gl_finishrate.c" $EGLGL;                    ok gl_finishrate $?
cc -O2 -o "$B/vk_ofa" "$S/vk_ofa.c" -lvulkan -lm;                            ok vk_ofa $?
# Wayland clients: protocol glue from wayland-scanner
wayland-scanner client-header "$WP/stable/xdg-shell/xdg-shell.xml" "$T/xdg-shell-client-protocol.h" \
  && wayland-scanner private-code "$WP/stable/xdg-shell/xdg-shell.xml" "$T/xdg-shell-protocol.c" \
  && cc -O2 -Wall -I"$T" -o "$B/wl_scene" "$S/wl_scene.c" "$T/xdg-shell-protocol.c" -lwayland-client -lwayland-egl $EGLGL
ok wl_scene $?
wayland-scanner client-header "$P/wlr-screencopy-unstable-v1.xml" "$T/wlr-screencopy-unstable-v1-client-protocol.h" \
  && wayland-scanner private-code "$P/wlr-screencopy-unstable-v1.xml" "$T/wlr-screencopy-unstable-v1-protocol.c" \
  && wayland-scanner client-header "$WP/unstable/linux-dmabuf/linux-dmabuf-unstable-v1.xml" "$T/linux-dmabuf-unstable-v1-client-protocol.h" \
  && wayland-scanner private-code "$WP/unstable/linux-dmabuf/linux-dmabuf-unstable-v1.xml" "$T/linux-dmabuf-unstable-v1-protocol.c" \
  && cc -O2 -I"$T" -I/usr/include/libdrm -o "$B/wlr_screencap" "$P/wlr_screencap.c" "$T/wlr-screencopy-unstable-v1-protocol.c" \
        "$T/linux-dmabuf-unstable-v1-protocol.c" $(pkg-config --cflags --libs wayland-client gbm libdrm)
ok wlr_screencap $?
[ -s "$T/cc.err" ] && { echo "--- compiler diagnostics (first 30)"; head -30 "$T/cc.err"; }
# glmark2 2023.01 (nvkvm-pv build_glmark2.sh), into $GSET_HOME/glmark2 — skipped when already built
GMP=${GSET_HOME:-/opt/gfxset}/glmark2
if [ ! -x "$GMP/bin/glmark2-wayland" ]; then
  rm -rf "$T/glmark2" && git clone -q --depth 1 -b 2023.01 https://github.com/glmark2/glmark2 "$T/glmark2" \
    && ( cd "$T/glmark2" && meson setup build --prefix="$GMP" -Dflavors=wayland-gl,wayland-glesv2,x11-gl,x11-glesv2 > "$T/gm.log" 2>&1 \
         && ninja -C build >> "$T/gm.log" 2>&1 && ninja -C build install >> "$T/gm.log" 2>&1 ) \
    || tail -15 "$T/gm.log"
fi
[ -x "$GMP/bin/glmark2-wayland" ]; ok glmark2 $?
ls "$GMP/bin" 2>/dev/null | tr '\n' ' '; echo
rm -rf "$T"
