#!/usr/bin/env bash
# Stage this lane's directories on the trusted host (no GPU, no VM touched): W/kf3-bins symlinks, scripts, launcher.
set -euo pipefail
ROOT=/data/claude-trapexp-20261019; HERE=$(cd "$(dirname "$0")" && pwd)
mkdir -p $ROOT/W/kf3-bins $ROOT/disk $ROOT/arms
for l in baseline boundary-tools; do ln -sfn /var/lib/kf-windows-20261005/$l $ROOT/W/$l; done
for d in $ROOT/kf3-bins/*; do ln -sfn $d $ROOT/W/kf3-bins/$(basename $d); done
mkdir -p $ROOT/scripts; [ "$HERE" = "$ROOT/scripts" ] || cp $HERE/*.ps1 $HERE/arm.py $HERE/launch.sh $ROOT/scripts/; cp $HERE/launch.sh $ROOT/launch.sh
ls $ROOT/W/kf3-bins
