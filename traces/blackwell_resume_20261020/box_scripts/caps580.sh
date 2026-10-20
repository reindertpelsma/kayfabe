#!/bin/bash
set -uo pipefail
exec >/root/prov/bwr_caps580.log 2>&1
echo "CAPS580_START $(date -u -Is)"
cd /root/kayfabe
[ -z "$(pgrep -x qemu-system-x86 || true)" ] || { echo QEMU_RUNNING; exit 3; }
HOST_DRIVER=580.159.04 bash scripts/bench/provision_host_driver.sh >/root/prov/bwr_driver580.log 2>&1; echo "SWAP580_RC=$?"
nvidia-smi --query-gpu=name,driver_version --format=csv,noheader
grep -q 'Open Kernel Module' /proc/driver/nvidia/version && echo OPEN_MODULE
python3 tools/read_display_caps_page.py 0000:00:07.0 > /root/prov/bwr_caps_page_580.txt; echo "READ_RC=$?"
grep -v '^WORD' /root/prov/bwr_caps_page_580.txt
HOST_DRIVER=595.91.07 bash scripts/bench/provision_host_driver.sh >/root/prov/bwr_driver595_back.log 2>&1; echo "SWAP595_RC=$?"
nvidia-smi --query-gpu=name,driver_version --format=csv,noheader
python3 tools/read_display_caps_page.py 0000:00:07.0 > /root/prov/bwr_caps_page_595.txt; echo "READ595_RC=$?"
bash scripts/bench/host_preflight.sh > /root/prov/bwr_preflight_after_swap.log 2>&1; grep VERDICT /root/prov/bwr_preflight_after_swap.log
echo "CAPS580_EXIT $(date -u -Is)"
