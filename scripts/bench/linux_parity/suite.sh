#!/bin/busybox sh
set -u
# Arguments: executable root (native) or /opt/lx2 (guest), and sync mode.
B=$1
SYNC=${2:-0}
export __GL_SHADER_DISK_CACHE=0 CUDA_CACHE_DISABLE=1
export __EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/10_nvidia.json
clock_probe() {
    if [ "$SYNC" = 1 ]; then
        echo "LX2_SYNC $1"
        read -r answer
        [ "$answer" = GO ] || return 1
    fi
    "$B/clock_probe" "$1"
}
run() {
    name=$1
    shift
    clock_probe "${name}-begin" || exit 2
    echo "LX2_BEGIN $name"
    "$@"
    rc=$?
    echo "LX2_EXIT $name rc=$rc"
    clock_probe "${name}-end" || exit 2
    [ "$rc" = 0 ] || exit "$rc"
}
run surface "$B/surface"
run cuda-spin env BENCH_SIZES=16,2048 BENCH_ITERS=100 BENCH_BATCH=10 \
    BENCH_CTX_FLAGS=1 "$B/cup8bench"
run cuda-block env BENCH_SIZES=16,2048 BENCH_ITERS=100 BENCH_BATCH=10 \
    BENCH_CTX_FLAGS=4 "$B/cup8bench"
run stream env BENCH_BW_ONLY=1 BENCH_BW=16,64,256 BENCH_BW_REPS=1 \
    BENCH_BW_ITERS=100 "$B/cup8bench"
echo LX2_SUITE_DONE
