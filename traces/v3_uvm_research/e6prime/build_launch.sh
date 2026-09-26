#!/usr/bin/env bash
set -uo pipefail
cd /root/e6p
I=/root/ogkm/src/common/sdk/nvidia/inc
gcc -O0 -g -o rmlaunch rmlaunch.c -I"$I" -I"$I/class" 2>&1 | head -40
[ -x /root/e6p/rmlaunch ] && echo "BUILD_OK" || echo "BUILD_FAILED"
