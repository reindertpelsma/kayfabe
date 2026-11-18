#!/bin/bash
# vastgate reset lane (L1): QMP system_reset x5 while the fast guest is busy.
# usage: reset_lane.sh <checkout> <tag>
set -uo pipefail
CK=${1:?}; T=${2:?}
BENCH=/workspace/bench
REV8=$(git -C "$CK" rev-parse --short=8 HEAD)
REAL=$BENCH/kf3-bins/$(ls -t $BENCH/kf3-bins | head -1)/qemu-system-x86_64
SOCK=/tmp/vg_$T.qmp
cat > /tmp/vg_qemu_wrap.sh <<EOF
#!/bin/bash
exec $REAL "\$@" -qmp unix:$SOCK,server=on,wait=off
EOF
chmod +x /tmp/vg_qemu_wrap.sh
echo "RESET_LANE_START $(date -Is) rev8=$REV8 real=$REAL"
(
  SER=$BENCH/fast_${T}_serial.log
  for i in 1 2 3 4 5; do
    for w in $(seq 1 300); do [ "$(grep -a -c 'FASTGUEST: arms' "$SER" 2>/dev/null)" -ge "$i" ] && break; sleep 1; done
    sleep 20
    python3 - "$SOCK" <<'P'
import socket,sys,time
s=socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); s.recv(4096)
s.send(b'{"execute":"qmp_capabilities"}\n'); s.recv(4096)
s.send(b'{"execute":"system_reset"}\n'); time.sleep(1); print("QMP reset sent", s.recv(4096)[:80])
P
    echo "RESET_SENT $i $(date -Is)"
  done
) > /root/prov/${T}_resets.log 2>&1 &
cd "$CK"
QEMU_BIN=/tmp/vg_qemu_wrap.sh KF_DEVICE=kf3 KF_ARMS="--defer-liveness --ce-client-guest-ram" KF_REQUIRE_FORWARD=${KF_REQUIRE_FORWARD:-1} \
  bash scripts/fastguest/run_fast_guest.sh "$T" 900 > /root/prov/${T}_run.log 2>&1
echo "RESET_LANE_RUN_RC=$?"
wait
echo "RESET_LANE_EXIT $(date -Is)"
