#!/bin/bash
export HOME=/root
export USER=root
source $HOME/.cargo/env
cd /root/kayfabe
CARGO_BUILD_JOBS=$(nproc) bash scripts/bench/box/merge_check.sh windows-olut-constructor-probe-20261005 cand_probe > /root/merge_check.log 2>&1
