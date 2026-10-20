#!/bin/bash
# usage: fixbuild.sh <name> <sha> <branch>
set -euo pipefail
. /root/bwr/common.sh
name=$1; sha=$2; br=$3
exec >/root/prov/bwr_${name}_build.log 2>&1
trap 'rc=$?; echo "FIXBUILD_EXIT name=$name rc=$rc $(date -u -Is)"' EXIT
export CARGO_BUILD_JOBS=32
cd /root/kayfabe
git fetch -q origin "$br"
git cat-file -e "$sha"
git diff --name-only $CAND "$sha" -- crates qemu cuda broker Cargo.toml Cargo.lock | tee /root/prov/bwr_${name}_scope.txt
[ -e /root/kayfabe-$name ] || git worktree add --detach /root/kayfabe-$name "$sha"
export CARGO_TARGET_DIR=/root/bwr-$name-target
nice -n 5 bash /root/kayfabe-$name/scripts/bench/build_kf3.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build-kf3-$name
bin=$(echo $sha | cut -c1-8)
[ -x /workspace/bench/kf3-bins/$bin/qemu-system-x86_64 ]
echo $bin > /root/bwr/$name.bin; echo $sha > /root/bwr/$name.src
sha256sum /workspace/bench/kf3-bins/$bin/qemu-system-x86_64 | tee -a /root/prov/bwr_binary_hashes.txt
echo "FIXBUILD_READY name=$name bin=$bin $(date -u -Is)"
