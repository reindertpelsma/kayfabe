# Rootless audit — pre-registration (written 2026-10-16 19:35 CEST, before any strace or user run)

STATUS: RESEARCH, 2026-10-16. Claim under test (owner): "kayfabe runs as an unprivileged user with only /dev/kvm
access: no root, no VFIO rebind, no kernel module or driver install, the host keeps its GPU and desktop, and
uninstalling is deleting one folder." Binary: kf3-bins/0a746117 (qemu-system-x86_64, kf3 device linked in).
Host: root@172.22.1.20, Ubuntu 26.04, kernel 7.0.0-34, nvidia 595.91.07 open modules, RTX 4070 (AD104).
R0 recon (device modes, sysctls) is read-only and was done before this file; the hypotheses below are about what the
product does when run, which is not yet measured.

Labels: MEASURED = observed in a command run for this audit; INFERRED = reasoned, not observed.

## Hypotheses and falsifiers

H1 (device set). The fast-guest QEMU needs only: /dev/kvm, /dev/nvidiactl, /dev/nvidia0, /dev/nvidia-uvm (all
    non-root openable on this host: kvm via group/ACL, nvidia* are 0666), memfd_create, and optionally /dev/udmabuf
    (root:kvm 0660). It never opens /dev/dri/*, /dev/vfio/*, a PCI sysfs resource file, /dev/mem, /dev/userfaultfd,
    /proc/*/pagemap of other processes, or /sys/kernel/debug.
    FALSIFIER: the root strace shows an open of any of those, or any open of an nvidia node other than the three.

H2 (no privileged RM). Every RM call kayfabe makes is a USER-privilege call: no NV_ESC_* escape that requires
    CAP_SYS_ADMIN, no RMCTRL_FLAGS_PRIVILEGED control. The gates already print PRIVILEGED_CHANNEL=0 privilege=USER, but
    the RM derives client privilege from the CALLER (root => USER_ROOT-class client), so the root runs to date do NOT
    prove this; only a non-root run can.
    FALSIFIER: the non-root run returns NV_ERR_INSUFFICIENT_PERMISSIONS (or any EPERM/EACCES from an RM escape) on any call
    that succeeds as root, in the raw-client path or the QEMU device path.

H3 (memlock). The default RLIMIT_MEMLOCK of an ordinary systemd user (8 MiB) does not block the guest: the product
    pins guest RAM through the nvidia driver (os_lock_user_pages: no RLIMIT_MEMLOCK accounting) and QEMU does not mlock.
    FALSIFIER: any ENOMEM/EPERM at mmap(MAP_LOCKED)/mlock/io_uring register/pin ioctl as kfrootless that is absent as root,
    or a raise of the limit being required for the run to pass.

H4 (udmabuf / hugetlb / userfaultfd). If the product uses /dev/udmabuf it works for group kvm (0660 root:kvm); it does
    not need /dev/hugepages (nr_hugepages=0 here) nor userfaultfd (vm.unprivileged_userfaultfd=0, /dev/userfaultfd 0600 root).
    FALSIFIER: an open of /dev/hugepages, userfaultfd(2) or /dev/userfaultfd in the strace, or a udmabuf failure as the user.

H5 (R1 smallest lane). As kfrootless (groups: kfrootless, kvm only), the smallest fast-guest arm (--timer) reaches
    FAST_VERDICT=PASS with unchanged scripts except for path/BENCH_DIR parameters (the scripts do pkill, nvidia-smi, a lock in
    /tmp, and write to $BENCH; none needs root). FALSIFIER: FAIL/NOTRUN/CRASH with a privilege-class error (EACCES/EPERM)
    in qemu log, or the script itself refusing (pkill of another uid is silently ineffective, which is acceptable).
    Note: the shipped vmlinuz in /workspace/bench/fastguest is mode 0600 root: this is a data-file mode, not a device
    requirement; copying it is expected and is NOT counted as a product privilege dependency.

H6 (v3_gates). The nine kf-gate binaries and gate 10 run as kfrootless and print the same verdicts as root.
    FALSIFIER: any gate verdict differs from the root run of the same binaries on the same host, or births_ok=0.

H7 (no capabilities). /proc/<qemu>/status of the running VM shows CapEff=0000000000000000, NoNewPrivs irrelevant,
    Uid != 0. FALSIFIER: anything else.

H8 (fast suite). 30/30 as root implies 30/30 as kfrootless. FALSIFIER: any arm differing; each differing arm is a
    privilege dependency candidate and is analysed by syscall/ioctl/errno before any fix.

H9 (Windows, R2). Heaven D3D11 300 s as kfrootless is within 5% of the root run of the same binary, with the same
    flags and the same host state. Display: -display none / VNC unix socket; no Wayland/X access is needed unless the
    broker lane is used. FALSIFIER: fps delta > 5% (run-to-run noise measured by a second root run first), or the VM needing
    any file/socket owned by the desktop user.

H10 (footprint, R3). A run creates only: files under the user home / the install folder, the run lock file in /tmp
    (kayfabe-fastguest.lock, a shared file), memfd (anonymous), and sockets under the run directory. No kernel module
    load, no udev change, no cgroup creation, no sysctl, no hugetlb reservation, no leftover process. nvidia-smi, lsmod and the
    desktop compositor are unchanged. FALSIFIER: a diff of lsmod / mount / /proc/sys / ps / ss / ls of /tmp /run /dev/shm /var
    before vs after shows anything not attributable to the user home, the install folder or the shared lock file.

## Rules for this audit
- One failure reason twice => stop and report; no wall-per-boot.
- Nothing system-wide is changed on the host except: creating and finally deleting user kfrootless (and its home).
- All GPU/VM jobs under flock -o /tmp/kayfabe-fastguest.lock. Own pids only.
