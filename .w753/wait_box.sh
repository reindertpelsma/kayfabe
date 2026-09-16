#!/usr/bin/env bash
for i in $(seq 1 60); do
  st=$(vastai show instance 51220903 --raw 2>/dev/null | python3 -c "import json,sys;print(json.load(sys.stdin).get('actual_status'))" 2>/dev/null)
  echo "$(date +%T) actual_status=$st"
  [ "$st" = "running" ] && exit 0
  sleep 10
done
exit 1
