#!/bin/bash
# r3_audit.sh : what did the unprivileged runs leave on the host outside the user's home and the install folder?
SINCE="2026-10-16 19:26:00"
echo "== date $(date -Is)"
echo "-- processes owned by kfrootless"; pgrep -u kfrootless -a | cut -c1-120; echo "(end)"
echo "-- any qemu / broker left"; pgrep -a -f "[q]emu-system|[n]vkvm-display-broker" | cut -c1-100; echo "(end)"
echo "-- files owned by uid 1002 outside /home/kfrootless (all local fs incl. tmpfs)"
find / -xdev -uid 1002 -not -path "/home/kfrootless/*" -not -path "/proc/*" 2>/dev/null | head -40
for d in /tmp /run /dev/shm /var/tmp /dev; do find $d -uid 1002 2>/dev/null; done | sort -u | head -40
echo "(end)"
echo "-- sockets / listeners owned by uid 1002"; ss -xlnp 2>/dev/null | grep -c 'uid:1002'; ss -tlnp 2>/dev/null | grep -E "pid=[0-9]+" | grep -c kfrootless
echo "-- root-owned changes since $SINCE in system dirs (excludes /data /home /root /proc /sys /dev/pts)"
find /etc /usr /opt /boot /var /srv /lib /lib64 /sbin /bin -xdev -newermt "$SINCE" -not -path "/var/lib/kf-windows-20261005/*" -not -path "/var/log/*" -not -path "/var/lib/systemd/*" -not -path "/var/cache/*" 2>/dev/null | head -40
echo "(end)"
echo "-- /var/log / journal growth attributable? (files changed)"; find /var/log -xdev -newermt "$SINCE" -type f 2>/dev/null | head -10
echo "-- /tmp and /run top-level listing (non-session)"; ls -la /tmp | grep -v -E "^total|\.X11|systemd-private|snap-private|tracker|ssh-|\.ICE|\.font|\.Test" | awk '{print $1,$3,$4,$5,$6,$7,$8,$9}' | head -40
ls -la /run | grep -i -E "kayfabe|nvkvm|kf3" ; ls /dev/shm; ls -la /dev/hugepages | head -3
echo "-- modules"; lsmod | grep -E "^(nvidia|vfio|kvm|iommufd|udmabuf|kayfabe|kf)" | awk '{print $1,$2,$3}'
echo "-- hugepages"; grep -E "HugePages_Total|Hugetlb" /proc/meminfo
echo "-- sysctl sample"; for s in vm.unprivileged_userfaultfd kernel.unprivileged_userns_clone kernel.apparmor_restrict_unprivileged_userns kernel.io_uring_disabled kernel.perf_event_paranoid kernel.yama.ptrace_scope vm.nr_hugepages vm.overcommit_memory vm.max_map_count kernel.unprivileged_bpf_disabled; do printf '%s=%s\n' $s "$(sysctl -n $s)"; done
echo "-- udev rules / modprobe.d / limits changed since $SINCE"; find /etc/udev /usr/lib/udev /lib/udev /etc/modprobe.d /usr/lib/modprobe.d /etc/security /etc/sysctl.d /etc/systemd -newermt "$SINCE" 2>/dev/null | head; echo "(end)"
echo "-- device node modes (unchanged vs R0?)"; stat -c '%A %U:%G %n' /dev/kvm /dev/nvidiactl /dev/nvidia0 /dev/nvidia-uvm /dev/udmabuf /dev/userfaultfd /dev/vfio/vfio
getfacl -p /dev/kvm 2>/dev/null | grep -E "user:|group:"
echo "-- cgroups: any kfrootless / kayfabe cgroup"; find /sys/fs/cgroup -maxdepth 3 \( -name "*1002*" -o -name "*kfrootless*" -o -name "*kayfabe*" \) 2>/dev/null | head; echo "(end)"
echo "-- nvidia"; nvidia-smi --query-gpu=name,driver_version,memory.used,display_active,pstate --format=csv,noheader; nvidia-smi --query-compute-apps=pid,name --format=csv,noheader; readlink /sys/bus/pci/devices/0000:01:00.0/driver
echo "-- Xid count (dmesg)"; dmesg | grep -c "NVRM: Xid"; dmesg | grep -i -E "kfrootless|segfault|BUG:|Oops|Call Trace" | tail -3
echo "-- gnome-shell / gdm same pids"; pgrep -u ubuntu -x gnome-shell; pgrep -x gdm3
echo "-- user home footprint"; du -sh /home/kfrootless 2>/dev/null; du -sh /home/kfrootless/.nv /home/kfrootless/.cache /home/kfrootless/.config /home/kfrootless/.local 2>/dev/null; find /home/kfrootless -maxdepth 2 -not -path "*/win/*" -not -path "*/bench/*" | head -30
