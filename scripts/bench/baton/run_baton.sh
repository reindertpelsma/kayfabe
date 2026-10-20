#!/usr/bin/env bash
# run_baton.sh - run the baton probe: solo | pair | heavylight.  Works on a bare-metal host and
# inside a (Linux) guest alike; needs only ./baton (gcc -O2 -o baton baton.c -ldl), python3.
#
#   ./run_baton.sh [-b BIN] [-o OUTDIR] [-k K_MS] [-H HEAVY_K_MS] [-K burn|clock] MODE [DURATION_S]
#
#   solo        one client (calibration baseline).             default 60 s
#   pair        two clients, both K_MS launches (default 5).    default 180 s
#   heavylight  two clients: 'light' K_MS and 'heavy' HEAVY_K_MS (default 50).   default 120 s
#
# Writes OUTDIR/<mode>.{A,B}.log, .dump.csv, nvsmi.csv (1 Hz, if nvidia-smi exists) and prints
# the report.  Aborts (SIGTERM to its own children only) if the kernel-log Xid count rises.
set -u
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN="$HERE/baton"; OUT="./baton-out"; K=5; HK=50; KERNEL=burn
while getopts "b:o:k:H:K:" o; do
  case $o in b) BIN=$OPTARG;; o) OUT=$OPTARG;; k) K=$OPTARG;; H) HK=$OPTARG;; K) KERNEL=$OPTARG;; *) exit 1;; esac
done
shift $((OPTIND - 1))
MODE=${1:?mode: solo|pair|heavylight}
case $MODE in solo) DUR=${2:-60};; pair) DUR=${2:-180};; heavylight) DUR=${2:-120};; *) echo "bad mode" >&2; exit 1;; esac
mkdir -p "$OUT"
xid() { dmesg 2>/dev/null | grep -c Xid; }
XID0=$(xid)
echo "start $(date -Is) mode=$MODE dur=$DUR k=$K hk=$HK kernel=$KERNEL xid0=$XID0 bin=$BIN" | tee "$OUT/$MODE.marker"

CPM=$("$BIN" --calibrate-only --kernel "$KERNEL" --k-ms "$K" 2>"$OUT/$MODE.calib.err" | sed -n 's/^CYCLES_PER_MS=//p')
[ -n "$CPM" ] || { echo "calibration failed:"; cat "$OUT/$MODE.calib.err"; exit 2; }
echo "cycles_per_ms=$CPM ($(cat "$OUT/$MODE.calib.err" | tr '\n' ' '))"

START=$(python3 -c 'import time; print("%.3f" % (time.time() + 6))')
PIDS=()
launch() {  # label k_ms
  "$BIN" --label "$1" --kernel "$KERNEL" --k-ms "$2" --duration "$DUR" --cycles-per-ms "$CPM" --start-at "$START" \
         --dump-all "$OUT/$MODE.$1.dump.csv" >"$OUT/$MODE.$1.log" 2>"$OUT/$MODE.$1.err" &
  PIDS+=($!)
}
case $MODE in
  solo) launch A "$K";;
  pair) launch A "$K"; launch B "$K";;
  heavylight) launch light "$K"; launch heavy "$HK";;
esac
if command -v nvidia-smi >/dev/null; then
  nvidia-smi --query-gpu=timestamp,utilization.gpu,clocks.sm,temperature.gpu --format=csv,noheader -l 1 \
    >"$OUT/$MODE.nvsmi.csv" 2>/dev/null &
  SMI=$!
fi
ABORT=0
end=$(( $(date +%s) + DUR + 40 ))
while :; do
  alive=0; for p in "${PIDS[@]}"; do kill -0 "$p" 2>/dev/null && alive=1; done
  [ $alive = 0 ] && break
  if [ "$(xid)" -gt "$XID0" ] || [ "$(date +%s)" -gt "$end" ]; then
    echo "ABORT: Xid rose or deadline passed; SIGTERM to own children" >&2
    kill -TERM "${PIDS[@]}" 2>/dev/null; ABORT=1; sleep 5; break
  fi
  sleep 1
done
wait "${PIDS[@]}" 2>/dev/null
[ -n "${SMI:-}" ] && kill "$SMI" 2>/dev/null
echo "exit $(date -Is) abort=$ABORT xid_now=$(xid)" | tee -a "$OUT/$MODE.marker"

args=()
for f in "$OUT/$MODE".*.log; do
  case $f in *.err) continue;; esac
  args+=("$f:${f%.log}.dump.csv")
done
python3 -I "$HERE/baton_parse.py" report "${args[@]}"
if [ -s "$OUT/$MODE.nvsmi.csv" ]; then
  awk -F', ' '{gsub(/ %/,"",$2); s+=$2; n++; if($2+0>m)m=$2+0} END{printf "nvidia-smi util.gpu: mean %.1f%% max %d%% (%d samples)\n", s/n, m, n}' "$OUT/$MODE.nvsmi.csv"
fi
