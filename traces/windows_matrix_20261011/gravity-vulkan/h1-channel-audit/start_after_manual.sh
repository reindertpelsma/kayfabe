#!/usr/bin/env bash
set -euo pipefail
trap 'rc=$?; echo "LATE_FIFO_OUTER_EXIT=$rc at=$(date -Is)"' EXIT
echo "LATE_FIFO_WAIT_LOCK=$(date -Is) owner_authorized_takeover=true"
flock -o /tmp/kayfabe-fastguest.lock bash -c '
set -euo pipefail
python3 /tmp/kf-preserve-manual1380.py
bash /tmp/kf-late-fifo-campaign-89e8ddf8.sh
'
