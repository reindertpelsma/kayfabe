#!/bin/bash
# gpu_liveness.sh OUT : while a guest runs, show the host GPU is still bound to nvidia and usable by the desktop user
out=$1
{ echo "== $(date -Is)"; echo "-- nvidia-smi"; nvidia-smi --query-gpu=name,driver_version,memory.used,utilization.gpu,display_active,pstate --format=csv,noheader
  nvidia-smi --query-compute-apps=pid,name,used_memory --format=csv,noheader
  echo "-- driver bound"; readlink /sys/bus/pci/devices/0000:01:00.0/driver
  echo "-- lsmod (nvidia/vfio)"; lsmod | grep -E "^(nvidia|vfio|kvm)" | awk '{print $1,$2,$3}'
  echo "-- gnome-shell"; pgrep -u ubuntu -a gnome-shell | cut -c1-80; ps -o pid,etimes,pcpu -p $(pgrep -u ubuntu -x gnome-shell | head -1)
  echo "-- vulkaninfo as desktop user (ubuntu)"
  setpriv --reuid=ubuntu --regid=ubuntu --init-groups env -i HOME=/home/ubuntu PATH=/usr/bin:/bin XDG_RUNTIME_DIR=/run/user/1000 timeout 60 vulkaninfo --summary 2>&1 | grep -E "deviceName|driverVersion|deviceType|GPU[0-9]" | head -12
  echo "-- eglinfo (nvidia device) as ubuntu"
  setpriv --reuid=ubuntu --regid=ubuntu --init-groups env -i HOME=/home/ubuntu PATH=/usr/bin:/bin XDG_RUNTIME_DIR=/run/user/1000 timeout 60 eglinfo -B 2>&1 | grep -E "^(Device|EGL vendor|OpenGL renderer|OpenGL core profile renderer)|renderer" | head -8
  echo "-- Xid count"; dmesg | grep -c "NVRM: Xid"
} > $out 2>&1
