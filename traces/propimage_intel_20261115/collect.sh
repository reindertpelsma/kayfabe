#!/bin/bash
# tar text/png evidence of run $1 to stdout (no qcow2, no binaries, no 3dmark-result, no step)
cd /root/prop
tar cf - --exclude='*.qcow2*' --exclude='*.3dmark-result' --exclude='*.exe' --exclude='*.sock' --exclude=broker.fifo --exclude=id_kf --exclude='*.dmp' $1.log out_$1 work_$1/run/last_cmd.txt work_$1/run/swtpm.log work_$1/run/broker.log 2>/dev/null
# bounded qemu.log: warnings and unusual lines only, plus tail
mkdir -p /tmp/qf_$1; grep -a -v "^kf3: \(w#\|family=\)" work_$1/run/qemu.log 2>/dev/null | cut -c1-300 | head -400 > /tmp/qf_$1/qemu_head.txt; tail -n 200 work_$1/run/qemu.log 2>/dev/null | cut -c1-300 > /tmp/qf_$1/qemu_tail.txt
