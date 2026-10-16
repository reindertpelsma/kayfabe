echo "== date $(date -Is)"; uname -a; cat /proc/cmdline
echo "== device modes"
ls -ld /dev/kvm /dev/nvidiactl /dev/nvidia0 /dev/nvidia-uvm /dev/nvidia-uvm-tools /dev/nvidia-modeset /dev/nvidia-caps /dev/udmabuf /dev/dri /dev/dri/* /dev/hugepages /dev/shm /dev/vfio /dev/vfio/* /dev/userfaultfd /dev/net/tun /dev/vhost-net /dev/vhost-vsock /dev/fuse 2>&1
ls -l /dev/nvidia-caps 2>&1
getent group kvm video render
echo "== udev rules mentioning nvidia/kvm"
grep -rl -i -E "nvidia|kvm|udmabuf" /etc/udev/rules.d /usr/lib/udev/rules.d /lib/udev/rules.d 2>/dev/null
echo "== modprobe nvidia options"
cat /etc/modprobe.d/*nvidia* 2>/dev/null | grep -v '^#' | head -20
cat /proc/driver/nvidia/params 2>/dev/null | grep -i -E "Device|Modify|Dynamic|Preserve|Temporary|Registry|Open"
echo "== memlock / limits"
ulimit -a | grep -E "locked|open files|max user proc|virtual|address"
cat /etc/security/limits.conf | grep -v '^#' | grep .; ls /etc/security/limits.d; cat /etc/security/limits.d/* 2>/dev/null | grep -v '^#' | grep .
systemctl show -p DefaultLimitMEMLOCK -p DefaultLimitNOFILE -p DefaultTasksMax 2>&1
cat /sys/fs/cgroup/user.slice/user-1000.slice/pids.max 2>&1; cat /sys/fs/cgroup/user.slice/memory.max 2>&1; cat /etc/systemd/system.conf.d/* /etc/systemd/user.conf.d/* 2>/dev/null | head
echo "== sysctls"
for s in vm.unprivileged_userfaultfd kernel.unprivileged_userns_clone kernel.apparmor_restrict_unprivileged_userns user.max_user_namespaces kernel.io_uring_disabled kernel.perf_event_paranoid kernel.yama.ptrace_scope vm.nr_hugepages vm.overcommit_memory vm.max_map_count kernel.unprivileged_bpf_disabled fs.file-max kernel.kptr_restrict vm.mmap_min_addr; do printf '%s=' $s; sysctl -n $s 2>&1; done
echo "== hugetlb / memfd"
grep -i huge /proc/meminfo; mount | grep -E "hugetlb|/dev/shm|tmpfs on /tmp|tmpfs on /run " ; df -h /dev/shm /tmp /run /run/user/1000 2>&1
cat /sys/kernel/mm/transparent_hugepage/enabled /sys/kernel/mm/transparent_hugepage/shmem_enabled
echo "== kvm / iommu"
cat /sys/module/kvm_intel/parameters/nested /sys/module/kvm_amd/parameters/nested 2>&1 | head -2
lsmod | grep -E "^kvm|vfio|udmabuf|nvidia|iommu|nouveau|vhost"
dmesg 2>/dev/null | grep -i -E "iommu|dmar|AMD-Vi" | head -5
ls /sys/kernel/iommu_groups 2>/dev/null | wc -l
lspci -nnk -s $(nvidia-smi --query-gpu=pci.bus_id --format=csv,noheader | sed 's/^0000//') 2>&1 | head
echo "== lsm"
cat /sys/kernel/security/lsm; aa-status 2>&1 | head -3; sestatus 2>&1 | head -1
echo "== display/session"
loginctl list-sessions 2>&1; ps -eo pid,user,comm | grep -E "Xorg|Xwayland|gnome-shell|kwin|sway|weston|gdm|sddm|lightdm" | head; ls -la /run/user/ 2>&1; ls /tmp/.X11-unix 2>&1
echo "== nvidia-smi"
nvidia-smi --query-gpu=name,driver_version,pci.bus_id,memory.used,display_active,persistence_mode,compute_mode --format=csv
nvidia-smi --query-compute-apps=pid,name --format=csv
echo "== caps of root shell"; grep Cap /proc/self/status
echo "== existing lock/leftovers"; ls -la /tmp/kayfabe* /run/kayfabe* /dev/shm 2>&1 | head -30
ps -eo pid,user,etime,cmd | grep -E "qemu|kf3|kayfabe" | grep -v grep | head
