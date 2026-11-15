#!/bin/bash
# usage: probe.sh <name> "<cpu arg>"  : boot with that -cpu, 150 s ssh wait, screendump at 120 s, no hold.
N=$1; export KF_INTEL_CPU_ARG="$2"
export KF_INTEL_SSH_WAIT=${KF_INTEL_SSH_WAIT:-150} QEMU=/workspace/bench/kf3-bins/84dff90b/qemu-system-x86_64
touch /root/prop/release_$N
bash /root/run_check.sh $N --apps edge --hold-until-file /root/prop/release_$N &
sleep 125
python3 - <<PY
import socket,json
try:
    s=socket.socket(socket.AF_UNIX); s.connect("/root/prop/work_$N/qmp.sock"); f=s.makefile("rw"); f.readline()
    def c(x):
        f.write(json.dumps(x)+"\n"); f.flush()
        while True:
            r=json.loads(f.readline())
            if "return" in r or "error" in r: return r
    c({"execute":"qmp_capabilities"}); c({"execute":"screendump","arguments":{"filename":"/root/prop/${N}_screen.ppm"}})
except Exception as e: print("screendump failed",e)
PY
pnmtopng /root/prop/${N}_screen.ppm > /root/prop/${N}_screen.png 2>/dev/null; rm -f /root/prop/${N}_screen.ppm
wait
