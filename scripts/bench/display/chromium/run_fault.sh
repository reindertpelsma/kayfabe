# guest side: GPU MMU fault injection from CUDA (needs only libcuda), then a health check.
export DISPLAY=:0 XAUTHORITY=/home/ubuntu/.Xauthority
for _ in $(seq 40); do pgrep -x cinnamon >/dev/null && break; sleep 2; done
sleep 10
echo "BEFORE $(date -Is)"; timeout 30 nvidia-smi --query-gpu=name,driver_version --format=csv,noheader
sudo dmesg > /tmp/dmesg.before
timeout 120 python3 -I ~/kf/mmu_fault.py "${CFG_N:-3}"
sleep 5
echo "AFTER $(date -Is)"; timeout 30 nvidia-smi --query-gpu=name,driver_version --format=csv,noheader; echo "SMI_RC=$?"
timeout 10 stdbuf -oL glxgears > /tmp/g.log 2>&1; tail -1 /tmp/g.log | sed 's/^/GEARS /'
sudo dmesg | diff /tmp/dmesg.before - | grep '^>' | cut -c1-260 | head -40
