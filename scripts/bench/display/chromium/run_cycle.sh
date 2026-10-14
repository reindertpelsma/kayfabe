# guest side: startup/teardown cycling of a Chromium-family app (the Windows failure came on launcher start):
# CFG_KIND chrome|electron, CFG_CYCLES (default 40), CFG_SEC load seconds per cycle (default 20), every 3rd cycle ends with SIGKILL.
export DISPLAY=:0 XAUTHORITY=/home/ubuntu/.Xauthority
for _ in $(seq 40); do pgrep -x cinnamon >/dev/null && break; sleep 2; done
sleep 10
echo "CYCLE_START $(date -Is) kind=$CFG_KIND cycles=${CFG_CYCLES:-40}"
mkdir -p ~/out/cycle
for i in $(seq 1 "${CFG_CYCLES:-40}"); do
    K9=""; [ $((i % 3)) = 0 ] && K9="--kill9"
    python3 ~/kf/cdp_load.py --kind "$CFG_KIND" --name "cycle$i" --minutes "$(python3 -c "print(${CFG_SEC:-20}/60)")" --outdir ~/out/cycle/$i --flags "${CFG_FLAGS:-}" --churn 6 $K9 > ~/out/cycle/$i.log 2>&1
    echo "cycle $i rc=$? $K9 $(date +%T) xid=$(sudo dmesg | grep -c Xid) $(grep -h -E 'GPU_PIDS_FINAL|SAMPLE' ~/out/cycle/$i.log | tail -1 | cut -c1-120)"
    sleep 2
done
echo "CYCLE_DONE $(date -Is)"
