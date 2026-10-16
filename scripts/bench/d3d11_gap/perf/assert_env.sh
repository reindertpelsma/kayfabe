#!/bin/bash
# assert_env.sh OUTFILE : print and save guest driver, host driver, kf3 binary path, KF3_* env of the running kayfabe QEMU (run 2995)
W=/var/lib/kf-windows-20261005; QP=$(sed -n "s/^QPID=//p" $W/windows-broker-run2995.state)
{
echo "ASSERT time=$(date -Is) qpid=$QP"
echo "ASSERT host_driver=$(nvidia-smi --query-gpu=driver_version --format=csv,noheader)"
echo "ASSERT guest_driver=$(timeout 60 python3 /data/kfagent_server.py run '(& nvidia-smi --query-gpu=driver_version --format=csv,noheader) -join ""' 40 | tr -d '\r\n')"
echo "ASSERT kf3_exe=$(readlink /proc/$QP/exe)"
echo "ASSERT kf3_env_begin"; tr '\0' '\n' < /proc/$QP/environ | grep '^KF3_' | sort; echo "ASSERT kf3_env_end"
echo "ASSERT cmdline_flags=$(tr '\0' ' ' < /proc/$QP/cmdline | grep -o 'kf3-gpu[^ ]*' | head -1 | cut -c1-300)"
echo "ASSERT broker=$(ps -eo user,args | grep '[n]vkvm-display-broker' | grep -v runuser | sed -E 's/--title.*//' | cut -c1-200)"
bash /data/claude-gap/perf/memrec.sh assert | sed 's/^/ASSERT /'
} | tee ${1:-/dev/null}
