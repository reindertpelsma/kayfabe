#!/bin/bash
# runwait.sh <tag> <timeout_s> <priority> "<args>" : blrun + blwait on the host, then print the summary
cd /data/claude-blender/wt/scripts/bench/blender
./blrun.sh "$1" "$2" "$3" "$4" >/dev/null
./blwait.sh "$1" $(( $2 + 120 ))
