#!/bin/bash
# st.sh : status of every step (start line, exit line, size of the log, last log line)
for s in /workspace/steps/*.start; do n=$(basename $s .start); printf '%-22s %s | %s | log=%s | %s\n' "$n" "$(cut -c1-40 $s)" "$(cat /workspace/steps/$n.exit 2>/dev/null || echo RUNNING)" "$(stat -c%s /workspace/steps/$n.log 2>/dev/null)" "$(tail -c 200 /workspace/steps/$n.log 2>/dev/null | tail -1 | cut -c1-160)"; done
