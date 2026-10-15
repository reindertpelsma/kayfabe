#!/usr/bin/env bash
# Trusted host only; caller holds the global GPU lock (flock -o /tmp/kayfabe-fastguest.lock).
# Regression run of the flag-free Windows default route (source ec45aad99): apps that passed before, batches of 8 (one disposable guest each).
set -uo pipefail
REV=ec45aad99a81cb16eb0554b77f0ff331c47cf6d3; SHORT=${REV:0:8}
W=/var/lib/kf-windows-20261005; TREE=/data/kf-memory-access-host-${REV:0:9}
ROOT=$W/claude-default-matrix-${REV:0:9}
B=$W/kayfabe-win-6fafcc6e/scripts/bench/windows/windows_broker_prod3.sh
POST=/tmp/kf-fifo-refusal-post-b081f705.sh
FLOOR=107374182400
APPS="atomics_t,bandwidthTest_demo,blackscholes_t,blender_cycles,cdp_quicksort,cg_t,cuda_callback,cuda_occupancy,cup2_ladder,cup3_ladder,cup8_ladder,cupy_asynccopy,cupy_cg,cupy_check,cupy_sha256,d3d11va_h264,d3d12_signal_probe,deviceQuery_demo,driver_status,dxdiag_report,dxgi_adapters,dxprobe_d3d11,dxprobe_d3d12,dxva2_h264,edge_video_vp9,edge_webgl,edge_webgl_headless,edge_webgpu,fft_t,furmark_gl_bench,furmark_gl_stress,furmark_glinfo,furmark_knot_gl,furmark_vk_bench,furmark_vkinfo,fwt_t,glgears_wgl,gputest_fur,gputest_gi,gputest_pixmark_piano,gputest_plot3d,gputest_tess,gputest_triangle,graphs_t,hashcat,hf_generate,hist_t,llama_cuda_bench,llama_cuda_gen,llama_vulkan_bench,mandelbrot_t,matmul_t,memcpy2d_t,nbody_demo_bench,nbody_t,nvdec_h264,nvenc_av1,nvenc_h264,nvenc_hevc,nvidia_smi,oceanFFT_demo,opencl_icd,ort_directml,randomFog_demo,reduce_t,rng_t,scan_t,sort_t,stream_created2nd_t,stream_created_t,stream_default_t,stream_nonblocking_t,stream_perthread_t,stream_two_t,streams_t,tc16_t,tcbf16_t,torch_burn,torch_correct,transpose_t,triad_t,vkpeak,vulkan_video_decode,vulkaninfo,zerocopy_t"
test "$(git -C $TREE rev-parse HEAD)" = "$REV" || exit 10
test -z "$(git -C $TREE status --porcelain --untracked-files=no)" || exit 11
! pgrep qemu-system || exit 12
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ || exit 13
test ! -e "$ROOT" || exit 14
mkdir -p "$ROOT"; date -Is > "$ROOT/START"
trap 'rc=$?; echo "$rc" > "$ROOT/EXIT"; echo CLAUDE_DEFAULT_MATRIX_EXIT=$rc at=$(date -Is)' EXIT
while IFS= read -r name; do unset "$name"; done < <(compgen -e | sed -n '/^KF3_/p')
export KF_LOCKED=1 WIN_TRACE=0 WIN_GSP_OBSERVER=0 WIN_DROP=KF3_TIMER_MAP WIN_DISABLE_MPO=1 PROD_EXTRA_FLAGS= WIN_FLAGS=
export WIN_PRE_GUEST_CMD="test \$(df -B1 --output=avail /var/lib | tail -1) -gt $FLOOR && bash $W/pti-iommu_nogdm.sh identity"
export WIN_POST_GUEST_CMD="bash $POST"
IFS= read -r KF_GUEST_PW < "$W/windows-desktop/secrets/win_password" || test -n "$KF_GUEST_PW"; export KF_GUEST_PW
cd "$TREE"
echo 'H-DEFAULT-MATRIX: the flag-free Windows USER relay route does not regress apps that passed before (79 PASS + 8 PASS_WITH_RECOVERY at earlier revisions).'
echo 'FALSIFIER: an app that passed before now FAILs/TIMEOUTs with a guest TDR, WER, host Xid or wrong output (staging/proof-only failures are listed separately).'
IFS=, read -r -a LIST <<< "$APPS"
batch_no=0
for ((i=0;i<${#LIST[@]};i+=8)); do
  batch=$(IFS=,; echo "${LIST[*]:i:8}")
  base=$((1700 + batch_no*10))
  echo "BATCH $batch_no base=$base apps=$batch at=$(date -Is)"
  if [ "$(df -B1 --output=avail /var/lib | tail -1)" -le $FLOOR ]; then echo "STOP free space below floor"; exit 20; fi
  bash scripts/bench/windows/appmatrix/run_windows_apps.sh --run-dir "$ROOT/b$batch_no" --kf3-rev "$SHORT" --broker "$B" --run-base "$base" \
      --override-dir "$W/codex-matrix-focus-tools-20261011" --iso "$W/codex-supplement-20261011/full-image-a61af1c3/kfapps_supplement.img" \
      --apps "$batch" --tier 3 --per-guest 8 --max-tdr 99 --no-isolate >> "$ROOT/console.log" 2>&1
  echo "BATCH_EXIT $batch_no rc=$? at=$(date -Is)"
  for ((n=base;n<base+10;n++)); do
    test -e "$W/windows-broker-run$n.state" && bash "$B" stop "$n" || true
    q="$W/boundary-kayfabe-$n/windows.qcow2"
    if [ -f "$q" ] && qemu-img info "$q" 2>/dev/null | grep -q 'backing file: /var/lib/kf-windows-20261005/baseline/windows.qcow2'; then rm -f "$q"; fi
  done
  bash "$POST" || true
  batch_no=$((batch_no+1))
done
python3 /tmp/kf-capture-xids.py "$ROOT" "$(date -u -d '-4 hours' '+%Y-%m-%d %H:%M:%S UTC')" || true
