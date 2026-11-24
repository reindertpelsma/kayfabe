#!/bin/bash
# waits for driver3, then the reboot lane: 580 host (PR, base), swap to 595.84 (PR, base; 580 guest and matched 595.84 guest)
while ! grep -aq DRIVER3_EXIT /root/prov/driver3_main.log 2>/dev/null; do sleep 15; done
echo "DRIVER4_START $(date -Is)"
W=/workspace/bench/sweep
bash /root/reboot_lane.sh pr_580 $W/hostabi_pr_580/src 580.159.04
bash /root/reboot_lane.sh base_580 $W/hostabi_base_580/src 580.159.04
cd /root/kayfabe; echo "SWAP_TO_595 $(date -Is)"; HOST_DRIVER=595.84 RUN_URL= bash scripts/bench/provision_host_driver.sh > /root/prov/swap_595_d4.log 2>&1; echo "SWAP rc=$? now=$(nvidia-smi --query-gpu=driver_version --format=csv,noheader)"
bash /root/reboot_lane.sh pr_595_g580 $W/hostabi_pr_595/src 580.159.04
bash /root/reboot_lane.sh base_595_g580 $W/hostabi_base_595/src 580.159.04
bash /root/reboot_lane.sh pr_595_g595 $W/hostabi_pr_595g595/src 595.84 /workspace/bench/guest-595.84.qcow2
bash /root/reboot_lane.sh base_595_g595 $W/hostabi_base_595g595/src 595.84 /workspace/bench/guest-595.84.qcow2
echo "DRIVER4_EXIT $(date -Is)"
