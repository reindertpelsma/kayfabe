# LX1 hardware progress

STATUS: IN PROGRESS, 2026-10-19 20:58 UTC. Not a gate or parity verdict.

MEASURED: original owner run 3018 used perf6, binary 0d7ba743 and
KF3_TSG_MIRROR=0. A byte-identical restore copy and private process/configuration
record were saved. Authorised flag shutdown completed with OWNER_SESSION_EXIT=0
at 20:53:45 UTC; actual QEMU, broker and NVIDIA clients were absent. The shared
flock was acquired normally (no contention/break); guardian checks its actual
PID/starttime/inode. Lock remains held during this provisioning phase.

MEASURED: binary MD5 values match preregistration. SHA256 A:
c7aa7528ae552c43a7bc3b8967aea871edcfc1ebfbec3f895b7de617082c299e;
B: d2c8d7490cc24cffd69167baea0803acbb433c312808274cf40f83f74f008c06.
Binary assets have pc-bios/qemu-bundle links to the existing verified QEMU build
resources. The A artifact was staged in its documented immutable revision
directory; no existing other-lane binary/branch was changed.

MEASURED: private full copy of the Linux seed (source unchanged), private guest
key copy and original NoCloud seed prepared. Seed SHA256:
2d0bd14d7088c2417e2db1efe203f5a68a3efa3d1e2dc567f2328eb9eb610687.
Stock QEMU provisioning boot succeeds, SSH and cloud-init complete. Guest:
Ubuntu 24.04.5 noble, kernel 6.8.0-142-generic, initially open 580.159.04 with
matching libcuda. Root initially 3.7 GiB used/28 GiB free, 12 GiB RAM.
Guest internet works through loopback-restricted slirp forwarding; signed noble
apt repositories supply tools, without using the host's newer distro packages.
Kernel upgrade packages and automatic upgrades are held/masked in this private
benchmark guest. No NVIDIA device is attached to the provisioning VM.

MEASURED: matching 595.91.07 installer from NVIDIA official release URL passes
self-integrity check; SHA256
ca23c88dd24b07a191644e1e11cfb7bcdd7537305749af40f980018b095e6313.
Public OGKM tag 595.91.07 resolves to
9f087a6d4e86d85acc0ce1d354d6276fe5047b29. It is staged for a source module
build against the guest's exact headers. No GPU measurement or driver-load gate
has run yet. CUDA toolkit dependencies may install distro driver libraries in
the GPU-free provisioning VM; the final exact-version installer/source build
and loaded-library/module checks must finish before freezing or using any arm.

MEASURED: initial idle host is RTX4070/open595.91.07/kernel7.0.0-34-generic,
P8, 9 MiB used. The complete retained current-boot kernel journal has 528 Xid
records; the owner's ring-based stop message reports 21. Those are different
retention windows. Preserve the full before journal and count subsequent
timestamp-attributed records; never infer a clean delta from a wrapped ring.
GPU reset_method advertises flr and bus; audio function has no reset_method
file. This is capability inventory, not proof of Linux VFIO reset reliability.

MEASURED: launch-plan seed correction followed tests-first: 35 tests run,
three new failures at 46dc2997f, then 35/35 passing at 50d35efd2. A tenth
mutant (omit seed) is killed by all three new tests. No product source changed.
All required Linux mirror gates and all workload/arm parity rows remain UNRUN
at this checkpoint. Large private provisioning logs stay under the lane host
directory; no image, installer, key or binary is committed.
