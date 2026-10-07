#!/usr/bin/env bash
# The no-PM adapter re-init loop on a SHARED bench box: build this checkout's kf3 (idle + locked),
# then boot the fat guest with uvm_reinit_loop_hook.sh (locked). Writes a start marker and an exit
# line, so "file exists but has no terminator" is detectable (CLAUDE.md: a killed job looks like a
# running one when absence of a result is the only check).
#   usage: uvm_reinit_box.sh <tag> [steps=build,loop]   env: URL_N, URL_PROG, KF_VAS_CENSUS, NVKVM_RAM_MB
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
TAG=${1:?tag}; STEPS=${2:-build,loop}
OUT=/workspace/bench/uvmwall; mkdir -p "$OUT"
say() { echo "[$(date -Is)] $*"; }
idle() { ! pgrep -x qemu-system-x86 >/dev/null && ! pgrep -x cargo >/dev/null \
         && ! pgrep -f '^(/usr/bin/)?bash [^-]\S*(fast_suite|run_fast_guest|boot_capture)\.sh' >/dev/null; }
locked() {
    exec 9>/tmp/kayfabe-fastguest.lock
    while :; do flock 9; idle && break; flock -u 9; sleep 5; done
    "$@"; local rc=$?
    flock -u 9; exec 9>&-
    return $rc
}
has() { case ",$STEPS," in *",$1,"*) return 0 ;; esac; return 1; }
REV=$(git -C "$HERE" rev-parse --short=8 HEAD)
say "URB_START tag=$TAG rev=$REV steps=$STEPS"
QB=/workspace/bench/kf3-bins/$REV/qemu-system-x86_64
if has build; then
    locked env PATH="$HOME/.cargo/bin:$PATH" CARGO_BUILD_JOBS=12 bash "$HERE/build_kf3.sh" /workspace/bench/qemu-10.2.4 > "$OUT/${TAG}_build.log" 2>&1
    say "BUILD_RC=$? $(grep -a KF3_BUILT "$OUT/${TAG}_build.log")"
fi
if has loop; then
    [ -x "$QB" ] || { say "URB_EXIT rc=2 no binary $QB"; exit 2; }
    locked env KF_DEVICE=kf3 QEMU_BIN="$QB" NVKVM_RAM_MB=${NVKVM_RAM_MB:-8192} KF_SMP=4 GQ_TIMEOUT=600 \
        KF_VAS_CENSUS=${KF_VAS_CENSUS:-1} POST_CAPTURE_HOOK="$HERE/uvm_reinit_loop_hook.sh" \
        bash "$HERE/boot_capture.sh" "$TAG" > "$OUT/${TAG}_driver.log" 2>&1
    say "LOOP_RC=$? $(grep -a URL_OUTCOME /workspace/bench/run_${TAG}_probe.log 2>/dev/null | tail -1)"
fi
say "URB_EXIT rc=0"
