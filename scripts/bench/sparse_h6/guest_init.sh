#!/bin/busybox sh
# E3 guest init (EXPERIMENT claude/sparse-hw-20261026): the LX2 phase-3 initramfs init with the suite replaced
# by the H6 sparse reproducer. Appended to the LX2 initrd as a second cpio archive (later files win).
/bin/busybox --install -s /bin
mount -t proc none /proc
mount -t sysfs none /sys
mount -t devtmpfs none /dev
exec </dev/ttyS0 >/dev/ttyS0 2>&1
stty -F /dev/ttyS0 -echo
mkdir -p /tmp /dev/nvidia-caps
mkfifo /tmp/power-ready
/opt/lx2/power_button --guest > /tmp/power-ready &
read -r ready < /tmp/power-ready
echo "$ready"
case "$ready" in
    LX2_POWER_BUTTON_READY*) ;;
    *) echo H6G_FAIL_power_button; poweroff -f ;;
esac
while read -r ko; do
    [ -f "/lib/modules/$ko" ] || continue
    if ! insmod "/lib/modules/$ko"; then
        echo "H6G_MODULE_FAIL $ko"
        dmesg
        poweroff -f
    fi
done < /lib/modules/loadorder
maj=$(awk '/nvidia-frontend|nvidiactl|^ *[0-9]+ nvidia$/ {print $1; exit}' /proc/devices)
[ -n "$maj" ] || { echo H6G_NO_DRIVER; poweroff -f; }
mknod /dev/nvidiactl c "$maj" 255
mknod /dev/nvidia0 c "$maj" 0
uvm=$(awk '/nvidia-uvm$/ {print $1; exit}' /proc/devices)
if [ -n "$uvm" ]; then
    mknod /dev/nvidia-uvm c "$uvm" 0
    mknod /dev/nvidia-uvm-tools c "$uvm" 1
fi
cap=$(awk '/nvidia-caps/ {print $1; exit}' /proc/devices)
[ -z "$cap" ] || mknod /dev/nvidia-caps/nvidia-cap1 c "$cap" 1
uname -a
cat /proc/driver/nvidia/version
RUN="seq"
for word in $(cat /proc/cmdline); do
    case "$word" in
        h6run=*) RUN=$(echo "${word#h6run=}" | tr ',' ' ') ;;
    esac
done
export __GL_SHADER_DISK_CACHE=0 CUDA_CACHE_DISABLE=1
echo "H6G_BEGIN run=$RUN"
for t in $RUN; do
    echo "H6G_TEST $t"
    case "$t" in
        seq)   timeout 150 /opt/lx2/sparse_h6 300 seq ;;
        chain) timeout 150 /opt/lx2/sparse_h6 300 chain ;;
        one)   timeout 100 /opt/lx2/sparse_h6 1 seq ;;
        neg)   timeout 90 /opt/lx2/gpu_wedge_probe fault ;;
        victim) timeout 60 /opt/lx2/gpu_wedge_probe victim ;;
    esac
    echo "H6G_EXIT $t rc=$?"
done
dmesg | tail -40
echo H6G_DONE
poweroff -f
