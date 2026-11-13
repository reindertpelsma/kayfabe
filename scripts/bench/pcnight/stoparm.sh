#!/bin/bash
# stoparm.sh ARM (on PC): clean guest shutdown, then own broker/swtpm by pid only
O=/workspace/bench/pcnight_20261113/arm_$1; Q=$(cat $O/qemu.pid)
ssh -i /workspace/bench/propimage_20261112/keys/id_ed25519 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ConnectTimeout=8 -p 33037 kfimage@127.0.0.1 'shutdown /s /t 0' >/dev/null 2>&1
for i in $(seq 60); do kill -0 $Q 2>/dev/null || break; sleep 3; done
if kill -0 $Q 2>/dev/null; then echo "qemu $Q still alive after 180s: QMP quit"; echo '{"execute":"qmp_capabilities"}' >/dev/null; kill $Q; sleep 3; fi
for p in $(cat $O/broker.pid); do ps -o comm= -p $p | grep -q nvkvm-display && kill $p; done
SP=$(cat /workspace/bench/propimage_20261112/run/swtpm.pid 2>/dev/null); [ -n "$SP" ] && ps -o comm= -p $SP | grep -q swtpm && kill $SP
sleep 2; echo "after stop: qemu=$(kill -0 $Q 2>/dev/null && echo alive || echo gone) broker=$(pgrep -u ubuntu -c -x nvkvm-display-b) swtpm=$(pgrep -c -x swtpm) xid_total=$(dmesg | grep -c 'NVRM: Xid')"
cp $O/qemu.out $O/qemu.out.full 2>/dev/null; gzip -f $O/qemu.out.full
