#!/bin/bash
# win_fdaudit.sh : boot the Windows guest as kfrootless for ~60 s and record what the process holds (fds, caps, limits, mappings of device nodes)
D=/home/kfrootless/win; L=fd1; RUN=$D/$L; EV=/data/claude-rootless/ev
/data/claude-rootless/bin/as_user.sh qemu-img create -q -f qcow2 -F qcow2 -b $D/u0.qcow2 $D/$L.qcow2 || exit 2
cp $D/u0.vars.fd $D/$L.vars.fd; chown kfrootless $D/$L.vars.fd
nohup flock -o /tmp/kayfabe-fastguest.lock /data/claude-rootless/bin/win_session.sh user $RUN $D/$L.qcow2 $D/$L.vars.fd > $EV/win_fd1_session.log 2>&1 &
for i in $(seq 60); do grep -q WIN_STARTED $EV/win_fd1_session.log 2>/dev/null && break; sleep 2; done
P=$(cat $RUN/qemu.pid); sleep 45
{ echo "pid=$P $(date -Is)"; ps -o pid,user,uid,etime,cmd -p $P | cut -c1-160
  grep -E "^(Uid|Gid|Groups|CapInh|CapPrm|CapEff|CapBnd|CapAmb|NoNewPrivs|Seccomp|VmLck|VmPin|VmRSS|Threads)" /proc/$P/status
  echo "--- limits"; grep -E "locked|open files|processes" /proc/$P/limits
  echo "--- fd targets (count)"; ls -l /proc/$P/fd | awk '{print $11}' | sed -E 's/\[[0-9]+\]//;s/:\[.*//' | sort | uniq -c | sort -rn
  echo "--- fds on /dev, /sys, /proc, /home"; ls -l /proc/$P/fd | awk '{print $11}' | grep -E "^/(dev|sys|proc|home|var|run|tmp|etc)" | sort | uniq -c
  echo "--- any vfio/dri/udmabuf/uffd/hugetlb/mem fd or map"; (ls -l /proc/$P/fd; cat /proc/$P/maps) | grep -E -i "vfio|/dev/dri|udmabuf|userfaultfd|hugetlb|/dev/mem|iommu" | head -5; echo "(end)"
  echo "--- mappings of /dev nodes"; awk '{print $6}' /proc/$P/maps | grep -E "^/dev/" | sort | uniq -c
  echo "--- all threads CapEff"; for t in /proc/$P/task/*; do grep CapEff $t/status; done | sort | uniq -c
  echo "--- children / net listeners of uid 1002"; pgrep -P $P -a | cut -c1-100; ss -tlnpu 2>/dev/null | grep "pid=$P" | head -3; ss -xlnp 2>/dev/null | grep "pid=$P" | cut -c1-140 | head
  echo "--- broker process"; pgrep -u kfrootless -a | cut -c1-110
  echo "--- cgroup / ns"; cat /proc/$P/cgroup; readlink /proc/$P/ns/user /proc/$P/ns/mnt
  echo "--- AppArmor"; cat /proc/$P/attr/current 2>/dev/null
} > $EV/win_fdaudit_user.txt 2>&1
touch $RUN/STOP; for i in $(seq 40); do grep -q SESSION_EXIT $EV/win_fd1_session.log && break; sleep 3; done
if ! grep -q SESSION_EXIT $EV/win_fd1_session.log; then kill $P; sleep 3; fi; tail -2 $EV/win_fd1_session.log
