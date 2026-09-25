#!/usr/bin/env bash
# LLM parity on a SHARED bench box: provision (guest image + host venv), then the guest lane
# without and with guest persistence mode, then the host lane — each GPU/qcow2 step under
# /tmp/kayfabe-fastguest.lock, and only when no other agent's QEMU / suite is running.
# /root/prov/LLM_TIMING exists for the timed part (other agents do not build while it does).
#   usage: llm_parity_box.sh <tag> <kf3-binary> [steps=gprov,hprov,guest,guest_pm,host]
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
TAG=${1:?tag}; QB=${2:?kf3 binary}; STEPS=${3:-gprov,hprov,guest,guest_pm,host}
OUT=/workspace/bench/llm; mkdir -p "$OUT"
say() { echo "[$(date -Is)] $*"; }
idle() { ! pgrep -x qemu-system-x86 >/dev/null && ! pgrep -f '[f]ast_suite|[r]un_fast_guest|[b]oot_capture' >/dev/null; }
locked() {  # wait for idle, then run "$@" holding the lock
    exec 9>/tmp/kayfabe-fastguest.lock
    while :; do
        flock 9
        idle && break
        flock -u 9; sleep 3
    done
    "$@"; local rc=$?
    flock -u 9; exec 9>&-
    return $rc
}
has() { case ",$STEPS," in *",$1,"*) return 0 ;; esac; return 1; }
say "LPB_START tag=$TAG qb=$QB steps=$STEPS rev=$(git -C "$HERE" rev-parse --short=8 HEAD)"
has gprov && { locked bash "$HERE/provision_guest_llm.sh" > "$OUT/${TAG}_gprov.log" 2>&1; say "GPROV_RC=$? $(tail -1 "$OUT/${TAG}_gprov.log")"; }
has hprov && { locked bash "$HERE/provision_host_llm.sh" > "$OUT/${TAG}_hprov.log" 2>&1; say "HPROV_RC=$? $(grep -a HOST_LLM_TOK_PER_S "$OUT/${TAG}_hprov.log")"; }
touch /root/prov/LLM_TIMING
trap 'rm -f /root/prov/LLM_TIMING' EXIT
gboot() {  # $1 boot tag, rest env
    local bt=$1; shift
    env "$@" KF_DEVICE=kf3 QEMU_BIN="$QB" NVKVM_RAM_MB=8192 KF_SMP=4 GQ_TIMEOUT=600 LP_GRAPH= \
        POST_CAPTURE_HOOK="$HERE/llm_parity_hook.sh" bash "$HERE/boot_capture.sh" "$bt" > "$OUT/${bt}_driver.log" 2>&1
}
has guest    && { locked gboot "${TAG}_g"  LP_PERF_NTOK=${LP_PERF_NTOK:-128}; say "GUEST_RC=$?"; }
has guest_pm && { locked gboot "${TAG}_gpm" LP_GUEST_PM=1; say "GUEST_PM_RC=$?"; }
has host     && { locked env LP_GRAPH= LLM_ROOT=/opt/llm-host LLM_PY=/opt/llm-host/venv/bin/python \
                      bash "$HERE/llm_parity.sh" local "${TAG}_host" > "$OUT/${TAG}_host.out" 2>&1; say "HOST_RC=$?"; }
say "LPB_DONE"
