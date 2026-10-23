#!/bin/bash
cd /data/claude-hc-20261023/q4lane
for n in 5102 5103 5104; do
  python3 -u ce714-energy-arm.py $n 714ec29d3fdca0a013d9d478755346b0f1af6fa7 hc-6d-ce-at-birth > energy-$n.log 2>&1
  echo "ARM_EXIT $n $? $(date -Is)" >> energy-more.log
  python3 ce714-lifecycles.py $n >> energy-more.log 2>&1
  # stop early on any non-PASS to avoid piling failures
  python3 -c "import json,sys;r=json.load(open(\"energy-bisect/$n/result.json\"));sys.exit(0 if r[\"verdict\"]==\"PASS\" and r[\"restored\"] else 1)" || { echo "STOP_NONPASS $n" >> energy-more.log; break; }
done
echo ALL_DONE >> energy-more.log
