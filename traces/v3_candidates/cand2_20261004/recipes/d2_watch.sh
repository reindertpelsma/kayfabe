#!/usr/bin/env bash
# Alongside the broker lane; starts observing only during its explicit idle hold.
set -euo pipefail
tag=${1:?tag}
bench=/workspace/bench
out=$bench/brk/$tag/d2_watch
exec > "/root/prov/${tag}_d2_watch.log" 2>&1
echo "START $(date -Is)"
trap 'rc=$?; touch "$bench/brk/$tag/release"; echo "EXIT rc=$rc $(date -Is)"' EXIT
seen=0
for _ in $(seq 1000); do
    if grep -aq '^BRK_HOLD up to' "$bench/run_${tag}_probe.log" 2>/dev/null; then seen=1; break; fi
    sleep 1
done
test "$seen" = 1
mkdir -p "$out"
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

if [ "${E3_PROGRESS:-0}" = 1 ]; then
    python3 - "$out" <<'PY_E3'
from pathlib import Path
import json,os,re,signal,subprocess,sys,time
out=Path(sys.argv[1])
pids=[]
for proc in Path('/proc').iterdir():
    if not proc.name.isdigit(): continue
    try: args=(proc/'cmdline').read_bytes().split(b'\0')
    except OSError: continue
    if args and Path(os.fsdecode(args[0])).name=='nvkvm-display-broker' and b'/run/user/1000/nvkvm/display.sock' in args:
        pids.append(int(proc.name))
assert len(pids)==1,pids
pid=pids[0]
start=time.monotonic()
os.kill(pid,signal.SIGSTOP)
try:
    state=Path(f'/proc/{pid}/status').read_text()
    assert re.search(r'^State:\s+T',state,re.M),state
    gssh='/root/kayfabe/scripts/bench/gssh_nv'
    alive=subprocess.check_output([gssh,'echo ALIVE'],text=True,timeout=15).strip()
    assert alive=='ALIVE',alive
    command='sudo -u ubuntu env DISPLAY=:0 XAUTHORITY=/home/ubuntu/.Xauthority timeout 25 stdbuf -oL glxgears 2>&1'
    result=subprocess.run([gssh,command],text=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=45)
    (out/'stopped_glxgears.txt').write_text(result.stdout)
    rates=[float(x) for x in re.findall(r'=\s*([0-9.]+) FPS',result.stdout)]
    assert rates and min(rates)>1,(result.returncode,result.stdout)
    assert re.search(r'^State:\s+T',Path(f'/proc/{pid}/status').read_text(),re.M)
    report=dict(broker_pid=pid,guest=alive,stopped_seconds=time.monotonic()-start,fps=rates,command_rc=result.returncode)
    (out/'stopped_progress.json').write_text(json.dumps(report,indent=2)+'\n')
    print('E3_PROGRESS_VERDICT PASS '+json.dumps(report),flush=True)
finally:
    os.kill(pid,signal.SIGCONT)
PY_E3
fi
