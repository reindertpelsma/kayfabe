#!/usr/bin/env bash
# Build kf3 QEMU for each revision given (serial; no GPU). Bins land in fixfs/kf3-bins/<rev>/.
set -uo pipefail
W=/var/lib/kf-windows-20261005
F=$W/fixfs
export PATH=$HOME/.cargo/bin:$PATH CARGO_TARGET_DIR=$F/target
cd $F/repo
echo "BISECT_BUILD_START $(date -Is) revs=$*"
for rev in "$@"; do
    git checkout -q --detach "$rev" || { echo "CHECKOUT_FAIL $rev"; continue; }
    short=$(git rev-parse --short=8 HEAD)
    if [ -x "$F/kf3-bins/$short/qemu-system-x86_64" ]; then echo "HAVE $short"; continue; fi
    bash scripts/bench/build_kf3.sh $F/qemu-src $F/qemu-build > $F/build_$short.log 2>&1
    echo "BUILD_EXIT $short $? $(date -Is)"
done
echo "BISECT_BUILD_DONE $(date -Is)"
