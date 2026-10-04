#!/usr/bin/env bash
# Alongside the broker lane; starts observing only during its explicit idle hold.
set -euo pipefail
tag=${1:?tag}
bench=/workspace/bench
out=$bench/brk/$tag/d2_watch
mkdir -p "$out"
exec > "$out/result.log" 2>&1
echo "START $(date -Is)"
trap 'rc=$?; touch "$bench/brk/$tag/release"; echo "EXIT rc=$rc $(date -Is)"' EXIT
seen=0
for _ in $(seq 1000); do
    if grep -aq '^BRK_HOLD up to' "$bench/run_${tag}_probe.log" 2>/dev/null; then seen=1; break; fi
    sleep 1
done
test "$seen" = 1
timeout 40 python3 /root/vnc_watch.py 127.0.0.1:5907 "$out" 0,0,96,96 30 &
viewer=$!
sleep 5
grep -q CONNECTED "$out/timeline.txt"
grep -a 'kf3: display fps\[' "$bench/run_${tag}_qemu.log" | tail -1 > "$out/before.txt"
sleep 15
grep -a 'kf3: display fps\[' "$bench/run_${tag}_qemu.log" | tail -1 > "$out/after.txt"
touch "$out/stop"
kill "$viewer" 2>/dev/null || true
wait "$viewer" 2>/dev/null || true
python3 - "$out" <<'PY'
from pathlib import Path
import re,sys
root=Path(sys.argv[1])
a=(root/'before.txt').read_text();b=(root/'after.txt').read_text()
def val(s,k): return float(re.search(r'\b'+k+r'=([0-9.]+)',s).group(1))
delta=val(b,'same')-val(a,'same')
print('D2_BEFORE '+a.strip())
print('D2_AFTER '+b.strip())
assert delta>100, ('no repeated unchanged frames',delta)
assert val(b,'copies')<5 and val(b,'checks')>10, b
assert val(b,'over')==0, b
print(f'D2_WATCH_VERDICT PASS same_delta={delta} copies_hz={val(b,"copies")} checks_hz={val(b,"checks")}')
PY
