#!/bin/bash
set -u
B=/var/lib/kf-windows-20261005
rm -rf $B/dm-work/sweep $B/dm-work/sel
cd $B/kayfabe-deferred-translated
git checkout -q -- traces/driver_matrix crates/kf-abi/src/generated/matrix.rs 2>/dev/null
nohup bash -c "echo DM-START \$(date -u +%FT%TZ); PATH=$B/dm-venv/bin:\$PATH DM_WORK=$B/dm-work DM_JOBS=8 bash tools/drivermatrix/regen.sh; echo DM-EXIT rc=\$? \$(date -u +%FT%TZ)" > $B/dm-regen.log 2>&1 < /dev/null &
disown
echo started
