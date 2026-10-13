#!/usr/bin/env bash
# Trusted host only. Reset test (Linux fast guest): N QMP system_reset while the guest is Running, then let the last boot finish.
# Uses its OWN lock and QMP socket; the fast-guest runner only ever kills QEMUs named kf-fastguest (never the owner's VM).
set -uo pipefail
REV=${REV:-c9ac4e77}; N=${N:-5}; BUDGET=${BUDGET:-300}; TAG=${TAG:-resetlin}
OUT=/data/claude-reset-linux-$REV; mkdir -p $OUT; : > $OUT/timeline.txt
Q=/workspace/bench/kf3-bins/$REV/qemu-system-x86_64; QMP=/tmp/kf-reset-test.qmp
printf '#!/bin/sh\nexec %s "$@" -qmp unix:%s,server=on,wait=off\n' "$Q" "$QMP" > /tmp/kf-reset-qemu-wrap.sh; chmod +x /tmp/kf-reset-qemu-wrap.sh
rm -f $QMP; xid0=$(dmesg | grep -c 'NVRM: Xid'); owner_q=$(pgrep -f 'name boundary-kayfabe' | head -1)
echo "START $(date -Is) rev=$REV resets=$N xid0=$xid0 owner_qemu=$owner_q" | tee -a $OUT/timeline.txt
cd /data/kf-reset-host-c9ac4e773
( QEMU_BIN=/tmp/kf-reset-qemu-wrap.sh KF_LOCK=/tmp/kf-reset-linux.lock KF_ARMS="${ARMS:---concurrent-fuzz}" bash scripts/fastguest/run_fast_guest.sh $TAG $BUDGET > $OUT/run.log 2>&1 ) &
RUNPID=$!
SER=/workspace/bench/fast_${TAG}_serial.log
python3 -I - "$SER" "$QMP" "$N" "$OUT/timeline.txt" "$owner_q" <<'PY'
import socket,json,time,sys,os,re
ser,qmp,n,tl,owner=sys.argv[1],sys.argv[2],int(sys.argv[3]),sys.argv[4],sys.argv[5]
def log(m):
    open(tl,"a").write(time.strftime("%H:%M:%S ")+m+"\n")
def ready_count():
    try: return open(ser,"rb").read().count(b"FASTGUEST: ready")
    except FileNotFoundError: return 0
t0=time.time()
while not os.path.exists(qmp):
    if time.time()-t0>60: log("NO QMP SOCKET"); sys.exit(3)
    time.sleep(0.5)
s=socket.socket(socket.AF_UNIX); s.connect(qmp); f=s.makefile("rw"); f.readline()
def c(x):
    f.write(json.dumps(x)+"\n"); f.flush()
    while True:
        l=json.loads(f.readline())
        if "return" in l or "error" in l: return l
c({"execute":"qmp_capabilities"})
for i in range(1,n+1):
    t=time.time()
    while ready_count()<i:
        if time.time()-t>120: log(f"boot {i} never reached ready"); sys.exit(4)
        time.sleep(0.5)
    time.sleep([9.0,12.0,15.0,17.5,19.5,7.0,13.0][(i-1)%7])
    alive=os.path.exists(f"/proc/{owner}") if owner else None
    log(f"boot {i} ready; sending system_reset #{i} (owner qemu alive={alive})")
    log(str(c({"execute":"system_reset"})))
log(f"all {n} resets sent; waiting for boot {n+1} to finish")
PY
wait $RUNPID; rc=$?
xid1=$(dmesg | grep -c 'NVRM: Xid')
echo "END $(date -Is) run_rc=$rc xid0=$xid0 xid1=$xid1 owner_alive=$( [ -n "$owner_q" ] && kill -0 $owner_q 2>/dev/null && echo yes || echo no)" | tee -a $OUT/timeline.txt
grep -a -c "Linux version" $SER | sed 's/^/boots seen: /' | tee -a $OUT/timeline.txt
grep -a "FASTGUEST:\|FAST_VERDICT" $OUT/run.log | tail -12 | tee -a $OUT/timeline.txt
