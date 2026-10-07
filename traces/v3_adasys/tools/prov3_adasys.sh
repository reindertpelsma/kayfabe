#!/bin/bash
mkdir -p /root/prov; exec > /root/prov/prov.log 2>&1
echo "START $(date -Is)"; rm -f /root/prov/READY
KAYFABE_BRANCH=v3-adasys bash /root/provision_box.sh > /root/prov/box.log 2>&1; echo "BOX_RC=$?"
export PATH=$HOME/.cargo/bin:$PATH
cd /root/kayfabe || { echo NO_REPO; exit 1; }
git log --oneline -1
bash scripts/bench/provision_host_driver.sh > /root/prov/driver.log 2>&1; echo "DRIVER_RC=$?"
nvidia-smi --query-gpu=name,driver_version --format=csv,noheader
bash scripts/bench/provision_bench_tree.sh > /root/prov/tree.log 2>&1; echo "TREE_RC=$?"; tail -2 /root/prov/tree.log
cargo build -q --release -p kayfabe-rm-ladder --bin kayfabe-rm-ladder; echo "LADDER_RC=$?"
bash scripts/fastguest/build_fast_guest.sh /workspace/bench/guest.qcow2 /workspace/bench/fastguest > /root/prov/fg.log 2>&1; echo "FG_RC=$?"; tail -2 /root/prov/fg.log
bash scripts/bench/build_kf3.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build-kf3 > /root/prov/kf3.log 2>&1; echo "KF3_RC=$?"; tail -1 /root/prov/kf3.log
V=$(nvidia-smi --query-gpu=driver_version --format=csv,noheader)
if [ "$V" = "580.159.04" ] && [ -x /workspace/bench/qemu-build-kf3/qemu-system-x86_64 ] && [ -f /workspace/bench/fastguest/initrd.cpio.gz ]; then
  touch /root/prov/READY; echo "READY driver=$V"
else echo "NOT_READY driver=$V"; fi
echo "EXIT $(date -Is)"
