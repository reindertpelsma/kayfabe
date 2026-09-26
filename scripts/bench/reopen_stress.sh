#!/usr/bin/env bash
# ★★★★★ w828 REOPEN STRESS — N fat-guest boots × (boot_capture's own nvidia-smi + the hook's M
# nvidia-smi/cup3 pairs). 0 failures required: every boot's REOPEN_VERDICT=PASS.
#
# usage: reopen_stress.sh <tag> [N=20] [M=20]      (on the bench box, in the kayfabe checkout)
# out:   $BENCH/ro_<tag>.out — one RO_ROW per boot, then RO_SUMMARY, with START/EXIT markers.
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
BENCH=${BENCH_DIR:-/workspace/bench}
TAG=${1:?usage: reopen_stress.sh <tag> [N] [M]}; N=${2:-20}; M=${3:-20}
OUT=$BENCH/ro_${TAG}.out
REV=$(git -C "$REPO" rev-parse --short=8 HEAD); DIRTY=$(git -C "$REPO" status --porcelain --untracked-files=no | wc -l)
# ★ QEMU_BIN (optional) names the kf3 binary when this checkout's HEAD is a scripts-only revision
# on top of the one that was built — the row then says which binary was measured.
echo "RO_START $(date -Is) rev=$REV dirty=$DIRTY n=$N m=$M cuda=${REOPEN_CUDA:-1} qemu=${QEMU_BIN:-kf3-bins/$REV}" > "$OUT"
pass=0; fail=0
for b in $(seq 1 "$N"); do
  t=ro_${TAG}_$b
  for _w in $(seq 1 60); do   # the previous VM's reservation drains after QEMU exits
    u=$(nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits 2>/dev/null | head -1 | tr -dc 0-9)
    [ -n "$u" ] && [ "$u" -le 512 ] && break; sleep 0.5
  done
  # ★ GPU runs are strictly serial across agents: hold the shared run lock for the whole boot
  # (boot_capture.sh itself takes none).
  exec 9>"${KF_LOCK:-/tmp/kayfabe-fastguest.lock}"; flock 9
  for _w in $(seq 1 60); do   # re-check after the lock: another agent's VM may just have exited
    u=$(nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits 2>/dev/null | head -1 | tr -dc 0-9)
    [ -n "$u" ] && [ "$u" -le 512 ] && break; sleep 0.5
  done
  KF_DEVICE=kf3 POST_CAPTURE_HOOK=$HERE/reopen_hook.sh REOPEN_M=$M GQ_TIMEOUT=${GQ_TIMEOUT:-1800} \
    bash "$HERE/boot_capture.sh" "$t" > "$BENCH/${t}_driver.log" 2>&1
  brc=$?
  exec 9>&-
  p=$BENCH/run_${t}_probe.log; q=$BENCH/run_${t}_qemu.log
  v=$(grep -a '^REOPEN_VERDICT=' "$p" 2>/dev/null | tail -1 | cut -d= -f2)
  kv=$(grep -a '^REOPEN_ROWS=\|^REOPEN_DMESG_' "$p" 2>/dev/null | sed 's/^REOPEN_//' | tr '\n' ' ')
  smi0=$(grep -a '^SMI_RC=' "$p" 2>/dev/null | head -1)
  gsprefused=$(grep -ac 'GSP command service REFUSED' "$q" 2>/dev/null || true)
  phases=$(grep -ac 'GSP phase .* -> Halted' "$q" 2>/dev/null || true)
  [ "$v" = PASS ] && pass=$((pass+1)) || fail=$((fail+1))
  echo "RO_ROW boot=$b verdict=${v:-NONE} boot_rc=$brc $smi0 halted_transitions=$phases gsp_refused_lines=$gsprefused $kv" >> "$OUT"
  tail -1 "$OUT"
done
echo "RO_SUMMARY boots=$N pass=$pass fail=$fail m=$M rev=$REV" >> "$OUT"
echo "RO_EXIT $(date -Is)" >> "$OUT"
cat "$OUT"
