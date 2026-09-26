#!/usr/bin/env bash
# itemdefs.sh — the items of the headless-graphics test set (sourced by items.sh; each item_<name>
# runs in its own session, cwd = its scratch dir $W, stdout = its log). Inventory and provenance of
# every item: docs/design/V3_GFX_TESTSET.md §1. Each item prints:
#   GSET_OK            its own self-check held (the floor that holds without a host)
#   GSET_FAIL <why>    its own self-check failed
#   GSET_NEED_MISSING  (via gset_need) a tool is absent → NOTRUN
#   gset_dig/gset_val  content digests (graded vs bare metal) / recorded values (not graded)
# Binaries and data live in $GSET_HOME (provisioned into the guest image by provision.sh — the
# bare-metal side runs the SAME files through hostroot.sh).
GB=$GSET_HOME/gfxbin
FF=$GSET_HOME/ff/bin/ffmpeg
BL=$GSET_HOME/blender/blender
gset_items(){ echo "vk_info vk_compute vk_render egl_render gl_info glx_vgl egl_offscreen vkpeak \
glmark2_offscreen weston_headless weston_glmark2 sway_screencopy sway_glmark2 vkcube_wayland \
ff_cuda ff_vulkan ff_opencl ff_placebo video_nvenc_nvdec \
blender_cycles_cuda blender_cycles_optix blender_eevee blender_workbench blender_eevee_vulkan"; }
gset_timeout(){ case $1 in vkpeak|video_*|blender_*|*glmark2*) echo 900 ;; *) echo 300 ;; esac; }

# ── Vulkan ────────────────────────────────────────────────────────────────────────────────────
item_vk_info(){  # vulkaninfo: the NVIDIA device, and its capability sets digested section by section
    gset_need vulkaninfo python3 || return 0
    vulkaninfo --summary > summary.txt 2>&1; rc=$?
    grep -E 'deviceName|driverVersion|apiVersion|driverID|conformanceVersion' summary.txt
    dev=$(grep -m1 'deviceName.*NVIDIA' summary.txt | sed 's/.*= //')
    gset_dig device "$(echo "$dev" | tr ' ' _)"
    # ★ the capability profile (vulkan-tools ≥1.3.2xx: --json writes VP_VULKANINFO_*.json per GPU)
    vulkaninfo --json=0 >/dev/null 2>json.err; j=$(ls VP_VULKANINFO_*NVIDIA*.json 2>/dev/null | head -1)
    [ -n "$j" ] || j=$(ls VP_VULKANINFO_*.json 2>/dev/null | head -1)
    python3 "$GSET_BIN/vkprofile_dig.py" "$j" 2>&1 || echo "GSET_FAIL no vulkaninfo profile ($(head -c 200 json.err))"
    [ $rc -eq 0 ] && [ -n "$dev" ] && echo GSET_OK || echo "GSET_FAIL vulkaninfo rc=$rc dev=[$dev]"
}
item_vk_compute(){  # 1 Mi-element SSBO kernel, every element checked on the CPU
    gset_need "$GB/vk_gfx" || return 0
    "$GB/vk_gfx" compute 2>&1 | tee vkc.txt
    gset_dig VKC_HASH "$(sed -n 's/^VKC_HASH=//p' vkc.txt)"
    grep -q '^VKC_BAD=0$' vkc.txt && echo GSET_OK || echo "GSET_FAIL $(grep -m1 -E 'VKC_BAD|VK_FAIL' vkc.txt)"
}
item_vk_render(){  # two passes, OPTIMAL tiling, D32 depth test, sampled second pass
    gset_need "$GB/vk_gfx" || return 0
    "$GB/vk_gfx" render "$W/vkr" 2>&1 | tee vkr.txt
    for k in A Z B; do gset_dig "VKR_HASH_$k" "$(sed -n "s/^VKR_HASH_$k=//p" vkr.txt)"; done
    grep -q '^VKR_REF_FAILS=0$' vkr.txt && echo GSET_OK || echo "GSET_FAIL $(grep -m1 -E 'REF_FAILS|VK_FAIL' vkr.txt)"
}
# ── GL ────────────────────────────────────────────────────────────────────────────────────────
item_egl_render(){  # EGL device platform, desktop GL 4.6: FBO RGBA8 + D24S8, depth test, textured pass
    gset_need "$GB/egl_gfx" || return 0
    "$GB/egl_gfx" 2>&1 | tee egl.txt
    for k in A Z B; do gset_dig "EGLR_HASH_$k" "$(sed -n "s/^EGLR_HASH_$k=//p" egl.txt)"; done
    grep -q '^EGLR_REF_FAILS=0$' egl.txt && grep -q '^EGL_RENDERER=.*NVIDIA' egl.txt && echo GSET_OK \
        || echo "GSET_FAIL $(grep -m1 -E 'REF_FAILS|EGL_FAIL|EGL_RENDERER' egl.txt)"
}
item_gl_info(){  # eglinfo on the device platform + the GL implementation limits/extensions (EGL, no X)
    gset_need eglinfo "$GB/gl_limits" || return 0
    eglinfo -B -p device > eglinfo.txt 2>&1; grep -iE 'renderer|version string|vendor' eglinfo.txt | head -8
    "$GB/gl_limits" > limits.txt 2>&1; rc=$?
    head -5 limits.txt
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
item_egl_offscreen(){  # nvkvm-pv tests/perf/apps/egl_offscreen.c (+ a whole-framebuffer readback digest)
    gset_need "$GB/egl_offscreen" || return 0
    "$GB/egl_offscreen" 600 20000 2>&1 | tee eo.txt
    gset_dig fb "$(sed -n 's/^EGLOFF_FB_MD5 //p' eo.txt)"
    gset_val Mtri_s "$(sed -n 's/^METRIC egl_gl_Mtri_s //p' eo.txt)"
    grep -q '^CHECK egl_gl_Mtri_s ok' eo.txt && echo GSET_OK || echo "GSET_FAIL $(grep -m1 'CHECK' eo.txt)"
}
item_vkpeak(){  # nihui/vkpeak 20250531: every figure must be a positive number; the test SET must equal bare metal
    gset_need "$GSET_HOME/vkpeak/vkpeak" || return 0
    "$GSET_HOME/vkpeak/vkpeak" 0 2>&1 | tee vkpeak.txt; rc=${PIPESTATUS[0]}
    grep -E '^[a-z0-9-]+ += +[0-9.]+ ' vkpeak.txt | while read -r n _ v u; do gset_val "$n" "$v$u"; done
    gset_dig testset "$(grep -oE '^[a-z0-9-]+ +=' vkpeak.txt | tr -d ' =' | md5sum | cut -c1-16)"
    nz=$(grep -cE '^[a-z0-9-]+ += +0(\.0+)? ' vkpeak.txt); nt=$(grep -cE '^[a-z0-9-]+ += +[0-9.]+ ' vkpeak.txt)
    [ $rc -eq 0 ] && [ "$nt" -ge 10 ] && [ "$nz" -eq 0 ] && echo GSET_OK || echo "GSET_FAIL vkpeak rc=$rc tests=$nt zero=$nz"
}

# ── ffmpeg GPU filters (the static BtbN n8.1 build V3_VIDEO_ENGINES measured with, sha 5d3a9e6b…) ──
# One deterministic CPU-made source; every GPU filter's frames are md5'd and compared with bare metal.
ff_src(){ "$FF" -hide_banner -loglevel error -y -f lavfi -i testsrc2=size=1280x720:rate=30 -frames:v 30 -pix_fmt nv12 -f rawvideo src.nv12 \
          && "$FF" -hide_banner -loglevel error -y -f lavfi -i testsrc2=size=1280x720:rate=30 -frames:v 30 -pix_fmt yuv420p -f rawvideo src.yuv; }
ff_run(){ # name, pix_fmt of src, init, vf  → frames md5 as a digest; a failed filter is a named FAIL
    local n=$1 pf=$2 init=$3 vf=$4 in=src.nv12; [ "$pf" = yuv420p ] && in=src.yuv
    "$FF" -hide_banner -loglevel error -y $init -f rawvideo -pix_fmt "$pf" -s 1280x720 -r 30 -i "$in" -vf "$vf" -f rawvideo "o_$n.raw" > "o_$n.log" 2>&1
    local rc=$? m; m=$(gset_md5 "o_$n.raw")
    echo "ff $n rc=$rc md5=$m bytes=$(stat -c %s "o_$n.raw" 2>/dev/null)"; [ $rc -eq 0 ] || { tail -3 "o_$n.log"; echo "GSET_FAIL $n rc=$rc"; }
    gset_dig "$n" "$m"
}
item_ff_cuda(){
    gset_need "$FF" || return 0; ff_src || { echo "GSET_FAIL source"; return 0; }
    local I="-init_hw_device cuda=cu:0 -filter_hw_device cu"
    ff_run scale_cuda     nv12 "$I" "hwupload_cuda,scale_cuda=640:360:interp_algo=bicubic,hwdownload,format=nv12"
    ff_run yadif_cuda     nv12 "$I" "hwupload_cuda,yadif_cuda=mode=send_field,hwdownload,format=nv12"
    ff_run bwdif_cuda     nv12 "$I" "hwupload_cuda,bwdif_cuda,hwdownload,format=nv12"
    ff_run bilateral_cuda yuv420p "$I" "hwupload_cuda,bilateral_cuda=sigmaS=3:sigmaR=0.2:window_size=5,hwdownload,format=yuv420p"
    ff_run colorspace_cuda nv12 "$I" "hwupload_cuda,colorspace_cuda=range=pc,hwdownload,format=nv12"
    ff_run pad_cuda       nv12 "$I" "hwupload_cuda,pad_cuda=1344:784:32:32,hwdownload,format=nv12"
    ff_run chromakey_cuda yuv420p "$I" "format=yuva420p,hwupload_cuda,chromakey_cuda=color=0x00ff00:similarity=0.2,hwdownload,format=yuva420p"
    ff_run thumbnail_cuda nv12 "$I" "hwupload_cuda,thumbnail_cuda=n=10,hwdownload,format=nv12"
    ff_run overlay_cuda   nv12 "$I" "split[a][b];[a]hwupload_cuda[m];[b]scale=320:180,hwupload_cuda[o];[m][o]overlay_cuda=x=64:y=48,hwdownload,format=nv12"
    echo GSET_OK
}
item_ff_vulkan(){
    gset_need "$FF" || return 0; ff_src || { echo "GSET_FAIL source"; return 0; }
    local I="-init_hw_device vulkan=vk:0 -filter_hw_device vk"
    ff_run scale_vulkan     nv12 "$I" "hwupload,scale_vulkan=w=640:h=360:scaler=bicubic,hwdownload,format=nv12"
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
    ff_run avgblur_opencl  yuv420p "$I" "hwupload,avgblur_opencl=sizeX=5,hwdownload,format=yuv420p"
    ff_run boxblur_opencl  yuv420p "$I" "hwupload,boxblur_opencl=luma_radius=4,hwdownload,format=yuv420p"
    ff_run unsharp_opencl  yuv420p "$I" "hwupload,unsharp_opencl=lx=5:ly=5:la=1.5,hwdownload,format=yuv420p"
    ff_run sobel_opencl    yuv420p "$I" "hwupload,sobel_opencl,hwdownload,format=yuv420p"
    ff_run convolution_opencl yuv420p "$I" "hwupload,convolution_opencl=0 1 0 1 -4 1 0 1 0:0 1 0 1 -4 1 0 1 0:0 1 0 1 -4 1 0 1 0:0 1 0 1 -4 1 0 1 0,hwdownload,format=yuv420p"
    ff_run nlmeans_opencl  yuv420p "$I" "hwupload,nlmeans_opencl=s=2,hwdownload,format=yuv420p"
    ff_run transpose_opencl yuv420p "$I" "hwupload,transpose_opencl=dir=clock,hwdownload,format=yuv420p"
    ff_run erosion_opencl  yuv420p "$I" "hwupload,erosion_opencl,hwdownload,format=yuv420p"
    echo GSET_OK
}
item_ff_placebo(){  # libplacebo (Vulkan): scaling + debanding + tone mapping chain
    gset_need "$FF" || return 0; ff_src || { echo "GSET_FAIL source"; return 0; }
    ff_run libplacebo_scale  yuv420p "-init_hw_device vulkan=vk:0 -filter_hw_device vk" "libplacebo=w=960:h=540:upscaler=ewa_lanczos:downscaler=mitchell:dithering=none:format=yuv420p"
    ff_run libplacebo_deband yuv420p "-init_hw_device vulkan=vk:0 -filter_hw_device vk" "libplacebo=deband=true:deband_iterations=2:dithering=none:format=yuv420p"
    echo GSET_OK
}
# ── NVENC / NVDEC: V3_VIDEO_ENGINES.md §5's lane, unchanged (md5 of every stream and decoded frame set) ──
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
# ── Blender 4.5 LTS, headless (-b): Cycles on CUDA and OptiX (compute), EEVEE and Workbench (GPU module) ──
bl_run(){ # engine device [extra blender args]
    local e=$1 d=$2; shift 2
    gset_need "$BL" || return 0
    "$BL" -b --factory-startup "$@" --python "$GSET_BIN/src/blender_scene.py" -- "$e" "$d" "$W/out.png" > bl.txt 2>&1; rc=$?
    grep -E 'BLENDER_|Error|error|GPU|backend|Fra:1 .*(Finished|Saved)' bl.txt | tail -12
    gset_dig png "$(gset_md5 "$W/out.png")"
    gset_val mean "$(sed -n 's/.* mean=\([0-9.]*\).*/\1/p' bl.txt | tail -1)"
    [ $rc -eq 0 ] && grep -q "^BLENDER_OK $e" bl.txt && echo GSET_OK || echo "GSET_FAIL blender $e/$d rc=$rc: $(grep -m1 -E 'BLENDER_FAIL|Error' bl.txt)"
}
item_blender_cycles_cuda(){ bl_run CYCLES CUDA; }
item_blender_cycles_optix(){ bl_run CYCLES OPTIX; }
item_blender_eevee(){ bl_run EEVEE GPU --gpu-backend opengl; }
item_blender_workbench(){ bl_run WORKBENCH GPU --gpu-backend opengl; }
item_blender_eevee_vulkan(){ bl_run EEVEE GPU --gpu-backend vulkan; }
