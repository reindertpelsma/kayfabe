#!/usr/bin/env bash
# itemdefs.sh — the items of the headless-graphics test set (sourced by items.sh; each item_<name>
# runs in its own session, cwd = its scratch dir $W, stdout = its log). docs/design/V3_GFX_TESTSET.md
# §1 has the inventory: every `pv_*`/named item below maps to one nvkvm-pv row H1..H25 and keeps
# nvkvm-pv's own pass criterion (quoted at the item); where the workload's output is deterministic the
# item ALSO emits a content digest, and the suite grades that against bare metal (a stronger check
# than nvkvm-pv's). Items marked EXTRA are not in nvkvm-pv's validated set.
# Each item prints:
#   GSET_OK            its own check held (the floor that holds without a host)
#   GSET_FAIL <why>    its own check failed
#   GSET_NEED_MISSING  (via gset_need) a tool is absent → NOTRUN
#   gset_dig/gset_val  content digests (graded vs bare metal) / recorded values (not graded)
# Binaries and data live in $GSET_HOME (provisioned into the guest image by provision.sh — the
# bare-metal side runs the SAME files through hostroot.sh).
GB=$GSET_HOME/gfxbin
FF=$GSET_HOME/ff/bin/ffmpeg
BL=$GSET_HOME/blender/blender
GM=$GSET_HOME/glmark2
gset_items(){ echo "pv_validate_vk pv_validate_gl pv_vk_rt_ext vk_ofa pv_vk_create_device vk_info vkpeak egl_offscreen \
gl_micro pv_fbo_formats pv_egl_dmabuf_export pv_dmabuf_import pv_xiso_sharing pv_signal_restart_export \
pv_gbm_egl_import pv_gbmprobe_gbmshot weston_headless weston_client_diff sway_screencap glmark2 \
pv_nvenc_nvdec_fps video_nvenc_nvdec geekbench_vulkan blender_opendata \
vk_compute vk_render egl_render gl_info glx_vgl ff_cuda ff_vulkan ff_opencl ff_placebo \
blender_cycles_cuda blender_cycles_optix blender_eevee blender_workbench blender_eevee_vulkan"; }
gset_timeout(){ case $1 in glmark2|blender_opendata|geekbench_vulkan) echo 1800 ;;
                           vkpeak|video_*|blender_*|pv_nvenc*) echo 900 ;; *) echo 300 ;; esac; }
# a probe's CHECK|name|STATUS|detail lines (nvkvm-pv validate.sh protocol) → name=STATUS, sorted
pv_checks(){ grep -a '^CHECK|' "$1" | awk -F'|' '{print $2"="$3}' | LC_ALL=C sort; }
png_dig(){ "$FF" -hide_banner -loglevel error -i "$1" -f rawvideo -pix_fmt rgba - 2>/dev/null | md5sum | cut -c1-16; }
xdg_up(){ export XDG_RUNTIME_DIR=$W/xdg; mkdir -p "$XDG_RUNTIME_DIR"; chmod 700 "$XDG_RUNTIME_DIR"; }
# ★ nvkvm-pv's guests had NO Mesa (setup_guest.sh:350-352), so a Vulkan app's device 0 was the NVIDIA GPU.
#   This image carries Mesa's ICDs (weston/sway pull them in), and lavapipe can enumerate first — the
#   probes that take device 0 would then test lavapipe on BOTH sides. Restrict the loader to NVIDIA's ICD.
nv_icd_only(){ local j; for j in /etc/vulkan/icd.d/nvidia_icd.json /usr/share/vulkan/icd.d/nvidia_icd.json; do
    [ -f "$j" ] && { export VK_DRIVER_FILES=$j VK_ICD_FILENAMES=$j; echo "vulkan ICD: $j only"; return 0; }; done
    echo "GSET_FAIL no NVIDIA Vulkan ICD json in the image"; return 1; }

# ══ nvkvm-pv H1 + H4: validate.sh phase 3 (Vulkan), probe extracted verbatim ═══════════════════════
# criterion (validate.sh): vendor 0x10DE and not a software device; data[i] == i*3+7 over N=4096;
# vk_import_host_ptr imports 2 MiB (SKIP when the extension is not advertised).
item_pv_validate_vk(){
    nv_icd_only || return 0
    gset_need "$GB/vk_probe" || return 0
    base64 -d < "$GSET_BIN/src/nvkvmpv/comp.spv.b64" > comp.spv
    "$GB/vk_probe" "$W/comp.spv" > vk.out 2>&1; rc=$?; cat vk.out
    pv_checks vk.out | tee checks.txt
    gset_dig checks "$(md5sum < checks.txt | cut -c1-16)"
    gset_dig vk_import_host_ptr "$(grep '^vk_import_host_ptr=' checks.txt | cut -d= -f2)"
    n=$(grep -c '=PASS$' checks.txt); nf=$(grep -cv -E '=PASS$|^vk_import_host_ptr=SKIP$' checks.txt)
    [ "$n" -ge 5 ] && [ "$nf" -eq 0 ] && echo GSET_OK || echo "GSET_FAIL vk_probe rc=$rc pass=$n other=$(grep -v '=PASS$' checks.txt | tr '\n' ' ')"
}
# ══ nvkvm-pv H2: validate.sh phase 4 (GL): EGL device platform, GLES2 draw, two probe pixels ═══════
# criterion: renderer names NVIDIA; (16,16) ≈ (255,127/128,0,255) AND (48,48) = (0,0,0,255).
item_pv_validate_gl(){
    gset_need "$GB/gl_probe" || return 0
    "$GB/gl_probe" > gl.out 2>&1; rc=$?; cat gl.out
    pv_checks gl.out | tee checks.txt
    gset_dig checks "$(md5sum < checks.txt | cut -c1-16)"
    gset_dig pixels "$(grep -a '^CHECK|gl_draw_pixel_check|' gl.out | cut -d'|' -f4 | md5sum | cut -c1-16)"
    [ "$(grep -c '=PASS$' checks.txt)" -eq 5 ] && echo GSET_OK || echo "GSET_FAIL gl_probe rc=$rc $(grep -v '=PASS$' checks.txt | tr '\n' ' ')"
}
# ══ nvkvm-pv H3: the RDR2 check — vkCreateDevice with 7 RT/NVX extensions (tests/repro, candidate rev) ═
item_pv_vk_rt_ext(){
    nv_icd_only || return 0
    gset_need "$GB/vk_device_extensions" || return 0
    "$GB/vk_device_extensions" VK_KHR_acceleration_structure VK_KHR_ray_query VK_KHR_ray_tracing_pipeline \
        VK_NV_ray_tracing VK_NV_optical_flow VK_NV_cuda_kernel_launch VK_NVX_binary_import > rt.out 2>&1; rc=$?
    cat rt.out; gset_dig result "$(grep -m1 '^RESULT' rt.out | tr ' ' _)"
    [ $rc -eq 0 ] && grep -q '^RESULT: 0 ' rt.out && echo GSET_OK || echo "GSET_FAIL rc=$rc $(grep -m1 RESULT rt.out)"
}
# ══ EXTRA (the engine behind H3's VK_NV_optical_flow): OFA does real work, with a known answer ═══════
# vk_ofa: a frame and its (5,3)-px shifted copy through vkCmdOpticalFlowExecuteNV; the median flow must
# recover the shift and the whole flow field is digested vs bare metal. Built on first use with the
# image's own gcc/headers into $GSET_OUT (so both sides run one binary).
item_vk_ofa(){
    nv_icd_only || return 0
    local B=$GSET_OUT/vk_ofa
    [ -x "$B" ] || gcc -O2 -o "$B" "$GSET_BIN/src/vk_ofa.c" -lvulkan -lm || { echo "GSET_FAIL vk_ofa build"; return 0; }
    "$B" > ofa.txt 2>&1; rc=$?; cat ofa.txt
    gset_dig flow_field "$(sed -n 's/^OFA_HASH //p' ofa.txt)"
    gset_dig median "$(sed -n 's/^OFA_MEDIAN_[XY] //p' ofa.txt | tr '\n' ,)"
    gset_val good_frac "$(sed -n 's/^OFA_GOOD_FRAC //p' ofa.txt)"
    [ $rc -eq 0 ] && grep -q '^OFA_OK' ofa.txt && echo GSET_OK || echo "GSET_FAIL rc=$rc $(grep -m1 OFA_FAIL ofa.txt)"
}
# ══ nvkvm-pv H22: vkCreateDevice (tests/repro/vk_create_device.c) ═════════════════════════════════
item_pv_vk_create_device(){
    nv_icd_only || return 0
    gset_need "$GB/vk_create_device" || return 0
    "$GB/vk_create_device" > vcd.out 2>&1; rc=$?; cat vcd.out
    grep -q 'RESULT: vkCreateDevice rc=0 OK' vcd.out && echo GSET_OK || echo "GSET_FAIL rc=$rc $(grep -m1 RESULT vcd.out)"
}
# ══ nvkvm-pv H6 (+EXTRA capability digests): vulkaninfo names the NVIDIA device ══════════════════
# criterion (graphics_remote.sh): deviceName matches nvidia|rtx|geforce.
item_vk_info(){
    nv_icd_only || return 0
    gset_need vulkaninfo python3 || return 0
    vulkaninfo --summary > summary.txt 2>&1; rc=$?
    grep -E 'deviceName|driverVersion|apiVersion|driverID|conformanceVersion' summary.txt
    dev=$(grep -m1 -E 'deviceName.*NVIDIA' summary.txt | sed 's/.*= //')
    gset_dig device "$(echo "$dev" | tr ' ' _)"
    # the capability profile: vulkan-tools writes VP_VULKANINFO_*.json per GPU for --json
    vulkaninfo --json=0 >/dev/null 2>json.err; j=$(ls VP_VULKANINFO_*.json 2>/dev/null | head -1)
    if [ -n "$j" ]; then python3 "$GSET_BIN/vkprofile_dig.py" "$j"; else echo "GSET_FAIL no vulkaninfo profile ($(head -c 200 json.err))"; fi
    [ $rc -eq 0 ] && echo "$dev" | grep -qiE 'nvidia|rtx|geforce' && echo GSET_OK || echo "GSET_FAIL vulkaninfo rc=$rc dev=[$dev]"
}
# ══ nvkvm-pv H7: vkpeak (nihui/vkpeak 20250531 release binary) ════════════════════════════════════
# criterion (graphics_remote.sh): an fp32-scalar GFLOPS figure. Here: every figure printed, the test
# SET (and which tests read 0) equal to bare metal; the numbers are recorded, not graded.
item_vkpeak(){
    nv_icd_only || return 0
    gset_need "$GSET_HOME/vkpeak/vkpeak" || return 0
    "$GSET_HOME/vkpeak/vkpeak" 0 > vkpeak.txt 2>&1; rc=$?; cat vkpeak.txt
    grep -E '^[a-z0-9-]+ += +[0-9.]+' vkpeak.txt | while read -r n _ v u; do gset_val "$n" "$v$u"; done
    gset_dig testset "$(grep -oE '^[a-z0-9-]+ += +[0-9.]+' vkpeak.txt | awk '{print $1, ($3+0>0)?"nz":"zero"}' | md5sum | cut -c1-16)"
    fp32=$(sed -n 's/^fp32-scalar *= *\([0-9.]*\).*/\1/p' vkpeak.txt)
    [ $rc -eq 0 ] && [ -n "$fp32" ] && awk "BEGIN{exit !($fp32>0)}" && echo GSET_OK || echo "GSET_FAIL vkpeak rc=$rc fp32=[$fp32]"
}
# ══ nvkvm-pv H8: egl_offscreen (verbatim + a whole-framebuffer digest) ════════════════════════════
# criterion (egl_offscreen.c): no GL error. Here also: the framebuffer digest equals bare metal.
item_egl_offscreen(){
    gset_need "$GB/egl_offscreen" || return 0
    "$GB/egl_offscreen" 2000 20000 > eo.txt 2>&1; rc=$?; cat eo.txt
    gset_dig fb "$(sed -n 's/^EGLOFF_FB_MD5 //p' eo.txt)"
    gset_val Mtri_s "$(sed -n 's/^METRIC egl_gl_Mtri_s //p' eo.txt)"
    grep -q '^CHECK egl_gl_Mtri_s ok' eo.txt && grep -qi 'renderer:.*NVIDIA' eo.txt && echo GSET_OK || echo "GSET_FAIL rc=$rc $(grep -m1 -E 'CHECK|renderer' eo.txt)"
}
# ══ nvkvm-pv H13: GL micro-probes — criterion: `CHECK ok` (no GL error); metrics recorded ═════════
item_gl_micro(){
    gset_need "$GB/gl_decompose" "$GB/gl_drawrate" "$GB/gl_finishrate" || return 0
    local bad=0 p
    for p in gl_decompose gl_drawrate gl_finishrate; do
        "$GB/$p" > $p.out 2>&1; rc=$?; sed "s/^/$p: /" $p.out
        grep '^METRIC' $p.out | while read -r _ k v; do gset_val "$p.$k" "$v"; done
        if [ $p = gl_decompose ]; then grep -q '^CHECK gl_decompose ok' $p.out || bad=1; else [ $rc -eq 0 ] || bad=1; fi
    done
    [ $bad -eq 0 ] && echo GSET_OK || echo "GSET_FAIL a probe failed"
}
# ══ nvkvm-pv H20: 5 FBO colour formats — criterion: `SUMMARY| 0/5 configurations incomplete` ══════
item_pv_fbo_formats(){
    gset_need "$GB/fbo_formats_probe" || return 0
    "$GB/fbo_formats_probe" > fbo.out 2>&1; rc=$?; cat fbo.out
    gset_dig summary "$(grep -a -E '^(SUMMARY|CASE|FORMAT)' fbo.out | md5sum | cut -c1-16)"
    grep -q '^SUMMARY| 0/5 configurations incomplete' fbo.out && echo GSET_OK || echo "GSET_FAIL rc=$rc $(grep -m1 SUMMARY fbo.out)"
}
# ══ nvkvm-pv H16: GL texture → EGLImage → eglExportDMABUFImageMESA — `RESULT: PASS` ═════════════════
item_pv_egl_dmabuf_export(){
    gset_need "$GB/egl_dmabuf_export_probe" || return 0
    "$GB/egl_dmabuf_export_probe" > exp.out 2>&1; rc=$?; cat exp.out
    grep -q 'RESULT: PASS' exp.out && echo GSET_OK || echo "GSET_FAIL rc=$rc $(grep -m1 RESULT exp.out)"
}
# ══ nvkvm-pv H17: same-process PRIME re-import as an EGLImage — `RESULT import=OK` (linear + block-linear)
item_pv_dmabuf_import(){
    gset_need "$GB/dmabuf_import_probe" || return 0
    local bad=0 n m
    for n in /dev/dri/card0 /dev/dri/renderD128; do for m in linear blocklinear; do
        "$GB/dmabuf_import_probe" "$n" "$m" > imp.out 2>&1; sed "s|^|$n $m: |" imp.out
        grep -q 'RESULT import=OK' imp.out || bad=1
        gset_dig "$(basename $n)_$m" "$(grep -m1 -o 'RESULT import=[A-Z]*' imp.out | tr ' ' _)"
    done; done
    [ $bad -eq 0 ] && echo GSET_OK || echo "GSET_FAIL an import failed"
}
# ══ nvkvm-pv H18: two processes share a buffer — `RESULT import=OK` and `RESULT xiso_bytes=PASS … quadrants=4`
item_pv_xiso_sharing(){
    gset_need "$GB/xiso_import_probe" "$GB/xiso_bytes_probe" || return 0
    "$GB/xiso_import_probe" "$W/xi.sock" > xi.out 2>&1; cat xi.out
    "$GB/xiso_bytes_probe" "$W/xb.sock" > xb.out 2>&1; cat xb.out
    gset_dig bytes "$(grep -m1 -o 'RESULT xiso_bytes=[A-Z]* checked=[0-9]* quadrants=[0-9]*' xb.out | tr ' ' _)"
    grep -q 'RESULT import=OK' xi.out && grep -q 'RESULT xiso_bytes=PASS' xb.out && echo GSET_OK \
        || echo "GSET_FAIL $(grep -m1 RESULT xi.out) $(grep -m1 RESULT xb.out)"
}
# ══ nvkvm-pv H19: 300 dma-buf exports under a fast SIGALRM — exit 0 = every export succeeded ════════
item_pv_signal_restart_export(){
    gset_need "$GB/signal_restart_export" || return 0
    "$GB/signal_restart_export" 300 > sig.out 2>&1; rc=$?; cat sig.out
    gset_dig result "$(grep -m1 -oE '[0-9]+/[0-9]+ dma-buf exports succeeded' sig.out | tr ' ' _)"
    [ $rc -eq 0 ] && echo GSET_OK || echo "GSET_FAIL rc=$rc $(grep -m1 signal_restart_export sig.out)"
}
# ══ nvkvm-pv H21: GBM bo → dma-buf → EGLImage → FBO (+ the NATIVE_PIXMAP sweep) ═══════════════════
# criterion: "host and guest must agree" (the NATIVE_PIXMAP import fails on BOTH) — so the grade here IS
# the host/guest equality of the whole normalized transcript; the item's own floor is its dma-buf path.
item_pv_gbm_egl_import(){
    gset_need "$GB/gbm_egl_import" || return 0
    local n
    for n in renderD128 card0; do
        "$GB/gbm_egl_import" /dev/dri/$n $n > gei_$n.out 2>&1; echo "rc=$?" >> gei_$n.out; cat gei_$n.out
        gset_dig "$n" "$(sed -E 's/0x[0-9a-f]{8,}/PTR/g; s/fd=[0-9]+/fd=N/g' gei_$n.out | md5sum | cut -c1-16)"
    done
    grep -q '\] ok ' gei_renderD128.out && echo GSET_OK || echo "GSET_FAIL no ok line on renderD128"
}
# ══ nvkvm-pv H23: EGL on the GBM platform on card0 (gbmprobe: `RESULT GPU-OK`) + gbmshot's PPM ═════
item_pv_gbmprobe_gbmshot(){
    gset_need "$GB/gbmprobe" "$GB/gbmshot" || return 0
    "$GB/gbmprobe" /dev/dri/card0 > gp.out 2>&1; cat gp.out
    "$GB/gbmshot" /dev/dri/card0 "$W/head.ppm" > gs.out 2>&1; rc=$?; cat gs.out
    gset_dig ppm "$(gset_md5 "$W/head.ppm")"
    grep -q 'RESULT GPU-OK' gp.out && [ -s "$W/head.ppm" ] && echo GSET_OK || echo "GSET_FAIL $(grep -m1 -E 'RESULT|FAIL' gp.out) gbmshot rc=$rc"
}
# ══ nvkvm-pv H14: headless weston (GL renderer) + es2gears_wayland + weston-screenshooter ══════════
# criterion (run_headless_compositor.sh): weston alive, GL renderer lines in its log, a screenshot.
# EXTRA, same item: a DETERMINISTIC composite (a solid background + wl_scene's static frame), digested.
weston_up(){ # socket [ini]
    xdg_up
    weston --backend=headless --renderer=gl --width=1280 --height=720 --idle-time=0 --socket="$1" \
           ${2:+--config="$2"} --debug --log="$W/weston_$1.log" > "weston_$1.out" 2>&1 & WPID=$!
    local i; for i in $(seq 1 100); do [ -S "$XDG_RUNTIME_DIR/$1" ] && break; sleep 0.2; done
    sleep 2; kill -0 $WPID 2>/dev/null || { echo "GSET_FAIL weston died: $(tail -5 weston_$1.log weston_$1.out | tr '\n' ' ')"; return 1; }
    grep -iE 'renderer|EGL vendor|GL version|GL renderer|GL vendor' "weston_$1.log" | head -8
}
shoot(){ # socket dest.png — weston-screenshooter into $W, moved to dest
    rm -f "$W"/wayland-screenshot*.png
    ( cd "$W" && WAYLAND_DISPLAY=$1 timeout 10 weston-screenshooter ) > shoot.out 2>&1
    local s; s=$(ls -t "$W"/wayland-screenshot*.png 2>/dev/null | head -1); [ -n "$s" ] && mv "$s" "$2"
}
item_weston_headless(){
    gset_need weston weston-screenshooter es2gears_wayland "$GB/wl_scene" "$FF" || return 0
    weston_up gset-a || return 0
    WAYLAND_DISPLAY=gset-a timeout 6 es2gears_wayland > gears.out 2>&1 & sleep 6
    shoot gset-a "$W/desktop.png"; ls -l "$W/desktop.png" 2>&1; cat shoot.out
    grep -iE 'GL_RENDERER|fps|error' gears.out | head -4
    kill $WPID 2>/dev/null; wait $WPID 2>/dev/null
    ok_pv=0; [ -s "$W/desktop.png" ] && grep -qi 'renderer.*NVIDIA' "$W/weston_gset-a.log" && ok_pv=1
    # EXTRA — the deterministic composite: no panel, no animations, a solid background
    printf '[core]\nidle-time=0\n[shell]\npanel-position=none\nbackground-color=0xff203040\nlocking=false\nanimation=none\nstartup-animation=none\nclose-animation=none\nfocus-animation=none\n' > "$W/det.ini"
    weston_up gset-b "$W/det.ini" || return 0
    WAYLAND_DISPLAY=gset-b "$GB/wl_scene" 60 > scene.txt 2>&1 & SP=$!
    for i in $(seq 1 150); do grep -q 'WL_SCENE_READY\|WL_SCENE_FAIL' scene.txt && break; sleep 0.2; done
    sleep 1; shoot gset-b "$W/composite.png"; cat scene.txt
    kill $SP 2>/dev/null; wait $SP 2>/dev/null; kill $WPID 2>/dev/null; wait $WPID 2>/dev/null
    gset_dig client_readback "$(sed -n 's/^WL_SCENE_HASH //p' scene.txt)"
    gset_dig composite "$( [ -s "$W/composite.png" ] && png_dig "$W/composite.png" || echo NOSHOT)"
    gset_val renderer "$(grep -m1 -oiE 'GL renderer: .*' "$W/weston_gset-a.log" | tr ' ' _)"
    [ $ok_pv = 1 ] && grep -q '^WL_SCENE_READY' scene.txt && grep -q '^WL_SCENE_RENDERER.*NVIDIA' scene.txt && [ -s "$W/composite.png" ] \
        && echo GSET_OK || echo "GSET_FAIL pv_criterion=$ok_pv scene_ready=$(grep -c WL_SCENE_READY scene.txt) $(grep -m1 WL_SCENE_FAIL scene.txt)"
}
# ══ nvkvm-pv H15: the client-pixel differential on headless weston (verify_client_window_differential.sh)
# criterion: A (no client) ≠ B (glmark2-wayland running) — the client drew; B ≠ C (1 s later) — it animates.
item_weston_client_diff(){
    gset_need weston weston-screenshooter "$GM/bin/glmark2-wayland" python3 || return 0
    weston_up gset-c || return 0
    shoot gset-c "$W/capA.png"
    WAYLAND_DISPLAY=gset-c GLMARK2_DATA_PATH=$GM/share/glmark2 "$GM/bin/glmark2-wayland" --run-forever > client.out 2>&1 & CP=$!
    sleep 12; kill -0 $CP 2>/dev/null && echo "client: ALIVE" || echo "client: EXITED EARLY"
    shoot gset-c "$W/capB.png"; sleep 1; shoot gset-c "$W/capC.png"
    kill $CP 2>/dev/null; wait $CP 2>/dev/null; kill $WPID 2>/dev/null; wait $WPID 2>/dev/null
    grep -iE 'GL_RENDERER' client.out | head -1
    python3 "$GSET_BIN/pngdiff.py" "$W/capA.png" "$W/capB.png" "$W/capC.png" | tee diff.out
    ab=$(sed -n 's/^AB_DIFF //p' diff.out); bc=$(sed -n 's/^BC_DIFF //p' diff.out)
    [ "${ab:-0}" -gt 0 ] && [ "${bc:-0}" -gt 0 ] && grep -qi 'NVIDIA' client.out && echo GSET_OK || echo "GSET_FAIL A-B=${ab:-?} B-C=${bc:-?}"
}
# ══ nvkvm-pv H24: headless sway + wlr-screencopy into a client dma-buf (run_screencap.sh + wlr_screencap.c)
# criterion: `RESULT captured=N/N`. EXTRA, same item: grim (SHM) + wlr_screencap's PPM of a DETERMINISTIC
# client (wl_scene on a solid background), digested vs bare metal.
# ⊘ sway refuses the proprietary NVIDIA kernel module unless --unsupported-gpu (measured smoke1: "Proprietary
#   Nvidia drivers are NOT supported"). nvkvm-pv's Mode-1 guests had no nvidia.ko, so sway never saw one.
sway_up(){ # config-extra (lines appended)
    xdg_up
    printf 'output HEADLESS-1 mode 1280x720@60Hz position 0 0 bg #203040 solid_color\ndefault_border none\nfocus_follows_mouse no\nxwayland disable\n%s\n' "${1:-}" > "$W/sway.conf"
    WLR_BACKENDS=headless WLR_RENDERER=gles2 WLR_NO_HARDWARE_CURSORS=1 WLR_LIBINPUT_NO_DEVICES=1 \
        WLR_RENDER_DRM_DEVICE=/dev/dri/renderD128 sway --unsupported-gpu -c "$W/sway.conf" > "$W/sway.log" 2>&1 & SWPID=$!
    local i; for i in $(seq 1 100); do ls "$XDG_RUNTIME_DIR"/wayland-? >/dev/null 2>&1 && break; sleep 0.2; done
    sleep 3; kill -0 $SWPID 2>/dev/null || { echo "GSET_FAIL sway died: $(tail -5 "$W/sway.log" | tr '\n' ' ')"; return 1; }
    export WAYLAND_DISPLAY=$(basename "$(ls "$XDG_RUNTIME_DIR"/wayland-? | head -1)")
    grep -iE 'GL_RENDERER|GL_VENDOR|EGL|renderer' "$W/sway.log" | head -6
}
item_sway_screencap(){
    gset_need sway es2gears_wayland grim "$GB/wlr_screencap" "$GB/wl_scene" "$FF" || return 0
    sway_up "exec es2gears_wayland" || return 0
    sleep 2; "$GB/wlr_screencap" 300 > cap1.out 2>&1; cat cap1.out
    kill $SWPID 2>/dev/null; wait $SWPID 2>/dev/null; sleep 1
    gset_val capture_fps "$(grep -o 'captured=[0-9/]*  *[0-9.]* fps' cap1.out | awk '{print $2}')"
    ok_pv=0; grep -q 'RESULT captured=300/300' cap1.out && ok_pv=1
    # EXTRA — deterministic client, captured over SHM (grim) and into a dma-buf (wlr_screencap --ppm)
    sway_up "exec $GB/wl_scene 60 > $W/scene.txt 2>&1" || return 0
    for i in $(seq 1 150); do grep -q 'WL_SCENE_READY\|WL_SCENE_FAIL' scene.txt 2>/dev/null && break; sleep 0.2; done
    # ⊘ the dma-buf capture's CONTENT cannot be read back by wlr_screencap on NVIDIA: its --ppm path is
    #   gbm_bo_map, which NVIDIA refuses for the block-linear capture bo ON BARE METAL ("content is GPU-only",
    #   shake2). So: the zero-copy path is graded by nvkvm-pv's own `captured=N/N`, the pixels by grim (the
    #   same wlr-screencopy protocol into an SHM buffer).
    sleep 1; grim -t png "$W/grim.png" > grim.out 2>&1; "$GB/wlr_screencap" 10 > cap2.out 2>&1
    cat scene.txt cap2.out grim.out 2>/dev/null
    kill $SWPID 2>/dev/null; wait $SWPID 2>/dev/null
    gset_dig client_readback "$(sed -n 's/^WL_SCENE_HASH //p' scene.txt)"
    gset_dig composite_shm "$( [ -s grim.png ] && png_dig grim.png || echo NOSHOT)"
    [ $ok_pv = 1 ] && grep -q '^WL_SCENE_READY' scene.txt && [ -s grim.png ] && grep -q 'RESULT captured=10/10' cap2.out && echo GSET_OK \
        || echo "GSET_FAIL pv_criterion=$ok_pv($(grep -m1 RESULT cap1.out)) scene_ready=$(grep -c WL_SCENE_READY scene.txt 2>/dev/null) grim=$(stat -c %s grim.png 2>/dev/null) cap2=$(grep -m1 RESULT cap2.out)"
}
# ══ nvkvm-pv H12: glmark2 2023.01 (built from source, same binary both sides) on headless weston ══════
# nvkvm-pv graded nothing here (metrics only). Here: both configurations complete with a Score and the
# same per-scene list as bare metal, AND `--validate` (glmark2's own reference-image check per scene)
# gives the same per-scene verdicts as bare metal. Scores are recorded, not graded.
item_glmark2(){
    gset_need weston "$GM/bin/glmark2-wayland" || return 0
    export GLMARK2_DATA_PATH=$GM/share/glmark2
    weston_up gset-g || return 0
    local bad=0 cfg a s
    for cfg in suite_offscreen suite_windowed; do
        a=""; [ $cfg = suite_offscreen ] && a=--off-screen
        WAYLAND_DISPLAY=gset-g "$GM/bin/glmark2-wayland" $a > gm_$cfg.txt 2>&1; rc=$?
        s=$(grep -E '^\s*glmark2 Score:' gm_$cfg.txt | grep -oE '[0-9]+' | tail -1)
        echo "glmark2 $cfg rc=$rc score=${s:-none}"; gset_val ${cfg}_score "${s:-none}"
        gset_dig ${cfg}_scenes "$(grep -oE '^\[[a-z0-9:=.,-]+\][^:]*: FPS:' gm_$cfg.txt | md5sum | cut -c1-16)"
        [ $rc -eq 0 ] && [ -n "$s" ] || bad=1
    done
    WAYLAND_DISPLAY=gset-g "$GM/bin/glmark2-wayland" --validate > gm_validate.txt 2>&1
    grep -E 'Validation' gm_validate.txt | head -40
    gset_dig validate "$(grep -E 'Validation:' gm_validate.txt | md5sum | cut -c1-16)"
    gset_val validate_counts "S$(grep -c 'Validation: Success' gm_validate.txt)/F$(grep -c 'Validation: Failure' gm_validate.txt)/U$(grep -c 'Validation: Unknown' gm_validate.txt)"
    grep -m1 -E 'GL_RENDERER' gm_suite_offscreen.txt
    kill $WPID 2>/dev/null; wait $WPID 2>/dev/null
    grep -q 'GL_RENDERER.*NVIDIA' gm_suite_offscreen.txt || bad=1
    [ $bad -eq 0 ] && echo GSET_OK || echo "GSET_FAIL glmark2 (see scores/rc above)"
}
# ══ nvkvm-pv H9 + H11: graphics_remote.sh's two ffmpeg rows, verbatim commands, the static ffmpeg ════
# criterion: an NVENC fps= and an NVDEC (h264_cuvid) speed= figure (a missing metric = FAIL).
item_pv_nvenc_nvdec_fps(){
    gset_need "$FF" || return 0
    "$FF" -y -hide_banner -f lavfi -i testsrc=size=1920x1080:rate=30:duration=10 -c:v libx264 -preset ultrafast -threads 1 -pix_fmt yuv420p src.mp4 > src.log 2>&1
    enc=$(timeout 90 "$FF" -y -hide_banner -benchmark -f lavfi -i testsrc=size=1920x1080:rate=30:duration=20 \
        -c:v h264_nvenc -preset p1 -f null - 2>&1 | tr '\r' '\n' | grep -oE 'fps= *[0-9.]+' | tail -1 | grep -oE '[0-9.]+')
    dec=$(timeout 90 "$FF" -y -hide_banner -benchmark -c:v h264_cuvid -i src.mp4 -f null - 2>&1 | tr '\r' '\n' | tee dec.log \
        | grep -oE 'speed= *[0-9.]+' | tail -1 | grep -oE '[0-9.]+')
    nfr=$(grep -oE 'frame= *[0-9]+' dec.log | tail -1 | grep -oE '[0-9]+')
    echo "nvenc_h264_fps=${enc:-none} nvdec_h264_speed=${dec:-none} frames=${nfr:-0}"
    gset_val nvenc_h264_fps "${enc:-none}"; gset_val nvdec_h264_speed "${dec:-none}"; gset_dig nvdec_frames "${nfr:-0}"
    [ -n "$enc" ] && [ -n "$dec" ] && [ "${nfr:-0}" = 300 ] && echo GSET_OK || echo "GSET_FAIL enc=[$enc] dec=[$dec] frames=[$nfr]"
}
# ══ nvkvm-pv H10/H11 (content): V3_VIDEO_ENGINES.md §5's lane — md5 of every stream and decoded frame set
item_video_nvenc_nvdec(){
    gset_need "$FF" || return 0
    VIDEO_COMPACT=0 bash "$GSET_BIN/video_lane.sh" "$FF" "$W/vl" > vl.txt 2>&1; rc=$?
    cat vl/results.txt 2>/dev/null
    sed -n 's/^\([a-z0-9_.]*\) md5=\([0-9a-f]*\).*/\1 \2/p' vl/results.txt | while read -r k v; do gset_dig "$k" "$v"; done
    grep -o 'psnr_[a-z0-9]* PSNR y:[^ ]*' vl/results.txt | while read -r k _ y; do gset_val "$k" "$y"; done
    gset_val nvenc_1080p "$(sed -n 's/^perf_nvenc_1080p fps=//p' vl/results.txt | tr -d ' ')"
    bad=$(grep -cE ' rc=[1-9]|bitexact_vs_sw=NO|md5=EMPTY' vl/results.txt)
    grep -q '^VIDEO_LANE_END' vl/results.txt && [ "$bad" -eq 0 ] && echo GSET_OK || echo "GSET_FAIL video lane rc=$rc bad_lines=$bad"
}
# ══ nvkvm-pv H5: Geekbench GPU, Vulkan backend (nvkvm-pv: GB7, composite vs bare metal, zero check) ══
item_geekbench_vulkan(){
    nv_icd_only || return 0
    local gbx; gbx=$(ls "$GSET_HOME"/geekbench/geekbench[0-9]* 2>/dev/null | grep -v '\.' | head -1)
    gset_need "${gbx:-/nonexistent/geekbench}" || return 0
    # ⊘ Geekbench 7.0.0 FREE: no --no-upload, and NO scores on the console — they exist only on the uploaded
    #   result page, which sits behind a Cloudflare challenge (measured shake2). So nvkvm-pv's criterion
    #   (composite vs bare metal, no workload at 0) is NOT reproducible without a Pro licence; graded here:
    #   every workload ran, the upload succeeded, no error line, and (by the suite) no host Xid. The result
    #   URL is recorded for a human to read the scores; the claim key is stripped.
    ( cd "$(dirname "$gbx")" && "$gbx" --gpu Vulkan ) > gb.txt 2>&1; rc=$?
    sed -E 's/(claim\?key=)[0-9a-zA-Z]+/\1<stripped>/' gb.txt | grep -vE '^\s*$' | tail -40
    grep -oE '^  Running .*' gb.txt | sed 's/^  Running //' > wl.txt
    gset_dig workloads "$(md5sum < wl.txt | cut -c1-16)"; gset_val workload_count "$(wc -l < wl.txt)"
    gset_val result_url "$(grep -m1 -oE 'https://browser.geekbench.com/v7/gpu/[0-9]+' gb.txt)"
    nerr=$(grep -ciE 'error|fail|abort|exception' gb.txt)
    [ $rc -eq 0 ] && [ "$(wc -l < wl.txt)" -ge 11 ] && grep -q 'Upload succeeded' gb.txt && [ "$nerr" -eq 0 ] && echo GSET_OK \
        || echo "GSET_FAIL geekbench rc=$rc workloads=$(wc -l < wl.txt) upload=$(grep -c 'Upload succeeded' gb.txt) error_lines=$nerr"
}
# ══ nvkvm-pv H25: Blender Open Data 4.5.0 (benchmark-launcher-cli 3.3.0), Cycles on CUDA ═════════════
# criterion: the launcher's render_time_no_sync / total per scene (recorded). Here: all scenes complete.
item_blender_opendata(){
    local L=$GSET_HOME/bod/benchmark-launcher-cli
    gset_need "$L" || return 0
    HOME=$GSET_HOME/bod "$L" benchmark --blender-version 4.5.0 --device-type CUDA --json monster junkshop classroom > bod.json 2> bod.err; rc=$?
    tail -20 bod.err
    python3 - bod.json <<'PY' | tee bod.txt
import json,sys
try: d=json.load(open(sys.argv[1]))
except Exception as e: print("BOD_PARSE_FAIL", e); sys.exit(0)
n=0
for r in d:
    s=r.get("scene",{}).get("label"); st=r.get("stats",{})
    if st.get("total_render_time"): n+=1
    print("BOD", s, "no_sync=%s total=%s spm=%s" % (st.get("render_time_no_sync"), st.get("total_render_time"), st.get("samples_per_minute")))
print("BOD_SCENES", n)
PY
    grep '^BOD ' bod.txt | while read -r _ s rest; do gset_val "$s" "$(echo "$rest" | tr ' ' ,)"; done
    n=$(sed -n 's/^BOD_SCENES //p' bod.txt)
    [ $rc -eq 0 ] && [ "${n:-0}" = 3 ] && echo GSET_OK || echo "GSET_FAIL launcher rc=$rc scenes=${n:-0}: $(tail -2 bod.err | tr '\n' ' ')"
}

# ══ EXTRA — the v3-gfx lane (V3_HEADLESS_GRAPHICS.md §6), CPU-reference-checked renders ═════════════
item_vk_compute(){
    nv_icd_only || return 0
    gset_need "$GB/vk_gfx" || return 0
    "$GB/vk_gfx" compute 2>&1 | tee vkc.txt
    gset_dig VKC_HASH "$(sed -n 's/^VKC_HASH=//p' vkc.txt)"
    grep -q '^VKC_BAD=0$' vkc.txt && echo GSET_OK || echo "GSET_FAIL $(grep -m1 -E 'VKC_BAD|VK_FAIL' vkc.txt)"
}
item_vk_render(){
    nv_icd_only || return 0
    gset_need "$GB/vk_gfx" || return 0
    "$GB/vk_gfx" render "$W/vkr" 2>&1 | tee vkr.txt
    for k in A Z B; do gset_dig "VKR_HASH_$k" "$(sed -n "s/^VKR_HASH_$k=//p" vkr.txt)"; done
    grep -q '^VKR_REF_FAILS=0$' vkr.txt && echo GSET_OK || echo "GSET_FAIL $(grep -m1 -E 'REF_FAILS|VK_FAIL' vkr.txt)"
}
item_egl_render(){
    gset_need "$GB/egl_gfx" || return 0
    "$GB/egl_gfx" 2>&1 | tee egl.txt
    for k in A Z B; do gset_dig "EGLR_HASH_$k" "$(sed -n "s/^EGLR_HASH_$k=//p" egl.txt)"; done
    grep -q '^EGLR_REF_FAILS=0$' egl.txt && grep -q '^EGL_RENDERER=.*NVIDIA' egl.txt && echo GSET_OK \
        || echo "GSET_FAIL $(grep -m1 -E 'REF_FAILS|EGL_FAIL|EGL_RENDERER' egl.txt)"
}
item_gl_info(){  # the GL implementation (strings, every extension, ~70 limits) on the EGL device platform
    gset_need "$GB/gl_limits" || return 0
    "$GB/gl_limits" > limits.txt 2>&1; rc=$?; head -4 limits.txt
    gset_dig gl_renderer "$(sed -n 's/^GL_RENDERER=//p' limits.txt | tr ' ' _)"
    gset_dig gl_extensions "$(grep '^EXT ' limits.txt | LC_ALL=C sort | md5sum | cut -c1-16)"
    gset_dig gl_limits "$(grep '^LIM ' limits.txt | LC_ALL=C sort | md5sum | cut -c1-16)"
    gset_val gl_ext_count "$(grep -c '^EXT ' limits.txt)"
    [ $rc -eq 0 ] && grep -q '^GL_RENDERER=.*NVIDIA' limits.txt && echo GSET_OK || echo "GSET_FAIL gl_limits rc=$rc"
}
item_glx_vgl(){  # "headless Xorg" as users mean it: Xvfb + VirtualGL (EGL back end), GLX in an X window
    gset_need Xvfb vglrun glxinfo "$GB/glx_gfx" || return 0
    Xvfb :17 -screen 0 1280x720x24 -nolisten tcp > xvfb.log 2>&1 & xp=$!; sleep 2
    DISPLAY=:17 vglrun -d egl glxinfo -B > glxinfo.txt 2>&1; grep -iE 'renderer|version string' glxinfo.txt | head -4
    DISPLAY=:17 vglrun -d egl "$GB/glx_gfx" 2>&1 | tee glx.txt
    kill $xp 2>/dev/null; wait $xp 2>/dev/null
    for k in A Z B; do gset_dig "GLXR_HASH_$k" "$(sed -n "s/^GLXR_HASH_$k=//p" glx.txt)"; done
    grep -qi 'renderer string.*NVIDIA' glxinfo.txt && grep -q '^GLXR_REF_FAILS=0$' glx.txt && echo GSET_OK \
        || echo "GSET_FAIL $(grep -m1 -iE 'REF_FAILS|renderer string|FAIL' glx.txt glxinfo.txt)"
}
# ══ EXTRA — ffmpeg GPU filters (the static BtbN n8.1 build of V3_VIDEO_ENGINES, sha 5d3a9e6b…) ═══════
ff_src(){ "$FF" -hide_banner -loglevel error -y -f lavfi -i testsrc2=size=1280x720:rate=30 -frames:v 30 -pix_fmt nv12 -f rawvideo src.nv12 \
          && "$FF" -hide_banner -loglevel error -y -f lavfi -i testsrc2=size=1280x720:rate=30 -frames:v 30 -pix_fmt yuv420p -f rawvideo src.yuv; }
ff_run(){ # name, pix_fmt of src, init, vf → the output frames' md5 is the digest; a failed filter is a named FAIL
    local n=$1 pf=$2 init=$3 vf=$4 in=src.nv12; [ "$pf" = yuv420p ] && in=src.yuv
    "$FF" -hide_banner -loglevel error -y $init -f rawvideo -pix_fmt "$pf" -s 1280x720 -r 30 -i "$in" -vf "$vf" -f rawvideo "o_$n.raw" > "o_$n.log" 2>&1
    local rc=$? m; m=$(gset_md5 "o_$n.raw")
    echo "ff $n rc=$rc md5=$m bytes=$(stat -c %s "o_$n.raw" 2>/dev/null)"; [ $rc -eq 0 ] || { tail -3 "o_$n.log"; echo "GSET_FAIL $n rc=$rc"; }
    rm -f "o_$n.raw"; gset_dig "$n" "$m"
}
item_ff_cuda(){
    gset_need "$FF" || return 0; ff_src || { echo "GSET_FAIL source"; return 0; }
    local I="-init_hw_device cuda=cu:0 -filter_hw_device cu"
    # ⊘ bilateral_cuda, pad_cuda and thumbnail_cuda are NOT here: all three SEGV (rc=139, after writing most
    #   of their output) on BARE METAL with this ffmpeg build (smoke1, 2026-09-26) — a workload fact, not ours.
    ff_run scale_cuda      nv12 "$I" "hwupload_cuda,scale_cuda=640:360:interp_algo=bicubic,hwdownload,format=nv12"
    ff_run yadif_cuda      nv12 "$I" "hwupload_cuda,yadif_cuda=mode=send_field,hwdownload,format=nv12"
    ff_run bwdif_cuda      nv12 "$I" "hwupload_cuda,bwdif_cuda,hwdownload,format=nv12"
    ff_run colorspace_cuda nv12 "$I" "hwupload_cuda,colorspace_cuda=range=pc,hwdownload,format=nv12"
    ff_run chromakey_cuda  yuv420p "$I" "format=yuva420p,hwupload_cuda,chromakey_cuda=color=0x00ff00:similarity=0.2,hwdownload,format=yuva420p"
    ff_run overlay_cuda    nv12 "$I" "split[a][b];[a]hwupload_cuda[m];[b]scale=320:180,hwupload_cuda[o];[m][o]overlay_cuda=x=64:y=48,hwdownload,format=nv12"
    echo GSET_OK
}
item_ff_vulkan(){
    nv_icd_only || return 0
    gset_need "$FF" || return 0; ff_src || { echo "GSET_FAIL source"; return 0; }
    local I="-init_hw_device vulkan=vk:0 -filter_hw_device vk"
    ff_run scale_vulkan     nv12 "$I" "hwupload,scale_vulkan=w=640:h=360:scaler=bilinear,hwdownload,format=nv12"
    ff_run avgblur_vulkan   yuv420p "$I" "hwupload,avgblur_vulkan=sizeX=5:sizeY=5,hwdownload,format=yuv420p"
    ff_run gblur_vulkan     yuv420p "$I" "hwupload,gblur_vulkan=sigma=2,hwdownload,format=yuv420p"
    ff_run chromaber_vulkan yuv420p "$I" "hwupload,chromaber_vulkan=dist_x=4:dist_y=2,hwdownload,format=yuv420p"
    ff_run flip_vulkan      yuv420p "$I" "hwupload,flip_vulkan,hwdownload,format=yuv420p"
    ff_run transpose_vulkan yuv420p "$I" "hwupload,transpose_vulkan=dir=clock,hwdownload,format=yuv420p"
    ff_run nlmeans_vulkan   yuv420p "$I" "hwupload,nlmeans_vulkan=s=2,hwdownload,format=yuv420p"
    ff_run bwdif_vulkan     yuv420p "$I" "hwupload,bwdif_vulkan,hwdownload,format=yuv420p"
    ff_run overlay_vulkan   yuv420p "$I" "split[a][b];[a]hwupload[m];[b]scale=320:180,hwupload[o];[m][o]overlay_vulkan=x=64:y=48,hwdownload,format=yuv420p"
    echo GSET_OK
}
item_ff_opencl(){
    gset_need "$FF" || return 0; ff_src || { echo "GSET_FAIL source"; return 0; }
    local I="-init_hw_device opencl=ocl:0.0 -filter_hw_device ocl"
    ff_run avgblur_opencl     yuv420p "$I" "hwupload,avgblur_opencl=sizeX=5,hwdownload,format=yuv420p"
    ff_run boxblur_opencl     yuv420p "$I" "hwupload,boxblur_opencl=luma_radius=4,hwdownload,format=yuv420p"
    ff_run unsharp_opencl     yuv420p "$I" "hwupload,unsharp_opencl=lx=5:ly=5:la=1.5,hwdownload,format=yuv420p"
    ff_run sobel_opencl       yuv420p "$I" "hwupload,sobel_opencl,hwdownload,format=yuv420p"
    ff_run convolution_opencl yuv420p "$I" "hwupload,convolution_opencl=0 1 0 1 -4 1 0 1 0:0 1 0 1 -4 1 0 1 0:0 1 0 1 -4 1 0 1 0:0 1 0 1 -4 1 0 1 0,hwdownload,format=yuv420p"
    ff_run nlmeans_opencl     yuv420p "$I" "hwupload,nlmeans_opencl=s=2,hwdownload,format=yuv420p"
    ff_run transpose_opencl   yuv420p "$I" "hwupload,transpose_opencl=dir=clock,hwdownload,format=yuv420p"
    ff_run erosion_opencl     yuv420p "$I" "hwupload,erosion_opencl,hwdownload,format=yuv420p"
    echo GSET_OK
}
item_ff_placebo(){
    nv_icd_only || return 0  # libplacebo (Vulkan): scaling + debanding
    gset_need "$FF" || return 0; ff_src || { echo "GSET_FAIL source"; return 0; }
    local I="-init_hw_device vulkan=vk:0 -filter_hw_device vk"
    ff_run libplacebo_scale  yuv420p "$I" "libplacebo=w=960:h=540:upscaler=ewa_lanczos:downscaler=mitchell:dithering=none:format=yuv420p"
    ff_run libplacebo_deband yuv420p "$I" "libplacebo=deband=true:deband_iterations=2:dithering=none:format=yuv420p"
    echo GSET_OK
}
# ══ EXTRA — Blender 4.5 LTS headless (-b): one deterministic scene per engine, PNG digest vs bare metal ═
bl_run(){ # engine device [extra blender args]
    local e=$1 d=$2; shift 2
    gset_need "$BL" || return 0
    "$BL" -b --factory-startup "$@" --python "$GSET_BIN/src/blender_scene.py" -- "$e" "$d" "$W/out.png" > bl.txt 2>&1; rc=$?
    grep -E 'BLENDER_|Error|error|GPU|backend|Fra:1 .*(Finished|Saved)' bl.txt | tail -12
    gset_dig png "$( [ -s "$W/out.png" ] && png_dig "$W/out.png" || echo NOIMG)"
    gset_val mean "$(sed -n 's/.* mean=\([0-9.]*\).*/\1/p' bl.txt | tail -1)"
    [ $rc -eq 0 ] && grep -q "^BLENDER_OK $e" bl.txt && echo GSET_OK || echo "GSET_FAIL blender $e/$d rc=$rc: $(grep -m1 -E 'BLENDER_FAIL|Error' bl.txt)"
}
item_blender_cycles_cuda(){ bl_run CYCLES CUDA; }
item_blender_cycles_optix(){ bl_run CYCLES OPTIX; }
item_blender_eevee(){ bl_run EEVEE GPU --gpu-backend opengl; }
item_blender_workbench(){ bl_run WORKBENCH GPU --gpu-backend opengl; }
item_blender_eevee_vulkan(){ nv_icd_only || return 0; bl_run EEVEE GPU --gpu-backend vulkan; }
