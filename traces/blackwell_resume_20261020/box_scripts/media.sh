#!/bin/bash
set -euo pipefail
. /root/bwr/common.sh
cd /root/kayfabe
exec >/root/prov/bwr_media.log 2>&1
trap 'rc=$?; echo "WIN_MEDIA_EXIT rc=$rc $(date -u -Is)"' EXIT
echo "WIN_MEDIA_START $(date -u -Is)"
bash scripts/bench/windows/win_vm.sh fetch
